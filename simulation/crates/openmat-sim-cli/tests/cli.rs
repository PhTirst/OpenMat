use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}
fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openmat-sim"))
        .args(args)
        .env_remove("OPENMAT_SIM_LLVM_LIBRARY")
        .output()
        .unwrap()
}
fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "openmat-sim-cli-{}-{epoch}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        // This exact absolute directory was newly created and is owned by this test.
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn native_parameter_document_recomputes_cached_values_and_rejects_invalid_source() {
    let directory = TestDirectory::new();
    let path = directory.0.join("parameters.omsim");
    let mut document = serde_json::json!({"format":"openmat-simulation","schemaVersion":9,
        "parameters":{"source":"K = 3;","bindings":{"constant":{"Value":"K"}}},
        "editor":{"labels":{},"bends":{}},
        "model":{"schemaVersion":1,"name":"parameters","settings":{"startTime":0,"stopTime":0.1,"maxStep":0.1},
            "blocks":[{"id":"constant","kind":{"type":"constant","value":[999]}},{"id":"scope","kind":{"type":"scope"}}],
            "connections":[{"from":{"block":"constant","port":"out"},"to":{"block":"scope","port":"in"}}]}});
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let result = run(&["run".as_ref(), path.as_os_str()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(
        json(&result)["result"]["frames"][0]["values"][0].as_f64(),
        Some(3.0)
    );
    document["parameters"]["source"] = serde_json::json!("K = missing;");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(!run(&["run".as_ref(), path.as_os_str()]).status.success());
    document["schemaVersion"] = serde_json::json!(8);
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(!run(&["check".as_ref(), path.as_os_str()]).status.success());
}

#[test]
fn check_and_run_are_machine_readable() {
    let model = fixture("first-order.omsim.json");
    let checked = run(&["check".as_ref(), model.as_os_str()]);
    assert!(checked.status.success());
    assert_eq!(json(&checked)["continuousStates"], 1);
    let result = run(&["run".as_ref(), model.as_os_str()]);
    assert!(result.status.success());
    let document = json(&result);
    assert_eq!(document["execution"]["backend"], "reference");
    let frames = document["result"]["frames"].as_array().unwrap();
    let final_value = frames.last().unwrap()["values"][0].as_f64().unwrap();
    assert!((final_value - (1.0 - (-1.0_f64).exp())).abs() < 3e-8);
}

#[test]
fn moved_conditional_snapshot_runs_without_original_source_files() {
    let directory = TestDirectory::new();
    let target = directory.0.join("counter.omsim");
    fs::copy(fixture("triggered-counter.omsim.json"), &target).unwrap();
    let output = run(&["run".as_ref(), target.as_os_str()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = json(&output);
    let frames = result["result"]["frames"].as_array().unwrap();
    let events = frames
        .iter()
        .filter(|f| {
            f["executionHits"]
                .as_array()
                .is_some_and(|hits| hits.iter().any(|h| h == "counter"))
        })
        .count();
    assert_eq!(events, 5);
    assert_eq!(frames.last().unwrap()["values"][0], 5.0);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn relative_authoring_files_load_m_sources_and_validate_versions_and_paths() {
    let directory = TestDirectory::new();
    fs::copy(fixture("pendulum.m"), directory.0.join("pendulum.m")).unwrap();
    let model: Value =
        serde_json::from_slice(&fs::read(fixture("pendulum.omsim.json")).unwrap()).unwrap();
    let mut document =
        serde_json::json!({"format":"openmat-simulation", "schemaVersion":2,"model":model});
    let path = directory.0.join("pendulum.omsim");
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_openmat-sim"))
            .current_dir(&directory.0)
            .args(["run", "pendulum.omsim"])
            .output()
            .unwrap()
    };
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let result = invoke();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        json(&result)["result"]["frames"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["time"],
        10.0
    );
    document["schemaVersion"] = 1.into();
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let result = invoke();
    assert!(!result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stderr).unwrap()["error"]["code"],
        "model_json"
    );
    document["schemaVersion"] = 2.into();
    document["model"]["blocks"][0]["kind"]["source"] = "../outside.m".into();
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let result = invoke();
    assert!(!result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stderr).unwrap()["error"]["code"],
        "source_file"
    );
}

#[test]
fn llvm_selection_fails_explicitly_when_unconfigured() {
    let model = fixture("first-order.omsim.json");
    let result = run(&[
        "run".as_ref(),
        model.as_os_str(),
        "--backend".as_ref(),
        "llvm".as_ref(),
    ]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let error: Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(error["error"]["code"], "llvm_library");
}

#[test]
fn output_files_are_saved_without_overwriting_existing_files() {
    let directory = TestDirectory::new();
    let output = directory.0.join("result.json");
    let model = fixture("delay-counter.omsim.json");
    let args = [
        "run".as_ref(),
        model.as_os_str(),
        "--output".as_ref(),
        output.as_os_str(),
    ];
    assert!(run(&args).status.success());
    let original = fs::read(&output).unwrap();
    assert!(!run(&args).status.success());
    assert_eq!(original, fs::read(&output).unwrap());
    let result: Value = serde_json::from_slice(&original).unwrap();
    assert!(result["result"]["frames"].as_array().unwrap().len() > 4);
}

#[test]
fn invalid_model_produces_a_port_diagnostic_and_no_result_file() {
    let directory = TestDirectory::new();
    let mut model: Value =
        serde_json::from_slice(&fs::read(fixture("first-order.omsim.json")).unwrap()).unwrap();
    model["connections"][0]["to"]["port"] = "typo".into();
    let model_path = directory.0.join("model.json");
    fs::write(&model_path, serde_json::to_vec(&model).unwrap()).unwrap();
    let output = directory.0.join("result.json");
    let result = run(&[
        "run".as_ref(),
        model_path.as_os_str(),
        "--output".as_ref(),
        output.as_os_str(),
    ]);
    assert!(!result.status.success());
    let error: Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(error["error"]["code"], "unknown_port");
    assert_eq!(error["error"]["block"], "sum");
    assert_eq!(error["error"]["port"], "typo");
    assert!(!output.exists());
}

#[test]
fn emit_llvm_does_not_require_loading_the_native_dependency() {
    let result = run(&[
        "emit-llvm".as_ref(),
        fixture("first-order.omsim.json").as_os_str(),
    ]);
    assert!(result.status.success());
    let ir = String::from_utf8(result.stdout).unwrap();
    assert!(ir.contains("define i32 @openmat_sim_eval"));
}

#[test]
fn storage_limits_and_unknown_options_fail_cleanly() {
    let model = fixture("first-order.omsim.json");
    for args in [
        vec!["--max-samples", "1"],
        vec!["--unknown", "true"],
        vec!["--backend", "invalid"],
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_openmat-sim"));
        let output = command.arg("run").arg(&model).args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stderr).unwrap()["ok"],
            false
        );
    }
}

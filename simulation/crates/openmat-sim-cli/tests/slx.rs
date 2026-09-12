// The shared authored fixture builder also has helpers used by loader tests.
#[allow(dead_code)]
#[path = "../../openmat-sim-slx/tests/support/mod.rs"]
mod support;

use std::process::{Command, Output};

use serde_json::Value;
use support::{Fixture, SYSTEM};

struct TestFile(std::path::PathBuf);
impl TestFile {
    fn new(extension: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "openmat-slx-cli-{}-{epoch}-{}.{extension}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )))
    }
}
impl Drop for TestFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn run(command: &str, file: &TestFile) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openmat-sim"))
        .arg(command)
        .arg(&file.0)
        .env_remove("OPENMAT_SIM_LLVM_LIBRARY")
        .output()
        .unwrap()
}
fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn imports_runs_and_roundtrips_supported_slx_without_touching_source() {
    let source = TestFile::new("SLX");
    let bytes = Fixture::feedback().package();
    std::fs::write(&source.0, &bytes).unwrap();
    let checked = run("check", &source);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert_eq!(json(&checked)["continuousStates"], 1);
    let inspection = run("inspect-slx", &source);
    assert!(inspection.status.success());
    assert_eq!(json(&inspection)["runnable"], true);
    let trajectory = run("run", &source);
    assert!(trajectory.status.success());
    let imported = run("import-slx", &source);
    assert!(imported.status.success());
    let model = TestFile::new("json");
    std::fs::write(&model.0, &imported.stdout).unwrap();
    let roundtrip = run("run", &model);
    assert!(roundtrip.status.success());
    assert_eq!(json(&trajectory)["result"], json(&roundtrip)["result"]);
    let ir = run("emit-llvm", &source);
    assert!(ir.status.success());
    assert!(
        String::from_utf8(ir.stdout)
            .unwrap()
            .contains("define i32 @openmat_sim_eval")
    );
    assert_eq!(std::fs::read(&source.0).unwrap(), bytes);
}

#[test]
fn unsupported_slx_is_inspectable_but_run_and_export_fail() {
    let source = TestFile::new("slx");
    let mut fixture = Fixture::feedback();
    fixture.edit(SYSTEM, "Integrator", "TransferFcn");
    std::fs::write(&source.0, fixture.package()).unwrap();
    let inspection = run("inspect-slx", &source);
    assert!(inspection.status.success());
    assert_eq!(json(&inspection)["runnable"], false);
    assert_eq!(json(&inspection)["issues"][0]["code"], "unsupported_block");
    for command in ["run", "check", "emit-llvm", "import-slx"] {
        let output = run(command, &source);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], "slx_compatibility");
        assert_eq!(error["error"]["issues"][0]["block"], "3");
    }
}

#[test]
fn invalid_packages_fail_with_structured_errors_and_no_output_file() {
    let source = TestFile::new("slx");
    let destination = TestFile::new("json");
    std::fs::write(&source.0, b"invalid package").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_openmat-sim"))
        .arg("inspect-slx")
        .arg(&source.0)
        .arg("--output")
        .arg(&destination.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "zip"
    );
    assert!(!destination.0.exists());
}

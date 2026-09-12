//! Numerical observations of independently authored local R2022b models only.
use openmat_sim::{
    CollectionLimits, Runner, compile_with_sources,
    numeric::{Kernel, ReferenceKernel},
};
use openmat_sim_llvm::LlvmKernel;
use openmat_sim_slx::ImportedSlx;
use serde_json::Value;
use std::path::PathBuf;

#[allow(clippy::too_many_lines)] // Keep the corpus and per-component oracle assertions together.
fn compare(native: bool) {
    let directory = PathBuf::from(std::env::var_os("OPENMAT_SLX_CONTROL_ORACLE_DIR").expect(
        "run Invoke-SlxOracle.ps1 -Profile control and set OPENMAT_SLX_CONTROL_ORACLE_DIR",
    ));
    let status: Value =
        serde_json::from_slice(&std::fs::read(directory.join("oracle-status.json")).unwrap())
            .unwrap();
    assert_eq!(status["ok"], true);
    assert_eq!(status["release"], "2022b");
    let parameters = std::fs::read_to_string(directory.join("parameters.m")).unwrap();
    for name in [
        "om_control_nested",
        "om_control_ss",
        "om_control_ss_direct",
        "om_control_tf",
        "om_control_tf_direct",
        "om_control_math",
        "om_control_routing",
        "om_control_step_000",
        "om_control_step_015",
        "om_control_step_020",
        "om_control_step_030",
        "om_control_step_060",
        "om_control_default_statespace",
        "om_control_default_transferfcn",
        "om_control_default_bias",
        "om_control_default_sin",
        "om_control_default_step",
        "om_control_tf_gain",
    ] {
        let bytes = std::fs::read(directory.join(format!("{name}.slx"))).unwrap();
        let imported = ImportedSlx::read(&bytes, name).unwrap();
        if ["om_control_default_sin", "om_control_default_step"].contains(&name) {
            let defaults: Value = serde_json::from_slice(
                &std::fs::read(directory.join(format!("{name}.parameters.json"))).unwrap(),
            )
            .unwrap();
            assert_eq!(defaults["SampleTime"], "-1");
            let errors = imported
                .lower_control(&parameters)
                .err()
                .expect("inherited source rates require explicit rejection");
            assert!(
                errors.iter().any(
                    |e| e.code == "sample_time" && e.parameter.as_deref() == Some("SampleTime")
                )
            );
            println!("{name}: inherited rate explicitly rejected");
            continue;
        }
        let lowered = imported
            .lower_control(&parameters)
            .unwrap_or_else(|e| panic!("{name}: {e:#?}"));
        let plan = compile_with_sources(&lowered.model, &lowered.sources).unwrap();
        let kernel: Box<dyn Kernel> = if native {
            let library = PathBuf::from(
                std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY")
                    .expect("native acceptance requires LLVM 22"),
            );
            Box::new(LlvmKernel::compile(plan.program().clone(), &library).unwrap())
        } else {
            Box::new(ReferenceKernel::new(plan.program().clone()))
        };
        let result = Runner::new(plan, kernel)
            .unwrap()
            .collect(CollectionLimits::default())
            .unwrap();
        let expected: Value = serde_json::from_slice(
            &std::fs::read(directory.join(format!("{name}.oracle.json"))).unwrap(),
        )
        .unwrap();
        let times = expected["time"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![expected["time"].clone()]);
        let values = if times.len() == 1 {
            vec![expected["values"].clone()]
        } else {
            expected["values"].as_array().unwrap().clone()
        };
        // R2022b logs an inf-rate constant routing signal once. OpenMat Scope
        // records each engine frame, so also verify that every extra frame holds it.
        if times.len() == 1 {
            let expected = values[0]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![values[0].clone()]);
            let expected: Vec<_> = expected.iter().map(|v| v.as_f64().unwrap()).collect();
            for frame in &result.frames {
                assert_eq!(frame.values, expected);
            }
        } else {
            assert_eq!(times.len(), result.frames.len(), "{name}: sample count");
        }
        assert_eq!(times.len(), values.len());
        let mut maximum = 0.0_f64;
        for ((frame, time), values) in result.frames.iter().zip(&times).zip(&values) {
            let time = time.as_f64().unwrap();
            assert!((frame.time - time).abs() < 1e-12, "{name}: sample time");
            let values = values
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![values.clone()]);
            assert_eq!(frame.values.len(), values.len(), "{name}: signal width");
            for (&actual, expected) in frame.values.iter().zip(values) {
                let expected = expected.as_f64().unwrap();
                let difference = (actual - expected).abs();
                maximum = maximum.max(difference);
                assert!(
                    difference <= 2e-12 * expected.abs().max(1.0),
                    "{name}, t={time}: actual={actual}, expected={expected}"
                );
            }
        }
        println!(
            "{name}: {} samples={}, max_abs_error={maximum:.3e}",
            if native { "llvm" } else { "reference" },
            result.frames.len()
        );
        assert_eq!(
            bytes,
            std::fs::read(directory.join(format!("{name}.slx"))).unwrap()
        );
    }
}

#[test]
#[ignore = "requires locally generated OpenMat control models in licensed MATLAB R2022b"]
fn reference_matches_r2022b_control_models() {
    compare(false);
}
#[test]
#[ignore = "requires local R2022b control observations and LLVM 22"]
fn llvm_matches_r2022b_control_models() {
    compare(true);
}

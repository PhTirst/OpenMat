//! Optional differential tests consume only locally generated `OpenMat` models.
use std::path::PathBuf;

use openmat_sim::numeric::{Kernel, ReferenceKernel};
use openmat_sim::{CollectionLimits, Runner, compile};
use openmat_sim_llvm::LlvmKernel;
use openmat_sim_slx::ImportedSlx;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    schema_version: u32,
    model: String,
    time: Vec<f64>,
    values: Vec<serde_json::Value>,
}

fn compare(native: bool) {
    let directory = PathBuf::from(
        std::env::var_os("OPENMAT_SLX_ORACLE_DIR")
            .expect("run simulation/tools/Invoke-SlxOracle.ps1 and set OPENMAT_SLX_ORACLE_DIR"),
    );
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("oracle-status.json")).unwrap())
            .unwrap();
    assert_eq!(status["ok"], true);
    assert_eq!(status["release"], "2022b");
    for name in [
        "om_slx_feedback",
        "om_slx_vector",
        "om_slx_counter",
        "om_slx_sampled",
    ] {
        let bytes = std::fs::read(directory.join(format!("{name}.slx"))).unwrap();
        let imported = ImportedSlx::read(&bytes, name).unwrap();
        let model = imported.lower().unwrap();
        let plan = compile(&model).unwrap();
        let kernel: Box<dyn Kernel> = if native {
            let library = PathBuf::from(
                std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY")
                    .expect("LLVM acceptance requires OPENMAT_SIM_LLVM_LIBRARY"),
            );
            let kernel = LlvmKernel::compile(plan.program().clone(), &library).unwrap();
            kernel.verify_abi_guards().unwrap();
            assert!(kernel.version().starts_with("22."));
            Box::new(kernel)
        } else {
            Box::new(ReferenceKernel::new(plan.program().clone()))
        };
        let result = Runner::new(plan, kernel)
            .unwrap()
            .collect(CollectionLimits::default())
            .unwrap();
        let oracle: Observation = serde_json::from_slice(
            &std::fs::read(directory.join(format!("{name}.oracle.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(oracle.schema_version, 1);
        assert_eq!(oracle.model, name);
        assert_eq!(oracle.time.len(), oracle.values.len());
        assert_eq!(
            result.frames.len(),
            oracle.time.len(),
            "sample count for {name}"
        );
        assert_eq!(result.scopes.len(), 1);
        let mut max_error: f64 = 0.0;
        for ((frame, &time), values) in result.frames.iter().zip(&oracle.time).zip(&oracle.values) {
            assert!((frame.time - time).abs() < 1e-12, "time for {name}");
            let expected = values.as_array().map_or_else(
                || vec![values.as_f64().unwrap()],
                |a| a.iter().map(|v| v.as_f64().unwrap()).collect(),
            );
            assert_eq!(frame.values.len(), expected.len());
            for (&actual, expected) in frame.values.iter().zip(expected) {
                let error = (actual - expected).abs();
                max_error = max_error.max(error);
                assert!(
                    error <= 2e-12 * expected.abs().max(1.0),
                    "{name} t={time} actual={actual} MATLAB={expected}"
                );
            }
        }
        println!(
            "{name}: backend={} samples={} max_abs_error={max_error:.3e}",
            if native { "llvm-orc" } else { "reference" },
            result.frames.len()
        );
        assert_eq!(
            bytes,
            std::fs::read(directory.join(format!("{name}.slx"))).unwrap()
        );
    }
}

#[test]
#[ignore = "requires locally generated MATLAB R2022b oracle packages"]
fn reference_matches_real_r2022b_slx_observations() {
    compare(false);
}

#[test]
#[ignore = "requires locally generated MATLAB R2022b packages and LLVM 22"]
fn llvm_orc_matches_real_r2022b_slx_observations() {
    compare(true);
}

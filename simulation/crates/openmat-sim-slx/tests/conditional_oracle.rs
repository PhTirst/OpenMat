//! Differential checks against OpenMat-authored models in licensed R2022b.
use openmat_sim::{
    CollectionLimits, Runner, compile_with_sources,
    numeric::{Kernel, ReferenceKernel},
};
use openmat_sim_llvm::LlvmKernel;
use openmat_sim_slx::ImportedSlx;
use serde_json::Value;
use std::path::PathBuf;

fn compare(native: bool) {
    let directory = PathBuf::from(
        std::env::var_os("OPENMAT_SLX_CONDITIONAL_ORACLE_DIR")
            .expect("run Invoke-SlxOracle.ps1 -Profile conditional"),
    );
    let status: Value =
        serde_json::from_slice(&std::fs::read(directory.join("oracle-status.json")).unwrap())
            .unwrap();
    assert_eq!(status["ok"], true);
    assert_eq!(status["release"], "2022b");
    let mut cases = std::fs::read_dir(&directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "slx"))
        .collect::<Vec<_>>();
    cases.sort();
    assert!(cases.len() >= 25);
    for path in cases {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let imported = ImportedSlx::read(&std::fs::read(&path).unwrap(), name).unwrap();
        assert!(
            imported.lower_hybrid("").is_err(),
            "old profile must reject conditional execution"
        );
        let lowered = imported
            .lower_conditional("")
            .unwrap_or_else(|e| panic!("{name}: {e:#?}"));
        let plan = compile_with_sources(&lowered.model, &lowered.sources).unwrap();
        let kernel = |p: &openmat_sim::numeric::Program| -> Box<dyn Kernel> {
            if native {
                Box::new(
                    LlvmKernel::compile(
                        p.clone(),
                        &PathBuf::from(
                            std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY")
                                .expect("LLVM runtime required"),
                        ),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(ReferenceKernel::new(p.clone()))
            }
        };
        let result = Runner::new_with_update(
            plan.clone(),
            kernel(plan.program()),
            plan.update_program().map(kernel),
        )
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
        let frames = result
            .frames
            .iter()
            .filter(|f| !f.sample_hits.is_empty())
            .collect::<Vec<_>>();
        let expected: Value = serde_json::from_slice(
            &std::fs::read(directory.join(format!("{name}.oracle.json"))).unwrap(),
        )
        .unwrap();
        let times = expected["time"].as_array().unwrap();
        let values = expected["values"].as_array().unwrap();
        assert_eq!(frames.len(), times.len(), "{name}: frame count");
        for ((frame, time), value) in frames.iter().zip(times).zip(values) {
            assert!((frame.time - time.as_f64().unwrap()).abs() < 1e-12);
            assert_eq!(
                frame.values,
                vec![value.as_f64().unwrap()],
                "{name} at {}",
                frame.time
            );
        }
        println!(
            "{name}: {} samples, {} matched",
            frames.len(),
            if native { "LLVM" } else { "reference" }
        );
    }
}

#[test]
#[ignore = "requires OpenMat-authored R2022b conditional oracle"]
fn conditional_reference_matches_r2022b() {
    compare(false);
}
#[test]
#[ignore = "requires OpenMat-authored R2022b conditional oracle and LLVM runtime"]
fn conditional_llvm_matches_r2022b() {
    compare(true);
}

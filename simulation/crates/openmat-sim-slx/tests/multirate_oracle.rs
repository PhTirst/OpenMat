//! Observations of independently authored local R2022b models, never upstream fixtures.
use openmat_sim::{
    CollectionLimits, Runner, compile_with_sources,
    model::SampleTime,
    numeric::{Kernel, ReferenceKernel},
};
use openmat_sim_llvm::LlvmKernel;
use openmat_sim_slx::ImportedSlx;
use serde_json::Value;
use std::path::PathBuf;

fn kernel(program: &openmat_sim::numeric::Program, native: bool) -> Box<dyn Kernel> {
    if native {
        let path = PathBuf::from(
            std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY").expect("LLVM library required"),
        );
        Box::new(LlvmKernel::compile(program.clone(), &path).unwrap())
    } else {
        Box::new(ReferenceKernel::new(program.clone()))
    }
}

#[allow(clippy::too_many_lines)]
fn compare(native: bool) {
    let directory = PathBuf::from(
        std::env::var_os("OPENMAT_SLX_MULTIRATE_ORACLE_DIR")
            .expect("run Invoke-SlxOracle.ps1 -Profile multirate"),
    );
    let read = |name: &str| -> Value {
        serde_json::from_slice(&std::fs::read(directory.join(name)).unwrap()).unwrap()
    };
    let status = read("oracle-status.json");
    assert_eq!(status["ok"], true);
    assert_eq!(status["release"], "2022b");
    for name in [
        "om_multirate_inherited_continuous",
        "om_multirate_inherited_discrete",
        "om_multirate_hold",
        "om_multirate_discretestatespace",
        "om_multirate_discretetransferfcn",
        "om_multirate_discreteintegrator",
        "om_multirate_rt_fast_to_slow_off",
        "om_multirate_rt_fast_to_slow_on",
        "om_multirate_rt_slow_to_fast_off",
        "om_multirate_rt_slow_to_fast_on",
        "om_multirate_closed_loop_plant",
        "om_multirate_closed_loop_control",
        "om_multirate_closed_loop_monitor",
        "om_multirate_inherited_filter",
    ] {
        let bytes = std::fs::read(directory.join(format!("{name}.slx"))).unwrap();
        let imported = ImportedSlx::read(&bytes, name).unwrap();
        let lowered = imported
            .lower_multirate("")
            .unwrap_or_else(|e| panic!("{name}: {e:#?}"));
        let plan = compile_with_sources(&lowered.model, &lowered.sources).unwrap();
        let sampling = plan.sampling().unwrap().clone();
        let scope_rate = plan.scopes()[0].sample_time.unwrap();
        let update = plan.update_program().map(|p| kernel(p, native));
        let result = Runner::new_with_update(plan.clone(), kernel(plan.program(), native), update)
            .unwrap()
            .collect(CollectionLimits::default())
            .unwrap();
        let frames: Vec<_> = result
            .frames
            .iter()
            .enumerate()
            .filter(|(i, frame)| match scope_rate {
                SampleTime::Discrete { period } => {
                    let clock = sampling
                        .clocks
                        .iter()
                        .find(|c| (c.period - period).abs() < 1e-12)
                        .unwrap();
                    frame.sample_hits.contains(&clock.id)
                }
                SampleTime::Constant => *i == 0,
                SampleTime::Continuous => true,
                SampleTime::Inherited => panic!("unresolved scope"),
            })
            .map(|(_, f)| f)
            .collect();
        let expected = read(&format!("{name}.oracle.json"));
        let times = expected["time"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![expected["time"].clone()]);
        let values = if times.len() == 1 {
            vec![expected["values"].clone()]
        } else {
            expected["values"].as_array().unwrap().clone()
        };
        assert_eq!(frames.len(), times.len(), "{name}: observed sample count");
        let mut maximum = 0.0_f64;
        for ((frame, time), value) in frames.iter().zip(&times).zip(values) {
            assert!(
                (frame.time - time.as_f64().unwrap()).abs() < 1e-12,
                "{name}: sampling instant"
            );
            let values = value.as_array().cloned().unwrap_or_else(|| vec![value]);
            assert_eq!(values.len(), frame.values.len(), "{name}: width");
            for (&actual, expected) in frame.values.iter().zip(values) {
                let expected = expected.as_f64().unwrap();
                let difference = (actual - expected).abs();
                maximum = maximum.max(difference);
                assert!(
                    difference < 2e-12 * expected.abs().max(1.0),
                    "{name}, t={}: actual={actual}, expected={expected}",
                    frame.time
                );
            }
        }
        let rates = read(&format!("{name}.rates.json"));
        for block in imported.document().systems.iter().flat_map(|s| &s.blocks) {
            if ["Scope", "RateTransition"].contains(&block.block_type.as_str()) {
                continue;
            }
            let id = format!("slx_{}", block.sid.replace(':', "_"));
            let rate = &rates[&block.name];
            let compiled = rate[0]
                .as_f64()
                .or_else(|| rate.as_array()?.last()?[0].as_f64());
            match sampling.blocks[&id] {
                SampleTime::Discrete { period } => assert!(
                    (period - compiled.unwrap()).abs() < 1e-12,
                    "{name}: {} rate",
                    block.name
                ),
                SampleTime::Continuous => assert_eq!(compiled, Some(0.0)),
                SampleTime::Constant => assert!(compiled.is_none()),
                SampleTime::Inherited => panic!("unresolved block"),
            }
        }
        println!(
            "{name}: {} observed_samples={}, max_abs_error={maximum:.3e}",
            if native { "llvm" } else { "reference" },
            frames.len()
        );
    }
}

#[test]
#[ignore = "requires original model observations from licensed MATLAB R2022b"]
fn reference_matches_r2022b_multirate_models() {
    compare(false);
}

#[test]
#[ignore = "requires original R2022b observations and configured LLVM runtime"]
fn llvm_matches_r2022b_multirate_models() {
    compare(true);
}

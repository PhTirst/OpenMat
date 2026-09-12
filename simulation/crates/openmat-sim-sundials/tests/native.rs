use openmat_sim::model::{BlockKind, Model};
use openmat_sim::numeric::{Kernel, ReferenceKernel};
use openmat_sim::solver::{ContinuousSolver, OdeStep};
use openmat_sim::{CollectionLimits, Runner, SourceBundle, compile, compile_with_sources};
use openmat_sim_sundials::{Cvode, Method, Options};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn directory() -> PathBuf {
    std::env::var_os("OPENMAT_SIM_SUNDIALS_DIRECTORY")
        .expect("run Prepare-Sundials.ps1 and set OPENMAT_SIM_SUNDIALS_DIRECTORY")
        .into()
}

#[test]
fn missing_runtime_and_invalid_options_are_explicit_errors() {
    assert!(Cvode::new(std::path::Path::new("missing"), 1, Options::default()).is_err());
    assert!(Cvode::new(std::path::Path::new("missing"), 2049, Options::default()).is_err());
}

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn stiff_feedback_and_sample_boundaries() {
    let mut model: Model =
        serde_json::from_str(include_str!("../../../examples/first-order.omsim.json")).unwrap();
    for block in &mut model.blocks {
        match &mut block.kind {
            BlockKind::Constant { value } => value[0] = 1000.,
            BlockKind::Gain { gain } => gain[0] = 1000.,
            _ => {}
        }
    }
    model.settings.max_step = 0.05;
    let plan = compile(&model).unwrap();
    let kernel = ReferenceKernel::new(plan.program().clone());
    let solver = Cvode::new(&directory(), 1, Options::default()).unwrap();
    let mut runner = Runner::new(plan, kernel)
        .unwrap()
        .with_solver(Box::new(solver))
        .unwrap();
    while let Some(frame) = runner.advance().unwrap() {
        assert!(
            (frame.values[0] - (1. - (-1000. * frame.time).exp())).abs() < 2e-5,
            "t={} x={}",
            frame.time,
            frame.values[0]
        );
    }
    assert_eq!(runner.time().to_bits(), 1.0_f64.to_bits());
    let stats = runner.solver_statistics().unwrap();
    assert!(stats.rhs_evaluations > stats.accepted_steps);
    assert!(stats.accepted_steps < 500);
    let model: Model = serde_json::from_str(include_str!(
        "../../../examples/sampled-feedback.omsim.json"
    ))
    .unwrap();
    let plan = compile(&model).unwrap();
    let reference = Runner::new(plan.clone(), ReferenceKernel::new(plan.program().clone()))
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
    let solver = Cvode::new(
        &directory(),
        plan.continuous_state_count(),
        Options {
            relative_tolerance: 1e-9,
            ..Options::default()
        },
    )
    .unwrap();
    let mut runner = Runner::new(plan.clone(), ReferenceKernel::new(plan.program().clone()))
        .unwrap()
        .with_solver(Box::new(solver))
        .unwrap();
    let mut hits = vec![runner.current_frame()];
    while let Some(frame) = runner.advance().unwrap() {
        if frame.sample_hit {
            hits.push(frame);
        }
    }
    let expected: Vec<_> = reference.frames.iter().filter(|f| f.sample_hit).collect();
    assert_eq!(hits.len(), expected.len());
    for (a, b) in hits.iter().zip(expected) {
        assert_eq!(a.time.to_bits(), b.time.to_bits());
        for (x, y) in a.values.iter().zip(&b.values) {
            assert!((x - y).abs() < 1e-5);
        }
    }
    assert_eq!(
        usize::try_from(runner.solver_statistics().unwrap().reinitializations).unwrap(),
        hits.len() - 2
    );
}

#[test]
#[ignore = "requires pinned SUNDIALS and LLVM runtimes"]
fn nonlinear_m_function_reference_and_llvm_with_both_methods() {
    let model: Model =
        serde_json::from_str(include_str!("../../../examples/pendulum.omsim.json")).unwrap();
    let sources: SourceBundle = [(
        "pendulum.m".into(),
        include_str!("../../../examples/pendulum.m").into(),
    )]
    .into();
    let plan = compile_with_sources(&model, &sources).unwrap();
    let expected = Runner::new(plan.clone(), ReferenceKernel::new(plan.program().clone()))
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
    let llvm = PathBuf::from(std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY").expect("LLVM required"));
    for method in [Method::Adams, Method::Bdf] {
        for native in [false, true] {
            let kernel: Box<dyn Kernel> = if native {
                Box::new(
                    openmat_sim_llvm::LlvmKernel::compile(plan.program().clone(), &llvm).unwrap(),
                )
            } else {
                Box::new(ReferenceKernel::new(plan.program().clone()))
            };
            let solver = Cvode::new(
                &directory(),
                2,
                Options {
                    method,
                    relative_tolerance: 1e-9,
                    absolute_tolerance: 1e-11,
                },
            )
            .unwrap();
            let result = Runner::new(plan.clone(), kernel)
                .unwrap()
                .with_solver(Box::new(solver))
                .unwrap()
                .collect(CollectionLimits::default())
                .unwrap();
            let actual = result.frames.last().unwrap();
            for (a, b) in actual
                .values
                .iter()
                .zip(&expected.frames.last().unwrap().values)
            {
                assert!(
                    (a - b).abs() < 2e-5,
                    "{method:?}, native={native}: {a} vs {b}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn callback_failure_and_panic_do_not_cross_ffi_or_commit_candidate() {
    for panic in [false, true] {
        let mut solver = Cvode::new(&directory(), 1, Options::default()).unwrap();
        let mut candidate = [123.];
        let cancel = AtomicBool::new(false);
        let mut rhs = |_: f64, _: &[f64], _: &mut [f64]| {
            assert!(!panic, "authored callback panic test");
            Err(openmat_sim::RunError::new("authored_failure", "test"))
        };
        let error = solver
            .advance(
                OdeStep {
                    time: 0.,
                    boundary: 1.,
                    max_step: 0.1,
                    state: &[1.],
                    candidate: &mut candidate,
                    cancel: &cancel,
                    reinitialize: true,
                },
                &mut rhs,
            )
            .unwrap_err();
        assert_eq!(
            error.code,
            if panic {
                "rhs_panic"
            } else {
                "authored_failure"
            }
        );
        assert_eq!(candidate[0].to_bits(), 123.0_f64.to_bits());
    }
}

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn cancellation_inside_trial_evaluations_preserves_candidate() {
    let mut solver = Cvode::new(&directory(), 1, Options::default()).unwrap();
    let cancel = AtomicBool::new(false);
    let mut calls = 0;
    let mut candidate = [123.];
    let mut rhs = |_: f64, state: &[f64], derivative: &mut [f64]| {
        calls += 1;
        derivative[0] = -1000. * state[0];
        if calls == 3 {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    };
    let error = solver
        .advance(
            OdeStep {
                time: 0.,
                boundary: 1.,
                max_step: 0.1,
                state: &[1.],
                candidate: &mut candidate,
                cancel: &cancel,
                reinitialize: true,
            },
            &mut rhs,
        )
        .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(calls, 3);
    assert_eq!(candidate[0].to_bits(), 123.0_f64.to_bits());
    assert_eq!(solver.statistics().accepted_steps, 0);
}

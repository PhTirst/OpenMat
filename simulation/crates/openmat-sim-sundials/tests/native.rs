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
fn hierarchical_experiment_runs_with_both_cvode_methods() {
    let doc: serde_json::Value = serde_json::from_str(include_str!(
        "../../../examples/experiment-control.omsim.json"
    ))
    .unwrap();
    let mut model: Model = serde_json::from_value(doc["model"].clone()).unwrap();
    if let BlockKind::Inport {
        data: Some(data), ..
    } = &mut model.blocks[0].kind
    {
        data.times = vec![0., 0.137, 0.413, 4.];
        data.values = vec![vec![1., 0.], vec![1., 0.2], vec![1., 0.3], vec![1., 0.9]];
    }
    let plan = compile(&model).unwrap();
    for method in [Method::Adams, Method::Bdf] {
        let solver = Cvode::new(
            &directory(),
            plan.continuous_state_count(),
            Options {
                method,
                relative_tolerance: 1e-9,
                absolute_tolerance: 1e-11,
            },
        )
        .unwrap();
        let actual = Runner::new_with_update(
            plan.clone(),
            ReferenceKernel::new(plan.program().clone()),
            plan.update_program().cloned().map(ReferenceKernel::new),
        )
        .unwrap()
        .with_solver(Box::new(solver))
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
        // CVODE publishes its accepted steps; only input knots and clock hits are mandatory boundaries.
        for t in [0.137, 0.413, 4.] {
            assert!(actual.frames.iter().any(|a| (a.time - t).abs() < 1e-14));
        }
        let output = actual
            .scopes
            .iter()
            .find(|s| s.block == "output")
            .unwrap()
            .offset;
        let a = (-3.0_f64).midpoint(5.0_f64.sqrt());
        let b = (-3.0_f64).midpoint(-5.0_f64.sqrt());
        let c = (2.0 + b) / (a - b);
        for f in &actual.frames {
            let expected = 1.0 + c * (a * f.time).exp() + (-1.0 - c) * (b * f.time).exp();
            assert!(
                (f.values[output] - expected).abs() < 2e-7,
                "{method:?} at {}",
                f.time
            );
        }
    }
}

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn irregular_piecewise_input_is_integrated_without_crossing_knots() {
    let model:Model=serde_json::from_value(serde_json::json!({
        "schemaVersion":7,"name":"triangle-area","settings":{"startTime":0,"stopTime":1,"maxStep":0.1},
        "blocks":[
            {"id":"data","kind":{"type":"inport","port":1,"data":{"times":[0,0.137,0.413,1],"values":[[0],[1],[0],[0]]}}},
            {"id":"integral","kind":{"type":"integrator","initial":[0]}},
            {"id":"output","kind":{"type":"outport","port":1}}
        ],"connections":[
            {"from":{"block":"data","port":"out"},"to":{"block":"integral","port":"in"}},
            {"from":{"block":"integral","port":"out"},"to":{"block":"output","port":"in"}}
        ]
    })).unwrap();
    let plan = compile(&model).unwrap();
    for method in [Method::Adams, Method::Bdf] {
        let solver = Cvode::new(
            &directory(),
            1,
            Options {
                method,
                relative_tolerance: 1e-9,
                absolute_tolerance: 1e-11,
            },
        )
        .unwrap();
        let actual = Runner::new_with_update(
            plan.clone(),
            ReferenceKernel::new(plan.program().clone()),
            plan.update_program().cloned().map(ReferenceKernel::new),
        )
        .unwrap()
        .with_solver(Box::new(solver))
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
        assert!(
            (actual.frames.last().unwrap().values[0] - 0.2065).abs() < 2e-8,
            "{method:?}"
        );
        for t in [0.137, 0.413] {
            assert!(actual.frames.iter().any(|a| (a.time - t).abs() < 1e-14));
        }
        assert!(actual.frames.iter().all(|f| !f.sample_hit));
    }
}

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn multirate_hold_boundaries_restart_cvode_without_creating_extra_samples() {
    for method in [Method::Adams, Method::Bdf] {
        let model: Model = serde_json::from_value(serde_json::json!({
            "schemaVersion":5,"name":"sampled-step-integration",
            "settings":{"startTime":0,"stopTime":0.5,"maxStep":0.01},
            "sampleTimes":{"source":{"kind":"discrete","period":0.1},"monitor":{"kind":"discrete","period":0.2}},
            "blocks":[
                {"id":"source","kind":{"type":"step","time":0.05,"before":[0],"after":[1]}},
                {"id":"state","kind":{"type":"integrator","initial":[0]}},
                {"id":"monitor","kind":{"type":"zeroOrderHold"}},
                {"id":"scope","kind":{"type":"scope"}},
                {"id":"sample_scope","kind":{"type":"scope"}}
            ],
            "connections":[
                {"from":{"block":"source","port":"out"},"to":{"block":"state","port":"in"}},
                {"from":{"block":"state","port":"out"},"to":{"block":"scope","port":"in"}},
                {"from":{"block":"state","port":"out"},"to":{"block":"monitor","port":"in"}},
                {"from":{"block":"monitor","port":"out"},"to":{"block":"sample_scope","port":"in"}}
            ]
        })).unwrap();
        let plan = compile(&model).unwrap();
        let continuous = plan
            .scopes()
            .iter()
            .find(|s| s.block == "scope")
            .unwrap()
            .offset;
        let sampled = plan
            .scopes()
            .iter()
            .find(|s| s.block == "sample_scope")
            .unwrap()
            .offset;
        let update = plan.update_program().cloned().map(ReferenceKernel::new);
        let solver = Cvode::new(
            &directory(),
            1,
            Options {
                method,
                relative_tolerance: 1e-9,
                absolute_tolerance: 1e-11,
            },
        )
        .unwrap();
        let mut runner = Runner::new_with_update(
            plan.clone(),
            ReferenceKernel::new(plan.program().clone()),
            update,
        )
        .unwrap()
        .with_solver(Box::new(solver))
        .unwrap();
        let mut hits = [1, 1];
        let mut held = 0.0;
        while let Some(frame) = runner.advance().unwrap() {
            assert!(
                (frame.values[continuous] - (frame.time - 0.1).max(0.0)).abs() < 2e-8,
                "{method:?} t={}",
                frame.time
            );
            for &id in &frame.sample_hits {
                hits[id] += 1;
            }
            if frame.sample_hits.contains(&1) {
                held = (frame.time - 0.1).max(0.0);
            }
            assert!((frame.values[sampled] - held).abs() < 2e-8);
        }
        assert_eq!(hits, [6, 3]);
        assert_eq!(runner.solver_statistics().unwrap().reinitializations, 4);
    }
}

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn scheduled_steps_use_left_limit_restart_and_do_not_create_discrete_ticks() {
    for event_time in [0.15_f64, 0.2, 0.5] {
        for method in [Method::Adams, Method::Bdf] {
            let model: Model = serde_json::from_value(serde_json::json!({
                "schemaVersion":4, "name":"scheduled-input",
                "settings":{"startTime":0,"stopTime":0.5,"maxStep":0.11,"sampleTime":0.1},
                "blocks":[
                    {"id":"input","kind":{"type":"step","time":event_time,"before":[0],"after":[1]}},
                    {"id":"state","kind":{"type":"integrator","initial":[0]}},
                    {"id":"delay","kind":{"type":"unitDelay","initial":[0]}},
                    {"id":"scope","kind":{"type":"scope"}}
                ],
                "connections":[
                    {"from":{"block":"input","port":"out"},"to":{"block":"state","port":"in"}},
                    {"from":{"block":"input","port":"out"},"to":{"block":"delay","port":"in"}},
                    {"from":{"block":"state","port":"out"},"to":{"block":"scope","port":"in"}}
                ]
            })).unwrap();
            let plan = compile(&model).unwrap();
            let solver = Cvode::new(
                &directory(),
                1,
                Options {
                    method,
                    relative_tolerance: 1e-9,
                    absolute_tolerance: 1e-11,
                },
            )
            .unwrap();
            let update = plan.update_program().cloned().map(ReferenceKernel::new);
            let mut runner = Runner::new_with_update(
                plan.clone(),
                ReferenceKernel::new(plan.program().clone()),
                update,
            )
            .unwrap()
            .with_solver(Box::new(solver))
            .unwrap();
            let mut hit_count = 1;
            let mut saw_event = false;
            while let Some(frame) = runner.advance().unwrap() {
                hit_count += usize::from(frame.sample_hit);
                saw_event |= frame.time.to_bits() == event_time.to_bits();
                assert!(
                    (frame.values[0] - (frame.time - event_time).max(0.0)).abs() < 2e-8,
                    "{method:?}, event={event_time}, t={}, value={}",
                    frame.time,
                    frame.values[0]
                );
            }
            assert!(saw_event);
            assert_eq!(hit_count, 6);
        }
    }
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

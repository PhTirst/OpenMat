use openmat_sim::solver::{ContinuousSolver, OdeEvents, OdeStep};
use openmat_sim::{CollectionLimits, Runner, compile, model::Model, numeric::ReferenceKernel};
use openmat_sim_sundials::{Cvode, Method, Options};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn locates_continuous_reset_between_base_ticks() {
    let directory = PathBuf::from(
        std::env::var_os("OPENMAT_SIM_SUNDIALS_DIRECTORY").expect("native runtime required"),
    );
    let m:Model=serde_json::from_value(serde_json::json!({"schemaVersion":6,"name":"located-reset","settings":{"startTime":0,"stopTime":0.5,"maxStep":0.01},"blocks":[
        {"id":"one","kind":{"type":"constant","value":[1]}},
        {"id":"clock","kind":{"type":"integrator","initial":[-0.137]}},
        {"id":"state","kind":{"type":"resetIntegrator","initial":[0.75],"gain":1,"discrete":false,"reset":"rising"}},
        {"id":"scope","kind":{"type":"scope"}}
    ],"connections":[
        {"from":{"block":"one","port":"out"},"to":{"block":"clock","port":"in"}},
        {"from":{"block":"one","port":"out"},"to":{"block":"state","port":"in"}},
        {"from":{"block":"clock","port":"out"},"to":{"block":"state","port":"reset"}},
        {"from":{"block":"state","port":"out"},"to":{"block":"scope","port":"in"}}
    ]})).unwrap();
    for method in [Method::Adams, Method::Bdf] {
        let p = compile(&m).unwrap();
        let flow = ReferenceKernel::new(p.program().clone());
        let update = p.update_program().cloned().map(ReferenceKernel::new);
        let solver = Cvode::new(
            &directory,
            2,
            Options {
                method,
                relative_tolerance: 1e-10,
                absolute_tolerance: 1e-12,
            },
        )
        .unwrap();
        let r = Runner::new_with_update(p, flow, update)
            .unwrap()
            .with_solver(Box::new(solver))
            .unwrap()
            .collect(CollectionLimits::default())
            .unwrap();
        let resets: Vec<_> = r
            .frames
            .iter()
            .filter(|f| f.events.iter().any(|e| e.kind == "reset"))
            .collect();
        assert_eq!(resets.len(), 1);
        assert!((resets[0].time - 0.137).abs() < 1e-9);
        assert!((resets[0].values[0] - 0.75).abs() < 1e-12);
        assert!((r.frames.last().unwrap().values[0] - 1.113).abs() < 1e-9);
    }
}

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn periodic_resets_restart_and_keep_discrete_clock_identity() {
    let directory = PathBuf::from(std::env::var_os("OPENMAT_SIM_SUNDIALS_DIRECTORY").unwrap());
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../../../examples/periodic-reset.omsim.json")).unwrap();
    let mut model: Model = serde_json::from_value(doc["model"].clone()).unwrap();
    let sources = serde_json::from_value(doc["sources"].clone()).unwrap();
    for either in [false, true] {
        if let openmat_sim::model::BlockKind::ResetIntegrator { reset, .. } = &mut model
            .blocks
            .iter_mut()
            .find(|b| b.id == "state")
            .unwrap()
            .kind
        {
            *reset = if either {
                openmat_sim::hybrid::ResetMode::Either
            } else {
                openmat_sim::hybrid::ResetMode::Rising
            };
        }
        for method in [Method::Adams, Method::Bdf] {
            let plan = openmat_sim::compile_with_sources(&model, &sources).unwrap();
            let flow = ReferenceKernel::new(plan.program().clone());
            let update = plan.update_program().cloned().map(ReferenceKernel::new);
            let solver = Cvode::new(
                &directory,
                2,
                Options {
                    method,
                    relative_tolerance: 1e-10,
                    absolute_tolerance: 1e-12,
                },
            )
            .unwrap();
            let result = Runner::new_with_update(plan, flow, update)
                .unwrap()
                .with_solver(Box::new(solver))
                .unwrap()
                .collect(CollectionLimits::default())
                .unwrap();
            let mut continuous = Vec::new();
            let mut discrete = Vec::new();
            for frame in &result.frames {
                for event in frame.events.iter().filter(|e| e.kind == "reset") {
                    if event.block == "state" {
                        continuous.push(frame.time);
                    }
                    if event.block == "discrete" {
                        discrete.push(frame.time);
                        assert_eq!(frame.sample_hits, vec![0]);
                    }
                }
            }
            assert_eq!(continuous.len(), if either { 8 } else { 4 });
            for (actual, time) in continuous.iter().zip(
                [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]
                    .into_iter()
                    .map(|v| if either { v } else { 2.0 * v }),
            ) {
                assert!((actual - (time + 0.4 / std::f64::consts::TAU)).abs() < 1e-9);
            }
            assert_eq!(discrete.len(), 4);
            for (i, time) in [0.0, 1.0, 2.0, 3.0].into_iter().enumerate() {
                assert!((discrete[i] - (time + 0.1)).abs() < 1e-9);
            }
            assert_eq!(
                result
                    .frames
                    .iter()
                    .filter(|f| f.sample_hits.contains(&0))
                    .count(),
                81
            );
        }
    }
}

#[test]
#[ignore = "requires pinned SUNDIALS runtime"]
fn event_callback_failures_never_publish_candidates_or_unwind_through_c() {
    let directory = PathBuf::from(std::env::var_os("OPENMAT_SIM_SUNDIALS_DIRECTORY").unwrap());
    for (mode, code) in [
        (0, "non_finite_event"),
        (1, "event_panic"),
        (2, "cancelled"),
    ] {
        let mut solver = Cvode::new(&directory, 1, Options::default()).unwrap();
        let cancel = AtomicBool::new(false);
        let mut candidate = [-99.0];
        let mut found = [0];
        let mut rhs = |_: f64, _: &[f64], dx: &mut [f64]| {
            dx[0] = 1.0;
            Ok(())
        };
        let mut roots = |_: f64, _: &[f64], out: &mut [f64]| {
            match mode {
                0 => out[0] = f64::NAN,
                1 => panic!("original callback panic fixture"),
                _ => {
                    cancel.store(true, Ordering::Relaxed);
                    out[0] = 1.0;
                }
            }
            Ok(())
        };
        let error = solver
            .advance_with_events(
                OdeStep {
                    time: 0.0,
                    boundary: 1.0,
                    max_step: 0.1,
                    state: &[0.0],
                    candidate: &mut candidate,
                    cancel: &cancel,
                    reinitialize: true,
                },
                &mut rhs,
                OdeEvents {
                    count: 1,
                    evaluate: &mut roots,
                    found: &mut found,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(candidate[0].to_bits(), (-99.0_f64).to_bits());
    }
}

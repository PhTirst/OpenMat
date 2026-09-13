use openmat_sim::{CollectionLimits, Runner, compile, model::Model, numeric::ReferenceKernel};
use serde_json::{Value, json};

#[allow(clippy::needless_pass_by_value)] // Fixture builders accept temporary JSON values.
fn model(blocks: Value, wires: &[(&str, &str, &str)]) -> Model {
    serde_json::from_value(json!({"schemaVersion":6,"name":"hybrid","settings":{"startTime":0,"stopTime":0.5,"maxStep":0.01},"blocks":blocks,
        "connections":wires.iter().map(|(from,to,port)|json!({"from":{"block":from,"port":"out"},"to":{"block":to,"port":port}})).collect::<Vec<_>>() })).unwrap()
}
#[allow(clippy::needless_pass_by_value)] // Fixture builders accept temporary JSON values.
fn block(id: &str, kind: Value) -> Value {
    json!({"id":id,"kind":kind})
}
fn run(model: &Model) -> openmat_sim::SimulationResult {
    let p = compile(model).unwrap();
    let flow = ReferenceKernel::new(p.program().clone());
    let update = p.update_program().cloned().map(ReferenceKernel::new);
    Runner::new_with_update(p, flow, update)
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap()
}

#[test]
fn control_broadcasts_and_normalizes_logic() {
    for (operation, inputs, expected) in [
        (
            json!({"type":"saturation","lower":[-0.5],"upper":[0.5]}),
            vec![json!([-1, 0, 1])],
            vec![-0.5, 0., 0.5],
        ),
        (
            json!({"type":"switch","criterion":"greaterEqual","threshold":0}),
            vec![json!([10, 20]), json!([-1, 0]), json!([-3])],
            vec![-3., 20.],
        ),
        (
            json!({"type":"relational","operator":"less"}),
            vec![json!([0, 2]), json!([1])],
            vec![1., 0.],
        ),
        (
            json!({"type":"logical","operator":"xor","inputs":2}),
            vec![json!([0, -2, 1]), json!([1, 0, 1])],
            vec![1., 1., 0.],
        ),
        (
            json!({"type":"abs"}),
            vec![json!([-2, 0, 3])],
            vec![2., 0., 3.],
        ),
        (
            json!({"type":"minMax","minimum":true,"inputs":2}),
            vec![json!([-2, 3]), json!([1])],
            vec![-2., 1.],
        ),
    ] {
        let mut blocks = vec![
            block(
                "operation",
                json!({"type":"control","operation":operation,"zeroCrossing":true}),
            ),
            block("scope", json!({"type":"scope"})),
        ];
        let ids: Vec<_> = (0..inputs.len()).map(|i| format!("source{i}")).collect();
        let ports: Vec<_> = (0..inputs.len()).map(|i| format!("in{i}")).collect();
        let mut wires = vec![("operation", "scope", "in")];
        for (i, value) in inputs.into_iter().enumerate() {
            blocks.push(block(&ids[i], json!({"type":"constant","value":value})));
            wires.push((&ids[i], "operation", &ports[i]));
        }
        let result = run(&model(json!(blocks), &wires));
        assert_eq!(result.frames[0].values, expected);
        assert_eq!(result.schema_version, 3);
    }
}

fn reset_model(discrete: bool) -> Model {
    let mut m = model(
        json!([
            block("one", json!({"type":"constant","value":[1]})),
            block("clock", json!({"type":"integrator","initial":[-0.137]})),
            block(
                "state",
                json!({"type":"resetIntegrator","initial":[0.75],"gain":1,"discrete":discrete,"reset":"rising"})
            ),
            block("scope", json!({"type":"scope"}))
        ]),
        &[
            ("one", "clock", "in"),
            ("one", "state", "in"),
            ("clock", "state", "reset"),
            ("state", "scope", "in"),
        ],
    );
    if discrete {
        m.sample_times.insert(
            "state".into(),
            openmat_sim::model::SampleTime::Discrete { period: 0.05 },
        );
    }
    m
}

#[test]
fn reset_occurs_at_major_step_or_own_clock_and_preserves_versions() {
    for (discrete, reset_time, end) in [(false, 0.14, 1.11), (true, 0.15, 1.10)] {
        let m = reset_model(discrete);
        let result = run(&m);
        let events: Vec<_> = result
            .frames
            .iter()
            .filter(|f| f.events.iter().any(|e| e.kind == "reset"))
            .collect();
        assert_eq!(events.len(), 1);
        assert!((events[0].time - reset_time).abs() < 1e-12);
        assert!((events[0].values[0] - 0.75).abs() < 1e-12);
        assert!((result.frames.last().unwrap().values[0] - end).abs() < 1e-12);
        let mut old = m;
        old.schema_version = 5;
        assert_eq!(compile(&old).unwrap_err().0.code, "reset_integrator");
    }
}

#[test]
fn rejects_bad_control_parameters_and_widths() {
    let mut m = model(
        json!([
            block("u", json!({"type":"constant","value":[1,2,3]})),
            block(
                "control",
                json!({"type":"control","operation":{"type":"saturation","lower":[2],"upper":[1]},"zeroCrossing":false})
            )
        ]),
        &[("u", "control", "in0")],
    );
    assert_eq!(compile(&m).unwrap_err().0.code, "control_parameter");
    if let openmat_sim::model::BlockKind::Control { operation, .. } = &mut m.blocks[1].kind {
        *operation = openmat_sim::hybrid::ControlOp::Saturation {
            lower: vec![-1., -1.],
            upper: vec![1.],
        };
    }
    assert_eq!(compile(&m).unwrap_err().0.code, "signal_width");
}

#[test]
fn reset_feedback_requires_a_real_state_boundary() {
    let mut m = reset_model(false);
    m.settings.sample_time = Some(0.01);
    m.connections
        .iter_mut()
        .find(|e| e.to.port == "reset")
        .unwrap()
        .from
        .block = "state".into();
    assert_eq!(compile(&m).unwrap_err().0.code, "reset_algebraic_loop");
    // A hold can sample the just-reset output at a coincident hit, so cannot hide the loop.
    m.sample_times.insert(
        "hold".into(),
        openmat_sim::model::SampleTime::Discrete { period: 0.01 },
    );
    m.blocks
        .push(serde_json::from_value(block("hold", json!({"type":"zeroOrderHold"}))).unwrap());
    m.connections.push(
        serde_json::from_value(
            json!({"from":{"block":"state","port":"out"},"to":{"block":"hold","port":"in"}}),
        )
        .unwrap(),
    );
    m.connections
        .iter_mut()
        .find(|e| e.to.port == "reset")
        .unwrap()
        .from
        .block = "hold".into();
    assert_eq!(compile(&m).unwrap_err().0.code, "reset_algebraic_loop");
    m.blocks.last_mut().unwrap().kind = openmat_sim::model::BlockKind::UnitDelay {
        initial: vec![-1.0],
    };
    assert!(compile(&m).is_ok());
}

#[test]
fn event_failure_preserves_the_last_accepted_frame() {
    let mut m = reset_model(false);
    m.blocks.push(serde_json::from_value(block("trigger", json!({"type":"mFunction","source":"trigger.m","entry":"trigger","parameters":[],"inputs":[{"name":"t","width":1}],"outputWidth":1}))).unwrap());
    m.connections
        .iter_mut()
        .find(|e| e.to.port == "reset")
        .unwrap()
        .from
        .block = "trigger".into();
    m.blocks[1].kind = openmat_sim::model::BlockKind::Integrator { initial: vec![0.0] };
    m.connections.push(
        serde_json::from_value(
            json!({"from":{"block":"clock","port":"out"},"to":{"block":"trigger","port":"t"}}),
        )
        .unwrap(),
    );
    let sources = [(
        "trigger.m".into(),
        "function y = trigger(t)\n y = sqrt(0.025 - t);\nend".into(),
    )]
    .into_iter()
    .collect();
    let plan = openmat_sim::compile_with_sources(&m, &sources).unwrap();
    let update = plan.update_program().cloned().map(ReferenceKernel::new);
    let mut runner = Runner::new_with_update(
        plan.clone(),
        ReferenceKernel::new(plan.program().clone()),
        update,
    )
    .unwrap();
    runner.advance().unwrap();
    runner.advance().unwrap();
    let last = serde_json::to_value(runner.current_frame()).unwrap();
    assert!(runner.advance().is_err());
    assert!(runner.is_failed());
    assert_eq!(serde_json::to_value(runner.current_frame()).unwrap(), last);
}

#[test]
fn vector_reduction_and_logical_state_validation() {
    let mut m = model(
        json!([
            block("u", json!({"type":"constant","value":[2,-3,1]})),
            block(
                "min",
                json!({"type":"control","operation":{"type":"minMax","minimum":true,"inputs":1},"zeroCrossing":true})
            ),
            block("scope", json!({"type":"scope"}))
        ]),
        &[("u", "min", "in0"), ("min", "scope", "in")],
    );
    assert_eq!(run(&m).frames[0].values, vec![-3.0]);
    m.blocks[1].kind = serde_json::from_value(json!({"type":"control","operation":{"type":"logical","operator":"not","inputs":1},"zeroCrossing":false})).unwrap();
    m.settings.sample_time = Some(0.01);
    m.blocks.push(
        serde_json::from_value(block(
            "delay",
            json!({"type":"unitDelay","initial":[2,0,0]}),
        ))
        .unwrap(),
    );
    m.connections.push(
        serde_json::from_value(
            json!({"from":{"block":"min","port":"out"},"to":{"block":"delay","port":"in"}}),
        )
        .unwrap(),
    );
    assert_eq!(compile(&m).unwrap_err().0.code, "signal_type");
}

#[test]
fn shipped_hybrid_examples_are_self_contained_and_observe_reset_clocks() {
    for text in [
        include_str!("../../../examples/saturated-pi.omsim.json"),
        include_str!("../../../examples/switched-control.omsim.json"),
        include_str!("../../../examples/periodic-reset.omsim.json"),
    ] {
        let doc: Value = serde_json::from_str(text).unwrap();
        let m = serde_json::from_value(doc["model"].clone()).unwrap();
        let sources = serde_json::from_value(doc["sources"].clone()).unwrap();
        let plan = openmat_sim::compile_with_sources(&m, &sources).unwrap();
        let update = plan.update_program().cloned().map(ReferenceKernel::new);
        let result = Runner::new_with_update(
            plan.clone(),
            ReferenceKernel::new(plan.program().clone()),
            update,
        )
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
        assert_eq!(result.frames.len(), 401);
        if plan.name() == "Periodic edge reset" {
            let resets: Vec<_> = result
                .frames
                .iter()
                .flat_map(|f| {
                    f.events
                        .iter()
                        .filter(|e| e.kind == "reset")
                        .map(move |e| (f, e))
                })
                .collect();
            assert_eq!(resets.len(), 8);
            for (f, e) in resets.iter().filter(|(_, e)| e.block == "discrete") {
                assert!(!f.sample_hits.is_empty());
                assert_eq!(e.surface, "reset");
            }
        }
    }
}

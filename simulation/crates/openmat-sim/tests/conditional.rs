// These tests compare exact integer states and binary fractions.
#![allow(clippy::float_cmp)]

use openmat_sim::{
    CollectionLimits, Runner, SourceBundle, compile, compile_with_sources,
    model::Model,
    numeric::{Instruction, Kernel, Program, ReferenceKernel},
};
use serde_json::{Value, json};

fn wire(a: &str, p: &str, b: &str, q: &str) -> Value {
    json!({"from":{"block":a,"port":p},"to":{"block":b,"port":q}})
}
fn counter(execution: &Value, initial: f64) -> Model {
    let port = if execution["type"] == "enabled" {
        "enable"
    } else {
        "trigger"
    };
    let levels = [initial, 0., 0., 1., 1., 0., 0., -1., 0., 1.];
    let mut blocks = vec![
        json!({"id":"base","kind":{"type":"constant","value":[initial]}}),
        json!({"id":"control","kind":{"type":"sum","signs":[1,1,1,1,1,1,1,1,1,1]}}),
        json!({"id":"counter","kind":{"type":"subsystem","inputs":0,"outputs":1,"execution":execution}}),
        json!({"id":"one","parent":"counter","kind":{"type":"constant","value":[1]}}),
        json!({"id":"memory","parent":"counter","kind":{"type":"unitDelay","initial":[3]}}),
        json!({"id":"increment","parent":"counter","kind":{"type":"sum","signs":[1,1]}}),
        json!({"id":"out","parent":"counter","kind":{"type":"outport","port":1}}),
        json!({"id":"scope","kind":{"type":"scope"}}),
    ];
    let mut wires = vec![
        wire("base", "out", "control", "in0"),
        wire("control", "out", "counter", port),
        wire("memory", "out", "increment", "in0"),
        wire("one", "out", "increment", "in1"),
        wire("increment", "out", "memory", "in"),
        wire("memory", "out", "out", "in"),
        wire("counter", "out1", "scope", "in"),
    ];
    let mut rates = serde_json::Map::new();
    for i in 1..levels.len() {
        let id = format!("step{i}");
        blocks.push(json!({"id":id,"kind":{"type":"step","time":f64::from(u32::try_from(i).unwrap())*0.1,"before":[0],"after":[levels[i]-levels[i-1]]}}));
        wires.push(wire(&id, "out", "control", &format!("in{i}")));
        rates.insert(id, json!({"kind":"discrete","period":0.1}));
    }
    serde_json::from_value(json!({"schemaVersion":8,"name":"conditional counter","settings":{"startTime":0,"stopTime":0.9,"maxStep":0.01},"blocks":blocks,"connections":wires,"sampleTimes":rates})).unwrap()
}
fn runner(model: &Model) -> Runner<ReferenceKernel> {
    let p = compile(model).unwrap();
    let flow = ReferenceKernel::new(p.program().clone());
    let update = p.update_program().cloned().map(ReferenceKernel::new);
    Runner::new_with_update(p, flow, update).unwrap()
}
fn enabled(states: &str, output: &str) -> Value {
    json!({"type":"enabled","period":0.1,"statesWhenEnabling":states,"outputs":[{"initial":[-7],"whenDisabled":output}]})
}
fn triggered(edge: &str) -> Value {
    json!({"type":"triggered","period":0.1,"edge":edge,"outputs":[{"initial":[-7],"whenDisabled":"held"}]})
}
fn samples(model: &Model) -> Vec<f64> {
    let result = runner(model).collect(CollectionLimits::default()).unwrap();
    result
        .frames
        .iter()
        .filter(|f| !f.sample_hits.is_empty())
        .map(|f| f.values[0])
        .collect()
}

#[test]
fn enabled_state_and_output_policies_are_independent() {
    for (states, output, initial, expected) in [
        (
            "held",
            "held",
            -1.,
            vec![-7., -7., -7., 3., 4., 4., 4., 4., 4., 5.],
        ),
        (
            "held",
            "reset",
            -1.,
            vec![-7., -7., -7., 3., 4., -7., -7., -7., -7., 5.],
        ),
        (
            "reset",
            "held",
            -1.,
            vec![-7., -7., -7., 3., 4., 4., 4., 4., 4., 3.],
        ),
        (
            "reset",
            "reset",
            -1.,
            vec![-7., -7., -7., 3., 4., -7., -7., -7., -7., 3.],
        ),
        (
            "held",
            "held",
            1.,
            vec![3., 3., 3., 4., 5., 5., 5., 5., 5., 6.],
        ),
    ] {
        assert_eq!(
            samples(&counter(&enabled(states, output), initial)),
            expected,
            "{states}/{output}/{initial}"
        );
    }
}

#[test]
fn trigger_edges_initialize_without_firing_and_suppress_adjacent_zero_crossings() {
    for (edge, expected) in [
        ("rising", vec![-7., 3., 3., 4., 4., 4., 4., 4., 5., 5.]),
        ("falling", vec![-7., -7., -7., -7., -7., 3., 3., 4., 4., 4.]),
        ("either", vec![-7., 3., 3., 4., 4., 5., 5., 6., 7., 7.]),
    ] {
        assert_eq!(samples(&counter(&triggered(edge), -1.)), expected, "{edge}");
    }
    for initial in [-1., 0., 1.] {
        let result = runner(&counter(&triggered("rising"), initial))
            .collect(CollectionLimits::default())
            .unwrap();
        assert!(result.frames[0].execution_hits.is_empty());
        let times: Vec<_> = result
            .frames
            .iter()
            .filter(|f| !f.execution_events.is_empty())
            .map(|f| f.time)
            .collect();
        assert_eq!(times.len(), if initial < 0. { 3 } else { 2 });
        assert!(
            result
                .frames
                .iter()
                .filter(|f| f.time > 0.8)
                .all(|f| f.execution_events.is_empty())
        );
    }
}

#[test]
fn enabled_forward_euler_discards_the_pending_increment_when_disabled() {
    for initial in [-1., 1.] {
        let mut model = counter(&enabled("held", "held"), initial);
        model
            .blocks
            .iter_mut()
            .find(|b| b.id == "memory")
            .unwrap()
            .kind =
            serde_json::from_value(json!({"type":"discreteIntegrator","initial":[3],"gain":5}))
                .unwrap();
        model
            .connections
            .iter_mut()
            .find(|e| e.to.block == "memory")
            .unwrap()
            .from
            .block = "one".into();
        let expected = if initial < 0. {
            vec![-7., -7., -7., 3., 3.5, 3.5, 3.5, 3.5, 3.5, 3.5]
        } else {
            vec![3., 3., 3., 3., 3.5, 3.5, 3.5, 3.5, 3.5, 3.5]
        };
        assert_eq!(samples(&model), expected);
        // An unrelated faster clock must not change the domain's enable history.
        model.blocks.push(
            serde_json::from_value(json!({"id":"fast","kind":{"type":"zeroOrderHold"}})).unwrap(),
        );
        model
            .connections
            .push(serde_json::from_value(wire("base", "out", "fast", "in")).unwrap());
        model.sample_times.insert(
            "fast".into(),
            openmat_sim::model::SampleTime::Discrete { period: 0.05 },
        );
        let p = compile(&model).unwrap();
        let clock = p
            .sampling()
            .unwrap()
            .clocks
            .iter()
            .find(|c| (c.period - 0.1).abs() < 1e-12)
            .unwrap()
            .id;
        let result = runner(&model).collect(CollectionLimits::default()).unwrap();
        let values: Vec<_> = result
            .frames
            .iter()
            .filter(|f| f.sample_hits.contains(&clock))
            .map(|f| f.values[0])
            .collect();
        assert_eq!(values, expected);
    }
}

#[test]
fn unsupported_domains_and_old_schema_do_not_silently_flatten() {
    let m = counter(&triggered("rising"), -1.);
    let mut old = m.clone();
    old.schema_version = 7;
    assert_eq!(compile(&old).unwrap_err().0.code, "schema_version");
    let mut continuous = m.clone();
    continuous
        .blocks
        .iter_mut()
        .find(|b| b.id == "memory")
        .unwrap()
        .kind = serde_json::from_value(json!({"type":"integrator","initial":[3]})).unwrap();
    assert_eq!(compile(&continuous).unwrap_err().0.code, "conditional_body");
    let mut explicit = m;
    explicit.sample_times.insert(
        "memory".into(),
        openmat_sim::model::SampleTime::Discrete { period: 0.1 },
    );
    assert_eq!(
        compile(&explicit).unwrap_err().0.code,
        "triggered_sample_time"
    );
}

#[test]
fn guarded_ir_is_verified_pruned_and_preserves_inactive_register_values() {
    let p = Program::new(
        2,
        vec![
            Instruction::Input(0),
            Instruction::Input(1),
            Instruction::Constant(1.),
            Instruction::Divide(2, 1),
        ],
        vec![3],
    )
    .unwrap()
    .with_guards(vec![None, None, None, Some(0)])
    .unwrap()
    .pruned();
    let mut k = ReferenceKernel::new(p);
    let mut out = [99.];
    k.evaluate(&[0., 0.], &mut out).unwrap();
    assert_eq!(out, [0.]);
    k.evaluate(&[1., 2.], &mut out).unwrap();
    assert_eq!(out, [0.5]);
    assert!(
        Program::new(0, vec![Instruction::Constant(1.)], vec![0])
            .unwrap()
            .with_guards(vec![Some(0)])
            .is_err()
    );
}

#[test]
fn inactive_m_callback_is_guarded_and_failure_preserves_last_frame() {
    let mut raw = serde_json::to_value(counter(&enabled("held", "held"), -1.)).unwrap();
    raw["blocks"].as_array_mut().unwrap().push(json!({"id":"unsafe","parent":"counter","kind":{"type":"mFunction","source":"inverse.m","entry":"inverse","inputs":[{"name":"u","width":1}],"parameters":[],"outputWidth":1}}));
    raw["blocks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"zero","parent":"counter","kind":{"type":"constant","value":[0]}}));
    raw["connections"]
        .as_array_mut()
        .unwrap()
        .push(wire("zero", "out", "unsafe", "u"));
    let model: Model = serde_json::from_value(raw).unwrap();
    let sources = SourceBundle::from([(
        "inverse.m".into(),
        "function y = inverse(u)\ny = 1 / u;\nend\n".into(),
    )]);
    let p = compile_with_sources(&model, &sources).unwrap();
    assert!(
        p.update_program()
            .unwrap()
            .guards()
            .iter()
            .any(Option::is_some)
    );
    let mut r = Runner::new_with_update(
        p.clone(),
        ReferenceKernel::new(p.program().clone()),
        p.update_program().cloned().map(ReferenceKernel::new),
    )
    .unwrap();
    loop {
        let before = json!(r.current_frame());
        match r.advance() {
            Ok(Some(_)) => {}
            Err(e) => {
                assert_eq!(e.code, "non_finite_output");
                assert_eq!(json!(r.current_frame()), before);
                assert!(r.time() < 0.3);
                break;
            }
            Ok(None) => panic!("enabled invalid callback must fail"),
        }
    }
}

#[test]
fn coincident_external_resets_do_not_commit_conditional_state_twice() {
    let mut model = counter(&triggered("rising"), -1.);
    let expected = samples(&model);
    for block in [
        json!({"id":"reset_input","kind":{"type":"constant","value":[0]}}),
        json!({"id":"reset_state","kind":{"type":"resetIntegrator","initial":[0],"gain":1,"discrete":true,"reset":"rising"}}),
    ] {
        model.blocks.push(serde_json::from_value(block).unwrap());
    }
    model.connections.extend([
        serde_json::from_value(wire("reset_input", "out", "reset_state", "in")).unwrap(),
        serde_json::from_value(wire("control", "out", "reset_state", "reset")).unwrap(),
    ]);
    model.sample_times.insert(
        "reset_state".into(),
        openmat_sim::model::SampleTime::Discrete { period: 0.1 },
    );
    let result = runner(&model).collect(CollectionLimits::default()).unwrap();
    assert!(
        result
            .frames
            .iter()
            .any(|f| !f.events.is_empty() && !f.execution_hits.is_empty())
    );
    assert_eq!(samples(&model), expected);
}

#[test]
fn copied_triggered_m_instances_own_their_states_and_scope_invocations() {
    let doc: Value = serde_json::from_str(include_str!(
        "../../../examples/triggered-counter.omsim.json"
    ))
    .unwrap();
    let mut model: Model = serde_json::from_value(doc["model"].clone()).unwrap();
    let sources: SourceBundle = serde_json::from_value(doc["sources"].clone()).unwrap();
    let ids: std::collections::BTreeSet<_> = model
        .blocks
        .iter()
        .filter(|b| b.id == "counter" || b.parent.as_deref() == Some("counter"))
        .map(|b| b.id.clone())
        .collect();
    let copies: Vec<_> = model
        .blocks
        .iter()
        .filter(|b| ids.contains(&b.id))
        .map(|b| {
            let mut b = b.clone();
            b.id = format!("copy_{}", b.id);
            b.parent = b.parent.map(|p| format!("copy_{p}"));
            b
        })
        .collect();
    let wires: Vec<_> = model
        .connections
        .iter()
        .filter(|e| ids.contains(&e.to.block))
        .map(|e| {
            let mut e = e.clone();
            e.to.block = format!("copy_{}", e.to.block);
            if ids.contains(&e.from.block) {
                e.from.block = format!("copy_{}", e.from.block);
            }
            e
        })
        .collect();
    model.blocks.extend(copies);
    model.connections.extend(wires);
    let p = compile_with_sources(&model, &sources).unwrap();
    let result = Runner::new_with_update(
        p.clone(),
        ReferenceKernel::new(p.program().clone()),
        p.update_program().cloned().map(ReferenceKernel::new),
    )
    .unwrap()
    .collect(CollectionLimits::default())
    .unwrap();
    for (scope, domain) in [
        ("invocations", "counter"),
        ("copy_invocations", "copy_counter"),
    ] {
        let info = result.scopes.iter().find(|s| s.block == scope).unwrap();
        assert_eq!(info.execution.as_deref(), Some(domain));
        let frames: Vec<_> = result
            .frames
            .iter()
            .filter(|f| f.execution_hits.iter().any(|d| d == domain))
            .collect();
        assert_eq!(frames.len(), 5);
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(
                frame.values[info.offset],
                f64::from(u32::try_from(index + 1).unwrap())
            );
            assert!((frame.time - (0.1 + f64::from(u32::try_from(index).unwrap()))).abs() < 1e-12);
        }
    }
}

#[test]
fn logical_initial_outputs_are_validated_at_conditional_boundaries() {
    let mut model = counter(&enabled("held", "held"), -1.);
    model.blocks.push(serde_json::from_value(json!({"id":"logic","parent":"counter","kind":{"type":"control","operation":{"operator":"not","type":"logical","inputs":1},"zeroCrossing":false}})).unwrap());
    model
        .connections
        .push(serde_json::from_value(wire("one", "out", "logic", "in0")).unwrap());
    let output = model
        .connections
        .iter_mut()
        .find(|e| e.to.block == "out")
        .unwrap();
    output.from.block = "logic".into();
    assert_eq!(compile(&model).unwrap_err().0.code, "signal_type");
}

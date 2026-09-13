#![allow(clippy::float_cmp)] // Held buffers and transactional rollback require exact equality.
use openmat_sim::{
    CollectionLimits, Runner, SourceBundle, compile_with_sources, model::Model,
    numeric::ReferenceKernel,
};
use serde_json::{Value, json};

fn model(blocks: Value, connections: Value, rates: Value) -> Model {
    let mut raw = json!({"schemaVersion":5,"name":"authored sampled model", "settings":{"startTime":0,"stopTime":0.5,"maxStep":0.01}});
    raw["blocks"] = blocks;
    raw["connections"] = connections;
    raw["sampleTimes"] = rates;
    serde_json::from_value(raw).unwrap()
}
fn edge(from: &str, to: &str) -> Value {
    json!({"from":{"block":from,"port":"out"},"to":{"block":to,"port":"in"}})
}
fn clock(period: f64) -> Value {
    json!({"kind":"discrete","period":period})
}
fn runner(model: &Model, sources: &SourceBundle) -> Runner<ReferenceKernel> {
    let plan = compile_with_sources(model, sources).unwrap();
    let update = plan.update_program().cloned().map(ReferenceKernel::new);
    Runner::new_with_update(
        plan.clone(),
        ReferenceKernel::new(plan.program().clone()),
        update,
    )
    .unwrap()
}

#[test]
fn sampled_step_holds_between_ticks_and_zero_order_hold_integrates_that_hold() {
    let m = model(
        json!([
            {"id":"source","kind":{"type":"step","time":0.05,"before":[0],"after":[1]}},
            {"id":"sample","kind":{"type":"zeroOrderHold"}},
            {"id":"state","kind":{"type":"integrator","initial":[0]}},
            {"id":"scope","kind":{"type":"scope"}}
        ]),
        json!([
            edge("source", "sample"),
            edge("sample", "state"),
            edge("state", "scope")
        ]),
        json!({"source":{"kind":"continuous"},"sample":clock(0.1)}),
    );
    let result = runner(&m, &SourceBundle::new())
        .collect(CollectionLimits::default())
        .unwrap();
    for frame in result.frames {
        assert!((frame.values[0] - (frame.time - 0.1).max(0.0)).abs() < 1e-13);
    }
}

#[test]
fn inherited_source_and_gain_backpropagate_from_an_explicit_delay_clock() {
    let m = model(
        json!([
            {"id":"source","kind":{"type":"step","time":0.05,"before":[0],"after":[1]}},
            {"id":"gain","kind":{"type":"gain","gain":[2]}},
            {"id":"state","kind":{"type":"unitDelay","initial":[-1]}},
            {"id":"scope","kind":{"type":"scope"}}
        ]),
        json!([
            edge("source", "gain"),
            edge("gain", "state"),
            edge("state", "scope")
        ]),
        json!({"state":clock(0.1)}),
    );
    let plan = compile_with_sources(&m, &SourceBundle::new()).unwrap();
    let sampling = plan.sampling().unwrap();
    assert_eq!(sampling.blocks["source"], sampling.blocks["state"]);
    assert_eq!(sampling.blocks["gain"], sampling.blocks["state"]);
    let result = runner(&m, &SourceBundle::new())
        .collect(CollectionLimits::default())
        .unwrap();
    assert_eq!(result.frames[0].values[0], -1.0);
    assert_eq!(result.frames[10].values[0], 0.0);
    assert_eq!(result.frames[20].values[0], 2.0);
    assert!(result.frames[9].sample_hits.is_empty());
    assert_eq!(result.frames[10].sample_hits, vec![0]);
}

#[test]
fn deterministic_slow_to_fast_transition_delays_one_source_period() {
    for deterministic in [false, true] {
        let m = model(
            json!([
                {"id":"source","kind":{"type":"step","time":0.05,"before":[0],"after":[1]}},
                {"id":"transfer","kind":{"type":"rateTransition","initial":[-2],"deterministic":deterministic}},
                {"id":"scope","kind":{"type":"scope"}}
            ]),
            json!([edge("source", "transfer"), edge("transfer", "scope")]),
            json!({"source":clock(0.1),"transfer":clock(0.02)}),
        );
        let result = runner(&m, &SourceBundle::new())
            .collect(CollectionLimits::default())
            .unwrap();
        assert_eq!(
            result.frames[0].values[0],
            if deterministic { -2.0 } else { 0.0 }
        );
        assert_eq!(
            result.frames[10].values[0],
            if deterministic { 0.0 } else { 1.0 }
        );
        assert_eq!(result.frames[20].values[0], 1.0);
        assert_eq!(result.frames[10].sample_hits, vec![0, 1]);
    }
}

#[test]
fn fast_to_slow_transition_samples_the_current_fast_output_on_coincidence() {
    for deterministic in [false, true] {
        let m = model(
            json!([
                {"id":"source","kind":{"type":"step","time":0.07,"before":[0],"after":[1]}},
                {"id":"transfer","kind":{"type":"rateTransition","initial":[-2],"deterministic":deterministic}},
                {"id":"scope","kind":{"type":"scope"}}
            ]),
            json!([edge("source", "transfer"), edge("transfer", "scope")]),
            json!({"source":clock(0.02),"transfer":clock(0.1)}),
        );
        let result = runner(&m, &SourceBundle::new())
            .collect(CollectionLimits::default())
            .unwrap();
        assert_eq!(result.frames[8].values[0], 0.0);
        assert_eq!(result.frames[10].values[0], 1.0);
    }
}

fn component_model(update: &str) -> (Model, SourceBundle) {
    let mut m = model(
        json!([
            {"id":"component","kind":{"type":"component","component":"counter"}},
            {"id":"scope","kind":{"type":"scope"}},
            {"id":"fast","kind":{"type":"step","time":0.05,"before":[0],"after":[1]}},
            {"id":"fast_scope","kind":{"type":"scope"}}
        ]),
        json!([edge("component", "scope"), edge("fast", "fast_scope")]),
        json!({"fast":clock(0.1)}),
    );
    m.components = serde_json::from_value(json!([{
        "id":"counter","name":"Counter","category":"Tests","icon":"delay",
        "inputs":[],"outputs":[{"name":"out","width":1}],"parameters":[],
        "continuousStates":0,"discreteStates":1,"sampleTime":0.2,
        "initialize":{"source":"initialize.m","entry":"initialize"},
        "outputsFunction":{"source":"outputs.m","entry":"outputs"},
        "update":{"source":"update.m","entry":"update"}
    }]))
    .unwrap();
    (
        m,
        SourceBundle::from([
            (
                "initialize.m".into(),
                "function y=initialize(p)\ny=0;\nend\n".into(),
            ),
            (
                "outputs.m".into(),
                "function y=outputs(t,x,q,u,p)\ny=q+t;\nend\n".into(),
            ),
            (
                "update.m".into(),
                format!("function y=update(t,x,q,u,p)\n{update}\nend\n"),
            ),
        ]),
    )
}

#[test]
fn distinct_component_clocks_hold_outputs_and_only_publish_their_own_states() {
    let (m, sources) = component_model("y=q+1;");
    let plan = compile_with_sources(&m, &sources).unwrap();
    let scope = plan
        .scopes()
        .iter()
        .find(|s| s.block == "scope")
        .unwrap()
        .offset;
    let result = runner(&m, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    assert_eq!(result.frames[10].values[scope], 0.0);
    assert_eq!(result.frames[20].values[scope], 1.2);
    assert_eq!(result.frames[30].values[scope], 1.2);
    assert_eq!(result.frames[40].values[scope], 2.4);
}

#[test]
fn inactive_invalid_update_is_masked_and_active_failure_keeps_the_last_frame() {
    let (m, sources) = component_model("if t < 0.05\ny=q+1;\nelse\ny=sqrt(-1);\nend");
    let mut run = runner(&m, &sources);
    for _ in 0..19 {
        run.advance().unwrap();
    }
    let before = run.current_frame();
    let q = run.discrete_output().to_vec();
    assert!(run.advance().is_err());
    assert!(run.is_failed());
    assert_eq!(run.time(), before.time);
    assert_eq!(run.current_frame().values, before.values);
    assert_eq!(run.discrete_output(), q);
}

#[test]
fn new_rates_require_schema_five_and_cannot_silently_cross_delay_domains() {
    let mut m = model(
        json!([
            {"id":"source","kind":{"type":"step","time":0.05,"before":[0],"after":[1]}},
            {"id":"delay","kind":{"type":"unitDelay","initial":[0]}},
            {"id":"scope","kind":{"type":"scope"}}
        ]),
        json!([edge("source", "delay"), edge("delay", "scope")]),
        json!({"source":clock(0.02),"delay":clock(0.1)}),
    );
    assert_eq!(
        compile_with_sources(&m, &SourceBundle::new())
            .unwrap_err()
            .0
            .code,
        "rate_transition_required"
    );
    m.schema_version = 4;
    assert_eq!(
        compile_with_sources(&m, &SourceBundle::new())
            .unwrap_err()
            .0
            .code,
        "schema_version"
    );
    m.schema_version = 5;
    m.sample_times.insert(
        "source".into(),
        openmat_sim::model::SampleTime::Discrete { period: 0.015 },
    );
    assert_eq!(
        compile_with_sources(&m, &SourceBundle::new())
            .unwrap_err()
            .0
            .code,
        "sample_grid"
    );
}

#[test]
fn rate_propagation_and_simulation_are_independent_of_file_order() {
    let (mut m, sources) = component_model("y=q+1;");
    let baseline = runner(&m, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    let rows = |result: &openmat_sim::SimulationResult| {
        result
            .scopes
            .iter()
            .map(|scope| {
                (
                    scope.block.clone(),
                    result
                        .frames
                        .iter()
                        .map(|f| f.values[scope.offset])
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    m.blocks.reverse();
    m.connections.reverse();
    let permuted = runner(&m, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    assert_eq!(rows(&baseline), rows(&permuted));
    assert_eq!(
        baseline
            .frames
            .iter()
            .map(|f| &f.sample_hits)
            .collect::<Vec<_>>(),
        permuted
            .frames
            .iter()
            .map(|f| &f.sample_hits)
            .collect::<Vec<_>>()
    );
}

#[test]
fn integer_clock_counts_and_cancelled_instances_do_not_affect_a_new_run() {
    let (mut m, sources) = component_model("y=q+1;");
    m.settings.stop_time = 20.0;
    let mut cancelled = runner(&m, &sources);
    for _ in 0..13 {
        cancelled.advance().unwrap();
    }
    let before = cancelled.current_frame();
    let flag = std::sync::atomic::AtomicBool::new(true);
    assert!(cancelled.advance_with_cancel(&flag).is_err());
    assert_eq!(cancelled.current_frame().values, before.values);
    assert_eq!(cancelled.current_frame().sample_hits, before.sample_hits);
    assert_eq!(cancelled.time(), before.time);
    let result = runner(&m, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    assert_eq!(result.frames.len(), 2001);
    assert_eq!(
        result
            .frames
            .iter()
            .filter(|f| f.sample_hits.contains(&0))
            .count(),
        201
    );
    assert_eq!(
        result
            .frames
            .iter()
            .filter(|f| f.sample_hits.contains(&1))
            .count(),
        101
    );
    let offset = result
        .scopes
        .iter()
        .find(|s| s.block == "scope")
        .unwrap()
        .offset;
    assert_eq!(result.frames.last().unwrap().values[offset], 120.0);
}

#[test]
fn sampled_algebraic_cycles_remain_errors_and_mixed_states_need_an_explicit_clock() {
    let m = model(
        json!([
            {"id":"a","kind":{"type":"gain","gain":[1]}},
            {"id":"b","kind":{"type":"gain","gain":[2]}},
            {"id":"scope","kind":{"type":"scope"}}
        ]),
        json!([edge("a", "b"), edge("b", "a"), edge("b", "scope")]),
        json!({"a":clock(0.1),"b":clock(0.1)}),
    );
    assert_eq!(
        compile_with_sources(&m, &SourceBundle::new())
            .unwrap_err()
            .0
            .code,
        "algebraic_loop"
    );
    let (mut mixed, mut sources) = component_model("y=q+1;");
    mixed.components[0].continuous_states = 1;
    mixed.components[0].sample_time = Some(-1.0);
    mixed.components[0].derivatives =
        Some(serde_json::from_value(json!({"source":"dx.m","entry":"dx"})).unwrap());
    sources.insert(
        "dx.m".into(),
        "function y=dx(t,x,q,u,p)\ny=-x;\nend\n".into(),
    );
    sources.insert(
        "initialize.m".into(),
        "function y=initialize(p)\ny=[0;0];\nend\n".into(),
    );
    assert_eq!(
        compile_with_sources(&mixed, &sources).unwrap_err().0.code,
        "sample_time"
    );
}

#[test]
fn unsupported_times_fail_with_block_diagnostics() {
    let (mut m, sources) = component_model("y=q+1;");
    m.sample_times
        .insert("missing".into(), openmat_sim::model::SampleTime::Inherited);
    assert_eq!(
        compile_with_sources(&m, &sources)
            .unwrap_err()
            .0
            .block
            .as_deref(),
        Some("missing")
    );
    m.sample_times.remove("missing");
    m.settings.start_time = 0.01;
    assert_eq!(
        compile_with_sources(&m, &sources).unwrap_err().0.code,
        "sample_time"
    );
    m.settings.start_time = 0.0;
    m.settings.stop_time = 0.505;
    assert_eq!(
        compile_with_sources(&m, &sources).unwrap_err().0.code,
        "sample_grid"
    );
}

#[test]
fn two_instances_of_one_inherited_component_keep_distinct_clocks_and_state() {
    let (mut m, sources) = component_model("y=q+1;");
    m.components[0].sample_time = Some(-1.0);
    m.sample_times.insert(
        "component".into(),
        openmat_sim::model::SampleTime::Discrete { period: 0.1 },
    );
    m.sample_times.insert(
        "second".into(),
        openmat_sim::model::SampleTime::Discrete { period: 0.2 },
    );
    let mut second = m
        .blocks
        .iter()
        .find(|b| b.id == "component")
        .unwrap()
        .clone();
    second.id = "second".into();
    m.blocks.push(second);
    let mut scope = m.blocks.iter().find(|b| b.id == "scope").unwrap().clone();
    scope.id = "second_scope".into();
    m.blocks.push(scope);
    m.connections
        .push(serde_json::from_value(edge("second", "second_scope")).unwrap());
    let result = runner(&m, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    let first = result
        .scopes
        .iter()
        .find(|s| s.block == "scope")
        .unwrap()
        .offset;
    let second = result
        .scopes
        .iter()
        .find(|s| s.block == "second_scope")
        .unwrap()
        .offset;
    assert_eq!(result.frames[10].values[first], 1.1);
    assert_eq!(result.frames[10].values[second], 0.0);
    assert_eq!(result.frames[20].values[first], 2.2);
    assert_eq!(result.frames[20].values[second], 1.2);
    m.blocks.reverse();
    let reordered = runner(&m, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    for (i, before) in result.scopes.iter().enumerate() {
        let after = reordered
            .scopes
            .iter()
            .find(|s| s.block == before.block)
            .unwrap()
            .offset;
        for (a, b) in result.frames.iter().zip(&reordered.frames) {
            assert_eq!(a.values[i], b.values[after]);
        }
    }
}

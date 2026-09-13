use openmat_sim::{
    CollectionLimits, Runner, compile,
    model::{BlockKind, Model},
    numeric::ReferenceKernel,
};
use serde_json::{Value, json};

fn run(m: &Model) -> openmat_sim::SimulationResult {
    let p = compile(m).unwrap();
    let kernel = ReferenceKernel::new(p.program().clone());
    let update = p.update_program().cloned().map(ReferenceKernel::new);
    Runner::new_with_update(p, kernel, update)
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap()
}
fn wire(a: &str, p: &str, b: &str, q: &str) -> Value {
    json!({"from":{"block":a,"port":p},"to":{"block":b,"port":q}})
}
fn fixture() -> Model {
    serde_json::from_value(json!({"schemaVersion":7,"name":"experiment","settings":{"startTime":0,"stopTime":1,"maxStep":0.1},"blocks":[
        {"id":"input","kind":{"type":"inport","port":1,"data":{"times":[0,0.137,0.413,1],"values":[[0],[1],[0],[0]]}}},
        {"id":"plant","kind":{"type":"subsystem","inputs":1,"outputs":1}},
        {"id":"u","parent":"plant","kind":{"type":"inport","port":1}},
        {"id":"integral","parent":"plant","kind":{"type":"integrator","initial":[0]}},
        {"id":"y","parent":"plant","kind":{"type":"outport","port":1}},
        {"id":"result","kind":{"type":"outport","port":1}}
    ],"connections":[wire("input","out","plant","in1"),wire("u","out","integral","in"),wire("integral","out","y","in"),wire("plant","out1","result","in")]})).unwrap()
}

#[test]
fn nested_boundary_aliases_preserve_numerics_and_irregular_input_knots() {
    let m = fixture();
    let result = run(&m);
    assert!((result.frames.last().unwrap().values[0] - 0.2065).abs() < 1e-13);
    for time in [0.1, 0.137, 0.2, 0.4, 0.413, 0.5, 1.0] {
        assert!(result.frames.iter().any(|f| (f.time - time).abs() < 1e-14));
    }
    let mut flat = m.clone();
    flat.blocks
        .retain(|b| b.id != "plant" && b.id != "u" && b.id != "y");
    for b in &mut flat.blocks {
        b.parent = None;
    }
    flat.connections = serde_json::from_value(json!([
        wire("input", "out", "integral", "in"),
        wire("integral", "out", "result", "in")
    ]))
    .unwrap();
    assert_eq!(json!(run(&flat).frames), json!(result.frames));
    let mut nested = m.clone();
    nested.blocks.extend(
        serde_json::from_value::<Vec<openmat_sim::model::Block>>(json!([
            {"id":"inside","parent":"plant","kind":{"type":"subsystem","inputs":1,"outputs":1}},
            {"id":"u2","parent":"inside","kind":{"type":"inport","port":1}},
            {"id":"y2","parent":"inside","kind":{"type":"outport","port":1}}
        ]))
        .unwrap(),
    );
    nested
        .blocks
        .iter_mut()
        .find(|b| b.id == "integral")
        .unwrap()
        .parent = Some("inside".into());
    nested.connections = serde_json::from_value(json!([
        wire("input", "out", "plant", "in1"),
        wire("u", "out", "inside", "in1"),
        wire("u2", "out", "integral", "in"),
        wire("integral", "out", "y2", "in"),
        wire("inside", "out1", "y", "in"),
        wire("plant", "out1", "result", "in")
    ]))
    .unwrap();
    assert_eq!(json!(run(&nested).frames), json!(result.frames));
}

#[test]
fn invalid_hierarchy_and_data_are_diagnosed_before_execution() {
    let base = fixture();
    let mut bad = base.clone();
    bad.schema_version = 6;
    assert_eq!(compile(&bad).unwrap_err().0.code, "schema_version");
    let mut bad = base.clone();
    bad.blocks[1].parent = Some("plant".into());
    assert_eq!(compile(&bad).unwrap_err().0.code, "hierarchy");
    let mut bad = base.clone();
    bad.connections[0].to.block = "integral".into();
    bad.connections[0].to.port = "in".into();
    assert_eq!(compile(&bad).unwrap_err().0.code, "hierarchy");
    let mut bad = base.clone();
    if let BlockKind::Inport { data, .. } = &mut bad.blocks[0].kind {
        *data = None;
    }
    assert_eq!(compile(&bad).unwrap_err().0.code, "input_data");
    let mut bad = base.clone();
    if let BlockKind::Inport {
        data: Some(data), ..
    } = &mut bad.blocks[0].kind
    {
        data.times[1] = 0.0;
    }
    assert_eq!(compile(&bad).unwrap_err().0.code, "input_data");
    let mut bad = base;
    bad.sample_times
        .insert("plant".into(), openmat_sim::model::SampleTime::Continuous);
    assert_eq!(compile(&bad).unwrap_err().0.code, "hierarchy");
}

#[test]
fn standard_routing_and_product_preserve_vector_channels() {
    let m:Model=serde_json::from_value(json!({"schemaVersion":7,"name":"routing","settings":{"startTime":0,"stopTime":1,"maxStep":0.1},"blocks":[
        {"id":"a","kind":{"type":"constant","value":[2,4]}},
        {"id":"b","kind":{"type":"constant","value":[2]}},
        {"id":"mul","kind":{"type":"standard","operation":{"type":"product","operations":"*/"}}},
        {"id":"mux","kind":{"type":"standard","operation":{"type":"mux","inputs":2}}},
        {"id":"split","kind":{"type":"standard","operation":{"type":"demux","widths":[2,1]}}},
        {"id":"out1","kind":{"type":"outport","port":1}},
        {"id":"out2","kind":{"type":"outport","port":2}}
    ],"connections":[wire("a","out","mul","in0"),wire("b","out","mul","in1"),wire("mul","out","mux","in0"),wire("b","out","mux","in1"),wire("mux","out","split","in"),wire("split","out0","out1","in"),wire("split","out1","out2","in")]})).unwrap();
    assert_eq!(run(&m).frames[0].values, vec![1., 2., 2.]);
}

#[test]
fn transfer_function_and_state_space_match_analytic_closed_loop() {
    for operation in [
        json!({"type":"transferFcn","numerator":[1],"denominator":[1,1]}),
        json!({"type":"stateSpace","a":[[-1]],"b":[[1]],"c":[[1]],"d":[[0]],"initial":[0]}),
    ] {
        let m:Model=serde_json::from_value(json!({"schemaVersion":7,"name":"linear","settings":{"startTime":0,"stopTime":1,"maxStep":0.01},"blocks":[
            {"id":"one","kind":{"type":"constant","value":[1]}},
            {"id":"error","kind":{"type":"sum","signs":[1,-1]}},
            {"id":"plant","kind":{"type":"standard","operation":operation}},
            {"id":"out","kind":{"type":"outport","port":1}}
        ],"connections":[wire("one","out","error","in0"),wire("plant","out","error","in1"),wire("error","out","plant","in"),wire("plant","out","out","in")]})).unwrap();
        assert!(
            (run(&m).frames.last().unwrap().values[0] - 0.5 * (1.0 - (-2.0f64).exp())).abs() < 1e-9
        );
    }
}

#[test]
fn input_knots_do_not_create_discrete_clock_hits() {
    let mut m = fixture();
    m.blocks
        .iter_mut()
        .find(|b| b.id == "integral")
        .unwrap()
        .kind = BlockKind::UnitDelay { initial: vec![0.] };
    m.sample_times.insert(
        "integral".into(),
        openmat_sim::model::SampleTime::Discrete { period: 0.2 },
    );
    let r = run(&m);
    assert_eq!(r.frames.iter().filter(|f| f.sample_hit).count(), 6);
    assert!(
        r.frames
            .iter()
            .filter(|f| f.time.to_bits() == 0.137_f64.to_bits()
                || f.time.to_bits() == 0.413_f64.to_bits())
            .all(|f| !f.sample_hit)
    );
}

#[test]
fn virtual_pass_through_loops_and_stale_sample_times_are_rejected() {
    let mut m = fixture();
    m.sample_times
        .insert("missing".into(), openmat_sim::model::SampleTime::Continuous);
    assert_eq!(compile(&m).unwrap_err().0.code, "sample_time");
    m.sample_times.clear();
    m.blocks.retain(|b| b.id != "integral");
    m.connections = serde_json::from_value(json!([
        wire("plant", "out1", "plant", "in1"),
        wire("u", "out", "y", "in"),
        wire("plant", "out1", "result", "in")
    ]))
    .unwrap();
    assert_eq!(compile(&m).unwrap_err().0.code, "hierarchy");
}

#[test]
fn invalid_standard_dimensions_and_overflowing_realizations_are_rejected() {
    use openmat_sim::authoring::StandardOp;
    for operation in [
        json!({"type":"transferFcn","numerator":[1,2],"denominator":[1]}),
        json!({"type":"transferFcn","numerator":[1e308,0],"denominator":[1,1e308]}),
        json!({"type":"stateSpace","a":[[-1,0],[0,-2]],"b":[[1]],"c":[[1]],"d":[[0]],"initial":[0]}),
        json!({"type":"demux","widths":[4096,1]}),
    ] {
        let op: StandardOp = serde_json::from_value(operation).unwrap();
        assert_eq!(op.validate().unwrap_err().0.code, "standard_parameter");
    }
}

#[test]
fn embedded_experiment_matches_the_analytic_pi_closed_loop() {
    let document: Value = serde_json::from_str(include_str!(
        "../../../examples/experiment-control.omsim.json"
    ))
    .unwrap();
    let m: Model = serde_json::from_value(document["model"].clone()).unwrap();
    let result = run(&m);
    let output = result
        .scopes
        .iter()
        .find(|s| s.block == "output")
        .unwrap()
        .offset;
    let a = (-3.0_f64).midpoint(5.0_f64.sqrt());
    let b = (-3.0_f64).midpoint(-5.0_f64.sqrt());
    let c = (2.0 + b) / (a - b);
    for f in &result.frames {
        let expected = 1.0 + c * (a * f.time).exp() + (-1.0 - c) * (b * f.time).exp();
        assert!(
            (f.values[output] - expected).abs() < 2e-9,
            "t={} y={} expected={expected}",
            f.time,
            f.values[output]
        );
    }
}

use std::cell::Cell;
use std::rc::Rc;

use openmat_sim::model::Model;
use openmat_sim::numeric::{EvalError, Kernel, Program, ReferenceKernel};
use openmat_sim::{CollectionLimits, Runner, SourceBundle, compile_with_sources};
use serde_json::{Value, json};

fn fixture() -> (Value, SourceBundle) {
    (
        json!({"schemaVersion":3,"name":"custom delay","settings":{"startTime":0,"stopTime":0.3,"maxStep":0.03,"sampleTime":0.1},
        "components":[{"id":"delay","name":"Delay","category":"Discrete","icon":"delay",
            "inputs":[{"name":"in","width":1}],"outputs":[{"name":"out","width":1}],
            "parameters":[{"name":"initial","value":[0]}],"continuousStates":0,"discreteStates":1,"sampleTime":0.1,
            "initialize":{"source":"init.m","entry":"init"},"outputsFunction":{"source":"output.m","entry":"output"},
            "update":{"source":"update.m","entry":"update"}}],
        "blocks":[{"id":"c","kind":{"type":"constant","value":[3]}},
            {"id":"d","kind":{"type":"component","component":"delay","parameters":{}}},
            {"id":"s","kind":{"type":"scope"}}],
        "connections":[{"from":{"block":"c","port":"out"},"to":{"block":"d","port":"in"}},
            {"from":{"block":"d","port":"out"},"to":{"block":"s","port":"in"}}]}),
        [
            ("init.m".into(), "function z = init(p)\nz = p;\nend".into()),
            (
                "output.m".into(),
                "function y = output(t, x, q, u, p)\ny = q;\nend".into(),
            ),
            (
                "update.m".into(),
                "function z = update(t, x, q, u, p)\nz = u;\nend".into(),
            ),
        ]
        .into(),
    )
}

fn runner(model: Value, sources: &SourceBundle) -> Runner<ReferenceKernel> {
    let model: Model = serde_json::from_value(model).unwrap();
    let plan = compile_with_sources(&model, sources).unwrap();
    let flow = ReferenceKernel::new(plan.program().clone());
    let update = plan.update_program().cloned().map(ReferenceKernel::new);
    Runner::new_with_update(plan, flow, update).unwrap()
}

#[test]
fn custom_delay_matches_builtin_and_commits_on_exact_ticks() {
    let (model, sources) = fixture();
    let a = runner(model.clone(), &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    let mut builtin = model;
    builtin["blocks"][1]["kind"] = json!({"type":"unitDelay","initial":[0]});
    let b = runner(builtin, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    assert_eq!(
        serde_json::to_value(&a.frames).unwrap(),
        serde_json::to_value(&b.frames).unwrap()
    );
    for frame in a.frames {
        assert_eq!(frame.values, vec![if frame.time < 0.1 { 0.0 } else { 3.0 }]);
    }
}

#[test]
fn instances_have_private_initial_states_and_parameters() {
    let (mut model, sources) = fixture();
    model["blocks"].as_array_mut().unwrap().extend([
        json!({"id":"e","kind":{"type":"component","component":"delay","parameters":{"initial":[7]}}}),
        json!({"id":"z","kind":{"type":"scope"}})]);
    model["connections"].as_array_mut().unwrap().extend([
        json!({"from":{"block":"c","port":"out"},"to":{"block":"e","port":"in"}}),
        json!({"from":{"block":"e","port":"out"},"to":{"block":"z","port":"in"}}),
    ]);
    let run = runner(model, &sources);
    assert_eq!(run.discrete_output(), &[0.0, 7.0]);
    assert_eq!(run.current_frame().values, vec![0.0, 7.0]);
}

#[test]
fn output_dependencies_allow_state_feedback_and_reject_algebraic_cycles() {
    let (mut model, mut sources) = fixture();
    model["connections"][0]["from"]["block"] = json!("d");
    runner(model.clone(), &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    sources.insert(
        "output.m".into(),
        "function y = output(t, x, q, u, p)\ny = q + u;\nend".into(),
    );
    let error =
        compile_with_sources(&serde_json::from_value(model).unwrap(), &sources).unwrap_err();
    assert_eq!(error.0.code, "algebraic_loop");
    assert_eq!(error.0.block.as_deref(), Some("d"));
}

#[test]
fn dependency_analysis_is_per_output_port() {
    let (mut model, mut sources) = fixture();
    model["components"][0]["outputs"] =
        json!([{"name":"held","width":1},{"name":"direct","width":1}]);
    model["connections"][0]["from"] = json!({"block":"d","port":"held"});
    model["connections"][1]["from"]["port"] = json!("direct");
    sources.insert(
        "output.m".into(),
        "function y = output(t, x, q, u, p)\ny = [q; u];\nend".into(),
    );
    runner(model, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
}

#[test]
fn matrix_multiply_column_major_indexing_and_static_indexed_loops() {
    let (model, mut sources) = fixture();
    sources.insert("update.m".into(), "function z = update(t, x, q, u, p)\nA = [1, 2; 3, 4];\nb = [u; 1];\nv = A * b;\nw = zeros(2, 1);\nfor i = 1:2\nw(i) = v(i) + A(i, 2);\nend\nz = w(1) + w(2);\nend".into());
    let frames = runner(model, &sources)
        .collect(CollectionLimits::default())
        .unwrap()
        .frames;
    assert_eq!(frames.last().unwrap().values, vec![24.0]);
}

#[test]
fn discrete_if_elseif_merges_locals_and_masks_inactive_nonfinite_results() {
    let (model, mut sources) = fixture();
    sources.insert("update.m".into(), "function z = update(t, x, q, u, p)\nif u > 4\na = sqrt(-1);\nelseif u >= 3\na = 8;\nelse\na = 9;\nend\nz = a;\nend".into());
    let frames = runner(model, &sources)
        .collect(CollectionLimits::default())
        .unwrap()
        .frames;
    assert_eq!(frames.last().unwrap().values, vec![8.0]);
}

#[test]
fn empty_loop_and_literal_shapes_follow_matlab_without_inventing_an_iteration() {
    let (model, mut sources) = fixture();
    sources.insert("update.m".into(), "function z = update(t,x,q,u,p)\ni=9;\nfor i=2:1\nu=100;\nend\nA=[];\nB=[zeros(0,3),zeros(0,2)];\nz=u+size(i,1)+size(i,2)+size(A,1)+size(A,2)+size(B,2);\nend".into());
    let result = runner(model, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    assert_eq!(result.frames.last().unwrap().values, vec![8.0]);
}

#[test]
fn continuous_mass_spring_damper_uses_matrix_rhs() {
    let (mut model, mut sources) = fixture();
    let d = &mut model["components"][0];
    d["continuousStates"] = json!(2);
    d["discreteStates"] = json!(0);
    d.as_object_mut().unwrap().remove("sampleTime");
    d.as_object_mut().unwrap().remove("update");
    d["derivatives"] = json!({"source":"rhs.m","entry":"rhs"});
    d["parameters"] = json!([{"name":"coefficients","value":[1,2,1]}]);
    sources.insert(
        "init.m".into(),
        "function z = init(p)\nz = [1; 0];\nend".into(),
    );
    sources.insert(
        "output.m".into(),
        "function y = output(t, x, q, u, p)\ny = x(1);\nend".into(),
    );
    sources.insert("rhs.m".into(), "function dx = rhs(t, x, q, u, p)\nA = [0, 1; -p(3)/p(1), -p(2)/p(1)];\nB = [0; 1/p(1)];\ndx = A*x + B*u;\nend".into());
    model["blocks"][0]["kind"]["value"] = json!([0]);
    let result = runner(model, &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    for frame in result.frames {
        assert!((frame.values[0] - (1.0 + frame.time) * (-frame.time).exp()).abs() < 1e-7);
    }
}

#[test]
fn rejects_dynamic_shapes_unbounded_work_and_continuous_branching() {
    for body in [
        "z = eval('u');",
        "z = zeros(u, 1);",
        "for i=1:100000\nz=u;\nend",
        "z = u(t);",
        "if u > 0\na=1;\nend\nz=a;",
        "if u>0\nz=[1;2];\nelse\nz=1;\nend",
        "z = u + missing;",
    ] {
        let (model, mut sources) = fixture();
        sources.insert(
            "update.m".into(),
            format!("function z = update(t, x, q, u, p)\n{body}\nend"),
        );
        let error =
            compile_with_sources(&serde_json::from_value(model).unwrap(), &sources).unwrap_err();
        assert_eq!(
            error.0.source_path.as_deref(),
            Some("update.m"),
            "{body}: {error}"
        );
        assert!(error.0.line.is_some());
    }
    let (model, mut sources) = fixture();
    sources.insert(
        "output.m".into(),
        "function y = output(t, x, q, u, p)\nif t>0\ny=q;\nelse\ny=u;\nend\nend".into(),
    );
    assert_eq!(
        compile_with_sources(&serde_json::from_value(model).unwrap(), &sources)
            .unwrap_err()
            .0
            .code,
        "function_subset"
    );
    for expression in ["u > 0", "~u"] {
        let (model, mut sources) = fixture();
        sources.insert(
            "output.m".into(),
            format!("function y = output(t,x,q,u,p)\ny = {expression};\nend"),
        );
        assert_eq!(
            compile_with_sources(&serde_json::from_value(model).unwrap(), &sources)
                .unwrap_err()
                .0
                .code,
            "function_condition"
        );
    }
}

struct Counting {
    inner: ReferenceKernel,
    count: Rc<Cell<usize>>,
}
impl Kernel for Counting {
    fn program(&self) -> &Program {
        self.inner.program()
    }
    fn evaluate(&mut self, inputs: &[f64], outputs: &mut [f64]) -> Result<(), EvalError> {
        self.count.set(self.count.get() + 1);
        self.inner.evaluate(inputs, outputs)
    }
}

#[test]
fn update_kernel_runs_once_per_tick_and_failure_keeps_last_accepted_frame() {
    let (mut model, mut sources) = fixture();
    model["blocks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"integrator","kind":{"type":"integrator","initial":[0]}}));
    model["connections"]
        .as_array_mut()
        .unwrap()
        .push(json!({"from":{"block":"c","port":"out"},"to":{"block":"integrator","port":"in"}}));
    model["settings"]["stopTime"] = json!(0.4);
    sources.insert(
        "update.m".into(),
        "function z = update(t, x, q, u, p)\nif t >= 0.2\nz = sqrt(-1);\nelse\nz = u;\nend\nend"
            .into(),
    );
    let plan = compile_with_sources(&serde_json::from_value(model).unwrap(), &sources).unwrap();
    let count = Rc::new(Cell::new(0));
    let flow = Counting {
        inner: ReferenceKernel::new(plan.program().clone()),
        count: Rc::new(Cell::new(0)),
    };
    let update = Counting {
        inner: ReferenceKernel::new(plan.update_program().unwrap().clone()),
        count: count.clone(),
    };
    let mut run = Runner::new_with_update(plan, flow, Some(update)).unwrap();
    assert_eq!(count.get(), 1);
    while run.time() < 0.19 {
        run.advance().unwrap();
    }
    let before = run.current_frame();
    assert_eq!(count.get(), 2);
    let error = run.advance().unwrap_err();
    assert_eq!(error.port.as_deref(), Some("update"));
    assert_eq!(count.get(), 3);
    assert_eq!(run.time().to_bits(), before.time.to_bits());
    assert_eq!(run.current_frame().values, before.values);
    assert!(run.is_failed());
}

#[test]
fn validates_versions_parameters_states_and_missing_update_kernel() {
    let (model, sources) = fixture();
    for mutate in ["version", "sample", "parameter", "state", "duplicate"] {
        let mut m = model.clone();
        match mutate {
            "version" => m["schemaVersion"] = json!(2),
            "sample" => m["components"][0]["sampleTime"] = json!(0.2),
            "parameter" => m["blocks"][1]["kind"]["parameters"] = json!({"unknown":[1]}),
            "state" => m["components"][0]["discreteStates"] = json!(2),
            _ => {
                let d = m["components"][0].clone();
                m["components"].as_array_mut().unwrap().push(d);
            }
        }
        assert!(
            compile_with_sources(&serde_json::from_value(m).unwrap(), &sources).is_err(),
            "{mutate}"
        );
    }
    let plan = compile_with_sources(&serde_json::from_value(model).unwrap(), &sources).unwrap();
    let kernel = ReferenceKernel::new(plan.program().clone());
    assert!(Runner::new(plan, kernel).is_err());
}

#[test]
#[ignore = "requires OpenMat-authored R2022b component_oracle output"]
fn matlab_r2022b_matrix_loop_branch_and_mixed_trajectory() {
    let directory = std::env::var_os("OPENMAT_COMPONENT_ORACLE_DIR")
        .expect("run simulation/tools/component_oracle.m in R2022b");
    let oracle: Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::Path::new(&directory).join("components.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(oracle["release"], "2022b");
    for (input, expected) in oracle["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["values"].as_array().unwrap())
    {
        let (mut model, mut sources) = fixture();
        model["blocks"][0]["kind"]["value"] = json!([input]);
        model["components"][0]["update"]["entry"] = json!("component_subset");
        sources.insert(
            "update.m".into(),
            include_str!("../../../tools/component_subset.m").into(),
        );
        let result = runner(model, &sources)
            .collect(CollectionLimits::default())
            .unwrap();
        assert_eq!(
            result.frames.last().unwrap().values[0].to_bits(),
            expected.as_f64().unwrap().to_bits()
        );
    }
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let model: Model =
        serde_json::from_str(include_str!("../../../examples/pi-control.omsim.json")).unwrap();
    let sources: SourceBundle = openmat_sim::component::source_references(&model)
        .into_iter()
        .map(|p| (p.to_owned(), std::fs::read_to_string(base.join(p)).unwrap()))
        .collect();
    let result = runner(serde_json::to_value(model).unwrap(), &sources)
        .collect(CollectionLimits::default())
        .unwrap();
    let ticks: Vec<_> = result.frames.iter().filter(|f| f.sample_hit).collect();
    assert_eq!(ticks.len(), oracle["trajectory"].as_array().unwrap().len());
    for (frame, row) in ticks.iter().zip(oracle["trajectory"].as_array().unwrap()) {
        assert!((frame.time - row[0].as_f64().unwrap()).abs() < 1e-12);
        for (actual, expected) in frame
            .values
            .iter()
            .zip(row.as_array().unwrap().iter().skip(1))
        {
            assert!(
                (actual - expected.as_f64().unwrap()).abs() < 2e-7,
                "{}: {actual} vs {expected}",
                frame.time
            );
        }
    }
}

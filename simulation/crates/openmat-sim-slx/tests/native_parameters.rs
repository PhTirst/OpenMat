//! Independently authored editor-parameter tests; vendor model files are not fixtures.
use openmat_sim::model::{BlockKind, Model};
use openmat_sim_slx::{AuthoringParameters, Parameters, resolve_parameters};
use serde_json::{Value, json};

fn model() -> Model {
    serde_json::from_value(json!({"schemaVersion":1,"name":"parameters","settings":{"startTime":0,"stopTime":1,"maxStep":0.1},
        "blocks":[{"id":"source","kind":{"type":"constant","value":[999]}},
            {"id":"gain","kind":{"type":"gain","gain":[999]}}, {"id":"scope","kind":{"type":"scope"}}],
        "connections":[{"from":{"block":"source","port":"out"},"to":{"block":"gain","port":"in"}},
            {"from":{"block":"gain","port":"out"},"to":{"block":"scope","port":"in"}}]})).unwrap()
}
fn bindings(source: &str, entries: Value) -> AuthoringParameters {
    AuthoringParameters {
        source: source.into(),
        bindings: serde_json::from_value(entries).unwrap(),
    }
}

#[test]
fn expressions_override_cached_values_and_change_with_model_parameters() {
    let original = model();
    let parameters = bindings(
        "K = 2;",
        json!({"source":{"Value":"3"},"gain":{"Gain":"K"}}),
    );
    let result = resolve_parameters(&original, &parameters).unwrap();
    assert!(matches!(&result.model.blocks[1].kind, BlockKind::Gain { gain } if gain == &[2.0]));
    let plan = openmat_sim::compile(&result.model).unwrap();
    let kernel = openmat_sim::numeric::ReferenceKernel::new(plan.program().clone());
    let runner = openmat_sim::Runner::new(plan, kernel).unwrap();
    assert_eq!(runner.current_frame().values, vec![6.0]);
    let updated = resolve_parameters(
        &result.model,
        &bindings("K = 4;", parameters_to_json(&parameters)),
    )
    .unwrap();
    assert!(matches!(&updated.model.blocks[1].kind, BlockKind::Gain { gain } if gain == &[4.0]));
    assert!(matches!(&original.blocks[1].kind, BlockKind::Gain { gain } if gain == &[999.0]));
}
fn parameters_to_json(parameters: &AuthoringParameters) -> Value {
    json!(parameters.bindings)
}

#[test]
fn invalid_parameters_never_fall_back_to_cached_values() {
    for text in ["missingK", "1/0", "eval('2')", "[1 2;3 4]", "[]"] {
        let issue =
            resolve_parameters(&model(), &bindings("", json!({"gain":{"Gain":text}}))).unwrap_err();
        assert_eq!(issue.block.as_deref(), Some("gain"));
        assert_eq!(issue.parameter.as_deref(), Some("Gain"));
    }
    assert!(resolve_parameters(&model(), &bindings("disp(1)", json!({}))).is_err());
    assert!(resolve_parameters(&model(), &bindings("", json!({"missing":{"Gain":"2"}}))).is_err());
    assert!(resolve_parameters(&model(), &bindings("", json!({"gain":{"Unknown":"2"}}))).is_err());
}

#[test]
fn unsupported_options_are_rejected_and_sample_time_is_resolved() {
    let issue = resolve_parameters(
        &model(),
        &bindings("", json!({"gain":{"Multiplication":"Matrix(K*u)"}})),
    )
    .unwrap_err();
    assert_eq!(issue.parameter.as_deref(), Some("Multiplication"));
    let result = resolve_parameters(
        &model(),
        &bindings("Ts = 0.1;", json!({"gain":{"SampleTime":"[Ts 0]"}})),
    )
    .unwrap();
    assert_eq!(result.model.schema_version, 5);
    assert_eq!(
        result.model.sample_times["gain"],
        openmat_sim::model::SampleTime::Discrete { period: 0.1 }
    );
    assert!(
        resolve_parameters(&model(), &bindings("", json!({"gain":{"SampleTime":"-2"}}))).is_err()
    );
    assert!(
        resolve_parameters(
            &model(),
            &bindings("", json!({"gain":{"SampleTime":"[0.1 0.01]"}}))
        )
        .is_err()
    );
    assert_eq!(
        resolve_parameters(
            &model(),
            &bindings("", json!({"gain":{"SampleTime":"[inf 0]"}}))
        )
        .unwrap()
        .model
        .sample_times["gain"],
        openmat_sim::model::SampleTime::Constant
    );
}

#[test]
fn conditional_output_and_control_parameters_have_independent_owners() {
    let raw: Value =
        serde_json::from_str(include_str!("../../../examples/enabled-control.omsim.json")).unwrap();
    let model: Model = serde_json::from_value(raw["model"].clone()).unwrap();
    let parent = model
        .blocks
        .iter()
        .find(|b| {
            matches!(
                &b.kind,
                BlockKind::Subsystem {
                    execution: Some(_),
                    ..
                }
            )
        })
        .unwrap();
    let output = model
        .blocks
        .iter()
        .find(|b| {
            b.parent.as_ref() == Some(&parent.id)
                && matches!(b.kind, BlockKind::Outport { port: 1 })
        })
        .unwrap();
    let result = resolve_parameters(&model, &bindings("initial = 7;", json!({parent.id.clone():{"StatesWhenEnabling":"reset"},output.id.clone():{"InitialOutput":"initial","OutputWhenDisabled":"held"}}))).unwrap();
    let block = result
        .model
        .blocks
        .iter()
        .find(|b| b.id == parent.id)
        .unwrap();
    let value = serde_json::to_value(block).unwrap();
    assert_eq!(value["kind"]["execution"]["statesWhenEnabling"], "reset");
    assert_eq!(
        value["kind"]["execution"]["outputs"][0]["initial"],
        json!([7.0])
    );
    assert_eq!(
        value["kind"]["execution"]["outputs"][0]["whenDisabled"],
        "held"
    );
}

#[test]
fn dimensions_are_resolved_together_and_matrices_keep_column_order() {
    let mut model = model();
    model.schema_version = 7;
    model.blocks[1].kind = serde_json::from_value(json!({"type":"standard","operation":{"type":"stateSpace","a":[[-1]],"b":[[1]],"c":[[1]],"d":[[0]],"initial":[0]}})).unwrap();
    let parameters = bindings(
        "K = 2;",
        json!({"gain":{"A":"[-1 2;3 -4]","B":"[1;2]","C":"[1 0]","D":"0","InitialCondition":"K"}}),
    );
    let result = resolve_parameters(&model, &parameters).unwrap();
    let value = serde_json::to_value(result.model.blocks[1].kind.clone()).unwrap();
    assert_eq!(value["operation"]["a"], json!([[-1.0, 2.0], [3.0, -4.0]]));
    assert_eq!(value["operation"]["initial"], json!([2.0, 2.0]));
    assert!(resolve_parameters(&model, &bindings("", json!({"gain":{"A":"eye(2)"}}))).is_err());
}

#[test]
fn bounded_binding_count_and_text() {
    let mut p = bindings("", json!({"gain":{"Gain":"2"}}));
    p.bindings
        .get_mut("gain")
        .unwrap()
        .insert("Gain".into(), "1".repeat(65536));
    assert!(resolve_parameters(&model(), &p).is_err());
}

#[test]
#[ignore = "requires a separately generated, independently authored R2022b probe"]
fn r2022b_block_editor_parameter_oracle() {
    let path = std::env::var("OPENMAT_BLOCK_EDITOR_ORACLE")
        .expect("set OPENMAT_BLOCK_EDITOR_ORACLE to block-editor.json");
    let raw: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(raw["release"], "2022b");
    let parameters = Parameters::from_script(raw["source"].as_str().unwrap()).unwrap();
    assert_eq!(raw["cases"].as_array().unwrap().len(), 8);
    for case in raw["cases"].as_array().unwrap() {
        let actual = parameters
            .evaluate(case["expression"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            actual.rows,
            usize::try_from(case["rows"].as_u64().unwrap()).unwrap()
        );
        assert_eq!(
            actual.columns,
            usize::try_from(case["columns"].as_u64().unwrap()).unwrap()
        );
        let expected = case["values"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![case["values"].clone()]);
        for (a, b) in actual.values.iter().zip(expected) {
            assert!((a - b.as_f64().unwrap()).abs() < 1e-12);
        }
    }
    let catalog: Value = serde_json::from_str(openmat_sim_slx::BLOCK_CATALOG).unwrap();
    for block in catalog["blocks"].as_array().unwrap() {
        for field in block["parameters"].as_array().unwrap() {
            if let Some(default) =
                raw["defaults"][block["slx"].as_str().unwrap()].get(field["name"].as_str().unwrap())
            {
                assert_eq!(
                    &field["defaultValue"], default,
                    "{}.{}",
                    block["slx"], field["name"]
                );
            }
        }
    }
}

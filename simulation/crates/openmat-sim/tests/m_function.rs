use openmat_sim::model::{BlockKind, Model};
use openmat_sim::numeric::{Kernel, ReferenceKernel};
use openmat_sim::{CollectionLimits, Runner, SourceBundle, compile_with_sources};

fn model() -> Model {
    serde_json::from_str(include_str!("../../../examples/pendulum.omsim.json")).unwrap()
}
fn sources(text: &str) -> SourceBundle {
    [("pendulum.m".into(), text.into())].into()
}

#[test]
fn nonlinear_column_function_uses_shared_ir() {
    let model = model();
    let plan = compile_with_sources(
        &model,
        &sources(include_str!("../../../examples/pendulum.m")),
    )
    .unwrap();
    let mut kernel = ReferenceKernel::new(plan.program().clone());
    let mut output = vec![0.; plan.program().output_count()];
    kernel.evaluate(&[0., 1.2, 0.3], &mut output).unwrap();
    assert_eq!(output[0].to_bits(), 0.3_f64.to_bits());
    assert!((output[1] - (-9.81 * 1.2_f64.sin() - 0.06)).abs() < 1e-14);
    let result = Runner::new(plan, kernel)
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
    assert_eq!(
        result.frames.last().unwrap().time.to_bits(),
        10.0_f64.to_bits()
    );
    assert!(result.frames.last().unwrap().values[0].abs() < 1.2);
}

#[test]
fn unsupported_dynamic_and_stateful_code_is_source_located() {
    for body in [
        "dx = eval('x');",
        "dx = evalin('base', 'x');",
        "assignin('base', 'a', x);",
        "persistent q;",
        "global q;",
        "if x(1) > 0\ndx = x;\nend",
        "dx = x(0);",
        "dx = x(p(1));",
        "dx = [x(1), x(2)];",
        "dx = unknown(x);",
        "dx(1) = x(1);",
    ] {
        let text = format!("function dx = pendulum(x, p)\n{body}\nend");
        let error = compile_with_sources(&model(), &sources(&text)).unwrap_err();
        assert_eq!(error.0.block.as_deref(), Some("rhs"), "{body}");
        assert_eq!(error.0.source_path.as_deref(), Some("pendulum.m"), "{body}");
        assert_eq!(error.0.line, Some(2), "{body}: {error}");
        assert!(error.0.column.unwrap() >= 1);
    }
}

#[test]
fn validates_source_snapshot_signature_version_and_width() {
    let mut model = model();
    assert_eq!(
        compile_with_sources(&model, &SourceBundle::new())
            .unwrap_err()
            .0
            .code,
        "source_missing"
    );
    let valid = sources(include_str!("../../../examples/pendulum.m"));
    model.schema_version = 1;
    assert_eq!(
        compile_with_sources(&model, &valid).unwrap_err().0.code,
        "schema_version"
    );
    model.schema_version = 2;
    if let BlockKind::MFunction { output_width, .. } = &mut model.blocks[0].kind {
        *output_width = 1;
    }
    assert_eq!(
        compile_with_sources(&model, &valid).unwrap_err().0.code,
        "signal_width"
    );
    let invalid: SourceBundle = [("../outside.m".into(), String::new())].into();
    assert_eq!(
        compile_with_sources(&model, &invalid).unwrap_err().0.code,
        "source_bundle"
    );
}

#[test]
fn local_shadowing_resolves_indexing_before_intrinsic_calls() {
    let text = "function dx = pendulum(x, p)\nsin = x;\ndx = [sin(2); sin(1)];\nend";
    let plan = compile_with_sources(&model(), &sources(text)).unwrap();
    let mut kernel = ReferenceKernel::new(plan.program().clone());
    let mut output = vec![0.; plan.program().output_count()];
    kernel.evaluate(&[0., 3., 5.], &mut output).unwrap();
    assert_eq!(&output[..2], &[5., 3.]);
}

#[test]
fn hostile_expression_depth_is_rejected_before_recursive_parsing() {
    for body in [
        format!("dx = {}x{};", "(".repeat(2000), ")".repeat(2000)),
        format!("dx = {}x;", "-".repeat(2000)),
        format!("dx = x{};", " + x".repeat(2000)),
    ] {
        let text = format!("function dx = pendulum(x, p)\n{body}\nend");
        let error = compile_with_sources(&model(), &sources(&text)).unwrap_err();
        assert_eq!(error.0.code, "function_complexity");
        assert_eq!(error.0.source_path.as_deref(), Some("pendulum.m"));
        assert_eq!(error.0.line, Some(2));
    }
}

#[test]
fn function_named_like_an_intrinsic_cannot_call_itself() {
    let mut model = model();
    if let BlockKind::MFunction { entry, .. } = &mut model.blocks[0].kind {
        *entry = "sin".into();
    }
    let error = compile_with_sources(
        &model,
        &sources("function dx = sin(x, p)\ndx = sin(x);\nend"),
    )
    .unwrap_err();
    assert_eq!(error.0.code, "function_call");
    assert_eq!(error.0.line, Some(2));
}

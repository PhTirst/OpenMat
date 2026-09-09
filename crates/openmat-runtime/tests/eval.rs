use openmat_array::ArrayData;
use openmat_runtime::{
    BuiltinContext, BuiltinErrorCategory, BuiltinRegistry, Interpreter, LineSpacing, NullOutput,
    OutputEvent, RuntimeErrorKind,
};
use openmat_source::SourceId;
use openmat_value::Value;

fn interpreter(source: &str) -> Interpreter {
    let parsed = openmat_parser::parse(SourceId::new(1), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let lowered = openmat_hir::lower(&parsed.syntax);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let module = openmat_compiler::compile(&lowered.file).expect("source should compile");
    Interpreter::new(module).expect("module should verify")
}

fn execute(source: &str) -> Interpreter {
    let mut runtime = interpreter(source);
    runtime.execute_entry(&[]).expect("source should execute");
    runtime
}

fn execute_with_display_builtins(source: &str) -> Interpreter {
    let parsed = openmat_parser::parse(SourceId::new(1), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let lowered = openmat_hir::lower(&parsed.syntax);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let module = openmat_compiler::compile(&lowered.file).expect("source should compile");
    let mut builtins = BuiltinRegistry::new();
    builtins
        .register(
            "show",
            |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                context.emit(OutputEvent::Display(arguments[0].clone()))?;
                Ok(Vec::new())
            },
        )
        .expect("show should register");
    builtins
        .register(
            "compact",
            |_arguments: &[Value], context: &mut BuiltinContext<'_>| {
                let mut format = context.display_format()?;
                format.line_spacing = LineSpacing::Compact;
                context.set_display_format(format)?;
                context.emit(OutputEvent::DisplayFormatChanged(format))?;
                Ok(Vec::new())
            },
        )
        .expect("compact should register");
    let mut runtime = Interpreter::with_components(module, builtins, Box::new(NullOutput))
        .expect("module should verify");
    runtime.execute_entry(&[]).expect("source should execute");
    runtime
}

fn char_text(value: &Value) -> String {
    let Value::Array(ArrayData::Char(array)) = value else {
        panic!("expected a char array, received {value:?}");
    };
    String::from_utf16(
        &array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
    )
    .expect("captured output should be valid UTF-16")
}

#[test]
fn eval_reads_and_writes_the_current_script_workspace() {
    let runtime =
        execute("x = 4; value = eval('x + 1'); eval('x = x + 2; y = x * 3;'); result = value + y;");

    assert_eq!(runtime.workspace().get("x"), Some(&Value::Double(6.0)));
    assert_eq!(runtime.workspace().get("y"), Some(&Value::Double(18.0)));
    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(23.0))
    );
    assert!(!runtime.workspace().contains("ans"));
}

#[test]
fn eval_statement_mode_updates_ans_but_output_mode_does_not() {
    let statement_runtime = execute("eval('1 + 2');");
    assert_eq!(
        statement_runtime.workspace().get("ans"),
        Some(&Value::Double(3.0))
    );

    let output_runtime = execute("value = eval(\"4 + 5\");");
    assert_eq!(
        output_runtime.workspace().get("value"),
        Some(&Value::Double(9.0))
    );
    assert!(!output_runtime.workspace().contains("ans"));
}

#[test]
fn eval_uses_the_calling_function_workspace() {
    let runtime = execute(
        "result = local_eval(4); function out = local_eval(seed); x = seed; y = eval('x + 1'); eval('z = x + 2;'); out = y + z; end",
    );

    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(11.0))
    );
    assert!(!runtime.workspace().contains("x"));
    assert!(!runtime.workspace().contains("z"));
}

#[test]
fn eval_forwards_multiple_outputs_from_a_visible_local_function() {
    let runtime = execute(
        "[m, n] = eval('pair()'); result = m * 10 + n; function [a, b] = pair(); a = 2; b = 3; end",
    );

    assert_eq!(runtime.workspace().get("m"), Some(&Value::Double(2.0)));
    assert_eq!(runtime.workspace().get("n"), Some(&Value::Double(3.0)));
    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(23.0))
    );
}

#[test]
fn eval_preserves_completed_assignments_before_a_runtime_error() {
    let runtime = execute(
        "x = 1; caught = false; try; eval('x = 2; definitely_missing_name'); catch err; caught = true; end; result = x;",
    );

    assert_eq!(runtime.workspace().get("x"), Some(&Value::Double(2.0)));
    assert_eq!(
        runtime.workspace().get("caught"),
        Some(&Value::Logical(true))
    );
    assert_eq!(runtime.workspace().get("result"), Some(&Value::Double(2.0)));
}

#[test]
fn eval_output_mode_rejects_statement_text_without_side_effects() {
    let runtime = execute(
        "x = 1; caught = false; try; value = eval('x = 3'); catch err; caught = true; end; result = x;",
    );

    assert_eq!(runtime.workspace().get("x"), Some(&Value::Double(1.0)));
    assert_eq!(
        runtime.workspace().get("caught"),
        Some(&Value::Logical(true))
    );
    assert_eq!(runtime.workspace().get("result"), Some(&Value::Double(1.0)));
}

#[test]
fn eval_scope_changes_flow_through_feval_and_function_handles() {
    let runtime = execute(
        "a = 4; first = feval('eval', 'a + 1'); h = @eval; h('b = a + 2;'); result = first + b;",
    );

    assert_eq!(runtime.workspace().get("a"), Some(&Value::Double(4.0)));
    assert_eq!(runtime.workspace().get("b"), Some(&Value::Double(6.0)));
    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(11.0))
    );
}

#[test]
fn eval_clear_removes_a_caller_binding() {
    let runtime = execute("x = 1; y = 2; eval('clear x'); result = y;");

    assert!(!runtime.workspace().contains("x"));
    assert_eq!(runtime.workspace().get("y"), Some(&Value::Double(2.0)));
}

#[test]
fn eval_clear_resets_a_named_function_local() {
    let runtime = execute(
        "result = clear_local(); function out = clear_local(); x = 1; eval('clear x'); try; y = x + 1; out = false; catch err; out = true; end; end",
    );

    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Logical(true))
    );
}

#[test]
fn eval_rejects_non_text_input_with_a_structured_error() {
    let mut runtime = interpreter("eval(42);");
    let error = runtime
        .execute_entry(&[])
        .expect_err("numeric eval source should fail");

    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Builtin {
            name,
            category: BuiltinErrorCategory::Type,
            identifier: Some(identifier),
            ..
        } if name == "eval" && identifier == "OpenMat:eval:InvalidText"
    ));
}

#[test]
fn evalc_captures_named_display_disp_and_suppressed_statements() {
    let runtime = execute_with_display_builtins(
        "named = evalc('1 + 2'); displayed = evalc('show(''hi'')'); quiet = evalc('x = 5;');",
    );

    assert_eq!(
        char_text(runtime.workspace().get("named").expect("named capture")),
        "\nans =\n\n     3\n\n"
    );
    assert_eq!(
        char_text(
            runtime
                .workspace()
                .get("displayed")
                .expect("display capture")
        ),
        "hi\n"
    );
    assert_eq!(
        char_text(runtime.workspace().get("quiet").expect("quiet capture")),
        ""
    );
    assert_eq!(runtime.workspace().get("x"), Some(&Value::Double(5.0)));
}

#[test]
fn evalc_returns_evaluated_outputs_after_the_captured_text() {
    let runtime = execute(
        "[captured, value] = evalc('4 + 5'); [caught, recovered] = evalc('missing_evalc_name', '6 + 5');",
    );

    assert_eq!(
        char_text(runtime.workspace().get("captured").expect("capture")),
        ""
    );
    assert_eq!(runtime.workspace().get("value"), Some(&Value::Double(9.0)));
    assert_eq!(
        char_text(runtime.workspace().get("caught").expect("catch capture")),
        ""
    );
    assert_eq!(
        runtime.workspace().get("recovered"),
        Some(&Value::Double(11.0))
    );
}

#[test]
fn evalc_tracks_format_changes_and_nests_without_leaking_inner_text() {
    let runtime = execute_with_display_builtins(
        "compact_text = evalc('compact(); 1 + 2'); outer = evalc('inner = evalc(''show(''''hidden'''')''); show(''shown'')');",
    );

    assert_eq!(
        char_text(
            runtime
                .workspace()
                .get("compact_text")
                .expect("compact capture")
        ),
        "ans =\n     3\n"
    );
    assert_eq!(
        char_text(runtime.workspace().get("outer").expect("outer capture")),
        "shown\n"
    );
    assert_eq!(runtime.display_format().line_spacing, LineSpacing::Compact);
}

#[test]
fn eval_catch_source_runs_lazily_in_the_current_workspace() {
    let runtime = execute(
        "x = 1; eval('x = 2; missing_primary_name', 'y = x + 3;'); recovered = eval('missing_result_name', '4 + 5'); successful = eval('6 + 1', 42);",
    );

    assert_eq!(runtime.workspace().get("x"), Some(&Value::Double(2.0)));
    assert_eq!(runtime.workspace().get("y"), Some(&Value::Double(5.0)));
    assert_eq!(
        runtime.workspace().get("recovered"),
        Some(&Value::Double(9.0))
    );
    assert_eq!(
        runtime.workspace().get("successful"),
        Some(&Value::Double(7.0))
    );
}

#[test]
fn eval_preserves_both_primary_and_catch_writes_when_catch_fails() {
    let runtime = execute(
        "x = 1; caught = false; try; eval('x = 2; missing_primary_name', 'y = x + 3; missing_catch_name'); catch err; caught = true; end; result = x + y;",
    );

    assert_eq!(runtime.workspace().get("result"), Some(&Value::Double(7.0)));
    assert_eq!(
        runtime.workspace().get("caught"),
        Some(&Value::Logical(true))
    );
}

#[test]
fn evalin_catch_source_runs_in_the_invoking_workspace() {
    let runtime = execute(
        "result = outer_scope(); function out = outer_scope(); x = 1; child = inner_scope(); out = child + x; end; function out = inner_scope(); evalin('caller', 'x = 3; missing_primary_name', 'local_y = 7;'); out = evalin('caller', 'missing_result_name', 'local_y + 2'); end",
    );

    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(12.0))
    );
}

#[test]
fn evalin_preserves_primary_and_local_catch_writes_when_catch_fails() {
    let runtime = execute(
        "result = outer_scope(); function out = outer_scope(); x = 1; child = inner_scope(); out = child + x; end; function out = inner_scope(); try; evalin('caller', 'x = 3; missing_primary_name', 'local_y = 7; missing_catch_name'); catch err; end; out = local_y; end",
    );

    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(10.0))
    );
}

#[test]
fn assignin_and_evalin_address_the_immediate_caller_workspace() {
    let runtime = execute(
        "result = outer_scope(); function out = outer_scope(); x = 1; child_value = inner_scope(); out = child_value + x + y + z; end; function out = inner_scope(); assignin('caller', 'x', 3); seen_x = evalin('caller', 'x'); evalin('caller', 'y = x + 2;'); assignin('caller', 'z', 7); out = seen_x + evalin('caller', 'y') + evalin('caller', 'z'); end",
    );

    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(30.0))
    );
}

#[test]
fn evalin_preserves_caller_writes_completed_before_an_error() {
    let runtime = execute(
        "result = outer_scope(); function out = outer_scope(); x = 1; inner_scope(); out = x; end; function inner_scope(); try; evalin('caller', 'x = 4; missing_evalin_name'); catch err; end; end",
    );

    assert_eq!(runtime.workspace().get("result"), Some(&Value::Double(4.0)));
}

#[test]
fn base_scope_remains_addressable_from_a_function() {
    let runtime = execute(
        "result = write_base(); observed = evalin('base', 'openmat_scope_base'); evalin('base', 'clear openmat_scope_base'); function out = write_base(); assignin('base', 'openmat_scope_base', 11); out = evalin('base', 'openmat_scope_base + 1'); end",
    );

    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(12.0))
    );
    assert_eq!(
        runtime.workspace().get("observed"),
        Some(&Value::Double(11.0))
    );
    assert!(!runtime.workspace().contains("openmat_scope_base"));
}

#[test]
fn assignin_base_is_immediately_visible_inside_dynamic_base_code() {
    let runtime = execute(
        "evalin('base', 'assignin(''base'', ''dynamic_base_value'', 5); observed = dynamic_base_value + 1;'); result = observed; clear dynamic_base_value;",
    );

    assert_eq!(runtime.workspace().get("result"), Some(&Value::Double(6.0)));
    assert!(!runtime.workspace().contains("dynamic_base_value"));
}

#[test]
fn caller_scope_at_the_entry_boundary_resolves_to_base() {
    let runtime = execute(
        "assignin(\"caller\", \"entry_value\", 5); result = evalin(\"caller\", \"entry_value + 1\");",
    );

    assert_eq!(
        runtime.workspace().get("entry_value"),
        Some(&Value::Double(5.0))
    );
    assert_eq!(runtime.workspace().get("result"), Some(&Value::Double(6.0)));
}

#[test]
fn assignin_uses_language_copy_semantics() {
    let runtime = execute(
        "source = [1, 2]; copy_to_caller(source); source(1) = 9; result = assigned(1); function copy_to_caller(value); assignin('caller', 'assigned', value); end",
    );

    assert_eq!(runtime.workspace().get("result"), Some(&Value::Double(1.0)));
}

#[test]
fn assignin_rejects_invalid_workspace_and_variable_names() {
    let mut invalid_workspace = interpreter("assignin('current', 'x', 1);");
    let workspace_error = invalid_workspace
        .execute_entry(&[])
        .expect_err("unsupported workspace should fail");
    assert!(matches!(
        workspace_error.kind,
        RuntimeErrorKind::Builtin {
            identifier: Some(identifier),
            ..
        } if identifier == "OpenMat:assignin:InvalidWorkspace"
    ));

    let mut invalid_name = interpreter("assignin('base', 'two words', 1);");
    let name_error = invalid_name
        .execute_entry(&[])
        .expect_err("invalid variable name should fail");
    assert!(matches!(
        name_error.kind,
        RuntimeErrorKind::Builtin {
            identifier: Some(identifier),
            ..
        } if identifier == "OpenMat:assignin:InvalidVariable"
    ));
}

#[test]
fn nested_eval_keeps_current_caller_and_base_targets_distinct() {
    let runtime = execute(
        "result = nested_scope(); caller_observed = caller_from_eval; base_observed = evalin('base', 'base_from_eval'); evalin('base', 'clear base_from_eval'); function out = nested_scope(); x = 1; eval('assignin(''caller'', ''caller_from_eval'', 7); x = x + 1; assignin(''base'', ''base_from_eval'', 9);'); out = x + evalin('base', 'base_from_eval'); end",
    );

    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(11.0))
    );
    assert_eq!(
        runtime.workspace().get("caller_observed"),
        Some(&Value::Double(7.0))
    );
    assert_eq!(
        runtime.workspace().get("base_observed"),
        Some(&Value::Double(9.0))
    );
    assert!(!runtime.workspace().contains("base_from_eval"));
}

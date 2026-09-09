use openmat_array::ArrayData;
use openmat_runtime::Interpreter;
use openmat_source::SourceId;
use openmat_value::{StructArray, Value};

fn module(source: &str, source_id: SourceId) -> openmat_bytecode::BytecodeModule {
    let parsed = openmat_parser::parse(source_id, source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let lowered = openmat_hir::lower(&parsed.syntax);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    openmat_compiler::compile(&lowered.file).expect("source should compile")
}

fn interpreter(source: &str) -> Interpreter {
    let source_id = SourceId::new(1);
    let mut runtime = Interpreter::new(module(source, source_id)).expect("module should verify");
    runtime.register_source_metadata(source_id, "last_error_test.m", source);
    runtime
}

fn execute(source: &str) -> Interpreter {
    let mut runtime = interpreter(source);
    runtime.execute_entry(&[]).expect("source should execute");
    runtime
}

fn char_text(value: &Value) -> String {
    let Value::Array(ArrayData::Char(array)) = value else {
        panic!("expected char array, received {value:?}");
    };
    String::from_utf16(
        &array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
    )
    .expect("char array should contain valid UTF-16")
}

fn struct_value<'a>(value: &'a Value, field: &str) -> &'a Value {
    let Value::Struct(array) = value else {
        panic!("expected struct array, received {value:?}");
    };
    let field = array.field_index(field).expect("field should exist");
    array.value_at(field, 0).expect("scalar field should exist")
}

fn struct_array(value: &Value) -> &StructArray {
    let Value::Struct(array) = value else {
        panic!("expected struct array, received {value:?}");
    };
    array
}

#[test]
fn lasterr_and_lasterror_share_state_and_return_the_previous_value() {
    let runtime = execute(
        "[initial_message, initial_id] = lasterr; initial = lasterror; \
         [old_message, old_id] = lasterr('alpha', 'OpenMat:Alpha'); \
         [message, id] = lasterr; replacement = lasterror; \
         replacement.message = 'beta'; replacement.identifier = 'OpenMat:Beta'; \
         old_state = lasterror(replacement); current = lasterror;",
    );
    let workspace = runtime.workspace();

    let initial_message = workspace.get("initial_message").expect("initial message");
    assert_eq!(char_text(initial_message), "");
    assert_eq!(initial_message.dimensions(), Some(&[0, 0][..]));
    assert_eq!(
        char_text(workspace.get("initial_id").expect("initial identifier")),
        ""
    );
    assert_eq!(
        char_text(workspace.get("old_message").expect("old message")),
        ""
    );
    assert_eq!(
        char_text(workspace.get("old_id").expect("old identifier")),
        ""
    );
    assert_eq!(
        char_text(workspace.get("message").expect("message")),
        "alpha"
    );
    assert_eq!(
        char_text(workspace.get("id").expect("identifier")),
        "OpenMat:Alpha"
    );

    let initial = workspace.get("initial").expect("initial state");
    assert_eq!(char_text(struct_value(initial, "message")), "");
    assert_eq!(char_text(struct_value(initial, "identifier")), "");
    assert_eq!(
        struct_array(struct_value(initial, "stack"))
            .shape()
            .dimensions(),
        [0, 1]
    );
    let old_state = workspace.get("old_state").expect("old state");
    assert_eq!(char_text(struct_value(old_state, "message")), "alpha");
    assert_eq!(
        char_text(struct_value(old_state, "identifier")),
        "OpenMat:Alpha"
    );
    let current = workspace.get("current").expect("current state");
    assert_eq!(char_text(struct_value(current, "message")), "beta");
    assert_eq!(
        char_text(struct_value(current, "identifier")),
        "OpenMat:Beta"
    );
}

#[test]
fn caught_errors_update_structured_state_before_the_catch_body() {
    let source = "lasterr('seed', 'Seed:Id');\ntry\n    missing_last_error_name;\ncatch\n    inside = lasterror;\nend\nvalue = 1 + 1;\nafter = lasterror;";
    let runtime = execute(source);

    for name in ["inside", "after"] {
        let state = runtime.workspace().get(name).expect("captured state");
        assert_eq!(
            char_text(struct_value(state, "identifier")),
            "OpenMat:UndefinedName"
        );
        assert!(char_text(struct_value(state, "message")).contains("missing_last_error_name"));
    }

    let stack = struct_array(struct_value(
        runtime.workspace().get("inside").expect("inside state"),
        "stack",
    ));
    assert_eq!(stack.shape().dimensions(), [1, 1]);
    assert_eq!(
        stack
            .field_names()
            .iter()
            .map(openmat_value::FieldName::as_str)
            .collect::<Vec<_>>(),
        vec!["file", "name", "line"]
    );
    assert_eq!(
        char_text(
            stack
                .value_at(stack.field_index("file").unwrap(), 0)
                .unwrap()
        ),
        "last_error_test.m"
    );
    assert_eq!(
        stack.value_at(stack.field_index("line").unwrap(), 0),
        Some(&Value::Double(3.0))
    );
}

#[test]
fn eval_fallback_observes_primary_error_and_newer_errors_replace_it() {
    let runtime = execute(
        "lasterr('seed', 'Seed:Id'); \
         eval('missing_primary_name', 'inside = lasterror;'); after = lasterror; \
         eval('if', 'compile_error = lasterror;'); \
         try; eval('missing_first_name', 'missing_second_name'); catch; latest = lasterror; end;",
    );

    for name in ["inside", "after"] {
        let state = runtime.workspace().get(name).expect("primary state");
        assert_eq!(
            char_text(struct_value(state, "identifier")),
            "OpenMat:UndefinedName"
        );
        assert!(char_text(struct_value(state, "message")).contains("missing_primary_name"));
    }
    let compile_error = runtime
        .workspace()
        .get("compile_error")
        .expect("dynamic compile state");
    assert_eq!(
        char_text(struct_value(compile_error, "identifier")),
        "OpenMat:eval:ParseError"
    );
    let latest = runtime.workspace().get("latest").expect("latest state");
    assert!(char_text(struct_value(latest, "message")).contains("missing_second_name"));
}

#[test]
fn lasterr_resets_stack_and_invalid_calls_become_the_last_error() {
    let runtime = execute(
        "try; missing_for_stack; catch; natural = lasterror; end; \
         lasterr('replacement'); replaced = lasterror; \
         old = lasterror(natural); restored = lasterror; \
         try; lasterr(42); catch; invalid = lasterror; end;",
    );

    let replaced = runtime
        .workspace()
        .get("replaced")
        .expect("replacement state");
    assert_eq!(char_text(struct_value(replaced, "message")), "replacement");
    assert_eq!(char_text(struct_value(replaced, "identifier")), "");
    assert_eq!(
        struct_array(struct_value(replaced, "stack"))
            .shape()
            .dimensions(),
        [0, 1]
    );
    let old = runtime.workspace().get("old").expect("old state");
    assert_eq!(char_text(struct_value(old, "message")), "replacement");
    let restored = runtime.workspace().get("restored").expect("restored state");
    assert!(char_text(struct_value(restored, "message")).contains("missing_for_stack"));
    assert_eq!(
        char_text(struct_value(
            runtime.workspace().get("invalid").expect("invalid state"),
            "identifier"
        )),
        "OpenMat:lasterr:InvalidText"
    );
}

#[test]
fn unhandled_errors_survive_module_replacement_and_session_clear_resets_them() {
    let failing_source = "missing_across_commands;";
    let mut runtime = interpreter(failing_source);
    runtime
        .execute_entry(&[])
        .expect_err("first command should fail");

    let getter_source = "observed = lasterror;";
    let getter_id = SourceId::new(2);
    runtime
        .replace_module(module(getter_source, getter_id))
        .expect("getter module should verify");
    runtime.register_source_metadata(getter_id, "getter.m", getter_source);
    runtime
        .execute_entry(&[])
        .expect("second command should inspect the state");
    assert!(
        char_text(struct_value(
            runtime.workspace().get("observed").expect("observed state"),
            "message"
        ))
        .contains("missing_across_commands")
    );

    runtime.clear_session();
    let cleared_source = "cleared = lasterror;";
    let cleared_id = SourceId::new(3);
    runtime
        .replace_module(module(cleared_source, cleared_id))
        .expect("cleared getter module should verify");
    runtime.register_source_metadata(cleared_id, "cleared.m", cleared_source);
    runtime
        .execute_entry(&[])
        .expect("cleared state should remain readable");
    assert_eq!(
        char_text(struct_value(
            runtime.workspace().get("cleared").expect("cleared state"),
            "message"
        )),
        ""
    );
}

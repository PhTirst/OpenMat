use openmat_array::ArrayData;
use openmat_bytecode::{
    ApplyArgument, BytecodeModule, Constant, ConstantId, ExceptionHandler, Function, FunctionId,
    Instruction, InstructionIndex, InstructionKind, LocalSlot, Register, SourceLocation,
};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistry, Interpreter,
    RuntimeErrorKind,
};
use openmat_value::{BytecodeFunctionHandle, FunctionHandle, Value};

fn pc(index: u32) -> InstructionIndex {
    InstructionIndex::new(index)
}

fn instruction(kind: InstructionKind) -> Instruction {
    Instruction::new(kind)
}

fn function(
    name: &str,
    registers: u32,
    locals: u32,
    parameters: u32,
    constants: Vec<Constant>,
    instructions: Vec<Instruction>,
    handlers: Vec<ExceptionHandler>,
) -> Function {
    let mut function = Function::new(name, registers, locals, parameters);
    function.constants = constants;
    function.instructions = instructions;
    function.exception_handlers = handlers;
    function
}

fn module(functions: Vec<Function>) -> BytecodeModule {
    BytecodeModule::new(functions, FunctionId::new(0))
}

fn scalar_string(value: &Value) -> String {
    let code_units = match value {
        Value::String(value) => value
            .as_scalar()
            .expect("value should be a string scalar")
            .code_units()
            .to_vec(),
        Value::Array(ArrayData::Char(value)) => {
            value.as_slice().iter().map(|unit| unit.get()).collect()
        }
        _ => panic!("value should be a text scalar"),
    };
    String::from_utf16(&code_units).expect("test strings should be valid UTF-16")
}

#[test]
#[allow(clippy::too_many_lines)]
fn executes_plain_bound_swallow_and_normal_paths() {
    let plain = function(
        "plain-catch",
        1,
        0,
        0,
        vec![
            Constant::String("missing".to_owned()),
            Constant::Double(13.0),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Jump { target: pc(4) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Jump { target: pc(4) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        vec![ExceptionHandler::catch(pc(0), pc(1), pc(2), pc(4), None)],
    );
    assert_eq!(
        Interpreter::new(module(vec![plain]))
            .unwrap()
            .execute_entry(&[])
            .unwrap(),
        vec![Value::Double(13.0)]
    );

    let bound = function(
        "bound-catch",
        3,
        1,
        0,
        vec![
            Constant::String("missing".to_owned()),
            Constant::String("isobject".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Jump { target: pc(6) }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(2)],
                callee: Register::new(1),
                arguments: vec![Register::new(0)],
            }),
            instruction(InstructionKind::Jump { target: pc(6) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2), Register::new(0)],
            }),
        ],
        vec![ExceptionHandler::catch(
            pc(0),
            pc(1),
            pc(2),
            pc(6),
            Some(LocalSlot::new(0)),
        )],
    );
    let bound = Interpreter::new(module(vec![bound]))
        .unwrap()
        .execute_entry(&[])
        .unwrap();
    assert_eq!(bound[0], Value::Logical(true));
    assert!(matches!(bound[1], Value::Object(_)));
    assert_eq!(bound[1].numel(), Some(1));

    let swallow = function(
        "swallow",
        1,
        0,
        0,
        vec![
            Constant::String("missing".to_owned()),
            Constant::Double(42.0),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        vec![ExceptionHandler::swallow(pc(0), pc(1), pc(1))],
    );
    assert_eq!(
        Interpreter::new(module(vec![swallow]))
            .unwrap()
            .execute_entry(&[])
            .unwrap(),
        vec![Value::Double(42.0)]
    );

    let normal = function(
        "normal-path",
        1,
        0,
        0,
        vec![Constant::Double(7.0), Constant::Double(99.0)],
        vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(3) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        vec![ExceptionHandler::catch(pc(0), pc(1), pc(2), pc(3), None)],
    );
    assert_eq!(
        Interpreter::new(module(vec![normal]))
            .unwrap()
            .execute_entry(&[])
            .unwrap(),
        vec![Value::Double(7.0)]
    );
}

#[test]
fn selects_the_innermost_handler_independent_of_table_order() {
    let inner = ExceptionHandler::catch(pc(1), pc(3), pc(4), pc(5), None);
    let outer = ExceptionHandler::catch(pc(0), pc(7), pc(8), pc(10), None);
    let nested = function(
        "nearest-handler",
        1,
        0,
        0,
        vec![
            Constant::Double(0.0),
            Constant::String("missing".to_owned()),
            Constant::Double(2.0),
            Constant::Double(1.0),
        ],
        vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(5) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Jump { target: pc(10) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(10) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(3),
            }),
            instruction(InstructionKind::Jump { target: pc(10) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        vec![outer, inner],
    );
    assert_eq!(
        Interpreter::new(module(vec![nested]))
            .unwrap()
            .execute_entry(&[])
            .unwrap(),
        vec![Value::Double(2.0)]
    );
}

#[test]
fn catch_body_failure_reaches_outer_and_preserves_completed_write() {
    let inner = ExceptionHandler::catch(pc(1), pc(2), pc(3), pc(9), Some(LocalSlot::new(0)));
    let outer = ExceptionHandler::catch(pc(0), pc(10), pc(11), pc(14), None);
    let nested = function(
        "catch-body-unwind",
        3,
        1,
        0,
        vec![
            Constant::Double(0.0),
            Constant::String("missing-inner".to_owned()),
            Constant::String("persisted".to_owned()),
            Constant::String("persisted object write".to_owned()),
            Constant::String("missing-catch".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Jump { target: pc(9) }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(3),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(2),
                src: Register::new(0),
            }),
            instruction(InstructionKind::StoreLocal {
                local: LocalSlot::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(2),
                name: ConstantId::new(4),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Jump { target: pc(9) }),
            instruction(InstructionKind::Jump { target: pc(14) }),
            instruction(InstructionKind::Jump { target: pc(14) }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(2),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Move {
                dst: Register::new(1),
                src: Register::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(14) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        vec![inner, outer],
    );
    let returned = Interpreter::new(module(vec![nested]))
        .unwrap()
        .execute_entry(&[])
        .unwrap();
    assert_eq!(scalar_string(&returned[0]), "persisted object write");
}

#[test]
fn catches_errors_propagated_from_a_called_bytecode_frame() {
    let caller = function(
        "caller",
        2,
        1,
        0,
        vec![
            Constant::Function(FunctionId::new(1)),
            Constant::String("stack".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            Instruction::located(
                InstructionKind::Call {
                    outputs: vec![Register::new(1)],
                    callee: Register::new(0),
                    arguments: Vec::new(),
                },
                SourceLocation::new(8, 10, 15),
            ),
            instruction(InstructionKind::Jump { target: pc(6) }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(1),
                object: Register::new(0),
                name: ConstantId::new(1),
            }),
            instruction(InstructionKind::Jump { target: pc(6) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        vec![ExceptionHandler::catch(
            pc(1),
            pc(2),
            pc(3),
            pc(6),
            Some(LocalSlot::new(0)),
        )],
    );
    let failing_function = function(
        "callee",
        1,
        0,
        0,
        vec![Constant::String("missing-in-callee".to_owned())],
        vec![
            Instruction::located(
                InstructionKind::LoadGlobal {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                    construct_if_class: false,
                },
                SourceLocation::new(9, 20, 30),
            ),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        Vec::new(),
    );
    let values = Interpreter::new(module(vec![caller, failing_function]))
        .unwrap()
        .execute_entry(&[])
        .unwrap();
    let Value::Struct(stack) = &values[0] else {
        panic!("cross-frame catch should return the exception stack");
    };
    assert_eq!(stack.shape().dimensions(), [2, 1]);
    let name = stack.field_index("name").unwrap();
    assert_eq!(scalar_string(stack.value_at(name, 0).unwrap()), "callee");
    assert_eq!(scalar_string(stack.value_at(name, 1).unwrap()), "caller");
}

#[test]
fn catches_builtin_and_invalid_handle_failures_at_apply_pc() {
    let apply = function(
        "builtin-apply",
        2,
        0,
        0,
        vec![
            Constant::String("explode".to_owned()),
            Constant::Double(8.0),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(1)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Jump { target: pc(5) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Jump { target: pc(5) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        vec![ExceptionHandler::catch(pc(1), pc(2), pc(3), pc(5), None)],
    );
    let mut registry = BuiltinRegistry::new();
    registry
        .register(
            "explode",
            |_arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "OpenMat test failure",
                ))
            },
        )
        .unwrap();
    assert_eq!(
        Interpreter::with_registry(module(vec![apply]), registry)
            .unwrap()
            .execute_entry(&[])
            .unwrap(),
        vec![Value::Double(8.0)]
    );

    let invalid_handle = function(
        "invalid-handle-apply",
        2,
        1,
        1,
        vec![Constant::Double(9.0)],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(1)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Jump { target: pc(5) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(5) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        vec![ExceptionHandler::catch(pc(1), pc(2), pc(3), pc(5), None)],
    );
    let bad = Value::Function(FunctionHandle::Bytecode(BytecodeFunctionHandle::new(999)));
    assert_eq!(
        Interpreter::new(module(vec![invalid_handle]))
            .unwrap()
            .execute_entry(&[bad])
            .unwrap(),
        vec![Value::Double(9.0)]
    );
}

#[test]
fn completed_local_and_workspace_writes_survive_unwind() {
    let writes = function(
        "write-preservation",
        2,
        1,
        0,
        vec![
            Constant::String("kept".to_owned()),
            Constant::Double(7.0),
            Constant::String("missing".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::StoreLocal {
                local: LocalSlot::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(2),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Jump { target: pc(7) }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(7) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        vec![ExceptionHandler::catch(pc(0), pc(4), pc(5), pc(7), None)],
    );
    let mut interpreter = Interpreter::new(module(vec![writes])).unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(7.0)]
    );
    assert_eq!(
        interpreter.workspace().get("kept"),
        Some(&Value::Double(7.0))
    );
}

#[test]
fn catches_structured_array_index_errors() {
    let indexing = function(
        "array-index-error",
        3,
        2,
        2,
        vec![Constant::Double(42.0)],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Jump { target: pc(6) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(6) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        vec![ExceptionHandler::catch(pc(2), pc(3), pc(4), pc(6), None)],
    );
    assert_eq!(
        Interpreter::new(module(vec![indexing]))
            .unwrap()
            .execute_entry(&[Value::Double(1.0), Value::Double(2.0)])
            .unwrap(),
        vec![Value::Double(42.0)]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn bound_exception_has_public_compatible_fields_and_survives_global_assignment() {
    let location = SourceLocation::new(7, 10, 20);
    let caught = function(
        "exception-fields",
        7,
        1,
        0,
        vec![
            Constant::String("missing".to_owned()),
            Constant::String("saved".to_owned()),
            Constant::String("identifier".to_owned()),
            Constant::String("message".to_owned()),
            Constant::String("stack".to_owned()),
            Constant::String("cause".to_owned()),
            Constant::String("isobject".to_owned()),
        ],
        vec![
            Instruction::located(
                InstructionKind::LoadGlobal {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                    construct_if_class: false,
                },
                location,
            ),
            instruction(InstructionKind::Jump { target: pc(13) }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(1),
                src: Register::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(1),
                object: Register::new(0),
                name: ConstantId::new(2),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(2),
                object: Register::new(0),
                name: ConstantId::new(3),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(3),
                object: Register::new(0),
                name: ConstantId::new(4),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(4),
                object: Register::new(0),
                name: ConstantId::new(5),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(5),
                name: ConstantId::new(6),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(6)],
                callee: Register::new(5),
                arguments: vec![Register::new(0)],
            }),
            instruction(InstructionKind::Jump { target: pc(13) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![
                    Register::new(0),
                    Register::new(1),
                    Register::new(2),
                    Register::new(3),
                    Register::new(4),
                    Register::new(6),
                ],
            }),
        ],
        vec![ExceptionHandler::catch(
            pc(0),
            pc(1),
            pc(2),
            pc(13),
            Some(LocalSlot::new(0)),
        )],
    );
    let mut interpreter = Interpreter::new(module(vec![caught])).unwrap();
    let values = interpreter.execute_entry(&[]).unwrap();
    assert!(matches!(values[0], Value::Object(_)));
    assert_eq!(values[0].numel(), Some(1));
    assert_eq!(interpreter.value_class_name(&values[0]), "MException");
    assert_eq!(values[5], Value::Logical(true));
    assert_eq!(scalar_string(&values[1]), "OpenMat:UndefinedName");
    assert!(scalar_string(&values[2]).starts_with("OpenMat runtime error"));

    let Value::Struct(stack) = &values[3] else {
        panic!("stack should be a struct array");
    };
    assert_eq!(stack.shape().dimensions(), [1, 1]);
    assert_eq!(
        stack
            .field_names()
            .iter()
            .map(openmat_value::FieldName::as_str)
            .collect::<Vec<_>>(),
        vec!["file", "name", "line"]
    );
    let name = stack.field_index("name").unwrap();
    let line = stack.field_index("line").unwrap();
    assert_eq!(
        scalar_string(stack.value_at(name, 0).unwrap()),
        "exception-fields"
    );
    assert_eq!(stack.value_at(line, 0), Some(&Value::Double(0.0)));

    let Value::Cell(cause) = &values[4] else {
        panic!("cause should be a cell array");
    };
    assert_eq!(cause.shape().dimensions(), [0, 0]);
    assert_eq!(cause.numel(), 0);
    assert!(matches!(
        interpreter.workspace().get("saved"),
        Some(Value::Object(_))
    ));
}

#[test]
fn cancellation_bypasses_handlers() {
    let cancelled = function(
        "cancelled",
        1,
        0,
        0,
        vec![Constant::Double(1.0), Constant::Double(2.0)],
        vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(3) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        vec![ExceptionHandler::catch(pc(0), pc(1), pc(2), pc(3), None)],
    );
    let mut interpreter = Interpreter::new(module(vec![cancelled])).unwrap();
    interpreter.cancellation_token().cancel();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(error.kind, RuntimeErrorKind::Cancelled));
}

#[test]
fn internal_invalid_state_bypasses_handlers() {
    let internal = function(
        "internal-state",
        4,
        0,
        0,
        vec![
            Constant::Double(1.0),
            Constant::String("corrupt-loop-index".to_owned()),
            Constant::Double(99.0),
        ],
        vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::ForEach {
                iterable: Register::new(0),
                index: Register::new(1),
                dst: Register::new(2),
                exit: pc(3),
            }),
            instruction(InstructionKind::Jump { target: pc(6) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Jump { target: pc(6) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        vec![ExceptionHandler::catch(pc(0), pc(3), pc(4), pc(6), None)],
    );
    let error = Interpreter::new(module(vec![internal]))
        .unwrap()
        .execute_entry(&[])
        .unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InvalidExecutionState { array: None, .. }
    ));
}

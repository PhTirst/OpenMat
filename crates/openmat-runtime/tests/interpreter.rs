use openmat_array::{ArrayData, Complex64 as ArrayComplex64, DenseArray, Logical, Shape};
use openmat_bytecode::{
    ApplyArgument, BinaryOperator, BytecodeModule, Constant, ConstantId, Function, FunctionId,
    Instruction, InstructionIndex, InstructionKind, LocalSlot, Register, SharedCaptureSource,
    SourceLocation,
};
use openmat_runtime::{
    BuiltinContext, BuiltinFunction, BuiltinRegistry, BuiltinResult, FileOpenAccess, FileOpenMode,
    FileSystemService, Interpreter, LineSpacing, LocalFileSystem, NumericFormat, OutputError,
    OutputEvent, OutputSink, RuntimeErrorKind,
};
use openmat_value::{BytecodeFunctionHandle, FunctionHandle, StringValue, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn instruction(kind: InstructionKind) -> Instruction {
    Instruction::new(kind)
}

fn module(functions: Vec<Function>) -> BytecodeModule {
    BytecodeModule::new(functions, FunctionId::new(0))
}

#[test]
fn executes_scalar_arithmetic() {
    let function = Function {
        name: "arithmetic".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(6.0), Constant::Double(7.0)],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Multiply,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let result = Interpreter::new(module(vec![function]))
        .expect("module should verify")
        .execute_entry(&[])
        .expect("arithmetic should execute");
    assert_eq!(result, vec![Value::Double(42.0)]);
}

#[test]
fn empty_parenthesized_apply_preserves_an_ordinary_value() {
    let function = Function {
        name: "empty_apply".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(7.0)],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(1)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };

    assert_eq!(
        Interpreter::new(module(vec![function]))
            .expect("module should verify")
            .execute_entry(&[]),
        Ok(vec![Value::Double(7.0)])
    );
}

#[test]
fn executes_conditional_branch() {
    let function = Function {
        name: "branch".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Logical(false),
            Constant::Double(1.0),
            Constant::Double(2.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::JumpIfFalse {
                condition: Register::new(0),
                target: InstructionIndex::new(4),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Jump {
                target: InstructionIndex::new(5),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let result = Interpreter::new(module(vec![function]))
        .expect("module should verify")
        .execute_entry(&[])
        .expect("branch should execute");
    assert_eq!(result, vec![Value::Double(2.0)]);
}

#[test]
fn executes_loop_with_backward_jump() {
    let function = Function {
        name: "loop".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Double(0.0),
            Constant::Double(1.0),
            Constant::Double(5.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::LessThan,
                dst: Register::new(3),
                lhs: Register::new(0),
                rhs: Register::new(2),
            }),
            instruction(InstructionKind::JumpIfFalse {
                condition: Register::new(3),
                target: InstructionIndex::new(7),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(0),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Jump {
                target: InstructionIndex::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let result = Interpreter::new(module(vec![function]))
        .expect("module should verify")
        .execute_entry(&[])
        .expect("loop should execute");
    assert_eq!(result, vec![Value::Double(5.0)]);
}

#[test]
fn calls_bytecode_function_with_frame_locals() {
    let caller = Function {
        name: "caller".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Function(FunctionId::new(1)),
            Constant::Double(19.0),
            Constant::Double(23.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(3)],
                callee: Register::new(0),
                arguments: vec![Register::new(1), Register::new(2)],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let add = Function {
        name: "add".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let result = Interpreter::new(module(vec![caller, add]))
        .expect("module should verify")
        .execute_entry(&[])
        .expect("function call should execute");
    assert_eq!(result, vec![Value::Double(42.0)]);
}

#[test]
fn variadic_functions_pack_extra_inputs_and_expand_requested_outputs() {
    let caller = Function {
        name: "caller".to_owned(),
        register_count: 7,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Function(FunctionId::new(1)),
            Constant::Double(10.0),
            Constant::Double(20.0),
            Constant::Double(30.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(3),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(4), Register::new(5), Register::new(6)],
                callee: Register::new(0),
                arguments: vec![Register::new(1), Register::new(2), Register::new(3)],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4), Register::new(5), Register::new(6)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let variadic = Function {
        name: "variadic".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadVariadicInputs {
                dst: Register::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::ReturnVariadic {
                fixed: vec![Register::new(1)],
                variadic: Register::new(0),
            }),
        ],
        exception_handlers: Vec::new(),
    };

    let result = Interpreter::new(module(vec![caller, variadic]))
        .expect("module should verify")
        .execute_entry(&[])
        .expect("variadic call should execute");
    assert_eq!(
        result,
        vec![
            Value::Double(10.0),
            Value::Double(20.0),
            Value::Double(30.0)
        ]
    );
}

#[test]
fn variadic_input_cells_use_zero_by_zero_empty_and_row_vector_shapes() {
    let variadic = Function {
        name: "variadic_cell".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadVariadicInputs {
                dst: Register::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(module(vec![variadic])).expect("variadic module");

    let empty = interpreter
        .execute_entry(&[])
        .expect("zero variadic inputs should execute")
        .pop()
        .expect("cell result");
    let Value::Cell(empty) = empty else {
        panic!("variadic inputs should produce a cell array");
    };
    assert_eq!(empty.shape().dimensions(), &[0, 0]);

    let populated = interpreter
        .execute_entry(&[Value::Double(4.0), Value::Double(5.0)])
        .expect("multiple variadic inputs should execute")
        .pop()
        .expect("cell result");
    let Value::Cell(populated) = populated else {
        panic!("variadic inputs should produce a cell array");
    };
    assert_eq!(populated.shape().dimensions(), &[1, 2]);
    assert_eq!(
        populated.values(),
        &[Value::Double(4.0), Value::Double(5.0)]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn nested_closure_shared_capture_survives_its_parent_frame() {
    let entry = Function {
        name: "entry".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Function(FunctionId::new(1))],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(1)],
                callee: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(2)],
                callee: Register::new(1),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(3)],
                callee: Register::new(1),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2), Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let factory = Function {
        name: "factory".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(0.0), Constant::String("count".to_owned())],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::StoreLocal {
                local: LocalSlot::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::MakeSharedClosure {
                dst: Register::new(1),
                function: FunctionId::new(2),
                captures: vec![(
                    ConstantId::new(1),
                    SharedCaptureSource::Local(LocalSlot::new(0)),
                )],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let increment = Function {
        name: "factory>increment".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::String("count".to_owned()), Constant::Double(1.0)],
        instructions: vec![
            instruction(InstructionKind::LoadCapture {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::StoreCapture {
                name: ConstantId::new(0),
                src: Register::new(2),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };

    assert_eq!(
        Interpreter::new(module(vec![entry, factory, increment]))
            .expect("module should verify")
            .execute_entry(&[])
            .expect("nested closure should execute after its parent returns"),
        vec![Value::Double(1.0), Value::Double(2.0)]
    );
}

#[test]
fn persists_workspace_store_and_load() {
    let function = Function {
        name: "workspace".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("answer".to_owned()),
            Constant::Double(42.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(module(vec![function])).expect("module should verify");
    let result = interpreter
        .execute_entry(&[])
        .expect("workspace operations should execute");
    assert_eq!(result, vec![Value::Double(42.0)]);
    assert_eq!(
        interpreter.workspace().get("answer"),
        Some(&Value::Double(42.0))
    );
}

#[test]
fn rejects_invalid_bytecode_before_execution() {
    let function = Function {
        name: "invalid".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(1.0)],
        instructions: vec![instruction(InstructionKind::LoadConstant {
            dst: Register::new(1),
            constant: ConstantId::new(0),
        })],
        exception_handlers: Vec::new(),
    };
    let Err(error) = Interpreter::new(module(vec![function])) else {
        panic!("invalid module must be rejected");
    };
    assert!(matches!(
        error.verification.kind,
        openmat_bytecode::VerificationErrorKind::InvalidRegister { .. }
    ));
}

#[test]
fn runtime_error_preserves_source_and_stack() {
    let location = SourceLocation::new(9, 20, 27);
    let function = Function {
        name: "missing_global".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::String("missing".to_owned())],
        instructions: vec![Instruction::located(
            InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            },
            location,
        )],
        exception_handlers: Vec::new(),
    };
    let error = Interpreter::new(module(vec![function]))
        .expect("module should verify")
        .execute_entry(&[])
        .expect_err("undefined global must fail");
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::UndefinedGlobal { ref name } if name == "missing"
    ));
    assert_eq!(error.location, Some(location));
    assert_eq!(error.stack.len(), 1);
    assert_eq!(error.stack[0].name, "missing_global");
    assert_eq!(error.stack[0].location, Some(location));
}

#[test]
fn cooperatively_cancels_running_loop() {
    let function = Function {
        name: "forever".to_owned(),
        register_count: 0,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![instruction(InstructionKind::Jump {
            target: InstructionIndex::new(0),
        })],
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(module(vec![function])).expect("module should verify");
    let cancellation = interpreter.cancellation_token();
    let execution = std::thread::spawn(move || interpreter.execute_entry(&[]));
    cancellation.cancel();
    let error = execution
        .join()
        .expect("interpreter thread should not panic")
        .expect_err("cancelled loop must fail");
    assert_eq!(error.kind, RuntimeErrorKind::Cancelled);
}

#[test]
fn arrays_share_storage_across_calls_workspace_and_returns_then_detach() {
    let caller = Function {
        name: "caller".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![
            Constant::Function(FunctionId::new(1)),
            Constant::String("saved".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(2)],
                callee: Register::new(0),
                arguments: vec![Register::new(1)],
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(1),
                src: Register::new(2),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(3),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let identity = Function {
        name: "identity".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let original = Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![1.0, 2.0, 3.0, 4.0]).unwrap(),
    ));
    let mut interpreter =
        Interpreter::new(module(vec![caller, identity])).expect("module should verify");
    let mut returned = interpreter
        .execute_entry(std::slice::from_ref(&original))
        .expect("array identity call should execute");

    let saved = interpreter.workspace().get("saved").unwrap();
    assert!(original.shares_array_storage_with(saved));
    assert!(original.shares_array_storage_with(&returned[0]));

    {
        let Some(ArrayData::F64(returned_array)) = returned[0].as_array_mut() else {
            panic!("returned value must be a real array");
        };
        *returned_array.get_mut_subscripts(&[1, 2]).unwrap() = 30.0;
    }
    assert!(!original.shares_array_storage_with(&returned[0]));
    assert!(original.shares_array_storage_with(interpreter.workspace().get("saved").unwrap()));

    let Some(ArrayData::F64(saved_array)) = interpreter
        .workspace_mut()
        .get_mut("saved")
        .and_then(Value::as_array_mut)
    else {
        panic!("workspace value must be a real array");
    };
    *saved_array.get_mut_linear(1).unwrap() = 10.0;
    assert!(!original.shares_array_storage_with(interpreter.workspace().get("saved").unwrap()));
    let Some(ArrayData::F64(original_array)) = original.as_array() else {
        panic!("original value must be a real array");
    };
    assert_eq!(original_array.as_slice(), &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn one_by_one_arrays_use_scalar_bytecode_semantics() {
    let function = Function {
        name: "scalar_arrays".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Multiply,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let scalar_array = |value| {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![value]).unwrap(),
        ))
    };
    let result = Interpreter::new(module(vec![function]))
        .expect("module should verify")
        .execute_entry(&[scalar_array(6.0), scalar_array(7.0)])
        .expect("one-by-one arrays should execute as scalars");
    assert_eq!(result, vec![Value::Double(42.0)]);
}

#[test]
fn non_scalar_arrays_use_scalar_expansion_for_addition() {
    let function = Function {
        name: "array_add".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let array = Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![1.0, 2.0]).unwrap(),
    ));
    let result = Interpreter::new(module(vec![function]))
        .expect("module should verify")
        .execute_entry(&[array, Value::Double(1.0)])
        .expect("array addition should use scalar expansion");
    let Value::Array(ArrayData::F64(result)) = &result[0] else {
        panic!("array addition should return a real array");
    };
    assert_eq!(result.shape().dimensions(), &[1, 2]);
    assert_eq!(result.as_slice(), &[2.0, 3.0]);
}

#[test]
fn conditional_branches_use_matlab_dense_array_truth_semantics() {
    let function = Function {
        name: "array_condition".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![Constant::Double(1.0), Constant::Double(2.0)],
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::JumpIfFalse {
                condition: Register::new(0),
                target: InstructionIndex::new(4),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Jump {
                target: InstructionIndex::new(5),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(module(vec![function])).expect("module should verify");
    let real = |dimensions, values| {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    };
    let complex = |values| {
        Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(Shape::new([1, 2]).unwrap(), values).unwrap(),
        ))
    };
    let logical = |values: Vec<bool>| {
        Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                values.into_iter().map(Logical::from).collect(),
            )
            .unwrap(),
        ))
    };

    let cases = [
        (real([1, 2], vec![1.0, 2.0]), 1.0),
        (real([1, 2], vec![1.0, 0.0]), 2.0),
        (real([0, 0], Vec::new()), 2.0),
        (
            complex(vec![
                ArrayComplex64::new(0.0, 1.0),
                ArrayComplex64::new(2.0, 0.0),
            ]),
            1.0,
        ),
        (
            complex(vec![ArrayComplex64::new(0.0, 1.0), ArrayComplex64::ZERO]),
            2.0,
        ),
        (logical(vec![true, true]), 1.0),
        (logical(vec![true, false]), 2.0),
    ];
    for (condition, expected) in cases {
        assert_eq!(
            interpreter.execute_entry(&[condition]).unwrap(),
            vec![Value::Double(expected)]
        );
    }
}

#[test]
fn short_circuit_jumps_do_not_read_undefined_rhs_globals() {
    let short_and = Function {
        name: "short_and".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Logical(false),
            Constant::String("undefined_rhs".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::JumpIfFalse {
                condition: Register::new(0),
                target: InstructionIndex::new(3),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let short_or = Function {
        name: "short_or".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Logical(true),
            Constant::String("undefined_rhs".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::JumpIfFalse {
                condition: Register::new(0),
                target: InstructionIndex::new(3),
            }),
            instruction(InstructionKind::Jump {
                target: InstructionIndex::new(4),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        exception_handlers: Vec::new(),
    };

    let and_result = Interpreter::new(module(vec![short_and]))
        .expect("short-and module should verify")
        .execute_entry(&[])
        .expect("false lhs must skip undefined rhs");
    assert_eq!(and_result, vec![Value::Logical(false)]);
    let or_result = Interpreter::new(module(vec![short_or]))
        .expect("short-or module should verify")
        .execute_entry(&[])
        .expect("true lhs must skip undefined rhs");
    assert_eq!(or_result, vec![Value::Logical(true)]);
}

#[test]
fn call_frames_expose_input_output_counts_and_clear_only_named_globals() {
    let entry = Function {
        name: "entry".to_owned(),
        register_count: 6,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Function(FunctionId::new(1)),
            Constant::Double(4.0),
            Constant::Double(5.0),
            Constant::String("first".to_owned()),
            Constant::String("second".to_owned()),
            Constant::String("openmat_result".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(3)],
                callee: Register::new(0),
                arguments: vec![Register::new(1), Register::new(2)],
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(4), Register::new(5)],
                callee: Register::new(0),
                arguments: vec![Register::new(1), Register::new(2)],
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(3),
                src: Register::new(1),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(4),
                src: Register::new(2),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(5),
                src: Register::new(3),
            }),
            instruction(InstructionKind::ClearGlobal {
                names: vec![ConstantId::new(3), ConstantId::new(4)],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3), Register::new(4), Register::new(5)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let metadata = Function {
        name: "metadata".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadCallOutputCount {
                dst: Register::new(0),
            }),
            instruction(InstructionKind::LoadCallInputCount {
                dst: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0), Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(module(vec![entry, metadata])).expect("valid module");

    assert_eq!(
        interpreter.execute_entry(&[]).expect("metadata execution"),
        vec![Value::Double(1.0), Value::Double(2.0), Value::Double(2.0)]
    );
    assert_eq!(
        interpreter.workspace().get("openmat_result"),
        Some(&Value::Double(1.0))
    );
    assert!(interpreter.workspace().get("first").is_none());
    assert!(interpreter.workspace().get("second").is_none());
}

#[test]
fn bytecode_function_handles_survive_module_replacement_through_workspace() {
    let save_handle = Function {
        name: "save_handle".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Function(FunctionId::new(1)),
            Constant::String("saved_handle".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(1),
                src: Register::new(0),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let metadata = Function {
        name: "metadata".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadCallInputCount {
                dst: Register::new(0),
            }),
            instruction(InstructionKind::LoadCallOutputCount {
                dst: Register::new(1),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0), Register::new(1), Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(module(vec![save_handle, metadata]))
        .expect("initial module should verify");
    interpreter
        .execute_entry(&[])
        .expect("handle should be stored in the workspace");

    let invoke_saved = Function {
        name: "invoke_saved".to_owned(),
        register_count: 5,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("saved_handle".to_owned()),
            Constant::Double(42.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2), Register::new(3), Register::new(4)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2), Register::new(3), Register::new(4)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    interpreter
        .replace_module(module(vec![invoke_saved]))
        .expect("replacement module should verify");
    assert_eq!(
        interpreter
            .execute_entry(&[])
            .expect("workspace handle should retain its source module"),
        vec![Value::Double(1.0), Value::Double(3.0), Value::Double(42.0)]
    );
}

#[test]
fn named_builtin_handles_ignore_workspace_values_and_preserve_call_metadata() {
    let mut registry = BuiltinRegistry::new();
    registry
        .register(
            "registered_target",
            |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                let requested_outputs = u32::try_from(context.requested_outputs())
                    .expect("test output count should fit u32");
                Ok(vec![
                    arguments[0].clone(),
                    Value::Double(f64::from(requested_outputs)),
                ])
            },
        )
        .expect("test built-in registration");
    let function = Function {
        name: "builtin_handle".to_owned(),
        register_count: 5,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("registered_target".to_owned()),
            Constant::String("saved_handle".to_owned()),
            Constant::Double(17.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(1),
                src: Register::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(3), Register::new(4)],
                target: Register::new(1),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3), Register::new(4)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut interpreter =
        Interpreter::with_registry(module(vec![function]), registry).expect("module should verify");
    interpreter
        .workspace_mut()
        .insert("registered_target".to_owned(), Value::Double(99.0));
    assert_eq!(
        interpreter
            .execute_entry(&[])
            .expect("built-in handle call"),
        vec![Value::Double(17.0), Value::Double(2.0)]
    );
    assert_eq!(
        interpreter.workspace().get("registered_target"),
        Some(&Value::Double(99.0))
    );
}

struct SharedTestOutput(Arc<Mutex<Vec<OutputEvent>>>);

impl OutputSink for SharedTestOutput {
    fn emit(&mut self, event: OutputEvent) -> Result<(), OutputError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn builtin_context_can_reenter_language_and_nested_builtin_services() {
    let mut registry = BuiltinRegistry::new();
    registry
        .register(
            "callback_driver",
            |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                let requested_outputs = context.requested_outputs();
                context.invoke(&arguments[0], &arguments[1..], requested_outputs)
            },
        )
        .unwrap();
    registry
        .register(
            "nested_runtime_builtin",
            |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                let _ = context.random_uniform()?;
                context.set_display_format(openmat_runtime::DisplayFormat {
                    numeric: NumericFormat::Long,
                    line_spacing: LineSpacing::Compact,
                })?;
                context.emit(OutputEvent::CommandText("nested callback\n".to_owned()))?;
                Ok(vec![arguments[0].clone()])
            },
        )
        .unwrap();

    let entry = Function {
        name: "callback_entry".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("callback_target".to_owned()),
            Constant::String("callback_driver".to_owned()),
            Constant::Double(42.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadFunctionHandle {
                dst: Register::new(1),
                name: ConstantId::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(3)],
                target: Register::new(1),
                arguments: vec![
                    ApplyArgument::Value(Register::new(0)),
                    ApplyArgument::Value(Register::new(2)),
                ],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let callback = Function {
        name: "callback_target".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![Constant::String("nested_runtime_builtin".to_owned())],
        instructions: vec![
            instruction(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = SharedTestOutput(Arc::clone(&events));
    let mut interpreter =
        Interpreter::with_components(module(vec![entry, callback]), registry, Box::new(output))
            .unwrap();
    let result = interpreter
        .execute_entry(&[])
        .expect("the callback should complete without a service deadlock");
    assert_eq!(result, [Value::Double(42.0)]);
    assert_eq!(interpreter.display_format().numeric, NumericFormat::Long);
    assert_eq!(
        interpreter.display_format().line_spacing,
        LineSpacing::Compact
    );
    assert_eq!(
        *events.lock().unwrap(),
        [OutputEvent::CommandText("nested callback\n".to_owned())]
    );
}

struct OwnedArrayMutation {
    reused_storage: Arc<AtomicBool>,
}

impl BuiltinFunction for OwnedArrayMutation {
    fn call(&self, arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
        self.call_owned(arguments.to_vec(), context)
    }

    fn call_owned(
        &self,
        mut arguments: Vec<Value>,
        _context: &mut BuiltinContext<'_>,
    ) -> BuiltinResult {
        let Value::Array(ArrayData::F64(array)) = &mut arguments[0] else {
            panic!("expected f64 array")
        };
        let before = array.as_slice().as_ptr();
        array.as_mut_slice()[0] = 42.0;
        self.reused_storage
            .store(before == array.as_slice().as_ptr(), Ordering::Relaxed);
        Ok(arguments)
    }
}

#[test]
fn builtin_apply_transfers_a_temporary_array_without_detaching_storage() {
    let reused_storage = Arc::new(AtomicBool::new(false));
    let mut registry = BuiltinRegistry::new();
    registry
        .register(
            "make_temporary",
            |_arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                Ok(vec![Value::Array(ArrayData::F64(
                    DenseArray::from_vec(Shape::new([2, 1]).unwrap(), vec![1.0, 2.0]).unwrap(),
                ))])
            },
        )
        .unwrap();
    registry
        .register(
            "mutate_owned",
            OwnedArrayMutation {
                reused_storage: Arc::clone(&reused_storage),
            },
        )
        .unwrap();

    let function = Function {
        name: "owned_builtin_arguments".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("make_temporary".to_owned()),
            Constant::String("mutate_owned".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(1)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::LoadFunctionHandle {
                dst: Register::new(2),
                name: ConstantId::new(1),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(3)],
                target: Register::new(2),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let result = Interpreter::with_registry(module(vec![function]), registry)
        .unwrap()
        .execute_entry(&[])
        .unwrap();
    assert!(reused_storage.load(Ordering::Relaxed));
    let Value::Array(ArrayData::F64(array)) = &result[0] else {
        panic!("expected f64 array")
    };
    assert_eq!(array.as_slice(), &[42.0, 2.0]);
}

#[test]
fn unknown_named_handle_and_handle_call_cancellation_are_structured_and_located() {
    let unknown_location = SourceLocation::new(31, 4, 19);
    let unknown_call_location = SourceLocation::new(31, 20, 35);
    let unknown = Function {
        name: "unknown_handle".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::String("absent_target".to_owned())],
        instructions: vec![
            Instruction::located(
                InstructionKind::LoadFunctionHandle {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                },
                unknown_location,
            ),
            Instruction::located(
                InstructionKind::Apply {
                    outputs: Vec::new(),
                    target: Register::new(0),
                    arguments: Vec::new(),
                },
                unknown_call_location,
            ),
        ],
        exception_handlers: Vec::new(),
    };
    let error = Interpreter::new(module(vec![unknown]))
        .expect("unknown-target module should verify")
        .execute_entry(&[])
        .expect_err("unknown handle invocation must fail lazily");
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::UnknownFunctionHandleTarget { ref name }
            if name == "absent_target"
    ));
    assert_eq!(error.location, Some(unknown_call_location));
    assert_eq!(error.stack[0].location, Some(unknown_call_location));

    let mut registry = BuiltinRegistry::new();
    registry
        .register(
            "cancel_target",
            |_arguments: &[Value], context: &mut BuiltinContext<'_>| {
                context.cancellation().cancel();
                Ok(Vec::new())
            },
        )
        .expect("test cancellation built-in");
    let call_location = SourceLocation::new(32, 20, 35);
    let cancellation = Function {
        name: "cancel_handle".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::String("cancel_target".to_owned())],
        instructions: vec![
            instruction(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            Instruction::located(
                InstructionKind::Apply {
                    outputs: Vec::new(),
                    target: Register::new(0),
                    arguments: Vec::new(),
                },
                call_location,
            ),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let cancellation_error = Interpreter::with_registry(module(vec![cancellation]), registry)
        .expect("cancellation module should verify")
        .execute_entry(&[])
        .expect_err("handle call must observe cancellation");
    assert_eq!(cancellation_error.kind, RuntimeErrorKind::Cancelled);
    assert_eq!(cancellation_error.location, Some(call_location));
}

fn closure_creation_module() -> BytecodeModule {
    let create = Function {
        name: "create_closure".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Double(10.0),
            Constant::String("captured".to_owned()),
            Constant::String("saved_closure".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            Instruction::located(
                InstructionKind::MakeClosure {
                    dst: Register::new(1),
                    function: FunctionId::new(1),
                    captures: vec![(ConstantId::new(1), Some(Register::new(0)), true)],
                },
                SourceLocation::new(40, 5, 24),
            ),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(2),
                src: Register::new(1),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let closure = Function {
        name: "<anonymous>".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![Constant::String("captured".to_owned())],
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    module(vec![create, closure])
}

fn invoke_saved_closure_module() -> BytecodeModule {
    let invoke = Function {
        name: "invoke_saved_closure".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("saved_closure".to_owned()),
            Constant::Double(2.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    module(vec![invoke])
}

fn stale_closure_dispatch_module() -> BytecodeModule {
    let dispatch_stale = Function {
        name: "dispatch_stale".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(1)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    module(vec![dispatch_stale])
}

#[test]
fn closure_captures_by_value_and_retains_owning_module_across_replacement() {
    let mut interpreter =
        Interpreter::new(closure_creation_module()).expect("closure module should verify");
    interpreter
        .execute_entry(&[])
        .expect("closure construction should execute");

    interpreter
        .replace_module(invoke_saved_closure_module())
        .expect("replacement module should verify");
    interpreter
        .workspace_mut()
        .insert("captured".to_owned(), Value::Double(100.0));
    assert_eq!(
        interpreter
            .execute_entry(&[])
            .expect("saved closure should retain code and capture"),
        vec![Value::Double(12.0)]
    );

    let stale = interpreter
        .workspace()
        .get("saved_closure")
        .expect("saved closure")
        .clone();
    interpreter.clear_session();
    interpreter
        .replace_module(stale_closure_dispatch_module())
        .expect("post-reset dispatch module should verify");
    assert!(matches!(
        interpreter
            .execute_entry(&[stale])
            .expect_err("reset must discard retained closure code")
            .kind,
        RuntimeErrorKind::InvalidFunctionHandle { .. }
    ));
}

#[test]
fn session_clear_closes_owned_file_streams() {
    let function = Function {
        name: "clear_files".to_owned(),
        register_count: 0,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![instruction(InstructionKind::Return { values: Vec::new() })],
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(module(vec![function])).unwrap();
    let path = std::env::temp_dir().join(format!(
        "openmat-clear-stream-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut files = LocalFileSystem::new(std::env::temp_dir()).unwrap();
    files
        .open_file(
            &path.to_string_lossy(),
            FileOpenMode {
                access: FileOpenAccess::Write,
                text: false,
            },
            "ieee-le",
            "UTF-8",
        )
        .unwrap();
    interpreter.set_file_system_service(Box::new(files));

    interpreter.clear_session();
    std::fs::remove_file(path).expect("session clear must release the operating-system handle");
}

#[test]
#[allow(clippy::too_many_lines)]
fn registered_handle_ids_do_not_conflict_with_legacy_direct_function_indices() {
    let dispatch = Function {
        name: "dispatch".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::MakeClosure {
                dst: Register::new(0),
                function: FunctionId::new(2),
                captures: Vec::new(),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(1),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let direct = Function {
        name: "legacy_direct".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(77.0)],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let closure = Function {
        name: "registered_closure".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(88.0)],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let direct_handle = Value::Function(FunctionHandle::Bytecode(BytecodeFunctionHandle::new(1)));
    let result = Interpreter::new(module(vec![dispatch, direct, closure]))
        .expect("dispatch module should verify")
        .execute_entry(&[direct_handle])
        .expect("legacy direct handle should not collide with registered closure");
    assert_eq!(result, vec![Value::Double(77.0)]);

    let missing_registered = Value::Function(FunctionHandle::Bytecode(
        BytecodeFunctionHandle::new((1 << 31) + 7),
    ));
    let mut interpreter = Interpreter::new(module(vec![Function {
        name: "missing_registered".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(1)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    }]))
    .expect("missing-handle module should verify");
    let error = interpreter
        .execute_entry(&[missing_registered])
        .expect_err("unregistered tagged handle must not use the direct-index fallback");
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InvalidFunctionHandle {
            handle: FunctionHandle::Bytecode(handle)
        } if handle == BytecodeFunctionHandle::new((1 << 31) + 7)
    ));
}

#[test]
fn closure_tail_apply_forwards_dynamic_requested_output_count() {
    let entry = Function {
        name: "tail_apply_entry".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::MakeClosure {
                dst: Register::new(0),
                function: FunctionId::new(1),
                captures: Vec::new(),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(1), Register::new(2)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(3)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(2), Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let closure = Function {
        name: "<anonymous-tail>".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Function(FunctionId::new(2))],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::ReturnApply {
                target: Register::new(0),
                arguments: Vec::new(),
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let pair = Function {
        name: "pair".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(4.0), Constant::Double(5.0)],
        instructions: vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0), Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };

    assert_eq!(
        Interpreter::new(module(vec![entry, closure, pair]))
            .expect("tail-apply module should verify")
            .execute_entry(&[])
            .expect("tail apply should forward two and one requested outputs"),
        vec![Value::Double(4.0), Value::Double(5.0), Value::Double(4.0)]
    );
}

#[test]
fn bare_value_context_invokes_every_unshadowed_builtin_and_handle_context_does_not() {
    let entry = Function {
        name: "bare_callable".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("missing".to_owned()),
            Constant::String("ordinary".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: true,
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(1),
                construct_if_class: true,
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(2),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(3),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Return {
                values: vec![
                    Register::new(0),
                    Register::new(1),
                    Register::new(2),
                    Register::new(3),
                ],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let build_registry = || {
        let mut registry = BuiltinRegistry::new();
        registry
            .register_bare_callable(
                "missing",
                |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                    assert!(arguments.is_empty());
                    assert_eq!(context.requested_outputs(), 1);
                    Ok(vec![Value::String(StringValue::missing())])
                },
            )
            .unwrap();
        registry
            .register(
                "ordinary",
                |_arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                    Ok(vec![Value::Double(1.0)])
                },
            )
            .unwrap();
        registry
    };
    let registry = build_registry();
    let missing_handle = registry.handle_by_name("missing").unwrap();
    let ordinary_handle = registry.handle_by_name("ordinary").unwrap();
    let bytecode = module(vec![entry]);
    let mut interpreter = Interpreter::with_registry(bytecode.clone(), registry).unwrap();
    let values = interpreter.execute_entry(&[]).unwrap();
    assert!(values[0].as_string_scalar().unwrap().is_missing());
    assert_eq!(values[1], Value::Double(1.0));
    assert_eq!(
        values[2],
        Value::Function(FunctionHandle::Builtin(missing_handle))
    );
    assert_eq!(
        values[3],
        Value::Function(FunctionHandle::Builtin(ordinary_handle))
    );
    assert_ne!(missing_handle, ordinary_handle);

    let mut shadowed = Interpreter::with_registry(bytecode, build_registry()).unwrap();
    shadowed
        .workspace_mut()
        .insert("missing", Value::Double(9.0));
    let values = shadowed.execute_entry(&[]).unwrap();
    assert_eq!(values[0], Value::Double(9.0));
}

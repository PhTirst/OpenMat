use openmat_array::{ArrayData, DenseArray, Shape};
use openmat_bytecode::{
    Access, BytecodeModule, ClassDefinition, ClassDefinitionId, ClassSemantics, Constant,
    ConstantId, ExceptionHandler, Function, FunctionId, Instruction, InstructionIndex,
    InstructionKind, LocalSlot, MethodDefinition, MethodKind, PersistentSlot, Register,
    SourceLocation, VerificationErrorKind,
};
use openmat_runtime::{Interpreter, RuntimeErrorKind};
use openmat_value::Value;

const LOCATION: SourceLocation = SourceLocation::new(71, 10, 24);

fn instruction(kind: InstructionKind) -> Instruction {
    Instruction::new(kind)
}

fn pc(index: u32) -> InstructionIndex {
    InstructionIndex::new(index)
}

fn function(
    name: &str,
    registers: u32,
    locals: u32,
    parameters: u32,
    persistent_slots: u32,
    constants: Vec<Constant>,
    instructions: Vec<Instruction>,
) -> Function {
    let mut function = Function::new(name, registers, locals, parameters)
        .with_persistent_slot_count(persistent_slots);
    function.constants = constants;
    function.instructions = instructions;
    function
}

fn module(functions: Vec<Function>) -> BytecodeModule {
    BytecodeModule::new(functions, FunctionId::new(0))
}

fn persistent_counter(name: &str) -> Function {
    function(
        name,
        3,
        0,
        0,
        1,
        vec![Constant::Double(0.0), Constant::Double(1.0)],
        vec![
            instruction(InstructionKind::DeclarePersistent {
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::LoadPersistent {
                dst: Register::new(0),
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::JumpIfFalse {
                condition: Register::new(0),
                target: pc(4),
            }),
            instruction(InstructionKind::Jump { target: pc(5) }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: openmat_bytecode::BinaryOperator::Add,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(0),
                src: Register::new(2),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    )
}

fn persistent_loader(name: &str) -> Function {
    function(
        name,
        1,
        0,
        0,
        1,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadPersistent {
                dst: Register::new(0),
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
    )
}

#[test]
fn global_declaration_preserves_existing_value_and_shares_across_functions() {
    let entry = function(
        "global_entry",
        4,
        0,
        0,
        0,
        vec![
            Constant::String("shared".to_owned()),
            Constant::Function(FunctionId::new(1)),
            Constant::Double(42.0),
        ],
        vec![
            instruction(InstructionKind::DeclareGlobal {
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(0),
                src: Register::new(2),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(3)],
                callee: Register::new(1),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0), Register::new(3)],
            }),
        ],
    );
    let reader = function(
        "global_reader",
        1,
        0,
        0,
        0,
        vec![Constant::String("shared".to_owned())],
        vec![
            instruction(InstructionKind::DeclareGlobal {
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
    );
    let bytecode = module(vec![entry, reader]);

    let mut existing = Interpreter::new(bytecode.clone()).unwrap();
    existing
        .workspace_mut()
        .insert("shared", Value::Double(7.0));
    assert_eq!(
        existing.execute_entry(&[]).unwrap(),
        vec![Value::Double(7.0), Value::Double(42.0)]
    );
    assert_eq!(
        existing.workspace().get("shared"),
        Some(&Value::Double(42.0))
    );

    let mut absent = Interpreter::new(bytecode).unwrap();
    assert_eq!(
        absent.execute_entry(&[]).unwrap(),
        vec![Value::empty_double(), Value::Double(42.0)]
    );
}

#[test]
fn persistent_repeated_calls_and_same_named_replacement_retain_state() {
    let mut interpreter = Interpreter::new(module(vec![persistent_counter("stable_counter")]))
        .expect("counter module should verify");
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(1.0)]
    );
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(2.0)]
    );
    assert_eq!(interpreter.persistent_binding_count(), 1);

    interpreter
        .replace_module(module(vec![persistent_counter("stable_counter")]))
        .unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(3.0)]
    );
    assert_eq!(interpreter.persistent_binding_count(), 1);
}

#[test]
fn persistent_slots_are_isolated_by_function_identity() {
    let entry = function(
        "function_isolation_entry",
        5,
        0,
        0,
        0,
        vec![
            Constant::Function(FunctionId::new(1)),
            Constant::Function(FunctionId::new(2)),
        ],
        vec![
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(1)],
                callee: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(3)],
                callee: Register::new(2),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Call {
                outputs: vec![Register::new(4)],
                callee: Register::new(0),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(3), Register::new(4)],
            }),
        ],
    );
    let mut interpreter = Interpreter::new(module(vec![
        entry,
        persistent_counter("first_counter"),
        persistent_counter("second_counter"),
    ]))
    .unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(1.0), Value::Double(1.0), Value::Double(2.0)]
    );
    assert_eq!(interpreter.persistent_binding_count(), 2);
}

fn same_named_method_module() -> BytecodeModule {
    let entry = function(
        "method_isolation_entry",
        5,
        0,
        0,
        0,
        vec![
            Constant::String("FirstPersistentClass".to_owned()),
            Constant::String("SecondPersistentClass".to_owned()),
            Constant::String("next".to_owned()),
        ],
        vec![
            instruction(InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(0),
            }),
            instruction(InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(1),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::ApplyField {
                outputs: vec![Register::new(1)],
                object: Register::new(0),
                name: ConstantId::new(2),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(2),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::ApplyField {
                outputs: vec![Register::new(3)],
                object: Register::new(2),
                name: ConstantId::new(2),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::ApplyField {
                outputs: vec![Register::new(4)],
                object: Register::new(0),
                name: ConstantId::new(2),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(3), Register::new(4)],
            }),
        ],
    );
    let method = |function| MethodDefinition {
        name: "next".to_owned(),
        function,
        kind: MethodKind::Static,
        access: Access::Public,
        is_abstract: false,
        is_external: false,
        constructor_output: None,
        location: LOCATION,
    };
    let class = |name: &str, method_definition| ClassDefinition {
        name: name.to_owned(),
        semantics: ClassSemantics::Value,
        superclass: None,
        declared_abstract: None,
        sealed: false,
        properties: Vec::new(),
        methods: vec![method(method_definition)],
        location: LOCATION,
    };
    module(vec![
        entry,
        persistent_counter("same_method_diagnostic_name"),
        persistent_counter("same_method_diagnostic_name"),
    ])
    .with_classes(vec![
        class("FirstPersistentClass", FunctionId::new(1)),
        class("SecondPersistentClass", FunctionId::new(2)),
    ])
}

#[test]
fn same_named_methods_are_isolated_by_caller_class() {
    let mut interpreter = Interpreter::new(same_named_method_module()).unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(1.0), Value::Double(1.0), Value::Double(2.0)]
    );
    assert_eq!(interpreter.persistent_binding_count(), 2);
}

#[test]
fn conditional_declaration_and_absent_valid_slots_create_canonical_storage() {
    let conditional = function(
        "conditional_persistent",
        4,
        1,
        1,
        2,
        vec![Constant::Double(11.0)],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::JumpIfFalse {
                condition: Register::new(0),
                target: pc(3),
            }),
            instruction(InstructionKind::DeclarePersistent {
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::LoadPersistent {
                dst: Register::new(1),
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(1),
                src: Register::new(2),
            }),
            instruction(InstructionKind::LoadPersistent {
                dst: Register::new(3),
                slot: PersistentSlot::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(3)],
            }),
        ],
    );
    let mut interpreter = Interpreter::new(module(vec![conditional])).unwrap();
    assert_eq!(
        interpreter.execute_entry(&[Value::Logical(false)]).unwrap(),
        vec![Value::empty_double(), Value::Double(11.0)]
    );
    assert_eq!(interpreter.persistent_binding_count(), 2);
}

#[test]
fn persistent_array_store_and_load_preserve_cow_semantics() {
    let store = function(
        "persistent_array",
        2,
        1,
        1,
        1,
        Vec::new(),
        vec![
            instruction(InstructionKind::DeclarePersistent {
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::LoadPersistent {
                dst: Register::new(1),
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
    );
    let original = Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![3.0, 4.0]).unwrap(),
    ));
    let mut interpreter = Interpreter::new(module(vec![store])).unwrap();
    let mut first = interpreter
        .execute_entry(std::slice::from_ref(&original))
        .unwrap();
    assert!(first[0].shares_array_storage_with(&original));

    let Some(ArrayData::F64(returned)) = first[0].as_array_mut() else {
        panic!("persistent array load should return a real array");
    };
    *returned.get_mut_linear(1).unwrap() = 30.0;
    assert!(!first[0].shares_array_storage_with(&original));

    interpreter
        .replace_module(module(vec![persistent_loader("persistent_array")]))
        .unwrap();
    let loaded = interpreter.execute_entry(&[]).unwrap();
    assert!(loaded[0].shares_array_storage_with(&original));
    let Some(ArrayData::F64(loaded)) = loaded[0].as_array() else {
        panic!("stored persistent value should remain a real array");
    };
    assert_eq!(loaded.as_slice(), &[3.0, 4.0]);
}

fn persistent_object_module(name: &str, semantics: ClassSemantics) -> BytecodeModule {
    let entry = function(
        &format!("{name}_persistent_entry"),
        2,
        0,
        0,
        1,
        vec![Constant::String(name.to_owned())],
        vec![
            instruction(InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: true,
            }),
            instruction(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::LoadPersistent {
                dst: Register::new(1),
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0), Register::new(1)],
            }),
        ],
    );
    module(vec![entry]).with_classes(vec![ClassDefinition {
        name: name.to_owned(),
        semantics,
        superclass: None,
        declared_abstract: None,
        sealed: false,
        properties: Vec::new(),
        methods: Vec::new(),
        location: LOCATION,
    }])
}

#[test]
fn persistent_store_uses_value_and_handle_object_assignment_semantics_and_clear_resets_session() {
    let mut values = Interpreter::new(persistent_object_module(
        "PersistentValue",
        ClassSemantics::Value,
    ))
    .unwrap();
    let returned = values.execute_entry(&[]).unwrap();
    let (Value::Object(original), Value::Object(stored)) = (&returned[0], &returned[1]) else {
        panic!("value-class persistent test should return objects");
    };
    assert_ne!(original, stored);
    assert_eq!(values.object_count(), 2);
    assert_eq!(values.class_count(), 2);
    assert_eq!(values.persistent_binding_count(), 1);
    values
        .workspace_mut()
        .insert("temporary", Value::Double(1.0));
    values.clear_session();
    assert!(values.workspace().is_empty());
    assert_eq!(values.persistent_binding_count(), 0);
    assert_eq!(values.class_count(), 1);
    assert_eq!(values.object_count(), 0);

    let mut handles = Interpreter::new(persistent_object_module(
        "PersistentHandle",
        ClassSemantics::Handle,
    ))
    .unwrap();
    let returned = handles.execute_entry(&[]).unwrap();
    let (Value::Object(original), Value::Object(stored)) = (&returned[0], &returned[1]) else {
        panic!("handle-class persistent test should return objects");
    };
    assert_eq!(original, stored);
    assert_eq!(handles.object_count(), 1);
}

#[test]
#[allow(clippy::too_many_lines)]
fn completed_persistent_writes_survive_catch_and_uncaught_unwind_with_location() {
    let caught = function(
        "caught_persistent_state",
        2,
        0,
        0,
        1,
        vec![
            Constant::Double(7.0),
            Constant::String("missing_after_caught_store".to_owned()),
        ],
        vec![
            instruction(InstructionKind::DeclarePersistent {
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Jump { target: pc(7) }),
            instruction(InstructionKind::LoadPersistent {
                dst: Register::new(0),
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::Jump { target: pc(7) }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
    )
    .with_exception_handlers(vec![ExceptionHandler::catch(
        pc(0),
        pc(4),
        pc(5),
        pc(7),
        None,
    )]);
    let mut interpreter = Interpreter::new(module(vec![caught])).unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(7.0)]
    );

    let unwind_location = SourceLocation::new(72, 30, 46);
    let unhandled = function(
        "unhandled_persistent_state",
        2,
        0,
        0,
        1,
        vec![
            Constant::Double(9.0),
            Constant::String("missing_after_unhandled_store".to_owned()),
        ],
        vec![
            instruction(InstructionKind::DeclarePersistent {
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(0),
                src: Register::new(0),
            }),
            Instruction::located(
                InstructionKind::LoadGlobal {
                    dst: Register::new(1),
                    name: ConstantId::new(1),
                    construct_if_class: false,
                },
                unwind_location,
            ),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
    );
    interpreter.replace_module(module(vec![unhandled])).unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::UndefinedGlobal { .. }
    ));
    assert_eq!(error.location, Some(unwind_location));
    assert_eq!(error.stack.len(), 1);
    assert_eq!(error.stack[0].name, "unhandled_persistent_state");
    assert_eq!(error.stack[0].location, Some(unwind_location));

    interpreter
        .replace_module(module(vec![persistent_loader(
            "unhandled_persistent_state",
        )]))
        .unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(9.0)]
    );
}

#[test]
fn pre_cancelled_declarations_do_not_create_storage_and_preserve_location_and_stack() {
    let cancelled = function(
        "cancelled_declarations",
        1,
        0,
        0,
        1,
        vec![
            Constant::String("cancelled_global".to_owned()),
            Constant::Double(5.0),
        ],
        vec![
            Instruction::located(
                InstructionKind::DeclareGlobal {
                    name: ConstantId::new(0),
                },
                LOCATION,
            ),
            instruction(InstructionKind::DeclarePersistent {
                slot: PersistentSlot::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(0),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(0),
                src: Register::new(0),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
    );
    let mut interpreter = Interpreter::new(module(vec![cancelled])).unwrap();
    interpreter.cancellation_token().cancel();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert_eq!(error.kind, RuntimeErrorKind::Cancelled);
    assert_eq!(error.location, Some(LOCATION));
    assert_eq!(error.stack.len(), 1);
    assert_eq!(error.stack[0].name, "cancelled_declarations");
    assert_eq!(error.stack[0].location, Some(LOCATION));
    assert!(interpreter.workspace().is_empty());
    assert_eq!(interpreter.persistent_binding_count(), 0);
}

#[test]
fn persistent_slot_out_of_bounds_is_rejected_by_verifier() {
    let invalid = function(
        "invalid_persistent_slot",
        1,
        0,
        0,
        1,
        Vec::new(),
        vec![instruction(InstructionKind::LoadPersistent {
            dst: Register::new(0),
            slot: PersistentSlot::new(1),
        })],
    );
    let Err(error) = Interpreter::new(module(vec![invalid])) else {
        panic!("slot must be rejected");
    };
    assert!(matches!(
        error.verification.kind,
        VerificationErrorKind::InvalidPersistentSlot { slot, count }
            if slot == PersistentSlot::new(1) && count == 1
    ));
}

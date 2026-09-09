use openmat_bytecode::{
    Access, ApplyArgument, BinaryOperator, BytecodeModule, ClassDefinition, ClassDefinitionId,
    ClassFeatureDefinition, ClassKind, ClassSemantics, Constant, ConstantId, EnumMemberDefinition,
    Function, FunctionId, Instruction, InstructionKind, LocalSlot, MethodDefinition, MethodKind,
    PropertyDefinition, PropertyKind, Register, SourceLocation,
};
use openmat_runtime::{Interpreter, RuntimeErrorKind};
use openmat_value::Value;

const LOCATION: SourceLocation = SourceLocation::new(1, 0, 1);

fn instruction(kind: InstructionKind) -> Instruction {
    Instruction::located(kind, LOCATION)
}

#[allow(clippy::too_many_lines)]
fn class_module(
    name: &str,
    semantics: ClassSemantics,
    failing_constructor: bool,
) -> BytecodeModule {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 0,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(0),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let default = Function {
        name: format!("{name}.<default:value>"),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(0.0)],
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
    let add = Function {
        name: format!("{name}.add"),
        register_count: 5,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: vec![Constant::String("value".to_owned())],
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(1),
                object: Register::new(0),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(3),
                lhs: Register::new(1),
                rhs: Register::new(2),
            }),
            instruction(InstructionKind::SetField {
                dst: Register::new(4),
                object: Register::new(0),
                name: ConstantId::new(0),
                value: Register::new(3),
            }),
            instruction(InstructionKind::StoreLocal {
                local: LocalSlot::new(0),
                src: Register::new(4),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut functions = vec![entry, default, add];
    let mut methods = vec![MethodDefinition {
        name: "add".to_owned(),
        function: FunctionId::new(2),
        kind: MethodKind::Instance,
        access: Access::Public,
        is_abstract: false,
        is_external: false,
        constructor_output: None,
        location: LOCATION,
    }];
    if failing_constructor {
        functions.push(Function {
            name: format!("{name}.{name}"),
            register_count: 2,
            pack_register_count: 0,
            local_count: 1,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![Constant::String("missing_during_construction".to_owned())],
            instructions: vec![
                instruction(InstructionKind::LoadGlobal {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                    construct_if_class: false,
                }),
                instruction(InstructionKind::LoadLocal {
                    dst: Register::new(1),
                    local: LocalSlot::new(0),
                }),
                instruction(InstructionKind::Return {
                    values: vec![Register::new(1)],
                }),
            ],
            exception_handlers: Vec::new(),
        });
        methods.push(MethodDefinition {
            name: name.to_owned(),
            function: FunctionId::new(3),
            kind: MethodKind::Instance,
            access: Access::Public,
            is_abstract: false,
            is_external: false,
            constructor_output: Some(LocalSlot::new(0)),
            location: LOCATION,
        });
    }
    BytecodeModule::new(functions, FunctionId::new(0)).with_classes(vec![ClassDefinition {
        name: name.to_owned(),
        semantics,
        superclass: None,
        declared_abstract: None,
        sealed: false,
        properties: vec![PropertyDefinition {
            name: "value".to_owned(),
            kind: PropertyKind::Stored,
            default: Some(FunctionId::new(1)),
            get_access: Some(Access::Public),
            set_access: Some(Access::Public),
            location: LOCATION,
        }],
        methods,
        location: LOCATION,
    }])
}

fn exercise_module(class_name: &str) -> BytecodeModule {
    let constants = vec![
        Constant::String(class_name.to_owned()),
        Constant::String("a".to_owned()),
        Constant::String("b".to_owned()),
        Constant::String("add".to_owned()),
        Constant::Double(4.0),
        Constant::String("value".to_owned()),
        Constant::String("a_value".to_owned()),
        Constant::String("b_value".to_owned()),
    ];
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 10,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants,
        instructions: vec![
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
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(1),
                src: Register::new(1),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(2),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(2),
                src: Register::new(2),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(4),
            }),
            instruction(InstructionKind::ApplyField {
                outputs: vec![Register::new(4)],
                object: Register::new(2),
                name: ConstantId::new(3),
                arguments: vec![ApplyArgument::Value(Register::new(3))],
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(1),
                src: Register::new(4),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(5),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(6),
                object: Register::new(5),
                name: ConstantId::new(5),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(6),
                src: Register::new(6),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(7),
                name: ConstantId::new(2),
                construct_if_class: false,
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(8),
                object: Register::new(7),
                name: ConstantId::new(5),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(7),
                src: Register::new(8),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![entry], FunctionId::new(0))
}

fn constructor_call_module(class_name: &str) -> BytecodeModule {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String(class_name.to_owned()),
            Constant::String("failed".to_owned()),
        ],
        instructions: vec![
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
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(1),
                src: Register::new(1),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![entry], FunctionId::new(0))
}

#[allow(clippy::too_many_lines)]
fn static_constant_module(name: &str) -> BytecodeModule {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 0,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(0),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let initializer = Function {
        name: format!("{name}.<constant:Factor>"),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(3.0)],
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
    let scale = Function {
        name: format!("{name}.scale"),
        register_count: 5,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![
            Constant::String(name.to_owned()),
            Constant::String("Factor".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(1),
                object: Register::new(0),
                name: ConstantId::new(1),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Multiply,
                dst: Register::new(3),
                lhs: Register::new(1),
                rhs: Register::new(2),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![entry, initializer, scale], FunctionId::new(0)).with_classes(vec![
        ClassDefinition {
            name: name.to_owned(),
            semantics: ClassSemantics::Value,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: vec![PropertyDefinition {
                name: "Factor".to_owned(),
                kind: PropertyKind::Constant,
                default: Some(FunctionId::new(1)),
                get_access: Some(Access::Public),
                set_access: None,
                location: LOCATION,
            }],
            methods: vec![MethodDefinition {
                name: "scale".to_owned(),
                function: FunctionId::new(2),
                kind: MethodKind::Static,
                access: Access::Public,
                is_abstract: false,
                is_external: false,
                constructor_output: None,
                location: LOCATION,
            }],
            location: LOCATION,
        },
    ])
}

fn static_constant_exercise_module(class_name: &str) -> BytecodeModule {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 5,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String(class_name.to_owned()),
            Constant::String("Factor".to_owned()),
            Constant::String("scale".to_owned()),
            Constant::Double(5.0),
            Constant::String("factor".to_owned()),
            Constant::String("scaled".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(1),
                object: Register::new(0),
                name: ConstantId::new(1),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(4),
                src: Register::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(3),
            }),
            instruction(InstructionKind::ApplyField {
                outputs: vec![Register::new(3)],
                object: Register::new(0),
                name: ConstantId::new(2),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(5),
                src: Register::new(3),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![entry], FunctionId::new(0))
}

fn constant_assignment_module(class_name: &str, construct: bool) -> BytecodeModule {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String(class_name.to_owned()),
            Constant::String("Factor".to_owned()),
            Constant::Double(9.0),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: construct,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::SetField {
                dst: Register::new(2),
                object: Register::new(0),
                name: ConstantId::new(1),
                value: Register::new(1),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![entry], FunctionId::new(0))
}

fn object_array_module(first_class: &str, second_class: &str) -> BytecodeModule {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 9,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String(first_class.to_owned()),
            Constant::String(second_class.to_owned()),
            Constant::Double(1.0),
            Constant::Double(2.0),
            Constant::Nothing,
            Constant::String("result".to_owned()),
        ],
        instructions: vec![
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
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(4),
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(4),
                target: Register::new(3),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
                value: Register::new(1),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(5),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(6)],
                target: Register::new(5),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(7),
                constant: ConstantId::new(3),
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(8),
                target: Register::new(4),
                arguments: vec![ApplyArgument::Value(Register::new(7))],
                value: Register::new(6),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(5),
                src: Register::new(8),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![entry], FunctionId::new(0))
}

fn copy_global_module(source: &str, destination: &str) -> BytecodeModule {
    BytecodeModule::new(
        vec![Function {
            name: "<entry>".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String(source.to_owned()),
                Constant::String(destination.to_owned()),
            ],
            instructions: vec![
                instruction(InstructionKind::LoadGlobal {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                    construct_if_class: false,
                }),
                instruction(InstructionKind::StoreGlobal {
                    name: ConstantId::new(1),
                    src: Register::new(0),
                }),
                instruction(InstructionKind::Return { values: Vec::new() }),
            ],
            exception_handlers: Vec::new(),
        }],
        FunctionId::new(0),
    )
}

#[test]
fn value_class_assignment_method_and_property_access_are_independent() {
    let mut interpreter =
        Interpreter::new(class_module("ValueCounter", ClassSemantics::Value, false)).unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(exercise_module("ValueCounter"))
        .unwrap();
    interpreter.execute_entry(&[]).unwrap();

    assert_eq!(
        interpreter.workspace().get("a_value"),
        Some(&Value::Double(4.0))
    );
    assert_eq!(
        interpreter.workspace().get("b_value"),
        Some(&Value::Double(0.0))
    );
}

#[test]
fn handle_class_assignment_and_method_dispatch_preserve_identity_aliases() {
    let mut interpreter =
        Interpreter::new(class_module("HandleCounter", ClassSemantics::Handle, false)).unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(exercise_module("HandleCounter"))
        .unwrap();
    interpreter.execute_entry(&[]).unwrap();

    assert_eq!(
        interpreter.workspace().get("a_value"),
        Some(&Value::Double(4.0))
    );
    assert_eq!(
        interpreter.workspace().get("b_value"),
        Some(&Value::Double(4.0))
    );
}

#[test]
fn failed_constructor_rolls_back_object_and_workspace_state() {
    let mut interpreter =
        Interpreter::new(class_module("Broken", ClassSemantics::Value, true)).unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(constructor_call_module("Broken"))
        .unwrap();
    let error = interpreter
        .execute_entry(&[])
        .expect_err("constructor body should fail");

    assert!(matches!(
        error.kind,
        RuntimeErrorKind::UndefinedGlobal { .. }
    ));
    assert_eq!(error.location, Some(LOCATION));
    assert!(!error.stack.is_empty());
    assert!(interpreter.workspace().get("failed").is_none());
    assert_eq!(interpreter.object_count(), 0);
    assert_eq!(interpreter.class_count(), 2);
}

#[test]
fn class_constant_and_static_method_execute_without_allocating_an_instance() {
    let mut interpreter = Interpreter::new(static_constant_module("Scale")).unwrap();
    interpreter.execute_entry(&[]).unwrap();
    assert_eq!(interpreter.object_count(), 0);

    interpreter
        .replace_module(static_constant_exercise_module("Scale"))
        .unwrap();
    interpreter.execute_entry(&[]).unwrap();

    assert_eq!(
        interpreter.workspace().get("factor"),
        Some(&Value::Double(3.0))
    );
    assert_eq!(
        interpreter.workspace().get("scaled"),
        Some(&Value::Double(15.0))
    );
    assert_eq!(interpreter.object_count(), 0);
}

#[test]
fn constant_rejects_class_and_instance_assignment() {
    let mut interpreter = Interpreter::new(static_constant_module("ReadOnly")).unwrap();
    interpreter.execute_entry(&[]).unwrap();

    for construct in [false, true] {
        interpreter
            .replace_module(constant_assignment_module("ReadOnly", construct))
            .unwrap();
        let error = interpreter
            .execute_entry(&[])
            .expect_err("constant assignment must fail");
        assert!(matches!(error.kind, RuntimeErrorKind::Object { .. }));
        assert!(error.to_string().contains("read-only"));
        assert_eq!(error.location, Some(LOCATION));
    }
}

#[allow(clippy::too_many_lines)]
fn inheritance_constructor_module(failing_base: bool) -> BytecodeModule {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("Derived".to_owned()),
            Constant::Double(4.0),
            Constant::String("created".to_owned()),
            Constant::String("Value".to_owned()),
            Constant::String("observed".to_owned()),
        ],
        instructions: vec![
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
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(2),
                src: Register::new(2),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(3),
                object: Register::new(2),
                name: ConstantId::new(3),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(4),
                src: Register::new(3),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut base_instructions = Vec::new();
    let mut base_constants = vec![Constant::String("Value".to_owned())];
    if failing_base {
        base_constants.push(Constant::String("missing_in_base_constructor".to_owned()));
        base_instructions.push(instruction(InstructionKind::LoadGlobal {
            dst: Register::new(0),
            name: ConstantId::new(1),
            construct_if_class: false,
        }));
    }
    base_instructions.extend([
        instruction(InstructionKind::LoadLocal {
            dst: Register::new(0),
            local: LocalSlot::new(1),
        }),
        instruction(InstructionKind::LoadLocal {
            dst: Register::new(1),
            local: LocalSlot::new(0),
        }),
        instruction(InstructionKind::SetField {
            dst: Register::new(2),
            object: Register::new(0),
            name: ConstantId::new(0),
            value: Register::new(1),
        }),
        instruction(InstructionKind::StoreLocal {
            local: LocalSlot::new(1),
            src: Register::new(2),
        }),
        instruction(InstructionKind::Return {
            values: vec![Register::new(2)],
        }),
    ]);
    let base_constructor = Function {
        name: "Base.Base".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: base_constants,
        instructions: base_instructions,
        exception_handlers: Vec::new(),
    };
    let derived_constructor = Function {
        name: "Derived.Derived".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![Constant::String("Base".to_owned())],
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::InvokeSuperclassConstructor {
                superclass: ConstantId::new(0),
                object: LocalSlot::new(1),
                arguments: vec![Register::new(0)],
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let constructor = |name: &str, function| MethodDefinition {
        name: name.to_owned(),
        function,
        kind: MethodKind::Instance,
        access: Access::Public,
        is_abstract: false,
        is_external: false,
        constructor_output: Some(LocalSlot::new(1)),
        location: LOCATION,
    };
    BytecodeModule::new(
        vec![entry, base_constructor, derived_constructor],
        FunctionId::new(0),
    )
    .with_classes(vec![
        ClassDefinition {
            name: "Base".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: vec![PropertyDefinition {
                name: "Value".to_owned(),
                default: None,
                kind: openmat_bytecode::PropertyKind::Stored,
                get_access: Some(Access::Public),
                set_access: Some(Access::Public),
                location: LOCATION,
            }],
            methods: vec![constructor("Base", FunctionId::new(1))],
            location: LOCATION,
        },
        ClassDefinition {
            name: "Derived".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: Some("Base".to_owned()),
            declared_abstract: None,
            sealed: false,
            properties: Vec::new(),
            methods: vec![constructor("Derived", FunctionId::new(2))],
            location: LOCATION,
        },
    ])
}

#[test]
fn direct_superclass_constructor_initializes_the_same_derived_object() {
    let mut interpreter = Interpreter::new(inheritance_constructor_module(false)).unwrap();
    interpreter.execute_entry(&[]).unwrap();

    assert_eq!(
        interpreter.workspace().get("observed"),
        Some(&Value::Double(4.0))
    );
}

#[test]
fn failed_superclass_constructor_rolls_back_the_derived_object_atomically() {
    let mut interpreter = Interpreter::new(inheritance_constructor_module(true)).unwrap();
    let error = interpreter
        .execute_entry(&[])
        .expect_err("base constructor body should fail");

    assert!(matches!(
        error.kind,
        RuntimeErrorKind::UndefinedGlobal { .. }
    ));
    assert!(interpreter.workspace().get("created").is_none());
    assert_eq!(interpreter.object_count(), 0);
    assert_eq!(interpreter.class_count(), 3);
}

#[test]
fn homogeneous_value_and_handle_arrays_apply_element_assignment_semantics() {
    let mut values =
        Interpreter::new(class_module("ValueItem", ClassSemantics::Value, false)).unwrap();
    values.execute_entry(&[]).unwrap();
    values
        .replace_module(object_array_module("ValueItem", "ValueItem"))
        .unwrap();
    values.execute_entry(&[]).unwrap();
    let Some(Value::ObjectArray(array)) = values.workspace().get("result") else {
        panic!("value-class indexed assignment must create an object array");
    };
    assert_eq!(array.shape().dimensions(), &[1, 2]);
    assert_ne!(array.as_slice()[0], array.as_slice()[1]);
    let original_value_handles = array.as_slice().to_vec();
    values
        .replace_module(copy_global_module("result", "copied"))
        .unwrap();
    values.execute_entry(&[]).unwrap();
    let Some(Value::ObjectArray(copied)) = values.workspace().get("copied") else {
        panic!("copied value-class array must remain an object array");
    };
    assert!(
        original_value_handles
            .iter()
            .zip(copied.as_slice())
            .all(|(original, copied)| original != copied)
    );

    let mut handles =
        Interpreter::new(class_module("HandleItem", ClassSemantics::Handle, false)).unwrap();
    handles.execute_entry(&[]).unwrap();
    handles
        .replace_module(object_array_module("HandleItem", "HandleItem"))
        .unwrap();
    handles.execute_entry(&[]).unwrap();
    let Some(Value::ObjectArray(array)) = handles.workspace().get("result") else {
        panic!("handle-class indexed assignment must create an object array");
    };
    assert_eq!(array.shape().dimensions(), &[1, 2]);
    assert_ne!(array.as_slice()[0], array.as_slice()[1]);
    let original_handle_elements = array.as_slice().to_vec();
    handles
        .replace_module(copy_global_module("result", "copied"))
        .unwrap();
    handles.execute_entry(&[]).unwrap();
    let Some(Value::ObjectArray(copied)) = handles.workspace().get("copied") else {
        panic!("copied handle-class array must remain an object array");
    };
    assert_eq!(original_handle_elements, copied.as_slice());
}

#[test]
fn heterogeneous_object_array_assignment_returns_structured_object_error() {
    let mut interpreter =
        Interpreter::new(class_module("FirstItem", ClassSemantics::Value, false)).unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(class_module("SecondItem", ClassSemantics::Value, false))
        .unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(object_array_module("FirstItem", "SecondItem"))
        .unwrap();
    let error = interpreter
        .execute_entry(&[])
        .expect_err("heterogeneous array assignment must fail");

    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Object {
            operation: "indexed assignment",
            ..
        }
    ));
    assert!(interpreter.workspace().get("result").is_none());
}

#[test]
#[allow(clippy::too_many_lines)]
fn dependent_accessors_execute_without_a_slot_and_preserve_value_updates() {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 0,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(0),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let default = Function {
        name: "DependentBox.<default:Base>".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(4.0)],
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
    let getter = Function {
        name: "DependentBox.get.Twice".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![Constant::String("Base".to_owned()), Constant::Double(2.0)],
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(1),
                object: Register::new(0),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Multiply,
                dst: Register::new(3),
                lhs: Register::new(1),
                rhs: Register::new(2),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let setter = Function {
        name: "DependentBox.set.Twice".to_owned(),
        register_count: 5,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: vec![Constant::String("Base".to_owned()), Constant::Double(2.0)],
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Divide,
                dst: Register::new(3),
                lhs: Register::new(1),
                rhs: Register::new(2),
            }),
            instruction(InstructionKind::SetField {
                dst: Register::new(4),
                object: Register::new(0),
                name: ConstantId::new(0),
                value: Register::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let class = ClassDefinition {
        name: "DependentBox".to_owned(),
        semantics: ClassSemantics::Value,
        superclass: None,
        declared_abstract: None,
        sealed: false,
        properties: vec![
            PropertyDefinition {
                name: "Base".to_owned(),
                kind: PropertyKind::Stored,
                default: Some(FunctionId::new(1)),
                get_access: Some(Access::Private),
                set_access: Some(Access::Private),
                location: LOCATION,
            },
            PropertyDefinition {
                name: "Twice".to_owned(),
                kind: PropertyKind::Dependent,
                default: None,
                get_access: Some(Access::Public),
                set_access: Some(Access::Public),
                location: LOCATION,
            },
        ],
        methods: vec![
            MethodDefinition {
                name: "get.Twice".to_owned(),
                function: FunctionId::new(2),
                kind: MethodKind::Instance,
                access: Access::Public,
                is_abstract: false,
                is_external: false,
                constructor_output: None,
                location: LOCATION,
            },
            MethodDefinition {
                name: "set.Twice".to_owned(),
                function: FunctionId::new(3),
                kind: MethodKind::Instance,
                access: Access::Public,
                is_abstract: false,
                is_external: false,
                constructor_output: None,
                location: LOCATION,
            },
        ],
        location: LOCATION,
    };
    let mut interpreter = Interpreter::new(
        BytecodeModule::new(vec![entry, default, getter, setter], FunctionId::new(0))
            .with_classes(vec![class]),
    )
    .unwrap();
    interpreter.execute_entry(&[]).unwrap();

    let exercise = Function {
        name: "<entry>".to_owned(),
        register_count: 7,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("DependentBox".to_owned()),
            Constant::String("Twice".to_owned()),
            Constant::Double(18.0),
            Constant::String("first".to_owned()),
            Constant::String("after".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: true,
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(1),
                object: Register::new(0),
                name: ConstantId::new(1),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(3),
                src: Register::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::SetField {
                dst: Register::new(3),
                object: Register::new(0),
                name: ConstantId::new(1),
                value: Register::new(2),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(4),
                object: Register::new(3),
                name: ConstantId::new(1),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(4),
                src: Register::new(4),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    interpreter
        .replace_module(BytecodeModule::new(vec![exercise], FunctionId::new(0)))
        .unwrap();
    interpreter.execute_entry(&[]).unwrap();

    assert_eq!(
        interpreter.workspace().get("first"),
        Some(&Value::Double(8.0))
    );
    assert_eq!(
        interpreter.workspace().get("after"),
        Some(&Value::Double(18.0))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn add_with_objects_dispatches_plus_and_missing_method_is_invalid_operands() {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 0,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            instruction(InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(0),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let plus = Function {
        name: "Addend.plus".to_owned(),
        register_count: 5,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: vec![Constant::String("Value".to_owned())],
        instructions: vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(1),
                object: Register::new(0),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::GetField {
                dst: Register::new(3),
                object: Register::new(2),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(4),
                lhs: Register::new(1),
                rhs: Register::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let class = ClassDefinition {
        name: "Addend".to_owned(),
        semantics: ClassSemantics::Value,
        superclass: None,
        declared_abstract: None,
        sealed: false,
        properties: vec![PropertyDefinition {
            name: "Value".to_owned(),
            kind: PropertyKind::Stored,
            default: None,
            get_access: Some(Access::Public),
            set_access: Some(Access::Public),
            location: LOCATION,
        }],
        methods: vec![MethodDefinition {
            name: "plus".to_owned(),
            function: FunctionId::new(1),
            kind: MethodKind::Instance,
            access: Access::Public,
            is_abstract: false,
            is_external: false,
            constructor_output: None,
            location: LOCATION,
        }],
        location: LOCATION,
    };
    let mut interpreter = Interpreter::new(
        BytecodeModule::new(vec![entry, plus], FunctionId::new(0)).with_classes(vec![class]),
    )
    .unwrap();
    interpreter.execute_entry(&[]).unwrap();

    let exercise = Function {
        name: "<entry>".to_owned(),
        register_count: 10,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("Addend".to_owned()),
            Constant::String("Value".to_owned()),
            Constant::Double(7.0),
            Constant::Double(5.0),
            Constant::String("sum".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: true,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::SetField {
                dst: Register::new(2),
                object: Register::new(0),
                name: ConstantId::new(1),
                value: Register::new(1),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(3),
                name: ConstantId::new(0),
                construct_if_class: true,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(4),
                constant: ConstantId::new(3),
            }),
            instruction(InstructionKind::SetField {
                dst: Register::new(5),
                object: Register::new(3),
                name: ConstantId::new(1),
                value: Register::new(4),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(6),
                lhs: Register::new(2),
                rhs: Register::new(5),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(4),
                src: Register::new(6),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    interpreter
        .replace_module(BytecodeModule::new(vec![exercise], FunctionId::new(0)))
        .unwrap();
    interpreter.execute_entry(&[]).unwrap();
    assert_eq!(
        interpreter.workspace().get("sum"),
        Some(&Value::Double(12.0))
    );

    let missing = class_module("NoPlus", ClassSemantics::Value, false);
    let mut missing_interpreter = Interpreter::new(missing).unwrap();
    missing_interpreter.execute_entry(&[]).unwrap();
    let mut invalid = exercise_module("NoPlus");
    let function = &mut invalid.functions[0];
    function.instructions.insert(
        function.instructions.len() - 1,
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::Add,
            dst: Register::new(9),
            lhs: Register::new(2),
            rhs: Register::new(2),
        }),
    );
    missing_interpreter.replace_module(invalid).unwrap();
    let error = missing_interpreter
        .execute_entry(&[])
        .expect_err("class without plus must reject addition");
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InvalidOperands { .. }
    ));
}

#[allow(clippy::too_many_lines)]
fn enumeration_class_module(name: &str, semantics: ClassSemantics) -> BytecodeModule {
    let entry = Function {
        name: "<entry>".to_owned(),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::Double(0.0),
            Constant::String("enum_initializer_count".to_owned()),
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
            instruction(InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(0),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let default = Function {
        name: format!("{name}.<default:Value>"),
        register_count: 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![Constant::Double(0.0)],
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
    let initializer = Function {
        name: format!("{name}.<enumeration:Only>"),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("enum_initializer_count".to_owned()),
            Constant::Double(1.0),
            Constant::Double(17.0),
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
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(0),
                src: Register::new(2),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let constructor = Function {
        name: format!("{name}.{name}"),
        register_count: 4,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![
            Constant::String("enum_constructor_allowed".to_owned()),
            Constant::String("Value".to_owned()),
        ],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::SetField {
                dst: Register::new(3),
                object: Register::new(1),
                name: ConstantId::new(1),
                value: Register::new(2),
            }),
            instruction(InstructionKind::StoreLocal {
                local: LocalSlot::new(1),
                src: Register::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(
        vec![entry, default, initializer, constructor],
        FunctionId::new(0),
    )
    .with_classes(vec![ClassDefinition {
        name: name.to_owned(),
        semantics,
        superclass: None,
        declared_abstract: None,
        sealed: false,
        properties: vec![PropertyDefinition {
            name: "Value".to_owned(),
            kind: PropertyKind::Stored,
            default: Some(FunctionId::new(1)),
            get_access: Some(Access::Public),
            set_access: Some(Access::Public),
            location: LOCATION,
        }],
        methods: vec![MethodDefinition {
            name: name.to_owned(),
            function: FunctionId::new(3),
            kind: MethodKind::Instance,
            access: Access::Public,
            is_abstract: false,
            is_external: false,
            constructor_output: Some(LocalSlot::new(1)),
            location: LOCATION,
        }],
        location: LOCATION,
    }])
    .with_class_features(vec![ClassFeatureDefinition {
        class: ClassDefinitionId::new(0),
        kind: ClassKind::Enumeration,
        enumeration_base: (semantics == ClassSemantics::Handle).then(|| "handle".to_owned()),
        enumeration_members: vec![EnumMemberDefinition {
            name: "Only".to_owned(),
            argument_initializer: FunctionId::new(2),
            argument_count: 1,
            location: LOCATION,
        }],
        events: Vec::new(),
        abstract_properties: Vec::new(),
        sealed_methods: Vec::new(),
        location: LOCATION,
    }])
}

fn enumeration_exercise_module(name: &str, allow_constructor: bool) -> BytecodeModule {
    let mut constants = vec![
        Constant::String(name.to_owned()),
        Constant::String("Only".to_owned()),
        Constant::String("Value".to_owned()),
        Constant::String("first_enum_value".to_owned()),
        Constant::String("second_enum_value".to_owned()),
    ];
    let mut instructions = Vec::new();
    let mut register_count = 6;
    if allow_constructor {
        constants.push(Constant::Double(1.0));
        constants.push(Constant::String("enum_constructor_allowed".to_owned()));
        instructions.extend([
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(5),
                constant: ConstantId::new(5),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(6),
                src: Register::new(5),
            }),
        ]);
        register_count = 7;
    }
    instructions.extend([
        instruction(InstructionKind::LoadGlobal {
            dst: Register::new(0),
            name: ConstantId::new(0),
            construct_if_class: false,
        }),
        instruction(InstructionKind::GetField {
            dst: Register::new(1),
            object: Register::new(0),
            name: ConstantId::new(1),
        }),
        instruction(InstructionKind::GetField {
            dst: Register::new(2),
            object: Register::new(1),
            name: ConstantId::new(2),
        }),
        instruction(InstructionKind::StoreGlobal {
            name: ConstantId::new(3),
            src: Register::new(2),
        }),
        instruction(InstructionKind::GetField {
            dst: Register::new(3),
            object: Register::new(0),
            name: ConstantId::new(1),
        }),
        instruction(InstructionKind::GetField {
            dst: Register::new(4),
            object: Register::new(3),
            name: ConstantId::new(2),
        }),
        instruction(InstructionKind::StoreGlobal {
            name: ConstantId::new(4),
            src: Register::new(4),
        }),
        instruction(InstructionKind::Return { values: Vec::new() }),
    ]);
    BytecodeModule::new(
        vec![Function {
            name: "<entry>".to_owned(),
            register_count,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants,
            instructions,
            exception_handlers: Vec::new(),
        }],
        FunctionId::new(0),
    )
}

#[test]
fn enumeration_member_factory_is_once_only_and_direct_construction_stays_blocked() {
    let mut interpreter = Interpreter::new(enumeration_class_module(
        "RuntimeState",
        ClassSemantics::Value,
    ))
    .unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(enumeration_exercise_module("RuntimeState", true))
        .unwrap();
    interpreter.execute_entry(&[]).unwrap();

    assert_eq!(
        interpreter.workspace().get("first_enum_value"),
        Some(&Value::Double(17.0))
    );
    assert_eq!(
        interpreter.workspace().get("second_enum_value"),
        Some(&Value::Double(17.0))
    );
    assert_eq!(
        interpreter.workspace().get("enum_initializer_count"),
        Some(&Value::Double(1.0))
    );

    interpreter
        .replace_module(constructor_call_module("RuntimeState"))
        .unwrap();
    let error = interpreter
        .execute_entry(&[])
        .expect_err("ordinary enumeration construction must stay unavailable");
    assert!(error.to_string().contains("declared member"));
}

#[test]
fn failed_enumeration_construction_rolls_back_and_later_lookup_retries_atomically() {
    let mut interpreter = Interpreter::new(enumeration_class_module(
        "RetryState",
        ClassSemantics::Value,
    ))
    .unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(enumeration_exercise_module("RetryState", false))
        .unwrap();
    let error = interpreter
        .execute_entry(&[])
        .expect_err("missing constructor gate should fail the first member construction");
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::UndefinedGlobal { .. }
    ));
    assert_eq!(interpreter.object_count(), 0);

    interpreter
        .replace_module(enumeration_exercise_module("RetryState", true))
        .unwrap();
    interpreter.execute_entry(&[]).unwrap();
    assert_eq!(
        interpreter.workspace().get("first_enum_value"),
        Some(&Value::Double(17.0))
    );
    assert_eq!(
        interpreter.workspace().get("enum_initializer_count"),
        Some(&Value::Double(2.0))
    );
}

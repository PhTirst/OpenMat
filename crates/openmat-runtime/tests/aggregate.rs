use openmat_array::{ArrayData, CharCodeUnit, DenseArray, IntegerArrayData, Shape};
use openmat_bytecode::{
    Access, ApplyArgument, AssignmentMode, BinaryOperator, BindingTarget, BytecodeModule,
    ClassDefinition, ClassDefinitionId, ClassSemantics, Constant, ConstantId, FieldOperand,
    Function, FunctionId, Instruction, InstructionKind, LocalSlot, MethodDefinition, MethodKind,
    PackRegister, PlaceStep, PropertyDefinition, PropertyKind, Register, SourceLocation,
    ValueSource,
};
use openmat_runtime::{
    ArrayRuntimeError, BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistry,
    Interpreter, RuntimeErrorKind,
};
use openmat_value::{
    BuiltinHandle, CellArray, ClassHandle, FieldName, FunctionHandle, ObjectArray, ObjectHandle,
    StructArray, Value,
};

const LOCATION: SourceLocation = SourceLocation::new(1, 0, 1);

fn instruction(kind: InstructionKind) -> Instruction {
    Instruction::new(kind)
}

fn function(
    name: &str,
    registers: u32,
    packs: u32,
    locals: u32,
    parameters: u32,
    constants: Vec<Constant>,
    instructions: Vec<Instruction>,
) -> Function {
    Function {
        name: name.to_owned(),
        register_count: registers,
        pack_register_count: packs,
        local_count: locals,
        persistent_slot_count: 0,
        parameter_count: parameters,
        argument_layout: None,
        constants,
        instructions,
        exception_handlers: Vec::new(),
    }
}

fn module(function: Function) -> BytecodeModule {
    BytecodeModule::new(vec![function], FunctionId::new(0))
}

fn workspace_paren_assignment_module() -> BytecodeModule {
    module(function(
        "workspace-paren-assignment",
        4,
        0,
        2,
        2,
        vec![Constant::String("root".to_owned())],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(3),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    2,
                ))])],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(0),
                src: Register::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
    ))
}

fn workspace_global_scalar_assignment_module() -> BytecodeModule {
    module(function(
        "workspace-global-scalar-assignment",
        5,
        0,
        2,
        2,
        vec![Constant::String("root".to_owned())],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(2),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::AssignBindingPlace {
                result: None,
                binding: BindingTarget::Workspace(ConstantId::new(0)),
                root: Register::new(2),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    0,
                ))])],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(3),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::ApplyBinding {
                outputs: vec![Register::new(4)],
                target: Register::new(3),
                arguments: vec![ApplyArgument::Value(Register::new(0))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4)],
            }),
        ],
    ))
}

#[test]
fn direct_workspace_scalar_assignment_detaches_real_array_aliases() {
    let original = real_array([1, 3], vec![1.0, 2.0, 3.0]);
    let mut interpreter = Interpreter::new(workspace_global_scalar_assignment_module()).unwrap();
    interpreter.workspace_mut().insert("root", original.clone());
    interpreter
        .workspace_mut()
        .insert("alias", original.clone());

    assert_eq!(
        interpreter
            .execute_entry(&[Value::Double(2.0), Value::Double(9.0)])
            .unwrap(),
        vec![Value::Double(9.0)]
    );

    let root = interpreter.workspace().get("root").unwrap();
    let alias = interpreter.workspace().get("alias").unwrap();
    assert_eq!(root, &real_array([1, 3], vec![1.0, 9.0, 3.0]));
    assert_eq!(alias, &original);
    assert!(!root.shares_storage_with(alias));
    assert!(alias.shares_storage_with(&original));
}

#[test]
fn failed_direct_workspace_scalar_assignment_restores_the_original_binding() {
    let original = real_array([1, 3], vec![1.0, 2.0, 3.0]);
    let mut interpreter = Interpreter::new(workspace_global_scalar_assignment_module()).unwrap();
    interpreter.workspace_mut().insert("root", original.clone());
    interpreter
        .workspace_mut()
        .insert("alias", original.clone());

    interpreter
        .execute_entry(&[Value::Double(4.0), Value::Double(9.0)])
        .expect_err("out-of-range assignment must fail");

    let root = interpreter.workspace().get("root").unwrap();
    let alias = interpreter.workspace().get("alias").unwrap();
    assert_eq!(root, &original);
    assert_eq!(alias, &original);
    assert!(root.shares_storage_with(alias));
}

#[allow(clippy::too_many_lines)]
fn aggregate_class_module(
    name: &str,
    semantics: ClassSemantics,
    property_access: Access,
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
    BytecodeModule::new(vec![entry, default, add], FunctionId::new(0)).with_classes(vec![
        ClassDefinition {
            name: name.to_owned(),
            semantics,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: vec![PropertyDefinition {
                name: "value".to_owned(),
                kind: PropertyKind::Stored,
                default: Some(FunctionId::new(1)),
                get_access: Some(property_access),
                set_access: Some(property_access),
                location: LOCATION,
            }],
            methods: vec![MethodDefinition {
                name: "add".to_owned(),
                function: FunctionId::new(2),
                kind: MethodKind::Instance,
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

#[allow(clippy::too_many_lines)]
fn aggregate_constant_class_module(name: &str) -> BytecodeModule {
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
        name: format!("{name}.<default>"),
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
        register_count: 4,
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
            properties: vec![
                PropertyDefinition {
                    name: "Factor".to_owned(),
                    kind: PropertyKind::Constant,
                    default: Some(FunctionId::new(1)),
                    get_access: Some(Access::Public),
                    set_access: None,
                    location: LOCATION,
                },
                PropertyDefinition {
                    name: "Secret".to_owned(),
                    kind: PropertyKind::Constant,
                    default: Some(FunctionId::new(1)),
                    get_access: Some(Access::Private),
                    set_access: None,
                    location: LOCATION,
                },
                PropertyDefinition {
                    name: "Stored".to_owned(),
                    kind: PropertyKind::Stored,
                    default: Some(FunctionId::new(1)),
                    get_access: Some(Access::Public),
                    set_access: Some(Access::Public),
                    location: LOCATION,
                },
            ],
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

fn class_aggregate_field_module(class_name: &str, field: &str, dynamic: bool) -> BytecodeModule {
    let operand = if dynamic {
        FieldOperand::Dynamic(Register::new(1))
    } else {
        FieldOperand::Static(ConstantId::new(1))
    };
    module(function(
        "class-aggregate-field",
        3,
        1,
        0,
        0,
        vec![
            Constant::String(class_name.to_owned()),
            Constant::String(field.to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                field: operand,
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(2)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    ))
}

fn construct_class_module(class_name: &str) -> BytecodeModule {
    module(function(
        "construct",
        2,
        0,
        0,
        0,
        vec![Constant::String(class_name.to_owned())],
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
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
    ))
}

fn object_aggregate_module(global_name: &str) -> BytecodeModule {
    module(function(
        "object-aggregate",
        8,
        4,
        0,
        0,
        vec![
            Constant::String(global_name.to_owned()),
            Constant::String("value".to_owned()),
            Constant::Double(4.0),
            Constant::String("add".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                field: FieldOperand::Static(ConstantId::new(1)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(1)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(3),
                root: Register::new(0),
                path: vec![PlaceStep::Field(FieldOperand::Static(ConstantId::new(1)))],
                source: ValueSource::One(Register::new(2)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(1),
                target: Register::new(3),
                field: FieldOperand::Static(ConstantId::new(1)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(4)],
                pack: PackRegister::new(1),
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(2),
                target: Register::new(0),
                field: FieldOperand::Static(ConstantId::new(1)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(5)],
                pack: PackRegister::new(2),
            }),
            instruction(InstructionKind::ApplyField {
                outputs: vec![Register::new(6)],
                object: Register::new(0),
                name: ConstantId::new(3),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(3),
                target: Register::new(6),
                field: FieldOperand::Static(ConstantId::new(1)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(7)],
                pack: PackRegister::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![
                    Register::new(0),
                    Register::new(1),
                    Register::new(3),
                    Register::new(4),
                    Register::new(5),
                    Register::new(6),
                    Register::new(7),
                ],
            }),
        ],
    ))
}

fn object_aggregate_read_module(global_name: &str) -> BytecodeModule {
    module(function(
        "object-aggregate-read",
        2,
        1,
        0,
        0,
        vec![
            Constant::String(global_name.to_owned()),
            Constant::String("value".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                field: FieldOperand::Static(ConstantId::new(1)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(1)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
    ))
}

fn nested_object_assign_module(global_name: &str) -> BytecodeModule {
    module(function(
        "nested-object-assign",
        8,
        4,
        0,
        0,
        vec![
            Constant::String(global_name.to_owned()),
            Constant::Double(1.0),
            Constant::Double(9.0),
            Constant::String("value".to_owned()),
        ],
        vec![
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
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(3),
                root: Register::new(0),
                path: vec![
                    PlaceStep::Brace(vec![ApplyArgument::Value(Register::new(1))]),
                    PlaceStep::Field(FieldOperand::Static(ConstantId::new(3))),
                ],
                source: ValueSource::One(Register::new(2)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(3),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(4)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(1),
                target: Register::new(4),
                field: FieldOperand::Static(ConstantId::new(3)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(5)],
                pack: PackRegister::new(1),
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(2),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(6)],
                pack: PackRegister::new(2),
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(3),
                target: Register::new(6),
                field: FieldOperand::Static(ConstantId::new(3)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(7)],
                pack: PackRegister::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![
                    Register::new(3),
                    Register::new(4),
                    Register::new(5),
                    Register::new(6),
                    Register::new(7),
                ],
            }),
        ],
    ))
}

fn builtin_copy_module(builtin_name: &str) -> BytecodeModule {
    module(function(
        "builtin-copy",
        4,
        0,
        0,
        0,
        vec![
            Constant::String("source".to_owned()),
            Constant::String(builtin_name.to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2), Register::new(3)],
                target: Register::new(1),
                arguments: vec![ApplyArgument::Value(Register::new(0))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2), Register::new(3)],
            }),
        ],
    ))
}

fn cell(dimensions: impl IntoIterator<Item = u64>, values: Vec<Value>) -> Value {
    Value::Cell(CellArray::from_values(Shape::new(dimensions).unwrap(), values).unwrap())
}

fn structure(
    dimensions: impl IntoIterator<Item = u64>,
    fields: &[&str],
    columns: Vec<Vec<Value>>,
) -> Value {
    let fields = fields
        .iter()
        .map(|name| FieldName::new(*name).unwrap())
        .collect();
    Value::Struct(
        StructArray::from_columns(Shape::new(dimensions).unwrap(), fields, columns).unwrap(),
    )
}

fn real_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn empty_double() -> Value {
    Value::empty_double()
}

fn char_scalar(code_unit: u16) -> Value {
    Value::Array(ArrayData::Char(
        DenseArray::from_vec(
            Shape::new([1, 1]).unwrap(),
            vec![CharCodeUnit::new(code_unit)],
        )
        .unwrap(),
    ))
}

#[allow(clippy::unnecessary_wraps)]
fn identity_builtin(
    arguments: &[Value],
    _context: &mut BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    Ok(arguments.to_vec())
}

fn copy_twice_success(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    let Some(value) = arguments.first() else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "copy_twice_success requires one input",
        ));
    };
    Ok(vec![
        context.language_copy(value)?,
        context.language_copy(value)?,
    ])
}

fn copy_twice_then_fail(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    let Some(value) = arguments.first() else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "copy_twice_then_fail requires one input",
        ));
    };
    let _first = context.language_copy(value)?;
    let _second = context.language_copy(value)?;
    Err(BuiltinError::new(
        BuiltinErrorCategory::Other,
        "intentional failure after copies",
    ))
}

#[test]
#[allow(clippy::too_many_lines)]
fn build_cell_is_column_major_and_brace_unpack_has_exact_arity() {
    let bytecode_function = function(
        "cell-pack",
        10,
        1,
        0,
        0,
        vec![
            Constant::Double(11.0),
            Constant::Double(22.0),
            Constant::Double(33.0),
            Constant::Double(44.0),
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
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(3),
            }),
            instruction(InstructionKind::BuildCell {
                dst: Register::new(4),
                rows: vec![
                    vec![
                        ValueSource::One(Register::new(0)),
                        ValueSource::One(Register::new(1)),
                    ],
                    vec![
                        ValueSource::One(Register::new(2)),
                        ValueSource::One(Register::new(3)),
                    ],
                ],
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(4),
                arguments: vec![ApplyArgument::Colon],
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![
                    Register::new(5),
                    Register::new(6),
                    Register::new(7),
                    Register::new(8),
                ],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(9)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![
                    Register::new(4),
                    Register::new(5),
                    Register::new(6),
                    Register::new(7),
                    Register::new(8),
                    Register::new(9),
                ],
            }),
        ],
    );
    let values = Interpreter::new(module(bytecode_function))
        .unwrap()
        .execute_entry(&[])
        .unwrap();
    let Value::Cell(built) = &values[0] else {
        panic!("expected cell result");
    };
    assert_eq!(built.shape().dimensions(), &[2, 2]);
    assert_eq!(
        built.values(),
        &[
            Value::Double(11.0),
            Value::Double(33.0),
            Value::Double(22.0),
            Value::Double(44.0),
        ]
    );
    assert_eq!(
        &values[1..5],
        &[
            Value::Double(11.0),
            Value::Double(33.0),
            Value::Double(22.0),
            Value::Double(44.0),
        ]
    );
    assert_eq!(values[5], Value::Double(11.0));

    let too_many = function(
        "too-many",
        3,
        1,
        1,
        1,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Colon],
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(1), Register::new(2)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(2)],
            }),
        ],
    );
    let error = Interpreter::new(module(too_many))
        .unwrap()
        .execute_entry(&[cell([1, 1], vec![Value::Double(1.0)])])
        .unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::MissingOutputs {
            requested: 2,
            returned: 1
        }
    ));
}

#[test]
fn expanded_pack_splices_apply_arguments_and_cell_rows() {
    let function = function(
        "expand",
        5,
        2,
        2,
        2,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(1),
                arguments: vec![ApplyArgument::Colon],
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Expand(PackRegister::new(0))],
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(1),
                target: Register::new(2),
                arguments: vec![ApplyArgument::Colon],
            }),
            instruction(InstructionKind::BuildCell {
                dst: Register::new(3),
                rows: vec![vec![ValueSource::Expand(PackRegister::new(0))]],
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(4)],
                pack: PackRegister::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2), Register::new(3), Register::new(4)],
            }),
        ],
    );
    let target = cell(
        [2, 2],
        vec![
            Value::Double(11.0),
            Value::Double(33.0),
            Value::Double(22.0),
            Value::Double(44.0),
        ],
    );
    let indices = cell([1, 2], vec![Value::Double(2.0), Value::Double(1.0)]);
    let values = Interpreter::new(module(function))
        .unwrap()
        .execute_entry(&[target, indices])
        .unwrap();
    assert_eq!(values[2], Value::Double(33.0));
    let Value::Cell(selected) = &values[0] else {
        panic!("expected indexed cell");
    };
    assert_eq!(selected.shape().dimensions(), &[1, 1]);
    assert_eq!(selected.values(), &[Value::Double(33.0)]);
    let Value::Cell(spliced) = &values[1] else {
        panic!("expected expanded cell row");
    };
    assert_eq!(spliced.shape().dimensions(), &[1, 2]);
    assert_eq!(spliced.values(), &[Value::Double(2.0), Value::Double(1.0)]);
}

#[test]
fn aggregate_transport_preserves_exact_char_integer_and_string_values() {
    let function = function(
        "exact",
        7,
        1,
        3,
        3,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(2),
            }),
            instruction(InstructionKind::BuildCell {
                dst: Register::new(3),
                rows: vec![vec![
                    ValueSource::One(Register::new(0)),
                    ValueSource::One(Register::new(1)),
                    ValueSource::One(Register::new(2)),
                ]],
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(3),
                arguments: vec![ApplyArgument::Colon],
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(4), Register::new(5), Register::new(6)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4), Register::new(5), Register::new(6)],
            }),
        ],
    );
    let character = Value::Array(ArrayData::Char(
        DenseArray::from_vec(
            Shape::new([1, 2]).unwrap(),
            vec![CharCodeUnit::new(0xd800), CharCodeUnit::new(0)],
        )
        .unwrap(),
    ));
    let integer = Value::Array(ArrayData::Integer(IntegerArrayData::I64(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![9_007_199_254_740_993_i64]).unwrap(),
    )));
    let string = Value::from("exact");
    let values = Interpreter::new(module(function))
        .unwrap()
        .execute_entry(&[character.clone(), integer.clone(), string.clone()])
        .unwrap();
    assert_eq!(values, vec![character, integer, string]);
}

#[test]
fn switch_match_dispatch_remains_unchanged_with_pack_register_frames() {
    let switch = function(
        "switch-regression",
        3,
        0,
        2,
        2,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::SwitchMatch {
                dst: Register::new(2),
                selector: Register::new(0),
                case_value: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    );
    assert_eq!(
        Interpreter::new(module(switch))
            .unwrap()
            .execute_entry(&[Value::Double(1.0), Value::Logical(true)])
            .unwrap(),
        vec![Value::Logical(true)]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn struct_static_dynamic_fields_and_end_use_canonical_order_and_extents() {
    let read = function(
        "fields",
        6,
        2,
        1,
        1,
        vec![
            Constant::String("a".to_owned()),
            Constant::String("b".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                field: FieldOperand::Static(ConstantId::new(0)),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(1),
                target: Register::new(0),
                field: FieldOperand::Dynamic(Register::new(1)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(2), Register::new(3)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(4), Register::new(5)],
                pack: PackRegister::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![
                    Register::new(2),
                    Register::new(3),
                    Register::new(4),
                    Register::new(5),
                ],
            }),
        ],
    );
    let value = structure(
        [2, 1],
        &["a", "b"],
        vec![
            vec![Value::Double(1.0), Value::Double(2.0)],
            vec![Value::Double(10.0), Value::Double(20.0)],
        ],
    );
    let values = Interpreter::new(module(read))
        .unwrap()
        .execute_entry(&[value])
        .unwrap();
    assert_eq!(
        values,
        vec![
            Value::Double(1.0),
            Value::Double(2.0),
            Value::Double(10.0),
            Value::Double(20.0),
        ]
    );

    let end = function(
        "end",
        4,
        0,
        1,
        1,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::ResolveEnd {
                dst: Register::new(1),
                target: Register::new(0),
                argument_index: 0,
                argument_count: 1,
            }),
            instruction(InstructionKind::ResolveEnd {
                dst: Register::new(2),
                target: Register::new(0),
                argument_index: 0,
                argument_count: 2,
            }),
            instruction(InstructionKind::ResolveEnd {
                dst: Register::new(3),
                target: Register::new(0),
                argument_index: 1,
                argument_count: 2,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(2), Register::new(3)],
            }),
        ],
    );
    let target = cell([2, 3, 4], vec![empty_double(); 24]);
    let values = Interpreter::new(module(end))
        .unwrap()
        .execute_entry(&[target])
        .unwrap();
    assert_eq!(
        values,
        vec![Value::Double(24.0), Value::Double(2.0), Value::Double(12.0)]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn nested_assign_isolated_growth_deletion_and_failure_are_transactional() {
    let nested = function(
        "nested",
        5,
        0,
        2,
        2,
        vec![
            Constant::Double(1.0),
            Constant::Double(2.0),
            Constant::String("payload".to_owned()),
        ],
        vec![
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
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(4),
                root: Register::new(0),
                path: vec![
                    PlaceStep::Brace(vec![ApplyArgument::Value(Register::new(2))]),
                    PlaceStep::Field(FieldOperand::Static(ConstantId::new(2))),
                    PlaceStep::Brace(vec![ApplyArgument::Value(Register::new(3))]),
                ],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4)],
            }),
        ],
    );
    let payload = cell([1, 2], vec![Value::Double(1.0), Value::Double(2.0)]);
    let record = structure([1, 1], &["payload"], vec![vec![payload]]);
    let original = cell([1, 1], vec![record]);
    let result = Interpreter::new(module(nested))
        .unwrap()
        .execute_entry(&[original.clone(), Value::Double(9.0)])
        .unwrap()
        .remove(0);
    let nested_value = |value: &Value| {
        let Value::Cell(root) = value else {
            panic!("expected root cell");
        };
        let Value::Struct(record) = root.value_at_offset(0).unwrap() else {
            panic!("expected nested struct");
        };
        let Value::Cell(payload) = record.value_at_name("payload", 0).unwrap() else {
            panic!("expected nested cell");
        };
        payload.value_at_offset(1).unwrap().clone()
    };
    assert_eq!(nested_value(&original), Value::Double(2.0));
    assert_eq!(nested_value(&result), Value::Double(9.0));
    assert!(!original.shares_storage_with(&result));

    let grow = function(
        "grow",
        4,
        0,
        2,
        2,
        vec![Constant::Double(3.0)],
        vec![
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
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(3),
                root: Register::new(0),
                path: vec![PlaceStep::Brace(vec![ApplyArgument::Value(Register::new(
                    2,
                ))])],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
    );
    let grown = Interpreter::new(module(grow))
        .unwrap()
        .execute_entry(&[cell([1, 1], vec![Value::Double(1.0)]), Value::Double(7.0)])
        .unwrap()
        .remove(0);
    let Value::Cell(grown) = grown else {
        panic!("expected grown cell");
    };
    assert_eq!(grown.shape().dimensions(), &[1, 3]);
    assert_eq!(grown.value_at_offset(0), Some(&Value::Double(1.0)));
    assert!(is_empty_double_value(grown.value_at_offset(1).unwrap()));
    assert_eq!(grown.value_at_offset(2), Some(&Value::Double(7.0)));

    let delete = function(
        "delete",
        4,
        0,
        3,
        3,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(2),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(3),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    1,
                ))])],
                source: ValueSource::One(Register::new(2)),
                mode: AssignmentMode::Delete,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
    );
    let deleted = Interpreter::new(module(delete))
        .unwrap()
        .execute_entry(&[
            cell(
                [1, 4],
                vec![
                    Value::Double(1.0),
                    Value::Double(2.0),
                    Value::Double(3.0),
                    Value::Double(4.0),
                ],
            ),
            real_array([1, 2], vec![2.0, 4.0]),
            empty_double(),
        ])
        .unwrap()
        .remove(0);
    let Value::Cell(deleted) = deleted else {
        panic!("expected deleted cell");
    };
    assert_eq!(deleted.shape().dimensions(), &[1, 2]);
    assert_eq!(deleted.values(), &[Value::Double(1.0), Value::Double(3.0)]);
}

#[test]
#[allow(clippy::too_many_lines)]
fn struct_linear_growth_after_deletion_gap_fills_every_existing_field() {
    let function = function(
        "struct-delete-grow",
        9,
        0,
        4,
        4,
        vec![
            Constant::Double(2.0),
            Constant::Double(4.0),
            Constant::String("beta".to_owned()),
            Constant::String("alpha".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(2),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(3),
                local: LocalSlot::new(3),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(4),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(5),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    4,
                ))])],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Delete,
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(6),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(7),
                root: Register::new(5),
                path: vec![
                    PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(6))]),
                    PlaceStep::Field(FieldOperand::Static(ConstantId::new(2))),
                ],
                source: ValueSource::One(Register::new(2)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(8),
                root: Register::new(7),
                path: vec![
                    PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(6))]),
                    PlaceStep::Field(FieldOperand::Static(ConstantId::new(3))),
                ],
                source: ValueSource::One(Register::new(3)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(8)],
            }),
        ],
    );
    let original = structure(
        [1, 3],
        &["beta", "alpha"],
        vec![
            vec![Value::Double(1.0), Value::Double(2.0), Value::Double(3.0)],
            vec![
                char_scalar(u16::from(b'a')),
                char_scalar(u16::from(b'b')),
                char_scalar(u16::from(b'c')),
            ],
        ],
    );
    let grown = Interpreter::new(module(function))
        .unwrap()
        .execute_entry(&[
            original.clone(),
            empty_double(),
            Value::Double(40.0),
            char_scalar(u16::from(b'd')),
        ])
        .unwrap()
        .remove(0);
    let Value::Struct(grown) = grown else {
        panic!("expected grown struct");
    };

    assert_eq!(grown.shape().dimensions(), &[1, 4]);
    assert_eq!(grown.value_at_name("beta", 0), Some(&Value::Double(1.0)));
    assert_eq!(grown.value_at_name("beta", 1), Some(&Value::Double(3.0)));
    assert!(is_empty_double_value(
        grown.value_at_name("beta", 2).unwrap()
    ));
    assert!(is_empty_double_value(
        grown.value_at_name("alpha", 2).unwrap()
    ));
    assert_eq!(grown.value_at_name("beta", 3), Some(&Value::Double(40.0)));
    assert_eq!(
        grown.value_at_name("alpha", 3),
        Some(&char_scalar(u16::from(b'd')))
    );
    assert_eq!(
        original,
        structure(
            [1, 3],
            &["beta", "alpha"],
            vec![
                vec![Value::Double(1.0), Value::Double(2.0), Value::Double(3.0)],
                vec![
                    char_scalar(u16::from(b'a')),
                    char_scalar(u16::from(b'b')),
                    char_scalar(u16::from(b'c')),
                ],
            ],
        )
    );
}

fn is_empty_double_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(ArrayData::F64(array))
            if array.shape().dimensions() == [0, 0] && array.numel() == 0
    )
}

#[test]
#[allow(clippy::too_many_lines)]
fn shaped_empty_empty_selection_collapsed_growth_and_bounds_are_structured() {
    let empty_literal = function(
        "empty-cell-literal",
        1,
        0,
        0,
        0,
        Vec::new(),
        vec![
            instruction(InstructionKind::BuildCell {
                dst: Register::new(0),
                rows: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
    );
    let value = Interpreter::new(module(empty_literal))
        .unwrap()
        .execute_entry(&[])
        .unwrap()
        .remove(0);
    let Value::Cell(value) = value else {
        panic!("empty literal must be a cell");
    };
    assert_eq!(value.shape().dimensions(), &[0, 0]);

    let shaped_empty = function(
        "shaped-empty",
        4,
        1,
        2,
        2,
        vec![Constant::String("new_field".to_owned())],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(1),
                arguments: vec![ApplyArgument::Colon],
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(2),
                root: Register::new(0),
                path: vec![PlaceStep::Field(FieldOperand::Static(ConstantId::new(0)))],
                source: ValueSource::Expand(PackRegister::new(0)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::BuildCell {
                dst: Register::new(3),
                rows: vec![vec![ValueSource::Expand(PackRegister::new(0))]],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2), Register::new(3)],
            }),
        ],
    );
    let returned = Interpreter::new(module(shaped_empty))
        .unwrap()
        .execute_entry(&[
            structure([0, 3], &["existing"], vec![Vec::new()]),
            cell([0, 3], Vec::new()),
        ])
        .unwrap();
    let Value::Struct(structure) = &returned[0] else {
        panic!("expected shaped empty struct");
    };
    assert_eq!(structure.shape().dimensions(), &[0, 3]);
    assert_eq!(
        structure
            .field_names()
            .iter()
            .map(FieldName::as_str)
            .collect::<Vec<_>>(),
        vec!["existing", "new_field"]
    );
    assert_eq!(structure.field_values(1), Some([].as_slice()));
    let Value::Cell(spliced) = &returned[1] else {
        panic!("expected empty pack cell splice");
    };
    assert_eq!(spliced.shape().dimensions(), &[1, 0]);

    let no_op = function(
        "empty-selection",
        5,
        0,
        3,
        3,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(2),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(3),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    1,
                ))])],
                source: ValueSource::One(Register::new(2)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(4),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    1,
                ))])],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Delete,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3), Register::new(4)],
            }),
        ],
    );
    let original = cell([1, 2], vec![Value::Double(1.0), Value::Double(2.0)]);
    let returned = Interpreter::new(module(no_op))
        .unwrap()
        .execute_entry(&[
            original.clone(),
            empty_double(),
            cell([1, 1], vec![Value::Double(9.0)]),
        ])
        .unwrap();
    assert!(returned[0].shares_storage_with(&original));
    assert!(returned[1].shares_storage_with(&original));

    let collapsed_growth = function(
        "collapsed-growth",
        5,
        0,
        2,
        2,
        vec![Constant::Double(2.0), Constant::Double(13.0)],
        vec![
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
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(4),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![
                    ApplyArgument::Value(Register::new(2)),
                    ApplyArgument::Value(Register::new(3)),
                ])],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4)],
            }),
        ],
    );
    let grown = Interpreter::new(module(collapsed_growth))
        .unwrap()
        .execute_entry(&[
            cell([2, 3, 4], vec![empty_double(); 24]),
            cell([1, 1], vec![Value::Double(7.0)]),
        ])
        .unwrap()
        .remove(0);
    let Value::Cell(grown) = grown else {
        panic!("expected grown cell");
    };
    assert_eq!(grown.shape().dimensions(), &[2, 3, 5]);
    assert_eq!(grown.value_at_offset(25), Some(&Value::Double(7.0)));
    assert!(is_empty_double_value(grown.value_at_offset(24).unwrap()));

    let bounds = function(
        "brace-bounds",
        3,
        1,
        1,
        1,
        vec![Constant::Double(2.0)],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(0),
            }),
            instruction(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(2)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    );
    let error = Interpreter::new(module(bounds))
        .unwrap()
        .execute_entry(&[cell([1, 1], vec![Value::Double(1.0)])])
        .unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } if matches!(detail.as_ref(), ArrayRuntimeError::IndexOutOfBounds { argument: 0, index: 2, extent: 1 })
    ));
}

#[test]
fn failed_multi_brace_assignment_does_not_publish_a_partial_root() {
    let function = function(
        "rollback",
        4,
        0,
        2,
        2,
        vec![Constant::String("root".to_owned())],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(3),
                root: Register::new(0),
                path: vec![PlaceStep::Brace(vec![ApplyArgument::Value(Register::new(
                    1,
                ))])],
                source: ValueSource::One(Register::new(2)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::StoreGlobal {
                name: ConstantId::new(0),
                src: Register::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
    );
    let original = cell([1, 2], vec![Value::Double(1.0), Value::Double(2.0)]);
    let mut interpreter = Interpreter::new(module(function)).unwrap();
    interpreter.workspace_mut().insert("root", original.clone());
    interpreter
        .workspace_mut()
        .insert("alias", original.clone());
    let error = interpreter
        .execute_entry(&[real_array([1, 2], vec![1.0, 2.0]), Value::Double(9.0)])
        .unwrap_err();
    let RuntimeErrorKind::InvalidExecutionState {
        array: Some(detail),
        ..
    } = error.kind
    else {
        panic!("expected structured assignment error");
    };
    assert!(matches!(
        detail.as_ref(),
        ArrayRuntimeError::AssignmentSizeMismatch {
            selected: 2,
            supplied: 1
        }
    ));
    assert_eq!(interpreter.workspace().get("root"), Some(&original));
    assert!(
        interpreter
            .workspace()
            .get("root")
            .unwrap()
            .shares_storage_with(&original)
    );
    assert_eq!(interpreter.workspace().get("alias"), Some(&original));
    assert!(
        interpreter
            .workspace()
            .get("alias")
            .unwrap()
            .shares_storage_with(&original)
    );
}

#[test]
fn failed_cell_cardinality_and_struct_schema_assignments_preserve_root_and_cow_alias() {
    let cell_root = cell([1, 2], vec![Value::Double(1.0), Value::Double(2.0)]);
    let mut cell_interpreter = Interpreter::new(workspace_paren_assignment_module()).unwrap();
    cell_interpreter
        .workspace_mut()
        .insert("root", cell_root.clone());
    cell_interpreter
        .workspace_mut()
        .insert("alias", cell_root.clone());
    let cell_error = cell_interpreter
        .execute_entry(&[
            cell(
                [1, 3],
                vec![Value::Double(3.0), Value::Double(4.0), Value::Double(5.0)],
            ),
            real_array([1, 2], vec![1.0, 2.0]),
        ])
        .unwrap_err();
    assert!(matches!(
        cell_error.kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } if matches!(
            detail.as_ref(),
            ArrayRuntimeError::AssignmentSizeMismatch {
                selected: 2,
                supplied: 3
            }
        )
    ));
    for name in ["root", "alias"] {
        let value = cell_interpreter.workspace().get(name).unwrap();
        assert_eq!(value, &cell_root);
        assert!(value.shares_storage_with(&cell_root));
    }
    assert!(
        cell_interpreter
            .workspace()
            .get("root")
            .unwrap()
            .shares_storage_with(cell_interpreter.workspace().get("alias").unwrap())
    );

    let struct_root = structure(
        [1, 2],
        &["a"],
        vec![vec![Value::Double(1.0), Value::Double(2.0)]],
    );
    let mut struct_interpreter = Interpreter::new(workspace_paren_assignment_module()).unwrap();
    struct_interpreter
        .workspace_mut()
        .insert("root", struct_root.clone());
    struct_interpreter
        .workspace_mut()
        .insert("alias", struct_root.clone());
    let struct_error = struct_interpreter
        .execute_entry(&[
            structure([1, 1], &["b"], vec![vec![Value::Double(9.0)]]),
            Value::Double(1.0),
        ])
        .unwrap_err();
    assert!(matches!(
        struct_error.kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } if matches!(
            detail.as_ref(),
            ArrayRuntimeError::StructSchemaMismatch { destination, source }
                if destination == &["a"] && source == &["b"]
        )
    ));
    for name in ["root", "alias"] {
        let value = struct_interpreter.workspace().get(name).unwrap();
        assert_eq!(value, &struct_root);
        assert!(value.shares_storage_with(&struct_root));
    }
    assert!(
        struct_interpreter
            .workspace()
            .get("root")
            .unwrap()
            .shares_storage_with(struct_interpreter.workspace().get("alias").unwrap())
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn struct_assignment_aligns_fields_and_struct_member_apply_is_separate_from_objects() {
    let assign = function(
        "struct-assign",
        4,
        0,
        3,
        3,
        Vec::new(),
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(2),
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(3),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    2,
                ))])],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
    );
    let destination = structure(
        [1, 2],
        &["a", "b"],
        vec![
            vec![Value::Double(1.0), Value::Double(2.0)],
            vec![Value::Double(10.0), Value::Double(20.0)],
        ],
    );
    let source = structure(
        [1, 1],
        &["b", "a"],
        vec![vec![Value::Double(90.0)], vec![Value::Double(9.0)]],
    );
    let assigned = Interpreter::new(module(assign))
        .unwrap()
        .execute_entry(&[destination, source, Value::Double(2.0)])
        .unwrap()
        .remove(0);
    let Value::Struct(assigned) = assigned else {
        panic!("expected assigned struct");
    };
    assert_eq!(
        assigned
            .field_names()
            .iter()
            .map(FieldName::as_str)
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(assigned.value_at_name("a", 1), Some(&Value::Double(9.0)));
    assert_eq!(assigned.value_at_name("b", 1), Some(&Value::Double(90.0)));

    let apply = function(
        "struct-apply",
        3,
        0,
        2,
        2,
        vec![Constant::String("f".to_owned())],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            instruction(InstructionKind::ApplyField {
                outputs: vec![Register::new(2)],
                object: Register::new(0),
                name: ConstantId::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    );
    let mut registry = BuiltinRegistry::new();
    let handle = registry.register("identity", identity_builtin).unwrap();
    let receiver = structure(
        [1, 1],
        &["f"],
        vec![vec![Value::Function(FunctionHandle::Builtin(handle))]],
    );
    let values = Interpreter::with_registry(module(apply.clone()), registry)
        .unwrap()
        .execute_entry(&[receiver, Value::Double(7.0)])
        .unwrap();
    assert_eq!(values, vec![Value::Double(7.0)]);

    let non_scalar = structure(
        [1, 2],
        &["f"],
        vec![vec![
            Value::Function(FunctionHandle::Builtin(handle)),
            Value::Function(FunctionHandle::Builtin(handle)),
        ]],
    );
    let mut registry = BuiltinRegistry::new();
    registry.register("identity", identity_builtin).unwrap();
    let error = Interpreter::with_registry(module(apply), registry)
        .unwrap()
        .execute_entry(&[non_scalar, Value::Double(7.0)])
        .unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } if matches!(detail.as_ref(), ArrayRuntimeError::StructMemberApplyRequiresScalar { numel: 2 })
    ));
}

#[test]
fn standalone_builtin_context_rejects_object_copy_without_runtime_service() {
    let cancellation = openmat_runtime::CancellationToken::new();
    let mut output = openmat_runtime::NullOutput;
    let mut context = openmat_runtime::BuiltinContext::new(1, &cancellation, &mut output);
    let error = context
        .language_copy(&Value::Object(openmat_value::ObjectHandle::new(7)))
        .unwrap_err();
    assert_eq!(error.category, BuiltinErrorCategory::LanguageCopy);
    assert_eq!(
        context.language_copy(&Value::Double(1.0)),
        Ok(Value::Double(1.0))
    );

    let aggregate = cell(
        [1, 1],
        vec![structure(
            [1, 1],
            &["payload"],
            vec![vec![real_array([1, 1], vec![3.0])]],
        )],
    );
    let copied = context.language_copy(&aggregate).unwrap();
    assert!(aggregate.shares_storage_with(&copied));
}

#[test]
#[allow(clippy::too_many_lines)]
fn aggregate_object_dispatch_preserves_value_handle_method_and_array_semantics() {
    let mut values = Interpreter::new(aggregate_class_module(
        "ValueBox",
        ClassSemantics::Value,
        Access::Public,
    ))
    .unwrap();
    values.execute_entry(&[]).unwrap();
    values
        .replace_module(construct_class_module("ValueBox"))
        .unwrap();
    let original = values.execute_entry(&[]).unwrap().remove(0);
    values.workspace_mut().insert("source", original.clone());
    values
        .replace_module(object_aggregate_module("source"))
        .unwrap();
    let returned = values.execute_entry(&[]).unwrap();
    let (
        Value::Object(original_handle),
        Value::Object(assigned_handle),
        Value::Object(method_handle),
    ) = (&returned[0], &returned[2], &returned[5])
    else {
        panic!("value-class aggregate dispatch must return scalar objects");
    };
    assert_ne!(original_handle, assigned_handle);
    assert_ne!(original_handle, method_handle);
    assert_ne!(assigned_handle, method_handle);
    assert_eq!(
        [&returned[1], &returned[3], &returned[4], &returned[6]],
        [
            &Value::Double(0.0),
            &Value::Double(4.0),
            &Value::Double(0.0),
            &Value::Double(4.0),
        ]
    );
    let original_handle = *original_handle;

    let scalar_array = Value::ObjectArray(
        ObjectArray::from_vec(
            ClassHandle::new(1),
            Shape::new([1, 1]).unwrap(),
            vec![original_handle],
        )
        .unwrap(),
    );
    values.workspace_mut().insert("scalar_array", scalar_array);
    values
        .replace_module(object_aggregate_read_module("scalar_array"))
        .unwrap();
    assert_eq!(values.execute_entry(&[]).unwrap(), vec![Value::Double(0.0)]);
    values
        .workspace_mut()
        .insert("nested", cell([1, 1], vec![Value::Object(original_handle)]));
    values
        .replace_module(nested_object_assign_module("nested"))
        .unwrap();
    let nested = values.execute_entry(&[]).unwrap();
    let (Value::Object(updated), Value::Object(original)) = (&nested[1], &nested[3]) else {
        panic!("nested value-class write-back must return scalar objects");
    };
    assert_ne!(updated, original);
    assert_eq!(
        (&nested[2], &nested[4]),
        (&Value::Double(9.0), &Value::Double(0.0))
    );

    let mut handles = Interpreter::new(aggregate_class_module(
        "HandleBox",
        ClassSemantics::Handle,
        Access::Public,
    ))
    .unwrap();
    handles.execute_entry(&[]).unwrap();
    handles
        .replace_module(construct_class_module("HandleBox"))
        .unwrap();
    let original = handles.execute_entry(&[]).unwrap().remove(0);
    handles.workspace_mut().insert("source", original);
    handles
        .replace_module(object_aggregate_module("source"))
        .unwrap();
    let returned = handles.execute_entry(&[]).unwrap();
    let (
        Value::Object(original_handle),
        Value::Object(assigned_handle),
        Value::Object(method_handle),
    ) = (&returned[0], &returned[2], &returned[5])
    else {
        panic!("handle-class aggregate dispatch must return scalar objects");
    };
    assert_eq!(original_handle, assigned_handle);
    assert_eq!(original_handle, method_handle);
    assert_eq!(
        [&returned[1], &returned[3], &returned[4], &returned[6]],
        [
            &Value::Double(0.0),
            &Value::Double(4.0),
            &Value::Double(4.0),
            &Value::Double(8.0),
        ]
    );
    let handle = *original_handle;
    handles
        .workspace_mut()
        .insert("nested", cell([1, 1], vec![Value::Object(handle)]));
    handles
        .replace_module(nested_object_assign_module("nested"))
        .unwrap();
    let nested = handles.execute_entry(&[]).unwrap();
    assert_eq!(nested[1], Value::Object(handle));
    assert_eq!(nested[3], Value::Object(handle));
    assert_eq!(
        (&nested[2], &nested[4]),
        (&Value::Double(9.0), &Value::Double(9.0))
    );
}

#[test]
fn aggregate_object_dispatch_preserves_class_access_control() {
    let mut interpreter = Interpreter::new(aggregate_class_module(
        "PrivateBox",
        ClassSemantics::Value,
        Access::Private,
    ))
    .unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(construct_class_module("PrivateBox"))
        .unwrap();
    let object = interpreter.execute_entry(&[]).unwrap().remove(0);
    interpreter.workspace_mut().insert("source", object);
    interpreter
        .replace_module(object_aggregate_read_module("source"))
        .unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::AccessViolation {
            operation: "property read",
            member,
            required: openmat_object::Access::Private,
        } if member == "value"
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn class_handle_aggregate_fields_read_only_constants_without_allocating_instances() {
    let mut interpreter = Interpreter::new(aggregate_constant_class_module("StaticBox")).unwrap();
    interpreter.execute_entry(&[]).unwrap();
    assert_eq!(interpreter.class_count(), 2);
    assert_eq!(interpreter.object_count(), 0);

    let success = function(
        "class-constant-success",
        6,
        2,
        0,
        0,
        vec![
            Constant::String("StaticBox".to_owned()),
            Constant::String("Factor".to_owned()),
            Constant::String("scale".to_owned()),
            Constant::Double(5.0),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                field: FieldOperand::Static(ConstantId::new(1)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(1)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(1),
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(1),
                target: Register::new(0),
                field: FieldOperand::Dynamic(Register::new(2)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(3)],
                pack: PackRegister::new(1),
            }),
            instruction(InstructionKind::LoadConstant {
                dst: Register::new(4),
                constant: ConstantId::new(3),
            }),
            instruction(InstructionKind::ApplyField {
                outputs: vec![Register::new(5)],
                object: Register::new(0),
                name: ConstantId::new(2),
                arguments: vec![ApplyArgument::Value(Register::new(4))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(3), Register::new(5)],
            }),
        ],
    );
    interpreter.replace_module(module(success)).unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(3.0), Value::Double(3.0), Value::Double(15.0)]
    );
    assert_eq!(interpreter.object_count(), 0);

    interpreter
        .replace_module(class_aggregate_field_module("StaticBox", "Stored", false))
        .unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Object {
            operation: "class property read",
            message,
        } if message.contains("stored instance property `Stored`")
    ));

    interpreter
        .replace_module(class_aggregate_field_module("StaticBox", "Missing", false))
        .unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Object {
            operation: "class property read",
            message,
        } if message.contains("Missing")
    ));

    interpreter
        .replace_module(class_aggregate_field_module("StaticBox", "Secret", true))
        .unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::AccessViolation {
            operation: "class property read",
            member,
            required: openmat_object::Access::Private,
        } if member == "Secret"
    ));

    let ordinary_function = function(
        "ordinary-function-aggregate-field",
        2,
        1,
        1,
        1,
        vec![Constant::String("Factor".to_owned())],
        vec![
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            instruction(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                field: FieldOperand::Static(ConstantId::new(0)),
            }),
            instruction(InstructionKind::Unpack {
                outputs: vec![Register::new(1)],
                pack: PackRegister::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
    );
    interpreter
        .replace_module(module(ordinary_function))
        .unwrap();
    let error = interpreter
        .execute_entry(&[Value::Function(FunctionHandle::Builtin(
            BuiltinHandle::new(99),
        ))])
        .unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } if matches!(
            detail.as_ref(),
            ArrayRuntimeError::InvalidOperand {
                operation: "aggregate field read",
                actual: openmat_value::ValueKind::Function,
            }
        )
    ));
    assert_eq!(interpreter.object_count(), 0);
}

#[test]
fn failed_object_field_assignment_rolls_back_the_offline_value_receiver() {
    let mut interpreter = Interpreter::new(aggregate_class_module(
        "RollbackBox",
        ClassSemantics::Value,
        Access::Public,
    ))
    .unwrap();
    interpreter.execute_entry(&[]).unwrap();
    interpreter
        .replace_module(construct_class_module("RollbackBox"))
        .unwrap();
    let root = interpreter.execute_entry(&[]).unwrap().remove(0);
    interpreter.workspace_mut().insert("root", root.clone());
    interpreter
        .workspace_mut()
        .insert("bad", Value::Object(ObjectHandle::new(u64::MAX)));
    let assign = function(
        "failed-object-field",
        3,
        0,
        0,
        0,
        vec![
            Constant::String("root".to_owned()),
            Constant::String("bad".to_owned()),
            Constant::String("value".to_owned()),
        ],
        vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(1),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            instruction(InstructionKind::AssignPlace {
                dst: Register::new(2),
                root: Register::new(0),
                path: vec![PlaceStep::Field(FieldOperand::Static(ConstantId::new(2)))],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Store,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    );
    interpreter.replace_module(module(assign)).unwrap();
    let before = interpreter.object_count();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(error.kind, RuntimeErrorKind::Object { .. }));
    assert_eq!(interpreter.object_count(), before);
    assert_eq!(interpreter.workspace().get("root"), Some(&root));

    interpreter
        .replace_module(object_aggregate_read_module("root"))
        .unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        vec![Value::Double(0.0)]
    );
}

#[test]
fn builtin_language_copy_is_atomic_and_obeys_object_assignment_semantics() {
    let mut registry = BuiltinRegistry::new();
    registry.register("copy_twice", copy_twice_success).unwrap();
    registry
        .register("copy_twice_then_fail", copy_twice_then_fail)
        .unwrap();
    let mut values = Interpreter::with_registry(
        aggregate_class_module("BuiltinValue", ClassSemantics::Value, Access::Public),
        registry,
    )
    .unwrap();
    values.execute_entry(&[]).unwrap();
    values
        .replace_module(construct_class_module("BuiltinValue"))
        .unwrap();
    let source = values.execute_entry(&[]).unwrap().remove(0);
    values.workspace_mut().insert("source", source.clone());
    let before = values.object_count();

    values
        .replace_module(builtin_copy_module("copy_twice_then_fail"))
        .unwrap();
    let error = values.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Builtin {
            category: BuiltinErrorCategory::Other,
            ..
        }
    ));
    assert_eq!(values.object_count(), before);
    assert_eq!(values.workspace().get("source"), Some(&source));

    values
        .replace_module(builtin_copy_module("copy_twice"))
        .unwrap();
    let copied = values.execute_entry(&[]).unwrap();
    let (Value::Object(source_handle), Value::Object(first), Value::Object(second)) =
        (&source, &copied[0], &copied[1])
    else {
        panic!("value-class built-in copies must remain scalar objects");
    };
    assert_ne!(source_handle, first);
    assert_ne!(source_handle, second);
    assert_ne!(first, second);
    assert_eq!(values.object_count(), before + 2);

    let mut registry = BuiltinRegistry::new();
    registry.register("copy_twice", copy_twice_success).unwrap();
    let mut handles = Interpreter::with_registry(
        aggregate_class_module("BuiltinHandle", ClassSemantics::Handle, Access::Public),
        registry,
    )
    .unwrap();
    handles.execute_entry(&[]).unwrap();
    handles
        .replace_module(construct_class_module("BuiltinHandle"))
        .unwrap();
    let source = handles.execute_entry(&[]).unwrap().remove(0);
    handles.workspace_mut().insert("source", source.clone());
    let before = handles.object_count();
    handles
        .replace_module(builtin_copy_module("copy_twice"))
        .unwrap();
    assert_eq!(
        handles.execute_entry(&[]).unwrap(),
        vec![source.clone(), source]
    );
    assert_eq!(handles.object_count(), before);
}

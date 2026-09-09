use openmat_array::{
    ArrayData, Complex64 as ArrayComplex64, ComplexInteger, DenseArray, IntegerArrayData, Shape,
};
use openmat_bytecode::{
    ApplyArgument, BinaryOperator, BytecodeModule, Constant, ConstantId, Function, FunctionId,
    Instruction, InstructionIndex, InstructionKind, LocalSlot, Register, SourceLocation,
};
use openmat_runtime::{ArrayRuntimeError, IndexOutOfBoundsDetails, Interpreter, RuntimeErrorKind};
use openmat_value::{StringArray, StringElement, StringValue, Value};

fn instruction(kind: InstructionKind) -> Instruction {
    Instruction::new(kind)
}

fn load(dst: u32, constant: u32) -> Instruction {
    instruction(InstructionKind::LoadConstant {
        dst: Register::new(dst),
        constant: ConstantId::new(constant),
    })
}

fn function(
    name: &str,
    register_count: u32,
    constants: Vec<Constant>,
    instructions: Vec<Instruction>,
) -> Function {
    Function {
        name: name.to_owned(),
        register_count,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants,
        instructions,
        exception_handlers: Vec::new(),
    }
}

fn execute(functions: Vec<Function>) -> (Interpreter, Vec<Value>) {
    let mut interpreter = Interpreter::new(BytecodeModule::new(functions, FunctionId::new(0)))
        .expect("test bytecode should verify");
    let values = interpreter
        .execute_entry(&[])
        .expect("test bytecode should execute");
    (interpreter, values)
}

fn real_array(value: &Value) -> (&[u64], &[f64]) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected real array, found {value:?}");
    };
    (array.shape().dimensions(), array.as_slice())
}

fn complex_array(value: &Value) -> (&[u64], &[ArrayComplex64]) {
    let Value::Array(ArrayData::ComplexF64(array)) = value else {
        panic!("expected complex array, found {value:?}");
    };
    (array.shape().dimensions(), array.as_slice())
}

#[test]
fn builds_empty_one_by_one_two_by_three_and_reverse_range_arrays() {
    let constants = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, -2.0]
        .into_iter()
        .map(Constant::Double)
        .collect();
    let mut instructions = (0..7).map(|index| load(index, index)).collect::<Vec<_>>();
    instructions.extend([
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(7),
            rows: Vec::new(),
        }),
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(8),
            rows: vec![vec![Register::new(0)]],
        }),
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(9),
            rows: vec![
                vec![Register::new(0), Register::new(1), Register::new(2)],
                vec![Register::new(3), Register::new(4), Register::new(5)],
            ],
        }),
        instruction(InstructionKind::Range {
            dst: Register::new(10),
            start: Register::new(4),
            step: Register::new(6),
            end: Register::new(0),
        }),
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(11),
            rows: vec![vec![Register::new(7), Register::new(0)]],
        }),
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::Add,
            dst: Register::new(12),
            lhs: Register::new(7),
            rhs: Register::new(0),
        }),
        instruction(InstructionKind::Return {
            values: vec![
                Register::new(7),
                Register::new(8),
                Register::new(9),
                Register::new(10),
                Register::new(11),
                Register::new(12),
            ],
        }),
    ]);
    let (_, values) = execute(vec![function("arrays", 13, constants, instructions)]);

    assert_eq!(real_array(&values[0]), (&[0, 0][..], &[][..]));
    assert_eq!(real_array(&values[1]), (&[1, 1][..], &[1.0][..]));
    assert_eq!(
        real_array(&values[2]),
        (&[2, 3][..], &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0][..])
    );
    assert_eq!(real_array(&values[3]), (&[1, 3][..], &[5.0, 3.0, 1.0][..]));
    assert_eq!(real_array(&values[4]), (&[1, 1][..], &[1.0][..]));
    assert_eq!(real_array(&values[5]), (&[0, 0][..], &[][..]));
}

#[test]
fn builds_owning_string_arrays_from_matrix_literals() {
    let string_matrix = function(
        "strings",
        3,
        vec![
            Constant::String("alpha".to_owned()),
            Constant::String("beta".to_owned()),
        ],
        vec![
            load(0, 0),
            load(1, 1),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(2),
                rows: vec![vec![Register::new(0), Register::new(1)]],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    );
    let (_, values) = execute(vec![string_matrix]);
    let value = &values[0];
    let array = value
        .as_string_array()
        .expect("matrix literal must produce explicit string-array storage");

    assert_eq!(value.class_name(), "string");
    assert_eq!(value.dimensions(), Some([1, 2].as_slice()));
    assert_eq!(value.numel(), Some(2));
    assert_eq!(array.as_slice()[0].code_units(), &[97, 108, 112, 104, 97]);
    assert_eq!(array.as_slice()[1].code_units(), &[98, 101, 116, 97]);

    let mut assigned = value.clone();
    assert!(value.shares_array_storage_with(&assigned));
    assigned
        .as_string_array_mut()
        .unwrap()
        .replace_linear(2, "changed")
        .unwrap();
    assert!(!value.shares_array_storage_with(&assigned));
    assert_eq!(
        value.as_string_array().unwrap().as_slice()[1].code_units(),
        &[98, 101, 116, 97]
    );
    assert_eq!(
        assigned.as_string_array().unwrap().as_slice()[1].code_units(),
        &[99, 104, 97, 110, 103, 101, 100]
    );
}

#[test]
fn mixed_string_and_numeric_matrix_literals_return_structured_errors() {
    let mixed = function(
        "mixed_strings",
        3,
        vec![Constant::String("alpha".to_owned()), Constant::Double(1.0)],
        vec![
            load(0, 0),
            load(1, 1),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(2),
                rows: vec![vec![Register::new(0), Register::new(1)]],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    );
    let mut interpreter = Interpreter::new(BytecodeModule::new(vec![mixed], FunctionId::new(0)))
        .expect("mixed matrix bytecode should verify");
    let error = interpreter
        .execute_entry(&[])
        .expect_err("mixed string and numeric concatenation must fail");
    let RuntimeErrorKind::InvalidExecutionState {
        array: Some(detail),
        ..
    } = error.kind
    else {
        panic!("expected a structured concatenation error");
    };
    assert!(matches!(
        detail.as_ref(),
        ArrayRuntimeError::InvalidOperand {
            operation: "string matrix concatenation",
            ..
        }
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn floating_ranges_match_r2022b_shape_and_exact_values() {
    let constants = [
        0.0,
        0.1,
        0.3,
        -0.1,
        0.29,
        0.5,
        f64::from_bits(0x3fd3_3333_3333_3332),
        f64::from_bits(0x3fd3_3333_3333_3331),
    ]
    .into_iter()
    .map(Constant::Double)
    .collect();
    let mut instructions = (0..8).map(|index| load(index, index)).collect::<Vec<_>>();
    instructions.extend([
        instruction(InstructionKind::Range {
            dst: Register::new(8),
            start: Register::new(0),
            step: Register::new(1),
            end: Register::new(2),
        }),
        instruction(InstructionKind::Range {
            dst: Register::new(9),
            start: Register::new(2),
            step: Register::new(3),
            end: Register::new(0),
        }),
        instruction(InstructionKind::Range {
            dst: Register::new(10),
            start: Register::new(0),
            step: Register::new(1),
            end: Register::new(4),
        }),
        instruction(InstructionKind::Range {
            dst: Register::new(11),
            start: Register::new(1),
            step: Register::new(1),
            end: Register::new(5),
        }),
        instruction(InstructionKind::Range {
            dst: Register::new(12),
            start: Register::new(0),
            step: Register::new(1),
            end: Register::new(6),
        }),
        instruction(InstructionKind::Range {
            dst: Register::new(13),
            start: Register::new(0),
            step: Register::new(1),
            end: Register::new(7),
        }),
        instruction(InstructionKind::Return {
            values: vec![
                Register::new(8),
                Register::new(9),
                Register::new(10),
                Register::new(11),
                Register::new(12),
                Register::new(13),
            ],
        }),
    ]);
    let (_, values) = execute(vec![function(
        "floating_ranges",
        14,
        constants,
        instructions,
    )]);

    let expected = [
        (
            &[1, 4][..],
            &[
                0x0000_0000_0000_0000,
                0x3fb9_9999_9999_999a,
                0x3fc9_9999_9999_9999,
                0x3fd3_3333_3333_3333,
            ][..],
        ),
        (
            &[1, 4][..],
            &[
                0x3fd3_3333_3333_3333,
                0x3fc9_9999_9999_9999,
                0x3fb9_9999_9999_999a,
                0x0000_0000_0000_0000,
            ][..],
        ),
        (
            &[1, 3][..],
            &[
                0x0000_0000_0000_0000,
                0x3fb9_9999_9999_999a,
                0x3fc9_9999_9999_999a,
            ][..],
        ),
        (
            &[1, 5][..],
            &[
                0x3fb9_9999_9999_999a,
                0x3fc9_9999_9999_999a,
                0x3fd3_3333_3333_3334,
                0x3fd9_9999_9999_999a,
                0x3fe0_0000_0000_0000,
            ][..],
        ),
        (
            &[1, 4][..],
            &[
                0x0000_0000_0000_0000,
                0x3fb9_9999_9999_999a,
                0x3fc9_9999_9999_9997,
                0x3fd3_3333_3333_3332,
            ][..],
        ),
        (
            &[1, 3][..],
            &[
                0x0000_0000_0000_0000,
                0x3fb9_9999_9999_999a,
                0x3fc9_9999_9999_999a,
            ][..],
        ),
    ];
    for (value, (expected_shape, expected_bits)) in values.iter().zip(expected) {
        let (shape, elements) = real_array(value);
        assert_eq!(shape, expected_shape);
        assert_eq!(
            elements
                .iter()
                .map(|element| element.to_bits())
                .collect::<Vec<_>>(),
            expected_bits
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn runtime_apply_dispatches_calls_and_all_index_forms() {
    let mut constants = (1..=6)
        .map(|value| Constant::Double(f64::from(value)))
        .collect::<Vec<_>>();
    constants.extend([
        Constant::Function(FunctionId::new(1)),
        Constant::Double(9.0),
    ]);
    let mut instructions = (0..6).map(|index| load(index, index)).collect::<Vec<_>>();
    instructions.extend([
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(6),
            rows: vec![
                vec![Register::new(0), Register::new(1), Register::new(2)],
                vec![Register::new(3), Register::new(4), Register::new(5)],
            ],
        }),
        instruction(InstructionKind::Apply {
            outputs: vec![Register::new(7)],
            target: Register::new(6),
            arguments: vec![ApplyArgument::Value(Register::new(3))],
        }),
        instruction(InstructionKind::Apply {
            outputs: vec![Register::new(8)],
            target: Register::new(6),
            arguments: vec![
                ApplyArgument::Value(Register::new(1)),
                ApplyArgument::Value(Register::new(2)),
            ],
        }),
        instruction(InstructionKind::Apply {
            outputs: vec![Register::new(9)],
            target: Register::new(6),
            arguments: vec![ApplyArgument::Colon, ApplyArgument::Value(Register::new(1))],
        }),
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::GreaterThan,
            dst: Register::new(10),
            lhs: Register::new(6),
            rhs: Register::new(2),
        }),
        instruction(InstructionKind::Apply {
            outputs: vec![Register::new(11)],
            target: Register::new(6),
            arguments: vec![ApplyArgument::Value(Register::new(10))],
        }),
        instruction(InstructionKind::Range {
            dst: Register::new(12),
            start: Register::new(1),
            step: Register::new(0),
            end: Register::new(3),
        }),
        instruction(InstructionKind::Apply {
            outputs: vec![Register::new(13)],
            target: Register::new(6),
            arguments: vec![ApplyArgument::Value(Register::new(12))],
        }),
        load(14, 6),
        load(15, 7),
        instruction(InstructionKind::Apply {
            outputs: vec![Register::new(16)],
            target: Register::new(14),
            arguments: vec![ApplyArgument::Value(Register::new(15))],
        }),
        instruction(InstructionKind::Range {
            dst: Register::new(17),
            start: Register::new(0),
            step: Register::new(0),
            end: Register::new(3),
        }),
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::GreaterThan,
            dst: Register::new(18),
            lhs: Register::new(17),
            rhs: Register::new(1),
        }),
        instruction(InstructionKind::Apply {
            outputs: vec![Register::new(19)],
            target: Register::new(17),
            arguments: vec![ApplyArgument::Value(Register::new(18))],
        }),
        instruction(InstructionKind::Return {
            values: vec![
                Register::new(7),
                Register::new(8),
                Register::new(9),
                Register::new(11),
                Register::new(13),
                Register::new(16),
                Register::new(19),
            ],
        }),
    ]);
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
    let (_, values) = execute(vec![
        function("apply", 20, constants, instructions),
        identity,
    ]);

    assert_eq!(values[0], Value::Double(5.0));
    assert_eq!(values[1], Value::Double(6.0));
    assert_eq!(real_array(&values[2]), (&[2, 1][..], &[2.0, 5.0][..]));
    assert_eq!(real_array(&values[3]), (&[3, 1][..], &[4.0, 5.0, 6.0][..]));
    assert_eq!(real_array(&values[4]), (&[1, 3][..], &[4.0, 2.0, 5.0][..]));
    assert_eq!(values[5], Value::Double(9.0));
    assert_eq!(real_array(&values[6]), (&[1, 2][..], &[3.0, 4.0][..]));
}

#[test]
fn indexed_assignment_detaches_cow_for_colon_and_logical_indices() {
    let constants = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 99.0, 7.0, 8.0, 0.0]
        .into_iter()
        .map(Constant::Double)
        .collect();
    let mut instructions = (0..10).map(|index| load(index, index)).collect::<Vec<_>>();
    instructions.extend([
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(10),
            rows: vec![
                vec![Register::new(0), Register::new(1), Register::new(2)],
                vec![Register::new(3), Register::new(4), Register::new(5)],
            ],
        }),
        instruction(InstructionKind::Move {
            dst: Register::new(11),
            src: Register::new(10),
        }),
        instruction(InstructionKind::IndexAssign {
            dst: Register::new(12),
            target: Register::new(11),
            arguments: vec![
                ApplyArgument::Value(Register::new(0)),
                ApplyArgument::Value(Register::new(1)),
            ],
            value: Register::new(6),
        }),
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(13),
            rows: vec![vec![Register::new(7)], vec![Register::new(8)]],
        }),
        instruction(InstructionKind::IndexAssign {
            dst: Register::new(14),
            target: Register::new(12),
            arguments: vec![ApplyArgument::Colon, ApplyArgument::Value(Register::new(2))],
            value: Register::new(13),
        }),
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::GreaterThan,
            dst: Register::new(15),
            lhs: Register::new(10),
            rhs: Register::new(3),
        }),
        instruction(InstructionKind::IndexAssign {
            dst: Register::new(16),
            target: Register::new(10),
            arguments: vec![ApplyArgument::Value(Register::new(15))],
            value: Register::new(9),
        }),
        instruction(InstructionKind::Return {
            values: vec![Register::new(10), Register::new(14), Register::new(16)],
        }),
    ]);
    let (_, values) = execute(vec![function("assign", 17, constants, instructions)]);

    assert!(!values[0].shares_array_storage_with(&values[1]));
    assert!(!values[0].shares_array_storage_with(&values[2]));
    assert_eq!(real_array(&values[0]).1, &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    assert_eq!(real_array(&values[1]).1, &[1.0, 4.0, 99.0, 5.0, 7.0, 8.0]);
    assert_eq!(real_array(&values[2]).1, &[1.0, 4.0, 2.0, 0.0, 3.0, 0.0]);
}

#[test]
#[allow(clippy::too_many_lines)]
fn transposes_complex_arrays_and_executes_array_arithmetic() {
    let constants = vec![
        Constant::Complex {
            real: 1.0,
            imaginary: 2.0,
        },
        Constant::Complex {
            real: 3.0,
            imaginary: -4.0,
        },
        Constant::Complex {
            real: 0.0,
            imaginary: 5.0,
        },
        Constant::Double(6.0),
    ];
    let mut instructions = (0..4).map(|index| load(index, index)).collect::<Vec<_>>();
    instructions.extend([
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(4),
            rows: vec![
                vec![Register::new(0), Register::new(1)],
                vec![Register::new(2), Register::new(3)],
            ],
        }),
        instruction(InstructionKind::Transpose {
            dst: Register::new(5),
            operand: Register::new(4),
            conjugate: true,
        }),
        instruction(InstructionKind::Transpose {
            dst: Register::new(6),
            operand: Register::new(4),
            conjugate: false,
        }),
        instruction(InstructionKind::Return {
            values: vec![Register::new(5), Register::new(6)],
        }),
    ]);
    let (_, values) = execute(vec![function("transpose", 7, constants, instructions)]);
    assert_eq!(
        complex_array(&values[0]),
        (
            &[2, 2][..],
            &[
                ArrayComplex64::new(1.0, -2.0),
                ArrayComplex64::new(3.0, 4.0),
                ArrayComplex64::new(0.0, -5.0),
                ArrayComplex64::new(6.0, 0.0),
            ][..]
        )
    );
    assert_eq!(
        complex_array(&values[1]).1,
        &[
            ArrayComplex64::new(1.0, 2.0),
            ArrayComplex64::new(3.0, -4.0),
            ArrayComplex64::new(0.0, 5.0),
            ArrayComplex64::new(6.0, 0.0),
        ]
    );

    let constants = (1..=10)
        .map(|value| Constant::Double(f64::from(value)))
        .collect();
    let mut arithmetic = (0..10).map(|index| load(index, index)).collect::<Vec<_>>();
    arithmetic.extend([
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(10),
            rows: vec![
                vec![Register::new(0), Register::new(1)],
                vec![Register::new(2), Register::new(3)],
            ],
        }),
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(11),
            rows: vec![
                vec![Register::new(4), Register::new(5)],
                vec![Register::new(6), Register::new(7)],
            ],
        }),
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::Multiply,
            dst: Register::new(12),
            lhs: Register::new(10),
            rhs: Register::new(11),
        }),
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::Add,
            dst: Register::new(13),
            lhs: Register::new(10),
            rhs: Register::new(9),
        }),
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::ElementPower,
            dst: Register::new(14),
            lhs: Register::new(10),
            rhs: Register::new(1),
        }),
        instruction(InstructionKind::Return {
            values: vec![Register::new(12), Register::new(13), Register::new(14)],
        }),
    ]);
    let (_, values) = execute(vec![function("arithmetic", 15, constants, arithmetic)]);
    assert_eq!(real_array(&values[0]).1, &[19.0, 43.0, 22.0, 50.0]);
    assert_eq!(real_array(&values[1]).1, &[11.0, 13.0, 12.0, 14.0]);
    assert_eq!(real_array(&values[2]).1, &[1.0, 9.0, 4.0, 16.0]);
}

#[test]
fn for_each_iterates_ranges_and_array_columns() {
    let range = function(
        "range_for",
        7,
        vec![
            Constant::Double(5.0),
            Constant::Double(-2.0),
            Constant::Double(1.0),
            Constant::Double(0.0),
        ],
        vec![
            load(0, 0),
            load(1, 1),
            load(2, 2),
            instruction(InstructionKind::Range {
                dst: Register::new(3),
                start: Register::new(0),
                step: Register::new(1),
                end: Register::new(2),
            }),
            load(4, 3),
            load(5, 3),
            instruction(InstructionKind::ForEach {
                iterable: Register::new(3),
                index: Register::new(5),
                dst: Register::new(6),
                exit: InstructionIndex::new(9),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(4),
                lhs: Register::new(4),
                rhs: Register::new(6),
            }),
            instruction(InstructionKind::Jump {
                target: InstructionIndex::new(6),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4)],
            }),
        ],
    );
    let (_, values) = execute(vec![range]);
    assert_eq!(values, [Value::Double(9.0)]);

    let constants = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0]
        .into_iter()
        .map(Constant::Double)
        .collect();
    let mut columns = (0..7).map(|index| load(index, index)).collect::<Vec<_>>();
    columns.extend([
        instruction(InstructionKind::BuildMatrix {
            dst: Register::new(7),
            rows: vec![
                vec![Register::new(0), Register::new(1), Register::new(2)],
                vec![Register::new(3), Register::new(4), Register::new(5)],
            ],
        }),
        load(8, 6),
        load(9, 6),
        instruction(InstructionKind::ForEach {
            iterable: Register::new(7),
            index: Register::new(9),
            dst: Register::new(10),
            exit: InstructionIndex::new(14),
        }),
        instruction(InstructionKind::Apply {
            outputs: vec![Register::new(11)],
            target: Register::new(10),
            arguments: vec![ApplyArgument::Value(Register::new(0))],
        }),
        instruction(InstructionKind::Binary {
            operator: BinaryOperator::Add,
            dst: Register::new(8),
            lhs: Register::new(8),
            rhs: Register::new(11),
        }),
        instruction(InstructionKind::Jump {
            target: InstructionIndex::new(10),
        }),
        instruction(InstructionKind::Return {
            values: vec![Register::new(8)],
        }),
    ]);
    let (_, values) = execute(vec![function("column_for", 12, constants, columns)]);
    assert_eq!(values, [Value::Double(6.0)]);
}

#[test]
fn out_of_bounds_and_concatenation_errors_are_structured_and_located() {
    let location = SourceLocation::new(44, 10, 14);
    let bad_index = function(
        "bad_index",
        4,
        vec![
            Constant::Double(1.0),
            Constant::Double(2.0),
            Constant::Double(3.0),
        ],
        vec![
            load(0, 0),
            load(1, 1),
            load(2, 2),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(3),
                rows: vec![vec![Register::new(0), Register::new(1)]],
            }),
            Instruction::located(
                InstructionKind::Apply {
                    outputs: vec![Register::new(0)],
                    target: Register::new(3),
                    arguments: vec![ApplyArgument::Value(Register::new(2))],
                },
                location,
            ),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
    );
    let mut interpreter =
        Interpreter::new(BytecodeModule::new(vec![bad_index], FunctionId::new(0)))
            .expect("error bytecode should verify");
    let error = interpreter
        .execute_entry(&[])
        .expect_err("out-of-bounds index should fail");
    let RuntimeErrorKind::InvalidExecutionState {
        array: Some(detail),
        ..
    } = &error.kind
    else {
        panic!("expected structured array error");
    };
    assert!(matches!(
        detail.as_ref(),
        ArrayRuntimeError::IndexOutOfBounds {
            argument: 0,
            index: 3,
            extent: 2,
        }
    ));
    assert_eq!(
        error.index_out_of_bounds(),
        Some(IndexOutOfBoundsDetails {
            argument: 0,
            index: 3,
            extent: 2,
        })
    );
    assert_eq!(error.location, Some(location));

    let function = function(
        "bad_concat",
        4,
        vec![
            Constant::Double(1.0),
            Constant::Double(2.0),
            Constant::Double(3.0),
        ],
        vec![
            load(0, 0),
            load(1, 1),
            load(2, 2),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(3),
                rows: vec![
                    vec![Register::new(0), Register::new(1)],
                    vec![Register::new(2)],
                ],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
    );
    let mut interpreter = Interpreter::new(BytecodeModule::new(vec![function], FunctionId::new(0)))
        .expect("error bytecode should verify");
    let error = interpreter
        .execute_entry(&[])
        .expect_err("ragged concatenation should fail");
    let RuntimeErrorKind::InvalidExecutionState {
        array: Some(detail),
        ..
    } = error.kind
    else {
        panic!("expected structured concatenation error");
    };
    assert!(matches!(
        detail.as_ref(),
        ArrayRuntimeError::ConcatenationMismatch { axis: 1, .. }
    ));
}

#[test]
fn exact_char_constants_concat_index_assignment_transpose_and_string_promotion() {
    let function = function(
        "exact_chars",
        12,
        vec![
            Constant::Char(Vec::new()),
            Constant::Char(vec![97, 98]),
            Constant::Char(vec![0xd83d, 0xde42]),
            Constant::Char(vec![99, 100]),
            Constant::Double(3.0),
            Constant::Char(vec![90]),
            Constant::String("tail".to_owned()),
            Constant::Char(vec![100, 111, 110, 39, 116]),
        ],
        vec![
            load(0, 0),
            load(1, 1),
            load(2, 2),
            load(3, 3),
            load(4, 4),
            load(5, 5),
            load(6, 6),
            load(7, 7),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(8),
                rows: vec![vec![Register::new(1), Register::new(3)]],
            }),
            instruction(InstructionKind::Transpose {
                dst: Register::new(9),
                operand: Register::new(8),
                conjugate: false,
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(10)],
                target: Register::new(8),
                arguments: vec![ApplyArgument::Value(Register::new(4))],
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(11),
                target: Register::new(8),
                arguments: vec![ApplyArgument::Value(Register::new(4))],
                value: Register::new(5),
            }),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(6),
                rows: vec![vec![Register::new(2), Register::new(6)]],
            }),
            instruction(InstructionKind::Return {
                values: vec![
                    Register::new(0),
                    Register::new(2),
                    Register::new(7),
                    Register::new(8),
                    Register::new(9),
                    Register::new(10),
                    Register::new(11),
                    Register::new(6),
                ],
            }),
        ],
    );
    let (_, values) = execute(vec![function]);

    let char_parts = values[..7]
        .iter()
        .map(|value| {
            let Value::Array(ArrayData::Char(array)) = value else {
                panic!("expected exact char storage, found {value:?}");
            };
            (
                array.shape().dimensions().to_vec(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| value.get())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(char_parts[0], (vec![0, 0], vec![]));
    assert_eq!(char_parts[1], (vec![1, 2], vec![0xd83d, 0xde42]));
    assert_eq!(char_parts[2], (vec![1, 5], vec![100, 111, 110, 39, 116]));
    assert_eq!(char_parts[3], (vec![1, 4], vec![97, 98, 99, 100]));
    assert_eq!(char_parts[4], (vec![4, 1], vec![97, 98, 99, 100]));
    assert_eq!(char_parts[5], (vec![1, 1], vec![99]));
    assert_eq!(char_parts[6], (vec![1, 4], vec![97, 98, 90, 100]));
    assert!(!values[3].shares_array_storage_with(&values[6]));

    let string = values[7]
        .as_string_array()
        .expect("mixed char/string concat must produce a string array");
    assert_eq!(string.shape().dimensions(), &[1, 2]);
    assert_eq!(string.as_slice()[0].code_units(), &[0xd83d, 0xde42]);
    assert_eq!(string.as_slice()[1].code_units(), &[116, 97, 105, 108]);
}

#[test]
#[allow(clippy::too_many_lines)]
fn exact_integer_index_assignment_transpose_cow_and_condition_rejection() {
    let exact = Function {
        name: "exact_integers".to_owned(),
        register_count: 7,
        pack_register_count: 0,
        local_count: 3,
        persistent_slot_count: 0,
        parameter_count: 3,
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
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(2),
            }),
            instruction(InstructionKind::Move {
                dst: Register::new(3),
                src: Register::new(0),
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(4),
                target: Register::new(3),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
                value: Register::new(1),
            }),
            instruction(InstructionKind::Transpose {
                dst: Register::new(5),
                operand: Register::new(4),
                conjugate: false,
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(6)],
                target: Register::new(4),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
            }),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(3),
                rows: vec![vec![Register::new(4), Register::new(1)]],
            }),
            instruction(InstructionKind::Return {
                values: vec![
                    Register::new(0),
                    Register::new(4),
                    Register::new(5),
                    Register::new(6),
                    Register::new(3),
                ],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let target = Value::Array(ArrayData::Integer(IntegerArrayData::U64(
        DenseArray::from_vec(Shape::new([1, 3]).unwrap(), vec![u64::MAX, 2, 3]).unwrap(),
    )));
    let supplied = Value::Array(ArrayData::Integer(IntegerArrayData::U64(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![9]).unwrap(),
    )));
    let index = Value::Array(ArrayData::Integer(IntegerArrayData::U64(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![2]).unwrap(),
    )));
    let mut interpreter = Interpreter::new(BytecodeModule::new(vec![exact], FunctionId::new(0)))
        .expect("exact integer bytecode should verify");
    let values = interpreter
        .execute_entry(&[target, supplied, index])
        .expect("exact integer bytecode should execute");
    let integer_parts = values
        .iter()
        .map(|value| {
            let Value::Array(ArrayData::Integer(integer)) = value else {
                panic!("expected integer storage, found {value:?}");
            };
            (
                integer.shape().dimensions().to_vec(),
                integer
                    .elements()
                    .map(|value| value.real_component().canonical_decimal())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        integer_parts[0],
        (
            vec![1, 3],
            vec![u64::MAX.to_string(), "2".to_owned(), "3".to_owned()]
        )
    );
    assert_eq!(
        integer_parts[1],
        (
            vec![1, 3],
            vec![u64::MAX.to_string(), "9".to_owned(), "3".to_owned()]
        )
    );
    assert_eq!(integer_parts[2].0, vec![3, 1]);
    assert_eq!(integer_parts[3], (vec![1, 1], vec!["9".to_owned()]));
    assert_eq!(
        integer_parts[4],
        (
            vec![1, 4],
            vec![
                u64::MAX.to_string(),
                "9".to_owned(),
                "3".to_owned(),
                "9".to_owned(),
            ]
        )
    );
    assert!(!values[0].shares_array_storage_with(&values[1]));

    let exact_bounds = Function {
        name: "exact_uint64_index_bounds".to_owned(),
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
    let target = Value::Array(ArrayData::Integer(IntegerArrayData::U8(
        DenseArray::from_vec(Shape::new([1, 3]).unwrap(), vec![1, 2, 3]).unwrap(),
    )));
    let index = Value::Array(ArrayData::Integer(IntegerArrayData::U64(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![u64::MAX]).unwrap(),
    )));
    let mut interpreter =
        Interpreter::new(BytecodeModule::new(vec![exact_bounds], FunctionId::new(0)))
            .expect("exact bounds bytecode should verify");
    let error = interpreter
        .execute_entry(&[target, index])
        .expect_err("uint64 max must remain exact and exceed a three-element target");
    assert_eq!(
        error.index_out_of_bounds(),
        Some(IndexOutOfBoundsDetails {
            argument: 0,
            index: u64::MAX,
            extent: 3,
        })
    );

    let promotion = Function {
        name: "complex_integer_assignment_promotion".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 3,
        persistent_slot_count: 0,
        parameter_count: 3,
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
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(2),
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(3),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
                value: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0), Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let target = Value::Array(ArrayData::Integer(IntegerArrayData::I8(
        DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![1, 2]).unwrap(),
    )));
    let supplied = Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(
        DenseArray::from_vec(
            Shape::new([1, 1]).unwrap(),
            vec![ComplexInteger::new(9, -3)],
        )
        .unwrap(),
    )));
    let index = Value::Array(ArrayData::Integer(IntegerArrayData::U64(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![2]).unwrap(),
    )));
    let mut interpreter =
        Interpreter::new(BytecodeModule::new(vec![promotion], FunctionId::new(0)))
            .expect("integer promotion bytecode should verify");
    let promoted = interpreter
        .execute_entry(&[target, supplied, index])
        .expect("same-class complex integer assignment should execute");
    let Value::Array(ArrayData::Integer(IntegerArrayData::I8(original))) = &promoted[0] else {
        panic!("original real integer storage must remain real");
    };
    assert_eq!(original.as_slice(), &[1, 2]);
    let Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(assigned))) = &promoted[1]
    else {
        panic!("real integer target must promote to same-class complex storage");
    };
    assert_eq!(
        assigned.as_slice(),
        &[ComplexInteger::new(1, 0), ComplexInteger::new(9, -3)]
    );

    let condition = Function {
        name: "complex_integer_condition".to_owned(),
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
            instruction(InstructionKind::JumpIfFalse {
                condition: Register::new(0),
                target: InstructionIndex::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let nonreal = Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![ComplexInteger::new(1, 2)]).unwrap(),
    )));
    let mut interpreter =
        Interpreter::new(BytecodeModule::new(vec![condition], FunctionId::new(0)))
            .expect("condition bytecode should verify");
    let error = interpreter
        .execute_entry(&[nonreal])
        .expect_err("non-real complex integer condition must fail");
    assert_eq!(
        error.kind,
        RuntimeErrorKind::InvalidCondition {
            actual: openmat_value::ValueKind::Integer
        }
    );

    let conjugate = Function {
        name: "complex_integer_conjugate_transpose".to_owned(),
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
            instruction(InstructionKind::Transpose {
                dst: Register::new(1),
                operand: Register::new(0),
                conjugate: true,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let nonreal = Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![ComplexInteger::new(1, 2)]).unwrap(),
    )));
    let mut interpreter =
        Interpreter::new(BytecodeModule::new(vec![conjugate], FunctionId::new(0)))
            .expect("conjugate bytecode should verify");
    let error = interpreter
        .execute_entry(&[nonreal])
        .expect_err("complex integer conjugate transpose must fail");
    let RuntimeErrorKind::InvalidExecutionState {
        array: Some(detail),
        ..
    } = error.kind
    else {
        panic!("expected structured complex integer transpose failure");
    };
    assert!(matches!(
        detail.as_ref(),
        ArrayRuntimeError::InvalidOperand {
            operation: "complex integer conjugate transpose",
            ..
        }
    ));

    let arithmetic = Function {
        name: "deferred_integer_arithmetic".to_owned(),
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
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(1),
                lhs: Register::new(0),
                rhs: Register::new(0),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let integer = Value::Array(ArrayData::Integer(IntegerArrayData::I8(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![1]).unwrap(),
    )));
    let mut interpreter =
        Interpreter::new(BytecodeModule::new(vec![arithmetic], FunctionId::new(0)))
            .expect("integer arithmetic bytecode should verify");
    let error = interpreter
        .execute_entry(&[integer])
        .expect_err("deferred integer arithmetic must not promote through double");
    assert_eq!(
        error.kind,
        RuntimeErrorKind::UnsupportedArrayOperator {
            operator: BinaryOperator::Add
        }
    );
}

#[test]
fn exact_string_index_and_assignment_preserve_surrogates_missing_and_cow() {
    let function = Function {
        name: "exact_string_indexing".to_owned(),
        register_count: 5,
        pack_register_count: 0,
        local_count: 3,
        persistent_slot_count: 0,
        parameter_count: 3,
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
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(2),
                local: LocalSlot::new(2),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(3)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(4),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
                value: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(0), Register::new(3), Register::new(4)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let target = Value::String(StringValue::Array(
        StringArray::from_elements(
            Shape::new([1, 3]).unwrap(),
            vec![
                StringElement::from_code_units(vec![0xd83d]),
                StringElement::from_code_units(Vec::<u16>::new()),
                StringElement::missing(),
            ],
        )
        .unwrap(),
    ));
    let supplied = Value::String(StringValue::scalar(StringElement::from_code_units(vec![
        0xde42,
    ])));
    let index = Value::Array(ArrayData::Integer(IntegerArrayData::U64(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![2]).unwrap(),
    )));
    let mut interpreter = Interpreter::new(BytecodeModule::new(vec![function], FunctionId::new(0)))
        .expect("exact string bytecode should verify");
    let values = interpreter
        .execute_entry(&[target, supplied, index])
        .expect("exact string bytecode should execute");
    let indexed = values[1]
        .as_string_scalar()
        .expect("one selected string element is scalar");
    assert!(!indexed.is_missing());
    assert!(indexed.code_units().is_empty());

    let original = values[0].as_string_array().unwrap();
    let assigned = values[2].as_string_array().unwrap();
    assert_eq!(original.as_slice()[0].code_units(), &[0xd83d]);
    assert!(!original.as_slice()[1].is_missing());
    assert!(original.as_slice()[1].code_units().is_empty());
    assert!(original.as_slice()[2].is_missing());
    assert_eq!(assigned.as_slice()[0].code_units(), &[0xd83d]);
    assert_eq!(assigned.as_slice()[1].code_units(), &[0xde42]);
    assert!(assigned.as_slice()[2].is_missing());
    assert!(!values[0].shares_array_storage_with(&values[2]));
}

#[test]
fn failed_string_assignment_preserves_workspace_value_and_shared_storage() {
    let atomic = Function {
        name: "string_assignment_atomicity".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: vec![openmat_bytecode::Constant::String("saved".to_owned())],
        instructions: vec![
            instruction(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: openmat_bytecode::ConstantId::new(0),
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
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(3),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
                value: Register::new(1),
            }),
            instruction(InstructionKind::StoreGlobal {
                name: openmat_bytecode::ConstantId::new(0),
                src: Register::new(3),
            }),
            instruction(InstructionKind::Return { values: Vec::new() }),
        ],
        exception_handlers: Vec::new(),
    };
    let original = Value::String(StringValue::Array(
        StringArray::from_elements(
            Shape::new([1, 3]).unwrap(),
            vec![
                StringElement::from_code_units(vec![0xd83d]),
                StringElement::default(),
                StringElement::missing(),
            ],
        )
        .unwrap(),
    ));
    let too_many = Value::String(StringValue::Array(
        StringArray::from_elements(
            Shape::new([1, 2]).unwrap(),
            vec![StringElement::from_utf8("x"), StringElement::missing()],
        )
        .unwrap(),
    ));
    let index = Value::Double(2.0);
    let mut interpreter =
        Interpreter::new(BytecodeModule::new(vec![atomic], FunctionId::new(0))).unwrap();
    interpreter
        .workspace_mut()
        .insert("saved", original.clone());
    let error = interpreter
        .execute_entry(&[too_many, index])
        .expect_err("failed string assignment must not store a partial value");
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } if matches!(
            detail.as_ref(),
            ArrayRuntimeError::AssignmentSizeMismatch {
                selected: 1,
                supplied: 2
            }
        )
    ));
    let saved = interpreter.workspace().get("saved").unwrap();
    assert_eq!(saved, &original);
    assert!(saved.shares_array_storage_with(&original));
}

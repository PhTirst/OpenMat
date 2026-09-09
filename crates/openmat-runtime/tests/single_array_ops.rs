use openmat_array::{
    ArrayData, Complex32, Complex64 as ArrayComplex64, DenseArray, IntegerArrayData, Logical, Shape,
};
use openmat_bytecode::{
    ApplyArgument, BinaryOperator, BytecodeModule, Function, FunctionId, Instruction,
    InstructionKind, LocalSlot, Register,
};
use openmat_runtime::{ArrayRuntimeError, IndexErrorKind, Interpreter, RuntimeErrorKind};
use openmat_value::{Complex64, Value};

fn instruction(kind: InstructionKind) -> Instruction {
    Instruction::new(kind)
}

fn function(
    name: &str,
    register_count: u32,
    parameter_count: u32,
    instructions: Vec<Instruction>,
) -> Function {
    Function {
        name: name.to_owned(),
        register_count,
        pack_register_count: 0,
        local_count: parameter_count,
        persistent_slot_count: 0,
        parameter_count,
        argument_layout: None,
        constants: Vec::new(),
        instructions,
        exception_handlers: Vec::new(),
    }
}

fn load_local(dst: u32, local: u32) -> Instruction {
    instruction(InstructionKind::LoadLocal {
        dst: Register::new(dst),
        local: LocalSlot::new(local),
    })
}

fn execute(function: Function, arguments: &[Value]) -> Result<Vec<Value>, RuntimeErrorKind> {
    let mut interpreter = Interpreter::new(BytecodeModule::new(vec![function], FunctionId::new(0)))
        .expect("single runtime test bytecode should verify");
    interpreter
        .execute_entry(arguments)
        .map_err(|error| error.kind)
}

fn binary_function(operator: BinaryOperator) -> Function {
    function(
        "single_binary",
        3,
        2,
        vec![
            load_local(0, 0),
            load_local(1, 1),
            instruction(InstructionKind::Binary {
                operator,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    )
}

fn execute_binary(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, RuntimeErrorKind> {
    execute(binary_function(operator), &[lhs.clone(), rhs.clone()])
        .map(|mut values| values.remove(0))
}

fn execute_cancelled_binary(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, RuntimeErrorKind> {
    let mut interpreter = Interpreter::new(BytecodeModule::new(
        vec![binary_function(operator)],
        FunctionId::new(0),
    ))
    .expect("single runtime test bytecode should verify");
    interpreter.cancellation_token().cancel();
    interpreter
        .execute_entry(&[lhs.clone(), rhs.clone()])
        .map(|mut values| values.remove(0))
        .map_err(|error| error.kind)
}

fn index_function() -> Function {
    function(
        "single_index",
        3,
        2,
        vec![
            load_local(0, 0),
            load_local(1, 1),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
    )
}

fn assignment_function() -> Function {
    function(
        "single_assignment",
        4,
        3,
        vec![
            load_local(0, 0),
            load_local(1, 1),
            load_local(2, 2),
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
    )
}

fn transpose_function() -> Function {
    function(
        "single_transpose",
        3,
        1,
        vec![
            load_local(0, 0),
            instruction(InstructionKind::Transpose {
                dst: Register::new(1),
                operand: Register::new(0),
                conjugate: false,
            }),
            instruction(InstructionKind::Transpose {
                dst: Register::new(2),
                operand: Register::new(0),
                conjugate: true,
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(2)],
            }),
        ],
    )
}

fn matrix_function(name: &str, parameter_count: u32, rows: Vec<Vec<u32>>) -> Function {
    let dst = Register::new(parameter_count);
    let mut instructions = (0..parameter_count)
        .map(|register| load_local(register, register))
        .collect::<Vec<_>>();
    instructions.push(instruction(InstructionKind::BuildMatrix {
        dst,
        rows: rows
            .into_iter()
            .map(|row| row.into_iter().map(Register::new).collect())
            .collect(),
    }));
    instructions.push(instruction(InstructionKind::Return { values: vec![dst] }));
    function(name, parameter_count + 1, parameter_count, instructions)
}

fn transpose_concat_function() -> Function {
    function(
        "single_transpose_concat",
        4,
        1,
        vec![
            load_local(0, 0),
            instruction(InstructionKind::Transpose {
                dst: Register::new(1),
                operand: Register::new(0),
                conjugate: true,
            }),
            instruction(InstructionKind::Transpose {
                dst: Register::new(2),
                operand: Register::new(0),
                conjugate: false,
            }),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(3),
                rows: vec![vec![Register::new(1), Register::new(2)]],
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(3)],
            }),
        ],
    )
}

fn concatenate(rows: Vec<Vec<u32>>, arguments: &[Value]) -> Result<Value, RuntimeErrorKind> {
    let parameter_count = u32::try_from(arguments.len()).expect("test operand count fits u32");
    execute(
        matrix_function("single_matrix", parameter_count, rows),
        arguments,
    )
    .map(|mut values| values.remove(0))
}

fn single_real(dimensions: [u64; 2], values: Vec<f32>) -> Value {
    Value::Array(ArrayData::F32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn single_complex(dimensions: [u64; 2], values: Vec<Complex32>) -> Value {
    Value::Array(ArrayData::ComplexF32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn double_real(dimensions: [u64; 2], values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn double_complex(dimensions: [u64; 2], values: Vec<ArrayComplex64>) -> Value {
    Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn logical(dimensions: [u64; 2], values: Vec<bool>) -> Value {
    Value::Array(ArrayData::Logical(
        DenseArray::from_vec(
            Shape::new(dimensions).unwrap(),
            values.into_iter().map(Logical::from).collect(),
        )
        .unwrap(),
    ))
}

fn real_parts(value: &Value) -> (&[u64], &[f32]) {
    let Value::Array(ArrayData::F32(array)) = value else {
        panic!("expected real single storage, found {value:?}");
    };
    (array.shape().dimensions(), array.as_slice())
}

fn complex_parts(value: &Value) -> (&[u64], &[Complex32]) {
    let Value::Array(ArrayData::ComplexF32(array)) = value else {
        panic!("expected complex single storage, found {value:?}");
    };
    (array.shape().dimensions(), array.as_slice())
}

fn logical_parts(value: &Value) -> (&[u64], Vec<bool>) {
    let Value::Array(ArrayData::Logical(array)) = value else {
        panic!("expected logical array storage, found {value:?}");
    };
    (
        array.shape().dimensions(),
        array.as_slice().iter().map(|value| value.get()).collect(),
    )
}

fn real_bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

fn complex_bits(values: &[Complex32]) -> Vec<(u32, u32)> {
    values
        .iter()
        .map(|value| (value.re.to_bits(), value.im.to_bits()))
        .collect()
}

fn array_error(error: RuntimeErrorKind) -> ArrayRuntimeError {
    let RuntimeErrorKind::InvalidExecutionState {
        array: Some(detail),
        ..
    } = error
    else {
        panic!("expected structured array error, found {error:?}");
    };
    *detail
}

#[test]
fn single_indices_preserve_selector_shape_column_major_order_and_exact_bits() {
    let values = [
        0x3f80_0001,
        0x4000_0001,
        0x4040_0001,
        0x4080_0001,
        0x40a0_0001,
        0x40c0_0001,
    ]
    .map(f32::from_bits);
    let target = single_real([2, 3], values.to_vec());
    let selector = single_real([2, 2], vec![6.0, 1.0, 4.0, 2.0]);
    let indexed = execute(index_function(), &[target.clone(), selector]).unwrap();
    let (shape, gathered) = real_parts(&indexed[0]);
    assert_eq!(shape, &[2, 2]);
    assert_eq!(
        real_bits(gathered),
        [values[5], values[0], values[3], values[1]]
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    );

    let scalar = execute(index_function(), &[target, single_real([1, 1], vec![3.0])]).unwrap();
    let (shape, gathered) = real_parts(&scalar[0]);
    assert_eq!(shape, &[1, 1]);
    assert_eq!(real_bits(gathered), vec![values[2].to_bits()]);

    let complex_values = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Complex32::new(
                *value,
                f32::from_bits(0x3f00_0000 + u32::try_from(index).unwrap()),
            )
        })
        .collect::<Vec<_>>();
    let target = single_complex([2, 3], complex_values.clone());
    let selector = single_real([2, 2], vec![6.0, 1.0, 4.0, 2.0]);
    let indexed = execute(index_function(), &[target, selector]).unwrap();
    let (shape, gathered) = complex_parts(&indexed[0]);
    assert_eq!(shape, &[2, 2]);
    assert_eq!(
        complex_bits(gathered),
        complex_bits(&[
            complex_values[5],
            complex_values[0],
            complex_values[3],
            complex_values[1],
        ])
    );
}

#[test]
fn single_indices_validate_finite_positive_integers_and_reject_complex_storage() {
    let target = single_real([1, 3], vec![10.0, 20.0, 30.0]);
    let invalid = [
        (f32::NAN, IndexErrorKind::NonFinite),
        (f32::INFINITY, IndexErrorKind::NonFinite),
        (1.5, IndexErrorKind::NonInteger),
        (0.0, IndexErrorKind::NonPositive),
        (-1.0, IndexErrorKind::NonPositive),
    ];
    for (selector, expected) in invalid {
        let error = execute(
            index_function(),
            &[target.clone(), single_real([1, 1], vec![selector])],
        )
        .unwrap_err();
        assert_eq!(
            array_error(error),
            ArrayRuntimeError::InvalidIndex {
                argument: 0,
                reason: expected,
            }
        );
    }

    let error = execute(
        index_function(),
        &[target.clone(), single_real([1, 1], vec![4.0])],
    )
    .unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::IndexOutOfBounds {
            argument: 0,
            index: 4,
            extent: 3,
        }
    );

    let complex_selector = single_complex([1, 1], vec![Complex32::new(2.0, 0.0)]);
    let error = execute(index_function(), &[target, complex_selector]).unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::InvalidIndex {
            argument: 0,
            reason: IndexErrorKind::NonInteger,
        }
    );
}

#[test]
fn real_single_assignment_expands_scalars_preserves_bits_and_detaches_cow() {
    let original_bits = [0x3f80_0001, 0x4000_0001, 0x4040_0001, 0x4080_0001];
    let replacement = f32::from_bits(0x7fc1_2345);
    let target = single_real([2, 2], original_bits.map(f32::from_bits).to_vec());
    let supplied = single_real([1, 1], vec![replacement]);
    let selector = single_real([1, 2], vec![2.0, 4.0]);
    let result = execute(assignment_function(), &[target, supplied, selector]).unwrap();

    assert!(!result[0].shares_storage_with(&result[1]));
    assert_eq!(real_bits(real_parts(&result[0]).1), original_bits);
    assert_eq!(
        real_bits(real_parts(&result[1]).1),
        vec![
            original_bits[0],
            replacement.to_bits(),
            original_bits[2],
            replacement.to_bits(),
        ]
    );
}

#[test]
fn single_assignment_supports_complex_sources_targets_and_real_target_promotion() {
    let complex_target = single_complex(
        [1, 3],
        vec![
            Complex32::new(1.0, 11.0),
            Complex32::new(2.0, 12.0),
            Complex32::new(3.0, 13.0),
        ],
    );
    let real_supplied = single_real(
        [1, 2],
        vec![f32::from_bits(0x3f80_0001), f32::from_bits(0x4000_0001)],
    );
    let selector = single_real([1, 2], vec![1.0, 3.0]);
    let result = execute(
        assignment_function(),
        &[complex_target, real_supplied, selector.clone()],
    )
    .unwrap();
    assert_eq!(
        complex_bits(complex_parts(&result[1]).1),
        complex_bits(&[
            Complex32::new(f32::from_bits(0x3f80_0001), 0.0),
            Complex32::new(2.0, 12.0),
            Complex32::new(f32::from_bits(0x4000_0001), 0.0),
        ])
    );

    let complex_supplied = single_complex(
        [1, 2],
        vec![
            Complex32::new(f32::from_bits(0x40a0_0001), -5.0),
            Complex32::new(f32::from_bits(0x40c0_0001), -6.0),
        ],
    );
    let complex_target = single_complex(
        [1, 3],
        vec![
            Complex32::new(1.0, 11.0),
            Complex32::new(2.0, 12.0),
            Complex32::new(3.0, 13.0),
        ],
    );
    let result = execute(
        assignment_function(),
        &[complex_target, complex_supplied.clone(), selector.clone()],
    )
    .unwrap();
    assert_eq!(
        complex_bits(complex_parts(&result[1]).1),
        complex_bits(&[
            complex_parts(&complex_supplied).1[0],
            Complex32::new(2.0, 12.0),
            complex_parts(&complex_supplied).1[1],
        ])
    );

    let real_target = single_real(
        [1, 3],
        vec![
            f32::from_bits(0x3f80_0001),
            f32::from_bits(0x4000_0001),
            f32::from_bits(0x4040_0001),
        ],
    );
    let result = execute(
        assignment_function(),
        &[real_target, complex_supplied.clone(), selector],
    )
    .unwrap();
    assert!(matches!(result[1], Value::Array(ArrayData::ComplexF32(_))));
    assert_eq!(
        complex_bits(complex_parts(&result[1]).1),
        complex_bits(&[
            complex_parts(&complex_supplied).1[0],
            Complex32::new(f32::from_bits(0x4000_0001), 0.0),
            complex_parts(&complex_supplied).1[1],
        ])
    );
}

#[test]
fn single_assignment_aligns_empty_mismatch_bounds_and_type_errors() {
    let target = single_real([1, 3], vec![1.0, 2.0, 3.0]);
    let empty_selector = single_real([0, 0], Vec::new());
    let result = execute(
        assignment_function(),
        &[
            target.clone(),
            single_real([1, 1], vec![9.0]),
            empty_selector,
        ],
    )
    .unwrap();
    assert_eq!(result[0], result[1]);
    assert!(result[0].shares_storage_with(&result[1]));

    let error = execute(
        assignment_function(),
        &[
            target.clone(),
            single_real([1, 3], vec![7.0, 8.0, 9.0]),
            single_real([1, 2], vec![1.0, 2.0]),
        ],
    )
    .unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::AssignmentSizeMismatch {
            selected: 2,
            supplied: 3,
        }
    );

    let error = execute(
        assignment_function(),
        &[
            target.clone(),
            single_real([1, 1], vec![9.0]),
            single_real([1, 1], vec![4.0]),
        ],
    )
    .unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::IndexOutOfBounds {
            argument: 0,
            index: 4,
            extent: 3,
        }
    );

    let error = execute(
        assignment_function(),
        &[target, Value::Double(9.0), single_real([1, 1], vec![2.0])],
    )
    .unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::InvalidOperand {
            operation: "single indexed assignment",
            actual: openmat_value::ValueKind::Double,
        }
    );
}

#[test]
fn single_transpose_and_ctranspose_preserve_exact_f32_storage() {
    let real_values = [
        f32::from_bits(0x0000_0001),
        f32::from_bits(0x8000_0000),
        f32::from_bits(0x3f80_0001),
        f32::from_bits(0x7fc1_2345),
        f32::from_bits(0x4000_0001),
        f32::from_bits(0x4040_0001),
    ];
    let result = execute(
        transpose_function(),
        &[single_real([2, 3], real_values.to_vec())],
    )
    .unwrap();
    let expected = [
        real_values[0],
        real_values[2],
        real_values[4],
        real_values[1],
        real_values[3],
        real_values[5],
    ];
    for value in &result {
        let (shape, values) = real_parts(value);
        assert_eq!(shape, &[3, 2]);
        assert_eq!(real_bits(values), real_bits(&expected));
    }

    let complex_values = real_values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Complex32::new(
                *value,
                f32::from_bits(0x3f00_0000 + u32::try_from(index).unwrap()),
            )
        })
        .collect::<Vec<_>>();
    let result = execute(
        transpose_function(),
        &[single_complex([2, 3], complex_values.clone())],
    )
    .unwrap();
    let order = [0, 2, 4, 1, 3, 5];
    let transposed = order
        .iter()
        .map(|index| complex_values[*index])
        .collect::<Vec<_>>();
    let conjugated = transposed
        .iter()
        .map(|value| value.conjugate())
        .collect::<Vec<_>>();
    assert_eq!(complex_parts(&result[0]).0, &[3, 2]);
    assert_eq!(
        complex_bits(complex_parts(&result[0]).1),
        complex_bits(&transposed)
    );
    assert_eq!(complex_parts(&result[1]).0, &[3, 2]);
    assert_eq!(
        complex_bits(complex_parts(&result[1]).1),
        complex_bits(&conjugated)
    );
}

#[test]
fn single_matrix_concatenates_row_ctranspose_and_transpose_as_exact_complex_f32() {
    let row_values = [
        Complex32::new(f32::from_bits(0x3f80_0001), f32::from_bits(0x4000_0001)),
        Complex32::new(f32::from_bits(0x4040_0001), -f32::from_bits(0x4080_0001)),
    ];
    let row = single_complex([1, 2], row_values.to_vec());
    let result = execute(transpose_concat_function(), std::slice::from_ref(&row)).unwrap();

    let (shape, values) = complex_parts(&result[0]);
    assert_eq!(shape, &[2, 2]);
    assert_eq!(
        complex_bits(values),
        complex_bits(&[
            row_values[0].conjugate(),
            row_values[1].conjugate(),
            row_values[0],
            row_values[1],
        ])
    );
    assert!(!result[0].shares_storage_with(&row));
    assert!(result[0].shares_storage_with(&result[0].clone()));
}

#[test]
fn single_matrix_preserves_horizontal_vertical_column_major_layout_and_cow() {
    let values = [
        0x3f80_0001,
        0x4000_0001,
        0x4040_0001,
        0x4080_0001,
        0x40a0_0001,
        0x40c0_0001,
    ]
    .map(f32::from_bits);
    let left = single_real([2, 2], values[..4].to_vec());
    let right = single_real([2, 1], values[4..].to_vec());
    let horizontal = concatenate(vec![vec![0, 1]], &[left.clone(), right.clone()]).unwrap();
    assert_eq!(real_parts(&horizontal).0, &[2, 3]);
    assert_eq!(real_bits(real_parts(&horizontal).1), real_bits(&values));
    assert!(!horizontal.shares_storage_with(&left));
    assert!(!horizontal.shares_storage_with(&right));
    assert!(horizontal.shares_storage_with(&horizontal.clone()));

    let top = single_real([1, 2], vec![values[0], values[3]]);
    let bottom = single_real([2, 2], vec![values[1], values[2], values[4], values[5]]);
    let vertical = concatenate(vec![vec![0], vec![1]], &[top, bottom]).unwrap();
    assert_eq!(real_parts(&vertical).0, &[3, 2]);
    assert_eq!(real_bits(real_parts(&vertical).1), real_bits(&values));
}

#[test]
fn single_matrix_promotes_real_and_complex_operands_without_widening_single_storage() {
    let real_values = [f32::from_bits(0x0000_0001), f32::from_bits(0x7fc1_2345)];
    let complex_values = [
        Complex32::new(f32::from_bits(0x8000_0000), f32::from_bits(0x3f00_0001)),
        Complex32::new(f32::from_bits(0x3f80_0001), -f32::from_bits(0x3f80_0001)),
    ];
    let result = concatenate(
        vec![vec![0, 1]],
        &[
            single_real([2, 1], real_values.to_vec()),
            single_complex([2, 1], complex_values.to_vec()),
        ],
    )
    .unwrap();
    assert_eq!(complex_parts(&result).0, &[2, 2]);
    assert_eq!(
        complex_bits(complex_parts(&result).1),
        complex_bits(&[
            Complex32::from(real_values[0]),
            Complex32::from(real_values[1]),
            complex_values[0],
            complex_values[1],
        ])
    );
}

#[test]
fn single_matrix_uses_r2022b_mixed_double_logical_and_complex_rules() {
    let exact = f32::from_bits(0x3f80_0001);
    let result = concatenate(
        vec![vec![0, 1, 2]],
        &[
            double_real([1, 2], vec![1.25, -2.5]),
            logical([1, 2], vec![false, true]),
            single_real([1, 1], vec![exact]),
        ],
    )
    .unwrap();
    assert_eq!(real_parts(&result).0, &[1, 5]);
    assert_eq!(
        real_bits(real_parts(&result).1),
        real_bits(&[1.25_f32, -2.5_f32, 0.0, 1.0, exact])
    );

    let result = concatenate(
        vec![vec![0, 1, 2]],
        &[
            single_real([1, 1], vec![exact]),
            double_complex(
                [1, 2],
                vec![
                    ArrayComplex64::new(2.25, -3.5),
                    ArrayComplex64::new(-4.75, 5.125),
                ],
            ),
            Value::Complex(Complex64::new(6.5, -7.25)),
        ],
    )
    .unwrap();
    assert_eq!(complex_parts(&result).0, &[1, 4]);
    assert_eq!(
        complex_bits(complex_parts(&result).1),
        complex_bits(&[
            Complex32::from(exact),
            Complex32::new(2.25, -3.5),
            Complex32::new(-4.75, 5.125),
            Complex32::new(6.5, -7.25),
        ])
    );

    let result = concatenate(
        vec![vec![0, 1]],
        &[
            double_complex([1, 0], Vec::new()),
            single_real([1, 2], vec![1.0, 2.0]),
        ],
    )
    .unwrap();
    assert_eq!(real_parts(&result).0, &[1, 2]);
    assert_eq!(real_bits(real_parts(&result).1), real_bits(&[1.0, 2.0]));
}

#[test]
fn single_matrix_preserves_empty_shapes_and_complex_single_empty_storage() {
    let result = concatenate(
        vec![vec![0, 1]],
        &[
            single_real([0, 0], Vec::new()),
            single_real([1, 2], vec![1.0, 2.0]),
        ],
    )
    .unwrap();
    assert_eq!(real_parts(&result).0, &[1, 2]);
    assert_eq!(real_bits(real_parts(&result).1), real_bits(&[1.0, 2.0]));

    let result = concatenate(
        vec![vec![0], vec![1]],
        &[
            single_real([0, 2], Vec::new()),
            single_real([1, 2], vec![3.0, 4.0]),
        ],
    )
    .unwrap();
    assert_eq!(real_parts(&result).0, &[1, 2]);
    assert_eq!(real_bits(real_parts(&result).1), real_bits(&[3.0, 4.0]));

    let result = concatenate(
        vec![vec![0, 1]],
        &[
            single_complex([1, 0], Vec::new()),
            single_real([1, 0], Vec::new()),
        ],
    )
    .unwrap();
    assert_eq!(complex_parts(&result).0, &[1, 0]);
    assert!(complex_parts(&result).1.is_empty());

    let result = concatenate(vec![vec![0]], &[single_real([0, 0], Vec::new())]).unwrap();
    assert_eq!(real_parts(&result).0, &[0, 0]);
}

#[test]
fn single_matrix_rejects_unmeasured_classes_and_nd_operands_structurally() {
    let error = concatenate(
        vec![vec![0, 1]],
        &[single_real([1, 1], vec![1.0]), Value::Nothing],
    )
    .unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::InvalidOperand {
            operation: "single matrix concatenation",
            actual: openmat_value::ValueKind::Nothing,
        }
    );

    let integer = Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![2_u8]).unwrap(),
    )));
    let error =
        concatenate(vec![vec![0, 1]], &[single_real([1, 1], vec![1.0]), integer]).unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::InvalidOperand {
            operation: "exact matrix concatenation",
            actual: openmat_value::ValueKind::Single,
        }
    );

    let nd_single = Value::Array(ArrayData::F32(
        DenseArray::from_vec(Shape::new([1, 1, 2]).unwrap(), vec![1.0, 2.0]).unwrap(),
    ));
    let error = concatenate(vec![vec![0]], &[nd_single]).unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::InvalidOperand {
            operation: "single matrix concatenation above two dimensions",
            actual: openmat_value::ValueKind::Single,
        }
    );

    let error = concatenate(
        vec![vec![0, 1]],
        &[
            single_real([2, 1], vec![1.0, 2.0]),
            single_real([1, 1], vec![3.0]),
        ],
    )
    .unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::ConcatenationMismatch {
            axis: 0,
            expected: 2,
            actual: 1,
        }
    );
}

#[test]
fn single_real_element_arithmetic_matches_r2022b_binary32_bits_and_detaches_cow() {
    let lhs = single_real(
        [1, 3],
        [0x3f80_0001, 0x4000_0001, 0x4040_0001]
            .map(f32::from_bits)
            .to_vec(),
    );
    let rhs = single_real(
        [1, 3],
        [0x3e80_0000, 0x3f80_0001, 0x3f00_0000]
            .map(f32::from_bits)
            .to_vec(),
    );
    let cases = [
        (
            BinaryOperator::Add,
            vec![0x3fa0_0001, 0x4040_0002, 0x4060_0001],
        ),
        (
            BinaryOperator::Subtract,
            vec![0x3f40_0002, 0x3f80_0001, 0x4020_0001],
        ),
        (
            BinaryOperator::ElementMultiply,
            vec![0x3e80_0001, 0x4000_0002, 0x3fc0_0001],
        ),
        (
            BinaryOperator::ElementDivide,
            vec![0x4080_0001, 0x4000_0000, 0x40c0_0001],
        ),
        (
            BinaryOperator::ElementPower,
            vec![0x3f80_0000, 0x4000_0002, 0x3fdd_b3d8],
        ),
    ];

    for (operator, expected) in cases {
        let result = execute_binary(operator, &lhs, &rhs).unwrap();
        assert_eq!(real_parts(&result).0, &[1, 3]);
        assert_eq!(real_bits(real_parts(&result).1), expected);
        assert!(!result.shares_storage_with(&lhs));
        assert!(!result.shares_storage_with(&rhs));
        assert!(result.shares_storage_with(&result.clone()));
    }
}

#[test]
fn single_mixed_double_retains_double_operand_precision_then_returns_single() {
    let single = single_real([1, 1], vec![1.0]);
    let doubles = double_real(
        [1, 2],
        vec![1.0 + 2_f64.powi(-25), 1.0 + 3.0 * 2_f64.powi(-24)],
    );
    let result = execute_binary(BinaryOperator::Add, &single, &doubles).unwrap();
    assert_eq!(real_parts(&result).0, &[1, 2]);
    assert_eq!(
        real_bits(real_parts(&result).1),
        vec![0x4000_0000, 0x4000_0001]
    );

    let result = execute_binary(
        BinaryOperator::Subtract,
        &single,
        &Value::Double(1.0 + 2_f64.powi(-25)),
    )
    .unwrap();
    assert_eq!(real_bits(real_parts(&result).1), vec![0xb300_0000]);

    let logicals = logical([1, 2], vec![true, false]);
    let result = execute_binary(
        BinaryOperator::Subtract,
        &logicals,
        &single_real([1, 2], vec![0.25, 2.0]),
    )
    .unwrap();
    assert_eq!(real_parts(&result).0, &[1, 2]);
    assert_eq!(real_bits(real_parts(&result).1), real_bits(&[0.75, -2.0]));

    let result = execute_binary(
        BinaryOperator::Add,
        &Value::Complex(Complex64::new(2.0, -3.0)),
        &single_complex([1, 1], vec![Complex32::new(1.0, 1.0)]),
    )
    .unwrap();
    assert_eq!(
        complex_bits(complex_parts(&result).1),
        complex_bits(&[Complex32::new(3.0, -2.0)])
    );

    let single_two = single_real([1, 1], vec![2.0]);
    for operator in [
        BinaryOperator::Add,
        BinaryOperator::Subtract,
        BinaryOperator::ElementMultiply,
        BinaryOperator::ElementDivide,
        BinaryOperator::ElementPower,
    ] {
        assert!(matches!(
            execute_binary(operator, &single_two, &Value::Double(3.0)).unwrap(),
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        ));
        assert!(matches!(
            execute_binary(operator, &Value::Logical(true), &single_two).unwrap(),
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        ));
    }
}

#[test]
fn finite_complex_single_arithmetic_and_real_power_promotion_match_r2022b() {
    let lhs = single_complex([1, 1], vec![Complex32::new(1.0, 2.0)]);
    let rhs = single_complex([1, 1], vec![Complex32::new(3.0, -2.0)]);

    let sum = execute_binary(BinaryOperator::Add, &lhs, &rhs).unwrap();
    assert_eq!(real_bits(real_parts(&sum).1), vec![4.0_f32.to_bits()]);

    let difference = execute_binary(BinaryOperator::Subtract, &lhs, &rhs).unwrap();
    assert_eq!(
        complex_bits(complex_parts(&difference).1),
        complex_bits(&[Complex32::new(-2.0, 4.0)])
    );

    let product = execute_binary(BinaryOperator::ElementMultiply, &lhs, &rhs).unwrap();
    assert_eq!(
        complex_bits(complex_parts(&product).1),
        complex_bits(&[Complex32::new(7.0, 4.0)])
    );

    let quotient = execute_binary(BinaryOperator::ElementDivide, &lhs, &rhs).unwrap();
    assert_eq!(
        complex_bits(complex_parts(&quotient).1),
        vec![(0xbd9d_89d9, 0x3f1d_89d9)]
    );

    let power = execute_binary(BinaryOperator::ElementPower, &lhs, &rhs).unwrap();
    assert_eq!(
        complex_bits(complex_parts(&power).1),
        vec![(0xc166_7e7e, 0x42ca_ac7c)]
    );

    let result = execute_binary(
        BinaryOperator::ElementPower,
        &single_real([1, 3], vec![-1.0, -4.0, 4.0]),
        &single_real([1, 1], vec![0.5]),
    )
    .unwrap();
    assert_eq!(complex_parts(&result).0, &[1, 3]);
    assert_eq!(
        complex_bits(complex_parts(&result).1),
        complex_bits(&[
            Complex32::new(0.0, 1.0),
            Complex32::new(0.0, 2.0),
            Complex32::new(2.0, 0.0),
        ])
    );
}

#[test]
fn finite_complex_single_scaling_handles_overflow_underflow_and_zero_divisors() {
    let maximum = f32::MAX;
    let minimum = f32::MIN_POSITIVE;
    for magnitude in [maximum, minimum] {
        let value = single_complex([1, 1], vec![Complex32::new(magnitude, magnitude)]);
        let power = execute_binary(
            BinaryOperator::ElementPower,
            &value,
            &single_real([1, 1], vec![1.0]),
        )
        .unwrap();
        assert_eq!(
            complex_bits(complex_parts(&power).1),
            complex_bits(&[Complex32::new(magnitude, magnitude)])
        );

        let quotient = execute_binary(BinaryOperator::ElementDivide, &value, &value).unwrap();
        assert_eq!(real_bits(real_parts(&quotient).1), vec![0x3f80_0000]);

        let conjugate = single_complex([1, 1], vec![Complex32::new(magnitude, -magnitude)]);
        let quotient = execute_binary(BinaryOperator::ElementDivide, &value, &conjugate).unwrap();
        assert_eq!(
            complex_bits(complex_parts(&quotient).1),
            vec![(0x0000_0000, 0x3f80_0000)]
        );
    }

    let large = single_complex([1, 1], vec![Complex32::new(maximum, maximum)]);
    let product = execute_binary(BinaryOperator::ElementMultiply, &large, &large).unwrap();
    assert_eq!(
        complex_bits(complex_parts(&product).1),
        vec![(0x0000_0000, 0x7f80_0000)]
    );
    let conjugate = single_complex([1, 1], vec![Complex32::new(maximum, -maximum)]);
    let product = execute_binary(BinaryOperator::ElementMultiply, &large, &conjugate).unwrap();
    assert_eq!(real_bits(real_parts(&product).1), vec![0x7f80_0000]);

    let numerator = single_complex([1, 1], vec![Complex32::new(1.0, 2.0)]);
    let positive_zero = single_complex([1, 1], vec![Complex32::new(0.0, -0.0)]);
    let quotient =
        execute_binary(BinaryOperator::ElementDivide, &numerator, &positive_zero).unwrap();
    assert_eq!(
        complex_bits(complex_parts(&quotient).1),
        vec![(0x7f80_0000, 0x7f80_0000)]
    );

    let negative_zero = single_complex([1, 1], vec![Complex32::new(-0.0, 0.0)]);
    let quotient =
        execute_binary(BinaryOperator::ElementDivide, &numerator, &negative_zero).unwrap();
    assert_eq!(
        complex_bits(complex_parts(&quotient).1),
        vec![(0xff80_0000, 0xff80_0000)]
    );

    let zero_over_zero = execute_binary(
        BinaryOperator::ElementDivide,
        &positive_zero,
        &positive_zero,
    )
    .unwrap();
    assert!(real_parts(&zero_over_zero).1[0].is_nan());
}

#[test]
fn single_comparisons_expand_scalars_preserve_mixed_precision_and_reject_ordered_complex() {
    let single_one = single_real([1, 1], vec![1.0]);
    let precise_double = Value::Double(1.0 + 2_f64.powi(-25));
    assert_eq!(
        execute_binary(BinaryOperator::Equal, &single_one, &precise_double).unwrap(),
        Value::Logical(false)
    );
    assert_eq!(
        execute_binary(BinaryOperator::NotEqual, &single_one, &precise_double).unwrap(),
        Value::Logical(true)
    );
    assert_eq!(
        execute_binary(BinaryOperator::LessThan, &single_one, &precise_double).unwrap(),
        Value::Logical(true)
    );

    let values = single_real(
        [1, 5],
        vec![f32::NAN, 0.0, -0.0, f32::INFINITY, f32::NEG_INFINITY],
    );
    let zero = Value::Logical(false);
    let cases = [
        (BinaryOperator::Equal, vec![false, true, true, false, false]),
        (
            BinaryOperator::NotEqual,
            vec![true, false, false, true, true],
        ),
        (
            BinaryOperator::LessThan,
            vec![false, false, false, false, true],
        ),
        (
            BinaryOperator::LessThanOrEqual,
            vec![false, true, true, false, true],
        ),
        (
            BinaryOperator::GreaterThan,
            vec![false, false, false, true, false],
        ),
        (
            BinaryOperator::GreaterThanOrEqual,
            vec![false, true, true, true, false],
        ),
    ];
    for (operator, expected) in cases {
        let result = execute_binary(operator, &values, &zero).unwrap();
        assert_eq!(logical_parts(&result), (&[1, 5][..], expected));
    }

    let complex = single_complex([1, 1], vec![Complex32::new(1.0, 0.0)]);
    assert_eq!(
        execute_binary(BinaryOperator::Equal, &complex, &single_one).unwrap(),
        Value::Logical(true)
    );
    let error = execute_binary(BinaryOperator::LessThan, &complex, &single_one).unwrap_err();
    assert_eq!(
        error,
        RuntimeErrorKind::InvalidOperands {
            operator: BinaryOperator::LessThan,
            lhs: openmat_value::ValueKind::Single,
            rhs: openmat_value::ValueKind::Single,
        }
    );
}

#[test]
fn single_real_special_values_preserve_signed_zero_inf_and_nan_semantics() {
    let values = single_real(
        [1, 5],
        vec![
            0.0,
            -0.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::from_bits(0x7fc1_2345),
        ],
    );
    let one = single_real([1, 1], vec![1.0]);
    for operator in [
        BinaryOperator::ElementMultiply,
        BinaryOperator::ElementDivide,
        BinaryOperator::ElementPower,
    ] {
        let result = execute_binary(operator, &values, &one).unwrap();
        let output = real_parts(&result).1;
        assert_eq!(
            real_bits(&output[..4]),
            vec![0x0000_0000, 0x8000_0000, 0x7f80_0000, 0xff80_0000]
        );
        assert!(output[4].is_nan());
    }
}

#[test]
fn single_binary_preserves_empty_shapes_reports_mismatch_and_honors_cancellation() {
    let empty = single_complex([0, 2], Vec::new());
    let result =
        execute_binary(BinaryOperator::Add, &empty, &single_real([1, 1], vec![1.0])).unwrap();
    assert_eq!(real_parts(&result).0, &[0, 2]);
    assert!(real_parts(&result).1.is_empty());
    assert!(!result.shares_storage_with(&empty));

    let nd = Value::Array(ArrayData::F32(
        DenseArray::from_vec(Shape::new([2, 1, 2]).unwrap(), vec![1.0, 2.0, 3.0, 4.0]).unwrap(),
    ));
    let result = execute_binary(BinaryOperator::Add, &single_real([1, 1], vec![1.0]), &nd).unwrap();
    assert_eq!(real_parts(&result).0, &[2, 1, 2]);
    assert_eq!(
        real_bits(real_parts(&result).1),
        real_bits(&[2.0, 3.0, 4.0, 5.0])
    );

    let empty_mismatch = execute_binary(
        BinaryOperator::Add,
        &single_real([0, 0], Vec::new()),
        &single_real([0, 2], Vec::new()),
    )
    .unwrap_err();
    assert_eq!(
        array_error(empty_mismatch),
        ArrayRuntimeError::ShapeMismatch {
            operation: "addition",
            lhs: vec![0, 0],
            rhs: vec![0, 2],
        }
    );

    let row = single_real([1, 2], vec![1.0, 2.0]);
    let column = single_real([2, 1], vec![3.0, 4.0]);
    let expanded = execute_binary(BinaryOperator::Add, &row, &column).unwrap();
    assert_eq!(real_parts(&expanded).0, &[2, 2]);
    assert_eq!(
        real_bits(real_parts(&expanded).1),
        real_bits(&[4.0, 5.0, 5.0, 6.0])
    );

    let lhs = single_real([2, 2], vec![1.0, 2.0, 3.0, 4.0]);
    let rhs = single_real([1, 3], vec![3.0, 4.0, 5.0]);
    let error = execute_binary(BinaryOperator::Add, &lhs, &rhs).unwrap_err();
    assert_eq!(
        array_error(error),
        ArrayRuntimeError::ShapeMismatch {
            operation: "addition",
            lhs: vec![2, 2],
            rhs: vec![1, 3],
        }
    );

    assert_eq!(
        execute_cancelled_binary(BinaryOperator::Add, &row, &single_real([1, 1], vec![1.0]))
            .unwrap_err(),
        RuntimeErrorKind::Cancelled
    );
}

#[test]
fn single_scalar_matrix_operators_and_unmeasured_nonfinite_complex_edges_remain_unsupported() {
    let lhs = single_real([1, 1], vec![2.0]);
    let rhs = single_real([1, 1], vec![3.0]);
    for operator in [
        BinaryOperator::Multiply,
        BinaryOperator::Divide,
        BinaryOperator::Power,
    ] {
        assert_eq!(
            execute_binary(operator, &lhs, &rhs).unwrap_err(),
            RuntimeErrorKind::UnsupportedArrayOperator { operator }
        );
    }

    let nonfinite = single_complex([1, 1], vec![Complex32::new(f32::INFINITY, 1.0)]);
    assert_eq!(
        execute_binary(BinaryOperator::Add, &nonfinite, &rhs).unwrap_err(),
        RuntimeErrorKind::UnsupportedArrayOperator {
            operator: BinaryOperator::Add,
        }
    );

    let zero = single_complex([1, 1], vec![Complex32::ZERO]);
    let complex_exponent = single_complex([1, 1], vec![Complex32::new(1.0, 1.0)]);
    assert_eq!(
        execute_binary(BinaryOperator::ElementPower, &zero, &complex_exponent).unwrap_err(),
        RuntimeErrorKind::UnsupportedArrayOperator {
            operator: BinaryOperator::ElementPower,
        }
    );
}

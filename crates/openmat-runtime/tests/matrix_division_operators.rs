use openmat_array::{ArrayData, Complex32, DenseArray, Shape};
use openmat_bytecode::{
    BinaryOperator, BytecodeModule, Function, FunctionId, Instruction, InstructionKind, LocalSlot,
    Register, SourceLocation,
};
use openmat_runtime::{Interpreter, LinalgError, RuntimeError, RuntimeLinalgError};
use openmat_value::{Complex64, Value};

const DIVISION_LOCATION: SourceLocation = SourceLocation::new(91, 12, 19);

fn binary_module(operator: BinaryOperator) -> BytecodeModule {
    BytecodeModule::new(
        vec![Function {
            name: "matrix_division_operator".to_owned(),
            register_count: 3,
            pack_register_count: 0,
            local_count: 2,
            persistent_slot_count: 0,
            parameter_count: 2,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![
                Instruction::new(InstructionKind::LoadLocal {
                    dst: Register::new(0),
                    local: LocalSlot::new(0),
                }),
                Instruction::new(InstructionKind::LoadLocal {
                    dst: Register::new(1),
                    local: LocalSlot::new(1),
                }),
                Instruction::located(
                    InstructionKind::Binary {
                        operator,
                        dst: Register::new(2),
                        lhs: Register::new(0),
                        rhs: Register::new(1),
                    },
                    DIVISION_LOCATION,
                ),
                Instruction::new(InstructionKind::Return {
                    values: vec![Register::new(2)],
                }),
            ],
            exception_handlers: Vec::new(),
        }],
        FunctionId::new(0),
    )
}

fn execute_binary(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, RuntimeError> {
    let mut interpreter = Interpreter::new(binary_module(operator))
        .expect("matrix-division test bytecode should verify");
    interpreter
        .execute_entry(&[lhs.clone(), rhs.clone()])
        .map(|mut values| values.remove(0))
}

fn double_real(dimensions: [u64; 2], values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
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

fn double_complex(dimensions: [u64; 2], values: Vec<openmat_array::Complex64>) -> Value {
    Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn double_parts(value: &Value) -> (&[u64], &[f64]) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected a real double array, found {value:?}");
    };
    (array.shape().dimensions(), array.as_slice())
}

fn single_parts(value: &Value) -> (&[u64], &[f32]) {
    let Value::Array(ArrayData::F32(array)) = value else {
        panic!("expected a real single array, found {value:?}");
    };
    (array.shape().dimensions(), array.as_slice())
}

fn single_complex_parts(value: &Value) -> (&[u64], &[Complex32]) {
    let Value::Array(ArrayData::ComplexF32(array)) = value else {
        panic!("expected a complex single array, found {value:?}");
    };
    (array.shape().dimensions(), array.as_slice())
}

fn double_complex_parts(value: &Value) -> (&[u64], &[openmat_array::Complex64]) {
    let Value::Array(ArrayData::ComplexF64(array)) = value else {
        panic!("expected a complex double array, found {value:?}");
    };
    (array.shape().dimensions(), array.as_slice())
}

#[test]
fn scalar_left_divisions_compute_rhs_over_lhs_for_double_complex_and_logical() {
    for operator in [
        BinaryOperator::LeftDivide,
        BinaryOperator::ElementLeftDivide,
    ] {
        assert_eq!(
            execute_binary(operator, &Value::Double(2.0), &Value::Double(8.0)).unwrap(),
            Value::Double(4.0)
        );
        assert_eq!(
            execute_binary(operator, &Value::Logical(true), &Value::Logical(true)).unwrap(),
            Value::Double(1.0)
        );
        assert_eq!(
            execute_binary(
                operator,
                &Value::Complex(Complex64::new(1.0, 1.0)),
                &Value::Complex(Complex64::new(0.0, 2.0)),
            )
            .unwrap(),
            Value::Complex(Complex64::new(1.0, 1.0))
        );
    }

    for (lhs, rhs) in [
        (Value::Double(8.0), Value::Double(2.0)),
        (
            double_real([1, 1], vec![8.0]),
            double_real([1, 1], vec![2.0]),
        ),
    ] {
        let result = execute_binary(BinaryOperator::LeftDivide, &lhs, &rhs).unwrap();
        assert_eq!(result.as_real_number(), Some(0.25));
    }
    for (lhs, rhs) in [
        (Value::Double(2.0), Value::Double(8.0)),
        (
            double_real([1, 1], vec![2.0]),
            double_real([1, 1], vec![8.0]),
        ),
    ] {
        let result = execute_binary(BinaryOperator::Divide, &lhs, &rhs).unwrap();
        assert_eq!(result.as_real_number(), Some(0.25));
    }
}

#[test]
fn element_left_divide_expands_either_scalar_and_preserves_array_shape() {
    let rhs_array = double_real([1, 2], vec![4.0, 8.0]);
    let result = execute_binary(
        BinaryOperator::ElementLeftDivide,
        &Value::Double(2.0),
        &rhs_array,
    )
    .unwrap();
    assert_eq!(double_parts(&result), (&[1, 2][..], &[2.0, 4.0][..]));

    let lhs_array = double_real([2, 1], vec![2.0, 4.0]);
    let result = execute_binary(
        BinaryOperator::ElementLeftDivide,
        &lhs_array,
        &Value::Double(8.0),
    )
    .unwrap();
    assert_eq!(double_parts(&result), (&[2, 1][..], &[4.0, 2.0][..]));

    let logical_lhs = Value::Array(ArrayData::Logical(
        DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![true.into(), true.into()]).unwrap(),
    ));
    let result = execute_binary(
        BinaryOperator::ElementLeftDivide,
        &logical_lhs,
        &double_real([1, 2], vec![3.0, 5.0]),
    )
    .unwrap();
    assert_eq!(double_parts(&result), (&[1, 2][..], &[3.0, 5.0][..]));
}

#[test]
fn single_left_division_uses_the_exact_reverse_element_divide_path() {
    let lhs = single_real([1, 3], vec![2.0, 4.0, 8.0]);
    let rhs = single_real([1, 1], vec![16.0]);
    let left = execute_binary(BinaryOperator::ElementLeftDivide, &lhs, &rhs).unwrap();
    let reversed = execute_binary(BinaryOperator::ElementDivide, &rhs, &lhs).unwrap();
    assert_eq!(left, reversed);
    assert_eq!(single_parts(&left).0, &[1, 3]);
    assert_eq!(
        single_parts(&left)
            .1
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        [8.0_f32, 4.0, 2.0].map(f32::to_bits).to_vec()
    );

    let complex_lhs = single_complex([1, 1], vec![Complex32::new(1.0, 2.0)]);
    let complex_rhs = single_complex([1, 1], vec![Complex32::new(3.0, -2.0)]);
    let left = execute_binary(
        BinaryOperator::ElementLeftDivide,
        &complex_lhs,
        &complex_rhs,
    )
    .unwrap();
    let reversed =
        execute_binary(BinaryOperator::ElementDivide, &complex_rhs, &complex_lhs).unwrap();
    assert_eq!(left, reversed);

    let matrix_left = execute_binary(
        BinaryOperator::LeftDivide,
        &single_real([1, 1], vec![2.0]),
        &single_real([1, 1], vec![8.0]),
    )
    .unwrap();
    assert_eq!(single_parts(&matrix_left).1[0].to_bits(), 4.0_f32.to_bits());
}

#[test]
fn non_scalar_matrix_left_divide_executes_real_and_complex_single_solves() {
    let lhs = double_real([2, 2], vec![1.0, 0.0, 0.0, 1.0]);
    let rhs = double_real([2, 1], vec![3.0, 4.0]);
    let result = execute_binary(BinaryOperator::LeftDivide, &lhs, &rhs).unwrap();
    assert_eq!(double_parts(&result), (&[2, 1][..], &[3.0, 4.0][..]));

    let lhs = single_complex(
        [2, 2],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::ZERO,
            Complex32::ZERO,
            Complex32::new(2.0, -1.0),
        ],
    );
    let rhs = single_complex(
        [2, 1],
        vec![Complex32::new(2.0, 0.0), Complex32::new(3.0, 0.0)],
    );
    let result = execute_binary(BinaryOperator::LeftDivide, &lhs, &rhs).unwrap();
    assert_eq!(single_complex_parts(&result).0, &[2, 1]);
    for (&actual, expected) in single_complex_parts(&result)
        .1
        .iter()
        .zip([Complex32::new(1.0, -1.0), Complex32::new(1.2, 0.6)])
    {
        assert!((actual.re - expected.re).abs() <= 5.0e-6);
        assert!((actual.im - expected.im).abs() <= 5.0e-6);
    }
}

#[test]
fn matrix_right_divide_executes_conjugate_transposed_left_solve() {
    let left = double_complex(
        [1, 2],
        vec![
            openmat_array::Complex64::new(2.0, 1.0),
            openmat_array::Complex64::new(3.0, -2.0),
        ],
    );
    let right = double_complex(
        [3, 2],
        vec![
            openmat_array::Complex64::new(1.0, 1.0),
            openmat_array::Complex64::ZERO,
            openmat_array::Complex64::new(1.0, 0.0),
            openmat_array::Complex64::ZERO,
            openmat_array::Complex64::new(1.0, -1.0),
            openmat_array::Complex64::new(1.0, 0.0),
        ],
    );
    let result = execute_binary(BinaryOperator::Divide, &left, &right).unwrap();
    assert_eq!(double_complex_parts(&result).0, &[1, 3]);
    for (&actual, expected) in double_complex_parts(&result).1.iter().zip([
        openmat_array::Complex64::new(1.5, -0.5),
        openmat_array::Complex64::new(2.5, 0.5),
        openmat_array::Complex64::ZERO,
    ]) {
        assert!((actual.re - expected.re).abs() <= 1.0e-12);
        assert!((actual.im - expected.im).abs() <= 1.0e-12);
    }

    let single_left = single_complex(
        [1, 2],
        vec![Complex32::new(2.0, 1.0), Complex32::new(3.0, -2.0)],
    );
    let single_right = single_complex(
        [3, 2],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::ZERO,
            Complex32::new(1.0, 0.0),
            Complex32::ZERO,
            Complex32::new(1.0, -1.0),
            Complex32::new(1.0, 0.0),
        ],
    );
    let result = execute_binary(BinaryOperator::Divide, &single_left, &single_right).unwrap();
    assert_eq!(single_complex_parts(&result).0, &[1, 3]);
    for (&actual, expected) in single_complex_parts(&result).1.iter().zip([
        Complex32::new(1.5, -0.5),
        Complex32::new(2.5, 0.5),
        Complex32::ZERO,
    ]) {
        assert!((actual.re - expected.re).abs() <= 5.0e-6);
        assert!((actual.im - expected.im).abs() <= 5.0e-6);
    }
}

#[test]
fn matrix_division_shape_errors_keep_structured_linalg_details_and_location() {
    let error = execute_binary(
        BinaryOperator::LeftDivide,
        &double_real([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
        &double_real([3, 1], vec![1.0; 3]),
    )
    .unwrap_err();
    assert_eq!(error.location, Some(DIVISION_LOCATION));
    assert!(matches!(
        error.linalg_error(),
        Some(RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
            operation: "matrix left division coefficient and right-hand-side rows",
            left: 2,
            right: 3,
        }))
    ));

    let error = execute_binary(
        BinaryOperator::Divide,
        &double_real([1, 2], vec![1.0, 2.0]),
        &double_real([3, 3], vec![0.0; 9]),
    )
    .unwrap_err();
    assert_eq!(error.location, Some(DIVISION_LOCATION));
    assert!(matches!(
        error.linalg_error(),
        Some(RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
            operation: "matrix right division operand columns",
            left: 2,
            right: 3,
        }))
    ));
}

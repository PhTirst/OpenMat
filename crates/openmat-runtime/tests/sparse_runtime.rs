use std::sync::{Arc, Mutex};

use openmat_array::{ArrayData, Complex64, DenseArray, Shape};
use openmat_bytecode::{
    ApplyArgument, BinaryOperator, BytecodeModule, Constant, ConstantId, Function, FunctionId,
    Instruction, InstructionKind, LocalSlot, Register,
};
use openmat_linalg::LinalgError;
use openmat_runtime::{
    BuiltinContext, BuiltinRegistry, Interpreter, OutputError, OutputEvent, OutputSink,
    RuntimeErrorKind, RuntimeLinalgError,
};
use openmat_value::{CooEntry, SparseArrayData, Value};

fn instruction(kind: InstructionKind) -> Instruction {
    Instruction::new(kind)
}

fn sparse(rows: u64, columns: u64, entries: &[(u64, u64, f64)]) -> Value {
    Value::Sparse(
        SparseArrayData::try_from_f64_coo(
            rows,
            columns,
            entries
                .iter()
                .map(|&(row, column, value)| CooEntry::new(row, column, value))
                .collect(),
            entries.len(),
            None,
        )
        .unwrap(),
    )
}

fn dense(rows: u64, columns: u64, values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new([rows, columns]).unwrap(), values).unwrap(),
    ))
}

fn complex_sparse(rows: u64, columns: u64, entries: &[(u64, u64, Complex64)]) -> Value {
    Value::Sparse(
        SparseArrayData::try_from_complex_f64_coo(
            rows,
            columns,
            entries
                .iter()
                .map(|&(row, column, value)| CooEntry::new(row, column, value))
                .collect(),
            entries.len(),
            None,
        )
        .unwrap(),
    )
}

fn complex_dense(rows: u64, columns: u64, values: Vec<Complex64>) -> Value {
    Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(Shape::new([rows, columns]).unwrap(), values).unwrap(),
    ))
}

fn left_divide_interpreter() -> Interpreter {
    binary_interpreter(BinaryOperator::LeftDivide, "sparse_left_divide")
}

#[derive(Clone)]
struct SharedOutput(Arc<Mutex<Vec<OutputEvent>>>);

impl OutputSink for SharedOutput {
    fn emit(&mut self, event: OutputEvent) -> Result<(), OutputError> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
        Ok(())
    }
}

fn warning_left_divide_interpreter(
    expected_identifier: &'static str,
    expected_message_fragment: &'static str,
) -> (Interpreter, Arc<Mutex<Vec<OutputEvent>>>) {
    let function = Function {
        name: "sparse_warning_left_divide".to_owned(),
        register_count: 6,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: vec![Constant::String("inspect_sparse_warning".to_owned())],
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
                operator: BinaryOperator::LeftDivide,
                dst: Register::new(2),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::LoadFunctionHandle {
                dst: Register::new(3),
                name: ConstantId::new(0),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(4), Register::new(5)],
                target: Register::new(3),
                arguments: Vec::new(),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(2), Register::new(4), Register::new(5)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut registry = BuiltinRegistry::new();
    registry
        .register(
            "inspect_sparse_warning",
            move |_arguments: &[Value], context: &mut BuiltinContext<'_>| {
                let (message, identifier) = context.last_warning()?;
                Ok(vec![
                    Value::Logical(identifier == expected_identifier),
                    Value::Logical(message.contains(expected_message_fragment)),
                ])
            },
        )
        .expect("warning inspection built-in registration should succeed");
    let events = Arc::new(Mutex::new(Vec::new()));
    let interpreter = Interpreter::with_components(
        BytecodeModule::new(vec![function], FunctionId::new(0)),
        registry,
        Box::new(SharedOutput(Arc::clone(&events))),
    )
    .expect("warning sparse left-division bytecode should verify");
    (interpreter, events)
}

fn binary_interpreter(operator: BinaryOperator, name: &str) -> Interpreter {
    let function = Function {
        name: name.to_owned(),
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
                operator,
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
    Interpreter::new(BytecodeModule::new(vec![function], FunctionId::new(0)))
        .expect("sparse left-division bytecode should verify")
}

fn real_result(value: &Value) -> (&[u64], &[f64]) {
    let Value::Array(ArrayData::F64(value)) = value else {
        panic!("expected dense real result, found {value:?}")
    };
    (value.shape().dimensions(), value.as_slice())
}

fn complex_result(value: &Value) -> (&[u64], &[Complex64]) {
    let Value::Array(ArrayData::ComplexF64(value)) = value else {
        panic!("expected dense complex result, found {value:?}")
    };
    (value.shape().dimensions(), value.as_slice())
}

fn assert_real_close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (&actual, &expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() <= 1.0e-11,
            "expected {expected}, received {actual}"
        );
    }
}

fn full(value: &Value) -> Vec<f64> {
    let Value::Sparse(value) = value else {
        panic!("expected sparse result, found {value:?}")
    };
    let ArrayData::F64(array) = value.try_to_dense(None).unwrap() else {
        panic!("expected sparse real double result")
    };
    array.as_slice().to_vec()
}

#[test]
fn interpreter_dispatches_sparse_arithmetic_index_assignment_and_literals() {
    let function = Function {
        name: "sparse_runtime".to_owned(),
        register_count: 10,
        pack_register_count: 0,
        local_count: 4,
        persistent_slot_count: 0,
        parameter_count: 4,
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
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(3),
                local: LocalSlot::new(3),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: Register::new(4),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::Multiply,
                dst: Register::new(5),
                lhs: Register::new(0),
                rhs: Register::new(1),
            }),
            instruction(InstructionKind::Apply {
                outputs: vec![Register::new(6)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(7),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(2))],
                value: Register::new(3),
            }),
            instruction(InstructionKind::BuildMatrix {
                dst: Register::new(8),
                rows: vec![vec![Register::new(0), Register::new(1)]],
            }),
            instruction(InstructionKind::Binary {
                operator: BinaryOperator::ElementMultiply,
                dst: Register::new(9),
                lhs: Register::new(0),
                rhs: Register::new(1),
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
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(BytecodeModule::new(vec![function], FunctionId::new(0)))
        .expect("sparse runtime bytecode should verify");
    let lhs = sparse(2, 2, &[(0, 0, 1.0), (1, 1, 2.0)]);
    let rhs = sparse(2, 2, &[(1, 0, 5.0), (0, 1, 4.0)]);
    let indices = dense(1, 3, vec![4.0, 1.0, 3.0]);
    let supplied = dense(1, 3, vec![9.0, 0.0, 7.0]);
    let values = interpreter
        .execute_entry(&[lhs, rhs, indices, supplied])
        .expect("sparse operations should execute");

    assert_eq!(full(&values[0]), vec![1.0, 5.0, 4.0, 2.0]);
    assert_eq!(full(&values[1]), vec![0.0, 10.0, 4.0, 0.0]);
    assert_eq!(values[2].dimensions(), Some([1, 3].as_slice()));
    assert_eq!(full(&values[2]), vec![2.0, 1.0, 0.0]);
    assert_eq!(full(&values[3]), vec![0.0, 0.0, 7.0, 9.0]);
    assert_eq!(values[4].dimensions(), Some([2, 4].as_slice()));
    assert_eq!(
        full(&values[4]),
        vec![1.0, 0.0, 0.0, 2.0, 0.0, 5.0, 4.0, 0.0]
    );
    assert_eq!(full(&values[5]), vec![0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn interpreter_multiplies_sparse_and_full_matrices_in_both_orders() {
    let mut interpreter = binary_interpreter(BinaryOperator::Multiply, "sparse_full_multiply");
    let sparse_left = sparse(2, 3, &[(0, 0, 1.0), (1, 1, 2.0), (0, 2, 3.0), (1, 2, 4.0)]);
    let full_right = dense(3, 2, vec![5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
    let result = interpreter
        .execute_entry(&[sparse_left, full_right])
        .expect("sparse-by-full should apply every full column through SpMV");
    let (shape, values) = real_result(&result[0]);
    assert_eq!(shape, &[2, 2]);
    assert_real_close(values, &[26.0, 40.0, 38.0, 58.0]);

    let full_left = dense(2, 3, vec![5.0, 8.0, 6.0, 9.0, 7.0, 10.0]);
    let sparse_right = sparse(3, 2, &[(0, 0, 1.0), (2, 0, 3.0), (1, 1, 2.0), (2, 1, 4.0)]);
    let result = binary_interpreter(BinaryOperator::Multiply, "full_sparse_multiply")
        .execute_entry(&[full_left, sparse_right])
        .expect("full-by-sparse should preserve full result storage");
    let (shape, values) = real_result(&result[0]);
    assert_eq!(shape, &[2, 2]);
    assert_real_close(values, &[26.0, 38.0, 40.0, 58.0]);
}

#[test]
fn interpreter_multiplies_complex_and_empty_sparse_full_matrices() {
    let coefficients = complex_sparse(
        2,
        2,
        &[
            (0, 0, Complex64::new(1.0, 1.0)),
            (1, 1, Complex64::new(2.0, -1.0)),
        ],
    );
    let full = complex_dense(
        2,
        2,
        vec![
            Complex64::new(1.0, 0.0),
            Complex64::new(2.0, 1.0),
            Complex64::new(-1.0, 1.0),
            Complex64::new(3.0, -2.0),
        ],
    );
    let sparse_full = binary_interpreter(BinaryOperator::Multiply, "complex_sparse_full")
        .execute_entry(&[coefficients.clone(), full.clone()])
        .expect("complex sparse-by-full multiplication should execute");
    let (shape, values) = complex_result(&sparse_full[0]);
    assert_eq!(shape, &[2, 2]);
    let expected = [
        Complex64::new(1.0, 1.0),
        Complex64::new(5.0, 0.0),
        Complex64::new(-2.0, 0.0),
        Complex64::new(4.0, -7.0),
    ];
    for (&actual, expected) in values.iter().zip(expected) {
        assert!((actual.re - expected.re).abs() <= 1.0e-11);
        assert!((actual.im - expected.im).abs() <= 1.0e-11);
    }

    let full_sparse = binary_interpreter(BinaryOperator::Multiply, "complex_full_sparse")
        .execute_entry(&[full, coefficients])
        .expect("complex full-by-sparse multiplication should execute");
    let (shape, values) = complex_result(&full_sparse[0]);
    assert_eq!(shape, &[2, 2]);
    let expected = [
        Complex64::new(1.0, 1.0),
        Complex64::new(1.0, 3.0),
        Complex64::new(-1.0, 3.0),
        Complex64::new(4.0, -7.0),
    ];
    for (&actual, expected) in values.iter().zip(expected) {
        assert!((actual.re - expected.re).abs() <= 1.0e-11);
        assert!((actual.im - expected.im).abs() <= 1.0e-11);
    }

    let sparse_empty = binary_interpreter(BinaryOperator::Multiply, "sparse_full_empty")
        .execute_entry(&[sparse(2, 0, &[]), dense(0, 3, Vec::new())])
        .expect("sparse-by-full empty inner dimension should preserve 2x3 shape");
    let (shape, values) = real_result(&sparse_empty[0]);
    assert_eq!(shape, &[2, 3]);
    assert_eq!(values, &[0.0; 6]);

    let full_empty = binary_interpreter(BinaryOperator::Multiply, "full_sparse_empty")
        .execute_entry(&[dense(3, 0, Vec::new()), sparse(0, 2, &[])])
        .expect("full-by-sparse empty inner dimension should preserve 3x2 shape");
    let (shape, values) = real_result(&full_empty[0]);
    assert_eq!(shape, &[3, 2]);
    assert_eq!(values, &[0.0; 6]);
}

#[test]
fn interpreter_rejects_sparse_full_matrix_multiply_dimension_mismatch() {
    let error = binary_interpreter(BinaryOperator::Multiply, "sparse_full_shape_error")
        .execute_entry(&[sparse(2, 3, &[]), dense(2, 1, vec![1.0, 2.0])])
        .expect_err("sparse-by-full inner dimensions must agree");
    assert!(matches!(
        error.linalg_error(),
        Some(RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
            left: 3,
            right: 2,
            ..
        }))
    ));
}

#[test]
fn interpreter_deletes_sparse_linear_elements_and_whole_columns() {
    let function = Function {
        name: "sparse_delete".to_owned(),
        register_count: 6,
        pack_register_count: 0,
        local_count: 4,
        persistent_slot_count: 0,
        parameter_count: 4,
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
            instruction(InstructionKind::LoadLocal {
                dst: Register::new(3),
                local: LocalSlot::new(3),
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(4),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
                value: Register::new(3),
            }),
            instruction(InstructionKind::IndexAssign {
                dst: Register::new(5),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Colon, ApplyArgument::Value(Register::new(2))],
                value: Register::new(3),
            }),
            instruction(InstructionKind::Return {
                values: vec![Register::new(4), Register::new(5)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let mut interpreter = Interpreter::new(BytecodeModule::new(vec![function], FunctionId::new(0)))
        .expect("sparse deletion bytecode should verify");
    let source = sparse(2, 3, &[(0, 0, 10.0), (1, 1, 50.0), (0, 2, 30.0)]);
    let linear_indices = dense(1, 2, vec![2.0, 5.0]);
    let column_index = Value::Double(2.0);
    let empty = dense(0, 0, Vec::new());
    let values = interpreter
        .execute_entry(&[source, linear_indices, column_index, empty])
        .expect("measured sparse deletions should execute");

    assert_eq!(values[0].dimensions(), Some([1, 4].as_slice()));
    assert_eq!(full(&values[0]), vec![10.0, 0.0, 50.0, 0.0]);
    assert_eq!(values[1].dimensions(), Some([2, 2].as_slice()));
    assert_eq!(full(&values[1]), vec![10.0, 0.0, 30.0, 0.0]);
}

#[test]
fn interpreter_solves_square_sparse_systems_with_multiple_right_hand_sides() {
    let coefficients = sparse(2, 2, &[(0, 0, 3.0), (1, 0, 1.0), (0, 1, 1.0), (1, 1, 2.0)]);
    let right_hand_side = dense(2, 2, vec![9.0, 8.0, 7.0, 9.0]);
    let result = left_divide_interpreter()
        .execute_entry(&[coefficients, right_hand_side])
        .expect("faer sparse LU should solve every dense right-hand side");

    let (shape, values) = real_result(&result[0]);
    assert_eq!(shape, &[2, 2]);
    assert_real_close(values, &[2.0, 3.0, 1.0, 4.0]);
}

#[test]
fn interpreter_solves_tall_and_complex_sparse_systems() {
    let tall = sparse(3, 2, &[(0, 0, 1.0), (2, 0, 1.0), (1, 1, 1.0), (2, 1, 1.0)]);
    let tall_result = left_divide_interpreter()
        .execute_entry(&[tall, dense(3, 1, vec![2.0, 3.0, 5.0])])
        .expect("faer sparse QR should solve a full-rank tall system");
    let (shape, values) = real_result(&tall_result[0]);
    assert_eq!(shape, &[2, 1]);
    assert_real_close(values, &[2.0, 3.0]);

    let coefficients = complex_sparse(
        2,
        2,
        &[
            (0, 0, Complex64::new(1.0, 1.0)),
            (1, 1, Complex64::new(2.0, -1.0)),
        ],
    );
    let right_hand_side = complex_dense(
        2,
        1,
        vec![Complex64::new(-1.0, 3.0), Complex64::new(-1.5, 2.0)],
    );
    let result = left_divide_interpreter()
        .execute_entry(&[coefficients, right_hand_side])
        .expect("faer sparse complex LU should solve the system");
    let (shape, values) = complex_result(&result[0]);
    assert_eq!(shape, &[2, 1]);
    for (&actual, expected) in values
        .iter()
        .zip([Complex64::new(1.0, 2.0), Complex64::new(-1.0, 0.5)])
    {
        assert!((actual.re - expected.re).abs() <= 1.0e-11);
        assert!((actual.im - expected.im).abs() <= 1.0e-11);
    }

    let promoted = left_divide_interpreter()
        .execute_entry(&[
            sparse(2, 2, &[(0, 0, 2.0), (1, 1, 4.0)]),
            complex_dense(
                2,
                1,
                vec![Complex64::new(2.0, 4.0), Complex64::new(-4.0, 2.0)],
            ),
        ])
        .expect("real sparse coefficients should promote for a complex right-hand side");
    let (_, values) = complex_result(&promoted[0]);
    for (&actual, expected) in values
        .iter()
        .zip([Complex64::new(1.0, 2.0), Complex64::new(-1.0, 0.5)])
    {
        assert!((actual.re - expected.re).abs() <= 1.0e-11);
        assert!((actual.im - expected.im).abs() <= 1.0e-11);
    }
}

#[test]
fn interpreter_preserves_sparse_left_division_empty_shape_and_structured_errors() {
    let empty = left_divide_interpreter()
        .execute_entry(&[sparse(0, 0, &[]), dense(0, 2, Vec::new())])
        .expect("empty sparse left division should have an independent dense result");
    let (shape, values) = real_result(&empty[0]);
    assert_eq!(shape, &[0, 2]);
    assert!(values.is_empty());

    let dimension_error = left_divide_interpreter()
        .execute_entry(&[
            sparse(2, 2, &[(0, 0, 1.0), (1, 1, 1.0)]),
            dense(3, 1, vec![1.0, 2.0, 3.0]),
        ])
        .expect_err("row mismatch must remain a structured linalg error");
    assert!(matches!(
        dimension_error.linalg_error(),
        Some(RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
            left: 2,
            right: 3,
            ..
        }))
    ));

    let underdetermined = left_divide_interpreter()
        .execute_entry(&[
            sparse(2, 3, &[(0, 0, 1.0), (1, 1, 1.0), (0, 2, 1.0), (1, 2, 1.0)]),
            dense(2, 2, vec![1.0, 2.0, 2.0, 3.0]),
        ])
        .expect("wide full-row-rank sparse systems should return leftmost basic solutions");
    let (shape, values) = real_result(&underdetermined[0]);
    assert_eq!(shape, &[3, 2]);
    assert_real_close(values, &[1.0, 2.0, 0.0, 2.0, 3.0, 0.0]);

    let rank_deficient_wide = left_divide_interpreter()
        .execute_entry(&[
            sparse(2, 3, &[(0, 0, 1.0), (1, 0, 2.0), (0, 1, 2.0), (1, 1, 4.0)]),
            dense(2, 1, vec![1.0, 2.0]),
        ])
        .expect_err("a wide sparse matrix without full row rank remains structured");
    assert!(matches!(
        rank_deficient_wide.linalg_error(),
        Some(RuntimeLinalgError::Linalg(LinalgError::RankDeficient {
            required_rank: 2,
            ..
        }))
    ));
}

#[test]
fn interpreter_solves_complex_underdetermined_sparse_systems() {
    let coefficients = complex_sparse(
        2,
        3,
        &[
            (0, 0, Complex64::new(1.0, 1.0)),
            (1, 1, Complex64::new(2.0, -1.0)),
            (0, 2, Complex64::new(1.0, 0.0)),
            (1, 2, Complex64::new(1.0, 0.0)),
        ],
    );
    let expected = [
        Complex64::new(1.0, 2.0),
        Complex64::new(-1.0, 0.5),
        Complex64::ZERO,
        Complex64::new(2.0, -1.0),
        Complex64::new(3.0, 2.0),
        Complex64::ZERO,
    ];
    let right_hand_side = complex_dense(
        2,
        2,
        vec![
            Complex64::new(-1.0, 3.0),
            Complex64::new(-1.5, 2.0),
            Complex64::new(3.0, 1.0),
            Complex64::new(8.0, 1.0),
        ],
    );
    let result = left_divide_interpreter()
        .execute_entry(&[coefficients, right_hand_side])
        .expect("complex sparse basic solve should preserve multiple right-hand sides");
    let (shape, values) = complex_result(&result[0]);
    assert_eq!(shape, &[3, 2]);
    for (&actual, expected) in values.iter().zip(expected) {
        assert!((actual.re - expected.re).abs() <= 1.0e-11);
        assert!((actual.im - expected.im).abs() <= 1.0e-11);
    }
}

#[test]
fn interpreter_returns_singular_fallback_and_records_warning() {
    let (mut interpreter, events) =
        warning_left_divide_interpreter("OpenMat:Sparse:SingularMatrix", "zero pivot");
    let result = interpreter
        .execute_entry(&[
            sparse(2, 2, &[(0, 0, 1.0), (1, 0, 2.0), (0, 1, 2.0), (1, 1, 4.0)]),
            dense(2, 2, vec![1.0, 2.0, 2.0, 5.0]),
        ])
        .expect("singular sparse solve should return warning-bearing fallback values");
    let (shape, values) = real_result(&result[0]);
    assert_eq!(shape, &[2, 2]);
    assert!(values[0].is_nan() && values[1].is_nan());
    assert!(values[2].is_infinite() && values[2].is_sign_positive());
    assert!(values[3].is_infinite() && values[3].is_sign_negative());
    assert_eq!(result[1..], [Value::Logical(true), Value::Logical(true)]);
    let events = events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(matches!(
        events.as_slice(),
        [OutputEvent::CommandText(message)] if message.starts_with("Warning: ") && message.contains("zero pivot")
    ));

    let structural = left_divide_interpreter()
        .execute_entry(&[
            sparse(2, 2, &[(0, 1, 1.0), (1, 1, 2.0)]),
            dense(2, 1, vec![1.0, 2.0]),
        ])
        .expect("singular fallback should preserve a later solvable structural pivot");
    let (_, values) = real_result(&structural[0]);
    assert!(values[0].is_nan());
    assert!((values[1] - 1.0).abs() <= 1.0e-12);
}

#[test]
fn interpreter_returns_rank_deficient_basic_least_squares_and_records_warning() {
    let (mut interpreter, events) = warning_left_divide_interpreter(
        "OpenMat:Sparse:RankDeficientMatrix",
        "basic least-squares solution",
    );
    let result = interpreter
        .execute_entry(&[
            sparse(
                3,
                2,
                &[
                    (0, 0, 1.0),
                    (1, 0, 2.0),
                    (2, 0, 3.0),
                    (0, 1, 2.0),
                    (1, 1, 4.0),
                    (2, 1, 6.0),
                ],
            ),
            dense(3, 2, vec![1.0, 2.0, 3.0, 0.0, 1.0, 0.0]),
        ])
        .expect("rank-deficient sparse solve should return a basic least-squares result");
    let (shape, values) = real_result(&result[0]);
    assert_eq!(shape, &[2, 2]);
    assert_real_close(values, &[1.0, 0.0, 1.0 / 7.0, 0.0]);
    assert_eq!(result[1..], [Value::Logical(true), Value::Logical(true)]);
    let events = events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(matches!(
        events.as_slice(),
        [OutputEvent::CommandText(message)] if message.starts_with("Warning: ") && message.contains("basic least-squares solution")
    ));

    let all_zero = left_divide_interpreter()
        .execute_entry(&[
            sparse(3, 2, &[]),
            dense(3, 2, vec![1.0, 0.0, 3.0, 2.0, 0.0, 4.0]),
        ])
        .expect("zero tall sparse solve should return the zero basic least-squares result");
    let (shape, values) = real_result(&all_zero[0]);
    assert_eq!(shape, &[2, 2]);
    assert!(values.iter().all(|value| value.abs() <= f64::EPSILON));
}

#[test]
fn interpreter_returns_complex_singular_and_rank_deficient_fallbacks() {
    let singular_coefficients = complex_sparse(
        2,
        2,
        &[
            (0, 0, Complex64::new(1.0, 1.0)),
            (1, 0, Complex64::new(2.0, 2.0)),
            (0, 1, Complex64::new(2.0, 2.0)),
            (1, 1, Complex64::new(4.0, 4.0)),
        ],
    );
    let singular = left_divide_interpreter()
        .execute_entry(&[
            singular_coefficients,
            complex_dense(
                2,
                2,
                vec![
                    Complex64::new(1.0, 1.0),
                    Complex64::new(2.0, 2.0),
                    Complex64::new(1.0, 0.0),
                    Complex64::new(3.0, 0.0),
                ],
            ),
        ])
        .expect("complex singular sparse solve should return fallback values");
    let (shape, values) = complex_result(&singular[0]);
    assert_eq!(shape, &[2, 2]);
    assert!(
        values
            .iter()
            .all(|value| value.re.is_nan() && value.im.is_nan())
    );

    let rank_deficient_coefficients = complex_sparse(
        3,
        2,
        &[
            (0, 0, Complex64::new(1.0, 1.0)),
            (1, 0, Complex64::new(2.0, 2.0)),
            (2, 0, Complex64::new(3.0, 3.0)),
            (0, 1, Complex64::new(2.0, 2.0)),
            (1, 1, Complex64::new(4.0, 4.0)),
            (2, 1, Complex64::new(6.0, 6.0)),
        ],
    );
    let rank_deficient = left_divide_interpreter()
        .execute_entry(&[
            rank_deficient_coefficients,
            complex_dense(
                3,
                1,
                vec![
                    Complex64::new(1.0, 0.0),
                    Complex64::new(2.0, 0.0),
                    Complex64::new(3.0, 0.0),
                ],
            ),
        ])
        .expect("complex rank-deficient sparse solve should return a basic least-squares result");
    let (shape, values) = complex_result(&rank_deficient[0]);
    assert_eq!(shape, &[2, 1]);
    assert!((values[0].re - 0.5).abs() <= 1.0e-11);
    assert!((values[0].im + 0.5).abs() <= 1.0e-11);
    assert_eq!(values[1], Complex64::ZERO);
}

#[test]
fn interpreter_preserves_sparse_solve_cancellation() {
    let mut cancelled = left_divide_interpreter();
    cancelled.cancellation_token().cancel();
    let error = cancelled
        .execute_entry(&[
            sparse(2, 2, &[(0, 0, 1.0), (1, 1, 1.0)]),
            dense(2, 1, vec![1.0, 2.0]),
        ])
        .expect_err("pre-existing cancellation must stop before sparse provider dispatch");
    assert_eq!(error.kind, RuntimeErrorKind::Cancelled);
}

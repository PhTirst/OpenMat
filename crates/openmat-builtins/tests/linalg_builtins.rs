use openmat_array::{
    ArrayData, Complex32, Complex64, DenseArray, IntegerArrayData, Logical, Shape,
};
use openmat_builtins::minimal_registry;
use openmat_linalg::ReferenceProvider;
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinInvocationError, CancellationToken,
    NullOutput,
};
use openmat_value::Value;

fn invoke(
    name: &str,
    arguments: &[Value],
    requested_outputs: usize,
) -> Result<Vec<Value>, BuiltinError> {
    let registry = minimal_registry().expect("fresh registry");
    let handle = registry.handle_by_name(name).expect("registered built-in");
    let cancellation = CancellationToken::new();
    let mut output = NullOutput;
    let provider = ReferenceProvider;
    let mut context = BuiltinContext::with_linalg_provider(
        requested_outputs,
        &cancellation,
        &mut output,
        &provider,
    );
    registry
        .invoke(handle, arguments, &mut context)
        .map_err(|error| match error {
            BuiltinInvocationError::Failed { error, .. } => error,
            BuiltinInvocationError::UnknownHandle(_) => panic!("looked-up handle must resolve"),
        })
}

fn double_matrix(dimensions: [u64; 2], values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn complex_matrix(dimensions: [u64; 2], values: Vec<Complex64>) -> Value {
    Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn single_matrix(dimensions: [u64; 2], values: Vec<f32>) -> Value {
    Value::Array(ArrayData::F32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn complex_single_matrix(dimensions: [u64; 2], values: Vec<Complex32>) -> Value {
    Value::Array(ArrayData::ComplexF32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn f64_array(value: &Value) -> (&[f64], &[u64]) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected double array, received {value:?}");
    };
    (array.as_slice(), array.shape().dimensions())
}

fn f32_array(value: &Value) -> (&[f32], &[u64]) {
    let Value::Array(ArrayData::F32(array)) = value else {
        panic!("expected single array, received {value:?}");
    };
    (array.as_slice(), array.shape().dimensions())
}

fn complex32_array(value: &Value) -> (&[Complex32], &[u64]) {
    let Value::Array(ArrayData::ComplexF32(array)) = value else {
        panic!("expected complex single array, received {value:?}");
    };
    (array.as_slice(), array.shape().dimensions())
}

fn complex64_array(value: &Value) -> (&[Complex64], &[u64]) {
    let Value::Array(ArrayData::ComplexF64(array)) = value else {
        panic!("expected complex double array, received {value:?}");
    };
    (array.as_slice(), array.shape().dimensions())
}

#[test]
fn sqrtm_covers_principal_branch_precision_residual_and_errors() {
    let positive = double_matrix([2, 2], vec![4.0, 0.0, 1.0, 9.0]);
    let outputs = invoke("sqrtm", std::slice::from_ref(&positive), 2).unwrap();
    let (root, shape) = f64_array(&outputs[0]);
    assert_eq!(shape, &[2, 2]);
    assert_close(root, &[2.0, 0.0, 0.2, 3.0], 1.0e-10);
    let Value::Double(residual) = outputs[1] else {
        panic!("double sqrtm residual must be double");
    };
    assert!(residual < 1.0e-12);

    let negative = double_matrix([2, 2], vec![-1.0, 0.0, 0.0, 4.0]);
    let outputs = invoke("sqrtm", &[negative], 1).unwrap();
    let (root, shape) = complex64_array(&outputs[0]);
    assert_eq!(shape, &[2, 2]);
    assert_eq!(root[0], Complex64::new(0.0, 1.0));
    assert_eq!(root[3], Complex64::new(2.0, 0.0));

    let single = invoke(
        "sqrtm",
        &[single_matrix([2, 2], vec![4.0, 0.0, 1.0, 9.0])],
        2,
    )
    .unwrap();
    assert!((f32_array(&single[0]).0[2] - 0.2).abs() < 5.0e-5);
    let Value::Array(ArrayData::F32(single_residual)) = &single[1] else {
        panic!("single sqrtm residual must be single");
    };
    assert_eq!(single_residual.shape().dimensions(), &[1, 1]);

    let empty = invoke("sqrtm", &[double_matrix([0, 0], Vec::new())], 2).unwrap();
    assert_eq!(f64_array(&empty[0]).1, &[0, 0]);
    assert!(matches!(empty[1], Value::Double(value) if value.is_nan()));

    let rectangular = invoke("sqrtm", &[double_matrix([2, 3], vec![0.0; 6])], 1).unwrap_err();
    assert_eq!(
        rectangular.identifier.as_deref(),
        Some("MATLAB:sqrtm:inputMustBeSquare")
    );
    let nonfinite = invoke(
        "sqrtm",
        &[double_matrix([2, 2], vec![f64::NAN, 0.0, 0.0, 1.0])],
        1,
    )
    .unwrap_err();
    assert_eq!(
        nonfinite.identifier.as_deref(),
        Some("MATLAB:sqrtm:inputMustBeFinite")
    );
}

#[test]
fn sqrtm_three_outputs_are_finite_stability_and_condition_estimates() {
    let input = double_matrix([2, 2], vec![1.0, 3.0, 2.0, 4.0]);
    let outputs = invoke("sqrtm", &[input], 3).unwrap();
    assert_eq!(outputs.len(), 3);
    let Value::Double(alpha) = outputs[1] else {
        panic!("stability estimate must be double");
    };
    let Value::Double(condition) = outputs[2] else {
        panic!("condition estimate must be double");
    };
    assert!((alpha - 1.130_650_702_421_557_3).abs() < 1.0e-12);
    assert!((condition - 2.847_750_562_402_201).abs() < 1.0e-10);
}

fn multiply_real(
    left: &[f64],
    left_rows: usize,
    left_columns: usize,
    right: &[f64],
    right_columns: usize,
) -> Vec<f64> {
    let mut output = vec![0.0; left_rows * right_columns];
    for column in 0..right_columns {
        for inner in 0..left_columns {
            for row in 0..left_rows {
                output[column * left_rows + row] +=
                    left[inner * left_rows + row] * right[column * left_columns + inner];
            }
        }
    }
    output
}

fn assert_close(actual: &[f64], expected: &[f64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, received {actual}"
        );
    }
}

#[test]
fn inv_preserves_precision_complexness_empty_shapes_and_input_cow() {
    let input = double_matrix([2, 2], vec![1.0, 3.0, 2.0, 4.0]);
    let alias = input.clone();
    let output = invoke("inv", std::slice::from_ref(&input), 1).unwrap();
    let (values, shape) = f64_array(&output[0]);
    assert_eq!(shape, &[2, 2]);
    assert_close(values, &[-2.0, 1.5, 1.0, -0.5], 1.0e-12);
    assert!(input.shares_storage_with(&alias));
    assert!(!output[0].shares_storage_with(&input));

    let single = invoke("inv", &[single_matrix([2, 2], vec![1.0, 3.0, 2.0, 4.0])], 1).unwrap();
    let (values, shape) = f32_array(&single[0]);
    assert_eq!(shape, &[2, 2]);
    assert!(
        values
            .iter()
            .zip([-2.0_f32, 1.5, 1.0, -0.5])
            .all(|(actual, expected)| (*actual - expected).abs() <= 1.0e-5)
    );

    let complex = invoke(
        "inv",
        &[complex_single_matrix(
            [2, 2],
            vec![
                Complex32::new(1.0, 1.0),
                Complex32::new(0.0, 0.0),
                Complex32::new(0.0, 0.0),
                Complex32::new(2.0, -1.0),
            ],
        )],
        1,
    )
    .unwrap();
    let (values, shape) = complex32_array(&complex[0]);
    assert_eq!(shape, &[2, 2]);
    assert_eq!(values[0], Complex32::new(0.5, -0.5));
    assert_eq!(values[3], Complex32::new(0.4, 0.2));

    let empty = invoke("inv", &[double_matrix([0, 0], vec![])], 1).unwrap();
    assert_eq!(f64_array(&empty[0]), (&[][..], &[0, 0][..]));
    assert_eq!(
        invoke("inv", &[Value::Logical(true)], 1)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::Type
    );
    assert_eq!(
        invoke("inv", &[double_matrix([2, 2], vec![1.0, 2.0, 2.0, 4.0])], 1,)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::Domain
    );
}

#[test]
fn lu_covers_packed_folded_matrix_vector_rectangular_and_empty_contracts() {
    let input = double_matrix([2, 3], vec![0.0, 2.0, 1.0, 3.0, 2.0, 4.0]);
    let input_alias = input.clone();

    let packed = invoke("lu", std::slice::from_ref(&input), 1).unwrap();
    assert_eq!(
        f64_array(&packed[0]),
        (&[2.0, 0.0, 3.0, 1.0, 4.0, 2.0][..], &[2, 3][..])
    );
    assert!(!packed[0].shares_storage_with(&input));
    assert!(input.shares_storage_with(&input_alias));

    let folded = invoke("lu", std::slice::from_ref(&input), 2).unwrap();
    assert_eq!(
        f64_array(&folded[0]),
        (&[0.0, 1.0, 1.0, 0.0][..], &[2, 2][..])
    );
    assert_eq!(
        f64_array(&folded[1]),
        (&[2.0, 0.0, 3.0, 1.0, 4.0, 2.0][..], &[2, 3][..])
    );
    let product = multiply_real(f64_array(&folded[0]).0, 2, 2, f64_array(&folded[1]).0, 3);
    assert_close(&product, f64_array(&input).0, 1.0e-12);

    let matrix = invoke("lu", std::slice::from_ref(&input), 3).unwrap();
    assert_eq!(
        f64_array(&matrix[0]),
        (&[1.0, 0.0, 0.0, 1.0][..], &[2, 2][..])
    );
    assert_eq!(
        f64_array(&matrix[2]),
        (&[0.0, 1.0, 1.0, 0.0][..], &[2, 2][..])
    );

    let vector = invoke("lu", &[input.clone(), Value::from("vector")], 3).unwrap();
    assert_eq!(f64_array(&vector[2]), (&[2.0, 1.0][..], &[1, 2][..]));
    let ignored_for_two = invoke("lu", &[input, Value::from("vector")], 2).unwrap();
    assert_eq!(f64_array(&ignored_for_two[0]).0, &[0.0, 1.0, 1.0, 0.0]);

    let empty = single_matrix([0, 3], Vec::new());
    let outputs = invoke("lu", &[empty, Value::from("matrix")], 3).unwrap();
    assert_eq!(f32_array(&outputs[0]).1, &[0, 0]);
    assert_eq!(f32_array(&outputs[1]).1, &[0, 3]);
    assert_eq!(f32_array(&outputs[2]).1, &[0, 0]);
}

#[test]
fn qr_covers_one_two_three_output_economy_and_permutation_forms() {
    let input = double_matrix([3, 2], vec![1.0, 3.0, 5.0, 2.0, 4.0, 7.0]);

    let one = invoke("qr", std::slice::from_ref(&input), 1).unwrap();
    assert_eq!(f64_array(&one[0]).1, &[3, 2]);
    assert!(f64_array(&one[0]).0[1].abs() <= 1.0e-12);
    assert!(f64_array(&one[0]).0[2].abs() <= 1.0e-12);

    let two = invoke("qr", std::slice::from_ref(&input), 2).unwrap();
    assert_eq!(f64_array(&two[0]).1, &[3, 3]);
    assert_eq!(f64_array(&two[1]).1, &[3, 2]);
    let reconstructed = multiply_real(f64_array(&two[0]).0, 3, 3, f64_array(&two[1]).0, 2);
    assert_close(&reconstructed, f64_array(&input).0, 1.0e-12);

    let pivoted = invoke("qr", std::slice::from_ref(&input), 3).unwrap();
    assert_eq!(
        f64_array(&pivoted[2]),
        (&[0.0, 1.0, 1.0, 0.0][..], &[2, 2][..])
    );
    let reconstructed = multiply_real(f64_array(&pivoted[0]).0, 3, 3, f64_array(&pivoted[1]).0, 2);
    assert_close(&reconstructed, &[2.0, 4.0, 7.0, 1.0, 3.0, 5.0], 1.0e-12);

    let economy = invoke("qr", &[input.clone(), Value::Double(0.0)], 3).unwrap();
    assert_eq!(f64_array(&economy[0]).1, &[3, 2]);
    assert_eq!(f64_array(&economy[1]).1, &[2, 2]);
    assert_eq!(f64_array(&economy[2]), (&[2.0, 1.0][..], &[1, 2][..]));

    let integer_zero = Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![0_i8]).unwrap(),
    )));
    let integer_economy = invoke("qr", &[input.clone(), integer_zero], 3).unwrap();
    assert_eq!(f64_array(&integer_economy[0]).1, &[3, 2]);
    assert_eq!(f64_array(&integer_economy[2]).1, &[1, 2]);

    let economy_matrix = invoke("qr", &[input.clone(), Value::from("econ")], 3).unwrap();
    assert_eq!(f64_array(&economy_matrix[2]).1, &[2, 2]);
    let economy_vector = invoke(
        "qr",
        &[input, Value::from("econ"), Value::from("vector")],
        3,
    )
    .unwrap();
    assert_eq!(f64_array(&economy_vector[2]).1, &[1, 2]);
}

#[test]
fn qr_preserves_complex_single_and_wide_or_empty_shapes() {
    let input = complex_single_matrix(
        [2, 2],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::new(0.0, 0.0),
            Complex32::new(2.0, -1.0),
            Complex32::new(3.0, 0.5),
        ],
    );
    let outputs = invoke("qr", &[input, Value::from("vector")], 3).unwrap();
    assert_eq!(complex32_array(&outputs[0]).1, &[2, 2]);
    assert_eq!(complex32_array(&outputs[1]).1, &[2, 2]);
    assert_eq!(f64_array(&outputs[2]).1, &[1, 2]);

    let wide = double_matrix([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 7.0]);
    let outputs = invoke("qr", &[wide, Value::Double(0.0)], 2).unwrap();
    assert_eq!(f64_array(&outputs[0]).1, &[2, 2]);
    assert_eq!(f64_array(&outputs[1]).1, &[2, 3]);

    let empty = single_matrix([0, 3], Vec::new());
    let outputs = invoke("qr", std::slice::from_ref(&empty), 3).unwrap();
    assert_eq!(f32_array(&outputs[0]).1, &[0, 0]);
    assert_eq!(f32_array(&outputs[1]).1, &[0, 3]);
    assert_eq!(f32_array(&outputs[2]).1, &[3, 3]);
}

#[test]
fn chol_covers_upper_lower_status_partial_complex_diagonal_and_empty() {
    let positive = complex_matrix(
        [2, 2],
        vec![
            Complex64::new(4.0, 0.0),
            Complex64::new(1.0, -1.0),
            Complex64::new(1.0, 1.0),
            Complex64::new(3.0, 0.0),
        ],
    );
    let upper = invoke("chol", std::slice::from_ref(&positive), 2).unwrap();
    let Value::Array(ArrayData::ComplexF64(factor)) = &upper[0] else {
        panic!("complex Cholesky factor");
    };
    assert_eq!(factor.shape().dimensions(), &[2, 2]);
    assert_eq!(factor.as_slice()[0], Complex64::new(2.0, 0.0));
    assert_eq!(factor.as_slice()[2], Complex64::new(0.5, 0.5));
    assert_eq!(upper[1], Value::Double(0.0));

    let lower = invoke("chol", &[positive, Value::from("lower")], 2).unwrap();
    let Value::Array(ArrayData::ComplexF64(factor)) = &lower[0] else {
        panic!("complex lower Cholesky factor");
    };
    assert_eq!(factor.as_slice()[1], Complex64::new(0.5, -0.5));

    let indefinite = double_matrix([3, 3], vec![1.0, 2.0, 0.0, 2.0, 1.0, 0.0, 0.0, 0.0, 3.0]);
    let partial = invoke("chol", std::slice::from_ref(&indefinite), 2).unwrap();
    assert_eq!(partial[0], Value::Double(1.0));
    assert_eq!(partial[1], Value::Double(2.0));
    assert_eq!(
        invoke("chol", std::slice::from_ref(&indefinite), 1)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::Domain
    );

    let imaginary_diagonal = Value::Complex(openmat_value::Complex64::new(1.0, 1.0));
    let partial = invoke("chol", &[imaginary_diagonal], 2).unwrap();
    assert_eq!(f64_array(&partial[0]).1, &[0, 0]);
    assert_eq!(partial[1], Value::Double(1.0));

    let empty = complex_single_matrix([0, 0], Vec::new());
    let outputs = invoke("chol", &[empty], 2).unwrap();
    assert_eq!(f32_array(&outputs[0]).1, &[0, 0]);
    assert_eq!(outputs[1], Value::Double(0.0));
}

#[test]
fn determinant_uses_lu_parity_preserves_precision_complexness_and_empty_identity() {
    assert_eq!(
        invoke("det", &[double_matrix([2, 2], vec![0.0, 2.0, 1.0, 3.0])], 1,).unwrap(),
        vec![Value::Double(-2.0)]
    );
    let single = invoke("det", &[single_matrix([2, 2], vec![0.0, 2.0, 1.0, 3.0])], 1).unwrap();
    assert_eq!(f32_array(&single[0]), (&[-2.0][..], &[1, 1][..]));

    let complex = invoke(
        "det",
        &[complex_single_matrix(
            [2, 2],
            vec![
                Complex32::new(0.0, 1.0),
                Complex32::new(2.0, 0.0),
                Complex32::new(1.0, 0.0),
                Complex32::new(3.0, -1.0),
            ],
        )],
        1,
    )
    .unwrap();
    assert_eq!(complex32_array(&complex[0]).0, &[Complex32::new(-1.0, 3.0)]);

    assert_eq!(
        invoke("det", &[double_matrix([0, 0], Vec::new())], 1).unwrap(),
        vec![Value::Double(1.0)]
    );
    let single_empty = invoke("det", &[single_matrix([0, 0], Vec::new())], 1).unwrap();
    assert_eq!(f32_array(&single_empty[0]).0, &[1.0]);
    assert_eq!(
        invoke("det", &[double_matrix([2, 2], vec![1.0, 2.0, 2.0, 4.0])], 1,).unwrap(),
        vec![Value::Double(0.0)]
    );
}

#[test]
fn factorization_builtins_reject_types_shapes_options_arities_and_cancellation() {
    let logical = Value::Array(ArrayData::Logical(
        DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![Logical::TRUE; 4]).unwrap(),
    ));
    for name in ["lu", "qr", "chol", "det"] {
        assert_eq!(
            invoke(name, std::slice::from_ref(&logical), 1)
                .unwrap_err()
                .category,
            BuiltinErrorCategory::Type
        );
    }
    let rectangular = double_matrix([2, 3], vec![0.0; 6]);
    for name in ["chol", "det"] {
        assert_eq!(
            invoke(name, std::slice::from_ref(&rectangular), 1)
                .unwrap_err()
                .category,
            BuiltinErrorCategory::Domain
        );
    }
    assert_eq!(
        invoke(
            "qr",
            &[
                rectangular.clone(),
                Value::Double(0.0),
                Value::from("vector")
            ],
            3
        )
        .unwrap_err()
        .category,
        BuiltinErrorCategory::Domain
    );
    assert_eq!(
        invoke("lu", std::slice::from_ref(&rectangular), 4)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::ArgumentCount
    );

    let registry = minimal_registry().unwrap();
    let handle = registry.handle_by_name("qr").unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(2, &cancellation, &mut output);
    let error = registry
        .invoke(handle, &[rectangular], &mut context)
        .unwrap_err();
    let BuiltinInvocationError::Failed { error, .. } = error else {
        panic!("registered handle");
    };
    assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
}

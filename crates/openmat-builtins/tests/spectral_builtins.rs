use openmat_array::{ArrayData, Complex32, Complex64, DenseArray, Logical, Shape};
use openmat_builtins::minimal_registry;
use openmat_linalg::ReferenceProvider;
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinInvocationError, CancellationToken,
    NullOutput,
};
use openmat_value::{Complex64 as ValueComplex64, Value};

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

fn complex64_array(value: &Value) -> (&[Complex64], &[u64]) {
    let Value::Array(ArrayData::ComplexF64(array)) = value else {
        panic!("expected complex double array, received {value:?}");
    };
    (array.as_slice(), array.shape().dimensions())
}

fn complex32_array(value: &Value) -> (&[Complex32], &[u64]) {
    let Value::Array(ArrayData::ComplexF32(array)) = value else {
        panic!("expected complex single array, received {value:?}");
    };
    (array.as_slice(), array.shape().dimensions())
}

fn multiply_real(
    left: &[f64],
    left_rows: usize,
    inner: usize,
    right: &[f64],
    right_columns: usize,
) -> Vec<f64> {
    let mut output = vec![0.0; left_rows * right_columns];
    for column in 0..right_columns {
        for shared in 0..inner {
            for row in 0..left_rows {
                output[column * left_rows + row] +=
                    left[shared * left_rows + row] * right[column * inner + shared];
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
fn pinv_preserves_shapes_precision_complexness_and_explicit_tolerance() {
    let row = double_matrix([1, 2], vec![1.0, 2.0]);
    let output = invoke("pinv", std::slice::from_ref(&row), 1).unwrap();
    let (values, shape) = f64_array(&output[0]);
    assert_eq!(shape, &[2, 1]);
    assert_close(values, &[0.2, 0.4], 1.0e-12);

    let truncated = invoke("pinv", &[row, Value::Double(3.0)], 1).unwrap();
    assert_eq!(f64_array(&truncated[0]), (&[0.0, 0.0][..], &[2, 1][..]));

    let single = invoke("pinv", &[single_matrix([1, 2], vec![1.0, 2.0])], 1).unwrap();
    let (values, shape) = f32_array(&single[0]);
    assert_eq!(shape, &[2, 1]);
    assert!((values[0] - 0.2).abs() <= 1.0e-6);
    assert!((values[1] - 0.4).abs() <= 1.0e-6);

    let complex = invoke(
        "pinv",
        &[complex_matrix(
            [1, 2],
            vec![Complex64::new(1.0, 1.0), Complex64::new(0.0, 0.0)],
        )],
        1,
    )
    .unwrap();
    let (values, shape) = complex64_array(&complex[0]);
    assert_eq!(shape, &[2, 1]);
    assert!((values[0].re - 0.5).abs() <= 1.0e-12);
    assert!((values[0].im + 0.5).abs() <= 1.0e-12);

    let empty = invoke("pinv", &[double_matrix([0, 3], vec![])], 1).unwrap();
    assert_eq!(f64_array(&empty[0]), (&[][..], &[3, 0][..]));
    assert_eq!(
        invoke("pinv", &[Value::Logical(true)], 1)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::Type
    );
}

#[test]
fn svd_covers_full_thin_legacy_zero_vector_shapes_and_reconstruction() {
    let tall = double_matrix([3, 2], vec![3.0, 0.0, 0.0, 1.0, 2.0, 2.0]);
    let shared = tall.clone();
    let values = invoke("svd", std::slice::from_ref(&tall), 1).unwrap();
    assert_eq!(f64_array(&values[0]).1, &[2, 1]);
    assert!(!values[0].shares_storage_with(&tall));
    assert!(tall.shares_storage_with(&shared));

    let full = invoke("svd", std::slice::from_ref(&tall), 3).unwrap();
    assert_eq!(f64_array(&full[0]).1, &[3, 3]);
    assert_eq!(f64_array(&full[1]).1, &[3, 2]);
    assert_eq!(f64_array(&full[2]).1, &[2, 2]);
    let us = multiply_real(f64_array(&full[0]).0, 3, 3, f64_array(&full[1]).0, 2);
    let v = f64_array(&full[2]).0;
    let vh = vec![v[0], v[2], v[1], v[3]];
    let reconstructed = multiply_real(&us, 3, 2, &vh, 2);
    assert_close(&reconstructed, f64_array(&tall).0, 1.0e-9);

    let thin = invoke("svd", &[tall, Value::from("econ")], 3).unwrap();
    assert_eq!(f64_array(&thin[0]).1, &[3, 2]);
    assert_eq!(f64_array(&thin[1]).1, &[2, 2]);
    assert_eq!(f64_array(&thin[2]).1, &[2, 2]);

    let wide = double_matrix([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 7.0]);
    let legacy = invoke("svd", &[wide.clone(), Value::Double(0.0)], 3).unwrap();
    assert_eq!(f64_array(&legacy[1]).1, &[2, 3]);
    assert_eq!(f64_array(&legacy[2]).1, &[3, 3]);
    let economy = invoke("svd", &[wide.clone(), Value::from("econ")], 3).unwrap();
    assert_eq!(f64_array(&economy[1]).1, &[2, 2]);
    assert_eq!(f64_array(&economy[2]).1, &[3, 2]);
    let vector = invoke(
        "svd",
        &[wide, Value::from("econ"), Value::from("vector")],
        3,
    )
    .unwrap();
    assert_eq!(f64_array(&vector[1]).1, &[2, 1]);
}

#[test]
fn svd_preserves_complex_single_and_empty_output_contracts() {
    let input = complex_single_matrix(
        [2, 2],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::new(0.0, 0.0),
            Complex32::new(2.0, -1.0),
            Complex32::new(3.0, 0.5),
        ],
    );
    let output = invoke("svd", &[input, Value::from("econ")], 3).unwrap();
    assert_eq!(complex32_array(&output[0]).1, &[2, 2]);
    assert_eq!(f32_array(&output[1]).1, &[2, 2]);
    assert_eq!(complex32_array(&output[2]).1, &[2, 2]);

    let empty = single_matrix([0, 3], Vec::new());
    let full = invoke("svd", std::slice::from_ref(&empty), 3).unwrap();
    assert_eq!(f32_array(&full[0]).1, &[0, 0]);
    assert_eq!(f32_array(&full[1]).1, &[0, 3]);
    assert_eq!(f32_array(&full[2]).1, &[3, 3]);
    let thin = invoke("svd", &[empty, Value::from("econ")], 3).unwrap();
    assert_eq!(f32_array(&thin[1]).1, &[0, 0]);
    assert_eq!(f32_array(&thin[2]).1, &[3, 0]);
}

#[test]
fn eig_covers_value_matrix_vector_left_right_realness_and_precision() {
    let pair = double_matrix([2, 2], vec![0.0, 1.0, -1.0, 0.0]);
    let one = invoke("eig", std::slice::from_ref(&pair), 1).unwrap();
    let (values, shape) = complex64_array(&one[0]);
    assert_eq!(shape, &[2, 1]);
    assert!(values.iter().all(|value| value.re.abs() < 1.0e-10));
    assert!((values[0].im.abs() - 1.0).abs() < 1.0e-8);
    assert!((values[0].im + values[1].im).abs() < 1.0e-8);

    let matrix = invoke("eig", std::slice::from_ref(&pair), 3).unwrap();
    assert_eq!(complex64_array(&matrix[0]).1, &[2, 2]);
    assert_eq!(complex64_array(&matrix[1]).1, &[2, 2]);
    assert_eq!(complex64_array(&matrix[2]).1, &[2, 2]);
    let vector = invoke("eig", &[pair, Value::from("vector")], 3).unwrap();
    assert_eq!(complex64_array(&vector[1]).1, &[2, 1]);

    let real = double_matrix([2, 2], vec![2.0, 0.0, 1.0, 3.0]);
    let real_output = invoke("eig", &[real], 3).unwrap();
    assert_eq!(f64_array(&real_output[0]).1, &[2, 2]);
    assert_eq!(f64_array(&real_output[1]).1, &[2, 2]);
    assert_eq!(f64_array(&real_output[2]).1, &[2, 2]);

    let single = single_matrix([2, 2], vec![0.0, 1.0, -1.0, 0.0]);
    let single_output = invoke("eig", &[single], 3).unwrap();
    assert_eq!(complex32_array(&single_output[0]).1, &[2, 2]);
    assert_eq!(complex32_array(&single_output[1]).1, &[2, 2]);
    assert_eq!(complex32_array(&single_output[2]).1, &[2, 2]);

    let complex = complex_matrix(
        [2, 2],
        vec![
            Complex64::new(1.0, 0.5),
            Complex64::new(3.0, -1.0),
            Complex64::new(2.0, 1.0),
            Complex64::new(4.0, 0.0),
        ],
    );
    let complex_output = invoke("eig", &[complex, Value::from("vector")], 3).unwrap();
    assert_eq!(complex64_array(&complex_output[0]).1, &[2, 2]);
    assert_eq!(complex64_array(&complex_output[1]).1, &[2, 1]);
    assert_eq!(complex64_array(&complex_output[2]).1, &[2, 2]);

    let empty = double_matrix([0, 0], Vec::new());
    let empty_output = invoke("eig", &[empty, Value::from("vector")], 3).unwrap();
    assert_eq!(f64_array(&empty_output[0]).1, &[0, 0]);
    assert_eq!(f64_array(&empty_output[1]).1, &[0, 1]);
    assert_eq!(f64_array(&empty_output[2]).1, &[0, 0]);
}

#[test]
fn rank_uses_precision_specific_default_and_explicit_strict_tolerances() {
    let eps = f64::EPSILON;
    assert_eq!(
        invoke(
            "rank",
            &[double_matrix([2, 2], vec![1.0, 0.0, 0.0, 2.0 * eps])],
            1,
        )
        .unwrap(),
        vec![Value::Double(1.0)]
    );
    assert_eq!(
        invoke(
            "rank",
            &[double_matrix([2, 2], vec![1.0, 0.0, 0.0, 5.0 * eps])],
            1,
        )
        .unwrap(),
        vec![Value::Double(2.0)]
    );
    assert_eq!(
        invoke(
            "rank",
            &[
                double_matrix([2, 2], vec![1.0, 0.0, 0.0, 1.0e-12]),
                Value::Double(1.0e-10),
            ],
            1,
        )
        .unwrap(),
        vec![Value::Double(1.0)]
    );
    assert_eq!(
        invoke(
            "rank",
            &[
                double_matrix([2, 2], vec![1.0, 0.0, 0.0, 0.0]),
                Value::Double(-1.0),
            ],
            1,
        )
        .unwrap(),
        vec![Value::Double(2.0)]
    );

    let single_eps = f32::EPSILON;
    assert_eq!(
        invoke(
            "rank",
            &[single_matrix([2, 2], vec![1.0, 0.0, 0.0, 2.0 * single_eps],)],
            1,
        )
        .unwrap(),
        vec![Value::Double(1.0)]
    );
    assert_eq!(
        invoke("rank", &[double_matrix([0, 3], Vec::new())], 1).unwrap(),
        vec![Value::Double(0.0)]
    );
    assert_eq!(
        invoke("rank", &[Value::Double(f64::NAN)], 1)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::Domain
    );
}

#[test]
fn cond_covers_two_one_infinity_frobenius_single_singular_and_empty() {
    let input = double_matrix([2, 2], vec![1.0, 3.0, 2.0, 5.0]);
    let Value::Double(two) = invoke("cond", std::slice::from_ref(&input), 1).unwrap()[0] else {
        panic!("double condition number");
    };
    assert!((two - 38.974_342_094_2).abs() < 1.0e-8);
    for (option, expected) in [
        (Value::Double(1.0), 56.0),
        (Value::Double(f64::INFINITY), 56.0),
        (Value::from("fro"), 39.0),
    ] {
        let Value::Double(actual) = invoke("cond", &[input.clone(), option], 1).unwrap()[0] else {
            panic!("double condition number");
        };
        assert!((actual - expected).abs() < 1.0e-9);
    }

    let single = invoke(
        "cond",
        &[single_matrix([2, 2], vec![1.0, 3.0, 2.0, 5.0])],
        1,
    )
    .unwrap();
    assert_eq!(f32_array(&single[0]).1, &[1, 1]);
    assert!((f32_array(&single[0]).0[0] - 38.974_34).abs() < 1.0e-4);

    assert_eq!(
        invoke(
            "cond",
            &[double_matrix([2, 2], vec![1.0, 0.0, 0.0, 0.0])],
            1,
        )
        .unwrap(),
        vec![Value::Double(f64::INFINITY)]
    );
    assert_eq!(
        invoke("cond", &[double_matrix([0, 3], Vec::new())], 1).unwrap(),
        vec![Value::Double(0.0)]
    );
    assert_eq!(
        invoke(
            "cond",
            &[double_matrix([0, 3], Vec::new()), Value::Double(1.0)],
            1,
        )
        .unwrap_err()
        .category,
        BuiltinErrorCategory::Domain
    );
}

#[test]
fn spectral_builtins_reject_types_shapes_options_arities_and_cancellation() {
    let logical = Value::Array(ArrayData::Logical(
        DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![Logical::TRUE; 4]).unwrap(),
    ));
    for name in ["svd", "eig", "rank", "cond"] {
        assert_eq!(
            invoke(name, std::slice::from_ref(&logical), 1)
                .unwrap_err()
                .category,
            BuiltinErrorCategory::Type
        );
    }
    let rectangular = double_matrix([2, 3], vec![0.0; 6]);
    assert_eq!(
        invoke("eig", std::slice::from_ref(&rectangular), 1)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::Domain
    );
    assert_eq!(
        invoke("cond", &[rectangular.clone(), Value::Double(1.0)], 1)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::Domain
    );
    assert_eq!(
        invoke(
            "eig",
            &[
                double_matrix([2, 2], vec![1.0; 4]),
                Value::from("nobalance")
            ],
            1
        )
        .unwrap_err()
        .category,
        BuiltinErrorCategory::Domain
    );
    assert_eq!(
        invoke(
            "rank",
            &[
                double_matrix([2, 2], vec![1.0; 4]),
                Value::Complex(ValueComplex64::new(1.0, 0.0)),
            ],
            1,
        )
        .unwrap_err()
        .category,
        BuiltinErrorCategory::Type
    );
    assert_eq!(
        invoke("svd", std::slice::from_ref(&rectangular), 4)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::ArgumentCount
    );

    let registry = minimal_registry().unwrap();
    let handle = registry.handle_by_name("svd").unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(3, &cancellation, &mut output);
    let error = registry
        .invoke(handle, &[rectangular], &mut context)
        .unwrap_err();
    let BuiltinInvocationError::Failed { error, .. } = error else {
        panic!("registered handle");
    };
    assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
}

#[test]
fn complex_double_rank_and_cond_dispatch_without_precision_widening_boundaries() {
    let input = complex_matrix(
        [2, 2],
        vec![
            Complex64::new(1.0, 1.0),
            Complex64::new(0.0, 0.0),
            Complex64::new(2.0, -1.0),
            Complex64::new(3.0, 0.5),
        ],
    );
    assert_eq!(
        invoke("rank", std::slice::from_ref(&input), 1).unwrap()[0],
        Value::Double(2.0)
    );
    let Value::Double(condition) = invoke("cond", &[input, Value::from("fro")], 1).unwrap()[0]
    else {
        panic!("double condition number");
    };
    assert!(condition.is_finite() && condition > 1.0);

    let single = complex_single_matrix(
        [2, 2],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::new(0.0, 0.0),
            Complex32::new(2.0, -1.0),
            Complex32::new(3.0, 0.5),
        ],
    );
    assert_eq!(
        invoke("rank", std::slice::from_ref(&single), 1).unwrap()[0],
        Value::Double(2.0)
    );
    let condition = invoke("cond", &[single, Value::from("fro")], 1).unwrap();
    assert!(f32_array(&condition[0]).0[0].is_finite());
}

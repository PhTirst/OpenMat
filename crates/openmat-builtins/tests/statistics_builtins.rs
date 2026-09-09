use openmat_array::{
    ArrayData, CharCodeUnit, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData, Logical, Shape,
};
use openmat_builtins::minimal_registry;
use openmat_runtime::{
    BuiltinContext, BuiltinErrorCategory, BuiltinResult, CancellationToken, VecOutput,
};
use openmat_value::{CellArray, Value};

fn invoke(name: &str, arguments: &[Value], outputs: usize) -> BuiltinResult {
    let registry = minimal_registry().unwrap();
    let handle = registry.handle_by_name(name).expect("registered statistic");
    let cancellation = CancellationToken::new();
    let mut output = VecOutput::new();
    let mut context = BuiltinContext::new(outputs, &cancellation, &mut output);
    registry
        .invoke(handle, arguments, &mut context)
        .map_err(|error| match error {
            openmat_runtime::BuiltinInvocationError::Failed { error, .. } => error,
            openmat_runtime::BuiltinInvocationError::UnknownHandle(_) => unreachable!(),
        })
}

fn real_array<const N: usize>(dimensions: [u64; N], values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn complex_array<const N: usize>(dimensions: [u64; N], values: Vec<ArrayComplex64>) -> Value {
    Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn single_array<const N: usize>(dimensions: [u64; N], values: Vec<f32>) -> Value {
    Value::Array(ArrayData::F32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn logical_array<const N: usize>(dimensions: [u64; N], values: Vec<bool>) -> Value {
    Value::Array(ArrayData::Logical(
        DenseArray::from_vec(
            Shape::new(dimensions).unwrap(),
            values.into_iter().map(Logical::from).collect(),
        )
        .unwrap(),
    ))
}

fn integer_array<const N: usize>(dimensions: [u64; N], values: Vec<i8>) -> Value {
    Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    )))
}

fn char_array<const N: usize>(dimensions: [u64; N], values: Vec<u16>) -> Value {
    Value::Array(ArrayData::Char(
        DenseArray::from_vec(
            Shape::new(dimensions).unwrap(),
            values.into_iter().map(CharCodeUnit::new).collect(),
        )
        .unwrap(),
    ))
}

fn f64_values(value: &Value) -> (Vec<u64>, Vec<ArrayComplex64>) {
    match value {
        Value::Double(value) => (vec![1, 1], vec![ArrayComplex64::new(*value, 0.0)]),
        Value::Complex(value) => (
            vec![1, 1],
            vec![ArrayComplex64::new(value.real, value.imaginary)],
        ),
        Value::Array(ArrayData::F64(array)) => (
            array.shape().dimensions().to_vec(),
            array
                .as_slice()
                .iter()
                .map(|value| ArrayComplex64::new(*value, 0.0))
                .collect(),
        ),
        Value::Array(ArrayData::ComplexF64(array)) => (
            array.shape().dimensions().to_vec(),
            array.as_slice().to_vec(),
        ),
        other => panic!("expected double output, got {}", other.class_name()),
    }
}

fn assert_real(value: &Value, shape: &[u64], expected: &[f64], tolerance: f64) {
    let (actual_shape, actual) = f64_values(value);
    assert_eq!(actual_shape, shape);
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        if expected.is_nan() {
            assert!(actual.re.is_nan());
        } else {
            assert!(
                (actual.re - expected).abs() <= tolerance,
                "{} != {}",
                actual.re,
                expected
            );
            assert!(actual.im.abs() <= f64::EPSILON);
        }
    }
}

fn cell(value: &Value) -> &CellArray {
    let Value::Cell(value) = value else {
        panic!("expected cell array");
    };
    value
}

#[test]
fn statistics_are_registered_and_mode_preserves_ties_frequency_and_shape() {
    let registry = minimal_registry().unwrap();
    for name in ["mode", "range", "bounds", "cov", "corrcoef"] {
        assert!(registry.handle_by_name(name).is_some(), "missing {name}");
    }

    let outputs = invoke("mode", &[real_array([1, 4], vec![3.0, 1.0, 3.0, 1.0])], 3).unwrap();
    assert_real(&outputs[0], &[1, 1], &[1.0], 0.0);
    assert_real(&outputs[1], &[1, 1], &[2.0], 0.0);
    let ties = cell(&outputs[2]);
    assert_eq!(ties.shape().dimensions(), &[1, 1]);
    assert_real(&ties.values()[0], &[2, 1], &[1.0, 3.0], 0.0);

    let nan_outputs = invoke(
        "mode",
        &[real_array([1, 3], vec![f64::NAN, f64::NAN, 1.0])],
        3,
    )
    .unwrap();
    assert_real(&nan_outputs[0], &[1, 1], &[1.0], 0.0);
    assert_real(&nan_outputs[1], &[1, 1], &[1.0], 0.0);
    let ties = cell(&nan_outputs[2]);
    let (_, tied_values) = f64_values(&ties.values()[0]);
    assert_eq!(tied_values.len(), 3);
    assert!((tied_values[0].re - 1.0).abs() <= f64::EPSILON);
    assert!(tied_values[1].re.is_nan() && tied_values[2].re.is_nan());
}

#[test]
fn mode_covers_dimension_vectors_native_classes_and_empty_boundaries() {
    let dimensions = real_array([1, 2], vec![1.0, 2.0]);
    let input = real_array([2, 2, 2], vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0]);
    let output = invoke("mode", &[input, dimensions], 2).unwrap();
    assert_real(&output[0], &[1, 1, 2], &[1.0, 3.0], 0.0);
    assert_real(&output[1], &[1, 1, 2], &[2.0, 2.0], 0.0);

    let logical = invoke("mode", &[logical_array([1, 3], vec![true, false, true])], 1).unwrap();
    assert!(
        matches!(&logical[0], Value::Array(ArrayData::Logical(array)) if array.as_slice() == [Logical::TRUE])
    );

    let integer = invoke("mode", &[integer_array([1, 3], vec![2, 1, 1])], 1).unwrap();
    assert!(
        matches!(&integer[0], Value::Array(ArrayData::Integer(IntegerArrayData::I8(array))) if array.as_slice() == [1])
    );

    let characters = invoke("mode", &[char_array([1, 4], vec![98, 97, 97, 98])], 3).unwrap();
    assert!(
        matches!(&characters[0], Value::Array(ArrayData::Char(array)) if array.as_slice() == [CharCodeUnit::new(97)])
    );
    assert_eq!(
        cell(&characters[2]).values()[0].dimensions(),
        Some([2, 1].as_slice())
    );

    let empty = invoke("mode", &[single_array([0, 3], Vec::new())], 2).unwrap();
    let Value::Array(ArrayData::F32(values)) = &empty[0] else {
        panic!("empty single mode must stay single");
    };
    assert_eq!(values.shape().dimensions(), &[1, 3]);
    assert!(values.as_slice().iter().all(|value| value.is_nan()));
    assert_real(&empty[1], &[1, 3], &[0.0, 0.0, 0.0], 0.0);

    let error = invoke("mode", &[char_array([0, 3], Vec::new())], 1).unwrap_err();
    assert_eq!(error.category, BuiltinErrorCategory::Domain);
    let error = invoke(
        "mode",
        &[real_array([1, 2], vec![1.0, 2.0]), Value::from("omitnan")],
        1,
    )
    .unwrap_err();
    assert_eq!(error.category, BuiltinErrorCategory::Type);
}

#[test]
fn range_and_bounds_cover_nan_complex_classes_dimensions_and_shaped_empty() {
    let input = real_array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_real(
        &invoke("range", std::slice::from_ref(&input), 1).unwrap()[0],
        &[1, 3],
        &[1.0, 1.0, 1.0],
        0.0,
    );
    let bounds = invoke("bounds", &[input, Value::Double(2.0)], 2).unwrap();
    assert_real(&bounds[0], &[2, 1], &[1.0, 2.0], 0.0);
    assert_real(&bounds[1], &[2, 1], &[5.0, 6.0], 0.0);

    let omitted = invoke("range", &[real_array([1, 3], vec![f64::NAN, 1.0, 3.0])], 1).unwrap();
    assert_real(&omitted[0], &[1, 1], &[2.0], 0.0);
    let included = invoke(
        "bounds",
        &[
            real_array([1, 3], vec![f64::NAN, 1.0, 3.0]),
            Value::from("includenan"),
        ],
        2,
    )
    .unwrap();
    assert_real(&included[0], &[1, 1], &[f64::NAN], 0.0);
    assert_real(&included[1], &[1, 1], &[f64::NAN], 0.0);

    let complex = invoke(
        "range",
        &[complex_array(
            [1, 2],
            vec![ArrayComplex64::new(1.0, 1.0), ArrayComplex64::new(2.0, 2.0)],
        )],
        1,
    )
    .unwrap();
    let (_, values) = f64_values(&complex[0]);
    assert_eq!(values, [ArrayComplex64::new(1.0, 1.0)]);
    let complex_single = Value::Array(ArrayData::ComplexF32(
        DenseArray::from_vec(
            Shape::new([1, 2]).unwrap(),
            vec![ArrayComplex32::new(1.0, 1.0), ArrayComplex32::new(2.0, 2.0)],
        )
        .unwrap(),
    ));
    let complex_single = invoke("range", &[complex_single], 1).unwrap();
    assert!(
        matches!(&complex_single[0], Value::Array(ArrayData::ComplexF32(array)) if array.as_slice() == [ArrayComplex32::new(1.0, 1.0)])
    );

    let integer = invoke("range", &[integer_array([1, 2], vec![i8::MIN, i8::MAX])], 1).unwrap();
    assert!(
        matches!(&integer[0], Value::Array(ArrayData::Integer(IntegerArrayData::I8(array))) if array.as_slice() == [i8::MAX])
    );
    let logical = invoke("range", &[logical_array([1, 2], vec![false, true])], 1).unwrap();
    assert_real(&logical[0], &[1, 1], &[1.0], 0.0);

    let empty = invoke("bounds", &[real_array([0, 3], Vec::new())], 2).unwrap();
    assert_real(&empty[0], &[0, 3], &[], 0.0);
    assert_real(&empty[1], &[0, 3], &[], 0.0);
    assert_eq!(
        invoke("bounds", &[real_array([1, 2], vec![1.0, 2.0])], 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn covariance_matches_matrix_vector_complex_rows_and_single_rules() {
    let matrix = real_array([3, 2], vec![1.0, 2.0, 4.0, 10.0, 20.0, 40.0]);
    let covariance = invoke("cov", &[matrix], 1).unwrap();
    assert_real(
        &covariance[0],
        &[2, 2],
        &[7.0 / 3.0, 70.0 / 3.0, 70.0 / 3.0, 700.0 / 3.0],
        1.0e-12,
    );

    let vectors = invoke(
        "cov",
        &[
            real_array([1, 3], vec![1.0, 2.0, 3.0]),
            real_array([3, 1], vec![2.0, 4.0, 8.0]),
        ],
        1,
    )
    .unwrap();
    assert_real(&vectors[0], &[2, 2], &[1.0, 3.0, 3.0, 28.0 / 3.0], 1.0e-12);

    let partial = invoke(
        "cov",
        &[
            real_array([3, 2], vec![1.0, 2.0, f64::NAN, f64::NAN, 4.0, 8.0]),
            Value::from("partialrows"),
        ],
        1,
    )
    .unwrap();
    assert_real(&partial[0], &[2, 2], &[0.5, 0.0, 0.0, 8.0], 1.0e-12);

    let complex = invoke(
        "cov",
        &[complex_array(
            [3, 2],
            vec![
                ArrayComplex64::new(1.0, 1.0),
                ArrayComplex64::new(2.0, 3.0),
                ArrayComplex64::new(4.0, 2.0),
                ArrayComplex64::new(2.0, 0.0),
                ArrayComplex64::new(5.0, 0.0),
                ArrayComplex64::new(9.0, 0.0),
            ],
        )],
        1,
    )
    .unwrap();
    let (_, values) = f64_values(&complex[0]);
    assert!((values[1].re - 16.0 / 3.0).abs() < 1.0e-12);
    assert!((values[1].im - 1.5).abs() < 1.0e-12);
    assert_eq!(values[2], ArrayComplex64::new(values[1].re, -values[1].im));

    let single = invoke(
        "cov",
        &[
            single_array([1, 3], vec![1.0, 2.0, 3.0]),
            real_array([1, 3], vec![2.0, 4.0, 8.0]),
        ],
        1,
    )
    .unwrap();
    assert!(
        matches!(&single[0], Value::Array(ArrayData::F32(array)) if array.shape().dimensions() == [2, 2])
    );

    let integer_error = invoke("cov", &[integer_array([1, 3], vec![1, 2, 3])], 1).unwrap_err();
    assert_eq!(integer_error.category, BuiltinErrorCategory::Type);

    let scalar_pair = invoke("cov", &[Value::Double(1.0), Value::Double(3.0)], 1).unwrap();
    assert_real(&scalar_pair[0], &[2, 2], &[0.0, 0.0, 0.0, 0.0], 0.0);
}

#[test]
fn corrcoef_matches_rows_complex_and_all_inference_outputs() {
    let result = invoke(
        "corrcoef",
        &[
            real_array([1, 4], vec![1.0, 2.0, 4.0, 8.0]),
            real_array([1, 4], vec![2.0, 1.0, 3.0, 9.0]),
        ],
        4,
    )
    .unwrap();
    assert_real(
        &result[0],
        &[2, 2],
        &[1.0, 0.951_237_475_566_885_5, 0.951_237_475_566_885_5, 1.0],
        1.0e-14,
    );
    assert_real(
        &result[1],
        &[2, 2],
        &[1.0, 0.048_762_524_433_114_49, 0.048_762_524_433_114_49, 1.0],
        1.0e-12,
    );
    assert_real(
        &result[2],
        &[2, 2],
        &[1.0, -0.114_826_794_280_545_4, -0.114_826_794_280_545_4, 1.0],
        1.0e-12,
    );
    assert_real(
        &result[3],
        &[2, 2],
        &[1.0, 0.999_008_739_823_477_4, 0.999_008_739_823_477_4, 1.0],
        1.0e-12,
    );

    let pairwise = invoke(
        "corrcoef",
        &[
            real_array([4, 2], vec![1.0, 2.0, 3.0, 4.0, f64::NAN, 4.0, 8.0, 16.0]),
            Value::from("Rows"),
            Value::from("pairwise"),
        ],
        2,
    )
    .unwrap();
    assert_real(
        &pairwise[0],
        &[2, 2],
        &[1.0, 0.981_980_506_061_965_9, 0.981_980_506_061_965_9, 1.0],
        1.0e-14,
    );
    assert_real(
        &pairwise[1],
        &[2, 2],
        &[1.0, 0.121_037_718_323_676_27, 0.121_037_718_323_676_27, 1.0],
        1.0e-12,
    );

    let complex = invoke(
        "corrcoef",
        &[complex_array(
            [3, 2],
            vec![
                ArrayComplex64::new(1.0, 1.0),
                ArrayComplex64::new(2.0, 3.0),
                ArrayComplex64::new(4.0, 2.0),
                ArrayComplex64::new(2.0, 0.0),
                ArrayComplex64::new(5.0, 0.0),
                ArrayComplex64::new(9.0, 0.0),
            ],
        )],
        1,
    )
    .unwrap();
    let (_, values) = f64_values(&complex[0]);
    assert!((values[1].re - 0.831_800_391_856_058).abs() < 1.0e-14);
    assert!((values[1].im - 0.233_943_860_209_516_32).abs() < 1.0e-14);

    let error = invoke(
        "corrcoef",
        &[complex_array(
            [1, 2],
            vec![ArrayComplex64::new(1.0, 1.0), ArrayComplex64::new(2.0, 2.0)],
        )],
        2,
    )
    .unwrap_err();
    assert_eq!(error.category, BuiltinErrorCategory::Type);

    let constant = invoke("corrcoef", &[real_array([1, 3], vec![1.0, 1.0, 1.0])], 1).unwrap();
    assert_real(&constant[0], &[1, 1], &[f64::NAN], 0.0);
    let empty = invoke("corrcoef", &[real_array([1, 0], Vec::new())], 1).unwrap();
    assert_real(&empty[0], &[1, 1], &[f64::NAN], 0.0);
}

#[test]
fn statistics_reject_output_and_option_errors_stably() {
    let input = real_array([1, 3], vec![1.0, 2.0, 3.0]);
    assert_eq!(
        invoke("range", std::slice::from_ref(&input), 2)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::ArgumentCount
    );
    assert_eq!(
        invoke("bounds", std::slice::from_ref(&input), 3)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::ArgumentCount
    );
    assert_eq!(
        invoke("cov", &[input.clone(), Value::Double(2.0)], 1)
            .unwrap_err()
            .category,
        BuiltinErrorCategory::Domain
    );
    assert_eq!(
        invoke(
            "corrcoef",
            &[input, Value::from("Alpha"), Value::Double(1.0)],
            1,
        )
        .unwrap_err()
        .category,
        BuiltinErrorCategory::Domain
    );
}

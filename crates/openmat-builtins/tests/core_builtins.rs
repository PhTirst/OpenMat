use std::sync::Arc;

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DType,
    DenseArray, IntegerArrayData, IntegerElement, Logical, Shape,
};
use openmat_builtins::{BuiltinRegistry, minimal_registry};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult, CancellationToken, VecOutput,
};
use openmat_value::{
    CellArray, Complex64, CooEntry, FieldName, SparseArrayData, SparseScalarValue, StringArray,
    StringElement, StringValue, Value,
};

fn invoke(name: &str, arguments: &[Value]) -> BuiltinResult {
    invoke_with_outputs(name, arguments, 1)
}

fn invoke_with_outputs(name: &str, arguments: &[Value], requested_outputs: usize) -> BuiltinResult {
    let registry = minimal_registry().expect("fresh registry must register the core built-ins");
    invoke_in_registry(&registry, name, arguments, requested_outputs)
}

fn invoke_in_registry(
    registry: &BuiltinRegistry,
    name: &str,
    arguments: &[Value],
    requested_outputs: usize,
) -> BuiltinResult {
    let cancellation = CancellationToken::new();
    invoke_in_registry_with_cancellation(
        registry,
        name,
        arguments,
        requested_outputs,
        &cancellation,
    )
}

fn invoke_in_registry_with_cancellation(
    registry: &BuiltinRegistry,
    name: &str,
    arguments: &[Value],
    requested_outputs: usize,
    cancellation: &CancellationToken,
) -> BuiltinResult {
    let handle = registry
        .handle_by_name(name)
        .expect("requested core built-in must be registered");
    let mut output = VecOutput::new();
    let mut context = BuiltinContext::new(requested_outputs, cancellation, &mut output);
    registry
        .invoke(handle, arguments, &mut context)
        .map_err(|error| match error {
            openmat_runtime::BuiltinInvocationError::Failed { error, .. } => error,
            openmat_runtime::BuiltinInvocationError::UnknownHandle(_) => {
                panic!("looked-up built-in handle must resolve")
            }
        })
}

#[test]
fn single_storage_merge_preserves_unsupported_dense_and_sparse_entry_points() {
    let scalar = Value::from(ArrayData::from(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![2.0_f32]).unwrap(),
    ));
    let empty = Value::from(ArrayData::from(
        DenseArray::<f32>::from_vec(Shape::new([0, 0]).unwrap(), vec![]).unwrap(),
    ));
    for value in [scalar, empty] {
        for name in ["sparse", "full", "zeros"] {
            assert_error(
                invoke(name, std::slice::from_ref(&value)),
                BuiltinErrorCategory::Type,
            );
        }
        assert_error(
            invoke("reshape", &[Value::Double(1.0), value, Value::Double(1.0)]),
            BuiltinErrorCategory::Type,
        );
    }
}

#[test]
fn pi_matches_r2022b_double_constant_and_rejects_inputs() {
    assert_eq!(
        invoke("pi", &[]),
        Ok(vec![Value::Double(std::f64::consts::PI)])
    );
    assert_error(
        invoke("pi", &[Value::Double(1.0)]),
        BuiltinErrorCategory::ArgumentCount,
    );
}

#[test]
fn error_and_assert_preserve_user_identifiers_and_formatted_messages() {
    let error = invoke_with_outputs(
        "error",
        &[
            Value::from("Probe:Expected"),
            Value::from("value %d"),
            Value::Double(7.0),
        ],
        0,
    )
    .unwrap_err();
    assert_eq!(error.identifier.as_deref(), Some("Probe:Expected"));
    assert_eq!(error.message, "value 7");

    assert_eq!(
        invoke_with_outputs("assert", &[Value::Logical(true)], 0),
        Ok(Vec::new())
    );
    let failed = invoke_with_outputs("assert", &[Value::Logical(false)], 0).unwrap_err();
    assert_eq!(
        failed.identifier.as_deref(),
        Some("MATLAB:assertion:failed")
    );
    let formatted = invoke_with_outputs(
        "assert",
        &[
            Value::Logical(false),
            Value::from("Probe:Assert"),
            Value::from("item %d"),
            Value::Double(3.0),
        ],
        0,
    )
    .unwrap_err();
    assert_eq!(formatted.identifier.as_deref(), Some("Probe:Assert"));
    assert_eq!(formatted.message, "item 3");
}

#[test]
fn trace_preserves_r2022b_class_sum_and_square_matrix_rules() {
    assert_eq!(
        invoke("trace", &[real_array([2, 2], vec![1.0, 3.0, 2.0, 4.0])]),
        Ok(vec![Value::Double(5.0)])
    );
    assert_eq!(
        invoke("trace", &[single_array([2, 2], vec![1.0, 3.0, 2.0, 4.0])]),
        Ok(vec![single_array([1, 1], vec![5.0])])
    );
    assert_eq!(
        invoke(
            "trace",
            &[complex_single_array(
                [2, 2],
                vec![
                    ArrayComplex32::new(1.0, 1.0),
                    ArrayComplex32::new(3.0, 0.0),
                    ArrayComplex32::new(2.0, 0.0),
                    ArrayComplex32::new(4.0, 2.0),
                ],
            )],
        ),
        Ok(vec![complex_single_array(
            [1, 1],
            vec![ArrayComplex32::new(5.0, 3.0)]
        )])
    );
    assert_eq!(
        invoke(
            "trace",
            &[exact_integer_array([2, 2], vec![1_i16, 3, 2, 4])]
        ),
        Ok(vec![Value::Double(5.0)])
    );
    assert_eq!(
        invoke("trace", &[char_array([2, 2], &[65, 67, 66, 68])]),
        Ok(vec![Value::Double(133.0)])
    );
    assert_error(
        invoke("trace", &[real_array([0, 3], vec![])]),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn cross_and_kron_cover_orientation_precision_complex_and_integer_classes() {
    assert_eq!(
        invoke(
            "cross",
            &[
                real_array([1, 3], vec![1.0, 0.0, 0.0]),
                real_array([1, 3], vec![0.0, 1.0, 0.0]),
            ],
        ),
        Ok(vec![real_array([1, 3], vec![0.0, 0.0, 1.0])])
    );
    assert_eq!(
        invoke(
            "cross",
            &[
                single_array([3, 1], vec![1.0, 0.0, 0.0]),
                real_array([3, 1], vec![0.0, 1.0, 0.0]),
                Value::Double(1.0),
            ],
        ),
        Ok(vec![single_array([3, 1], vec![0.0, 0.0, 1.0])])
    );
    assert_eq!(
        invoke(
            "cross",
            &[
                exact_integer_array([1, 3], vec![255_u8, 0, 0]),
                exact_integer_array([1, 3], vec![0_u8, 2, 0]),
            ],
        ),
        Ok(vec![exact_integer_array([1, 3], vec![0_u8, 0, 255])])
    );
    assert_eq!(
        invoke(
            "kron",
            &[
                real_array([1, 2], vec![1.0, 2.0]),
                real_array([2, 1], vec![3.0, 4.0]),
            ],
        ),
        Ok(vec![real_array([2, 2], vec![3.0, 4.0, 6.0, 8.0])])
    );
    assert_eq!(
        invoke(
            "kron",
            &[
                exact_integer_array([1, 2], vec![1_i16, 2]),
                exact_integer_array([2, 1], vec![3_i16, 4]),
            ],
        ),
        Ok(vec![exact_integer_array([2, 2], vec![3_i16, 4, 6, 8])])
    );
    assert_error(
        invoke(
            "kron",
            &[
                exact_integer_array([1, 1], vec![1_i16]),
                real_array([1, 1], vec![2.0]),
            ],
        ),
        BuiltinErrorCategory::Type,
    );
}

#[test]
fn predicates_cover_complex_special_values_types_shapes_and_exact_equality() {
    let complex = complex_array(
        [1, 4],
        vec![
            ArrayComplex64::new(f64::NAN, 0.0),
            ArrayComplex64::new(f64::INFINITY, f64::NAN),
            ArrayComplex64::new(1.0, f64::INFINITY),
            ArrayComplex64::new(2.0, 0.0),
        ],
    );
    assert_eq!(
        invoke("isnan", std::slice::from_ref(&complex)),
        Ok(vec![logical_array([1, 4], vec![true, true, false, false])])
    );
    assert_eq!(
        invoke("isinf", std::slice::from_ref(&complex)),
        Ok(vec![logical_array([1, 4], vec![false, true, true, false])])
    );
    assert_eq!(
        invoke("isfinite", std::slice::from_ref(&complex)),
        Ok(vec![logical_array([1, 4], vec![false, false, false, true])])
    );
    assert_eq!(
        invoke("isreal", &[complex]),
        Ok(vec![Value::Logical(false)])
    );

    assert_eq!(
        invoke("isnumeric", &[exact_integer_array([1, 1], vec![1_u8])]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("isfloat", &[single_array([1, 1], vec![1.0])]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("isinteger", &[exact_integer_array([1, 1], vec![1_i16])]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("islogical", &[Value::Logical(true)]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("isscalar", &[Value::from("text")]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("isvector", &[real_array([0, 1], Vec::new())]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("ismatrix", &[real_array([2, 2, 2], vec![0.0; 8])]),
        Ok(vec![Value::Logical(false)])
    );

    assert_eq!(
        invoke(
            "isequal",
            &[Value::Double(1.0), exact_integer_array([1, 1], vec![1_u8])]
        ),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke(
            "isequal",
            &[
                Value::Double(9_007_199_254_740_992.0),
                exact_integer_array([1, 1], vec![9_007_199_254_740_993_u64])
            ]
        ),
        Ok(vec![Value::Logical(false)])
    );
    assert_eq!(
        invoke(
            "isequal",
            &[Value::Double(f64::NAN), Value::Double(f64::NAN)]
        ),
        Ok(vec![Value::Logical(false)])
    );
    assert_eq!(
        invoke(
            "isequaln",
            &[Value::Double(f64::NAN), Value::Double(f64::NAN)]
        ),
        Ok(vec![Value::Logical(true)])
    );
}

#[test]
fn binary_math_functions_apply_r2022b_nd_implicit_expansion() {
    let left = real_array([2, 1], vec![-5.0, 5.0]);
    let right = real_array([1, 3], vec![3.0, -3.0, f64::INFINITY]);
    assert_eq!(
        invoke("mod", &[left.clone(), right.clone()]),
        Ok(vec![real_array(
            [2, 3],
            vec![1.0, 2.0, -2.0, -1.0, f64::INFINITY, 5.0]
        )])
    );
    assert_eq!(
        invoke("rem", &[left.clone(), right.clone()]),
        Ok(vec![real_array(
            [2, 3],
            vec![-2.0, 2.0, -2.0, 2.0, -5.0, 5.0]
        )])
    );

    let hypot = only_output(invoke(
        "hypot",
        &[
            single_array([2, 1], vec![3.0, 4.0]),
            real_array([1, 2], vec![4.0, 12.0]),
        ],
    ));
    assert_eq!(
        hypot,
        single_array(
            [2, 2],
            vec![
                5.0,
                4.0_f32.hypot(4.0),
                3.0_f32.hypot(12.0),
                4.0_f32.hypot(12.0)
            ]
        )
    );
    assert_eq!(
        invoke(
            "atan2",
            &[
                real_array([0, 1], Vec::new()),
                real_array([1, 3], vec![1.0, 2.0, 3.0]),
            ]
        ),
        Ok(vec![real_array([0, 3], Vec::new())])
    );
}

#[test]
fn flip_circshift_and_ndgrid_preserve_column_major_shapes_and_classes() {
    let matrix = real_array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(
        invoke("flip", std::slice::from_ref(&matrix)),
        Ok(vec![real_array([2, 3], vec![2.0, 1.0, 4.0, 3.0, 6.0, 5.0])])
    );
    assert_eq!(
        invoke("circshift", &[matrix, real_array([1, 2], vec![1.0, -1.0]),]),
        Ok(vec![real_array([2, 3], vec![4.0, 3.0, 6.0, 5.0, 2.0, 1.0])])
    );

    let outputs = invoke_with_outputs(
        "ndgrid",
        &[
            single_array([1, 2], vec![1.0, 2.0]),
            exact_integer_array([1, 3], vec![3_u8, 4, 5]),
        ],
        2,
    )
    .expect("ndgrid must return one class-preserving grid per input");
    assert_eq!(
        outputs,
        vec![
            single_array([2, 3], vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0]),
            exact_integer_array([2, 3], vec![3_u8, 3, 4, 4, 5, 5]),
        ]
    );
}

#[test]
fn diff_and_cumulative_functions_follow_dimensions_classes_and_saturation() {
    let matrix = real_array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(
        invoke("diff", std::slice::from_ref(&matrix)),
        Ok(vec![real_array([1, 3], vec![1.0, 1.0, 1.0])])
    );
    assert_eq!(
        invoke("diff", &[matrix.clone(), Value::Double(2.0)]),
        Ok(vec![real_array([1, 2], vec![0.0, 0.0])])
    );
    assert_eq!(
        invoke("diff", &[matrix, Value::Double(2.0), Value::Double(1.0)]),
        Ok(vec![real_array([0, 3], Vec::new())])
    );

    assert_eq!(
        invoke("diff", &[exact_integer_array([1, 3], vec![1_u8, 3, 2])]),
        Ok(vec![exact_integer_array([1, 2], vec![2_u8, 0])])
    );
    assert_eq!(
        invoke("cumsum", &[exact_integer_array([1, 2], vec![250_u8, 10])]),
        Ok(vec![exact_integer_array([1, 2], vec![250_u8, 255])])
    );
    assert_eq!(
        invoke("cumprod", &[exact_integer_array([1, 2], vec![20_u8, 20])]),
        Ok(vec![exact_integer_array([1, 2], vec![20_u8, 255])])
    );
    assert_eq!(
        invoke(
            "cumsum",
            &[
                real_array([1, 3], vec![1.0, f64::NAN, 2.0]),
                Value::from("omitnan")
            ]
        ),
        Ok(vec![real_array([1, 3], vec![1.0, 1.0, 3.0])])
    );
}

#[test]
fn unique_and_ismember_preserve_classes_indices_nan_and_cross_numeric_identity() {
    let input = real_array([1, 7], vec![3.0, f64::NAN, 1.0, 3.0, f64::NAN, -0.0, 0.0]);
    let unique = invoke_with_outputs("unique", &[input], 3)
        .expect("unique must return values and both index maps");
    let expected = real_array([1, 5], vec![-0.0, 1.0, 3.0, f64::NAN, f64::NAN]);
    let Value::Array(ArrayData::F64(actual)) = &unique[0] else {
        panic!("unique values must retain double storage");
    };
    let Value::Array(ArrayData::F64(expected)) = expected else {
        unreachable!();
    };
    assert_eq!(actual.shape(), expected.shape());
    assert_eq!(actual.as_slice()[..3], expected.as_slice()[..3]);
    assert!(actual.as_slice()[3].is_nan() && actual.as_slice()[4].is_nan());
    assert_eq!(unique[1], real_array([5, 1], vec![6.0, 3.0, 1.0, 2.0, 5.0]));
    assert_eq!(
        unique[2],
        real_array([7, 1], vec![3.0, 4.0, 2.0, 3.0, 5.0, 1.0, 1.0])
    );

    assert_eq!(
        invoke_with_outputs(
            "unique",
            &[
                exact_integer_array([1, 4], vec![3_u8, 1, 3, 2]),
                Value::from("stable")
            ],
            2,
        ),
        Ok(vec![
            exact_integer_array([1, 3], vec![3_u8, 1, 2]),
            real_array([3, 1], vec![1.0, 2.0, 4.0]),
        ])
    );

    let member = invoke_with_outputs(
        "ismember",
        &[
            real_array([1, 4], vec![3.0, f64::NAN, 1.0, 0.0]),
            single_array([1, 4], vec![1.0, 3.0, f32::NAN, -0.0]),
        ],
        2,
    );
    assert_eq!(
        member,
        Ok(vec![
            logical_array([1, 4], vec![true, false, true, true]),
            real_array([1, 4], vec![2.0, 0.0, 1.0, 4.0]),
        ])
    );
}

fn real_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn complex_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<ArrayComplex64>) -> Value {
    Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn single_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f32>) -> Value {
    Value::Array(ArrayData::F32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn complex_single_array(
    dimensions: impl IntoIterator<Item = u64>,
    values: Vec<ArrayComplex32>,
) -> Value {
    Value::Array(ArrayData::ComplexF32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn logical_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<bool>) -> Value {
    Value::Array(ArrayData::Logical(
        DenseArray::from_vec(
            Shape::new(dimensions).unwrap(),
            values.into_iter().map(Logical::from).collect(),
        )
        .unwrap(),
    ))
}

fn string_array(dimensions: impl IntoIterator<Item = u64>, values: &[&str]) -> Value {
    Value::from(
        StringArray::from_vec(
            Shape::new(dimensions).unwrap(),
            values.iter().map(|value| Arc::from(*value)).collect(),
        )
        .unwrap(),
    )
}

fn exact_integer_array<T: IntegerElement>(
    dimensions: impl IntoIterator<Item = u64>,
    values: Vec<T>,
) -> Value {
    Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    )))
}

fn char_array(dimensions: impl IntoIterator<Item = u64>, values: &[u16]) -> Value {
    Value::Array(ArrayData::Char(
        DenseArray::from_vec(
            Shape::new(dimensions).unwrap(),
            values.iter().copied().map(CharCodeUnit::new).collect(),
        )
        .unwrap(),
    ))
}

fn char_row(value: &str) -> Value {
    let values = value.encode_utf16().collect::<Vec<_>>();
    char_array(
        [1, u64::try_from(values.len()).expect("test text fits u64")],
        &values,
    )
}

fn exact_string_array(
    dimensions: impl IntoIterator<Item = u64>,
    values: Vec<StringElement>,
) -> Value {
    Value::String(StringValue::Array(
        StringArray::from_elements(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn only_output(result: BuiltinResult) -> Value {
    let mut values = result.expect("built-in invocation must succeed");
    assert_eq!(values.len(), 1);
    values.remove(0)
}

#[allow(clippy::float_cmp)]
fn assert_real_array(value: &Value, dimensions: &[u64], expected: &[f64]) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected real dense array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice().len(), expected.len());
    for (actual, expected) in array.as_slice().iter().zip(expected) {
        if expected.is_nan() {
            assert!(actual.is_nan());
        } else {
            assert_eq!(actual, expected);
        }
    }
}

fn assert_real_array_close(value: &Value, dimensions: &[u64], expected: &[f64], tolerance: f64) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected real dense array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice().len(), expected.len());
    for (actual, expected) in array.as_slice().iter().zip(expected) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected:.17e}, received {actual:.17e}"
        );
    }
}

fn assert_complex_array(value: &Value, dimensions: &[u64], expected: &[ArrayComplex64]) {
    let Value::Array(ArrayData::ComplexF64(array)) = value else {
        panic!("expected complex dense array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice(), expected);
}

fn assert_single_array(value: &Value, dimensions: &[u64], expected: &[f32]) {
    let Value::Array(ArrayData::F32(array)) = value else {
        panic!("expected real single dense array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice().len(), expected.len());
    for (actual, expected) in array.as_slice().iter().zip(expected) {
        if expected.is_nan() {
            assert!(actual.is_nan());
        } else {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }
}

fn assert_complex_single_array(value: &Value, dimensions: &[u64], expected: &[ArrayComplex32]) {
    let Value::Array(ArrayData::ComplexF32(array)) = value else {
        panic!("expected complex single dense array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice().len(), expected.len());
    for (actual, expected) in array.as_slice().iter().zip(expected) {
        if expected.re.is_nan() {
            assert!(actual.re.is_nan());
        } else {
            assert_eq!(actual.re.to_bits(), expected.re.to_bits());
        }
        if expected.im.is_nan() {
            assert!(actual.im.is_nan());
        } else {
            assert_eq!(actual.im.to_bits(), expected.im.to_bits());
        }
    }
}

fn assert_logical_array(value: &Value, dimensions: &[u64], expected: &[bool]) {
    let Value::Array(ArrayData::Logical(array)) = value else {
        panic!("expected logical dense array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(
        array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
        expected
    );
}

fn assert_integer_array(
    value: &Value,
    class: &str,
    dimensions: &[u64],
    complex: bool,
    expected: &[(&str, &str)],
) {
    let Value::Array(ArrayData::Integer(integer)) = value else {
        panic!("expected exact integer array, received {value:?}");
    };
    assert_eq!(integer.class_name(), class);
    assert_eq!(integer.shape().dimensions(), dimensions);
    assert_eq!(integer.is_complex(), complex);
    assert_eq!(integer.numel(), u64::try_from(expected.len()).unwrap());
    let actual = integer
        .elements()
        .map(|value| {
            let value = value.canonical_decimal();
            (
                value.real_component().to_owned(),
                value.imaginary_component().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let expected = expected
        .iter()
        .map(|(real, imaginary)| ((*real).to_owned(), (*imaginary).to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn assert_error(result: BuiltinResult, category: BuiltinErrorCategory) -> BuiltinError {
    let error = result.expect_err("invocation must fail with a structured error");
    assert_eq!(error.category, category);
    assert!(!error.message.is_empty());
    error
}

#[test]
fn typed_constructors_constants_and_common_type_predicates_match_r2022b() {
    assert_eq!(
        invoke(
            "zeros",
            &[Value::Double(2.0), Value::Double(3.0), char_row("single")],
        ),
        Ok(vec![single_array([2, 3], vec![0.0; 6])])
    );
    assert_integer_array(
        &only_output(invoke("ones", &[Value::Double(2.0), char_row("uint16")])),
        "uint16",
        &[2, 2],
        false,
        &[("1", "0"); 4],
    );
    let prototype = complex_single_array([1, 1], vec![ArrayComplex32::new(4.0, 2.0)]);
    assert_eq!(
        invoke(
            "zeros",
            &[
                Value::Double(1.0),
                Value::Double(2.0),
                char_row("like"),
                prototype,
            ],
        ),
        Ok(vec![complex_single_array(
            [1, 2],
            vec![ArrayComplex32::new(0.0, 0.0); 2],
        )])
    );
    assert_integer_array(
        &only_output(invoke(
            "eye",
            &[Value::Double(2.0), Value::Double(3.0), char_row("uint8")],
        )),
        "uint8",
        &[2, 3],
        false,
        &[
            ("1", "0"),
            ("0", "0"),
            ("0", "0"),
            ("1", "0"),
            ("0", "0"),
            ("0", "0"),
        ],
    );
    assert_logical_array(
        &only_output(invoke("true", &[real_array([1, 2], vec![1.0, 2.0])])),
        &[1, 2],
        &[true, true],
    );
    assert_logical_array(
        &only_output(invoke("false", &[Value::Double(2.0), Value::Double(1.0)])),
        &[2, 1],
        &[false, false],
    );
    assert_eq!(
        invoke("i", &[]),
        Ok(vec![Value::Complex(Complex64::new(0.0, 1.0))])
    );
    assert_eq!(invoke("j", &[]), invoke("i", &[]));

    let cell = only_output(invoke("cell", &[Value::Double(1.0)]));
    let structure = only_output(invoke("struct", &[]));
    let table = only_output(invoke("table", &[Value::Double(1.0)]));
    for (name, value, expected) in [
        ("ischar", char_row("x"), true),
        ("isstring", string_array([1, 1], &["x"]), true),
        ("iscell", cell, true),
        ("isstruct", structure, true),
        ("isobject", table, true),
        ("isobject", Value::Double(1.0), false),
    ] {
        assert_eq!(invoke(name, &[value]), Ok(vec![Value::Logical(expected)]));
    }
    assert_eq!(
        invoke("isrow", &[real_array([1, 0], Vec::new())]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("iscolumn", &[real_array([0, 1], Vec::new())]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("isrow", &[real_array([0, 0], Vec::new())]),
        Ok(vec![Value::Logical(false)])
    );

    assert_error(
        invoke("zeros", &[Value::Double(2.0), char_row("char")]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "true",
            &[Value::Double(2.0), char_row("like"), Value::Double(1.0)],
        ),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn metadata_and_constructors_cover_empty_and_nd_shapes() {
    let higher = real_array([2, 1, 4], vec![0.0; 8]);
    assert_eq!(
        invoke("ndims", std::slice::from_ref(&higher)),
        Ok(vec![Value::Double(3.0)])
    );
    assert_eq!(
        invoke("length", std::slice::from_ref(&higher)),
        Ok(vec![Value::Double(4.0)])
    );

    let empty = real_array([0, 3], Vec::new());
    assert_eq!(
        invoke("length", std::slice::from_ref(&empty)),
        Ok(vec![Value::Double(0.0)])
    );
    assert_eq!(
        invoke("isempty", std::slice::from_ref(&empty)),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("isempty", &[Value::Double(0.0)]),
        Ok(vec![Value::Logical(false)])
    );
    assert_eq!(
        invoke("isempty", &[Value::from("")]),
        Ok(vec![Value::Logical(false)])
    );
    assert_eq!(
        invoke("isempty", &[string_array([1, 1], &[""])]),
        Ok(vec![Value::Logical(false)])
    );
    assert_eq!(
        invoke("isempty", &[string_array([0, 2], &[])]),
        Ok(vec![Value::Logical(true)])
    );
    let transposed_empty = real_array([3, 0], Vec::new());
    assert_eq!(
        invoke("length", &[transposed_empty]),
        Ok(vec![Value::Double(0.0)])
    );

    assert_real_array(&only_output(invoke("zeros", &[])), &[1, 1], &[0.0]);
    let dimensions = real_array([1, 3], vec![2.0, 3.0, 1.0]);
    assert_real_array(
        &only_output(invoke("zeros", &[dimensions])),
        &[2, 3],
        &[0.0; 6],
    );
    assert_real_array(
        &only_output(invoke("ones", &[Value::Double(0.0), Value::Double(3.0)])),
        &[0, 3],
        &[],
    );
    assert_real_array(
        &only_output(invoke("eye", &[real_array([1, 2], vec![2.0, 3.0])])),
        &[2, 3],
        &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    );
}

#[test]
fn string_array_metadata_and_isa_remain_distinct_from_char_and_numeric() {
    let strings = string_array([1, 2], &["alpha", "beta"]);
    assert_eq!(
        invoke("class", std::slice::from_ref(&strings)),
        Ok(vec![Value::from("string")])
    );
    assert_eq!(
        invoke("isa", &[strings.clone(), Value::from("string")]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("isa", &[strings.clone(), Value::from("double")]),
        Ok(vec![Value::Logical(false)])
    );
    assert_real_array(
        &only_output(invoke("size", std::slice::from_ref(&strings))),
        &[1, 2],
        &[1.0, 2.0],
    );
    assert_eq!(
        invoke("numel", std::slice::from_ref(&strings)),
        Ok(vec![Value::Double(2.0)])
    );

    let scalar_class_name = string_array([1, 1], &["string"]);
    assert_eq!(
        invoke("isa", &[strings.clone(), scalar_class_name]),
        Ok(vec![Value::Logical(true)])
    );
    assert_error(
        invoke(
            "isa",
            &[strings, string_array([1, 2], &["string", "double"])],
        ),
        BuiltinErrorCategory::Type,
    );
}

#[test]
fn strcmp_covers_scalars_utf8_arrays_broadcast_empty_and_errors() {
    assert_eq!(
        invoke(
            "strcmp",
            &[
                Value::from("OpenMatDerivedNumber"),
                Value::from("OpenMatDerivedNumber"),
            ],
        ),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("strcmp", &[Value::from("Alpha"), Value::from("alpha")]),
        Ok(vec![Value::Logical(false)])
    );
    assert_eq!(
        invoke("strcmp", &[Value::from("中文🙂"), Value::from("中文🙂")]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("strcmp", &[Value::from(""), Value::from("")]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke(
            "strcmp",
            &[string_array([1, 1], &["alpha"]), Value::from("alpha"),],
        ),
        Ok(vec![Value::Logical(true)])
    );

    let left = string_array([2, 2], &["alpha", "βeta", "Gamma", ""]);
    let right = string_array([2, 2], &["alpha", "BETA", "Gamma", ""]);
    let compared = only_output(invoke("strcmp", &[left.clone(), right]));
    assert_logical_array(&compared, &[2, 2], &[true, false, true, true]);
    let compared_clone = compared.clone();
    assert!(compared.shares_array_storage_with(&compared_clone));

    assert_logical_array(
        &only_output(invoke("strcmp", &[left, Value::from("alpha")])),
        &[2, 2],
        &[true, false, false, false],
    );
    assert_logical_array(
        &only_output(invoke(
            "strcmp",
            &[
                Value::from("alpha"),
                string_array([1, 3], &["alpha", "beta", "alpha"]),
            ],
        )),
        &[1, 3],
        &[true, false, true],
    );

    assert_logical_array(
        &only_output(invoke(
            "strcmp",
            &[string_array([0, 2], &[]), Value::from("x")],
        )),
        &[0, 2],
        &[],
    );
    assert_logical_array(
        &only_output(invoke(
            "strcmp",
            &[string_array([0, 2], &[]), string_array([0, 2], &[])],
        )),
        &[0, 2],
        &[],
    );

    assert_error(
        invoke(
            "strcmp",
            &[
                string_array([1, 2], &["a", "b"]),
                string_array([2, 1], &["a", "b"]),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("strcmp", &[Value::Double(1.0), Value::from("1")]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("strcmp", &[Value::from("1"), Value::Logical(true)]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("strcmp", &[Value::from("one")]),
        BuiltinErrorCategory::ArgumentCount,
    );
    assert_error(
        invoke(
            "strcmp",
            &[Value::from("one"), Value::from("one"), Value::from("one")],
        ),
        BuiltinErrorCategory::ArgumentCount,
    );
    assert_error(
        invoke_with_outputs("strcmp", &[Value::from("a"), Value::from("a")], 2),
        BuiltinErrorCategory::ArgumentCount,
    );
}

#[test]
fn strcmp_observes_a_cancelled_token_before_array_traversal() {
    let registry = minimal_registry().expect("fresh registry must register the core built-ins");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let values = vec!["same"; 8_192];
    let input = string_array([1, 8_192], &values);

    assert_error(
        invoke_in_registry_with_cancellation(
            &registry,
            "strcmp",
            &[input, Value::from("same")],
            1,
            &cancellation,
        ),
        BuiltinErrorCategory::Cancelled,
    );
}

#[test]
fn reshape_preserves_column_major_order_classes_and_same_shape_cow() {
    let original = real_array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let reshaped = only_output(invoke(
        "reshape",
        &[original.clone(), Value::Double(3.0), Value::Double(2.0)],
    ));
    assert_real_array(&reshaped, &[3, 2], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);

    let same = only_output(invoke(
        "reshape",
        &[original.clone(), real_array([1, 2], vec![2.0, 3.0])],
    ));
    assert!(original.shares_array_storage_with(&same));

    let inferred = only_output(invoke(
        "reshape",
        &[
            original,
            Value::Double(2.0),
            real_array([0, 0], Vec::new()),
            Value::Double(1.0),
        ],
    ));
    assert_real_array(&inferred, &[2, 3], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);

    let logical = logical_array([1, 2], vec![true, false]);
    let logical_reshaped = only_output(invoke(
        "reshape",
        &[logical, Value::Double(2.0), Value::Double(1.0)],
    ));
    assert_logical_array(&logical_reshaped, &[2, 1], &[true, false]);

    let complex = complex_array(
        [1, 2],
        vec![ArrayComplex64::new(1.0, 2.0), ArrayComplex64::new(3.0, 4.0)],
    );
    let complex_reshaped = only_output(invoke(
        "reshape",
        &[complex, Value::Double(2.0), Value::Double(1.0)],
    ));
    assert_complex_array(
        &complex_reshaped,
        &[2, 1],
        &[ArrayComplex64::new(1.0, 2.0), ArrayComplex64::new(3.0, 4.0)],
    );
}

#[test]
fn reshape_and_concatenation_preserve_sparse_storage() {
    let sparse = Value::Sparse(
        SparseArrayData::try_from_f64_coo(
            2,
            2,
            vec![CooEntry::new(0, 0, 1.0), CooEntry::new(1, 1, 2.0)],
            0,
            None,
        )
        .expect("valid sparse fixture"),
    );
    let reshaped = only_output(invoke(
        "reshape",
        &[sparse.clone(), Value::Double(1.0), Value::Double(4.0)],
    ));
    let Value::Sparse(reshaped) = reshaped else {
        panic!("reshape must preserve sparse storage");
    };
    assert_eq!(reshaped.shape().dimensions(), &[1, 4]);
    assert_eq!(reshaped.nnz(), 2);
    assert_eq!(
        reshaped.stored_entry(1).map(|entry| entry.value),
        Some(SparseScalarValue::F64(2.0))
    );

    let left = Value::Sparse(
        SparseArrayData::try_from_f64_coo(2, 1, vec![CooEntry::new(0, 0, 1.0)], 0, None)
            .expect("valid sparse fixture"),
    );
    let horizontal = only_output(invoke(
        "horzcat",
        &[left.clone(), real_array([2, 1], vec![0.0, 2.0])],
    ));
    let Value::Sparse(horizontal) = horizontal else {
        panic!("mixed horizontal concatenation must preserve sparse storage");
    };
    assert_eq!(horizontal.shape().dimensions(), &[2, 2]);
    assert_eq!(horizontal.nnz(), 2);

    let vertical = only_output(invoke("vertcat", &[left, real_array([1, 1], vec![3.0])]));
    let Value::Sparse(vertical) = vertical else {
        panic!("mixed vertical concatenation must preserve sparse storage");
    };
    assert_eq!(vertical.shape().dimensions(), &[3, 1]);
    assert_eq!(vertical.nnz(), 2);

    assert_error(
        invoke("cat", &[Value::Double(3.0), sparse.clone(), sparse]),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn squeeze_permute_and_repmat_cover_nd_order_classes_empty_and_checked_parameters() {
    assert_real_array(
        &only_output(invoke(
            "squeeze",
            &[real_array([1, 1, 3], vec![1.0, 2.0, 3.0])],
        )),
        &[3, 1],
        &[1.0, 2.0, 3.0],
    );
    assert_real_array(
        &only_output(invoke(
            "squeeze",
            &[real_array([1, 3, 1], vec![1.0, 2.0, 3.0])],
        )),
        &[1, 3],
        &[1.0, 2.0, 3.0],
    );
    assert_single_array(
        &only_output(invoke(
            "squeeze",
            &[single_array([1, 1, 3], vec![1.0, 2.0, 3.0])],
        )),
        &[3, 1],
        &[1.0, 2.0, 3.0],
    );
    assert_integer_array(
        &only_output(invoke(
            "squeeze",
            &[exact_integer_array([1, 1, 2], vec![u64::MAX, 2])],
        )),
        "uint64",
        &[2, 1],
        false,
        &[("18446744073709551615", "0"), ("2", "0")],
    );
    assert_real_array(
        &only_output(invoke("squeeze", &[real_array([1, 1, 0], Vec::new())])),
        &[0, 1],
        &[],
    );

    assert_real_array(
        &only_output(invoke(
            "permute",
            &[
                real_array([2, 1, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
                real_array([1, 3], vec![3.0, 1.0, 2.0]),
            ],
        )),
        &[3, 2],
        &[1.0, 3.0, 5.0, 2.0, 4.0, 6.0],
    );
    assert_logical_array(
        &only_output(invoke(
            "permute",
            &[
                logical_array([2, 1, 2], vec![false, true, true, false]),
                exact_integer_array([1, 3], vec![3_i8, 1, 2]),
            ],
        )),
        &[2, 2],
        &[false, true, true, false],
    );
    assert_real_array(
        &only_output(invoke(
            "permute",
            &[
                real_array([0, 2, 3], Vec::new()),
                single_array([1, 3], vec![3.0, 1.0, 2.0]),
            ],
        )),
        &[3, 0, 2],
        &[],
    );

    assert_real_array(
        &only_output(invoke(
            "repmat",
            &[
                real_array([2, 2], vec![1.0, 2.0, 3.0, 4.0]),
                real_array([1, 2], vec![2.0, 3.0]),
            ],
        )),
        &[4, 6],
        &[
            1.0, 2.0, 1.0, 2.0, 3.0, 4.0, 3.0, 4.0, 1.0, 2.0, 1.0, 2.0, 3.0, 4.0, 3.0, 4.0, 1.0,
            2.0, 1.0, 2.0, 3.0, 4.0, 3.0, 4.0,
        ],
    );
    assert_real_array(
        &only_output(invoke(
            "fix",
            &[complex_array(
                [1, 2],
                vec![
                    ArrayComplex64::new(1.8, 0.2),
                    ArrayComplex64::new(-1.8, -0.2),
                ],
            )],
        )),
        &[1, 2],
        &[1.0, -1.0],
    );
    assert_integer_array(
        &only_output(invoke(
            "repmat",
            &[
                exact_integer_array([1, 2], vec![i8::MIN, i8::MAX]),
                Value::Double(2.0),
            ],
        )),
        "int8",
        &[2, 4],
        false,
        &[
            ("-128", "0"),
            ("-128", "0"),
            ("127", "0"),
            ("127", "0"),
            ("-128", "0"),
            ("-128", "0"),
            ("127", "0"),
            ("127", "0"),
        ],
    );
    assert_real_array(
        &only_output(invoke(
            "repmat",
            &[
                real_array([2, 2], vec![1.0, 2.0, 3.0, 4.0]),
                Value::Double(-1.0),
            ],
        )),
        &[0, 0],
        &[],
    );
    assert_error(
        invoke(
            "permute",
            &[
                real_array([2, 1, 3], vec![1.0; 6]),
                real_array([1, 2], vec![3.0, 1.0]),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "repmat",
            &[
                real_array([2, 2], vec![1.0; 4]),
                real_array([0, 0], Vec::new()),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn conversions_cover_scalar_expansion_complex_and_cow_identity_paths() {
    let real = real_array([1, 2], vec![0.0, f64::NAN]);
    let doubled = only_output(invoke("double", std::slice::from_ref(&real)));
    assert!(real.shares_array_storage_with(&doubled));
    assert_logical_array(
        &only_output(invoke("logical", std::slice::from_ref(&real))),
        &[1, 2],
        &[false, true],
    );

    let complex = complex_array(
        [1, 2],
        vec![
            ArrayComplex64::new(1.0, 2.0),
            ArrayComplex64::new(-3.0, -4.0),
        ],
    );
    assert_real_array(
        &only_output(invoke("real", std::slice::from_ref(&complex))),
        &[1, 2],
        &[1.0, -3.0],
    );
    assert_real_array(
        &only_output(invoke("imag", std::slice::from_ref(&complex))),
        &[1, 2],
        &[2.0, -4.0],
    );
    assert_complex_array(
        &only_output(invoke("conj", &[complex])),
        &[1, 2],
        &[
            ArrayComplex64::new(1.0, -2.0),
            ArrayComplex64::new(-3.0, 4.0),
        ],
    );

    assert_complex_array(
        &only_output(invoke(
            "complex",
            &[real_array([1, 2], vec![1.0, 2.0]), Value::Double(3.0)],
        )),
        &[1, 2],
        &[ArrayComplex64::new(1.0, 3.0), ArrayComplex64::new(2.0, 3.0)],
    );
    assert_eq!(
        invoke("real", &[Value::Logical(true)]),
        Ok(vec![Value::Double(1.0)])
    );
    assert_eq!(
        invoke("conj", &[Value::Logical(true)]),
        Ok(vec![Value::Double(1.0)])
    );
}

#[test]
fn single_conversion_preserves_precision_shape_specials_and_cow_identity() {
    let source = real_array(
        [2, 3],
        vec![-0.0, 1.5, -2.25, f64::MAX, f64::NEG_INFINITY, f64::NAN],
    );
    let converted = only_output(invoke("single", &[source]));
    assert_single_array(
        &converted,
        &[2, 3],
        &[-0.0, 1.5, -2.25, f32::INFINITY, f32::NEG_INFINITY, f32::NAN],
    );
    assert_eq!(converted.class_name(), "single");
    assert_eq!(converted.dtype(), Some(DType::F32));
    assert!(!converted.is_complex_numeric());

    assert_single_array(
        &only_output(invoke("single", &[Value::Double(-3.25)])),
        &[1, 1],
        &[-3.25],
    );
    assert_single_array(
        &only_output(invoke(
            "single",
            &[logical_array([1, 2], vec![true, false])],
        )),
        &[1, 2],
        &[1.0, 0.0],
    );
    assert_single_array(
        &only_output(invoke("single", &[char_array([1, 2], &[0, u16::MAX])])),
        &[1, 2],
        &[0.0, 65_535.0],
    );

    let identity = only_output(invoke("single", std::slice::from_ref(&converted)));
    assert!(converted.shares_array_storage_with(&identity));

    let empty = only_output(invoke("single", &[real_array([0, 3], Vec::new())]));
    assert_single_array(&empty, &[0, 3], &[]);

    let complex = only_output(invoke(
        "single",
        &[complex_array(
            [1, 2],
            vec![
                ArrayComplex64::new(-0.0, 0.0),
                ArrayComplex64::new(f64::NAN, f64::INFINITY),
            ],
        )],
    ));
    assert_complex_single_array(
        &complex,
        &[1, 2],
        &[
            ArrayComplex32::new(-0.0, 0.0),
            ArrayComplex32::new(f32::NAN, f32::INFINITY),
        ],
    );
    assert_eq!(complex.dtype(), Some(DType::ComplexF32));
    assert!(complex.is_complex_numeric());

    let exact_integer = only_output(invoke(
        "single",
        &[exact_integer_array([1, 1], vec![u64::MAX])],
    ));
    assert_single_array(&exact_integer, &[1, 1], &[f32::from_bits(0x5f80_0000)]);
    assert_integer_array(
        &only_output(invoke("int8", &[single_array([1, 2], vec![-1.5, 200.0])])),
        "int8",
        &[1, 2],
        false,
        &[("-2", "0"), ("127", "0")],
    );

    let widened = only_output(invoke(
        "double",
        &[single_array([1, 2], vec![f32::MIN_POSITIVE, -0.0])],
    ));
    assert_real_array(
        &widened,
        &[1, 2],
        &[f64::from(f32::MIN_POSITIVE), f64::from(-0.0_f32)],
    );
}

#[test]
fn complex_and_shape_builtins_keep_single_storage() {
    let real = single_array([1, 2], vec![1.0, -2.0]);
    let imaginary = single_array([1, 1], vec![-0.0]);
    let complex = only_output(invoke("complex", &[real, imaginary]));
    assert_complex_single_array(
        &complex,
        &[1, 2],
        &[
            ArrayComplex32::new(1.0, -0.0),
            ArrayComplex32::new(-2.0, -0.0),
        ],
    );
    assert_eq!(complex.class_name(), "single");
    assert_eq!(
        invoke("isa", &[complex.clone(), Value::from("single")]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("numel", std::slice::from_ref(&complex)),
        Ok(vec![Value::Double(2.0)])
    );

    let one_input = only_output(invoke("complex", std::slice::from_ref(&complex)));
    assert!(complex.shares_array_storage_with(&one_input));

    assert_single_array(
        &only_output(invoke("real", std::slice::from_ref(&complex))),
        &[1, 2],
        &[1.0, -2.0],
    );
    assert_single_array(
        &only_output(invoke("imag", std::slice::from_ref(&complex))),
        &[1, 2],
        &[-0.0, -0.0],
    );
    assert_complex_single_array(
        &only_output(invoke("conj", std::slice::from_ref(&complex))),
        &[1, 2],
        &[
            ArrayComplex32::new(1.0, 0.0),
            ArrayComplex32::new(-2.0, 0.0),
        ],
    );

    let reshaped = only_output(invoke(
        "reshape",
        &[complex, Value::Double(2.0), Value::Double(1.0)],
    ));
    assert_complex_single_array(
        &reshaped,
        &[2, 1],
        &[
            ArrayComplex32::new(1.0, -0.0),
            ArrayComplex32::new(-2.0, -0.0),
        ],
    );

    assert_error(
        invoke(
            "complex",
            &[single_array([1, 1], vec![1.0]), Value::Double(2.0)],
        ),
        BuiltinErrorCategory::Type,
    );
}

#[test]
fn sum_and_product_reduce_default_explicit_empty_and_complex_dimensions() {
    let matrix = real_array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_real_array(
        &only_output(invoke("sum", std::slice::from_ref(&matrix))),
        &[1, 3],
        &[3.0, 7.0, 11.0],
    );
    assert_real_array(
        &only_output(invoke("prod", std::slice::from_ref(&matrix))),
        &[1, 3],
        &[2.0, 12.0, 30.0],
    );
    assert_real_array(
        &only_output(invoke("sum", &[matrix, Value::Double(2.0)])),
        &[2, 1],
        &[9.0, 12.0],
    );

    let empty = real_array([0, 3], Vec::new());
    assert_real_array(
        &only_output(invoke("sum", std::slice::from_ref(&empty))),
        &[1, 3],
        &[0.0, 0.0, 0.0],
    );
    assert_real_array(
        &only_output(invoke("prod", &[empty])),
        &[1, 3],
        &[1.0, 1.0, 1.0],
    );

    let higher = complex_array(
        [2, 1, 2],
        vec![
            ArrayComplex64::new(1.0, 1.0),
            ArrayComplex64::new(2.0, -1.0),
            ArrayComplex64::new(3.0, 2.0),
            ArrayComplex64::new(4.0, -2.0),
        ],
    );
    assert_complex_array(
        &only_output(invoke("sum", &[higher])),
        &[1, 1, 2],
        &[ArrayComplex64::new(3.0, 0.0), ArrayComplex64::new(7.0, 0.0)],
    );

    let logical = logical_array([2, 2], vec![true, true, false, true]);
    assert_real_array(
        &only_output(invoke("sum", &[logical])),
        &[1, 2],
        &[2.0, 1.0],
    );
}

#[test]
fn sum_covers_all_vecdim_missing_and_r2022b_output_classes() {
    let with_nan = real_array([2, 2], vec![1.0, 3.0, f64::NAN, 4.0]);
    assert_real_array(
        &only_output(invoke("sum", std::slice::from_ref(&with_nan))),
        &[1, 2],
        &[4.0, f64::NAN],
    );
    assert_real_array(
        &only_output(invoke("sum", &[with_nan.clone(), char_row("omitnan")])),
        &[1, 2],
        &[4.0, 4.0],
    );
    assert_eq!(
        invoke("sum", &[with_nan, char_row("all"), char_row("omitnan")],),
        Ok(vec![Value::Double(8.0)])
    );

    assert_real_array(
        &only_output(invoke(
            "sum",
            &[
                real_array([2, 3, 4], (1..=24).map(f64::from).collect()),
                real_array([1, 2], vec![1.0, 3.0]),
            ],
        )),
        &[1, 3],
        &[84.0, 100.0, 116.0],
    );
    assert_single_array(
        &only_output(invoke(
            "sum",
            &[single_array([2, 2], vec![1.0, 3.0, 2.0, 4.0])],
        )),
        &[1, 2],
        &[4.0, 6.0],
    );
    assert_real_array(
        &only_output(invoke(
            "sum",
            &[
                single_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]),
                char_row("double"),
            ],
        )),
        &[1, 2],
        &[4.0, 6.0],
    );

    assert_real_array(
        &only_output(invoke(
            "sum",
            &[exact_integer_array([2, 2], vec![100_i8, -100, 100, -100])],
        )),
        &[1, 2],
        &[0.0, 0.0],
    );
    assert_eq!(
        invoke(
            "sum",
            &[
                exact_integer_array([1, 2], vec![200_u8, 100]),
                char_row("native"),
            ],
        ),
        Ok(vec![exact_integer_array([1, 1], vec![u8::MAX])])
    );
    assert_logical_array(
        &only_output(invoke(
            "sum",
            &[
                logical_array([2, 2], vec![true, true, false, true]),
                char_row("native"),
            ],
        )),
        &[1, 2],
        &[true, true],
    );
    assert_error(
        invoke("sum", &[char_row("AZ"), char_row("native")]),
        BuiltinErrorCategory::Domain,
    );

    assert_eq!(
        invoke(
            "sum",
            &[
                complex_array(
                    [1, 3],
                    vec![
                        ArrayComplex64::new(1.0, 2.0),
                        ArrayComplex64::new(f64::NAN, 3.0),
                        ArrayComplex64::new(4.0, f64::NAN),
                    ],
                ),
                char_row("omitnan"),
            ],
        ),
        Ok(vec![Value::Complex(Complex64::new(1.0, 2.0))])
    );
}

#[test]
fn product_covers_all_missing_single_and_saturating_native_classes() {
    let with_nan = real_array([2, 2], vec![1.0, 3.0, f64::NAN, 4.0]);
    assert_real_array(
        &only_output(invoke("prod", std::slice::from_ref(&with_nan))),
        &[1, 2],
        &[3.0, f64::NAN],
    );
    assert_real_array(
        &only_output(invoke("prod", &[with_nan, char_row("omitnan")])),
        &[1, 2],
        &[3.0, 4.0],
    );
    assert_eq!(
        invoke(
            "prod",
            &[
                real_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]),
                char_row("all"),
            ],
        ),
        Ok(vec![Value::Double(24.0)])
    );
    assert_single_array(
        &only_output(invoke(
            "prod",
            &[single_array([2, 2], vec![1.0, 3.0, 2.0, 4.0])],
        )),
        &[1, 2],
        &[3.0, 8.0],
    );
    assert_real_array(
        &only_output(invoke(
            "prod",
            &[exact_integer_array([2, 2], vec![10_i8, -10, 20, -20])],
        )),
        &[1, 2],
        &[-100.0, -400.0],
    );
    assert_eq!(
        invoke(
            "prod",
            &[
                exact_integer_array([1, 2], vec![20_u8, 20]),
                char_row("native"),
            ],
        ),
        Ok(vec![exact_integer_array([1, 1], vec![u8::MAX])])
    );
    assert_logical_array(
        &only_output(invoke(
            "prod",
            &[
                logical_array([2, 2], vec![true, true, false, true]),
                char_row("native"),
            ],
        )),
        &[1, 2],
        &[true, false],
    );
    assert_eq!(
        invoke("prod", &[real_array([0, 3], Vec::new()), char_row("all")],),
        Ok(vec![Value::Double(1.0)])
    );
}

#[test]
fn any_and_all_cover_classes_nan_omission_dimensions_vecdim_all_and_empty_identities() {
    let higher = real_array([2, 1, 2], vec![0.0, 1.0, 0.0, 2.0]);
    assert_logical_array(
        &only_output(invoke("any", std::slice::from_ref(&higher))),
        &[1, 1, 2],
        &[true, true],
    );
    assert_logical_array(
        &only_output(invoke("all", &[higher.clone(), Value::Double(2.0)])),
        &[2, 1, 2],
        &[false, true, false, true],
    );
    assert_eq!(
        invoke(
            "any",
            &[higher.clone(), real_array([1, 2], vec![1.0, 3.0]),],
        ),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("all", &[higher, Value::from("all")]),
        Ok(vec![Value::Logical(false)])
    );

    assert_logical_array(
        &only_output(invoke(
            "any",
            &[
                single_array([1, 2], vec![0.0, f32::NAN]),
                Value::Double(4.0),
            ],
        )),
        &[1, 2],
        &[false, false],
    );
    assert_eq!(
        invoke("all", &[real_array([1, 2], vec![f64::NAN, 1.0])]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke(
            "any",
            &[complex_array(
                [1, 2],
                vec![
                    ArrayComplex64::new(f64::NAN, 1.0),
                    ArrayComplex64::new(0.0, f64::NAN),
                ],
            )],
        ),
        Ok(vec![Value::Logical(false)])
    );
    assert_eq!(
        invoke("any", &[exact_integer_array([1, 2], vec![0_i8, 2])]),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("all", &[char_array([1, 2], &[65, 66])]),
        Ok(vec![Value::Logical(true)])
    );

    assert_logical_array(
        &only_output(invoke("any", &[real_array([0, 3], Vec::new())])),
        &[1, 3],
        &[false, false, false],
    );
    assert_logical_array(
        &only_output(invoke("all", &[real_array([0, 3], Vec::new())])),
        &[1, 3],
        &[true, true, true],
    );
    assert_eq!(
        invoke("any", &[real_array([0, 3], Vec::new()), Value::from("all")]),
        Ok(vec![Value::Logical(false)])
    );
    assert_eq!(
        invoke("all", &[real_array([0, 3], Vec::new()), Value::from("all")]),
        Ok(vec![Value::Logical(true)])
    );
    assert_error(
        invoke(
            "any",
            &[
                real_array([2, 2], vec![1.0; 4]),
                real_array([1, 2], vec![1.0, 1.0]),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn find_covers_orientation_nd_subscripts_limits_direction_values_classes_and_empty_shapes() {
    assert_real_array(
        &only_output(invoke(
            "find",
            &[real_array([1, 4], vec![0.0, 2.0, 0.0, 3.0])],
        )),
        &[1, 2],
        &[2.0, 4.0],
    );
    assert_real_array(
        &only_output(invoke(
            "find",
            &[real_array([4, 1], vec![0.0, 2.0, 0.0, 3.0])],
        )),
        &[2, 1],
        &[2.0, 4.0],
    );
    let outputs = invoke_with_outputs(
        "find",
        &[real_array([2, 1, 2], vec![0.0, 2.0, 0.0, 3.0])],
        3,
    )
    .unwrap();
    assert_eq!(outputs.len(), 3);
    assert_real_array(&outputs[0], &[2, 1], &[2.0, 2.0]);
    assert_real_array(&outputs[1], &[2, 1], &[1.0, 2.0]);
    assert_real_array(&outputs[2], &[2, 1], &[2.0, 3.0]);

    assert_real_array(
        &only_output(invoke(
            "find",
            &[
                real_array([1, 4], vec![1.0, 2.0, 0.0, 3.0]),
                Value::Double(2.0),
                Value::from("last"),
            ],
        )),
        &[1, 2],
        &[2.0, 4.0],
    );
    assert_real_array(
        &only_output(invoke(
            "find",
            &[
                real_array([1, 4], vec![1.0, 2.0, 0.0, 3.0]),
                single_array([1, 1], vec![1.0]),
                Value::from("first"),
            ],
        )),
        &[1, 1],
        &[1.0],
    );

    let logical =
        invoke_with_outputs("find", &[logical_array([1, 2], vec![false, true])], 3).unwrap();
    assert_logical_array(&logical[2], &[1, 1], &[true]);
    let integer = invoke_with_outputs(
        "find",
        &[exact_integer_array([1, 2], vec![0_i8, i8::MIN])],
        3,
    )
    .unwrap();
    assert_integer_array(&integer[2], "int8", &[1, 1], false, &[("-128", "0")]);
    let single = invoke_with_outputs("find", &[single_array([1, 2], vec![0.0, 2.0])], 3).unwrap();
    assert_single_array(&single[2], &[1, 1], &[2.0]);
    assert_real_array(
        &only_output(invoke("find", &[Value::Double(f64::NAN)])),
        &[1, 1],
        &[1.0],
    );

    assert_real_array(
        &only_output(invoke("find", &[real_array([0, 3], Vec::new())])),
        &[0, 1],
        &[],
    );
    assert_real_array(
        &only_output(invoke("find", &[real_array([1, 0], Vec::new())])),
        &[1, 0],
        &[],
    );
    assert_error(
        invoke(
            "find",
            &[real_array([1, 2], vec![1.0, 0.0]), Value::Double(0.0)],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "find",
            &[
                real_array([1, 2], vec![1.0, 0.0]),
                Value::Double(1.0),
                Value::from("middle"),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn extrema_cover_nan_complex_logical_empty_and_dimension_forms() {
    let with_nan = real_array([1, 3], vec![f64::NAN, 2.0, -1.0]);
    assert_real_array(
        &only_output(invoke("min", std::slice::from_ref(&with_nan))),
        &[1, 1],
        &[-1.0],
    );
    assert_real_array(&only_output(invoke("max", &[with_nan])), &[1, 1], &[2.0]);

    let tied = complex_array(
        [1, 3],
        vec![
            ArrayComplex64::new(3.0, 4.0),
            ArrayComplex64::new(4.0, 3.0),
            ArrayComplex64::new(-3.0, -4.0),
        ],
    );
    assert_complex_array(
        &only_output(invoke("min", std::slice::from_ref(&tied))),
        &[1, 1],
        &[ArrayComplex64::new(-3.0, -4.0)],
    );
    assert_complex_array(
        &only_output(invoke("max", &[tied])),
        &[1, 1],
        &[ArrayComplex64::new(3.0, 4.0)],
    );

    let logical = logical_array([1, 2], vec![false, true]);
    assert_logical_array(
        &only_output(invoke("min", std::slice::from_ref(&logical))),
        &[1, 1],
        &[false],
    );
    assert_logical_array(&only_output(invoke("max", &[logical])), &[1, 1], &[true]);

    let empty_rows = real_array([0, 3], Vec::new());
    assert_real_array(&only_output(invoke("min", &[empty_rows])), &[0, 3], &[]);
    let empty_columns = real_array([3, 0], Vec::new());
    assert_real_array(&only_output(invoke("max", &[empty_columns])), &[1, 0], &[]);

    let matrix = real_array([2, 2], vec![1.0, 4.0, 3.0, 2.0]);
    assert_real_array(
        &only_output(invoke(
            "min",
            &[matrix, real_array([0, 0], Vec::new()), Value::Double(2.0)],
        )),
        &[2, 1],
        &[1.0, 2.0],
    );
}

#[test]
fn extrema_reductions_cover_indices_all_vecdim_missing_classes_and_methods() {
    let matrix = real_array([2, 3], vec![3.0, f64::NAN, 1.0, 2.0, 1.0, 0.0]);
    let outputs = invoke_with_outputs("min", std::slice::from_ref(&matrix), 2)
        .expect("two-output min must succeed");
    assert_real_array(&outputs[0], &[1, 3], &[3.0, 1.0, 0.0]);
    assert_real_array(&outputs[1], &[1, 3], &[1.0, 1.0, 2.0]);
    let outputs = invoke_with_outputs("max", &[matrix], 2).expect("two-output max must succeed");
    assert_real_array(&outputs[0], &[1, 3], &[3.0, 2.0, 1.0]);
    assert_real_array(&outputs[1], &[1, 3], &[1.0, 2.0, 1.0]);

    let outputs = invoke_with_outputs(
        "min",
        &[
            real_array([1, 3], vec![3.0, f64::NAN, 1.0]),
            real_array([0, 0], vec![]),
            char_row("includenan"),
        ],
        2,
    )
    .expect("includenan min must succeed");
    assert_real_array(&outputs[0], &[1, 1], &[f64::NAN]);
    assert_real_array(&outputs[1], &[1, 1], &[2.0]);

    let outputs = invoke_with_outputs(
        "min",
        &[
            real_array([2, 2], vec![3.0, 2.0, 1.0, 0.0]),
            real_array([0, 0], vec![]),
            char_row("all"),
            char_row("linear"),
        ],
        2,
    )
    .expect("all-linear min must succeed");
    assert_real_array(&outputs[0], &[1, 1], &[0.0]);
    assert_real_array(&outputs[1], &[1, 1], &[4.0]);

    let outputs = invoke_with_outputs(
        "min",
        &[
            real_array([2, 3, 4], (1..=24).map(f64::from).collect()),
            real_array([0, 0], vec![]),
            real_array([1, 2], vec![1.0, 3.0]),
            char_row("linear"),
        ],
        2,
    )
    .expect("vecdim-linear min must succeed");
    assert_real_array(&outputs[0], &[1, 3], &[1.0, 3.0, 5.0]);
    assert_real_array(&outputs[1], &[1, 3], &[1.0, 3.0, 5.0]);

    let outputs = invoke_with_outputs("min", &[single_array([2, 2], vec![3.0, 2.0, 1.0, 0.0])], 2)
        .expect("single min with index must succeed");
    assert_single_array(&outputs[0], &[1, 2], &[2.0, 0.0]);
    assert_real_array(&outputs[1], &[1, 2], &[2.0, 2.0]);
    assert_eq!(
        invoke("min", &[exact_integer_array([1, 3], vec![3_i16, 1, 2])]),
        Ok(vec![exact_integer_array([1, 1], vec![1_i16])])
    );
    let outputs = invoke_with_outputs("min", &[char_row("AZ")], 2)
        .expect("char reduction must return double value and index");
    assert_real_array(&outputs[0], &[1, 1], &[65.0]);
    assert_real_array(&outputs[1], &[1, 1], &[1.0]);

    let outputs = invoke_with_outputs(
        "min",
        &[
            real_array([1, 2], vec![-5.0, 3.0]),
            real_array([0, 0], vec![]),
            char_row("ComparisonMethod"),
            char_row("abs"),
        ],
        2,
    )
    .expect("absolute comparison min must succeed");
    assert_real_array(&outputs[0], &[1, 1], &[3.0]);
    assert_real_array(&outputs[1], &[1, 1], &[2.0]);
}

#[test]
fn extrema_elementwise_cover_expansion_nan_complex_and_single() {
    assert_real_array(
        &only_output(invoke(
            "min",
            &[
                real_array([2, 2], vec![1.0, 5.0, 4.0, 2.0]),
                real_array([2, 2], vec![2.0, 4.0, 3.0, 6.0]),
            ],
        )),
        &[2, 2],
        &[1.0, 4.0, 3.0, 2.0],
    );
    assert_real_array(
        &only_output(invoke(
            "min",
            &[
                real_array([2, 1], vec![1.0, 4.0]),
                real_array([1, 2], vec![2.0, 3.0]),
            ],
        )),
        &[2, 2],
        &[1.0, 2.0, 1.0, 3.0],
    );
    assert_single_array(
        &only_output(invoke(
            "max",
            &[
                single_array([2, 2], vec![1.0, 5.0, 4.0, 2.0]),
                Value::Double(3.0),
            ],
        )),
        &[2, 2],
        &[3.0, 5.0, 4.0, 3.0],
    );
    assert_real_array(
        &only_output(invoke(
            "min",
            &[
                real_array([1, 2], vec![f64::NAN, 1.0]),
                real_array([1, 2], vec![2.0, f64::NAN]),
                char_row("includenan"),
            ],
        )),
        &[1, 2],
        &[f64::NAN, f64::NAN],
    );

    assert_complex_array(
        &only_output(invoke(
            "min",
            &[
                complex_array(
                    [1, 2],
                    vec![ArrayComplex64::new(3.0, 4.0), ArrayComplex64::new(1.0, 1.0)],
                ),
                complex_array(
                    [1, 2],
                    vec![
                        ArrayComplex64::new(4.0, 3.0),
                        ArrayComplex64::new(-2.0, 0.0),
                    ],
                ),
                char_row("ComparisonMethod"),
                char_row("real"),
            ],
        )),
        &[1, 2],
        &[
            ArrayComplex64::new(3.0, 4.0),
            ArrayComplex64::new(-2.0, 0.0),
        ],
    );
}

#[test]
fn extrema_elementwise_preserve_integer_logical_and_char_classes() {
    assert_eq!(
        invoke(
            "min",
            &[
                exact_integer_array([1, 2], vec![3_i8, 1]),
                exact_integer_array([1, 2], vec![2_i8, 4]),
            ],
        ),
        Ok(vec![exact_integer_array([1, 2], vec![2_i8, 1])])
    );
    assert_eq!(
        invoke(
            "min",
            &[
                exact_integer_array([1, 2], vec![1_u8, 4]),
                Value::Double(3.5),
            ],
        ),
        Ok(vec![exact_integer_array([1, 2], vec![1_u8, 4])])
    );
    assert_logical_array(
        &only_output(invoke(
            "min",
            &[
                logical_array([1, 2], vec![true, false]),
                logical_array([1, 2], vec![false, true]),
            ],
        )),
        &[1, 2],
        &[false, false],
    );
    assert_logical_array(
        &only_output(invoke(
            "min",
            &[
                logical_array([1, 2], vec![true, false]),
                Value::Logical(true),
            ],
        )),
        &[1, 2],
        &[true, false],
    );
    assert_real_array(
        &only_output(invoke("min", &[char_row("AZ"), char_row("BY")])),
        &[1, 2],
        &[65.0, 89.0],
    );
}

#[test]
fn dot_covers_orientation_matrix_complex_empty_and_provider_paths() {
    let row = real_array([1, 2], vec![1.0, 2.0]);
    let column = real_array([2, 1], vec![1.0, 2.0]);
    assert_eq!(invoke("dot", &[row, column]), Ok(vec![Value::Double(5.0)]));

    let factor = 2.0_f64.powi(-27);
    let left_factor = 1.0 + factor;
    let right_factor = 1.0 - factor;
    let mut fma_left = vec![0.0; 4_097];
    let mut fma_right = vec![0.0; 4_097];
    fma_left[4_095] = -1.0;
    fma_right[4_095] = 1.0;
    fma_left[4_096] = left_factor;
    fma_right[4_096] = right_factor;
    let expected_fma = left_factor.mul_add(right_factor, -1.0);
    assert_ne!(expected_fma.to_bits(), 0.0_f64.to_bits());
    assert_eq!(
        invoke(
            "dot",
            &[
                real_array([1, 4_097], fma_left),
                real_array([1, 4_097], fma_right),
            ],
        ),
        Ok(vec![Value::Double(expected_fma)])
    );

    let left = complex_array(
        [1, 2],
        vec![ArrayComplex64::new(1.0, 2.0), ArrayComplex64::new(3.0, 0.0)],
    );
    let right = complex_array(
        [1, 2],
        vec![ArrayComplex64::new(4.0, 5.0), ArrayComplex64::new(6.0, 0.0)],
    );
    assert_eq!(
        invoke("dot", &[left, right]),
        Ok(vec![Value::Complex(Complex64::new(32.0, -3.0))])
    );

    let left = real_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]);
    let right = real_array([2, 2], vec![5.0, 7.0, 6.0, 8.0]);
    assert_real_array(
        &only_output(invoke("dot", &[left, right])),
        &[1, 2],
        &[26.0, 44.0],
    );
    assert_eq!(
        invoke(
            "dot",
            &[
                real_array([0, 0], Vec::new()),
                real_array([0, 0], Vec::new()),
            ],
        ),
        Ok(vec![Value::Double(0.0)])
    );
}

#[test]
fn norm_covers_r2022b_vectors_matrices_orders_classes_and_provider_paths() {
    assert_eq!(
        invoke("norm", &[real_array([1, 2], vec![3.0, 4.0])]),
        Ok(vec![Value::Double(5.0)])
    );
    assert_eq!(
        invoke(
            "norm",
            &[complex_array([1, 1], vec![ArrayComplex64::new(3.0, 4.0)])]
        ),
        Ok(vec![Value::Double(5.0)])
    );
    assert_eq!(
        invoke("norm", &[real_array([0, 0], Vec::new())]),
        Ok(vec![Value::Double(0.0)])
    );
    let mut special_values = vec![0.0; 4_097];
    special_values[0] = f64::INFINITY;
    special_values[4_096] = f64::NAN;
    let special_norm = only_output(invoke("norm", &[real_array([1, 4_097], special_values)]));
    let Value::Double(special_norm) = special_norm else {
        panic!("norm must return a real scalar");
    };
    assert!(special_norm.is_nan());
}

#[test]
fn norm_matrix_orders_classes_and_provider_paths_match_r2022b() {
    assert_eq!(
        invoke("norm", &[real_array([2, 2], vec![1.0, 0.0, 0.0, 2.0])]),
        Ok(vec![Value::Double(2.0)])
    );

    let matrix = real_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]);
    let Value::Double(spectral) = only_output(invoke("norm", std::slice::from_ref(&matrix))) else {
        panic!("matrix norm must return a double scalar");
    };
    assert!((spectral - 5.464_985_704_219_043).abs() < 1.0e-12);
    assert_eq!(
        invoke("norm", &[matrix.clone(), Value::Double(1.0)]),
        Ok(vec![Value::Double(6.0)])
    );
    assert_eq!(
        invoke("norm", &[matrix.clone(), Value::Double(f64::INFINITY)]),
        Ok(vec![Value::Double(7.0)])
    );
    let Value::Double(frobenius) = only_output(invoke("norm", &[matrix, char_row("fro")])) else {
        panic!("Frobenius norm must return a double scalar");
    };
    assert!((frobenius - 30.0_f64.sqrt()).abs() < 1.0e-14);

    let Value::Array(ArrayData::F32(single)) = only_output(invoke(
        "norm",
        &[single_array([2, 2], vec![1.0, 3.0, 2.0, 4.0])],
    )) else {
        panic!("single matrix norm must return a single scalar");
    };
    assert_eq!(single.shape().dimensions(), &[1, 1]);
    assert!((single.as_slice()[0] - 5.464_986).abs() < 1.0e-5);

    let Value::Double(complex_spectral) = only_output(invoke(
        "norm",
        &[complex_array(
            [2, 2],
            vec![
                ArrayComplex64::new(1.0, 2.0),
                ArrayComplex64::new(3.0, 0.0),
                ArrayComplex64::new(2.0, 0.0),
                ArrayComplex64::new(4.0, -1.0),
            ],
        )],
    )) else {
        panic!("complex matrix norm must return a double scalar");
    };
    assert!((complex_spectral - 5.791_287_847_477_92).abs() < 1.0e-12);

    let vector = real_array([1, 2], vec![3.0, 4.0]);
    let Value::Double(power_three) =
        only_output(invoke("norm", &[vector.clone(), Value::Double(3.0)]))
    else {
        panic!("vector p-norm must return a double scalar");
    };
    assert!((power_three - 4.497_941_445_275_415).abs() < 1.0e-14);
    assert_eq!(
        invoke("norm", &[vector.clone(), Value::Double(f64::NEG_INFINITY)]),
        Ok(vec![Value::Double(3.0)])
    );
    assert_eq!(
        invoke("norm", &[vector.clone(), Value::Double(0.0)]),
        Ok(vec![Value::Double(f64::INFINITY)])
    );
    assert_error(
        invoke("norm", &[vector, Value::Double(-1.0)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "norm",
            &[
                real_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]),
                Value::Double(3.0),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
    let Value::Double(matrix_nan) = only_output(invoke(
        "norm",
        &[real_array([2, 2], vec![f64::NAN, 0.0, 0.0, 1.0])],
    )) else {
        panic!("matrix norm with NaN must return a double scalar");
    };
    assert!(matrix_nan.is_nan());
}

#[test]
fn rounding_builtins_preserve_r2022b_classes_shapes_complex_and_empty_values() {
    let doubles = real_array([1, 5], vec![-2.5, -0.5, -0.0, 0.5, 2.5]);
    assert_real_array(
        &only_output(invoke("fix", std::slice::from_ref(&doubles))),
        &[1, 5],
        &[-2.0, -0.0, -0.0, 0.0, 2.0],
    );
    assert_real_array(
        &only_output(invoke("floor", std::slice::from_ref(&doubles))),
        &[1, 5],
        &[-3.0, -1.0, -0.0, 0.0, 2.0],
    );
    assert_real_array(
        &only_output(invoke("ceil", &[doubles])),
        &[1, 5],
        &[-2.0, -0.0, -0.0, 1.0, 3.0],
    );

    assert_single_array(
        &only_output(invoke("floor", &[single_array([1, 2], vec![-1.25, 1.25])])),
        &[1, 2],
        &[-2.0, 1.0],
    );
    assert_complex_array(
        &only_output(invoke(
            "ceil",
            &[complex_array(
                [1, 2],
                vec![
                    ArrayComplex64::new(1.8, 2.2),
                    ArrayComplex64::new(-1.8, -2.2),
                ],
            )],
        )),
        &[1, 2],
        &[
            ArrayComplex64::new(2.0, 3.0),
            ArrayComplex64::new(-1.0, -2.0),
        ],
    );
    assert_real_array(
        &only_output(invoke("fix", &[logical_array([1, 2], vec![false, true])])),
        &[1, 2],
        &[0.0, 1.0],
    );
    let integer = exact_integer_array([1, 2], vec![i64::MIN, i64::MAX]);
    let unchanged = only_output(invoke("ceil", std::slice::from_ref(&integer)));
    assert!(integer.shares_array_storage_with(&unchanged));
    assert_integer_array(
        &unchanged,
        "int64",
        &[1, 2],
        false,
        &[("-9223372036854775808", "0"), ("9223372036854775807", "0")],
    );
    assert_real_array(
        &only_output(invoke("floor", &[char_array([1, 2], &[65, 0xd83d])])),
        &[1, 2],
        &[65.0, 55_357.0],
    );
    assert_single_array(
        &only_output(invoke("fix", &[single_array([0, 3], Vec::new())])),
        &[0, 3],
        &[],
    );
}

#[test]
fn elementary_builtins_cover_double_single_complex_negative_log_empty_and_rejections() {
    let exponential = only_output(invoke(
        "exp",
        &[complex_array([1, 1], vec![ArrayComplex64::new(1.0, 2.0)])],
    ));
    let Value::Array(ArrayData::ComplexF64(exponential)) = exponential else {
        panic!("complex exp must retain complex storage");
    };
    assert!((exponential.as_slice()[0].re - -1.131_204_383_756_813_5).abs() < 1.0e-14);
    assert!((exponential.as_slice()[0].im - 2.471_726_672_004_818_8).abs() < 1.0e-14);

    let logarithm = only_output(invoke("log", &[real_array([1, 3], vec![-1.0, 0.0, 10.0])]));
    let Value::Array(ArrayData::ComplexF64(logarithm)) = logarithm else {
        panic!("negative real logarithm must produce complex double storage");
    };
    assert_eq!(logarithm.shape().dimensions(), &[1, 3]);
    assert_eq!(
        logarithm.as_slice()[0],
        ArrayComplex64::new(0.0, std::f64::consts::PI)
    );
    assert!(
        logarithm.as_slice()[1].re.is_infinite() && logarithm.as_slice()[1].re.is_sign_negative()
    );
    assert_eq!(logarithm.as_slice()[1].im.to_bits(), 0.0_f64.to_bits());
    assert!((logarithm.as_slice()[2].re - std::f64::consts::LN_10).abs() < 1.0e-15);

    let single_sine = only_output(invoke("sin", &[single_array([1, 2], vec![-0.5, 0.5])]));
    assert_single_array(&single_sine, &[1, 2], &[(-0.5_f32).sin(), 0.5_f32.sin()]);
    let square_root = only_output(invoke("sqrt", &[real_array([1, 2], vec![-1.0, 4.0])]));
    assert_complex_array(
        &square_root,
        &[1, 2],
        &[ArrayComplex64::new(0.0, 1.0), ArrayComplex64::new(2.0, 0.0)],
    );
    assert_single_array(
        &only_output(invoke("sqrt", &[single_array([2, 1], vec![1.0, 4.0])])),
        &[2, 1],
        &[1.0, 2.0],
    );
    assert_real_array(
        &only_output(invoke("cos", &[real_array([0, 3], Vec::new())])),
        &[0, 3],
        &[],
    );
    for name in ["exp", "log", "log10", "sin", "cos", "tan"] {
        assert_error(
            invoke(name, &[logical_array([1, 1], vec![true])]),
            BuiltinErrorCategory::Type,
        );
        assert_error(
            invoke(name, &[exact_integer_array([1, 1], vec![1_i8])]),
            BuiltinErrorCategory::Type,
        );
    }
}

#[test]
fn expm1_log1p_and_angle_preserve_precision_branches_classes_and_shapes() {
    let tiny = 1.0e-12;
    let expm1 = only_output(invoke(
        "expm1",
        &[complex_array(
            [1, 2],
            vec![
                ArrayComplex64::new(tiny, 0.0),
                ArrayComplex64::new(tiny, -2.0 * tiny),
            ],
        )],
    ));
    let Value::Array(ArrayData::ComplexF64(expm1)) = expm1 else {
        panic!("complex expm1 must retain complex double storage");
    };
    assert_eq!(expm1.shape().dimensions(), &[1, 2]);
    assert_eq!(expm1.as_slice()[0].re.to_bits(), tiny.exp_m1().to_bits());
    assert_eq!(expm1.as_slice()[0].im.to_bits(), 0.0_f64.to_bits());
    assert!((expm1.as_slice()[1].re - tiny).abs() < 3.0e-24);
    assert!((expm1.as_slice()[1].im + 2.0 * tiny).abs() < 3.0e-24);

    let log1p = only_output(invoke(
        "log1p",
        &[real_array(
            [1, 6],
            vec![0.0, tiny, -1.0, -2.0, f64::INFINITY, f64::NAN],
        )],
    ));
    let Value::Array(ArrayData::ComplexF64(log1p)) = log1p else {
        panic!("log1p below minus one must promote the complete array to complex storage");
    };
    assert_eq!(log1p.shape().dimensions(), &[1, 6]);
    assert_eq!(log1p.as_slice()[1].re.to_bits(), tiny.ln_1p().to_bits());
    assert!(log1p.as_slice()[2].re.is_infinite() && log1p.as_slice()[2].re.is_sign_negative());
    assert_eq!(
        log1p.as_slice()[3],
        ArrayComplex64::new(0.0, std::f64::consts::PI)
    );
    assert!(log1p.as_slice()[4].re.is_infinite() && log1p.as_slice()[4].re.is_sign_positive());
    assert!(log1p.as_slice()[5].re.is_nan());
    assert_eq!(log1p.as_slice()[5].im.to_bits(), 0.0_f64.to_bits());

    assert_single_array(
        &only_output(invoke(
            "angle",
            &[complex_single_array(
                [2, 1],
                vec![
                    ArrayComplex32::new(-1.0, 0.0),
                    ArrayComplex32::new(0.0, 1.0),
                ],
            )],
        )),
        &[2, 1],
        &[std::f32::consts::PI, std::f32::consts::FRAC_PI_2],
    );
    assert_real_array(
        &only_output(invoke("angle", &[real_array([0, 3], Vec::new())])),
        &[0, 3],
        &[],
    );

    for name in ["expm1", "log1p", "angle"] {
        assert_error(
            invoke(name, &[logical_array([1, 1], vec![true])]),
            BuiltinErrorCategory::Type,
        );
    }
}

#[test]
fn meshgrid_matches_r2022b_vector_orientation_shape_order_and_precision() {
    let outputs = invoke_with_outputs(
        "meshgrid",
        &[
            real_array([2, 1], vec![1.0, 2.0]),
            single_array([1, 3], vec![3.0, 4.0, 5.0]),
        ],
        2,
    )
    .unwrap();
    assert_real_array(&outputs[0], &[3, 2], &[1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
    assert_single_array(&outputs[1], &[3, 2], &[3.0, 4.0, 5.0, 3.0, 4.0, 5.0]);

    let square =
        invoke_with_outputs("meshgrid", &[real_array([3, 1], vec![1.0, 2.0, 3.0])], 2).unwrap();
    assert_real_array(
        &square[0],
        &[3, 3],
        &[1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0],
    );
    assert_real_array(
        &square[1],
        &[3, 3],
        &[1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0],
    );

    let volume = invoke_with_outputs(
        "meshgrid",
        &[
            real_array([1, 2], vec![10.0, 20.0]),
            single_array([3, 1], vec![1.0, 2.0, 3.0]),
            real_array([1, 2], vec![7.0, 8.0]),
        ],
        3,
    )
    .unwrap();
    assert_real_array(
        &volume[0],
        &[3, 2, 2],
        &[
            10.0, 10.0, 10.0, 20.0, 20.0, 20.0, 10.0, 10.0, 10.0, 20.0, 20.0, 20.0,
        ],
    );
    assert_single_array(
        &volume[1],
        &[3, 2, 2],
        &[1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0],
    );
    assert_real_array(
        &volume[2],
        &[3, 2, 2],
        &[7.0, 7.0, 7.0, 7.0, 7.0, 7.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0],
    );

    let cubic = invoke_with_outputs("meshgrid", &[real_array([1, 2], vec![1.0, 2.0])], 3).unwrap();
    for output in &cubic {
        assert_eq!(output.dimensions(), Some(&[2, 2, 2][..]));
    }
    assert_error(
        invoke_with_outputs(
            "meshgrid",
            &[
                real_array([1, 2], vec![1.0, 2.0]),
                real_array([1, 2], vec![3.0, 4.0]),
            ],
            3,
        ),
        BuiltinErrorCategory::ArgumentCount,
    );
}

#[test]
fn isosurface_returns_matlab_ordered_faces_vertices_and_scalar_structure() {
    let field = real_array([2, 2, 2], vec![0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
    let outputs = invoke_with_outputs("isosurface", &[field.clone(), Value::Double(0.5)], 2)
        .expect("two-output isosurface must succeed");
    let Value::Array(ArrayData::F64(faces)) = &outputs[0] else {
        panic!("isosurface faces must be a real double matrix")
    };
    let Value::Array(ArrayData::F64(vertices)) = &outputs[1] else {
        panic!("isosurface vertices must be a real double matrix")
    };
    assert_eq!(faces.shape().extent(1), 3);
    assert_eq!(vertices.shape().extent(1), 3);
    assert!(!faces.is_empty());
    assert!(!vertices.is_empty());
    let vertex_rows = usize::try_from(vertices.shape().extent(0)).unwrap();
    assert!(
        vertices.as_slice()[..vertex_rows]
            .iter()
            .all(|x| (*x - 1.5).abs() <= f64::EPSILON)
    );
    let vertex_upper = f64::from(u32::try_from(vertices.shape().extent(0)).unwrap());
    assert!(
        faces
            .as_slice()
            .iter()
            .all(|index| *index >= 1.0 && *index <= vertex_upper)
    );

    let structure = only_output(invoke("isosurface", &[field, Value::Double(0.5)]));
    let Value::Struct(structure) = structure else {
        panic!("one-output isosurface must return a structure")
    };
    assert_eq!(
        structure
            .field_names()
            .iter()
            .map(openmat_value::FieldName::as_str)
            .collect::<Vec<_>>(),
        ["vertices", "faces"]
    );
}

#[test]
fn diagonal_builtins_cover_vector_matrix_offsets_classes_empty_and_nd_errors() {
    assert_real_array(
        &only_output(invoke(
            "diag",
            &[real_array([1, 3], vec![1.0, 2.0, 3.0]), Value::Double(1.0)],
        )),
        &[4, 4],
        &[
            0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0,
        ],
    );
    assert_real_array(
        &only_output(invoke(
            "diag",
            &[real_array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])],
        )),
        &[2, 1],
        &[1.0, 4.0],
    );
    assert_logical_array(
        &only_output(invoke("diag", &[logical_array([1, 2], vec![false, true])])),
        &[2, 2],
        &[false, false, false, true],
    );
    assert_integer_array(
        &only_output(invoke(
            "diag",
            &[exact_integer_array([1, 2], vec![1_i8, 2_i8])],
        )),
        "int8",
        &[2, 2],
        false,
        &[("1", "0"), ("0", "0"), ("0", "0"), ("2", "0")],
    );
    assert_real_array(
        &only_output(invoke("diag", &[real_array([0, 3], Vec::new())])),
        &[0, 1],
        &[],
    );
    assert_real_array(
        &only_output(invoke("diag", &[real_array([0, 0], Vec::new())])),
        &[0, 0],
        &[],
    );
    assert_error(
        invoke("diag", &[real_array([2, 1, 2], vec![1.0; 4])]),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn triangular_builtins_preserve_storage_and_canonicalize_zeroed_complex_outputs() {
    let matrix = real_array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_real_array(
        &only_output(invoke("triu", std::slice::from_ref(&matrix))),
        &[2, 3],
        &[1.0, 0.0, 3.0, 4.0, 5.0, 6.0],
    );
    assert_real_array(
        &only_output(invoke("tril", &[matrix, Value::Double(-1.0)])),
        &[2, 3],
        &[0.0, 2.0, 0.0, 0.0, 0.0, 0.0],
    );
    assert_single_array(
        &only_output(invoke(
            "triu",
            &[
                single_array([1, 3], vec![1.0, 2.0, 3.0]),
                Value::Double(1.0),
            ],
        )),
        &[1, 3],
        &[0.0, 2.0, 3.0],
    );
    assert_integer_array(
        &only_output(invoke(
            "tril",
            &[exact_integer_array([1, 3], vec![1_u64, 2, 3])],
        )),
        "uint64",
        &[1, 3],
        false,
        &[("1", "0"), ("0", "0"), ("0", "0")],
    );
    let zeroed = only_output(invoke(
        "tril",
        &[
            complex_array(
                [1, 2],
                vec![ArrayComplex64::new(1.0, 3.0), ArrayComplex64::new(2.0, 4.0)],
            ),
            Value::Double(-1.0),
        ],
    ));
    assert_real_array(&zeroed, &[1, 2], &[0.0, 0.0]);
    assert_real_array(
        &only_output(invoke("triu", &[real_array([0, 3], Vec::new())])),
        &[0, 3],
        &[],
    );
    assert_error(
        invoke("triu", &[real_array([2, 1, 2], vec![1.0; 4])]),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn checked_dimensions_and_unsupported_forms_return_stable_errors() {
    assert_error(
        invoke("zeros", &[Value::Double(f64::INFINITY)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("ones", &[Value::Double(-1.0)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "zeros",
            &[real_array([1, 2], vec![9_223_372_036_854_775_808.0, 3.0])],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "zeros",
            &[real_array([1, 2], vec![18_446_744_073_709_549_568.0, 1.0])],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "reshape",
            &[
                real_array([1, 3], vec![1.0, 2.0, 3.0]),
                Value::Double(2.0),
                Value::Double(2.0),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("eye", &[real_array([1, 3], vec![2.0, 3.0, 4.0])]),
        BuiltinErrorCategory::Domain,
    );
    assert_eq!(
        invoke("min", &[Value::Double(1.0), Value::Double(2.0)]),
        Ok(vec![Value::Double(1.0)])
    );
    let scalar_maximum = invoke_with_outputs("max", &[Value::Double(1.0)], 2)
        .expect("scalar max supports a double index output");
    assert_eq!(scalar_maximum[0], Value::Double(1.0));
    assert_real_array(&scalar_maximum[1], &[1, 1], &[1.0]);
    assert_error(
        invoke(
            "dot",
            &[
                real_array([2, 2], vec![1.0; 4]),
                real_array([1, 4], vec![1.0; 4]),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("norm", &[Value::Logical(true)]),
        BuiltinErrorCategory::Type,
    );
    assert_error(invoke("sum", &[]), BuiltinErrorCategory::ArgumentCount);
}

#[test]
fn cancelled_token_rejects_an_array_traversal_builtin() {
    let registry = minimal_registry().expect("fresh registry must register the core built-ins");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let input = real_array([1, 8_192], vec![1.0; 8_192]);

    assert_error(
        invoke_in_registry_with_cancellation(&registry, "complex", &[input], 1, &cancellation),
        BuiltinErrorCategory::Cancelled,
    );
}

#[test]
fn all_integer_constructors_round_saturate_and_preserve_exact_boundaries() {
    let input = real_array(
        [1, 5],
        vec![f64::NEG_INFINITY, -2.5, f64::NAN, 0.5, f64::INFINITY],
    );
    for (name, minimum, maximum, negative_half) in [
        ("int8", "-128", "127", "-3"),
        ("uint8", "0", "255", "0"),
        ("int16", "-32768", "32767", "-3"),
        ("uint16", "0", "65535", "0"),
        ("int32", "-2147483648", "2147483647", "-3"),
        ("uint32", "0", "4294967295", "0"),
        ("int64", "-9223372036854775808", "9223372036854775807", "-3"),
        ("uint64", "0", "18446744073709551615", "0"),
    ] {
        let converted = only_output(invoke(name, std::slice::from_ref(&input)));
        assert_integer_array(
            &converted,
            name,
            &[1, 5],
            false,
            &[
                (minimum, "0"),
                (negative_half, "0"),
                ("0", "0"),
                ("1", "0"),
                (maximum, "0"),
            ],
        );
        let (real_dtype, complex_dtype) = match name {
            "int8" => (DType::I8, DType::ComplexI8),
            "uint8" => (DType::U8, DType::ComplexU8),
            "int16" => (DType::I16, DType::ComplexI16),
            "uint16" => (DType::U16, DType::ComplexU16),
            "int32" => (DType::I32, DType::ComplexI32),
            "uint32" => (DType::U32, DType::ComplexU32),
            "int64" => (DType::I64, DType::ComplexI64),
            "uint64" => (DType::U64, DType::ComplexU64),
            _ => unreachable!("the test enumerates all integer constructors"),
        };
        assert_eq!(converted.dtype(), Some(real_dtype));

        let converted = only_output(invoke(name, &[Value::Complex(Complex64::new(1.0, 2.0))]));
        assert_integer_array(&converted, name, &[1, 1], true, &[("1", "2")]);
        assert_eq!(converted.dtype(), Some(complex_dtype));
    }

    let maximum = exact_integer_array([1, 1], vec![u64::MAX]);
    let preserved = only_output(invoke("uint64", std::slice::from_ref(&maximum)));
    assert_integer_array(
        &preserved,
        "uint64",
        &[1, 1],
        false,
        &[("18446744073709551615", "0")],
    );
    let saturated = only_output(invoke("int64", &[maximum]));
    assert_integer_array(
        &saturated,
        "int64",
        &[1, 1],
        false,
        &[("9223372036854775807", "0")],
    );
}

#[test]
fn complex_integer_conversion_is_componentwise_and_collapses_zero_imaginary_parts() {
    let input = complex_array(
        [1, 3],
        vec![
            ArrayComplex64::new(1.4, 99.4),
            ArrayComplex64::new(-1.5, -99.5),
            ArrayComplex64::new(200.0, 1.0),
        ],
    );
    let converted = only_output(invoke("int8", &[input]));
    assert_integer_array(
        &converted,
        "int8",
        &[1, 3],
        true,
        &[("1", "99"), ("-2", "-100"), ("127", "1")],
    );
    assert_error(
        invoke("logical", std::slice::from_ref(&converted)),
        BuiltinErrorCategory::Type,
    );

    let collapsed = only_output(invoke("int16", &[Value::Complex(Complex64::new(1.2, 0.4))]));
    assert_integer_array(&collapsed, "int16", &[1, 1], false, &[("1", "0")]);
}

#[test]
fn char_constructor_preserves_all_code_units_and_saturates_numeric_inputs() {
    let code_units = exact_integer_array([1, 4], vec![0_u16, 0xd83d, 0xde42, u16::MAX]);
    let characters = only_output(invoke("char", std::slice::from_ref(&code_units)));
    let Value::Array(ArrayData::Char(array)) = &characters else {
        panic!("char constructor must return exact char storage");
    };
    assert_eq!(array.shape().dimensions(), &[1, 4]);
    assert_eq!(
        array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
        [0, 0xd83d, 0xde42, u16::MAX]
    );
    let round_trip = only_output(invoke("uint16", &[characters]));
    assert_integer_array(
        &round_trip,
        "uint16",
        &[1, 4],
        false,
        &[("0", "0"), ("55357", "0"), ("56898", "0"), ("65535", "0")],
    );

    let saturated = only_output(invoke(
        "char",
        &[real_array([1, 4], vec![-1.0, 65.5, 65.9, 70_000.0])],
    ));
    let Value::Array(ArrayData::Char(array)) = saturated else {
        panic!("char constructor must return char storage");
    };
    assert_eq!(
        array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
        [0, 65, 65, u16::MAX]
    );
    let integer_saturation = only_output(invoke(
        "char",
        &[exact_integer_array([1, 3], vec![-1_i64, 65, i64::MAX])],
    ));
    let Value::Array(ArrayData::Char(array)) = integer_saturation else {
        panic!("integer-to-char conversion must return char storage");
    };
    assert_eq!(
        array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
        [0, 65, u16::MAX]
    );
}

#[test]
fn integer_metadata_conversions_reshape_cow_and_exact_dimensions_work() {
    let source = exact_integer_array([1, 2], vec![u64::MAX, 2]);
    assert_eq!(
        invoke("class", std::slice::from_ref(&source)),
        Ok(vec![Value::from("uint64")])
    );
    assert_eq!(
        invoke(
            "isa",
            &[
                source.clone(),
                char_array([1, 6], &[117, 105, 110, 116, 54, 52])
            ]
        ),
        Ok(vec![Value::Logical(true)])
    );
    assert_eq!(
        invoke("numel", std::slice::from_ref(&source)),
        Ok(vec![Value::Double(2.0)])
    );
    assert_eq!(
        invoke("logical", std::slice::from_ref(&source)),
        Ok(vec![logical_array([1, 2], vec![true, true])])
    );
    assert_real_array(
        &only_output(invoke("double", std::slice::from_ref(&source))),
        &[1, 2],
        &[18_446_744_073_709_551_616.0, 2.0],
    );

    let unchanged = only_output(invoke(
        "reshape",
        &[source.clone(), Value::Double(1.0), Value::Double(2.0)],
    ));
    assert!(source.shares_array_storage_with(&unchanged));
    let reshaped = only_output(invoke(
        "reshape",
        &[
            source.clone(),
            exact_integer_array([1, 1], vec![2_u8]),
            exact_integer_array([1, 1], vec![1_u16]),
        ],
    ));
    assert!(!source.shares_array_storage_with(&reshaped));
    assert_integer_array(
        &reshaped,
        "uint64",
        &[2, 1],
        false,
        &[("18446744073709551615", "0"), ("2", "0")],
    );
}

#[test]
fn strcmp_preserves_utf16_surrogates_empty_and_missing_truth() {
    let strings = exact_string_array(
        [1, 3],
        vec![
            StringElement::from_code_units(vec![0xd83d]),
            StringElement::from_code_units(Vec::<u16>::new()),
            StringElement::missing(),
        ],
    );
    assert_logical_array(
        &only_output(invoke("strcmp", &[strings.clone(), strings.clone()])),
        &[1, 3],
        &[true, true, false],
    );
    assert_logical_array(
        &only_output(invoke("strcmp", &[char_array([1, 1], &[0xd83d]), strings])),
        &[1, 3],
        &[true, false, false],
    );
}

#[test]
fn cancelled_token_rejects_integer_conversion_before_exact_traversal() {
    let registry = minimal_registry().expect("fresh registry must register the core built-ins");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let input = exact_integer_array([1, 8_192], vec![u64::MAX; 8_192]);
    assert_error(
        invoke_in_registry_with_cancellation(&registry, "int8", &[input], 1, &cancellation),
        BuiltinErrorCategory::Cancelled,
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn statistics_cover_dimensions_native_classes_missing_values_and_complex_dispersion() {
    let matrix = real_array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_real_array(
        &only_output(invoke("mean", std::slice::from_ref(&matrix))),
        &[1, 3],
        &[1.5, 3.5, 5.5],
    );
    assert_real_array(
        &only_output(invoke("mean", &[matrix.clone(), Value::Double(2.0)])),
        &[2, 1],
        &[3.0, 4.0],
    );
    assert_eq!(
        invoke("mean", &[matrix.clone(), Value::from("all")]),
        Ok(vec![Value::Double(3.5)])
    );
    assert_eq!(
        invoke(
            "median",
            &[matrix.clone(), real_array([1, 2], vec![1.0, 2.0])],
        ),
        Ok(vec![Value::Double(3.5)])
    );

    assert_single_array(
        &only_output(invoke(
            "mean",
            &[single_array([1, 3], vec![1.0e10, 1.0, -1.0e10])],
        )),
        &[1, 1],
        &[0.0],
    );
    assert_integer_array(
        &only_output(invoke(
            "mean",
            &[
                exact_integer_array([1, 2], vec![1_i8, 2]),
                Value::from("native"),
            ],
        )),
        "int8",
        &[1, 1],
        false,
        &[("2", "0")],
    );
    assert_eq!(
        invoke(
            "mean",
            &[
                logical_array([1, 2], vec![true, true]),
                Value::from("native"),
            ],
        ),
        Ok(vec![Value::Double(0.5)])
    );
    assert_eq!(
        invoke(
            "mean",
            &[
                real_array([1, 3], vec![1.0, f64::NAN, 3.0]),
                Value::from("omitnan"),
            ],
        ),
        Ok(vec![Value::Double(2.0)])
    );

    assert_integer_array(
        &only_output(invoke(
            "median",
            &[exact_integer_array([1, 4], vec![-2_i8, 1, -1, 0])],
        )),
        "int8",
        &[1, 1],
        false,
        &[("0", "0")],
    );
    assert_logical_array(
        &only_output(invoke(
            "median",
            &[logical_array([1, 2], vec![false, true])],
        )),
        &[1, 1],
        &[true],
    );
    let odd_char = only_output(invoke("median", &[char_array([1, 3], &[97, 98, 99])]));
    let Value::Array(ArrayData::Char(odd_char)) = odd_char else {
        panic!("odd char median must preserve char storage");
    };
    assert_eq!(odd_char.as_slice()[0].get(), 98);
    assert_eq!(
        invoke("median", &[char_array([1, 2], &[97, 98])]),
        Ok(vec![Value::Double(97.5)])
    );
    assert_error(
        invoke("median", &[char_array([0, 3], &[])]),
        BuiltinErrorCategory::Domain,
    );
    assert_eq!(
        invoke(
            "median",
            &[complex_array(
                [1, 2],
                vec![
                    ArrayComplex64::new(0.0, -1.0),
                    ArrayComplex64::new(0.0, 1.0),
                ],
            )],
        ),
        Ok(vec![Value::Double(0.0)])
    );

    let complex = complex_array(
        [1, 2],
        vec![ArrayComplex64::new(1.0, 1.0), ArrayComplex64::new(3.0, 3.0)],
    );
    assert_eq!(
        invoke("var", std::slice::from_ref(&complex)),
        Ok(vec![Value::Double(4.0)])
    );
    assert_eq!(
        invoke("std", &[complex, Value::Double(1.0)]),
        Ok(vec![Value::Double(2.0_f64.sqrt())])
    );
    assert_error(
        invoke("std", &[exact_integer_array([1, 2], vec![1_i8, 2])]),
        BuiltinErrorCategory::Type,
    );
    assert_real_array(
        &only_output(invoke("mean", &[real_array([0, 3], Vec::new())])),
        &[1, 3],
        &[f64::NAN, f64::NAN, f64::NAN],
    );
}

#[test]
fn sort_is_stable_class_preserving_and_supports_complex_and_missing_policies() {
    let input = real_array([1, 4], vec![3.0, f64::NAN, 1.0, 1.0]);
    let outputs = invoke_with_outputs("sort", std::slice::from_ref(&input), 2).unwrap();
    assert_real_array(&outputs[0], &[1, 4], &[1.0, 1.0, 3.0, f64::NAN]);
    assert_real_array(&outputs[1], &[1, 4], &[3.0, 4.0, 1.0, 2.0]);
    let outputs = invoke_with_outputs(
        "sort",
        &[
            input,
            Value::Double(2.0),
            Value::from("descend"),
            Value::from("MissingPlacement"),
            Value::from("first"),
        ],
        2,
    )
    .unwrap();
    assert_real_array(&outputs[0], &[1, 4], &[f64::NAN, 3.0, 1.0, 1.0]);
    assert_real_array(&outputs[1], &[1, 4], &[2.0, 1.0, 3.0, 4.0]);

    let complex = complex_array(
        [1, 4],
        vec![
            ArrayComplex64::new(0.0, 1.0),
            ArrayComplex64::new(0.0, -1.0),
            ArrayComplex64::new(1.0, 0.0),
            ArrayComplex64::new(-1.0, 0.0),
        ],
    );
    let outputs = invoke_with_outputs("sort", &[complex], 2).unwrap();
    assert_complex_array(
        &outputs[0],
        &[1, 4],
        &[
            ArrayComplex64::new(0.0, -1.0),
            ArrayComplex64::new(1.0, 0.0),
            ArrayComplex64::new(0.0, 1.0),
            ArrayComplex64::new(-1.0, 0.0),
        ],
    );
    assert_real_array(&outputs[1], &[1, 4], &[2.0, 3.0, 1.0, 4.0]);

    assert_single_array(
        &only_output(invoke(
            "sort",
            &[single_array([1, 3], vec![2.0, -1.0, 0.0])],
        )),
        &[1, 3],
        &[-1.0, 0.0, 2.0],
    );
    assert_integer_array(
        &only_output(invoke(
            "sort",
            &[exact_integer_array([1, 3], vec![2_i16, -1, 0])],
        )),
        "int16",
        &[1, 3],
        false,
        &[("-1", "0"), ("0", "0"), ("2", "0")],
    );
    assert_logical_array(
        &only_output(invoke(
            "sort",
            &[logical_array([1, 3], vec![true, false, true])],
        )),
        &[1, 3],
        &[false, true, true],
    );
}

#[test]
fn concatenation_preserves_column_major_shape_and_matlab_numeric_dominance() {
    let left = real_array([2, 2], vec![1.0, 2.0, 3.0, 4.0]);
    let right = real_array([2, 2], vec![5.0, 6.0, 7.0, 8.0]);
    assert_real_array(
        &only_output(invoke(
            "cat",
            &[Value::Double(3.0), left.clone(), right.clone()],
        )),
        &[2, 2, 2],
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
    );
    assert_real_array(
        &only_output(invoke(
            "horzcat",
            &[
                real_array([2, 1], vec![1.0, 2.0]),
                real_array([2, 1], vec![3.0, 4.0]),
            ],
        )),
        &[2, 2],
        &[1.0, 2.0, 3.0, 4.0],
    );
    assert_single_array(
        &only_output(invoke(
            "horzcat",
            &[Value::Double(1.5), single_array([1, 1], vec![2.5])],
        )),
        &[1, 2],
        &[1.5, 2.5],
    );
    assert_integer_array(
        &only_output(invoke(
            "horzcat",
            &[
                exact_integer_array([1, 1], vec![1_i8]),
                Value::Double(130.0),
                Value::Double(-1.5),
            ],
        )),
        "int8",
        &[1, 3],
        false,
        &[("1", "0"), ("127", "0"), ("-2", "0")],
    );
    let characters = only_output(invoke(
        "horzcat",
        &[char_array([1, 1], &[65]), Value::Double(66.9)],
    ));
    let Value::Array(ArrayData::Char(characters)) = characters else {
        panic!("char dominance must produce char storage");
    };
    assert_eq!(characters.shape().dimensions(), &[1, 2]);
    assert_eq!(
        characters
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
        [65, 66]
    );
    let preserved = only_output(invoke("vertcat", std::slice::from_ref(&left)));
    assert!(left.shares_array_storage_with(&preserved));
    assert_eq!(invoke("horzcat", &[]), Ok(vec![Value::empty_double()]));
    assert_error(
        invoke(
            "horzcat",
            &[
                real_array([2, 1], vec![1.0, 2.0]),
                real_array([3, 1], vec![3.0, 4.0, 5.0]),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn sequences_and_round_cover_precision_complex_counts_digits_and_cow() {
    assert_real_array(
        &only_output(invoke(
            "linspace",
            &[Value::Double(0.0), Value::Double(1.0), Value::Double(5.0)],
        )),
        &[1, 5],
        &[0.0, 0.25, 0.5, 0.75, 1.0],
    );
    assert_single_array(
        &only_output(invoke(
            "linspace",
            &[
                Value::Double(0.0),
                single_array([1, 1], vec![1.0]),
                exact_integer_array([1, 1], vec![3_u8]),
            ],
        )),
        &[1, 3],
        &[0.0, 0.5, 1.0],
    );
    assert_complex_array(
        &only_output(invoke(
            "linspace",
            &[
                Value::Double(1.0),
                Value::Double(2.0),
                Value::Complex(Complex64::new(4.0, 1.0)),
            ],
        )),
        &[1, 4],
        &[
            ArrayComplex64::new(1.0, 0.0),
            ArrayComplex64::new(1.3, -0.099_999_999_999_999_99),
            ArrayComplex64::new(1.6, -0.199_999_999_999_999_98),
            ArrayComplex64::new(2.0, 0.0),
        ],
    );
    assert_real_array(
        &only_output(invoke(
            "linspace",
            &[
                Value::Double(1.0),
                Value::Double(2.0),
                Value::Double(f64::NEG_INFINITY),
            ],
        )),
        &[1, 0],
        &[],
    );
    assert_real_array(
        &only_output(invoke(
            "linspace",
            &[
                Value::Double(1.0),
                Value::Double(2.0),
                Value::Double(f64::NAN),
            ],
        )),
        &[1, 1],
        &[f64::NAN],
    );
    assert_real_array(
        &only_output(invoke(
            "linspace",
            &[
                Value::Double(1.0),
                Value::Double(2.0),
                Value::Complex(Complex64::new(4.0, f64::INFINITY)),
            ],
        )),
        &[1, 4],
        &[1.0, 1.0, 1.0, 2.0],
    );
    assert_real_array(
        &only_output(invoke(
            "logspace",
            &[Value::Double(0.0), Value::Double(2.0), Value::Double(3.0)],
        )),
        &[1, 3],
        &[1.0, 10.0, 100.0],
    );
    assert_real_array_close(
        &only_output(invoke(
            "logspace",
            &[
                Value::Double(0.0),
                Value::Double(std::f64::consts::PI),
                Value::Double(3.0),
            ],
        )),
        &[1, 3],
        &[1.0, std::f64::consts::PI.sqrt(), std::f64::consts::PI],
        1.0e-14,
    );

    assert_real_array(
        &only_output(invoke(
            "round",
            &[
                real_array([1, 3], vec![1.25, -1.25, 1.005]),
                Value::Double(2.0),
            ],
        )),
        &[1, 3],
        &[1.25, -1.25, 1.01],
    );
    assert_single_array(
        &only_output(invoke(
            "round",
            &[
                single_array([1, 3], vec![1.005, 2.675, 1.015]),
                Value::Double(2.0),
            ],
        )),
        &[1, 3],
        &[1.0, 2.67, 1.01],
    );
    assert_real_array(
        &only_output(invoke(
            "round",
            &[
                real_array([1, 2], vec![1234.5, 0.012_345]),
                Value::Double(3.0),
                Value::from("significant"),
            ],
        )),
        &[1, 2],
        &[1230.0, 0.0123],
    );
    assert_error(
        invoke(
            "round",
            &[
                Value::Double(12.345),
                Value::Double(0.0),
                Value::from("significant"),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_eq!(
        invoke("round", &[Value::Complex(Complex64::new(1.5, -2.5))],),
        Ok(vec![Value::Complex(Complex64::new(2.0, -3.0))])
    );
    let integers = exact_integer_array([1, 2], vec![1_i16, 2]);
    let rounded = only_output(invoke("round", std::slice::from_ref(&integers)));
    assert!(integers.shares_array_storage_with(&rounded));
    assert_error(
        invoke(
            "round",
            &[integers, exact_integer_array([1, 1], vec![1_u8])],
        ),
        BuiltinErrorCategory::Type,
    );
}

#[test]
fn text_conversion_builtins_cover_format_streams_shapes_complex_and_special_values() {
    assert_eq!(
        invoke(
            "sprintf",
            &[
                char_row("%g,"),
                real_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]),
            ],
        ),
        Ok(vec![char_row("1,3,2,4,")])
    );
    assert_eq!(
        invoke(
            "sprintf",
            &[
                char_row("%*.*f"),
                Value::Double(8.0),
                Value::Double(3.0),
                Value::Double(1.25),
            ],
        ),
        Ok(vec![char_row("   1.250")])
    );
    assert_eq!(
        invoke("sprintf", &[Value::from("%04d"), Value::Double(12.0)]),
        Ok(vec![Value::from("0012")])
    );
    assert_eq!(
        invoke(
            "sprintf",
            &[char_row("line\\n%+08.2f"), Value::Double(1.25),],
        ),
        Ok(vec![char_row("line\n+0001.25")])
    );
    assert_eq!(
        invoke(
            "sprintf",
            &[
                char_row("%d|%g|%d"),
                exact_integer_array([1, 1], vec![-2_i16]),
                single_array([1, 1], vec![1.25]),
                Value::Logical(true),
            ],
        ),
        Ok(vec![char_row("-2|1.25|1")])
    );
    assert_error(invoke("sprintf", &[]), BuiltinErrorCategory::ArgumentCount);
    assert_error(
        invoke("sprintf", &[char_row("%s"), Value::Nothing]),
        BuiltinErrorCategory::Type,
    );
}

#[test]
fn num2str_and_str2double_cover_classes_shapes_complex_specials_and_errors() {
    assert_eq!(
        invoke("num2str", &[Value::Double(std::f64::consts::PI)]),
        Ok(vec![char_row("3.1416")])
    );
    assert_eq!(
        invoke("num2str", &[real_array([1, 3], vec![1.0, 2.0, 3.0])],),
        Ok(vec![char_row("1  2  3")])
    );
    assert_eq!(
        invoke(
            "num2str",
            &[real_array([1, 2], vec![1.23, 0.0]), char_row("%.2f"),],
        ),
        Ok(vec![char_row("1.230.00")])
    );
    assert_eq!(
        invoke("num2str", &[Value::Complex(Complex64::new(1.0, -2.0))],),
        Ok(vec![char_row("1-2i")])
    );
    assert_eq!(
        invoke("num2str", &[real_array([0, 3], vec![])]),
        Ok(vec![char_array([0, 0], &[])])
    );
    assert_eq!(
        invoke("num2str", &[single_array([1, 1], vec![1.0 / 3.0])]),
        Ok(vec![char_row("0.3333333")])
    );
    assert_eq!(
        invoke("num2str", &[logical_array([1, 2], vec![true, false])],),
        Ok(vec![char_row("1  0")])
    );
    assert_eq!(
        invoke("num2str", &[exact_integer_array([1, 2], vec![-2_i16, 300])],),
        Ok(vec![char_row("-2  300")])
    );
    assert_eq!(
        invoke(
            "num2str",
            &[real_array([1, 2], vec![f64::INFINITY, f64::NAN])],
        ),
        Ok(vec![char_row("Inf  NaN")])
    );

    assert_eq!(
        invoke("str2double", &[char_row(" 1 + 2*i ")]),
        Ok(vec![Value::Complex(Complex64::new(1.0, 2.0))])
    );
    assert_eq!(
        invoke("str2double", &[char_row("0x10")]),
        Ok(vec![Value::Double(16.0)])
    );
    assert_eq!(
        invoke("str2double", &[char_row("1,234.5")]),
        Ok(vec![Value::Double(1234.5)])
    );
    assert!(matches!(
        invoke("str2double", &[char_row("not-a-number")]),
        Ok(values) if matches!(values.as_slice(), [Value::Double(value)] if value.is_nan())
    ));

    let parsed = only_output(invoke(
        "str2double",
        &[string_array([2, 2], &["1.2", "bad", "Inf", "1+2i"])],
    ));
    let Value::Array(ArrayData::ComplexF64(parsed)) = parsed else {
        panic!("mixed real and complex string input must produce a complex double array");
    };
    assert_eq!(parsed.shape().dimensions(), [2, 2]);
    assert_eq!(parsed.as_slice()[0], ArrayComplex64::new(1.2, 0.0));
    assert!(parsed.as_slice()[1].re.is_nan());
    assert_eq!(
        parsed.as_slice()[2],
        ArrayComplex64::new(f64::INFINITY, 0.0)
    );
    assert_eq!(parsed.as_slice()[3], ArrayComplex64::new(1.0, 2.0));

    assert_error(
        invoke("num2str", &[Value::from("text")]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("str2double", &[Value::Double(1.0)]),
        BuiltinErrorCategory::Type,
    );
}

#[test]
fn regexp_outputs_preserve_indices_captures_names_and_splits() {
    let outputs = invoke_with_outputs(
        "regexp",
        &[
            char_row("ab12 cd345"),
            char_row("(?<letters>[a-z]+)(?<digits>\\d+)"),
        ],
        7,
    )
    .unwrap();
    assert_eq!(outputs.len(), 7);
    assert_real_array(&outputs[0], &[1, 2], &[1.0, 6.0]);
    assert_real_array(&outputs[1], &[1, 2], &[4.0, 10.0]);

    let Value::Cell(extents) = &outputs[2] else {
        panic!("token extents must be one cell per match");
    };
    assert_eq!(extents.shape().dimensions(), [1, 2]);
    assert_real_array(
        extents.value_at_offset(0).unwrap(),
        &[2, 2],
        &[1.0, 3.0, 2.0, 4.0],
    );

    let Value::Cell(matches) = &outputs[3] else {
        panic!("matches must be a cell row");
    };
    assert_eq!(matches.values(), &[char_row("ab12"), char_row("cd345")]);

    let Value::Cell(tokens) = &outputs[4] else {
        panic!("tokens must be a nested cell row");
    };
    let Value::Cell(first_tokens) = tokens.value_at_offset(0).unwrap() else {
        panic!("each match token entry must be a cell row");
    };
    assert_eq!(first_tokens.values(), &[char_row("ab"), char_row("12")]);

    let Value::Struct(names) = &outputs[5] else {
        panic!("named captures must be a struct row");
    };
    assert_eq!(names.shape().dimensions(), [1, 2]);
    assert_eq!(
        names
            .field_names()
            .iter()
            .map(FieldName::as_str)
            .collect::<Vec<_>>(),
        ["letters", "digits"]
    );

    let Value::Cell(split) = &outputs[6] else {
        panic!("split output must be a cell row");
    };
    assert_eq!(
        split.values(),
        &[
            char_array([0, 0], &[]),
            char_row(" "),
            char_array([0, 0], &[]),
        ]
    );
}

#[test]
fn regexp_match_outputs_preserve_char_and_string_text_classes() {
    assert_eq!(
        invoke(
            "regexp",
            &[char_row("a1b22"), char_row("\\d+"), char_row("match")],
        ),
        Ok(vec![Value::Cell(
            CellArray::from_values(
                Shape::new([1, 2]).unwrap(),
                vec![char_row("1"), char_row("22")],
            )
            .unwrap(),
        )])
    );
    assert_eq!(
        invoke(
            "regexp",
            &[Value::from("a1b22"), char_row("\\d+"), char_row("match"),],
        ),
        Ok(vec![string_array([1, 2], &["1", "22"])])
    );

    let string_outputs = invoke_with_outputs(
        "regexp",
        &[
            Value::from("ab12 cd345"),
            char_row("(?<letters>[a-z]+)(?<digits>\\d+)"),
        ],
        7,
    )
    .unwrap();
    assert_eq!(string_outputs[3], string_array([1, 2], &["ab12", "cd345"]));
    let Value::Cell(string_tokens) = &string_outputs[4] else {
        panic!("string tokens must retain the outer cell row");
    };
    assert_eq!(
        string_tokens.value_at_offset(0),
        Some(&string_array([1, 2], &["ab", "12"]))
    );
    let Value::Struct(string_names) = &string_outputs[5] else {
        panic!("string named captures must be a struct row");
    };
    assert_eq!(
        string_names
            .field_values(0)
            .and_then(|values| values.first())
            .expect("first named capture must exist"),
        &Value::from("ab")
    );
    assert_eq!(string_outputs[6], string_array([1, 3], &["", " ", ""]));
}

#[test]
fn regexp_and_regexpi_cover_match_options_and_collection_inputs() {
    assert_eq!(
        invoke(
            "regexp",
            &[char_row("a1b22"), char_row("\\d+"), char_row("once")],
        ),
        Ok(vec![Value::Double(2.0)])
    );
    assert_real_array(
        &only_output(invoke(
            "regexp",
            &[char_row("abc"), char_row(""), char_row("emptymatch")],
        )),
        &[1, 4],
        &[1.0, 2.0, 3.0, 4.0],
    );
    assert_real_array(
        &only_output(invoke("regexpi", &[char_row("aA"), char_row("a")])),
        &[1, 2],
        &[1.0, 2.0],
    );

    let texts = Value::Cell(
        CellArray::from_values(
            Shape::new([1, 3]).unwrap(),
            vec![char_row("ab12"), char_row("none"), char_row("x34")],
        )
        .unwrap(),
    );
    let Value::Cell(indices) = only_output(invoke("regexp", &[texts, char_row("\\d+")])) else {
        panic!("cell input must preserve an outer cell shape");
    };
    assert_eq!(indices.shape().dimensions(), [1, 3]);
    assert_eq!(indices.value_at_offset(0), Some(&Value::Double(3.0)));
    assert_real_array(indices.value_at_offset(1).unwrap(), &[0, 0], &[]);
    assert_eq!(indices.value_at_offset(2), Some(&Value::Double(2.0)));
}

#[test]
fn regexprep_preserves_text_classes_shapes_capture_expansion_and_match_options() {
    assert_eq!(
        invoke(
            "regexprep",
            &[char_row("abc123def"), char_row("(\\d+)"), char_row("<$1>"),],
        ),
        Ok(vec![char_row("abc<123>def")])
    );
    assert_eq!(
        invoke(
            "regexprep",
            &[
                char_row("AaA"),
                char_row("a"),
                char_row("x"),
                char_row("ignorecase"),
            ],
        ),
        Ok(vec![char_row("xxx")])
    );
    assert_eq!(
        invoke(
            "regexprep",
            &[
                char_row("a1b22"),
                char_row("\\d+"),
                char_row("#"),
                char_row("once"),
            ],
        ),
        Ok(vec![char_row("a#b22")])
    );
    assert_eq!(
        invoke(
            "regexprep",
            &[
                char_row("abc"),
                char_row(""),
                char_row(":"),
                char_row("emptymatch"),
            ],
        ),
        Ok(vec![char_row(":a:b:c:")])
    );
    assert_eq!(
        invoke(
            "regexprep",
            &[
                string_array([2, 2], &["a1", "b22", "none", "c3"]),
                char_row("\\d+"),
                char_row("#"),
            ],
        ),
        Ok(vec![string_array([2, 2], &["a#", "b#", "none", "c#"])])
    );
    assert_error(
        invoke("regexp", &[Value::Double(1.0), char_row(".")]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("regexp", &[char_row("a"), char_row("(")]),
        BuiltinErrorCategory::Domain,
    );
}

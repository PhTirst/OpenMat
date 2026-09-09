use openmat_array::{ArrayData, DenseArray, IntegerArrayData, Shape};
use openmat_runtime::{BuiltinContext, BuiltinErrorCategory, CancellationToken, VecOutput};
use openmat_value::Value;

use super::*;

fn invoke(
    function: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
    arguments: &[Value],
    outputs: usize,
) -> BuiltinResult {
    let cancellation = CancellationToken::new();
    let mut sink = VecOutput::new();
    let mut context = BuiltinContext::new(outputs, &cancellation, &mut sink);
    function(arguments, &mut context)
}

fn real_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn integer_array<T: openmat_array::IntegerElement>(
    dimensions: impl IntoIterator<Item = u64>,
    values: Vec<T>,
) -> Value {
    Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    )))
}

fn only(result: BuiltinResult) -> Value {
    let mut values = result.unwrap();
    assert_eq!(values.len(), 1);
    values.remove(0)
}

#[test]
fn canonical_trailing_singletons_keep_ipermute_and_repelem_mappings_exact() {
    let input = real_array([2, 2], vec![1.0, 2.0, 3.0, 4.0]);
    let identity = only(invoke(
        ipermute_builtin,
        &[input, real_array([1, 3], vec![1.0, 2.0, 3.0])],
        1,
    ));
    let Value::Array(ArrayData::F64(identity)) = identity else {
        panic!("ipermute class");
    };
    assert_eq!(identity.shape().dimensions(), &[2, 2]);
    assert_eq!(identity.as_slice(), &[1.0, 2.0, 3.0, 4.0]);

    let repeated = only(invoke(
        repelem_builtin,
        &[
            real_array([2, 2, 2], (1..=8).map(f64::from).collect()),
            Value::Double(1.0),
            Value::Double(1.0),
            real_array([1, 2], vec![0.0, 1.0]),
        ],
        1,
    ));
    let Value::Array(ArrayData::F64(repeated)) = repeated else {
        panic!("repelem class");
    };
    assert_eq!(repeated.shape().dimensions(), &[2, 2]);
    assert_eq!(repeated.as_slice(), &[5.0, 6.0, 7.0, 8.0]);
}

#[test]
fn checked_repetition_allocation_and_cancelled_rearrangement_fail_without_mutation() {
    let input = real_array([1, 2], vec![1.0, 2.0]);
    let error = invoke(
        repelem_builtin,
        &[Value::Double(1.0), integer_array([1, 1], vec![u64::MAX])],
        1,
    )
    .unwrap_err();
    assert_eq!(error.category, BuiltinErrorCategory::Domain);

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut sink = VecOutput::new();
    let mut context = BuiltinContext::new(1, &cancellation, &mut sink);
    let error = rot90_builtin(std::slice::from_ref(&input), &mut context).unwrap_err();
    assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
    let Value::Array(ArrayData::F64(input)) = input else {
        unreachable!();
    };
    assert_eq!(input.as_slice(), &[1.0, 2.0]);
}

#[test]
fn nd_moving_dimension_and_cumulative_beyond_rank_preserve_shape_and_cow() {
    let input = real_array([2, 2, 2], (1..=8).map(f64::from).collect());
    let moved = only(invoke(
        movsum_builtin,
        &[input.clone(), Value::Double(3.0), Value::Double(3.0)],
        1,
    ));
    let Value::Array(ArrayData::F64(moved)) = moved else {
        panic!("movsum class");
    };
    assert_eq!(moved.shape().dimensions(), &[2, 2, 2]);
    assert_eq!(
        moved.as_slice(),
        &[6.0, 8.0, 10.0, 12.0, 6.0, 8.0, 10.0, 12.0]
    );

    let unchanged = only(invoke(
        cummin_builtin,
        &[input.clone(), Value::Double(5.0)],
        1,
    ));
    assert!(unchanged.shares_array_storage_with(&input));
}

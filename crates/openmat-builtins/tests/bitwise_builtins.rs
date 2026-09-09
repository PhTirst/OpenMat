#[path = "../src/core_bitwise.rs"]
mod core_bitwise;

use openmat_array::{ArrayData, CharCodeUnit, DenseArray, IntegerArrayData, IntegerElement, Shape};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinInvocationError, BuiltinResult,
    CancellationToken, NullOutput,
};
use openmat_value::Value;

fn registry() -> openmat_runtime::BuiltinRegistry {
    let mut registry = openmat_runtime::BuiltinRegistry::new();
    core_bitwise::register_bitwise(&mut registry).unwrap();
    registry
}

fn invoke(name: &str, arguments: &[Value]) -> BuiltinResult {
    let registry = registry();
    let handle = registry
        .handle_by_name(name)
        .expect("registered bitwise built-in");
    let cancellation = CancellationToken::new();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(1, &cancellation, &mut output);
    registry
        .invoke(handle, arguments, &mut context)
        .map_err(invocation_error)
}

fn invoke_with_outputs(name: &str, arguments: &[Value], outputs: usize) -> BuiltinResult {
    let registry = registry();
    let handle = registry
        .handle_by_name(name)
        .expect("registered bitwise built-in");
    let cancellation = CancellationToken::new();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(outputs, &cancellation, &mut output);
    registry
        .invoke(handle, arguments, &mut context)
        .map_err(invocation_error)
}

fn invocation_error(error: BuiltinInvocationError) -> BuiltinError {
    match error {
        BuiltinInvocationError::Failed { error, .. } => error,
        BuiltinInvocationError::UnknownHandle(_) => panic!("looked-up handle must resolve"),
    }
}

fn double_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn integer_array<T: IntegerElement>(
    dimensions: impl IntoIterator<Item = u64>,
    values: Vec<T>,
) -> Value {
    Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    )))
}

fn char_row(value: &str) -> Value {
    let values: Vec<_> = value.encode_utf16().map(CharCodeUnit::new).collect();
    Value::Array(ArrayData::Char(
        DenseArray::from_vec(
            Shape::new([1, u64::try_from(values.len()).unwrap()]).unwrap(),
            values,
        )
        .unwrap(),
    ))
}

fn only_output(result: BuiltinResult) -> Value {
    let mut values = result.unwrap();
    assert_eq!(values.len(), 1);
    values.remove(0)
}

fn assert_error(result: BuiltinResult, category: BuiltinErrorCategory) {
    assert_eq!(result.unwrap_err().category, category);
}

fn assert_double_array(value: &Value, dimensions: &[u64], expected: &[f64]) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected double array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice(), expected);
}

fn assert_integer_array<T: IntegerElement + std::fmt::Debug + PartialEq>(
    value: &Value,
    dimensions: &[u64],
    expected: &[T],
) {
    let Value::Array(ArrayData::Integer(integer)) = value else {
        panic!("expected integer array, received {value:?}");
    };
    let array = integer
        .as_typed::<T>()
        .expect("expected exact integer class");
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice(), expected);
}

#[test]
fn registers_the_complete_bitwise_family() {
    let registry = registry();
    for name in ["bitand", "bitor", "bitxor", "bitshift", "bitget", "bitset"] {
        assert!(registry.handle_by_name(name).is_some(), "missing `{name}`");
    }
}

#[test]
fn binary_operations_cover_double_logical_and_all_integer_classes() {
    assert_eq!(
        only_output(invoke("bitand", &[Value::Double(7.0), Value::Double(3.0)])),
        Value::Double(3.0)
    );
    assert_eq!(
        only_output(invoke(
            "bitor",
            &[Value::Logical(true), Value::Logical(false)]
        )),
        Value::Logical(true)
    );
    assert_eq!(
        only_output(invoke(
            "bitxor",
            &[Value::Logical(true), Value::Double(3.0)]
        )),
        Value::Double(2.0)
    );

    macro_rules! check_class {
        ($type:ty) => {{
            let left = integer_array([1, 3], vec![1 as $type, 2 as $type, 7 as $type]);
            let right = integer_array([1, 3], vec![3 as $type, 1 as $type, 4 as $type]);
            assert_integer_array::<$type>(
                &only_output(invoke("bitand", &[left.clone(), right.clone()])),
                &[1, 3],
                &[1 as $type, 0 as $type, 4 as $type],
            );
            assert_integer_array::<$type>(
                &only_output(invoke("bitor", &[left.clone(), right.clone()])),
                &[1, 3],
                &[3 as $type, 3 as $type, 7 as $type],
            );
            assert_integer_array::<$type>(
                &only_output(invoke("bitxor", &[left, right])),
                &[1, 3],
                &[2 as $type, 3 as $type, 3 as $type],
            );
        }};
    }
    check_class!(i8);
    check_class!(u8);
    check_class!(i16);
    check_class!(u16);
    check_class!(i32);
    check_class!(u32);
    check_class!(i64);
    check_class!(u64);
}

#[test]
fn shift_get_and_set_preserve_all_integer_classes() {
    macro_rules! check_class {
        ($type:ty) => {{
            let input = integer_array([1, 1], vec![3 as $type]);
            assert_integer_array::<$type>(
                &only_output(invoke("bitshift", &[input.clone(), Value::Double(1.0)])),
                &[1, 1],
                &[6 as $type],
            );
            assert_integer_array::<$type>(
                &only_output(invoke("bitget", &[input.clone(), Value::Double(2.0)])),
                &[1, 1],
                &[1 as $type],
            );
            assert_integer_array::<$type>(
                &only_output(invoke("bitset", &[input, Value::Double(3.0)])),
                &[1, 1],
                &[7 as $type],
            );
        }};
    }
    check_class!(i8);
    check_class!(u8);
    check_class!(i16);
    check_class!(u16);
    check_class!(i32);
    check_class!(u32);
    check_class!(i64);
    check_class!(u64);
}

#[test]
fn binary_operations_implicitly_expand_and_preserve_empty_shapes() {
    assert_double_array(
        &only_output(invoke(
            "bitxor",
            &[
                double_array([2, 1], vec![1.0, 2.0]),
                double_array([1, 3], vec![1.0, 2.0, 3.0]),
            ],
        )),
        &[2, 3],
        &[0.0, 3.0, 3.0, 0.0, 2.0, 1.0],
    );
    let left = integer_array([2, 1], vec![1_u8, 2]);
    let right = integer_array([1, 3], vec![1_u8, 2, 3]);
    assert_integer_array::<u8>(
        &only_output(invoke("bitand", &[left, right])),
        &[2, 3],
        &[1, 0, 0, 2, 1, 2],
    );

    let left = integer_array::<u8>([0, 1], Vec::new());
    let right = integer_array::<u8>([1, 3], vec![1, 2, 3]);
    assert_integer_array::<u8>(&only_output(invoke("bitor", &[left, right])), &[0, 3], &[]);
}

#[test]
fn integer_first_operand_accepts_only_an_in_range_scalar_double_exception() {
    let input = integer_array([1, 2], vec![-1_i8, 7]);
    assert_integer_array::<i8>(
        &only_output(invoke("bitand", &[input.clone(), Value::Double(3.0)])),
        &[1, 2],
        &[3, 3],
    );
    assert_error(
        invoke("bitand", &[input.clone(), Value::Double(128.0)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("bitand", &[Value::Double(3.0), input]),
        BuiltinErrorCategory::Type,
    );
}

#[test]
fn shifts_use_fixed_width_wrap_and_arithmetic_signed_right_shift() {
    let signed = integer_array([1, 7], vec![-128_i8, -3, -1, 0, 1, 64, 127]);
    assert_integer_array::<i8>(
        &only_output(invoke("bitshift", &[signed.clone(), Value::Double(-1.0)])),
        &[1, 7],
        &[-64, -2, -1, 0, 0, 32, 63],
    );
    assert_integer_array::<i8>(
        &only_output(invoke("bitshift", &[signed, Value::Double(1.0)])),
        &[1, 7],
        &[0, -6, -2, 0, 2, -128, -2],
    );
    assert_integer_array::<u8>(
        &only_output(invoke(
            "bitshift",
            &[
                integer_array([1, 3], vec![1_u8, 128, 255]),
                double_array([1, 3], vec![-100.0, -7.0, 8.0]),
            ],
        )),
        &[1, 3],
        &[0, 1, 0],
    );
}

#[test]
fn bit_positions_and_set_values_expand_scalars_without_implicit_expansion() {
    assert_integer_array::<u8>(
        &only_output(invoke(
            "bitget",
            &[
                integer_array([1, 1], vec![3_u8]),
                double_array([1, 3], vec![1.0, 2.0, 3.0]),
            ],
        )),
        &[1, 3],
        &[1, 1, 0],
    );
    assert_integer_array::<u8>(
        &only_output(invoke(
            "bitset",
            &[
                integer_array([1, 1], vec![0_u8]),
                double_array([1, 3], vec![1.0, 2.0, 3.0]),
                double_array([1, 3], vec![0.0, f64::NAN, -1.0]),
            ],
        )),
        &[1, 3],
        &[0, 2, 4],
    );

    let column = integer_array([2, 1], vec![1_u8, 2]);
    let row = double_array([1, 3], vec![1.0, 2.0, 3.0]);
    for name in ["bitshift", "bitget"] {
        assert_error(
            invoke(name, &[column.clone(), row.clone()]),
            BuiltinErrorCategory::Domain,
        );
    }
    assert_error(
        invoke("bitset", &[column, row]),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn double_uses_64_bits_and_assumed_types_change_width_and_signedness() {
    assert_eq!(
        only_output(invoke(
            "bitget",
            &[Value::Double(2_f64.powi(63)), Value::Double(64.0)]
        )),
        Value::Double(1.0)
    );
    assert_eq!(
        only_output(invoke(
            "bitset",
            &[
                Value::Double(-1.0),
                Value::Double(8.0),
                Value::Double(0.0),
                char_row("int8"),
            ],
        )),
        Value::Double(127.0)
    );
    assert_eq!(
        only_output(invoke(
            "bitshift",
            &[Value::Double(255.0), Value::Double(1.0), char_row("uint8")],
        )),
        Value::Double(254.0)
    );
    assert_eq!(
        only_output(invoke(
            "bitand",
            &[Value::Double(3.0), Value::Double(1.0), Value::from("uint8"),],
        )),
        Value::Double(1.0)
    );
    assert_error(
        invoke(
            "bitget",
            &[Value::Double(1.0), Value::Double(9.0), char_row("uint8")],
        ),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn all_functions_preserve_zero_by_n_shapes() {
    let empty = integer_array::<u16>([0, 3], Vec::new());
    for name in ["bitshift", "bitget", "bitset"] {
        assert_integer_array::<u16>(
            &only_output(invoke(name, &[empty.clone(), Value::Double(1.0)])),
            &[0, 3],
            &[],
        );
    }
}

#[test]
fn invalid_types_domains_counts_and_output_counts_are_classified() {
    assert_error(
        invoke("bitand", &[Value::Double(-1.0), Value::Double(1.0)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("bitshift", &[Value::Logical(true), Value::Double(1.0)]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("bitshift", &[Value::Double(1.0), Value::Double(1.5)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "bitget",
            &[integer_array([1, 1], vec![1_u8]), Value::Double(0.0)],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "bitget",
            &[integer_array([1, 1], vec![1_u8]), Value::Double(9.0)],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "bitset",
            &[integer_array([1, 1], vec![0_u8]), Value::Logical(true)],
        ),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke(
            "bitand",
            &[
                integer_array([1, 1], vec![1_u8]),
                integer_array([1, 1], vec![1_u16]),
            ],
        ),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("bitor", &[Value::Double(1.0)]),
        BuiltinErrorCategory::ArgumentCount,
    );
    assert_error(
        invoke_with_outputs("bitxor", &[Value::Double(1.0), Value::Double(1.0)], 2),
        BuiltinErrorCategory::ArgumentCount,
    );

    let empty = integer_array::<u8>([0, 3], Vec::new());
    assert_error(
        invoke("bitand", &[empty.clone(), Value::Double(256.0)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("bitshift", &[empty.clone(), Value::Double(1.5)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("bitget", &[empty.clone(), Value::Double(0.0)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke("bitset", &[empty, Value::Double(9.0)]),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn cancellation_is_checked_before_mapping() {
    let registry = registry();
    let handle = registry.handle_by_name("bitand").unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(1, &cancellation, &mut output);
    let error = registry
        .invoke(
            handle,
            &[Value::Double(1.0), Value::Double(1.0)],
            &mut context,
        )
        .unwrap_err();
    let BuiltinInvocationError::Failed { error, .. } = error else {
        panic!("registered handle must resolve");
    };
    assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
}

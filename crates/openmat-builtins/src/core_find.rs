use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, ComplexInteger,
    DenseArray, IntegerArrayData, IntegerElement, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{StringValue, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, expect_argument_count_range, expect_max_outputs,
    positive_integer_dimension, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy)]
enum Direction {
    First,
    Last,
}

pub(super) fn find_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if let Some(result) = crate::core_sparse::try_find_sparse_builtin(arguments, context) {
        return result;
    }
    expect_argument_count_range("find", arguments, 1, 3)?;
    expect_max_outputs("find", context, 3)?;
    context.check_cancelled()?;
    let dimensions = arguments[0].dimensions().ok_or_else(|| {
        type_error(
            "find",
            1,
            "numeric, logical, char, or integer array",
            &arguments[0],
        )
    })?;
    let maximum = arguments.get(1).map(positive_find_count).transpose()?;
    let direction = arguments
        .get(2)
        .map_or(Ok(Direction::First), find_direction)?;
    let mut offsets = nonzero_offsets(&arguments[0], context)?;
    if let Some(maximum) = maximum
        && offsets.len() > maximum
    {
        match direction {
            Direction::First => offsets.truncate(maximum),
            Direction::Last => {
                let first = offsets.len() - maximum;
                offsets.drain(..first);
            }
        }
    }
    context.check_cancelled()?;
    let row_output = dimensions.len() == 2 && dimensions[0] == 1;
    let shape = index_shape(row_output, offsets.len())?;
    let requested_outputs = context.requested_outputs();
    if requested_outputs == 0 {
        return Ok(Vec::new());
    }
    let mut outputs = reserved_outputs(requested_outputs)?;
    if requested_outputs == 1 {
        outputs.push(linear_indices(&offsets, shape, context)?);
        return Ok(outputs);
    }

    let rows = dimensions[0];
    outputs.push(subscript_indices(
        "find row indices",
        &offsets,
        shape.clone(),
        context,
        |offset| {
            if rows == 0 { 1 } else { offset % rows + 1 }
        },
    )?);
    outputs.push(subscript_indices(
        "find column indices",
        &offsets,
        shape.clone(),
        context,
        |offset| {
            if rows == 0 { 1 } else { offset / rows + 1 }
        },
    )?);
    if requested_outputs == 3 {
        outputs.push(selected_values(&arguments[0], &offsets, shape, context)?);
    }
    context.check_cancelled()?;
    Ok(outputs)
}

fn positive_find_count(value: &Value) -> Result<usize, BuiltinError> {
    let count = if let Some(value) = value.as_real_single() {
        let value = f64::from(value);
        if !value.is_finite()
            || value < 1.0
            || value.fract() != 0.0
            || value >= U64_EXCLUSIVE_UPPER_BOUND
        {
            return Err(find_count_error());
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            value as u64
        }
    } else {
        positive_integer_dimension("find", 2, value)?
    };
    Ok(usize::try_from(count).unwrap_or(usize::MAX))
}

fn find_count_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "input 2 to `find` must be a positive integer scalar",
    )
}

fn find_direction(value: &Value) -> Result<Direction, BuiltinError> {
    let code_units: Vec<u16> = match value {
        Value::String(StringValue::Scalar(value)) if !value.is_missing() => {
            value.code_units().to_vec()
        }
        Value::String(StringValue::Array(value)) if value.numel() == 1 => value
            .as_slice()
            .first()
            .filter(|value| !value.is_missing())
            .map(|value| value.code_units().to_vec())
            .ok_or_else(direction_error)?,
        Value::Array(ArrayData::Char(value))
            if value.shape().ndims() == 2 && value.shape().extent(0) == 1 =>
        {
            value.as_slice().iter().map(|value| value.get()).collect()
        }
        value => {
            return Err(type_error(
                "find",
                3,
                "'first' or 'last' char row or string scalar",
                value,
            ));
        }
    };
    match code_units.as_slice() {
        [102, 105, 114, 115, 116] => Ok(Direction::First),
        [108, 97, 115, 116] => Ok(Direction::Last),
        _ => Err(direction_error()),
    }
}

fn direction_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "input 3 to `find` must be 'first' or 'last'",
    )
}

fn nonzero_offsets(
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<usize>, BuiltinError> {
    let length = value
        .numel()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| type_error("find", 1, "numeric, logical, char, or integer array", value))?;
    let mut offsets = Vec::new();
    offsets.try_reserve(length.min(1_024)).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`find` cannot allocate its index buffer",
        )
    })?;
    macro_rules! scan_slice {
        ($slice:expr, $nonzero:expr) => {{
            for (index, element) in $slice.iter().enumerate() {
                check_cancelled_at(context, index)?;
                if $nonzero(element) {
                    offsets.try_reserve(1).map_err(|_| {
                        BuiltinError::new(
                            BuiltinErrorCategory::Domain,
                            "`find` cannot grow its index buffer",
                        )
                    })?;
                    offsets.push(index);
                }
            }
        }};
    }
    match value {
        Value::Logical(value) => {
            if *value {
                offsets.push(0);
            }
        }
        Value::Double(value) => {
            if *value != 0.0 {
                offsets.push(0);
            }
        }
        Value::Complex(value) => {
            if value.real != 0.0 || value.imaginary != 0.0 {
                offsets.push(0);
            }
        }
        Value::Array(ArrayData::F32(array)) => {
            scan_slice!(array.as_slice(), |value: &f32| *value != 0.0);
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            scan_slice!(array.as_slice(), |value: &ArrayComplex32| value.re != 0.0
                || value.im != 0.0);
        }
        Value::Array(ArrayData::Logical(array)) => {
            scan_slice!(array.as_slice(), |value: &Logical| value.get());
        }
        Value::Array(ArrayData::F64(array)) => {
            scan_slice!(array.as_slice(), |value: &f64| *value != 0.0);
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            scan_slice!(array.as_slice(), |value: &ArrayComplex64| value.re != 0.0
                || value.im != 0.0);
        }
        Value::Array(ArrayData::Char(array)) => {
            scan_slice!(
                array.as_slice(),
                |value: &openmat_array::CharCodeUnit| value.get() != 0
            );
        }
        Value::Array(ArrayData::Integer(array)) => {
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                if !value.real_component().is_zero()
                    || value
                        .imaginary_component()
                        .is_some_and(|value| !value.is_zero())
                {
                    offsets.try_reserve(1).map_err(|_| {
                        BuiltinError::new(
                            BuiltinErrorCategory::Domain,
                            "`find` cannot grow its index buffer",
                        )
                    })?;
                    offsets.push(index);
                }
            }
        }
        value => {
            return Err(type_error(
                "find",
                1,
                "numeric, logical, char, or integer array",
                value,
            ));
        }
    }
    Ok(offsets)
}

fn index_shape(row: bool, length: usize) -> Result<Shape, BuiltinError> {
    let length = u64::try_from(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`find` result length does not fit the runtime shape model",
        )
    })?;
    Shape::new(if row { [1, length] } else { [length, 1] }).map_err(|error| array_error(&error))
}

fn linear_indices(
    offsets: &[usize],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    subscript_indices("find linear indices", offsets, shape, context, |offset| {
        offset + 1
    })
}

fn subscript_indices<F>(
    name: &str,
    offsets: &[usize],
    shape: Shape,
    context: &BuiltinContext<'_>,
    mut index: F,
) -> Result<Value, BuiltinError>
where
    F: FnMut(u64) -> u64,
{
    let mut values = reserved_values(name, offsets.len())?;
    for (position, offset) in offsets.iter().copied().enumerate() {
        check_cancelled_at(context, position)?;
        let offset = u64::try_from(offset).map_err(|_| index_error())?;
        #[allow(clippy::cast_precision_loss)]
        values.push(index(offset) as f64);
    }
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn selected_values(
    value: &Value,
    offsets: &[usize],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! selected_array {
        ($array:expr, $wrap:expr) => {{
            let output = select_dense($array, offsets, shape, context)?;
            $wrap(output)
        }};
    }
    match value {
        Value::Logical(value) => scalar_selected(Logical::from(*value), offsets, shape, context)
            .map(ArrayData::Logical)
            .map(Value::Array),
        Value::Double(value) => scalar_selected(*value, offsets, shape, context)
            .map(ArrayData::F64)
            .map(Value::Array),
        Value::Complex(value) => {
            let output = scalar_selected(
                ArrayComplex64::new(value.real, value.imaginary),
                offsets,
                shape,
                context,
            )?;
            complex_f64_output(output)
        }
        Value::Array(ArrayData::F32(array)) => Ok(selected_array!(array, |array| {
            Value::Array(ArrayData::F32(array))
        })),
        Value::Array(ArrayData::ComplexF32(array)) => {
            complex_f32_output(select_dense(array, offsets, shape, context)?)
        }
        Value::Array(ArrayData::Logical(array)) => Ok(selected_array!(array, |array| {
            Value::Array(ArrayData::Logical(array))
        })),
        Value::Array(ArrayData::F64(array)) => Ok(selected_array!(array, |array| {
            Value::Array(ArrayData::F64(array))
        })),
        Value::Array(ArrayData::ComplexF64(array)) => {
            complex_f64_output(select_dense(array, offsets, shape, context)?)
        }
        Value::Array(ArrayData::Char(array)) => Ok(selected_array!(array, |array| {
            Value::Array(ArrayData::Char(array))
        })),
        Value::Array(ArrayData::Integer(array)) => selected_integer(array, offsets, shape, context)
            .map(ArrayData::Integer)
            .map(Value::Array),
        value => Err(type_error(
            "find",
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

fn scalar_selected<T: Clone>(
    value: T,
    offsets: &[usize],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let source = DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map_err(|error| array_error(&error))?;
    select_dense(&source, offsets, shape, context)
}

fn select_dense<T: Clone>(
    input: &DenseArray<T>,
    offsets: &[usize],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let mut values = reserved_values("find values", offsets.len())?;
    for (position, offset) in offsets.iter().copied().enumerate() {
        check_cancelled_at(context, position)?;
        values.push(
            input
                .as_slice()
                .get(offset)
                .ok_or_else(index_error)?
                .clone(),
        );
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn selected_integer(
    input: &IntegerArrayData,
    offsets: &[usize],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! real_variant {
        ($array:expr) => {
            select_dense($array, offsets, shape, context).map(IntegerArrayData::from_typed)
        };
    }
    macro_rules! complex_variant {
        ($array:expr) => {
            selected_complex_integer($array, offsets, shape, context)
        };
    }
    match input {
        IntegerArrayData::I8(array) => real_variant!(array),
        IntegerArrayData::ComplexI8(array) => complex_variant!(array),
        IntegerArrayData::U8(array) => real_variant!(array),
        IntegerArrayData::ComplexU8(array) => complex_variant!(array),
        IntegerArrayData::I16(array) => real_variant!(array),
        IntegerArrayData::ComplexI16(array) => complex_variant!(array),
        IntegerArrayData::U16(array) => real_variant!(array),
        IntegerArrayData::ComplexU16(array) => complex_variant!(array),
        IntegerArrayData::I32(array) => real_variant!(array),
        IntegerArrayData::ComplexI32(array) => complex_variant!(array),
        IntegerArrayData::U32(array) => real_variant!(array),
        IntegerArrayData::ComplexU32(array) => complex_variant!(array),
        IntegerArrayData::I64(array) => real_variant!(array),
        IntegerArrayData::ComplexI64(array) => complex_variant!(array),
        IntegerArrayData::U64(array) => real_variant!(array),
        IntegerArrayData::ComplexU64(array) => complex_variant!(array),
    }
}

fn selected_complex_integer<T>(
    input: &DenseArray<ComplexInteger<T>>,
    offsets: &[usize],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError>
where
    T: IntegerElement + Copy + Default + PartialEq,
    ComplexInteger<T>: IntegerElement,
{
    let output = select_dense(input, offsets, shape, context)?;
    if output
        .as_slice()
        .iter()
        .all(|value| value.im() == T::default())
    {
        let shape = output.shape().clone();
        let values = output.as_slice().iter().map(|value| value.re()).collect();
        DenseArray::from_vec(shape, values)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))
    } else {
        Ok(IntegerArrayData::from_typed(output))
    }
}

fn complex_f64_output(input: DenseArray<ArrayComplex64>) -> Result<Value, BuiltinError> {
    if input.as_slice().iter().all(|value| value.im == 0.0) {
        let shape = input.shape().clone();
        let values = input.as_slice().iter().map(|value| value.re).collect();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        Ok(Value::Array(ArrayData::ComplexF64(input)))
    }
}

fn complex_f32_output(input: DenseArray<ArrayComplex32>) -> Result<Value, BuiltinError> {
    if input.as_slice().iter().all(|value| value.im == 0.0) {
        let shape = input.shape().clone();
        let values = input.as_slice().iter().map(|value| value.re).collect();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        Ok(Value::Array(ArrayData::ComplexF32(input)))
    }
}

fn reserved_values<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {length} elements"),
        )
    })?;
    Ok(values)
}

fn reserved_outputs(length: usize) -> Result<Vec<Value>, BuiltinError> {
    reserved_values("find outputs", length)
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn index_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`find` computed an invalid checked column-major index",
    )
}

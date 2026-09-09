use openmat_array::{ArrayData, DenseArray, IntegerArrayData, IntegerComponent, Logical, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{CellArray, ObjectArray, StringArray, StringValue, StructArray, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count,
    expect_argument_count_range, expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

pub(super) fn flip_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("flip", arguments, 1, 2)?;
    expect_max_outputs("flip", context, 1)?;
    context.check_cancelled()?;
    let dimensions = array_dimensions("flip", &arguments[0])?;
    let axis = arguments.get(1).map_or_else(
        || Ok(default_axis(dimensions)),
        |value| dimension("flip", 2, value),
    )?;
    let offsets = flipped_offsets("flip", dimensions, axis, context)?;
    let output = reorder_value(
        "flip",
        &arguments[0],
        Shape::new(dimensions.iter().copied()).map_err(|error| array_error(&error))?,
        &offsets,
        context,
    )?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn fliplr_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    fixed_flip("fliplr", 2, arguments, context)
}

pub(super) fn flipud_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    fixed_flip("flipud", 1, arguments, context)
}

fn fixed_flip(
    name: &str,
    axis: u64,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = array_dimensions(name, &arguments[0])?;
    let offsets = flipped_offsets(name, dimensions, axis, context)?;
    let output = reorder_value(
        name,
        &arguments[0],
        Shape::new(dimensions.iter().copied()).map_err(|error| array_error(&error))?,
        &offsets,
        context,
    )?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn circshift_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("circshift", arguments, 2, 3)?;
    expect_max_outputs("circshift", context, 1)?;
    context.check_cancelled()?;
    let dimensions = array_dimensions("circshift", &arguments[0])?;
    let supplied = shift_vector(&arguments[1], context)?;
    if supplied.is_empty() {
        return Err(shift_error());
    }
    let rank = dimensions.len().max(supplied.len());
    let mut shifts = vec![0_i128; rank];
    if supplied.len() == 1 {
        let axis = arguments.get(2).map_or_else(
            || Ok(default_axis(dimensions)),
            |value| dimension("circshift", 3, value),
        )?;
        let axis = usize::try_from(axis - 1).map_err(|_| shift_error())?;
        if axis >= shifts.len() {
            shifts.resize(axis + 1, 0);
        }
        shifts[axis] = supplied[0];
    } else {
        if arguments.len() == 3 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::ArgumentCount,
                "the dimension form of `circshift` requires a scalar shift",
            ));
        }
        shifts[..supplied.len()].copy_from_slice(&supplied);
    }
    let offsets = shifted_offsets(dimensions, &shifts, context)?;
    let output = reorder_value(
        "circshift",
        &arguments[0],
        Shape::new(dimensions.iter().copied()).map_err(|error| array_error(&error))?,
        &offsets,
        context,
    )?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn fftshift_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    frequency_shift("fftshift", arguments, context, false)
}

pub(super) fn ifftshift_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    frequency_shift("ifftshift", arguments, context, true)
}

fn frequency_shift(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    inverse: bool,
) -> BuiltinResult {
    expect_argument_count_range(name, arguments, 1, 2)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = array_dimensions(name, &arguments[0])?;
    let mut shifts = vec![0_i128; dimensions.len()];
    if let Some(value) = arguments.get(1) {
        let axis = dimension(name, 2, value)?;
        let axis = usize::try_from(axis - 1).map_err(|_| mapping_error(name))?;
        if shifts.len() <= axis {
            shifts.resize(axis + 1, 0);
        }
        let extent = dimensions.get(axis).copied().unwrap_or(1);
        shifts[axis] = frequency_shift_amount(extent, inverse);
    } else {
        for (shift, extent) in shifts.iter_mut().zip(dimensions.iter().copied()) {
            *shift = frequency_shift_amount(extent, inverse);
        }
    }
    let offsets = shifted_offsets(dimensions, &shifts, context)?;
    let shape = Shape::new(dimensions.iter().copied()).map_err(|error| array_error(&error))?;
    let output = reorder_value(name, &arguments[0], shape, &offsets, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn frequency_shift_amount(extent: u64, inverse: bool) -> i128 {
    let half = i128::from(extent / 2);
    if inverse { -half } else { half }
}

pub(super) fn ndgrid_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "built-in `ndgrid` expects at least 1 input but received 0",
        ));
    }
    if context.requested_outputs() > arguments.len() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`ndgrid` cannot return more outputs than inputs",
        ));
    }
    context.check_cancelled()?;
    let mut extents = Vec::new();
    extents
        .try_reserve_exact(arguments.len())
        .map_err(|_| mapping_error("ndgrid"))?;
    for (index, argument) in arguments.iter().enumerate() {
        check_cancelled_at(context, index)?;
        let dimensions = array_dimensions("ndgrid", argument)?;
        if argument.numel() != Some(0)
            && !(dimensions.len() == 2 && (dimensions[0] == 1 || dimensions[1] == 1))
        {
            return Err(type_error(
                "ndgrid",
                index + 1,
                "vector or empty array",
                argument,
            ));
        }
        extents.push(argument.numel().unwrap_or(0));
    }
    if extents.len() == 1 {
        extents.push(1);
    }
    let shape = Shape::new(extents.iter().copied()).map_err(|error| array_error(&error))?;
    let output_length = host_length("ndgrid", shape.numel())?;
    let mut outputs = Vec::new();
    outputs
        .try_reserve_exact(context.requested_outputs())
        .map_err(|_| mapping_error("ndgrid"))?;
    for (axis, argument) in arguments
        .iter()
        .take(context.requested_outputs())
        .enumerate()
    {
        let stride = shape
            .dimensions()
            .iter()
            .take(axis)
            .try_fold(1_u64, |value, extent| value.checked_mul(*extent))
            .ok_or_else(|| mapping_error("ndgrid"))?;
        let extent = argument.numel().unwrap_or(0);
        let mut offsets = Vec::new();
        offsets
            .try_reserve_exact(output_length)
            .map_err(|_| mapping_error("ndgrid"))?;
        for output_offset in 0..output_length {
            check_cancelled_at(context, output_offset)?;
            let output_offset =
                u64::try_from(output_offset).map_err(|_| mapping_error("ndgrid"))?;
            let coordinate = if extent == 0 {
                0
            } else {
                (output_offset / stride) % extent
            };
            offsets.push(usize::try_from(coordinate).map_err(|_| mapping_error("ndgrid"))?);
        }
        outputs.push(reorder_value(
            "ndgrid",
            argument,
            shape.clone(),
            &offsets,
            context,
        )?);
    }
    context.check_cancelled()?;
    Ok(outputs)
}

fn array_dimensions<'a>(name: &str, value: &'a Value) -> Result<&'a [u64], BuiltinError> {
    value
        .dimensions()
        .ok_or_else(|| type_error(name, 1, "array value", value))
}

fn default_axis(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|axis| u64::try_from(axis + 1).ok())
        .unwrap_or(1)
}

fn dimension(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        let value = match value {
            IntegerComponent::Signed(value) => u64::try_from(value).ok(),
            IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
        };
        return value.filter(|value| *value > 0).ok_or_else(shift_error);
    }
    let value = match value {
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            array.as_slice().first().copied()
        }
        _ => value.as_real_single().map(f64::from),
    }
    .ok_or_else(|| type_error(name, position, "positive integer dimension", value))?;
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(shift_error());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn shift_vector(value: &Value, context: &BuiltinContext<'_>) -> Result<Vec<i128>, BuiltinError> {
    let mut output = Vec::new();
    let mut push_float = |value: f64| -> Result<(), BuiltinError> {
        if !value.is_finite()
            || value.fract() != 0.0
            || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value)
        {
            return Err(shift_error());
        }
        #[allow(clippy::cast_possible_truncation)]
        output.push(i128::from(value as i64));
        Ok(())
    };
    match value {
        Value::Double(value) => push_float(*value)?,
        Value::Array(ArrayData::F64(array)) if is_vector_or_empty(array.shape()) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                push_float(*value)?;
            }
        }
        Value::Array(ArrayData::F32(array)) if is_vector_or_empty(array.shape()) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                push_float(f64::from(*value))?;
            }
        }
        Value::Array(ArrayData::Integer(array)) if is_vector_or_empty(array.shape()) => {
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                if value.is_complex() {
                    return Err(shift_error());
                }
                let component = value.real_component();
                let shift = if let Some(value) = component.as_signed() {
                    value
                } else {
                    i128::try_from(component.as_unsigned().ok_or_else(shift_error)?)
                        .map_err(|_| shift_error())?
                };
                output.push(shift);
            }
        }
        _ => return Err(shift_error()),
    }
    Ok(output)
}

fn is_vector_or_empty(shape: &Shape) -> bool {
    shape.numel() == 0 || shape.ndims() == 2 && (shape.extent(0) == 1 || shape.extent(1) == 1)
}

fn flipped_offsets(
    name: &str,
    dimensions: &[u64],
    axis: u64,
    context: &BuiltinContext<'_>,
) -> Result<Vec<usize>, BuiltinError> {
    let axis = usize::try_from(axis - 1).map_err(|_| mapping_error(name))?;
    mapped_offsets(name, dimensions, context, |coordinates| {
        if let Some(coordinate) = coordinates.get_mut(axis) {
            let extent = dimensions[axis];
            if extent > 0 {
                *coordinate = extent - 1 - *coordinate;
            }
        }
        Ok(())
    })
}

fn shifted_offsets(
    dimensions: &[u64],
    shifts: &[i128],
    context: &BuiltinContext<'_>,
) -> Result<Vec<usize>, BuiltinError> {
    mapped_offsets("circshift", dimensions, context, |coordinates| {
        for (axis, coordinate) in coordinates.iter_mut().enumerate() {
            let extent = i128::from(dimensions[axis]);
            if extent > 0 {
                let shift = shifts.get(axis).copied().unwrap_or(0).rem_euclid(extent);
                *coordinate = u64::try_from((i128::from(*coordinate) - shift).rem_euclid(extent))
                    .map_err(|_| mapping_error("circshift"))?;
            }
        }
        Ok(())
    })
}

fn mapped_offsets(
    name: &str,
    dimensions: &[u64],
    context: &BuiltinContext<'_>,
    mut transform: impl FnMut(&mut [u64]) -> Result<(), BuiltinError>,
) -> Result<Vec<usize>, BuiltinError> {
    let shape = Shape::new(dimensions.iter().copied()).map_err(|error| array_error(&error))?;
    let length = host_length(name, shape.numel())?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| mapping_error(name))?;
    for offset in 0..length {
        check_cancelled_at(context, offset)?;
        let mut remainder = u64::try_from(offset).map_err(|_| mapping_error(name))?;
        let mut coordinates = Vec::with_capacity(dimensions.len());
        for extent in dimensions.iter().copied() {
            coordinates.push(if extent == 0 { 0 } else { remainder % extent });
            if extent != 0 {
                remainder /= extent;
            }
        }
        transform(&mut coordinates)?;
        let mut source = 0_u64;
        let mut stride = 1_u64;
        for (coordinate, extent) in coordinates.iter().zip(dimensions) {
            source = coordinate
                .checked_mul(stride)
                .and_then(|value| source.checked_add(value))
                .ok_or_else(|| mapping_error(name))?;
            stride = stride
                .checked_mul(*extent)
                .ok_or_else(|| mapping_error(name))?;
        }
        output.push(usize::try_from(source).map_err(|_| mapping_error(name))?);
    }
    Ok(output)
}

fn reorder_value(
    name: &str,
    input: &Value,
    shape: Shape,
    offsets: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! reorder_dense_value {
        ($array:expr, $wrap:expr) => {{
            let array = reorder_dense(name, $array, shape, offsets, context)?;
            Ok($wrap(array))
        }};
    }
    if shape.numel() == 1 && input.is_scalar() {
        return Ok(input.clone());
    }
    match input {
        Value::Logical(value) => scalar_dense(name, Logical::from(*value), shape, offsets, context)
            .map(ArrayData::Logical)
            .map(Value::Array),
        Value::Double(value) => scalar_dense(name, *value, shape, offsets, context)
            .map(ArrayData::F64)
            .map(Value::Array),
        Value::Complex(value) => scalar_dense(
            name,
            openmat_array::Complex64::new(value.real, value.imaginary),
            shape,
            offsets,
            context,
        )
        .map(ArrayData::ComplexF64)
        .map(Value::Array),
        Value::Array(ArrayData::F32(array)) => {
            reorder_dense_value!(array, |array| { Value::Array(ArrayData::F32(array)) })
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::ComplexF32(array)))
        }
        Value::Array(ArrayData::Logical(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Array(ArrayData::F64(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => reorder_dense_value!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Value::Array(ArrayData::Char(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::Char(array)))
        }
        Value::Array(ArrayData::Integer(array)) => {
            reorder_integer(name, array, shape, offsets, context)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        Value::String(value) => {
            let elements = reorder_slice(name, value.numel(), offsets, context, |index| {
                value.element(index).cloned()
            })?;
            StringArray::from_elements(shape, elements)
                .map(StringValue::Array)
                .map(Value::String)
                .map_err(|error| array_error(&error))
        }
        Value::Cell(value) => {
            let elements = reorder_slice(name, value.numel(), offsets, context, |index| {
                value.value_at_offset(index).cloned()
            })?;
            CellArray::from_values(shape, elements)
                .map(Value::Cell)
                .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))
        }
        Value::Struct(value) => reorder_struct(name, value, shape, offsets, context),
        Value::ObjectArray(value) => {
            let handles = reorder_slice(name, value.numel(), offsets, context, |index| {
                value.as_slice().get(index).copied()
            })?;
            ObjectArray::from_vec(value.class_handle(), shape, handles)
                .map(Value::ObjectArray)
                .map_err(|error| array_error(&error))
        }
        Value::GraphicsArray(value) if shape.dimensions().len() == 2 && shape.extent(1) == 1 => {
            let handles = reorder_slice(name, value.numel(), offsets, context, |index| {
                value.as_slice().get(index).copied()
            })?;
            openmat_value::GraphicsHandleArray::column(value.class(), handles)
                .map(Value::GraphicsArray)
                .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))
        }
        Value::Object(_) | Value::Graphics(_) if shape.numel() == 1 => Ok(input.clone()),
        value => Err(type_error(name, 1, "reorderable array", value)),
    }
}

fn scalar_dense<T: Clone>(
    name: &str,
    value: T,
    shape: Shape,
    offsets: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let input = DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map_err(|error| array_error(&error))?;
    reorder_dense(name, &input, shape, offsets, context)
}

fn reorder_dense<T: Clone>(
    name: &str,
    input: &DenseArray<T>,
    shape: Shape,
    offsets: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let values = reorder_slice(name, input.numel(), offsets, context, |index| {
        input.as_slice().get(index).cloned()
    })?;
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn reorder_slice<T>(
    name: &str,
    input_length: u64,
    offsets: &[usize],
    context: &BuiltinContext<'_>,
    mut value_at: impl FnMut(usize) -> Option<T>,
) -> Result<Vec<T>, BuiltinError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(offsets.len())
        .map_err(|_| mapping_error(name))?;
    for (index, offset) in offsets.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        output.push(value_at(offset).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` mapped offset {offset} outside {input_length} elements"),
            )
        })?);
    }
    Ok(output)
}

fn reorder_integer(
    name: &str,
    input: &IntegerArrayData,
    shape: Shape,
    offsets: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! variant {
        ($array:expr) => {
            reorder_dense(name, $array, shape, offsets, context).map(IntegerArrayData::from_typed)
        };
    }
    match input {
        IntegerArrayData::I8(array) => variant!(array),
        IntegerArrayData::ComplexI8(array) => variant!(array),
        IntegerArrayData::U8(array) => variant!(array),
        IntegerArrayData::ComplexU8(array) => variant!(array),
        IntegerArrayData::I16(array) => variant!(array),
        IntegerArrayData::ComplexI16(array) => variant!(array),
        IntegerArrayData::U16(array) => variant!(array),
        IntegerArrayData::ComplexU16(array) => variant!(array),
        IntegerArrayData::I32(array) => variant!(array),
        IntegerArrayData::ComplexI32(array) => variant!(array),
        IntegerArrayData::U32(array) => variant!(array),
        IntegerArrayData::ComplexU32(array) => variant!(array),
        IntegerArrayData::I64(array) => variant!(array),
        IntegerArrayData::ComplexI64(array) => variant!(array),
        IntegerArrayData::U64(array) => variant!(array),
        IntegerArrayData::ComplexU64(array) => variant!(array),
    }
}

fn reorder_struct(
    name: &str,
    input: &StructArray,
    shape: Shape,
    offsets: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let fields = input.field_names().to_vec();
    let mut columns = Vec::new();
    columns
        .try_reserve_exact(input.field_count())
        .map_err(|_| mapping_error(name))?;
    for field in 0..input.field_count() {
        let values = input
            .field_values(field)
            .ok_or_else(|| mapping_error(name))?;
        columns.push(reorder_slice(
            name,
            input.numel(),
            offsets,
            context,
            |index| values.get(index).cloned(),
        )?);
    }
    StructArray::from_columns(shape, fields, columns)
        .map(Value::Struct)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))
}

fn host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| mapping_error(name))
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

fn shift_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`circshift` requires finite integer shifts and a positive integer dimension",
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not map the requested array shape"),
    )
}

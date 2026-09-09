use openmat_array::{ArrayData, DenseArray, IntegerArrayData, IntegerComponent, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count,
    expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

pub(super) fn squeeze_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("squeeze", arguments, 1)?;
    expect_max_outputs("squeeze", context, 1)?;
    context.check_cancelled()?;
    let dimensions = arguments[0].dimensions().ok_or_else(|| {
        type_error(
            "squeeze",
            1,
            "numeric, logical, char, or integer array",
            &arguments[0],
        )
    })?;
    if dimensions.len() == 2 {
        return Ok(vec![arguments[0].clone()]);
    }
    let mut squeezed = reserved_dimensions("squeeze", dimensions.len())?;
    for (index, extent) in dimensions.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        if extent != 1 {
            squeezed.push(extent);
        }
    }
    match squeezed.len() {
        0 => squeezed.extend([1, 1]),
        1 => squeezed.push(1),
        _ => {}
    }
    let shape = Shape::new(squeezed).map_err(|error| array_error(&error))?;
    let output = reshape_value("squeeze", &arguments[0], shape, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn permute_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("permute", arguments, 2)?;
    expect_max_outputs("permute", context, 1)?;
    context.check_cancelled()?;
    let input_dimensions = arguments[0].dimensions().ok_or_else(|| {
        type_error(
            "permute",
            1,
            "numeric, logical, char, or integer array",
            &arguments[0],
        )
    })?;
    let order = positive_integer_vector("permute", 2, &arguments[1], context)?;
    if order.len() < input_dimensions.len() {
        return Err(permutation_error());
    }
    for expected in 1..=order.len() {
        let expected = u64::try_from(expected).map_err(|_| permutation_error())?;
        if order.iter().filter(|value| **value == expected).count() != 1 {
            return Err(permutation_error());
        }
    }
    let mut output_dimensions = reserved_dimensions("permute", order.len())?;
    for (index, dimension) in order.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let source = usize::try_from(dimension - 1).map_err(|_| permutation_error())?;
        output_dimensions.push(input_dimensions.get(source).copied().unwrap_or(1));
    }
    let shape = Shape::new(output_dimensions).map_err(|error| array_error(&error))?;
    let output = permute_value(&arguments[0], input_dimensions, &order, shape, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn repmat_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `repmat` expects at least 2 inputs but received {}",
                arguments.len()
            ),
        ));
    }
    expect_max_outputs("repmat", context, 1)?;
    context.check_cancelled()?;
    let input_dimensions = arguments[0].dimensions().ok_or_else(|| {
        type_error(
            "repmat",
            1,
            "numeric, logical, char, or integer array",
            &arguments[0],
        )
    })?;
    let mut repetitions = if arguments.len() == 2 {
        repetition_vector(&arguments[1], context)?
    } else {
        let mut values = reserved_dimensions("repmat", arguments.len() - 1)?;
        for (index, value) in arguments.iter().skip(1).enumerate() {
            check_cancelled_at(context, index)?;
            values.push(repetition_scalar(index + 2, value)?);
        }
        values
    };
    if repetitions.is_empty() {
        return Err(repetition_error(2));
    }
    if repetitions.len() == 1 {
        repetitions.push(repetitions[0]);
    }
    let rank = input_dimensions.len().max(repetitions.len());
    let mut output_dimensions = reserved_dimensions("repmat", rank)?;
    for index in 0..rank {
        check_cancelled_at(context, index)?;
        let input = input_dimensions.get(index).copied().unwrap_or(1);
        let repetition = repetitions.get(index).copied().unwrap_or(1);
        output_dimensions.push(input.checked_mul(repetition).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "the requested `repmat` output dimensions overflowed",
            )
        })?);
    }
    let shape = Shape::new(output_dimensions).map_err(|error| array_error(&error))?;
    let output = repmat_value(&arguments[0], input_dimensions, shape, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn reshape_value(
    name: &str,
    value: &Value,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! reshape_array {
        ($array:expr, $wrap:expr) => {{
            let values = clone_values(name, $array.as_slice(), context)?;
            let array = DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?;
            $wrap(array)
        }};
    }
    match value {
        Value::Logical(value) => scalar_reshape(*value, &shape).map(Value::Logical),
        Value::Double(value) => scalar_reshape(*value, &shape).map(Value::Double),
        Value::Complex(value) => scalar_reshape(*value, &shape).map(Value::Complex),
        Value::Array(ArrayData::F32(array)) => Ok(reshape_array!(array, |array| {
            Value::Array(ArrayData::F32(array))
        })),
        Value::Array(ArrayData::ComplexF32(array)) => Ok(reshape_array!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        })),
        Value::Array(ArrayData::Logical(array)) => Ok(reshape_array!(array, |array| {
            Value::Array(ArrayData::Logical(array))
        })),
        Value::Array(ArrayData::F64(array)) => Ok(reshape_array!(array, |array| {
            Value::Array(ArrayData::F64(array))
        })),
        Value::Array(ArrayData::ComplexF64(array)) => Ok(reshape_array!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        })),
        Value::Array(ArrayData::Char(array)) => Ok(reshape_array!(array, |array| {
            Value::Array(ArrayData::Char(array))
        })),
        Value::Array(ArrayData::Integer(array)) => reshape_integer(name, array, shape, context)
            .map(ArrayData::Integer)
            .map(Value::Array),
        value => Err(type_error(
            name,
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

fn scalar_reshape<T>(value: T, shape: &Shape) -> Result<T, BuiltinError> {
    if shape.numel() == 1 {
        Ok(value)
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "a scalar rearrangement unexpectedly changed its element count",
        ))
    }
}

fn permute_value(
    value: &Value,
    input_dimensions: &[u64],
    order: &[u64],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! permute_array {
        ($array:expr, $wrap:expr) => {{
            let output = permute_dense($array, input_dimensions, order, shape, context)?;
            $wrap(output)
        }};
    }
    match value {
        Value::Logical(value) => scalar_permute(*value, &shape).map(Value::Logical),
        Value::Double(value) => scalar_permute(*value, &shape).map(Value::Double),
        Value::Complex(value) => scalar_permute(*value, &shape).map(Value::Complex),
        Value::Array(ArrayData::F32(array)) => Ok(permute_array!(array, |array| {
            Value::Array(ArrayData::F32(array))
        })),
        Value::Array(ArrayData::ComplexF32(array)) => Ok(permute_array!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        })),
        Value::Array(ArrayData::Logical(array)) => Ok(permute_array!(array, |array| {
            Value::Array(ArrayData::Logical(array))
        })),
        Value::Array(ArrayData::F64(array)) => Ok(permute_array!(array, |array| {
            Value::Array(ArrayData::F64(array))
        })),
        Value::Array(ArrayData::ComplexF64(array)) => Ok(permute_array!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        })),
        Value::Array(ArrayData::Char(array)) => Ok(permute_array!(array, |array| {
            Value::Array(ArrayData::Char(array))
        })),
        Value::Array(ArrayData::Integer(array)) => {
            permute_integer(array, input_dimensions, order, shape, context)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        value => Err(type_error(
            "permute",
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

fn scalar_permute<T>(value: T, shape: &Shape) -> Result<T, BuiltinError> {
    if shape.numel() == 1 {
        Ok(value)
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "a scalar permutation unexpectedly changed its element count",
        ))
    }
}

fn repmat_value(
    value: &Value,
    input_dimensions: &[u64],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! repmat_array {
        ($array:expr, $wrap:expr) => {{
            let output = repmat_dense($array, input_dimensions, shape, context)?;
            $wrap(output)
        }};
    }
    match value {
        Value::Logical(value) => {
            let input = DenseArray::from_vec(
                Shape::new([1, 1]).unwrap(),
                vec![openmat_array::Logical::from(*value)],
            )
            .map_err(|error| array_error(&error))?;
            Ok(repmat_array!(&input, |array| {
                Value::Array(ArrayData::Logical(array))
            }))
        }
        Value::Double(value) => {
            let input = DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![*value])
                .map_err(|error| array_error(&error))?;
            Ok(repmat_array!(&input, |array| {
                Value::Array(ArrayData::F64(array))
            }))
        }
        Value::Complex(value) => {
            let input = DenseArray::from_vec(
                Shape::new([1, 1]).unwrap(),
                vec![openmat_array::Complex64::new(value.real, value.imaginary)],
            )
            .map_err(|error| array_error(&error))?;
            Ok(repmat_array!(&input, |array| {
                Value::Array(ArrayData::ComplexF64(array))
            }))
        }
        Value::Array(ArrayData::F32(array)) => Ok(repmat_array!(array, |array| {
            Value::Array(ArrayData::F32(array))
        })),
        Value::Array(ArrayData::ComplexF32(array)) => Ok(repmat_array!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        })),
        Value::Array(ArrayData::Logical(array)) => Ok(repmat_array!(array, |array| {
            Value::Array(ArrayData::Logical(array))
        })),
        Value::Array(ArrayData::F64(array)) => Ok(repmat_array!(array, |array| {
            Value::Array(ArrayData::F64(array))
        })),
        Value::Array(ArrayData::ComplexF64(array)) => Ok(repmat_array!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        })),
        Value::Array(ArrayData::Char(array)) => Ok(repmat_array!(array, |array| {
            Value::Array(ArrayData::Char(array))
        })),
        Value::Array(ArrayData::Integer(array)) => {
            repmat_integer(array, input_dimensions, shape, context)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        value => Err(type_error(
            "repmat",
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

fn permute_dense<T: Clone>(
    input: &DenseArray<T>,
    input_dimensions: &[u64],
    order: &[u64],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let length = checked_host_length("permute", shape.numel())?;
    let mut source_strides = reserved_dimensions("permute", input_dimensions.len())?;
    let mut stride = 1_u64;
    for extent in input_dimensions.iter().copied() {
        source_strides.push(stride);
        stride = stride
            .checked_mul(extent)
            .ok_or_else(|| mapping_error("permute"))?;
    }
    let mut values = reserved_values("permute", length)?;
    for output_offset in 0..length {
        check_cancelled_at(context, output_offset)?;
        let mut remainder = u64::try_from(output_offset).map_err(|_| mapping_error("permute"))?;
        let mut input_offset = 0_u64;
        for (output_axis, source_dimension) in order.iter().copied().enumerate() {
            let extent = shape.dimensions().get(output_axis).copied().unwrap_or(1);
            let coordinate = if extent == 0 { 0 } else { remainder % extent };
            if extent != 0 {
                remainder /= extent;
            }
            let source_axis =
                usize::try_from(source_dimension - 1).map_err(|_| mapping_error("permute"))?;
            let source_stride = source_strides.get(source_axis).copied().unwrap_or(stride);
            input_offset = coordinate
                .checked_mul(source_stride)
                .and_then(|value| input_offset.checked_add(value))
                .ok_or_else(|| mapping_error("permute"))?;
        }
        let input_offset = usize::try_from(input_offset).map_err(|_| mapping_error("permute"))?;
        values.push(
            input
                .as_slice()
                .get(input_offset)
                .ok_or_else(|| mapping_error("permute"))?
                .clone(),
        );
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn repmat_dense<T: Clone>(
    input: &DenseArray<T>,
    input_dimensions: &[u64],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let length = checked_host_length("repmat", shape.numel())?;
    let mut values = reserved_values("repmat", length)?;
    for output_offset in 0..length {
        check_cancelled_at(context, output_offset)?;
        let mut remainder = u64::try_from(output_offset).map_err(|_| mapping_error("repmat"))?;
        let mut input_offset = 0_u64;
        let mut input_stride = 1_u64;
        for (axis, output_extent) in shape.dimensions().iter().copied().enumerate() {
            let coordinate = if output_extent == 0 {
                0
            } else {
                remainder % output_extent
            };
            if output_extent != 0 {
                remainder /= output_extent;
            }
            let input_extent = input_dimensions.get(axis).copied().unwrap_or(1);
            let input_coordinate = if input_extent == 0 {
                0
            } else {
                coordinate % input_extent
            };
            input_offset = input_coordinate
                .checked_mul(input_stride)
                .and_then(|value| input_offset.checked_add(value))
                .ok_or_else(|| mapping_error("repmat"))?;
            input_stride = input_stride
                .checked_mul(input_extent)
                .ok_or_else(|| mapping_error("repmat"))?;
        }
        let input_offset = usize::try_from(input_offset).map_err(|_| mapping_error("repmat"))?;
        values.push(
            input
                .as_slice()
                .get(input_offset)
                .ok_or_else(|| mapping_error("repmat"))?
                .clone(),
        );
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn reshape_integer(
    name: &str,
    input: &IntegerArrayData,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! variant {
        ($array:expr) => {{
            let values = clone_values(name, $array.as_slice(), context)?;
            DenseArray::from_vec(shape, values)
                .map(IntegerArrayData::from_typed)
                .map_err(|error| array_error(&error))
        }};
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

fn permute_integer(
    input: &IntegerArrayData,
    dimensions: &[u64],
    order: &[u64],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! variant {
        ($array:expr) => {
            permute_dense($array, dimensions, order, shape, context)
                .map(IntegerArrayData::from_typed)
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

fn repmat_integer(
    input: &IntegerArrayData,
    dimensions: &[u64],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! variant {
        ($array:expr) => {
            repmat_dense($array, dimensions, shape, context).map(IntegerArrayData::from_typed)
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

fn positive_integer_vector(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    numeric_vector(name, position, value, context, |value| {
        if value < 1.0 || value.fract() != 0.0 {
            Err(permutation_error())
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok(value as u64)
        }
    })
}

fn repetition_vector(
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    numeric_vector("repmat", 2, value, context, |value| {
        repetition_float(2, value)
    })
}

fn numeric_vector<F>(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
    mut convert: F,
) -> Result<Vec<u64>, BuiltinError>
where
    F: FnMut(f64) -> Result<u64, BuiltinError>,
{
    if let Some(value) = exact_real_integer_scalar(value) {
        return integer_component_u64(name, position, value).map(|value| vec![value]);
    }
    if let Some(value) = scalar_numeric_f64(value) {
        return convert(value).map(|value| vec![value]);
    }
    let length = value
        .numel()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| type_error(name, position, "real integer scalar or vector", value))?;
    let mut output = reserved_dimensions(name, length)?;
    macro_rules! convert_slice {
        ($slice:expr, $to_f64:expr) => {{
            for (index, value) in $slice.iter().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(convert($to_f64(value))?);
            }
        }};
    }
    match value {
        Value::Array(ArrayData::F64(array)) => {
            convert_slice!(array.as_slice(), |value: &f64| *value);
        }
        Value::Array(ArrayData::ComplexF64(array))
            if array.as_slice().iter().all(|v| v.im == 0.0) =>
        {
            convert_slice!(array.as_slice(), |value: &openmat_array::Complex64| value
                .re);
        }
        Value::Array(ArrayData::Logical(array)) => {
            convert_slice!(
                array.as_slice(),
                |value: &openmat_array::Logical| f64::from(value.get())
            );
        }
        Value::Array(ArrayData::F32(array)) => {
            convert_slice!(array.as_slice(), |value: &f32| f64::from(*value));
        }
        Value::Array(ArrayData::ComplexF32(array))
            if array.as_slice().iter().all(|value| value.im == 0.0) =>
        {
            convert_slice!(array.as_slice(), |value: &openmat_array::Complex32| {
                f64::from(value.re)
            });
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(integer_component_u64(
                    name,
                    position,
                    value.real_component(),
                )?);
            }
        }
        value => {
            return Err(type_error(
                name,
                position,
                "real integer scalar or vector",
                value,
            ));
        }
    }
    Ok(output)
}

fn integer_component_u64(
    name: &str,
    position: usize,
    value: IntegerComponent,
) -> Result<u64, BuiltinError> {
    match value {
        IntegerComponent::Signed(value) => {
            if name == "repmat" {
                Ok(u64::try_from(value).unwrap_or(0))
            } else if value < 1 {
                Err(permutation_error())
            } else {
                u64::try_from(value).map_err(|_| permutation_error())
            }
        }
        IntegerComponent::Unsigned(value) => {
            let value = u64::try_from(value).map_err(|_| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input {position} to `{name}` exceeds the runtime shape range"),
                )
            })?;
            if name != "repmat" && value < 1 {
                Err(permutation_error())
            } else {
                Ok(value)
            }
        }
    }
}

fn scalar_numeric_f64(value: &Value) -> Option<f64> {
    if let Some(value) = value.as_real_number() {
        return Some(value);
    }
    if let Some(value) = value.as_real_single() {
        return Some(f64::from(value));
    }
    None
}

fn repetition_scalar(position: usize, value: &Value) -> Result<u64, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        return match value {
            IntegerComponent::Signed(value) => Ok(u64::try_from(value).unwrap_or(0)),
            IntegerComponent::Unsigned(value) => {
                u64::try_from(value).map_err(|_| repetition_error(position))
            }
        };
    }
    let value = value
        .as_real_number()
        .or_else(|| value.as_real_single().map(f64::from))
        .ok_or_else(|| type_error("repmat", position, "real integer scalar", value))?;
    repetition_float(position, value)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn repetition_float(position: usize, value: f64) -> Result<u64, BuiltinError> {
    if !value.is_finite() || value.fract() != 0.0 || value >= U64_EXCLUSIVE_UPPER_BOUND {
        return Err(repetition_error(position));
    }
    if value <= 0.0 {
        Ok(0)
    } else {
        Ok(value as u64)
    }
}

fn repetition_error(position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `repmat` must contain finite integer repetition counts"),
    )
}

fn permutation_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "input 2 to `permute` must be a permutation containing every input dimension",
    )
}

fn clone_values<T: Clone>(
    name: &str,
    input: &[T],
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut values = reserved_values(name, input.len())?;
    for chunk in input.chunks(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
        values.extend_from_slice(chunk);
    }
    Ok(values)
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

fn reserved_dimensions(name: &str, length: usize) -> Result<Vec<u64>, BuiltinError> {
    reserved_values(name, length)
}

fn checked_host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` output length {length} does not fit this host"),
        )
    })
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` computed an invalid checked column-major mapping"),
    )
}

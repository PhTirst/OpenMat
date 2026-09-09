use openmat_array::{ArrayData, CharCodeUnit, DenseArray, IntegerArrayData, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::{
    array_error, exact_real_integer_scalar, expect_argument_count, expect_argument_count_range,
    expect_max_outputs, positive_integer_dimension, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone)]
struct FloatInput<T> {
    shape: Shape,
    values: Vec<T>,
    scalar_variant: bool,
}

#[derive(Clone)]
enum RealInput {
    Double(FloatInput<f64>),
    Single(FloatInput<f32>),
}

pub(super) fn cart2pol_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("cart2pol", arguments, 2, 3)?;
    expect_max_outputs("cart2pol", context, arguments.len())?;
    context.check_cancelled()?;
    let requested = context.requested_outputs().max(1);
    let x = real_input("cart2pol", 1, &arguments[0], context)?;
    let y = real_input("cart2pol", 2, &arguments[1], context)?;
    let mut outputs = Vec::with_capacity(requested);
    outputs.push(binary_value(
        "cart2pol",
        &y,
        &x,
        context,
        f64::atan2,
        f32::atan2,
    )?);
    if requested >= 2 {
        outputs.push(binary_value(
            "cart2pol",
            &x,
            &y,
            context,
            f64::hypot,
            f32::hypot,
        )?);
    }
    if requested >= 3 {
        outputs.push(arguments[2].clone());
    }
    context.check_cancelled()?;
    Ok(outputs)
}

pub(super) fn pol2cart_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("pol2cart", arguments, 2, 3)?;
    expect_max_outputs("pol2cart", context, arguments.len())?;
    context.check_cancelled()?;
    let requested = context.requested_outputs().max(1);
    let theta = real_input("pol2cart", 1, &arguments[0], context)?;
    let rho = real_input("pol2cart", 2, &arguments[1], context)?;
    let cosine = unary_value("pol2cart", &theta, context, f64::cos, f32::cos)?;
    let cosine = real_input("pol2cart", 1, &cosine, context)?;
    let mut outputs = Vec::with_capacity(requested);
    outputs.push(binary_value(
        "pol2cart",
        &rho,
        &cosine,
        context,
        |left, right| left * right,
        |left, right| left * right,
    )?);
    if requested >= 2 {
        let sine = unary_value("pol2cart", &theta, context, f64::sin, f32::sin)?;
        let sine = real_input("pol2cart", 1, &sine, context)?;
        outputs.push(binary_value(
            "pol2cart",
            &rho,
            &sine,
            context,
            |left, right| left * right,
            |left, right| left * right,
        )?);
    }
    if requested >= 3 {
        outputs.push(arguments[2].clone());
    }
    context.check_cancelled()?;
    Ok(outputs)
}

pub(super) fn cart2sph_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("cart2sph", arguments, 3)?;
    expect_max_outputs("cart2sph", context, 3)?;
    context.check_cancelled()?;
    let requested = context.requested_outputs().max(1);
    let x = real_input("cart2sph", 1, &arguments[0], context)?;
    let y = real_input("cart2sph", 2, &arguments[1], context)?;
    let mut outputs = Vec::with_capacity(requested);
    outputs.push(binary_value(
        "cart2sph",
        &y,
        &x,
        context,
        f64::atan2,
        f32::atan2,
    )?);
    if requested >= 2 {
        let horizontal = binary_value("cart2sph", &x, &y, context, f64::hypot, f32::hypot)?;
        let horizontal = real_input("cart2sph", 1, &horizontal, context)?;
        let z = real_input("cart2sph", 3, &arguments[2], context)?;
        outputs.push(binary_value(
            "cart2sph",
            &z,
            &horizontal,
            context,
            f64::atan2,
            f32::atan2,
        )?);
        if requested >= 3 {
            outputs.push(binary_value(
                "cart2sph",
                &horizontal,
                &z,
                context,
                f64::hypot,
                f32::hypot,
            )?);
        }
    }
    context.check_cancelled()?;
    Ok(outputs)
}

pub(super) fn sph2cart_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("sph2cart", arguments, 3)?;
    expect_max_outputs("sph2cart", context, 3)?;
    context.check_cancelled()?;
    let requested = context.requested_outputs().max(1);
    let azimuth = real_input("sph2cart", 1, &arguments[0], context)?;
    let elevation = real_input("sph2cart", 2, &arguments[1], context)?;
    let radius = real_input("sph2cart", 3, &arguments[2], context)?;
    let elevation_cosine = unary_value("sph2cart", &elevation, context, f64::cos, f32::cos)?;
    let elevation_cosine = real_input("sph2cart", 2, &elevation_cosine, context)?;
    let horizontal = binary_value(
        "sph2cart",
        &radius,
        &elevation_cosine,
        context,
        |left, right| left * right,
        |left, right| left * right,
    )?;
    let horizontal = real_input("sph2cart", 2, &horizontal, context)?;
    let azimuth_cosine = unary_value("sph2cart", &azimuth, context, f64::cos, f32::cos)?;
    let azimuth_cosine = real_input("sph2cart", 1, &azimuth_cosine, context)?;
    let mut outputs = Vec::with_capacity(requested);
    outputs.push(binary_value(
        "sph2cart",
        &horizontal,
        &azimuth_cosine,
        context,
        |left, right| left * right,
        |left, right| left * right,
    )?);
    if requested >= 2 {
        let azimuth_sine = unary_value("sph2cart", &azimuth, context, f64::sin, f32::sin)?;
        let azimuth_sine = real_input("sph2cart", 1, &azimuth_sine, context)?;
        outputs.push(binary_value(
            "sph2cart",
            &horizontal,
            &azimuth_sine,
            context,
            |left, right| left * right,
            |left, right| left * right,
        )?);
    }
    if requested >= 3 {
        let elevation_sine = unary_value("sph2cart", &elevation, context, f64::sin, f32::sin)?;
        let elevation_sine = real_input("sph2cart", 2, &elevation_sine, context)?;
        outputs.push(binary_value(
            "sph2cart",
            &radius,
            &elevation_sine,
            context,
            |left, right| left * right,
            |left, right| left * right,
        )?);
    }
    context.check_cancelled()?;
    Ok(outputs)
}

pub(super) fn unwrap_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("unwrap", arguments, 1, 3)?;
    expect_max_outputs("unwrap", context, 1)?;
    context.check_cancelled()?;
    let tolerance = arguments
        .get(1)
        .map_or(Ok(std::f64::consts::PI), unwrap_tolerance)?;
    let dimensions = arguments[0]
        .dimensions()
        .ok_or_else(|| type_error("unwrap", 1, "real numeric array", &arguments[0]))?;
    let axis = arguments.get(2).map_or_else(
        || Ok(default_axis(dimensions)),
        |value| positive_integer_dimension("unwrap", 3, value),
    )?;
    let output = unwrap_value(&arguments[0], tolerance, axis, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn real_input(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<RealInput, BuiltinError> {
    let scalar_shape = || Shape::new([1, 1]).map_err(|error| array_error(&error));
    match value {
        Value::Double(value) => Ok(RealInput::Double(FloatInput {
            shape: scalar_shape()?,
            values: vec![*value],
            scalar_variant: true,
        })),
        Value::Array(ArrayData::F64(array)) => Ok(RealInput::Double(FloatInput {
            shape: array.shape().clone(),
            values: copy_values(name, array.as_slice(), context)?,
            scalar_variant: false,
        })),
        Value::Array(ArrayData::F32(array)) => Ok(RealInput::Single(FloatInput {
            shape: array.shape().clone(),
            values: copy_values(name, array.as_slice(), context)?,
            scalar_variant: false,
        })),
        value => Err(type_error(
            name,
            position,
            "real double or single array",
            value,
        )),
    }
}

fn unary_value<FD, FS>(
    name: &str,
    input: &RealInput,
    context: &BuiltinContext<'_>,
    mut double_operation: FD,
    mut single_operation: FS,
) -> Result<Value, BuiltinError>
where
    FD: FnMut(f64) -> f64,
    FS: FnMut(f32) -> f32,
{
    match input {
        RealInput::Double(input) => {
            let values = map_values(name, &input.values, context, |value| {
                double_operation(*value)
            })?;
            double_output(input.shape.clone(), values, input.scalar_variant)
        }
        RealInput::Single(input) => {
            let values = map_values(name, &input.values, context, |value| {
                single_operation(*value)
            })?;
            single_output(input.shape.clone(), values)
        }
    }
}

fn binary_value<FD, FS>(
    name: &str,
    left: &RealInput,
    right: &RealInput,
    context: &BuiltinContext<'_>,
    mut double_operation: FD,
    mut single_operation: FS,
) -> Result<Value, BuiltinError>
where
    FD: FnMut(f64, f64) -> f64,
    FS: FnMut(f32, f32) -> f32,
{
    if let (RealInput::Double(left), RealInput::Double(right)) = (left, right) {
        let shape = compatible_shape(name, &[&left.shape, &right.shape])?;
        let values = map2(name, left, right, &shape, context, &mut double_operation)?;
        double_output(shape, values, left.scalar_variant && right.scalar_variant)
    } else {
        let left = as_single_input(name, left, context)?;
        let right = as_single_input(name, right, context)?;
        let shape = compatible_shape(name, &[&left.shape, &right.shape])?;
        let values = map2(name, &left, &right, &shape, context, &mut single_operation)?;
        single_output(shape, values)
    }
}

fn as_single_input(
    name: &str,
    input: &RealInput,
    context: &BuiltinContext<'_>,
) -> Result<FloatInput<f32>, BuiltinError> {
    match input {
        RealInput::Single(input) => Ok(input.clone()),
        RealInput::Double(input) => Ok(FloatInput {
            shape: input.shape.clone(),
            values: map_values(name, &input.values, context, |value| f64_to_f32(*value))?,
            scalar_variant: input.scalar_variant,
        }),
    }
}

fn compatible_shape(name: &str, shapes: &[&Shape]) -> Result<Shape, BuiltinError> {
    let rank = shapes.iter().map(|shape| shape.ndims()).max().unwrap_or(2);
    let mut dimensions = Vec::new();
    dimensions.try_reserve_exact(rank).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate implicit-expansion shape metadata"),
        )
    })?;
    for axis in 0..rank {
        let mut extent = 1_u64;
        for shape in shapes {
            let current = shape.extent(axis);
            if current != 1 {
                if extent != 1 && extent != current {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!(
                            "inputs to `{name}` have incompatible extents {extent} and {current} in dimension {}",
                            axis + 1
                        ),
                    ));
                }
                extent = current;
            }
        }
        dimensions.push(extent);
    }
    Shape::new(dimensions).map_err(|error| array_error(&error))
}

fn map2<T: Copy, U, F>(
    name: &str,
    left: &FloatInput<T>,
    right: &FloatInput<T>,
    shape: &Shape,
    context: &BuiltinContext<'_>,
    operation: &mut F,
) -> Result<Vec<U>, BuiltinError>
where
    F: FnMut(T, T) -> U,
{
    map_offsets(name, shape, context, |index| {
        let left = input_value(name, left, shape, index)?;
        let right = input_value(name, right, shape, index)?;
        Ok(operation(left, right))
    })
}

fn map_offsets<U, F>(
    name: &str,
    shape: &Shape,
    context: &BuiltinContext<'_>,
    mut value: F,
) -> Result<Vec<U>, BuiltinError>
where
    F: FnMut(usize) -> Result<U, BuiltinError>,
{
    let length = usize::try_from(shape.numel()).map_err(|_| host_length_error(name))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(name, length))?;
    for index in 0..length {
        check_cancelled_at(context, index)?;
        output.push(value(index)?);
    }
    Ok(output)
}

fn input_value<T: Copy>(
    name: &str,
    input: &FloatInput<T>,
    output_shape: &Shape,
    output_offset: usize,
) -> Result<T, BuiltinError> {
    let offset = broadcast_offset(name, output_offset, output_shape, &input.shape)?;
    input
        .values
        .get(offset)
        .copied()
        .ok_or_else(|| mapping_error(name))
}

fn broadcast_offset(
    name: &str,
    output_offset: usize,
    output_shape: &Shape,
    input_shape: &Shape,
) -> Result<usize, BuiltinError> {
    let mut remainder = u64::try_from(output_offset).map_err(|_| mapping_error(name))?;
    let mut input_offset = 0_u64;
    let mut input_stride = 1_u64;
    for (axis, output_extent) in output_shape.dimensions().iter().copied().enumerate() {
        let coordinate = if output_extent == 0 {
            0
        } else {
            remainder % output_extent
        };
        if output_extent != 0 {
            remainder /= output_extent;
        }
        let input_extent = input_shape.extent(axis);
        let input_coordinate = if input_extent == 1 { 0 } else { coordinate };
        input_offset = input_coordinate
            .checked_mul(input_stride)
            .and_then(|value| input_offset.checked_add(value))
            .ok_or_else(|| mapping_error(name))?;
        input_stride = input_stride
            .checked_mul(input_extent)
            .ok_or_else(|| mapping_error(name))?;
    }
    usize::try_from(input_offset).map_err(|_| mapping_error(name))
}

fn double_output(
    shape: Shape,
    values: Vec<f64>,
    scalar_variant: bool,
) -> Result<Value, BuiltinError> {
    if scalar_variant {
        return values.first().copied().map(Value::Double).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                "coordinate transform result was empty",
            )
        });
    }
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn single_output(shape: Shape, values: Vec<f32>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F32)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_precision_loss)]
fn unwrap_tolerance(value: &Value) -> Result<f64, BuiltinError> {
    if value.numel() == Some(0) && value.dtype().is_some() {
        return Ok(std::f64::consts::PI);
    }
    let tolerance = value
        .as_real_number()
        .or_else(|| value.as_real_single().map(f64::from))
        .or_else(|| {
            exact_real_integer_scalar(value).map(|component| match component {
                openmat_array::IntegerComponent::Signed(value) => value as f64,
                openmat_array::IntegerComponent::Unsigned(value) => value as f64,
            })
        })
        .ok_or_else(|| type_error("unwrap", 2, "real scalar tolerance or []", value))?;
    if tolerance.is_nan() {
        Ok(std::f64::consts::PI)
    } else {
        Ok(tolerance.abs())
    }
}

fn unwrap_value(
    value: &Value,
    tolerance: f64,
    axis: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match value {
        Value::Double(value) => Ok(Value::Double(*value)),
        Value::Logical(value) => Ok(Value::Logical(*value)),
        Value::Array(ArrayData::F64(array)) => unwrap_dense(
            array,
            tolerance,
            axis,
            context,
            |value| value,
            |value| value,
        )
        .map(ArrayData::F64)
        .map(Value::Array),
        Value::Array(ArrayData::F32(array)) => unwrap_dense(
            array,
            f64::from(f64_to_f32(tolerance)),
            axis,
            context,
            f64::from,
            f64_to_f32,
        )
        .map(ArrayData::F32)
        .map(Value::Array),
        Value::Array(ArrayData::Logical(array)) => {
            Ok(Value::Array(ArrayData::Logical(array.clone())))
        }
        Value::Array(ArrayData::Char(array)) => unwrap_dense(
            array,
            tolerance,
            axis,
            context,
            |value| f64::from(value.get()),
            rounded_char,
        )
        .map(ArrayData::Char)
        .map(Value::Array),
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            unwrap_integer(array, tolerance, axis, context)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        value => Err(type_error(
            "unwrap",
            1,
            "real numeric, logical, or char array",
            value,
        )),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]
fn unwrap_integer(
    array: &IntegerArrayData,
    tolerance: f64,
    axis: u64,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! signed {
        ($array:expr, $kind:ty) => {{
            unwrap_dense(
                $array,
                tolerance,
                axis,
                context,
                |value| value as f64,
                |value| {
                    value
                        .round()
                        .clamp(<$kind>::MIN as f64, <$kind>::MAX as f64) as $kind
                },
            )
            .map(IntegerArrayData::from_typed)
        }};
    }
    macro_rules! unsigned {
        ($array:expr, $kind:ty) => {{
            unwrap_dense(
                $array,
                tolerance,
                axis,
                context,
                |value| value as f64,
                |value| value.round().clamp(0.0, <$kind>::MAX as f64) as $kind,
            )
            .map(IntegerArrayData::from_typed)
        }};
    }
    match array {
        IntegerArrayData::I8(array) => signed!(array, i8),
        IntegerArrayData::U8(array) => unsigned!(array, u8),
        IntegerArrayData::I16(array) => signed!(array, i16),
        IntegerArrayData::U16(array) => unsigned!(array, u16),
        IntegerArrayData::I32(array) => signed!(array, i32),
        IntegerArrayData::U32(array) => unsigned!(array, u32),
        IntegerArrayData::I64(array) => signed!(array, i64),
        IntegerArrayData::U64(array) => unsigned!(array, u64),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "`unwrap` does not accept complex integer arrays",
        )),
    }
}

fn unwrap_dense<T: Copy, FTo, FFrom>(
    array: &DenseArray<T>,
    tolerance: f64,
    axis: u64,
    context: &BuiltinContext<'_>,
    mut to_f64: FTo,
    mut from_f64: FFrom,
) -> Result<DenseArray<T>, BuiltinError>
where
    FTo: FnMut(T) -> f64,
    FFrom: FnMut(f64) -> T,
{
    let mut output = copy_values("unwrap", array.as_slice(), context)?;
    let axis = usize::try_from(axis - 1).map_err(|_| mapping_error("unwrap"))?;
    if axis >= array.shape().ndims() || output.is_empty() {
        return DenseArray::from_vec(array.shape().clone(), output)
            .map_err(|error| array_error(&error));
    }
    let extent =
        usize::try_from(array.shape().extent(axis)).map_err(|_| host_length_error("unwrap"))?;
    if extent < 2 {
        return DenseArray::from_vec(array.shape().clone(), output)
            .map_err(|error| array_error(&error));
    }
    let stride = axis_stride(array.shape(), axis)?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error("unwrap"))?;
    for block_start in (0..output.len()).step_by(block) {
        for inner in 0..stride {
            let mut previous = None;
            let mut correction = 0.0;
            for along in 0..extent {
                let index = block_start + inner + along * stride;
                check_cancelled_at(context, index)?;
                let current = to_f64(output[index]);
                if !current.is_finite() {
                    continue;
                }
                if let Some(previous) = previous {
                    let delta: f64 = current - previous;
                    if delta.abs() >= tolerance {
                        correction += principal_delta(delta) - delta;
                    }
                }
                output[index] = from_f64(current + correction);
                previous = Some(current);
            }
        }
    }
    DenseArray::from_vec(array.shape().clone(), output).map_err(|error| array_error(&error))
}

fn principal_delta(delta: f64) -> f64 {
    let period = 2.0 * std::f64::consts::PI;
    let mut wrapped = delta - period * (delta / period).round();
    if wrapped == -std::f64::consts::PI && delta > 0.0 {
        wrapped = std::f64::consts::PI;
    }
    wrapped
}

fn axis_stride(shape: &Shape, axis: usize) -> Result<usize, BuiltinError> {
    shape
        .dimensions()
        .iter()
        .take(axis)
        .try_fold(1_u64, |stride, extent| stride.checked_mul(*extent))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| mapping_error("unwrap"))
}

fn default_axis(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|axis| u64::try_from(axis + 1).ok())
        .unwrap_or(1)
}

fn copy_values<T: Copy>(
    name: &str,
    values: &[T],
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    map_values(name, values, context, |value| *value)
}

fn map_values<T, U, F>(
    name: &str,
    values: &[T],
    context: &BuiltinContext<'_>,
    mut map: F,
) -> Result<Vec<U>, BuiltinError>
where
    F: FnMut(&T) -> U,
{
    let mut output = Vec::new();
    output
        .try_reserve_exact(values.len())
        .map_err(|_| allocation_error(name, values.len()))?;
    for (index, value) in values.iter().enumerate() {
        check_cancelled_at(context, index)?;
        output.push(map(value));
    }
    Ok(output)
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

fn allocation_error(name: &str, length: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` cannot allocate storage for {length} elements"),
    )
}

fn host_length_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` output does not fit this host"),
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not map its operands"),
    )
}

#[allow(clippy::cast_possible_truncation)]
fn f64_to_f32(value: f64) -> f32 {
    value as f32
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rounded_char(value: f64) -> CharCodeUnit {
    CharCodeUnit::new(value.round().clamp(0.0, f64::from(u16::MAX)) as u16)
}

#[cfg(test)]
mod tests {
    use openmat_runtime::{BuiltinContext, CancellationToken, NullOutput};

    use super::*;

    fn invoke(
        builtin: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
        arguments: &[Value],
        outputs: usize,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut sink = NullOutput;
        let mut context = BuiltinContext::new(outputs, &cancellation, &mut sink);
        builtin(arguments, &mut context)
    }

    fn double_array(dimensions: &[u64], values: Vec<f64>) -> Value {
        DenseArray::from_vec(Shape::new(dimensions.iter().copied()).unwrap(), values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .unwrap()
    }

    #[test]
    fn cartesian_polar_transforms_preserve_precision_and_passthrough_z() {
        let theta = DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![0.0_f32, 1.0])
            .map(ArrayData::F32)
            .map(Value::Array)
            .unwrap();
        let outputs = invoke(
            pol2cart_builtin,
            &[theta, Value::Double(2.0), Value::from("z-pass")],
            3,
        )
        .unwrap();
        assert!(matches!(
            outputs[0],
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        ));
        assert!(matches!(
            outputs[1],
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        ));
        assert_eq!(outputs[2], Value::from("z-pass"));

        let outputs = invoke(
            cart2pol_builtin,
            &[Value::Double(0.0), Value::Double(-1.0)],
            2,
        )
        .unwrap();
        assert_eq!(outputs[0], Value::Double(-std::f64::consts::FRAC_PI_2));
        assert_eq!(outputs[1], Value::Double(1.0));
    }

    #[test]
    fn spherical_outputs_follow_their_own_dependencies() {
        let x = double_array(&[2, 1], vec![1.0, 2.0]);
        let y = double_array(&[1, 2], vec![3.0, 4.0]);
        let z = DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![5.0_f32])
            .map(ArrayData::F32)
            .map(Value::Array)
            .unwrap();
        let spherical = invoke(cart2sph_builtin, &[x, y, z], 3).unwrap();
        assert!(matches!(spherical[0], Value::Array(ArrayData::F64(_))));
        assert!(matches!(
            spherical[1],
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        ));
        assert!(matches!(
            spherical[2],
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        ));
        for output in &spherical {
            assert_eq!(output.dimensions(), Some([2, 2].as_slice()));
        }

        let azimuth = DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![1.0_f32, 2.0])
            .map(ArrayData::F32)
            .map(Value::Array)
            .unwrap();
        let cartesian = invoke(
            sph2cart_builtin,
            &[azimuth, Value::Double(3.0), Value::Double(4.0)],
            3,
        )
        .unwrap();
        assert!(matches!(
            cartesian[0],
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        ));
        assert!(matches!(
            cartesian[1],
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        ));
        assert_eq!(cartesian[2].dimensions(), Some([1, 1].as_slice()));
        assert!(matches!(cartesian[2], Value::Double(_)));
    }

    #[test]
    fn unwrap_skips_nonfinite_samples_and_preserves_shape_and_class() {
        let input = double_array(&[1, 6], vec![0.0, 3.5, 7.0, f64::NAN, 0.0, 4.0]);
        let output = invoke(unwrap_builtin, &[input], 1).unwrap();
        let Value::Array(ArrayData::F64(output)) = output[0].clone() else {
            panic!("unwrap must preserve double storage");
        };
        let expected = [
            0.0,
            -2.783_185_307_179_586,
            -5.566_370_614_359_172_5,
            f64::NAN,
            -std::f64::consts::TAU,
            -8.566_370_614_359_172,
        ];
        for (actual, expected) in output.as_slice().iter().zip(expected) {
            assert!((actual.is_nan() && expected.is_nan()) || (actual - expected).abs() < 1.0e-12);
        }

        let integer = DenseArray::from_vec(Shape::new([1, 4]).unwrap(), vec![0_i32, 4, 10, -10])
            .map(IntegerArrayData::I32)
            .map(ArrayData::Integer)
            .map(Value::Array)
            .unwrap();
        let output = invoke(unwrap_builtin, &[integer], 1).unwrap();
        let Value::Array(ArrayData::Integer(IntegerArrayData::I32(output))) = &output[0] else {
            panic!("unwrap must preserve integer storage");
        };
        assert_eq!(output.as_slice(), &[0, -2, -3, -4]);
    }
}

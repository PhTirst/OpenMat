use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerComponent, Shape,
};
use openmat_fft::{FftComplex32, FftComplex64, FftDirection, FftError, FftErrorKind, FftProvider};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::core_linalg::{keyword, wrap_complex32, wrap_complex64};
use crate::{U64_EXCLUSIVE_UPPER_BOUND, array_error, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InverseReality {
    Automatic,
    Symmetric,
    Nonsymmetric,
}

enum FourierData {
    F64(Vec<FftComplex64>),
    F32(Vec<FftComplex32>),
}

struct FourierArray {
    shape: Shape,
    data: FourierData,
}

pub(super) fn fft_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    transform_one_dimension("fft", arguments, context, FftDirection::Forward)
}

pub(super) fn ifft_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    let (arguments, reality) = inverse_arguments("ifft", arguments, 3)?;
    transform_one_dimension_with_reality("ifft", arguments, context, reality)
}

pub(super) fn fft2_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    transform_two_dimensions(
        "fft2",
        arguments,
        context,
        FftDirection::Forward,
        InverseReality::Nonsymmetric,
    )
}

pub(super) fn ifft2_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (arguments, reality) = inverse_arguments("ifft2", arguments, 3)?;
    transform_two_dimensions("ifft2", arguments, context, FftDirection::Inverse, reality)
}

pub(super) fn fftn_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    transform_all_dimensions(
        "fftn",
        arguments,
        context,
        FftDirection::Forward,
        InverseReality::Nonsymmetric,
    )
}

pub(super) fn ifftn_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (arguments, reality) = inverse_arguments("ifftn", arguments, 2)?;
    transform_all_dimensions("ifftn", arguments, context, FftDirection::Inverse, reality)
}

fn transform_one_dimension(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    direction: FftDirection,
) -> BuiltinResult {
    transform_one_dimension_inner(
        name,
        arguments,
        context,
        direction,
        InverseReality::Nonsymmetric,
    )
}

fn transform_one_dimension_with_reality(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    reality: InverseReality,
) -> BuiltinResult {
    transform_one_dimension_inner(name, arguments, context, FftDirection::Inverse, reality)
}

fn transform_one_dimension_inner(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    direction: FftDirection,
    reality: InverseReality,
) -> BuiltinResult {
    if !(1..=3).contains(&arguments.len()) {
        return Err(argument_count_error(name, "1 to 3", arguments.len()));
    }
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let mut array = numeric_input(name, &arguments[0], context)?;
    let length = arguments
        .get(1)
        .map(|value| optional_length(name, 2, value))
        .transpose()?
        .flatten();
    let axis = arguments.get(2).map_or_else(
        || Ok(default_fft_axis(&array.shape)),
        |value| positive_dimension(name, 3, value),
    )?;
    let axis = usize::try_from(axis - 1).map_err(|_| shape_error(name))?;
    let mut dimensions = array.shape.dimensions().to_vec();
    if let Some(length) = length {
        if dimensions.len() <= axis {
            dimensions.resize(axis + 1, 1);
        }
        dimensions[axis] = length;
    }
    let target = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    array.resize(name, target, context)?;
    array.transform(name, &[axis], direction, reality, context)?;
    Ok(vec![array.into_value(context)?])
}

fn transform_two_dimensions(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    direction: FftDirection,
    reality: InverseReality,
) -> BuiltinResult {
    if arguments.len() != 1 && arguments.len() != 3 {
        return Err(argument_count_error(name, "1 or 3", arguments.len()));
    }
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let mut array = numeric_input(name, &arguments[0], context)?;
    let mut dimensions = array.shape.dimensions().to_vec();
    dimensions.resize(dimensions.len().max(2), 1);
    if arguments.len() == 3 {
        if let Some(rows) = optional_length(name, 2, &arguments[1])? {
            dimensions[0] = rows;
        }
        if let Some(columns) = optional_length(name, 3, &arguments[2])? {
            dimensions[1] = columns;
        }
    }
    let target = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    array.resize(name, target, context)?;
    array.transform(name, &[0, 1], direction, reality, context)?;
    Ok(vec![array.into_value(context)?])
}

fn transform_all_dimensions(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    direction: FftDirection,
    reality: InverseReality,
) -> BuiltinResult {
    if !(1..=2).contains(&arguments.len()) {
        return Err(argument_count_error(name, "1 or 2", arguments.len()));
    }
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let mut array = numeric_input(name, &arguments[0], context)?;
    let dimensions = arguments.get(1).map_or_else(
        || Ok(None),
        |value| size_vector(name, value, context).map(Some),
    )?;
    let target = if let Some(dimensions) = dimensions.filter(|value| !value.is_empty()) {
        if dimensions.len() < array.shape.ndims() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` size vector must cover every input dimension"),
            ));
        }
        Shape::new(dimensions).map_err(|error| array_error(&error))?
    } else {
        array.shape.clone()
    };
    let axes = (0..target.ndims()).collect::<Vec<_>>();
    array.resize(name, target, context)?;
    array.transform(name, &axes, direction, reality, context)?;
    Ok(vec![array.into_value(context)?])
}

impl FourierArray {
    fn resize(
        &mut self,
        name: &str,
        target: Shape,
        context: &BuiltinContext<'_>,
    ) -> Result<(), BuiltinError> {
        if self.shape == target {
            return Ok(());
        }
        self.data = match &self.data {
            FourierData::F64(values) => {
                FourierData::F64(resize_values(name, values, &self.shape, &target, context)?)
            }
            FourierData::F32(values) => {
                FourierData::F32(resize_values(name, values, &self.shape, &target, context)?)
            }
        };
        self.shape = target;
        Ok(())
    }

    fn transform(
        &mut self,
        name: &str,
        axes: &[usize],
        direction: FftDirection,
        reality: InverseReality,
        context: &BuiltinContext<'_>,
    ) -> Result<(), BuiltinError> {
        let force_real = direction == FftDirection::Inverse
            && match reality {
                InverseReality::Symmetric => true,
                InverseReality::Nonsymmetric => false,
                InverseReality::Automatic => match &self.data {
                    FourierData::F64(values) => conjugate_symmetric_f64(values, &self.shape, axes),
                    FourierData::F32(values) => conjugate_symmetric_f32(values, &self.shape, axes),
                },
            };
        match &mut self.data {
            FourierData::F64(values) => {
                for &axis in axes {
                    if direction == FftDirection::Inverse && reality == InverseReality::Symmetric {
                        enforce_axis_symmetry_f64(values, &self.shape, axis)?;
                    }
                    transform_axis_f64(
                        name,
                        values,
                        &self.shape,
                        axis,
                        direction,
                        context.fft_provider(),
                        context,
                    )?;
                }
                if force_real {
                    for value in values {
                        value.imaginary = 0.0;
                    }
                }
            }
            FourierData::F32(values) => {
                for &axis in axes {
                    if direction == FftDirection::Inverse && reality == InverseReality::Symmetric {
                        enforce_axis_symmetry_f32(values, &self.shape, axis)?;
                    }
                    transform_axis_f32(
                        name,
                        values,
                        &self.shape,
                        axis,
                        direction,
                        context.fft_provider(),
                        context,
                    )?;
                }
                if force_real {
                    for value in values {
                        value.imaginary = 0.0;
                    }
                }
            }
        }
        context.check_cancelled()
    }

    fn into_value(self, context: &BuiltinContext<'_>) -> Result<Value, BuiltinError> {
        match self.data {
            FourierData::F64(values) => {
                let values = values
                    .into_iter()
                    .map(|value| ArrayComplex64::new(value.real, value.imaginary))
                    .collect();
                let array = DenseArray::from_vec(self.shape, values)
                    .map_err(|error| array_error(&error))?;
                wrap_complex64(array, context)
            }
            FourierData::F32(values) => {
                let values = values
                    .into_iter()
                    .map(|value| ArrayComplex32::new(value.real, value.imaginary))
                    .collect();
                let array = DenseArray::from_vec(self.shape, values)
                    .map_err(|error| array_error(&error))?;
                wrap_complex32(array, context)
            }
        }
    }
}

fn numeric_input(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<FourierArray, BuiltinError> {
    let scalar_shape = || Shape::new([1, 1]).map_err(|error| array_error(&error));
    match value {
        Value::Logical(value) => Ok(FourierArray {
            shape: scalar_shape()?,
            data: FourierData::F64(vec![FftComplex64::new(f64::from(u8::from(*value)), 0.0)]),
        }),
        Value::Double(value) => Ok(FourierArray {
            shape: scalar_shape()?,
            data: FourierData::F64(vec![FftComplex64::new(*value, 0.0)]),
        }),
        Value::Complex(value) => Ok(FourierArray {
            shape: scalar_shape()?,
            data: FourierData::F64(vec![FftComplex64::new(value.real, value.imaginary)]),
        }),
        Value::Array(ArrayData::F32(array)) => Ok(FourierArray {
            shape: array.shape().clone(),
            data: FourierData::F32(
                array
                    .as_slice()
                    .iter()
                    .map(|value| FftComplex32::new(*value, 0.0))
                    .collect(),
            ),
        }),
        Value::Array(ArrayData::ComplexF32(array)) => Ok(FourierArray {
            shape: array.shape().clone(),
            data: FourierData::F32(
                array
                    .as_slice()
                    .iter()
                    .map(|value| FftComplex32::new(value.re, value.im))
                    .collect(),
            ),
        }),
        Value::Array(ArrayData::F64(array)) => Ok(FourierArray {
            shape: array.shape().clone(),
            data: FourierData::F64(
                array
                    .as_slice()
                    .iter()
                    .map(|value| FftComplex64::new(*value, 0.0))
                    .collect(),
            ),
        }),
        Value::Array(ArrayData::ComplexF64(array)) => Ok(FourierArray {
            shape: array.shape().clone(),
            data: FourierData::F64(
                array
                    .as_slice()
                    .iter()
                    .map(|value| FftComplex64::new(value.re, value.im))
                    .collect(),
            ),
        }),
        Value::Array(ArrayData::Logical(array)) => Ok(FourierArray {
            shape: array.shape().clone(),
            data: FourierData::F64(
                array
                    .as_slice()
                    .iter()
                    .map(|value| FftComplex64::new(f64::from(value.get()), 0.0))
                    .collect(),
            ),
        }),
        Value::Array(ArrayData::Integer(array)) => {
            let mut values = reserved_values(name, array.numel())?;
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                values.push(FftComplex64::new(
                    integer_component_f64(value.real_component()),
                    value
                        .imaginary_component()
                        .map_or(0.0, integer_component_f64),
                ));
            }
            Ok(FourierArray {
                shape: array.shape().clone(),
                data: FourierData::F64(values),
            })
        }
        _ => Err(type_error(
            name,
            1,
            "double, single, logical, or integer numeric array",
            value,
        )),
    }
}

fn resize_values<T: Copy + Default>(
    name: &str,
    source: &[T],
    source_shape: &Shape,
    target_shape: &Shape,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let target_length = checked_host_length(name, target_shape.numel())?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(target_length)
        .map_err(|_| shape_error(name))?;
    for target_offset in 0..target_length {
        check_cancelled_at(context, target_offset)?;
        let target_offset_u64 = u64::try_from(target_offset).map_err(|_| shape_error(name))?;
        let mut source_offset = 0_u64;
        let mut inside = true;
        for axis in 0..target_shape.ndims() {
            let target_extent = target_shape.extent(axis);
            let coordinate = if target_extent == 0 {
                0
            } else {
                (target_offset_u64 / target_shape.stride(axis)) % target_extent
            };
            if coordinate >= source_shape.extent(axis) {
                inside = false;
                break;
            }
            source_offset = source_offset
                .checked_add(
                    coordinate
                        .checked_mul(source_shape.stride(axis))
                        .ok_or_else(|| shape_error(name))?,
                )
                .ok_or_else(|| shape_error(name))?;
        }
        let value = if inside {
            usize::try_from(source_offset)
                .ok()
                .and_then(|offset| source.get(offset))
                .copied()
                .unwrap_or_default()
        } else {
            T::default()
        };
        output.push(value);
    }
    Ok(output)
}

fn transform_axis_f64(
    name: &str,
    values: &mut [FftComplex64],
    shape: &Shape,
    axis: usize,
    direction: FftDirection,
    provider: &dyn FftProvider,
    context: &BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    let length = checked_host_length(name, shape.extent(axis))?;
    if length <= 1 || values.is_empty() {
        return Ok(());
    }
    let mut batch = gather_axis(name, values, shape, axis, context)?;
    provider
        .transform_f64(&mut batch, length, direction, context.cancellation_flag())
        .map_err(|error| fft_error(name, error))?;
    if direction == FftDirection::Inverse {
        #[allow(clippy::cast_precision_loss)]
        let scale = 1.0 / length as f64;
        for value in &mut batch {
            value.real *= scale;
            value.imaginary *= scale;
        }
    }
    scatter_axis(name, values, &batch, shape, axis, context)
}

fn transform_axis_f32(
    name: &str,
    values: &mut [FftComplex32],
    shape: &Shape,
    axis: usize,
    direction: FftDirection,
    provider: &dyn FftProvider,
    context: &BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    let length = checked_host_length(name, shape.extent(axis))?;
    if length <= 1 || values.is_empty() {
        return Ok(());
    }
    let mut batch = gather_axis(name, values, shape, axis, context)?;
    provider
        .transform_f32(&mut batch, length, direction, context.cancellation_flag())
        .map_err(|error| fft_error(name, error))?;
    if direction == FftDirection::Inverse {
        #[allow(clippy::cast_precision_loss)]
        let scale = 1.0 / length as f32;
        for value in &mut batch {
            value.real *= scale;
            value.imaginary *= scale;
        }
    }
    scatter_axis(name, values, &batch, shape, axis, context)
}

fn gather_axis<T: Copy>(
    name: &str,
    values: &[T],
    shape: &Shape,
    axis: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let length = checked_host_length(name, shape.extent(axis))?;
    let stride = checked_host_length(name, shape.stride(axis))?;
    let block = length
        .checked_mul(stride)
        .ok_or_else(|| shape_error(name))?;
    if block == 0 || !values.len().is_multiple_of(block) {
        return Err(shape_error(name));
    }
    let blocks = values.len() / block;
    let mut output = Vec::new();
    output
        .try_reserve_exact(values.len())
        .map_err(|_| shape_error(name))?;
    for outer in 0..blocks {
        for inner in 0..stride {
            for coordinate in 0..length {
                check_cancelled_at(context, output.len())?;
                let offset = outer * block + coordinate * stride + inner;
                output.push(*values.get(offset).ok_or_else(|| shape_error(name))?);
            }
        }
    }
    Ok(output)
}

fn scatter_axis<T: Copy>(
    name: &str,
    values: &mut [T],
    batch: &[T],
    shape: &Shape,
    axis: usize,
    context: &BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    let length = checked_host_length(name, shape.extent(axis))?;
    let stride = checked_host_length(name, shape.stride(axis))?;
    let block = length
        .checked_mul(stride)
        .ok_or_else(|| shape_error(name))?;
    if block == 0 || values.len() != batch.len() || !values.len().is_multiple_of(block) {
        return Err(shape_error(name));
    }
    let blocks = values.len() / block;
    let mut input = 0;
    for outer in 0..blocks {
        for inner in 0..stride {
            for coordinate in 0..length {
                check_cancelled_at(context, input)?;
                let offset = outer * block + coordinate * stride + inner;
                values[offset] = batch[input];
                input += 1;
            }
        }
    }
    Ok(())
}

fn conjugate_symmetric_f64(values: &[FftComplex64], shape: &Shape, axes: &[usize]) -> bool {
    conjugate_symmetric(values, shape, axes, |left, right| {
        close_f64(left.real, right.real) && close_f64(left.imaginary, -right.imaginary)
    })
}

fn enforce_axis_symmetry_f64(
    values: &mut [FftComplex64],
    shape: &Shape,
    axis: usize,
) -> Result<(), BuiltinError> {
    enforce_axis_symmetry(values, shape, axis, |value| {
        FftComplex64::new(value.real, -value.imaginary)
    })?;
    enforce_self_conjugate(values, shape, axis, |value| value.imaginary = 0.0)
}

fn enforce_axis_symmetry_f32(
    values: &mut [FftComplex32],
    shape: &Shape,
    axis: usize,
) -> Result<(), BuiltinError> {
    enforce_axis_symmetry(values, shape, axis, |value| {
        FftComplex32::new(value.real, -value.imaginary)
    })?;
    enforce_self_conjugate(values, shape, axis, |value| value.imaginary = 0.0)
}

fn enforce_axis_symmetry<T: Copy>(
    values: &mut [T],
    shape: &Shape,
    axis: usize,
    conjugate: impl Fn(T) -> T,
) -> Result<(), BuiltinError> {
    let name = "ifft";
    let length = checked_host_length(name, shape.extent(axis))?;
    let stride = checked_host_length(name, shape.stride(axis))?;
    let block = length
        .checked_mul(stride)
        .ok_or_else(|| shape_error(name))?;
    if length <= 1 || values.is_empty() {
        return Ok(());
    }
    if block == 0 || !values.len().is_multiple_of(block) {
        return Err(shape_error(name));
    }
    for outer in 0..values.len() / block {
        for inner in 0..stride {
            let base = outer * block + inner;
            for positive in 1..=length.saturating_sub(1) / 2 {
                let negative = length - positive;
                values[base + negative * stride] = conjugate(values[base + positive * stride]);
            }
        }
    }
    Ok(())
}

fn enforce_self_conjugate<T>(
    values: &mut [T],
    shape: &Shape,
    axis: usize,
    clear_imaginary: impl Fn(&mut T),
) -> Result<(), BuiltinError> {
    let name = "ifft";
    let length = checked_host_length(name, shape.extent(axis))?;
    let stride = checked_host_length(name, shape.stride(axis))?;
    let block = length
        .checked_mul(stride)
        .ok_or_else(|| shape_error(name))?;
    if length == 0 || values.is_empty() {
        return Ok(());
    }
    if block == 0 || !values.len().is_multiple_of(block) {
        return Err(shape_error(name));
    }
    for outer in 0..values.len() / block {
        for inner in 0..stride {
            let base = outer * block + inner;
            clear_imaginary(&mut values[base]);
            if length.is_multiple_of(2) {
                clear_imaginary(&mut values[base + length / 2 * stride]);
            }
        }
    }
    Ok(())
}

fn conjugate_symmetric_f32(values: &[FftComplex32], shape: &Shape, axes: &[usize]) -> bool {
    conjugate_symmetric(values, shape, axes, |left, right| {
        close_f32(left.real, right.real) && close_f32(left.imaginary, -right.imaginary)
    })
}

fn conjugate_symmetric<T>(
    values: &[T],
    shape: &Shape,
    axes: &[usize],
    close: impl Fn(T, T) -> bool,
) -> bool
where
    T: Copy,
{
    values.iter().copied().enumerate().all(|(offset, value)| {
        let Ok(mut remainder) = u64::try_from(offset) else {
            return false;
        };
        let mut reflected = 0_u64;
        for axis in 0..shape.ndims() {
            let extent = shape.extent(axis);
            let coordinate = if extent == 0 { 0 } else { remainder % extent };
            if extent != 0 {
                remainder /= extent;
            }
            let coordinate = if axes.contains(&axis) && extent > 0 {
                (extent - coordinate) % extent
            } else {
                coordinate
            };
            let Some(component) = coordinate.checked_mul(shape.stride(axis)) else {
                return false;
            };
            let Some(offset) = reflected.checked_add(component) else {
                return false;
            };
            reflected = offset;
        }
        usize::try_from(reflected)
            .ok()
            .and_then(|reflected| values.get(reflected))
            .copied()
            .is_some_and(|reflected| close(value, reflected))
    })
}

#[allow(clippy::float_cmp)]
fn close_f64(left: f64, right: f64) -> bool {
    left == right
        || left.is_finite()
            && right.is_finite()
            && (left - right).abs() <= 32.0 * f64::EPSILON * left.abs().max(right.abs()).max(1.0)
}

#[allow(clippy::float_cmp)]
fn close_f32(left: f32, right: f32) -> bool {
    left == right
        || left.is_finite()
            && right.is_finite()
            && (left - right).abs() <= 32.0 * f32::EPSILON * left.abs().max(right.abs()).max(1.0)
}

fn inverse_arguments<'a>(
    name: &str,
    arguments: &'a [Value],
    maximum_without_option: usize,
) -> Result<(&'a [Value], InverseReality), BuiltinError> {
    if arguments.is_empty() {
        return Err(argument_count_error(name, "at least 1", 0));
    }
    let mut values = arguments;
    let mut reality = InverseReality::Automatic;
    if let Some(option) = arguments.last().and_then(keyword) {
        reality = match option.as_str() {
            "symmetric" => InverseReality::Symmetric,
            "nonsymmetric" => InverseReality::Nonsymmetric,
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("`{name}` received an unsupported trailing option"),
                ));
            }
        };
        values = &arguments[..arguments.len() - 1];
    }
    if values.is_empty() || values.len() > maximum_without_option {
        return Err(argument_count_error(
            name,
            &format!("1 to {maximum_without_option}, plus an optional symmetry flag"),
            arguments.len(),
        ));
    }
    Ok((values, reality))
}

fn optional_length(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<Option<u64>, BuiltinError> {
    if value.numel() == Some(0) && is_numeric_value(value) {
        return Ok(None);
    }
    scalar_nonnegative_integer(name, position, value).map(Some)
}

fn positive_dimension(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    let dimension = scalar_nonnegative_integer(name, position, value)?;
    if dimension == 0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` dimension must be a positive integer"),
        ));
    }
    Ok(dimension)
}

fn scalar_nonnegative_integer(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<u64, BuiltinError> {
    let number = match value {
        Value::Logical(value) => Some(f64::from(u8::from(*value))),
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F32(array)) if array.numel() == 1 => {
            array.as_slice().first().copied().map(f64::from)
        }
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            array.as_slice().first().copied()
        }
        Value::Array(ArrayData::Logical(array)) if array.numel() == 1 => {
            array.as_slice().first().map(|value| f64::from(value.get()))
        }
        Value::Array(ArrayData::Integer(array)) if array.numel() == 1 && !array.is_complex() => {
            array
                .element(0)
                .map(|value| integer_component_f64(value.real_component()))
        }
        _ => None,
    }
    .ok_or_else(|| type_error(name, position, "nonnegative integer scalar", value))?;
    if !number.is_finite()
        || number < 0.0
        || number.fract() != 0.0
        || number >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires nonnegative integer transform sizes"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as u64)
}

fn size_vector(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    let dimensions = value
        .dimensions()
        .ok_or_else(|| type_error(name, 2, "real row size vector", value))?;
    if value.numel() != Some(0) && !(dimensions.len() == 2 && dimensions[0] == 1) {
        return Err(type_error(name, 2, "real row size vector", value));
    }
    let mut output = Vec::new();
    let length = checked_host_length(name, value.numel().unwrap_or(0))?;
    output
        .try_reserve_exact(length)
        .map_err(|_| shape_error(name))?;
    match value {
        Value::Array(ArrayData::F64(array)) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(float_size(name, *value)?);
            }
        }
        Value::Array(ArrayData::F32(array)) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(float_size(name, f64::from(*value))?);
            }
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(integer_size(name, value.real_component())?);
            }
        }
        Value::Array(ArrayData::Logical(array)) => {
            output.extend(array.as_slice().iter().map(|value| u64::from(value.get())));
        }
        _ => return Err(type_error(name, 2, "real row size vector", value)),
    }
    Ok(output)
}

fn float_size(name: &str, value: f64) -> Result<u64, BuiltinError> {
    if !value.is_finite()
        || value < 0.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(shape_error(name));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn integer_size(name: &str, value: IntegerComponent) -> Result<u64, BuiltinError> {
    match value {
        IntegerComponent::Signed(value) => u64::try_from(value).map_err(|_| shape_error(name)),
        IntegerComponent::Unsigned(value) => u64::try_from(value).map_err(|_| shape_error(name)),
    }
}

fn default_fft_axis(shape: &Shape) -> u64 {
    shape
        .dimensions()
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|axis| u64::try_from(axis + 1).ok())
        .unwrap_or(2)
}

fn is_numeric_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::Logical(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Integer(_)
            )
    )
}

#[allow(clippy::cast_precision_loss)]
fn integer_component_f64(value: IntegerComponent) -> f64 {
    match value {
        IntegerComponent::Signed(value) => value as f64,
        IntegerComponent::Unsigned(value) => value as f64,
    }
}

fn reserved_values<T>(name: &str, length: u64) -> Result<Vec<T>, BuiltinError> {
    let length = checked_host_length(name, length)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| shape_error(name))?;
    Ok(output)
}

fn checked_host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| shape_error(name))
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

fn fft_error(name: &str, error: FftError) -> BuiltinError {
    let category = match error.kind {
        FftErrorKind::Cancelled => BuiltinErrorCategory::Cancelled,
        FftErrorKind::InvalidShape => BuiltinErrorCategory::Domain,
        FftErrorKind::ProviderFailure => BuiltinErrorCategory::Other,
    };
    BuiltinError::new(category, format!("`{name}` FFT provider failed: {error}"))
}

fn shape_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not represent the requested transform shape"),
    )
}

fn argument_count_error(name: &str, expected: &str, actual: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::ArgumentCount,
        format!("built-in `{name}` expects {expected} inputs but received {actual}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_default_dimension_follows_row_vector_orientation() {
        let shape = Shape::new([1, 1]).unwrap();
        assert_eq!(default_fft_axis(&shape), 2);
        let empty = Shape::new([0, 0]).unwrap();
        assert_eq!(default_fft_axis(&empty), 1);
    }

    #[test]
    fn conjugate_symmetry_reflects_only_transformed_axes() {
        let shape = Shape::new([1, 3]).unwrap();
        let values = [
            FftComplex64::new(1.0, 0.0),
            FftComplex64::new(2.0, 3.0),
            FftComplex64::new(2.0, -3.0),
        ];
        assert!(conjugate_symmetric_f64(&values, &shape, &[1]));
        assert!(!conjugate_symmetric_f64(&values, &shape, &[0]));
    }
}

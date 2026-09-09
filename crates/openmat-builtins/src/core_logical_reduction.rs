use openmat_array::{ArrayData, IntegerComponent, Logical, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{StringValue, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, exact_real_integer_scalar, expect_argument_count_range,
    expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy)]
enum LogicalReduction {
    Any,
    All,
}

impl LogicalReduction {
    const fn name(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::All => "all",
        }
    }

    const fn identity(self) -> bool {
        matches!(self, Self::All)
    }

    const fn combine(self, accumulator: bool, value: bool) -> bool {
        match self {
            Self::Any => accumulator || value,
            Self::All => accumulator && value,
        }
    }
}

pub(super) fn any_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    logical_reduction_builtin(LogicalReduction::Any, arguments, context)
}

pub(super) fn all_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    logical_reduction_builtin(LogicalReduction::All, arguments, context)
}

fn logical_reduction_builtin(
    reduction: LogicalReduction,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = reduction.name();
    expect_argument_count_range(name, arguments, 1, 2)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = arguments[0].dimensions().ok_or_else(|| {
        type_error(
            name,
            1,
            "numeric, logical, char, or integer array",
            &arguments[0],
        )
    })?;
    let selection = arguments.get(1).map_or_else(
        || {
            Ok(DimensionSelection::Dimensions(vec![default_dimension(
                dimensions,
            )]))
        },
        |value| dimension_selection(name, value, context),
    )?;
    let selected = match selection {
        DimensionSelection::All => {
            let dimension_count = dimensions.len();
            let mut selected = Vec::new();
            selected.try_reserve_exact(dimension_count).map_err(|_| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("`{name}` cannot allocate its reduction dimension list"),
                )
            })?;
            for index in 0..dimension_count {
                let dimension = u64::try_from(index)
                    .ok()
                    .and_then(|index| index.checked_add(1))
                    .ok_or_else(|| dimension_error(name))?;
                selected.push(dimension);
            }
            selected
        }
        DimensionSelection::Dimensions(dimensions) => dimensions,
    };
    let output_dimensions = reduced_dimensions(name, dimensions, &selected, context)?;
    let output_shape = Shape::new(output_dimensions)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))?;
    let output_length = checked_host_length(name, output_shape.numel())?;
    let mut output = filled_logicals(name, output_length, reduction.identity(), context)?;
    reduce_input(
        name,
        reduction,
        &arguments[0],
        dimensions,
        output_shape.dimensions(),
        &selected,
        &mut output,
        context,
    )?;
    context.check_cancelled()?;
    if output_shape.numel() == 1 {
        let value = output.first().copied().ok_or_else(|| mapping_error(name))?;
        Ok(vec![Value::Logical(value)])
    } else {
        let values = output.into_iter().map(Logical::from).collect();
        openmat_array::DenseArray::from_vec(output_shape, values)
            .map(ArrayData::Logical)
            .map(Value::Array)
            .map(|value| vec![value])
            .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))
    }
}

enum DimensionSelection {
    All,
    Dimensions(Vec<u64>),
}

fn dimension_selection(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<DimensionSelection, BuiltinError> {
    if is_all_keyword(value) {
        return Ok(DimensionSelection::All);
    }
    let dimensions = dimension_values(name, value, context)?;
    if dimensions.is_empty() {
        return Err(dimension_error(name));
    }
    let mut unique = Vec::new();
    unique.try_reserve_exact(dimensions.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate its reduction dimension list"),
        )
    })?;
    for (index, dimension) in dimensions.into_iter().enumerate() {
        check_cancelled_at(context, index)?;
        if unique.contains(&dimension) {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("dimension inputs to `{name}` must be unique"),
            ));
        }
        unique.push(dimension);
    }
    Ok(DimensionSelection::Dimensions(unique))
}

fn is_all_keyword(value: &Value) -> bool {
    let expected = [u16::from(b'a'), u16::from(b'l'), u16::from(b'l')];
    match value {
        Value::String(StringValue::Scalar(value)) => {
            !value.is_missing() && value.code_units() == expected
        }
        Value::String(StringValue::Array(value)) if value.numel() == 1 => value
            .as_slice()
            .first()
            .is_some_and(|value| !value.is_missing() && value.code_units() == expected),
        Value::Array(ArrayData::Char(value))
            if value.shape().ndims() == 2 && value.shape().extent(0) == 1 =>
        {
            value
                .as_slice()
                .iter()
                .map(|value| value.get())
                .eq(expected)
        }
        _ => false,
    }
}

fn dimension_values(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    if let Some(component) = exact_real_integer_scalar(value) {
        return checked_integer_dimension(name, component).map(|value| vec![value]);
    }
    if let Some(value) = value.as_real_number() {
        return checked_float_dimension(name, value).map(|value| vec![value]);
    }
    if let Some(value) = value.as_real_single() {
        return checked_float_dimension(name, f64::from(value)).map(|value| vec![value]);
    }
    let length = value
        .numel()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            type_error(
                name,
                2,
                "positive integer dimension scalar or vector",
                value,
            )
        })?;
    let mut dimensions = Vec::new();
    dimensions.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate a dimension vector of length {length}"),
        )
    })?;
    match value {
        Value::Array(ArrayData::F64(array)) => {
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                dimensions.push(checked_float_dimension(name, value)?);
            }
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                if value.im != 0.0 {
                    return Err(dimension_error(name));
                }
                dimensions.push(checked_float_dimension(name, value.re)?);
            }
        }
        Value::Array(ArrayData::Logical(array)) => {
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                dimensions.push(checked_float_dimension(name, f64::from(value.get()))?);
            }
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                dimensions.push(checked_integer_dimension(name, value.real_component())?);
            }
        }
        Value::Array(ArrayData::F32(array)) => {
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                dimensions.push(checked_float_dimension(name, f64::from(value))?);
            }
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                if value.im != 0.0 {
                    return Err(dimension_error(name));
                }
                dimensions.push(checked_float_dimension(name, f64::from(value.re))?);
            }
        }
        _ => {
            return Err(type_error(
                name,
                2,
                "positive integer dimension scalar or vector, or 'all'",
                value,
            ));
        }
    }
    Ok(dimensions)
}

fn checked_integer_dimension(name: &str, value: IntegerComponent) -> Result<u64, BuiltinError> {
    let value = match value {
        IntegerComponent::Signed(value) => u64::try_from(value).ok(),
        IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
    };
    value
        .filter(|value| *value > 0)
        .ok_or_else(|| dimension_error(name))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn checked_float_dimension(name: &str, value: f64) -> Result<u64, BuiltinError> {
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(dimension_error(name));
    }
    Ok(value as u64)
}

fn dimension_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("dimension inputs to `{name}` must be unique positive integers"),
    )
}

fn default_dimension(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|index| u64::try_from(index).ok())
        .and_then(|index| index.checked_add(1))
        .unwrap_or(1)
}

fn reduced_dimensions(
    name: &str,
    input: &[u64],
    selected: &[u64],
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    let mut output = Vec::new();
    output.try_reserve_exact(input.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate its output dimensions"),
        )
    })?;
    for (index, extent) in input.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let dimension = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| dimension_error(name))?;
        output.push(if selected.contains(&dimension) {
            1
        } else {
            extent
        });
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn reduce_input(
    name: &str,
    reduction: LogicalReduction,
    input: &Value,
    input_dimensions: &[u64],
    output_dimensions: &[u64],
    selected: &[u64],
    output: &mut [bool],
    context: &BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    macro_rules! reduce_slice {
        ($slice:expr, $truth:expr) => {{
            for (index, value) in $slice.iter().enumerate() {
                check_cancelled_at(context, index)?;
                reduce_one(
                    name,
                    reduction,
                    index,
                    $truth(value),
                    input_dimensions,
                    output_dimensions,
                    selected,
                    output,
                )?;
            }
        }};
    }
    match input {
        Value::Logical(value) => reduce_one(
            name,
            reduction,
            0,
            Some(*value),
            input_dimensions,
            output_dimensions,
            selected,
            output,
        )?,
        Value::Double(value) => reduce_one(
            name,
            reduction,
            0,
            finite_truth(*value),
            input_dimensions,
            output_dimensions,
            selected,
            output,
        )?,
        Value::Complex(value) => reduce_one(
            name,
            reduction,
            0,
            complex_truth(value.real, value.imaginary),
            input_dimensions,
            output_dimensions,
            selected,
            output,
        )?,
        Value::Array(ArrayData::F32(array)) => {
            reduce_slice!(array.as_slice(), |value: &f32| finite_truth(f64::from(
                *value
            )));
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            reduce_slice!(array.as_slice(), |value: &openmat_array::Complex32| {
                complex_truth(f64::from(value.re), f64::from(value.im))
            });
        }
        Value::Array(ArrayData::Logical(array)) => {
            reduce_slice!(array.as_slice(), |value: &Logical| Some(value.get()));
        }
        Value::Array(ArrayData::F64(array)) => {
            reduce_slice!(array.as_slice(), |value: &f64| finite_truth(*value));
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            reduce_slice!(array.as_slice(), |value: &openmat_array::Complex64| {
                complex_truth(value.re, value.im)
            });
        }
        Value::Array(ArrayData::Char(array)) => {
            reduce_slice!(array.as_slice(), |value: &openmat_array::CharCodeUnit| {
                Some(value.get() != 0)
            });
        }
        Value::Array(ArrayData::Integer(array)) => {
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                let truth = !value.real_component().is_zero()
                    || value
                        .imaginary_component()
                        .is_some_and(|value| !value.is_zero());
                reduce_one(
                    name,
                    reduction,
                    index,
                    Some(truth),
                    input_dimensions,
                    output_dimensions,
                    selected,
                    output,
                )?;
            }
        }
        value => {
            return Err(type_error(
                name,
                1,
                "numeric, logical, char, or integer array",
                value,
            ));
        }
    }
    Ok(())
}

fn finite_truth(value: f64) -> Option<bool> {
    (!value.is_nan()).then_some(value != 0.0)
}

fn complex_truth(real: f64, imaginary: f64) -> Option<bool> {
    if real.is_nan() || imaginary.is_nan() {
        None
    } else {
        Some(real != 0.0 || imaginary != 0.0)
    }
}

#[allow(clippy::too_many_arguments)]
fn reduce_one(
    name: &str,
    reduction: LogicalReduction,
    input_offset: usize,
    value: Option<bool>,
    input_dimensions: &[u64],
    output_dimensions: &[u64],
    selected: &[u64],
    output: &mut [bool],
) -> Result<(), BuiltinError> {
    let Some(value) = value else {
        return Ok(());
    };
    let mut input_offset = u64::try_from(input_offset).map_err(|_| mapping_error(name))?;
    let mut output_offset = 0_u64;
    let mut output_stride = 1_u64;
    for (index, extent) in input_dimensions.iter().copied().enumerate() {
        let coordinate = if extent == 0 {
            0
        } else {
            input_offset % extent
        };
        if extent != 0 {
            input_offset /= extent;
        }
        let dimension = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| mapping_error(name))?;
        let coordinate = if selected.contains(&dimension) {
            0
        } else {
            coordinate
        };
        output_offset = coordinate
            .checked_mul(output_stride)
            .and_then(|value| output_offset.checked_add(value))
            .ok_or_else(|| mapping_error(name))?;
        output_stride = output_stride
            .checked_mul(output_dimensions.get(index).copied().unwrap_or(1))
            .ok_or_else(|| mapping_error(name))?;
    }
    let output_offset = usize::try_from(output_offset).map_err(|_| mapping_error(name))?;
    let accumulator = output
        .get_mut(output_offset)
        .ok_or_else(|| mapping_error(name))?;
    *accumulator = reduction.combine(*accumulator, value);
    Ok(())
}

fn filled_logicals(
    name: &str,
    length: usize,
    value: bool,
    context: &BuiltinContext<'_>,
) -> Result<Vec<bool>, BuiltinError> {
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {length} logical elements"),
        )
    })?;
    while output.len() < length {
        context.check_cancelled()?;
        let target = output
            .len()
            .saturating_add(CANCELLATION_CHECK_INTERVAL)
            .min(length);
        output.resize(target, value);
    }
    Ok(output)
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
        format!("`{name}` computed an invalid checked reduction offset"),
    )
}

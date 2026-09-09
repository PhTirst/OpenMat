#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use openmat_array::{ArrayData, DenseArray, IntegerArrayData, IntegerComponent, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{FieldName, StringValue, StructArray, Value};

use crate::{array_error, expect_argument_count_range, expect_max_outputs, type_error};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputClass {
    Double,
    Single,
}

pub(super) fn rand_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    random_array("rand", arguments, context, |context| {
        context.random_uniform()
    })
}

pub(super) fn randn_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    random_array("randn", arguments, context, |context| {
        context.random_normal()
    })
}

fn random_array(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    mut draw: impl FnMut(&mut BuiltinContext<'_>) -> Result<f64, BuiltinError>,
) -> BuiltinResult {
    expect_max_outputs(name, context, 1)?;
    let (shape_arguments, output_class) = split_output_class(name, arguments)?;
    let dimensions = crate::core_shape::constructor_dimensions(name, shape_arguments, context)?;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let length = usize::try_from(shape.numel()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` output length exceeds host limits"),
        )
    })?;
    match output_class {
        OutputClass::Double => {
            let mut values = Vec::new();
            values
                .try_reserve_exact(length)
                .map_err(|_| allocation_error(name))?;
            for _ in 0..length {
                values.push(draw(context)?);
            }
            DenseArray::from_vec(shape, values)
                .map(|array| vec![Value::Array(ArrayData::F64(array))])
                .map_err(|error| array_error(&error))
        }
        OutputClass::Single => {
            let mut values = Vec::new();
            values
                .try_reserve_exact(length)
                .map_err(|_| allocation_error(name))?;
            for _ in 0..length {
                values.push(draw(context)? as f32);
            }
            DenseArray::from_vec(shape, values)
                .map(|array| vec![Value::Array(ArrayData::F32(array))])
                .map_err(|error| array_error(&error))
        }
    }
}

pub(super) fn randi_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("randi", arguments, 1, usize::MAX)?;
    expect_max_outputs("randi", context, 1)?;
    let (minimum, maximum) = integer_range(&arguments[0])?;
    let (shape_arguments, output_class) = split_output_class("randi", &arguments[1..])?;
    let dimensions = crate::core_shape::constructor_dimensions("randi", shape_arguments, context)?;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let length = usize::try_from(shape.numel()).map_err(|_| allocation_error("randi"))?;
    let span = (maximum - minimum + 1) as f64;
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| allocation_error("randi"))?;
    for _ in 0..length {
        let offset = (context.random_uniform()? * span).floor();
        values.push(minimum as f64 + offset);
    }
    match output_class {
        OutputClass::Double => DenseArray::from_vec(shape, values)
            .map(|array| vec![Value::Array(ArrayData::F64(array))])
            .map_err(|error| array_error(&error)),
        OutputClass::Single => DenseArray::from_vec(
            shape,
            values.into_iter().map(|value| value as f32).collect(),
        )
        .map(|array| vec![Value::Array(ArrayData::F32(array))])
        .map_err(|error| array_error(&error)),
    }
}

pub(super) fn randperm_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("randperm", arguments, 1, 2)?;
    expect_max_outputs("randperm", context, 1)?;
    let n = nonnegative_integer("randperm", 1, &arguments[0])?;
    let count = arguments
        .get(1)
        .map_or(Ok(n), |value| nonnegative_integer("randperm", 2, value))?;
    if count > n {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`randperm` sample count cannot exceed the population",
        ));
    }
    let n = usize::try_from(n).map_err(|_| allocation_error("randperm"))?;
    let count = usize::try_from(count).map_err(|_| allocation_error("randperm"))?;
    let mut values = (1..=n).collect::<Vec<_>>();
    for index in 0..count {
        let remaining = n - index;
        let offset = (context.random_uniform()? * remaining as f64).floor() as usize;
        values.swap(index, index + offset.min(remaining - 1));
    }
    let selected = values
        .into_iter()
        .take(count)
        .map(|value| value as f64)
        .collect::<Vec<_>>();
    let shape = Shape::new([1, count as u64]).map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, selected)
        .map(|array| vec![Value::Array(ArrayData::F64(array))])
        .map_err(|error| array_error(&error))
}

pub(super) fn rng_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("rng", arguments, 0, 2)?;
    expect_max_outputs("rng", context, 1)?;
    let previous = context.random_snapshot()?;
    match arguments {
        [] => {}
        [value] => {
            if let Some(seed) = exact_u32(value) {
                context.random_reseed(seed)?;
            } else {
                match text_argument("rng", 1, value)?.as_str() {
                    "default" => context.random_reseed(0)?,
                    "shuffle" => context.random_shuffle()?,
                    _ => {
                        return Err(BuiltinError::new(
                            BuiltinErrorCategory::Domain,
                            "`rng` supports a uint32 seed, 'default', or 'shuffle'",
                        ));
                    }
                }
            }
        }
        [seed, generator] => {
            if text_argument("rng", 2, generator)? != "twister" {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "OpenMat currently supports only the `twister` generator",
                ));
            }
            context.random_reseed(
                exact_u32(seed)
                    .ok_or_else(|| type_error("rng", 1, "nonnegative uint32 seed", seed))?,
            )?;
        }
        _ => unreachable!("validated argument count"),
    }
    if context.requested_outputs() == 0 {
        Ok(Vec::new())
    } else {
        Ok(vec![snapshot_value(previous)?])
    }
}

fn snapshot_value(snapshot: openmat_runtime::RandomSnapshot) -> Result<Value, BuiltinError> {
    let shape = Shape::new([625, 1]).map_err(|error| array_error(&error))?;
    let state = DenseArray::from_vec(shape, snapshot.state).map_err(|error| array_error(&error))?;
    let fields = ["Type", "Seed", "State"]
        .into_iter()
        .map(|name| {
            FieldName::new(name)
                .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let seed_shape = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    let seed = DenseArray::from_vec(seed_shape, vec![snapshot.seed])
        .map_err(|error| array_error(&error))?;
    let columns = vec![
        vec![Value::from("twister")],
        vec![Value::Array(ArrayData::Integer(IntegerArrayData::U32(
            seed,
        )))],
        vec![Value::Array(ArrayData::Integer(IntegerArrayData::U32(
            state,
        )))],
    ];
    let scalar = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    StructArray::from_columns(scalar, fields, columns)
        .map(Value::Struct)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn split_output_class<'a>(
    name: &str,
    arguments: &'a [Value],
) -> Result<(&'a [Value], OutputClass), BuiltinError> {
    let Some(last) = arguments.last() else {
        return Ok((arguments, OutputClass::Double));
    };
    let Ok(text) = text_argument(name, arguments.len(), last) else {
        return Ok((arguments, OutputClass::Double));
    };
    let output_class = match text.as_str() {
        "double" => OutputClass::Double,
        "single" => OutputClass::Single,
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` output class must be 'double' or 'single'"),
            ));
        }
    };
    Ok((&arguments[..arguments.len() - 1], output_class))
}

fn integer_range(value: &Value) -> Result<(i64, i64), BuiltinError> {
    let values = real_values(value).ok_or_else(|| {
        type_error(
            "randi",
            1,
            "positive integer maximum or two-integer range",
            value,
        )
    })?;
    let (minimum, maximum) = match values.as_slice() {
        [maximum] => (1.0, *maximum),
        [minimum, maximum] => (*minimum, *maximum),
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 1 to `randi` must be a scalar or two-element range",
            ));
        }
    };
    if !minimum.is_finite()
        || !maximum.is_finite()
        || minimum.fract() != 0.0
        || maximum.fract() != 0.0
        || minimum > maximum
        || minimum < i64::MIN as f64
        || maximum > i64::MAX as f64
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`randi` range bounds must be ordered finite integers",
        ));
    }
    Ok((minimum as i64, maximum as i64))
}

fn real_values(value: &Value) -> Option<Vec<f64>> {
    match value {
        Value::Double(value) => Some(vec![*value]),
        Value::Array(ArrayData::F32(array)) => Some(
            array
                .as_slice()
                .iter()
                .map(|value| f64::from(*value))
                .collect(),
        ),
        Value::Array(ArrayData::F64(array)) => Some(array.as_slice().to_vec()),
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => Some(
            array
                .elements()
                .map(|value| match value.real_component() {
                    IntegerComponent::Signed(value) => value as f64,
                    IntegerComponent::Unsigned(value) => value as f64,
                })
                .collect(),
        ),
        _ => None,
    }
}

fn nonnegative_integer(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    let Some(value) =
        real_values(value).and_then(|values| values.first().copied().filter(|_| values.len() == 1))
    else {
        return Err(type_error(
            name,
            position,
            "nonnegative integer scalar",
            value,
        ));
    };
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value >= u64::MAX as f64 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a nonnegative integer scalar"),
        ));
    }
    Ok(value as u64)
}

fn exact_u32(value: &Value) -> Option<u32> {
    let value = real_values(value)?;
    let value = *value.first().filter(|_| value.len() == 1)?;
    if value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= f64::from(u32::MAX) {
        Some(value as u32)
    } else {
        None
    }
}

fn text_argument(name: &str, position: usize, value: &Value) -> Result<String, BuiltinError> {
    let units = match value {
        Value::String(StringValue::Scalar(value)) if !value.is_missing() => value.code_units(),
        Value::Array(ArrayData::Char(array))
            if array.shape().ndims() == 2 && array.shape().extent(0) == 1 =>
        {
            return String::from_utf16(
                &array
                    .as_slice()
                    .iter()
                    .map(|unit| unit.get())
                    .collect::<Vec<_>>(),
            )
            .map(|text| text.to_ascii_lowercase())
            .map_err(|_| type_error(name, position, "ASCII text scalar", value));
        }
        _ => return Err(type_error(name, position, "text scalar", value)),
    };
    String::from_utf16(units)
        .map(|text| text.to_ascii_lowercase())
        .map_err(|_| type_error(name, position, "ASCII text scalar", value))
}

fn allocation_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` output allocation failed"),
    )
}

#[cfg(test)]
mod tests {
    use openmat_runtime::{CancellationToken, NullOutput, RandomSession};

    use super::*;

    #[test]
    fn randn_supports_matrix_and_single_overloads() {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut random = RandomSession::new();
        let mut context =
            BuiltinContext::with_random_service(1, &cancellation, &mut output, &mut random);
        let values = randn_builtin(
            &[
                Value::Double(2.0),
                Value::Double(3.0),
                Value::from("single"),
            ],
            &mut context,
        )
        .unwrap();
        assert!(matches!(
            &values[0],
            Value::Array(ArrayData::F32(array)) if array.shape().dimensions() == [2, 3]
        ));
    }

    #[test]
    fn rng_seed_replays_uniform_arrays() {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut random = RandomSession::new();
        let mut context =
            BuiltinContext::with_random_service(0, &cancellation, &mut output, &mut random);
        rng_builtin(&[Value::Double(7.0)], &mut context).unwrap();
        let first = rand_builtin(&[Value::Double(1.0), Value::Double(3.0)], &mut context).unwrap();
        rng_builtin(&[Value::Double(7.0)], &mut context).unwrap();
        let second = rand_builtin(&[Value::Double(1.0), Value::Double(3.0)], &mut context).unwrap();
        assert_eq!(first, second);
    }
}

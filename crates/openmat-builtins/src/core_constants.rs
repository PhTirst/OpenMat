use openmat_array::{ArrayData, DenseArray, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{Complex64, Value};

use crate::core_elementary::map_values;
use crate::core_shape::constructor_dimensions;
use crate::{
    array_error, expect_argument_count, expect_argument_count_range, expect_max_outputs, type_error,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FloatClass {
    Double,
    Single,
}

pub(crate) fn pi_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("pi", arguments, 0)?;
    expect_max_outputs("pi", context, 1)?;
    context.check_cancelled()?;
    Ok(vec![Value::Double(std::f64::consts::PI)])
}

pub(crate) fn i_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    imaginary_unit_builtin("i", arguments, context)
}

pub(crate) fn j_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    imaginary_unit_builtin("j", arguments, context)
}

fn imaginary_unit_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 0)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    Ok(vec![Value::Complex(Complex64::new(0.0, 1.0))])
}

pub(crate) fn eps_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("eps", arguments, 0, 1)?;
    expect_max_outputs("eps", context, 1)?;
    context.check_cancelled()?;
    let output = match arguments.first() {
        None => Value::Double(f64::EPSILON),
        Some(value) if is_text(value) => match float_class_argument("eps", 1, value)? {
            FloatClass::Double => Value::Double(f64::EPSILON),
            FloatClass::Single => single_scalar(f32::EPSILON)?,
        },
        Some(Value::Double(value)) => Value::Double(eps_f64(*value)),
        Some(Value::Complex(value)) => Value::Double(eps_f64(value.real.hypot(value.imaginary))),
        Some(Value::Array(ArrayData::F64(array))) => {
            let values = map_values("eps", array.as_slice(), context, |value| eps_f64(*value))?;
            dense_f64(array.shape().clone(), values)?
        }
        Some(Value::Array(ArrayData::ComplexF64(array))) => {
            let values = map_values("eps", array.as_slice(), context, |value| {
                eps_f64(value.re.hypot(value.im))
            })?;
            dense_f64(array.shape().clone(), values)?
        }
        Some(Value::Array(ArrayData::F32(array))) => {
            let values = map_values("eps", array.as_slice(), context, |value| eps_f32(*value))?;
            dense_f32(array.shape().clone(), values)?
        }
        Some(Value::Array(ArrayData::ComplexF32(array))) => {
            let values = map_values("eps", array.as_slice(), context, |value| {
                eps_f32(value.re.hypot(value.im))
            })?;
            dense_f32(array.shape().clone(), values)?
        }
        Some(value) => return Err(type_error("eps", 1, "double or single array", value)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(crate) fn inf_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    filled_float_constant("Inf", arguments, context, f64::INFINITY, f32::INFINITY)
}

pub(crate) fn nan_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    filled_float_constant("NaN", arguments, context, f64::NAN, f32::NAN)
}

pub(crate) fn realmin_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    floating_limit(
        "realmin",
        arguments,
        context,
        f64::MIN_POSITIVE,
        f32::MIN_POSITIVE,
    )
}

pub(crate) fn realmax_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    floating_limit("realmax", arguments, context, f64::MAX, f32::MAX)
}

pub(crate) fn flintmax_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    floating_limit(
        "flintmax",
        arguments,
        context,
        9_007_199_254_740_992.0,
        16_777_216.0,
    )
}

fn floating_limit(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    double: f64,
    single: f32,
) -> BuiltinResult {
    expect_argument_count_range(name, arguments, 0, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let output = match arguments.first() {
        None => Value::Double(double),
        Some(value) => match float_class_argument(name, 1, value)? {
            FloatClass::Double => Value::Double(double),
            FloatClass::Single => single_scalar(single)?,
        },
    };
    Ok(vec![output])
}

fn filled_float_constant(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    double: f64,
    single: f32,
) -> BuiltinResult {
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let (dimension_arguments, class) = match arguments.last() {
        Some(value) if is_text(value) => (
            &arguments[..arguments.len() - 1],
            float_class_argument(name, arguments.len(), value)?,
        ),
        _ => (arguments, FloatClass::Double),
    };
    if arguments.is_empty() {
        return Ok(vec![Value::Double(double)]);
    }
    let dimensions = constructor_dimensions(name, dimension_arguments, context)?;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let output = match class {
        FloatClass::Double => DenseArray::from_elem(shape, double)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))?,
        FloatClass::Single => DenseArray::from_elem(shape, single)
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))?,
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn float_class_argument(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<FloatClass, BuiltinError> {
    let code_units = text_code_units(value).ok_or_else(|| {
        type_error(
            name,
            position,
            "floating-point class-name char row or string scalar",
            value,
        )
    })?;
    match code_units.as_slice() {
        [100, 111, 117, 98, 108, 101] => Ok(FloatClass::Double),
        [115, 105, 110, 103, 108, 101] => Ok(FloatClass::Single),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be 'double' or 'single'"),
        )),
    }
}

fn is_text(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Array(ArrayData::Char(_)))
}

fn text_code_units(value: &Value) -> Option<Vec<u16>> {
    match value {
        Value::String(value) => {
            let element = value.as_scalar()?;
            (!element.is_missing()).then(|| element.code_units().to_vec())
        }
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            Some(array.as_slice().iter().map(|value| value.get()).collect())
        }
        _ => None,
    }
}

fn dense_f64(shape: Shape, values: Vec<f64>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn dense_f32(shape: Shape, values: Vec<f32>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F32)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn single_scalar(value: f32) -> Result<Value, BuiltinError> {
    dense_f32(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
}

fn eps_f64(value: f64) -> f64 {
    let exponent = (value.abs().to_bits() >> 52) & 0x7ff;
    match exponent {
        0 => f64::from_bits(1),
        0x7ff => f64::NAN,
        exponent => spacing_f64(i32::try_from(exponent).expect("f64 exponent fits i32") - 1023),
    }
}

fn spacing_f64(exponent: i32) -> f64 {
    let spacing_exponent = exponent - 52;
    if spacing_exponent >= -1022 {
        let bits = u64::try_from(spacing_exponent + 1023).expect("normal exponent is positive");
        f64::from_bits(bits << 52)
    } else {
        let shift = u32::try_from(spacing_exponent + 1074).expect("subnormal shift is positive");
        f64::from_bits(1_u64 << shift)
    }
}

fn eps_f32(value: f32) -> f32 {
    let exponent = (value.abs().to_bits() >> 23) & 0xff;
    match exponent {
        0 => f32::from_bits(1),
        0xff => f32::NAN,
        exponent => spacing_f32(i32::try_from(exponent).expect("f32 exponent fits i32") - 127),
    }
}

fn spacing_f32(exponent: i32) -> f32 {
    let spacing_exponent = exponent - 23;
    if spacing_exponent >= -126 {
        let bits = u32::try_from(spacing_exponent + 127).expect("normal exponent is positive");
        f32::from_bits(bits << 23)
    } else {
        let shift = u32::try_from(spacing_exponent + 149).expect("subnormal shift is positive");
        f32::from_bits(1_u32 << shift)
    }
}

use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, IntegerComponent,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::{
    exact_real_integer_scalar, expect_argument_count_range, expect_max_outputs, type_error,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum RoundMode {
    Decimals,
    Significant,
}

pub(super) fn round_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("round", arguments, 1, 3)?;
    expect_max_outputs("round", context, 1)?;
    context.check_cancelled()?;
    let digits = arguments.get(1).map_or(Ok(0_i128), round_digits)?;
    let mode = arguments
        .get(2)
        .map_or(Ok(RoundMode::Decimals), round_mode)?;
    if mode == RoundMode::Significant && digits < 1 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "significant digits supplied to `round` must be a positive integer",
        ));
    }
    let multiple_arguments = arguments.len() > 1;
    let output = match &arguments[0] {
        Value::Logical(value) if !multiple_arguments => Value::Double(f64::from(*value)),
        Value::Double(value) => Value::Double(round_f64(*value, digits, mode)),
        Value::Complex(value) => crate::core_elementary::complex_scalar_output(round_complex_f64(
            ArrayComplex64::new(value.real, value.imaginary),
            digits,
            mode,
        )),
        Value::Array(ArrayData::F32(array)) => {
            let values =
                crate::core_elementary::map_values("round", array.as_slice(), context, |value| {
                    round_f32(*value, digits, mode)
                })?;
            openmat_array::DenseArray::from_vec(array.shape().clone(), values)
                .map(ArrayData::F32)
                .map(Value::Array)
                .map_err(|error| array_error(&error))?
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            let values =
                crate::core_elementary::map_values("round", array.as_slice(), context, |value| {
                    round_complex_f32(*value, digits, mode)
                })?;
            crate::core_elementary::complex_f32_array_output(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::Logical(array)) if !multiple_arguments => {
            let values =
                crate::core_elementary::map_values("round", array.as_slice(), context, |value| {
                    f64::from(value.get())
                })?;
            openmat_array::DenseArray::from_vec(array.shape().clone(), values)
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(|error| array_error(&error))?
        }
        Value::Array(ArrayData::F64(array)) => {
            let values =
                crate::core_elementary::map_values("round", array.as_slice(), context, |value| {
                    round_f64(*value, digits, mode)
                })?;
            openmat_array::DenseArray::from_vec(array.shape().clone(), values)
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(|error| array_error(&error))?
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let values =
                crate::core_elementary::map_values("round", array.as_slice(), context, |value| {
                    round_complex_f64(*value, digits, mode)
                })?;
            crate::core_elementary::complex_f64_array_output(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::Char(array)) if !multiple_arguments => {
            let values =
                crate::core_elementary::map_values("round", array.as_slice(), context, |value| {
                    f64::from(value.get())
                })?;
            openmat_array::DenseArray::from_vec(array.shape().clone(), values)
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(|error| array_error(&error))?
        }
        Value::Array(ArrayData::Integer(_)) if !multiple_arguments => arguments[0].clone(),
        value if multiple_arguments => {
            return Err(type_error(
                "round",
                1,
                "double or single input when digits are supplied",
                value,
            ));
        }
        value => {
            return Err(type_error(
                "round",
                1,
                "numeric, logical, char, or integer array",
                value,
            ));
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn round_digits(value: &Value) -> Result<i128, BuiltinError> {
    if matches!(
        value,
        Value::Logical(_) | Value::Array(ArrayData::Logical(_))
    ) {
        return Err(digits_error());
    }
    if let Some(value) = exact_real_integer_scalar(value) {
        return match value {
            IntegerComponent::Signed(value) => Ok(value),
            IntegerComponent::Unsigned(value) => i128::try_from(value).map_err(|_| digits_error()),
        };
    }
    if let Some(value) = value.as_real_single().map(f64::from) {
        return checked_float_digits(value);
    }
    match value {
        Value::Double(value) => checked_float_digits(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            checked_float_digits(array.as_slice()[0])
        }
        _ => Err(digits_error()),
    }
}

fn checked_float_digits(value: f64) -> Result<i128, BuiltinError> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(digits_error());
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as i128)
}

fn round_mode(value: &Value) -> Result<RoundMode, BuiltinError> {
    match keyword(value).as_deref() {
        Some("decimals") => Ok(RoundMode::Decimals),
        Some("significant") => Ok(RoundMode::Significant),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 3 to `round` must be 'decimals' or 'significant'",
        )),
    }
}

fn keyword(value: &Value) -> Option<String> {
    if let Some(value) = value.as_string_scalar()
        && !value.is_missing()
    {
        return Some(value.to_utf8_lossy().to_ascii_lowercase());
    }
    match value {
        Value::Array(ArrayData::Char(array))
            if array.shape().ndims() == 2 && array.shape().extent(0) == 1 =>
        {
            String::from_utf16(
                &array
                    .as_slice()
                    .iter()
                    .map(|value| value.get())
                    .collect::<Vec<_>>(),
            )
            .ok()
            .map(|value| value.to_ascii_lowercase())
        }
        _ => None,
    }
}

fn round_complex_f64(value: ArrayComplex64, digits: i128, mode: RoundMode) -> ArrayComplex64 {
    ArrayComplex64::new(
        round_f64(value.re, digits, mode),
        round_f64(value.im, digits, mode),
    )
}

fn round_complex_f32(value: ArrayComplex32, digits: i128, mode: RoundMode) -> ArrayComplex32 {
    ArrayComplex32::new(
        round_f32(value.re, digits, mode),
        round_f32(value.im, digits, mode),
    )
}

#[allow(clippy::cast_possible_truncation)]
fn round_f64(value: f64, digits: i128, mode: RoundMode) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    let decimals = match mode {
        RoundMode::Decimals => digits,
        RoundMode::Significant => digits
            .saturating_sub(1)
            .saturating_sub(value.abs().log10().floor() as i128),
    };
    scale_round_f64(value, decimals)
}

#[allow(clippy::cast_possible_truncation)]
fn round_f32(value: f32, digits: i128, mode: RoundMode) -> f32 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    let decimals = match mode {
        RoundMode::Decimals => digits,
        RoundMode::Significant => digits
            .saturating_sub(1)
            .saturating_sub(value.abs().log10().floor() as i128),
    };
    scale_round_f32(value, decimals)
}

fn scale_round_f64(value: f64, decimals: i128) -> f64 {
    if decimals > 308 {
        return value;
    }
    if decimals < -324 {
        return 0.0_f64.copysign(value);
    }
    if decimals >= 0 {
        let scale = 10.0_f64.powi(i32::try_from(decimals).unwrap_or(i32::MAX));
        let scaled = value * scale;
        if scaled.is_finite() {
            round_scaled_f64(scaled) / scale
        } else {
            value
        }
    } else {
        let scale = 10.0_f64.powi(i32::try_from(-decimals).unwrap_or(i32::MAX));
        if scale.is_infinite() {
            0.0_f64.copysign(value)
        } else {
            round_scaled_f64(value / scale) * scale
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
fn scale_round_f32(value: f32, decimals: i128) -> f32 {
    if decimals > 38 {
        return value;
    }
    if decimals < -46 {
        return 0.0_f32.copysign(value);
    }
    if decimals >= 0 {
        let scale = 10.0_f64.powi(i32::try_from(decimals).unwrap_or(i32::MAX));
        let scaled = f64::from(value) * scale;
        if scaled.is_finite() {
            (round_scaled_f64(scaled) / scale) as f32
        } else {
            value
        }
    } else {
        let scale = 10.0_f64.powi(i32::try_from(-decimals).unwrap_or(i32::MAX));
        if scale.is_infinite() {
            0.0_f32.copysign(value)
        } else {
            (round_scaled_f64(f64::from(value) / scale) * scale) as f32
        }
    }
}

fn round_scaled_f64(value: f64) -> f64 {
    let magnitude = value.abs();
    let lower = magnitude.floor();
    let half = lower + 0.5;
    let rounded = if magnitude < half && half - magnitude <= magnitude.max(1.0) * f64::EPSILON {
        lower + 1.0
    } else {
        magnitude.round()
    };
    rounded.copysign(value)
}

fn digits_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "input 2 to `round` must be a finite real integer scalar",
    )
}

fn array_error(error: &openmat_array::ArrayError) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string())
}

use openmat_array::{ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count_range,
    expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Precision {
    Double,
    Single,
}

#[derive(Clone, Copy)]
struct ScalarEndpoint {
    re: f64,
    im: f64,
    precision: Precision,
}

#[derive(Clone, Copy)]
struct SequenceCount {
    length: u64,
    interpolation_re: f64,
    interpolation_im: f64,
    nan_count: bool,
}

pub(super) fn linspace_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    sequence_builtin("linspace", false, 100, arguments, context)
}

pub(super) fn logspace_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    sequence_builtin("logspace", true, 50, arguments, context)
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn sequence_builtin(
    name: &'static str,
    logarithmic: bool,
    default_count: u64,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range(name, arguments, 2, 3)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let start = scalar_endpoint(name, 1, &arguments[0])?;
    let mut end = scalar_endpoint(name, 2, &arguments[1])?;
    let count = arguments.get(2).map_or(
        Ok(SequenceCount {
            length: default_count,
            interpolation_re: default_count as f64,
            interpolation_im: 0.0,
            nan_count: false,
        }),
        |value| sequence_count(name, value),
    )?;
    let precision = if start.precision == Precision::Single || end.precision == Precision::Single {
        Precision::Single
    } else {
        Precision::Double
    };
    if logarithmic && end.im == 0.0 {
        let is_pi = match precision {
            Precision::Double => end.re == std::f64::consts::PI,
            Precision::Single => end.re as f32 == std::f32::consts::PI,
        };
        if is_pi {
            end.re = match precision {
                Precision::Double => std::f64::consts::PI.log10(),
                Precision::Single => f64::from(std::f32::consts::PI.log10()),
            };
        }
    }
    let output = match precision {
        Precision::Double => sequence_f64(name, start, end, count, logarithmic, context)?,
        Precision::Single => sequence_f32(name, start, end, count, logarithmic, context)?,
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn scalar_endpoint(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<ScalarEndpoint, BuiltinError> {
    if let Some(value) = value.as_complex_single() {
        return Ok(ScalarEndpoint {
            re: f64::from(value.re),
            im: f64::from(value.im),
            precision: Precision::Single,
        });
    }
    if let Some(value) = value.as_complex_number() {
        return Ok(ScalarEndpoint {
            re: value.real,
            im: value.imaginary,
            precision: Precision::Double,
        });
    }
    if let Value::Array(ArrayData::Char(array)) = value
        && array.numel() == 1
    {
        return Ok(ScalarEndpoint {
            re: f64::from(array.as_slice()[0].get()),
            im: 0.0,
            precision: Precision::Double,
        });
    }
    Err(type_error(
        name,
        position,
        "scalar double, single, logical, or char endpoint",
        value,
    ))
}

#[allow(clippy::cast_precision_loss)]
fn sequence_count(name: &str, value: &Value) -> Result<SequenceCount, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        let signed = match value {
            openmat_array::IntegerComponent::Signed(value) => value,
            openmat_array::IntegerComponent::Unsigned(value) => {
                i128::try_from(value).map_err(|_| count_error(name))?
            }
        };
        let length = if signed <= 0 {
            0
        } else {
            u64::try_from(signed).map_err(|_| count_error(name))?
        };
        return Ok(SequenceCount {
            length,
            interpolation_re: signed as f64,
            interpolation_im: 0.0,
            nan_count: false,
        });
    }
    let (re, im) = if let Some(value) = value.as_complex_single() {
        (f64::from(value.re), f64::from(value.im))
    } else if let Some(value) = value.as_complex_number() {
        (value.real, value.imaginary)
    } else {
        return Err(type_error(name, 3, "finite numeric scalar count", value));
    };
    if re == f64::INFINITY {
        return Err(count_error(name));
    }
    if re.is_nan() {
        return Ok(SequenceCount {
            length: 1,
            interpolation_re: re,
            interpolation_im: im.floor(),
            nan_count: true,
        });
    }
    let floored_re = re.floor();
    if floored_re >= U64_EXCLUSIVE_UPPER_BOUND {
        return Err(count_error(name));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let length = if floored_re <= 0.0 {
        0
    } else {
        floored_re as u64
    };
    Ok(SequenceCount {
        length,
        interpolation_re: floored_re,
        interpolation_im: im.floor(),
        nan_count: false,
    })
}

#[allow(clippy::cast_precision_loss)]
fn sequence_f64(
    name: &str,
    start: ScalarEndpoint,
    end: ScalarEndpoint,
    count: SequenceCount,
    logarithmic: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let length = checked_length(name, count.length)?;
    let shape = Shape::new([1, count.length]).map_err(|error| array_error(&error))?;
    let mut values = reserved_values(name, length)?;
    if length == 1 {
        values.push(if count.nan_count {
            ArrayComplex64::new(
                f64::NAN,
                if count.interpolation_im == 0.0 {
                    0.0
                } else {
                    f64::NAN
                },
            )
        } else {
            ArrayComplex64::new(end.re, end.im)
        });
    } else if length > 1 {
        let denominator = ArrayComplex64::new(count.interpolation_re - 1.0, count.interpolation_im);
        for index in 0..length {
            check_cancelled_at(context, index)?;
            let value = if index == 0 {
                ArrayComplex64::new(start.re, start.im)
            } else if index + 1 == length {
                ArrayComplex64::new(end.re, end.im)
            } else {
                let ratio = complex_div_f64(ArrayComplex64::new(index as f64, 0.0), denominator);
                let delta = ArrayComplex64::new(end.re - start.re, end.im - start.im);
                complex_add_f64(
                    ArrayComplex64::new(start.re, start.im),
                    complex_mul_f64(delta, ratio),
                )
            };
            values.push(if logarithmic {
                complex_pow10_f64(value)
            } else {
                value
            });
        }
    }
    crate::core_elementary::complex_f64_array_output(shape, values)
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn sequence_f32(
    name: &str,
    start: ScalarEndpoint,
    end: ScalarEndpoint,
    count: SequenceCount,
    logarithmic: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let length = checked_length(name, count.length)?;
    let shape = Shape::new([1, count.length]).map_err(|error| array_error(&error))?;
    let start = ArrayComplex32::new(start.re as f32, start.im as f32);
    let end = ArrayComplex32::new(end.re as f32, end.im as f32);
    let mut values = reserved_values(name, length)?;
    if length == 1 {
        values.push(if count.nan_count {
            ArrayComplex32::new(
                f32::NAN,
                if count.interpolation_im == 0.0 {
                    0.0
                } else {
                    f32::NAN
                },
            )
        } else {
            end
        });
    } else if length > 1 {
        let denominator = ArrayComplex32::new(
            (count.interpolation_re - 1.0) as f32,
            count.interpolation_im as f32,
        );
        for index in 0..length {
            check_cancelled_at(context, index)?;
            let value = if index == 0 {
                start
            } else if index + 1 == length {
                end
            } else {
                let ratio = complex_div_f32(ArrayComplex32::new(index as f32, 0.0), denominator);
                let delta = ArrayComplex32::new(end.re - start.re, end.im - start.im);
                complex_add_f32(start, complex_mul_f32(delta, ratio))
            };
            values.push(if logarithmic {
                complex_pow10_f32(value)
            } else {
                value
            });
        }
    }
    crate::core_elementary::complex_f32_array_output(shape, values)
}

fn complex_add_f64(left: ArrayComplex64, right: ArrayComplex64) -> ArrayComplex64 {
    ArrayComplex64::new(left.re + right.re, left.im + right.im)
}

fn complex_mul_f64(left: ArrayComplex64, right: ArrayComplex64) -> ArrayComplex64 {
    ArrayComplex64::new(
        left.re * right.re - left.im * right.im,
        left.re * right.im + left.im * right.re,
    )
}

fn complex_div_f64(left: ArrayComplex64, right: ArrayComplex64) -> ArrayComplex64 {
    if left.re.is_finite()
        && left.im.is_finite()
        && (right.re.is_infinite() || right.im.is_infinite())
    {
        return ArrayComplex64::ZERO;
    }
    if right.re.abs() >= right.im.abs() {
        let ratio = right.im / right.re;
        let denominator = right.re + right.im * ratio;
        ArrayComplex64::new(
            (left.re + left.im * ratio) / denominator,
            (left.im - left.re * ratio) / denominator,
        )
    } else {
        let ratio = right.re / right.im;
        let denominator = right.im + right.re * ratio;
        ArrayComplex64::new(
            (left.re * ratio + left.im) / denominator,
            (left.im * ratio - left.re) / denominator,
        )
    }
}

fn complex_pow10_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        return ArrayComplex64::new(10.0_f64.powf(value.re), value.im);
    }
    let re = std::f64::consts::LN_10 * value.re;
    let im = std::f64::consts::LN_10 * value.im;
    let magnitude = re.exp();
    ArrayComplex64::new(magnitude * im.cos(), magnitude * im.sin())
}

fn complex_add_f32(left: ArrayComplex32, right: ArrayComplex32) -> ArrayComplex32 {
    ArrayComplex32::new(left.re + right.re, left.im + right.im)
}

fn complex_mul_f32(left: ArrayComplex32, right: ArrayComplex32) -> ArrayComplex32 {
    ArrayComplex32::new(
        left.re * right.re - left.im * right.im,
        left.re * right.im + left.im * right.re,
    )
}

fn complex_div_f32(left: ArrayComplex32, right: ArrayComplex32) -> ArrayComplex32 {
    if left.re.is_finite()
        && left.im.is_finite()
        && (right.re.is_infinite() || right.im.is_infinite())
    {
        return ArrayComplex32::ZERO;
    }
    if right.re.abs() >= right.im.abs() {
        let ratio = right.im / right.re;
        let denominator = right.re + right.im * ratio;
        ArrayComplex32::new(
            (left.re + left.im * ratio) / denominator,
            (left.im - left.re * ratio) / denominator,
        )
    } else {
        let ratio = right.re / right.im;
        let denominator = right.im + right.re * ratio;
        ArrayComplex32::new(
            (left.re * ratio + left.im) / denominator,
            (left.im * ratio - left.re) / denominator,
        )
    }
}

fn complex_pow10_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        return ArrayComplex32::new(10.0_f32.powf(value.re), value.im);
    }
    let re = std::f32::consts::LN_10 * value.re;
    let im = std::f32::consts::LN_10 * value.im;
    let magnitude = re.exp();
    ArrayComplex32::new(magnitude * im.cos(), magnitude * im.sin())
}

fn reserved_values<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate {length} output elements"),
        )
    })?;
    Ok(values)
}

fn checked_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` output length {length} does not fit this host"),
        )
    })
}

fn count_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input 3 to `{name}` must have a finite count within the runtime shape range"),
    )
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

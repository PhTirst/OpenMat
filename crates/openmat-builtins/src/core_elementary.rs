use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{Complex64, Value};

use crate::{array_error, expect_argument_count, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

pub(super) fn abs_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("abs", arguments, 1)?;
    expect_max_outputs("abs", context, 1)?;
    context.check_cancelled()?;

    let output = match &arguments[0] {
        Value::Logical(value) => Value::Double(f64::from(*value)),
        Value::Double(value) => Value::Double(value.abs()),
        Value::Complex(value) => Value::Double(value.abs()),
        Value::Array(ArrayData::F32(array)) => {
            map_dense("abs", array, context, |value| value.abs())
                .map(ArrayData::F32)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            map_dense("abs", array, context, |value| value.re.hypot(value.im))
                .map(ArrayData::F32)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::Logical(array)) => {
            map_dense("abs", array, context, |value| f64::from(value.get()))
                .map(ArrayData::F64)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::F64(array)) => {
            map_dense("abs", array, context, |value| value.abs())
                .map(ArrayData::F64)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            map_dense("abs", array, context, |value| value.re.hypot(value.im))
                .map(ArrayData::F64)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::Char(array)) => {
            map_dense("abs", array, context, |value| f64::from(value.get()))
                .map(ArrayData::F64)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::Integer(array)) => {
            Value::Array(ArrayData::Integer(abs_integer_array(array, context)?))
        }
        value => {
            return Err(type_error(
                "abs",
                1,
                "numeric, logical, char, or integer array",
                value,
            ));
        }
    };

    context.check_cancelled()?;
    Ok(vec![output])
}

fn abs_integer_array(
    input: &IntegerArrayData,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! signed_abs {
        ($array:expr, $variant:ident) => {
            map_dense("abs", $array, context, |value| value.saturating_abs())
                .map(IntegerArrayData::$variant)
        };
    }

    match input {
        IntegerArrayData::I8(array) => signed_abs!(array, I8),
        IntegerArrayData::U8(array) => Ok(IntegerArrayData::U8(array.clone())),
        IntegerArrayData::I16(array) => signed_abs!(array, I16),
        IntegerArrayData::U16(array) => Ok(IntegerArrayData::U16(array.clone())),
        IntegerArrayData::I32(array) => signed_abs!(array, I32),
        IntegerArrayData::U32(array) => Ok(IntegerArrayData::U32(array.clone())),
        IntegerArrayData::I64(array) => signed_abs!(array, I64),
        IntegerArrayData::U64(array) => Ok(IntegerArrayData::U64(array.clone())),
        IntegerArrayData::ComplexI8(_)
        | IntegerArrayData::ComplexU8(_)
        | IntegerArrayData::ComplexI16(_)
        | IntegerArrayData::ComplexU16(_)
        | IntegerArrayData::ComplexI32(_)
        | IntegerArrayData::ComplexU32(_)
        | IntegerArrayData::ComplexI64(_)
        | IntegerArrayData::ComplexU64(_) => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "`abs` does not accept complex integer arrays",
        )),
    }
}

#[derive(Clone, Copy)]
enum RoundingOperation {
    Fix,
    Floor,
    Ceil,
}

impl RoundingOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::Fix => "fix",
            Self::Floor => "floor",
            Self::Ceil => "ceil",
        }
    }

    fn f64(self, value: f64) -> f64 {
        match self {
            Self::Fix => value.trunc(),
            Self::Floor => value.floor(),
            Self::Ceil => value.ceil(),
        }
    }

    fn f32(self, value: f32) -> f32 {
        match self {
            Self::Fix => value.trunc(),
            Self::Floor => value.floor(),
            Self::Ceil => value.ceil(),
        }
    }
}

pub(super) fn fix_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    rounding_builtin(RoundingOperation::Fix, arguments, context)
}

pub(super) fn floor_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    rounding_builtin(RoundingOperation::Floor, arguments, context)
}

pub(super) fn ceil_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    rounding_builtin(RoundingOperation::Ceil, arguments, context)
}

fn rounding_builtin(
    operation: RoundingOperation,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = operation.name();
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Logical(value) => Value::Double(f64::from(*value)),
        Value::Double(value) => Value::Double(operation.f64(*value)),
        Value::Complex(value) => complex_scalar_output(ArrayComplex64::new(
            operation.f64(value.real),
            operation.f64(value.imaginary),
        )),
        Value::Array(ArrayData::F32(array)) => {
            map_dense(name, array, context, |value| operation.f32(*value))
                .map(ArrayData::F32)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| {
                ArrayComplex32::new(operation.f32(value.re), operation.f32(value.im))
            })?;
            complex_f32_array_output(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::Logical(array)) => {
            map_dense(name, array, context, |value| f64::from(value.get()))
                .map(ArrayData::F64)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::F64(array)) => {
            map_dense(name, array, context, |value| operation.f64(*value))
                .map(ArrayData::F64)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| {
                ArrayComplex64::new(operation.f64(value.re), operation.f64(value.im))
            })?;
            complex_f64_array_output(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::Char(array)) => map_dense(name, array, context, |value| {
            operation.f64(f64::from(value.get()))
        })
        .map(ArrayData::F64)
        .map(Value::Array)?,
        Value::Array(ArrayData::Integer(_)) => arguments[0].clone(),
        value => {
            return Err(type_error(
                name,
                1,
                "numeric, logical, char, or integer array",
                value,
            ));
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

#[derive(Clone, Copy)]
enum ElementaryOperation {
    Sqrt,
    Exp,
    Expm1,
    Log,
    Log1p,
    Log10,
    Sin,
    Cos,
    Tan,
}

impl ElementaryOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::Sqrt => "sqrt",
            Self::Exp => "exp",
            Self::Expm1 => "expm1",
            Self::Log => "log",
            Self::Log1p => "log1p",
            Self::Log10 => "log10",
            Self::Sin => "sin",
            Self::Cos => "cos",
            Self::Tan => "tan",
        }
    }

    fn real_f64(self, value: f64) -> ArrayComplex64 {
        match self {
            Self::Sqrt => complex_sqrt_f64(ArrayComplex64::new(value, 0.0)),
            Self::Exp => ArrayComplex64::new(value.exp(), 0.0),
            Self::Expm1 => ArrayComplex64::new(value.exp_m1(), 0.0),
            Self::Log => complex_log_f64(ArrayComplex64::new(value, 0.0)),
            Self::Log1p => complex_log1p_f64(ArrayComplex64::new(value, 0.0)),
            Self::Log10 => complex_log10_f64(ArrayComplex64::new(value, 0.0)),
            Self::Sin => ArrayComplex64::new(value.sin(), 0.0),
            Self::Cos => ArrayComplex64::new(value.cos(), 0.0),
            Self::Tan => ArrayComplex64::new(value.tan(), 0.0),
        }
    }

    fn complex_f64(self, value: ArrayComplex64) -> ArrayComplex64 {
        match self {
            Self::Sqrt => complex_sqrt_f64(value),
            Self::Exp => complex_exp_f64(value),
            Self::Expm1 => complex_expm1_f64(value),
            Self::Log => complex_log_f64(value),
            Self::Log1p => complex_log1p_f64(value),
            Self::Log10 => complex_log10_f64(value),
            Self::Sin => complex_sin_f64(value),
            Self::Cos => complex_cos_f64(value),
            Self::Tan => complex_tan_f64(value),
        }
    }

    fn real_f32(self, value: f32) -> ArrayComplex32 {
        match self {
            Self::Sqrt => complex_sqrt_f32(ArrayComplex32::new(value, 0.0)),
            Self::Exp => ArrayComplex32::new(value.exp(), 0.0),
            Self::Expm1 => ArrayComplex32::new(narrow_f64_to_f32(f64::from(value).exp_m1()), 0.0),
            Self::Log => complex_log_f32(ArrayComplex32::new(value, 0.0)),
            Self::Log1p => complex_log1p_f32(ArrayComplex32::new(value, 0.0)),
            Self::Log10 => complex_log10_f32(ArrayComplex32::new(value, 0.0)),
            Self::Sin => ArrayComplex32::new(value.sin(), 0.0),
            Self::Cos => ArrayComplex32::new(value.cos(), 0.0),
            Self::Tan => ArrayComplex32::new(value.tan(), 0.0),
        }
    }

    fn complex_f32(self, value: ArrayComplex32) -> ArrayComplex32 {
        match self {
            Self::Sqrt => complex_sqrt_f32(value),
            Self::Exp => complex_exp_f32(value),
            Self::Expm1 => complex_expm1_f32(value),
            Self::Log => complex_log_f32(value),
            Self::Log1p => complex_log1p_f32(value),
            Self::Log10 => complex_log10_f32(value),
            Self::Sin => complex_sin_f32(value),
            Self::Cos => complex_cos_f32(value),
            Self::Tan => complex_tan_f32(value),
        }
    }
}

pub(super) fn sqrt_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Sqrt, arguments, context)
}

pub(super) fn exp_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Exp, arguments, context)
}

pub(super) fn expm1_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Expm1, arguments, context)
}

pub(super) fn log_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Log, arguments, context)
}

pub(super) fn log1p_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Log1p, arguments, context)
}

pub(super) fn log10_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Log10, arguments, context)
}

pub(super) fn sin_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Sin, arguments, context)
}

pub(super) fn cos_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Cos, arguments, context)
}

pub(super) fn tan_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    elementary_builtin(ElementaryOperation::Tan, arguments, context)
}

fn elementary_builtin(
    operation: ElementaryOperation,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = operation.name();
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Double(value) => complex_scalar_output(operation.real_f64(*value)),
        Value::Complex(value) => complex_scalar_output(
            operation.complex_f64(ArrayComplex64::new(value.real, value.imaginary)),
        ),
        Value::Array(ArrayData::F64(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| {
                operation.real_f64(*value)
            })?;
            complex_f64_array_output(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| {
                operation.complex_f64(*value)
            })?;
            complex_f64_array_output(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::F32(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| {
                operation.real_f32(*value)
            })?;
            complex_f32_array_output(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| {
                operation.complex_f32(*value)
            })?;
            complex_f32_array_output(array.shape().clone(), values)?
        }
        value => return Err(type_error(name, 1, "double or single array", value)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn complex_scalar_output(value: ArrayComplex64) -> Value {
    if value.im == 0.0 {
        Value::Double(value.re)
    } else {
        Value::Complex(Complex64::new(value.re, value.im))
    }
}

pub(super) fn complex_f64_array_output(
    shape: openmat_array::Shape,
    values: Vec<ArrayComplex64>,
) -> Result<Value, BuiltinError> {
    if values.iter().all(|value| value.im == 0.0) {
        let values = values.into_iter().map(|value| value.re).collect();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

pub(super) fn complex_f32_array_output(
    shape: openmat_array::Shape,
    values: Vec<ArrayComplex32>,
) -> Result<Value, BuiltinError> {
    if values.iter().all(|value| value.im == 0.0) {
        let values = values.into_iter().map(|value| value.re).collect();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

fn complex_exp_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        return ArrayComplex64::new(value.re.exp(), value.im);
    }
    let magnitude = value.re.exp();
    ArrayComplex64::new(magnitude * value.im.cos(), magnitude * value.im.sin())
}

fn complex_expm1_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        return ArrayComplex64::new(value.re.exp_m1(), value.im);
    }
    let expm1_real = value.re.exp_m1();
    let sin_half_imaginary = (0.5 * value.im).sin();
    ArrayComplex64::new(
        expm1_real * value.im.cos() - 2.0 * sin_half_imaginary * sin_half_imaginary,
        (expm1_real + 1.0) * value.im.sin(),
    )
}

fn complex_log1p_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        if value.re.is_nan() {
            return ArrayComplex64::new(f64::NAN, value.im);
        }
        if value.re >= -1.0 {
            return ArrayComplex64::new(value.re.ln_1p(), value.im);
        }
        return ArrayComplex64::new((-value.re - 1.0).ln(), std::f64::consts::PI);
    }
    let squared_delta = value
        .re
        .mul_add(value.re, value.im * value.im)
        .mul_add(1.0, 2.0 * value.re);
    let real = if squared_delta.is_finite() && squared_delta >= -1.0 {
        0.5 * squared_delta.ln_1p()
    } else {
        (value.re + 1.0).hypot(value.im).ln()
    };
    ArrayComplex64::new(real, value.im.atan2(value.re + 1.0))
}

fn complex_sqrt_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        return if value.re >= 0.0 {
            ArrayComplex64::new(value.re.sqrt(), value.im)
        } else {
            ArrayComplex64::new(0.0, (-value.re).sqrt())
        };
    }
    let magnitude = value.re.hypot(value.im);
    let real = f64::midpoint(magnitude, value.re).sqrt();
    let imaginary_magnitude = f64::midpoint(magnitude, -value.re).sqrt();
    ArrayComplex64::new(real, imaginary_magnitude.copysign(value.im))
}

fn complex_log_f64(value: ArrayComplex64) -> ArrayComplex64 {
    ArrayComplex64::new(value.re.hypot(value.im).ln(), value.im.atan2(value.re))
}

fn complex_log10_f64(value: ArrayComplex64) -> ArrayComplex64 {
    let value = complex_log_f64(value);
    ArrayComplex64::new(
        value.re * std::f64::consts::LOG10_E,
        value.im * std::f64::consts::LOG10_E,
    )
}

fn complex_sin_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        return ArrayComplex64::new(value.re.sin(), value.im);
    }
    ArrayComplex64::new(
        value.re.sin() * value.im.cosh(),
        value.re.cos() * value.im.sinh(),
    )
}

fn complex_cos_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        return ArrayComplex64::new(value.re.cos(), -value.im);
    }
    ArrayComplex64::new(
        value.re.cos() * value.im.cosh(),
        -(value.re.sin() * value.im.sinh()),
    )
}

fn complex_tan_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        return ArrayComplex64::new(value.re.tan(), value.im);
    }
    if value.im.abs() > 350.0 {
        return ArrayComplex64::new(0.0_f64.copysign((2.0 * value.re).sin()), value.im.signum());
    }
    let denominator = (2.0 * value.re).cos() + (2.0 * value.im).cosh();
    ArrayComplex64::new(
        (2.0 * value.re).sin() / denominator,
        (2.0 * value.im).sinh() / denominator,
    )
}

fn complex_exp_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        return ArrayComplex32::new(value.re.exp(), value.im);
    }
    let magnitude = value.re.exp();
    ArrayComplex32::new(magnitude * value.im.cos(), magnitude * value.im.sin())
}

fn complex_expm1_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        return ArrayComplex32::new(narrow_f64_to_f32(f64::from(value.re).exp_m1()), value.im);
    }
    let real = f64::from(value.re);
    let imaginary = f64::from(value.im);
    let expm1_real = real.exp_m1();
    let sin_half_imaginary = (0.5 * imaginary).sin();
    ArrayComplex32::new(
        narrow_f64_to_f32(
            expm1_real * imaginary.cos() - 2.0 * sin_half_imaginary * sin_half_imaginary,
        ),
        narrow_f64_to_f32((expm1_real + 1.0) * imaginary.sin()),
    )
}

#[allow(clippy::cast_possible_truncation)]
fn narrow_f64_to_f32(value: f64) -> f32 {
    value as f32
}

fn complex_log1p_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        if value.re.is_nan() {
            return ArrayComplex32::new(f32::NAN, value.im);
        }
        if value.re >= -1.0 {
            return ArrayComplex32::new(value.re.ln_1p(), value.im);
        }
        return ArrayComplex32::new((-value.re - 1.0).ln(), std::f32::consts::PI);
    }
    let squared_delta = value
        .re
        .mul_add(value.re, value.im * value.im)
        .mul_add(1.0, 2.0 * value.re);
    let real = if squared_delta.is_finite() && squared_delta >= -1.0 {
        0.5 * squared_delta.ln_1p()
    } else {
        (value.re + 1.0).hypot(value.im).ln()
    };
    ArrayComplex32::new(real, value.im.atan2(value.re + 1.0))
}

fn complex_sqrt_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        return if value.re >= 0.0 {
            ArrayComplex32::new(value.re.sqrt(), value.im)
        } else {
            ArrayComplex32::new(0.0, (-value.re).sqrt())
        };
    }
    let magnitude = value.re.hypot(value.im);
    let real = f32::midpoint(magnitude, value.re).sqrt();
    let imaginary_magnitude = f32::midpoint(magnitude, -value.re).sqrt();
    ArrayComplex32::new(real, imaginary_magnitude.copysign(value.im))
}

fn complex_log_f32(value: ArrayComplex32) -> ArrayComplex32 {
    ArrayComplex32::new(value.re.hypot(value.im).ln(), value.im.atan2(value.re))
}

fn complex_log10_f32(value: ArrayComplex32) -> ArrayComplex32 {
    let value = complex_log_f32(value);
    ArrayComplex32::new(
        value.re * std::f32::consts::LOG10_E,
        value.im * std::f32::consts::LOG10_E,
    )
}

fn complex_sin_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        return ArrayComplex32::new(value.re.sin(), value.im);
    }
    ArrayComplex32::new(
        value.re.sin() * value.im.cosh(),
        value.re.cos() * value.im.sinh(),
    )
}

fn complex_cos_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        return ArrayComplex32::new(value.re.cos(), -value.im);
    }
    ArrayComplex32::new(
        value.re.cos() * value.im.cosh(),
        -(value.re.sin() * value.im.sinh()),
    )
}

fn complex_tan_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        return ArrayComplex32::new(value.re.tan(), value.im);
    }
    if value.im.abs() > 40.0 {
        return ArrayComplex32::new(0.0_f32.copysign((2.0 * value.re).sin()), value.im.signum());
    }
    let denominator = (2.0 * value.re).cos() + (2.0 * value.im).cosh();
    ArrayComplex32::new(
        (2.0 * value.re).sin() / denominator,
        (2.0 * value.im).sinh() / denominator,
    )
}

fn map_dense<T, U, F>(
    name: &str,
    input: &DenseArray<T>,
    context: &BuiltinContext<'_>,
    transform: F,
) -> Result<DenseArray<U>, BuiltinError>
where
    F: FnMut(&T) -> U,
{
    let values = map_values(name, input.as_slice(), context, transform)?;
    DenseArray::from_vec(input.shape().clone(), values).map_err(|error| array_error(&error))
}

pub(super) fn map_values<T, U, F>(
    name: &str,
    input: &[T],
    context: &BuiltinContext<'_>,
    mut transform: F,
) -> Result<Vec<U>, BuiltinError>
where
    F: FnMut(&T) -> U,
{
    let mut output = Vec::new();
    output.try_reserve_exact(input.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!(
                "`{name}` cannot allocate storage for {} elements",
                input.len()
            ),
        )
    })?;
    for chunk in input.chunks(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
        output.extend(chunk.iter().map(&mut transform));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use openmat_array::{CharCodeUnit, ComplexInteger, Logical, Shape};
    use openmat_runtime::{CancellationToken, VecOutput};

    use super::*;

    fn invoke_abs(value: Value) -> Result<Value, BuiltinError> {
        let cancellation = CancellationToken::new();
        let mut output = VecOutput::new();
        let mut context = BuiltinContext::new(1, &cancellation, &mut output);
        let mut values = abs_builtin(&[value], &mut context)?;
        Ok(values.remove(0))
    }

    #[test]
    fn abs_preserves_shape_and_floating_class() {
        let double = Value::Array(ArrayData::F64(
            DenseArray::from_vec(
                Shape::new([2, 2]).unwrap(),
                vec![-3.0, f64::NAN, 4.0, f64::NEG_INFINITY],
            )
            .unwrap(),
        ));
        let Value::Array(ArrayData::F64(output)) = invoke_abs(double).unwrap() else {
            panic!("double array abs must return a real double array");
        };
        assert_eq!(output.shape().dimensions(), &[2, 2]);
        assert_eq!(output.as_slice()[0].to_bits(), 3.0_f64.to_bits());
        assert!(output.as_slice()[1].is_nan());
        assert_eq!(output.as_slice()[2].to_bits(), 4.0_f64.to_bits());
        assert_eq!(output.as_slice()[3].to_bits(), f64::INFINITY.to_bits());

        let complex_single = Value::Array(ArrayData::ComplexF32(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                vec![
                    ArrayComplex32::new(-3.0, 4.0),
                    ArrayComplex32::new(0.0, -5.0),
                ],
            )
            .unwrap(),
        ));
        let Value::Array(ArrayData::F32(output)) = invoke_abs(complex_single).unwrap() else {
            panic!("complex single abs must return a real single array");
        };
        assert_eq!(output.shape().dimensions(), &[1, 2]);
        assert_eq!(output.as_slice(), &[5.0, 5.0]);
    }

    #[test]
    fn abs_matches_logical_char_and_integer_class_rules() {
        let logical = Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                vec![Logical::FALSE, Logical::TRUE],
            )
            .unwrap(),
        ));
        let Value::Array(ArrayData::F64(output)) = invoke_abs(logical).unwrap() else {
            panic!("logical abs must return double");
        };
        assert_eq!(output.as_slice(), &[0.0, 1.0]);

        let characters = Value::Array(ArrayData::Char(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                vec![CharCodeUnit::new(65), CharCodeUnit::new(122)],
            )
            .unwrap(),
        ));
        let Value::Array(ArrayData::F64(output)) = invoke_abs(characters).unwrap() else {
            panic!("char abs must return double");
        };
        assert_eq!(output.as_slice(), &[65.0, 122.0]);

        let signed = Value::Array(ArrayData::Integer(IntegerArrayData::I8(
            DenseArray::from_vec(Shape::new([1, 3]).unwrap(), vec![i8::MIN, -3, 4]).unwrap(),
        )));
        let Value::Array(ArrayData::Integer(IntegerArrayData::I8(output))) =
            invoke_abs(signed).unwrap()
        else {
            panic!("signed integer abs must preserve the integer class");
        };
        assert_eq!(output.as_slice(), &[i8::MAX, 3, 4]);

        let unsigned = Value::Array(ArrayData::Integer(IntegerArrayData::U64(
            DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![0, u64::MAX]).unwrap(),
        )));
        let Value::Array(ArrayData::Integer(IntegerArrayData::U64(output))) =
            invoke_abs(unsigned).unwrap()
        else {
            panic!("unsigned integer abs must preserve the integer class");
        };
        assert_eq!(output.as_slice(), &[0, u64::MAX]);
    }

    #[test]
    fn abs_preserves_empty_shape_and_rejects_complex_integer_storage() {
        let empty = Value::Array(ArrayData::F32(
            DenseArray::from_vec(Shape::new([0, 3]).unwrap(), Vec::new()).unwrap(),
        ));
        let Value::Array(ArrayData::F32(output)) = invoke_abs(empty).unwrap() else {
            panic!("empty single abs must preserve single storage");
        };
        assert_eq!(output.shape().dimensions(), &[0, 3]);

        let complex_integer = Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(
            DenseArray::from_vec(
                Shape::new([1, 1]).unwrap(),
                vec![ComplexInteger::new(-3, 4)],
            )
            .unwrap(),
        )));
        let error = invoke_abs(complex_integer).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Type);
    }

    #[test]
    fn elementary_mapping_checks_cancellation_before_traversal() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut output = VecOutput::new();
        let context = BuiltinContext::new(1, &cancellation, &mut output);
        let input = vec![1.0_f64; 8_192];
        let error = map_values("exp", &input, &context, |value| value.exp()).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
    }

    #[test]
    fn complex_formulas_cover_finite_reference_points() {
        let value = ArrayComplex64::new(1.0, 2.0);
        let exponential = complex_exp_f64(value);
        assert!((exponential.re - -1.131_204_383_756_813_5).abs() < 1.0e-15);
        assert!((exponential.im - 2.471_726_672_004_818_8).abs() < 1.0e-15);
        let tangent = complex_tan_f64(value);
        assert!((tangent.re - 0.033_812_826_079_896_61).abs() < 1.0e-15);
        assert!((tangent.im - 1.014_793_616_146_633_5).abs() < 1.0e-15);
        assert_eq!(
            complex_sqrt_f64(ArrayComplex64::new(-4.0, 0.0)),
            ArrayComplex64::new(0.0, 2.0)
        );
    }
}

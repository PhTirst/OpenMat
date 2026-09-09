use num_complex::Complex as NumComplex;
use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::core_elementary::{
    complex_f32_array_output, complex_f64_array_output, complex_scalar_output, map_values,
};
use crate::{array_error, expect_argument_count, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnaryMath {
    Sign,
    Angle,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Deg2Rad,
    Rad2Deg,
}

impl UnaryMath {
    const fn name(self) -> &'static str {
        match self {
            Self::Sign => "sign",
            Self::Angle => "angle",
            Self::Asin => "asin",
            Self::Acos => "acos",
            Self::Atan => "atan",
            Self::Sinh => "sinh",
            Self::Cosh => "cosh",
            Self::Tanh => "tanh",
            Self::Asinh => "asinh",
            Self::Acosh => "acosh",
            Self::Atanh => "atanh",
            Self::Deg2Rad => "deg2rad",
            Self::Rad2Deg => "rad2deg",
        }
    }

    fn real_f64(self, value: f64) -> ArrayComplex64 {
        let output = match self {
            Self::Sign => NumComplex::new(real_sign_f64(value), 0.0),
            Self::Angle => NumComplex::new(real_angle_f64(value), 0.0),
            Self::Asin => asin_real_f64(value),
            Self::Acos => acos_real_f64(value),
            Self::Atan => NumComplex::new(value.atan(), 0.0),
            Self::Sinh => NumComplex::new(value.sinh(), 0.0),
            Self::Cosh => NumComplex::new(value.cosh(), 0.0),
            Self::Tanh => NumComplex::new(value.tanh(), 0.0),
            Self::Asinh => NumComplex::new(value.asinh(), 0.0),
            Self::Acosh => acosh_real_f64(value),
            Self::Atanh => atanh_real_f64(value),
            Self::Deg2Rad => NumComplex::new(value * (std::f64::consts::PI / 180.0), 0.0),
            Self::Rad2Deg => NumComplex::new(value * (180.0 / std::f64::consts::PI), 0.0),
        };
        ArrayComplex64::new(output.re, output.im)
    }

    fn complex_f64(self, value: ArrayComplex64) -> ArrayComplex64 {
        let value = NumComplex::new(value.re, value.im);
        let output = match self {
            Self::Sign => complex_sign_f64(value),
            Self::Angle => NumComplex::new(complex_angle_f64(value), 0.0),
            Self::Asin => value.asin(),
            Self::Acos => value.acos(),
            Self::Atan => complex_atan_f64(value),
            Self::Sinh => value.sinh(),
            Self::Cosh => value.cosh(),
            Self::Tanh => complex_tanh_f64(value),
            Self::Asinh => value.asinh(),
            Self::Acosh => value.acosh(),
            Self::Atanh => value.atanh(),
            Self::Deg2Rad => value * (std::f64::consts::PI / 180.0),
            Self::Rad2Deg => value * (180.0 / std::f64::consts::PI),
        };
        ArrayComplex64::new(output.re, output.im)
    }

    fn real_f32(self, value: f32) -> ArrayComplex32 {
        let output = match self {
            Self::Sign => NumComplex::new(real_sign_f32(value), 0.0),
            Self::Angle => NumComplex::new(real_angle_f32(value), 0.0),
            Self::Asin => asin_real_f32(value),
            Self::Acos => acos_real_f32(value),
            Self::Atan => NumComplex::new(value.atan(), 0.0),
            Self::Sinh => NumComplex::new(value.sinh(), 0.0),
            Self::Cosh => NumComplex::new(value.cosh(), 0.0),
            Self::Tanh => NumComplex::new(value.tanh(), 0.0),
            Self::Asinh => NumComplex::new(value.asinh(), 0.0),
            Self::Acosh => acosh_real_f32(value),
            Self::Atanh => atanh_real_f32(value),
            Self::Deg2Rad => NumComplex::new(value * (std::f32::consts::PI / 180.0), 0.0),
            Self::Rad2Deg => NumComplex::new(value * (180.0 / std::f32::consts::PI), 0.0),
        };
        ArrayComplex32::new(output.re, output.im)
    }

    fn complex_f32(self, value: ArrayComplex32) -> ArrayComplex32 {
        let value = NumComplex::new(value.re, value.im);
        let output = match self {
            Self::Sign => complex_sign_f32(value),
            Self::Angle => NumComplex::new(complex_angle_f32(value), 0.0),
            Self::Asin => value.asin(),
            Self::Acos => value.acos(),
            Self::Atan => complex_atan_f32(value),
            Self::Sinh => value.sinh(),
            Self::Cosh => value.cosh(),
            Self::Tanh => complex_tanh_f32(value),
            Self::Asinh => value.asinh(),
            Self::Acosh => value.acosh(),
            Self::Atanh => value.atanh(),
            Self::Deg2Rad => value * (std::f32::consts::PI / 180.0),
            Self::Rad2Deg => value * (180.0 / std::f32::consts::PI),
        };
        ArrayComplex32::new(output.re, output.im)
    }
}

macro_rules! unary_builtin {
    ($function:ident, $operation:ident) => {
        pub(super) fn $function(
            arguments: &[Value],
            context: &mut BuiltinContext<'_>,
        ) -> BuiltinResult {
            unary_math_builtin(UnaryMath::$operation, arguments, context)
        }
    };
}

unary_builtin!(sign_builtin, Sign);
unary_builtin!(angle_builtin, Angle);
unary_builtin!(asin_builtin, Asin);
unary_builtin!(acos_builtin, Acos);
unary_builtin!(atan_builtin, Atan);
unary_builtin!(sinh_builtin, Sinh);
unary_builtin!(cosh_builtin, Cosh);
unary_builtin!(tanh_builtin, Tanh);
unary_builtin!(asinh_builtin, Asinh);
unary_builtin!(acosh_builtin, Acosh);
unary_builtin!(atanh_builtin, Atanh);
unary_builtin!(deg2rad_builtin, Deg2Rad);
unary_builtin!(rad2deg_builtin, Rad2Deg);

fn unary_math_builtin(
    operation: UnaryMath,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BinaryMath {
    Mod,
    Rem,
    Hypot,
    Atan2,
}

impl BinaryMath {
    const fn name(self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::Rem => "rem",
            Self::Hypot => "hypot",
            Self::Atan2 => "atan2",
        }
    }

    const fn accepts_complex(self) -> bool {
        matches!(self, Self::Hypot)
    }

    fn f64(self, left: NumComplex<f64>, right: NumComplex<f64>) -> f64 {
        match self {
            Self::Mod => matlab_mod_f64(left.re, right.re),
            Self::Rem => left.re % right.re,
            Self::Hypot => left.re.hypot(left.im).hypot(right.re.hypot(right.im)),
            Self::Atan2 => left.re.atan2(right.re),
        }
    }

    fn f32(self, left: NumComplex<f32>, right: NumComplex<f32>) -> f32 {
        match self {
            Self::Mod => matlab_mod_f32(left.re, right.re),
            Self::Rem => left.re % right.re,
            Self::Hypot => left.re.hypot(left.im).hypot(right.re.hypot(right.im)),
            Self::Atan2 => left.re.atan2(right.re),
        }
    }
}

macro_rules! binary_builtin {
    ($function:ident, $operation:ident) => {
        pub(super) fn $function(
            arguments: &[Value],
            context: &mut BuiltinContext<'_>,
        ) -> BuiltinResult {
            binary_math_builtin(BinaryMath::$operation, arguments, context)
        }
    };
}

binary_builtin!(mod_builtin, Mod);
binary_builtin!(rem_builtin, Rem);
binary_builtin!(hypot_builtin, Hypot);
binary_builtin!(atan2_builtin, Atan2);

fn binary_math_builtin(
    operation: BinaryMath,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = operation.name();
    expect_argument_count(name, arguments, 2)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let output = if arguments.iter().any(|value| {
        matches!(
            value,
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        )
    }) {
        binary_single(operation, arguments, context)?
    } else {
        binary_double(operation, arguments, context)?
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

#[derive(Clone)]
struct MathInput<T> {
    shape: Shape,
    values: Vec<NumComplex<T>>,
    scalar_variant: bool,
    complex: bool,
}

fn binary_double(
    operation: BinaryMath,
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    let left = double_input(name, 1, &arguments[0], context)?;
    let right = double_input(name, 2, &arguments[1], context)?;
    reject_complex(operation, &left, &right)?;
    let shape = compatible_shape(name, &left, &right)?;
    let scalar_variant = left.scalar_variant && right.scalar_variant;
    let values = map_binary(name, &left, &right, &shape, context, |left, right| {
        operation.f64(left, right)
    })?;
    if scalar_variant {
        return values.first().copied().map(Value::Double).ok_or_else(|| {
            BuiltinError::new(BuiltinErrorCategory::Other, "scalar math result was empty")
        });
    }
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn binary_single(
    operation: BinaryMath,
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    let left = single_input(name, 1, &arguments[0], context)?;
    let right = single_input(name, 2, &arguments[1], context)?;
    reject_complex(operation, &left, &right)?;
    let shape = compatible_shape(name, &left, &right)?;
    let values = map_binary(name, &left, &right, &shape, context, |left, right| {
        operation.f32(left, right)
    })?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F32)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn double_input(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<MathInput<f64>, BuiltinError> {
    let scalar_shape = || Shape::new([1, 1]).map_err(|error| array_error(&error));
    match value {
        Value::Double(value) => Ok(MathInput {
            shape: scalar_shape()?,
            values: vec![NumComplex::new(*value, 0.0)],
            scalar_variant: true,
            complex: false,
        }),
        Value::Complex(value) => Ok(MathInput {
            shape: scalar_shape()?,
            values: vec![NumComplex::new(value.real, value.imaginary)],
            scalar_variant: true,
            complex: true,
        }),
        Value::Array(ArrayData::F64(array)) => Ok(MathInput {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| {
                NumComplex::new(*value, 0.0)
            })?,
            scalar_variant: false,
            complex: false,
        }),
        Value::Array(ArrayData::ComplexF64(array)) => Ok(MathInput {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| {
                NumComplex::new(value.re, value.im)
            })?,
            scalar_variant: false,
            complex: true,
        }),
        value => Err(type_error(name, position, "double or single array", value)),
    }
}

fn single_input(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<MathInput<f32>, BuiltinError> {
    match value {
        Value::Array(ArrayData::F32(array)) => Ok(MathInput {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| {
                NumComplex::new(*value, 0.0)
            })?,
            scalar_variant: false,
            complex: false,
        }),
        Value::Array(ArrayData::ComplexF32(array)) => Ok(MathInput {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| {
                NumComplex::new(value.re, value.im)
            })?,
            scalar_variant: false,
            complex: true,
        }),
        _ => double_input_as_single(name, position, value, context),
    }
}

fn double_input_as_single(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<MathInput<f32>, BuiltinError> {
    let input = double_input(name, position, value, context)?;
    let values = map_values(name, &input.values, context, |value| {
        NumComplex::new(f64_to_f32(value.re), f64_to_f32(value.im))
    })?;
    Ok(MathInput {
        shape: input.shape,
        values,
        scalar_variant: input.scalar_variant,
        complex: input.complex,
    })
}

fn compatible_shape<T>(
    name: &str,
    left: &MathInput<T>,
    right: &MathInput<T>,
) -> Result<Shape, BuiltinError> {
    let left_dimensions = left.shape.dimensions();
    let right_dimensions = right.shape.dimensions();
    let rank = left_dimensions.len().max(right_dimensions.len());
    let mut dimensions = Vec::new();
    dimensions.try_reserve_exact(rank).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate implicit-expansion shape metadata"),
        )
    })?;
    for axis in 0..rank {
        let left = left_dimensions.get(axis).copied().unwrap_or(1);
        let right = right_dimensions.get(axis).copied().unwrap_or(1);
        let extent = if left == right {
            left
        } else if left == 1 {
            right
        } else if right == 1 {
            left
        } else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!(
                    "inputs to `{name}` have incompatible extents {left} and {right} in dimension {}",
                    axis + 1
                ),
            ));
        };
        dimensions.push(extent);
    }
    Shape::new(dimensions).map_err(|error| array_error(&error))
}

fn reject_complex<T>(
    operation: BinaryMath,
    left: &MathInput<T>,
    right: &MathInput<T>,
) -> Result<(), BuiltinError> {
    if operation.accepts_complex() || (!left.complex && !right.complex) {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("`{}` requires real-valued inputs", operation.name()),
        ))
    }
}

fn map_binary<T: Copy, U, F>(
    name: &str,
    left: &MathInput<T>,
    right: &MathInput<T>,
    shape: &Shape,
    context: &BuiltinContext<'_>,
    mut operation: F,
) -> Result<Vec<U>, BuiltinError>
where
    F: FnMut(NumComplex<T>, NumComplex<T>) -> U,
{
    let length = usize::try_from(shape.numel()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` implicit-expansion output does not fit this host"),
        )
    })?;
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {length} elements"),
        )
    })?;
    for index in 0..length {
        if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
            context.check_cancelled()?;
        }
        let left_offset = broadcast_offset(name, index, shape, &left.shape)?;
        let right_offset = broadcast_offset(name, index, shape, &right.shape)?;
        output.push(operation(
            *left
                .values
                .get(left_offset)
                .ok_or_else(|| broadcast_mapping_error(name))?,
            *right
                .values
                .get(right_offset)
                .ok_or_else(|| broadcast_mapping_error(name))?,
        ));
    }
    Ok(output)
}

fn broadcast_offset(
    name: &str,
    output_offset: usize,
    output_shape: &Shape,
    input_shape: &Shape,
) -> Result<usize, BuiltinError> {
    let mut remainder = u64::try_from(output_offset).map_err(|_| broadcast_mapping_error(name))?;
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
            .ok_or_else(|| broadcast_mapping_error(name))?;
        input_stride = input_stride
            .checked_mul(input_extent)
            .ok_or_else(|| broadcast_mapping_error(name))?;
    }
    usize::try_from(input_offset).map_err(|_| broadcast_mapping_error(name))
}

fn broadcast_mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not map its implicit-expansion operands"),
    )
}

fn real_sign_f64(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value.signum() }
}

fn real_sign_f32(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value.signum() }
}

fn real_angle_f64(value: f64) -> f64 {
    if value.is_nan() {
        f64::NAN
    } else if value < 0.0 {
        std::f64::consts::PI
    } else {
        0.0
    }
}

fn real_angle_f32(value: f32) -> f32 {
    if value.is_nan() {
        f32::NAN
    } else if value < 0.0 {
        std::f32::consts::PI
    } else {
        0.0
    }
}

fn complex_angle_f64(value: NumComplex<f64>) -> f64 {
    if value.re == 0.0 && value.im == 0.0 {
        0.0
    } else {
        value.im.atan2(value.re)
    }
}

fn complex_angle_f32(value: NumComplex<f32>) -> f32 {
    if value.re == 0.0 && value.im == 0.0 {
        0.0
    } else {
        value.im.atan2(value.re)
    }
}

fn complex_sign_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    let magnitude = value.re.hypot(value.im);
    if magnitude == 0.0 {
        NumComplex::new(0.0, 0.0)
    } else {
        NumComplex::new(value.re / magnitude, value.im / magnitude)
    }
}

fn complex_sign_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    let magnitude = value.re.hypot(value.im);
    if magnitude == 0.0 {
        NumComplex::new(0.0, 0.0)
    } else {
        NumComplex::new(value.re / magnitude, value.im / magnitude)
    }
}

fn asin_real_f64(value: f64) -> NumComplex<f64> {
    if value.abs() <= 1.0 || value.is_nan() {
        NumComplex::new(value.asin(), 0.0)
    } else if value.is_sign_positive() {
        NumComplex::new(std::f64::consts::FRAC_PI_2, -value.acosh())
    } else {
        NumComplex::new(-std::f64::consts::FRAC_PI_2, (-value).acosh())
    }
}

fn asin_real_f32(value: f32) -> NumComplex<f32> {
    if value.abs() <= 1.0 || value.is_nan() {
        NumComplex::new(value.asin(), 0.0)
    } else if value.is_sign_positive() {
        NumComplex::new(std::f32::consts::FRAC_PI_2, -value.acosh())
    } else {
        NumComplex::new(-std::f32::consts::FRAC_PI_2, (-value).acosh())
    }
}

fn acos_real_f64(value: f64) -> NumComplex<f64> {
    if value.abs() <= 1.0 || value.is_nan() {
        NumComplex::new(value.acos(), 0.0)
    } else if value.is_sign_positive() {
        NumComplex::new(0.0, value.acosh())
    } else {
        NumComplex::new(std::f64::consts::PI, -(-value).acosh())
    }
}

fn acos_real_f32(value: f32) -> NumComplex<f32> {
    if value.abs() <= 1.0 || value.is_nan() {
        NumComplex::new(value.acos(), 0.0)
    } else if value.is_sign_positive() {
        NumComplex::new(0.0, value.acosh())
    } else {
        NumComplex::new(std::f32::consts::PI, -(-value).acosh())
    }
}

fn acosh_real_f64(value: f64) -> NumComplex<f64> {
    if value >= 1.0 || value.is_nan() {
        NumComplex::new(value.acosh(), 0.0)
    } else if value >= -1.0 {
        NumComplex::new(0.0, value.acos())
    } else {
        NumComplex::new((-value).acosh(), std::f64::consts::PI)
    }
}

fn acosh_real_f32(value: f32) -> NumComplex<f32> {
    if value >= 1.0 || value.is_nan() {
        NumComplex::new(value.acosh(), 0.0)
    } else if value >= -1.0 {
        NumComplex::new(0.0, value.acos())
    } else {
        NumComplex::new((-value).acosh(), std::f32::consts::PI)
    }
}

fn atanh_real_f64(value: f64) -> NumComplex<f64> {
    if value.abs() <= 1.0 || value.is_nan() {
        NumComplex::new(value.atanh(), 0.0)
    } else if value.is_infinite() {
        NumComplex::new(0.0, std::f64::consts::FRAC_PI_2.copysign(value))
    } else {
        let real = 0.5 * ((1.0 + value).abs().ln() - (1.0 - value).abs().ln());
        NumComplex::new(real, std::f64::consts::FRAC_PI_2.copysign(value))
    }
}

fn atanh_real_f32(value: f32) -> NumComplex<f32> {
    if value.abs() <= 1.0 || value.is_nan() {
        NumComplex::new(value.atanh(), 0.0)
    } else if value.is_infinite() {
        NumComplex::new(0.0, std::f32::consts::FRAC_PI_2.copysign(value))
    } else {
        let real = 0.5 * ((1.0 + value).abs().ln() - (1.0 - value).abs().ln());
        NumComplex::new(real, std::f32::consts::FRAC_PI_2.copysign(value))
    }
}

fn complex_atan_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    if value.re.is_infinite() && value.im.is_finite() {
        NumComplex::new(
            std::f64::consts::FRAC_PI_2.copysign(value.re),
            0.0_f64.copysign(value.im),
        )
    } else {
        value.atan()
    }
}

fn complex_atan_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    if value.re.is_infinite() && value.im.is_finite() {
        NumComplex::new(
            std::f32::consts::FRAC_PI_2.copysign(value.re),
            0.0_f32.copysign(value.im),
        )
    } else {
        value.atan()
    }
}

fn complex_tanh_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    if value.im == 0.0 {
        return NumComplex::new(value.re.tanh(), value.im);
    }
    if value.re.abs() > 350.0 {
        return NumComplex::new(value.re.signum(), 0.0_f64.copysign((2.0 * value.im).sin()));
    }
    let denominator = (2.0 * value.re).cosh() + (2.0 * value.im).cos();
    NumComplex::new(
        (2.0 * value.re).sinh() / denominator,
        (2.0 * value.im).sin() / denominator,
    )
}

fn complex_tanh_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    if value.im == 0.0 {
        return NumComplex::new(value.re.tanh(), value.im);
    }
    if value.re.abs() > 40.0 {
        return NumComplex::new(value.re.signum(), 0.0_f32.copysign((2.0 * value.im).sin()));
    }
    let denominator = (2.0 * value.re).cosh() + (2.0 * value.im).cos();
    NumComplex::new(
        (2.0 * value.re).sinh() / denominator,
        (2.0 * value.im).sin() / denominator,
    )
}

fn matlab_mod_f64(left: f64, right: f64) -> f64 {
    if right == 0.0 {
        return left;
    }
    if !left.is_finite() || right.is_nan() {
        return f64::NAN;
    }
    if right.is_infinite() {
        if left == 0.0 {
            return 0.0_f64.copysign(right);
        }
        return if left.is_sign_positive() == right.is_sign_positive() {
            left
        } else {
            right
        };
    }
    let remainder = left % right;
    if remainder == 0.0 {
        0.0_f64.copysign(right)
    } else if remainder.is_sign_positive() == right.is_sign_positive() {
        remainder
    } else {
        remainder + right
    }
}

fn matlab_mod_f32(left: f32, right: f32) -> f32 {
    if right == 0.0 {
        return left;
    }
    if !left.is_finite() || right.is_nan() {
        return f32::NAN;
    }
    if right.is_infinite() {
        if left == 0.0 {
            return 0.0_f32.copysign(right);
        }
        return if left.is_sign_positive() == right.is_sign_positive() {
            left
        } else {
            right
        };
    }
    let remainder = left % right;
    if remainder == 0.0 {
        0.0_f32.copysign(right)
    } else if remainder.is_sign_positive() == right.is_sign_positive() {
        remainder
    } else {
        remainder + right
    }
}

#[allow(clippy::cast_possible_truncation)]
fn f64_to_f32(value: f64) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_branch_conventions_match_r2022b() {
        assert_eq!(asin_real_f64(-2.0).im.to_bits(), 2.0_f64.acosh().to_bits());
        assert_eq!(
            asin_real_f64(2.0).im.to_bits(),
            (-2.0_f64.acosh()).to_bits()
        );
        assert_eq!(
            acosh_real_f64(-1.0).im.to_bits(),
            std::f64::consts::PI.to_bits()
        );
        assert_eq!(
            atanh_real_f64(-2.0).im.to_bits(),
            (-std::f64::consts::FRAC_PI_2).to_bits()
        );
        assert_eq!(
            atanh_real_f64(2.0).im.to_bits(),
            std::f64::consts::FRAC_PI_2.to_bits()
        );
    }

    #[test]
    fn mod_zero_infinity_and_sign_rules_match_r2022b() {
        assert_eq!(matlab_mod_f64(-1.0, 0.0).to_bits(), (-1.0_f64).to_bits());
        assert!(matlab_mod_f64(-5.0, f64::INFINITY).is_infinite());
        assert!(matlab_mod_f64(-5.0, f64::INFINITY).is_sign_positive());
        assert!(matlab_mod_f64(5.0, f64::NEG_INFINITY).is_infinite());
        assert!(matlab_mod_f64(5.0, f64::NEG_INFINITY).is_sign_negative());
        assert!(matlab_mod_f64(0.0, -3.0).is_sign_negative());
    }
}

//! MATLAB-compatible degree, reciprocal, and base-two math built-ins.
use num_complex::Complex as NumComplex;
use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::core_elementary::{
    complex_f32_array_output, complex_f64_array_output, complex_scalar_output, map_values,
};
use crate::{
    array_error, expect_argument_count, expect_argument_count_range, expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;
const DEGREES_PER_RADIAN_F64: f64 = 180.0 / std::f64::consts::PI;
const DEGREES_PER_RADIAN_F32: f32 = 180.0 / std::f32::consts::PI;
const RADIANS_PER_DEGREE_F64: f64 = std::f64::consts::PI / 180.0;
const RADIANS_PER_DEGREE_F32: f32 = std::f32::consts::PI / 180.0;
const LOG_TWO_F64: f64 = std::f64::consts::LN_2;
const LOG_TWO_F32: f32 = std::f32::consts::LN_2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnaryExtendedMath {
    Sind,
    Cosd,
    Tand,
    Asind,
    Acosd,
    Atand,
    Sec,
    Csc,
    Cot,
    Sech,
    Csch,
    Coth,
    Asec,
    Acsc,
    Acot,
    Asech,
    Acsch,
    Acoth,
}

impl UnaryExtendedMath {
    const fn name(self) -> &'static str {
        match self {
            Self::Sind => "sind",
            Self::Cosd => "cosd",
            Self::Tand => "tand",
            Self::Asind => "asind",
            Self::Acosd => "acosd",
            Self::Atand => "atand",
            Self::Sec => "sec",
            Self::Csc => "csc",
            Self::Cot => "cot",
            Self::Sech => "sech",
            Self::Csch => "csch",
            Self::Coth => "coth",
            Self::Asec => "asec",
            Self::Acsc => "acsc",
            Self::Acot => "acot",
            Self::Asech => "asech",
            Self::Acsch => "acsch",
            Self::Acoth => "acoth",
        }
    }

    fn real_f64(self, value: f64) -> ArrayComplex64 {
        let output = match self {
            Self::Sind => NumComplex::new(sind_f64(value), 0.0),
            Self::Cosd => NumComplex::new(cosd_f64(value), 0.0),
            Self::Tand => NumComplex::new(tand_f64(value), 0.0),
            Self::Asind => scale_complex_f64(asin_real_f64(value), DEGREES_PER_RADIAN_F64),
            Self::Acosd => scale_complex_f64(acos_real_f64(value), DEGREES_PER_RADIAN_F64),
            Self::Atand => NumComplex::new(value.atan() * DEGREES_PER_RADIAN_F64, 0.0),
            Self::Sec => NumComplex::new(value.cos().recip(), 0.0),
            Self::Csc => NumComplex::new(value.sin().recip(), 0.0),
            Self::Cot => NumComplex::new(value.tan().recip(), 0.0),
            Self::Sech => NumComplex::new(value.cosh().recip(), 0.0),
            Self::Csch => NumComplex::new(value.sinh().recip(), 0.0),
            Self::Coth => NumComplex::new(value.tanh().recip(), 0.0),
            Self::Asec => acos_real_f64(value.recip()),
            Self::Acsc => asin_real_f64(value.recip()),
            Self::Acot => NumComplex::new(value.recip().atan(), 0.0),
            Self::Asech => acosh_real_f64(value.recip()),
            Self::Acsch => NumComplex::new(value.recip().asinh(), 0.0),
            Self::Acoth => atanh_real_f64(value.recip()),
        };
        ArrayComplex64::new(output.re, output.im)
    }

    fn real_f32(self, value: f32) -> ArrayComplex32 {
        let output = match self {
            Self::Sind => NumComplex::new(sind_f32(value), 0.0),
            Self::Cosd => NumComplex::new(cosd_f32(value), 0.0),
            Self::Tand => NumComplex::new(tand_f32(value), 0.0),
            Self::Asind => scale_complex_f32(asin_real_f32(value), DEGREES_PER_RADIAN_F32),
            Self::Acosd => scale_complex_f32(acos_real_f32(value), DEGREES_PER_RADIAN_F32),
            Self::Atand => NumComplex::new(value.atan() * DEGREES_PER_RADIAN_F32, 0.0),
            Self::Sec => NumComplex::new(value.cos().recip(), 0.0),
            Self::Csc => NumComplex::new(value.sin().recip(), 0.0),
            Self::Cot => NumComplex::new(value.tan().recip(), 0.0),
            Self::Sech => NumComplex::new(value.cosh().recip(), 0.0),
            Self::Csch => NumComplex::new(value.sinh().recip(), 0.0),
            Self::Coth => NumComplex::new(value.tanh().recip(), 0.0),
            Self::Asec => acos_real_f32(value.recip()),
            Self::Acsc => asin_real_f32(value.recip()),
            Self::Acot => NumComplex::new(value.recip().atan(), 0.0),
            Self::Asech => acosh_real_f32(value.recip()),
            Self::Acsch => NumComplex::new(value.recip().asinh(), 0.0),
            Self::Acoth => atanh_real_f32(value.recip()),
        };
        ArrayComplex32::new(output.re, output.im)
    }

    fn complex_f64(self, value: ArrayComplex64) -> ArrayComplex64 {
        if value.im == 0.0 {
            return self.real_f64(value.re);
        }
        let value = NumComplex::new(value.re, value.im);
        let output = match self {
            Self::Sind => complex_sind_f64(value),
            Self::Cosd => complex_cosd_f64(value),
            Self::Tand => complex_tand_f64(value),
            Self::Asind => value.asin() * DEGREES_PER_RADIAN_F64,
            Self::Acosd => value.acos() * DEGREES_PER_RADIAN_F64,
            Self::Atand => complex_atan_f64(value) * DEGREES_PER_RADIAN_F64,
            Self::Sec => reciprocal_f64(value.cos()),
            Self::Csc => reciprocal_f64(value.sin()),
            Self::Cot => reciprocal_f64(value.tan()),
            Self::Sech => complex_sech_f64(value),
            Self::Csch => complex_csch_f64(value),
            Self::Coth => complex_coth_f64(value),
            Self::Asec => reciprocal_f64(value).acos(),
            Self::Acsc => reciprocal_f64(value).asin(),
            Self::Acot => complex_atan_f64(reciprocal_f64(value)),
            Self::Asech => reciprocal_f64(value).acosh(),
            Self::Acsch => reciprocal_f64(value).asinh(),
            Self::Acoth => reciprocal_f64(value).atanh(),
        };
        ArrayComplex64::new(output.re, output.im)
    }

    fn complex_f32(self, value: ArrayComplex32) -> ArrayComplex32 {
        if value.im == 0.0 {
            return self.real_f32(value.re);
        }
        let value = NumComplex::new(value.re, value.im);
        let output = match self {
            Self::Sind => complex_sind_f32(value),
            Self::Cosd => complex_cosd_f32(value),
            Self::Tand => complex_tand_f32(value),
            Self::Asind => value.asin() * DEGREES_PER_RADIAN_F32,
            Self::Acosd => value.acos() * DEGREES_PER_RADIAN_F32,
            Self::Atand => complex_atan_f32(value) * DEGREES_PER_RADIAN_F32,
            Self::Sec => reciprocal_f32(value.cos()),
            Self::Csc => reciprocal_f32(value.sin()),
            Self::Cot => reciprocal_f32(value.tan()),
            Self::Sech => complex_sech_f32(value),
            Self::Csch => complex_csch_f32(value),
            Self::Coth => complex_coth_f32(value),
            Self::Asec => reciprocal_f32(value).acos(),
            Self::Acsc => reciprocal_f32(value).asin(),
            Self::Acot => complex_atan_f32(reciprocal_f32(value)),
            Self::Asech => reciprocal_f32(value).acosh(),
            Self::Acsch => reciprocal_f32(value).asinh(),
            Self::Acoth => reciprocal_f32(value).atanh(),
        };
        ArrayComplex32::new(output.re, output.im)
    }
}

macro_rules! unary_extended_builtin {
    ($function:ident, $operation:ident) => {
        pub(super) fn $function(
            arguments: &[Value],
            context: &mut BuiltinContext<'_>,
        ) -> BuiltinResult {
            unary_extended_math_builtin(UnaryExtendedMath::$operation, arguments, context)
        }
    };
}

unary_extended_builtin!(sind_builtin, Sind);
unary_extended_builtin!(cosd_builtin, Cosd);
unary_extended_builtin!(tand_builtin, Tand);
unary_extended_builtin!(asind_builtin, Asind);
unary_extended_builtin!(acosd_builtin, Acosd);
unary_extended_builtin!(atand_builtin, Atand);
unary_extended_builtin!(sec_builtin, Sec);
unary_extended_builtin!(csc_builtin, Csc);
unary_extended_builtin!(cot_builtin, Cot);
unary_extended_builtin!(sech_builtin, Sech);
unary_extended_builtin!(csch_builtin, Csch);
unary_extended_builtin!(coth_builtin, Coth);
unary_extended_builtin!(asec_builtin, Asec);
unary_extended_builtin!(acsc_builtin, Acsc);
unary_extended_builtin!(acot_builtin, Acot);
unary_extended_builtin!(asech_builtin, Asech);
unary_extended_builtin!(acsch_builtin, Acsch);
unary_extended_builtin!(acoth_builtin, Acoth);

fn unary_extended_math_builtin(
    operation: UnaryExtendedMath,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = operation.name();
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let output = map_unary_input(
        name,
        &arguments[0],
        context,
        |value| operation.real_f64(value),
        |value| operation.complex_f64(value),
        |value| operation.real_f32(value),
        |value| operation.complex_f32(value),
    )?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn map_unary_input<FR64, FC64, FR32, FC32>(
    name: &str,
    input: &Value,
    context: &BuiltinContext<'_>,
    mut real_f64: FR64,
    mut complex_f64: FC64,
    mut real_f32: FR32,
    mut complex_f32: FC32,
) -> Result<Value, BuiltinError>
where
    FR64: FnMut(f64) -> ArrayComplex64,
    FC64: FnMut(ArrayComplex64) -> ArrayComplex64,
    FR32: FnMut(f32) -> ArrayComplex32,
    FC32: FnMut(ArrayComplex32) -> ArrayComplex32,
{
    match input {
        Value::Double(value) => Ok(complex_scalar_output(real_f64(*value))),
        Value::Complex(value) => Ok(complex_scalar_output(complex_f64(ArrayComplex64::new(
            value.real,
            value.imaginary,
        )))),
        Value::Array(ArrayData::F64(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| real_f64(*value))?;
            complex_f64_array_output(array.shape().clone(), values)
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| complex_f64(*value))?;
            complex_f64_array_output(array.shape().clone(), values)
        }
        Value::Array(ArrayData::F32(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| real_f32(*value))?;
            complex_f32_array_output(array.shape().clone(), values)
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            let values = map_values(name, array.as_slice(), context, |value| complex_f32(*value))?;
            complex_f32_array_output(array.shape().clone(), values)
        }
        value => Err(type_error(name, 1, "double or single array", value)),
    }
}

pub(super) fn atan2d_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("atan2d", arguments, 2)?;
    expect_max_outputs("atan2d", context, 1)?;
    context.check_cancelled()?;
    let output = if has_single_input(arguments) {
        let left = real_single_input("atan2d", 1, &arguments[0], context, false)?;
        let right = real_single_input("atan2d", 2, &arguments[1], context, false)?;
        let shape = compatible_shape("atan2d", &left.shape, &right.shape)?;
        let values = map_binary("atan2d", &left, &right, &shape, context, |left, right| {
            left.atan2(right) * DEGREES_PER_RADIAN_F32
        })?;
        dense_single(shape, values)?
    } else {
        let left = real_double_input("atan2d", 1, &arguments[0], context, false)?;
        let right = real_double_input("atan2d", 2, &arguments[1], context, false)?;
        let shape = compatible_shape("atan2d", &left.shape, &right.shape)?;
        let scalar_variant = left.scalar_variant && right.scalar_variant;
        let values = map_binary("atan2d", &left, &right, &shape, context, |left, right| {
            left.atan2(right) * DEGREES_PER_RADIAN_F64
        })?;
        dense_double_or_scalar(shape, values, scalar_variant)?
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn log2_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("log2", arguments, 1)?;
    expect_max_outputs("log2", context, 2)?;
    context.check_cancelled()?;
    let outputs = if context.requested_outputs() <= 1 {
        vec![map_unary_input(
            "log2",
            &arguments[0],
            context,
            log2_real_f64,
            log2_complex_f64,
            log2_real_f32,
            log2_complex_f32,
        )?]
    } else {
        log2_parts(&arguments[0], context)?
    };
    context.check_cancelled()?;
    Ok(outputs)
}

pub(super) fn pow2_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("pow2", arguments, 1, 2)?;
    expect_max_outputs("pow2", context, 1)?;
    context.check_cancelled()?;
    let output = if arguments.len() == 1 {
        map_unary_input(
            "pow2",
            &arguments[0],
            context,
            pow2_real_f64,
            pow2_complex_f64,
            pow2_real_f32,
            pow2_complex_f32,
        )?
    } else if has_single_input(arguments) {
        pow2_binary_single(arguments, context)?
    } else {
        pow2_binary_double(arguments, context)?
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn nextpow2_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("nextpow2", arguments, 1)?;
    expect_max_outputs("nextpow2", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Logical(_) => Value::Double(0.0),
        Value::Double(value) => Value::Double(nextpow2_f64(value.abs())),
        Value::Complex(value) => Value::Double(nextpow2_f64(value.real.hypot(value.imaginary))),
        Value::Array(ArrayData::Logical(array)) => {
            let values = map_values("nextpow2", array.as_slice(), context, |_| 0.0)?;
            dense_double(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::F64(array)) => {
            let values = map_values("nextpow2", array.as_slice(), context, |value| {
                nextpow2_f64(value.abs())
            })?;
            dense_double(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let values = map_values("nextpow2", array.as_slice(), context, |value| {
                nextpow2_f64(value.re.hypot(value.im))
            })?;
            dense_double(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::F32(array)) => {
            let values = map_values("nextpow2", array.as_slice(), context, |value| {
                nextpow2_f32(value.abs())
            })?;
            dense_single(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            let values = map_values("nextpow2", array.as_slice(), context, |value| {
                nextpow2_f32(value.re.hypot(value.im))
            })?;
            dense_single(array.shape().clone(), values)?
        }
        Value::Array(ArrayData::Integer(array)) => {
            Value::Array(ArrayData::Integer(nextpow2_integer(array, context)?))
        }
        value => return Err(type_error("nextpow2", 1, "numeric or logical array", value)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

#[derive(Clone)]
struct RealInput<T> {
    shape: Shape,
    values: Vec<T>,
    scalar_variant: bool,
}

fn has_single_input(arguments: &[Value]) -> bool {
    arguments.iter().any(|value| {
        matches!(
            value,
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        )
    })
}

fn real_double_input(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
    ignore_complex_part: bool,
) -> Result<RealInput<f64>, BuiltinError> {
    let scalar_shape = || Shape::new([1, 1]).map_err(|error| array_error(&error));
    match value {
        Value::Double(value) => Ok(RealInput {
            shape: scalar_shape()?,
            values: vec![*value],
            scalar_variant: true,
        }),
        Value::Complex(value) if ignore_complex_part => Ok(RealInput {
            shape: scalar_shape()?,
            values: vec![value.real],
            scalar_variant: true,
        }),
        Value::Array(ArrayData::F64(array)) => Ok(RealInput {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| *value)?,
            scalar_variant: false,
        }),
        Value::Array(ArrayData::ComplexF64(array)) if ignore_complex_part => Ok(RealInput {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| value.re)?,
            scalar_variant: false,
        }),
        value => Err(type_error(
            name,
            position,
            "real double or single array",
            value,
        )),
    }
}

fn real_single_input(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
    ignore_complex_part: bool,
) -> Result<RealInput<f32>, BuiltinError> {
    match value {
        Value::Array(ArrayData::F32(array)) => Ok(RealInput {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| *value)?,
            scalar_variant: false,
        }),
        Value::Array(ArrayData::ComplexF32(array)) if ignore_complex_part => Ok(RealInput {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| value.re)?,
            scalar_variant: false,
        }),
        _ => {
            let input = real_double_input(name, position, value, context, ignore_complex_part)?;
            Ok(RealInput {
                shape: input.shape,
                values: map_values(name, &input.values, context, |value| f64_to_f32(*value))?,
                scalar_variant: input.scalar_variant,
            })
        }
    }
}

fn compatible_shape(name: &str, left: &Shape, right: &Shape) -> Result<Shape, BuiltinError> {
    let left_dimensions = left.dimensions();
    let right_dimensions = right.dimensions();
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

fn scalar_expansion_shape(name: &str, left: &Shape, right: &Shape) -> Result<Shape, BuiltinError> {
    if left.numel() == 1 {
        return Ok(right.clone());
    }
    if right.numel() == 1 || left.dimensions() == right.dimensions() {
        return Ok(left.clone());
    }
    Err(BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("inputs to `{name}` must have equal sizes or one input must be scalar"),
    ))
}

fn map_binary<T: Copy, U, F>(
    name: &str,
    left: &RealInput<T>,
    right: &RealInput<T>,
    shape: &Shape,
    context: &BuiltinContext<'_>,
    mut operation: F,
) -> Result<Vec<U>, BuiltinError>
where
    F: FnMut(T, T) -> U,
{
    let length = usize::try_from(shape.numel()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` output does not fit this host"),
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
                .ok_or_else(|| mapping_error(name))?,
            *right
                .values
                .get(right_offset)
                .ok_or_else(|| mapping_error(name))?,
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
    if input_shape.numel() == 1 {
        return Ok(0);
    }
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

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not map its operands"),
    )
}

fn dense_double(shape: Shape, values: Vec<f64>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn dense_double_or_scalar(
    shape: Shape,
    values: Vec<f64>,
    scalar_variant: bool,
) -> Result<Value, BuiltinError> {
    if scalar_variant {
        return values.first().copied().map(Value::Double).ok_or_else(|| {
            BuiltinError::new(BuiltinErrorCategory::Other, "scalar math result was empty")
        });
    }
    dense_double(shape, values)
}

fn dense_single(shape: Shape, values: Vec<f32>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F32)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn pow2_binary_double(
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let left = real_double_input("pow2", 1, &arguments[0], context, true)?;
    let right = real_double_input("pow2", 2, &arguments[1], context, true)?;
    let shape = scalar_expansion_shape("pow2", &left.shape, &right.shape)?;
    let scalar_variant = left.scalar_variant && right.scalar_variant;
    let values = map_binary("pow2", &left, &right, &shape, context, scale_pow2_f64)?;
    dense_double_or_scalar(shape, values, scalar_variant)
}

fn pow2_binary_single(
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let left = real_single_input("pow2", 1, &arguments[0], context, true)?;
    let right = real_single_input("pow2", 2, &arguments[1], context, true)?;
    let shape = scalar_expansion_shape("pow2", &left.shape, &right.shape)?;
    let values = map_binary("pow2", &left, &right, &shape, context, scale_pow2_f32)?;
    dense_single(shape, values)
}

fn log2_parts(input: &Value, context: &BuiltinContext<'_>) -> BuiltinResult {
    match input {
        Value::Double(value) => {
            let (fraction, exponent) = frexp_f64(*value);
            Ok(vec![Value::Double(fraction), Value::Double(exponent)])
        }
        Value::Complex(value) => {
            let (fraction, exponent) = frexp_f64(value.real);
            Ok(vec![Value::Double(fraction), Value::Double(exponent)])
        }
        Value::Array(ArrayData::F64(array)) => {
            let (fractions, exponents) = frexp_array_f64(array.as_slice(), context)?;
            Ok(vec![
                dense_double(array.shape().clone(), fractions)?,
                dense_double(array.shape().clone(), exponents)?,
            ])
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let real = map_values("log2", array.as_slice(), context, |value| value.re)?;
            let (fractions, exponents) = frexp_array_f64(&real, context)?;
            Ok(vec![
                dense_double(array.shape().clone(), fractions)?,
                dense_double(array.shape().clone(), exponents)?,
            ])
        }
        Value::Array(ArrayData::F32(array)) => {
            let (fractions, exponents) = frexp_array_f32(array.as_slice(), context)?;
            Ok(vec![
                dense_single(array.shape().clone(), fractions)?,
                dense_single(array.shape().clone(), exponents)?,
            ])
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            let real = map_values("log2", array.as_slice(), context, |value| value.re)?;
            let (fractions, exponents) = frexp_array_f32(&real, context)?;
            Ok(vec![
                dense_single(array.shape().clone(), fractions)?,
                dense_single(array.shape().clone(), exponents)?,
            ])
        }
        value => Err(type_error("log2", 1, "double or single array", value)),
    }
}

fn frexp_array_f64(
    input: &[f64],
    context: &BuiltinContext<'_>,
) -> Result<(Vec<f64>, Vec<f64>), BuiltinError> {
    let parts = map_values("log2", input, context, |value| frexp_f64(*value))?;
    Ok(parts.into_iter().unzip())
}

fn frexp_array_f32(
    input: &[f32],
    context: &BuiltinContext<'_>,
) -> Result<(Vec<f32>, Vec<f32>), BuiltinError> {
    let parts = map_values("log2", input, context, |value| frexp_f32(*value))?;
    Ok(parts.into_iter().unzip())
}

fn nextpow2_integer(
    input: &IntegerArrayData,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! signed_variant {
        ($array:expr, $variant:ident, $kind:ty) => {{
            let values = map_values("nextpow2", $array.as_slice(), context, |value| {
                let magnitude = value.unsigned_abs();
                let exponent = integer_nextpow2(magnitude);
                <$kind>::try_from(exponent).unwrap_or(<$kind>::MAX)
            })?;
            DenseArray::from_vec($array.shape().clone(), values)
                .map(IntegerArrayData::$variant)
                .map_err(|error| array_error(&error))
        }};
    }
    macro_rules! unsigned_variant {
        ($array:expr, $variant:ident, $kind:ty) => {{
            let values = map_values("nextpow2", $array.as_slice(), context, |value| {
                <$kind>::try_from(integer_nextpow2(*value)).unwrap_or(<$kind>::MAX)
            })?;
            DenseArray::from_vec($array.shape().clone(), values)
                .map(IntegerArrayData::$variant)
                .map_err(|error| array_error(&error))
        }};
    }

    match input {
        IntegerArrayData::I8(array) => signed_variant!(array, I8, i8),
        IntegerArrayData::U8(array) => unsigned_variant!(array, U8, u8),
        IntegerArrayData::I16(array) => signed_variant!(array, I16, i16),
        IntegerArrayData::U16(array) => unsigned_variant!(array, U16, u16),
        IntegerArrayData::I32(array) => signed_variant!(array, I32, i32),
        IntegerArrayData::U32(array) => unsigned_variant!(array, U32, u32),
        IntegerArrayData::I64(array) => signed_variant!(array, I64, i64),
        IntegerArrayData::U64(array) => unsigned_variant!(array, U64, u64),
        IntegerArrayData::ComplexI8(_)
        | IntegerArrayData::ComplexU8(_)
        | IntegerArrayData::ComplexI16(_)
        | IntegerArrayData::ComplexU16(_)
        | IntegerArrayData::ComplexI32(_)
        | IntegerArrayData::ComplexU32(_)
        | IntegerArrayData::ComplexI64(_)
        | IntegerArrayData::ComplexU64(_) => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "`nextpow2` does not accept complex integer arrays",
        )),
    }
}

fn integer_nextpow2<T>(value: T) -> u32
where
    T: Copy + Into<u128>,
{
    let value = value.into();
    if value <= 1 {
        0
    } else {
        u128::BITS - (value - 1).leading_zeros()
    }
}

fn reduced_degrees_f64(value: f64) -> f64 {
    let mut reduced = value % 360.0;
    if reduced > 180.0 {
        reduced -= 360.0;
    } else if reduced < -180.0 {
        reduced += 360.0;
    }
    reduced
}

fn reduced_degrees_f32(value: f32) -> f32 {
    let mut reduced = value % 360.0;
    if reduced > 180.0 {
        reduced -= 360.0;
    } else if reduced < -180.0 {
        reduced += 360.0;
    }
    reduced
}

fn sind_f64(value: f64) -> f64 {
    if !value.is_finite() {
        return f64::NAN;
    }
    let reduced = reduced_degrees_f64(value);
    if reduced == 0.0 || exact_f64(reduced.abs(), 180.0) {
        return 0.0_f64.copysign(value);
    }
    if exact_f64(reduced.abs(), 90.0) {
        return reduced.signum();
    }
    (reduced * RADIANS_PER_DEGREE_F64).sin()
}

fn sind_f32(value: f32) -> f32 {
    if !value.is_finite() {
        return f32::NAN;
    }
    let reduced = reduced_degrees_f32(value);
    if reduced == 0.0 || exact_f32(reduced.abs(), 180.0) {
        return 0.0_f32.copysign(value);
    }
    if exact_f32(reduced.abs(), 90.0) {
        return reduced.signum();
    }
    (reduced * RADIANS_PER_DEGREE_F32).sin()
}

fn cosd_f64(value: f64) -> f64 {
    if !value.is_finite() {
        return f64::NAN;
    }
    let reduced = reduced_degrees_f64(value);
    if reduced == 0.0 {
        return 1.0;
    }
    if exact_f64(reduced.abs(), 90.0) {
        return 0.0;
    }
    if exact_f64(reduced.abs(), 180.0) {
        return -1.0;
    }
    (reduced * RADIANS_PER_DEGREE_F64).cos()
}

fn cosd_f32(value: f32) -> f32 {
    if !value.is_finite() {
        return f32::NAN;
    }
    let reduced = reduced_degrees_f32(value);
    if reduced == 0.0 {
        return 1.0;
    }
    if exact_f32(reduced.abs(), 90.0) {
        return 0.0;
    }
    if exact_f32(reduced.abs(), 180.0) {
        return -1.0;
    }
    (reduced * RADIANS_PER_DEGREE_F32).cos()
}

fn tand_f64(value: f64) -> f64 {
    if !value.is_finite() {
        return f64::NAN;
    }
    let reduced = reduced_degrees_f64(value);
    if reduced == 0.0 || exact_f64(reduced.abs(), 180.0) {
        return 0.0_f64.copysign(value);
    }
    if exact_f64(reduced.abs(), 90.0) {
        return f64::INFINITY.copysign(reduced);
    }
    (reduced * RADIANS_PER_DEGREE_F64).tan()
}

fn tand_f32(value: f32) -> f32 {
    if !value.is_finite() {
        return f32::NAN;
    }
    let reduced = reduced_degrees_f32(value);
    if reduced == 0.0 || exact_f32(reduced.abs(), 180.0) {
        return 0.0_f32.copysign(value);
    }
    if exact_f32(reduced.abs(), 90.0) {
        return f32::INFINITY.copysign(reduced);
    }
    (reduced * RADIANS_PER_DEGREE_F32).tan()
}

fn complex_sind_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    let imaginary = value.im * RADIANS_PER_DEGREE_F64;
    NumComplex::new(
        sind_f64(value.re) * imaginary.cosh(),
        cosd_f64(value.re) * imaginary.sinh(),
    )
}

fn complex_sind_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    let imaginary = value.im * RADIANS_PER_DEGREE_F32;
    NumComplex::new(
        sind_f32(value.re) * imaginary.cosh(),
        cosd_f32(value.re) * imaginary.sinh(),
    )
}

fn complex_cosd_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    let imaginary = value.im * RADIANS_PER_DEGREE_F64;
    NumComplex::new(
        cosd_f64(value.re) * imaginary.cosh(),
        -sind_f64(value.re) * imaginary.sinh(),
    )
}

fn complex_cosd_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    let imaginary = value.im * RADIANS_PER_DEGREE_F32;
    NumComplex::new(
        cosd_f32(value.re) * imaginary.cosh(),
        -sind_f32(value.re) * imaginary.sinh(),
    )
}

fn complex_tand_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    if value.re.is_finite()
        && value.im != 0.0
        && exact_f64(reduced_degrees_f64(value.re).abs(), 90.0)
    {
        return NumComplex::new(f64::NAN, f64::NAN);
    }
    let imaginary = 2.0 * value.im * RADIANS_PER_DEGREE_F64;
    if imaginary.abs() > 350.0 {
        return NumComplex::new(
            0.0_f64.copysign(sind_f64(2.0 * value.re)),
            value.im.signum(),
        );
    }
    let denominator = cosd_f64(2.0 * value.re) + imaginary.cosh();
    NumComplex::new(
        sind_f64(2.0 * value.re) / denominator,
        imaginary.sinh() / denominator,
    )
}

fn complex_tand_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    if value.re.is_finite()
        && value.im != 0.0
        && exact_f32(reduced_degrees_f32(value.re).abs(), 90.0)
    {
        return NumComplex::new(f32::NAN, f32::NAN);
    }
    let imaginary = 2.0 * value.im * RADIANS_PER_DEGREE_F32;
    if imaginary.abs() > 40.0 {
        return NumComplex::new(
            0.0_f32.copysign(sind_f32(2.0 * value.re)),
            value.im.signum(),
        );
    }
    let denominator = cosd_f32(2.0 * value.re) + imaginary.cosh();
    NumComplex::new(
        sind_f32(2.0 * value.re) / denominator,
        imaginary.sinh() / denominator,
    )
}

fn complex_sech_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    if value.re.abs() > 350.0 && value.im.is_finite() {
        let magnitude = 2.0 * (-value.re.abs()).exp();
        return NumComplex::new(
            magnitude * value.im.cos(),
            -magnitude * value.re.signum() * value.im.sin(),
        );
    }
    reciprocal_f64(value.cosh())
}

fn complex_sech_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    if value.re.abs() > 40.0 && value.im.is_finite() {
        let magnitude = 2.0 * (-value.re.abs()).exp();
        return NumComplex::new(
            magnitude * value.im.cos(),
            -magnitude * value.re.signum() * value.im.sin(),
        );
    }
    reciprocal_f32(value.cosh())
}

fn complex_csch_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    if value.re.abs() > 350.0 && value.im.is_finite() {
        let magnitude = 2.0 * (-value.re.abs()).exp();
        return NumComplex::new(
            magnitude * value.re.signum() * value.im.cos(),
            -magnitude * value.im.sin(),
        );
    }
    reciprocal_f64(value.sinh())
}

fn complex_csch_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    if value.re.abs() > 40.0 && value.im.is_finite() {
        let magnitude = 2.0 * (-value.re.abs()).exp();
        return NumComplex::new(
            magnitude * value.re.signum() * value.im.cos(),
            -magnitude * value.im.sin(),
        );
    }
    reciprocal_f32(value.sinh())
}

fn complex_coth_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    if value.re.abs() > 350.0 && value.im.is_finite() {
        return NumComplex::new(value.re.signum(), 0.0_f64.copysign(-(2.0 * value.im).sin()));
    }
    reciprocal_f64(value.tanh())
}

fn complex_coth_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    if value.re.abs() > 40.0 && value.im.is_finite() {
        return NumComplex::new(value.re.signum(), 0.0_f32.copysign(-(2.0 * value.im).sin()));
    }
    reciprocal_f32(value.tanh())
}

fn reciprocal_f64(value: NumComplex<f64>) -> NumComplex<f64> {
    if value.re.is_nan() || value.im.is_nan() {
        return NumComplex::new(f64::NAN, f64::NAN);
    }
    if value.re.is_infinite() || value.im.is_infinite() {
        return NumComplex::new(0.0_f64.copysign(value.re), 0.0_f64.copysign(-value.im));
    }
    NumComplex::new(1.0, 0.0) / value
}

fn reciprocal_f32(value: NumComplex<f32>) -> NumComplex<f32> {
    if value.re.is_nan() || value.im.is_nan() {
        return NumComplex::new(f32::NAN, f32::NAN);
    }
    if value.re.is_infinite() || value.im.is_infinite() {
        return NumComplex::new(0.0_f32.copysign(value.re), 0.0_f32.copysign(-value.im));
    }
    NumComplex::new(1.0, 0.0) / value
}

fn scale_complex_f64(value: NumComplex<f64>, scale: f64) -> NumComplex<f64> {
    NumComplex::new(value.re * scale, value.im * scale)
}

fn scale_complex_f32(value: NumComplex<f32>, scale: f32) -> NumComplex<f32> {
    NumComplex::new(value.re * scale, value.im * scale)
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
        NumComplex::new(
            0.0_f64.copysign(value),
            std::f64::consts::FRAC_PI_2.copysign(value),
        )
    } else {
        let real = 0.5 * ((1.0 + value).abs().ln() - (1.0 - value).abs().ln());
        NumComplex::new(real, std::f64::consts::FRAC_PI_2.copysign(value))
    }
}

fn atanh_real_f32(value: f32) -> NumComplex<f32> {
    if value.abs() <= 1.0 || value.is_nan() {
        NumComplex::new(value.atanh(), 0.0)
    } else if value.is_infinite() {
        NumComplex::new(
            0.0_f32.copysign(value),
            std::f32::consts::FRAC_PI_2.copysign(value),
        )
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

fn log2_real_f64(value: f64) -> ArrayComplex64 {
    if value < 0.0 {
        ArrayComplex64::new((-value).log2(), std::f64::consts::PI / LOG_TWO_F64)
    } else {
        ArrayComplex64::new(value.log2(), 0.0)
    }
}

fn log2_real_f32(value: f32) -> ArrayComplex32 {
    if value < 0.0 {
        ArrayComplex32::new((-value).log2(), std::f32::consts::PI / LOG_TWO_F32)
    } else {
        ArrayComplex32::new(value.log2(), 0.0)
    }
}

fn log2_complex_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 && (value.re == 0.0 || !value.re.is_sign_negative()) {
        return log2_real_f64(value.re);
    }
    let output = NumComplex::new(value.re, value.im).ln() / LOG_TWO_F64;
    ArrayComplex64::new(output.re, output.im)
}

fn log2_complex_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 && (value.re == 0.0 || !value.re.is_sign_negative()) {
        return log2_real_f32(value.re);
    }
    let output = NumComplex::new(value.re, value.im).ln() / LOG_TWO_F32;
    ArrayComplex32::new(output.re, output.im)
}

fn pow2_real_f64(value: f64) -> ArrayComplex64 {
    ArrayComplex64::new(value.exp2(), 0.0)
}

fn pow2_real_f32(value: f32) -> ArrayComplex32 {
    ArrayComplex32::new(value.exp2(), 0.0)
}

fn pow2_complex_f64(value: ArrayComplex64) -> ArrayComplex64 {
    if value.im == 0.0 {
        return pow2_real_f64(value.re);
    }
    let output = (NumComplex::new(value.re, value.im) * LOG_TWO_F64).exp();
    ArrayComplex64::new(output.re, output.im)
}

fn pow2_complex_f32(value: ArrayComplex32) -> ArrayComplex32 {
    if value.im == 0.0 {
        return pow2_real_f32(value.re);
    }
    let output = (NumComplex::new(value.re, value.im) * LOG_TWO_F32).exp();
    ArrayComplex32::new(output.re, output.im)
}

fn frexp_f64(value: f64) -> (f64, f64) {
    if value == 0.0 || !value.is_finite() {
        return (value, 0.0);
    }
    let mut normalized = value;
    let mut adjustment = 0_i32;
    let mut bits = normalized.to_bits();
    if bits & 0x7ff0_0000_0000_0000 == 0 {
        normalized *= f64::from_bits((1_023_u64 + 54) << 52);
        bits = normalized.to_bits();
        adjustment = -54;
    }
    let biased = i32::try_from((bits >> 52) & 0x7ff).unwrap_or_default();
    let exponent = biased - 1_022 + adjustment;
    let fraction_bits =
        (bits & 0x800f_ffff_ffff_ffff) | (u64::try_from(1_022).unwrap_or_default() << 52);
    (f64::from_bits(fraction_bits), f64::from(exponent))
}

fn frexp_f32(value: f32) -> (f32, f32) {
    if value == 0.0 || !value.is_finite() {
        return (value, 0.0);
    }
    let mut normalized = value;
    let mut adjustment = 0_i32;
    let mut bits = normalized.to_bits();
    if bits & 0x7f80_0000 == 0 {
        normalized *= f32::from_bits((127_u32 + 25) << 23);
        bits = normalized.to_bits();
        adjustment = -25;
    }
    let biased = i32::try_from((bits >> 23) & 0xff).unwrap_or_default();
    let exponent = biased - 126 + adjustment;
    let fraction_bits = (bits & 0x807f_ffff) | (126_u32 << 23);
    (f32::from_bits(fraction_bits), i32_to_f32(exponent))
}

fn scale_pow2_f64(value: f64, exponent: f64) -> f64 {
    if exponent.is_nan() || value.is_nan() {
        return f64::NAN.copysign(value);
    }
    if value == 0.0 || value.is_infinite() {
        return value;
    }
    let exponent = exponent.trunc();
    if exponent > 4_096.0 {
        return f64::INFINITY.copysign(value);
    }
    if exponent < -4_096.0 {
        return 0.0_f64.copysign(value);
    }
    scalbn_f64(value, f64_to_i32(exponent))
}

fn scale_pow2_f32(value: f32, exponent: f32) -> f32 {
    if exponent.is_nan() || value.is_nan() {
        return f32::NAN.copysign(value);
    }
    if value == 0.0 || value.is_infinite() {
        return value;
    }
    let exponent = exponent.trunc();
    if exponent > 512.0 {
        return f32::INFINITY.copysign(value);
    }
    if exponent < -512.0 {
        return 0.0_f32.copysign(value);
    }
    scalbn_f32(value, f32_to_i32(exponent))
}

fn scalbn_f64(mut value: f64, mut exponent: i32) -> f64 {
    if exponent > 1_023 {
        value *= f64::from_bits(0x7fe0_0000_0000_0000);
        exponent -= 1_023;
        if exponent > 1_023 {
            value *= f64::from_bits(0x7fe0_0000_0000_0000);
            exponent -= 1_023;
            exponent = exponent.min(1_023);
        }
    } else if exponent < -1_022 {
        value *= f64::from_bits(0x0010_0000_0000_0000);
        exponent += 1_022;
        if exponent < -1_022 {
            value *= f64::from_bits(0x0010_0000_0000_0000);
            exponent += 1_022;
            exponent = exponent.max(-1_022);
        }
    }
    let power = u64::try_from(exponent + 1_023).unwrap_or_default() << 52;
    value * f64::from_bits(power)
}

fn scalbn_f32(mut value: f32, mut exponent: i32) -> f32 {
    if exponent > 127 {
        value *= f32::from_bits(0x7f00_0000);
        exponent -= 127;
        if exponent > 127 {
            value *= f32::from_bits(0x7f00_0000);
            exponent -= 127;
            exponent = exponent.min(127);
        }
    } else if exponent < -126 {
        value *= f32::from_bits(0x0080_0000);
        exponent += 126;
        if exponent < -126 {
            value *= f32::from_bits(0x0080_0000);
            exponent += 126;
            exponent = exponent.max(-126);
        }
    }
    let power = u32::try_from(exponent + 127).unwrap_or_default() << 23;
    value * f32::from_bits(power)
}

fn nextpow2_f64(magnitude: f64) -> f64 {
    if magnitude == 0.0 || !magnitude.is_finite() {
        return magnitude;
    }
    let (fraction, exponent) = frexp_f64(magnitude);
    if exact_f64(fraction, 0.5) {
        exponent - 1.0
    } else {
        exponent
    }
}

fn nextpow2_f32(magnitude: f32) -> f32 {
    if magnitude == 0.0 || !magnitude.is_finite() {
        return magnitude;
    }
    let (fraction, exponent) = frexp_f32(magnitude);
    if exact_f32(fraction, 0.5) {
        exponent - 1.0
    } else {
        exponent
    }
}

fn exact_f64(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

fn exact_f32(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

#[allow(clippy::cast_possible_truncation)]
fn f64_to_f32(value: f64) -> f32 {
    value as f32
}

#[allow(clippy::cast_possible_truncation)]
fn f64_to_i32(value: f64) -> i32 {
    value as i32
}

#[allow(clippy::cast_possible_truncation)]
fn f32_to_i32(value: f32) -> i32 {
    value as i32
}

#[allow(clippy::cast_precision_loss)]
fn i32_to_f32(value: i32) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use openmat_array::ComplexInteger;
    use openmat_runtime::{CancellationToken, VecOutput};

    use super::*;

    fn invoke(
        builtin: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
        arguments: &[Value],
        requested_outputs: usize,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut output = VecOutput::new();
        let mut context = BuiltinContext::new(requested_outputs, &cancellation, &mut output);
        builtin(arguments, &mut context)
    }

    fn double_array(dimensions: [u64; 2], values: Vec<f64>) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    #[test]
    fn degree_trigonometry_uses_matlab_exact_quadrants_and_signed_zero() {
        let input = double_array(
            [1, 9],
            vec![-360.0, -270.0, -180.0, -90.0, -0.0, 0.0, 90.0, 180.0, 270.0],
        );
        let Value::Array(ArrayData::F64(sine)) =
            &invoke(sind_builtin, std::slice::from_ref(&input), 1).unwrap()[0]
        else {
            panic!("sind must return a real double array");
        };
        assert_eq!(
            sine.as_slice()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            [-0.0, 1.0, -0.0, -1.0, -0.0, 0.0, 1.0, 0.0, -1.0].map(f64::to_bits)
        );

        let Value::Array(ArrayData::F64(cosine)) =
            &invoke(cosd_builtin, std::slice::from_ref(&input), 1).unwrap()[0]
        else {
            panic!("cosd must return a real double array");
        };
        assert_eq!(
            cosine.as_slice(),
            &[1.0, 0.0, -1.0, 0.0, 1.0, 1.0, 0.0, -1.0, 0.0]
        );

        let Value::Array(ArrayData::F64(tangent)) = &invoke(tand_builtin, &[input], 1).unwrap()[0]
        else {
            panic!("tand must return a real double array");
        };
        assert_eq!(
            tangent
                .as_slice()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            [
                -0.0,
                f64::INFINITY,
                -0.0,
                f64::NEG_INFINITY,
                -0.0,
                0.0,
                f64::INFINITY,
                0.0,
                f64::NEG_INFINITY,
            ]
            .map(f64::to_bits)
        );

        let complex_pole = Value::Complex(openmat_value::Complex64::new(90.0, 3.0));
        let Value::Complex(complex_pole) = invoke(tand_builtin, &[complex_pole], 1).unwrap()[0]
        else {
            panic!("complex tand must preserve complex storage at exact poles");
        };
        assert!(complex_pole.real.is_nan());
        assert!(complex_pole.imaginary.is_nan());
    }

    #[test]
    fn atan2d_broadcasts_and_preserves_axis_conventions() {
        let left = double_array([2, 1], vec![-0.0, 1.0]);
        let right = double_array([1, 3], vec![-1.0, 0.0, 1.0]);
        let Value::Array(ArrayData::F64(output)) =
            &invoke(atan2d_builtin, &[left, right], 1).unwrap()[0]
        else {
            panic!("atan2d must return a double array");
        };
        assert_eq!(output.shape().dimensions(), &[2, 3]);
        assert_eq!(output.as_slice(), &[-180.0, 135.0, -0.0, 90.0, -0.0, 45.0]);
        assert!(output.as_slice()[0].is_sign_negative());
        assert!(output.as_slice()[2].is_sign_negative());
    }

    #[test]
    fn reciprocal_functions_cover_real_poles_and_complex_domains() {
        assert_eq!(
            invoke(csc_builtin, &[Value::Double(-0.0)], 1),
            Ok(vec![Value::Double(f64::NEG_INFINITY)])
        );
        assert_eq!(
            invoke(coth_builtin, &[Value::Double(0.0)], 1),
            Ok(vec![Value::Double(f64::INFINITY)])
        );
        let output = invoke(asec_builtin, &[Value::Double(0.0)], 1).unwrap();
        let Value::Complex(value) = output[0] else {
            panic!("asec(0) must be complex");
        };
        assert_eq!(value.real.to_bits(), 0.0_f64.to_bits());
        assert_eq!(value.imaginary.to_bits(), f64::INFINITY.to_bits());

        let output = invoke(acoth_builtin, &[Value::Double(-0.0)], 1).unwrap();
        let Value::Complex(value) = output[0] else {
            panic!("acoth(-0) must be complex");
        };
        assert_eq!(value.real.to_bits(), (-0.0_f64).to_bits());
        assert_eq!(
            value.imaginary.to_bits(),
            (-std::f64::consts::FRAC_PI_2).to_bits()
        );
    }

    #[test]
    fn all_complex_formulas_match_r2022b_at_a_finite_reference_point() {
        type Builtin = fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult;
        let cases: [(Builtin, f64, f64); 20] = [
            (
                sind_builtin,
                0.017_463_040_130_989_836,
                0.034_908_356_719_322_166,
            ),
            (
                cosd_builtin,
                1.000_456_899_060_819_8,
                -0.000_609_327_633_073_149,
            ),
            (
                tand_builtin,
                0.017_433_807_258_672_687,
                0.034_903_032_457_085_134,
            ),
            (asind_builtin, 24.469_800_520_702_19, 87.580_662_372_692_79),
            (acosd_builtin, 65.530_199_479_297_8, -87.580_662_372_692_79),
            (atand_builtin, 76.717_474_411_461, 23.053_499_942_704_924),
            (
                sec_builtin,
                0.151_176_298_265_577_2,
                0.226_973_675_393_721_57,
            ),
            (
                csc_builtin,
                0.228_375_065_599_686_54,
                -0.141_363_021_612_407_8,
            ),
            (
                cot_builtin,
                0.032_797_755_533_752_52,
                -0.984_329_226_458_191,
            ),
            (
                sech_builtin,
                -0.413_149_344_266_940_06,
                -0.687_527_438_655_479,
            ),
            (
                csch_builtin,
                -0.221_500_930_850_509_43,
                -0.635_493_799_253_899_9,
            ),
            (
                coth_builtin,
                0.821_329_797_493_851_7,
                0.171_383_612_909_185_02,
            ),
            (asec_builtin, 1.384_478_272_687_081_5, 0.396_568_230_112_329),
            (
                acsc_builtin,
                0.186_318_054_107_815_54,
                -0.396_568_230_112_329,
            ),
            (
                acot_builtin,
                0.231_823_804_500_403_1,
                -0.402_359_478_108_525_07,
            ),
            (
                asech_builtin,
                0.396_568_230_112_329,
                -1.384_478_272_687_081_5,
            ),
            (
                acsch_builtin,
                0.215_612_418_555_829_66,
                -0.401_586_391_667_806_13,
            ),
            (
                acoth_builtin,
                0.173_286_795_139_986_32,
                -0.392_699_081_698_724_2,
            ),
            (
                log2_builtin,
                1.160_964_047_443_681_3,
                1.597_277_964_688_108_8,
            ),
            (
                pow2_builtin,
                0.366_913_949_486_603_44,
                1.966_055_480_822_487_5,
            ),
        ];
        let input = Value::Complex(openmat_value::Complex64::new(1.0, 2.0));
        for (builtin, expected_real, expected_imaginary) in cases {
            let output = invoke(builtin, std::slice::from_ref(&input), 1).unwrap();
            let Value::Complex(value) = output[0] else {
                panic!("finite complex reference output must remain complex");
            };
            assert!((value.real - expected_real).abs() < 2.0e-13);
            assert!((value.imaginary - expected_imaginary).abs() < 2.0e-13);
        }
    }

    #[test]
    fn complex_infinity_limits_follow_r2022b() {
        let input = Value::Complex(openmat_value::Complex64::new(f64::INFINITY, 1.0));
        assert_eq!(
            invoke(sech_builtin, std::slice::from_ref(&input), 1),
            Ok(vec![Value::Double(0.0)])
        );
        assert_eq!(
            invoke(csch_builtin, std::slice::from_ref(&input), 1),
            Ok(vec![Value::Double(0.0)])
        );
        assert_eq!(
            invoke(coth_builtin, std::slice::from_ref(&input), 1),
            Ok(vec![Value::Double(1.0)])
        );
        assert_eq!(
            invoke(asec_builtin, std::slice::from_ref(&input), 1),
            Ok(vec![Value::Double(std::f64::consts::FRAC_PI_2)])
        );
        assert_eq!(
            invoke(acsc_builtin, std::slice::from_ref(&input), 1),
            Ok(vec![Value::Double(0.0)])
        );
        assert_eq!(
            invoke(acot_builtin, std::slice::from_ref(&input), 1),
            Ok(vec![Value::Double(0.0)])
        );
        let output = invoke(asech_builtin, &[input], 1).unwrap();
        let Value::Complex(value) = output[0] else {
            panic!("asech(inf+i) has a purely imaginary complex limit");
        };
        assert_eq!(value.real.to_bits(), 0.0_f64.to_bits());
        assert_eq!(
            value.imaginary.to_bits(),
            std::f64::consts::FRAC_PI_2.to_bits()
        );

        let negative = Value::Complex(openmat_value::Complex64::new(f64::NEG_INFINITY, 1.0));
        let output = invoke(asech_builtin, std::slice::from_ref(&negative), 1).unwrap();
        let Value::Complex(value) = output[0] else {
            panic!("asech(-inf+i) has a purely imaginary complex limit");
        };
        assert_eq!(value.real.to_bits(), 0.0_f64.to_bits());
        assert_eq!(
            value.imaginary.to_bits(),
            std::f64::consts::FRAC_PI_2.to_bits()
        );
        assert_eq!(
            invoke(acsch_builtin, std::slice::from_ref(&negative), 1),
            Ok(vec![Value::Double(-0.0)])
        );
        assert_eq!(
            invoke(acoth_builtin, &[negative], 1),
            Ok(vec![Value::Double(-0.0)])
        );
    }

    #[test]
    fn log2_one_and_two_output_forms_preserve_class_and_complex_rules() {
        let one = invoke(log2_builtin, &[Value::Double(-8.0)], 1).unwrap();
        let Value::Complex(value) = one[0] else {
            panic!("one-output log2 of a negative value must be complex");
        };
        assert_eq!(value.real.to_bits(), 3.0_f64.to_bits());
        assert_eq!(
            value.imaginary.to_bits(),
            (std::f64::consts::PI / LOG_TWO_F64).to_bits()
        );

        let two = invoke(
            log2_builtin,
            &[Value::Complex(openmat_value::Complex64::new(-8.0, 4.0))],
            2,
        )
        .unwrap();
        assert_eq!(two, vec![Value::Double(-0.5), Value::Double(4.0)]);

        let single = Value::Array(ArrayData::F32(
            DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![3.0, 8.0]).unwrap(),
        ));
        let output = invoke(log2_builtin, &[single], 2).unwrap();
        for value in output {
            assert!(matches!(value, Value::Array(ArrayData::F32(_))));
        }

        assert_eq!(frexp_f64(f64::MAX), (1.0 - f64::EPSILON / 2.0, 1_024.0));
        assert_eq!(frexp_f64(f64::from_bits(1)), (0.5, -1_073.0));
        assert_eq!(scale_pow2_f64(1.0, -1_074.0).to_bits(), 1);
    }

    #[test]
    fn pow2_binary_truncates_exponents_and_only_scalar_expands() {
        let values = double_array([1, 4], vec![3.0, 3.0, 3.0, 3.0]);
        let exponents = double_array([1, 4], vec![-1.9, -0.9, 1.9, 2.1]);
        let Value::Array(ArrayData::F64(output)) =
            &invoke(pow2_builtin, &[values, exponents], 1).unwrap()[0]
        else {
            panic!("pow2 must return a double array");
        };
        assert_eq!(output.as_slice(), &[1.5, 3.0, 6.0, 12.0]);

        let broadcast = invoke(
            pow2_builtin,
            &[
                double_array([1, 3], vec![1.0, 2.0, 3.0]),
                Value::Double(2.0),
            ],
            1,
        )
        .unwrap();
        let Value::Array(ArrayData::F64(output)) = &broadcast[0] else {
            panic!("pow2 scalar expansion must retain array shape");
        };
        assert_eq!(output.as_slice(), &[4.0, 8.0, 12.0]);

        let error = invoke(
            pow2_builtin,
            &[
                double_array([2, 1], vec![1.0, 2.0]),
                double_array([1, 2], vec![1.0, 2.0]),
            ],
            1,
        )
        .unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn nextpow2_preserves_integer_classes_and_uses_complex_magnitude() {
        let integers = Value::Array(ArrayData::Integer(IntegerArrayData::I8(
            DenseArray::from_vec(
                Shape::new([1, 5]).unwrap(),
                vec![i8::MIN, -3, 0, 3, i8::MAX],
            )
            .unwrap(),
        )));
        let output = invoke(nextpow2_builtin, &[integers], 1).unwrap();
        let Value::Array(ArrayData::Integer(IntegerArrayData::I8(output))) = &output[0] else {
            panic!("integer nextpow2 must preserve its class");
        };
        assert_eq!(output.as_slice(), &[7, 2, 0, 2, 7]);

        assert_eq!(
            invoke(
                nextpow2_builtin,
                &[Value::Complex(openmat_value::Complex64::new(3.0, 4.0))],
                1,
            ),
            Ok(vec![Value::Double(3.0)])
        );
        assert_eq!(
            invoke(nextpow2_builtin, &[Value::Logical(true)], 1),
            Ok(vec![Value::Double(0.0)])
        );
    }

    #[test]
    fn extended_unary_math_rejects_logical_integer_and_excess_outputs() {
        type Builtin = fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult;
        let unary_builtins: [Builtin; 20] = [
            sind_builtin,
            cosd_builtin,
            tand_builtin,
            asind_builtin,
            acosd_builtin,
            atand_builtin,
            sec_builtin,
            csc_builtin,
            cot_builtin,
            sech_builtin,
            csch_builtin,
            coth_builtin,
            asec_builtin,
            acsc_builtin,
            acot_builtin,
            asech_builtin,
            acsch_builtin,
            acoth_builtin,
            log2_builtin,
            pow2_builtin,
        ];
        for builtin in unary_builtins {
            let logical_error = invoke(builtin, &[Value::Logical(true)], 1).unwrap_err();
            assert_eq!(logical_error.category, BuiltinErrorCategory::Type);
        }
        let atan2d_error = invoke(
            atan2d_builtin,
            &[Value::Logical(true), Value::Double(1.0)],
            1,
        )
        .unwrap_err();
        assert_eq!(atan2d_error.category, BuiltinErrorCategory::Type);
        let integer = Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![ComplexInteger::new(1, 2)])
                .unwrap(),
        )));
        let integer_error = invoke(sec_builtin, &[integer], 1).unwrap_err();
        assert_eq!(integer_error.category, BuiltinErrorCategory::Type);
        let pow2_error =
            invoke(pow2_builtin, &[Value::Double(1.0), Value::Logical(true)], 1).unwrap_err();
        assert_eq!(pow2_error.category, BuiltinErrorCategory::Type);
        let output_error = invoke(acosd_builtin, &[Value::Double(0.0)], 2).unwrap_err();
        assert_eq!(output_error.category, BuiltinErrorCategory::ArgumentCount);
    }

    #[test]
    fn empty_shapes_and_single_class_are_preserved() {
        let empty = Value::Array(ArrayData::ComplexF32(
            DenseArray::from_vec(Shape::new([0, 3]).unwrap(), Vec::new()).unwrap(),
        ));
        let output = invoke(asech_builtin, &[empty], 1).unwrap();
        let Value::Array(ArrayData::F32(output)) = &output[0] else {
            panic!("an all-real empty complex single result uses real single storage");
        };
        assert_eq!(output.shape().dimensions(), &[0, 3]);
    }
}

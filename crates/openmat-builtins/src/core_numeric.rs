use openmat_array::{
    ArrayData, CharCodeUnit, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64,
    ComplexInteger, DenseArray, IntegerArrayData, IntegerComponent, IntegerElement, Logical, Shape,
};
use openmat_linalg::{LinalgProvider, ReferenceProvider, SvdRequest, SvdVectors};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{Complex64, Value};

use crate::{
    array_error, expect_argument_count, expect_argument_count_range, expect_max_outputs, type_error,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumericClass {
    Logical,
    Real,
    Complex,
}

struct NumericData {
    dimensions: Vec<u64>,
    values: Vec<ArrayComplex64>,
    class: NumericClass,
    scalar_variant: bool,
}

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy)]
enum SourceComponent {
    Float(f64),
    Signed(i128),
    Unsigned(u128),
}

#[derive(Clone, Copy)]
struct SourceNumber {
    real: SourceComponent,
    imaginary: SourceComponent,
}

struct ConversionInput {
    shape: Shape,
    values: Vec<SourceNumber>,
    complex: bool,
}

#[derive(Clone, Copy)]
enum IntegerTarget {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
}

pub(super) fn int8_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    integer_builtin("int8", IntegerTarget::I8, arguments, context)
}

pub(super) fn uint8_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    integer_builtin("uint8", IntegerTarget::U8, arguments, context)
}

pub(super) fn int16_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    integer_builtin("int16", IntegerTarget::I16, arguments, context)
}

pub(super) fn uint16_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    integer_builtin("uint16", IntegerTarget::U16, arguments, context)
}

pub(super) fn int32_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    integer_builtin("int32", IntegerTarget::I32, arguments, context)
}

pub(super) fn uint32_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    integer_builtin("uint32", IntegerTarget::U32, arguments, context)
}

pub(super) fn int64_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    integer_builtin("int64", IntegerTarget::I64, arguments, context)
}

pub(super) fn uint64_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    integer_builtin("uint64", IntegerTarget::U64, arguments, context)
}

pub(super) fn char_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("char", arguments, 1)?;
    expect_max_outputs("char", context, 1)?;
    context.check_cancelled()?;
    if let Value::Array(ArrayData::Char(array)) = &arguments[0] {
        return Ok(vec![Value::Array(ArrayData::Char(array.clone()))]);
    }
    if let Value::String(string) = &arguments[0] {
        let scalar = string.as_scalar().ok_or_else(|| {
            type_error(
                "char",
                1,
                "string scalar, real numeric, logical, char, or integer array",
                &arguments[0],
            )
        })?;
        if scalar.is_missing() {
            return Err(type_error(
                "char",
                1,
                "nonmissing string scalar",
                &arguments[0],
            ));
        }
        let values = scalar
            .code_units()
            .iter()
            .copied()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        let columns = u64::try_from(values.len()).map_err(|_| {
            BuiltinError::new(BuiltinErrorCategory::Domain, "`char` output is too large")
        })?;
        let dimensions = if columns == 0 { [0, 0] } else { [1, columns] };
        let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
        let array = DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?;
        return Ok(vec![Value::Array(ArrayData::Char(array))]);
    }
    let input = conversion_input("char", &arguments[0], context)?;
    if input.complex {
        return Err(type_error(
            "char",
            1,
            "real numeric, logical, char, or integer array",
            &arguments[0],
        ));
    }
    let mut values = reserved_vec("char", input.values.len())?;
    for (index, value) in input.values.iter().enumerate() {
        check_cancelled_at(context, index)?;
        let value = convert_char_component(value.real);
        values.push(CharCodeUnit::new(
            u16::try_from(value).expect("char conversion clamps to u16"),
        ));
    }
    let array = DenseArray::from_vec(input.shape, values).map_err(|error| array_error(&error))?;
    context.check_cancelled()?;
    Ok(vec![Value::Array(ArrayData::Char(array))])
}

pub(super) fn single_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("single", arguments, 1)?;
    expect_max_outputs("single", context, 1)?;
    context.check_cancelled()?;
    if let Value::Array(array @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) = &arguments[0] {
        return Ok(vec![Value::Array(array.clone())]);
    }

    let input = conversion_input("single", &arguments[0], context)?;
    let output = if input.complex {
        let values = checked_map_slice("single", &input.values, context, |value| {
            ArrayComplex32::new(
                source_component_to_f32(value.real),
                source_component_to_f32(value.imaginary),
            )
        })?;
        DenseArray::from_vec(input.shape, values)
            .map(ArrayData::ComplexF32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))?
    } else {
        let values = checked_map_slice("single", &input.values, context, |value| {
            source_component_to_f32(value.real)
        })?;
        DenseArray::from_vec(input.shape, values)
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))?
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn source_component_to_f32(value: SourceComponent) -> f32 {
    match value {
        SourceComponent::Float(value) => value as f32,
        SourceComponent::Signed(value) => value as f32,
        SourceComponent::Unsigned(value) => value as f32,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn convert_char_component(value: SourceComponent) -> u128 {
    match value {
        SourceComponent::Signed(value) => {
            u128::try_from(value).map_or(0, |value| value.min(u128::from(u16::MAX)))
        }
        SourceComponent::Unsigned(value) => value.min(u128::from(u16::MAX)),
        SourceComponent::Float(value) => {
            if value.is_nan() || value == f64::NEG_INFINITY || value <= 0.0 {
                0
            } else if value == f64::INFINITY || value >= f64::from(u16::MAX) {
                u128::from(u16::MAX)
            } else {
                value.trunc() as u128
            }
        }
    }
}

fn integer_builtin(
    name: &'static str,
    target: IntegerTarget,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let input = conversion_input(name, &arguments[0], context)?;
    let output = match target {
        IntegerTarget::I8 => build_signed_integer::<i8>(name, input, context)?,
        IntegerTarget::U8 => build_unsigned_integer::<u8>(name, input, context)?,
        IntegerTarget::I16 => build_signed_integer::<i16>(name, input, context)?,
        IntegerTarget::U16 => build_unsigned_integer::<u16>(name, input, context)?,
        IntegerTarget::I32 => build_signed_integer::<i32>(name, input, context)?,
        IntegerTarget::U32 => build_unsigned_integer::<u32>(name, input, context)?,
        IntegerTarget::I64 => build_signed_integer::<i64>(name, input, context)?,
        IntegerTarget::U64 => build_unsigned_integer::<u64>(name, input, context)?,
    };
    context.check_cancelled()?;
    Ok(vec![Value::Array(ArrayData::Integer(output))])
}

fn conversion_input(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<ConversionInput, BuiltinError> {
    let (shape, values, complex) = match value {
        Value::Logical(value) => {
            return scalar_conversion_input(
                SourceComponent::Unsigned(u128::from(*value)),
                SourceComponent::Unsigned(0),
                false,
            );
        }
        Value::Double(value) => {
            return scalar_conversion_input(
                SourceComponent::Float(*value),
                SourceComponent::Float(0.0),
                false,
            );
        }
        Value::Complex(value) => {
            return scalar_conversion_input(
                SourceComponent::Float(value.real),
                SourceComponent::Float(value.imaginary),
                true,
            );
        }
        Value::Array(ArrayData::Logical(array)) => (
            array.shape().clone(),
            checked_map_slice(name, array.as_slice(), context, |value| SourceNumber {
                real: SourceComponent::Unsigned(u128::from(value.get())),
                imaginary: SourceComponent::Unsigned(0),
            })?,
            false,
        ),
        Value::Array(ArrayData::F64(array)) => (
            array.shape().clone(),
            checked_map_slice(name, array.as_slice(), context, |value| SourceNumber {
                real: SourceComponent::Float(*value),
                imaginary: SourceComponent::Float(0.0),
            })?,
            false,
        ),
        Value::Array(ArrayData::ComplexF64(array)) => (
            array.shape().clone(),
            checked_map_slice(name, array.as_slice(), context, |value| SourceNumber {
                real: SourceComponent::Float(value.re),
                imaginary: SourceComponent::Float(value.im),
            })?,
            true,
        ),
        Value::Array(ArrayData::F32(array)) => (
            array.shape().clone(),
            checked_map_slice(name, array.as_slice(), context, |value| SourceNumber {
                real: SourceComponent::Float(f64::from(*value)),
                imaginary: SourceComponent::Float(0.0),
            })?,
            false,
        ),
        Value::Array(ArrayData::ComplexF32(array)) => (
            array.shape().clone(),
            checked_map_slice(name, array.as_slice(), context, |value| SourceNumber {
                real: SourceComponent::Float(f64::from(value.re)),
                imaginary: SourceComponent::Float(f64::from(value.im)),
            })?,
            true,
        ),
        Value::Array(ArrayData::Char(array)) => (
            array.shape().clone(),
            checked_map_slice(name, array.as_slice(), context, |value| SourceNumber {
                real: SourceComponent::Unsigned(u128::from(value.get())),
                imaginary: SourceComponent::Unsigned(0),
            })?,
            false,
        ),
        Value::Array(ArrayData::Integer(integer)) => {
            return integer_conversion_input(name, integer, context);
        }
        Value::Sparse(_)
        | Value::String(_)
        | Value::Cell(_)
        | Value::Struct(_)
        | Value::Table(_)
        | Value::Nothing
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => {
            return Err(type_error(
                name,
                1,
                "numeric, logical, char, or integer array",
                value,
            ));
        }
    };
    Ok(ConversionInput {
        shape,
        values,
        complex,
    })
}

fn scalar_conversion_input(
    real: SourceComponent,
    imaginary: SourceComponent,
    complex: bool,
) -> Result<ConversionInput, BuiltinError> {
    Ok(ConversionInput {
        shape: Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        values: vec![SourceNumber { real, imaginary }],
        complex,
    })
}

fn integer_conversion_input(
    name: &str,
    integer: &IntegerArrayData,
    context: &BuiltinContext<'_>,
) -> Result<ConversionInput, BuiltinError> {
    let mut values = reserved_vec(name, checked_host_length(name, integer.numel())?)?;
    for (index, value) in integer.elements().enumerate() {
        check_cancelled_at(context, index)?;
        values.push(SourceNumber {
            real: source_integer_component(value.real_component()),
            imaginary: value
                .imaginary_component()
                .map_or(SourceComponent::Unsigned(0), source_integer_component),
        });
    }
    Ok(ConversionInput {
        shape: integer.shape().clone(),
        values,
        complex: integer.is_complex(),
    })
}

const fn source_integer_component(value: IntegerComponent) -> SourceComponent {
    match value {
        IntegerComponent::Signed(value) => SourceComponent::Signed(value),
        IntegerComponent::Unsigned(value) => SourceComponent::Unsigned(value),
    }
}

fn build_signed_integer<T>(
    name: &str,
    input: ConversionInput,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError>
where
    T: IntegerElement + Copy + Default + Eq + TryFrom<i128>,
    ComplexInteger<T>: IntegerElement,
{
    let minimum = signed_minimum::<T>();
    let maximum = signed_maximum::<T>();
    let mut converted = reserved_vec(name, input.values.len())?;
    let mut complex = false;
    for (index, value) in input.values.iter().enumerate() {
        check_cancelled_at(context, index)?;
        let real = T::try_from(convert_signed(value.real, minimum, maximum))
            .ok()
            .expect("signed conversion clamps to target range");
        let imaginary = T::try_from(convert_signed(value.imaginary, minimum, maximum))
            .ok()
            .expect("signed conversion clamps to target range");
        complex |= imaginary != T::default();
        converted.push((real, imaginary));
    }
    if complex {
        let values = converted
            .into_iter()
            .map(|(real, imaginary)| ComplexInteger::new(real, imaginary))
            .collect();
        DenseArray::from_vec(input.shape, values)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))
    } else {
        let values = converted.into_iter().map(|(real, _)| real).collect();
        DenseArray::from_vec(input.shape, values)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))
    }
}

fn build_unsigned_integer<T>(
    name: &str,
    input: ConversionInput,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError>
where
    T: IntegerElement + Copy + Default + Eq + TryFrom<u128>,
    ComplexInteger<T>: IntegerElement,
{
    let maximum = unsigned_maximum::<T>();
    let mut converted = reserved_vec(name, input.values.len())?;
    let mut complex = false;
    for (index, value) in input.values.iter().enumerate() {
        check_cancelled_at(context, index)?;
        let real = T::try_from(convert_unsigned(value.real, maximum))
            .ok()
            .expect("unsigned conversion clamps to target range");
        let imaginary = T::try_from(convert_unsigned(value.imaginary, maximum))
            .ok()
            .expect("unsigned conversion clamps to target range");
        complex |= imaginary != T::default();
        converted.push((real, imaginary));
    }
    if complex {
        let values = converted
            .into_iter()
            .map(|(real, imaginary)| ComplexInteger::new(real, imaginary))
            .collect();
        DenseArray::from_vec(input.shape, values)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))
    } else {
        let values = converted.into_iter().map(|(real, _)| real).collect();
        DenseArray::from_vec(input.shape, values)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))
    }
}

fn signed_minimum<T>() -> i128 {
    match std::mem::size_of::<T>() {
        1 => i128::from(i8::MIN),
        2 => i128::from(i16::MIN),
        4 => i128::from(i32::MIN),
        8 => i128::from(i64::MIN),
        _ => unreachable!("integer constructor target width is fixed"),
    }
}

fn signed_maximum<T>() -> i128 {
    match std::mem::size_of::<T>() {
        1 => i128::from(i8::MAX),
        2 => i128::from(i16::MAX),
        4 => i128::from(i32::MAX),
        8 => i128::from(i64::MAX),
        _ => unreachable!("integer constructor target width is fixed"),
    }
}

fn unsigned_maximum<T>() -> u128 {
    match std::mem::size_of::<T>() {
        1 => u128::from(u8::MAX),
        2 => u128::from(u16::MAX),
        4 => u128::from(u32::MAX),
        8 => u128::from(u64::MAX),
        _ => unreachable!("integer constructor target width is fixed"),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn convert_signed(value: SourceComponent, minimum: i128, maximum: i128) -> i128 {
    match value {
        SourceComponent::Signed(value) => value.clamp(minimum, maximum),
        SourceComponent::Unsigned(value) => {
            i128::try_from(value).map_or(maximum, |value| value.min(maximum))
        }
        SourceComponent::Float(value) => {
            if value.is_nan() {
                return 0;
            }
            if value == f64::INFINITY {
                return maximum;
            }
            if value == f64::NEG_INFINITY {
                return minimum;
            }
            let value = value.round();
            #[allow(clippy::cast_precision_loss)]
            if value <= minimum as f64 {
                minimum
            } else {
                #[allow(clippy::cast_precision_loss)]
                if value >= maximum as f64 {
                    maximum
                } else {
                    value as i128
                }
            }
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn convert_unsigned(value: SourceComponent, maximum: u128) -> u128 {
    match value {
        SourceComponent::Signed(value) => {
            u128::try_from(value).map_or(0, |value| value.min(maximum))
        }
        SourceComponent::Unsigned(value) => value.min(maximum),
        SourceComponent::Float(value) => {
            if value.is_nan() || value == f64::NEG_INFINITY || value <= 0.0 {
                return 0;
            }
            if value == f64::INFINITY {
                return maximum;
            }
            let value = value.round();
            #[allow(clippy::cast_precision_loss)]
            if value >= maximum as f64 {
                maximum
            } else {
                value as u128
            }
        }
    }
}

pub(super) fn logical_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("logical", arguments, 1)?;
    expect_max_outputs("logical", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Logical(value) => Value::Logical(*value),
        Value::Double(value) => Value::Logical(*value != 0.0),
        Value::Complex(value) => Value::Logical(!value.is_zero()),
        Value::Array(ArrayData::Logical(array)) => Value::Array(ArrayData::Logical(array.clone())),
        Value::Array(ArrayData::F64(array)) => Value::Array(ArrayData::Logical(map_array(
            "logical",
            array,
            context,
            |value| Logical::from(*value != 0.0),
        )?)),
        Value::Array(ArrayData::ComplexF64(array)) => Value::Array(ArrayData::Logical(map_array(
            "logical",
            array,
            context,
            |value| Logical::from(value.re != 0.0 || value.im != 0.0),
        )?)),
        Value::Array(ArrayData::F32(array)) => Value::Array(ArrayData::Logical(map_array(
            "logical",
            array,
            context,
            |value| Logical::from(*value != 0.0),
        )?)),
        Value::Array(ArrayData::ComplexF32(array)) => Value::Array(ArrayData::Logical(map_array(
            "logical",
            array,
            context,
            |value| Logical::from(value.re != 0.0 || value.im != 0.0),
        )?)),
        Value::Array(ArrayData::Char(array)) => Value::Array(ArrayData::Logical(map_array(
            "logical",
            array,
            context,
            |value| Logical::from(value.get() != 0),
        )?)),
        Value::Array(ArrayData::Integer(integer)) if integer.is_complex() => {
            return Err(type_error(
                "logical",
                1,
                "real numeric, logical, char, or integer array",
                &arguments[0],
            ));
        }
        Value::Array(ArrayData::Integer(integer)) => {
            let mut values =
                reserved_vec("logical", checked_host_length("logical", integer.numel())?)?;
            for (index, value) in integer.elements().enumerate() {
                check_cancelled_at(context, index)?;
                values.push(Logical::from(!value.real_component().is_zero()));
            }
            let array = DenseArray::from_vec(integer.shape().clone(), values)
                .map_err(|error| array_error(&error))?;
            Value::Array(ArrayData::Logical(array))
        }
        value => return Err(type_error("logical", 1, "numeric or logical array", value)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn double_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("double", arguments, 1)?;
    expect_max_outputs("double", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Logical(value) => Value::Double(f64::from(*value)),
        Value::Double(_)
        | Value::Complex(_)
        | Value::Array(ArrayData::F64(_) | ArrayData::ComplexF64(_)) => arguments[0].clone(),
        Value::Array(ArrayData::Logical(array)) => Value::Array(ArrayData::F64(map_array(
            "double",
            array,
            context,
            |value| f64::from(value.get()),
        )?)),
        Value::Array(ArrayData::Char(array)) => Value::Array(ArrayData::F64(map_array(
            "double",
            array,
            context,
            |value| f64::from(value.get()),
        )?)),
        Value::Array(ArrayData::Integer(integer)) => integer_to_double(integer, context)?,
        Value::Array(ArrayData::F32(array)) => {
            map_array("double", array, context, |value| f64::from(*value))
                .map(ArrayData::F64)
                .map(Value::Array)?
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            map_array("double", array, context, |value| {
                ArrayComplex64::new(f64::from(value.re), f64::from(value.im))
            })
            .map(ArrayData::ComplexF64)
            .map(Value::Array)?
        }
        value => return Err(type_error("double", 1, "numeric or logical array", value)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn integer_to_double(
    integer: &IntegerArrayData,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if integer.is_complex() {
        let mut values = reserved_vec("double", checked_host_length("double", integer.numel())?)?;
        for (index, value) in integer.elements().enumerate() {
            check_cancelled_at(context, index)?;
            values.push(ArrayComplex64::new(
                integer_component_to_f64(value.real_component()),
                integer_component_to_f64(
                    value
                        .imaginary_component()
                        .expect("complex integer element has imaginary storage"),
                ),
            ));
        }
        DenseArray::from_vec(integer.shape().clone(), values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        let mut values = reserved_vec("double", checked_host_length("double", integer.numel())?)?;
        for (index, value) in integer.elements().enumerate() {
            check_cancelled_at(context, index)?;
            values.push(integer_component_to_f64(value.real_component()));
        }
        DenseArray::from_vec(integer.shape().clone(), values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

#[allow(clippy::cast_precision_loss)]
fn integer_component_to_f64(value: IntegerComponent) -> f64 {
    match value {
        IntegerComponent::Signed(value) => value as f64,
        IntegerComponent::Unsigned(value) => value as f64,
    }
}

pub(super) fn real_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("real", arguments, 1)?;
    expect_max_outputs("real", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Logical(value) => Value::Double(f64::from(*value)),
        Value::Double(_) | Value::Array(ArrayData::F64(_) | ArrayData::F32(_)) => {
            arguments[0].clone()
        }
        Value::Complex(value) => Value::Double(value.real),
        Value::Array(ArrayData::Logical(array)) => Value::Array(ArrayData::F64(map_array(
            "real",
            array,
            context,
            |value| f64::from(value.get()),
        )?)),
        Value::Array(ArrayData::ComplexF64(array)) => Value::Array(ArrayData::F64(map_array(
            "real",
            array,
            context,
            |value| value.re,
        )?)),
        Value::Array(ArrayData::ComplexF32(array)) => Value::Array(ArrayData::F32(map_array(
            "real",
            array,
            context,
            |value| value.re,
        )?)),
        Value::Array(ArrayData::Char(_) | ArrayData::Integer(_)) => {
            return Err(type_error(
                "real",
                1,
                "double or logical array",
                &arguments[0],
            ));
        }
        value => return Err(type_error("real", 1, "numeric or logical array", value)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn imag_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("imag", arguments, 1)?;
    expect_max_outputs("imag", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Logical(_) | Value::Double(_) => Value::Double(0.0),
        Value::Complex(value) => Value::Double(value.imaginary),
        Value::Array(ArrayData::Logical(array)) => {
            Value::Array(ArrayData::F64(map_array("imag", array, context, |_| 0.0)?))
        }
        Value::Array(ArrayData::F64(array)) => {
            Value::Array(ArrayData::F64(map_array("imag", array, context, |_| 0.0)?))
        }
        Value::Array(ArrayData::ComplexF64(array)) => Value::Array(ArrayData::F64(map_array(
            "imag",
            array,
            context,
            |value| value.im,
        )?)),
        Value::Array(ArrayData::F32(array)) => {
            Value::Array(ArrayData::F32(map_array("imag", array, context, |_| {
                0.0_f32
            })?))
        }
        Value::Array(ArrayData::ComplexF32(array)) => Value::Array(ArrayData::F32(map_array(
            "imag",
            array,
            context,
            |value| value.im,
        )?)),
        Value::Array(ArrayData::Char(_) | ArrayData::Integer(_)) => {
            return Err(type_error(
                "imag",
                1,
                "double or logical array",
                &arguments[0],
            ));
        }
        value => return Err(type_error("imag", 1, "numeric or logical array", value)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn conj_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("conj", arguments, 1)?;
    expect_max_outputs("conj", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Logical(value) => Value::Double(f64::from(*value)),
        Value::Double(_) | Value::Array(ArrayData::F64(_) | ArrayData::F32(_)) => {
            arguments[0].clone()
        }
        Value::Complex(value) => Value::Complex(Complex64::new(value.real, -value.imaginary)),
        Value::Array(ArrayData::Logical(array)) => Value::Array(ArrayData::F64(map_array(
            "conj",
            array,
            context,
            |value| f64::from(value.get()),
        )?)),
        Value::Array(ArrayData::ComplexF64(array)) => Value::Array(ArrayData::ComplexF64(
            map_array("conj", array, context, |value| value.conjugate())?,
        )),
        Value::Array(ArrayData::ComplexF32(array)) => Value::Array(ArrayData::ComplexF32(
            map_array("conj", array, context, |value| value.conjugate())?,
        )),
        Value::Array(ArrayData::Char(_) | ArrayData::Integer(_)) => {
            return Err(type_error(
                "conj",
                1,
                "double or logical array",
                &arguments[0],
            ));
        }
        value => return Err(type_error("conj", 1, "numeric or logical array", value)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn complex_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("complex", arguments, 1, 2)?;
    expect_max_outputs("complex", context, 1)?;
    context.check_cancelled()?;
    if arguments.iter().any(|value| {
        matches!(
            value,
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        )
    }) {
        return complex_single_builtin(arguments, context);
    }
    let real = numeric_data("complex", 1, &arguments[0], context)?;
    let output = if arguments.len() == 1 {
        numeric_output(
            "complex",
            real.dimensions,
            real.values,
            NumericClass::Complex,
            real.scalar_variant,
            context,
        )?
    } else {
        let imaginary = numeric_data("complex", 2, &arguments[1], context)?;
        if real.class == NumericClass::Complex || imaginary.class == NumericClass::Complex {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                "the two-input form of `complex` requires real numeric inputs",
            ));
        }
        let (dimensions, scalar_variant) =
            compatible_binary_shape("complex", &real, &imaginary, context)?;
        let length = checked_binary_length("complex", &real, &imaginary)?;
        let mut values = reserved_vec("complex", length)?;
        for index in 0..length {
            check_cancelled_at(context, index)?;
            let real_value = broadcast_value(&real, index).re;
            let imaginary_value = broadcast_value(&imaginary, index).re;
            values.push(ArrayComplex64::new(real_value, imaginary_value));
        }
        numeric_output(
            "complex",
            dimensions,
            values,
            NumericClass::Complex,
            scalar_variant,
            context,
        )?
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn complex_single_builtin(arguments: &[Value], context: &BuiltinContext<'_>) -> BuiltinResult {
    let Value::Array(real @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) = &arguments[0] else {
        return Err(type_error("complex", 1, "single array", &arguments[0]));
    };
    if arguments.len() == 1 {
        let output = match real {
            ArrayData::F32(array) => Value::Array(ArrayData::ComplexF32(map_array(
                "complex",
                array,
                context,
                |value| ArrayComplex32::new(*value, 0.0),
            )?)),
            ArrayData::ComplexF32(_) => arguments[0].clone(),
            _ => return Err(type_error("complex", 1, "single array", &arguments[0])),
        };
        context.check_cancelled()?;
        return Ok(vec![output]);
    }

    let Value::Array(ArrayData::F32(real)) = &arguments[0] else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "the two-input single form of `complex` requires real single inputs",
        ));
    };
    let Value::Array(ArrayData::F32(imaginary)) = &arguments[1] else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "the two-input single form of `complex` requires real single inputs",
        ));
    };
    let shape = if real.shape().dimensions() == imaginary.shape().dimensions() {
        real.shape().clone()
    } else if real.numel() == 1 {
        imaginary.shape().clone()
    } else if imaginary.numel() == 1 {
        real.shape().clone()
    } else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the two-input form of `complex` requires equal shapes or one scalar input",
        ));
    };
    let length = checked_host_length("complex", shape.numel())?;
    let mut values = reserved_vec("complex", length)?;
    for index in 0..length {
        check_cancelled_at(context, index)?;
        let real = real.as_slice()[if real.numel() == 1 { 0 } else { index }];
        let imaginary = imaginary.as_slice()[if imaginary.numel() == 1 { 0 } else { index }];
        values.push(ArrayComplex32::new(real, imaginary));
    }
    let output = DenseArray::from_vec(shape, values)
        .map(ArrayData::ComplexF32)
        .map(Value::Array)
        .map_err(|error| array_error(&error))?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn dot_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("dot", arguments, 2)?;
    expect_max_outputs("dot", context, 1)?;
    context.check_cancelled()?;
    let left = numeric_data("dot", 1, &arguments[0], context)?;
    let right = numeric_data("dot", 2, &arguments[1], context)?;
    let vector_pair = is_vector(&left.dimensions)
        && is_vector(&right.dimensions)
        && left.values.len() == right.values.len()
        || is_canonical_empty_matrix(&left) && is_canonical_empty_matrix(&right);
    if !vector_pair && left.dimensions != right.dimensions {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "inputs to `dot` must have equal shapes, or both be vectors with equal lengths",
        ));
    }
    let complex = left.class == NumericClass::Complex || right.class == NumericClass::Complex;
    let output = if vector_pair {
        if complex {
            let mut value = ArrayComplex64::ZERO;
            for (index, (left, right)) in left
                .values
                .iter()
                .copied()
                .zip(right.values.iter().copied())
                .enumerate()
            {
                check_cancelled_at(context, index)?;
                value += conjugate_product(left, right);
            }
            Value::Complex(Complex64::from(value))
        } else {
            Value::Double(reference_dot_with_cancellation(&left, &right, context)?)
        }
    } else {
        let dimension = default_dimension(&left.dimensions);
        let (shape, mapping) = reduction_shape("dot", &left.dimensions, dimension, false, context)?;
        let output_length = checked_host_length("dot", shape.numel())?;
        let mut values = filled_reserved_vec("dot", output_length, ArrayComplex64::ZERO, context)?;
        for (index, (left, right)) in left
            .values
            .iter()
            .copied()
            .zip(right.values.iter().copied())
            .enumerate()
        {
            check_cancelled_at(context, index)?;
            let output = mapping.output_offset("dot", index)?;
            let accumulator = values
                .get_mut(output)
                .ok_or_else(|| internal_offset_error("dot"))?;
            *accumulator += conjugate_product(left, right);
        }
        numeric_output(
            "dot",
            checked_clone_slice("dot", shape.dimensions(), context)?,
            values,
            if complex {
                NumericClass::Complex
            } else {
                NumericClass::Real
            },
            left.scalar_variant && right.scalar_variant,
            context,
        )?
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn norm_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("norm", arguments, 1, 2)?;
    expect_max_outputs("norm", context, 1)?;
    context.check_cancelled()?;
    let order = arguments.get(1).map_or(Ok(NormOrder::Two), norm_order)?;
    let output = norm_value(&arguments[0], order, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NormOrder {
    One,
    Two,
    Infinity,
    NegativeInfinity,
    Frobenius,
    Power(f64),
}

#[allow(clippy::float_cmp)]
fn norm_order(value: &Value) -> Result<NormOrder, BuiltinError> {
    if let Some(option) = crate::core_linalg::keyword(value) {
        return if option == "fro" {
            Ok(NormOrder::Frobenius)
        } else {
            Err(norm_order_error())
        };
    }
    let value = real_scalar_value(value).ok_or_else(norm_order_error)?;
    if value == 1.0 {
        Ok(NormOrder::One)
    } else if value == 2.0 {
        Ok(NormOrder::Two)
    } else if value == f64::INFINITY {
        Ok(NormOrder::Infinity)
    } else if value == f64::NEG_INFINITY {
        Ok(NormOrder::NegativeInfinity)
    } else if value.is_finite() && value >= 0.0 {
        Ok(NormOrder::Power(value))
    } else {
        Err(norm_order_error())
    }
}

fn norm_value(
    value: &Value,
    order: NormOrder,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match value {
        Value::Double(value) => {
            norm_real_array(&scalar_dense_array(*value)?, order, context).map(Value::Double)
        }
        Value::Complex(value) => norm_complex_array(
            &scalar_dense_array(ArrayComplex64::new(value.real, value.imaginary))?,
            order,
            context,
        )
        .map(Value::Double),
        Value::Array(ArrayData::F32(array)) => {
            let result = norm_real_single_array(array, order, context)?;
            single_scalar(result)
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            let result = norm_complex_single_array(array, order, context)?;
            single_scalar(result)
        }
        Value::Array(ArrayData::F64(array)) => {
            norm_real_array(array, order, context).map(Value::Double)
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            norm_complex_array(array, order, context).map(Value::Double)
        }
        _ => Err(type_error(
            "norm",
            1,
            "double or single vector or matrix",
            value,
        )),
    }
}

fn norm_real_array(
    array: &DenseArray<f64>,
    order: NormOrder,
    context: &BuiltinContext<'_>,
) -> Result<f64, BuiltinError> {
    norm_array(
        array.shape(),
        array.as_slice(),
        order,
        context,
        f64::abs,
        || {
            context
                .linalg_provider()
                .svd_f64(
                    SvdRequest::new(array, SvdVectors::None)
                        .with_cancellation_flag(context.cancellation_flag()),
                )
                .map_err(|error| linalg_error("norm", &error))
                .map(|result| {
                    result
                        .singular_values
                        .as_slice()
                        .first()
                        .copied()
                        .unwrap_or(0.0)
                })
        },
    )
}

fn norm_complex_array(
    array: &DenseArray<ArrayComplex64>,
    order: NormOrder,
    context: &BuiltinContext<'_>,
) -> Result<f64, BuiltinError> {
    norm_array(
        array.shape(),
        array.as_slice(),
        order,
        context,
        |value| value.re.hypot(value.im),
        || {
            context
                .linalg_provider()
                .svd_complex64(
                    SvdRequest::new(array, SvdVectors::None)
                        .with_cancellation_flag(context.cancellation_flag()),
                )
                .map_err(|error| linalg_error("norm", &error))
                .map(|result| {
                    result
                        .singular_values
                        .as_slice()
                        .first()
                        .copied()
                        .unwrap_or(0.0)
                })
        },
    )
}

fn norm_real_single_array(
    array: &DenseArray<f32>,
    order: NormOrder,
    context: &BuiltinContext<'_>,
) -> Result<f32, BuiltinError> {
    norm_array_single(
        array.shape(),
        array.as_slice(),
        order,
        context,
        f32::abs,
        || {
            context
                .linalg_provider()
                .svd_f32(
                    SvdRequest::new(array, SvdVectors::None)
                        .with_cancellation_flag(context.cancellation_flag()),
                )
                .map_err(|error| linalg_error("norm", &error))
                .map(|result| {
                    result
                        .singular_values
                        .as_slice()
                        .first()
                        .copied()
                        .unwrap_or(0.0)
                })
        },
    )
}

fn norm_complex_single_array(
    array: &DenseArray<ArrayComplex32>,
    order: NormOrder,
    context: &BuiltinContext<'_>,
) -> Result<f32, BuiltinError> {
    norm_array_single(
        array.shape(),
        array.as_slice(),
        order,
        context,
        |value| value.re.hypot(value.im),
        || {
            context
                .linalg_provider()
                .svd_complex32(
                    SvdRequest::new(array, SvdVectors::None)
                        .with_cancellation_flag(context.cancellation_flag()),
                )
                .map_err(|error| linalg_error("norm", &error))
                .map(|result| {
                    result
                        .singular_values
                        .as_slice()
                        .first()
                        .copied()
                        .unwrap_or(0.0)
                })
        },
    )
}

fn norm_array<T: Copy>(
    shape: &Shape,
    values: &[T],
    order: NormOrder,
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f64,
    spectral: impl FnOnce() -> Result<f64, BuiltinError>,
) -> Result<f64, BuiltinError> {
    if shape.ndims() > 2 {
        return Err(norm_dimension_error());
    }
    if values.is_empty() {
        return Ok(0.0);
    }
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        if magnitude(value).is_nan() {
            return Ok(f64::NAN);
        }
    }
    let matrix = shape.extent(0) > 1 && shape.extent(1) > 1;
    if matrix {
        return match order {
            NormOrder::One => matrix_one_norm(shape, values, context, magnitude),
            NormOrder::Two => spectral(),
            NormOrder::Infinity => matrix_infinity_norm(shape, values, context, magnitude),
            NormOrder::Frobenius => stable_two_norm(values, context, magnitude),
            NormOrder::NegativeInfinity | NormOrder::Power(_) => Err(norm_order_error()),
        };
    }
    vector_norm(values, order, context, magnitude)
}

fn norm_array_single<T: Copy>(
    shape: &Shape,
    values: &[T],
    order: NormOrder,
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f32,
    spectral: impl FnOnce() -> Result<f32, BuiltinError>,
) -> Result<f32, BuiltinError> {
    if shape.ndims() > 2 {
        return Err(norm_dimension_error());
    }
    if values.is_empty() {
        return Ok(0.0);
    }
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        if magnitude(value).is_nan() {
            return Ok(f32::NAN);
        }
    }
    let matrix = shape.extent(0) > 1 && shape.extent(1) > 1;
    if matrix {
        return match order {
            NormOrder::One => matrix_one_norm_single(shape, values, context, magnitude),
            NormOrder::Two => spectral(),
            NormOrder::Infinity => matrix_infinity_norm_single(shape, values, context, magnitude),
            NormOrder::Frobenius => stable_two_norm_single(values, context, magnitude),
            NormOrder::NegativeInfinity | NormOrder::Power(_) => Err(norm_order_error()),
        };
    }
    vector_norm_single(values, order, context, magnitude)
}

fn matrix_one_norm<T: Copy>(
    shape: &Shape,
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f64,
) -> Result<f64, BuiltinError> {
    let rows = checked_host_length("norm", shape.extent(0))?;
    let mut maximum = 0.0_f64;
    for column in values.chunks(rows) {
        context.check_cancelled()?;
        let sum = column.iter().copied().map(&magnitude).sum::<f64>();
        if sum.is_nan() {
            return Ok(f64::NAN);
        }
        maximum = maximum.max(sum);
    }
    Ok(maximum)
}

fn matrix_infinity_norm<T: Copy>(
    shape: &Shape,
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f64,
) -> Result<f64, BuiltinError> {
    let rows = checked_host_length("norm", shape.extent(0))?;
    let mut sums = filled_reserved_vec("norm", rows, 0.0_f64, context)?;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        sums[index % rows] += magnitude(value);
    }
    if sums.iter().any(|value| value.is_nan()) {
        return Ok(f64::NAN);
    }
    Ok(sums.into_iter().fold(0.0_f64, f64::max))
}

fn matrix_one_norm_single<T: Copy>(
    shape: &Shape,
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f32,
) -> Result<f32, BuiltinError> {
    let rows = checked_host_length("norm", shape.extent(0))?;
    let mut maximum = 0.0_f32;
    for column in values.chunks(rows) {
        context.check_cancelled()?;
        let sum = column.iter().copied().map(&magnitude).sum::<f32>();
        if sum.is_nan() {
            return Ok(f32::NAN);
        }
        maximum = maximum.max(sum);
    }
    Ok(maximum)
}

fn matrix_infinity_norm_single<T: Copy>(
    shape: &Shape,
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f32,
) -> Result<f32, BuiltinError> {
    let rows = checked_host_length("norm", shape.extent(0))?;
    let mut sums = filled_reserved_vec("norm", rows, 0.0_f32, context)?;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        sums[index % rows] += magnitude(value);
    }
    if sums.iter().any(|value| value.is_nan()) {
        return Ok(f32::NAN);
    }
    Ok(sums.into_iter().fold(0.0_f32, f32::max))
}

fn vector_norm<T: Copy>(
    values: &[T],
    order: NormOrder,
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f64,
) -> Result<f64, BuiltinError> {
    match order {
        NormOrder::One => sum_magnitudes(values, context, magnitude),
        NormOrder::Two | NormOrder::Frobenius => stable_two_norm(values, context, magnitude),
        NormOrder::Infinity => extremum_magnitude(values, context, magnitude, false),
        NormOrder::NegativeInfinity => extremum_magnitude(values, context, magnitude, true),
        NormOrder::Power(power) => power_norm(values, power, context, magnitude),
    }
}

fn vector_norm_single<T: Copy>(
    values: &[T],
    order: NormOrder,
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f32,
) -> Result<f32, BuiltinError> {
    match order {
        NormOrder::One => sum_magnitudes_single(values, context, magnitude),
        NormOrder::Two | NormOrder::Frobenius => stable_two_norm_single(values, context, magnitude),
        NormOrder::Infinity => extremum_magnitude_single(values, context, magnitude, false),
        NormOrder::NegativeInfinity => extremum_magnitude_single(values, context, magnitude, true),
        NormOrder::Power(power) => power_norm_single(values, power, context, magnitude),
    }
}

fn sum_magnitudes<T: Copy>(
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f64,
) -> Result<f64, BuiltinError> {
    let mut sum = 0.0;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        sum += magnitude(value);
    }
    Ok(sum)
}

fn sum_magnitudes_single<T: Copy>(
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f32,
) -> Result<f32, BuiltinError> {
    let mut sum = 0.0_f32;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        sum += magnitude(value);
    }
    Ok(sum)
}

fn stable_two_norm<T: Copy>(
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f64,
) -> Result<f64, BuiltinError> {
    let mut norm = 0.0_f64;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        norm = norm.hypot(magnitude(value));
    }
    Ok(norm)
}

fn stable_two_norm_single<T: Copy>(
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f32,
) -> Result<f32, BuiltinError> {
    let mut norm = 0.0_f32;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        norm = norm.hypot(magnitude(value));
    }
    Ok(norm)
}

fn extremum_magnitude<T: Copy>(
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f64,
    minimum: bool,
) -> Result<f64, BuiltinError> {
    let mut selected = if minimum { f64::INFINITY } else { 0.0 };
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let value = magnitude(value);
        if value.is_nan() {
            return Ok(f64::NAN);
        }
        selected = if minimum {
            selected.min(value)
        } else {
            selected.max(value)
        };
    }
    Ok(selected)
}

fn extremum_magnitude_single<T: Copy>(
    values: &[T],
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f32,
    minimum: bool,
) -> Result<f32, BuiltinError> {
    let mut selected = if minimum { f32::INFINITY } else { 0.0 };
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let value = magnitude(value);
        if value.is_nan() {
            return Ok(f32::NAN);
        }
        selected = if minimum {
            selected.min(value)
        } else {
            selected.max(value)
        };
    }
    Ok(selected)
}

#[allow(clippy::float_cmp)]
fn power_norm<T: Copy>(
    values: &[T],
    power: f64,
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f64,
) -> Result<f64, BuiltinError> {
    if power == 0.0 {
        return Ok(match values.len() {
            0 => 0.0,
            1 => 1.0,
            _ => f64::INFINITY,
        });
    }
    let maximum = extremum_magnitude(values, context, &magnitude, false)?;
    if maximum == 0.0 || !maximum.is_finite() {
        return Ok(maximum);
    }
    let mut scaled = 0.0;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        scaled += (magnitude(value) / maximum).powf(power);
    }
    Ok(maximum * scaled.powf(power.recip()))
}

#[allow(clippy::float_cmp)]
fn power_norm_single<T: Copy>(
    values: &[T],
    power: f64,
    context: &BuiltinContext<'_>,
    magnitude: impl Fn(T) -> f32,
) -> Result<f32, BuiltinError> {
    if power == 0.0 {
        return Ok(match values.len() {
            0 => 0.0,
            1 => 1.0,
            _ => f32::INFINITY,
        });
    }
    let maximum = extremum_magnitude_single(values, context, &magnitude, false)?;
    if maximum == 0.0 || !maximum.is_finite() {
        return Ok(maximum);
    }
    #[allow(clippy::cast_possible_truncation)]
    let power = power as f32;
    let mut scaled = 0.0_f32;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        scaled += (magnitude(value) / maximum).powf(power);
    }
    Ok(maximum * scaled.powf(power.recip()))
}

fn single_scalar(value: f32) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map(ArrayData::F32)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn scalar_dense_array<T>(value: T) -> Result<DenseArray<T>, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map_err(|error| array_error(&error))
}

fn real_scalar_value(value: &Value) -> Option<f64> {
    match value {
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            array.as_slice().first().copied()
        }
        Value::Array(ArrayData::F32(array)) if array.numel() == 1 => {
            array.as_slice().first().copied().map(f64::from)
        }
        _ => None,
    }
}

fn norm_order_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`norm` received an unsupported vector or matrix norm order",
    )
}

fn norm_dimension_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`norm` accepts only vectors and 2-D matrices",
    )
}

fn numeric_data(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<NumericData, BuiltinError> {
    let source_dimensions = value
        .dimensions()
        .ok_or_else(|| type_error(name, position, "numeric or logical array", value))?;
    let dimensions = checked_clone_slice(name, source_dimensions, context)?;
    let (values, class, scalar_variant) = match value {
        Value::Logical(value) => (
            vec![ArrayComplex64::new(f64::from(*value), 0.0)],
            NumericClass::Logical,
            true,
        ),
        Value::Double(value) => (
            vec![ArrayComplex64::new(*value, 0.0)],
            NumericClass::Real,
            true,
        ),
        Value::Complex(value) => (
            vec![ArrayComplex64::from(*value)],
            NumericClass::Complex,
            true,
        ),
        Value::Array(ArrayData::F64(array)) => (
            checked_map_slice(name, array.as_slice(), context, |value| {
                ArrayComplex64::new(*value, 0.0)
            })?,
            NumericClass::Real,
            false,
        ),
        Value::Array(ArrayData::ComplexF64(array)) => (
            checked_clone_slice(name, array.as_slice(), context)?,
            NumericClass::Complex,
            false,
        ),
        Value::Array(ArrayData::Logical(array)) => (
            checked_map_slice(name, array.as_slice(), context, |value| {
                ArrayComplex64::new(f64::from(value.get()), 0.0)
            })?,
            NumericClass::Logical,
            false,
        ),
        Value::Array(ArrayData::Char(_) | ArrayData::Integer(_)) => {
            return Err(type_error(name, position, "double or logical array", value));
        }
        _ => {
            return Err(type_error(
                name,
                position,
                "numeric or logical array",
                value,
            ));
        }
    };
    Ok(NumericData {
        dimensions,
        values,
        class,
        scalar_variant,
    })
}

fn numeric_output(
    name: &str,
    dimensions: Vec<u64>,
    values: Vec<ArrayComplex64>,
    class: NumericClass,
    scalar_variant: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    if scalar_variant && shape.numel() == 1 {
        let value = values
            .first()
            .copied()
            .ok_or_else(|| internal_offset_error(name))?;
        return Ok(match class {
            NumericClass::Logical => Value::Logical(value.re != 0.0),
            NumericClass::Real => Value::Double(value.re),
            NumericClass::Complex => Value::Complex(Complex64::from(value)),
        });
    }
    match class {
        NumericClass::Complex => DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error)),
        NumericClass::Real => {
            let values = checked_map_slice(name, &values, context, |value| value.re)?;
            DenseArray::from_vec(shape, values)
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }
        NumericClass::Logical => {
            let values = checked_map_slice(name, &values, context, |value| {
                Logical::from(value.re != 0.0)
            })?;
            DenseArray::from_vec(shape, values)
                .map(ArrayData::Logical)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }
    }
}

fn compatible_binary_shape(
    name: &str,
    left: &NumericData,
    right: &NumericData,
    context: &BuiltinContext<'_>,
) -> Result<(Vec<u64>, bool), BuiltinError> {
    if left.dimensions == right.dimensions {
        Ok((
            checked_clone_slice(name, &left.dimensions, context)?,
            left.scalar_variant && right.scalar_variant,
        ))
    } else if left.values.len() == 1 {
        Ok((
            checked_clone_slice(name, &right.dimensions, context)?,
            right.scalar_variant,
        ))
    } else if right.values.len() == 1 {
        Ok((
            checked_clone_slice(name, &left.dimensions, context)?,
            left.scalar_variant,
        ))
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("inputs to `{name}` must have equal shapes or one input must be scalar"),
        ))
    }
}

fn checked_binary_length(
    name: &str,
    left: &NumericData,
    right: &NumericData,
) -> Result<usize, BuiltinError> {
    if left.values.len() == right.values.len() {
        Ok(left.values.len())
    } else if left.values.len() == 1 {
        Ok(right.values.len())
    } else if right.values.len() == 1 {
        Ok(left.values.len())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("inputs to `{name}` have incompatible element counts"),
        ))
    }
}

fn broadcast_value(input: &NumericData, index: usize) -> ArrayComplex64 {
    if input.values.len() == 1 {
        input.values[0]
    } else {
        input.values[index]
    }
}

fn default_dimension(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|index| u64::try_from(index).ok())
        .and_then(|index| index.checked_add(1))
        .unwrap_or(1)
}

struct ReductionMapping {
    stride: u64,
    block: u64,
}

impl ReductionMapping {
    fn output_offset(&self, name: &str, input_offset: usize) -> Result<usize, BuiltinError> {
        let input_offset = u64::try_from(input_offset).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` input offset does not fit the runtime shape model"),
            )
        })?;
        if self.stride == 0 || self.block == 0 {
            return Err(internal_offset_error(name));
        }
        let lower = input_offset % self.stride;
        let upper = input_offset / self.block;
        let output = upper
            .checked_mul(self.stride)
            .and_then(|offset| offset.checked_add(lower))
            .ok_or_else(|| internal_offset_error(name))?;
        usize::try_from(output).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` output offset does not fit this host"),
            )
        })
    }
}

fn reduction_shape(
    name: &str,
    dimensions: &[u64],
    dimension: u64,
    preserve_empty_reduction_extent: bool,
    context: &BuiltinContext<'_>,
) -> Result<(Shape, ReductionMapping), BuiltinError> {
    let index = usize::try_from(dimension - 1).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("dimension input to `{name}` does not fit this host"),
        )
    })?;
    let extent = dimensions.get(index).copied().unwrap_or(1);
    let mut output_dimensions = checked_clone_slice(name, dimensions, context)?;
    if let Some(output_extent) = output_dimensions.get_mut(index)
        && !(preserve_empty_reduction_extent && extent == 0)
    {
        *output_extent = 1;
    }
    let shape = Shape::new(output_dimensions).map_err(|error| array_error(&error))?;
    let mut stride = 1_u64;
    for (stride_index, extent) in dimensions.iter().copied().take(index).enumerate() {
        check_cancelled_at(context, stride_index)?;
        stride = stride.checked_mul(extent).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` reduction stride overflowed"),
            )
        })?;
    }
    let block = stride.checked_mul(extent).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` reduction block size overflowed"),
        )
    })?;
    Ok((shape, ReductionMapping { stride, block }))
}

fn conjugate_product(left: ArrayComplex64, right: ArrayComplex64) -> ArrayComplex64 {
    ArrayComplex64::new(
        left.re.mul_add(right.re, left.im * right.im),
        left.re.mul_add(right.im, -(left.im * right.re)),
    )
}

fn is_vector(dimensions: &[u64]) -> bool {
    dimensions.len() == 2 && (dimensions[0] == 1 || dimensions[1] == 1)
}

fn is_canonical_empty_matrix(input: &NumericData) -> bool {
    input.values.is_empty() && input.dimensions == [0, 0]
}

fn reference_dot_with_cancellation(
    left: &NumericData,
    right: &NumericData,
    context: &BuiltinContext<'_>,
) -> Result<f64, BuiltinError> {
    let mut accumulated = None;
    for (left_chunk, right_chunk) in left
        .values
        .chunks(CANCELLATION_CHECK_INTERVAL)
        .zip(right.values.chunks(CANCELLATION_CHECK_INTERVAL))
    {
        context.check_cancelled()?;
        let prefix_length = usize::from(accumulated.is_some());
        let length = left_chunk.len().checked_add(prefix_length).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`dot` provider chunk length overflowed",
            )
        })?;
        let mut left_values = reserved_vec("dot", length)?;
        let mut right_values = reserved_vec("dot", length)?;

        // Feeding the previous accumulator back as `accumulator * 1` lets the
        // provider continue its `left.mul_add(right, sum)` fold. This preserves
        // the element-wise FMA order across cancellation boundaries.
        if let Some(value) = accumulated {
            left_values.push(value);
            right_values.push(1.0);
        }
        for (left, right) in left_chunk.iter().zip(right_chunk) {
            left_values.push(left.re);
            right_values.push(right.re);
        }

        let length = u64::try_from(length).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`dot` provider chunk length does not fit the runtime shape model",
            )
        })?;
        let shape = Shape::new([length, 1]).map_err(|error| array_error(&error))?;
        let left = DenseArray::from_vec(shape.clone(), left_values)
            .map_err(|error| array_error(&error))?;
        let right =
            DenseArray::from_vec(shape, right_values).map_err(|error| array_error(&error))?;
        accumulated = Some(
            ReferenceProvider
                .dot_f64(&left, &right)
                .map_err(|error| linalg_error("dot", &error))?,
        );
    }
    context.check_cancelled()?;
    Ok(accumulated.unwrap_or(0.0))
}

fn map_array<T, U, F>(
    name: &str,
    input: &DenseArray<T>,
    context: &BuiltinContext<'_>,
    transform: F,
) -> Result<DenseArray<U>, BuiltinError>
where
    F: FnMut(&T) -> U,
{
    let values = checked_map_slice(name, input.as_slice(), context, transform)?;
    DenseArray::from_vec(input.shape().clone(), values).map_err(|error| array_error(&error))
}

fn checked_map_slice<T, U, F>(
    name: &str,
    input: &[T],
    context: &BuiltinContext<'_>,
    mut transform: F,
) -> Result<Vec<U>, BuiltinError>
where
    F: FnMut(&T) -> U,
{
    let mut output = reserved_vec(name, input.len())?;
    for chunk in input.chunks(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
        output.extend(chunk.iter().map(&mut transform));
    }
    Ok(output)
}

fn checked_clone_slice<T: Clone>(
    name: &str,
    input: &[T],
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_vec(name, input.len())?;
    for chunk in input.chunks(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
        output.extend_from_slice(chunk);
    }
    Ok(output)
}

fn reserved_vec<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {length} elements"),
        )
    })?;
    Ok(output)
}

fn filled_reserved_vec<T: Clone>(
    name: &str,
    length: usize,
    value: T,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_vec(name, length)?;
    while output.len() < length {
        context.check_cancelled()?;
        let target = output
            .len()
            .saturating_add(CANCELLATION_CHECK_INTERVAL)
            .min(length);
        output.resize(target, value.clone());
    }
    Ok(output)
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn checked_host_length(name: &str, numel: u64) -> Result<usize, BuiltinError> {
    usize::try_from(numel).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` array length {numel} does not fit this host"),
        )
    })
}

fn internal_offset_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` computed an invalid checked array offset"),
    )
}

fn linalg_error(name: &str, error: &openmat_linalg::LinalgError) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Other,
        format!("`{name}` reference provider failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use openmat_runtime::{CancellationToken, VecOutput};

    use super::*;

    #[test]
    fn checked_map_observes_cancellation_between_fixed_size_chunks() {
        let cancellation = CancellationToken::new();
        let cancellation_request = cancellation.clone();
        let mut output = VecOutput::new();
        let context = BuiltinContext::new(1, &cancellation, &mut output);
        let input = vec![0_u8; CANCELLATION_CHECK_INTERVAL * 2];
        let mut visited = 0_usize;

        let error = checked_map_slice("test-map", &input, &context, |value| {
            visited += 1;
            if visited == CANCELLATION_CHECK_INTERVAL {
                cancellation_request.cancel();
            }
            *value
        })
        .expect_err("the second chunk must observe cancellation");

        assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
        assert_eq!(visited, CANCELLATION_CHECK_INTERVAL);
    }
}

//! MATLAB R2022b-compatible fixed-width bitwise built-ins.
//!
//! The casts in this module are deliberate bit-pattern reinterpretations or
//! follow explicit range checks at the MATLAB numeric boundary.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use openmat_array::{
    ArrayData, DenseArray, IntegerArrayData, IntegerComponent, IntegerElement, Logical, Shape,
};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistrationError, BuiltinRegistry,
    BuiltinResult,
};
use openmat_value::{StringValue, Value};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;
const SCALAR_DIMENSIONS: [u64; 2] = [1, 1];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BinaryOperation {
    And,
    Or,
    Xor,
}

impl BinaryOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::And => "bitand",
            Self::Or => "bitor",
            Self::Xor => "bitxor",
        }
    }

    const fn apply(self, left: u64, right: u64) -> u64 {
        match self {
            Self::And => left & right,
            Self::Or => left | right,
            Self::Xor => left ^ right,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WordSpec {
    bits: u32,
    signed: bool,
}

impl WordSpec {
    const I8: Self = Self::signed(8);
    const U8: Self = Self::unsigned(8);
    const I16: Self = Self::signed(16);
    const U16: Self = Self::unsigned(16);
    const I32: Self = Self::signed(32);
    const U32: Self = Self::unsigned(32);
    const I64: Self = Self::signed(64);
    const U64: Self = Self::unsigned(64);

    const fn signed(bits: u32) -> Self {
        Self { bits, signed: true }
    }

    const fn unsigned(bits: u32) -> Self {
        Self {
            bits,
            signed: false,
        }
    }

    const fn mask(self) -> u64 {
        if self.bits == 64 {
            u64::MAX
        } else {
            (1_u64 << self.bits) - 1
        }
    }

    const fn sign_bit(self) -> u64 {
        1_u64 << (self.bits - 1)
    }

    const fn class_name(self) -> &'static str {
        match (self.signed, self.bits) {
            (true, 8) => "int8",
            (false, 8) => "uint8",
            (true, 16) => "int16",
            (false, 16) => "uint16",
            (true, 32) => "int32",
            (false, 32) => "uint32",
            (true, 64) => "int64",
            (false, 64) => "uint64",
            _ => "integer",
        }
    }

    fn decode_f64(self, raw: u64) -> f64 {
        if self.signed {
            sign_extend(raw & self.mask(), self) as f64
        } else {
            (raw & self.mask()) as f64
        }
    }
}

trait BitWord: IntegerElement + Copy {
    const SPEC: WordSpec;

    fn to_raw(self) -> u64;
    fn from_raw(raw: u64) -> Self;
}

macro_rules! impl_unsigned_word {
    ($type:ty, $bits:literal) => {
        impl BitWord for $type {
            const SPEC: WordSpec = WordSpec::unsigned($bits);

            fn to_raw(self) -> u64 {
                self as u64
            }

            fn from_raw(raw: u64) -> Self {
                raw as Self
            }
        }
    };
}

macro_rules! impl_signed_word {
    ($type:ty, $unsigned:ty, $bits:literal) => {
        impl BitWord for $type {
            const SPEC: WordSpec = WordSpec::signed($bits);

            fn to_raw(self) -> u64 {
                (self as $unsigned) as u64
            }

            fn from_raw(raw: u64) -> Self {
                (raw as $unsigned) as Self
            }
        }
    };
}

impl_signed_word!(i8, u8, 8);
impl_unsigned_word!(u8, 8);
impl_signed_word!(i16, u16, 16);
impl_unsigned_word!(u16, 16);
impl_signed_word!(i32, u32, 32);
impl_unsigned_word!(u32, 32);
impl_signed_word!(i64, u64, 64);
impl_unsigned_word!(u64, 64);

pub(super) fn register_bitwise(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("bitand", bitand_builtin)?;
    registry.register("bitor", bitor_builtin)?;
    registry.register("bitxor", bitxor_builtin)?;
    registry.register("bitshift", bitshift_builtin)?;
    registry.register("bitget", bitget_builtin)?;
    registry.register("bitset", bitset_builtin)?;
    Ok(())
}

pub(super) fn bitand_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    binary_builtin(BinaryOperation::And, arguments, context)
}

pub(super) fn bitor_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    binary_builtin(BinaryOperation::Or, arguments, context)
}

pub(super) fn bitxor_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    binary_builtin(BinaryOperation::Xor, arguments, context)
}

pub(super) fn bitshift_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    validate_call("bitshift", arguments, 2, 3, context)?;
    context.check_cancelled()?;
    let assumed = arguments
        .get(2)
        .map(|value| assumed_type("bitshift", 3, value))
        .transpose()?;
    primary_word_spec("bitshift", &arguments[0], assumed)?;
    let control = RealControlView::new("bitshift", 2, &arguments[1])?;
    validate_control_values("bitshift", control, context, |value| {
        value.shift_amount("bitshift", 2, 64).map(|_| ())
    })?;
    let output = map_primary(
        "bitshift",
        &arguments[0],
        assumed,
        &[control],
        context,
        |raw, spec, values| {
            let amount = values[0].shift_amount("bitshift", 2, spec.bits)?;
            Ok(shift_word(raw, amount, spec))
        },
    )?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn bitget_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    validate_call("bitget", arguments, 2, 3, context)?;
    context.check_cancelled()?;
    let assumed = arguments
        .get(2)
        .map(|value| assumed_type("bitget", 3, value))
        .transpose()?;
    let validation_width = primary_word_spec("bitget", &arguments[0], assumed)?.bits;
    let control = RealControlView::new("bitget", 2, &arguments[1])?;
    validate_control_values("bitget", control, context, |value| {
        value
            .bit_position("bitget", 2, validation_width)
            .map(|_| ())
    })?;
    let output = map_primary(
        "bitget",
        &arguments[0],
        assumed,
        &[control],
        context,
        |raw, spec, values| {
            let position = values[0].bit_position("bitget", 2, spec.bits)?;
            Ok((raw >> (position - 1)) & 1)
        },
    )?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn bitset_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    validate_call("bitset", arguments, 2, 4, context)?;
    context.check_cancelled()?;

    let third_is_type = arguments.len() == 3 && arguments.get(2).is_some_and(is_text_scalar);
    let assumed_position = if arguments.len() == 4 {
        Some(4)
    } else if third_is_type {
        Some(3)
    } else {
        None
    };
    let assumed = assumed_position
        .map(|position| assumed_type("bitset", position, &arguments[position - 1]))
        .transpose()?;
    let validation_width = primary_word_spec("bitset", &arguments[0], assumed)?.bits;
    let position = RealControlView::new("bitset", 2, &arguments[1])?;
    validate_control_values("bitset", position, context, |value| {
        value
            .bit_position("bitset", 2, validation_width)
            .map(|_| ())
    })?;
    let value = if arguments.len() >= 3 && !third_is_type {
        Some(RealControlView::new("bitset", 3, &arguments[2])?)
    } else {
        None
    };
    let output = if let Some(value) = value {
        map_primary(
            "bitset",
            &arguments[0],
            assumed,
            &[position, value],
            context,
            |raw, spec, values| {
                let position = values[0].bit_position("bitset", 2, spec.bits)?;
                let mask = 1_u64 << (position - 1);
                Ok(if values[1].is_nonzero() {
                    raw | mask
                } else {
                    raw & !mask
                })
            },
        )?
    } else {
        map_primary(
            "bitset",
            &arguments[0],
            assumed,
            &[position],
            context,
            |raw, spec, values| {
                let position = values[0].bit_position("bitset", 2, spec.bits)?;
                Ok(raw | (1_u64 << (position - 1)))
            },
        )?
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn binary_builtin(
    operation: BinaryOperation,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = operation.name();
    validate_call(name, arguments, 2, 3, context)?;
    context.check_cancelled()?;
    let assumed = arguments
        .get(2)
        .map(|value| assumed_type(name, 3, value))
        .transpose()?;

    let output = match &arguments[0] {
        Value::Array(ArrayData::Integer(integer)) if !integer.is_complex() => {
            integer_binary(operation, integer, &arguments[1], assumed, context)?
        }
        Value::Array(ArrayData::Integer(_)) => return Err(complex_error(name)),
        left if is_double_or_logical(left) => {
            double_or_logical_binary(operation, left, &arguments[1], assumed, context)?
        }
        left => return Err(primary_type_error(name, left, true)),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn integer_binary(
    operation: BinaryOperation,
    left: &IntegerArrayData,
    right: &Value,
    assumed: Option<WordSpec>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! dispatch {
        ($array:expr, $type:ty) => {
            integer_binary_typed::<$type>(operation, $array, right, assumed, context)
        };
    }
    match left {
        IntegerArrayData::I8(array) => dispatch!(array, i8),
        IntegerArrayData::U8(array) => dispatch!(array, u8),
        IntegerArrayData::I16(array) => dispatch!(array, i16),
        IntegerArrayData::U16(array) => dispatch!(array, u16),
        IntegerArrayData::I32(array) => dispatch!(array, i32),
        IntegerArrayData::U32(array) => dispatch!(array, u32),
        IntegerArrayData::I64(array) => dispatch!(array, i64),
        IntegerArrayData::U64(array) => dispatch!(array, u64),
        IntegerArrayData::ComplexI8(_)
        | IntegerArrayData::ComplexU8(_)
        | IntegerArrayData::ComplexI16(_)
        | IntegerArrayData::ComplexU16(_)
        | IntegerArrayData::ComplexI32(_)
        | IntegerArrayData::ComplexU32(_)
        | IntegerArrayData::ComplexI64(_)
        | IntegerArrayData::ComplexU64(_) => Err(complex_error(operation.name())),
    }
}

enum IntegerSecond<'a, T> {
    Typed(&'a DenseArray<T>),
    DoubleScalar(u64),
}

impl<T: BitWord> IntegerSecond<'_, T> {
    fn dimensions(&self) -> &[u64] {
        match self {
            Self::Typed(array) => array.shape().dimensions(),
            Self::DoubleScalar(_) => &SCALAR_DIMENSIONS,
        }
    }

    fn raw_at(&self, offset: usize, name: &str) -> Result<u64, BuiltinError> {
        match self {
            Self::Typed(array) => array
                .as_slice()
                .get(offset)
                .copied()
                .map(BitWord::to_raw)
                .ok_or_else(|| mapping_error(name)),
            Self::DoubleScalar(value) => Ok(*value),
        }
    }
}

fn integer_binary_typed<T: BitWord>(
    operation: BinaryOperation,
    left: &DenseArray<T>,
    right: &Value,
    assumed: Option<WordSpec>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    if assumed.is_some_and(|spec| spec != T::SPEC) {
        return Err(invalid_assumed_type(name, T::SPEC.class_name()));
    }
    let right = match right {
        Value::Array(ArrayData::Integer(integer)) if !integer.is_complex() => integer
            .as_typed::<T>()
            .map(IntegerSecond::Typed)
            .ok_or_else(|| mixed_class_error(name))?,
        value if double_scalar(value).is_some() => IntegerSecond::DoubleScalar(double_to_raw(
            name,
            2,
            double_scalar(value).expect("guarded scalar"),
            T::SPEC,
            true,
        )?),
        _ => return Err(mixed_class_error(name)),
    };
    let shape = implicit_shape(name, left.shape().dimensions(), right.dimensions())?;
    let length = host_length(name, &shape)?;
    let mut values = reserved_vec(name, length)?;
    for offset in 0..length {
        check_cancelled_at(context, offset)?;
        let left_offset = broadcast_offset(name, offset, &shape, left.shape().dimensions())?;
        let right_offset = broadcast_offset(name, offset, &shape, right.dimensions())?;
        let left = left
            .as_slice()
            .get(left_offset)
            .copied()
            .ok_or_else(|| mapping_error(name))?
            .to_raw();
        let raw = operation.apply(left, right.raw_at(right_offset, name)?) & T::SPEC.mask();
        values.push(T::from_raw(raw));
    }
    dense_integer(name, shape, values)
}

#[derive(Clone, Copy)]
enum DoubleLogicalView<'a> {
    DoubleScalar(f64),
    Double(&'a DenseArray<f64>),
    LogicalScalar(bool),
    Logical(&'a DenseArray<Logical>),
}

impl DoubleLogicalView<'_> {
    fn new(value: &Value) -> Option<DoubleLogicalView<'_>> {
        match value {
            Value::Double(value) => Some(DoubleLogicalView::DoubleScalar(*value)),
            Value::Array(ArrayData::F64(array)) => Some(DoubleLogicalView::Double(array)),
            Value::Logical(value) => Some(DoubleLogicalView::LogicalScalar(*value)),
            Value::Array(ArrayData::Logical(array)) => Some(DoubleLogicalView::Logical(array)),
            _ => None,
        }
    }

    fn dimensions_borrowed(&self) -> &[u64] {
        match self {
            Self::DoubleScalar(_) | Self::LogicalScalar(_) => &SCALAR_DIMENSIONS,
            Self::Double(array) => array.shape().dimensions(),
            Self::Logical(array) => array.shape().dimensions(),
        }
    }

    const fn is_logical(self) -> bool {
        matches!(self, Self::LogicalScalar(_) | Self::Logical(_))
    }

    const fn scalar_variant(self) -> bool {
        matches!(self, Self::DoubleScalar(_) | Self::LogicalScalar(_))
    }

    fn validate(
        self,
        name: &str,
        position: usize,
        spec: WordSpec,
        context: &BuiltinContext<'_>,
    ) -> Result<(), BuiltinError> {
        let length = match self {
            Self::DoubleScalar(_) | Self::LogicalScalar(_) => 1,
            Self::Double(array) => array.as_slice().len(),
            Self::Logical(array) => array.as_slice().len(),
        };
        for offset in 0..length {
            check_cancelled_at(context, offset)?;
            self.raw_at(offset, name, position, spec)?;
        }
        Ok(())
    }

    fn raw_at(
        self,
        offset: usize,
        name: &str,
        position: usize,
        spec: WordSpec,
    ) -> Result<u64, BuiltinError> {
        match self {
            Self::DoubleScalar(value) => double_to_raw(name, position, value, spec, false),
            Self::Double(array) => array
                .as_slice()
                .get(offset)
                .copied()
                .ok_or_else(|| mapping_error(name))
                .and_then(|value| double_to_raw(name, position, value, spec, false)),
            Self::LogicalScalar(value) => Ok(u64::from(value)),
            Self::Logical(array) => array
                .as_slice()
                .get(offset)
                .copied()
                .map(Logical::get)
                .map(u64::from)
                .ok_or_else(|| mapping_error(name)),
        }
    }
}

fn double_or_logical_binary(
    operation: BinaryOperation,
    left: &Value,
    right: &Value,
    assumed: Option<WordSpec>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    let left = DoubleLogicalView::new(left).expect("caller checked the left operand");
    let right = DoubleLogicalView::new(right).ok_or_else(|| mixed_class_error(name))?;
    let both_logical = left.is_logical() && right.is_logical();
    if both_logical && assumed.is_some() {
        return Err(invalid_assumed_type(name, "logical"));
    }
    let spec = assumed.unwrap_or(WordSpec::U64);
    left.validate(name, 1, spec, context)?;
    right.validate(name, 2, spec, context)?;
    let shape = implicit_shape(
        name,
        left.dimensions_borrowed(),
        right.dimensions_borrowed(),
    )?;
    let length = host_length(name, &shape)?;
    if both_logical {
        let mut values = reserved_vec(name, length)?;
        for offset in 0..length {
            check_cancelled_at(context, offset)?;
            let left_offset = broadcast_offset(name, offset, &shape, left.dimensions_borrowed())?;
            let right_offset = broadcast_offset(name, offset, &shape, right.dimensions_borrowed())?;
            values.push(Logical::from(
                operation.apply(
                    left.raw_at(left_offset, name, 1, spec)?,
                    right.raw_at(right_offset, name, 2, spec)?,
                ) != 0,
            ));
        }
        if left.scalar_variant() && right.scalar_variant() {
            return values
                .first()
                .copied()
                .map(Logical::get)
                .map(Value::Logical)
                .ok_or_else(|| mapping_error(name));
        }
        return DenseArray::from_vec(shape, values)
            .map(ArrayData::from_typed)
            .map(Value::Array)
            .map_err(|_| allocation_error(name));
    }

    let mut values = reserved_vec(name, length)?;
    for offset in 0..length {
        check_cancelled_at(context, offset)?;
        let left_offset = broadcast_offset(name, offset, &shape, left.dimensions_borrowed())?;
        let right_offset = broadcast_offset(name, offset, &shape, right.dimensions_borrowed())?;
        values.push(spec.decode_f64(
            operation.apply(
                left.raw_at(left_offset, name, 1, spec)?,
                right.raw_at(right_offset, name, 2, spec)?,
            ) & spec.mask(),
        ));
    }
    dense_double(
        name,
        shape,
        values,
        left.scalar_variant() && right.scalar_variant(),
    )
}

fn primary_word_spec(
    name: &str,
    primary: &Value,
    assumed: Option<WordSpec>,
) -> Result<WordSpec, BuiltinError> {
    let stored = match primary {
        Value::Double(_) | Value::Array(ArrayData::F64(_)) => {
            return Ok(assumed.unwrap_or(WordSpec::U64));
        }
        Value::Array(ArrayData::Integer(IntegerArrayData::I8(_))) => WordSpec::I8,
        Value::Array(ArrayData::Integer(IntegerArrayData::U8(_))) => WordSpec::U8,
        Value::Array(ArrayData::Integer(IntegerArrayData::I16(_))) => WordSpec::I16,
        Value::Array(ArrayData::Integer(IntegerArrayData::U16(_))) => WordSpec::U16,
        Value::Array(ArrayData::Integer(IntegerArrayData::I32(_))) => WordSpec::I32,
        Value::Array(ArrayData::Integer(IntegerArrayData::U32(_))) => WordSpec::U32,
        Value::Array(ArrayData::Integer(IntegerArrayData::I64(_))) => WordSpec::I64,
        Value::Array(ArrayData::Integer(IntegerArrayData::U64(_))) => WordSpec::U64,
        Value::Complex(_)
        | Value::Array(
            ArrayData::ComplexF64(_)
            | ArrayData::Integer(_)
            | openmat_array::ArrayData::ComplexF32(_),
        ) => {
            return Err(complex_error(name));
        }
        value => return Err(primary_type_error(name, value, false)),
    };
    if assumed.is_some_and(|spec| spec != stored) {
        return Err(invalid_assumed_type(name, stored.class_name()));
    }
    Ok(stored)
}

fn map_primary<F>(
    name: &str,
    primary: &Value,
    assumed: Option<WordSpec>,
    controls: &[RealControlView<'_>],
    context: &BuiltinContext<'_>,
    operation: F,
) -> Result<Value, BuiltinError>
where
    F: FnMut(u64, WordSpec, &[RealNumber]) -> Result<u64, BuiltinError>,
{
    match primary {
        Value::Double(_) | Value::Array(ArrayData::F64(_)) => map_double_primary(
            name,
            primary,
            assumed.unwrap_or(WordSpec::U64),
            controls,
            context,
            operation,
        ),
        Value::Array(ArrayData::Integer(integer)) if !integer.is_complex() => {
            macro_rules! dispatch {
                ($array:expr, $type:ty) => {
                    map_integer_primary::<$type, _>(
                        name, $array, assumed, controls, context, operation,
                    )
                };
            }
            match integer {
                IntegerArrayData::I8(array) => dispatch!(array, i8),
                IntegerArrayData::U8(array) => dispatch!(array, u8),
                IntegerArrayData::I16(array) => dispatch!(array, i16),
                IntegerArrayData::U16(array) => dispatch!(array, u16),
                IntegerArrayData::I32(array) => dispatch!(array, i32),
                IntegerArrayData::U32(array) => dispatch!(array, u32),
                IntegerArrayData::I64(array) => dispatch!(array, i64),
                IntegerArrayData::U64(array) => dispatch!(array, u64),
                IntegerArrayData::ComplexI8(_)
                | IntegerArrayData::ComplexU8(_)
                | IntegerArrayData::ComplexI16(_)
                | IntegerArrayData::ComplexU16(_)
                | IntegerArrayData::ComplexI32(_)
                | IntegerArrayData::ComplexU32(_)
                | IntegerArrayData::ComplexI64(_)
                | IntegerArrayData::ComplexU64(_) => Err(complex_error(name)),
            }
        }
        Value::Complex(_)
        | Value::Array(ArrayData::ComplexF64(_) | openmat_array::ArrayData::ComplexF32(_)) => {
            Err(complex_error(name))
        }
        value => Err(primary_type_error(name, value, false)),
    }
}

fn map_double_primary<F>(
    name: &str,
    primary: &Value,
    spec: WordSpec,
    controls: &[RealControlView<'_>],
    context: &BuiltinContext<'_>,
    mut operation: F,
) -> Result<Value, BuiltinError>
where
    F: FnMut(u64, WordSpec, &[RealNumber]) -> Result<u64, BuiltinError>,
{
    let (values, dimensions, scalar_variant) = match primary {
        Value::Double(value) => (std::slice::from_ref(value), &SCALAR_DIMENSIONS[..], true),
        Value::Array(ArrayData::F64(array)) => {
            (array.as_slice(), array.shape().dimensions(), false)
        }
        _ => unreachable!("caller checked the primary input"),
    };
    for (offset, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, offset)?;
        double_to_raw(name, 1, value, spec, false)?;
    }
    let shape = scalar_expansion_shape(name, dimensions, controls)?;
    let length = host_length(name, &shape)?;
    let mut output = reserved_vec(name, length)?;
    let mut control_values = reserved_vec(name, controls.len())?;
    for offset in 0..length {
        check_cancelled_at(context, offset)?;
        control_values.clear();
        for control in controls {
            control_values.push(control.value_at(expanded_offset(control, offset), name)?);
        }
        let primary_offset = if values.len() == 1 { 0 } else { offset };
        let value = *values
            .get(primary_offset)
            .ok_or_else(|| mapping_error(name))?;
        let raw = double_to_raw(name, 1, value, spec, false)?;
        output.push(spec.decode_f64(operation(raw, spec, &control_values)? & spec.mask()));
    }
    dense_double(
        name,
        shape,
        output,
        scalar_variant && controls.iter().all(|view| view.scalar_variant()),
    )
}

fn map_integer_primary<T, F>(
    name: &str,
    primary: &DenseArray<T>,
    assumed: Option<WordSpec>,
    controls: &[RealControlView<'_>],
    context: &BuiltinContext<'_>,
    mut operation: F,
) -> Result<Value, BuiltinError>
where
    T: BitWord,
    F: FnMut(u64, WordSpec, &[RealNumber]) -> Result<u64, BuiltinError>,
{
    if assumed.is_some_and(|spec| spec != T::SPEC) {
        return Err(invalid_assumed_type(name, T::SPEC.class_name()));
    }
    let shape = scalar_expansion_shape(name, primary.shape().dimensions(), controls)?;
    let length = host_length(name, &shape)?;
    let mut output = reserved_vec(name, length)?;
    let mut control_values = reserved_vec(name, controls.len())?;
    for offset in 0..length {
        check_cancelled_at(context, offset)?;
        control_values.clear();
        for control in controls {
            control_values.push(control.value_at(expanded_offset(control, offset), name)?);
        }
        let primary_offset = if primary.as_slice().len() == 1 {
            0
        } else {
            offset
        };
        let raw = primary
            .as_slice()
            .get(primary_offset)
            .copied()
            .map(BitWord::to_raw)
            .ok_or_else(|| mapping_error(name))?;
        output.push(T::from_raw(
            operation(raw, T::SPEC, &control_values)? & T::SPEC.mask(),
        ));
    }
    dense_integer(name, shape, output)
}

#[derive(Clone, Copy, Debug)]
enum RealNumber {
    Float(f64),
    Signed(i128),
    Unsigned(u128),
}

impl RealNumber {
    fn shift_amount(self, name: &str, position: usize, width: u32) -> Result<i32, BuiltinError> {
        let width = i32::try_from(width).expect("supported word widths fit i32");
        match self {
            Self::Float(value) => {
                if !value.is_finite() || value.fract() != 0.0 {
                    return Err(integer_control_error(name, position));
                }
                if value >= f64::from(width) {
                    Ok(width)
                } else if value <= -f64::from(width) {
                    Ok(-width)
                } else {
                    Ok(value as i32)
                }
            }
            Self::Signed(value) => {
                if value >= i128::from(width) {
                    Ok(width)
                } else if value <= -i128::from(width) {
                    Ok(-width)
                } else {
                    i32::try_from(value).map_err(|_| integer_control_error(name, position))
                }
            }
            Self::Unsigned(value) => {
                if value >= width as u128 {
                    Ok(width)
                } else {
                    i32::try_from(value).map_err(|_| integer_control_error(name, position))
                }
            }
        }
    }

    fn bit_position(self, name: &str, position: usize, width: u32) -> Result<u32, BuiltinError> {
        let position_value = match self {
            Self::Float(value)
                if value.is_finite()
                    && value.fract() == 0.0
                    && value >= 1.0
                    && value <= f64::from(width) =>
            {
                value as u32
            }
            Self::Signed(value) if value >= 1 && value <= i128::from(width) => value as u32,
            Self::Unsigned(value) if value >= 1 && value <= u128::from(width) => value as u32,
            _ => return Err(bit_position_error(name, position, width)),
        };
        Ok(position_value)
    }

    const fn is_nonzero(self) -> bool {
        match self {
            Self::Float(value) => value != 0.0,
            Self::Signed(value) => value != 0,
            Self::Unsigned(value) => value != 0,
        }
    }
}

#[derive(Clone, Copy)]
enum RealControlView<'a> {
    DoubleScalar(f64),
    Double(&'a DenseArray<f64>),
    Single(&'a DenseArray<f32>),
    Integer(&'a IntegerArrayData),
}

impl<'a> RealControlView<'a> {
    fn new(name: &str, position: usize, value: &'a Value) -> Result<Self, BuiltinError> {
        match value {
            Value::Double(value) => Ok(Self::DoubleScalar(*value)),
            Value::Array(ArrayData::F64(array)) => Ok(Self::Double(array)),
            Value::Array(openmat_array::ArrayData::F32(array)) => Ok(Self::Single(array)),
            Value::Array(ArrayData::Integer(integer)) if !integer.is_complex() => {
                Ok(Self::Integer(integer))
            }
            Value::Complex(_)
            | Value::Array(
                ArrayData::ComplexF64(_)
                | ArrayData::Integer(_)
                | openmat_array::ArrayData::ComplexF32(_),
            ) => Err(complex_error(name)),
            _ => Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                format!("input {position} to `{name}` must be a real numeric array"),
            )),
        }
    }

    fn dimensions(self) -> &'a [u64] {
        match self {
            Self::DoubleScalar(_) => &SCALAR_DIMENSIONS,
            Self::Double(array) => array.shape().dimensions(),
            Self::Single(array) => array.shape().dimensions(),
            Self::Integer(array) => array.shape().dimensions(),
        }
    }

    fn numel(self) -> usize {
        match self {
            Self::DoubleScalar(_) => 1,
            Self::Double(array) => array.as_slice().len(),
            Self::Single(array) => array.as_slice().len(),
            Self::Integer(array) => array.elements().len(),
        }
    }

    const fn scalar_variant(self) -> bool {
        matches!(self, Self::DoubleScalar(_))
    }

    fn value_at(self, offset: usize, name: &str) -> Result<RealNumber, BuiltinError> {
        match self {
            Self::DoubleScalar(value) if offset == 0 => Ok(RealNumber::Float(value)),
            Self::DoubleScalar(_) => Err(mapping_error(name)),
            Self::Double(array) => array
                .as_slice()
                .get(offset)
                .copied()
                .map(RealNumber::Float)
                .ok_or_else(|| mapping_error(name)),
            Self::Single(array) => array
                .as_slice()
                .get(offset)
                .copied()
                .map(f64::from)
                .map(RealNumber::Float)
                .ok_or_else(|| mapping_error(name)),
            Self::Integer(array) => {
                let value = array.element(offset).ok_or_else(|| mapping_error(name))?;
                match value.real_component() {
                    IntegerComponent::Signed(value) => Ok(RealNumber::Signed(value)),
                    IntegerComponent::Unsigned(value) => Ok(RealNumber::Unsigned(value)),
                }
            }
        }
    }
}

fn validate_control_values<F>(
    name: &str,
    control: RealControlView<'_>,
    context: &BuiltinContext<'_>,
    mut validate: F,
) -> Result<(), BuiltinError>
where
    F: FnMut(RealNumber) -> Result<(), BuiltinError>,
{
    for offset in 0..control.numel() {
        check_cancelled_at(context, offset)?;
        validate(control.value_at(offset, name)?)?;
    }
    Ok(())
}

fn scalar_expansion_shape(
    name: &str,
    primary_dimensions: &[u64],
    controls: &[RealControlView<'_>],
) -> Result<Shape, BuiltinError> {
    let primary_numel = checked_numel(name, primary_dimensions)?;
    let mut dimensions = if primary_numel == 1 {
        None
    } else {
        Some(primary_dimensions)
    };
    for control in controls {
        if control.numel() == 1 {
            continue;
        }
        match dimensions {
            Some(existing) if existing != control.dimensions() => return Err(shape_error(name)),
            Some(_) => {}
            None => dimensions = Some(control.dimensions()),
        }
    }
    Shape::new(dimensions.unwrap_or(primary_dimensions).iter().copied())
        .map_err(|_| allocation_error(name))
}

fn expanded_offset(control: &RealControlView<'_>, output_offset: usize) -> usize {
    if control.numel() == 1 {
        0
    } else {
        output_offset
    }
}

fn implicit_shape(name: &str, left: &[u64], right: &[u64]) -> Result<Shape, BuiltinError> {
    let rank = left.len().max(right.len()).max(2);
    let mut dimensions = reserved_vec(name, rank)?;
    for axis in 0..rank {
        let left = left.get(axis).copied().unwrap_or(1);
        let right = right.get(axis).copied().unwrap_or(1);
        let extent = if left == right {
            left
        } else if left == 1 {
            right
        } else if right == 1 {
            left
        } else {
            return Err(shape_error(name));
        };
        dimensions.push(extent);
    }
    Shape::new(dimensions).map_err(|_| allocation_error(name))
}

fn broadcast_offset(
    name: &str,
    output_offset: usize,
    output_shape: &Shape,
    input_dimensions: &[u64],
) -> Result<usize, BuiltinError> {
    let mut input_offset = 0_u64;
    let mut input_stride = 1_u64;
    let output_offset = u64::try_from(output_offset).map_err(|_| mapping_error(name))?;
    for axis in 0..output_shape.ndims() {
        let input_extent = input_dimensions.get(axis).copied().unwrap_or(1);
        if input_extent != 1 {
            let output_extent = output_shape.extent(axis);
            if output_extent == 0 {
                return Err(mapping_error(name));
            }
            let coordinate = (output_offset / output_shape.stride(axis)) % output_extent;
            input_offset = input_offset
                .checked_add(
                    coordinate
                        .checked_mul(input_stride)
                        .ok_or_else(|| mapping_error(name))?,
                )
                .ok_or_else(|| mapping_error(name))?;
        }
        input_stride = input_stride
            .checked_mul(input_extent)
            .ok_or_else(|| mapping_error(name))?;
    }
    usize::try_from(input_offset).map_err(|_| mapping_error(name))
}

fn checked_numel(name: &str, dimensions: &[u64]) -> Result<u64, BuiltinError> {
    dimensions.iter().try_fold(1_u64, |total, extent| {
        total
            .checked_mul(*extent)
            .ok_or_else(|| allocation_error(name))
    })
}

fn double_to_raw(
    name: &str,
    position: usize,
    value: f64,
    spec: WordSpec,
    mixed_integer: bool,
) -> Result<u64, BuiltinError> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(value_range_error(name, position, spec, mixed_integer));
    }
    if spec.signed {
        let bound = 2_f64.powi(i32::try_from(spec.bits - 1).expect("word width fits i32"));
        if value < -bound || value >= bound {
            return Err(value_range_error(name, position, spec, mixed_integer));
        }
        let signed = value as i64;
        Ok((signed as u64) & spec.mask())
    } else {
        let upper = 2_f64.powi(i32::try_from(spec.bits).expect("word width fits i32"));
        if value < 0.0 || value >= upper {
            return Err(value_range_error(name, position, spec, mixed_integer));
        }
        Ok((value as u64) & spec.mask())
    }
}

fn sign_extend(raw: u64, spec: WordSpec) -> i64 {
    let raw = raw & spec.mask();
    if spec.bits == 64 || raw & spec.sign_bit() == 0 {
        raw as i64
    } else {
        (raw | !spec.mask()) as i64
    }
}

fn shift_word(raw: u64, amount: i32, spec: WordSpec) -> u64 {
    let raw = raw & spec.mask();
    if amount >= i32::try_from(spec.bits).expect("word width fits i32") {
        return 0;
    }
    if amount <= -i32::try_from(spec.bits).expect("word width fits i32") {
        return if spec.signed && raw & spec.sign_bit() != 0 {
            spec.mask()
        } else {
            0
        };
    }
    if amount >= 0 {
        (raw << amount.unsigned_abs()) & spec.mask()
    } else if spec.signed {
        (sign_extend(raw, spec) >> amount.unsigned_abs()) as u64 & spec.mask()
    } else {
        raw >> amount.unsigned_abs()
    }
}

fn assumed_type(name: &str, position: usize, value: &Value) -> Result<WordSpec, BuiltinError> {
    let text = text_scalar(name, position, value)?;
    match text.as_str() {
        "int8" => Ok(WordSpec::I8),
        "uint8" => Ok(WordSpec::U8),
        "int16" => Ok(WordSpec::I16),
        "uint16" => Ok(WordSpec::U16),
        "int32" => Ok(WordSpec::I32),
        "uint32" => Ok(WordSpec::U32),
        "int64" => Ok(WordSpec::I64),
        "uint64" => Ok(WordSpec::U64),
        _ => Err(unknown_assumed_type(name, position)),
    }
}

fn text_scalar(name: &str, position: usize, value: &Value) -> Result<String, BuiltinError> {
    let units = match value {
        Value::String(StringValue::Scalar(value)) if !value.is_missing() => value.code_units(),
        Value::String(StringValue::Array(value)) if value.numel() == 1 => value
            .as_slice()
            .first()
            .filter(|value| !value.is_missing())
            .map(openmat_value::StringElement::code_units)
            .ok_or_else(|| text_type_error(name, position))?,
        Value::Array(ArrayData::Char(array))
            if array.shape().ndims() == 2 && array.shape().extent(0) == 1 =>
        {
            if array.as_slice().len() > "uint64".len() {
                return Err(unknown_assumed_type(name, position));
            }
            let mut text = String::new();
            text.try_reserve_exact(array.as_slice().len())
                .map_err(|_| allocation_error(name))?;
            for unit in array.as_slice() {
                let byte = u8::try_from(unit.get()).map_err(|_| text_type_error(name, position))?;
                text.push(char::from(byte).to_ascii_lowercase());
            }
            return Ok(text);
        }
        _ => return Err(text_type_error(name, position)),
    };
    if units.len() > "uint64".len() {
        return Err(unknown_assumed_type(name, position));
    }
    let mut text = String::new();
    text.try_reserve_exact(units.len())
        .map_err(|_| allocation_error(name))?;
    for unit in units.iter().copied() {
        let byte = u8::try_from(unit).map_err(|_| text_type_error(name, position))?;
        text.push(char::from(byte).to_ascii_lowercase());
    }
    Ok(text)
}

fn is_text_scalar(value: &Value) -> bool {
    match value {
        Value::String(StringValue::Scalar(_)) => true,
        Value::String(StringValue::Array(array)) => array.numel() == 1,
        Value::Array(ArrayData::Char(array)) => {
            array.shape().ndims() == 2 && array.shape().extent(0) == 1
        }
        _ => false,
    }
}

fn is_double_or_logical(value: &Value) -> bool {
    matches!(
        value,
        Value::Double(_)
            | Value::Logical(_)
            | Value::Array(ArrayData::F64(_) | ArrayData::Logical(_))
    )
}

fn double_scalar(value: &Value) -> Option<f64> {
    match value {
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            array.as_slice().first().copied()
        }
        _ => None,
    }
}

fn dense_integer<T: BitWord>(
    name: &str,
    shape: Shape,
    values: Vec<T>,
) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values)
        .map(IntegerArrayData::from_typed)
        .map(ArrayData::Integer)
        .map(Value::Array)
        .map_err(|_| allocation_error(name))
}

fn dense_double(
    name: &str,
    shape: Shape,
    values: Vec<f64>,
    scalar_variant: bool,
) -> Result<Value, BuiltinError> {
    if scalar_variant {
        return values
            .first()
            .copied()
            .map(Value::Double)
            .ok_or_else(|| mapping_error(name));
    }
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|_| allocation_error(name))
}

fn reserved_vec<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(name))?;
    Ok(values)
}

fn host_length(name: &str, shape: &Shape) -> Result<usize, BuiltinError> {
    usize::try_from(shape.numel()).map_err(|_| allocation_error(name))
}

fn validate_call(
    name: &str,
    arguments: &[Value],
    minimum: usize,
    maximum: usize,
    context: &BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    if !(minimum..=maximum).contains(&arguments.len()) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `{name}` expects {minimum} to {maximum} inputs but received {}",
                arguments.len()
            ),
        ));
    }
    if context.requested_outputs() > 1 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("built-in `{name}` returns at most 1 output"),
        ));
    }
    Ok(())
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn primary_type_error(name: &str, value: &Value, binary: bool) -> BuiltinError {
    let expected = if binary {
        "real double, logical, or fixed-width integer array"
    } else {
        "real double or fixed-width integer array"
    };
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!(
            "input 1 to `{name}` must be a {expected}; received `{}`",
            value.class_name()
        ),
    )
}

fn complex_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!("`{name}` does not accept complex inputs"),
    )
}

fn mixed_class_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!("inputs to `{name}` have incompatible numeric classes"),
    )
}

fn invalid_assumed_type(name: &str, class: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("the assumed type for `{name}` must match the `{class}` input class"),
    )
}

fn unknown_assumed_type(name: &str, position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `{name}` names an unsupported assumed integer type"),
    )
}

fn value_range_error(
    name: &str,
    position: usize,
    spec: WordSpec,
    mixed_integer: bool,
) -> BuiltinError {
    let detail = if mixed_integer {
        "the scalar double mixed with an integer array"
    } else {
        "a finite integer in the selected word range"
    };
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!(
            "input {position} to `{name}` must be {detail} (`{}`)",
            spec.class_name()
        ),
    )
}

fn integer_control_error(name: &str, position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `{name}` must contain finite integer values"),
    )
}

fn bit_position_error(name: &str, position: usize, width: u32) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `{name}` must contain bit positions from 1 through {width}"),
    )
}

fn shape_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("inputs to `{name}` have incompatible sizes"),
    )
}

fn text_type_error(name: &str, position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!("input {position} to `{name}` must be a character row or string scalar"),
    )
}

fn allocation_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` output allocation failed"),
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not map an output element to its input"),
    )
}

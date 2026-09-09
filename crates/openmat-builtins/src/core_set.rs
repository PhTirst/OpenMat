use std::cmp::Ordering;

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32, Complex64 as ArrayComplex64, DenseArray, IntegerArrayData,
    IntegerComponent, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{StringArray, StringElement, StringValue, Value};

use crate::{array_error, expect_argument_count_range, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

trait SetElement: Clone {
    fn set_equal(&self, other: &Self) -> bool;
    fn set_compare(&self, other: &Self) -> Ordering;
}

macro_rules! impl_set_ord {
    ($($kind:ty),+ $(,)?) => {$(
        impl SetElement for $kind {
            fn set_equal(&self, other: &Self) -> bool { self == other }
            fn set_compare(&self, other: &Self) -> Ordering { self.cmp(other) }
        }
    )+};
}

impl_set_ord!(i8, u8, i16, u16, i32, u32, i64, u64);

impl SetElement for Logical {
    fn set_equal(&self, other: &Self) -> bool {
        self == other
    }

    fn set_compare(&self, other: &Self) -> Ordering {
        self.get().cmp(&other.get())
    }
}

impl SetElement for CharCodeUnit {
    fn set_equal(&self, other: &Self) -> bool {
        self == other
    }

    fn set_compare(&self, other: &Self) -> Ordering {
        self.get().cmp(&other.get())
    }
}

macro_rules! impl_set_float {
    ($kind:ty) => {
        impl SetElement for $kind {
            #[allow(clippy::float_cmp)]
            fn set_equal(&self, other: &Self) -> bool {
                self == other
            }

            fn set_compare(&self, other: &Self) -> Ordering {
                match (self.is_nan(), other.is_nan()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    (false, false) => self.partial_cmp(other).unwrap_or(Ordering::Equal),
                }
            }
        }
    };
}

impl_set_float!(f32);
impl_set_float!(f64);

macro_rules! impl_set_complex {
    ($kind:ty) => {
        impl SetElement for $kind {
            #[allow(clippy::float_cmp)]
            fn set_equal(&self, other: &Self) -> bool {
                self.re == other.re && self.im == other.im
            }

            fn set_compare(&self, other: &Self) -> Ordering {
                let missing = self.re.is_nan() || self.im.is_nan();
                let other_missing = other.re.is_nan() || other.im.is_nan();
                match (missing, other_missing) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    (false, false) => self
                        .re
                        .hypot(self.im)
                        .partial_cmp(&other.re.hypot(other.im))
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| {
                            self.im
                                .atan2(self.re)
                                .partial_cmp(&other.im.atan2(other.re))
                                .unwrap_or(Ordering::Equal)
                        }),
                }
            }
        }
    };
}

impl_set_complex!(Complex32);
impl_set_complex!(ArrayComplex64);

impl SetElement for StringElement {
    fn set_equal(&self, other: &Self) -> bool {
        self == other
    }

    fn set_compare(&self, other: &Self) -> Ordering {
        match (self.is_missing(), other.is_missing()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.code_units().cmp(other.code_units()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UniqueOrder {
    Sorted,
    Stable,
}

pub(super) fn unique_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("unique", arguments, 1, 2)?;
    expect_max_outputs("unique", context, 3)?;
    context.check_cancelled()?;
    let order = arguments
        .get(1)
        .map_or(Ok(UniqueOrder::Sorted), unique_order)?;
    let outputs = unique_value(&arguments[0], order, context.requested_outputs(), context)?;
    context.check_cancelled()?;
    Ok(outputs)
}

fn unique_order(value: &Value) -> Result<UniqueOrder, BuiltinError> {
    match keyword(value).as_deref() {
        Some("sorted") => Ok(UniqueOrder::Sorted),
        Some("stable") => Ok(UniqueOrder::Stable),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`unique` currently accepts the R2022b sorted or stable vector forms",
        )),
    }
}

fn unique_value(
    input: &Value,
    order: UniqueOrder,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    macro_rules! unique_dense_value {
        ($array:expr, $wrap:expr) => {{
            let result = unique_dense($array, order, context)?;
            unique_outputs(
                $wrap(build_selected($array.shape(), result.values)?),
                $array.shape(),
                result.indices,
                result.inverse,
                requested_outputs,
            )
        }};
    }
    match input {
        Value::Logical(value) => {
            let array = scalar_array(Logical::from(*value))?;
            unique_dense_value!(&array, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Double(value) => {
            let array = scalar_array(*value)?;
            unique_dense_value!(&array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Complex(value) => {
            let array = scalar_array(ArrayComplex64::new(value.real, value.imaginary))?;
            unique_dense_value!(&array, |array| Value::Array(ArrayData::ComplexF64(array)))
        }
        Value::Array(ArrayData::F32(array)) => {
            unique_dense_value!(array, |array| { Value::Array(ArrayData::F32(array)) })
        }
        Value::Array(ArrayData::ComplexF32(array)) => unique_dense_value!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        }),
        Value::Array(ArrayData::Logical(array)) => {
            unique_dense_value!(array, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Array(ArrayData::F64(array)) => {
            unique_dense_value!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => unique_dense_value!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Value::Array(ArrayData::Char(array)) => {
            unique_dense_value!(array, |array| Value::Array(ArrayData::Char(array)))
        }
        Value::Array(ArrayData::Integer(array)) => {
            unique_integer(array, order, requested_outputs, context)
        }
        Value::String(value) => {
            let shape = Shape::new(value.dimensions().iter().copied())
                .map_err(|error| array_error(&error))?;
            let elements = (0..host_length("unique", value.numel())?)
                .map(|index| {
                    value
                        .element(index)
                        .cloned()
                        .ok_or_else(|| mapping_error("unique"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let array =
                DenseArray::from_vec(shape, elements).map_err(|error| array_error(&error))?;
            let result = unique_dense(&array, order, context)?;
            let selected = build_selected(array.shape(), result.values)?;
            let string =
                StringArray::from_elements(selected.shape().clone(), selected.as_slice().to_vec())
                    .map_err(|error| array_error(&error))?;
            unique_outputs(
                Value::String(StringValue::Array(string)),
                array.shape(),
                result.indices,
                result.inverse,
                requested_outputs,
            )
        }
        value => Err(type_error(
            "unique",
            1,
            "numeric, logical, char, integer, or string array",
            value,
        )),
    }
}

struct UniqueResult<T> {
    values: Vec<T>,
    indices: Vec<f64>,
    inverse: Vec<f64>,
}

fn unique_dense<T: SetElement>(
    input: &DenseArray<T>,
    order: UniqueOrder,
    context: &BuiltinContext<'_>,
) -> Result<UniqueResult<T>, BuiltinError> {
    let mut values = Vec::new();
    let mut indices = Vec::new();
    let mut inverse = vec![0.0; input.as_slice().len()];
    match order {
        UniqueOrder::Stable => {
            for (original, value) in input.as_slice().iter().enumerate() {
                check_cancelled_at(context, original)?;
                if let Some(group) = values.iter().position(|existing| value.set_equal(existing)) {
                    inverse[original] = index_as_f64(group)?;
                } else {
                    values.push(value.clone());
                    indices.push(index_as_f64(original)?);
                    inverse[original] = index_as_f64(values.len() - 1)?;
                }
            }
        }
        UniqueOrder::Sorted => {
            let mut entries: Vec<_> = input.as_slice().iter().cloned().enumerate().collect();
            entries.sort_by(|left, right| left.1.set_compare(&right.1));
            for (entry, (original, value)) in entries.into_iter().enumerate() {
                check_cancelled_at(context, entry)?;
                if values
                    .last()
                    .is_some_and(|existing| value.set_equal(existing))
                {
                    inverse[original] = index_as_f64(values.len() - 1)?;
                } else {
                    values.push(value);
                    indices.push(index_as_f64(original)?);
                    inverse[original] = index_as_f64(values.len() - 1)?;
                }
            }
        }
    }
    Ok(UniqueResult {
        values,
        indices,
        inverse,
    })
}

fn build_selected<T: Clone>(
    input_shape: &Shape,
    values: Vec<T>,
) -> Result<DenseArray<T>, BuiltinError> {
    let length = u64::try_from(values.len()).map_err(|_| mapping_error("unique"))?;
    let shape = unique_shape(input_shape, length)?;
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn unique_shape(input: &Shape, length: u64) -> Result<Shape, BuiltinError> {
    let dimensions = input.dimensions();
    let output = if input.numel() == 0 && dimensions == [0, 0] {
        vec![0, 0]
    } else if dimensions.len() == 2 && dimensions[0] == 1 {
        vec![1, length]
    } else {
        vec![length, 1]
    };
    Shape::new(output).map_err(|error| array_error(&error))
}

fn unique_outputs(
    values: Value,
    input_shape: &Shape,
    indices: Vec<f64>,
    inverse: Vec<f64>,
    requested: usize,
) -> Result<Vec<Value>, BuiltinError> {
    let mut outputs = Vec::new();
    outputs
        .try_reserve_exact(requested)
        .map_err(|_| mapping_error("unique"))?;
    if requested > 0 {
        outputs.push(values);
    }
    if requested > 1 {
        outputs.push(index_column(indices)?);
    }
    if requested > 2 {
        let expected = host_length("unique", input_shape.numel())?;
        if inverse.len() != expected {
            return Err(mapping_error("unique"));
        }
        outputs.push(index_column(inverse)?);
    }
    Ok(outputs)
}

fn unique_integer(
    input: &IntegerArrayData,
    order: UniqueOrder,
    requested: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    macro_rules! variant {
        ($array:expr) => {{
            let result = unique_dense($array, order, context)?;
            let selected = build_selected($array.shape(), result.values)?;
            unique_outputs(
                Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(selected))),
                $array.shape(),
                result.indices,
                result.inverse,
                requested,
            )
        }};
    }
    match input {
        IntegerArrayData::I8(array) => variant!(array),
        IntegerArrayData::U8(array) => variant!(array),
        IntegerArrayData::I16(array) => variant!(array),
        IntegerArrayData::U16(array) => variant!(array),
        IntegerArrayData::I32(array) => variant!(array),
        IntegerArrayData::U32(array) => variant!(array),
        IntegerArrayData::I64(array) => variant!(array),
        IntegerArrayData::U64(array) => variant!(array),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "`unique` does not accept complex integer storage",
        )),
    }
}

pub(super) fn ismember_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("ismember", arguments, 2, 2)?;
    expect_max_outputs("ismember", context, 2)?;
    context.check_cancelled()?;
    let (shape, scalar, left) = comparable_values("ismember", 1, &arguments[0], context)?;
    let (_, _, right) = comparable_values("ismember", 2, &arguments[1], context)?;
    let mut membership = Vec::new();
    let mut locations = Vec::new();
    membership
        .try_reserve_exact(left.len())
        .map_err(|_| mapping_error("ismember"))?;
    locations
        .try_reserve_exact(left.len())
        .map_err(|_| mapping_error("ismember"))?;
    for (left_index, left) in left.iter().enumerate() {
        check_cancelled_at(context, left_index)?;
        let mut location = 0_usize;
        for (right_index, right) in right.iter().enumerate() {
            if left.equal(right) {
                location = right_index + 1;
            }
        }
        membership.push(Logical::from(location != 0));
        #[allow(clippy::cast_precision_loss)]
        locations.push(location as f64);
    }
    let logical = if scalar {
        Value::Logical(membership.first().is_some_and(|value| value.get()))
    } else {
        DenseArray::from_vec(shape.clone(), membership)
            .map(ArrayData::Logical)
            .map(Value::Array)
            .map_err(|error| array_error(&error))?
    };
    let mut outputs = vec![logical];
    if context.requested_outputs() > 1 {
        let location = if scalar {
            Value::Double(locations.first().copied().unwrap_or(0.0))
        } else {
            DenseArray::from_vec(shape, locations)
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(|error| array_error(&error))?
        };
        outputs.push(location);
    }
    context.check_cancelled()?;
    Ok(outputs)
}

#[derive(Clone, Debug)]
enum Comparable {
    Numeric(ComparableNumeric),
    String(StringElement),
}

impl Comparable {
    fn equal(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Numeric(left), Self::Numeric(right)) => left.equal(*right),
            (Self::String(left), Self::String(right)) => left == right,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ComparableNumeric {
    real: ComparableComponent,
    imaginary: ComparableComponent,
}

impl ComparableNumeric {
    const fn real(value: ComparableComponent) -> Self {
        Self {
            real: value,
            imaginary: ComparableComponent::Integer(IntegerComponent::Unsigned(0)),
        }
    }

    fn equal(self, other: Self) -> bool {
        comparable_components_equal(self.real, other.real)
            && comparable_components_equal(self.imaginary, other.imaginary)
    }
}

#[derive(Clone, Copy, Debug)]
enum ComparableComponent {
    Float(f64),
    Integer(IntegerComponent),
}

fn comparable_values(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<(Shape, bool, Vec<Comparable>), BuiltinError> {
    let dimensions = value
        .dimensions()
        .ok_or_else(|| type_error(name, position, "numeric, char, or string array", value))?;
    let shape = Shape::new(dimensions.iter().copied()).map_err(|error| array_error(&error))?;
    let scalar = matches!(
        value,
        Value::Logical(_) | Value::Double(_) | Value::Complex(_)
    );
    let length = host_length(name, value.numel().unwrap_or(0))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| mapping_error(name))?;
    for index in 0..length {
        check_cancelled_at(context, index)?;
        output.push(
            comparable_element(value, index).ok_or_else(|| {
                type_error(name, position, "numeric, char, or string array", value)
            })?,
        );
    }
    Ok((shape, scalar, output))
}

fn comparable_element(value: &Value, index: usize) -> Option<Comparable> {
    let numeric = match value {
        Value::Logical(value) if index == 0 => Some(ComparableNumeric::real(integer_bool(*value))),
        Value::Double(value) if index == 0 => {
            Some(ComparableNumeric::real(ComparableComponent::Float(*value)))
        }
        Value::Complex(value) if index == 0 => Some(ComparableNumeric {
            real: ComparableComponent::Float(value.real),
            imaginary: ComparableComponent::Float(value.imaginary),
        }),
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .get(index)
            .map(|value| ComparableNumeric::real(ComparableComponent::Float(f64::from(*value)))),
        Value::Array(ArrayData::ComplexF32(array)) => {
            array.as_slice().get(index).map(|value| ComparableNumeric {
                real: ComparableComponent::Float(f64::from(value.re)),
                imaginary: ComparableComponent::Float(f64::from(value.im)),
            })
        }
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .get(index)
            .map(|value| ComparableNumeric::real(ComparableComponent::Float(*value))),
        Value::Array(ArrayData::ComplexF64(array)) => {
            array.as_slice().get(index).map(|value| ComparableNumeric {
                real: ComparableComponent::Float(value.re),
                imaginary: ComparableComponent::Float(value.im),
            })
        }
        Value::Array(ArrayData::Logical(array)) => array
            .as_slice()
            .get(index)
            .map(|value| ComparableNumeric::real(integer_bool(value.get()))),
        Value::Array(ArrayData::Char(array)) => array.as_slice().get(index).map(|value| {
            ComparableNumeric::real(ComparableComponent::Integer(IntegerComponent::Unsigned(
                u128::from(value.get()),
            )))
        }),
        Value::Array(ArrayData::Integer(array)) => {
            array.element(index).map(|value| ComparableNumeric {
                real: ComparableComponent::Integer(value.real_component()),
                imaginary: ComparableComponent::Integer(
                    value
                        .imaginary_component()
                        .unwrap_or(IntegerComponent::Unsigned(0)),
                ),
            })
        }
        _ => None,
    };
    if let Some(value) = numeric {
        return Some(Comparable::Numeric(value));
    }
    match value {
        Value::String(value) => value.element(index).cloned().map(Comparable::String),
        _ => None,
    }
}

fn integer_bool(value: bool) -> ComparableComponent {
    ComparableComponent::Integer(IntegerComponent::Unsigned(u128::from(u8::from(value))))
}

#[allow(clippy::float_cmp)]
fn comparable_components_equal(left: ComparableComponent, right: ComparableComponent) -> bool {
    match (left, right) {
        (ComparableComponent::Float(left), ComparableComponent::Float(right)) => left == right,
        (ComparableComponent::Integer(left), ComparableComponent::Integer(right)) => {
            integer_identity(left) == integer_identity(right)
        }
        (ComparableComponent::Float(float), ComparableComponent::Integer(integer))
        | (ComparableComponent::Integer(integer), ComparableComponent::Float(float)) => {
            float_integer_identity(float).is_some_and(|value| value == integer_identity(integer))
        }
    }
}

fn integer_identity(value: IntegerComponent) -> (bool, u128) {
    if let Some(value) = value.as_signed() {
        (value.is_negative(), value.unsigned_abs())
    } else {
        (false, value.as_unsigned().unwrap_or(0))
    }
}

fn float_integer_identity(value: f64) -> Option<(bool, u128)> {
    if !value.is_finite() || value.fract() != 0.0 {
        return None;
    }
    if value >= 0.0 {
        if value >= 18_446_744_073_709_551_616.0 {
            return None;
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let magnitude = value as u64;
        Some((false, u128::from(magnitude)))
    } else {
        if value < -9_223_372_036_854_775_808.0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        let integer = value as i64;
        Some((true, u128::from(integer.unsigned_abs())))
    }
}

fn scalar_array<T: Clone>(value: T) -> Result<DenseArray<T>, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map_err(|error| array_error(&error))
}

fn index_column(values: Vec<f64>) -> Result<Value, BuiltinError> {
    let length = u64::try_from(values.len()).map_err(|_| mapping_error("unique"))?;
    DenseArray::from_vec(
        Shape::new([length, 1]).map_err(|error| array_error(&error))?,
        values,
    )
    .map(ArrayData::F64)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_precision_loss)]
fn index_as_f64(index: usize) -> Result<f64, BuiltinError> {
    index
        .checked_add(1)
        .map(|value| value as f64)
        .ok_or_else(|| mapping_error("unique"))
}

fn keyword(value: &Value) -> Option<String> {
    let units: Vec<u16> = match value {
        Value::String(value) => value.as_scalar()?.code_units().to_vec(),
        Value::Array(ArrayData::Char(array))
            if array.shape().ndims() == 2 && array.shape().extent(0) == 1 =>
        {
            array.as_slice().iter().map(|value| value.get()).collect()
        }
        _ => return None,
    };
    String::from_utf16(&units)
        .ok()
        .map(|value| value.to_ascii_lowercase())
}

fn host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| mapping_error(name))
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not allocate or map the requested array"),
    )
}

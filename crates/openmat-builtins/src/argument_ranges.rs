//! Scalar-bound range validators. Integers never round through binary64 and
//! sparse inputs are checked without materializing their implicit zeros.
use std::cmp::Ordering;

use super::{check, text_scalar, validation_error};
use crate::{BuiltinRegistry, expect_argument_count, expect_max_outputs};
use openmat_array::{ArrayData, IntegerComponent};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinRegistrationError, BuiltinResult};
use openmat_value::{SparseArrayData, Value};

pub(super) fn register(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    macro_rules! comparison {
        ($name:literal, $test:expr) => {
            registry.register(
                $name,
                |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                    expect_argument_count($name, arguments, 2)?;
                    expect_max_outputs($name, context, 0)?;
                    let bound = scalar(&arguments[1], context)?;
                    let valid = visit(&arguments[0], context, |value| {
                        value.compare(bound).is_some_and($test)
                    })?;
                    check($name, valid)
                },
            )?;
        };
    }
    comparison!("mustBeGreaterThan", |order| order == Ordering::Greater);
    comparison!("mustBeGreaterThanOrEqual", |order| order != Ordering::Less);
    comparison!("mustBeLessThan", |order| order == Ordering::Less);
    comparison!("mustBeLessThanOrEqual", |order| order != Ordering::Greater);
    registry.register("mustBeInRange", in_range)?;
    Ok(())
}

fn in_range(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if !(3..=5).contains(&arguments.len()) {
        return Err(validation_error("mustBeInRange arity"));
    }
    expect_max_outputs("mustBeInRange", context, 0)?;
    let lower = scalar(&arguments[1], context)?;
    let upper = scalar(&arguments[2], context)?;
    let flags = arguments[3..]
        .iter()
        .map(bound_flag)
        .collect::<Result<Vec<_>, _>>()?;
    if flags.len() == 2 && flags != [1, 2] && flags != [2, 1] {
        return Err(validation_error("mustBeInRange flags"));
    }
    let exclude = flags.into_iter().fold(0, |a, b| a | b);
    let valid = visit(&arguments[0], context, |value| {
        value.compare(lower).is_some_and(|order| {
            order == Ordering::Greater || (order == Ordering::Equal && exclude & 1 == 0)
        }) && value.compare(upper).is_some_and(|order| {
            order == Ordering::Less || (order == Ordering::Equal && exclude & 2 == 0)
        })
    })?;
    check("mustBeInRange", valid)
}

fn bound_flag(value: &Value) -> Result<u8, BuiltinError> {
    let text = text_scalar(value)
        .ok_or_else(|| validation_error("mustBeInRange flag"))?
        .to_ascii_lowercase();
    let mut candidates = [
        ("inclusive", 0),
        ("exclusive", 3),
        ("exclude-lower", 1),
        ("exclude-upper", 2),
    ]
    .into_iter()
    .filter(|(name, _)| !text.is_empty() && name.starts_with(&text));
    let (_, flag) = candidates
        .next()
        .ok_or_else(|| validation_error("mustBeInRange flag"))?;
    if candidates.next().is_some() {
        return Err(validation_error("mustBeInRange ambiguous flag"));
    }
    Ok(flag)
}

#[derive(Clone, Copy)]
enum Number {
    Float(f64),
    Integer(i128),
}

impl Number {
    fn integer(component: IntegerComponent) -> Result<Self, BuiltinError> {
        Ok(Self::Integer(match component {
            IntegerComponent::Signed(value) => value,
            IntegerComponent::Unsigned(value) => {
                i128::try_from(value).map_err(|_| validation_error("integer bound"))?
            }
        }))
    }

    fn compare(self, other: Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Float(a), Self::Float(b)) => a.partial_cmp(&b),
            (Self::Integer(a), Self::Integer(b)) => Some(a.cmp(&b)),
            (Self::Integer(a), Self::Float(b)) => integer_float(a, b),
            (Self::Float(a), Self::Integer(b)) => integer_float(b, a).map(Ordering::reverse),
        }
    }
}

// Stored integers are at most 64-bit. Bound the floating value first, then
// compare its integral and fractional parts without rounding the integer.
#[allow(clippy::cast_possible_truncation)]
fn integer_float(integer: i128, float: f64) -> Option<Ordering> {
    if float.is_nan() {
        return None;
    }
    if float >= 18_446_744_073_709_551_616.0 {
        return Some(Ordering::Less);
    }
    if float < -9_223_372_036_854_775_808.0 {
        return Some(Ordering::Greater);
    }
    let order = integer.cmp(&(float as i128));
    if order == Ordering::Equal {
        0.0_f64.partial_cmp(&float.fract())
    } else {
        Some(order)
    }
}

fn scalar(value: &Value, context: &BuiltinContext<'_>) -> Result<Number, BuiltinError> {
    if value.numel() != Some(1) {
        return Err(validation_error("scalar bound"));
    }
    let mut result = None;
    visit(value, context, |number| {
        result = Some(number);
        true
    })?;
    result.ok_or_else(|| validation_error("scalar bound"))
}

fn visit(
    value: &Value,
    context: &BuiltinContext<'_>,
    mut test: impl FnMut(Number) -> bool,
) -> Result<bool, BuiltinError> {
    context.check_cancelled()?;
    macro_rules! values {
        ($iterator:expr, $convert:expr) => {{
            for (index, value) in $iterator.enumerate() {
                if index.is_multiple_of(1024) {
                    context.check_cancelled()?;
                }
                if !test(($convert)(value)) {
                    return Ok(false);
                }
            }
            Ok(true)
        }};
    }
    match value {
        Value::Double(value) => Ok(test(Number::Float(*value))),
        Value::Logical(value) => Ok(test(Number::Integer(i128::from(*value)))),
        Value::Array(ArrayData::F64(array)) => {
            values!(array.as_slice().iter(), |v: &f64| Number::Float(*v))
        }
        Value::Array(ArrayData::F32(array)) => {
            values!(array.as_slice().iter(), |v: &f32| Number::Float(f64::from(
                *v
            )))
        }
        Value::Array(ArrayData::Logical(array)) => {
            values!(array.as_slice().iter(), |v: &openmat_array::Logical| {
                Number::Integer(i128::from(v.get()))
            })
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            let length =
                usize::try_from(array.numel()).map_err(|_| validation_error("numeric size"))?;
            for index in 0..length {
                if index.is_multiple_of(1024) {
                    context.check_cancelled()?;
                }
                let element = array
                    .element(index)
                    .ok_or_else(|| validation_error("integer element"))?;
                if !test(Number::integer(element.real_component())?) {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Value::Sparse(SparseArrayData::F64(matrix)) => {
            if value.numel().unwrap_or(0) > matrix.values().len() as u64
                && !test(Number::Float(0.0))
            {
                return Ok(false);
            }
            values!(matrix.values().iter(), |v: &f64| Number::Float(*v))
        }
        Value::Sparse(SparseArrayData::Logical(matrix)) => {
            if value.numel().unwrap_or(0) > matrix.values().len() as u64
                && !test(Number::Integer(0))
            {
                return Ok(false);
            }
            values!(matrix.values().iter(), |v: &openmat_array::Logical| {
                Number::Integer(i128::from(v.get()))
            })
        }
        _ => Err(validation_error("real numeric or logical")),
    }
}

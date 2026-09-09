//! Shared argument-size conversion and source-callable validation functions.
use crate::{
    BuiltinRegistry, array_error, core_numeric, core_predicates, core_rearrange, core_set,
    core_shape, expect_argument_count, expect_max_outputs,
};
use openmat_array::{ArrayData, DenseArray, Shape};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistrationError, BuiltinResult,
};
use openmat_value::Value;

#[path = "argument_ranges.rs"]
mod ranges;

pub(super) fn register(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    registry.register("__openmat_validate_argument_size", validate_size)?;
    registry.register("isfield", isfield)?;
    registry.register("__openmat_argument_isfield", isfield)?;
    registry.register("__openmat_order_argument_fields", order_fields)?;
    registry.register("__openmat_repeating_output", repeating_output)?;
    ranges::register(registry)?;
    macro_rules! predicate {
        ($name:literal, $function:path) => {
            registry.register(
                $name,
                |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                    expect_argument_count($name, arguments, 1)?;
                    expect_max_outputs($name, context, 0)?;
                    let values = $function(arguments, context)?;
                    check($name, values.first().is_some_and(all_true))
                },
            )?;
        };
    }
    predicate!("mustBeNumeric", core_predicates::isnumeric_builtin);
    predicate!("mustBeFloat", core_predicates::isfloat_builtin);
    predicate!("mustBeReal", core_predicates::isreal_builtin);
    predicate!("mustBeFinite", core_predicates::isfinite_builtin);
    predicate!("mustBeNonNan", non_nan);
    registry.register(
        "mustBeNonempty",
        |arguments: &[Value], context: &mut BuiltinContext<'_>| {
            expect_argument_count("mustBeNonempty", arguments, 1)?;
            expect_max_outputs("mustBeNonempty", context, 0)?;
            context.check_cancelled()?;
            check(
                "mustBeNonempty",
                matches!(arguments[0], Value::Function(_))
                    || arguments[0].numel().is_some_and(|n| n != 0),
            )
        },
    )?;
    macro_rules! real {
        ($name:literal, $predicate:expr) => {
            registry.register(
                $name,
                |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                    expect_argument_count($name, arguments, 1)?;
                    expect_max_outputs($name, context, 0)?;
                    let values = real_values(&arguments[0], context)?;
                    check($name, values.iter().copied().all($predicate))
                },
            )?;
        };
    }
    real!("mustBePositive", |x: f64| x > 0.0);
    real!("mustBeNonnegative", |x: f64| x >= 0.0);
    real!("mustBeNegative", |x: f64| x < 0.0);
    real!("mustBeNonpositive", |x: f64| x <= 0.0);
    registry.register("mustBeNonzero", nonzero)?;
    real!("mustBeInteger", |x: f64| x.is_finite() && x.fract() == 0.0);
    registry.register(
        "mustBeMember",
        |arguments: &[Value], context: &mut BuiltinContext<'_>| {
            expect_argument_count("mustBeMember", arguments, 2)?;
            expect_max_outputs("mustBeMember", context, 0)?;
            let values = core_set::ismember_builtin(arguments, context)?;
            check("mustBeMember", values.first().is_some_and(all_true))
        },
    )?;
    Ok(())
}

fn repeating_output(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("repeating output", arguments, 1)?;
    expect_max_outputs("repeating output", context, 1)?;
    context.check_cancelled()?;
    match &arguments[0] {
        Value::Cell(_) => Ok(vec![arguments[0].clone()]),
        Value::Nothing => Ok(vec![Value::Cell(
            openmat_value::CellArray::from_values(
                Shape::new([1, 0]).map_err(|e| array_error(&e))?,
                Vec::new(),
            )
            .map_err(|e| BuiltinError::new(BuiltinErrorCategory::Domain, e.to_string()))?,
        )]),
        _ => Err(validation_error("repeating output cell")),
    }
}

fn order_fields(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("argument fields", arguments, 2)?;
    expect_max_outputs("argument fields", context, 1)?;
    context.check_cancelled()?;
    let (Value::Struct(value), Value::Cell(order)) = (&arguments[0], &arguments[1]) else {
        return Err(validation_error("argument fields"));
    };
    let fields = order
        .values()
        .iter()
        .filter_map(text_scalar)
        .filter_map(|name| value.field_index(&name))
        .collect::<Vec<_>>();
    let result = openmat_value::StructArray::from_columns(
        value.shape().clone(),
        fields
            .iter()
            .map(|i| value.field_names()[*i].clone())
            .collect(),
        fields
            .iter()
            .map(|i| value.field_values(*i).unwrap_or(&[]).to_vec())
            .collect(),
    )
    .map_err(|e| BuiltinError::new(BuiltinErrorCategory::Domain, e.to_string()))?;
    Ok(vec![Value::Struct(result)])
}

fn nonzero(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("mustBeNonzero", arguments, 1)?;
    expect_max_outputs("mustBeNonzero", context, 0)?;
    context.check_cancelled()?;
    let valid = match &arguments[0] {
        Value::Complex(value) => value.real != 0.0 || value.imaginary != 0.0,
        Value::Array(ArrayData::ComplexF64(array)) => array
            .as_slice()
            .iter()
            .all(|value| value.re != 0.0 || value.im != 0.0),
        Value::Array(openmat_array::ArrayData::ComplexF32(array)) => array
            .as_slice()
            .iter()
            .all(|value| value.re != 0.0 || value.im != 0.0),
        value => real_values(value, context)?
            .iter()
            .all(|value| *value != 0.0),
    };
    check("mustBeNonzero", valid)
}

fn isfield(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("isfield", arguments, 2)?;
    expect_max_outputs("isfield", context, 1)?;
    context.check_cancelled()?;
    let present = |name: Option<String>| matches!(&arguments[0], Value::Struct(value) if name.is_some_and(|name| value.field_index(&name).is_some()));
    let (shape, names) = match &arguments[1] {
        Value::Cell(array) => (
            Some(array.shape().clone()),
            array.values().iter().map(text_scalar).collect::<Vec<_>>(),
        ),
        Value::String(value) if value.as_scalar().is_none() => {
            let shape = Shape::new(value.dimensions().to_vec()).map_err(|e| array_error(&e))?;
            let length =
                usize::try_from(value.numel()).map_err(|_| validation_error("field name count"))?;
            let names = (0..length)
                .map(|i| {
                    value
                        .element(i)
                        .filter(|v| !v.is_missing())
                        .and_then(|v| String::from_utf16(v.code_units()).ok())
                })
                .collect();
            (Some(shape), names)
        }
        value => (None, vec![text_scalar(value)]),
    };
    let values = names.into_iter().map(present).collect::<Vec<_>>();
    match shape {
        None => Ok(vec![Value::Logical(values[0])]),
        Some(shape) => Ok(vec![Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                shape,
                values
                    .into_iter()
                    .map(openmat_array::Logical::from)
                    .collect(),
            )
            .map_err(|e| array_error(&e))?,
        ))]),
    }
}

fn text_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => value
            .as_scalar()
            .filter(|v| !v.is_missing())
            .and_then(|v| String::from_utf16(v.code_units()).ok()),
        Value::Array(ArrayData::Char(array)) if array.shape().dimensions().first() == Some(&1) => {
            String::from_utf16(&array.as_slice().iter().map(|v| v.get()).collect::<Vec<_>>()).ok()
        }
        _ => None,
    }
}

fn validation_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("value failed {name} validation"),
    )
    .with_identifier(format!("OpenMat:validators:{name}"))
}
fn check(name: &str, valid: bool) -> BuiltinResult {
    if valid {
        Ok(Vec::new())
    } else {
        Err(validation_error(name))
    }
}
fn all_true(value: &Value) -> bool {
    match value {
        Value::Logical(value) => *value,
        Value::Array(ArrayData::Logical(array)) => array.as_slice().iter().all(|value| value.get()),
        _ => false,
    }
}
fn non_nan(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    let values = core_predicates::isnan_builtin(arguments, context)?;
    let any = match values.first() {
        Some(Value::Logical(value)) => *value,
        Some(Value::Array(ArrayData::Logical(array))) => {
            array.as_slice().iter().any(|value| value.get())
        }
        _ => true,
    };
    Ok(vec![Value::Logical(!any)])
}
fn real_values(value: &Value, context: &mut BuiltinContext<'_>) -> Result<Vec<f64>, BuiltinError> {
    if value.dtype().is_none() || value.class_name() == "char" {
        return Err(validation_error("numeric"));
    }
    let values = core_numeric::double_builtin(std::slice::from_ref(value), context)?;
    match values.into_iter().next() {
        Some(Value::Double(value)) => Ok(vec![value]),
        Some(Value::Array(ArrayData::F64(array))) => Ok(array.as_slice().to_vec()),
        _ => Err(validation_error("real numeric")),
    }
}

// Extents are checked as integral, nonnegative values below 2^64 before casting.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn validate_size(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("argument size", arguments, 2)?;
    expect_max_outputs("argument size", context, 1)?;
    context.check_cancelled()?;
    let specifications = real_values(&arguments[1], context)?;
    let actual = if matches!(arguments[0], Value::Function(_)) {
        &[1, 1]
    } else {
        arguments[0]
            .dimensions()
            .ok_or_else(|| validation_error("size"))?
    };
    if specifications.len() < 2
        || specifications.iter().any(|n| {
            !n.is_finite() || *n < -1.0 || n.fract() != 0.0 || *n >= 18_446_744_073_709_551_616.0
        })
    {
        return Err(validation_error("size declaration"));
    }
    let matches = |dimensions: &[u64]| {
        dimensions.len() <= specifications.len()
            && specifications
                .iter()
                .enumerate()
                .all(|(i, n)| *n < 0.0 || dimensions.get(i).copied().unwrap_or(1) == *n as u64)
    };
    if matches(actual) {
        return Ok(vec![arguments[0].clone()]);
    }
    let scalar = arguments[0].numel() == Some(1);
    let mut desired: Vec<u64> = specifications
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if *n < 0.0 {
                actual.get(i).copied().unwrap_or(1)
            } else {
                *n as u64
            }
        })
        .collect();
    if !scalar {
        if arguments[0].numel() == Some(0) && specifications.contains(&-1.0) {
            for (i, n) in specifications.iter().enumerate() {
                if *n < 0.0 {
                    desired[i] = 0;
                }
            }
        } else {
            // Only vector orientation may change automatically, not arbitrary reshaping.
            if actual.len() != 2 || !actual.contains(&1) {
                return Err(validation_error("size"));
            }
            let reversed = [actual[1], actual[0]];
            if !matches(&reversed) {
                return Err(validation_error("size"));
            }
            desired = reversed.to_vec();
        }
    }
    let shape = Shape::new(vec![1, desired.len() as u64]).map_err(|e| array_error(&e))?;
    let sizes = Value::Array(ArrayData::Integer(openmat_array::IntegerArrayData::U64(
        DenseArray::from_vec(shape, desired).map_err(|e| array_error(&e))?,
    )));
    let values = [arguments[0].clone(), sizes];
    if scalar {
        core_rearrange::repmat_builtin(&values, context)
    } else {
        core_shape::reshape_builtin(&values, context)
    }
}

use openmat_array::{ArrayData, CharCodeUnit, DenseArray, Shape};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult, OutputEvent,
};
use openmat_value::{FieldName, StructArray, Value};

use crate::{core_text_conversion, expect_argument_count_range, expect_max_outputs, type_error};

pub(super) fn error_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("error", arguments, 1, usize::MAX)?;
    expect_max_outputs("error", context, 0)?;
    context.check_cancelled()?;

    let first = scalar_text("error", 1, &arguments[0])?;
    let (identifier, message_arguments) = if is_message_identifier(&first) {
        if arguments.len() == 1 {
            return Err(user_error(
                "MATLAB:error:missingMessageArgument",
                "an error identifier must be followed by message text",
            ));
        }
        (first, &arguments[1..])
    } else {
        (String::new(), arguments)
    };
    let message = formatted_message(message_arguments, context)?;
    if message.is_empty() && identifier.is_empty() {
        return Ok(Vec::new());
    }
    Err(user_error(identifier, message))
}

pub(super) fn assert_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("assert", arguments, 1, usize::MAX)?;
    expect_max_outputs("assert", context, 0)?;
    context.check_cancelled()?;

    let condition = scalar_assertion_condition(&arguments[0]).ok_or_else(|| {
        user_error(
            "MATLAB:assertion:LogicalScalar",
            "the assertion condition must be convertible to a scalar logical value",
        )
    })?;
    if condition {
        return Ok(Vec::new());
    }
    if arguments.len() == 1 {
        return Err(user_error("MATLAB:assertion:failed", "Assertion failed."));
    }

    let first = scalar_text("assert", 2, &arguments[1])?;
    let (identifier, message_arguments) = if is_message_identifier(&first) {
        if arguments.len() == 2 {
            return Err(user_error(
                "MATLAB:error:missingMessageArgument",
                "an assertion identifier must be followed by message text",
            ));
        }
        (first, &arguments[2..])
    } else {
        (String::new(), &arguments[1..])
    };
    let message = formatted_message(message_arguments, context)?;
    if message.is_empty() {
        return Err(user_error(
            "MATLAB:assertion:emptyMessageNotAllowed",
            "assertion message text must not be empty",
        ));
    }
    Err(user_error(identifier, message))
}

pub(super) fn warning_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("warning", arguments, 1, usize::MAX)?;
    expect_max_outputs("warning", context, 1)?;
    context.check_cancelled()?;

    let first = scalar_text("warning", 1, &arguments[0])?;
    if matches!(first.to_ascii_lowercase().as_str(), "on" | "off" | "query") && arguments.len() <= 2
    {
        return warning_control(&first, arguments.get(1), context);
    }

    let (identifier, message_arguments) = if is_message_identifier(&first) {
        if arguments.len() == 1 {
            return Err(user_error(
                "MATLAB:warning:missingMessageArgument",
                "a warning identifier must be followed by message text",
            ));
        }
        (first, &arguments[1..])
    } else {
        (String::new(), arguments)
    };
    let message = formatted_message(message_arguments, context)?;
    context.set_last_warning(message.clone(), identifier.clone())?;
    if context.warning_enabled(&identifier)? {
        context.emit(OutputEvent::CommandText(format!("Warning: {message}\n")))?;
    }
    if context.requested_outputs() == 0 {
        Ok(Vec::new())
    } else {
        Ok(vec![char_value("")?])
    }
}

pub(super) fn lastwarn_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("lastwarn", arguments, 0, 2)?;
    expect_max_outputs("lastwarn", context, 2)?;
    context.check_cancelled()?;

    let (previous_message, previous_identifier) = context.last_warning()?;
    let previous_message = previous_message.to_owned();
    let previous_identifier = previous_identifier.to_owned();
    if !arguments.is_empty() {
        let message = scalar_text("lastwarn", 1, &arguments[0])?;
        let identifier = arguments
            .get(1)
            .map(|value| scalar_text("lastwarn", 2, value))
            .transpose()?
            .unwrap_or_default();
        context.set_last_warning(message, identifier)?;
    }

    match context.requested_outputs() {
        0 => Ok(Vec::new()),
        1 => Ok(vec![char_value(&previous_message)?]),
        2 => Ok(vec![
            char_value(&previous_message)?,
            char_value(&previous_identifier)?,
        ]),
        _ => unreachable!("output arity was validated"),
    }
}

fn warning_control(
    command: &str,
    identifier: Option<&Value>,
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let identifier = identifier
        .map(|value| scalar_text("warning", 2, value))
        .transpose()?
        .unwrap_or_else(|| "all".to_owned());
    let command = command.to_ascii_lowercase();
    let state = if command == "query" {
        context.warning_enabled(&identifier)?
    } else {
        context.set_warning_enabled(Some(&identifier), command == "on")?
    };
    if context.requested_outputs() == 0 {
        Ok(Vec::new())
    } else {
        Ok(vec![warning_state_value(state, &identifier)?])
    }
}

fn warning_state_value(enabled: bool, identifier: &str) -> Result<Value, BuiltinError> {
    let fields = ["state", "identifier"]
        .into_iter()
        .map(FieldName::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let shape = Shape::new([1, 1])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    StructArray::from_columns(
        shape,
        fields,
        vec![
            vec![char_value(if enabled { "on" } else { "off" })?],
            vec![char_value(identifier)?],
        ],
    )
    .map(Value::Struct)
    .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn char_value(text: &str) -> Result<Value, BuiltinError> {
    let values = text
        .encode_utf16()
        .map(CharCodeUnit::new)
        .collect::<Vec<_>>();
    let dimensions = if values.is_empty() {
        [0, 0]
    } else {
        [1, u64::try_from(values.len()).unwrap_or(u64::MAX)]
    };
    let shape = Shape::new(dimensions)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn formatted_message(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> Result<String, BuiltinError> {
    let mut output = core_text_conversion::sprintf_builtin(arguments, context)?;
    let value = output.pop().ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "message formatting returned no text value",
        )
    })?;
    scalar_text("message formatting", 1, &value)
}

fn scalar_text(name: &str, position: usize, value: &Value) -> Result<String, BuiltinError> {
    let code_units = match value {
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            array
                .as_slice()
                .iter()
                .map(|value| value.get())
                .collect::<Vec<_>>()
        }
        Value::String(string) => string
            .as_scalar()
            .filter(|value| !value.is_missing())
            .map(|value| value.code_units().to_vec())
            .ok_or_else(|| type_error(name, position, "non-missing text scalar", value))?,
        _ => {
            return Err(type_error(
                name,
                position,
                "char row or string scalar",
                value,
            ));
        }
    };
    Ok(String::from_utf16_lossy(&code_units))
}

fn is_message_identifier(value: &str) -> bool {
    let mut saw_separator = false;
    for part in value.split(':') {
        if part.is_empty() {
            return false;
        }
        let mut characters = part.chars();
        let Some(first) = characters.next() else {
            return false;
        };
        if !(first == '_' || first.is_alphabetic()) {
            return false;
        }
        if !characters.all(|character| character == '_' || character.is_alphanumeric()) {
            return false;
        }
        saw_separator = true;
    }
    saw_separator && value.contains(':')
}

fn scalar_assertion_condition(value: &Value) -> Option<bool> {
    match value {
        Value::Logical(value) => Some(*value),
        Value::Double(value) if !value.is_nan() => Some(*value != 0.0),
        Value::Complex(value) if value.imaginary == 0.0 && !value.real.is_nan() => {
            Some(value.real != 0.0)
        }
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => array
            .as_slice()
            .first()
            .copied()
            .filter(|value| !value.is_nan())
            .map(|value| value != 0.0),
        Value::Array(ArrayData::ComplexF64(array)) if array.numel() == 1 => array
            .as_slice()
            .first()
            .copied()
            .filter(|value| value.im == 0.0 && !value.re.is_nan())
            .map(|value| value.re != 0.0),
        Value::Array(ArrayData::Logical(array)) if array.numel() == 1 => {
            array.as_slice().first().map(|value| value.get())
        }
        Value::Array(ArrayData::Char(array)) if array.numel() == 1 => {
            array.as_slice().first().map(|value| value.get() != 0)
        }
        Value::Array(ArrayData::Integer(array)) if array.numel() == 1 => {
            let value = array.element(0)?;
            let imaginary = value.imaginary_component();
            if imaginary.is_some_and(|value| !value.is_zero()) {
                None
            } else {
                Some(!value.real_component().is_zero())
            }
        }
        Value::Array(ArrayData::F32(array)) if array.numel() == 1 => array
            .as_slice()
            .first()
            .copied()
            .filter(|value| !value.is_nan())
            .map(|value| value != 0.0),
        Value::Array(ArrayData::ComplexF32(array)) if array.numel() == 1 => array
            .as_slice()
            .first()
            .copied()
            .filter(|value| value.im == 0.0 && !value.re.is_nan())
            .map(|value| value.re != 0.0),
        _ => None,
    }
}

fn user_error(identifier: impl Into<String>, message: impl Into<String>) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Other, message).with_identifier(identifier)
}

#[cfg(test)]
mod tests {
    use super::is_message_identifier;

    #[test]
    fn message_identifiers_require_colon_separated_identifier_parts() {
        assert!(is_message_identifier("Probe:Expected"));
        assert!(is_message_identifier("MATLAB:assertion:failed"));
        assert!(!is_message_identifier("plain message"));
        assert!(!is_message_identifier("bad id:message"));
        assert!(!is_message_identifier(":missing"));
        assert!(!is_message_identifier("missing:"));
    }
}

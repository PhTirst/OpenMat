use std::fmt::Write as _;

use openmat_array::{ArrayData, CharCodeUnit, DenseArray, Shape};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult, DisplayFormat, LineSpacing,
    NumericFormat, OutputEvent,
};
use openmat_value::{CellArray, FieldName, StructArray, Value};

use crate::{array_error, expect_argument_count, expect_max_outputs, type_error};

pub(super) fn clc_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("clc", arguments, 0)?;
    expect_max_outputs("clc", context, 0)?;
    context.emit(OutputEvent::CommandWindowClear)?;
    Ok(Vec::new())
}

pub(super) fn pwd_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("pwd", arguments, 0)?;
    expect_max_outputs("pwd", context, 1)?;
    let path = context.working_directory()?.to_string_lossy().into_owned();
    if context.requested_outputs() == 0 {
        context.emit(OutputEvent::CommandText(format!("{path}\n")))?;
        Ok(Vec::new())
    } else {
        Ok(vec![char_row(&path)?])
    }
}

pub(super) fn cd_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.len() > 1 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`cd` expects zero or one input",
        ));
    }
    expect_max_outputs("cd", context, 1)?;
    let previous = context.working_directory()?;
    if let Some(argument) = arguments.first() {
        let path = text_argument("cd", 1, argument)?;
        context.change_working_directory(&path)?;
    }
    let previous = previous.to_string_lossy().into_owned();
    if context.requested_outputs() == 0 {
        if arguments.is_empty() {
            context.emit(OutputEvent::CommandText(format!("{previous}\n")))?;
        }
        Ok(Vec::new())
    } else {
        Ok(vec![char_row(&previous)?])
    }
}

pub(super) fn format_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("format", context, 1)?;
    let mut format = context.display_format()?;
    if !arguments.is_empty() {
        let options = arguments
            .iter()
            .enumerate()
            .map(|(index, value)| {
                text_argument("format", index + 1, value).map(|text| text.to_ascii_lowercase())
            })
            .collect::<Result<Vec<_>, _>>()?;
        apply_format_options(&mut format, &options)?;
        context.set_display_format(format)?;
        context.emit(OutputEvent::DisplayFormatChanged(format))?;
    }
    if context.requested_outputs() == 0 {
        Ok(Vec::new())
    } else {
        Ok(vec![format_value(format)?])
    }
}

pub(super) fn who_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("who", context, 1)?;
    let patterns = text_arguments("who", arguments)?;
    let names = matching_workspace_names(context, &patterns)?;
    if context.requested_outputs() == 0 {
        let text = if names.is_empty() {
            String::new()
        } else {
            format!("{}\n", names.join("  "))
        };
        context.emit(OutputEvent::CommandText(text))?;
        Ok(Vec::new())
    } else {
        Ok(vec![name_cell(&names)?])
    }
}

pub(super) fn whos_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("whos", context, 1)?;
    let patterns = text_arguments("whos", arguments)?;
    let names = matching_workspace_names(context, &patterns)?;
    let details = whos_value(context, &names)?;
    if context.requested_outputs() == 0 {
        context.emit(OutputEvent::CommandText(format_whos_text(context, &names)?))?;
        Ok(Vec::new())
    } else {
        Ok(vec![details])
    }
}

pub(super) fn exist_var_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("exist", arguments, 1)?;
    expect_max_outputs("exist", context, 1)?;
    Ok(vec![Value::Double(f64::from(!matches!(
        arguments[0],
        Value::Nothing
    )))])
}

fn apply_format_options(
    format: &mut DisplayFormat,
    options: &[String],
) -> Result<(), BuiltinError> {
    let normalized = options.iter().map(String::as_str).collect::<Vec<_>>();
    match normalized.as_slice() {
        ["compact"] => format.line_spacing = LineSpacing::Compact,
        ["loose"] => format.line_spacing = LineSpacing::Loose,
        ["default"] => *format = DisplayFormat::default(),
        ["short"] => format.numeric = NumericFormat::Short,
        ["long"] => format.numeric = NumericFormat::Long,
        ["short", "e"] | ["shorte"] => format.numeric = NumericFormat::ShortE,
        ["long", "e"] | ["longe"] => format.numeric = NumericFormat::LongE,
        ["short", "g"] | ["shortg"] => format.numeric = NumericFormat::ShortG,
        ["long", "g"] | ["longg"] => format.numeric = NumericFormat::LongG,
        ["short", "eng"] | ["shorteng"] => format.numeric = NumericFormat::ShortEng,
        ["long", "eng"] | ["longeng"] => format.numeric = NumericFormat::LongEng,
        ["bank"] => format.numeric = NumericFormat::Bank,
        ["hex"] => format.numeric = NumericFormat::Hex,
        ["rat" | "rational"] => format.numeric = NumericFormat::Rational,
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!(
                    "unsupported `format` option sequence `{}`",
                    options.join(" ")
                ),
            ));
        }
    }
    Ok(())
}

fn format_value(format: DisplayFormat) -> Result<Value, BuiltinError> {
    scalar_struct(
        &["NumericFormat", "LineSpacing"],
        vec![
            vec![Value::from(format.numeric.matlab_name())],
            vec![Value::from(format.line_spacing.matlab_name())],
        ],
    )
}

fn text_arguments(name: &str, arguments: &[Value]) -> Result<Vec<String>, BuiltinError> {
    arguments
        .iter()
        .enumerate()
        .map(|(index, value)| text_argument(name, index + 1, value))
        .collect()
}

fn text_argument(name: &str, position: usize, value: &Value) -> Result<String, BuiltinError> {
    let code_units = match value {
        Value::String(string) => {
            let scalar = string.as_scalar().ok_or_else(|| {
                type_error(
                    name,
                    position,
                    "nonmissing string scalar or char row",
                    value,
                )
            })?;
            if scalar.is_missing() {
                return Err(type_error(
                    name,
                    position,
                    "nonmissing string scalar or char row",
                    value,
                ));
            }
            scalar.code_units().to_vec()
        }
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            array
                .as_slice()
                .iter()
                .copied()
                .map(CharCodeUnit::get)
                .collect()
        }
        _ => {
            return Err(type_error(
                name,
                position,
                "nonmissing string scalar or char row",
                value,
            ));
        }
    };
    String::from_utf16(&code_units).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input {position} to `{name}` is not valid UTF-16 text"),
        )
    })
}

fn matching_workspace_names(
    context: &BuiltinContext<'_>,
    patterns: &[String],
) -> Result<Vec<String>, BuiltinError> {
    let workspace = context.workspace()?;
    Ok(workspace
        .iter()
        .map(|(name, _)| name)
        .filter(|name| {
            patterns.is_empty()
                || patterns
                    .iter()
                    .any(|pattern| wildcard_matches(pattern.as_bytes(), name.as_bytes()))
        })
        .map(str::to_owned)
        .collect())
}

fn wildcard_matches(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index, mut star, mut retry) = (0, 0, None, 0);
    while value_index < value.len() {
        if pattern
            .get(pattern_index)
            .is_some_and(|item| *item == b'?' || item.eq_ignore_ascii_case(&value[value_index]))
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern.get(pattern_index) == Some(&b'*') {
            star = Some(pattern_index);
            pattern_index += 1;
            retry = value_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            retry += 1;
            value_index = retry;
        } else {
            return false;
        }
    }
    pattern[pattern_index..].iter().all(|item| *item == b'*')
}

fn name_cell(names: &[String]) -> Result<Value, BuiltinError> {
    let rows = u64::try_from(names.len()).map_err(|_| size_error("who result is too large"))?;
    let shape = Shape::new([rows, 1]).map_err(|error| array_error(&error))?;
    let values = names
        .iter()
        .map(|name| char_row(name))
        .collect::<Result<Vec<_>, _>>()?;
    CellArray::from_values(shape, values)
        .map(Value::Cell)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn whos_value(context: &BuiltinContext<'_>, names: &[String]) -> Result<Value, BuiltinError> {
    const FIELDS: [&str; 9] = [
        "name",
        "size",
        "bytes",
        "class",
        "global",
        "sparse",
        "complex",
        "nesting",
        "persistent",
    ];
    let workspace = context.workspace()?;
    let mut columns = (0..FIELDS.len())
        .map(|_| Vec::with_capacity(names.len()))
        .collect::<Vec<_>>();
    for name in names {
        let value = workspace.get(name).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                "workspace changed while constructing `whos` output",
            )
        })?;
        columns[0].push(char_row(name)?);
        columns[1].push(size_row(value.dimensions().unwrap_or(&[]))?);
        columns[2].push(Value::Double(estimated_bytes(value)));
        columns[3].push(char_row(value.class_name())?);
        columns[4].push(Value::Logical(false));
        columns[5].push(Value::Logical(false));
        columns[6].push(Value::Logical(value.is_complex_numeric()));
        columns[7].push(empty_struct()?);
        columns[8].push(Value::Logical(false));
    }
    let rows = u64::try_from(names.len()).map_err(|_| size_error("whos result is too large"))?;
    let shape = Shape::new([rows, 1]).map_err(|error| array_error(&error))?;
    let fields = FIELDS
        .into_iter()
        .map(field_name)
        .collect::<Result<Vec<_>, _>>()?;
    StructArray::from_columns(shape, fields, columns)
        .map(Value::Struct)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn format_whos_text(
    context: &BuiltinContext<'_>,
    names: &[String],
) -> Result<String, BuiltinError> {
    let workspace = context.workspace()?;
    let mut text = String::from("  Name      Size        Bytes  Class\n\n");
    for name in names {
        let value = workspace.get(name).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                "workspace snapshot is inconsistent",
            )
        })?;
        let dimensions = value.dimensions().unwrap_or(&[]);
        let size = dimensions
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join("x");
        writeln!(
            &mut text,
            "  {name:<8}  {size:<10}  {:>5}  {}",
            estimated_bytes(value),
            value.class_name()
        )
        .expect("writing to an owned String cannot fail");
    }
    Ok(text)
}

fn scalar_struct(fields: &[&str], columns: Vec<Vec<Value>>) -> Result<Value, BuiltinError> {
    let fields = fields
        .iter()
        .map(|name| field_name(name))
        .collect::<Result<Vec<_>, _>>()?;
    let shape = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    StructArray::from_columns(shape, fields, columns)
        .map(Value::Struct)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn empty_struct() -> Result<Value, BuiltinError> {
    let shape = Shape::new([0, 0]).map_err(|error| array_error(&error))?;
    StructArray::empty(shape, Vec::new())
        .map(Value::Struct)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn field_name(name: &str) -> Result<FieldName, BuiltinError> {
    FieldName::new(name)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn char_row(text: &str) -> Result<Value, BuiltinError> {
    let values = text
        .encode_utf16()
        .map(CharCodeUnit::new)
        .collect::<Vec<_>>();
    let columns = u64::try_from(values.len()).map_err(|_| size_error("text is too large"))?;
    let shape = Shape::new([1, columns]).map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, values)
        .map(|array| Value::Array(ArrayData::Char(array)))
        .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_precision_loss)]
fn size_row(dimensions: &[u64]) -> Result<Value, BuiltinError> {
    let values = dimensions
        .iter()
        .map(|value| *value as f64)
        .collect::<Vec<_>>();
    let columns = u64::try_from(values.len()).map_err(|_| size_error("shape is too large"))?;
    let shape = Shape::new([1, columns]).map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, values)
        .map(|array| Value::Array(ArrayData::F64(array)))
        .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_precision_loss)]
fn estimated_bytes(value: &Value) -> f64 {
    let elements = value.numel().unwrap_or(0);
    let width = value
        .dtype()
        .map_or(0, openmat_array::DType::element_width_bytes);
    elements.saturating_mul(width as u64) as f64
}

fn size_error(message: &str) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Domain, message)
}

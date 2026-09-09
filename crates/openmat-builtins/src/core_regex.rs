use openmat_array::{ArrayData, CharCodeUnit, DenseArray, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_text::{RegexErrorKind, RegexMatch, RegexMatches, RegexOptions, Utf16Range};
use openmat_value::{
    CellArray, FieldName, StringArray, StringElement, StringValue, StructArray, Value,
};

use crate::{aggregate_error, array_error, type_error};

const MAX_REGEX_OUTPUTS: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputKind {
    Start,
    End,
    TokenExtents,
    Match,
    Tokens,
    Names,
    Split,
}

const DEFAULT_OUTPUTS: [OutputKind; MAX_REGEX_OUTPUTS] = [
    OutputKind::Start,
    OutputKind::End,
    OutputKind::TokenExtents,
    OutputKind::Match,
    OutputKind::Tokens,
    OutputKind::Names,
    OutputKind::Split,
];

#[derive(Clone, Debug)]
struct RegexpOptions {
    provider: RegexOptions,
    once: bool,
    empty_matches: bool,
    outputs: Vec<OutputKind>,
}

impl RegexpOptions {
    fn new(case_insensitive: bool) -> Self {
        Self {
            provider: RegexOptions {
                case_insensitive,
                ..RegexOptions::default()
            },
            once: false,
            empty_matches: false,
            outputs: Vec::new(),
        }
    }

    fn parse(
        name: &str,
        values: &[Value],
        first_position: usize,
        case_insensitive: bool,
    ) -> Result<Self, BuiltinError> {
        let mut options = Self::new(case_insensitive);
        for (offset, value) in values.iter().enumerate() {
            let option = scalar_text(name, offset + first_position, value)?;
            let option = ascii_option(name, &option)?;
            match option.as_str() {
                "once" => options.once = true,
                "ignorecase" => options.provider.case_insensitive = true,
                "matchcase" => options.provider.case_insensitive = false,
                "dotall" => options.provider.dot_matches_new_line = true,
                "dotexceptnewline" => options.provider.dot_matches_new_line = false,
                "lineanchors" => options.provider.multi_line = true,
                "stringanchors" => options.provider.multi_line = false,
                "freespacing" => options.provider.ignore_whitespace = true,
                "literalspacing" => options.provider.ignore_whitespace = false,
                "emptymatch" => options.empty_matches = true,
                "noemptymatch" => options.empty_matches = false,
                "start" => options.outputs.push(OutputKind::Start),
                "end" => options.outputs.push(OutputKind::End),
                "tokenextents" => options.outputs.push(OutputKind::TokenExtents),
                "match" => options.outputs.push(OutputKind::Match),
                "tokens" => options.outputs.push(OutputKind::Tokens),
                "names" => options.outputs.push(OutputKind::Names),
                "split" => options.outputs.push(OutputKind::Split),
                _ => {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("`{name}` does not support option '{option}'"),
                    ));
                }
            }
        }
        Ok(options)
    }

    fn selected_outputs(&self, requested: usize) -> Result<Vec<OutputKind>, BuiltinError> {
        if requested > MAX_REGEX_OUTPUTS {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::ArgumentCount,
                format!("regular expression functions support at most {MAX_REGEX_OUTPUTS} outputs"),
            ));
        }
        let available = if self.outputs.is_empty() {
            DEFAULT_OUTPUTS.as_slice()
        } else {
            &self.outputs
        };
        if requested > available.len() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::ArgumentCount,
                format!(
                    "regular expression call selected {} outputs but {requested} were requested",
                    available.len()
                ),
            ));
        }
        Ok(available[..requested].to_vec())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextInputKind {
    Char,
    StringScalar,
    StringArray,
    Cell,
}

struct TextInput {
    kind: TextInputKind,
    shape: Shape,
    elements: Vec<Option<Vec<u16>>>,
}

struct ScalarResult<'a> {
    input: &'a [u16],
    capture_names: Vec<Option<String>>,
    matches: Vec<RegexMatch>,
    once: bool,
    string_text: bool,
}

pub(super) fn regexp_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    regexp_impl("regexp", arguments, context, false)
}

pub(super) fn regexpi_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    regexp_impl("regexpi", arguments, context, true)
}

fn regexp_impl(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    case_insensitive: bool,
) -> BuiltinResult {
    if arguments.len() < 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("built-in `{name}` expects at least 2 inputs"),
        ));
    }
    context.check_cancelled()?;
    let input = TextInput::new(name, 1, &arguments[0])?;
    let pattern = scalar_text(name, 2, &arguments[1])?;
    let options = RegexpOptions::parse(name, &arguments[2..], 3, case_insensitive)?;
    let outputs = options.selected_outputs(context.requested_outputs())?;
    if outputs.is_empty() {
        return Ok(Vec::new());
    }

    let mut columns = (0..outputs.len())
        .map(|_| Vec::with_capacity(input.elements.len()))
        .collect::<Vec<_>>();
    for (index, element) in input.elements.iter().enumerate() {
        context.check_cancelled()?;
        let element = element.as_deref().unwrap_or(&[]);
        let result = execute_regex(
            context,
            &pattern,
            element,
            &options,
            matches!(
                input.kind,
                TextInputKind::StringScalar | TextInputKind::StringArray
            ),
        )?;
        for (column, output) in outputs.iter().copied().enumerate() {
            columns[column].push(scalar_output(&result, output)?);
        }
        if index.is_multiple_of(4_096) {
            context.check_cancelled()?;
        }
    }
    context.check_cancelled()?;
    if matches!(
        input.kind,
        TextInputKind::Char | TextInputKind::StringScalar
    ) {
        Ok(columns
            .into_iter()
            .map(|mut values| values.remove(0))
            .collect())
    } else {
        columns
            .into_iter()
            .map(|values| cell_value(input.shape.clone(), values))
            .collect()
    }
}

pub(super) fn regexprep_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "built-in `regexprep` expects at least 3 inputs",
        ));
    }
    if context.requested_outputs() > 1 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "built-in `regexprep` supports at most 1 output",
        ));
    }
    context.check_cancelled()?;
    let input = TextInput::new("regexprep", 1, &arguments[0])?;
    let pattern = scalar_text("regexprep", 2, &arguments[1])?;
    let replacement = scalar_text("regexprep", 3, &arguments[2])?;
    let options = RegexpOptions::parse("regexprep", &arguments[3..], 4, false)?;
    if !options.outputs.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`regexprep` does not accept regexp output-selection options",
        ));
    }
    let mut replaced = Vec::with_capacity(input.elements.len());
    for element in &input.elements {
        context.check_cancelled()?;
        let Some(element) = element else {
            replaced.push(None);
            continue;
        };
        let result = execute_regex(
            context,
            &pattern,
            element,
            &options,
            matches!(
                input.kind,
                TextInputKind::StringScalar | TextInputKind::StringArray
            ),
        )?;
        replaced.push(Some(expand_replacement(&result, &replacement)?));
    }
    context.check_cancelled()?;
    if context.requested_outputs() == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![replacement_output(input.kind, input.shape, replaced)?])
}

impl TextInput {
    fn new(name: &str, position: usize, value: &Value) -> Result<Self, BuiltinError> {
        match value {
            Value::Array(ArrayData::Char(array)) => Ok(Self {
                kind: TextInputKind::Char,
                shape: Shape::new([1, 1]).map_err(|error| array_error(&error))?,
                elements: vec![Some(char_row_units(name, position, array)?)],
            }),
            Value::String(StringValue::Scalar(element)) => Ok(Self {
                kind: TextInputKind::StringScalar,
                shape: Shape::new([1, 1]).map_err(|error| array_error(&error))?,
                elements: vec![string_units(element)],
            }),
            Value::String(StringValue::Array(array)) => Ok(Self {
                kind: TextInputKind::StringArray,
                shape: array.shape().clone(),
                elements: array.as_slice().iter().map(string_units).collect(),
            }),
            Value::Cell(cell) => {
                let mut elements = Vec::with_capacity(cell.values().len());
                for value in cell.values() {
                    let Value::Array(ArrayData::Char(array)) = value else {
                        return Err(type_error(
                            name,
                            position,
                            "cell array containing character row vectors",
                            value,
                        ));
                    };
                    elements.push(Some(char_row_units(name, position, array)?));
                }
                Ok(Self {
                    kind: TextInputKind::Cell,
                    shape: cell.shape().clone(),
                    elements,
                })
            }
            value => Err(type_error(
                name,
                position,
                "character row vector, string array, or cell array of character vectors",
                value,
            )),
        }
    }
}

fn string_units(value: &StringElement) -> Option<Vec<u16>> {
    (!value.is_missing()).then(|| value.code_units().to_vec())
}

fn scalar_text(name: &str, position: usize, value: &Value) -> Result<Vec<u16>, BuiltinError> {
    match value {
        Value::Array(ArrayData::Char(array)) => char_row_units(name, position, array),
        Value::String(value) => {
            let Some(element) = value.as_scalar() else {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Type,
                    format!(
                        "input {position} to `{name}` must be a character row vector or nonmissing string scalar"
                    ),
                ));
            };
            if element.is_missing() {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Type,
                    format!(
                        "input {position} to `{name}` must be a character row vector or nonmissing string scalar"
                    ),
                ));
            }
            Ok(element.code_units().to_vec())
        }
        value => Err(type_error(
            name,
            position,
            "character row vector or nonmissing string scalar",
            value,
        )),
    }
}

fn char_row_units(
    name: &str,
    position: usize,
    value: &DenseArray<CharCodeUnit>,
) -> Result<Vec<u16>, BuiltinError> {
    if value.shape().dimensions() == [0, 0]
        || (value.shape().ndims() == 2 && value.shape().extent(0) == 1)
    {
        Ok(value.as_slice().iter().map(|value| value.get()).collect())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input {position} to `{name}` must be a character row vector"),
        ))
    }
}

fn ascii_option(name: &str, value: &[u16]) -> Result<String, BuiltinError> {
    let mut output = String::with_capacity(value.len());
    for unit in value {
        let byte = u8::try_from(*unit).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` option names must contain ASCII characters"),
            )
        })?;
        output.push(char::from(byte).to_ascii_lowercase());
    }
    Ok(output)
}

fn execute_regex<'a>(
    context: &BuiltinContext<'_>,
    pattern: &[u16],
    input: &'a [u16],
    options: &RegexpOptions,
    string_text: bool,
) -> Result<ScalarResult<'a>, BuiltinError> {
    let RegexMatches {
        capture_names,
        matches,
    } = context
        .regex_provider()
        .find_all(pattern, input, options.provider)
        .map_err(regex_error)?;
    let mut matches = matches
        .into_iter()
        .filter(|value| options.empty_matches || value.full.start != value.full.end)
        .collect::<Vec<_>>();
    if options.once {
        matches.truncate(1);
    }
    Ok(ScalarResult {
        input,
        capture_names,
        matches,
        once: options.once,
        string_text,
    })
}

fn regex_error(error: openmat_text::RegexError) -> BuiltinError {
    let category = match error.kind {
        RegexErrorKind::InvalidPatternText
        | RegexErrorKind::InvalidInputText
        | RegexErrorKind::InvalidPattern => BuiltinErrorCategory::Domain,
        RegexErrorKind::BacktrackLimit => BuiltinErrorCategory::Other,
    };
    BuiltinError::new(category, error.to_string())
}

fn scalar_output(result: &ScalarResult<'_>, output: OutputKind) -> Result<Value, BuiltinError> {
    match output {
        OutputKind::Start => numeric_row(
            result
                .matches
                .iter()
                .map(|value| index_as_double(value.full.start.saturating_add(1)))
                .collect(),
        ),
        OutputKind::End => numeric_row(
            result
                .matches
                .iter()
                .map(|value| index_as_double(value.full.end))
                .collect(),
        ),
        OutputKind::TokenExtents => token_extents_output(result),
        OutputKind::Match => match_output(result),
        OutputKind::Tokens => tokens_output(result),
        OutputKind::Names => names_output(result),
        OutputKind::Split => split_output(result),
    }
}

#[allow(clippy::cast_precision_loss)]
fn index_as_double(value: u64) -> f64 {
    value as f64
}

fn numeric_row(values: Vec<f64>) -> Result<Value, BuiltinError> {
    if values.is_empty() {
        return Ok(Value::empty_double());
    }
    if values.len() == 1 {
        return Ok(Value::Double(values[0]));
    }
    let columns = u64::try_from(values.len()).map_err(|_| output_too_large())?;
    DenseArray::from_vec(
        Shape::new([1, columns]).map_err(|error| array_error(&error))?,
        values,
    )
    .map(ArrayData::F64)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn token_extents_output(result: &ScalarResult<'_>) -> Result<Value, BuiltinError> {
    if result.once {
        return result.matches.first().map_or_else(
            || Ok(Value::empty_double()),
            |value| token_extent_matrix(&value.captures),
        );
    }
    let values = result
        .matches
        .iter()
        .map(|value| token_extent_matrix(&value.captures))
        .collect::<Result<Vec<_>, _>>()?;
    cell_row(values)
}

fn token_extent_matrix(captures: &[Option<Utf16Range>]) -> Result<Value, BuiltinError> {
    if captures.is_empty() {
        return Ok(Value::empty_double());
    }
    let mut values = Vec::with_capacity(captures.len().saturating_mul(2));
    values.extend(captures.iter().map(|capture| {
        capture.map_or(f64::NAN, |range| {
            index_as_double(range.start.saturating_add(1))
        })
    }));
    values.extend(
        captures
            .iter()
            .map(|capture| capture.map_or(f64::NAN, |range| index_as_double(range.end))),
    );
    let rows = u64::try_from(captures.len()).map_err(|_| output_too_large())?;
    DenseArray::from_vec(
        Shape::new([rows, 2]).map_err(|error| array_error(&error))?,
        values,
    )
    .map(ArrayData::F64)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn match_output(result: &ScalarResult<'_>) -> Result<Value, BuiltinError> {
    if result.string_text {
        if result.once {
            return result.matches.first().map_or_else(
                || string_row(Vec::new()),
                |value| {
                    Ok(Value::String(StringValue::scalar(
                        slice_range(result.input, value.full)?.to_vec(),
                    )))
                },
            );
        }
        return string_row(
            result
                .matches
                .iter()
                .map(|value| slice_range(result.input, value.full).map(<[u16]>::to_vec))
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    if result.once {
        return result.matches.first().map_or_else(
            || char_value(Vec::new(), false),
            |value| char_value(slice_range(result.input, value.full)?.to_vec(), false),
        );
    }
    cell_row(
        result
            .matches
            .iter()
            .map(|value| char_value(slice_range(result.input, value.full)?.to_vec(), false))
            .collect::<Result<Vec<_>, BuiltinError>>()?,
    )
}

fn tokens_output(result: &ScalarResult<'_>) -> Result<Value, BuiltinError> {
    if result.once {
        return result.matches.first().map_or_else(
            || empty_cell([0, 0]),
            |value| {
                if result.string_text {
                    capture_string_row(result.input, &value.captures)
                } else {
                    capture_text_cell(result.input, &value.captures)
                }
            },
        );
    }
    let matches = result
        .matches
        .iter()
        .map(|value| {
            if result.string_text {
                capture_string_row(result.input, &value.captures)
            } else {
                capture_text_cell(result.input, &value.captures)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    cell_row(matches)
}

fn capture_text_cell(
    input: &[u16],
    captures: &[Option<Utf16Range>],
) -> Result<Value, BuiltinError> {
    let values = captures
        .iter()
        .map(|capture| {
            capture.map_or_else(
                || char_value(Vec::new(), false),
                |range| char_value(slice_range(input, range)?.to_vec(), false),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    cell_row(values)
}

fn capture_string_row(
    input: &[u16],
    captures: &[Option<Utf16Range>],
) -> Result<Value, BuiltinError> {
    string_row(
        captures
            .iter()
            .map(|capture| {
                capture.map_or_else(
                    || Ok(Vec::new()),
                    |range| slice_range(input, range).map(<[u16]>::to_vec),
                )
            })
            .collect::<Result<Vec<_>, BuiltinError>>()?,
    )
}

fn names_output(result: &ScalarResult<'_>) -> Result<Value, BuiltinError> {
    let named = result
        .capture_names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| name.as_ref().map(|name| (index, name)))
        .collect::<Vec<_>>();
    let fields = named
        .iter()
        .map(|(_, name)| {
            FieldName::new(name.as_str()).map_err(|error| aggregate_error("regexp", &error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let selected = if result.once {
        result.matches.iter().take(1).collect::<Vec<_>>()
    } else {
        result.matches.iter().collect::<Vec<_>>()
    };
    let shape = if selected.is_empty() {
        Shape::new([0, 0]).map_err(|error| array_error(&error))?
    } else {
        Shape::new([
            1,
            u64::try_from(selected.len()).map_err(|_| output_too_large())?,
        ])
        .map_err(|error| array_error(&error))?
    };
    let mut columns = Vec::with_capacity(named.len());
    for (capture, _) in named {
        columns.push(
            selected
                .iter()
                .map(|value| {
                    value.captures.get(capture).copied().flatten().map_or_else(
                        || text_scalar_value(Vec::new(), result.string_text),
                        |range| {
                            text_scalar_value(
                                slice_range(result.input, range)?.to_vec(),
                                result.string_text,
                            )
                        },
                    )
                })
                .collect::<Result<Vec<_>, BuiltinError>>()?,
        );
    }
    StructArray::from_columns(shape, fields, columns)
        .map(Value::Struct)
        .map_err(|error| aggregate_error("regexp", &error))
}

fn split_output(result: &ScalarResult<'_>) -> Result<Value, BuiltinError> {
    let mut pieces = Vec::with_capacity(result.matches.len().saturating_add(1));
    let mut cursor = 0_u64;
    for value in &result.matches {
        pieces.push(
            slice_range(
                result.input,
                Utf16Range {
                    start: cursor,
                    end: value.full.start,
                },
            )?
            .to_vec(),
        );
        cursor = value.full.end;
    }
    pieces.push(
        slice_range(
            result.input,
            Utf16Range {
                start: cursor,
                end: u64::try_from(result.input.len()).map_err(|_| output_too_large())?,
            },
        )?
        .to_vec(),
    );
    if result.string_text {
        string_row(pieces)
    } else {
        cell_row(
            pieces
                .into_iter()
                .map(|value| char_value(value, false))
                .collect::<Result<Vec<_>, _>>()?,
        )
    }
}

fn expand_replacement(
    result: &ScalarResult<'_>,
    replacement: &[u16],
) -> Result<Vec<u16>, BuiltinError> {
    let mut output = Vec::new();
    let mut cursor = 0_u64;
    for matched in &result.matches {
        output.extend_from_slice(slice_range(
            result.input,
            Utf16Range {
                start: cursor,
                end: matched.full.start,
            },
        )?);
        expand_template(
            &mut output,
            replacement,
            result.input,
            matched,
            &result.capture_names,
        )?;
        cursor = matched.full.end;
    }
    output.extend_from_slice(slice_range(
        result.input,
        Utf16Range {
            start: cursor,
            end: u64::try_from(result.input.len()).map_err(|_| output_too_large())?,
        },
    )?);
    Ok(output)
}

fn expand_template(
    output: &mut Vec<u16>,
    template: &[u16],
    input: &[u16],
    matched: &RegexMatch,
    capture_names: &[Option<String>],
) -> Result<(), BuiltinError> {
    let mut index = 0_usize;
    while index < template.len() {
        if template[index] != u16::from(b'$') {
            output.push(template[index]);
            index += 1;
            continue;
        }
        if template.get(index + 1) == Some(&u16::from(b'$')) {
            output.push(u16::from(b'$'));
            index += 2;
            continue;
        }
        if let Some((capture, consumed)) = numeric_template_capture(&template[index + 1..]) {
            append_capture(output, input, matched, capture)?;
            index += consumed + 1;
            continue;
        }
        if let Some((name, consumed)) = named_template_capture(&template[index + 1..]) {
            if let Some(capture) = capture_names
                .iter()
                .position(|candidate| candidate.as_deref() == Some(name.as_str()))
            {
                append_capture(output, input, matched, capture + 1)?;
            }
            index += consumed + 1;
            continue;
        }
        output.push(u16::from(b'$'));
        index += 1;
    }
    Ok(())
}

fn numeric_template_capture(value: &[u16]) -> Option<(usize, usize)> {
    let mut digits = 0_usize;
    let mut capture = 0_usize;
    for unit in value.iter().take(2) {
        let digit = u8::try_from(*unit).ok()?.checked_sub(b'0')?;
        if digit > 9 {
            break;
        }
        capture = capture.checked_mul(10)?.checked_add(usize::from(digit))?;
        digits += 1;
    }
    (digits > 0).then_some((capture, digits))
}

fn named_template_capture(value: &[u16]) -> Option<(String, usize)> {
    let (open, close) = match value.first().copied() {
        Some(value) if value == u16::from(b'<') => (u16::from(b'<'), u16::from(b'>')),
        Some(value) if value == u16::from(b'{') => (u16::from(b'{'), u16::from(b'}')),
        _ => return None,
    };
    debug_assert_eq!(value.first(), Some(&open));
    let end = value.iter().position(|unit| *unit == close)?;
    let name = String::from_utf16(&value[1..end]).ok()?;
    Some((name, end + 1))
}

fn append_capture(
    output: &mut Vec<u16>,
    input: &[u16],
    matched: &RegexMatch,
    capture: usize,
) -> Result<(), BuiltinError> {
    let range = if capture == 0 {
        Some(matched.full)
    } else {
        matched.captures.get(capture - 1).copied().flatten()
    };
    if let Some(range) = range {
        output.extend_from_slice(slice_range(input, range)?);
    }
    Ok(())
}

fn replacement_output(
    kind: TextInputKind,
    shape: Shape,
    values: Vec<Option<Vec<u16>>>,
) -> Result<Value, BuiltinError> {
    match kind {
        TextInputKind::Char => char_value(
            values.into_iter().next().flatten().unwrap_or_default(),
            false,
        ),
        TextInputKind::StringScalar => Ok(Value::String(StringValue::scalar(
            values
                .into_iter()
                .next()
                .flatten()
                .map_or_else(StringElement::missing, StringElement::from_code_units),
        ))),
        TextInputKind::StringArray => StringArray::from_elements(
            shape,
            values
                .into_iter()
                .map(|value| {
                    value.map_or_else(StringElement::missing, StringElement::from_code_units)
                })
                .collect(),
        )
        .map(StringValue::array)
        .map(Value::String)
        .map_err(|error| array_error(&error)),
        TextInputKind::Cell => {
            let values = values
                .into_iter()
                .map(|value| char_value(value.unwrap_or_default(), false))
                .collect::<Result<Vec<_>, _>>()?;
            cell_value(shape, values)
        }
    }
}

fn char_value(values: Vec<u16>, row_empty: bool) -> Result<Value, BuiltinError> {
    let columns = u64::try_from(values.len()).map_err(|_| output_too_large())?;
    let dimensions = if columns == 0 && !row_empty {
        [0, 0]
    } else {
        [1, columns]
    };
    DenseArray::from_vec(
        Shape::new(dimensions).map_err(|error| array_error(&error))?,
        values.into_iter().map(CharCodeUnit::new).collect(),
    )
    .map(ArrayData::Char)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn cell_row(values: Vec<Value>) -> Result<Value, BuiltinError> {
    if values.is_empty() {
        return empty_cell([0, 0]);
    }
    cell_value(
        Shape::new([
            1,
            u64::try_from(values.len()).map_err(|_| output_too_large())?,
        ])
        .map_err(|error| array_error(&error))?,
        values,
    )
}

fn string_row(values: Vec<Vec<u16>>) -> Result<Value, BuiltinError> {
    let columns = u64::try_from(values.len()).map_err(|_| output_too_large())?;
    StringArray::from_elements(
        Shape::new([1, columns]).map_err(|error| array_error(&error))?,
        values
            .into_iter()
            .map(StringElement::from_code_units)
            .collect(),
    )
    .map(StringValue::array)
    .map(Value::String)
    .map_err(|error| array_error(&error))
}

fn text_scalar_value(values: Vec<u16>, string: bool) -> Result<Value, BuiltinError> {
    if string {
        Ok(Value::String(StringValue::scalar(values)))
    } else {
        char_value(values, false)
    }
}

fn empty_cell(dimensions: [u64; 2]) -> Result<Value, BuiltinError> {
    cell_value(
        Shape::new(dimensions).map_err(|error| array_error(&error))?,
        Vec::new(),
    )
}

fn cell_value(shape: Shape, values: Vec<Value>) -> Result<Value, BuiltinError> {
    CellArray::from_values(shape, values)
        .map(Value::Cell)
        .map_err(|error| aggregate_error("regexp", &error))
}

fn slice_range(input: &[u16], range: Utf16Range) -> Result<&[u16], BuiltinError> {
    let start = usize::try_from(range.start).map_err(|_| invalid_provider_range())?;
    let end = usize::try_from(range.end).map_err(|_| invalid_provider_range())?;
    input.get(start..end).ok_or_else(invalid_provider_range)
}

fn invalid_provider_range() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Other,
        "regular-expression provider returned an invalid UTF-16 range",
    )
}

fn output_too_large() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "regular-expression output is too large for this host",
    )
}

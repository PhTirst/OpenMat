use std::char::decode_utf16;

use openmat_array::{ArrayData, CharCodeUnit, DenseArray, Logical, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{CellArray, StringArray, StringElement, StringValue, Value};

use crate::{aggregate_error, array_error, expect_argument_count, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextKind {
    Char,
    StringScalar,
    StringArray,
    Cell,
}

struct TextList {
    kind: TextKind,
    shape: Shape,
    elements: Vec<Option<Vec<u16>>>,
}

impl TextList {
    fn new(name: &str, position: usize, value: &Value) -> Result<Self, BuiltinError> {
        match value {
            Value::Array(ArrayData::Char(array)) => Ok(Self {
                kind: TextKind::Char,
                shape: scalar_shape()?,
                elements: vec![Some(char_row_units(name, position, array)?)],
            }),
            Value::String(StringValue::Scalar(element)) => Ok(Self {
                kind: TextKind::StringScalar,
                shape: scalar_shape()?,
                elements: vec![string_units(element)],
            }),
            Value::String(StringValue::Array(array)) => Ok(Self {
                kind: TextKind::StringArray,
                shape: array.shape().clone(),
                elements: array.as_slice().iter().map(string_units).collect(),
            }),
            Value::Cell(cell) => {
                let mut elements = Vec::with_capacity(cell.values().len());
                for item in cell.values() {
                    let Value::Array(ArrayData::Char(array)) = item else {
                        return Err(type_error(
                            name,
                            position,
                            "cell array containing character row vectors",
                            item,
                        ));
                    };
                    elements.push(Some(char_row_units(name, position, array)?));
                }
                Ok(Self {
                    kind: TextKind::Cell,
                    shape: cell.shape().clone(),
                    elements,
                })
            }
            other => Err(type_error(
                name,
                position,
                "character row vector, string array, or cell array of character vectors",
                other,
            )),
        }
    }

    fn len(&self) -> usize {
        self.elements.len()
    }
    fn is_scalar(&self) -> bool {
        self.len() == 1
    }
}

pub(super) fn strcmp_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    compare_builtin("strcmp", arguments, context, None, false)
}

pub(super) fn strcmpi_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    compare_builtin("strcmpi", arguments, context, None, true)
}

pub(super) fn strncmp_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let count = prefix_count("strncmp", arguments)?;
    compare_builtin("strncmp", &arguments[..2], context, Some(count), false)
}

pub(super) fn strncmpi_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let count = prefix_count("strncmpi", arguments)?;
    compare_builtin("strncmpi", &arguments[..2], context, Some(count), true)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn prefix_count(name: &str, arguments: &[Value]) -> Result<usize, BuiltinError> {
    expect_argument_count(name, arguments, 3)?;
    let value = scalar_f64(&arguments[2]).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input 3 to `{name}` must be a real integer scalar"),
        )
    })?;
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input 3 to `{name}` must be a finite integer scalar"),
        ));
    }
    if value <= 0.0 {
        Ok(0)
    } else {
        Ok(value as usize)
    }
}

fn compare_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    prefix: Option<usize>,
    ignore_case: bool,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 2)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    if let (Value::Array(ArrayData::Char(left)), Value::Array(ArrayData::Char(right))) =
        (&arguments[0], &arguments[1])
    {
        let left_units = units_of(left).collect::<Vec<_>>();
        let right_units = units_of(right).collect::<Vec<_>>();
        let equal = if prefix.is_none() {
            left.shape() == right.shape()
                && compare_units(&left_units, &right_units, prefix, ignore_case)
        } else {
            compare_units(&left_units, &right_units, prefix, ignore_case)
        };
        return Ok(vec![Value::Logical(equal)]);
    }
    let left = compare_list(name, 1, &arguments[0])?;
    let right = compare_list(name, 2, &arguments[1])?;
    let (shape, force_array) = comparison_shape(name, &left, &right)?;
    let length = checked_len(shape.numel(), name)?;
    let mut values = reserved_vec(length, name, "logical elements")?;
    for index in 0..length {
        check_cancelled_at(context, index)?;
        let left_value = &left.elements[if left.is_scalar() { 0 } else { index }];
        let right_value = &right.elements[if right.is_scalar() { 0 } else { index }];
        values.push(Logical::from(match (left_value, right_value) {
            (Some(left), Some(right)) => compare_units(left, right, prefix, ignore_case),
            _ => false,
        }));
    }
    context.check_cancelled()?;
    if length == 1 && !force_array {
        Ok(vec![Value::Logical(values[0].get())])
    } else {
        logical_result(shape, values)
    }
}

fn compare_list(name: &str, position: usize, value: &Value) -> Result<TextList, BuiltinError> {
    match value {
        Value::Array(ArrayData::Char(array)) => Ok(TextList {
            kind: TextKind::Char,
            shape: scalar_shape()?,
            elements: vec![Some(char_row_units(name, position, array)?)],
        }),
        Value::String(StringValue::Scalar(element)) => Ok(TextList {
            kind: TextKind::StringScalar,
            shape: scalar_shape()?,
            elements: vec![string_units(element)],
        }),
        Value::String(StringValue::Array(array)) => Ok(TextList {
            kind: TextKind::StringArray,
            shape: array.shape().clone(),
            elements: array.as_slice().iter().map(string_units).collect(),
        }),
        Value::Cell(cell) => Ok(TextList {
            kind: TextKind::Cell,
            shape: cell.shape().clone(),
            elements: cell
                .values()
                .iter()
                .map(|item| match item {
                    Value::Array(ArrayData::Char(array)) => {
                        char_row_units(name, position, array).ok()
                    }
                    _ => None,
                })
                .collect(),
        }),
        other => Err(type_error(
            name,
            position,
            "char array, string scalar/array, or cell array",
            other,
        )),
    }
}

fn comparison_shape(
    name: &str,
    left: &TextList,
    right: &TextList,
) -> Result<(Shape, bool), BuiltinError> {
    let dimensions = if left.is_scalar() {
        right.shape.dimensions()
    } else if right.is_scalar() || left.shape == right.shape {
        left.shape.dimensions()
    } else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("inputs to `{name}` must have equal shapes or one input must be scalar"),
        ));
    };
    let force_array = matches!(left.kind, TextKind::Cell)
        || matches!(right.kind, TextKind::Cell)
        || left.len() != 1
        || right.len() != 1;
    Shape::new(dimensions.iter().copied())
        .map(|shape| (shape, force_array))
        .map_err(|error| array_error(&error))
}

fn compare_units(left: &[u16], right: &[u16], prefix: Option<usize>, ignore_case: bool) -> bool {
    let count = prefix.unwrap_or(usize::MAX);
    if count == 0 {
        return true;
    }
    if prefix.is_none() && left.len() != right.len() {
        return false;
    }
    let left = &left[..left.len().min(count)];
    let right = &right[..right.len().min(count)];
    units_equal(left, right, ignore_case)
}

pub(super) fn strlength_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("strlength", arguments, 1)?;
    expect_max_outputs("strlength", context, 1)?;
    context.check_cancelled()?;
    let input = TextList::new("strlength", 1, &arguments[0])?;
    let values = input
        .elements
        .iter()
        .map(|value| {
            value
                .as_ref()
                .map_or(f64::NAN, |value| usize_as_double(value.len()))
        })
        .collect::<Vec<_>>();
    if matches!(input.kind, TextKind::Char | TextKind::StringScalar) {
        Ok(vec![Value::Double(values[0])])
    } else {
        f64_result(input.shape, values)
    }
}

pub(super) fn upper_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    case_builtin("upper", arguments, context, true)
}
pub(super) fn lower_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    case_builtin("lower", arguments, context, false)
}

fn case_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    upper: bool,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    if let Value::Array(ArrayData::Char(array)) = &arguments[0] {
        return Ok(vec![char_array_case(array, upper)?]);
    }
    let mut input = TextList::new(name, 1, &arguments[0])?;
    for (index, element) in input.elements.iter_mut().enumerate() {
        check_cancelled_at(context, index)?;
        if let Some(units) = element {
            *units = if matches!(input.kind, TextKind::Cell) {
                units
                    .iter()
                    .map(|unit| simple_case_unit(*unit, upper))
                    .collect()
            } else {
                map_unicode_case(units, upper)
            };
        }
    }
    Ok(vec![text_output(input)?])
}

pub(super) fn reverse_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    unary_text_builtin("reverse", arguments, context, |units| units.reverse())
}

pub(super) fn strip_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.is_empty() || arguments.len() > 3 {
        return Err(argument_count("strip", "1 to 3"));
    }
    expect_max_outputs("strip", context, 1)?;
    let side = if arguments.len() >= 2 {
        ascii_scalar_text("strip", 2, &arguments[1])?
    } else {
        "both".to_owned()
    };
    if !matches!(side.as_str(), "left" | "right" | "both") {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 2 to `strip` must be 'left', 'right', or 'both'",
        ));
    }
    let strip_character = if arguments.len() == 3 {
        let units = scalar_text("strip", 3, &arguments[2])?;
        if unicode_scalar_count(&units) != 1 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "strip character must contain exactly one character",
            ));
        }
        Some(units)
    } else {
        None
    };
    let mut input = TextList::new("strip", 1, &arguments[0])?;
    for units in input.elements.iter_mut().flatten() {
        *units = trim_units(units, &side, strip_character.as_deref(), false);
    }
    Ok(vec![text_output(input)?])
}

pub(super) fn strtrim_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    matrix_trim_builtin("strtrim", arguments, context, true)
}
pub(super) fn deblank_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    matrix_trim_builtin("deblank", arguments, context, false)
}

fn matrix_trim_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    both: bool,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    if let Value::Array(ArrayData::Char(array)) = &arguments[0] {
        return Ok(vec![trim_char_matrix(array, both, !both)?]);
    }
    let mut input = TextList::new(name, 1, &arguments[0])?;
    for units in input.elements.iter_mut().flatten() {
        *units = trim_units(units, if both { "both" } else { "right" }, None, !both);
    }
    Ok(vec![text_output(input)?])
}

pub(super) fn contains_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    predicate_builtin("contains", arguments, context, Predicate::Contains)
}
pub(super) fn starts_with_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    predicate_builtin("startsWith", arguments, context, Predicate::Starts)
}
pub(super) fn ends_with_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    predicate_builtin("endsWith", arguments, context, Predicate::Ends)
}

#[derive(Clone, Copy)]
enum Predicate {
    Contains,
    Starts,
    Ends,
}

fn predicate_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    predicate: Predicate,
) -> BuiltinResult {
    if arguments.len() < 2 || arguments.len() > 4 || arguments.len() == 3 {
        return Err(argument_count(name, "2 or 4"));
    }
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let input = TextList::new(name, 1, &arguments[0])?;
    let patterns = TextList::new(name, 2, &arguments[1])?;
    let ignore_case = if arguments.len() == 4 {
        if ascii_scalar_text(name, 3, &arguments[2])? != "ignorecase" {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` supports only the IgnoreCase name-value option"),
            ));
        }
        logical_scalar(name, 4, &arguments[3])?
    } else {
        false
    };
    let mut values = reserved_vec(input.len(), name, "logical elements")?;
    for (index, element) in input.elements.iter().enumerate() {
        check_cancelled_at(context, index)?;
        let matched = element.as_ref().is_some_and(|text| {
            patterns
                .elements
                .iter()
                .flatten()
                .any(|pattern| match predicate {
                    Predicate::Contains => find_units(text, pattern, ignore_case).next().is_some(),
                    Predicate::Starts => starts_with_units(text, pattern, ignore_case),
                    Predicate::Ends => ends_with_units(text, pattern, ignore_case),
                })
        });
        values.push(Logical::from(matched));
    }
    if matches!(input.kind, TextKind::Char | TextKind::StringScalar) {
        Ok(vec![Value::Logical(values[0].get())])
    } else {
        logical_result(input.shape, values)
    }
}

pub(super) fn strfind_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 2 || arguments.len() > 4 || arguments.len() == 3 {
        return Err(argument_count("strfind", "2 or 4"));
    }
    expect_max_outputs("strfind", context, 1)?;
    context.check_cancelled()?;
    let input = TextList::new("strfind", 1, &arguments[0])?;
    let pattern = scalar_text("strfind", 2, &arguments[1])?;
    let force_cell = if arguments.len() == 4 {
        if ascii_scalar_text("strfind", 3, &arguments[2])? != "forcecelloutput" {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`strfind` supports only the ForceCellOutput name-value option",
            ));
        }
        logical_scalar("strfind", 4, &arguments[3])?
    } else {
        false
    };
    let mut outputs = Vec::with_capacity(input.len());
    for (index, element) in input.elements.iter().enumerate() {
        check_cancelled_at(context, index)?;
        let matches = if pattern.is_empty() {
            Vec::new()
        } else {
            element.as_ref().map_or_else(Vec::new, |text| {
                find_units(text, &pattern, false)
                    .map(|index| usize_as_double(index) + 1.0)
                    .collect()
            })
        };
        outputs.push(double_row(matches)?);
    }
    if matches!(input.kind, TextKind::Char | TextKind::StringScalar) && !force_cell {
        Ok(vec![outputs.remove(0)])
    } else {
        Ok(vec![cell_value(input.shape, outputs)?])
    }
}

pub(super) fn strrep_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("strrep", arguments, 3)?;
    expect_max_outputs("strrep", context, 1)?;
    context.check_cancelled()?;
    let mut input = TextList::new("strrep", 1, &arguments[0])?;
    let old = TextList::new("strrep", 2, &arguments[1])?;
    let new = TextList::new("strrep", 3, &arguments[2])?;
    let shape = broadcast_shape("strrep", &[&input, &old, &new])?;
    let length = checked_len(shape.numel(), "strrep")?;
    let mut elements = reserved_vec(length, "strrep", "text elements")?;
    for index in 0..length {
        check_cancelled_at(context, index)?;
        elements.push(
            match (pick(&input, index), pick(&old, index), pick(&new, index)) {
                (Some(text), Some(old), Some(new)) => Some(if old.is_empty() {
                    text.to_vec()
                } else {
                    replace_all(text, old, new)
                }),
                _ => None,
            },
        );
    }
    input.kind = if matches!(input.kind, TextKind::Cell)
        || matches!(old.kind, TextKind::Cell)
        || matches!(new.kind, TextKind::Cell)
    {
        TextKind::Cell
    } else if length == 1
        && !matches!(input.kind, TextKind::StringArray)
        && !matches!(old.kind, TextKind::StringArray)
        && !matches!(new.kind, TextKind::StringArray)
    {
        if matches!(input.kind, TextKind::Char)
            && matches!(old.kind, TextKind::Char)
            && matches!(new.kind, TextKind::Char)
        {
            TextKind::Char
        } else {
            TextKind::StringScalar
        }
    } else {
        TextKind::StringArray
    };
    input.shape = shape;
    input.elements = elements;
    Ok(vec![text_output(input)?])
}

pub(super) fn erase_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    replace_patterns_builtin("erase", arguments, context, true)
}
pub(super) fn replace_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    replace_patterns_builtin("replace", arguments, context, false)
}

fn replace_patterns_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    erase: bool,
) -> BuiltinResult {
    expect_argument_count(name, arguments, if erase { 2 } else { 3 })?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let mut input = TextList::new(name, 1, &arguments[0])?;
    let patterns = TextList::new(name, 2, &arguments[1])?;
    let replacement_input = if erase {
        None
    } else {
        Some(TextList::new(name, 3, &arguments[2])?)
    };
    if let Some(replacements) = &replacement_input
        && !replacements.is_scalar()
        && replacements.shape != patterns.shape
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "replace text must be scalar or the same size as pattern",
        ));
    }
    let replacements =
        replacement_input.map_or_else(|| vec![Some(Vec::new())], |input| input.elements);
    for (index, element) in input.elements.iter_mut().enumerate() {
        check_cancelled_at(context, index)?;
        let Some(text) = element else { continue };
        let mut changed = text.clone();
        for (pattern_index, pattern) in patterns.elements.iter().enumerate() {
            let Some(pattern) = pattern else { continue };
            if erase && pattern.is_empty() {
                continue;
            }
            let replacement = replacements
                .get(if replacements.len() == 1 {
                    0
                } else {
                    pattern_index
                })
                .and_then(Option::as_deref)
                .unwrap_or(&[]);
            changed = if pattern.is_empty() {
                insert_between(&changed, replacement)
            } else {
                replace_all(&changed, pattern, replacement)
            };
        }
        *text = changed;
    }
    Ok(vec![text_output(input)?])
}

fn unary_text_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    operation: impl Fn(&mut Vec<u16>),
) -> BuiltinResult {
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let mut input = TextList::new(name, 1, &arguments[0])?;
    for (index, element) in input.elements.iter_mut().enumerate() {
        check_cancelled_at(context, index)?;
        if let Some(units) = element {
            operation(units);
        }
    }
    Ok(vec![text_output(input)?])
}

fn text_output(input: TextList) -> Result<Value, BuiltinError> {
    match input.kind {
        TextKind::Char => char_value(
            input
                .elements
                .into_iter()
                .next()
                .flatten()
                .unwrap_or_default(),
        ),
        TextKind::StringScalar => Ok(Value::String(StringValue::scalar(
            input
                .elements
                .into_iter()
                .next()
                .flatten()
                .map_or_else(StringElement::missing, StringElement::from_code_units),
        ))),
        TextKind::StringArray => StringArray::from_elements(
            input.shape,
            input
                .elements
                .into_iter()
                .map(|value| {
                    value.map_or_else(StringElement::missing, StringElement::from_code_units)
                })
                .collect(),
        )
        .map(StringValue::array)
        .map(Value::String)
        .map_err(|error| array_error(&error)),
        TextKind::Cell => cell_value(
            input.shape,
            input
                .elements
                .into_iter()
                .map(|value| char_value(value.unwrap_or_default()))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    }
}

fn char_value(units: Vec<u16>) -> Result<Value, BuiltinError> {
    let columns = u64::try_from(units.len()).map_err(|_| output_too_large())?;
    let shape = if columns == 0 { [0, 0] } else { [1, columns] };
    DenseArray::from_vec(
        Shape::new(shape).map_err(|error| array_error(&error))?,
        units.into_iter().map(CharCodeUnit::new).collect(),
    )
    .map(ArrayData::Char)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn char_array_case(array: &DenseArray<CharCodeUnit>, upper: bool) -> Result<Value, BuiltinError> {
    let values = array
        .as_slice()
        .iter()
        .map(|unit| CharCodeUnit::new(simple_case_unit(unit.get(), upper)))
        .collect();
    DenseArray::from_vec(array.shape().clone(), values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn simple_case_unit(unit: u16, upper: bool) -> u16 {
    let Some(character) = char::from_u32(u32::from(unit)) else {
        return unit;
    };
    if upper {
        let mut mapped = character.to_uppercase();
        let first = mapped.next().unwrap_or(character);
        if mapped.next().is_none() && first.len_utf16() == 1 {
            first as u16
        } else {
            unit
        }
    } else {
        let mut mapped = character.to_lowercase();
        let first = mapped.next().unwrap_or(character);
        if mapped.next().is_none() && first.len_utf16() == 1 {
            first as u16
        } else {
            unit
        }
    }
}

fn map_unicode_case(units: &[u16], upper: bool) -> Vec<u16> {
    let mut output = Vec::with_capacity(units.len());
    for decoded in decode_utf16(units.iter().copied()) {
        match decoded {
            Ok(character) => {
                if upper {
                    for mapped in character.to_uppercase() {
                        let mut encoded = [0_u16; 2];
                        output.extend_from_slice(mapped.encode_utf16(&mut encoded));
                    }
                } else {
                    for mapped in character.to_lowercase() {
                        let mut encoded = [0_u16; 2];
                        output.extend_from_slice(mapped.encode_utf16(&mut encoded));
                    }
                }
            }
            Err(error) => output.push(error.unpaired_surrogate()),
        }
    }
    output
}

fn lowercase_units(units: &[u16]) -> Vec<u16> {
    units
        .iter()
        .map(|unit| simple_case_unit(*unit, false))
        .collect()
}

fn trim_char_matrix(
    array: &DenseArray<CharCodeUnit>,
    both: bool,
    trim_null: bool,
) -> Result<Value, BuiltinError> {
    if array.shape().ndims() != 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "character input must be two-dimensional",
        ));
    }
    let rows = checked_len(array.shape().extent(0), "text trim")?;
    let columns = checked_len(array.shape().extent(1), "text trim")?;
    let column_blank = |column: usize| {
        (0..rows)
            .all(|row| is_trim_character(array.as_slice()[column * rows + row].get(), trim_null))
    };
    let mut start = 0;
    if both {
        while start < columns && column_blank(start) {
            start += 1;
        }
    }
    let mut end = columns;
    while end > start && column_blank(end - 1) {
        end -= 1;
    }
    let dimensions = if start == end {
        [0, 0]
    } else {
        [rows as u64, (end - start) as u64]
    };
    DenseArray::from_vec(
        Shape::new(dimensions).map_err(|error| array_error(&error))?,
        array.as_slice()[start * rows..end * rows].to_vec(),
    )
    .map(ArrayData::Char)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn trim_units(
    units: &[u16],
    side: &str,
    strip_character: Option<&[u16]>,
    trim_null: bool,
) -> Vec<u16> {
    if let Some(pattern) = strip_character {
        let mut start = 0;
        let mut end = units.len();
        if matches!(side, "left" | "both") {
            while start < end && starts_with_units(&units[start..end], pattern, false) {
                start += pattern.len();
            }
        }
        if matches!(side, "right" | "both") {
            while end > start && ends_with_units(&units[start..end], pattern, false) {
                end -= pattern.len();
            }
        }
        return units[start..end].to_vec();
    }
    let mut start = 0;
    let mut end = units.len();
    if matches!(side, "left" | "both") {
        while start < end && is_trim_character(units[start], trim_null) {
            start += 1;
        }
    }
    if matches!(side, "right" | "both") {
        while end > start && is_trim_character(units[end - 1], trim_null) {
            end -= 1;
        }
    }
    units[start..end].to_vec()
}

fn is_trim_character(unit: u16, trim_null: bool) -> bool {
    (trim_null && unit == 0) || char::from_u32(u32::from(unit)).is_some_and(char::is_whitespace)
}
fn starts_with_units(text: &[u16], pattern: &[u16], ignore_case: bool) -> bool {
    text.get(..pattern.len())
        .is_some_and(|candidate| units_equal(candidate, pattern, ignore_case))
}
fn ends_with_units(text: &[u16], pattern: &[u16], ignore_case: bool) -> bool {
    text.len()
        .checked_sub(pattern.len())
        .and_then(|start| text.get(start..))
        .is_some_and(|candidate| units_equal(candidate, pattern, ignore_case))
}
fn units_equal(left: &[u16], right: &[u16], ignore_case: bool) -> bool {
    if ignore_case {
        lowercase_units(left) == lowercase_units(right)
    } else {
        left == right
    }
}

fn find_units<'a>(
    text: &'a [u16],
    pattern: &'a [u16],
    ignore_case: bool,
) -> impl Iterator<Item = usize> + 'a {
    let end = text
        .len()
        .checked_sub(pattern.len())
        .map_or(0, |last| last + 1);
    (0..end).filter(move |index| {
        text.get(*index..*index + pattern.len())
            .is_some_and(|candidate| units_equal(candidate, pattern, ignore_case))
    })
}

fn replace_all(text: &[u16], old: &[u16], new: &[u16]) -> Vec<u16> {
    let mut output = Vec::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        if text[index..].starts_with(old) {
            output.extend_from_slice(new);
            index += old.len();
        } else {
            output.push(text[index]);
            index += 1;
        }
    }
    output
}

fn insert_between(text: &[u16], new: &[u16]) -> Vec<u16> {
    let mut output = Vec::with_capacity(text.len().saturating_add(new.len()));
    output.extend_from_slice(new);
    for unit in text {
        output.push(*unit);
        output.extend_from_slice(new);
    }
    output
}

fn broadcast_shape(name: &str, inputs: &[&TextList]) -> Result<Shape, BuiltinError> {
    let mut dimensions: Option<&[u64]> = None;
    for input in inputs {
        if input.is_scalar() {
            continue;
        }
        if let Some(existing) = dimensions {
            if existing != input.shape.dimensions() {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("non-scalar inputs to `{name}` must have equal shapes"),
                ));
            }
        } else {
            dimensions = Some(input.shape.dimensions());
        }
    }
    Shape::new(dimensions.unwrap_or(&[1, 1]).iter().copied()).map_err(|error| array_error(&error))
}

fn pick(input: &TextList, index: usize) -> Option<&[u16]> {
    input.elements[if input.is_scalar() { 0 } else { index }].as_deref()
}

fn scalar_text(name: &str, position: usize, value: &Value) -> Result<Vec<u16>, BuiltinError> {
    let input = TextList::new(name, position, value)?;
    if !input.is_scalar() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input {position} to `{name}` must be a text scalar"),
        ));
    }
    input.elements.into_iter().next().flatten().ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input {position} to `{name}` must be nonmissing"),
        )
    })
}

fn ascii_scalar_text(name: &str, position: usize, value: &Value) -> Result<String, BuiltinError> {
    let units = scalar_text(name, position, value)?;
    let mut output = String::with_capacity(units.len());
    for unit in units {
        let byte = u8::try_from(unit).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("input {position} to `{name}` must contain ASCII text"),
            )
        })?;
        output.push(char::from(byte).to_ascii_lowercase());
    }
    Ok(output)
}

fn logical_scalar(name: &str, position: usize, value: &Value) -> Result<bool, BuiltinError> {
    match value {
        Value::Logical(value) => Ok(*value),
        Value::Double(value) if value.is_finite() => Ok(*value != 0.0),
        Value::Array(ArrayData::Logical(array)) if array.numel() == 1 => {
            Ok(array.as_slice()[0].get())
        }
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => Ok(array.as_slice()[0] != 0.0),
        other => Err(type_error(name, position, "logical scalar", other)),
    }
}

fn scalar_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Double(value) => Some(*value),
        Value::Logical(value) => Some(if *value { 1.0 } else { 0.0 }),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            array.as_slice().first().copied()
        }
        _ => None,
    }
}

fn char_row_units(
    name: &str,
    position: usize,
    array: &DenseArray<CharCodeUnit>,
) -> Result<Vec<u16>, BuiltinError> {
    if array.shape().dimensions() == [0, 0]
        || (array.shape().ndims() == 2 && array.shape().extent(0) == 1)
    {
        Ok(units_of(array).collect())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input {position} to `{name}` must be a character row vector"),
        ))
    }
}

fn units_of(array: &DenseArray<CharCodeUnit>) -> impl Iterator<Item = u16> + '_ {
    array.as_slice().iter().map(|value| value.get())
}
fn string_units(element: &StringElement) -> Option<Vec<u16>> {
    (!element.is_missing()).then(|| element.code_units().to_vec())
}
fn unicode_scalar_count(units: &[u16]) -> usize {
    decode_utf16(units.iter().copied()).count()
}

fn logical_result(shape: Shape, values: Vec<Logical>) -> BuiltinResult {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map(|value| vec![value])
        .map_err(|error| array_error(&error))
}

fn f64_result(shape: Shape, values: Vec<f64>) -> BuiltinResult {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map(|value| vec![value])
        .map_err(|error| array_error(&error))
}

fn double_row(values: Vec<f64>) -> Result<Value, BuiltinError> {
    let columns = u64::try_from(values.len()).map_err(|_| output_too_large())?;
    let shape = if columns == 0 { [0, 0] } else { [1, columns] };
    DenseArray::from_vec(
        Shape::new(shape).map_err(|error| array_error(&error))?,
        values,
    )
    .map(ArrayData::F64)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn cell_value(shape: Shape, values: Vec<Value>) -> Result<Value, BuiltinError> {
    CellArray::from_values(shape, values)
        .map(Value::Cell)
        .map_err(|error| aggregate_error("string", &error))
}
fn scalar_shape() -> Result<Shape, BuiltinError> {
    Shape::new([1, 1]).map_err(|error| array_error(&error))
}
fn checked_len(length: u64, name: &str) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` output length does not fit this host"),
        )
    })
}

#[allow(clippy::cast_precision_loss)]
fn usize_as_double(value: usize) -> f64 {
    value as f64
}

fn reserved_vec<T>(length: usize, name: &str, kind: &str) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {length} {kind}"),
        )
    })?;
    Ok(values)
}

fn argument_count(name: &str, expected: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::ArgumentCount,
        format!("built-in `{name}` expects {expected} inputs"),
    )
}
fn output_too_large() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "string output is too large for this host",
    )
}
fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

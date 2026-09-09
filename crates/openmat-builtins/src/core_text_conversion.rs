use openmat_array::{
    ArrayData, CharCodeUnit, Complex64 as ArrayComplex64, DenseArray, IntegerComponent, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{CellArray, Complex64, StringElement, StringValue, Value};

use crate::{array_error, expect_argument_count_range, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;
const MAX_FORMAT_FIELD: usize = 1_000_000;

#[derive(Clone, Copy, Debug)]
enum NumericComponent {
    Float(f64),
    Signed(i128),
    Unsigned(u128),
}

impl NumericComponent {
    #[allow(clippy::cast_precision_loss)]
    fn as_f64(self) -> f64 {
        match self {
            Self::Float(value) => value,
            Self::Signed(value) => value as f64,
            Self::Unsigned(value) => value as f64,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct NumericElement {
    real: NumericComponent,
    imaginary: Option<NumericComponent>,
}

impl NumericElement {
    const fn real(real: NumericComponent) -> Self {
        Self {
            real,
            imaginary: None,
        }
    }

    const fn complex(real: NumericComponent, imaginary: NumericComponent) -> Self {
        Self {
            real,
            imaginary: Some(imaginary),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
struct FormatFlags {
    left: bool,
    plus: bool,
    space: bool,
    alternate: bool,
    zero: bool,
}

#[derive(Clone, Copy, Debug)]
enum FieldValue {
    Fixed(usize),
    Dynamic,
}

#[derive(Clone, Copy, Debug)]
struct FormatSpec {
    flags: FormatFlags,
    width: Option<FieldValue>,
    precision: Option<FieldValue>,
    conversion: u16,
}

#[derive(Clone, Debug)]
enum FormatToken {
    Literal(Vec<u16>),
    Conversion(FormatSpec),
}

#[derive(Clone, Debug)]
enum StreamValue {
    Numeric(NumericElement),
    Text(Vec<u16>),
}

struct ArgumentCursor<'a> {
    value: &'a Value,
    offset: usize,
}

struct ValueStream<'a> {
    arguments: Vec<ArgumentCursor<'a>>,
    argument: usize,
}

impl<'a> ValueStream<'a> {
    fn new(arguments: &'a [Value]) -> Self {
        Self {
            arguments: arguments
                .iter()
                .map(|value| ArgumentCursor { value, offset: 0 })
                .collect(),
            argument: 0,
        }
    }

    fn next(&mut self, conversion: u16) -> Result<Option<StreamValue>, BuiltinError> {
        while let Some(cursor) = self.arguments.get_mut(self.argument) {
            if let Some(value) = next_stream_value(cursor, conversion)? {
                return Ok(Some(value));
            }
            self.argument += 1;
        }
        Ok(None)
    }

    fn has_remaining(&self) -> bool {
        self.arguments
            .iter()
            .skip(self.argument)
            .any(|cursor| cursor.offset < stream_value_len(cursor.value))
    }
}

fn stream_value_len(value: &Value) -> usize {
    match value {
        Value::Double(_)
        | Value::Complex(_)
        | Value::Logical(_)
        | Value::String(StringValue::Scalar(_)) => 1,
        Value::Array(ArrayData::F32(array)) => array.as_slice().len(),
        Value::Array(ArrayData::ComplexF32(array)) => array.as_slice().len(),
        Value::Array(ArrayData::F64(array)) => array.as_slice().len(),
        Value::Array(ArrayData::ComplexF64(array)) => array.as_slice().len(),
        Value::Array(ArrayData::Logical(array)) => array.as_slice().len(),
        Value::Array(ArrayData::Char(array)) => array.as_slice().len(),
        Value::Array(ArrayData::Integer(array)) => {
            usize::try_from(array.numel()).unwrap_or(usize::MAX)
        }
        Value::String(StringValue::Array(array)) => array.as_slice().len(),
        Value::Nothing
        | Value::Sparse(_)
        | Value::Cell(_)
        | Value::Struct(_)
        | Value::Table(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => 0,
    }
}

pub(super) fn sprintf_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "built-in `sprintf` expects at least 1 input but received 0",
        ));
    }
    expect_max_outputs("sprintf", context, 1)?;
    context.check_cancelled()?;
    let (format, output_string) = scalar_format("sprintf", 1, &arguments[0])?;
    for (index, argument) in arguments[1..].iter().enumerate() {
        validate_sprintf_argument(index + 2, argument)?;
    }
    let tokens = parse_format(&format)?;
    let mut stream = ValueStream::new(&arguments[1..]);
    let mut output = Vec::new();
    let has_conversion = tokens
        .iter()
        .any(|token| matches!(token, FormatToken::Conversion(_)));
    let mut cycle = 0_usize;

    loop {
        let before = output.len();
        let mut consumed = false;
        for token in &tokens {
            match token {
                FormatToken::Literal(literal) => output.extend_from_slice(literal),
                FormatToken::Conversion(spec) => {
                    let mut spec = *spec;
                    if !resolve_dynamic_fields(&mut spec, &mut stream)? {
                        return text_output(output, output_string, true);
                    }
                    let Some(value) = stream.next(spec.conversion)? else {
                        return text_output(output, output_string, true);
                    };
                    consumed = true;
                    output.extend(format_stream_value(value, spec)?);
                }
            }
            check_cancelled_at(context, output.len())?;
        }
        cycle = cycle.saturating_add(1);
        if !has_conversion || !consumed || output.len() == before || !stream.has_remaining() {
            break;
        }
        if cycle.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
            context.check_cancelled()?;
        }
    }
    context.check_cancelled()?;
    text_output(output, output_string, true)
}

fn validate_sprintf_argument(position: usize, value: &Value) -> Result<(), BuiltinError> {
    if matches!(
        value,
        Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Logical(_)
                    | ArrayData::Char(_)
                    | ArrayData::Integer(_)
            )
            | Value::String(_)
    ) {
        Ok(())
    } else {
        Err(type_error(
            "sprintf",
            position,
            "numeric, logical, character, or string value",
            value,
        ))
    }
}

pub(super) fn num2str_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("num2str", arguments, 1, 2)?;
    expect_max_outputs("num2str", context, 1)?;
    context.check_cancelled()?;
    let input = NumericMatrix::new(&arguments[0])?;
    if input.rows == 0 || input.columns == 0 {
        return char_matrix(&[], false);
    }

    let mode = if let Some(option) = arguments.get(1) {
        Num2StrMode::from_value(option)?
    } else {
        Num2StrMode::Default
    };
    let rows = format_num2str_rows(&input, &mode, context)?;
    char_matrix(&rows, true)
}

pub(super) fn str2double_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("str2double", arguments, 1, 1)?;
    expect_max_outputs("str2double", context, 1)?;
    context.check_cancelled()?;

    match &arguments[0] {
        Value::Array(ArrayData::Char(array)) => {
            let parsed = parse_char_input(array);
            Ok(parsed_scalar_output(parsed))
        }
        Value::String(value) => parse_string_value(value, context),
        Value::Cell(value) => parse_cell_value(value, context),
        value => Err(type_error(
            "str2double",
            1,
            "char array, string array, or cell array of character vectors",
            value,
        )),
    }
}

fn scalar_format(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<(Vec<u16>, bool), BuiltinError> {
    match value {
        Value::Array(ArrayData::Char(array)) => {
            let dimensions = array.shape().dimensions();
            if dimensions == [0, 0] || (array.shape().ndims() == 2 && array.shape().extent(0) == 1)
            {
                Ok((
                    array.as_slice().iter().map(|value| value.get()).collect(),
                    false,
                ))
            } else {
                Err(type_error(
                    name,
                    position,
                    "character row vector or string scalar",
                    value,
                ))
            }
        }
        Value::String(value) => {
            let element = value.as_scalar().ok_or_else(|| {
                type_error(
                    name,
                    position,
                    "character row vector or string scalar",
                    &Value::String(value.clone()),
                )
            })?;
            if element.is_missing() {
                return Err(type_error(
                    name,
                    position,
                    "nonmissing character row vector or string scalar",
                    &Value::String(value.clone()),
                ));
            }
            Ok((element.code_units().to_vec(), true))
        }
        value => Err(type_error(
            name,
            position,
            "character row vector or string scalar",
            value,
        )),
    }
}

fn parse_format(format: &[u16]) -> Result<Vec<FormatToken>, BuiltinError> {
    let mut tokens = Vec::new();
    let mut literal = Vec::new();
    let mut index = 0_usize;
    while index < format.len() {
        match format[index] {
            value if value == u16::from(b'\\') => {
                let (escaped, consumed) = parse_escape(format, index);
                literal.extend(escaped);
                index += consumed;
            }
            value if value == u16::from(b'%') => {
                if format.get(index + 1) == Some(&u16::from(b'%')) {
                    literal.push(u16::from(b'%'));
                    index += 2;
                    continue;
                }
                if !literal.is_empty() {
                    tokens.push(FormatToken::Literal(std::mem::take(&mut literal)));
                }
                let (spec, consumed) = parse_format_spec(&format[index..])?;
                tokens.push(FormatToken::Conversion(spec));
                index += consumed;
            }
            value => {
                literal.push(value);
                index += 1;
            }
        }
    }
    if !literal.is_empty() || tokens.is_empty() {
        tokens.push(FormatToken::Literal(literal));
    }
    Ok(tokens)
}

fn parse_escape(format: &[u16], index: usize) -> (Vec<u16>, usize) {
    let Some(next) = format.get(index + 1).copied() else {
        return (vec![u16::from(b'\\')], 1);
    };
    let escaped = match u8::try_from(next).ok() {
        Some(b'n') => Some(u16::from(b'\n')),
        Some(b't') => Some(u16::from(b'\t')),
        Some(b'r') => Some(u16::from(b'\r')),
        Some(b'b') => Some(u16::from(8_u8)),
        Some(b'f') => Some(u16::from(12_u8)),
        Some(b'v') => Some(u16::from(11_u8)),
        Some(b'\\') => Some(u16::from(b'\\')),
        _ => None,
    };
    escaped.map_or_else(
        || (vec![u16::from(b'\\'), next], 2),
        |value| (vec![value], 2),
    )
}

fn parse_format_spec(format: &[u16]) -> Result<(FormatSpec, usize), BuiltinError> {
    let mut index = 1_usize;
    let mut flags = FormatFlags::default();
    loop {
        match format
            .get(index)
            .copied()
            .and_then(|value| u8::try_from(value).ok())
        {
            Some(b'-') => flags.left = true,
            Some(b'+') => flags.plus = true,
            Some(b' ') => flags.space = true,
            Some(b'#') => flags.alternate = true,
            Some(b'0') => flags.zero = true,
            _ => break,
        }
        index += 1;
    }
    let (width, next) = parse_field_value(format, index, false)?;
    index = next;
    let precision = if format.get(index) == Some(&u16::from(b'.')) {
        let (precision, next) = parse_field_value(format, index + 1, true)?;
        index = next;
        Some(precision.unwrap_or(FieldValue::Fixed(0)))
    } else {
        None
    };

    while matches!(
        format
            .get(index)
            .copied()
            .and_then(|value| u8::try_from(value).ok()),
        Some(b'h' | b'l' | b'L')
    ) {
        index += 1;
    }
    let conversion = format.get(index).copied().ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`sprintf` format ends before its conversion character",
        )
    })?;
    if !matches!(
        u8::try_from(conversion).ok(),
        Some(
            b'd' | b'i'
                | b'u'
                | b'o'
                | b'x'
                | b'X'
                | b'f'
                | b'F'
                | b'e'
                | b'E'
                | b'g'
                | b'G'
                | b'c'
                | b's'
        )
    ) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`sprintf` does not support conversion character U+{conversion:04X}"),
        ));
    }
    Ok((
        FormatSpec {
            flags,
            width,
            precision,
            conversion,
        },
        index + 1,
    ))
}

fn parse_field_value(
    format: &[u16],
    mut index: usize,
    precision: bool,
) -> Result<(Option<FieldValue>, usize), BuiltinError> {
    if format.get(index) == Some(&u16::from(b'*')) {
        return Ok((Some(FieldValue::Dynamic), index + 1));
    }
    let start = index;
    let mut value = 0_usize;
    while let Some(digit) = format
        .get(index)
        .copied()
        .and_then(|value| u8::try_from(value).ok())
        .and_then(|value| value.checked_sub(b'0'))
        .filter(|value| *value <= 9)
    {
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(usize::from(digit)))
            .ok_or_else(|| format_field_too_large(precision))?;
        if value > MAX_FORMAT_FIELD {
            return Err(format_field_too_large(precision));
        }
        index += 1;
    }
    Ok(if index == start {
        (None, index)
    } else {
        (Some(FieldValue::Fixed(value)), index)
    })
}

fn format_field_too_large(precision: bool) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!(
            "`sprintf` {} exceeds the supported allocation bound",
            if precision {
                "precision"
            } else {
                "field width"
            }
        ),
    )
}

fn resolve_dynamic_fields(
    spec: &mut FormatSpec,
    stream: &mut ValueStream<'_>,
) -> Result<bool, BuiltinError> {
    if matches!(spec.width, Some(FieldValue::Dynamic)) {
        let Some(width) = dynamic_field(stream, "field width")? else {
            return Ok(false);
        };
        if width < 0 {
            spec.flags.left = true;
        }
        spec.width = Some(FieldValue::Fixed(width.unsigned_abs()));
    }
    if matches!(spec.precision, Some(FieldValue::Dynamic)) {
        let Some(precision) = dynamic_field(stream, "precision")? else {
            return Ok(false);
        };
        spec.precision = if precision < 0 {
            None
        } else {
            Some(FieldValue::Fixed(precision.unsigned_abs()))
        };
    }
    Ok(true)
}

fn dynamic_field(stream: &mut ValueStream<'_>, field: &str) -> Result<Option<isize>, BuiltinError> {
    let Some(value) = stream.next(u16::from(b'd'))? else {
        return Ok(None);
    };
    let StreamValue::Numeric(value) = value else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`sprintf` dynamic {field} requires a numeric value"),
        ));
    };
    let value = value.real.as_f64();
    if !value.is_finite() || value.fract() != 0.0 || value.abs() > 1_000_000.0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`sprintf` dynamic {field} must be a bounded integer"),
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(Some(value as isize))
}

fn next_stream_value(
    cursor: &mut ArgumentCursor<'_>,
    conversion: u16,
) -> Result<Option<StreamValue>, BuiltinError> {
    match cursor.value {
        Value::Double(value) if cursor.offset == 0 => {
            cursor.offset = 1;
            Ok(Some(StreamValue::Numeric(NumericElement::real(
                NumericComponent::Float(*value),
            ))))
        }
        Value::Complex(value) if cursor.offset == 0 => {
            cursor.offset = 1;
            Ok(Some(StreamValue::Numeric(NumericElement::complex(
                NumericComponent::Float(value.real),
                NumericComponent::Float(value.imaginary),
            ))))
        }
        Value::Logical(value) if cursor.offset == 0 => {
            cursor.offset = 1;
            Ok(Some(StreamValue::Numeric(NumericElement::real(
                NumericComponent::Unsigned(u128::from(*value)),
            ))))
        }
        Value::Array(ArrayData::F32(array)) => {
            Ok(array.as_slice().get(cursor.offset).copied().map(|value| {
                cursor.offset += 1;
                StreamValue::Numeric(NumericElement::real(NumericComponent::Float(f64::from(
                    value,
                ))))
            }))
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            Ok(array.as_slice().get(cursor.offset).copied().map(|value| {
                cursor.offset += 1;
                StreamValue::Numeric(NumericElement::complex(
                    NumericComponent::Float(f64::from(value.re)),
                    NumericComponent::Float(f64::from(value.im)),
                ))
            }))
        }
        Value::Array(ArrayData::F64(array)) => {
            Ok(array.as_slice().get(cursor.offset).copied().map(|value| {
                cursor.offset += 1;
                StreamValue::Numeric(NumericElement::real(NumericComponent::Float(value)))
            }))
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            Ok(array.as_slice().get(cursor.offset).copied().map(|value| {
                cursor.offset += 1;
                StreamValue::Numeric(NumericElement::complex(
                    NumericComponent::Float(value.re),
                    NumericComponent::Float(value.im),
                ))
            }))
        }
        Value::Array(ArrayData::Logical(array)) => {
            Ok(array.as_slice().get(cursor.offset).copied().map(|value| {
                cursor.offset += 1;
                StreamValue::Numeric(NumericElement::real(NumericComponent::Unsigned(
                    u128::from(bool::from(value)),
                )))
            }))
        }
        Value::Array(ArrayData::Integer(array)) => Ok(array.element(cursor.offset).map(|value| {
            cursor.offset += 1;
            StreamValue::Numeric(integer_numeric_element(value))
        })),
        Value::Array(ArrayData::Char(array)) => {
            if conversion == u16::from(b's') && cursor.offset == 0 {
                cursor.offset = array.as_slice().len();
                return Ok(Some(StreamValue::Text(
                    array.as_slice().iter().map(|value| value.get()).collect(),
                )));
            }
            Ok(array.as_slice().get(cursor.offset).copied().map(|value| {
                cursor.offset += 1;
                StreamValue::Numeric(NumericElement::real(NumericComponent::Unsigned(
                    u128::from(value.get()),
                )))
            }))
        }
        Value::String(value) => next_string_stream_value(value, cursor, conversion),
        Value::Nothing
        | Value::Sparse(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::Logical(_)
        | Value::Cell(_)
        | Value::Struct(_)
        | Value::Table(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => Ok(None),
    }
}

fn next_string_stream_value(
    value: &StringValue,
    cursor: &mut ArgumentCursor<'_>,
    conversion: u16,
) -> Result<Option<StreamValue>, BuiltinError> {
    let element = match value {
        StringValue::Scalar(element) if cursor.offset == 0 => Some(element),
        StringValue::Array(array) => array.as_slice().get(cursor.offset),
        StringValue::Scalar(_) => None,
    };
    let Some(element) = element else {
        return Ok(None);
    };
    cursor.offset += 1;
    if element.is_missing() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`sprintf` cannot format a missing string",
        ));
    }
    if conversion == u16::from(b's') {
        Ok(Some(StreamValue::Text(element.code_units().to_vec())))
    } else {
        let Some(first) = element.code_units().first().copied() else {
            return Ok(None);
        };
        Ok(Some(StreamValue::Numeric(NumericElement::real(
            NumericComponent::Unsigned(u128::from(first)),
        ))))
    }
}

fn integer_numeric_element(value: openmat_array::IntegerElementValue) -> NumericElement {
    let real = integer_component(value.real_component());
    value.imaginary_component().map_or_else(
        || NumericElement::real(real),
        |imaginary| NumericElement::complex(real, integer_component(imaginary)),
    )
}

const fn integer_component(value: IntegerComponent) -> NumericComponent {
    match value {
        IntegerComponent::Signed(value) => NumericComponent::Signed(value),
        IntegerComponent::Unsigned(value) => NumericComponent::Unsigned(value),
    }
}

fn format_stream_value(value: StreamValue, spec: FormatSpec) -> Result<Vec<u16>, BuiltinError> {
    let raw = match value {
        StreamValue::Text(value) if spec.conversion == u16::from(b's') => value,
        StreamValue::Text(value) => {
            let first = value.first().copied().unwrap_or_default();
            format_numeric_value(
                NumericElement::real(NumericComponent::Unsigned(u128::from(first))),
                spec,
            )?
            .encode_utf16()
            .collect()
        }
        StreamValue::Numeric(value) if spec.conversion == u16::from(b's') => {
            vec![numeric_char(value.real)]
        }
        StreamValue::Numeric(value) if spec.conversion == u16::from(b'c') => {
            vec![numeric_char(value.real)]
        }
        StreamValue::Numeric(value) => format_numeric_value(value, spec)?.encode_utf16().collect(),
    };
    if matches!(spec.conversion, value if value == u16::from(b's') || value == u16::from(b'c')) {
        Ok(apply_text_width(raw, spec))
    } else {
        Ok(raw)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn numeric_char(value: NumericComponent) -> u16 {
    let value = value.as_f64();
    if !value.is_finite() || value <= 0.0 {
        0
    } else if value >= f64::from(u16::MAX) {
        u16::MAX
    } else {
        value.trunc() as u16
    }
}

fn format_numeric_value(value: NumericElement, spec: FormatSpec) -> Result<String, BuiltinError> {
    let conversion = u8::try_from(spec.conversion).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`sprintf` encountered a non-ASCII conversion",
        )
    })?;
    let precision = fixed_field(spec.precision);
    let raw = match conversion {
        b'd' | b'i' => format_signed_decimal(value.real, precision),
        b'u' => format_unsigned_decimal(value.real, precision),
        b'o' => format_radix(value.real, 8, false, precision, spec.flags.alternate),
        b'x' => format_radix(value.real, 16, false, precision, spec.flags.alternate),
        b'X' => format_radix(value.real, 16, true, precision, spec.flags.alternate),
        b'f' | b'F' => format_fixed(
            value.real.as_f64(),
            precision.unwrap_or(6),
            spec.flags.alternate,
        ),
        b'e' | b'E' => format_exponential(
            value.real.as_f64(),
            precision.unwrap_or(6),
            conversion == b'E',
            spec.flags.alternate,
        ),
        b'g' | b'G' => format_general(
            value.real.as_f64(),
            precision.unwrap_or(6).max(1),
            conversion == b'G',
            spec.flags.alternate,
        ),
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`sprintf` conversion is not numeric",
            ));
        }
    };
    Ok(apply_numeric_width_and_sign(raw, spec))
}

const fn fixed_field(value: Option<FieldValue>) -> Option<usize> {
    match value {
        Some(FieldValue::Fixed(value)) => Some(value),
        Some(FieldValue::Dynamic) | None => None,
    }
}

fn format_signed_decimal(value: NumericComponent, precision: Option<usize>) -> String {
    let mut raw = match value {
        NumericComponent::Signed(value) => value.to_string(),
        NumericComponent::Unsigned(value) => value.to_string(),
        NumericComponent::Float(value) if value.is_finite() && value.fract() == 0.0 => {
            format!("{value:.0}")
        }
        NumericComponent::Float(value) => format_exponential(value, 6, false, false),
    };
    apply_integer_precision(&mut raw, precision);
    raw
}

fn format_unsigned_decimal(value: NumericComponent, precision: Option<usize>) -> String {
    let mut raw = match value {
        NumericComponent::Unsigned(value) => value.to_string(),
        NumericComponent::Signed(value) if value >= 0 => value.to_string(),
        NumericComponent::Signed(value) => value.to_string(),
        NumericComponent::Float(value)
            if value.is_finite() && value >= 0.0 && value.fract() == 0.0 =>
        {
            format!("{value:.0}")
        }
        NumericComponent::Float(value) => format_exponential(value, 6, false, false),
    };
    apply_integer_precision(&mut raw, precision);
    raw
}

fn apply_integer_precision(value: &mut String, precision: Option<usize>) {
    let Some(precision) = precision else {
        return;
    };
    let sign_len = usize::from(value.starts_with(['-', '+']));
    let digits = value.len().saturating_sub(sign_len);
    if precision <= digits
        || !value[sign_len..]
            .bytes()
            .all(|value| value.is_ascii_digit())
    {
        return;
    }
    value.insert_str(sign_len, &"0".repeat(precision - digits));
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn format_radix(
    value: NumericComponent,
    radix: u32,
    upper: bool,
    precision: Option<usize>,
    alternate: bool,
) -> String {
    let integer = match value {
        NumericComponent::Unsigned(value) => value,
        NumericComponent::Signed(value) if value >= 0 => value as u128,
        NumericComponent::Signed(value) => value as u128,
        NumericComponent::Float(value) if value.is_finite() => value.trunc() as u128,
        NumericComponent::Float(value) => return format_general(value, 6, upper, false),
    };
    let mut digits = match (radix, upper) {
        (8, _) => format!("{integer:o}"),
        (16, false) => format!("{integer:x}"),
        (16, true) => format!("{integer:X}"),
        _ => unreachable!("supported radices are fixed"),
    };
    if let Some(precision) = precision
        && precision > digits.len()
    {
        digits.insert_str(0, &"0".repeat(precision - digits.len()));
    }
    if alternate && integer != 0 {
        match (radix, upper) {
            (8, _) => digits.insert(0, '0'),
            (16, false) => digits.insert_str(0, "0x"),
            (16, true) => digits.insert_str(0, "0X"),
            _ => {}
        }
    }
    digits
}

fn format_fixed(value: f64, precision: usize, alternate: bool) -> String {
    if let Some(special) = special_float(value) {
        return special;
    }
    let mut output = format!("{value:.precision$}");
    if alternate && precision == 0 {
        output.push('.');
    }
    output
}

fn format_exponential(value: f64, precision: usize, upper: bool, alternate: bool) -> String {
    if let Some(mut special) = special_float(value) {
        if upper {
            special.make_ascii_uppercase();
        }
        return special;
    }
    let raw = format!("{value:.precision$e}");
    let Some((mut mantissa, exponent)) = raw
        .split_once('e')
        .map(|(left, right)| (left.to_owned(), right.parse::<i32>().unwrap_or_default()))
    else {
        return raw;
    };
    if alternate && precision == 0 {
        mantissa.push('.');
    }
    let marker = if upper { 'E' } else { 'e' };
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}{marker}{sign}{:02}", exponent.unsigned_abs())
}

fn format_general(value: f64, precision: usize, upper: bool, alternate: bool) -> String {
    if let Some(mut special) = special_float(value) {
        if upper {
            special.make_ascii_uppercase();
        }
        return special;
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        };
    }
    let exponent = decimal_exponent(value);
    let mut output = if exponent < -4 || exponent >= i32::try_from(precision).unwrap_or(i32::MAX) {
        let mut output = format_exponential(value, precision.saturating_sub(1), upper, alternate);
        if !alternate {
            trim_exponential_fraction(&mut output);
        }
        output
    } else {
        let decimals = usize::try_from(i32::try_from(precision).unwrap_or(i32::MAX) - exponent - 1)
            .unwrap_or_default();
        let mut output = format_fixed(value, decimals, alternate);
        if !alternate {
            trim_decimal(&mut output);
        }
        output
    };
    if upper {
        output.make_ascii_uppercase();
    }
    output
}

fn special_float(value: f64) -> Option<String> {
    if value.is_nan() {
        Some("NaN".to_owned())
    } else if value == f64::INFINITY {
        Some("Inf".to_owned())
    } else if value == f64::NEG_INFINITY {
        Some("-Inf".to_owned())
    } else {
        None
    }
}

#[allow(clippy::cast_possible_truncation)]
fn decimal_exponent(value: f64) -> i32 {
    value.abs().log10().floor() as i32
}

fn trim_decimal(value: &mut String) {
    if !value.contains('.') {
        return;
    }
    while value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
}

fn trim_exponential_fraction(value: &mut String) {
    let Some(marker) = value.find(['e', 'E']) else {
        return;
    };
    let exponent = value.split_off(marker);
    trim_decimal(value);
    value.push_str(&exponent);
}

fn apply_numeric_width_and_sign(mut value: String, spec: FormatSpec) -> String {
    if !value.starts_with('-') && (spec.flags.plus || spec.flags.space) {
        value.insert(0, if spec.flags.plus { '+' } else { ' ' });
    }
    let Some(width) = fixed_field(spec.width) else {
        return value;
    };
    if width <= value.len() {
        return value;
    }
    let padding = width - value.len();
    if spec.flags.left {
        value.push_str(&" ".repeat(padding));
    } else if spec.flags.zero {
        let sign_len = usize::from(value.starts_with(['-', '+', ' ']));
        value.insert_str(sign_len, &"0".repeat(padding));
    } else {
        value.insert_str(0, &" ".repeat(padding));
    }
    value
}

fn apply_text_width(mut value: Vec<u16>, spec: FormatSpec) -> Vec<u16> {
    let Some(width) = fixed_field(spec.width) else {
        return value;
    };
    if width <= value.len() {
        if let Some(precision) = fixed_field(spec.precision) {
            value.truncate(precision);
        }
        return value;
    }
    let padding = vec![u16::from(b' '); width - value.len()];
    if spec.flags.left {
        value.extend(padding);
        value
    } else {
        let mut output = padding;
        output.extend(value);
        output
    }
}

struct NumericMatrix {
    values: Vec<NumericElement>,
    rows: usize,
    columns: usize,
    single: bool,
}

impl NumericMatrix {
    fn new(value: &Value) -> Result<Self, BuiltinError> {
        let dimensions = value.dimensions().ok_or_else(|| {
            type_error("num2str", 1, "numeric, logical, or character matrix", value)
        })?;
        if dimensions.len() != 2 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input to `num2str` must be two-dimensional",
            ));
        }
        let rows = usize::try_from(dimensions[0]).map_err(|_| matrix_too_large())?;
        let columns = usize::try_from(dimensions[1]).map_err(|_| matrix_too_large())?;
        let values = numeric_elements(value)?;
        Ok(Self {
            values,
            rows,
            columns,
            single: matches!(
                value,
                Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
            ),
        })
    }
}

enum Num2StrMode {
    Default,
    Precision(usize),
    Format(Vec<FormatToken>),
}

impl Num2StrMode {
    fn from_value(value: &Value) -> Result<Self, BuiltinError> {
        if let Some(precision) = scalar_nonnegative_integer(value) {
            if precision > MAX_FORMAT_FIELD {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`num2str` precision exceeds the supported allocation bound",
                ));
            }
            return Ok(Self::Precision(precision));
        }
        let (format, _) = scalar_format("num2str", 2, value)?;
        Ok(Self::Format(parse_format(&format)?))
    }
}

fn scalar_nonnegative_integer(value: &Value) -> Option<usize> {
    let value = match value {
        Value::Double(value) => *value,
        Value::Array(ArrayData::F32(array)) if array.numel() == 1 => {
            f64::from(*array.as_slice().first()?)
        }
        Value::Array(ArrayData::Integer(array)) if array.numel() == 1 => {
            return match array.element(0)?.real_component() {
                IntegerComponent::Signed(value) => usize::try_from(value).ok(),
                IntegerComponent::Unsigned(value) => usize::try_from(value).ok(),
            };
        }
        _ => return None,
    };
    if value.is_finite() && value >= 0.0 && value.fract() == 0.0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(value as usize)
    } else {
        None
    }
}

fn format_num2str_rows(
    input: &NumericMatrix,
    mode: &Num2StrMode,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Vec<u16>>, BuiltinError> {
    match mode {
        Num2StrMode::Format(tokens) => {
            let mut rows = Vec::with_capacity(input.rows);
            for row in 0..input.rows {
                let mut output = Vec::new();
                for column in 0..input.columns {
                    let index = row + column * input.rows;
                    check_cancelled_at(context, index)?;
                    output.extend(format_num2str_custom(input.values[index], tokens)?);
                }
                rows.push(output);
            }
            Ok(rows)
        }
        Num2StrMode::Default | Num2StrMode::Precision(_) => {
            let precision = match mode {
                Num2StrMode::Precision(value) => (*value).max(1),
                Num2StrMode::Default if input.single => 7,
                Num2StrMode::Default if needs_extended_default_precision(input) => 12,
                Num2StrMode::Default => 5,
                Num2StrMode::Format(_) => unreachable!(),
            };
            let mut fields = vec![Vec::<String>::with_capacity(input.columns); input.rows];
            let mut widths = vec![0_usize; input.columns];
            for (column, width) in widths.iter_mut().enumerate() {
                for (row, row_fields) in fields.iter_mut().enumerate() {
                    let index = row + column * input.rows;
                    check_cancelled_at(context, index)?;
                    let field = format_num2str_general(input.values[index], precision);
                    *width = (*width).max(field.encode_utf16().count());
                    row_fields.push(field);
                }
            }
            Ok(fields
                .into_iter()
                .map(|row| {
                    let mut output = Vec::new();
                    for (column, field) in row.into_iter().enumerate() {
                        if column > 0 {
                            output.extend([u16::from(b' '), u16::from(b' ')]);
                        }
                        let units = field.encode_utf16().collect::<Vec<_>>();
                        output.extend(std::iter::repeat_n(
                            u16::from(b' '),
                            widths[column].saturating_sub(units.len()),
                        ));
                        output.extend(units);
                    }
                    output
                })
                .collect())
        }
    }
}

fn needs_extended_default_precision(input: &NumericMatrix) -> bool {
    let maximum = input
        .values
        .iter()
        .map(|value| value.real.as_f64().abs())
        .filter(|value| value.is_finite())
        .fold(0.0_f64, f64::max);
    maximum >= 1.0e8
        && input.values.iter().any(|value| {
            let real = value.real.as_f64();
            real.is_finite() && real != 0.0 && real.abs() < maximum && real.fract() != 0.0
        })
}

fn format_num2str_general(value: NumericElement, precision: usize) -> String {
    let mut real = format_general(value.real.as_f64(), precision, false, false);
    if real == "-0" {
        "0".clone_into(&mut real);
    }
    let Some(imaginary) = value.imaginary else {
        return real;
    };
    let imaginary = imaginary.as_f64();
    let mut imaginary_text = format_general(imaginary.abs(), precision, false, false);
    if imaginary_text == "-0" {
        "0".clone_into(&mut imaginary_text);
    }
    format!(
        "{real}{}{imaginary_text}i",
        if imaginary.is_sign_negative() {
            '-'
        } else {
            '+'
        }
    )
}

fn format_num2str_custom(
    value: NumericElement,
    tokens: &[FormatToken],
) -> Result<Vec<u16>, BuiltinError> {
    let mut output = Vec::new();
    let mut used = false;
    for token in tokens {
        match token {
            FormatToken::Literal(value) => output.extend_from_slice(value),
            FormatToken::Conversion(spec) if !used => {
                let mut spec = *spec;
                if matches!(spec.width, Some(FieldValue::Dynamic))
                    || matches!(spec.precision, Some(FieldValue::Dynamic))
                {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "`num2str` format cannot use dynamic width or precision",
                    ));
                }
                let real = format_numeric_value(NumericElement::real(value.real), spec)?;
                output.extend(real.encode_utf16());
                if let Some(imaginary) = value.imaginary {
                    let imaginary_value = imaginary.as_f64();
                    output.push(u16::from(if imaginary_value.is_sign_negative() {
                        b'-'
                    } else {
                        b'+'
                    }));
                    spec.flags.plus = false;
                    spec.flags.space = false;
                    let imaginary = NumericComponent::Float(imaginary_value.abs());
                    output.extend(
                        format_numeric_value(NumericElement::real(imaginary), spec)?.encode_utf16(),
                    );
                    output.push(u16::from(b'i'));
                }
                used = true;
            }
            FormatToken::Conversion(_) => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`num2str` format must contain exactly one conversion",
                ));
            }
        }
    }
    if !used {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`num2str` format must contain one conversion",
        ));
    }
    Ok(output)
}

fn numeric_elements(value: &Value) -> Result<Vec<NumericElement>, BuiltinError> {
    let values = match value {
        Value::Double(value) => vec![NumericElement::real(NumericComponent::Float(*value))],
        Value::Complex(value) => vec![NumericElement::complex(
            NumericComponent::Float(value.real),
            NumericComponent::Float(value.imaginary),
        )],
        Value::Logical(value) => vec![NumericElement::real(NumericComponent::Unsigned(
            u128::from(*value),
        ))],
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .iter()
            .map(|value| NumericElement::real(NumericComponent::Float(f64::from(*value))))
            .collect(),
        Value::Array(ArrayData::ComplexF32(array)) => array
            .as_slice()
            .iter()
            .map(|value| {
                NumericElement::complex(
                    NumericComponent::Float(f64::from(value.re)),
                    NumericComponent::Float(f64::from(value.im)),
                )
            })
            .collect(),
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .iter()
            .map(|value| NumericElement::real(NumericComponent::Float(*value)))
            .collect(),
        Value::Array(ArrayData::ComplexF64(array)) => array
            .as_slice()
            .iter()
            .map(|value| {
                NumericElement::complex(
                    NumericComponent::Float(value.re),
                    NumericComponent::Float(value.im),
                )
            })
            .collect(),
        Value::Array(ArrayData::Logical(array)) => array
            .as_slice()
            .iter()
            .map(|value| {
                NumericElement::real(NumericComponent::Unsigned(u128::from(bool::from(*value))))
            })
            .collect(),
        Value::Array(ArrayData::Char(array)) => array
            .as_slice()
            .iter()
            .map(|value| NumericElement::real(NumericComponent::Unsigned(u128::from(value.get()))))
            .collect(),
        Value::Array(ArrayData::Integer(array)) => {
            array.elements().map(integer_numeric_element).collect()
        }
        value => {
            return Err(type_error(
                "num2str",
                1,
                "numeric, logical, or character matrix",
                value,
            ));
        }
    };
    Ok(values)
}

fn parse_char_input(value: &DenseArray<CharCodeUnit>) -> ParsedNumber {
    if value.shape().dimensions() == [0, 0] {
        return ParsedNumber::invalid();
    }
    if value.shape().ndims() != 2 || value.shape().extent(0) != 1 {
        return ParsedNumber::invalid();
    }
    parse_utf16(
        &value
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
    )
}

fn parse_string_value(value: &StringValue, context: &BuiltinContext<'_>) -> BuiltinResult {
    match value {
        StringValue::Scalar(value) => Ok(parsed_scalar_output(parse_string_element(value))),
        StringValue::Array(array) => {
            let values = array
                .as_slice()
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    check_cancelled_at(context, index)?;
                    Ok(parse_string_element(value))
                })
                .collect::<Result<Vec<_>, BuiltinError>>()?;
            parsed_array_output(array.shape().clone(), values)
        }
    }
}

fn parse_cell_value(value: &CellArray, context: &BuiltinContext<'_>) -> BuiltinResult {
    let mut values = Vec::with_capacity(value.values().len());
    for (index, element) in value.values().iter().enumerate() {
        check_cancelled_at(context, index)?;
        values.push(match element {
            Value::Array(ArrayData::Char(array)) => parse_char_input(array),
            Value::String(value) => value
                .as_scalar()
                .map_or_else(ParsedNumber::invalid, parse_string_element),
            value => {
                return Err(type_error(
                    "str2double",
                    1,
                    "cell array containing character vectors",
                    value,
                ));
            }
        });
    }
    parsed_array_output(value.shape().clone(), values)
}

fn parse_string_element(value: &StringElement) -> ParsedNumber {
    if value.is_missing() {
        ParsedNumber::invalid()
    } else {
        parse_utf16(value.code_units())
    }
}

#[derive(Clone, Copy, Debug)]
struct ParsedNumber {
    value: Complex64,
    complex: bool,
}

impl ParsedNumber {
    const fn real(value: f64) -> Self {
        Self {
            value: Complex64::new(value, 0.0),
            complex: false,
        }
    }

    const fn complex(real: f64, imaginary: f64) -> Self {
        Self {
            value: Complex64::new(real, imaginary),
            complex: true,
        }
    }

    const fn invalid() -> Self {
        Self::real(f64::NAN)
    }
}

fn parse_utf16(value: &[u16]) -> ParsedNumber {
    String::from_utf16(value)
        .ok()
        .map_or_else(ParsedNumber::invalid, |value| parse_numeric_text(&value))
}

fn parse_numeric_text(value: &str) -> ParsedNumber {
    let normalized = value
        .chars()
        .filter(|character| !character.is_whitespace() && *character != ',' && *character != '*')
        .collect::<String>()
        .to_ascii_lowercase()
        .replace('j', "i");
    if normalized.is_empty() {
        return ParsedNumber::invalid();
    }
    if let Some(value) = parse_real_text(&normalized) {
        return ParsedNumber::real(value);
    }
    if normalized.matches('i').count() != 1 {
        return ParsedNumber::invalid();
    }

    let boundaries = additive_boundaries(&normalized);
    let mut starts = std::iter::once(0)
        .chain(boundaries.iter().copied())
        .collect::<Vec<_>>();
    starts.push(normalized.len());
    let mut real = None;
    let mut imaginary = None;
    for window in starts.windows(2) {
        let term = &normalized[window[0]..window[1]];
        if let Some(term) = term.strip_suffix('i') {
            if imaginary.is_some() {
                return ParsedNumber::invalid();
            }
            imaginary = parse_imaginary_text(term);
            if imaginary.is_none() {
                return ParsedNumber::invalid();
            }
        } else {
            if real.is_some() {
                return ParsedNumber::invalid();
            }
            real = parse_real_text(term);
            if real.is_none() {
                return ParsedNumber::invalid();
            }
        }
    }
    imaginary.map_or_else(ParsedNumber::invalid, |imaginary| {
        let real = real.unwrap_or_else(|| {
            if imaginary.is_sign_negative() {
                -0.0
            } else {
                0.0
            }
        });
        ParsedNumber::complex(real, imaginary)
    })
}

fn additive_boundaries(value: &str) -> Vec<usize> {
    value
        .char_indices()
        .skip(1)
        .filter(|(index, character)| {
            matches!(character, '+' | '-')
                && !matches!(
                    value.as_bytes().get(index.saturating_sub(1)),
                    Some(b'e' | b'E')
                )
        })
        .map(|(index, _)| index)
        .collect()
}

fn parse_imaginary_text(value: &str) -> Option<f64> {
    match value {
        "" | "+" => Some(1.0),
        "-" => Some(-1.0),
        value => parse_real_text(value),
    }
}

fn parse_real_text(value: &str) -> Option<f64> {
    match value {
        "inf" | "+inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        "nan" | "+nan" | "-nan" => Some(f64::NAN),
        _ => parse_prefixed_integer(value).or_else(|| value.parse().ok()),
    }
}

#[allow(clippy::cast_precision_loss)]
fn parse_prefixed_integer(value: &str) -> Option<f64> {
    let (negative, value) = value
        .strip_prefix('-')
        .map_or((false, value), |value| (true, value));
    let value = value.strip_prefix('+').unwrap_or(value);
    let (radix, digits) = value
        .strip_prefix("0x")
        .map(|value| (16, value))
        .or_else(|| value.strip_prefix("0b").map(|value| (2, value)))?;
    let value = u128::from_str_radix(digits, radix).ok()? as f64;
    Some(if negative { -value } else { value })
}

fn parsed_scalar_output(value: ParsedNumber) -> Vec<Value> {
    vec![if value.complex {
        Value::Complex(value.value)
    } else {
        Value::Double(value.value.real)
    }]
}

fn parsed_array_output(shape: Shape, values: Vec<ParsedNumber>) -> BuiltinResult {
    if values.iter().any(|value| value.complex) {
        DenseArray::from_vec(
            shape,
            values
                .into_iter()
                .map(|value| ArrayComplex64::new(value.value.real, value.value.imaginary))
                .collect(),
        )
        .map(ArrayData::ComplexF64)
        .map(Value::Array)
        .map(|value| vec![value])
        .map_err(|error| array_error(&error))
    } else {
        DenseArray::from_vec(
            shape,
            values.into_iter().map(|value| value.value.real).collect(),
        )
        .map(ArrayData::F64)
        .map(Value::Array)
        .map(|value| vec![value])
        .map_err(|error| array_error(&error))
    }
}

fn text_output(value: Vec<u16>, string: bool, empty_row: bool) -> BuiltinResult {
    if string {
        Ok(vec![Value::String(StringValue::scalar(
            StringElement::from_code_units(value),
        ))])
    } else {
        let columns = u64::try_from(value.len()).map_err(|_| matrix_too_large())?;
        let dimensions = if columns == 0 && !empty_row {
            [0, 0]
        } else {
            [1, columns]
        };
        let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
        DenseArray::from_vec(shape, value.into_iter().map(CharCodeUnit::new).collect())
            .map(ArrayData::Char)
            .map(Value::Array)
            .map(|value| vec![value])
            .map_err(|error| array_error(&error))
    }
}

fn char_matrix(rows: &[Vec<u16>], nonempty_rows: bool) -> BuiltinResult {
    if rows.is_empty() {
        return text_output(Vec::new(), false, nonempty_rows);
    }
    let row_count = rows.len();
    let columns = rows.iter().map(Vec::len).max().unwrap_or_default();
    let mut values = Vec::with_capacity(row_count.saturating_mul(columns));
    for column in 0..columns {
        for row in rows {
            values.push(CharCodeUnit::new(
                row.get(column).copied().unwrap_or(u16::from(b' ')),
            ));
        }
    }
    let shape = Shape::new([
        u64::try_from(row_count).map_err(|_| matrix_too_large())?,
        u64::try_from(columns).map_err(|_| matrix_too_large())?,
    ])
    .map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map(|value| vec![value])
        .map_err(|error| array_error(&error))
}

fn matrix_too_large() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "text conversion output is too large for this host",
    )
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

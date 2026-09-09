use std::collections::BTreeSet;

use openmat_array::{ArrayData, DenseArray, IntegerArrayData, Shape};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult, DEFAULT_MAX_FILE_BYTES,
    FileSeekOrigin,
};
use openmat_value::{CellArray, Value};

use crate::{array_error, expect_max_outputs, stream_io, type_error};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConversionKind {
    F64,
    F32,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Text,
    QuotedText,
}

#[derive(Clone, Copy, Debug)]
struct Conversion {
    kind: ConversionKind,
    suppress: bool,
}

#[derive(Debug)]
struct FormatPlan {
    conversions: Vec<Conversion>,
    literal_delimiters: BTreeSet<char>,
}

#[derive(Clone, Debug)]
struct TextScanOptions {
    delimiters: Option<BTreeSet<char>>,
    whitespace: Option<BTreeSet<char>>,
    header_lines: usize,
    multiple_delimiters_as_one: bool,
    collect_output: bool,
    empty_value: f64,
    return_on_error: bool,
    treat_as_empty: BTreeSet<String>,
    comment_marker: Option<String>,
    end_of_line: EndOfLine,
}

#[derive(Clone, Copy, Debug)]
enum EndOfLine {
    Auto,
    Lf,
    Cr,
    CrLf,
}

impl Default for TextScanOptions {
    fn default() -> Self {
        Self {
            delimiters: None,
            whitespace: None,
            header_lines: 0,
            multiple_delimiters_as_one: false,
            collect_output: false,
            empty_value: f64::NAN,
            return_on_error: true,
            treat_as_empty: BTreeSet::new(),
            comment_marker: None,
            end_of_line: EndOfLine::Auto,
        }
    }
}

impl TextScanOptions {
    fn is_whitespace(&self, character: char) -> bool {
        self.whitespace
            .as_ref()
            .map_or_else(|| character.is_whitespace(), |set| set.contains(&character))
    }
}

#[derive(Clone, Copy, Debug)]
struct LineSpan {
    start: usize,
    end: usize,
    next: usize,
}

#[derive(Clone, Copy, Debug)]
struct FieldSpan {
    start: usize,
    end: usize,
    after: usize,
}

#[derive(Debug)]
enum ColumnBuilder {
    F64(Vec<f64>),
    F32(Vec<f32>),
    I8(Vec<i8>),
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    I64(Vec<i64>),
    U64(Vec<u64>),
    Text(Vec<Value>),
}

impl ColumnBuilder {
    fn new(kind: ConversionKind) -> Self {
        match kind {
            ConversionKind::F64 => Self::F64(Vec::new()),
            ConversionKind::F32 => Self::F32(Vec::new()),
            ConversionKind::I8 => Self::I8(Vec::new()),
            ConversionKind::U8 => Self::U8(Vec::new()),
            ConversionKind::I16 => Self::I16(Vec::new()),
            ConversionKind::U16 => Self::U16(Vec::new()),
            ConversionKind::I32 => Self::I32(Vec::new()),
            ConversionKind::U32 => Self::U32(Vec::new()),
            ConversionKind::I64 => Self::I64(Vec::new()),
            ConversionKind::U64 => Self::U64(Vec::new()),
            ConversionKind::Text | ConversionKind::QuotedText => Self::Text(Vec::new()),
        }
    }

    const fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::F64(_), Self::F64(_))
                | (Self::F32(_), Self::F32(_))
                | (Self::I8(_), Self::I8(_))
                | (Self::U8(_), Self::U8(_))
                | (Self::I16(_), Self::I16(_))
                | (Self::U16(_), Self::U16(_))
                | (Self::I32(_), Self::I32(_))
                | (Self::U32(_), Self::U32(_))
                | (Self::I64(_), Self::I64(_))
                | (Self::U64(_), Self::U64(_))
                | (Self::Text(_), Self::Text(_))
        )
    }

    fn len(&self) -> usize {
        match self {
            Self::F64(values) => values.len(),
            Self::F32(values) => values.len(),
            Self::I8(values) => values.len(),
            Self::U8(values) => values.len(),
            Self::I16(values) => values.len(),
            Self::U16(values) => values.len(),
            Self::I32(values) => values.len(),
            Self::U32(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::U64(values) => values.len(),
            Self::Text(values) => values.len(),
        }
    }
}

#[derive(Debug)]
enum ParsedScalar {
    F64(f64),
    F32(f32),
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    Text(Value),
}

#[derive(Debug)]
struct ScanOutcome {
    columns: Vec<ColumnBuilder>,
    consumed: usize,
    failure: Option<BuiltinError>,
}

pub(super) fn textscan_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `textscan` expects at least 2 inputs but received {}",
                arguments.len()
            ),
        ));
    }
    expect_max_outputs("textscan", context, 1)?;
    let format = stream_io::text_scalar("textscan", 2, &arguments[1])?;
    let plan = parse_format(&format)?;
    let (repeat_limit, option_start) = parse_repeat_limit(arguments)?;
    let mut options = parse_options(&arguments[option_start..], option_start + 1)?;
    if options.delimiters.is_none() && !plan.literal_delimiters.is_empty() {
        options.delimiters = Some(plan.literal_delimiters.clone());
    }

    let file_identifier = match &arguments[0] {
        value if is_text_value(value) => None,
        value => Some(stream_io::file_identifier("textscan", 1, value)?),
    };
    let (bytes, bom_bytes) = if let Some(identifier) = file_identifier {
        let info = context.open_file_info(identifier)?;
        if !info.encoding.eq_ignore_ascii_case("UTF-8")
            && !info.encoding.eq_ignore_ascii_case("UTF8")
        {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!(
                    "`textscan` currently requires a UTF-8 stream, but file identifier {identifier} uses {}",
                    info.encoding
                ),
            ));
        }
        let maximum = usize::try_from(DEFAULT_MAX_FILE_BYTES).unwrap_or(usize::MAX);
        let bytes = context.read_open_file(identifier, maximum)?;
        let bom = usize::from(bytes.starts_with(&[0xEF, 0xBB, 0xBF])) * 3;
        (bytes, bom)
    } else {
        let text = stream_io::text_scalar("textscan", 1, &arguments[0])?;
        if text.is_empty() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 1 to `textscan` cannot be empty text",
            ));
        }
        let bytes = text.into_bytes();
        let bom = usize::from(bytes.starts_with(&[0xEF, 0xBB, 0xBF])) * 3;
        (bytes, bom)
    };
    let decoded = match std::str::from_utf8(&bytes[bom_bytes..]) {
        Ok(decoded) => decoded,
        Err(error) => {
            if let Some(identifier) = file_identifier {
                let unread = i64::try_from(bytes.len()).map_err(|_| size_error())?;
                context.seek_open_file(identifier, -unread, FileSeekOrigin::Current)?;
            }
            return Err(BuiltinError::new(
                BuiltinErrorCategory::FileSystem,
                format!(
                    "`textscan` encountered invalid UTF-8 at byte offset {}",
                    bom_bytes + error.valid_up_to()
                ),
            ));
        }
    };
    let mut outcome = scan_text(decoded, &plan, &options, repeat_limit);
    outcome.consumed = outcome.consumed.saturating_add(bom_bytes).min(bytes.len());

    if let Some(identifier) = file_identifier {
        let unread = bytes.len().saturating_sub(outcome.consumed);
        if unread != 0 {
            let offset = i64::try_from(unread).map_err(|_| size_error())?;
            context.seek_open_file(identifier, -offset, FileSeekOrigin::Current)?;
        }
    }
    if let Some(error) = outcome.failure {
        return Err(error);
    }

    let values = build_outputs(outcome.columns, options.collect_output)?;
    let width = u64::try_from(values.len()).map_err(|_| size_error())?;
    let shape = Shape::new([1, width]).map_err(|error| array_error(&error))?;
    let output = CellArray::from_values(shape, values)
        .map(Value::Cell)
        .map_err(|error| aggregate_error(&error))?;
    Ok(stream_io::requested_outputs(vec![output], context))
}

fn parse_repeat_limit(arguments: &[Value]) -> Result<(Option<usize>, usize), BuiltinError> {
    let Some(value) = arguments.get(2) else {
        return Ok((None, 2));
    };
    if is_text_value(value) {
        return Ok((None, 2));
    }
    let value = stream_io::numeric_scalar("textscan", 3, value)?;
    if value.is_infinite() && value.is_sign_positive() {
        return Ok((None, 3));
    }
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 3 to `textscan` must be a nonnegative integer or Inf",
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let value = value as u128;
    usize::try_from(value)
        .map(Some)
        .map(|limit| (limit, 3))
        .map_err(|_| size_error())
}

fn parse_options(
    arguments: &[Value],
    first_position: usize,
) -> Result<TextScanOptions, BuiltinError> {
    if !arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`textscan` options must be name-value pairs",
        ));
    }
    let mut options = TextScanOptions::default();
    for (pair, values) in arguments.chunks_exact(2).enumerate() {
        let name_position = first_position + pair * 2;
        let value_position = name_position + 1;
        let name = stream_io::text_scalar("textscan", name_position, &values[0])?;
        match name.to_ascii_lowercase().as_str() {
            "delimiter" => {
                let delimiters = text_list(value_position, &values[1])?;
                let characters = delimiters
                    .iter()
                    .flat_map(|value| value.chars())
                    .collect::<BTreeSet<_>>();
                if characters.is_empty() {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "the `Delimiter` option cannot be empty",
                    ));
                }
                options.delimiters = Some(characters);
            }
            "whitespace" => {
                let text = stream_io::text_scalar("textscan", value_position, &values[1])?;
                options.whitespace = Some(text.chars().collect());
            }
            "headerlines" => {
                options.header_lines = nonnegative_integer(value_position, &values[1])?;
            }
            "multipledelimsasone" => {
                options.multiple_delimiters_as_one = logical_scalar(value_position, &values[1])?;
            }
            "collectoutput" => {
                options.collect_output = logical_scalar(value_position, &values[1])?;
            }
            "emptyvalue" => {
                options.empty_value =
                    stream_io::numeric_scalar("textscan", value_position, &values[1])?;
            }
            "returnonerror" => {
                options.return_on_error = logical_scalar(value_position, &values[1])?;
            }
            "treatasempty" => {
                options.treat_as_empty =
                    text_list(value_position, &values[1])?.into_iter().collect();
            }
            "commentstyle" => {
                let markers = text_list(value_position, &values[1])?;
                if markers.len() != 1 || markers[0].is_empty() {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "this `textscan` tranche supports one nonempty line-comment marker",
                    ));
                }
                options.comment_marker = markers.into_iter().next();
            }
            "endofline" => {
                let value = stream_io::text_scalar("textscan", value_position, &values[1])?;
                options.end_of_line = match value.as_str() {
                    "" => EndOfLine::Auto,
                    "\n" => EndOfLine::Lf,
                    "\r" => EndOfLine::Cr,
                    "\r\n" => EndOfLine::CrLf,
                    _ => {
                        return Err(BuiltinError::new(
                            BuiltinErrorCategory::Domain,
                            "custom `EndOfLine` sequences are not implemented yet",
                        ));
                    }
                };
            }
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("`{name}` is not a supported `textscan` option"),
                ));
            }
        }
    }
    Ok(options)
}

fn parse_format(format: &str) -> Result<FormatPlan, BuiltinError> {
    if format.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the `textscan` format cannot be empty",
        ));
    }
    let characters = format.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut conversions = Vec::new();
    let mut literal_delimiters = BTreeSet::new();
    while index < characters.len() {
        let character = characters[index];
        if character != '%' {
            if !character.is_whitespace() {
                if character.is_alphanumeric() {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "alphanumeric literal matching in `textscan` formats is not implemented yet",
                    ));
                }
                literal_delimiters.insert(character);
            }
            index += 1;
            continue;
        }
        index += 1;
        if characters.get(index) == Some(&'%') {
            literal_delimiters.insert('%');
            index += 1;
            continue;
        }
        let suppress = characters.get(index) == Some(&'*');
        if suppress {
            index += 1;
        }
        let width_start = index;
        while characters.get(index).is_some_and(char::is_ascii_digit) {
            index += 1;
        }
        if index != width_start {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "field-width `textscan` conversions are not implemented yet",
            ));
        }
        let Some(specifier) = characters.get(index).copied() else {
            return Err(format_error(format));
        };
        index += 1;
        let suffix_start = index;
        while characters.get(index).is_some_and(char::is_ascii_digit) {
            index += 1;
        }
        let suffix = characters[suffix_start..index].iter().collect::<String>();
        let kind = match (specifier, suffix.as_str()) {
            ('f', "" | "64") => ConversionKind::F64,
            ('f', "32") => ConversionKind::F32,
            ('d', "" | "32") => ConversionKind::I32,
            ('d', "8") => ConversionKind::I8,
            ('d', "16") => ConversionKind::I16,
            ('d', "64") => ConversionKind::I64,
            ('u', "" | "32") => ConversionKind::U32,
            ('u', "8") => ConversionKind::U8,
            ('u', "16") => ConversionKind::U16,
            ('u', "64") => ConversionKind::U64,
            ('s', "") => ConversionKind::Text,
            ('q', "") => ConversionKind::QuotedText,
            _ => return Err(format_error(format)),
        };
        conversions.try_reserve(1).map_err(|_| size_error())?;
        conversions.push(Conversion { kind, suppress });
    }
    if conversions.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the `textscan` format must contain at least one conversion",
        ));
    }
    Ok(FormatPlan {
        conversions,
        literal_delimiters,
    })
}

fn scan_text(
    text: &str,
    plan: &FormatPlan,
    options: &TextScanOptions,
    repeat_limit: Option<usize>,
) -> ScanOutcome {
    let mut columns = plan
        .conversions
        .iter()
        .filter(|conversion| !conversion.suppress)
        .map(|conversion| ColumnBuilder::new(conversion.kind))
        .collect::<Vec<_>>();
    if repeat_limit == Some(0) {
        return ScanOutcome {
            columns,
            consumed: 0,
            failure: None,
        };
    }
    let lines = line_spans(text, options.end_of_line);
    let mut consumed = 0;
    let mut repeats = 0;
    for (line_index, line) in lines.iter().copied().enumerate() {
        if line_index < options.header_lines {
            consumed = line.next;
            continue;
        }
        let effective_end = comment_start(text, line, options.comment_marker.as_deref());
        if text[line.start..effective_end]
            .chars()
            .all(|character| options.is_whitespace(character))
        {
            consumed = line.next;
            continue;
        }
        let fields = split_fields(text, line.start, effective_end, plan, options);
        if fields.is_empty() {
            consumed = line.next;
            continue;
        }
        let input_width = plan.conversions.len();
        for chunk_start in (0..fields.len()).step_by(input_width) {
            let chunk = &fields[chunk_start..fields.len().min(chunk_start + input_width)];
            let mut output_index = 0;
            for (conversion_index, conversion) in plan.conversions.iter().copied().enumerate() {
                let field = chunk.get(conversion_index).copied();
                let token = field.map(|field| field_text(text, field, options));
                match parse_scalar(token.as_deref(), conversion.kind, options) {
                    Ok(value) => {
                        if !conversion.suppress {
                            push_scalar(&mut columns[output_index], value);
                            output_index += 1;
                        }
                        consumed = field.map_or(effective_end, |field| field.after);
                    }
                    Err(_) if options.return_on_error => {
                        consumed = field.map_or(effective_end, |field| {
                            trimmed_token_start(text, field, options)
                        });
                        return ScanOutcome {
                            columns,
                            consumed,
                            failure: None,
                        };
                    }
                    Err(error) => {
                        consumed = field.map_or(effective_end, |field| {
                            trimmed_token_start(text, field, options)
                        });
                        return ScanOutcome {
                            columns,
                            consumed,
                            failure: Some(
                                error.with_identifier("MATLAB:textscan:handleErrorAndShowInfo"),
                            ),
                        };
                    }
                }
            }
            repeats += 1;
            if repeat_limit == Some(repeats) {
                return ScanOutcome {
                    columns,
                    consumed,
                    failure: None,
                };
            }
        }
        consumed = line.next;
    }
    ScanOutcome {
        columns,
        consumed: text.len(),
        failure: None,
    }
}

fn line_spans(text: &str, end_of_line: EndOfLine) -> Vec<LineSpan> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let width = match end_of_line {
            EndOfLine::Auto if bytes[index] == b'\r' => {
                usize::from(bytes.get(index + 1) == Some(&b'\n')) + 1
            }
            EndOfLine::Auto | EndOfLine::Lf if bytes[index] == b'\n' => 1,
            EndOfLine::Cr if bytes[index] == b'\r' => 1,
            EndOfLine::CrLf if bytes[index..].starts_with(b"\r\n") => 2,
            _ => {
                index += 1;
                continue;
            }
        };
        let mut end = index;
        if matches!(end_of_line, EndOfLine::Lf)
            && bytes.get(index.wrapping_sub(1)) == Some(&b'\r')
            && index > start
        {
            end -= 1;
        }
        let next = index + width;
        lines.push(LineSpan { start, end, next });
        start = next;
        index = next;
    }
    if start < text.len() {
        lines.push(LineSpan {
            start,
            end: text.len(),
            next: text.len(),
        });
    }
    lines
}

fn comment_start(text: &str, line: LineSpan, marker: Option<&str>) -> usize {
    let Some(marker) = marker else {
        return line.end;
    };
    let content = &text[line.start..line.end];
    let mut quoted = false;
    let mut offset = 0;
    while offset < content.len() {
        let remainder = &content[offset..];
        if remainder.starts_with('"') {
            quoted = !quoted;
            offset += 1;
            continue;
        }
        if !quoted && remainder.starts_with(marker) {
            return line.start + offset;
        }
        offset += remainder.chars().next().map_or(1, char::len_utf8);
    }
    line.end
}

fn split_fields(
    text: &str,
    start: usize,
    end: usize,
    plan: &FormatPlan,
    options: &TextScanOptions,
) -> Vec<FieldSpan> {
    if let Some(delimiters) = &options.delimiters {
        return split_delimited_fields(
            text,
            start,
            end,
            delimiters,
            options.multiple_delimiters_as_one,
            plan.conversions
                .iter()
                .any(|conversion| conversion.kind == ConversionKind::QuotedText),
        );
    }
    split_whitespace_fields(
        text,
        start,
        end,
        options,
        plan.conversions
            .iter()
            .any(|conversion| conversion.kind == ConversionKind::QuotedText),
    )
}

fn split_delimited_fields(
    text: &str,
    start: usize,
    end: usize,
    delimiters: &BTreeSet<char>,
    multiple_as_one: bool,
    quote_aware: bool,
) -> Vec<FieldSpan> {
    let mut fields = Vec::new();
    let mut field_start = start;
    let mut quoted = false;
    for (relative, character) in text[start..end].char_indices() {
        let absolute = start + relative;
        if quote_aware && character == '"' {
            quoted = !quoted;
            continue;
        }
        if !quoted && delimiters.contains(&character) {
            let after = absolute + character.len_utf8();
            if !multiple_as_one || absolute != field_start {
                fields.push(FieldSpan {
                    start: field_start,
                    end: absolute,
                    after,
                });
            }
            field_start = after;
        }
    }
    if !multiple_as_one || field_start != end {
        fields.push(FieldSpan {
            start: field_start,
            end,
            after: end,
        });
    }
    fields
}

fn split_whitespace_fields(
    text: &str,
    start: usize,
    end: usize,
    options: &TextScanOptions,
    quote_aware: bool,
) -> Vec<FieldSpan> {
    let mut fields = Vec::new();
    let mut field_start = None;
    let mut previous_end = start;
    let mut quoted = false;
    for (relative, character) in text[start..end].char_indices() {
        let absolute = start + relative;
        let character_end = absolute + character.len_utf8();
        if quote_aware && character == '"' {
            quoted = !quoted;
            if field_start.is_none() {
                field_start = Some(absolute);
            }
        } else if !quoted && options.is_whitespace(character) {
            if let Some(field_start) = field_start.take() {
                fields.push(FieldSpan {
                    start: field_start,
                    end: absolute,
                    after: character_end,
                });
            } else if let Some(last) = fields.last_mut() {
                last.after = character_end;
            }
        } else if field_start.is_none() {
            field_start = Some(absolute);
        }
        previous_end = character_end;
    }
    if let Some(field_start) = field_start {
        fields.push(FieldSpan {
            start: field_start,
            end: previous_end,
            after: previous_end,
        });
    }
    fields
}

fn field_text(text: &str, field: FieldSpan, options: &TextScanOptions) -> String {
    text[field.start..field.end]
        .trim_start_matches(|character| options.is_whitespace(character))
        .to_owned()
}

fn trimmed_token_start(text: &str, field: FieldSpan, options: &TextScanOptions) -> usize {
    let raw = &text[field.start..field.end];
    let trimmed = raw.trim_start_matches(|character| options.is_whitespace(character));
    field.end - trimmed.len()
}

fn parse_scalar(
    token: Option<&str>,
    kind: ConversionKind,
    options: &TextScanOptions,
) -> Result<ParsedScalar, BuiltinError> {
    if matches!(kind, ConversionKind::Text | ConversionKind::QuotedText) {
        let text = token.unwrap_or("");
        let text = if kind == ConversionKind::QuotedText {
            unquote(text.trim_end_matches(|character| options.is_whitespace(character)))
        } else {
            text.to_owned()
        };
        return stream_io::char_text("textscan", &text).map(ParsedScalar::Text);
    }
    let token = token.unwrap_or("").trim();
    let empty = token.is_empty() || options.treat_as_empty.contains(token);
    let number = if empty {
        options.empty_value
    } else {
        parse_number(token).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`textscan` cannot convert `{token}` using the requested numeric format"),
            )
        })?
    };
    Ok(match kind {
        ConversionKind::F64 => ParsedScalar::F64(number),
        ConversionKind::F32 => ParsedScalar::F32(narrow_f32(number)),
        ConversionKind::I8 => ParsedScalar::I8(signed_integer(number, i8::MIN, i8::MAX)),
        ConversionKind::U8 => ParsedScalar::U8(unsigned_integer(number, u8::MAX)),
        ConversionKind::I16 => ParsedScalar::I16(signed_integer(number, i16::MIN, i16::MAX)),
        ConversionKind::U16 => ParsedScalar::U16(unsigned_integer(number, u16::MAX)),
        ConversionKind::I32 => ParsedScalar::I32(signed_integer(number, i32::MIN, i32::MAX)),
        ConversionKind::U32 => ParsedScalar::U32(unsigned_integer(number, u32::MAX)),
        ConversionKind::I64 => ParsedScalar::I64(signed_integer(number, i64::MIN, i64::MAX)),
        ConversionKind::U64 => ParsedScalar::U64(unsigned_integer(number, u64::MAX)),
        ConversionKind::Text | ConversionKind::QuotedText => unreachable!(),
    })
}

fn parse_number(token: &str) -> Option<f64> {
    let normalized = if token.contains(['d', 'D']) {
        token
            .chars()
            .map(|character| match character {
                'd' | 'D' => 'e',
                character => character,
            })
            .collect::<String>()
    } else {
        token.to_owned()
    };
    normalized.parse().ok()
}

fn unquote(token: &str) -> String {
    let token = token.trim_start();
    if token.len() >= 2 && token.starts_with('"') && token.ends_with('"') {
        token[1..token.len() - 1].replace("\"\"", "\"")
    } else {
        token.to_owned()
    }
}

fn push_scalar(column: &mut ColumnBuilder, scalar: ParsedScalar) {
    match (column, scalar) {
        (ColumnBuilder::F64(values), ParsedScalar::F64(value)) => values.push(value),
        (ColumnBuilder::F32(values), ParsedScalar::F32(value)) => values.push(value),
        (ColumnBuilder::I8(values), ParsedScalar::I8(value)) => values.push(value),
        (ColumnBuilder::U8(values), ParsedScalar::U8(value)) => values.push(value),
        (ColumnBuilder::I16(values), ParsedScalar::I16(value)) => values.push(value),
        (ColumnBuilder::U16(values), ParsedScalar::U16(value)) => values.push(value),
        (ColumnBuilder::I32(values), ParsedScalar::I32(value)) => values.push(value),
        (ColumnBuilder::U32(values), ParsedScalar::U32(value)) => values.push(value),
        (ColumnBuilder::I64(values), ParsedScalar::I64(value)) => values.push(value),
        (ColumnBuilder::U64(values), ParsedScalar::U64(value)) => values.push(value),
        (ColumnBuilder::Text(values), ParsedScalar::Text(value)) => values.push(value),
        _ => unreachable!("format plan and output column must have the same type"),
    }
}

fn build_outputs(
    columns: Vec<ColumnBuilder>,
    collect_output: bool,
) -> Result<Vec<Value>, BuiltinError> {
    if !collect_output {
        return columns
            .into_iter()
            .map(|column| build_group(vec![column]))
            .collect();
    }
    let mut groups: Vec<Vec<ColumnBuilder>> = Vec::new();
    for column in columns {
        if let Some(group) = groups.last_mut()
            && group[0].same_kind(&column)
            && group[0].len() == column.len()
        {
            group.push(column);
        } else {
            groups.push(vec![column]);
        }
    }
    groups.into_iter().map(build_group).collect()
}

fn build_group(group: Vec<ColumnBuilder>) -> Result<Value, BuiltinError> {
    let rows = group.first().map_or(0, ColumnBuilder::len);
    let columns = group.len();
    let shape = Shape::new([
        u64::try_from(rows).map_err(|_| size_error())?,
        u64::try_from(columns).map_err(|_| size_error())?,
    ])
    .map_err(|error| array_error(&error))?;
    macro_rules! numeric_group {
        ($variant:ident, $wrap:expr) => {{
            let mut output = Vec::new();
            output
                .try_reserve_exact(rows.saturating_mul(columns))
                .map_err(|_| size_error())?;
            for column in group {
                let ColumnBuilder::$variant(values) = column else {
                    unreachable!();
                };
                output.extend(values);
            }
            DenseArray::from_vec(shape, output)
                .map($wrap)
                .map_err(|error| array_error(&error))
        }};
    }
    match &group[0] {
        ColumnBuilder::F64(_) => {
            numeric_group!(F64, |array| Value::Array(ArrayData::F64(array)))
        }
        ColumnBuilder::F32(_) => {
            numeric_group!(F32, |array| { Value::Array(ArrayData::F32(array)) })
        }
        ColumnBuilder::I8(_) => numeric_group!(I8, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::I8(array)))
        }),
        ColumnBuilder::U8(_) => numeric_group!(U8, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::U8(array)))
        }),
        ColumnBuilder::I16(_) => numeric_group!(I16, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::I16(array)))
        }),
        ColumnBuilder::U16(_) => numeric_group!(U16, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::U16(array)))
        }),
        ColumnBuilder::I32(_) => numeric_group!(I32, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::I32(array)))
        }),
        ColumnBuilder::U32(_) => numeric_group!(U32, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::U32(array)))
        }),
        ColumnBuilder::I64(_) => numeric_group!(I64, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::I64(array)))
        }),
        ColumnBuilder::U64(_) => numeric_group!(U64, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::U64(array)))
        }),
        ColumnBuilder::Text(_) => {
            let mut output = Vec::new();
            output
                .try_reserve_exact(rows.saturating_mul(columns))
                .map_err(|_| size_error())?;
            for column in group {
                let ColumnBuilder::Text(values) = column else {
                    unreachable!();
                };
                output.extend(values);
            }
            CellArray::from_values(shape, output)
                .map(Value::Cell)
                .map_err(|error| aggregate_error(&error))
        }
    }
}

fn text_list(position: usize, value: &Value) -> Result<Vec<String>, BuiltinError> {
    if let Value::Cell(cell) = value {
        return cell
            .values()
            .iter()
            .map(|value| stream_io::text_scalar("textscan", position, value))
            .collect();
    }
    stream_io::text_scalar("textscan", position, value).map(|value| vec![value])
}

fn logical_scalar(position: usize, value: &Value) -> Result<bool, BuiltinError> {
    match value {
        Value::Logical(value) => Ok(*value),
        Value::Double(value) if *value == 0.0 => Ok(false),
        Value::Double(value) if value.to_bits() == 1.0_f64.to_bits() => Ok(true),
        Value::Array(ArrayData::Logical(array)) if array.numel() == 1 => {
            Ok(array.as_slice()[0].get())
        }
        _ => Err(type_error("textscan", position, "logical scalar", value)),
    }
}

fn nonnegative_integer(position: usize, value: &Value) -> Result<usize, BuiltinError> {
    let value = stream_io::numeric_scalar("textscan", position, value)?;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `textscan` must be a nonnegative integer scalar"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let value = value as u128;
    usize::try_from(value).map_err(|_| size_error())
}

fn is_text_value(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Array(ArrayData::Char(_)))
}

fn format_error(format: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{format}` is not a supported `textscan` format"),
    )
}

fn aggregate_error(error: &openmat_value::AggregateError) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string())
}

fn size_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "the requested `textscan` result exceeds session or host limits",
    )
}

#[allow(clippy::cast_possible_truncation)]
fn narrow_f32(value: f64) -> f32 {
    value as f32
}

trait SignedTarget: Sized {
    const MINIMUM: f64;
    const MAXIMUM: f64;
    fn from_clamped(value: f64) -> Self;
}

trait UnsignedTarget: Sized {
    const MAXIMUM: f64;
    fn from_clamped(value: f64) -> Self;
}

macro_rules! signed_target {
    ($type:ty) => {
        #[allow(clippy::cast_precision_loss)]
        impl SignedTarget for $type {
            const MINIMUM: f64 = <$type>::MIN as f64;
            const MAXIMUM: f64 = <$type>::MAX as f64;

            #[allow(clippy::cast_possible_truncation)]
            fn from_clamped(value: f64) -> Self {
                value as Self
            }
        }
    };
}

macro_rules! unsigned_target {
    ($type:ty) => {
        #[allow(clippy::cast_precision_loss)]
        impl UnsignedTarget for $type {
            const MAXIMUM: f64 = <$type>::MAX as f64;

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            fn from_clamped(value: f64) -> Self {
                value as Self
            }
        }
    };
}

signed_target!(i8);
signed_target!(i16);
signed_target!(i32);
signed_target!(i64);
unsigned_target!(u8);
unsigned_target!(u16);
unsigned_target!(u32);
unsigned_target!(u64);

fn signed_integer<T: SignedTarget>(value: f64, _minimum: T, _maximum: T) -> T {
    let value = if value.is_nan() { 0.0 } else { value.round() };
    T::from_clamped(value.clamp(T::MINIMUM, T::MAXIMUM))
}

fn unsigned_integer<T: UnsignedTarget>(value: f64, _maximum: T) -> T {
    let value = if value.is_nan() { 0.0 } else { value.round() };
    T::from_clamped(value.clamp(0.0, T::MAXIMUM))
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmat_runtime::{CancellationToken, LocalFileSystem, NullOutput};

    fn char_value(text: &str) -> Value {
        stream_io::char_text("test", text).unwrap()
    }

    fn call(
        arguments: &[Value],
        outputs: usize,
        files: Option<&mut LocalFileSystem>,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        if let Some(files) = files {
            let mut context = BuiltinContext::with_file_system_service(
                outputs,
                &cancellation,
                &mut output,
                files,
            );
            textscan_builtin(arguments, &mut context)
        } else {
            let mut context = BuiltinContext::new(outputs, &cancellation, &mut output);
            textscan_builtin(arguments, &mut context)
        }
    }

    fn outer_values(result: &[Value]) -> &[Value] {
        let Value::Cell(cell) = &result[0] else {
            panic!("textscan must return a cell row");
        };
        cell.values()
    }

    #[test]
    fn core_formats_preserve_column_types_and_suppression() {
        let single = call(&[char_value("1"), char_value("%f")], 1, None).unwrap();
        assert!(
            matches!(&outer_values(&single)[0], Value::Array(ArrayData::F64(array)) if array.as_slice() == [1.0])
        );

        let input = char_value("1.5 alpha -2 3.25 7\n4.5 beta 8 9.5 10\n");
        let result = call(&[input, char_value("%f %s %d %f32 %u16")], 1, None).unwrap();
        let values = outer_values(&result);
        assert_eq!(values.len(), 5);
        let Value::Array(ArrayData::F64(first)) = &values[0] else {
            panic!("expected double column");
        };
        assert_eq!(first.as_slice(), &[1.5, 4.5]);
        let Value::Cell(text) = &values[1] else {
            panic!("expected cellstr column");
        };
        assert_eq!(text.shape().dimensions(), &[2, 1]);
        let Value::Array(ArrayData::Integer(IntegerArrayData::I32(integer))) = &values[2] else {
            panic!("expected int32 column");
        };
        assert_eq!(integer.as_slice(), &[-2, 8]);
        assert!(
            matches!(&values[3], Value::Array(ArrayData::F32(array)) if array.as_slice() == [3.25, 9.5])
        );
        assert!(
            matches!(&values[4], Value::Array(ArrayData::Integer(IntegerArrayData::U16(array))) if array.as_slice() == [7, 10])
        );

        let skipped = call(&[char_value("1 x\n2 y\n"), char_value("%*f %s")], 1, None).unwrap();
        assert_eq!(outer_values(&skipped).len(), 1);

        let quoted = call(
            &[char_value("1 \"two words\"\n"), char_value("%f %q")],
            1,
            None,
        )
        .unwrap();
        let Value::Cell(words) = &outer_values(&quoted)[1] else {
            panic!("expected quoted text column");
        };
        assert_eq!(words.values(), &[char_value("two words")]);
    }

    #[test]
    fn csv_empty_header_and_collect_options_match_observed_shapes() {
        let result = call(
            &[
                char_value("header\n1,,3\n4,5,\n"),
                char_value("%f%f%f"),
                char_value("Delimiter"),
                char_value(","),
                char_value("HeaderLines"),
                Value::Double(1.0),
                char_value("EmptyValue"),
                Value::Double(-9.0),
                char_value("CollectOutput"),
                Value::Logical(true),
            ],
            1,
            None,
        )
        .unwrap();
        let values = outer_values(&result);
        assert_eq!(values.len(), 1);
        let Value::Array(ArrayData::F64(matrix)) = &values[0] else {
            panic!("expected collected double matrix");
        };
        assert_eq!(matrix.shape().dimensions(), &[2, 3]);
        assert_eq!(matrix.as_slice(), &[1.0, 4.0, -9.0, 5.0, 3.0, -9.0]);

        let carriage_returns = call(
            &[
                char_value("1,2\r3,4\r"),
                char_value("%f%f"),
                char_value("Delimiter"),
                char_value(","),
                char_value("EndOfLine"),
                char_value("\r"),
                char_value("CollectOutput"),
                Value::Logical(true),
            ],
            1,
            None,
        )
        .unwrap();
        assert!(
            matches!(&outer_values(&carriage_returns)[0], Value::Array(ArrayData::F64(array)) if array.as_slice() == [1.0, 3.0, 2.0, 4.0])
        );
    }

    #[test]
    fn finite_file_scan_restores_unread_bytes_to_the_stream_position() {
        let directory = std::env::temp_dir().join(format!(
            "openmat-textscan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("data.txt"),
            b"1.5 alpha -2 3.25 7\n4.5 beta 8 9.5 10\n",
        )
        .unwrap();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let open = super::stream_io::fopen_builtin;
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        {
            let mut context =
                BuiltinContext::with_file_system_service(1, &cancellation, &mut output, &mut files);
            let identifier =
                open(&[char_value("data.txt"), char_value("r")], &mut context).unwrap();
            assert_eq!(identifier, vec![Value::Double(3.0)]);
        }

        let result = call(
            &[
                Value::Double(3.0),
                char_value("%f %s %d %f32 %u16"),
                Value::Double(1.0),
            ],
            1,
            Some(&mut files),
        )
        .unwrap();
        assert_eq!(outer_values(&result).len(), 5);
        {
            let mut context =
                BuiltinContext::with_file_system_service(1, &cancellation, &mut output, &mut files);
            assert_eq!(context.tell_open_file(3).unwrap(), 19);
            context.close_open_file(3).unwrap();
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn return_on_error_keeps_completed_columns_and_empty_input_columns_are_zero_by_one() {
        let partial = call(
            &[char_value("1 2\n3 bad\n4 5\n"), char_value("%f%f")],
            1,
            None,
        )
        .unwrap();
        let columns = outer_values(&partial);
        assert!(
            matches!(&columns[0], Value::Array(ArrayData::F64(array)) if array.as_slice() == [1.0, 3.0])
        );
        assert!(
            matches!(&columns[1], Value::Array(ArrayData::F64(array)) if array.as_slice() == [2.0])
        );

        let directory = std::env::temp_dir().join(format!(
            "openmat-textscan-empty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("empty.txt"), []).unwrap();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        {
            let mut context =
                BuiltinContext::with_file_system_service(1, &cancellation, &mut output, &mut files);
            super::stream_io::fopen_builtin(
                &[char_value("empty.txt"), char_value("r")],
                &mut context,
            )
            .unwrap();
        }
        let empty = call(
            &[Value::Double(3.0), char_value("%f%s%d")],
            1,
            Some(&mut files),
        )
        .unwrap();
        for value in outer_values(&empty) {
            assert_eq!(value.dimensions(), Some([0, 1].as_slice()));
        }
        {
            let mut context =
                BuiltinContext::with_file_system_service(0, &cancellation, &mut output, &mut files);
            context.close_open_file(3).unwrap();
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn return_on_error_false_stops_the_file_at_the_invalid_token() {
        let directory = std::env::temp_dir().join(format!(
            "openmat-textscan-error-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("data.txt"), b"1 2\n3 bad\n4 5\n").unwrap();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        {
            let mut context =
                BuiltinContext::with_file_system_service(1, &cancellation, &mut output, &mut files);
            super::stream_io::fopen_builtin(
                &[char_value("data.txt"), char_value("r")],
                &mut context,
            )
            .unwrap();
        }

        let error = call(
            &[
                Value::Double(3.0),
                char_value("%f%f"),
                char_value("ReturnOnError"),
                Value::Logical(false),
            ],
            1,
            Some(&mut files),
        )
        .unwrap_err();
        assert_eq!(
            error.identifier.as_deref(),
            Some("MATLAB:textscan:handleErrorAndShowInfo")
        );
        {
            let mut context =
                BuiltinContext::with_file_system_service(0, &cancellation, &mut output, &mut files);
            assert_eq!(context.tell_open_file(3).unwrap(), 6);
            context.close_open_file(3).unwrap();
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

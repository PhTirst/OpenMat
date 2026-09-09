use openmat_array::{
    ArrayData, CharCodeUnit, DenseArray, IntegerArrayData, IntegerComponent, Shape,
};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult, DEFAULT_MAX_FILE_BYTES,
    FileOpenAccess, FileOpenMode, FileSeekOrigin,
};
use openmat_value::Value;

use crate::{
    array_error, exact_real_integer_scalar, expect_argument_count, expect_argument_count_range,
    expect_max_outputs, type_error,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ByteOrder {
    Little,
    Big,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrimitiveKind {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl PrimitiveKind {
    const fn width(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ReadPrecision {
    source: PrimitiveKind,
    destination: PrimitiveKind,
}

#[derive(Clone, Copy, Debug)]
enum NumericValue {
    Signed(i128),
    Unsigned(u128),
    Float(f64),
}

#[derive(Clone, Copy, Debug)]
enum ReadSize {
    Column(Option<usize>),
    Matrix { rows: usize, columns: Option<usize> },
}

impl ReadSize {
    fn maximum_elements(self) -> Option<usize> {
        match self {
            Self::Column(elements) => elements,
            Self::Matrix { rows, columns } => columns.and_then(|columns| rows.checked_mul(columns)),
        }
    }

    fn output_shape(self, count: usize) -> Result<(Shape, usize), BuiltinError> {
        if count == 0 {
            return Shape::new([0, 0])
                .map(|shape| (shape, 0))
                .map_err(|error| array_error(&error));
        }
        let (dimensions, elements) = match self {
            Self::Column(_) => ([count, 1], count),
            Self::Matrix { rows: 0, .. } => ([0, 0], 0),
            Self::Matrix { rows, columns } => {
                let used_columns = count.div_ceil(rows).min(columns.unwrap_or(usize::MAX));
                let elements = rows.checked_mul(used_columns).ok_or_else(|| {
                    BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "the requested `fread` output shape exceeds host limits",
                    )
                })?;
                ([rows, used_columns], elements)
            }
        };
        let rows = u64::try_from(dimensions[0]).map_err(|_| size_limit_error("fread"))?;
        let columns = u64::try_from(dimensions[1]).map_err(|_| size_limit_error("fread"))?;
        Shape::new([rows, columns])
            .map(|shape| (shape, elements))
            .map_err(|error| array_error(&error))
    }
}

pub(super) fn fopen_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("fopen", arguments, 1, 4)?;
    if arguments.len() == 1 {
        if let Some(identifier) = optional_file_identifier(&arguments[0])? {
            expect_max_outputs("fopen", context, 4)?;
            let outputs = match context.open_file_info(identifier) {
                Ok(info) => vec![
                    char_text("fopen", &info.filename)?,
                    char_text("fopen", &info.permission)?,
                    char_text("fopen", &info.machine_format)?,
                    char_text("fopen", &info.encoding)?,
                ],
                Err(error) if error.identifier.as_deref() == Some("MATLAB:badfid_mx") => vec![
                    char_text("fopen", "")?,
                    char_text("fopen", "")?,
                    char_text("fopen", "")?,
                    char_text("fopen", "")?,
                ],
                Err(error) => return Err(error),
            };
            return Ok(requested_outputs(outputs, context));
        }
        let path = text_scalar("fopen", 1, &arguments[0])?;
        if path.eq_ignore_ascii_case("all") {
            expect_max_outputs("fopen", context, 1)?;
            let identifiers = context.open_file_identifiers()?;
            return Ok(requested_outputs(
                vec![identifier_row(&identifiers)?],
                context,
            ));
        }
    }

    expect_max_outputs("fopen", context, 2)?;
    let path = text_scalar("fopen", 1, &arguments[0])?;
    let permission = arguments
        .get(1)
        .map(|value| text_scalar("fopen", 2, value))
        .transpose()?;
    let mode = match permission {
        Some(permission) => parse_open_mode(&permission)?,
        None => default_open_mode(),
    };
    let machine_format = arguments
        .get(2)
        .map(|value| text_scalar("fopen", 3, value))
        .transpose()?;
    let machine_format = match machine_format {
        Some(value) => canonical_machine_format(&value)?.to_owned(),
        None => default_machine_format(),
    };
    let encoding = arguments
        .get(3)
        .map(|value| text_scalar("fopen", 4, value))
        .transpose()?;
    let encoding = match encoding {
        Some(value) => canonical_encoding(value)?,
        None => "UTF-8".to_owned(),
    };

    match context.open_file(&path, mode, &machine_format, &encoding) {
        Ok(identifier) => Ok(requested_outputs(
            vec![
                Value::Double(f64::from(identifier)),
                char_text("fopen", "")?,
            ],
            context,
        )),
        Err(error) if error.category == BuiltinErrorCategory::FileSystem => {
            let message = error.message;
            Ok(requested_outputs(
                vec![Value::Double(-1.0), char_text("fopen", &message)?],
                context,
            ))
        }
        Err(error) => Err(error),
    }
}

pub(super) fn fclose_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("fclose", arguments, 1)?;
    expect_max_outputs("fclose", context, 1)?;
    if text_scalar_if_present(&arguments[0]).is_some_and(|value| value.eq_ignore_ascii_case("all"))
    {
        context.close_all_open_files()?;
    } else {
        context.close_open_file(file_identifier("fclose", 1, &arguments[0])?)?;
    }
    Ok(requested_outputs(vec![Value::Double(0.0)], context))
}

pub(super) fn ftell_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("ftell", arguments, 1)?;
    expect_max_outputs("ftell", context, 1)?;
    let identifier = file_identifier("ftell", 1, &arguments[0])?;
    let position = context.tell_open_file(identifier)?;
    #[allow(clippy::cast_precision_loss)]
    let position = position as f64;
    Ok(requested_outputs(vec![Value::Double(position)], context))
}

pub(super) fn fseek_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("fseek", arguments, 3)?;
    expect_max_outputs("fseek", context, 1)?;
    let identifier = file_identifier("fseek", 1, &arguments[0])?;
    let offset = signed_integer_scalar("fseek", 2, &arguments[1])?;
    let origin = seek_origin(&arguments[2])?;
    match context.seek_open_file(identifier, offset, origin) {
        Ok(_) => Ok(requested_outputs(vec![Value::Double(0.0)], context)),
        Err(error) if error.category == BuiltinErrorCategory::FileSystem => {
            Ok(requested_outputs(vec![Value::Double(-1.0)], context))
        }
        Err(error) => Err(error),
    }
}

pub(super) fn fread_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("fread", arguments, 1, 5)?;
    expect_max_outputs("fread", context, 2)?;
    let identifier = file_identifier("fread", 1, &arguments[0])?;
    let size = arguments
        .get(1)
        .map(parse_read_size)
        .transpose()?
        .unwrap_or(ReadSize::Column(None));
    let precision = arguments
        .get(2)
        .map(|value| text_scalar("fread", 3, value))
        .transpose()?
        .map_or_else(
            || {
                Ok(ReadPrecision {
                    source: PrimitiveKind::U8,
                    destination: PrimitiveKind::F64,
                })
            },
            |value| parse_read_precision(&value),
        )?;
    if let Some(skip) = arguments.get(3) {
        let skip = nonnegative_integer_scalar("fread", 4, skip)?;
        if skip != 0 {
            return Err(unsupported_skip("fread"));
        }
    }
    let info = context.open_file_info(identifier)?;
    let machine_format = arguments
        .get(4)
        .map(|value| text_scalar("fread", 5, value))
        .transpose()?
        .unwrap_or(info.machine_format);
    let order = parse_byte_order(&machine_format)?;
    let maximum_bytes = read_byte_limit(size, precision.source.width())?;
    let bytes = context.read_open_file(identifier, maximum_bytes)?;
    let values = decode_values(&bytes, precision.source, order);
    let count = values.len();
    let (shape, output_elements) = size.output_shape(count)?;
    let output = build_numeric_output(values, output_elements, shape, precision.destination)?;
    Ok(requested_outputs(
        vec![output, Value::Double(count_as_f64(count))],
        context,
    ))
}

pub(super) fn fwrite_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("fwrite", arguments, 2, 5)?;
    expect_max_outputs("fwrite", context, 1)?;
    let identifier = file_identifier("fwrite", 1, &arguments[0])?;
    let values = numeric_values("fwrite", 2, &arguments[1])?;
    let precision = arguments
        .get(2)
        .map(|value| text_scalar("fwrite", 3, value))
        .transpose()?
        .map_or(Ok(PrimitiveKind::U8), |value| parse_primitive(&value))?;
    if let Some(skip) = arguments.get(3) {
        let skip = nonnegative_integer_scalar("fwrite", 4, skip)?;
        if skip != 0 {
            return Err(unsupported_skip("fwrite"));
        }
    }
    let info = context.open_file_info(identifier)?;
    let machine_format = arguments
        .get(4)
        .map(|value| text_scalar("fwrite", 5, value))
        .transpose()?
        .unwrap_or(info.machine_format);
    let order = parse_byte_order(&machine_format)?;
    let bytes = encode_values(&values, precision, order)?;
    context.write_open_file(identifier, &bytes)?;
    Ok(requested_outputs(
        vec![Value::Double(count_as_f64(values.len()))],
        context,
    ))
}

const fn default_open_mode() -> FileOpenMode {
    FileOpenMode {
        access: FileOpenAccess::Read,
        text: false,
    }
}

fn parse_open_mode(permission: &str) -> Result<FileOpenMode, BuiltinError> {
    let mut characters = permission.chars();
    let access = match characters.next() {
        Some('r') => FileOpenAccess::Read,
        Some('w') => FileOpenAccess::Write,
        Some('a') => FileOpenAccess::Append,
        _ => return Err(invalid_permission(permission)),
    };
    let mut update = false;
    let mut binary = false;
    let mut text = false;
    for character in characters {
        match character {
            '+' if !update => update = true,
            'b' if !binary && !text => binary = true,
            't' if !text && !binary => text = true,
            _ => return Err(invalid_permission(permission)),
        }
    }
    let access = match (access, update) {
        (FileOpenAccess::Read, true) => FileOpenAccess::ReadUpdate,
        (FileOpenAccess::Write, true) => FileOpenAccess::WriteUpdate,
        (FileOpenAccess::Append, true) => FileOpenAccess::AppendUpdate,
        (access, false) => access,
        _ => unreachable!(),
    };
    Ok(FileOpenMode { access, text })
}

fn invalid_permission(permission: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{permission}` is not a supported `fopen` permission"),
    )
}

fn default_machine_format() -> String {
    native_machine_format().to_owned()
}

const fn native_machine_format() -> &'static str {
    if cfg!(target_endian = "little") {
        "ieee-le"
    } else {
        "ieee-be"
    }
}

fn canonical_machine_format(value: &str) -> Result<&'static str, BuiltinError> {
    match value.to_ascii_lowercase().as_str() {
        "n" | "native" => Ok(native_machine_format()),
        "l" | "ieee-le" => Ok("ieee-le"),
        "b" | "ieee-be" => Ok("ieee-be"),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{value}` is not a supported machine format"),
        )),
    }
}

fn canonical_encoding(value: String) -> Result<String, BuiltinError> {
    if value.is_empty() || value.contains('\0') {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the `fopen` encoding must be a nonempty label without null characters",
        ));
    }
    if value.eq_ignore_ascii_case("utf-8") || value.eq_ignore_ascii_case("utf8") {
        Ok("UTF-8".to_owned())
    } else {
        Ok(value)
    }
}

fn parse_byte_order(value: &str) -> Result<ByteOrder, BuiltinError> {
    match canonical_machine_format(value)? {
        "ieee-le" => Ok(ByteOrder::Little),
        "ieee-be" => Ok(ByteOrder::Big),
        _ => unreachable!(),
    }
}

fn parse_read_precision(value: &str) -> Result<ReadPrecision, BuiltinError> {
    if let Some(destination) = value.strip_prefix('*') {
        let kind = parse_primitive(destination)?;
        return Ok(ReadPrecision {
            source: kind,
            destination: kind,
        });
    }
    if let Some((source, destination)) = value.split_once("=>") {
        return Ok(ReadPrecision {
            source: parse_primitive(source)?,
            destination: parse_primitive(destination)?,
        });
    }
    Ok(ReadPrecision {
        source: parse_primitive(value)?,
        destination: PrimitiveKind::F64,
    })
}

fn parse_primitive(value: &str) -> Result<PrimitiveKind, BuiltinError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "int8" | "integer*1" | "signed char" | "schar" => Ok(PrimitiveKind::I8),
        "uint8" | "unsigned char" | "uchar" | "char" => Ok(PrimitiveKind::U8),
        "int16" | "integer*2" | "short" => Ok(PrimitiveKind::I16),
        "uint16" | "unsigned short" => Ok(PrimitiveKind::U16),
        "int32" | "integer*4" | "int" | "long" => Ok(PrimitiveKind::I32),
        "uint32" | "unsigned int" | "ulong" | "unsigned long" => Ok(PrimitiveKind::U32),
        "int64" | "integer*8" => Ok(PrimitiveKind::I64),
        "uint64" => Ok(PrimitiveKind::U64),
        "single" | "float32" | "real*4" | "float" => Ok(PrimitiveKind::F32),
        "double" | "float64" | "real*8" => Ok(PrimitiveKind::F64),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{value}` is not a supported binary precision"),
        )),
    }
}

fn parse_read_size(value: &Value) -> Result<ReadSize, BuiltinError> {
    let values = real_numeric_values("fread", 2, value)?;
    match values.as_slice() {
        [value] if value.is_infinite() && value.is_sign_positive() => Ok(ReadSize::Column(None)),
        [value] => Ok(ReadSize::Column(Some(size_component("fread", *value)?))),
        [rows, columns] => Ok(ReadSize::Matrix {
            rows: size_component("fread", *rows)?,
            columns: if columns.is_infinite() && columns.is_sign_positive() {
                None
            } else {
                Some(size_component("fread", *columns)?)
            },
        }),
        _ => Err(type_error(
            "fread",
            2,
            "a nonnegative scalar, Inf, or two-element size vector",
            value,
        )),
    }
}

fn size_component(name: &str, value: f64) -> Result<usize, BuiltinError> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("the `{name}` size must contain nonnegative integer values"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let value = value as u128;
    usize::try_from(value).map_err(|_| size_limit_error(name))
}

fn read_byte_limit(size: ReadSize, width: usize) -> Result<usize, BuiltinError> {
    let transfer_limit = usize::try_from(DEFAULT_MAX_FILE_BYTES).unwrap_or(usize::MAX);
    match size.maximum_elements() {
        Some(elements) => elements
            .checked_mul(width)
            .filter(|bytes| *bytes <= transfer_limit)
            .ok_or_else(|| size_limit_error("fread")),
        None => Ok(transfer_limit - transfer_limit % width),
    }
}

fn decode_values(bytes: &[u8], kind: PrimitiveKind, order: ByteOrder) -> Vec<NumericValue> {
    bytes
        .chunks_exact(kind.width())
        .map(|bytes| decode_value(bytes, kind, order))
        .collect()
}

fn decode_value(bytes: &[u8], kind: PrimitiveKind, order: ByteOrder) -> NumericValue {
    macro_rules! decode {
        ($type:ty) => {{
            let bytes: [u8; size_of::<$type>()] = bytes.try_into().expect("checked chunk width");
            match order {
                ByteOrder::Little => <$type>::from_le_bytes(bytes),
                ByteOrder::Big => <$type>::from_be_bytes(bytes),
            }
        }};
    }
    match kind {
        PrimitiveKind::I8 => NumericValue::Signed(i128::from(i8::from_ne_bytes([bytes[0]]))),
        PrimitiveKind::U8 => NumericValue::Unsigned(u128::from(bytes[0])),
        PrimitiveKind::I16 => NumericValue::Signed(i128::from(decode!(i16))),
        PrimitiveKind::U16 => NumericValue::Unsigned(u128::from(decode!(u16))),
        PrimitiveKind::I32 => NumericValue::Signed(i128::from(decode!(i32))),
        PrimitiveKind::U32 => NumericValue::Unsigned(u128::from(decode!(u32))),
        PrimitiveKind::I64 => NumericValue::Signed(i128::from(decode!(i64))),
        PrimitiveKind::U64 => NumericValue::Unsigned(u128::from(decode!(u64))),
        PrimitiveKind::F32 => NumericValue::Float(f64::from(decode!(f32))),
        PrimitiveKind::F64 => NumericValue::Float(decode!(f64)),
    }
}

fn encode_values(
    values: &[NumericValue],
    kind: PrimitiveKind,
    order: ByteOrder,
) -> Result<Vec<u8>, BuiltinError> {
    let byte_count = values
        .len()
        .checked_mul(kind.width())
        .filter(|bytes| u64::try_from(*bytes).unwrap_or(u64::MAX) <= DEFAULT_MAX_FILE_BYTES)
        .ok_or_else(|| size_limit_error("fwrite"))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(byte_count)
        .map_err(|_| size_limit_error("fwrite"))?;
    for value in values {
        encode_value(&mut output, *value, kind, order);
    }
    Ok(output)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn encode_value(output: &mut Vec<u8>, value: NumericValue, kind: PrimitiveKind, order: ByteOrder) {
    macro_rules! encode {
        ($value:expr) => {{
            let bytes = match order {
                ByteOrder::Little => $value.to_le_bytes(),
                ByteOrder::Big => $value.to_be_bytes(),
            };
            output.extend_from_slice(&bytes);
        }};
    }
    match kind {
        PrimitiveKind::I8 => {
            output.push(to_i128(value, i8::MIN.into(), i8::MAX.into()) as i8 as u8);
        }
        PrimitiveKind::U8 => output.push(to_u128(value, u8::MAX.into()) as u8),
        PrimitiveKind::I16 => encode!(to_i128(value, i16::MIN.into(), i16::MAX.into()) as i16),
        PrimitiveKind::U16 => encode!(to_u128(value, u16::MAX.into()) as u16),
        PrimitiveKind::I32 => encode!(to_i128(value, i32::MIN.into(), i32::MAX.into()) as i32),
        PrimitiveKind::U32 => encode!(to_u128(value, u32::MAX.into()) as u32),
        PrimitiveKind::I64 => encode!(to_i128(value, i64::MIN.into(), i64::MAX.into()) as i64),
        PrimitiveKind::U64 => encode!(to_u128(value, u64::MAX.into()) as u64),
        PrimitiveKind::F32 => encode!(to_f64(value) as f32),
        PrimitiveKind::F64 => encode!(to_f64(value)),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn build_numeric_output(
    mut values: Vec<NumericValue>,
    output_elements: usize,
    shape: Shape,
    kind: PrimitiveKind,
) -> Result<Value, BuiltinError> {
    values.resize(output_elements, NumericValue::Unsigned(0));
    macro_rules! dense {
        ($type:ty, $convert:expr, $wrap:expr) => {{
            let values = values.into_iter().map($convert).collect::<Vec<$type>>();
            DenseArray::from_vec(shape, values)
                .map($wrap)
                .map_err(|error| array_error(&error))
        }};
    }
    match kind {
        PrimitiveKind::F64 => dense!(f64, to_f64, |array| Value::Array(ArrayData::F64(array))),
        PrimitiveKind::F32 => dense!(f32, |value| to_f64(value) as f32, |array| {
            Value::Array(ArrayData::F32(array))
        }),
        PrimitiveKind::I8 => dense!(
            i8,
            |value| to_i128(value, i8::MIN.into(), i8::MAX.into()) as i8,
            |array| { Value::Array(ArrayData::Integer(IntegerArrayData::I8(array))) }
        ),
        PrimitiveKind::U8 => dense!(u8, |value| to_u128(value, u8::MAX.into()) as u8, |array| {
            Value::Array(ArrayData::Integer(IntegerArrayData::U8(array)))
        }),
        PrimitiveKind::I16 => dense!(
            i16,
            |value| to_i128(value, i16::MIN.into(), i16::MAX.into()) as i16,
            |array| { Value::Array(ArrayData::Integer(IntegerArrayData::I16(array))) }
        ),
        PrimitiveKind::U16 => dense!(
            u16,
            |value| to_u128(value, u16::MAX.into()) as u16,
            |array| { Value::Array(ArrayData::Integer(IntegerArrayData::U16(array))) }
        ),
        PrimitiveKind::I32 => dense!(
            i32,
            |value| to_i128(value, i32::MIN.into(), i32::MAX.into()) as i32,
            |array| { Value::Array(ArrayData::Integer(IntegerArrayData::I32(array))) }
        ),
        PrimitiveKind::U32 => dense!(
            u32,
            |value| to_u128(value, u32::MAX.into()) as u32,
            |array| { Value::Array(ArrayData::Integer(IntegerArrayData::U32(array))) }
        ),
        PrimitiveKind::I64 => dense!(
            i64,
            |value| to_i128(value, i64::MIN.into(), i64::MAX.into()) as i64,
            |array| { Value::Array(ArrayData::Integer(IntegerArrayData::I64(array))) }
        ),
        PrimitiveKind::U64 => dense!(
            u64,
            |value| to_u128(value, u64::MAX.into()) as u64,
            |array| { Value::Array(ArrayData::Integer(IntegerArrayData::U64(array))) }
        ),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn to_i128(value: NumericValue, minimum: i128, maximum: i128) -> i128 {
    match value {
        NumericValue::Signed(value) => value.clamp(minimum, maximum),
        NumericValue::Unsigned(value) => i128::try_from(value).unwrap_or(i128::MAX).min(maximum),
        NumericValue::Float(value) if value.is_nan() => 0,
        NumericValue::Float(value) => value.round().clamp(minimum as f64, maximum as f64) as i128,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn to_u128(value: NumericValue, maximum: u128) -> u128 {
    match value {
        NumericValue::Signed(value) => u128::try_from(value).unwrap_or(0).min(maximum),
        NumericValue::Unsigned(value) => value.min(maximum),
        NumericValue::Float(value) if value.is_nan() => 0,
        NumericValue::Float(value) => value.round().clamp(0.0, maximum as f64) as u128,
    }
}

#[allow(clippy::cast_precision_loss)]
fn to_f64(value: NumericValue) -> f64 {
    match value {
        NumericValue::Signed(value) => value as f64,
        NumericValue::Unsigned(value) => value as f64,
        NumericValue::Float(value) => value,
    }
}

fn numeric_values(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<Vec<NumericValue>, BuiltinError> {
    macro_rules! signed {
        ($array:expr) => {
            $array
                .as_slice()
                .iter()
                .copied()
                .map(|value| NumericValue::Signed(i128::from(value)))
                .collect()
        };
    }
    macro_rules! unsigned {
        ($array:expr) => {
            $array
                .as_slice()
                .iter()
                .copied()
                .map(|value| NumericValue::Unsigned(u128::from(value)))
                .collect()
        };
    }
    let values = match value {
        Value::Logical(value) => vec![NumericValue::Unsigned(u128::from(*value))],
        Value::Double(value) => vec![NumericValue::Float(*value)],
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .iter()
            .copied()
            .map(|value| NumericValue::Float(f64::from(value)))
            .collect(),
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .iter()
            .copied()
            .map(NumericValue::Float)
            .collect(),
        Value::Array(ArrayData::Logical(array)) => array
            .as_slice()
            .iter()
            .copied()
            .map(|value| NumericValue::Unsigned(u128::from(value.get())))
            .collect(),
        Value::Array(ArrayData::Char(array)) => array
            .as_slice()
            .iter()
            .copied()
            .map(|value| NumericValue::Unsigned(u128::from(value.get())))
            .collect(),
        Value::Array(ArrayData::Integer(IntegerArrayData::I8(array))) => signed!(array),
        Value::Array(ArrayData::Integer(IntegerArrayData::U8(array))) => unsigned!(array),
        Value::Array(ArrayData::Integer(IntegerArrayData::I16(array))) => signed!(array),
        Value::Array(ArrayData::Integer(IntegerArrayData::U16(array))) => unsigned!(array),
        Value::Array(ArrayData::Integer(IntegerArrayData::I32(array))) => signed!(array),
        Value::Array(ArrayData::Integer(IntegerArrayData::U32(array))) => unsigned!(array),
        Value::Array(ArrayData::Integer(IntegerArrayData::I64(array))) => signed!(array),
        Value::Array(ArrayData::Integer(IntegerArrayData::U64(array))) => unsigned!(array),
        _ => {
            return Err(type_error(
                name,
                position,
                "a real full numeric array",
                value,
            ));
        }
    };
    Ok(values)
}

fn real_numeric_values(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<Vec<f64>, BuiltinError> {
    numeric_values(name, position, value).map(|values| values.into_iter().map(to_f64).collect())
}

fn seek_origin(value: &Value) -> Result<FileSeekOrigin, BuiltinError> {
    if let Some(text) = text_scalar_if_present(value) {
        return match text.to_ascii_lowercase().as_str() {
            "bof" => Ok(FileSeekOrigin::Start),
            "cof" => Ok(FileSeekOrigin::Current),
            "eof" => Ok(FileSeekOrigin::End),
            _ => Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{text}` is not a supported `fseek` origin"),
            )),
        };
    }
    match signed_integer_scalar("fseek", 3, value)? {
        -1 => Ok(FileSeekOrigin::Start),
        0 => Ok(FileSeekOrigin::Current),
        1 => Ok(FileSeekOrigin::End),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 3 to `fseek` must be -1, 0, 1, 'bof', 'cof', or 'eof'",
        )),
    }
}

fn optional_file_identifier(value: &Value) -> Result<Option<u32>, BuiltinError> {
    if is_numeric_scalar(value) {
        file_identifier("fopen", 1, value).map(Some)
    } else {
        Ok(None)
    }
}

fn is_numeric_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Double(_) | Value::Logical(_) | Value::Array(ArrayData::F32(_))
    ) || value.dtype().is_some_and(|_| value.numel() == Some(1))
}

pub(super) fn file_identifier(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<u32, BuiltinError> {
    if let Some(component) = exact_real_integer_scalar(value) {
        return match component {
            IntegerComponent::Signed(value) => u32::try_from(value),
            IntegerComponent::Unsigned(value) => u32::try_from(value),
        }
        .map_err(|_| bad_file_identifier_value(name, position));
    }
    let number = numeric_scalar(name, position, value)?;
    if !number.is_finite() || number < 0.0 || number.fract() != 0.0 || number > f64::from(u32::MAX)
    {
        return Err(bad_file_identifier_value(name, position));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as u32)
}

fn bad_file_identifier_value(name: &str, position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `{name}` is not an open file identifier"),
    )
    .with_identifier("MATLAB:badfid_mx")
}

fn nonnegative_integer_scalar(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<usize, BuiltinError> {
    let value = numeric_scalar(name, position, value)?;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a nonnegative integer scalar"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let integer = value as u128;
    usize::try_from(integer).map_err(|_| size_limit_error(name))
}

#[allow(clippy::cast_precision_loss)]
fn signed_integer_scalar(name: &str, position: usize, value: &Value) -> Result<i64, BuiltinError> {
    if let Some(component) = exact_real_integer_scalar(value) {
        return match component {
            IntegerComponent::Signed(value) => i64::try_from(value),
            IntegerComponent::Unsigned(value) => i64::try_from(value),
        }
        .map_err(|_| size_limit_error(name));
    }
    let value = numeric_scalar(name, position, value)?;
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i64::MIN as f64
        || value > i64::MAX as f64
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be an integer scalar in the int64 range"),
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as i64)
}

pub(super) fn numeric_scalar(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<f64, BuiltinError> {
    let values = real_numeric_values(name, position, value)?;
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(type_error(name, position, "a real numeric scalar", value)),
    }
}

pub(super) fn text_scalar(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<String, BuiltinError> {
    text_scalar_if_present(value).ok_or_else(|| {
        type_error(
            name,
            position,
            "a nonmissing string scalar or char row",
            value,
        )
    })
}

fn text_scalar_if_present(value: &Value) -> Option<String> {
    let code_units = match value {
        Value::String(string) => {
            let scalar = string.as_scalar()?;
            if scalar.is_missing() {
                return None;
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
        _ => return None,
    };
    String::from_utf16(&code_units).ok()
}

pub(super) fn char_text(name: &str, text: &str) -> Result<Value, BuiltinError> {
    let values = text
        .encode_utf16()
        .map(CharCodeUnit::new)
        .collect::<Vec<_>>();
    let length = u64::try_from(values.len()).map_err(|_| size_limit_error(name))?;
    let shape = if length == 0 {
        Shape::new([0, 0])
    } else {
        Shape::new([1, length])
    }
    .map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn identifier_row(identifiers: &[u32]) -> Result<Value, BuiltinError> {
    let values = identifiers
        .iter()
        .copied()
        .map(f64::from)
        .collect::<Vec<_>>();
    let length = u64::try_from(values.len()).map_err(|_| size_limit_error("fopen"))?;
    let shape = if length == 0 {
        Shape::new([0, 0])
    } else {
        Shape::new([1, length])
    }
    .map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

pub(super) fn requested_outputs(
    mut outputs: Vec<Value>,
    context: &BuiltinContext<'_>,
) -> Vec<Value> {
    outputs.truncate(context.requested_outputs());
    outputs
}

fn count_as_f64(count: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let count = count as f64;
    count
}

fn unsupported_skip(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("nonzero skip values are not implemented for `{name}` yet"),
    )
}

fn size_limit_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("the requested `{name}` transfer exceeds session or host limits"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmat_runtime::{CancellationToken, LocalFileSystem, NullOutput};

    fn temporary_directory() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "openmat-stream-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn char_value(text: &str) -> Value {
        char_text("test", text).unwrap()
    }

    fn call(
        function: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
        arguments: &[Value],
        outputs: usize,
        files: &mut LocalFileSystem,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context =
            BuiltinContext::with_file_system_service(outputs, &cancellation, &mut output, files);
        function(arguments, &mut context)
    }

    #[test]
    fn identifiers_are_reused_and_queryable() {
        let directory = temporary_directory();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let first = call(
            fopen_builtin,
            &[char_value("first.bin"), char_value("w")],
            1,
            &mut files,
        )
        .unwrap();
        let second = call(
            fopen_builtin,
            &[char_value("second.bin"), char_value("w")],
            1,
            &mut files,
        )
        .unwrap();
        assert_eq!(first, vec![Value::Double(3.0)]);
        assert_eq!(second, vec![Value::Double(4.0)]);
        call(fclose_builtin, &[Value::Double(3.0)], 1, &mut files).unwrap();
        let reused = call(
            fopen_builtin,
            &[char_value("third.bin"), char_value("w")],
            1,
            &mut files,
        )
        .unwrap();
        assert_eq!(reused, vec![Value::Double(3.0)]);
        let query = call(fopen_builtin, &[Value::Double(3.0)], 4, &mut files).unwrap();
        assert_eq!(query[0], char_value("third.bin"));
        assert_eq!(query[1], char_value("wb"));
        assert_eq!(query[2], char_value(native_machine_format()));
        assert_eq!(query[3], char_value("UTF-8"));
        let all = call(fopen_builtin, &[char_value("all")], 1, &mut files).unwrap();
        let Value::Array(ArrayData::F64(all)) = &all[0] else {
            panic!("expected numeric identifier row");
        };
        assert_eq!(all.as_slice(), &[3.0, 4.0]);
        call(fclose_builtin, &[char_value("all")], 1, &mut files).unwrap();
        let invalid_query = call(fopen_builtin, &[Value::Double(3.0)], 4, &mut files).unwrap();
        assert_eq!(invalid_query, vec![char_value(""); 4]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn binary_round_trip_preserves_type_shape_count_and_position() {
        let directory = temporary_directory();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        call(
            fopen_builtin,
            &[
                char_value("values.bin"),
                char_value("w+"),
                char_value("ieee-be"),
            ],
            1,
            &mut files,
        )
        .unwrap();
        let input = Value::Array(ArrayData::Integer(IntegerArrayData::U16(
            DenseArray::from_vec(Shape::new([3, 1]).unwrap(), vec![1, 258, 65_535]).unwrap(),
        )));
        let written = call(
            fwrite_builtin,
            &[Value::Double(3.0), input, char_value("uint16")],
            1,
            &mut files,
        )
        .unwrap();
        assert_eq!(written, vec![Value::Double(3.0)]);
        assert_eq!(
            call(ftell_builtin, &[Value::Double(3.0)], 1, &mut files).unwrap(),
            vec![Value::Double(6.0)]
        );
        call(
            fseek_builtin,
            &[Value::Double(3.0), Value::Double(0.0), char_value("bof")],
            1,
            &mut files,
        )
        .unwrap();
        let read = call(
            fread_builtin,
            &[
                Value::Double(3.0),
                Value::Array(ArrayData::F64(
                    DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![2.0, f64::INFINITY])
                        .unwrap(),
                )),
                char_value("uint16=>uint16"),
            ],
            2,
            &mut files,
        )
        .unwrap();
        assert_eq!(read[1], Value::Double(3.0));
        let Value::Array(ArrayData::Integer(IntegerArrayData::U16(array))) = &read[0] else {
            panic!("expected uint16 output");
        };
        assert_eq!(array.shape().dimensions(), &[2, 2]);
        assert_eq!(array.as_slice(), &[1, 258, 65_535, 0]);
        assert_eq!(
            call(
                fseek_builtin,
                &[Value::Double(3.0), Value::Double(-7.0), char_value("cof"),],
                1,
                &mut files,
            )
            .unwrap(),
            vec![Value::Double(-1.0)]
        );
        assert_eq!(
            call(ftell_builtin, &[Value::Double(3.0)], 1, &mut files).unwrap(),
            vec![Value::Double(6.0)]
        );
        call(fclose_builtin, &[Value::Double(3.0)], 0, &mut files).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_failure_is_a_minus_one_result_but_bad_identifier_is_an_error() {
        let directory = temporary_directory();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let failure = call(
            fopen_builtin,
            &[char_value("missing.bin"), char_value("r")],
            2,
            &mut files,
        )
        .unwrap();
        assert_eq!(failure[0], Value::Double(-1.0));
        let Value::Array(ArrayData::Char(message)) = &failure[1] else {
            panic!("expected fopen message");
        };
        assert_ne!(message.numel(), 0);

        let error = call(ftell_builtin, &[Value::Double(99.0)], 1, &mut files).unwrap_err();
        assert_eq!(error.identifier.as_deref(), Some("MATLAB:badfid_mx"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn empty_read_uses_the_observed_zero_by_zero_shape() {
        let directory = temporary_directory();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        call(
            fopen_builtin,
            &[char_value("empty.bin"), char_value("w+")],
            1,
            &mut files,
        )
        .unwrap();
        let read = call(
            fread_builtin,
            &[Value::Double(3.0), Value::Double(f64::INFINITY)],
            2,
            &mut files,
        )
        .unwrap();
        let Value::Array(ArrayData::F64(array)) = &read[0] else {
            panic!("expected empty double output");
        };
        assert_eq!(array.shape().dimensions(), &[0, 0]);
        assert_eq!(read[1], Value::Double(0.0));
        call(fclose_builtin, &[Value::Double(3.0)], 0, &mut files).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}

use std::{collections::BTreeMap, path::Path};

use openmat_array::{ArrayData, CharCodeUnit, Complex32, Complex64, DenseArray, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{
    CellArray, FieldName, StringArray, StringElement, StringValue, StructArray, TableArray,
    TableVariableName, Value,
};

use openmat_mat::{MatErrorKind, MatLimits, MatVariable, MatVersion};

use crate::{array_error, expect_argument_count, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Delimiter {
    Character(char),
    Whitespace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputType {
    Double,
    Single,
}

struct ReadMatrixOptions {
    delimiter: Option<Delimiter>,
    header_lines: Option<usize>,
    output_type: OutputType,
}

impl Default for ReadMatrixOptions {
    fn default() -> Self {
        Self {
            delimiter: None,
            header_lines: None,
            output_type: OutputType::Double,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableTextType {
    CellChar,
    String,
}

struct ReadTableOptions {
    delimiter: Option<Delimiter>,
    read_variable_names: Option<bool>,
    preserve_variable_names: bool,
    file_type_text: bool,
    text_type: TableTextType,
}

impl Default for ReadTableOptions {
    fn default() -> Self {
        Self {
            delimiter: None,
            read_variable_names: None,
            preserve_variable_names: false,
            file_type_text: false,
            text_type: TableTextType::CellChar,
        }
    }
}

#[derive(Clone, Copy)]
struct WriteTableOptions {
    delimiter: Delimiter,
    delimiter_explicit: bool,
    write_variable_names: bool,
    file_type_text: bool,
}

pub(super) fn fileread_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("fileread", arguments, 1)?;
    expect_max_outputs("fileread", context, 1)?;
    let path = text_scalar("fileread", 1, &arguments[0])?;
    let bytes = context.read_file(&path)?;
    let text = String::from_utf8(bytes).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::FileSystem,
            format!(
                "cannot decode `{path}` as UTF-8 text; invalid byte at offset {}",
                error.utf8_error().valid_up_to()
            ),
        )
    })?;
    Ok(vec![char_text(&text)?])
}

pub(super) fn readmatrix_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_file_arguments("readmatrix", arguments)?;
    expect_max_outputs("readmatrix", context, 1)?;
    let path = text_scalar("readmatrix", 1, &arguments[0])?;
    reject_spreadsheet_extension("readmatrix", &path)?;
    let options = parse_readmatrix_options(&arguments[1..])?;
    let bytes = context.read_file(&path)?;
    let text = decode_utf8_text("readmatrix", &path, bytes)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let delimiter = options
        .delimiter
        .unwrap_or_else(|| detect_delimiter(text, &path));
    let mut records = parse_records(text, delimiter)?;
    records.retain(|record| record.iter().any(|field| !field.trim().is_empty()));

    let skipped = options.header_lines.unwrap_or_else(|| {
        records
            .iter()
            .take_while(|record| !record.iter().any(|field| parse_number(field).is_some()))
            .count()
    });
    let records = records.get(skipped..).unwrap_or_default();
    matrix_from_records(records, options.output_type, context)
}

pub(super) fn writematrix_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 2 || !(arguments.len() - 2).is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `writematrix` expects a matrix, filename, and name-value pairs but received {} inputs",
                arguments.len()
            ),
        ));
    }
    expect_max_outputs("writematrix", context, 0)?;
    let path = text_scalar("writematrix", 2, &arguments[1])?;
    reject_spreadsheet_extension("writematrix", &path)?;
    let delimiter = parse_writematrix_options(&arguments[2..])?;
    let text = format_matrix(&arguments[0], delimiter, context)?;
    context.write_file(&path, text.as_bytes())?;
    Ok(Vec::new())
}

pub(super) fn readtable_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_file_arguments("readtable", arguments)?;
    expect_max_outputs("readtable", context, 1)?;
    let path = text_scalar("readtable", 1, &arguments[0])?;
    reject_spreadsheet_extension("readtable", &path)?;
    let options = parse_readtable_options(&arguments[1..])?;
    validate_tsv_options(
        "readtable",
        &path,
        options.delimiter.is_some(),
        options.file_type_text,
    )?;
    let bytes = context.read_file(&path)?;
    let text = decode_utf8_text("readtable", &path, bytes)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let delimiter = options
        .delimiter
        .unwrap_or_else(|| detect_delimiter(text, &path));
    let mut records = parse_records(text, delimiter)?;
    records.retain(|record| record.iter().any(|field| !field.trim().is_empty()));

    let column_count = records.iter().map(Vec::len).max().unwrap_or(0);
    if column_count == 0 {
        return TableArray::empty(0)
            .map(Value::Table)
            .map(|table| vec![table])
            .map_err(table_error);
    }
    for record in &mut records {
        record.resize(column_count, String::new());
    }
    let detected_header = detect_table_header(&records);
    let consume_header = options.read_variable_names == Some(true) || detected_header;
    let use_header_names = consume_header && options.read_variable_names != Some(false);
    let header_names = if consume_header && !records.is_empty() {
        Some(records.remove(0))
    } else {
        None
    };
    let inferred_names = if use_header_names {
        header_names.unwrap_or_default()
    } else {
        (1..=column_count)
            .map(|index| format!("Var{index}"))
            .collect()
    };
    let names = normalize_table_names(inferred_names, options.preserve_variable_names)?;
    let rows = records.len();
    let mut variables = Vec::with_capacity(column_count);
    for column in 0..column_count {
        variables.push(infer_table_column(
            &records,
            column,
            options.text_type,
            context,
        )?);
    }
    let rows = u64::try_from(rows).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`readtable` row count is too large",
        )
    })?;
    TableArray::from_parts(rows, names, variables)
        .map(Value::Table)
        .map(|table| vec![table])
        .map_err(table_error)
}

pub(super) fn writetable_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 2 || !(arguments.len() - 2).is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `writetable` expects a table, filename, and name-value pairs but received {} inputs",
                arguments.len()
            ),
        ));
    }
    expect_max_outputs("writetable", context, 0)?;
    let Value::Table(table) = &arguments[0] else {
        return Err(type_error("writetable", 1, "table", &arguments[0]));
    };
    let path = text_scalar("writetable", 2, &arguments[1])?;
    reject_spreadsheet_extension("writetable", &path)?;
    let options = parse_writetable_options(&arguments[2..])?;
    validate_tsv_options(
        "writetable",
        &path,
        options.delimiter_explicit,
        options.file_type_text,
    )?;
    let text = format_table(table, options, context)?;
    context.write_file(&path, text.as_bytes())?;
    Ok(Vec::new())
}

pub(super) fn load_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "built-in load expects a filename",
        ));
    }
    expect_max_outputs("load", context, 1)?;
    let path = mat_path(&text_scalar("load", 1, &arguments[0])?);
    let filters = parse_load_filters(&arguments[1..])?;
    let bytes = context.read_file(&path)?;
    let decoded = openmat_mat::decode_cancellable(
        &bytes,
        MatLimits::default(),
        Some(context.cancellation_flag()),
    )
    .map_err(|error| mat_builtin_error(&path, "load", &error))?;
    let mut selected = BTreeMap::new();
    for variable in decoded {
        if filters.is_empty()
            || filters
                .iter()
                .any(|pattern| wildcard_matches(pattern, &variable.name))
        {
            selected.insert(variable.name, variable.value);
        }
    }
    if context.requested_outputs() == 0 {
        for (name, value) in selected {
            context.stage_scope_binding(name, &value)?;
        }
        return Ok(Vec::new());
    }
    Ok(vec![structure_from_bindings(selected)?])
}

pub(super) fn save_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "built-in save expects a filename",
        ));
    }
    expect_max_outputs("save", context, 0)?;
    let path = mat_path(&text_scalar("save", 1, &arguments[0])?);
    let options = parse_save_options(&arguments[1..])?;
    let workspace = context.workspace()?;
    let available = workspace
        .iter()
        .map(|(name, value)| (name.to_owned(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let selected = select_save_bindings(&available, &options.patterns)?;
    let mut output_version = options.version;
    let mut merged = if options.append {
        let bytes = context.read_file(&path)?;
        if !options.version_explicit && openmat_mat::is_v73(&bytes) {
            output_version = MatVersion::V73;
        }
        openmat_mat::decode_cancellable(
            &bytes,
            MatLimits::default(),
            Some(context.cancellation_flag()),
        )
        .map_err(|error| mat_builtin_error(&path, "append to", &error))?
        .into_iter()
        .map(|variable| (variable.name, variable.value))
        .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    merged.extend(selected);
    let variables = merged
        .into_iter()
        .map(|(name, value)| MatVariable::new(name, value))
        .collect::<Vec<_>>();
    let bytes = openmat_mat::encode_cancellable(
        &variables,
        output_version,
        MatLimits::default(),
        Some(context.cancellation_flag()),
    )
    .map_err(|error| mat_builtin_error(&path, "save", &error))?;
    context.write_file(&path, &bytes)?;
    Ok(Vec::new())
}

struct SaveOptions {
    version: MatVersion,
    version_explicit: bool,
    append: bool,
    patterns: Vec<String>,
}

fn parse_load_filters(arguments: &[Value]) -> Result<Vec<String>, BuiltinError> {
    let mut filters = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let value = text_scalar("load", index + 2, argument)?;
        if value.starts_with('-') {
            match value.to_ascii_lowercase().as_str() {
                "-mat" => {}
                "-ascii" | "-regexp" => {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("load option '{value}' is not implemented"),
                    ));
                }
                _ => {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("unsupported load option '{value}'"),
                    ));
                }
            }
        } else {
            filters.push(value);
        }
    }
    Ok(filters)
}

fn parse_save_options(arguments: &[Value]) -> Result<SaveOptions, BuiltinError> {
    let mut version = MatVersion::V7;
    let mut version_explicit = false;
    let mut append = false;
    let mut patterns = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let value = text_scalar("save", index + 2, argument)?;
        if value.starts_with('-') {
            match value.to_ascii_lowercase().as_str() {
                "-v6" => {
                    version = MatVersion::V6;
                    version_explicit = true;
                }
                "-v7" | "-mat" => {
                    version = MatVersion::V7;
                    version_explicit = true;
                }
                "-v7.3" => {
                    version = MatVersion::V73;
                    version_explicit = true;
                }
                "-append" => append = true,
                "-nocompression" | "-regexp" | "-struct" | "-ascii" => {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("save option '{value}' is not implemented"),
                    ));
                }
                _ => {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("unsupported save option '{value}'"),
                    ));
                }
            }
        } else {
            patterns.push(value);
        }
    }
    Ok(SaveOptions {
        version,
        version_explicit,
        append,
        patterns,
    })
}

fn mat_builtin_error(path: &str, operation: &str, error: &openmat_mat::MatError) -> BuiltinError {
    let category = match error.kind() {
        MatErrorKind::Cancelled => BuiltinErrorCategory::Cancelled,
        MatErrorKind::InvalidValue | MatErrorKind::Unsupported | MatErrorKind::LimitExceeded => {
            BuiltinErrorCategory::Domain
        }
        MatErrorKind::InvalidFormat | MatErrorKind::Hdf5 => BuiltinErrorCategory::FileSystem,
    };
    BuiltinError::new(
        category,
        format!("cannot {operation} MAT-file '{path}': {error}"),
    )
}

fn select_save_bindings(
    available: &BTreeMap<String, Value>,
    patterns: &[String],
) -> Result<BTreeMap<String, Value>, BuiltinError> {
    if patterns.is_empty() {
        return Ok(available.clone());
    }
    let mut selected = BTreeMap::new();
    for pattern in patterns {
        let wildcard = pattern.contains(['*', '?']);
        let mut matched = false;
        for (name, value) in available {
            if wildcard_matches(pattern, name) {
                selected.insert(name.clone(), value.clone());
                matched = true;
            }
        }
        if !matched && !wildcard {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("variable '{pattern}' was not found for save"),
            ));
        }
    }
    Ok(selected)
}

fn structure_from_bindings(bindings: BTreeMap<String, Value>) -> Result<Value, BuiltinError> {
    let mut fields = Vec::with_capacity(bindings.len());
    let mut columns = Vec::with_capacity(bindings.len());
    for (name, value) in bindings {
        fields.push(FieldName::new(name).map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::FileSystem,
                format!("MAT-file contains an invalid variable name: {error}"),
            )
        })?);
        columns.push(vec![value]);
    }
    let shape = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    StructArray::from_columns(shape, fields, columns)
        .map(Value::Struct)
        .map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                format!("cannot construct the load result structure: {error}"),
            )
        })
}

fn mat_path(path: &str) -> String {
    if Path::new(path).extension().is_none() {
        format!("{path}.mat")
    } else {
        path.to_owned()
    }
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;
    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if *token == b'*' {
            current[0] = previous[0];
            for index in 1..=value.len() {
                current[index] = previous[index] || current[index - 1];
            }
        } else {
            for index in 1..=value.len() {
                current[index] =
                    previous[index - 1] && (*token == b'?' || *token == value[index - 1]);
            }
        }
        previous = current;
    }
    previous[value.len()]
}

fn expect_file_arguments(name: &str, arguments: &[Value]) -> Result<(), BuiltinError> {
    if !arguments.is_empty() && (arguments.len() - 1).is_multiple_of(2) {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `{name}` expects a filename followed by name-value pairs but received {} inputs",
                arguments.len()
            ),
        ))
    }
}

fn parse_readmatrix_options(arguments: &[Value]) -> Result<ReadMatrixOptions, BuiltinError> {
    let mut options = ReadMatrixOptions::default();
    for (pair, values) in arguments.chunks_exact(2).enumerate() {
        let position = pair * 2 + 2;
        let name = text_scalar("readmatrix", position, &values[0])?.to_ascii_lowercase();
        match name.as_str() {
            "delimiter" => {
                options.delimiter = Some(delimiter_value("readmatrix", position + 1, &values[1])?);
            }
            "numheaderlines" => {
                options.header_lines = Some(nonnegative_integer_scalar(
                    "readmatrix",
                    position + 1,
                    &values[1],
                )?);
            }
            "outputtype" => {
                options.output_type = match text_scalar("readmatrix", position + 1, &values[1])?
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "double" => OutputType::Double,
                    "single" => OutputType::Single,
                    other => {
                        return Err(BuiltinError::new(
                            BuiltinErrorCategory::Domain,
                            format!("unsupported `readmatrix` OutputType `{other}`"),
                        ));
                    }
                };
            }
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("unsupported `readmatrix` option `{name}`"),
                ));
            }
        }
    }
    Ok(options)
}

fn parse_writematrix_options(arguments: &[Value]) -> Result<Delimiter, BuiltinError> {
    let mut delimiter = Delimiter::Character(',');
    for (pair, values) in arguments.chunks_exact(2).enumerate() {
        let position = pair * 2 + 3;
        let name = text_scalar("writematrix", position, &values[0])?.to_ascii_lowercase();
        match name.as_str() {
            "delimiter" => {
                delimiter = delimiter_value("writematrix", position + 1, &values[1])?;
            }
            "filetype" => {
                let value = text_scalar("writematrix", position + 1, &values[1])?;
                if !value.eq_ignore_ascii_case("text") {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("unsupported `writematrix` FileType `{value}`"),
                    ));
                }
            }
            "writemode" => {
                let value = text_scalar("writematrix", position + 1, &values[1])?;
                if !value.eq_ignore_ascii_case("overwrite") {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "`writematrix` currently supports WriteMode `overwrite` only",
                    ));
                }
            }
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("unsupported `writematrix` option `{name}`"),
                ));
            }
        }
    }
    if delimiter == Delimiter::Whitespace {
        Ok(Delimiter::Character(' '))
    } else {
        Ok(delimiter)
    }
}

fn parse_readtable_options(arguments: &[Value]) -> Result<ReadTableOptions, BuiltinError> {
    let mut options = ReadTableOptions::default();
    for (pair, values) in arguments.chunks_exact(2).enumerate() {
        let position = pair * 2 + 2;
        let name = text_scalar("readtable", position, &values[0])?.to_ascii_lowercase();
        match name.as_str() {
            "delimiter" => {
                options.delimiter = Some(delimiter_value("readtable", position + 1, &values[1])?);
            }
            "readvariablenames" => {
                options.read_variable_names =
                    Some(logical_option("readtable", position + 1, &values[1])?);
            }
            "variablenamingrule" => {
                options.preserve_variable_names =
                    match text_scalar("readtable", position + 1, &values[1])?
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "modify" => false,
                        "preserve" => true,
                        other => {
                            return Err(BuiltinError::new(
                                BuiltinErrorCategory::Domain,
                                format!("unsupported `readtable` VariableNamingRule `{other}`"),
                            ));
                        }
                    };
            }
            "texttype" => {
                options.text_type = match text_scalar("readtable", position + 1, &values[1])?
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "char" => TableTextType::CellChar,
                    "string" => TableTextType::String,
                    other => {
                        return Err(BuiltinError::new(
                            BuiltinErrorCategory::Domain,
                            format!("unsupported `readtable` TextType `{other}`"),
                        ));
                    }
                };
            }
            "filetype" => {
                let value = text_scalar("readtable", position + 1, &values[1])?;
                if !value.eq_ignore_ascii_case("text") {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("unsupported `readtable` FileType `{value}`"),
                    ));
                }
                options.file_type_text = true;
            }
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("unsupported `readtable` option `{name}`"),
                ));
            }
        }
    }
    Ok(options)
}

fn parse_writetable_options(arguments: &[Value]) -> Result<WriteTableOptions, BuiltinError> {
    let mut options = WriteTableOptions {
        delimiter: Delimiter::Character(','),
        delimiter_explicit: false,
        write_variable_names: true,
        file_type_text: false,
    };
    for (pair, values) in arguments.chunks_exact(2).enumerate() {
        let position = pair * 2 + 3;
        let name = text_scalar("writetable", position, &values[0])?.to_ascii_lowercase();
        match name.as_str() {
            "delimiter" => {
                options.delimiter = delimiter_value("writetable", position + 1, &values[1])?;
                options.delimiter_explicit = true;
            }
            "writevariablenames" => {
                options.write_variable_names =
                    logical_option("writetable", position + 1, &values[1])?;
            }
            "filetype" => {
                let value = text_scalar("writetable", position + 1, &values[1])?;
                if !value.eq_ignore_ascii_case("text") {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("unsupported `writetable` FileType `{value}`"),
                    ));
                }
                options.file_type_text = true;
            }
            "writemode" => {
                let value = text_scalar("writetable", position + 1, &values[1])?;
                if !value.eq_ignore_ascii_case("overwrite") {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "`writetable` currently supports WriteMode `overwrite` only",
                    ));
                }
            }
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("unsupported `writetable` option `{name}`"),
                ));
            }
        }
    }
    if options.delimiter == Delimiter::Whitespace {
        options.delimiter = Delimiter::Character(' ');
    }
    Ok(options)
}

fn logical_option(name: &str, position: usize, value: &Value) -> Result<bool, BuiltinError> {
    match value {
        Value::Logical(value) => Ok(*value),
        Value::Double(number) => match number.to_bits() {
            0 | 0x8000_0000_0000_0000 => Ok(false),
            0x3ff0_0000_0000_0000 => Ok(true),
            _ => Err(type_error(name, position, "logical scalar", value)),
        },
        _ => Err(type_error(name, position, "logical scalar", value)),
    }
}

fn normalize_table_names(
    names: Vec<String>,
    preserve: bool,
) -> Result<Vec<TableVariableName>, BuiltinError> {
    let mut used = BTreeMap::<String, usize>::new();
    let mut result = Vec::with_capacity(names.len());
    for (index, name) in names.into_iter().enumerate() {
        let base = if preserve {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                format!("Var{}", index + 1)
            } else {
                trimmed.to_owned()
            }
        } else {
            matlab_identifier_name(&name, index + 1)
        };
        let count = used.entry(base.clone()).or_default();
        let unique = if *count == 0 {
            base.clone()
        } else {
            format!("{base}_{count}")
        };
        *count += 1;
        result.push(TableVariableName::new(unique).map_err(table_error)?);
    }
    Ok(result)
}

fn matlab_identifier_name(value: &str, position: usize) -> String {
    let mut name = value.trim().to_owned();
    name.retain(|character| character.is_alphanumeric() || character == '_');
    if name.is_empty() {
        return format!("Var{position}");
    }
    if !name.chars().next().is_some_and(char::is_alphabetic) {
        name.insert(0, 'x');
    }
    name
}

fn infer_table_column(
    records: &[Vec<String>],
    column: usize,
    text_type: TableTextType,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let fields = records
        .iter()
        .map(|record| record.get(column).map_or("", String::as_str))
        .collect::<Vec<_>>();
    let mut parsed = Vec::with_capacity(fields.len());
    let mut numeric_anchor = false;
    let mut compatible = true;
    let mut complex = false;
    for (index, field) in fields.iter().enumerate() {
        let trimmed = field.trim();
        if table_missing_text(trimmed) {
            parsed.push(Complex64::new(f64::NAN, 0.0));
        } else if let Some((value, is_complex)) = parse_number(trimmed) {
            parsed.push(value);
            numeric_anchor = true;
            complex |= is_complex;
        } else {
            compatible = false;
            break;
        }
        if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
            context.check_cancelled()?;
        }
    }
    let rows = u64::try_from(fields.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`readtable` row count is too large",
        )
    })?;
    let shape = Shape::new([rows, 1]).map_err(|error| array_error(&error))?;
    let all_blank = fields.iter().all(|field| field.trim().is_empty());
    if compatible && (numeric_anchor || all_blank) {
        return if complex {
            DenseArray::from_vec(shape, parsed)
                .map(ArrayData::ComplexF64)
                .map(Value::Array)
        } else {
            DenseArray::from_vec(shape, parsed.into_iter().map(|value| value.re).collect())
                .map(ArrayData::F64)
                .map(Value::Array)
        }
        .map_err(|error| array_error(&error));
    }
    match text_type {
        TableTextType::String => {
            let elements = fields
                .into_iter()
                .map(|field| {
                    if field.is_empty() {
                        StringElement::missing()
                    } else {
                        StringElement::from_utf8(field)
                    }
                })
                .collect();
            StringArray::from_elements(shape, elements)
                .map(StringValue::Array)
                .map(Value::String)
                .map_err(|error| array_error(&error))
        }
        TableTextType::CellChar => {
            let values = fields
                .into_iter()
                .map(char_text)
                .collect::<Result<Vec<_>, _>>()?;
            CellArray::from_values(shape, values)
                .map(Value::Cell)
                .map_err(table_error)
        }
    }
}

fn table_missing_text(value: &str) -> bool {
    value.is_empty() || value.eq_ignore_ascii_case("na") || value.eq_ignore_ascii_case("missing")
}

fn delimiter_value(name: &str, position: usize, value: &Value) -> Result<Delimiter, BuiltinError> {
    let text = text_scalar(name, position, value)?;
    match text.to_ascii_lowercase().as_str() {
        "comma" => Ok(Delimiter::Character(',')),
        "tab" | "\\t" => Ok(Delimiter::Character('\t')),
        "space" => Ok(Delimiter::Whitespace),
        "semicolon" => Ok(Delimiter::Character(';')),
        "bar" | "pipe" => Ok(Delimiter::Character('|')),
        _ => {
            let mut characters = text.chars();
            let Some(character) = characters.next() else {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input {position} to `{name}` must name one delimiter"),
                ));
            };
            if characters.next().is_some() || matches!(character, '\r' | '\n' | '"') {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input {position} to `{name}` must name one delimiter"),
                ));
            }
            Ok(if character.is_whitespace() {
                Delimiter::Whitespace
            } else {
                Delimiter::Character(character)
            })
        }
    }
}

fn reject_spreadsheet_extension(name: &str, path: &str) -> Result<(), BuiltinError> {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if ["xls", "xlsx", "xlsm", "xlsb", "ods"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!(
                "`{name}` spreadsheet input is not implemented; text-delimited files are supported"
            ),
        ))
    } else {
        Ok(())
    }
}

fn decode_utf8_text(name: &str, path: &str, bytes: Vec<u8>) -> Result<String, BuiltinError> {
    String::from_utf8(bytes).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::FileSystem,
            format!(
                "`{name}` cannot decode `{path}` as UTF-8 text; invalid byte at offset {}",
                error.utf8_error().valid_up_to()
            ),
        )
    })
}

fn detect_delimiter(text: &str, path: &str) -> Delimiter {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("csv") {
        return Delimiter::Character(',');
    }
    let mut counts = [(0usize, ','), (0, '\t'), (0, ';'), (0, '|')];
    for line in text.lines().filter(|line| !line.trim().is_empty()).take(8) {
        let mut quoted = false;
        for character in line.chars() {
            if character == '"' {
                quoted = !quoted;
            } else if !quoted {
                for (count, candidate) in &mut counts {
                    if character == *candidate {
                        *count += 1;
                    }
                }
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(count, _)| *count)
        .filter(|(count, _)| *count > 0)
        .map_or(Delimiter::Whitespace, |(_, character)| {
            Delimiter::Character(character)
        })
}

fn detect_table_header(records: &[Vec<String>]) -> bool {
    let Some(first) = records.first() else {
        return false;
    };
    if first.is_empty() || first.iter().any(|field| field.trim().is_empty()) {
        return false;
    }
    !first.iter().any(|field| {
        let field = field.trim();
        !table_missing_text(field) && parse_number(field).is_some()
    })
}

fn validate_tsv_options(
    name: &str,
    path: &str,
    delimiter_explicit: bool,
    file_type_text: bool,
) -> Result<(), BuiltinError> {
    let is_tsv = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tsv"));
    if is_tsv && !(delimiter_explicit && file_type_text) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires FileType 'text' and an explicit Delimiter for .tsv files"),
        ));
    }
    Ok(())
}

fn parse_records(text: &str, delimiter: Delimiter) -> Result<Vec<Vec<String>>, BuiltinError> {
    if delimiter == Delimiter::Whitespace {
        return Ok(text
            .lines()
            .map(|line| line.split_whitespace().map(str::to_owned).collect())
            .collect());
    }
    let Delimiter::Character(delimiter) = delimiter else {
        unreachable!();
    };
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut characters = text.chars().peekable();
    let mut quoted = false;
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                field.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            value if value == delimiter && !quoted => {
                record.push(std::mem::take(&mut field));
            }
            '\r' | '\n' if !quoted => {
                if character == '\r' && characters.peek() == Some(&'\n') {
                    characters.next();
                }
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            value => field.push(value),
        }
    }
    if quoted {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "delimited text contains an unterminated quoted field",
        ));
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

fn matrix_from_records(
    records: &[Vec<String>],
    output_type: OutputType,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let rows = records.len();
    let columns = records.iter().map(Vec::len).max().unwrap_or(0);
    if rows == 0 || columns == 0 {
        let shape = Shape::new([0, 0]).map_err(|error| array_error(&error))?;
        return match output_type {
            OutputType::Double => DenseArray::from_vec(shape, Vec::<f64>::new())
                .map(ArrayData::F64)
                .map(Value::Array),
            OutputType::Single => DenseArray::from_vec(shape, Vec::<f32>::new())
                .map(ArrayData::F32)
                .map(Value::Array),
        }
        .map(|value| vec![value])
        .map_err(|error| array_error(&error));
    }
    let length = rows.checked_mul(columns).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`readmatrix` result is too large",
        )
    })?;
    let mut values = vec![Complex64::new(f64::NAN, 0.0); length];
    let mut complex = false;
    for (row, record) in records.iter().enumerate() {
        for (column, field) in record.iter().enumerate() {
            let offset = row + column * rows;
            if let Some((value, is_complex)) = parse_number(field) {
                values[offset] = value;
                complex |= is_complex;
            }
            if offset.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                context.check_cancelled()?;
            }
        }
    }
    let rows = u64::try_from(rows).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`readmatrix` row count is too large",
        )
    })?;
    let columns = u64::try_from(columns).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`readmatrix` column count is too large",
        )
    })?;
    let shape = Shape::new([rows, columns]).map_err(|error| array_error(&error))?;
    match (output_type, complex) {
        (OutputType::Double, false) => {
            DenseArray::from_vec(shape, values.into_iter().map(|value| value.re).collect())
                .map(ArrayData::F64)
                .map(Value::Array)
        }
        (OutputType::Double, true) => DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array),
        (OutputType::Single, false) => DenseArray::from_vec(
            shape,
            values
                .into_iter()
                .map(|value| narrow_to_single(value.re))
                .collect(),
        )
        .map(ArrayData::F32)
        .map(Value::Array),
        (OutputType::Single, true) => DenseArray::from_vec(
            shape,
            values
                .into_iter()
                .map(|value| Complex32::new(narrow_to_single(value.re), narrow_to_single(value.im)))
                .collect(),
        )
        .map(ArrayData::ComplexF32)
        .map(Value::Array),
    }
    .map(|value| vec![value])
    .map_err(|error| array_error(&error))
}

fn parse_number(field: &str) -> Option<(Complex64, bool)> {
    let field = field.trim();
    if field.is_empty() || field.eq_ignore_ascii_case("na") || field.eq_ignore_ascii_case("missing")
    {
        return Some((Complex64::new(f64::NAN, 0.0), false));
    }
    if let Some(value) = parse_real(field) {
        return Some((Complex64::new(value, 0.0), false));
    }
    let body = field.strip_suffix(['i', 'j'])?.trim();
    let split = body
        .char_indices()
        .skip(1)
        .filter(|(index, character)| {
            matches!(character, '+' | '-')
                && !matches!(
                    body.as_bytes().get(index.saturating_sub(1)),
                    Some(b'e' | b'E')
                )
        })
        .map(|(index, _)| index)
        .last();
    let (real, imaginary) = if let Some(split) = split {
        (
            parse_real(&body[..split])?,
            parse_imaginary(&body[split..])?,
        )
    } else {
        (0.0, parse_imaginary(body)?)
    };
    Some((Complex64::new(real, imaginary), true))
}

fn parse_real(value: &str) -> Option<f64> {
    match value.trim().to_ascii_lowercase().as_str() {
        "inf" | "+inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        "nan" | "+nan" | "-nan" => Some(f64::NAN),
        _ => value.trim().parse().ok(),
    }
}

fn parse_imaginary(value: &str) -> Option<f64> {
    match value.trim() {
        "" | "+" => Some(1.0),
        "-" => Some(-1.0),
        other => parse_real(other),
    }
}

fn format_table(
    table: &TableArray,
    options: WriteTableOptions,
    context: &BuiltinContext<'_>,
) -> Result<String, BuiltinError> {
    let separator = match options.delimiter {
        Delimiter::Character(character) => character,
        Delimiter::Whitespace => ' ',
    };
    let rows = usize::try_from(table.row_count()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`writetable` row count is too large",
        )
    })?;
    let mut widths = Vec::with_capacity(table.variable_count());
    for index in 0..table.variable_count() {
        let variable = table.variable(index).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                "`writetable` encountered incomplete variable storage",
            )
        })?;
        widths.push(table_variable_output_width(variable, table.row_count())?);
    }

    let mut output = String::new();
    if options.write_variable_names {
        let mut first = true;
        for (index, width) in widths.iter().copied().enumerate() {
            let name = table.variable_names()[index].as_str();
            for column in 0..width {
                if !first {
                    output.push(separator);
                }
                first = false;
                let header = if width == 1 {
                    name.to_owned()
                } else {
                    format!("{name}_{}", column + 1)
                };
                output.push_str(&delimited_field(&header, separator, false));
            }
        }
        output.push_str(platform_newline());
    }
    for row in 0..rows {
        let mut first = true;
        for (variable_index, width) in widths.iter().copied().enumerate() {
            let variable = table.variable(variable_index).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Other,
                    "`writetable` encountered incomplete variable storage",
                )
            })?;
            for column in 0..width {
                if !first {
                    output.push(separator);
                }
                first = false;
                let (field, text) = table_field(variable, row, column, rows)?;
                output.push_str(&delimited_field(&field, separator, text));
                let checkpoint = row.saturating_mul(width).saturating_add(column);
                if checkpoint.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                    context.check_cancelled()?;
                }
            }
        }
        output.push_str(platform_newline());
    }
    Ok(output)
}

fn table_variable_output_width(value: &Value, rows: u64) -> Result<usize, BuiltinError> {
    if matches!(value, Value::Array(ArrayData::Char(_))) {
        return Ok(1);
    }
    let dimensions = value
        .dimensions()
        .ok_or_else(|| type_error("writetable", 1, "table with array-valued variables", value))?;
    if dimensions.len() != 2 || dimensions[0] != rows {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`writetable` currently requires two-dimensional table variables",
        ));
    }
    usize::try_from(dimensions[1]).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`writetable` variable width is too large",
        )
    })
}

fn table_field(
    value: &Value,
    row: usize,
    column: usize,
    rows: usize,
) -> Result<(String, bool), BuiltinError> {
    if let Value::Array(ArrayData::Char(array)) = value {
        let columns = usize::try_from(array.shape().extent(1)).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`writetable` char variable width is too large",
            )
        })?;
        let code_units = (0..columns)
            .filter_map(|column| array.as_slice().get(row + column * rows))
            .map(|value| value.get())
            .collect::<Vec<_>>();
        return Ok((String::from_utf16_lossy(&code_units), false));
    }
    let offset = row
        .checked_add(column.saturating_mul(rows))
        .ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`writetable` variable offset overflowed",
            )
        })?;
    if numeric_dimensions(value).is_some() {
        return format_numeric_element(value, offset).map(|value| (value, false));
    }
    match value {
        Value::String(strings) => {
            let element = strings.element(offset).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Other,
                    "`writetable` string variable storage is incomplete",
                )
            })?;
            Ok((
                if element.is_missing() {
                    String::new()
                } else {
                    element.to_utf8_lossy()
                },
                false,
            ))
        }
        Value::Cell(cell) => {
            let element = cell.value_at_offset(offset).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Other,
                    "`writetable` cell variable storage is incomplete",
                )
            })?;
            if numeric_dimensions(element).is_some() && element.numel() == Some(1) {
                return format_numeric_element(element, 0).map(|value| (value, false));
            }
            match element {
                Value::Array(ArrayData::Char(_)) | Value::String(_) => {
                    text_scalar("writetable", 1, element).map(|value| (value, false))
                }
                _ => Err(type_error(
                    "writetable",
                    1,
                    "table containing scalar numeric or text cell values",
                    element,
                )),
            }
        }
        _ => Err(type_error(
            "writetable",
            1,
            "table containing numeric, logical, char, string, or cell variables",
            value,
        )),
    }
}

fn delimited_field(value: &str, separator: char, always_quote: bool) -> String {
    if always_quote
        || value
            .chars()
            .any(|character| matches!(character, '"' | '\r' | '\n') || character == separator)
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn platform_newline() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

fn table_error(error: impl std::fmt::Display) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string())
}

fn format_matrix(
    value: &Value,
    delimiter: Delimiter,
    context: &BuiltinContext<'_>,
) -> Result<String, BuiltinError> {
    let dimensions = numeric_dimensions(value).ok_or_else(|| {
        type_error(
            "writematrix",
            1,
            "real or complex numeric, integer, or logical matrix",
            value,
        )
    })?;
    if dimensions.len() != 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 1 to `writematrix` must be a two-dimensional matrix",
        ));
    }
    let rows = usize::try_from(dimensions[0]).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`writematrix` row count is too large",
        )
    })?;
    let columns = usize::try_from(dimensions[1]).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`writematrix` column count is too large",
        )
    })?;
    let separator = match delimiter {
        Delimiter::Character(character) => character,
        Delimiter::Whitespace => ' ',
    };
    let mut output = String::new();
    for row in 0..rows {
        for column in 0..columns {
            if column != 0 {
                output.push(separator);
            }
            let offset = row + column * rows;
            output.push_str(&format_numeric_element(value, offset)?);
            if offset.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                context.check_cancelled()?;
            }
        }
        output.push_str(if cfg!(windows) { "\r\n" } else { "\n" });
    }
    Ok(output)
}

fn numeric_dimensions(value: &Value) -> Option<Vec<u64>> {
    match value {
        Value::Array(array @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) => {
            Some(array.shape().dimensions().to_vec())
        }
        Value::Array(ArrayData::F64(array)) => Some(array.shape().dimensions().to_vec()),
        Value::Array(ArrayData::ComplexF64(array)) => Some(array.shape().dimensions().to_vec()),
        Value::Array(ArrayData::Logical(array)) => Some(array.shape().dimensions().to_vec()),
        Value::Array(ArrayData::Integer(array)) => Some(array.shape().dimensions().to_vec()),
        Value::Double(_) | Value::Complex(_) | Value::Logical(_) => Some(vec![1, 1]),
        _ => None,
    }
}

fn format_numeric_element(value: &Value, offset: usize) -> Result<String, BuiltinError> {
    match value {
        Value::Double(value) if offset == 0 => Some(format_float(*value, 15)),
        Value::Complex(value) if offset == 0 => {
            Some(format_complex(value.real, value.imaginary, 15))
        }
        Value::Logical(value) if offset == 0 => Some(if *value { "1" } else { "0" }.to_owned()),
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| format_float(f64::from(*value), 7)),
        Value::Array(ArrayData::ComplexF32(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| format_complex(f64::from(value.re), f64::from(value.im), 7)),
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| format_float(*value, 15)),
        Value::Array(ArrayData::ComplexF64(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| format_complex(value.re, value.im, 15)),
        Value::Array(ArrayData::Logical(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| if bool::from(*value) { "1" } else { "0" }.to_owned()),
        Value::Array(ArrayData::Integer(array)) => {
            array.element(offset).map(format_integer_element)
        }
        _ => None,
    }
    .ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "`writematrix` could not access a validated matrix element",
        )
    })
}

fn format_integer_element(value: openmat_array::IntegerElementValue) -> String {
    let real = value.real_component().canonical_decimal();
    let Some(imaginary) = value.imaginary_component() else {
        return real;
    };
    let imaginary = imaginary.canonical_decimal();
    if imaginary.starts_with('-') {
        format!("{real}{imaginary}i")
    } else {
        format!("{real}+{imaginary}i")
    }
}

fn format_complex(real: f64, imaginary: f64, precision: usize) -> String {
    let real = format_float(real, precision);
    let imaginary_text = format_float(imaginary.abs(), precision);
    if imaginary.is_sign_negative() {
        format!("{real}-{imaginary_text}i")
    } else {
        format!("{real}+{imaginary_text}i")
    }
}

fn format_float(value: f64, precision: usize) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == f64::INFINITY {
        return "Inf".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-Inf".to_owned();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_owned();
    }
    let exponent = decimal_exponent(value);
    if exponent < -4 || exponent >= i32::try_from(precision).unwrap_or(i32::MAX) {
        let scientific = format!("{:.*e}", precision.saturating_sub(1), value);
        normalize_scientific(&scientific)
    } else {
        let decimals = usize::try_from(i32::try_from(precision).unwrap_or(i32::MAX) - exponent - 1)
            .unwrap_or(0);
        trim_decimal(format!("{value:.decimals$}"))
    }
}

fn normalize_scientific(value: &str) -> String {
    let Some((mantissa, exponent)) = value.split_once('e') else {
        return value.to_owned();
    };
    let mantissa = trim_decimal(mantissa.to_owned());
    let exponent_value = exponent.parse::<i32>().unwrap_or(0);
    let sign = if exponent_value < 0 { '-' } else { '+' };
    let magnitude = exponent_value.unsigned_abs();
    format!("{mantissa}e{sign}{magnitude:02}")
}

fn trim_decimal(mut value: String) -> String {
    if value.contains('.') {
        while value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
    }
    value
}

fn text_scalar(name: &str, position: usize, value: &Value) -> Result<String, BuiltinError> {
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

fn nonnegative_integer_scalar(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<usize, BuiltinError> {
    let value = match value {
        Value::Double(value) => *value,
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => array.as_slice()[0],
        _ => {
            return Err(type_error(
                name,
                position,
                "nonnegative integer scalar",
                value,
            ));
        }
    };
    if !value.is_finite()
        || value < 0.0
        || value.fract() != 0.0
        || value >= 18_446_744_073_709_551_616.0
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a nonnegative integer scalar"),
        ));
    }
    usize::try_from(nonnegative_f64_to_u64(value)).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` exceeds this host's index range"),
        )
    })
}

#[allow(clippy::cast_possible_truncation)]
fn narrow_to_single(value: f64) -> f32 {
    value as f32
}

#[allow(clippy::cast_possible_truncation)]
fn decimal_exponent(value: f64) -> i32 {
    value.abs().log10().floor() as i32
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn nonnegative_f64_to_u64(value: f64) -> u64 {
    value as u64
}

fn char_text(text: &str) -> Result<Value, BuiltinError> {
    let values = text
        .encode_utf16()
        .map(CharCodeUnit::new)
        .collect::<Vec<_>>();
    let length = u64::try_from(values.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`fileread` result is too large",
        )
    })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use openmat_runtime::{CancellationToken, LocalFileSystem, NullOutput};

    fn temporary_directory() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "openmat-file-builtins-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn path_value(path: &str) -> Value {
        Value::from(path)
    }

    #[test]
    fn save_options_accept_v73_and_track_explicit_format() {
        let options = parse_save_options(&[path_value("-v7.3")]).unwrap();
        assert_eq!(options.version, MatVersion::V73);
        assert!(options.version_explicit);
        assert!(!options.append);

        let options = parse_save_options(&[path_value("-append")]).unwrap();
        assert_eq!(options.version, MatVersion::V7);
        assert!(!options.version_explicit);
        assert!(options.append);
    }

    #[test]
    fn fileread_preserves_bom_and_line_endings_and_empty_shape() {
        let directory = temporary_directory();
        std::fs::write(
            directory.join("sample.txt"),
            [0xef, 0xbb, 0xbf, b'A', b'\r', b'\n'],
        )
        .unwrap();
        std::fs::write(directory.join("empty.txt"), []).unwrap();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context =
            BuiltinContext::with_file_system_service(1, &cancellation, &mut output, &mut files);
        let value = fileread_builtin(&[path_value("sample.txt")], &mut context).unwrap();
        let Value::Array(ArrayData::Char(value)) = &value[0] else {
            panic!("expected char output");
        };
        assert_eq!(value.shape().dimensions(), [1, 4]);
        assert_eq!(
            value
                .as_slice()
                .iter()
                .map(|value| value.get())
                .collect::<Vec<_>>(),
            [0xfeff, 65, 13, 10]
        );
        let empty = fileread_builtin(&[path_value("empty.txt")], &mut context).unwrap();
        assert_eq!(empty[0].dimensions(), Some([0, 0].as_slice()));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn readmatrix_detects_headers_ragged_rows_and_complex_values() {
        let directory = temporary_directory();
        std::fs::write(
            directory.join("sample.csv"),
            "name,a,b,c\r\nrow,1,2,\r\nother,3+4i,NA,-Inf\r\n",
        )
        .unwrap();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context =
            BuiltinContext::with_file_system_service(1, &cancellation, &mut output, &mut files);
        let value = readmatrix_builtin(&[path_value("sample.csv")], &mut context).unwrap();
        let Value::Array(ArrayData::ComplexF64(value)) = &value[0] else {
            panic!("expected complex double matrix");
        };
        assert_eq!(value.shape().dimensions(), [2, 4]);
        assert!(value.as_slice()[0].re.is_nan());
        assert_eq!(value.as_slice()[3], Complex64::new(3.0, 4.0));
        assert!(value.as_slice()[5].re.is_nan());
        assert!(value.as_slice()[6].re.is_nan());
        assert_eq!(value.as_slice()[7], Complex64::new(f64::NEG_INFINITY, 0.0));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn writematrix_uses_matlab_text_order_and_roundtrips() {
        let directory = temporary_directory();
        let shape = Shape::new([2, 3]).unwrap();
        let matrix = Value::Array(ArrayData::F64(
            DenseArray::from_vec(shape, vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]).unwrap(),
        ));
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context =
            BuiltinContext::with_file_system_service(0, &cancellation, &mut output, &mut files);
        writematrix_builtin(&[matrix, path_value("matrix.csv")], &mut context).unwrap();
        let expected = if cfg!(windows) {
            "1,2,3\r\n4,5,6\r\n"
        } else {
            "1,2,3\n4,5,6\n"
        };
        assert_eq!(
            std::fs::read_to_string(directory.join("matrix.csv")).unwrap(),
            expected
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn readtable_infers_numeric_and_text_columns_with_ordered_names() {
        let directory = temporary_directory();
        std::fs::write(
            directory.join("sample.csv"),
            "First Name,score,note\r\nAlice,1,ok\r\nBob,,\"a,b\"\r\n",
        )
        .unwrap();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context =
            BuiltinContext::with_file_system_service(1, &cancellation, &mut output, &mut files);
        let loaded = readtable_builtin(&[path_value("sample.csv")], &mut context).unwrap();
        let Value::Table(table) = &loaded[0] else {
            panic!("expected table output");
        };
        assert_eq!(table.shape().dimensions(), [2, 3]);
        assert_eq!(
            table
                .variable_names()
                .iter()
                .map(TableVariableName::as_str)
                .collect::<Vec<_>>(),
            ["FirstName", "score", "note"]
        );
        let Value::Array(ArrayData::F64(score)) = table.variable(1).unwrap() else {
            panic!("score must be inferred as double");
        };
        assert_eq!(score.as_slice()[0].to_bits(), 1.0_f64.to_bits());
        assert!(score.as_slice()[1].is_nan());
        assert!(matches!(table.variable(0), Some(Value::Cell(_))));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn readtable_separates_header_detection_from_name_selection() {
        let directory = temporary_directory();
        std::fs::write(
            directory.join("headerless.csv"),
            "10,alpha,\r\n20,beta,\r\n",
        )
        .unwrap();
        std::fs::write(directory.join("header.csv"), "Number,Word\r\n10,alpha\r\n").unwrap();
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context =
            BuiltinContext::with_file_system_service(1, &cancellation, &mut output, &mut files);

        let loaded = readtable_builtin(&[path_value("headerless.csv")], &mut context).unwrap();
        let Value::Table(headerless) = &loaded[0] else {
            panic!("expected table output");
        };
        assert_eq!(headerless.shape().dimensions(), [2, 3]);
        assert_eq!(
            headerless
                .variable_names()
                .iter()
                .map(TableVariableName::as_str)
                .collect::<Vec<_>>(),
            ["Var1", "Var2", "Var3"]
        );
        let Value::Array(ArrayData::F64(blank)) = headerless.variable(2).unwrap() else {
            panic!("all-blank columns must infer as double");
        };
        assert!(blank.as_slice().iter().all(|value| value.is_nan()));

        let loaded = readtable_builtin(
            &[
                path_value("header.csv"),
                path_value("ReadVariableNames"),
                Value::Logical(false),
            ],
            &mut context,
        )
        .unwrap();
        let Value::Table(generated) = &loaded[0] else {
            panic!("expected table output");
        };
        assert_eq!(generated.row_count(), 1);
        assert_eq!(
            generated
                .variable_names()
                .iter()
                .map(TableVariableName::as_str)
                .collect::<Vec<_>>(),
            ["Var1", "Var2"]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn writetable_expands_matrix_variables_and_quotes_text() {
        let directory = temporary_directory();
        let numeric = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![1.0, 3.0, 2.0, 4.0]).unwrap(),
        ));
        let text = Value::Cell(
            CellArray::from_values(
                Shape::new([2, 1]).unwrap(),
                vec![char_text("x").unwrap(), char_text("a,b").unwrap()],
            )
            .unwrap(),
        );
        let table = Value::Table(
            TableArray::from_variables(
                vec![
                    TableVariableName::new("A").unwrap(),
                    TableVariableName::new("B").unwrap(),
                ],
                vec![numeric, text],
            )
            .unwrap(),
        );
        let mut files = LocalFileSystem::new(&directory).unwrap();
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context =
            BuiltinContext::with_file_system_service(0, &cancellation, &mut output, &mut files);
        writetable_builtin(&[table, path_value("table.csv")], &mut context).unwrap();
        let newline = platform_newline();
        assert_eq!(
            std::fs::read_to_string(directory.join("table.csv")).unwrap(),
            format!("A_1,A_2,B{newline}1,2,x{newline}3,4,\"a,b\"{newline}")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn matlab_general_number_format_matches_measured_r2022b_examples() {
        assert_eq!(format_float(1.0 / 3.0, 15), "0.333333333333333");
        assert_eq!(format_float(-0.0, 15), "-0");
        assert_eq!(format_float(1e20, 15), "1e+20");
        assert_eq!(format_float(1e-10, 15), "1e-10");
        assert_eq!(format_float(f64::INFINITY, 15), "Inf");
        assert_eq!(format_float(f64::NAN, 15), "NaN");
        assert_eq!(format_float(f64::from(1.0_f32 / 3.0), 7), "0.3333333");
    }
}

//! MATLAB-compatible filesystem metadata, mutation, and path built-ins.

use std::{env, path::Path, time::UNIX_EPOCH};

use openmat_array::{ArrayData, CharCodeUnit, DenseArray, Logical, Shape};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistrationError, BuiltinRegistry,
    BuiltinResult, FileSystemMetadata, SearchPathPosition,
};
use openmat_value::{FieldName, StringArray, StringElement, StringValue, StructArray, Value};

use crate::{
    array_error, expect_argument_count, expect_argument_count_range, expect_max_outputs, type_error,
};

const DIRECTORY_FIELDS: [&str; 6] = ["name", "folder", "date", "bytes", "isdir", "datenum"];

/// Registers the filesystem/path tranche without changing the shared default registry.
///
/// The integration task owns the single call site in `lib.rs`; keeping the
/// function here lets focused tests validate the complete registration list.
#[allow(dead_code)]
pub(super) fn register_filesystem_extended(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("dir", dir_builtin)?;
    registry.register("exist", exist_builtin)?;
    registry.register("isfile", isfile_builtin)?;
    registry.register("isfolder", isfolder_builtin)?;
    registry.register("mkdir", mkdir_builtin)?;
    registry.register("rmdir", rmdir_builtin)?;
    registry.register("delete", delete_builtin)?;
    registry.register("isvalid", isvalid_builtin)?;
    registry.register("copyfile", copyfile_builtin)?;
    registry.register("movefile", movefile_builtin)?;
    registry.register("fullfile", fullfile_builtin)?;
    registry.register("fileparts", fileparts_builtin)?;
    registry.register("which", which_builtin)?;
    registry.register("path", path_builtin)?;
    registry.register("addpath", addpath_builtin)?;
    registry.register("rmpath", rmpath_builtin)?;
    registry.register("genpath", genpath_builtin)?;
    Ok(())
}

fn path_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("path", context, 1)?;
    let previous = search_path_text(context)?;
    if !arguments.is_empty() {
        let paths = path_arguments("path", arguments)?;
        context.replace_search_path(&paths)?;
    }
    if context.requested_outputs() == 0 {
        if arguments.is_empty() {
            context.emit(openmat_runtime::OutputEvent::CommandText(format!(
                "{previous}\n"
            )))?;
        }
        Ok(Vec::new())
    } else {
        char_value(&previous).map(|value| vec![value])
    }
}

fn addpath_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`addpath` expects at least one directory",
        ));
    }
    expect_max_outputs("addpath", context, 1)?;
    let previous = search_path_text(context)?;
    let mut values = arguments;
    let position = arguments
        .last()
        .and_then(|value| text_scalar("addpath", arguments.len(), value).ok())
        .and_then(|option| match option.to_ascii_lowercase().as_str() {
            "-begin" => Some(SearchPathPosition::Begin),
            "-end" => Some(SearchPathPosition::End),
            _ => None,
        });
    if position.is_some() {
        values = &arguments[..arguments.len() - 1];
    }
    if values.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`addpath` requires a directory before its placement option",
        ));
    }
    let paths = path_arguments("addpath", values)?;
    context.add_search_path(&paths, position.unwrap_or(SearchPathPosition::Begin))?;
    if context.requested_outputs() == 0 {
        Ok(Vec::new())
    } else {
        char_value(&previous).map(|value| vec![value])
    }
}

fn rmpath_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`rmpath` expects at least one directory",
        ));
    }
    expect_max_outputs("rmpath", context, 1)?;
    let previous = search_path_text(context)?;
    let paths = path_arguments("rmpath", arguments)?;
    context.remove_search_path(&paths)?;
    if context.requested_outputs() == 0 {
        Ok(Vec::new())
    } else {
        char_value(&previous).map(|value| vec![value])
    }
}

fn genpath_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("genpath", arguments, 1)?;
    expect_max_outputs("genpath", context, 1)?;
    let root = text_scalar("genpath", 1, &arguments[0])?;
    let paths = context.generate_search_path(&root)?;
    let mut text = join_search_paths(paths.iter().map(std::path::PathBuf::as_path))?;
    if !text.is_empty() {
        text.push(if cfg!(windows) { ';' } else { ':' });
    }
    char_value(&text).map(|value| vec![value])
}

fn search_path_text(context: &mut BuiltinContext<'_>) -> Result<String, BuiltinError> {
    let snapshot = context.search_path()?;
    join_search_paths(snapshot.paths.iter().map(std::path::PathBuf::as_path))
}

fn join_search_paths<'a>(
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<String, BuiltinError> {
    env::join_paths(paths)
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::FileSystem,
                format!("search path cannot be represented: {error}"),
            )
        })
}

fn path_arguments(
    function: &'static str,
    arguments: &[Value],
) -> Result<Vec<String>, BuiltinError> {
    let mut paths = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let value = text_scalar(function, index + 1, argument)?;
        if value.is_empty() {
            continue;
        }
        paths.extend(
            env::split_paths(&value)
                .map(|path| path.to_string_lossy().into_owned())
                .filter(|path| !path.is_empty()),
        );
    }
    Ok(paths)
}

fn dir_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("dir", arguments, 0, 1)?;
    expect_max_outputs("dir", context, 1)?;
    let path = arguments
        .first()
        .map(|value| text_scalar("dir", 1, value))
        .transpose()?
        .unwrap_or_else(|| ".".to_owned());
    if path.is_empty() {
        return directory_value(Vec::new()).map(|value| vec![value]);
    }
    let entries = context.directory_entries(&path).unwrap_or_default();
    directory_value(entries).map(|value| vec![value])
}

fn exist_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("exist", arguments, 1, 2)?;
    expect_max_outputs("exist", context, 1)?;
    let name = text_scalar("exist", 1, &arguments[0])?;
    let kind = arguments
        .get(1)
        .map(|value| text_scalar("exist", 2, value))
        .transpose()?
        .map(|value| value.to_ascii_lowercase());

    let workspace_match = kind.as_deref().is_none_or(|value| value == "var")
        && context
            .workspace()
            .is_ok_and(|workspace| workspace.contains(&name));
    if workspace_match {
        return Ok(vec![Value::Double(1.0)]);
    }
    if kind.as_deref() == Some("var") || name.is_empty() {
        return Ok(vec![Value::Double(0.0)]);
    }
    if matches!(kind.as_deref(), Some("builtin" | "class")) {
        return Ok(vec![Value::Double(0.0)]);
    }

    let metadata = context.file_metadata(&name).ok().or_else(|| {
        (kind.as_deref() != Some("dir") && Path::new(&name).extension().is_none())
            .then(|| context.file_metadata(&format!("{name}.m")).ok())
            .flatten()
    });
    let caller = context.caller_source().map(str::to_owned);
    let resolved_source = (metadata.is_none()
        && kind.as_deref() != Some("dir")
        && Path::new(&name).extension().is_none())
    .then(|| {
        context
            .resolve_matlab_source(caller.as_deref(), &name)
            .ok()
            .flatten()
    })
    .flatten();
    let metadata = metadata.or_else(|| {
        resolved_source.map(|source| FileSystemMetadata {
            name: source
                .path
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            path: source.path,
            bytes: 0,
            is_file: true,
            is_directory: false,
            modified: None,
        })
    });
    let code = metadata.map_or(0, |metadata| {
        if metadata.is_directory {
            7
        } else if kind.as_deref() == Some("dir") {
            0
        } else {
            file_exist_code(&metadata.path)
        }
    });
    Ok(vec![Value::Double(f64::from(code))])
}

fn file_exist_code(path: &Path) -> i32 {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mexw64" | "mexa64" | "mexmaci64") => 3,
        Some("mdl" | "slx") => 4,
        Some("p") => 6,
        _ => 2,
    }
}

fn isfile_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    file_predicate_builtin("isfile", arguments, context, |metadata| metadata.is_file)
}

fn isfolder_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    file_predicate_builtin("isfolder", arguments, context, |metadata| {
        metadata.is_directory
    })
}

fn file_predicate_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    predicate: impl Fn(&FileSystemMetadata) -> bool,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    let paths = text_values(name, 1, &arguments[0])?;
    let values = paths
        .values
        .iter()
        .map(|path| {
            path.as_ref()
                .and_then(|path| context.file_metadata(path).ok())
                .is_some_and(|metadata| predicate(&metadata))
        })
        .collect::<Vec<_>>();
    paths.logical_result(values).map(|value| vec![value])
}

fn mkdir_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("mkdir", arguments, 1, 2)?;
    expect_max_outputs("mkdir", context, 3)?;
    let mut path = text_scalar("mkdir", 1, &arguments[0])?;
    if let Some(child) = arguments.get(1) {
        path = join_platform_paths(&[path, text_scalar("mkdir", 2, child)?]);
    }
    let existed = context
        .file_metadata(&path)
        .is_ok_and(|metadata| metadata.is_directory);
    let result = context.create_directory(&path);
    if result.is_ok() && existed {
        status_outputs(
            context,
            true,
            "Directory already exists.",
            "MATLAB:MKDIR:DirectoryExists",
            "mkdir",
        )
    } else {
        mutation_outputs(context, result, "mkdir", "MATLAB:MKDIR:OSError")
    }
}

fn rmdir_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("rmdir", arguments, 1, 2)?;
    expect_max_outputs("rmdir", context, 3)?;
    let path = text_scalar("rmdir", 1, &arguments[0])?;
    let recursive = arguments
        .get(1)
        .map(|value| text_scalar("rmdir", 2, value))
        .transpose()?
        .is_some_and(|option| option.eq_ignore_ascii_case("s"));
    if arguments.len() == 2 && !recursive {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 2 to `rmdir` must be 's'",
        ));
    }
    let result = context.remove_directory(&path, recursive);
    mutation_outputs(
        context,
        result,
        "rmdir",
        "MATLAB:RMDIR:NoDirectoriesRemoved",
    )
}

fn delete_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "built-in `delete` expects at least one input",
        ));
    }
    expect_max_outputs("delete", context, 0)?;

    if matches!(arguments[0], Value::Object(_) | Value::ObjectArray(_)) {
        if arguments.len() != 1 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "object `delete` accepts exactly one scalar or homogeneous object array",
            ));
        }
        let expected = object_element_count("delete", &arguments[0])?;
        if let Some(elements) = context.delete_object(&arguments[0])?
            && elements != expected
        {
            return Err(lifecycle_result_length_error(
                "delete array",
                expected,
                elements,
            ));
        }
        return Ok(Vec::new());
    }
    if arguments
        .iter()
        .any(|argument| matches!(argument, Value::Object(_) | Value::ObjectArray(_)))
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`delete` cannot mix filesystem paths and object receivers",
        ));
    }

    for (index, argument) in arguments.iter().enumerate() {
        let path = text_scalar("delete", index + 1, argument)?;
        if has_wildcard(&path) {
            for entry in context.directory_entries(&path).unwrap_or_default() {
                if entry.is_file {
                    let entry_path = entry.path.to_string_lossy().into_owned();
                    context.remove_file(&entry_path)?;
                }
            }
        } else if context
            .file_metadata(&path)
            .is_ok_and(|metadata| metadata.is_file)
        {
            context.remove_file(&path)?;
        }
    }
    Ok(Vec::new())
}

fn object_element_count(name: &str, value: &Value) -> Result<usize, BuiltinError> {
    match value {
        Value::Object(_) => Ok(1),
        Value::ObjectArray(array) => usize::try_from(array.numel()).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("the object array is too large for `{name}`"),
            )
        }),
        _ => Err(type_error(
            name,
            1,
            "object scalar or homogeneous object array",
            value,
        )),
    }
}

/// Implements the handle-class validity predicate through the runtime-owned
/// object-lifecycle service. The integration task owns default registration.
#[allow(dead_code)]
pub(super) fn isvalid_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("isvalid", arguments, 1)?;
    expect_max_outputs("isvalid", context, 1)?;

    context
        .object_validity(&arguments[0])
        .map(|value| vec![value])
}

fn lifecycle_result_length_error(kind: &str, expected: usize, actual: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::HostService,
        format!(
            "the object-lifecycle service reported {actual} processed elements for an object {kind} with {expected} elements"
        ),
    )
}

fn copyfile_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    transfer_builtin("copyfile", arguments, context, false)
}

fn movefile_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    transfer_builtin("movefile", arguments, context, true)
}

fn transfer_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    moving: bool,
) -> BuiltinResult {
    expect_argument_count_range(name, arguments, 2, 3)?;
    expect_max_outputs(name, context, 3)?;
    let source = text_scalar(name, 1, &arguments[0])?;
    let destination = text_scalar(name, 2, &arguments[1])?;
    let force = arguments
        .get(2)
        .map(|value| text_scalar(name, 3, value))
        .transpose()?
        .is_some_and(|option| option.eq_ignore_ascii_case("f"));
    if arguments.len() == 3 && !force {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input 3 to `{name}` must be 'f'"),
        ));
    }

    let operation = if has_wildcard(&source) {
        transfer_wildcard(context, &source, &destination, force, moving)
    } else if moving {
        context.move_path(&source, &destination, force)
    } else {
        context.copy_path(&source, &destination, force)
    };
    let identifier = if moving {
        "MATLAB:MOVEFILE:OSError"
    } else {
        "MATLAB:COPYFILE:OSError"
    };
    mutation_outputs(context, operation, name, identifier)
}

fn transfer_wildcard(
    context: &mut BuiltinContext<'_>,
    source: &str,
    destination: &str,
    force: bool,
    moving: bool,
) -> Result<(), BuiltinError> {
    let entries = context
        .directory_entries(source)?
        .into_iter()
        .filter(|entry| !matches!(entry.name.as_str(), "." | ".."))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::FileSystem,
            format!("no files match `{source}`"),
        ));
    }
    if !context
        .file_metadata(destination)
        .is_ok_and(|metadata| metadata.is_directory)
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::FileSystem,
            "a wildcard transfer destination must be an existing directory",
        ));
    }
    for entry in entries {
        let source = entry.path.to_string_lossy().into_owned();
        if moving {
            context.move_path(&source, destination, force)?;
        } else {
            context.copy_path(&source, destination, force)?;
        }
    }
    Ok(())
}

fn fullfile_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "built-in `fullfile` expects at least one path component",
        ));
    }
    expect_max_outputs("fullfile", context, 1)?;
    let inputs = arguments
        .iter()
        .enumerate()
        .map(|(index, value)| text_values("fullfile", index + 1, value))
        .collect::<Result<Vec<_>, _>>()?;
    let output_shape = common_text_shape("fullfile", &inputs)?;
    let output_length = output_shape.as_ref().map_or(1, |shape| {
        usize::try_from(shape.numel()).unwrap_or(usize::MAX)
    });
    let mut values = Vec::with_capacity(output_length);
    for offset in 0..output_length {
        let components = inputs
            .iter()
            .map(|input| input.expanded_value(offset))
            .collect::<Option<Vec<_>>>();
        values.push(components.map(|components| join_platform_paths(&components)));
    }
    let any_string = inputs.iter().any(|input| input.string_input);
    if any_string {
        string_values(output_shape.unwrap_or_else(scalar_shape), values).map(|value| vec![value])
    } else {
        char_value(
            values
                .first()
                .and_then(Option::as_deref)
                .unwrap_or_default(),
        )
        .map(|value| vec![value])
    }
}

fn fileparts_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("fileparts", arguments, 1)?;
    expect_max_outputs("fileparts", context, 3)?;
    let input = text_values("fileparts", 1, &arguments[0])?;
    let mut paths = Vec::with_capacity(input.values.len());
    let mut names = Vec::with_capacity(input.values.len());
    let mut extensions = Vec::with_capacity(input.values.len());
    for value in &input.values {
        if let Some(value) = value {
            let (path, name, extension) = split_fileparts(value);
            paths.push(Some(path));
            names.push(Some(name));
            extensions.push(Some(extension));
        } else {
            paths.push(None);
            names.push(None);
            extensions.push(None);
        }
    }
    let outputs = if input.string_input {
        let shape = input.shape.clone().unwrap_or_else(scalar_shape);
        vec![
            string_values(shape.clone(), paths)?,
            string_values(shape.clone(), names)?,
            string_values(shape, extensions)?,
        ]
    } else {
        vec![
            char_value(paths[0].as_deref().unwrap_or_default())?,
            char_value(names[0].as_deref().unwrap_or_default())?,
            char_value(extensions[0].as_deref().unwrap_or_default())?,
        ]
    };
    Ok(outputs
        .into_iter()
        .take(context.requested_outputs())
        .collect())
}

fn which_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("which", arguments, 1)?;
    expect_max_outputs("which", context, 1)?;
    let name = text_scalar("which", 1, &arguments[0])?;
    let caller = context.caller_source().map(str::to_owned);
    let found = (Path::new(&name).extension().is_none() || is_qualified_matlab_name(&name))
        .then(|| {
            context
                .resolve_matlab_source(caller.as_deref(), &name)
                .ok()
                .flatten()
        })
        .flatten()
        .map(|source| source.path.to_string_lossy().into_owned())
        .or_else(|| {
            current_directory_match(context, &name)
                .map(|metadata| metadata.path.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    char_value(&found).map(|value| vec![value])
}

fn is_qualified_matlab_name(name: &str) -> bool {
    name.contains('.')
        && !name.contains(['/', '\\', ':', '\0'])
        && name.split('.').all(|component| !component.is_empty())
}

fn current_directory_match(
    context: &mut BuiltinContext<'_>,
    name: &str,
) -> Option<FileSystemMetadata> {
    context
        .file_metadata(name)
        .ok()
        .filter(|metadata| metadata.is_file)
        .or_else(|| {
            (Path::new(name).extension().is_none())
                .then(|| context.file_metadata(&format!("{name}.m")).ok())
                .flatten()
                .filter(|metadata| metadata.is_file)
        })
}

#[allow(clippy::cast_precision_loss)] // MATLAB exposes directory byte counts as doubles.
fn directory_value(entries: Vec<FileSystemMetadata>) -> Result<Value, BuiltinError> {
    let fields = DIRECTORY_FIELDS
        .into_iter()
        .map(FieldName::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(aggregate_error)?;
    let shape = Shape::new([u64::try_from(entries.len()).unwrap_or(u64::MAX), 1])
        .map_err(|error| array_error(&error))?;
    if entries.is_empty() {
        return StructArray::empty(shape, fields)
            .map(Value::Struct)
            .map_err(aggregate_error);
    }
    let mut names = Vec::with_capacity(entries.len());
    let mut folders = Vec::with_capacity(entries.len());
    let mut dates = Vec::with_capacity(entries.len());
    let mut bytes = Vec::with_capacity(entries.len());
    let mut directories = Vec::with_capacity(entries.len());
    let mut datenums = Vec::with_capacity(entries.len());
    for entry in entries {
        names.push(char_value(&entry.name)?);
        folders.push(char_value(
            &entry
                .path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_string_lossy(),
        )?);
        let (date, datenum) = directory_time(entry.modified);
        dates.push(char_value(&date)?);
        bytes.push(Value::Double(entry.bytes as f64));
        directories.push(Value::Logical(entry.is_directory));
        datenums.push(Value::Double(datenum));
    }
    StructArray::from_columns(
        shape,
        fields,
        vec![names, folders, dates, bytes, directories, datenums],
    )
    .map(Value::Struct)
    .map_err(aggregate_error)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn directory_time(modified: Option<std::time::SystemTime>) -> (String, f64) {
    let Some(modified) = modified else {
        return (String::new(), 0.0);
    };
    let seconds = modified.duration_since(UNIX_EPOCH).map_or_else(
        |error| -error.duration().as_secs_f64(),
        |value| value.as_secs_f64(),
    );
    let whole_seconds = seconds.floor() as i64;
    let days = whole_seconds.div_euclid(86_400);
    let seconds_of_day = whole_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_unix_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = seconds_of_day % 3_600 / 60;
    let second = seconds_of_day % 60;
    let month_name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][usize::try_from(month.saturating_sub(1)).unwrap_or(0)];
    (
        format!("{day:02}-{month_name}-{year:04} {hour:02}:{minute:02}:{second:02}"),
        719_529.0 + seconds / 86_400.0,
    )
}

fn civil_from_unix_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
    let month = month_piece + if month_piece < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn mutation_outputs(
    context: &BuiltinContext<'_>,
    result: Result<(), BuiltinError>,
    name: &str,
    identifier: &str,
) -> BuiltinResult {
    match result {
        Ok(()) => status_outputs(context, true, "", "", name),
        Err(error) if context.requested_outputs() == 0 => Err(error),
        Err(error) => status_outputs(context, false, &error.message, identifier, name),
    }
}

fn status_outputs(
    context: &BuiltinContext<'_>,
    status: bool,
    message: &str,
    identifier: &str,
    name: &str,
) -> BuiltinResult {
    expect_max_outputs(name, context, 3)?;
    let outputs = [
        Value::Logical(status),
        char_value(message)?,
        char_value(identifier)?,
    ];
    Ok(outputs
        .into_iter()
        .take(context.requested_outputs())
        .collect())
}

#[derive(Clone)]
struct TextValues {
    shape: Option<Shape>,
    values: Vec<Option<String>>,
    string_input: bool,
}

impl TextValues {
    fn expanded_value(&self, offset: usize) -> Option<String> {
        let source = if self.values.len() == 1 { 0 } else { offset };
        self.values.get(source).cloned().flatten()
    }

    fn logical_result(&self, values: Vec<bool>) -> Result<Value, BuiltinError> {
        if !self.string_input {
            return Ok(Value::Logical(values.first().copied().unwrap_or(false)));
        }
        let shape = self.shape.clone().unwrap_or_else(scalar_shape);
        let values = values.into_iter().map(Logical::from).collect::<Vec<_>>();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::Logical)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

fn text_values(name: &str, position: usize, value: &Value) -> Result<TextValues, BuiltinError> {
    match value {
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            let code_units = array
                .as_slice()
                .iter()
                .map(|value| value.get())
                .collect::<Vec<_>>();
            let text = String::from_utf16(&code_units).map_err(|_| {
                BuiltinError::new(
                    BuiltinErrorCategory::Type,
                    format!("input {position} to `{name}` is not valid UTF-16 text"),
                )
            })?;
            Ok(TextValues {
                shape: None,
                values: vec![Some(text)],
                string_input: false,
            })
        }
        Value::String(strings) => {
            let values = (0..usize::try_from(strings.numel()).unwrap_or(usize::MAX))
                .map(|offset| {
                    strings.element(offset).and_then(|element| {
                        (!element.is_missing()).then(|| element.to_utf8_lossy())
                    })
                })
                .collect();
            let shape = Shape::new(strings.dimensions().iter().copied())
                .map_err(|error| array_error(&error))?;
            Ok(TextValues {
                shape: Some(shape),
                values,
                string_input: true,
            })
        }
        _ => Err(type_error(
            name,
            position,
            "char row or string array",
            value,
        )),
    }
}

fn text_scalar(name: &str, position: usize, value: &Value) -> Result<String, BuiltinError> {
    let values = text_values(name, position, value)?;
    if values.values.len() != 1 {
        return Err(type_error(name, position, "text scalar", value));
    }
    values.values[0]
        .clone()
        .ok_or_else(|| type_error(name, position, "non-missing text scalar", value))
}

fn common_text_shape(name: &str, inputs: &[TextValues]) -> Result<Option<Shape>, BuiltinError> {
    let mut shape: Option<Shape> = None;
    for input in inputs {
        if input.values.len() == 1 {
            continue;
        }
        if let Some(existing) = &shape
            && existing.dimensions()
                != input
                    .shape
                    .as_ref()
                    .map_or(&[] as &[u64], Shape::dimensions)
        {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("non-scalar string inputs to `{name}` must have the same size"),
            ));
        }
        shape.clone_from(&input.shape);
    }
    Ok(shape)
}

fn string_values(shape: Shape, values: Vec<Option<String>>) -> Result<Value, BuiltinError> {
    let values = values
        .into_iter()
        .map(|value| {
            value.map_or_else(StringElement::missing, |value| {
                StringElement::from_utf8(&value)
            })
        })
        .collect();
    StringArray::from_elements(shape, values)
        .map(StringValue::array)
        .map(Value::String)
        .map_err(|error| array_error(&error))
}

fn scalar_shape() -> Shape {
    Shape::new([1, 1]).expect("scalar shape is valid")
}

fn char_value(text: &str) -> Result<Value, BuiltinError> {
    let values = text
        .encode_utf16()
        .map(CharCodeUnit::new)
        .collect::<Vec<_>>();
    let shape = if values.is_empty() {
        Shape::new([0, 0])
    } else {
        Shape::new([1, u64::try_from(values.len()).unwrap_or(u64::MAX)])
    }
    .map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn split_fileparts(path: &str) -> (String, String, String) {
    if path.is_empty() {
        return (String::new(), String::new(), String::new());
    }
    if path.ends_with(['/', '\\']) {
        return (trim_trailing_separators(path), String::new(), String::new());
    }
    let separator = path.rfind(['/', '\\']);
    let (folder, leaf) = separator.map_or(("", path), |index| {
        let folder_end = if index == 2 && path.as_bytes().get(1) == Some(&b':') {
            index + 1
        } else {
            index
        };
        (&path[..folder_end], &path[index + 1..])
    });
    let dot = leaf.rfind('.');
    let (name, extension) = dot.map_or((leaf, ""), |index| (&leaf[..index], &leaf[index..]));
    (folder.to_owned(), name.to_owned(), extension.to_owned())
}

fn trim_trailing_separators(path: &str) -> String {
    let minimum = if path.starts_with("\\\\") {
        2
    } else if path.as_bytes().get(1) == Some(&b':') {
        3.min(path.len())
    } else {
        usize::from(path.starts_with(['/', '\\']))
    };
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.len() < minimum {
        path[..minimum].to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn join_platform_paths(components: &[String]) -> String {
    platform_join(components)
}

#[cfg(windows)]
fn platform_join(components: &[String]) -> String {
    let mut output = String::new();
    for component in components {
        let component = component.replace('/', "\\");
        if component.is_empty() || component == "." {
            continue;
        }
        if output.is_empty() {
            output = component;
            continue;
        }
        let root_length = windows_root_length(&output);
        while output.ends_with('\\') && output.len() > root_length {
            output.pop();
        }
        let component = component.trim_start_matches('\\');
        if !component.is_empty() {
            if !output.ends_with('\\') {
                output.push('\\');
            }
            output.push_str(component);
        }
    }
    output
}

#[cfg(windows)]
fn windows_root_length(path: &str) -> usize {
    if path.starts_with("\\\\") {
        2
    } else if path.as_bytes().get(1) == Some(&b':') && path.as_bytes().get(2) == Some(&b'\\') {
        3
    } else {
        0
    }
}

#[cfg(not(windows))]
fn platform_join(components: &[String]) -> String {
    let mut output = String::new();
    for component in components {
        if component.is_empty() || component == "." {
            continue;
        }
        if output.is_empty() {
            output = component.to_owned();
            continue;
        }
        while output.ends_with('/') && output.len() > usize::from(output.starts_with('/')) {
            output.pop();
        }
        let component = component.trim_start_matches('/');
        if !component.is_empty() {
            if !output.ends_with('/') {
                output.push('/');
            }
            output.push_str(component);
        }
    }
    output
}

fn has_wildcard(path: &str) -> bool {
    path.contains(['*', '?'])
}

fn aggregate_error(error: impl std::fmt::Display) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Other, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmat_runtime::{
        CancellationToken, LocalFileSystem, NullOutput, ObjectLifecycleRequest,
        ObjectLifecycleResult, ObjectLifecycleService,
    };
    use openmat_value::{ClassHandle, ObjectArray, ObjectHandle};
    use std::{
        fs, io,
        path::{Component, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TemporaryDirectory {
        path: PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Self {
            let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
            loop {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
                let path = parent.join(format!(
                    "openmat-filesystem-builtins-{}-{nonce}-{counter}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("cannot create unique temporary directory: {error}"),
                }
            }
        }

        fn path(&self, relative: &str) -> PathBuf {
            let relative = Path::new(relative);
            assert!(!relative.is_absolute());
            assert!(relative.components().all(|component| {
                matches!(component, Component::Normal(_) | Component::CurDir)
            }));
            let target = self.path.join(relative);
            assert!(target.starts_with(&self.path));
            target
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
            assert!(self.path.starts_with(parent));
            if self.path.exists() {
                fs::remove_dir_all(&self.path).unwrap();
            }
        }
    }

    fn invoke(
        builtin: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
        arguments: &[Value],
        requested_outputs: usize,
        files: &mut LocalFileSystem,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context = BuiltinContext::with_file_system_service(
            requested_outputs,
            &cancellation,
            &mut output,
            files,
        );
        builtin(arguments, &mut context)
    }

    struct FakeObjectLifecycle {
        result: ObjectLifecycleResult,
        requests: Vec<(&'static str, Value)>,
    }

    impl ObjectLifecycleService for FakeObjectLifecycle {
        fn request(
            &mut self,
            request: ObjectLifecycleRequest<'_>,
        ) -> Result<ObjectLifecycleResult, BuiltinError> {
            let (kind, value) = match request {
                ObjectLifecycleRequest::IsValid(value) => ("isvalid", value),
                ObjectLifecycleRequest::Delete(value) => ("delete", value),
            };
            self.requests.push((kind, value.clone()));
            Ok(self.result.clone())
        }
    }

    fn invoke_lifecycle(
        builtin: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
        arguments: &[Value],
        requested_outputs: usize,
        service: &mut dyn ObjectLifecycleService,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context = BuiltinContext::with_object_lifecycle_service(
            requested_outputs,
            &cancellation,
            &mut output,
            service,
        );
        builtin(arguments, &mut context)
    }

    fn object_array(shape: [u64; 2], handles: &[u64]) -> Value {
        Value::ObjectArray(
            ObjectArray::from_vec(
                ClassHandle::new(7),
                Shape::new(shape).unwrap(),
                handles.iter().copied().map(ObjectHandle::new).collect(),
            )
            .unwrap(),
        )
    }

    fn char_text(value: &Value) -> String {
        let Value::Array(ArrayData::Char(value)) = value else {
            panic!("expected char output");
        };
        String::from_utf16(
            &value
                .as_slice()
                .iter()
                .map(|value| value.get())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn registration_list_contains_the_complete_filesystem_tranche() {
        let mut registry = BuiltinRegistry::new();
        register_filesystem_extended(&mut registry).unwrap();
        for name in [
            "dir",
            "exist",
            "isfile",
            "isfolder",
            "mkdir",
            "rmdir",
            "delete",
            "isvalid",
            "copyfile",
            "movefile",
            "fullfile",
            "fileparts",
            "which",
            "path",
            "addpath",
            "rmpath",
            "genpath",
        ] {
            assert!(registry.handle_by_name(name).is_some(), "missing {name}");
        }
        assert_eq!(registry.len(), 17);
    }

    #[test]
    fn lifecycle_builtins_reject_unsupported_types_and_ambiguous_object_arguments() {
        let directory = TemporaryDirectory::new();
        let mut files = LocalFileSystem::new(&directory.path).unwrap();
        let error = invoke(isvalid_builtin, &[Value::Double(1.0)], 1, &mut files).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Type);

        let object = Value::Object(ObjectHandle::new(41));
        let error = invoke(delete_builtin, &[object.clone(), object], 0, &mut files).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn lifecycle_builtins_forward_typed_requests_without_faking_destruction() {
        let empty = object_array([0, 3], &[]);
        let mut service = FakeObjectLifecycle {
            result: ObjectLifecycleResult::Validity(Vec::new()),
            requests: Vec::new(),
        };
        let result = invoke_lifecycle(
            isvalid_builtin,
            std::slice::from_ref(&empty),
            1,
            &mut service,
        )
        .unwrap();
        assert_eq!(result[0].dimensions(), Some([0, 3].as_slice()));
        assert_eq!(service.requests, [("isvalid", empty)]);

        let handles = object_array([2, 2], &[51, 52, 53, 51]);
        service.result = ObjectLifecycleResult::HandleDeleteRequested { elements: 4 };
        service.requests.clear();
        invoke_lifecycle(
            delete_builtin,
            std::slice::from_ref(&handles),
            0,
            &mut service,
        )
        .unwrap();
        assert_eq!(service.requests, [("delete", handles)]);

        let value_object = Value::Object(ObjectHandle::new(61));
        service.result = ObjectLifecycleResult::ValueDeleteMethodRequested;
        service.requests.clear();
        invoke_lifecycle(
            delete_builtin,
            std::slice::from_ref(&value_object),
            0,
            &mut service,
        )
        .unwrap();
        assert_eq!(service.requests, [("delete", value_object)]);
    }

    #[test]
    fn fullfile_and_fileparts_match_measured_windows_path_cases() {
        let directory = TemporaryDirectory::new();
        let mut files = LocalFileSystem::new(&directory.path).unwrap();
        let joined = invoke(
            fullfile_builtin,
            &[
                char_value("a/").unwrap(),
                char_value("/b/").unwrap(),
                char_value("c").unwrap(),
            ],
            1,
            &mut files,
        )
        .unwrap();
        assert_eq!(
            char_text(&joined[0]),
            platform_join(&["a/".into(), "/b/".into(), "c".into()])
        );

        let parts = invoke(
            fileparts_builtin,
            &[char_value(r"C:\alpha\beta.tar.gz").unwrap()],
            3,
            &mut files,
        )
        .unwrap();
        assert_eq!(char_text(&parts[0]), r"C:\alpha");
        assert_eq!(char_text(&parts[1]), "beta.tar");
        assert_eq!(char_text(&parts[2]), ".gz");

        let dotfile = invoke(
            fileparts_builtin,
            &[char_value(".bashrc").unwrap()],
            3,
            &mut files,
        )
        .unwrap();
        assert_eq!(char_text(&dotfile[0]), "");
        assert_eq!(char_text(&dotfile[1]), "");
        assert_eq!(char_text(&dotfile[2]), ".bashrc");
    }

    #[test]
    fn string_array_paths_preserve_shape_and_missing_predicates_are_false() {
        let directory = TemporaryDirectory::new();
        fs::write(directory.path("alpha.txt"), b"alpha").unwrap();
        let mut files = LocalFileSystem::new(&directory.path).unwrap();
        let strings = Value::String(StringValue::array(
            StringArray::from_elements(
                Shape::new([1, 3]).unwrap(),
                vec![
                    StringElement::from_utf8("alpha.txt"),
                    StringElement::from_utf8("missing.txt"),
                    StringElement::missing(),
                ],
            )
            .unwrap(),
        ));
        let result = invoke(isfile_builtin, &[strings], 1, &mut files).unwrap();
        let Value::Array(ArrayData::Logical(values)) = &result[0] else {
            panic!("expected logical array");
        };
        assert_eq!(values.shape().dimensions(), [1, 3]);
        assert_eq!(
            values
                .as_slice()
                .iter()
                .map(|value| value.get())
                .collect::<Vec<_>>(),
            [true, false, false]
        );

        let empty_strings = Value::String(StringValue::array(
            StringArray::from_elements(Shape::new([0, 2]).unwrap(), Vec::new()).unwrap(),
        ));
        let joined = invoke(
            fullfile_builtin,
            &[char_value("root").unwrap(), empty_strings],
            1,
            &mut files,
        )
        .unwrap();
        assert_eq!(joined[0].dimensions(), Some([0, 2].as_slice()));
    }

    #[test]
    fn file_mutations_stay_inside_one_validated_unique_temporary_directory() {
        let directory = TemporaryDirectory::new();
        let mut files = LocalFileSystem::new(&directory.path).unwrap();

        let made = invoke(
            mkdir_builtin,
            &[char_value("source/nested").unwrap()],
            3,
            &mut files,
        )
        .unwrap();
        assert_eq!(made[0], Value::Logical(true));
        fs::write(directory.path("source/alpha.txt"), b"alpha").unwrap();
        fs::write(directory.path("source/nested/beta.txt"), b"beta").unwrap();

        let copied = invoke(
            copyfile_builtin,
            &[char_value("source").unwrap(), char_value("copied").unwrap()],
            3,
            &mut files,
        )
        .unwrap();
        assert_eq!(copied[0], Value::Logical(true));
        assert_eq!(
            fs::read(directory.path("copied/alpha.txt")).unwrap(),
            b"alpha"
        );

        fs::write(directory.path("destination.txt"), b"old").unwrap();
        let moved = invoke(
            movefile_builtin,
            &[
                char_value("copied/alpha.txt").unwrap(),
                char_value("destination.txt").unwrap(),
            ],
            3,
            &mut files,
        )
        .unwrap();
        assert_eq!(moved[0], Value::Logical(true));
        assert_eq!(
            fs::read(directory.path("destination.txt")).unwrap(),
            b"alpha"
        );

        invoke(
            delete_builtin,
            &[char_value("copied/nested/*.txt").unwrap()],
            0,
            &mut files,
        )
        .unwrap();
        assert!(!directory.path("copied/nested/beta.txt").exists());
        let removed = invoke(
            rmdir_builtin,
            &[char_value("copied").unwrap(), char_value("s").unwrap()],
            3,
            &mut files,
        )
        .unwrap();
        assert_eq!(removed[0], Value::Logical(true));
        assert!(!directory.path("copied").exists());
    }

    #[test]
    fn dir_exist_and_which_distinguish_files_directories_and_current_source() {
        let directory = TemporaryDirectory::new();
        fs::write(directory.path("sample.m"), b"openmat_result = 1;").unwrap();
        fs::create_dir(directory.path("folder")).unwrap();
        let mut files = LocalFileSystem::new(&directory.path).unwrap();

        let listing = invoke(dir_builtin, &[char_value("*.m").unwrap()], 1, &mut files).unwrap();
        let Value::Struct(listing) = &listing[0] else {
            panic!("expected dir struct");
        };
        assert_eq!(
            listing
                .field_names()
                .iter()
                .map(FieldName::as_str)
                .collect::<Vec<_>>(),
            DIRECTORY_FIELDS
        );
        assert_eq!(
            char_text(listing.value_at_name("name", 0).unwrap()),
            "sample.m"
        );

        let file_code = invoke(
            exist_builtin,
            &[char_value("sample.m").unwrap()],
            1,
            &mut files,
        )
        .unwrap();
        let directory_code = invoke(
            exist_builtin,
            &[char_value("folder").unwrap(), char_value("file").unwrap()],
            1,
            &mut files,
        )
        .unwrap();
        assert_eq!(file_code, [Value::Double(2.0)]);
        assert_eq!(directory_code, [Value::Double(7.0)]);

        let found = invoke(which_builtin, &[Value::from("sample")], 1, &mut files).unwrap();
        assert_eq!(
            fs::canonicalize(PathBuf::from(char_text(&found[0]))).unwrap(),
            fs::canonicalize(directory.path("sample.m")).unwrap()
        );
    }
}

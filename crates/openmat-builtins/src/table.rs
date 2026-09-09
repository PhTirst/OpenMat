use std::{cmp::Ordering, collections::BTreeMap};

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32, Complex64 as ArrayComplex64, ComplexInteger, DenseArray,
    IntegerArrayData, IntegerComponent, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{
    CellArray, Complex64 as ValueComplex64, FieldName, GraphicsHandleArray, ObjectArray,
    StringArray, StringElement, StringValue, StructArray, TableArray, TableRowName,
    TableVariableName, Value,
};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count,
    expect_argument_count_range, expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

pub(super) fn table_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("table", context, 1)?;
    context.check_cancelled()?;
    let constructor = table_constructor_arguments(arguments)?;
    let names = constructor
        .variable_names
        .unwrap_or_else(|| default_names(constructor.variables.len()));
    let row_count = constructor
        .variables
        .first()
        .and_then(Value::dimensions)
        .and_then(|dimensions| dimensions.first().copied())
        .unwrap_or(0);
    let table = TableArray::from_parts_with_row_names(
        row_count,
        names,
        constructor.variables.to_vec(),
        constructor.row_names,
    )
    .map_err(|error| table_model_error("table", error))?;
    Ok(vec![Value::Table(table)])
}

pub(super) fn array2table_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("array2table", context, 1)?;
    let (input, requested_names) = one_input_constructor("array2table", arguments)?;
    context.check_cancelled()?;
    let (row_count, variables) = split_matrix_columns("array2table", input, context)?;
    let names = requested_names.unwrap_or_else(|| default_names(variables.len()));
    let table = TableArray::from_parts(row_count, names, variables)
        .map_err(|error| table_model_error("array2table", error))?;
    Ok(vec![Value::Table(table)])
}

pub(super) fn cell2table_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("cell2table", context, 1)?;
    let (input, requested_names) = one_input_constructor("cell2table", arguments)?;
    let Value::Cell(cell) = input else {
        return Err(type_error(
            "cell2table",
            1,
            "two-dimensional cell array",
            input,
        ));
    };
    let dimensions = cell.shape().dimensions();
    if dimensions.len() != 2 {
        return Err(domain_error(
            "cell2table",
            "input 1 must be a two-dimensional cell array",
        ));
    }
    let row_count = dimensions[0];
    let rows = host_length(row_count)?;
    let columns = host_length(dimensions[1])?;
    let names = requested_names.unwrap_or_else(|| default_names(columns));
    let mut variables = Vec::with_capacity(columns);
    for column in 0..columns {
        cancellation_checkpoint(context, column)?;
        let start = column
            .checked_mul(rows)
            .ok_or_else(|| mapping_error("cell2table"))?;
        let end = start
            .checked_add(rows)
            .ok_or_else(|| mapping_error("cell2table"))?;
        let values = cell
            .values()
            .get(start..end)
            .ok_or_else(|| mapping_error("cell2table"))?;
        variables.push(merge_cell_column(values, row_count, context)?);
    }
    context.check_cancelled()?;
    let table = TableArray::from_parts(row_count, names, variables)
        .map_err(|error| table_model_error("cell2table", error))?;
    Ok(vec![Value::Table(table)])
}

pub(super) fn struct2table_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("struct2table", context, 1)?;
    let as_array = parse_boolean_option("struct2table", arguments, "AsArray", false)?;
    context.check_cancelled()?;
    let Value::Struct(structure) = &arguments[0] else {
        return Err(type_error(
            "struct2table",
            1,
            "scalar struct",
            &arguments[0],
        ));
    };
    let names = structure
        .field_names()
        .iter()
        .map(|name| TableVariableName::new(name.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| table_model_error("struct2table", error))?;
    let rows = if as_array {
        structure.numel()
    } else if structure.numel() == 1 {
        if structure.field_count() == 0 {
            1
        } else {
            structure
                .value_at(0, 0)
                .and_then(Value::dimensions)
                .and_then(|dimensions| dimensions.first().copied())
                .unwrap_or(0)
        }
    } else {
        structure.numel()
    };
    let mut variables = Vec::with_capacity(structure.field_count());
    for field in 0..structure.field_count() {
        cancellation_checkpoint(context, field)?;
        let values = structure
            .field_values(field)
            .ok_or_else(|| mapping_error("struct2table"))?;
        let variable = if as_array {
            cell_column(values, rows)?
        } else if structure.numel() == 1 {
            values
                .first()
                .cloned()
                .ok_or_else(|| mapping_error("struct2table"))?
        } else {
            merge_struct_field(values, rows, context)?
        };
        variables.push(variable);
    }
    let table = TableArray::from_parts(rows, names, variables)
        .map_err(|error| table_model_error("struct2table", error))?;
    Ok(vec![Value::Table(table)])
}

pub(super) fn table2array_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("table2array", arguments, 1)?;
    expect_max_outputs("table2array", context, 1)?;
    let table = table_argument("table2array", &arguments[0])?;
    context.check_cancelled()?;
    Ok(vec![concatenate_table_variables(table, context)?])
}

pub(super) fn table2cell_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("table2cell", arguments, 1)?;
    expect_max_outputs("table2cell", context, 1)?;
    let table = table_argument("table2cell", &arguments[0])?;
    let rows = host_length(table.row_count())?;
    let columns = table.variable_count();
    let length = rows
        .checked_mul(columns)
        .ok_or_else(|| mapping_error("table2cell"))?;
    let mut values = Vec::with_capacity(length);
    for variable in 0..columns {
        let source = table
            .variable(variable)
            .ok_or_else(|| mapping_error("table2cell"))?;
        for row in 0..rows {
            cancellation_checkpoint(context, values.len())?;
            let row = u64::try_from(row).map_err(|_| mapping_error("table2cell"))?;
            let value = slice_rows("table2cell", source, row, 1, context)?;
            values.push(collapse_table_cell_value(value));
        }
    }
    context.check_cancelled()?;
    let width = u64::try_from(columns).map_err(|_| mapping_error("table2cell"))?;
    let shape = Shape::new([table.row_count(), width]).map_err(|error| array_error(&error))?;
    let cell = CellArray::from_values(shape, values)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))?;
    Ok(vec![Value::Cell(cell)])
}

pub(super) fn table2struct_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("table2struct", context, 1)?;
    let to_scalar = parse_boolean_option("table2struct", arguments, "ToScalar", false)?;
    let table = table_argument("table2struct", &arguments[0])?;
    context.check_cancelled()?;
    let fields = table
        .variable_names()
        .iter()
        .map(|name| {
            FieldName::new(name.as_str()).map_err(|_| {
                domain_error(
                    "table2struct",
                    format!(
                        "table variable name {:?} cannot be represented by the current struct field model",
                        name.as_str()
                    ),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut columns = Vec::with_capacity(table.variable_count());
    for index in 0..table.variable_count() {
        cancellation_checkpoint(context, index)?;
        let variable = table
            .variable(index)
            .ok_or_else(|| mapping_error("table2struct"))?;
        if to_scalar {
            columns.push(vec![variable.clone()]);
            continue;
        }
        let mut values = Vec::with_capacity(host_length(table.row_count())?);
        for row in 0..table.row_count() {
            let value = slice_rows("table2struct", variable, row, 1, context)?;
            values.push(collapse_table_cell_value(value));
        }
        columns.push(values);
    }
    let shape = if to_scalar {
        Shape::new([1, 1])
    } else {
        Shape::new([table.row_count(), 1])
    }
    .map_err(|error| array_error(&error))?;
    let structure = StructArray::from_columns(shape, fields, columns)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))?;
    Ok(vec![Value::Struct(structure)])
}

pub(super) fn height_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("height", arguments, 1)?;
    expect_max_outputs("height", context, 1)?;
    let table = table_argument("height", &arguments[0])?;
    Ok(vec![dimension_value(table.row_count())])
}

pub(super) fn width_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("width", arguments, 1)?;
    expect_max_outputs("width", context, 1)?;
    let table = table_argument("width", &arguments[0])?;
    let width = u64::try_from(table.variable_count()).map_err(|_| mapping_error("width"))?;
    Ok(vec![dimension_value(width)])
}

pub(super) fn istable_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("istable", arguments, 1)?;
    expect_max_outputs("istable", context, 1)?;
    Ok(vec![Value::Logical(matches!(
        arguments[0],
        Value::Table(_)
    ))])
}

pub(super) fn head_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    select_end_rows("head", arguments, context, false)
}

pub(super) fn tail_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    select_end_rows("tail", arguments, context, true)
}

pub(super) fn sortrows_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("sortrows", arguments, 2, 3)?;
    expect_max_outputs("sortrows", context, 1)?;
    let table = table_argument("sortrows", &arguments[0])?;
    let variable = table_variable_selector("sortrows", table, &arguments[1])?;
    let descending = arguments
        .get(2)
        .map(|value| required_text_scalar("sortrows", 3, value))
        .transpose()?
        .is_some_and(|direction| direction.eq_ignore_ascii_case("descend"));
    if let Some(direction) = arguments.get(2) {
        let direction = required_text_scalar("sortrows", 3, direction)?;
        if !direction.eq_ignore_ascii_case("ascend") && !direction.eq_ignore_ascii_case("descend") {
            return Err(domain_error(
                "sortrows",
                "direction must be 'ascend' or 'descend'",
            ));
        }
    }
    let key = table
        .variable(variable)
        .ok_or_else(|| mapping_error("sortrows"))?;
    let mut rows = (0..host_length(table.row_count())?)
        .map(|row| Ok((row, table_key_at("sortrows", key, row)?)))
        .collect::<Result<Vec<_>, BuiltinError>>()?;
    rows.sort_by(|(_, left), (_, right)| {
        let ordering = left.cmp(right);
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    let rows = rows.into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let sorted = gather_table_rows("sortrows", table, &rows, context)?;
    Ok(vec![Value::Table(sorted)])
}

pub(super) fn ismissing_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("ismissing", arguments, 1)?;
    expect_max_outputs("ismissing", context, 1)?;
    let table = table_argument("ismissing", &arguments[0])?;
    let rows = host_length(table.row_count())?;
    let mut values = Vec::with_capacity(rows.saturating_mul(table.variable_count()));
    for variable in 0..table.variable_count() {
        cancellation_checkpoint(context, variable)?;
        let mask = table_variable_missing_mask(
            "ismissing",
            table
                .variable(variable)
                .ok_or_else(|| mapping_error("ismissing"))?,
        )?;
        values.extend(mask.into_iter().map(Logical::from));
    }
    let shape = Shape::new([table.row_count(), table.variable_count() as u64])
        .map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map(|value| vec![value])
        .map_err(|error| array_error(&error))
}

pub(super) fn rmmissing_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("rmmissing", arguments, 1)?;
    expect_max_outputs("rmmissing", context, 1)?;
    let table = table_argument("rmmissing", &arguments[0])?;
    let rows = host_length(table.row_count())?;
    let mut missing = vec![false; rows];
    for variable in 0..table.variable_count() {
        cancellation_checkpoint(context, variable)?;
        let mask = table_variable_missing_mask(
            "rmmissing",
            table
                .variable(variable)
                .ok_or_else(|| mapping_error("rmmissing"))?,
        )?;
        for (row, value) in missing.iter_mut().zip(mask) {
            *row |= value;
        }
    }
    let kept = missing
        .iter()
        .enumerate()
        .filter_map(|(row, missing)| (!missing).then_some(row))
        .collect::<Vec<_>>();
    Ok(vec![Value::Table(gather_table_rows(
        "rmmissing",
        table,
        &kept,
        context,
    )?)])
}

pub(super) fn fillmissing_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("fillmissing", arguments, 5)?;
    expect_max_outputs("fillmissing", context, 1)?;
    let table = table_argument("fillmissing", &arguments[0])?;
    let method = required_text_scalar("fillmissing", 2, &arguments[1])?;
    if !method.eq_ignore_ascii_case("constant") {
        return Err(domain_error(
            "fillmissing",
            "the current table implementation supports only the 'constant' method",
        ));
    }
    let option = required_text_scalar("fillmissing", 4, &arguments[3])?;
    if !option.eq_ignore_ascii_case("DataVariables") {
        return Err(domain_error(
            "fillmissing",
            "the current table implementation requires the 'DataVariables' option",
        ));
    }
    let selected = table_variable_selectors("fillmissing", table, &arguments[4])?;
    let mut filled = table.clone();
    for variable in selected {
        cancellation_checkpoint(context, variable)?;
        let value = table
            .variable(variable)
            .ok_or_else(|| mapping_error("fillmissing"))?;
        let replacement = fill_table_variable_missing("fillmissing", value, &arguments[2])?;
        filled
            .replace_variable(variable, replacement)
            .map_err(|error| table_model_error("fillmissing", error))?;
    }
    Ok(vec![Value::Table(filled)])
}

pub(super) fn innerjoin_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (left, right, key, _) = join_arguments("innerjoin", arguments, false)?;
    expect_max_outputs("innerjoin", context, 1)?;
    let left_key = left
        .variable_by_name(&key)
        .ok_or_else(|| domain_error("innerjoin", format!("left key `{key}` does not exist")))?;
    let right_key = right
        .variable_by_name(&key)
        .ok_or_else(|| domain_error("innerjoin", format!("right key `{key}` does not exist")))?;
    let right_rows = keyed_rows("innerjoin", right_key, right.row_count())?;
    let mut pairs = Vec::new();
    for left_row in 0..host_length(left.row_count())? {
        cancellation_checkpoint(context, left_row)?;
        let key = table_key_at("innerjoin", left_key, left_row)?;
        if let Some(matches) = right_rows.get(&key) {
            pairs.extend(
                matches
                    .iter()
                    .map(|right_row| (key.clone(), (left_row, *right_row))),
            );
        }
    }
    pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
    let pairs = pairs.into_iter().map(|(_, pair)| pair).collect::<Vec<_>>();
    let table = joined_table("innerjoin", left, right, &key, &pairs, context)?;
    Ok(vec![Value::Table(table)])
}

pub(super) fn outerjoin_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (left, right, key, merge_keys) = join_arguments("outerjoin", arguments, true)?;
    expect_max_outputs("outerjoin", context, 1)?;
    if !merge_keys {
        return Err(domain_error(
            "outerjoin",
            "the current implementation requires 'MergeKeys', true",
        ));
    }
    let left_key = left
        .variable_by_name(&key)
        .ok_or_else(|| domain_error("outerjoin", format!("left key `{key}` does not exist")))?;
    let right_key = right
        .variable_by_name(&key)
        .ok_or_else(|| domain_error("outerjoin", format!("right key `{key}` does not exist")))?;
    let right_rows = keyed_rows("outerjoin", right_key, right.row_count())?;
    let mut pairs = Vec::new();
    let mut matched_right = vec![false; host_length(right.row_count())?];
    for left_row in 0..host_length(left.row_count())? {
        cancellation_checkpoint(context, left_row)?;
        let key = table_key_at("outerjoin", left_key, left_row)?;
        if let Some(matches) = right_rows.get(&key) {
            for right_row in matches {
                pairs.push((key.clone(), (Some(left_row), Some(*right_row))));
                matched_right[*right_row] = true;
            }
        } else {
            pairs.push((key, (Some(left_row), None)));
        }
    }
    for (row, matched) in matched_right.iter().enumerate() {
        if !matched {
            pairs.push((
                table_key_at("outerjoin", right_key, row)?,
                (None, Some(row)),
            ));
        }
    }
    pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
    let pairs = pairs.into_iter().map(|(_, pair)| pair).collect::<Vec<_>>();
    let table = outer_joined_table("outerjoin", left, right, &key, &pairs, context)?;
    Ok(vec![Value::Table(table)])
}

pub(super) fn groupcounts_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("groupcounts", arguments, 2)?;
    expect_max_outputs("groupcounts", context, 1)?;
    let table = table_argument("groupcounts", &arguments[0])?;
    let group = required_text_scalar("groupcounts", 2, &arguments[1])?;
    let group_index = table
        .variable_index(&group)
        .ok_or_else(|| domain_error("groupcounts", format!("variable `{group}` does not exist")))?;
    let (first_rows, counts) = table_groups("groupcounts", table, group_index, context)?;
    let grouped = gather_value_rows(
        "groupcounts",
        table
            .variable(group_index)
            .ok_or_else(|| mapping_error("groupcounts"))?,
        &first_rows,
        context,
    )?;
    let count_values = counts
        .iter()
        .copied()
        .map(usize_to_double)
        .collect::<Vec<_>>();
    let percent = counts
        .iter()
        .map(|count| {
            if table.row_count() == 0 {
                f64::NAN
            } else {
                usize_to_double(*count) * 100.0 / u64_to_double(table.row_count())
            }
        })
        .collect::<Vec<_>>();
    let names = [group.as_str(), "GroupCount", "Percent"]
        .into_iter()
        .map(|name| {
            TableVariableName::new(name).map_err(|error| table_model_error("groupcounts", error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let variables = vec![
        grouped,
        double_column(count_values)?,
        double_column(percent)?,
    ];
    TableArray::from_parts(counts.len() as u64, names, variables)
        .map(Value::Table)
        .map(|table| vec![table])
        .map_err(|error| table_model_error("groupcounts", error))
}

pub(super) fn groupsummary_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("groupsummary", arguments, 4)?;
    expect_max_outputs("groupsummary", context, 1)?;
    let table = table_argument("groupsummary", &arguments[0])?;
    let group = required_text_scalar("groupsummary", 2, &arguments[1])?;
    let method = required_text_scalar("groupsummary", 3, &arguments[2])?;
    let data = required_text_scalar("groupsummary", 4, &arguments[3])?;
    if !method.eq_ignore_ascii_case("mean") {
        return Err(domain_error(
            "groupsummary",
            "the current implementation supports only the 'mean' method",
        ));
    }
    let group_index = table.variable_index(&group).ok_or_else(|| {
        domain_error("groupsummary", format!("variable `{group}` does not exist"))
    })?;
    let data_index = table
        .variable_index(&data)
        .ok_or_else(|| domain_error("groupsummary", format!("variable `{data}` does not exist")))?;
    let (first_rows, counts, members) =
        table_group_members("groupsummary", table, group_index, context)?;
    let grouped = gather_value_rows(
        "groupsummary",
        table
            .variable(group_index)
            .ok_or_else(|| mapping_error("groupsummary"))?,
        &first_rows,
        context,
    )?;
    let data_value = table
        .variable(data_index)
        .ok_or_else(|| mapping_error("groupsummary"))?;
    let means = members
        .iter()
        .map(|rows| mean_table_rows("groupsummary", data_value, rows))
        .collect::<Result<Vec<_>, _>>()?;
    let names = vec![
        TableVariableName::new(group).map_err(|error| table_model_error("groupsummary", error))?,
        TableVariableName::new("GroupCount")
            .map_err(|error| table_model_error("groupsummary", error))?,
        TableVariableName::new(format!("mean_{data}"))
            .map_err(|error| table_model_error("groupsummary", error))?,
    ];
    let variables = vec![
        grouped,
        double_column(counts.iter().copied().map(usize_to_double).collect())?,
        double_column(means)?,
    ];
    TableArray::from_parts(counts.len() as u64, names, variables)
        .map(Value::Table)
        .map(|table| vec![table])
        .map_err(|error| table_model_error("groupsummary", error))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FloatKey(u64);

impl FloatKey {
    fn new(value: f64) -> Self {
        Self(if value == 0.0 {
            0
        } else if value.is_nan() {
            f64::NAN.to_bits()
        } else {
            value.to_bits()
        })
    }

    fn value(self) -> f64 {
        f64::from_bits(self.0)
    }
}

impl PartialOrd for FloatKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FloatKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value().total_cmp(&other.value())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TableKey {
    Logical(bool),
    Signed(i128),
    Unsigned(u128),
    Number(FloatKey),
    Text(String),
    Missing,
}

fn table_variable_selector(
    name: &str,
    table: &TableArray,
    value: &Value,
) -> Result<usize, BuiltinError> {
    if let Some(text) = text_scalar(name, 2, value)? {
        return table
            .variable_index(&text)
            .ok_or_else(|| domain_error(name, format!("variable `{text}` does not exist")));
    }
    let index = nonnegative_integer(name, 2, value)?;
    if index == 0 || index > table.variable_count() as u64 {
        return Err(domain_error(name, "table variable index is out of range"));
    }
    host_length(index - 1)
}

fn table_variable_selectors(
    name: &str,
    table: &TableArray,
    value: &Value,
) -> Result<Vec<usize>, BuiltinError> {
    if text_scalar(name, 5, value)?.is_some() {
        return table_variable_selector(name, table, value).map(|index| vec![index]);
    }
    text_vector(name, 5, value, "table variable name or vector of names")?
        .into_iter()
        .map(|text| {
            table
                .variable_index(&text)
                .ok_or_else(|| domain_error(name, format!("variable `{text}` does not exist")))
        })
        .collect()
}

fn table_key_at(name: &str, value: &Value, row: usize) -> Result<TableKey, BuiltinError> {
    match value {
        Value::Logical(item) if row == 0 => Ok(TableKey::Logical(*item)),
        Value::Double(item) if row == 0 => Ok(TableKey::Number(FloatKey::new(*item))),
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .get(row)
            .copied()
            .map(|value| TableKey::Number(FloatKey::new(f64::from(value))))
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::Logical(array)) => array
            .as_slice()
            .get(row)
            .map(|value| TableKey::Logical(value.get()))
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .get(row)
            .copied()
            .map(|value| TableKey::Number(FloatKey::new(value)))
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::Integer(integer)) => {
            let element = integer.element(row).ok_or_else(|| mapping_error(name))?;
            if element.is_complex() {
                return Err(domain_error(
                    name,
                    "complex values cannot be used as table keys",
                ));
            }
            Ok(match element.real_component() {
                IntegerComponent::Signed(value) => TableKey::Signed(value),
                IntegerComponent::Unsigned(value) => TableKey::Unsigned(value),
            })
        }
        Value::String(strings) => {
            let element = strings.element(row).ok_or_else(|| mapping_error(name))?;
            if element.is_missing() {
                Ok(TableKey::Missing)
            } else {
                string_element_text(name, 1, element).map(TableKey::Text)
            }
        }
        Value::Cell(cell) => {
            if host_length(cell.shape().dimensions()[0])? != cell.values().len() {
                return Err(domain_error(
                    name,
                    "multi-column cell variables cannot be used as table keys",
                ));
            }
            let value = cell.values().get(row).ok_or_else(|| mapping_error(name))?;
            if let Some(text) = text_scalar(name, 1, value)? {
                Ok(TableKey::Text(text))
            } else {
                table_key_at(name, value, 0)
            }
        }
        _ => Err(domain_error(
            name,
            "the selected table key must be a real numeric, logical, string, or cellstr column",
        )),
    }
}

fn gather_table_rows(
    name: &str,
    table: &TableArray,
    rows: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<TableArray, BuiltinError> {
    let mut variables = Vec::with_capacity(table.variable_count());
    for index in 0..table.variable_count() {
        cancellation_checkpoint(context, index)?;
        variables.push(gather_value_rows(
            name,
            table.variable(index).ok_or_else(|| mapping_error(name))?,
            rows,
            context,
        )?);
    }
    let row_names = table.row_names().map(|names| {
        rows.iter()
            .map(|row| names.get(*row).cloned().ok_or_else(|| mapping_error(name)))
            .collect::<Result<Vec<_>, _>>()
    });
    TableArray::from_parts_with_row_names(
        rows.len() as u64,
        table.variable_names().to_vec(),
        variables,
        row_names.transpose()?,
    )
    .map_err(|error| table_model_error(name, error))
}

fn gather_value_rows(
    name: &str,
    value: &Value,
    rows: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! gather_array {
        ($array:expr, $wrap:expr) => {{
            let array = gather_dense_rows(name, $array, rows, context)?;
            Ok($wrap(array))
        }};
    }
    match value {
        Value::Logical(item) => {
            let source = scalar_dense(Logical::from(*item))?;
            gather_array!(&source, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Double(item) => {
            let source = scalar_dense(*item)?;
            gather_array!(&source, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Complex(item) => {
            let source = scalar_dense(ArrayComplex64::new(item.real, item.imaginary))?;
            gather_array!(&source, |array| Value::Array(ArrayData::ComplexF64(array)))
        }
        Value::Array(ArrayData::F32(array)) => {
            gather_array!(array, |array| { Value::Array(ArrayData::F32(array)) })
        }
        Value::Array(ArrayData::ComplexF32(array)) => gather_array!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        }),
        Value::Array(ArrayData::Logical(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Array(ArrayData::F64(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::ComplexF64(array)))
        }
        Value::Array(ArrayData::Char(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::Char(array)))
        }
        Value::Array(ArrayData::Integer(integer)) => {
            gather_integer_rows(name, integer, rows, context)
        }
        Value::String(strings) => {
            let values = (0..host_length(strings.numel())?)
                .map(|offset| {
                    strings
                        .element(offset)
                        .cloned()
                        .ok_or_else(|| mapping_error(name))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (shape, values) =
                gather_flat_rows(name, strings.dimensions(), &values, rows, context)?;
            StringArray::from_elements(shape, values)
                .map(StringValue::Array)
                .map(Value::String)
                .map_err(|error| array_error(&error))
        }
        Value::Cell(cell) => {
            let (shape, values) = gather_flat_rows(
                name,
                cell.shape().dimensions(),
                cell.values(),
                rows,
                context,
            )?;
            CellArray::from_values(shape, values)
                .map(Value::Cell)
                .map_err(|error| table_model_error(name, error))
        }
        Value::Table(table) => gather_table_rows(name, table, rows, context).map(Value::Table),
        _ => Err(domain_error(
            name,
            "this table variable class does not yet support arbitrary row gathering",
        )),
    }
}

fn gather_dense_rows<T: Clone>(
    name: &str,
    array: &DenseArray<T>,
    rows: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let (shape, values) = gather_flat_rows(
        name,
        array.shape().dimensions(),
        array.as_slice(),
        rows,
        context,
    )?;
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn gather_flat_rows<T: Clone>(
    name: &str,
    dimensions: &[u64],
    values: &[T],
    rows: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<(Shape, Vec<T>), BuiltinError> {
    let source_rows = dimensions
        .first()
        .copied()
        .ok_or_else(|| mapping_error(name))?;
    let host_rows = host_length(source_rows)?;
    if rows.iter().any(|row| *row >= host_rows) {
        return Err(mapping_error(name));
    }
    let mut dimensions = dimensions.to_vec();
    dimensions[0] = rows.len() as u64;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    if source_rows == 0 || rows.is_empty() {
        return Ok((shape, Vec::new()));
    }
    let blocks = values.len() / host_rows;
    let mut selected = Vec::with_capacity(host_length(shape.numel())?);
    for block in 0..blocks {
        cancellation_checkpoint(context, block)?;
        let offset = block
            .checked_mul(host_rows)
            .ok_or_else(|| mapping_error(name))?;
        for row in rows {
            selected.push(
                values
                    .get(offset + row)
                    .cloned()
                    .ok_or_else(|| mapping_error(name))?,
            );
        }
    }
    Ok((shape, selected))
}

fn gather_integer_rows(
    name: &str,
    integer: &IntegerArrayData,
    rows: &[usize],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! gather {
        ($array:expr, $variant:ident) => {{
            let array = gather_dense_rows(name, $array, rows, context)?;
            Ok(Value::Array(ArrayData::Integer(
                IntegerArrayData::$variant(array),
            )))
        }};
    }
    match integer {
        IntegerArrayData::I8(array) => gather!(array, I8),
        IntegerArrayData::ComplexI8(array) => gather!(array, ComplexI8),
        IntegerArrayData::U8(array) => gather!(array, U8),
        IntegerArrayData::ComplexU8(array) => gather!(array, ComplexU8),
        IntegerArrayData::I16(array) => gather!(array, I16),
        IntegerArrayData::ComplexI16(array) => gather!(array, ComplexI16),
        IntegerArrayData::U16(array) => gather!(array, U16),
        IntegerArrayData::ComplexU16(array) => gather!(array, ComplexU16),
        IntegerArrayData::I32(array) => gather!(array, I32),
        IntegerArrayData::ComplexI32(array) => gather!(array, ComplexI32),
        IntegerArrayData::U32(array) => gather!(array, U32),
        IntegerArrayData::ComplexU32(array) => gather!(array, ComplexU32),
        IntegerArrayData::I64(array) => gather!(array, I64),
        IntegerArrayData::ComplexI64(array) => gather!(array, ComplexI64),
        IntegerArrayData::U64(array) => gather!(array, U64),
        IntegerArrayData::ComplexU64(array) => gather!(array, ComplexU64),
    }
}

fn table_variable_missing_mask(name: &str, value: &Value) -> Result<Vec<bool>, BuiltinError> {
    fn rows_any<T>(
        dimensions: &[u64],
        values: &[T],
        predicate: impl Fn(&T) -> bool,
    ) -> Result<Vec<bool>, BuiltinError> {
        let rows = host_length(dimensions.first().copied().unwrap_or(0))?;
        let mut result = vec![false; rows];
        if rows == 0 {
            return Ok(result);
        }
        for (offset, value) in values.iter().enumerate() {
            result[offset % rows] |= predicate(value);
        }
        Ok(result)
    }

    match value {
        Value::Double(value) => Ok(vec![value.is_nan()]),
        Value::Complex(value) => Ok(vec![value.real.is_nan() || value.imaginary.is_nan()]),
        Value::Array(ArrayData::F32(array)) => {
            rows_any(array.shape().dimensions(), array.as_slice(), |value| {
                value.is_nan()
            })
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            rows_any(array.shape().dimensions(), array.as_slice(), |value| {
                value.re.is_nan() || value.im.is_nan()
            })
        }
        Value::Array(ArrayData::F64(array)) => {
            rows_any(array.shape().dimensions(), array.as_slice(), |value| {
                value.is_nan()
            })
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            rows_any(array.shape().dimensions(), array.as_slice(), |value| {
                value.re.is_nan() || value.im.is_nan()
            })
        }
        Value::String(strings) => {
            let rows = host_length(strings.dimensions()[0])?;
            let mut result = vec![false; rows];
            if rows != 0 {
                for offset in 0..host_length(strings.numel())? {
                    result[offset % rows] |= strings
                        .element(offset)
                        .ok_or_else(|| mapping_error(name))?
                        .is_missing();
                }
            }
            Ok(result)
        }
        Value::Cell(cell) => {
            let rows = host_length(cell.shape().dimensions()[0])?;
            let mut result = vec![false; rows];
            if rows != 0 {
                for (offset, value) in cell.values().iter().enumerate() {
                    result[offset % rows] |= value_is_missing(value);
                }
            }
            Ok(result)
        }
        Value::Logical(_)
        | Value::Array(ArrayData::Logical(_) | ArrayData::Char(_) | ArrayData::Integer(_)) => {
            let rows = value
                .dimensions()
                .and_then(|dimensions| dimensions.first().copied())
                .unwrap_or(1);
            Ok(vec![false; host_length(rows)?])
        }
        _ => Err(domain_error(
            name,
            "this table variable class does not define missing values",
        )),
    }
}

fn value_is_missing(value: &Value) -> bool {
    match value {
        Value::Double(value) => value.is_nan(),
        Value::Complex(value) => value.real.is_nan() || value.imaginary.is_nan(),
        Value::Array(ArrayData::F32(array)) => array.as_slice().iter().any(|value| value.is_nan()),
        Value::Array(ArrayData::ComplexF32(array)) => array
            .as_slice()
            .iter()
            .any(|value| value.re.is_nan() || value.im.is_nan()),
        Value::Array(ArrayData::F64(array)) => array.as_slice().iter().any(|value| value.is_nan()),
        Value::Array(ArrayData::ComplexF64(array)) => array
            .as_slice()
            .iter()
            .any(|value| value.re.is_nan() || value.im.is_nan()),
        Value::String(StringValue::Scalar(value)) => value.is_missing(),
        Value::String(StringValue::Array(array)) => {
            array.as_slice().iter().any(StringElement::is_missing)
        }
        _ => false,
    }
}

fn fill_table_variable_missing(
    name: &str,
    value: &Value,
    replacement: &Value,
) -> Result<Value, BuiltinError> {
    let replacement_number = || {
        replacement.as_real_number().ok_or_else(|| {
            type_error(
                name,
                3,
                "real numeric scalar matching the selected variable",
                replacement,
            )
        })
    };
    match value {
        Value::Double(value) => Ok(Value::Double(if value.is_nan() {
            replacement_number()?
        } else {
            *value
        })),
        Value::Array(ArrayData::F64(array)) => {
            let replacement = replacement_number()?;
            DenseArray::from_vec(
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| if value.is_nan() { replacement } else { *value })
                    .collect(),
            )
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
        }
        Value::Array(ArrayData::F32(array)) => {
            #[allow(clippy::cast_possible_truncation)]
            let replacement = replacement_number()? as f32;
            DenseArray::from_vec(
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| if value.is_nan() { replacement } else { *value })
                    .collect(),
            )
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
        }
        Value::String(strings) => {
            let replacement = required_text_scalar(name, 3, replacement)?;
            let replacement = StringElement::from(replacement);
            let values = (0..host_length(strings.numel())?)
                .map(|offset| {
                    strings
                        .element(offset)
                        .cloned()
                        .map(|value| {
                            if value.is_missing() {
                                replacement.clone()
                            } else {
                                value
                            }
                        })
                        .ok_or_else(|| mapping_error(name))
                })
                .collect::<Result<Vec<_>, _>>()?;
            StringArray::from_elements(
                Shape::new(strings.dimensions().to_vec()).map_err(|error| array_error(&error))?,
                values,
            )
            .map(StringValue::Array)
            .map(Value::String)
            .map_err(|error| array_error(&error))
        }
        _ => Err(domain_error(
            name,
            "the selected table variable class does not support constant missing-value fill",
        )),
    }
}

fn join_arguments<'a>(
    name: &str,
    arguments: &'a [Value],
    outer: bool,
) -> Result<(&'a TableArray, &'a TableArray, String, bool), BuiltinError> {
    expect_argument_count(name, arguments, if outer { 6 } else { 4 })?;
    let left = table_argument(name, &arguments[0])?;
    let right = table_argument(name, &arguments[1])?;
    let keys = required_text_scalar(name, 3, &arguments[2])?;
    if !keys.eq_ignore_ascii_case("Keys") {
        return Err(domain_error(name, "input 3 must be the 'Keys' option"));
    }
    let key = required_text_scalar(name, 4, &arguments[3])?;
    let merge_keys = if outer {
        let option = required_text_scalar(name, 5, &arguments[4])?;
        if !option.eq_ignore_ascii_case("MergeKeys") {
            return Err(domain_error(name, "input 5 must be the 'MergeKeys' option"));
        }
        match arguments[5] {
            Value::Logical(value) => value,
            Value::Double(value) if matches!(value.to_bits(), 0 | 0x8000_0000_0000_0000) => false,
            Value::Double(value) if value.to_bits() == 1.0_f64.to_bits() => true,
            _ => return Err(type_error(name, 6, "logical scalar", &arguments[5])),
        }
    } else {
        true
    };
    Ok((left, right, key, merge_keys))
}

fn keyed_rows(
    name: &str,
    value: &Value,
    row_count: u64,
) -> Result<BTreeMap<TableKey, Vec<usize>>, BuiltinError> {
    let mut rows = BTreeMap::<TableKey, Vec<usize>>::new();
    for row in 0..host_length(row_count)? {
        rows.entry(table_key_at(name, value, row)?)
            .or_default()
            .push(row);
    }
    Ok(rows)
}

fn joined_table(
    name: &str,
    left: &TableArray,
    right: &TableArray,
    key: &str,
    pairs: &[(usize, usize)],
    context: &BuiltinContext<'_>,
) -> Result<TableArray, BuiltinError> {
    let left_rows = pairs.iter().map(|(left, _)| *left).collect::<Vec<_>>();
    let right_rows = pairs.iter().map(|(_, right)| *right).collect::<Vec<_>>();
    let mut names = left.variable_names().to_vec();
    let mut variables = Vec::with_capacity(left.variable_count() + right.variable_count() - 1);
    for index in 0..left.variable_count() {
        variables.push(gather_value_rows(
            name,
            left.variable(index).ok_or_else(|| mapping_error(name))?,
            &left_rows,
            context,
        )?);
    }
    for index in 0..right.variable_count() {
        let variable_name = right
            .variable_names()
            .get(index)
            .ok_or_else(|| mapping_error(name))?;
        if variable_name.as_str() == key {
            continue;
        }
        if names
            .iter()
            .any(|name| name.as_str() == variable_name.as_str())
        {
            return Err(domain_error(
                name,
                format!(
                    "non-key variable `{}` occurs in both input tables",
                    variable_name.as_str()
                ),
            ));
        }
        names.push(variable_name.clone());
        variables.push(gather_value_rows(
            name,
            right.variable(index).ok_or_else(|| mapping_error(name))?,
            &right_rows,
            context,
        )?);
    }
    TableArray::from_parts(pairs.len() as u64, names, variables)
        .map_err(|error| table_model_error(name, error))
}

fn outer_joined_table(
    name: &str,
    left: &TableArray,
    right: &TableArray,
    key: &str,
    pairs: &[(Option<usize>, Option<usize>)],
    context: &BuiltinContext<'_>,
) -> Result<TableArray, BuiltinError> {
    let left_key_index = left
        .variable_index(key)
        .ok_or_else(|| mapping_error(name))?;
    let right_key_index = right
        .variable_index(key)
        .ok_or_else(|| mapping_error(name))?;
    let mut names = Vec::with_capacity(left.variable_count() + right.variable_count() - 1);
    let mut variables = Vec::with_capacity(names.capacity());
    names.push(
        left.variable_names()
            .get(left_key_index)
            .ok_or_else(|| mapping_error(name))?
            .clone(),
    );
    variables.push(gather_merged_rows(
        name,
        left.variable(left_key_index)
            .ok_or_else(|| mapping_error(name))?,
        right
            .variable(right_key_index)
            .ok_or_else(|| mapping_error(name))?,
        pairs,
        context,
    )?);
    let left_rows = pairs.iter().map(|(left, _)| *left).collect::<Vec<_>>();
    let right_rows = pairs.iter().map(|(_, right)| *right).collect::<Vec<_>>();
    for index in 0..left.variable_count() {
        if index == left_key_index {
            continue;
        }
        let variable_name = left
            .variable_names()
            .get(index)
            .ok_or_else(|| mapping_error(name))?;
        names.push(variable_name.clone());
        variables.push(gather_optional_value_rows(
            name,
            left.variable(index).ok_or_else(|| mapping_error(name))?,
            &left_rows,
            context,
        )?);
    }
    for index in 0..right.variable_count() {
        if index == right_key_index {
            continue;
        }
        let variable_name = right
            .variable_names()
            .get(index)
            .ok_or_else(|| mapping_error(name))?;
        if names
            .iter()
            .any(|name| name.as_str() == variable_name.as_str())
        {
            return Err(domain_error(
                name,
                format!(
                    "non-key variable `{}` occurs in both input tables",
                    variable_name.as_str()
                ),
            ));
        }
        names.push(variable_name.clone());
        variables.push(gather_optional_value_rows(
            name,
            right.variable(index).ok_or_else(|| mapping_error(name))?,
            &right_rows,
            context,
        )?);
    }
    TableArray::from_parts(pairs.len() as u64, names, variables)
        .map_err(|error| table_model_error(name, error))
}

fn gather_optional_value_rows(
    name: &str,
    value: &Value,
    rows: &[Option<usize>],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! gather_array {
        ($array:expr, $missing:expr, $wrap:expr) => {{
            let (shape, values) = gather_optional_flat_rows(
                name,
                $array.shape().dimensions(),
                $array.as_slice(),
                rows,
                $missing,
                context,
            )?;
            DenseArray::from_vec(shape, values)
                .map($wrap)
                .map_err(|error| array_error(&error))
        }};
    }
    match value {
        Value::Double(item) => {
            let source = scalar_dense(*item)?;
            gather_array!(&source, f64::NAN, |array| Value::Array(ArrayData::F64(
                array
            )))
        }
        Value::Array(ArrayData::F64(array)) => {
            gather_array!(array, f64::NAN, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::F32(array)) => {
            gather_array!(array, f32::NAN, |array| Value::Array(ArrayData::F32(array)))
        }
        Value::Array(ArrayData::Logical(array)) => {
            gather_array!(array, Logical::from(false), |array| Value::Array(
                ArrayData::Logical(array)
            ))
        }
        Value::String(strings) => {
            let source = (0..host_length(strings.numel())?)
                .map(|offset| {
                    strings
                        .element(offset)
                        .cloned()
                        .ok_or_else(|| mapping_error(name))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (shape, values) = gather_optional_flat_rows(
                name,
                strings.dimensions(),
                &source,
                rows,
                StringElement::missing(),
                context,
            )?;
            StringArray::from_elements(shape, values)
                .map(StringValue::Array)
                .map(Value::String)
                .map_err(|error| array_error(&error))
        }
        Value::Cell(cell) => {
            let (shape, values) = gather_optional_flat_rows(
                name,
                cell.shape().dimensions(),
                cell.values(),
                rows,
                empty_char_value()?,
                context,
            )?;
            CellArray::from_values(shape, values)
                .map(Value::Cell)
                .map_err(|error| table_model_error(name, error))
        }
        _ => Err(domain_error(
            name,
            "this table variable class does not support unmatched outer-join rows",
        )),
    }
}

fn gather_optional_flat_rows<T: Clone>(
    name: &str,
    dimensions: &[u64],
    values: &[T],
    rows: &[Option<usize>],
    missing: T,
    context: &BuiltinContext<'_>,
) -> Result<(Shape, Vec<T>), BuiltinError> {
    let source_rows = host_length(
        dimensions
            .first()
            .copied()
            .ok_or_else(|| mapping_error(name))?,
    )?;
    if rows.iter().flatten().any(|row| *row >= source_rows) {
        return Err(mapping_error(name));
    }
    let mut dimensions = dimensions.to_vec();
    dimensions[0] = rows.len() as u64;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let blocks = if source_rows == 0 {
        1
    } else {
        values.len() / source_rows
    };
    let mut selected = Vec::with_capacity(host_length(shape.numel())?);
    for block in 0..blocks {
        cancellation_checkpoint(context, block)?;
        for row in rows {
            selected.push(match row {
                Some(row) => values
                    .get(block * source_rows + row)
                    .cloned()
                    .ok_or_else(|| mapping_error(name))?,
                None => missing.clone(),
            });
        }
    }
    Ok((shape, selected))
}

fn gather_merged_rows(
    name: &str,
    left: &Value,
    right: &Value,
    rows: &[(Option<usize>, Option<usize>)],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match (left, right) {
        (Value::Array(ArrayData::F64(left)), Value::Array(ArrayData::F64(right))) => {
            let values = rows
                .iter()
                .map(|(left_row, right_row)| match (left_row, right_row) {
                    (Some(row), _) => left.as_slice().get(*row).copied(),
                    (None, Some(row)) => right.as_slice().get(*row).copied(),
                    (None, None) => None,
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| mapping_error(name))?;
            double_column(values)
        }
        (Value::String(left), Value::String(right)) => {
            let values = rows
                .iter()
                .map(|(left_row, right_row)| match (left_row, right_row) {
                    (Some(row), _) => left.element(*row).cloned(),
                    (None, Some(row)) => right.element(*row).cloned(),
                    (None, None) => None,
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| mapping_error(name))?;
            let shape = Shape::new([rows.len() as u64, 1]).map_err(|error| array_error(&error))?;
            StringArray::from_elements(shape, values)
                .map(StringValue::Array)
                .map(Value::String)
                .map_err(|error| array_error(&error))
        }
        (Value::Cell(left), Value::Cell(right)) => {
            let values = rows
                .iter()
                .map(|(left_row, right_row)| match (left_row, right_row) {
                    (Some(row), _) => left.values().get(*row).cloned(),
                    (None, Some(row)) => right.values().get(*row).cloned(),
                    (None, None) => None,
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| mapping_error(name))?;
            let shape = Shape::new([rows.len() as u64, 1]).map_err(|error| array_error(&error))?;
            CellArray::from_values(shape, values)
                .map(Value::Cell)
                .map_err(|error| table_model_error(name, error))
        }
        _ => {
            let _ = context;
            Err(domain_error(
                name,
                "left and right join keys must have the same supported class",
            ))
        }
    }
}

fn empty_char_value() -> Result<Value, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([0, 0]).map_err(|error| array_error(&error))?,
        Vec::<CharCodeUnit>::new(),
    )
    .map(ArrayData::Char)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn table_groups(
    name: &str,
    table: &TableArray,
    group_index: usize,
    context: &BuiltinContext<'_>,
) -> Result<(Vec<usize>, Vec<usize>), BuiltinError> {
    let (first_rows, counts, _) = table_group_members(name, table, group_index, context)?;
    Ok((first_rows, counts))
}

type TableGroupMembers = (Vec<usize>, Vec<usize>, Vec<Vec<usize>>);

fn table_group_members(
    name: &str,
    table: &TableArray,
    group_index: usize,
    context: &BuiltinContext<'_>,
) -> Result<TableGroupMembers, BuiltinError> {
    let value = table
        .variable(group_index)
        .ok_or_else(|| mapping_error(name))?;
    let mut groups = BTreeMap::<TableKey, Vec<usize>>::new();
    for row in 0..host_length(table.row_count())? {
        cancellation_checkpoint(context, row)?;
        groups
            .entry(table_key_at(name, value, row)?)
            .or_default()
            .push(row);
    }
    let members = groups.into_values().collect::<Vec<_>>();
    let first_rows = members
        .iter()
        .filter_map(|rows| rows.first().copied())
        .collect::<Vec<_>>();
    let counts = members.iter().map(Vec::len).collect::<Vec<_>>();
    Ok((first_rows, counts, members))
}

fn double_column(values: Vec<f64>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([values.len() as u64, 1]).map_err(|error| array_error(&error))?,
        values,
    )
    .map(ArrayData::F64)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_precision_loss)]
fn usize_to_double(value: usize) -> f64 {
    value as f64
}

#[allow(clippy::cast_precision_loss)]
fn u64_to_double(value: u64) -> f64 {
    value as f64
}

fn mean_table_rows(name: &str, value: &Value, rows: &[usize]) -> Result<f64, BuiltinError> {
    let values = match value {
        Value::Double(value) => std::slice::from_ref(value),
        Value::Array(ArrayData::F64(array))
            if array.shape().dimensions().get(1).copied().unwrap_or(1) == 1 =>
        {
            array.as_slice()
        }
        _ => {
            return Err(domain_error(
                name,
                "the current mean summary requires a real double column variable",
            ));
        }
    };
    if rows.is_empty() {
        return Ok(f64::NAN);
    }
    let mut sum = 0.0;
    let mut count = 0_u64;
    for row in rows {
        let value = *values.get(*row).ok_or_else(|| mapping_error(name))?;
        if !value.is_nan() {
            sum += value;
            count += 1;
        }
    }
    if count == 0 {
        Ok(f64::NAN)
    } else {
        #[allow(clippy::cast_precision_loss)]
        Ok(sum / count as f64)
    }
}

fn select_end_rows(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    from_end: bool,
) -> BuiltinResult {
    expect_argument_count_range(name, arguments, 1, 2)?;
    expect_max_outputs(name, context, 1)?;
    let table = table_argument(name, &arguments[0])?;
    let requested = arguments
        .get(1)
        .map_or(Ok(8), |value| nonnegative_integer(name, 2, value))?;
    let count = requested.min(table.row_count());
    let start = if from_end {
        table.row_count() - count
    } else {
        0
    };
    let selected = select_table_rows(name, table, start, count, context)?;
    Ok(vec![Value::Table(selected)])
}

fn parse_boolean_option(
    name: &str,
    arguments: &[Value],
    option: &str,
    default: bool,
) -> Result<bool, BuiltinError> {
    if arguments.len() == 1 {
        return Ok(default);
    }
    if arguments.len() != 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("built-in `{name}` expects one input and an optional trailing '{option}' pair"),
        ));
    }
    let property = required_text_scalar(name, 2, &arguments[1])?;
    if !property.eq_ignore_ascii_case(option) {
        return Err(domain_error(
            name,
            format!("the only supported option is '{option}'"),
        ));
    }
    match &arguments[2] {
        Value::Logical(value) => Ok(*value),
        Value::Double(value) => match value.to_bits() {
            0 | 0x8000_0000_0000_0000 => Ok(false),
            0x3ff0_0000_0000_0000 => Ok(true),
            _ => Err(type_error(name, 3, "logical scalar", &arguments[2])),
        },
        value => Err(type_error(name, 3, "logical scalar", value)),
    }
}

struct TableConstructorArguments<'a> {
    variables: &'a [Value],
    variable_names: Option<Vec<TableVariableName>>,
    row_names: Option<Vec<TableRowName>>,
}

fn table_constructor_arguments(
    arguments: &[Value],
) -> Result<TableConstructorArguments<'_>, BuiltinError> {
    let mut variable_names_value = None;
    let mut row_names_value = None;
    let mut end = arguments.len();
    while end >= 2 {
        let Some(property) = text_scalar("table", end - 1, &arguments[end - 2])? else {
            break;
        };
        if property.eq_ignore_ascii_case("VariableNames") {
            if variable_names_value
                .replace((&arguments[end - 1], end))
                .is_some()
            {
                return Err(domain_error(
                    "table",
                    "'VariableNames' may be specified only once",
                ));
            }
        } else if property.eq_ignore_ascii_case("RowNames") {
            if row_names_value
                .replace((&arguments[end - 1], end))
                .is_some()
            {
                return Err(domain_error(
                    "table",
                    "'RowNames' may be specified only once",
                ));
            }
        } else {
            break;
        }
        end -= 2;
    }
    let variables = &arguments[..end];
    let variable_names = variable_names_value
        .map(|(value, position)| parse_variable_names("table", position, value, variables.len()))
        .transpose()?;
    let row_names = row_names_value
        .map(|(value, position)| parse_row_names("table", position, value))
        .transpose()?;
    Ok(TableConstructorArguments {
        variables,
        variable_names,
        row_names,
    })
}

fn parse_row_names(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<Vec<TableRowName>, BuiltinError> {
    text_vector(name, position, value, "cellstr or string vector")?
        .into_iter()
        .map(|text| TableRowName::new(text).map_err(|error| table_model_error(name, error)))
        .collect()
}

fn one_input_constructor<'a>(
    name: &str,
    arguments: &'a [Value],
) -> Result<(&'a Value, Option<Vec<TableVariableName>>), BuiltinError> {
    if arguments.len() == 1 {
        return Ok((&arguments[0], None));
    }
    if arguments.len() != 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `{name}` expects one input and an optional trailing 'VariableNames' pair"
            ),
        ));
    }
    let property = required_text_scalar(name, 2, &arguments[1])?;
    if !property.eq_ignore_ascii_case("VariableNames") {
        return Err(domain_error(
            name,
            "the only supported constructor option is 'VariableNames'",
        ));
    }
    let dimensions = arguments[0]
        .dimensions()
        .ok_or_else(|| type_error(name, 1, "two-dimensional array", &arguments[0]))?;
    if dimensions.len() != 2 {
        return Err(domain_error(
            name,
            "input 1 must be a two-dimensional array",
        ));
    }
    let expected = host_length(dimensions[1])?;
    let names = parse_variable_names(name, 3, &arguments[2], expected)?;
    Ok((&arguments[0], Some(names)))
}

fn parse_variable_names(
    name: &str,
    position: usize,
    value: &Value,
    expected: usize,
) -> Result<Vec<TableVariableName>, BuiltinError> {
    let texts = text_vector(name, position, value, "cellstr or string row")?;
    if !row_shape(value.dimensions().unwrap_or(&[])) {
        return Err(type_error(name, position, "cellstr or string row", value));
    }
    if texts.len() != expected {
        return Err(domain_error(
            name,
            format!(
                "'VariableNames' must contain {expected} names, found {}",
                texts.len()
            ),
        ));
    }
    texts
        .into_iter()
        .map(|text| TableVariableName::new(text).map_err(|error| table_model_error(name, error)))
        .collect()
}

fn text_vector(
    name: &str,
    position: usize,
    value: &Value,
    expected: &'static str,
) -> Result<Vec<String>, BuiltinError> {
    match value {
        Value::String(strings) if vector_shape(strings.dimensions()) => strings
            .as_array()
            .map_or_else(
                || strings.as_scalar().into_iter().cloned().collect(),
                |array| array.as_slice().to_vec(),
            )
            .into_iter()
            .map(|element| string_element_text(name, position, &element))
            .collect::<Result<Vec<_>, _>>(),
        Value::Cell(cell) if vector_shape(cell.shape().dimensions()) => cell
            .values()
            .iter()
            .map(|element| {
                required_text_scalar(name, position, element)
                    .map_err(|_| type_error(name, position, expected, value))
            })
            .collect::<Result<Vec<_>, _>>(),
        _ => Err(type_error(name, position, expected, value)),
    }
}

fn split_matrix_columns(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<(u64, Vec<Value>), BuiltinError> {
    let dimensions = value
        .dimensions()
        .ok_or_else(|| type_error(name, 1, "numeric, logical, char, or string matrix", value))?;
    if dimensions.len() != 2 {
        return Err(domain_error(
            name,
            "input 1 must be a two-dimensional array",
        ));
    }
    let rows = dimensions[0];
    let columns = match value {
        Value::Logical(item) => split_scalar_column(Logical::from(*item), |array| {
            Value::Array(ArrayData::Logical(array))
        })?,
        Value::Double(item) => {
            split_scalar_column(*item, |array| Value::Array(ArrayData::F64(array)))?
        }
        Value::Complex(item) => {
            split_scalar_column(ArrayComplex64::new(item.real, item.imaginary), |array| {
                Value::Array(ArrayData::ComplexF64(array))
            })?
        }
        Value::Array(ArrayData::F32(array)) => {
            split_dense_columns(array, |column| Value::Array(ArrayData::F32(column)))?
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            split_dense_columns(array, |column| Value::Array(ArrayData::ComplexF32(column)))?
        }
        Value::Array(ArrayData::Logical(array)) => {
            split_dense_columns(array, |column| Value::Array(ArrayData::Logical(column)))?
        }
        Value::Array(ArrayData::F64(array)) => {
            split_dense_columns(array, |column| Value::Array(ArrayData::F64(column)))?
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            split_dense_columns(array, |column| Value::Array(ArrayData::ComplexF64(column)))?
        }
        Value::Array(ArrayData::Char(array)) => {
            split_dense_columns(array, |column| Value::Array(ArrayData::Char(column)))?
        }
        Value::Array(ArrayData::Integer(integer)) => split_integer_columns(integer)?,
        Value::String(strings) => split_string_columns(strings)?,
        _ => {
            return Err(type_error(
                name,
                1,
                "numeric, logical, char, or string matrix",
                value,
            ));
        }
    };
    for index in 0..columns.len() {
        cancellation_checkpoint(context, index)?;
    }
    Ok((rows, columns))
}

fn split_scalar_column<T: Clone>(
    value: T,
    wrap: impl FnOnce(DenseArray<T>) -> Value,
) -> Result<Vec<Value>, BuiltinError> {
    let shape = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    let array = DenseArray::from_vec(shape, vec![value]).map_err(|error| array_error(&error))?;
    Ok(vec![wrap(array)])
}

fn split_dense_columns<T: Clone>(
    array: &DenseArray<T>,
    mut wrap: impl FnMut(DenseArray<T>) -> Value,
) -> Result<Vec<Value>, BuiltinError> {
    let dimensions = array.shape().dimensions();
    let rows = host_length(dimensions[0])?;
    let columns = host_length(dimensions[1])?;
    let column_shape = Shape::new([dimensions[0], 1]).map_err(|error| array_error(&error))?;
    let mut result = Vec::with_capacity(columns);
    for column in 0..columns {
        let start = column
            .checked_mul(rows)
            .ok_or_else(|| mapping_error("array2table"))?;
        let end = start
            .checked_add(rows)
            .ok_or_else(|| mapping_error("array2table"))?;
        let values = array
            .as_slice()
            .get(start..end)
            .ok_or_else(|| mapping_error("array2table"))?
            .to_vec();
        let column = DenseArray::from_vec(column_shape.clone(), values)
            .map_err(|error| array_error(&error))?;
        result.push(wrap(column));
    }
    Ok(result)
}

fn split_integer_columns(integer: &IntegerArrayData) -> Result<Vec<Value>, BuiltinError> {
    macro_rules! split {
        ($array:expr, $variant:ident) => {
            split_dense_columns($array, |column| {
                Value::Array(ArrayData::Integer(IntegerArrayData::$variant(column)))
            })
        };
    }
    match integer {
        IntegerArrayData::I8(array) => split!(array, I8),
        IntegerArrayData::ComplexI8(array) => split!(array, ComplexI8),
        IntegerArrayData::U8(array) => split!(array, U8),
        IntegerArrayData::ComplexU8(array) => split!(array, ComplexU8),
        IntegerArrayData::I16(array) => split!(array, I16),
        IntegerArrayData::ComplexI16(array) => split!(array, ComplexI16),
        IntegerArrayData::U16(array) => split!(array, U16),
        IntegerArrayData::ComplexU16(array) => split!(array, ComplexU16),
        IntegerArrayData::I32(array) => split!(array, I32),
        IntegerArrayData::ComplexI32(array) => split!(array, ComplexI32),
        IntegerArrayData::U32(array) => split!(array, U32),
        IntegerArrayData::ComplexU32(array) => split!(array, ComplexU32),
        IntegerArrayData::I64(array) => split!(array, I64),
        IntegerArrayData::ComplexI64(array) => split!(array, ComplexI64),
        IntegerArrayData::U64(array) => split!(array, U64),
        IntegerArrayData::ComplexU64(array) => split!(array, ComplexU64),
    }
}

fn split_string_columns(strings: &StringValue) -> Result<Vec<Value>, BuiltinError> {
    let dimensions = strings.dimensions();
    let rows = host_length(dimensions[0])?;
    let columns = host_length(dimensions[1])?;
    let shape = Shape::new([dimensions[0], 1]).map_err(|error| array_error(&error))?;
    let mut result = Vec::with_capacity(columns);
    for column in 0..columns {
        let start = column
            .checked_mul(rows)
            .ok_or_else(|| mapping_error("array2table"))?;
        let end = start
            .checked_add(rows)
            .ok_or_else(|| mapping_error("array2table"))?;
        let values = (start..end)
            .map(|offset| {
                strings
                    .element(offset)
                    .cloned()
                    .ok_or_else(|| mapping_error("array2table"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let array = StringArray::from_elements(shape.clone(), values)
            .map_err(|error| array_error(&error))?;
        result.push(Value::String(StringValue::Array(array)));
    }
    Ok(result)
}

fn merge_cell_column(
    values: &[Value],
    row_count: u64,
    context: &mut BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if values.is_empty() || !mergeable_cell_column(values) {
        return cell_column(values, row_count);
    }
    if matches!(values[0], Value::String(_)) {
        return merge_string_rows(values);
    }
    let mut outputs = crate::core_concat::vertcat_builtin(values, context)?;
    outputs.pop().ok_or_else(|| mapping_error("cell2table"))
}

fn merge_struct_field(
    values: &[Value],
    row_count: u64,
    context: &mut BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    // MATLAB treats a character row as one text value per struct record,
    // rather than vertically joining those rows into a character matrix.
    if values
        .iter()
        .any(|value| matches!(value, Value::Array(ArrayData::Char(_))))
    {
        return cell_column(values, row_count);
    }
    merge_cell_column(values, row_count, context)
}

fn mergeable_cell_column(values: &[Value]) -> bool {
    let Some(first) = values.first() else {
        return false;
    };
    if !matches!(
        first,
        Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::Logical(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Char(_)
                    | ArrayData::Integer(_)
            )
            | Value::String(_)
    ) {
        return false;
    }
    let Some(first_dimensions) = first.dimensions() else {
        return false;
    };
    if first_dimensions.len() != 2 || first_dimensions[0] != 1 {
        return false;
    }
    let class = first.class_name();
    if first.dtype().is_some_and(openmat_array::DType::is_integer)
        && values.iter().any(Value::is_complex_numeric)
    {
        return false;
    }
    values.iter().all(|value| {
        value.class_name() == class
            && value.dimensions().is_some_and(|dimensions| {
                dimensions.len() == 2 && dimensions[0] == 1 && dimensions[1] == first_dimensions[1]
            })
    })
}

fn merge_string_rows(values: &[Value]) -> Result<Value, BuiltinError> {
    let columns = values[0]
        .dimensions()
        .and_then(|dimensions| dimensions.get(1))
        .copied()
        .ok_or_else(|| mapping_error("cell2table"))?;
    let rows = u64::try_from(values.len()).map_err(|_| mapping_error("cell2table"))?;
    let mut elements = Vec::with_capacity(host_length(
        rows.checked_mul(columns)
            .ok_or_else(|| mapping_error("cell2table"))?,
    )?);
    let host_columns = host_length(columns)?;
    for column in 0..host_columns {
        for value in values {
            let Value::String(strings) = value else {
                return Err(mapping_error("cell2table"));
            };
            elements.push(
                strings
                    .element(column)
                    .cloned()
                    .ok_or_else(|| mapping_error("cell2table"))?,
            );
        }
    }
    let shape = Shape::new([rows, columns]).map_err(|error| array_error(&error))?;
    let array = StringArray::from_elements(shape, elements).map_err(|error| array_error(&error))?;
    Ok(Value::String(StringValue::Array(array)))
}

fn cell_column(values: &[Value], row_count: u64) -> Result<Value, BuiltinError> {
    let shape = Shape::new([row_count, 1]).map_err(|error| array_error(&error))?;
    let cell = CellArray::from_values(shape, values.to_vec())
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))?;
    Ok(Value::Cell(cell))
}

macro_rules! concatenate_integer {
    ($variables:expr, $shape:expr, $real:ident, $complex:ident, $component:ty) => {{
        let complex = $variables.iter().any(|value| value.is_complex_numeric());
        if complex {
            let mut data = Vec::new();
            for value in &$variables {
                let Value::Array(ArrayData::Integer(integer)) = value else {
                    return Err(mapping_error("table2array"));
                };
                match integer {
                    IntegerArrayData::$real(array) => data.extend(
                        array
                            .as_slice()
                            .iter()
                            .copied()
                            .map(|item| ComplexInteger::new(item, 0 as $component)),
                    ),
                    IntegerArrayData::$complex(array) => {
                        data.extend(array.as_slice().iter().copied());
                    }
                    _ => return Err(mapping_error("table2array")),
                }
            }
            let array = DenseArray::from_vec($shape, data).map_err(|error| array_error(&error))?;
            Value::Array(ArrayData::Integer(IntegerArrayData::$complex(array)))
        } else {
            let mut data = Vec::new();
            for value in &$variables {
                let Value::Array(ArrayData::Integer(IntegerArrayData::$real(array))) = value else {
                    return Err(mapping_error("table2array"));
                };
                data.extend(array.as_slice().iter().copied());
            }
            let array = DenseArray::from_vec($shape, data).map_err(|error| array_error(&error))?;
            Value::Array(ArrayData::Integer(IntegerArrayData::$real(array)))
        }
    }};
}

fn concatenate_table_variables(
    table: &TableArray,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if table.variable_count() == 0 {
        let shape = Shape::new([table.row_count(), 0]).map_err(|error| array_error(&error))?;
        let array = DenseArray::from_vec(shape, Vec::new()).map_err(|error| array_error(&error))?;
        return Ok(Value::Array(ArrayData::F64(array)));
    }
    let variables = (0..table.variable_count())
        .map(|index| {
            table
                .variable(index)
                .ok_or_else(|| mapping_error("table2array"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first_class = variables[0].class_name();
    let mut total_columns = 0_u64;
    for (index, value) in variables.iter().enumerate() {
        cancellation_checkpoint(context, index)?;
        let Some(dimensions) = value.dimensions() else {
            return Err(type_error(
                "table2array",
                1,
                "table with two-dimensional homogeneous variables",
                value,
            ));
        };
        if dimensions.len() != 2 || dimensions[0] != table.row_count() {
            return Err(domain_error(
                "table2array",
                "every table variable must be two-dimensional for horizontal concatenation",
            ));
        }
        if value.class_name() != first_class || !table2array_class(value) {
            return Err(domain_error(
                "table2array",
                "table variables must have one losslessly compatible numeric, logical, char, or string class",
            ));
        }
        total_columns = total_columns
            .checked_add(dimensions[1])
            .ok_or_else(|| mapping_error("table2array"))?;
    }
    let shape =
        Shape::new([table.row_count(), total_columns]).map_err(|error| array_error(&error))?;
    let output = match first_class {
        "double" => concatenate_double(&variables, shape)?,
        "single" => concatenate_single(&variables, shape)?,
        "logical" => concatenate_logical(&variables, shape)?,
        "char" => concatenate_char(&variables, shape)?,
        "string" => concatenate_string(&variables, shape)?,
        "cell" => concatenate_cell(&variables, shape)?,
        "int8" => concatenate_integer!(variables, shape, I8, ComplexI8, i8),
        "uint8" => concatenate_integer!(variables, shape, U8, ComplexU8, u8),
        "int16" => concatenate_integer!(variables, shape, I16, ComplexI16, i16),
        "uint16" => concatenate_integer!(variables, shape, U16, ComplexU16, u16),
        "int32" => concatenate_integer!(variables, shape, I32, ComplexI32, i32),
        "uint32" => concatenate_integer!(variables, shape, U32, ComplexU32, u32),
        "int64" => concatenate_integer!(variables, shape, I64, ComplexI64, i64),
        "uint64" => concatenate_integer!(variables, shape, U64, ComplexU64, u64),
        _ => {
            return Err(domain_error(
                "table2array",
                "table variables must be numeric, logical, char, or string arrays",
            ));
        }
    };
    context.check_cancelled()?;
    Ok(output)
}

fn table2array_class(value: &Value) -> bool {
    matches!(
        value,
        Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::Logical(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Char(_)
                    | ArrayData::Integer(_)
            )
            | Value::String(_)
            | Value::Cell(_)
    )
}

fn concatenate_double(variables: &[&Value], shape: Shape) -> Result<Value, BuiltinError> {
    if variables.iter().any(|value| value.is_complex_numeric()) {
        let mut data = Vec::new();
        for value in variables {
            match value {
                Value::Double(item) => data.push(ArrayComplex64::new(*item, 0.0)),
                Value::Complex(item) => {
                    data.push(ArrayComplex64::new(item.real, item.imaginary));
                }
                Value::Array(ArrayData::F64(array)) => data.extend(
                    array
                        .as_slice()
                        .iter()
                        .copied()
                        .map(|item| ArrayComplex64::new(item, 0.0)),
                ),
                Value::Array(ArrayData::ComplexF64(array)) => {
                    data.extend(array.as_slice().iter().copied());
                }
                _ => return Err(mapping_error("table2array")),
            }
        }
        let array = DenseArray::from_vec(shape, data).map_err(|error| array_error(&error))?;
        Ok(Value::Array(ArrayData::ComplexF64(array)))
    } else {
        let mut data = Vec::new();
        for value in variables {
            match value {
                Value::Double(item) => data.push(*item),
                Value::Array(ArrayData::F64(array)) => {
                    data.extend(array.as_slice().iter().copied());
                }
                _ => return Err(mapping_error("table2array")),
            }
        }
        let array = DenseArray::from_vec(shape, data).map_err(|error| array_error(&error))?;
        Ok(Value::Array(ArrayData::F64(array)))
    }
}

fn concatenate_single(variables: &[&Value], shape: Shape) -> Result<Value, BuiltinError> {
    if variables.iter().any(|value| value.is_complex_numeric()) {
        let mut data = Vec::new();
        for value in variables {
            match value {
                Value::Array(ArrayData::F32(array)) => data.extend(
                    array
                        .as_slice()
                        .iter()
                        .copied()
                        .map(|item| Complex32::new(item, 0.0)),
                ),
                Value::Array(ArrayData::ComplexF32(array)) => {
                    data.extend(array.as_slice().iter().copied());
                }
                _ => return Err(mapping_error("table2array")),
            }
        }
        let array = DenseArray::from_vec(shape, data).map_err(|error| array_error(&error))?;
        Ok(Value::Array(ArrayData::ComplexF32(array)))
    } else {
        let mut data = Vec::new();
        for value in variables {
            let Value::Array(ArrayData::F32(array)) = value else {
                return Err(mapping_error("table2array"));
            };
            data.extend(array.as_slice().iter().copied());
        }
        let array = DenseArray::from_vec(shape, data).map_err(|error| array_error(&error))?;
        Ok(Value::Array(ArrayData::F32(array)))
    }
}

fn concatenate_logical(variables: &[&Value], shape: Shape) -> Result<Value, BuiltinError> {
    let mut data = Vec::new();
    for value in variables {
        match value {
            Value::Logical(item) => data.push(Logical::from(*item)),
            Value::Array(ArrayData::Logical(array)) => {
                data.extend(array.as_slice().iter().copied());
            }
            _ => return Err(mapping_error("table2array")),
        }
    }
    let array = DenseArray::from_vec(shape, data).map_err(|error| array_error(&error))?;
    Ok(Value::Array(ArrayData::Logical(array)))
}

fn concatenate_char(variables: &[&Value], shape: Shape) -> Result<Value, BuiltinError> {
    let mut data = Vec::new();
    for value in variables {
        let Value::Array(ArrayData::Char(array)) = value else {
            return Err(mapping_error("table2array"));
        };
        data.extend(array.as_slice().iter().copied());
    }
    let array = DenseArray::from_vec(shape, data).map_err(|error| array_error(&error))?;
    Ok(Value::Array(ArrayData::Char(array)))
}

fn concatenate_string(variables: &[&Value], shape: Shape) -> Result<Value, BuiltinError> {
    let mut data = Vec::new();
    for value in variables {
        let Value::String(strings) = value else {
            return Err(mapping_error("table2array"));
        };
        for offset in 0..host_length(strings.numel())? {
            data.push(
                strings
                    .element(offset)
                    .cloned()
                    .ok_or_else(|| mapping_error("table2array"))?,
            );
        }
    }
    let array = StringArray::from_elements(shape, data).map_err(|error| array_error(&error))?;
    Ok(Value::String(StringValue::Array(array)))
}

fn concatenate_cell(variables: &[&Value], shape: Shape) -> Result<Value, BuiltinError> {
    let mut values = Vec::with_capacity(host_length(shape.numel())?);
    for value in variables {
        let Value::Cell(cell) = value else {
            return Err(mapping_error("table2array"));
        };
        values.extend(cell.values().iter().cloned());
    }
    CellArray::from_values(shape, values)
        .map(Value::Cell)
        .map_err(|error| table_model_error("table2array", error))
}

fn select_table_rows(
    name: &str,
    table: &TableArray,
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<TableArray, BuiltinError> {
    let mut variables = Vec::with_capacity(table.variable_count());
    for index in 0..table.variable_count() {
        cancellation_checkpoint(context, index)?;
        let variable = table.variable(index).ok_or_else(|| mapping_error(name))?;
        variables.push(slice_rows(name, variable, start, count, context)?);
    }
    let row_names = table.row_names().map(|names| {
        let start = host_length(start)?;
        let end = start
            .checked_add(host_length(count)?)
            .ok_or_else(|| mapping_error(name))?;
        names
            .get(start..end)
            .map(<[TableRowName]>::to_vec)
            .ok_or_else(|| mapping_error(name))
    });
    TableArray::from_parts_with_row_names(
        count,
        table.variable_names().to_vec(),
        variables,
        row_names.transpose()?,
    )
    .map_err(|error| table_model_error(name, error))
}

fn slice_rows(
    name: &str,
    value: &Value,
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! slice_array {
        ($array:expr, $wrap:expr) => {{
            let array = slice_dense_rows(name, $array, start, count, context)?;
            Ok($wrap(array))
        }};
    }
    match value {
        Value::Logical(item) => {
            let source = scalar_dense(Logical::from(*item))?;
            slice_array!(&source, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Double(item) => {
            let source = scalar_dense(*item)?;
            slice_array!(&source, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Complex(item) => {
            let source = scalar_dense(ArrayComplex64::new(item.real, item.imaginary))?;
            slice_array!(&source, |array| Value::Array(ArrayData::ComplexF64(array)))
        }
        Value::Array(ArrayData::F32(array)) => {
            slice_array!(array, |array| { Value::Array(ArrayData::F32(array)) })
        }
        Value::Array(ArrayData::ComplexF32(array)) => slice_array!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        }),
        Value::Array(ArrayData::Logical(array)) => {
            slice_array!(array, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Array(ArrayData::F64(array)) => {
            slice_array!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            slice_array!(array, |array| Value::Array(ArrayData::ComplexF64(array)))
        }
        Value::Array(ArrayData::Char(array)) => {
            slice_array!(array, |array| Value::Array(ArrayData::Char(array)))
        }
        Value::Array(ArrayData::Integer(integer)) => {
            slice_integer_rows(name, integer, start, count, context)
        }
        Value::String(strings) => slice_string_rows(name, strings, start, count, context),
        Value::Cell(cell) => {
            let (shape, values) = slice_flat_rows(
                name,
                cell.shape().dimensions(),
                cell.values(),
                start,
                count,
                context,
            )?;
            let cell = CellArray::from_values(shape, values).map_err(|error| {
                BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string())
            })?;
            Ok(Value::Cell(cell))
        }
        Value::Struct(structure) => slice_struct_rows(name, structure, start, count, context),
        Value::Table(table) => Ok(Value::Table(select_table_rows(
            name, table, start, count, context,
        )?)),
        Value::Object(_) | Value::ObjectArray(_) => {
            slice_object_rows(name, value, start, count, context)
        }
        Value::Graphics(_) | Value::GraphicsArray(_) => {
            slice_graphics_rows(name, value, start, count, context)
        }
        Value::Nothing | Value::Sparse(_) | Value::Function(_) => Err(type_error(
            name,
            1,
            "table whose variables support row selection",
            value,
        )),
    }
}

fn slice_struct_rows(
    name: &str,
    structure: &StructArray,
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let mut columns = Vec::with_capacity(structure.field_count());
    let mut selected_shape = None;
    for field in 0..structure.field_count() {
        let values = structure
            .field_values(field)
            .ok_or_else(|| mapping_error(name))?;
        let (shape, values) = slice_flat_rows(
            name,
            structure.shape().dimensions(),
            values,
            start,
            count,
            context,
        )?;
        selected_shape = Some(shape);
        columns.push(values);
    }
    let shape = selected_shape.unwrap_or_else(|| {
        let mut dimensions = structure.shape().dimensions().to_vec();
        dimensions[0] = count;
        Shape::new(dimensions).expect("validated struct dimensions remain valid")
    });
    let structure = StructArray::from_columns(shape, structure.field_names().to_vec(), columns)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))?;
    Ok(Value::Struct(structure))
}

fn slice_object_rows(
    name: &str,
    value: &Value,
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match value {
        Value::Object(object) if start == 0 && count == 1 => Ok(Value::Object(*object)),
        Value::Object(_) => Err(domain_error(
            name,
            "empty row selection for a scalar object is not representable by the current object value model",
        )),
        Value::ObjectArray(array) => {
            let (shape, values) = slice_flat_rows(
                name,
                array.shape().dimensions(),
                array.as_slice(),
                start,
                count,
                context,
            )?;
            let array = ObjectArray::from_vec(array.class_handle(), shape, values)
                .map_err(|error| array_error(&error))?;
            Ok(Value::ObjectArray(array))
        }
        _ => Err(mapping_error(name)),
    }
}

fn slice_graphics_rows(
    name: &str,
    value: &Value,
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match value {
        Value::Graphics(handle) if start == 0 && count == 1 => Ok(Value::Graphics(*handle)),
        Value::Graphics(handle) => {
            let array =
                GraphicsHandleArray::column(handle.class(), Vec::new()).map_err(|error| {
                    BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string())
                })?;
            Ok(Value::GraphicsArray(array))
        }
        Value::GraphicsArray(array) => {
            let (_, handles) = slice_flat_rows(
                name,
                array.shape().dimensions(),
                array.as_slice(),
                start,
                count,
                context,
            )?;
            let array = GraphicsHandleArray::column(array.class(), handles).map_err(|error| {
                BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string())
            })?;
            Ok(Value::GraphicsArray(array))
        }
        _ => Err(mapping_error(name)),
    }
}

fn slice_dense_rows<T: Clone>(
    name: &str,
    array: &DenseArray<T>,
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let (shape, values) = slice_flat_rows(
        name,
        array.shape().dimensions(),
        array.as_slice(),
        start,
        count,
        context,
    )?;
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn slice_flat_rows<T: Clone>(
    name: &str,
    dimensions: &[u64],
    values: &[T],
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<(Shape, Vec<T>), BuiltinError> {
    let rows = dimensions
        .first()
        .copied()
        .ok_or_else(|| mapping_error(name))?;
    let end = start
        .checked_add(count)
        .ok_or_else(|| mapping_error(name))?;
    if end > rows {
        return Err(mapping_error(name));
    }
    let mut selected_dimensions = dimensions.to_vec();
    selected_dimensions[0] = count;
    let shape = Shape::new(selected_dimensions).map_err(|error| array_error(&error))?;
    if rows == 0 || count == 0 {
        return Ok((shape, Vec::new()));
    }
    let host_rows = host_length(rows)?;
    let host_start = host_length(start)?;
    let host_end = host_length(end)?;
    let blocks = values.len() / host_rows;
    let capacity = host_length(shape.numel())?;
    let mut selected = Vec::with_capacity(capacity);
    for block in 0..blocks {
        cancellation_checkpoint(context, block)?;
        let offset = block
            .checked_mul(host_rows)
            .ok_or_else(|| mapping_error(name))?;
        selected.extend_from_slice(
            values
                .get(offset + host_start..offset + host_end)
                .ok_or_else(|| mapping_error(name))?,
        );
    }
    Ok((shape, selected))
}

fn slice_integer_rows(
    name: &str,
    integer: &IntegerArrayData,
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! slice {
        ($array:expr, $variant:ident) => {{
            let array = slice_dense_rows(name, $array, start, count, context)?;
            Ok(Value::Array(ArrayData::Integer(
                IntegerArrayData::$variant(array),
            )))
        }};
    }
    match integer {
        IntegerArrayData::I8(array) => slice!(array, I8),
        IntegerArrayData::ComplexI8(array) => slice!(array, ComplexI8),
        IntegerArrayData::U8(array) => slice!(array, U8),
        IntegerArrayData::ComplexU8(array) => slice!(array, ComplexU8),
        IntegerArrayData::I16(array) => slice!(array, I16),
        IntegerArrayData::ComplexI16(array) => slice!(array, ComplexI16),
        IntegerArrayData::U16(array) => slice!(array, U16),
        IntegerArrayData::ComplexU16(array) => slice!(array, ComplexU16),
        IntegerArrayData::I32(array) => slice!(array, I32),
        IntegerArrayData::ComplexI32(array) => slice!(array, ComplexI32),
        IntegerArrayData::U32(array) => slice!(array, U32),
        IntegerArrayData::ComplexU32(array) => slice!(array, ComplexU32),
        IntegerArrayData::I64(array) => slice!(array, I64),
        IntegerArrayData::ComplexI64(array) => slice!(array, ComplexI64),
        IntegerArrayData::U64(array) => slice!(array, U64),
        IntegerArrayData::ComplexU64(array) => slice!(array, ComplexU64),
    }
}

fn slice_string_rows(
    name: &str,
    strings: &StringValue,
    start: u64,
    count: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let values = (0..host_length(strings.numel())?)
        .map(|offset| {
            strings
                .element(offset)
                .cloned()
                .ok_or_else(|| mapping_error(name))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (shape, values) =
        slice_flat_rows(name, strings.dimensions(), &values, start, count, context)?;
    let array = StringArray::from_elements(shape, values).map_err(|error| array_error(&error))?;
    Ok(Value::String(StringValue::Array(array)))
}

fn collapse_table_cell_value(value: Value) -> Value {
    match value {
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            Value::Double(array.as_slice()[0])
        }
        Value::Array(ArrayData::ComplexF64(array)) if array.numel() == 1 => {
            let item = array.as_slice()[0];
            Value::Complex(ValueComplex64::new(item.re, item.im))
        }
        Value::Array(ArrayData::Logical(array)) if array.numel() == 1 => {
            Value::Logical(array.as_slice()[0].get())
        }
        Value::String(StringValue::Array(array)) if array.numel() == 1 => {
            Value::String(StringValue::Scalar(array.as_slice()[0].clone()))
        }
        Value::Cell(cell) if cell.numel() == 1 => cell.values()[0].clone(),
        Value::ObjectArray(array) if array.numel() == 1 => Value::Object(array.as_slice()[0]),
        Value::GraphicsArray(array) if array.numel() == 1 => Value::Graphics(array.as_slice()[0]),
        value => value,
    }
}

fn scalar_dense<T: Clone>(value: T) -> Result<DenseArray<T>, BuiltinError> {
    let shape = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, vec![value]).map_err(|error| array_error(&error))
}

fn table_argument<'a>(name: &str, value: &'a Value) -> Result<&'a TableArray, BuiltinError> {
    value
        .as_table()
        .ok_or_else(|| type_error(name, 1, "table", value))
}

fn default_names(count: usize) -> Vec<TableVariableName> {
    (1..=count)
        .map(|index| {
            TableVariableName::new(format!("Var{index}"))
                .expect("generated table variable names are valid")
        })
        .collect()
}

fn row_shape(dimensions: &[u64]) -> bool {
    dimensions.len() == 2 && dimensions[0] == 1
}

fn vector_shape(dimensions: &[u64]) -> bool {
    dimensions.len() == 2 && (dimensions[0] == 1 || dimensions[1] == 1)
}

fn required_text_scalar(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<String, BuiltinError> {
    text_scalar(name, position, value)?.ok_or_else(|| {
        type_error(
            name,
            position,
            "character row or non-missing string scalar",
            value,
        )
    })
}

fn text_scalar(name: &str, position: usize, value: &Value) -> Result<Option<String>, BuiltinError> {
    match value {
        Value::String(strings) => {
            let Some(element) = strings.as_scalar() else {
                return Ok(None);
            };
            string_element_text(name, position, element).map(Some)
        }
        Value::Array(ArrayData::Char(array)) if row_shape(array.shape().dimensions()) => {
            let units = array
                .as_slice()
                .iter()
                .map(|unit| unit.get())
                .collect::<Vec<_>>();
            String::from_utf16(&units).map(Some).map_err(|_| {
                domain_error(
                    name,
                    format!("input {position} contains invalid UTF-16 text"),
                )
            })
        }
        _ => Ok(None),
    }
}

fn string_element_text(
    name: &str,
    position: usize,
    element: &StringElement,
) -> Result<String, BuiltinError> {
    if element.is_missing() {
        return Err(domain_error(
            name,
            format!("input {position} contains a missing variable name"),
        ));
    }
    String::from_utf16(element.code_units()).map_err(|_| {
        domain_error(
            name,
            format!("input {position} contains invalid UTF-16 text"),
        )
    })
}

fn nonnegative_integer(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    if let Some(component) = exact_real_integer_scalar(value) {
        let result = match component {
            IntegerComponent::Signed(value) => u64::try_from(value).ok(),
            IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
        };
        return result.ok_or_else(|| nonnegative_integer_error(name, position));
    }
    let Some(number) = value.as_real_number() else {
        return Err(type_error(
            name,
            position,
            "nonnegative integer scalar",
            value,
        ));
    };
    if !number.is_finite()
        || number < 0.0
        || number.fract() != 0.0
        || number >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(nonnegative_integer_error(name, position));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as u64)
}

fn nonnegative_integer_error(name: &str, position: usize) -> BuiltinError {
    domain_error(
        name,
        format!("input {position} must be a nonnegative integer scalar"),
    )
}

#[allow(clippy::cast_precision_loss)]
fn dimension_value(value: u64) -> Value {
    Value::Double(value as f64)
}

fn cancellation_checkpoint(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

fn host_length(value: u64) -> Result<usize, BuiltinError> {
    usize::try_from(value).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "table operation exceeds host addressable storage",
        )
    })
}

fn table_model_error(name: &str, error: impl std::fmt::Display) -> BuiltinError {
    domain_error(name, error.to_string())
}

fn domain_error(name: &str, detail: impl std::fmt::Display) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Domain, format!("`{name}`: {detail}"))
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Other,
        format!("`{name}` could not map validated table storage"),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openmat_array::{CharCodeUnit, Complex64 as ArrayComplex64};
    use openmat_runtime::{CancellationToken, VecOutput};

    use super::*;

    fn invoke(
        builtin: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
        arguments: &[Value],
    ) -> Result<Vec<Value>, BuiltinError> {
        let cancellation = CancellationToken::new();
        let mut output = VecOutput::new();
        let mut context = BuiltinContext::new(1, &cancellation, &mut output);
        builtin(arguments, &mut context)
    }

    fn f64_array(dimensions: [u64; 2], values: &[f64]) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values.to_vec()).unwrap(),
        ))
    }

    fn string_row(values: &[&str]) -> Value {
        let shape = Shape::new([1, values.len() as u64]).unwrap();
        let elements = values
            .iter()
            .map(|value| StringElement::from_utf8(value))
            .collect();
        Value::String(StringValue::Array(
            StringArray::from_elements(shape, elements).unwrap(),
        ))
    }

    fn table_value(names: &[&str], variables: Vec<Value>) -> Value {
        let names = names
            .iter()
            .map(|name| TableVariableName::new(*name).unwrap())
            .collect();
        Value::Table(TableArray::from_variables(names, variables).unwrap())
    }

    fn output_table(outputs: &[Value]) -> &TableArray {
        let Value::Table(table) = &outputs[0] else {
            panic!("expected a table output");
        };
        table
    }

    #[test]
    fn table_preserves_whole_variables_and_uses_runtime_default_names() {
        let matrix = f64_array([2, 2], &[1.0, 2.0, 3.0, 4.0]);
        let column = f64_array([2, 1], &[5.0, 6.0]);
        let outputs = invoke(table_builtin, &[matrix.clone(), column.clone()]).unwrap();
        let table = output_table(&outputs);
        assert_eq!(table.shape().dimensions(), &[2, 2]);
        assert_eq!(table.variable_names(), default_names(2));
        assert_eq!(table.variable(0), Some(&matrix));
        assert_eq!(table.variable(1), Some(&column));

        let empty = invoke(table_builtin, &[]).unwrap();
        assert_eq!(output_table(&empty).shape().dimensions(), &[0, 0]);
    }

    #[test]
    fn table_accepts_string_variable_names_and_rejects_row_mismatch() {
        let names = string_row(&["left value", "right"]);
        let arguments = [
            f64_array([2, 1], &[1.0, 2.0]),
            f64_array([2, 1], &[3.0, 4.0]),
            Value::from("VariableNames"),
            names,
        ];
        let outputs = invoke(table_builtin, &arguments).unwrap();
        let table = output_table(&outputs);
        assert_eq!(table.variable_names()[0].as_str(), "left value");
        assert_eq!(table.variable_names()[1].as_str(), "right");

        let error = invoke(
            table_builtin,
            &[f64_array([2, 1], &[1.0, 2.0]), f64_array([1, 1], &[3.0])],
        )
        .unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn array2table_splits_column_major_matrix_without_changing_class() {
        let matrix = f64_array([2, 2], &[1.0, 2.0, 3.0, 4.0]);
        let outputs = invoke(
            array2table_builtin,
            &[
                matrix,
                Value::from("VariableNames"),
                string_row(&["A", "B"]),
            ],
        )
        .unwrap();
        let table = output_table(&outputs);
        assert_eq!(table.variable_names()[0].as_str(), "A");
        assert_eq!(table.variable_names()[1].as_str(), "B");
        let Value::Array(ArrayData::F64(first)) = table.variable(0).unwrap() else {
            panic!("first column must remain double");
        };
        let Value::Array(ArrayData::F64(second)) = table.variable(1).unwrap() else {
            panic!("second column must remain double");
        };
        assert_eq!(first.as_slice(), &[1.0, 2.0]);
        assert_eq!(second.as_slice(), &[3.0, 4.0]);
    }

    #[test]
    fn zero_column_and_zero_row_constructors_preserve_explicit_table_height_and_width() {
        let empty_matrix = f64_array([3, 0], &[]);
        let outputs = invoke(array2table_builtin, &[empty_matrix]).unwrap();
        assert_eq!(output_table(&outputs).shape().dimensions(), &[3, 0]);

        let empty_cell = CellArray::from_values(Shape::new([0, 2]).unwrap(), Vec::new()).unwrap();
        let outputs = invoke(cell2table_builtin, &[Value::Cell(empty_cell)]).unwrap();
        let table = output_table(&outputs);
        assert_eq!(table.shape().dimensions(), &[0, 2]);
        assert!(
            matches!(table.variable(0), Some(Value::Cell(cell)) if cell.shape().dimensions() == [0, 1])
        );
        assert!(
            matches!(table.variable(1), Some(Value::Cell(cell)) if cell.shape().dimensions() == [0, 1])
        );
    }

    #[test]
    fn cell2table_merges_homogeneous_columns_and_preserves_incompatible_cells() {
        let cell = CellArray::from_values(
            Shape::new([2, 2]).unwrap(),
            vec![
                Value::Double(1.0),
                Value::Double(2.0),
                Value::from("x"),
                Value::from("y"),
            ],
        )
        .unwrap();
        let outputs = invoke(cell2table_builtin, &[Value::Cell(cell)]).unwrap();
        let table = output_table(&outputs);
        assert!(matches!(
            table.variable(0),
            Some(Value::Array(ArrayData::F64(array))) if array.as_slice() == [1.0, 2.0]
        ));
        assert!(matches!(
            table.variable(1),
            Some(Value::String(StringValue::Array(array))) if array.shape().dimensions() == [2, 1]
        ));

        let mixed = CellArray::from_values(
            Shape::new([2, 1]).unwrap(),
            vec![Value::Double(1.0), Value::from("x")],
        )
        .unwrap();
        let outputs = invoke(cell2table_builtin, &[Value::Cell(mixed)]).unwrap();
        assert!(matches!(
            output_table(&outputs).variable(0),
            Some(Value::Cell(_))
        ));
    }

    #[test]
    fn scalar_struct_conversion_keeps_complete_field_values() {
        let fields = vec![FieldName::new("A").unwrap(), FieldName::new("B").unwrap()];
        let first = f64_array([2, 1], &[1.0, 2.0]);
        let second = f64_array([2, 2], &[3.0, 4.0, 5.0, 6.0]);
        let structure = StructArray::from_columns(
            Shape::new([1, 1]).unwrap(),
            fields,
            vec![vec![first.clone()], vec![second.clone()]],
        )
        .unwrap();
        let outputs = invoke(struct2table_builtin, &[Value::Struct(structure)]).unwrap();
        let table = output_table(&outputs);
        assert_eq!(table.shape().dimensions(), &[2, 2]);
        assert_eq!(table.variable(0), Some(&first));
        assert_eq!(table.variable(1), Some(&second));

        let converted = invoke(table2struct_builtin, &outputs).unwrap();
        let Value::Struct(structure) = &converted[0] else {
            panic!("table2struct must return a struct");
        };
        assert_eq!(structure.shape().dimensions(), &[2, 1]);
        assert_eq!(structure.value_at_name("A", 0), Some(&Value::Double(1.0)));
        assert_eq!(structure.value_at_name("A", 1), Some(&Value::Double(2.0)));
        assert!(matches!(
            structure.value_at_name("B", 0),
            Some(Value::Array(ArrayData::F64(array)))
                if array.shape().dimensions() == [1, 2] && array.as_slice() == [3.0, 5.0]
        ));

        let converted = invoke(
            table2struct_builtin,
            &[
                outputs[0].clone(),
                Value::from("ToScalar"),
                Value::Logical(true),
            ],
        )
        .unwrap();
        let Value::Struct(structure) = &converted[0] else {
            panic!("ToScalar table2struct must return a struct");
        };
        assert_eq!(structure.shape().dimensions(), &[1, 1]);
        assert_eq!(structure.value_at_name("A", 0), Some(&first));
        assert_eq!(structure.value_at_name("B", 0), Some(&second));
    }

    #[test]
    fn table2struct_rejects_names_outside_current_struct_schema() {
        let table = table_value(&["not a field"], vec![f64_array([1, 1], &[1.0])]);
        let error = invoke(table2struct_builtin, &[table]).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn table2struct_preserves_empty_default_and_to_scalar_shapes() {
        let empty = Value::Table(TableArray::empty(0).unwrap());
        let outputs = invoke(table2struct_builtin, std::slice::from_ref(&empty)).unwrap();
        let Value::Struct(structure) = &outputs[0] else {
            panic!("table2struct must return a struct");
        };
        assert_eq!(structure.shape().dimensions(), &[0, 1]);
        assert_eq!(structure.field_count(), 0);

        let outputs = invoke(
            table2struct_builtin,
            &[empty, Value::from("ToScalar"), Value::Logical(true)],
        )
        .unwrap();
        let Value::Struct(structure) = &outputs[0] else {
            panic!("ToScalar table2struct must return a struct");
        };
        assert_eq!(structure.shape().dimensions(), &[1, 1]);
        assert_eq!(structure.field_count(), 0);
    }

    #[test]
    fn struct_array_conversion_combines_numeric_rows_and_keeps_text_as_cells() {
        let structure = StructArray::from_columns(
            Shape::new([2, 1]).unwrap(),
            vec![FieldName::new("A").unwrap(), FieldName::new("B").unwrap()],
            vec![
                vec![Value::Double(1.0), Value::Double(2.0)],
                vec![char_row("a"), char_row("b")],
            ],
        )
        .unwrap();
        let source = Value::Struct(structure.clone());
        let outputs = invoke(struct2table_builtin, std::slice::from_ref(&source)).unwrap();
        let table = output_table(&outputs);
        assert_eq!(table.shape().dimensions(), &[2, 2]);
        assert!(matches!(
            table.variable(0),
            Some(Value::Array(ArrayData::F64(array))) if array.as_slice() == [1.0, 2.0]
        ));
        assert!(matches!(
            table.variable(1),
            Some(Value::Cell(cell)) if cell.shape().dimensions() == [2, 1]
        ));

        let outputs = invoke(
            struct2table_builtin,
            &[source, Value::from("AsArray"), Value::Logical(true)],
        )
        .unwrap();
        let table = output_table(&outputs);
        assert_eq!(table.shape().dimensions(), &[2, 2]);
        assert!(matches!(table.variable(0), Some(Value::Cell(_))));
        assert!(matches!(table.variable(1), Some(Value::Cell(_))));
    }

    #[test]
    fn table2array_concatenates_same_class_and_never_widens_classes() {
        let complex = Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                Shape::new([2, 1]).unwrap(),
                vec![ArrayComplex64::new(3.0, 4.0), ArrayComplex64::new(5.0, 0.0)],
            )
            .unwrap(),
        ));
        let table = table_value(&["A", "B"], vec![f64_array([2, 1], &[1.0, 2.0]), complex]);
        let outputs = invoke(table2array_builtin, &[table]).unwrap();
        let Value::Array(ArrayData::ComplexF64(array)) = &outputs[0] else {
            panic!("same-class real and complex doubles must concatenate losslessly");
        };
        assert_eq!(array.shape().dimensions(), &[2, 2]);
        assert_eq!(array.as_slice()[0], ArrayComplex64::new(1.0, 0.0));
        assert_eq!(array.as_slice()[2], ArrayComplex64::new(3.0, 4.0));

        let single = Value::Array(ArrayData::F32(
            DenseArray::from_vec(Shape::new([2, 1]).unwrap(), vec![1.0_f32, 2.0]).unwrap(),
        ));
        let mixed = table_value(&["A", "B"], vec![f64_array([2, 1], &[1.0, 2.0]), single]);
        assert_eq!(
            invoke(table2array_builtin, &[mixed]).unwrap_err().category,
            BuiltinErrorCategory::Domain
        );
    }

    #[test]
    fn table2array_concatenates_cell_variables_in_column_major_order() {
        let left = Value::Cell(
            CellArray::from_values(
                Shape::new([2, 1]).unwrap(),
                vec![char_row("a"), char_row("b")],
            )
            .unwrap(),
        );
        let right = Value::Cell(
            CellArray::from_values(
                Shape::new([2, 1]).unwrap(),
                vec![char_row("c"), char_row("d")],
            )
            .unwrap(),
        );
        let table = table_value(&["A", "B"], vec![left, right]);
        let outputs = invoke(table2array_builtin, &[table]).unwrap();
        let Value::Cell(cell) = &outputs[0] else {
            panic!("homogeneous cell variables must concatenate to a cell array");
        };
        assert_eq!(cell.shape().dimensions(), &[2, 2]);
        assert_eq!(cell.value_at_offset(0), Some(&char_row("a")));
        assert_eq!(cell.value_at_offset(1), Some(&char_row("b")));
        assert_eq!(cell.value_at_offset(2), Some(&char_row("c")));
        assert_eq!(cell.value_at_offset(3), Some(&char_row("d")));
    }

    #[test]
    fn table2cell_has_table_shape_and_keeps_multi_column_row_payloads() {
        let table = table_value(
            &["A", "B"],
            vec![
                f64_array([2, 1], &[1.0, 2.0]),
                f64_array([2, 2], &[3.0, 4.0, 5.0, 6.0]),
            ],
        );
        let outputs = invoke(table2cell_builtin, &[table]).unwrap();
        let Value::Cell(cell) = &outputs[0] else {
            panic!("table2cell must return a cell array");
        };
        assert_eq!(cell.shape().dimensions(), &[2, 2]);
        assert_eq!(cell.values()[0], Value::Double(1.0));
        assert_eq!(cell.values()[1], Value::Double(2.0));
        let Value::Array(ArrayData::F64(first_row)) = &cell.values()[2] else {
            panic!("multi-column table variable must stay grouped by table cell");
        };
        let Value::Array(ArrayData::F64(second_row)) = &cell.values()[3] else {
            panic!("multi-column table variable must stay grouped by table cell");
        };
        assert_eq!(first_row.shape().dimensions(), &[1, 2]);
        assert_eq!(first_row.as_slice(), &[3.0, 5.0]);
        assert_eq!(second_row.as_slice(), &[4.0, 6.0]);
    }

    #[test]
    fn dimensions_predicate_and_head_tail_follow_table_rows() {
        let values = (1..=10).map(f64::from).collect::<Vec<_>>();
        let table = table_value(&["A"], vec![f64_array([10, 1], &values)]);
        assert_eq!(
            invoke(height_builtin, std::slice::from_ref(&table)).unwrap(),
            vec![Value::Double(10.0)]
        );
        assert_eq!(
            invoke(width_builtin, std::slice::from_ref(&table)).unwrap(),
            vec![Value::Double(1.0)]
        );
        assert_eq!(
            invoke(istable_builtin, std::slice::from_ref(&table)).unwrap(),
            vec![Value::Logical(true)]
        );
        assert_eq!(
            invoke(istable_builtin, &[Value::Double(1.0)]).unwrap(),
            vec![Value::Logical(false)]
        );

        let head = invoke(head_builtin, std::slice::from_ref(&table)).unwrap();
        let tail = invoke(tail_builtin, std::slice::from_ref(&table)).unwrap();
        assert_eq!(output_table(&head).row_count(), 8);
        assert_eq!(output_table(&tail).row_count(), 8);
        let Value::Array(ArrayData::F64(head_values)) = output_table(&head).variable(0).unwrap()
        else {
            panic!("head must preserve double storage");
        };
        let Value::Array(ArrayData::F64(tail_values)) = output_table(&tail).variable(0).unwrap()
        else {
            panic!("tail must preserve double storage");
        };
        assert_eq!(
            head_values.as_slice(),
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
        );
        assert_eq!(
            tail_values.as_slice(),
            &[3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
        );

        let empty = invoke(head_builtin, &[table, Value::Double(0.0)]).unwrap();
        assert_eq!(output_table(&empty).shape().dimensions(), &[0, 1]);
        assert_eq!(
            invoke(
                head_builtin,
                &[
                    table_value(&["A"], vec![f64_array([1, 1], &[1.0])]),
                    Value::Double(-1.0)
                ]
            )
            .unwrap_err()
            .category,
            BuiltinErrorCategory::Domain
        );
    }

    #[test]
    fn variable_names_accept_cellstr_rows() {
        let names = CellArray::from_values(
            Shape::new([1, 2]).unwrap(),
            vec![char_row("first"), char_row("second")],
        )
        .unwrap();
        let outputs = invoke(
            array2table_builtin,
            &[
                f64_array([1, 2], &[1.0, 2.0]),
                Value::from("VariableNames"),
                Value::Cell(names),
            ],
        )
        .unwrap();
        assert_eq!(output_table(&outputs).variable_names()[0].as_str(), "first");
        assert_eq!(
            output_table(&outputs).variable_names()[1].as_str(),
            "second"
        );
    }

    fn char_row(value: &str) -> Value {
        let units = value
            .encode_utf16()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        let shape = Shape::new([1, units.len() as u64]).unwrap();
        Value::Array(ArrayData::Char(DenseArray::from_vec(shape, units).unwrap()))
    }

    #[test]
    fn table2array_preserves_string_elements_and_missing_state() {
        let left = StringArray::from_elements(
            Shape::new([2, 1]).unwrap(),
            vec![StringElement::from_utf8("a"), StringElement::missing()],
        )
        .unwrap();
        let right = StringArray::from_vec(
            Shape::new([2, 1]).unwrap(),
            vec![Arc::<str>::from("b"), Arc::<str>::from("c")],
        )
        .unwrap();
        let table = table_value(&["A", "B"], vec![Value::from(left), Value::from(right)]);
        let outputs = invoke(table2array_builtin, &[table]).unwrap();
        let Value::String(StringValue::Array(strings)) = &outputs[0] else {
            panic!("string table variables must remain string");
        };
        assert_eq!(strings.shape().dimensions(), &[2, 2]);
        assert!(strings.as_slice()[1].is_missing());
        assert_eq!(strings.as_slice()[2].to_utf8_lossy(), "b");
    }

    #[test]
    fn sort_and_missing_table_operations_preserve_rows_and_classes() {
        let names = StringArray::from_elements(
            Shape::new([4, 1]).unwrap(),
            vec![
                StringElement::from_utf8("Alice"),
                StringElement::from_utf8("Bob"),
                StringElement::missing(),
                StringElement::from_utf8("David"),
            ],
        )
        .unwrap();
        let table = table_value(
            &["Name", "Score"],
            vec![
                Value::from(names),
                f64_array([4, 1], &[88.0, f64::NAN, 79.0, 95.0]),
            ],
        );
        let sorted = invoke(
            sortrows_builtin,
            &[table.clone(), Value::from("Score"), Value::from("descend")],
        )
        .unwrap();
        let Value::Array(ArrayData::F64(scores)) = output_table(&sorted).variable(1).unwrap()
        else {
            panic!("sortrows must preserve double storage")
        };
        assert!(scores.as_slice()[0].is_nan());
        assert_eq!(scores.as_slice()[1..3], [95.0, 88.0]);

        let missing = invoke(ismissing_builtin, std::slice::from_ref(&table)).unwrap();
        let Value::Array(ArrayData::Logical(missing)) = &missing[0] else {
            panic!("ismissing(table) must return a logical matrix")
        };
        assert_eq!(
            missing
                .as_slice()
                .iter()
                .map(|value| value.get())
                .collect::<Vec<_>>(),
            [false, false, true, false, false, true, false, false]
        );

        let removed = invoke(rmmissing_builtin, std::slice::from_ref(&table)).unwrap();
        assert_eq!(output_table(&removed).row_count(), 2);
        let filled = invoke(
            fillmissing_builtin,
            &[
                table,
                Value::from("constant"),
                Value::Double(0.0),
                Value::from("DataVariables"),
                Value::from("Score"),
            ],
        )
        .unwrap();
        let Value::Array(ArrayData::F64(scores)) = output_table(&filled).variable(1).unwrap()
        else {
            panic!("fillmissing must preserve double storage")
        };
        assert_eq!(scores.as_slice(), &[88.0, 0.0, 79.0, 95.0]);
    }

    #[test]
    fn joins_merge_numeric_keys_and_supply_missing_unmatched_rows() {
        let people = Value::Cell(
            CellArray::from_values(
                Shape::new([3, 1]).unwrap(),
                ["Carol", "Alice", "Bob"]
                    .into_iter()
                    .map(char_row)
                    .collect(),
            )
            .unwrap(),
        );
        let left = table_value(
            &["ID", "Name"],
            vec![f64_array([3, 1], &[3.0, 1.0, 2.0]), people],
        );
        let right = table_value(
            &["ID", "Score"],
            vec![
                f64_array([3, 1], &[4.0, 2.0, 1.0]),
                f64_array([3, 1], &[77.0, 85.0, 90.0]),
            ],
        );
        let inner = invoke(
            innerjoin_builtin,
            &[
                left.clone(),
                right.clone(),
                Value::from("Keys"),
                Value::from("ID"),
            ],
        )
        .unwrap();
        let inner = output_table(&inner);
        assert_eq!(inner.shape().dimensions(), &[2, 3]);
        let Value::Array(ArrayData::F64(ids)) = inner.variable(0).unwrap() else {
            panic!("inner keys must remain double")
        };
        assert_eq!(ids.as_slice(), &[1.0, 2.0]);

        let outer = invoke(
            outerjoin_builtin,
            &[
                left,
                right,
                Value::from("Keys"),
                Value::from("ID"),
                Value::from("MergeKeys"),
                Value::Logical(true),
            ],
        )
        .unwrap();
        let table = output_table(&outer);
        assert_eq!(table.shape().dimensions(), &[4, 3]);
        let Value::Array(ArrayData::F64(ids)) = table.variable(0).unwrap() else {
            panic!("merged keys must remain double")
        };
        let Value::Array(ArrayData::F64(scores)) = table.variable(2).unwrap() else {
            panic!("right data must remain double")
        };
        assert_eq!(ids.as_slice(), &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(scores.as_slice()[..2], [90.0, 85.0]);
        assert!(scores.as_slice()[2].is_nan());
        assert_eq!(scores.as_slice()[3].to_bits(), 77.0_f64.to_bits());
    }

    #[test]
    fn grouping_outputs_match_r2022b_schema_and_values() {
        let groups = Value::Cell(
            CellArray::from_values(
                Shape::new([6, 1]).unwrap(),
                ["A", "A", "B", "B", "B", "C"]
                    .into_iter()
                    .map(char_row)
                    .collect(),
            )
            .unwrap(),
        );
        let table = table_value(
            &["Group", "Value"],
            vec![
                groups,
                f64_array([6, 1], &[10.0, 20.0, 5.0, 15.0, 25.0, 100.0]),
            ],
        );
        let counts = invoke(groupcounts_builtin, &[table.clone(), Value::from("Group")]).unwrap();
        let counts = output_table(&counts);
        assert_eq!(
            counts
                .variable_names()
                .iter()
                .map(TableVariableName::as_str)
                .collect::<Vec<_>>(),
            ["Group", "GroupCount", "Percent"]
        );
        let Value::Array(ArrayData::F64(values)) = counts.variable(1).unwrap() else {
            panic!("group counts must be double")
        };
        assert_eq!(values.as_slice(), &[2.0, 3.0, 1.0]);

        let summary = invoke(
            groupsummary_builtin,
            &[
                table,
                Value::from("Group"),
                Value::from("mean"),
                Value::from("Value"),
            ],
        )
        .unwrap();
        let summary = output_table(&summary);
        assert_eq!(summary.variable_names()[2].as_str(), "mean_Value");
        let Value::Array(ArrayData::F64(values)) = summary.variable(2).unwrap() else {
            panic!("group means must be double")
        };
        assert_eq!(values.as_slice(), &[15.0, 15.0, 100.0]);
    }
}

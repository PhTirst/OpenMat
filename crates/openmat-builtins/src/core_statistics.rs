use std::cmp::Ordering;

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData, IntegerComponent, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{CellArray, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, aggregate_error, array_error, exact_real_integer_scalar,
    expect_argument_count_range, expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MissingPolicy {
    Include,
    Omit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MeanOutput {
    Default,
    Double,
    Native,
}

#[derive(Clone, Copy)]
enum Dispersion {
    StandardDeviation,
    Variance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExtremaOutput {
    Range,
    Bounds,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CovarianceRows {
    Include,
    Complete,
    Pairwise,
}

struct PreparedMatrix64 {
    rows: usize,
    columns: usize,
    values: Vec<ArrayComplex64>,
    complex: bool,
}

struct PreparedMatrix32 {
    rows: usize,
    columns: usize,
    values: Vec<ArrayComplex32>,
    complex: bool,
}

pub(super) struct ReductionPlan {
    pub(super) input_dimensions: Vec<u64>,
    pub(super) selected: Vec<u64>,
    pub(super) output_shape: Shape,
    pub(super) output_length: usize,
}

#[derive(Clone, Copy, Default)]
struct Acc64 {
    re: f64,
    im: f64,
    count: u64,
}

#[derive(Clone, Copy, Default)]
struct Acc32 {
    re: f32,
    im: f32,
    count: u64,
}

pub(super) fn mean_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("mean", arguments, 1, 4)?;
    expect_max_outputs("mean", context, 1)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions("mean", &arguments[0])?;
    let (selection, missing, output) = parse_mean_options(arguments, context)?;
    let plan = reduction_plan("mean", dimensions, selection, context)?;
    let value = match (&arguments[0], output) {
        (Value::Array(ArrayData::Integer(array)), MeanOutput::Native) => {
            mean_native_integer(array, &plan, context)?
        }
        (Value::Logical(_) | Value::Array(ArrayData::Logical(_)), MeanOutput::Native) => {
            mean_native_logical(&arguments[0], &plan, context)?
        }
        (Value::Array(ArrayData::Char(_)), MeanOutput::Native) => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`mean` cannot use native accumulation for char input",
            ));
        }
        (
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_)),
            MeanOutput::Default | MeanOutput::Native,
        ) => mean_single(&arguments[0], &plan, missing, context)?,
        _ => mean_double(&arguments[0], &plan, missing, context)?,
    };
    context.check_cancelled()?;
    Ok(vec![value])
}

pub(super) fn median_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("median", arguments, 1, 3)?;
    expect_max_outputs("median", context, 1)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions("median", &arguments[0])?;
    let (selection, missing) = parse_reduction_options("median", &arguments[1..], 2, context)?;
    let plan = reduction_plan("median", dimensions, selection, context)?;
    let value = match &arguments[0] {
        Value::Array(ArrayData::Integer(array)) => median_integer(array, &plan, context)?,
        Value::Logical(_) | Value::Array(ArrayData::Logical(_)) => {
            median_logical(&arguments[0], &plan, context)?
        }
        Value::Array(ArrayData::Char(array)) => median_char(array, &plan, context)?,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_)) => {
            median_single(&arguments[0], &plan, missing, context)?
        }
        _ => median_double(&arguments[0], &plan, missing, context)?,
    };
    context.check_cancelled()?;
    Ok(vec![value])
}

pub(super) fn std_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    dispersion_builtin(Dispersion::StandardDeviation, arguments, context)
}

pub(super) fn var_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    dispersion_builtin(Dispersion::Variance, arguments, context)
}

pub(super) fn mode_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("mode", arguments, 1, 2)?;
    expect_max_outputs("mode", context, 3)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions("mode", &arguments[0])?;
    let selection = match arguments.get(1) {
        None => None,
        Some(value) if keyword(value).as_deref() == Some("all") => Some(Vec::new()),
        Some(value) => Some(dimension_values("mode", 2, value, context)?),
    };
    let plan = reduction_plan("mode", dimensions, selection, context)?;
    let (mode, frequency, ties) = match &arguments[0] {
        Value::Array(ArrayData::Integer(array)) => mode_integer(array, &plan, context)?,
        Value::Logical(_) | Value::Array(ArrayData::Logical(_)) => {
            mode_logical(&arguments[0], &plan, context)?
        }
        Value::Array(ArrayData::Char(array)) => mode_char(array, &plan, context)?,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_)) => {
            mode_single(&arguments[0], &plan, context)?
        }
        _ => mode_double(&arguments[0], &plan, context)?,
    };
    let mut outputs = vec![mode];
    if context.requested_outputs() > 1 {
        outputs.push(frequency);
    }
    if context.requested_outputs() > 2 {
        outputs.push(ties);
    }
    context.check_cancelled()?;
    Ok(outputs)
}

pub(super) fn range_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    extrema_statistics_builtin(ExtremaOutput::Range, arguments, context)
}

pub(super) fn bounds_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    extrema_statistics_builtin(ExtremaOutput::Bounds, arguments, context)
}

pub(super) fn cov_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("cov", arguments, 1, 4)?;
    expect_max_outputs("cov", context, 1)?;
    context.check_cancelled()?;
    let options = parse_covariance_arguments(arguments, context)?;
    let single = is_single_input(&arguments[0]) || options.second.is_some_and(is_single_input);
    let output = if single {
        let matrix = prepare_matrix32("cov", &arguments[0], options.second, context)?;
        covariance32(&matrix, options.rows, options.normalized, context)?
    } else {
        let matrix = prepare_matrix64("cov", &arguments[0], options.second, context)?;
        covariance64(&matrix, options.rows, options.normalized, context)?
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn corrcoef_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("corrcoef", arguments, 1, 6)?;
    expect_max_outputs("corrcoef", context, 4)?;
    context.check_cancelled()?;
    let options = parse_corrcoef_arguments(arguments, context)?;
    let single = is_single_input(&arguments[0]) || options.second.is_some_and(is_single_input);
    let outputs = if single {
        let matrix = prepare_matrix32("corrcoef", &arguments[0], options.second, context)?;
        correlation32(
            &matrix,
            options.rows,
            options.alpha,
            context.requested_outputs(),
            context,
        )?
    } else {
        let matrix = prepare_matrix64("corrcoef", &arguments[0], options.second, context)?;
        correlation64(
            &matrix,
            options.rows,
            options.alpha,
            context.requested_outputs(),
            context,
        )?
    };
    context.check_cancelled()?;
    Ok(outputs)
}

struct ModeSummary<T> {
    modes: Vec<T>,
    frequencies: Vec<f64>,
    ties: Vec<Vec<T>>,
}

#[allow(clippy::cast_precision_loss)]
fn summarize_modes<T: Copy>(
    mut groups: Vec<Vec<T>>,
    empty: T,
    context: &BuiltinContext<'_>,
    mut compare: impl FnMut(&T, &T) -> Ordering,
    mut equal: impl FnMut(T, T) -> bool,
) -> Result<ModeSummary<T>, BuiltinError> {
    let mut modes = reserved_vec("mode", groups.len())?;
    let mut frequencies = reserved_vec("mode", groups.len())?;
    let mut ties = reserved_vec("mode", groups.len())?;
    for (group_index, group) in groups.iter_mut().enumerate() {
        check_cancelled_at(context, group_index)?;
        stable_sort_by("mode", group, context, &mut compare)?;
        if group.is_empty() {
            modes.push(empty);
            frequencies.push(0.0);
            ties.push(Vec::new());
            continue;
        }
        let mut best = 0_usize;
        let mut group_ties = Vec::new();
        let mut start = 0_usize;
        while start < group.len() {
            let mut end = start + 1;
            while end < group.len() && equal(group[start], group[end]) {
                end += 1;
            }
            let count = end - start;
            if count > best {
                best = count;
                group_ties.clear();
                group_ties.push(group[start]);
            } else if count == best {
                group_ties.push(group[start]);
            }
            start = end;
        }
        modes.push(group_ties[0]);
        frequencies.push(best as f64);
        ties.push(group_ties);
    }
    Ok(ModeSummary {
        modes,
        frequencies,
        ties,
    })
}

fn tie_shape(length: usize) -> Result<Shape, BuiltinError> {
    let length = u64::try_from(length).map_err(|_| mapping_error("mode"))?;
    Shape::new(if length == 0 { [1, 0] } else { [length, 1] }).map_err(|error| array_error(&error))
}

fn mode_outputs(
    plan: &ReductionPlan,
    mode: Value,
    frequencies: Vec<f64>,
    ties: Vec<Value>,
) -> Result<(Value, Value, Value), BuiltinError> {
    let frequency = real_f64_output(plan.output_shape.clone(), frequencies)?;
    let ties = CellArray::from_values(plan.output_shape.clone(), ties)
        .map(Value::Cell)
        .map_err(|error| aggregate_error("mode", &error))?;
    Ok((mode, frequency, ties))
}

#[allow(clippy::float_cmp)]
fn mode_double(
    input: &Value,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value, Value), BuiltinError> {
    let complex_order = input.is_complex_numeric();
    let mut groups = complex64_groups("mode", plan, context)?;
    visit_f64(input, "mode", context, |index, value| {
        groups[output_offset("mode", index, plan)?].push(value);
        Ok(())
    })?;
    let summary = summarize_modes(
        groups,
        ArrayComplex64::new(f64::NAN, 0.0),
        context,
        |left, right| compare_complex64(*left, *right, complex_order),
        |left, right| left.re == right.re && left.im == right.im,
    )?;
    let mode = complex_f64_output(plan.output_shape.clone(), summary.modes)?;
    let mut ties = reserved_vec("mode", summary.ties.len())?;
    for values in summary.ties {
        ties.push(complex_f64_output(tie_shape(values.len())?, values)?);
    }
    mode_outputs(plan, mode, summary.frequencies, ties)
}

#[allow(clippy::float_cmp)]
fn mode_single(
    input: &Value,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value, Value), BuiltinError> {
    let complex_order = input.is_complex_numeric();
    let mut groups = complex32_groups("mode", plan, context)?;
    visit_f32(input, "mode", context, |index, value| {
        groups[output_offset("mode", index, plan)?].push(value);
        Ok(())
    })?;
    let summary = summarize_modes(
        groups,
        ArrayComplex32::new(f32::NAN, 0.0),
        context,
        |left, right| compare_complex32(*left, *right, complex_order),
        |left, right| left.re == right.re && left.im == right.im,
    )?;
    let mode =
        crate::core_elementary::complex_f32_array_output(plan.output_shape.clone(), summary.modes)?;
    let mut ties = reserved_vec("mode", summary.ties.len())?;
    for values in summary.ties {
        ties.push(crate::core_elementary::complex_f32_array_output(
            tie_shape(values.len())?,
            values,
        )?);
    }
    mode_outputs(plan, mode, summary.frequencies, ties)
}

fn mode_logical(
    input: &Value,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value, Value), BuiltinError> {
    let mut groups: Vec<Vec<bool>> = typed_groups("mode", plan, context)?;
    visit_logical(input, "mode", context, |index, value| {
        groups[output_offset("mode", index, plan)?].push(value);
        Ok(())
    })?;
    let summary = summarize_modes(groups, false, context, Ord::cmp, |left, right| {
        left == right
    })?;
    let logical_value = |shape: Shape, values: Vec<bool>| {
        DenseArray::from_vec(shape, values.into_iter().map(Logical::from).collect())
            .map(ArrayData::Logical)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    };
    let mode = logical_value(plan.output_shape.clone(), summary.modes)?;
    let mut ties = reserved_vec("mode", summary.ties.len())?;
    for values in summary.ties {
        ties.push(logical_value(tie_shape(values.len())?, values)?);
    }
    mode_outputs(plan, mode, summary.frequencies, ties)
}

fn mode_char(
    input: &DenseArray<CharCodeUnit>,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value, Value), BuiltinError> {
    if input.numel() == 0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`mode` does not define a value for an empty char array",
        ));
    }
    let mut groups: Vec<Vec<u16>> = typed_groups("mode", plan, context)?;
    for (index, value) in input.as_slice().iter().enumerate() {
        groups[output_offset("mode", index, plan)?].push(value.get());
    }
    let summary = summarize_modes(groups, 0, context, Ord::cmp, |left, right| left == right)?;
    let char_value = |shape: Shape, values: Vec<u16>| {
        DenseArray::from_vec(shape, values.into_iter().map(CharCodeUnit::new).collect())
            .map(ArrayData::Char)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    };
    let mode = char_value(plan.output_shape.clone(), summary.modes)?;
    let mut ties = reserved_vec("mode", summary.ties.len())?;
    for values in summary.ties {
        ties.push(char_value(tie_shape(values.len())?, values)?);
    }
    mode_outputs(plan, mode, summary.frequencies, ties)
}

macro_rules! mode_integer_variant {
    ($array:expr, $plan:expr, $context:expr) => {{
        let mut groups = typed_groups("mode", $plan, $context)?;
        for (index, value) in $array.as_slice().iter().copied().enumerate() {
            groups[output_offset("mode", index, $plan)?].push(value);
        }
        let summary = summarize_modes(groups, 0, $context, Ord::cmp, |left, right| left == right)?;
        let integer_value = |shape: Shape, values| {
            DenseArray::from_vec(shape, values)
                .map(IntegerArrayData::from_typed)
                .map(ArrayData::Integer)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        };
        let mode = integer_value($plan.output_shape.clone(), summary.modes)?;
        let mut ties = reserved_vec("mode", summary.ties.len())?;
        for values in summary.ties {
            ties.push(integer_value(tie_shape(values.len())?, values)?);
        }
        mode_outputs($plan, mode, summary.frequencies, ties)
    }};
}

fn mode_integer(
    input: &IntegerArrayData,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value, Value), BuiltinError> {
    match input {
        IntegerArrayData::I8(array) => mode_integer_variant!(array, plan, context),
        IntegerArrayData::U8(array) => mode_integer_variant!(array, plan, context),
        IntegerArrayData::I16(array) => mode_integer_variant!(array, plan, context),
        IntegerArrayData::U16(array) => mode_integer_variant!(array, plan, context),
        IntegerArrayData::I32(array) => mode_integer_variant!(array, plan, context),
        IntegerArrayData::U32(array) => mode_integer_variant!(array, plan, context),
        IntegerArrayData::I64(array) => mode_integer_variant!(array, plan, context),
        IntegerArrayData::U64(array) => mode_integer_variant!(array, plan, context),
        _ => Err(type_error(
            "mode",
            1,
            "real integer array",
            &Value::Array(ArrayData::Integer(input.clone())),
        )),
    }
}

struct ExtremaSummary<T> {
    minima: Vec<T>,
    maxima: Vec<T>,
}

fn summarize_extrema<T: Copy>(
    groups: Vec<Vec<T>>,
    missing: MissingPolicy,
    missing_value: T,
    context: &BuiltinContext<'_>,
    mut is_missing: impl FnMut(T) -> bool,
    mut compare: impl FnMut(T, T) -> Ordering,
) -> Result<ExtremaSummary<T>, BuiltinError> {
    let mut minima = reserved_vec("bounds", groups.len())?;
    let mut maxima = reserved_vec("bounds", groups.len())?;
    for (group_index, group) in groups.into_iter().enumerate() {
        check_cancelled_at(context, group_index)?;
        let mut minimum = None;
        let mut maximum = None;
        let mut included_missing = false;
        for value in group {
            if is_missing(value) {
                if missing == MissingPolicy::Include {
                    included_missing = true;
                    break;
                }
                continue;
            }
            if minimum.is_none_or(|current| compare(value, current) == Ordering::Less) {
                minimum = Some(value);
            }
            if maximum.is_none_or(|current| compare(value, current) == Ordering::Greater) {
                maximum = Some(value);
            }
        }
        if included_missing {
            minima.push(missing_value);
            maxima.push(missing_value);
        } else {
            minima.push(minimum.unwrap_or(missing_value));
            maxima.push(maximum.unwrap_or(missing_value));
        }
    }
    Ok(ExtremaSummary { minima, maxima })
}

fn extrema_statistics_builtin(
    operation: ExtremaOutput,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = match operation {
        ExtremaOutput::Range => "range",
        ExtremaOutput::Bounds => "bounds",
    };
    expect_argument_count_range(name, arguments, 1, 3)?;
    expect_max_outputs(
        name,
        context,
        if operation == ExtremaOutput::Range {
            1
        } else {
            2
        },
    )?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions(name, &arguments[0])?;
    let (selection, missing) = parse_extrema_statistics_options(name, &arguments[1..], context)?;
    let plan = extrema_statistics_plan(name, dimensions, selection, context)?;
    let mut outputs = match &arguments[0] {
        Value::Array(ArrayData::Integer(array)) => {
            extrema_integer(operation, array, &plan, context)?
        }
        Value::Logical(_) | Value::Array(ArrayData::Logical(_)) => {
            extrema_logical(operation, &arguments[0], &plan, context)?
        }
        Value::Array(ArrayData::Char(array)) => extrema_char(operation, array, &plan, context)?,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_)) => {
            extrema_single(operation, &arguments[0], &plan, missing, context)?
        }
        _ => extrema_double(operation, &arguments[0], &plan, missing, context)?,
    };
    if operation == ExtremaOutput::Bounds && context.requested_outputs() < 2 {
        outputs.truncate(1);
    }
    context.check_cancelled()?;
    Ok(outputs)
}

fn parse_extrema_statistics_options(
    name: &str,
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<(Option<Vec<u64>>, MissingPolicy), BuiltinError> {
    let mut selection = None;
    let mut missing = MissingPolicy::Omit;
    for (offset, argument) in arguments.iter().enumerate() {
        check_cancelled_at(context, offset)?;
        match keyword(argument).as_deref() {
            Some("all") => set_selection(name, &mut selection, Vec::new())?,
            Some("omitnan") => missing = MissingPolicy::Omit,
            Some("includenan") => missing = MissingPolicy::Include,
            Some(_) => return Err(option_error(name)),
            None => {
                let dimensions = dimension_values(name, offset + 2, argument, context)?;
                set_selection(name, &mut selection, dimensions)?;
            }
        }
    }
    Ok((selection, missing))
}

fn extrema_statistics_plan(
    name: &str,
    dimensions: &[u64],
    selection: Option<Vec<u64>>,
    context: &BuiltinContext<'_>,
) -> Result<ReductionPlan, BuiltinError> {
    let mut selected = match selection {
        Some(selected) if selected.is_empty() => all_dimensions_from_slice(dimensions)?,
        Some(selected) => selected,
        None => vec![default_dimension(dimensions)],
    };
    selected.sort_unstable();
    let mut output_dimensions = reserved_vec(name, dimensions.len())?;
    for (index, extent) in dimensions.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let dimension = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| mapping_error(name))?;
        output_dimensions.push(if selected.binary_search(&dimension).is_ok() {
            u64::from(extent != 0)
        } else {
            extent
        });
    }
    let output_shape = Shape::new(output_dimensions).map_err(|error| array_error(&error))?;
    let output_length = usize::try_from(output_shape.numel()).map_err(|_| mapping_error(name))?;
    Ok(ReductionPlan {
        input_dimensions: dimensions.to_vec(),
        selected,
        output_shape,
        output_length,
    })
}

fn extrema_double(
    operation: ExtremaOutput,
    input: &Value,
    plan: &ReductionPlan,
    missing: MissingPolicy,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    let complex_order = input.is_complex_numeric();
    let mut groups = complex64_groups("bounds", plan, context)?;
    visit_f64(input, "bounds", context, |index, value| {
        groups[output_offset("bounds", index, plan)?].push(value);
        Ok(())
    })?;
    let summary = summarize_extrema(
        groups,
        missing,
        ArrayComplex64::new(f64::NAN, 0.0),
        context,
        |value| value.re.is_nan() || value.im.is_nan(),
        |left, right| compare_complex64(left, right, complex_order),
    )?;
    match operation {
        ExtremaOutput::Range => {
            let values = summary
                .maxima
                .into_iter()
                .zip(summary.minima)
                .map(|(maximum, minimum)| {
                    ArrayComplex64::new(maximum.re - minimum.re, maximum.im - minimum.im)
                })
                .collect();
            Ok(vec![complex_f64_output(plan.output_shape.clone(), values)?])
        }
        ExtremaOutput::Bounds => Ok(vec![
            complex_f64_output(plan.output_shape.clone(), summary.minima)?,
            complex_f64_output(plan.output_shape.clone(), summary.maxima)?,
        ]),
    }
}

fn extrema_single(
    operation: ExtremaOutput,
    input: &Value,
    plan: &ReductionPlan,
    missing: MissingPolicy,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    let complex_order = input.is_complex_numeric();
    let mut groups = complex32_groups("bounds", plan, context)?;
    visit_f32(input, "bounds", context, |index, value| {
        groups[output_offset("bounds", index, plan)?].push(value);
        Ok(())
    })?;
    let summary = summarize_extrema(
        groups,
        missing,
        ArrayComplex32::new(f32::NAN, 0.0),
        context,
        |value| value.re.is_nan() || value.im.is_nan(),
        |left, right| compare_complex32(left, right, complex_order),
    )?;
    let output = |values| {
        crate::core_elementary::complex_f32_array_output(plan.output_shape.clone(), values)
    };
    match operation {
        ExtremaOutput::Range => {
            let values = summary
                .maxima
                .into_iter()
                .zip(summary.minima)
                .map(|(maximum, minimum)| {
                    ArrayComplex32::new(maximum.re - minimum.re, maximum.im - minimum.im)
                })
                .collect();
            Ok(vec![output(values)?])
        }
        ExtremaOutput::Bounds => Ok(vec![output(summary.minima)?, output(summary.maxima)?]),
    }
}

fn logical_array_value(shape: Shape, values: Vec<bool>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values.into_iter().map(Logical::from).collect())
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn extrema_logical(
    operation: ExtremaOutput,
    input: &Value,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    let mut groups: Vec<Vec<bool>> = typed_groups("bounds", plan, context)?;
    visit_logical(input, "bounds", context, |index, value| {
        groups[output_offset("bounds", index, plan)?].push(value);
        Ok(())
    })?;
    let summary = summarize_extrema(
        groups,
        MissingPolicy::Omit,
        false,
        context,
        |_| false,
        |left, right| left.cmp(&right),
    )?;
    match operation {
        ExtremaOutput::Range => Ok(vec![real_f64_output(
            plan.output_shape.clone(),
            summary
                .maxima
                .into_iter()
                .zip(summary.minima)
                .map(|(maximum, minimum)| f64::from(maximum) - f64::from(minimum))
                .collect(),
        )?]),
        ExtremaOutput::Bounds => Ok(vec![
            logical_array_value(plan.output_shape.clone(), summary.minima)?,
            logical_array_value(plan.output_shape.clone(), summary.maxima)?,
        ]),
    }
}

fn extrema_char(
    operation: ExtremaOutput,
    input: &DenseArray<CharCodeUnit>,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    let mut groups: Vec<Vec<u16>> = typed_groups("bounds", plan, context)?;
    for (index, value) in input.as_slice().iter().enumerate() {
        groups[output_offset("bounds", index, plan)?].push(value.get());
    }
    let summary = summarize_extrema(
        groups,
        MissingPolicy::Omit,
        0,
        context,
        |_| false,
        |left, right| left.cmp(&right),
    )?;
    match operation {
        ExtremaOutput::Range => Ok(vec![real_f64_output(
            plan.output_shape.clone(),
            summary
                .maxima
                .into_iter()
                .zip(summary.minima)
                .map(|(maximum, minimum)| f64::from(maximum) - f64::from(minimum))
                .collect(),
        )?]),
        ExtremaOutput::Bounds => Ok(vec![
            real_f64_output(
                plan.output_shape.clone(),
                summary.minima.into_iter().map(f64::from).collect(),
            )?,
            real_f64_output(
                plan.output_shape.clone(),
                summary.maxima.into_iter().map(f64::from).collect(),
            )?,
        ]),
    }
}

macro_rules! extrema_integer_variant {
    ($operation:expr, $array:expr, $plan:expr, $context:expr) => {{
        let mut groups = typed_groups("bounds", $plan, $context)?;
        for (index, value) in $array.as_slice().iter().copied().enumerate() {
            groups[output_offset("bounds", index, $plan)?].push(value);
        }
        let summary = summarize_extrema(
            groups,
            MissingPolicy::Omit,
            0,
            $context,
            |_| false,
            |left, right| left.cmp(&right),
        )?;
        let integer_value = |values| {
            DenseArray::from_vec($plan.output_shape.clone(), values)
                .map(IntegerArrayData::from_typed)
                .map(ArrayData::Integer)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        };
        match $operation {
            ExtremaOutput::Range => Ok(vec![integer_value(
                summary
                    .maxima
                    .into_iter()
                    .zip(summary.minima)
                    .map(|(maximum, minimum)| maximum.saturating_sub(minimum))
                    .collect(),
            )?]),
            ExtremaOutput::Bounds => Ok(vec![
                integer_value(summary.minima)?,
                integer_value(summary.maxima)?,
            ]),
        }
    }};
}

fn extrema_integer(
    operation: ExtremaOutput,
    input: &IntegerArrayData,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    match input {
        IntegerArrayData::I8(array) => extrema_integer_variant!(operation, array, plan, context),
        IntegerArrayData::U8(array) => extrema_integer_variant!(operation, array, plan, context),
        IntegerArrayData::I16(array) => extrema_integer_variant!(operation, array, plan, context),
        IntegerArrayData::U16(array) => extrema_integer_variant!(operation, array, plan, context),
        IntegerArrayData::I32(array) => extrema_integer_variant!(operation, array, plan, context),
        IntegerArrayData::U32(array) => extrema_integer_variant!(operation, array, plan, context),
        IntegerArrayData::I64(array) => extrema_integer_variant!(operation, array, plan, context),
        IntegerArrayData::U64(array) => extrema_integer_variant!(operation, array, plan, context),
        _ => Err(type_error(
            "bounds",
            1,
            "real integer array",
            &Value::Array(ArrayData::Integer(input.clone())),
        )),
    }
}

struct CovarianceArguments<'a> {
    second: Option<&'a Value>,
    normalized: bool,
    rows: CovarianceRows,
}

struct CorrcoefArguments<'a> {
    second: Option<&'a Value>,
    alpha: f64,
    rows: CovarianceRows,
}

fn parse_covariance_arguments<'a>(
    arguments: &'a [Value],
    context: &BuiltinContext<'_>,
) -> Result<CovarianceArguments<'a>, BuiltinError> {
    let mut second = None;
    let mut normalized = false;
    let mut rows = CovarianceRows::Include;
    let mut offset = 1_usize;
    if let Some(argument) = arguments.get(offset) {
        if keyword(argument).is_some() {
            rows = covariance_rows("cov", argument)?;
            offset += 1;
        } else if let Some(weight) = covariance_normalization_value(argument) {
            normalized = weight;
            offset += 1;
        } else {
            validate_covariance_input("cov", offset + 1, argument)?;
            second = Some(argument);
            offset += 1;
            if let Some(weight) = arguments.get(offset)
                && keyword(weight).is_none()
            {
                normalized = covariance_normalization(weight)?;
                offset += 1;
            }
        }
    }
    if let Some(argument) = arguments.get(offset) {
        rows = covariance_rows("cov", argument)?;
        offset += 1;
    }
    if offset != arguments.len() {
        return Err(option_error("cov"));
    }
    context.check_cancelled()?;
    Ok(CovarianceArguments {
        second,
        normalized,
        rows,
    })
}

fn parse_corrcoef_arguments<'a>(
    arguments: &'a [Value],
    context: &BuiltinContext<'_>,
) -> Result<CorrcoefArguments<'a>, BuiltinError> {
    let mut second = None;
    let mut offset = 1_usize;
    if let Some(argument) = arguments.get(offset) {
        match keyword(argument).as_deref() {
            Some("alpha" | "rows") => {}
            Some(_) => return Err(option_error("corrcoef")),
            None => {
                validate_covariance_input("corrcoef", offset + 1, argument)?;
                second = Some(argument);
                offset += 1;
            }
        }
    }
    let mut alpha = 0.05_f64;
    let mut rows = CovarianceRows::Include;
    let mut alpha_seen = false;
    let mut rows_seen = false;
    while offset < arguments.len() {
        context.check_cancelled()?;
        let name = keyword(&arguments[offset]).ok_or_else(|| option_error("corrcoef"))?;
        let value = arguments
            .get(offset + 1)
            .ok_or_else(|| option_error("corrcoef"))?;
        match name.as_str() {
            "alpha" if !alpha_seen => {
                alpha = value
                    .as_real_number()
                    .or_else(|| value.as_real_single().map(f64::from))
                    .filter(|value| value.is_finite() && *value > 0.0 && *value < 1.0)
                    .ok_or_else(|| {
                        BuiltinError::new(
                            BuiltinErrorCategory::Domain,
                            "`corrcoef` alpha must be a finite scalar strictly between zero and one",
                        )
                    })?;
                alpha_seen = true;
            }
            "rows" if !rows_seen => {
                rows = match keyword(value).as_deref() {
                    Some("all") => CovarianceRows::Include,
                    Some("complete") => CovarianceRows::Complete,
                    Some("pairwise") => CovarianceRows::Pairwise,
                    _ => return Err(option_error("corrcoef")),
                };
                rows_seen = true;
            }
            _ => return Err(option_error("corrcoef")),
        }
        offset += 2;
    }
    Ok(CorrcoefArguments {
        second,
        alpha,
        rows,
    })
}

fn covariance_rows(name: &str, value: &Value) -> Result<CovarianceRows, BuiltinError> {
    match keyword(value).as_deref() {
        Some("includenan") => Ok(CovarianceRows::Include),
        Some("omitrows") => Ok(CovarianceRows::Complete),
        Some("partialrows") => Ok(CovarianceRows::Pairwise),
        _ => Err(option_error(name)),
    }
}

fn covariance_normalization(value: &Value) -> Result<bool, BuiltinError> {
    covariance_normalization_value(value).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the normalization input to `cov` must be zero or one",
        )
    })
}

#[allow(clippy::float_cmp)]
fn covariance_normalization_value(value: &Value) -> Option<bool> {
    let value = value
        .as_real_number()
        .or_else(|| value.as_real_single().map(f64::from))
        .or_else(|| exact_real_integer_scalar(value).map(integer_component_f64))?;
    if value == 0.0 {
        Some(false)
    } else if value == 1.0 {
        Some(true)
    } else {
        None
    }
}

const fn is_single_input(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    )
}

fn validate_covariance_input(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<(), BuiltinError> {
    match value {
        Value::Logical(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::Array(
            ArrayData::F32(_)
            | ArrayData::ComplexF32(_)
            | ArrayData::Logical(_)
            | ArrayData::F64(_)
            | ArrayData::ComplexF64(_)
            | ArrayData::Char(_),
        ) => {
            let dimensions = value.dimensions().ok_or_else(|| {
                type_error(name, position, "two-dimensional numeric array", value)
            })?;
            if dimensions.len() > 2 {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input {position} to `{name}` must be two-dimensional"),
                ));
            }
            Ok(())
        }
        _ => Err(type_error(
            name,
            position,
            "double, single, logical, or char array",
            value,
        )),
    }
}

fn prepare_matrix64(
    name: &str,
    first: &Value,
    second: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> Result<PreparedMatrix64, BuiltinError> {
    validate_covariance_input(name, 1, first)?;
    let first_values = collect_f64(name, first, context)?;
    let complex = first.is_complex_numeric() || second.is_some_and(Value::is_complex_numeric);
    if let Some(second) = second {
        validate_covariance_input(name, 2, second)?;
        let second_values = collect_f64(name, second, context)?;
        if first_values.len() != second_values.len() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("inputs to `{name}` must contain the same number of elements"),
            ));
        }
        let mut values = reserved_vec(name, first_values.len().saturating_mul(2))?;
        values.extend(first_values);
        values.extend(second_values);
        let mut rows = values.len() / 2;
        let mut columns = 2;
        if rows == 1 && name == "corrcoef" {
            rows = 2;
            columns = 1;
        }
        Ok(PreparedMatrix64 {
            rows,
            columns,
            values,
            complex,
        })
    } else {
        let dimensions = first.dimensions().ok_or_else(|| mapping_error(name))?;
        let original_rows = usize::try_from(dimensions[0]).map_err(|_| mapping_error(name))?;
        let original_columns = usize::try_from(dimensions[1]).map_err(|_| mapping_error(name))?;
        let (rows, columns) = if original_rows == 1 || original_columns == 1 {
            (first_values.len(), 1)
        } else {
            (original_rows, original_columns)
        };
        Ok(PreparedMatrix64 {
            rows,
            columns,
            values: first_values,
            complex,
        })
    }
}

#[allow(clippy::cast_possible_truncation)]
fn prepare_matrix32(
    name: &str,
    first: &Value,
    second: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> Result<PreparedMatrix32, BuiltinError> {
    let matrix = prepare_matrix64(name, first, second, context)?;
    let mut values = reserved_vec(name, matrix.values.len())?;
    for (index, value) in matrix.values.into_iter().enumerate() {
        check_cancelled_at(context, index)?;
        values.push(ArrayComplex32::new(value.re as f32, value.im as f32));
    }
    Ok(PreparedMatrix32 {
        rows: matrix.rows,
        columns: matrix.columns,
        values,
        complex: matrix.complex,
    })
}

fn collect_f64(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<ArrayComplex64>, BuiltinError> {
    let length = value
        .numel()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| mapping_error(name))?;
    let mut values = reserved_vec(name, length)?;
    visit_f64(value, name, context, |_, value| {
        values.push(value);
        Ok(())
    })?;
    Ok(values)
}

fn square_shape(name: &str, columns: usize) -> Result<Shape, BuiltinError> {
    let columns = u64::try_from(columns).map_err(|_| mapping_error(name))?;
    Shape::new([columns, columns]).map_err(|error| array_error(&error))
}

fn complete_rows64(
    matrix: &PreparedMatrix64,
    context: &BuiltinContext<'_>,
) -> Result<Vec<bool>, BuiltinError> {
    let mut complete = filled_values("cov", matrix.rows, true, context)?;
    for column in 0..matrix.columns {
        for (row, slot) in complete.iter_mut().enumerate() {
            let value = matrix.values[row + column * matrix.rows];
            *slot &= !complex64_missing(value);
        }
    }
    Ok(complete)
}

fn complete_rows32(
    matrix: &PreparedMatrix32,
    context: &BuiltinContext<'_>,
) -> Result<Vec<bool>, BuiltinError> {
    let mut complete = filled_values("cov", matrix.rows, true, context)?;
    for column in 0..matrix.columns {
        for (row, slot) in complete.iter_mut().enumerate() {
            let value = matrix.values[row + column * matrix.rows];
            *slot &= !complex32_missing(value);
        }
    }
    Ok(complete)
}

const fn complex64_missing(value: ArrayComplex64) -> bool {
    value.re.is_nan() || value.im.is_nan()
}

const fn complex32_missing(value: ArrayComplex32) -> bool {
    value.re.is_nan() || value.im.is_nan()
}

#[allow(clippy::cast_precision_loss, clippy::needless_range_loop)]
fn covariance_pair64(
    matrix: &PreparedMatrix64,
    left: usize,
    right: usize,
    rows: CovarianceRows,
    complete: &[bool],
    normalized: bool,
    context: &BuiltinContext<'_>,
) -> Result<(ArrayComplex64, usize), BuiltinError> {
    let mut left_mean = ArrayComplex64::ZERO;
    let mut right_mean = ArrayComplex64::ZERO;
    let mut count = 0_usize;
    for row in 0..matrix.rows {
        check_cancelled_at(context, row)?;
        let left_value = matrix.values[row + left * matrix.rows];
        let right_value = matrix.values[row + right * matrix.rows];
        let include = match rows {
            CovarianceRows::Include => true,
            CovarianceRows::Complete => complete[row],
            CovarianceRows::Pairwise => {
                !complex64_missing(left_value) && !complex64_missing(right_value)
            }
        };
        if include {
            left_mean.re += left_value.re;
            left_mean.im += left_value.im;
            right_mean.re += right_value.re;
            right_mean.im += right_value.im;
            count += 1;
        }
    }
    if count == 0 {
        return Ok((ArrayComplex64::new(f64::NAN, 0.0), 0));
    }
    let divisor = count as f64;
    left_mean.re /= divisor;
    left_mean.im /= divisor;
    right_mean.re /= divisor;
    right_mean.im /= divisor;
    let mut sum = ArrayComplex64::ZERO;
    for row in 0..matrix.rows {
        let left_value = matrix.values[row + left * matrix.rows];
        let right_value = matrix.values[row + right * matrix.rows];
        let include = match rows {
            CovarianceRows::Include => true,
            CovarianceRows::Complete => complete[row],
            CovarianceRows::Pairwise => {
                !complex64_missing(left_value) && !complex64_missing(right_value)
            }
        };
        if include {
            let left_re = left_value.re - left_mean.re;
            let left_im = left_value.im - left_mean.im;
            let right_re = right_value.re - right_mean.re;
            let right_im = right_value.im - right_mean.im;
            sum.re += left_re * right_re + left_im * right_im;
            sum.im += left_re * right_im - left_im * right_re;
        }
    }
    let denominator = if normalized {
        count
    } else {
        count.saturating_sub(1).max(1)
    } as f64;
    Ok((
        ArrayComplex64::new(sum.re / denominator, sum.im / denominator),
        count,
    ))
}

#[allow(clippy::cast_precision_loss, clippy::needless_range_loop)]
fn covariance_pair32(
    matrix: &PreparedMatrix32,
    left: usize,
    right: usize,
    rows: CovarianceRows,
    complete: &[bool],
    normalized: bool,
    context: &BuiltinContext<'_>,
) -> Result<(ArrayComplex32, usize), BuiltinError> {
    let mut left_mean = ArrayComplex32::ZERO;
    let mut right_mean = ArrayComplex32::ZERO;
    let mut count = 0_usize;
    for row in 0..matrix.rows {
        check_cancelled_at(context, row)?;
        let left_value = matrix.values[row + left * matrix.rows];
        let right_value = matrix.values[row + right * matrix.rows];
        let include = match rows {
            CovarianceRows::Include => true,
            CovarianceRows::Complete => complete[row],
            CovarianceRows::Pairwise => {
                !complex32_missing(left_value) && !complex32_missing(right_value)
            }
        };
        if include {
            left_mean.re += left_value.re;
            left_mean.im += left_value.im;
            right_mean.re += right_value.re;
            right_mean.im += right_value.im;
            count += 1;
        }
    }
    if count == 0 {
        return Ok((ArrayComplex32::new(f32::NAN, 0.0), 0));
    }
    let divisor = count as f32;
    left_mean.re /= divisor;
    left_mean.im /= divisor;
    right_mean.re /= divisor;
    right_mean.im /= divisor;
    let mut sum = ArrayComplex32::ZERO;
    for row in 0..matrix.rows {
        let left_value = matrix.values[row + left * matrix.rows];
        let right_value = matrix.values[row + right * matrix.rows];
        let include = match rows {
            CovarianceRows::Include => true,
            CovarianceRows::Complete => complete[row],
            CovarianceRows::Pairwise => {
                !complex32_missing(left_value) && !complex32_missing(right_value)
            }
        };
        if include {
            let left_re = left_value.re - left_mean.re;
            let left_im = left_value.im - left_mean.im;
            let right_re = right_value.re - right_mean.re;
            let right_im = right_value.im - right_mean.im;
            sum.re += left_re * right_re + left_im * right_im;
            sum.im += left_re * right_im - left_im * right_re;
        }
    }
    let denominator = if normalized {
        count
    } else {
        count.saturating_sub(1).max(1)
    } as f32;
    Ok((
        ArrayComplex32::new(sum.re / denominator, sum.im / denominator),
        count,
    ))
}

fn covariance64(
    matrix: &PreparedMatrix64,
    rows: CovarianceRows,
    normalized: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let shape = square_shape("cov", matrix.columns)?;
    let complete = complete_rows64(matrix, context)?;
    let mut values = reserved_vec("cov", matrix.columns.saturating_mul(matrix.columns))?;
    for right in 0..matrix.columns {
        for left in 0..matrix.columns {
            values.push(
                covariance_pair64(matrix, left, right, rows, &complete, normalized, context)?.0,
            );
        }
    }
    if matrix.complex {
        complex_f64_output(shape, values)
    } else {
        real_f64_output(shape, values.into_iter().map(|value| value.re).collect())
    }
}

fn covariance32(
    matrix: &PreparedMatrix32,
    rows: CovarianceRows,
    normalized: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let shape = square_shape("cov", matrix.columns)?;
    let complete = complete_rows32(matrix, context)?;
    let mut values = reserved_vec("cov", matrix.columns.saturating_mul(matrix.columns))?;
    for right in 0..matrix.columns {
        for left in 0..matrix.columns {
            values.push(
                covariance_pair32(matrix, left, right, rows, &complete, normalized, context)?.0,
            );
        }
    }
    crate::core_elementary::complex_f32_array_output(shape, values)
}

#[allow(clippy::cast_precision_loss, clippy::needless_range_loop)]
fn correlation_pair64(
    matrix: &PreparedMatrix64,
    left: usize,
    right: usize,
    rows: CovarianceRows,
    complete: &[bool],
    context: &BuiltinContext<'_>,
) -> Result<(ArrayComplex64, usize), BuiltinError> {
    let mut left_mean = ArrayComplex64::ZERO;
    let mut right_mean = ArrayComplex64::ZERO;
    let mut count = 0_usize;
    for row in 0..matrix.rows {
        let left_value = matrix.values[row + left * matrix.rows];
        let right_value = matrix.values[row + right * matrix.rows];
        let include = match rows {
            CovarianceRows::Include => true,
            CovarianceRows::Complete => complete[row],
            CovarianceRows::Pairwise => {
                !complex64_missing(left_value) && !complex64_missing(right_value)
            }
        };
        if include {
            left_mean.re += left_value.re;
            left_mean.im += left_value.im;
            right_mean.re += right_value.re;
            right_mean.im += right_value.im;
            count += 1;
        }
    }
    if count == 0 {
        return Ok((ArrayComplex64::new(f64::NAN, 0.0), 0));
    }
    let divisor = count as f64;
    left_mean.re /= divisor;
    left_mean.im /= divisor;
    right_mean.re /= divisor;
    right_mean.im /= divisor;
    let mut cross = ArrayComplex64::ZERO;
    let mut left_square = 0.0_f64;
    let mut right_square = 0.0_f64;
    for row in 0..matrix.rows {
        check_cancelled_at(context, row)?;
        let left_value = matrix.values[row + left * matrix.rows];
        let right_value = matrix.values[row + right * matrix.rows];
        let include = match rows {
            CovarianceRows::Include => true,
            CovarianceRows::Complete => complete[row],
            CovarianceRows::Pairwise => {
                !complex64_missing(left_value) && !complex64_missing(right_value)
            }
        };
        if include {
            let left_re = left_value.re - left_mean.re;
            let left_im = left_value.im - left_mean.im;
            let right_re = right_value.re - right_mean.re;
            let right_im = right_value.im - right_mean.im;
            cross.re += left_re * right_re + left_im * right_im;
            cross.im += left_re * right_im - left_im * right_re;
            left_square += left_re * left_re + left_im * left_im;
            right_square += right_re * right_re + right_im * right_im;
        }
    }
    let denominator = left_square.sqrt() * right_square.sqrt();
    Ok((
        ArrayComplex64::new(cross.re / denominator, cross.im / denominator),
        count,
    ))
}

#[allow(clippy::cast_precision_loss, clippy::needless_range_loop)]
fn correlation_pair32(
    matrix: &PreparedMatrix32,
    left: usize,
    right: usize,
    rows: CovarianceRows,
    complete: &[bool],
    context: &BuiltinContext<'_>,
) -> Result<(ArrayComplex32, usize), BuiltinError> {
    let mut left_mean = ArrayComplex32::ZERO;
    let mut right_mean = ArrayComplex32::ZERO;
    let mut count = 0_usize;
    for row in 0..matrix.rows {
        let left_value = matrix.values[row + left * matrix.rows];
        let right_value = matrix.values[row + right * matrix.rows];
        let include = match rows {
            CovarianceRows::Include => true,
            CovarianceRows::Complete => complete[row],
            CovarianceRows::Pairwise => {
                !complex32_missing(left_value) && !complex32_missing(right_value)
            }
        };
        if include {
            left_mean.re += left_value.re;
            left_mean.im += left_value.im;
            right_mean.re += right_value.re;
            right_mean.im += right_value.im;
            count += 1;
        }
    }
    if count == 0 {
        return Ok((ArrayComplex32::new(f32::NAN, 0.0), 0));
    }
    let divisor = count as f32;
    left_mean.re /= divisor;
    left_mean.im /= divisor;
    right_mean.re /= divisor;
    right_mean.im /= divisor;
    let mut cross = ArrayComplex32::ZERO;
    let mut left_square = 0.0_f32;
    let mut right_square = 0.0_f32;
    for row in 0..matrix.rows {
        check_cancelled_at(context, row)?;
        let left_value = matrix.values[row + left * matrix.rows];
        let right_value = matrix.values[row + right * matrix.rows];
        let include = match rows {
            CovarianceRows::Include => true,
            CovarianceRows::Complete => complete[row],
            CovarianceRows::Pairwise => {
                !complex32_missing(left_value) && !complex32_missing(right_value)
            }
        };
        if include {
            let left_re = left_value.re - left_mean.re;
            let left_im = left_value.im - left_mean.im;
            let right_re = right_value.re - right_mean.re;
            let right_im = right_value.im - right_mean.im;
            cross.re += left_re * right_re + left_im * right_im;
            cross.im += left_re * right_im - left_im * right_re;
            left_square += left_re * left_re + left_im * left_im;
            right_square += right_re * right_re + right_im * right_im;
        }
    }
    let denominator = left_square.sqrt() * right_square.sqrt();
    Ok((
        ArrayComplex32::new(cross.re / denominator, cross.im / denominator),
        count,
    ))
}

fn correlation64(
    matrix: &PreparedMatrix64,
    rows: CovarianceRows,
    alpha: f64,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    if requested_outputs > 1 && matrix.complex {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "multiple outputs from `corrcoef` require real input",
        ));
    }
    let shape = square_shape("corrcoef", matrix.columns)?;
    let complete = complete_rows64(matrix, context)?;
    let length = matrix.columns.saturating_mul(matrix.columns);
    let mut correlations = reserved_vec("corrcoef", length)?;
    let mut counts = reserved_vec("corrcoef", length)?;
    for right in 0..matrix.columns {
        for left in 0..matrix.columns {
            let (correlation, count) =
                correlation_pair64(matrix, left, right, rows, &complete, context)?;
            correlations.push(correlation);
            counts.push(count);
        }
    }
    let correlation = if matrix.complex {
        complex_f64_output(shape.clone(), correlations.clone())?
    } else {
        real_f64_output(
            shape.clone(),
            correlations.iter().map(|value| value.re).collect(),
        )?
    };
    let mut outputs = vec![correlation];
    if requested_outputs > 1 {
        let (probabilities, lower, upper) =
            correlation_inference(&correlations, &counts, matrix.columns, alpha, context)?;
        outputs.push(real_f64_output(shape.clone(), probabilities)?);
        if requested_outputs > 2 {
            outputs.push(real_f64_output(shape.clone(), lower)?);
        }
        if requested_outputs > 3 {
            outputs.push(real_f64_output(shape, upper)?);
        }
    }
    Ok(outputs)
}

#[allow(clippy::cast_possible_truncation)]
fn correlation32(
    matrix: &PreparedMatrix32,
    rows: CovarianceRows,
    alpha: f64,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Value>, BuiltinError> {
    if requested_outputs > 1 && matrix.complex {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "multiple outputs from `corrcoef` require real input",
        ));
    }
    let shape = square_shape("corrcoef", matrix.columns)?;
    let complete = complete_rows32(matrix, context)?;
    let length = matrix.columns.saturating_mul(matrix.columns);
    let mut correlations = reserved_vec("corrcoef", length)?;
    let mut counts = reserved_vec("corrcoef", length)?;
    for right in 0..matrix.columns {
        for left in 0..matrix.columns {
            let (correlation, count) =
                correlation_pair32(matrix, left, right, rows, &complete, context)?;
            correlations.push(correlation);
            counts.push(count);
        }
    }
    let correlation =
        crate::core_elementary::complex_f32_array_output(shape.clone(), correlations.clone())?;
    let mut outputs = vec![correlation];
    if requested_outputs > 1 {
        let correlations64 = correlations
            .iter()
            .map(|value| ArrayComplex64::new(f64::from(value.re), f64::from(value.im)))
            .collect::<Vec<_>>();
        let (probabilities, lower, upper) =
            correlation_inference(&correlations64, &counts, matrix.columns, alpha, context)?;
        let single_output = |shape: Shape, values: Vec<f64>| {
            crate::core_elementary::complex_f32_array_output(
                shape,
                values
                    .into_iter()
                    .map(|value| ArrayComplex32::new(value as f32, 0.0))
                    .collect(),
            )
        };
        outputs.push(single_output(shape.clone(), probabilities)?);
        if requested_outputs > 2 {
            outputs.push(single_output(shape.clone(), lower)?);
        }
        if requested_outputs > 3 {
            outputs.push(single_output(shape, upper)?);
        }
    }
    Ok(outputs)
}

#[allow(clippy::type_complexity)]
fn correlation_inference(
    correlations: &[ArrayComplex64],
    counts: &[usize],
    columns: usize,
    alpha: f64,
    context: &BuiltinContext<'_>,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>), BuiltinError> {
    let mut probabilities = reserved_vec("corrcoef", correlations.len())?;
    let mut lower = reserved_vec("corrcoef", correlations.len())?;
    let mut upper = reserved_vec("corrcoef", correlations.len())?;
    let critical = inverse_standard_normal(1.0 - alpha / 2.0);
    for (index, (correlation, count)) in correlations.iter().zip(counts).enumerate() {
        check_cancelled_at(context, index)?;
        let row = index % columns.max(1);
        let column = index / columns.max(1);
        if row == column {
            probabilities.push(1.0);
            lower.push(1.0);
            upper.push(1.0);
            continue;
        }
        let value = correlation.re;
        probabilities.push(correlation_probability(value, *count));
        let (lo, hi) = correlation_interval(value, *count, critical);
        lower.push(lo);
        upper.push(hi);
    }
    Ok((probabilities, lower, upper))
}

#[allow(clippy::cast_precision_loss, clippy::float_cmp)]
fn correlation_probability(correlation: f64, count: usize) -> f64 {
    if count <= 2 || !correlation.is_finite() || correlation.abs() > 1.0 {
        return f64::NAN;
    }
    if correlation.abs() == 1.0 {
        return 0.0;
    }
    let degrees = (count - 2) as f64;
    regularized_beta(
        (1.0 - correlation * correlation).clamp(0.0, 1.0),
        degrees / 2.0,
        0.5,
    )
}

#[allow(clippy::cast_precision_loss, clippy::float_cmp)]
fn correlation_interval(correlation: f64, count: usize, critical: f64) -> (f64, f64) {
    if count <= 3 || !correlation.is_finite() || correlation.abs() > 1.0 {
        return (f64::NAN, f64::NAN);
    }
    if correlation.abs() == 1.0 {
        return (correlation, correlation);
    }
    let radius = critical / ((count - 3) as f64).sqrt();
    let center = correlation.atanh();
    ((center - radius).tanh(), (center + radius).tanh())
}

fn regularized_beta(value: f64, left: f64, right: f64) -> f64 {
    if value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    let logarithm = ln_gamma(left + right) - ln_gamma(left) - ln_gamma(right)
        + left * value.ln()
        + right * (-value).ln_1p();
    let front = logarithm.exp();
    if value < (left + 1.0) / (left + right + 2.0) {
        front * beta_continued_fraction(value, left, right) / left
    } else {
        1.0 - front * beta_continued_fraction(1.0 - value, right, left) / right
    }
}

#[allow(clippy::cast_precision_loss)]
fn beta_continued_fraction(value: f64, left: f64, right: f64) -> f64 {
    const MAX_ITERATIONS: usize = 256;
    const EPSILON: f64 = 3.0e-14;
    const FLOOR: f64 = 1.0e-300;
    let sum = left + right;
    let mut denominator = 1.0 - sum * value / (left + 1.0);
    if denominator.abs() < FLOOR {
        denominator = FLOOR;
    }
    denominator = 1.0 / denominator;
    let mut fraction = denominator;
    let mut numerator = 1.0_f64;
    for iteration in 1..=MAX_ITERATIONS {
        let iteration = iteration as f64;
        let even = iteration * (right - iteration) * value
            / ((left + 2.0 * iteration - 1.0) * (left + 2.0 * iteration));
        denominator = 1.0 + even * denominator;
        if denominator.abs() < FLOOR {
            denominator = FLOOR;
        }
        numerator = 1.0 + even / numerator;
        if numerator.abs() < FLOOR {
            numerator = FLOOR;
        }
        denominator = 1.0 / denominator;
        fraction *= denominator * numerator;
        let odd = -(left + iteration) * (sum + iteration) * value
            / ((left + 2.0 * iteration) * (left + 2.0 * iteration + 1.0));
        denominator = 1.0 + odd * denominator;
        if denominator.abs() < FLOOR {
            denominator = FLOOR;
        }
        numerator = 1.0 + odd / numerator;
        if numerator.abs() < FLOOR {
            numerator = FLOOR;
        }
        denominator = 1.0 / denominator;
        let delta = denominator * numerator;
        fraction *= delta;
        if (delta - 1.0).abs() <= EPSILON {
            break;
        }
    }
    fraction
}

#[allow(clippy::cast_precision_loss)]
fn ln_gamma(value: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    let shifted = value - 1.0;
    let mut series = COEFFICIENTS[0];
    for (index, coefficient) in COEFFICIENTS.iter().enumerate().skip(1) {
        series += coefficient / (shifted + index as f64);
    }
    let base = shifted + 7.5;
    0.918_938_533_204_672_7 + (shifted + 0.5) * base.ln() - base + series.ln()
}

fn polynomial(value: f64, coefficients: &[f64]) -> f64 {
    coefficients
        .iter()
        .rev()
        .fold(0.0, |result, coefficient| result * value + coefficient)
}

fn inverse_standard_normal(probability: f64) -> f64 {
    const CENTRAL_NUMERATOR: [f64; 8] = [
        3.387_132_872_796_366_5,
        133.141_667_891_784_38,
        1_971.590_950_306_551_3,
        13_731.693_765_509_46,
        45_921.953_931_549_87,
        67_265.770_927_008_7,
        33_430.575_583_588_13,
        2_509.080_928_730_122_7,
    ];
    const CENTRAL_DENOMINATOR: [f64; 8] = [
        1.0,
        42.313_330_701_600_91,
        687.187_007_492_057_9,
        5_394.196_021_424_751,
        21_213.794_301_586_597,
        39_307.895_800_092_71,
        28_729.085_735_721_943,
        5_226.495_278_852_854,
    ];
    const TAIL_NUMERATOR: [f64; 8] = [
        1.423_437_110_749_683_5,
        4.630_337_846_156_545,
        5.769_497_221_460_691,
        3.647_848_324_763_204_5,
        1.270_458_252_452_368_4,
        0.241_780_725_177_450_6,
        0.022_723_844_989_269_185,
        0.000_774_545_014_278_341_4,
    ];
    const TAIL_DENOMINATOR: [f64; 8] = [
        1.0,
        2.053_191_626_637_759,
        1.676_384_830_183_803_8,
        0.689_767_334_985_1,
        0.148_103_976_427_480_08,
        0.015_198_666_563_616_457,
        0.000_547_593_808_499_534_5,
        1.050_750_071_644_416_9e-9,
    ];
    const FAR_NUMERATOR: [f64; 8] = [
        6.657_904_643_501_104,
        5.463_784_911_164_114,
        1.784_826_539_917_291_3,
        0.296_560_571_828_504_9,
        0.026_532_189_526_576_124,
        0.001_242_660_947_388_078_4,
        0.000_027_115_555_687_434_876,
        2.010_334_399_292_288e-7,
    ];
    const FAR_DENOMINATOR: [f64; 8] = [
        1.0,
        0.599_832_206_555_887_9,
        0.136_929_880_922_735_8,
        0.014_875_361_290_850_615,
        0.000_786_869_131_145_613_2,
        0.000_018_463_183_175_100_547,
        1.421_511_758_316_445_8e-7,
        2.044_263_103_389_94e-15,
    ];
    let centered = probability - 0.5;
    if centered.abs() <= 0.425 {
        let radius = 0.180_625 - centered * centered;
        return centered * polynomial(radius, &CENTRAL_NUMERATOR)
            / polynomial(radius, &CENTRAL_DENOMINATOR);
    }
    let tail = if centered < 0.0 {
        probability
    } else {
        1.0 - probability
    };
    let mut radius = (-tail.ln()).sqrt();
    let result = if radius <= 5.0 {
        radius -= 1.6;
        polynomial(radius, &TAIL_NUMERATOR) / polynomial(radius, &TAIL_DENOMINATOR)
    } else {
        radius -= 5.0;
        polynomial(radius, &FAR_NUMERATOR) / polynomial(radius, &FAR_DENOMINATOR)
    };
    if centered < 0.0 { -result } else { result }
}

fn dispersion_builtin(
    operation: Dispersion,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = match operation {
        Dispersion::StandardDeviation => "std",
        Dispersion::Variance => "var",
    };
    expect_argument_count_range(name, arguments, 1, 4)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = dispersion_dimensions(name, &arguments[0])?;
    let (normalized, option_start) = parse_normalization(arguments)?;
    let (selection, missing) =
        parse_reduction_options(name, &arguments[option_start..], option_start + 1, context)?;
    let plan = reduction_plan(name, dimensions, selection, context)?;
    let variance = match &arguments[0] {
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_)) => {
            variance_single(&arguments[0], &plan, missing, normalized, context)?
        }
        _ => variance_double(&arguments[0], &plan, missing, normalized, context)?,
    };
    let output = match (operation, variance) {
        (Dispersion::Variance, value) => value,
        (Dispersion::StandardDeviation, Value::Array(ArrayData::F32(array))) => {
            let values = map_reserved(name, array.as_slice(), context, |value| value.sqrt())?;
            DenseArray::from_vec(array.shape().clone(), values)
                .map(ArrayData::F32)
                .map(Value::Array)
                .map_err(|error| array_error(&error))?
        }
        (Dispersion::StandardDeviation, Value::Double(value)) => Value::Double(value.sqrt()),
        (Dispersion::StandardDeviation, Value::Array(ArrayData::F64(array))) => {
            let values = map_reserved(name, array.as_slice(), context, |value| value.sqrt())?;
            DenseArray::from_vec(array.shape().clone(), values)
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(|error| array_error(&error))?
        }
        (Dispersion::StandardDeviation, _) => {
            return Err(mapping_error(name));
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn parse_mean_options(
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<(Option<Vec<u64>>, MissingPolicy, MeanOutput), BuiltinError> {
    let mut selection = None;
    let mut missing = MissingPolicy::Include;
    let mut output = MeanOutput::Default;
    for (offset, value) in arguments[1..].iter().enumerate() {
        check_cancelled_at(context, offset)?;
        match keyword(value).as_deref() {
            Some("all") => set_selection("mean", &mut selection, all_dimensions(&arguments[0])?)?,
            Some("omitnan") => missing = MissingPolicy::Omit,
            Some("includenan") => missing = MissingPolicy::Include,
            Some("default") => output = MeanOutput::Default,
            Some("double") => output = MeanOutput::Double,
            Some("native") => output = MeanOutput::Native,
            Some(_) => return Err(option_error("mean")),
            None => {
                let dimensions = dimension_values("mean", offset + 2, value, context)?;
                set_selection("mean", &mut selection, dimensions)?;
            }
        }
    }
    Ok((selection, missing, output))
}

fn parse_reduction_options(
    name: &str,
    arguments: &[Value],
    first_position: usize,
    context: &BuiltinContext<'_>,
) -> Result<(Option<Vec<u64>>, MissingPolicy), BuiltinError> {
    let mut selection = None;
    let mut missing = MissingPolicy::Include;
    for (offset, value) in arguments.iter().enumerate() {
        check_cancelled_at(context, offset)?;
        match keyword(value).as_deref() {
            Some("all") => {
                // `all` semantically selects every existing input dimension.
                // An empty selection is completed by `reduction_plan`.
                set_selection(name, &mut selection, Vec::new())?;
            }
            Some("omitnan") => missing = MissingPolicy::Omit,
            Some("includenan") => missing = MissingPolicy::Include,
            Some(_) => return Err(option_error(name)),
            None => {
                let dimensions = dimension_values(name, first_position + offset, value, context)?;
                set_selection(name, &mut selection, dimensions)?;
            }
        }
    }
    Ok((selection, missing))
}

#[allow(clippy::float_cmp)]
fn parse_normalization(arguments: &[Value]) -> Result<(bool, usize), BuiltinError> {
    let Some(value) = arguments.get(1) else {
        return Ok((false, 1));
    };
    if keyword(value).is_some() {
        return Ok((false, 1));
    }
    if value.numel() == Some(0) {
        return Ok((false, 2));
    }
    let weight = exact_real_integer_scalar(value)
        .map(|value| match value {
            IntegerComponent::Signed(value) => value == 1,
            IntegerComponent::Unsigned(value) => value == 1,
        })
        .or_else(|| value.as_real_number().map(|value| value == 1.0))
        .or_else(|| value.as_real_single().map(|value| value == 1.0))
        .ok_or_else(normalization_error)?;
    let valid_zero = value.as_real_number() == Some(0.0)
        || value.as_real_single() == Some(0.0)
        || exact_real_integer_scalar(value).is_some_and(IntegerComponent::is_zero);
    if !weight && !valid_zero {
        return Err(normalization_error());
    }
    Ok((weight, 2))
}

fn set_selection(
    name: &str,
    selection: &mut Option<Vec<u64>>,
    dimensions: Vec<u64>,
) -> Result<(), BuiltinError> {
    if selection.is_some() {
        Err(option_error(name))
    } else {
        *selection = Some(dimensions);
        Ok(())
    }
}

pub(super) fn reduction_plan(
    name: &str,
    dimensions: &[u64],
    selection: Option<Vec<u64>>,
    context: &BuiltinContext<'_>,
) -> Result<ReductionPlan, BuiltinError> {
    let mut selected = match selection {
        Some(selected) if selected.is_empty() => all_dimensions_from_slice(dimensions)?,
        Some(selected) => selected,
        None => vec![default_dimension(dimensions)],
    };
    selected.sort_unstable();
    let mut output_dimensions = reserved_vec(name, dimensions.len())?;
    for (index, extent) in dimensions.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let dimension = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| mapping_error(name))?;
        output_dimensions.push(if selected.binary_search(&dimension).is_ok() {
            1
        } else {
            extent
        });
    }
    let output_shape = Shape::new(output_dimensions).map_err(|error| array_error(&error))?;
    let output_length = usize::try_from(output_shape.numel()).map_err(|_| mapping_error(name))?;
    Ok(ReductionPlan {
        input_dimensions: dimensions.to_vec(),
        selected,
        output_shape,
        output_length,
    })
}

#[allow(clippy::cast_precision_loss)]
fn mean_double(
    input: &Value,
    plan: &ReductionPlan,
    missing: MissingPolicy,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let mut output = filled_acc64("mean", plan.output_length, context)?;
    visit_f64(input, "mean", context, |index, value| {
        if missing == MissingPolicy::Omit && (value.re.is_nan() || value.im.is_nan()) {
            return Ok(());
        }
        let target = output_offset("mean", index, plan)?;
        let accumulator = output
            .get_mut(target)
            .ok_or_else(|| mapping_error("mean"))?;
        accumulator.re += value.re;
        accumulator.im += value.im;
        accumulator.count = accumulator
            .count
            .checked_add(1)
            .ok_or_else(|| mapping_error("mean"))?;
        Ok(())
    })?;
    let mut values = reserved_vec("mean", output.len())?;
    for (index, value) in output.into_iter().enumerate() {
        check_cancelled_at(context, index)?;
        values.push(if value.count == 0 {
            ArrayComplex64::new(f64::NAN, 0.0)
        } else {
            let count = value.count as f64;
            ArrayComplex64::new(value.re / count, value.im / count)
        });
    }
    complex_f64_output(plan.output_shape.clone(), values)
}

#[allow(clippy::cast_precision_loss)]
fn mean_single(
    input: &Value,
    plan: &ReductionPlan,
    missing: MissingPolicy,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let mut output = filled_acc32("mean", plan.output_length, context)?;
    visit_f32(input, "mean", context, |index, value| {
        if missing == MissingPolicy::Omit && (value.re.is_nan() || value.im.is_nan()) {
            return Ok(());
        }
        let target = output_offset("mean", index, plan)?;
        let accumulator = output
            .get_mut(target)
            .ok_or_else(|| mapping_error("mean"))?;
        accumulator.re += value.re;
        accumulator.im += value.im;
        accumulator.count = accumulator
            .count
            .checked_add(1)
            .ok_or_else(|| mapping_error("mean"))?;
        Ok(())
    })?;
    let mut values = reserved_vec("mean", output.len())?;
    for (index, value) in output.into_iter().enumerate() {
        check_cancelled_at(context, index)?;
        values.push(if value.count == 0 {
            ArrayComplex32::new(f32::NAN, 0.0)
        } else {
            let count = value.count as f32;
            ArrayComplex32::new(value.re / count, value.im / count)
        });
    }
    crate::core_elementary::complex_f32_array_output(plan.output_shape.clone(), values)
}

#[allow(clippy::cast_precision_loss)]
fn mean_native_logical(
    input: &Value,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let mut any = filled_values("mean", plan.output_length, false, context)?;
    let mut counts = filled_values("mean", plan.output_length, 0_u64, context)?;
    visit_logical(input, "mean", context, |index, value| {
        let target = output_offset("mean", index, plan)?;
        any[target] |= value;
        counts[target] = counts[target]
            .checked_add(1)
            .ok_or_else(|| mapping_error("mean"))?;
        Ok(())
    })?;
    let mut values = reserved_vec("mean", any.len())?;
    for (index, (value, count)) in any.into_iter().zip(counts).enumerate() {
        check_cancelled_at(context, index)?;
        values.push(if count == 0 {
            f64::NAN
        } else {
            f64::from(value) / count as f64
        });
    }
    real_f64_output(plan.output_shape.clone(), values)
}

macro_rules! native_integer_mean {
    ($array:expr, $plan:expr, $context:expr, $kind:ty, signed) => {{
        let mut sums = filled_values("mean", $plan.output_length, 0_i128, $context)?;
        let mut counts = filled_values("mean", $plan.output_length, 0_u128, $context)?;
        for (index, value) in $array.as_slice().iter().copied().enumerate() {
            check_cancelled_at($context, index)?;
            let target = output_offset("mean", index, $plan)?;
            sums[target] = sums[target]
                .checked_add(i128::from(value))
                .ok_or_else(|| mapping_error("mean"))?;
            counts[target] += 1;
        }
        let mut values = reserved_vec("mean", sums.len())?;
        for (sum, count) in sums.into_iter().zip(counts) {
            values.push(if count == 0 {
                0
            } else {
                <$kind>::try_from(rounded_signed_ratio(sum, count)?)
                    .map_err(|_| mapping_error("mean"))?
            });
        }
        DenseArray::from_vec($plan.output_shape.clone(), values)
            .map(IntegerArrayData::from_typed)
            .map(ArrayData::Integer)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }};
    ($array:expr, $plan:expr, $context:expr, $kind:ty, unsigned) => {{
        let mut sums = filled_values("mean", $plan.output_length, 0_u128, $context)?;
        let mut counts = filled_values("mean", $plan.output_length, 0_u128, $context)?;
        for (index, value) in $array.as_slice().iter().copied().enumerate() {
            check_cancelled_at($context, index)?;
            let target = output_offset("mean", index, $plan)?;
            sums[target] = sums[target]
                .checked_add(u128::from(value))
                .ok_or_else(|| mapping_error("mean"))?;
            counts[target] += 1;
        }
        let mut values = reserved_vec("mean", sums.len())?;
        for (sum, count) in sums.into_iter().zip(counts) {
            values.push(if count == 0 {
                0
            } else {
                <$kind>::try_from(rounded_unsigned_ratio(sum, count)?)
                    .map_err(|_| mapping_error("mean"))?
            });
        }
        DenseArray::from_vec($plan.output_shape.clone(), values)
            .map(IntegerArrayData::from_typed)
            .map(ArrayData::Integer)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }};
}

fn mean_native_integer(
    input: &IntegerArrayData,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match input {
        IntegerArrayData::I8(array) => native_integer_mean!(array, plan, context, i8, signed),
        IntegerArrayData::U8(array) => native_integer_mean!(array, plan, context, u8, unsigned),
        IntegerArrayData::I16(array) => native_integer_mean!(array, plan, context, i16, signed),
        IntegerArrayData::U16(array) => native_integer_mean!(array, plan, context, u16, unsigned),
        IntegerArrayData::I32(array) => native_integer_mean!(array, plan, context, i32, signed),
        IntegerArrayData::U32(array) => native_integer_mean!(array, plan, context, u32, unsigned),
        IntegerArrayData::I64(array) => native_integer_mean!(array, plan, context, i64, signed),
        IntegerArrayData::U64(array) => native_integer_mean!(array, plan, context, u64, unsigned),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "`mean` does not accept complex integer storage",
        )),
    }
}

fn rounded_signed_ratio(sum: i128, count: u128) -> Result<i128, BuiltinError> {
    let count = i128::try_from(count).map_err(|_| mapping_error("mean"))?;
    let quotient = sum / count;
    let remainder = sum % count;
    let twice = remainder.unsigned_abs().saturating_mul(2);
    let unsigned_count = u128::try_from(count).map_err(|_| mapping_error("mean"))?;
    Ok(if twice >= unsigned_count {
        quotient + if sum < 0 { -1 } else { 1 }
    } else {
        quotient
    })
}

fn rounded_unsigned_ratio(sum: u128, count: u128) -> Result<u128, BuiltinError> {
    if count == 0 {
        return Err(mapping_error("mean"));
    }
    let quotient = sum / count;
    let remainder = sum % count;
    Ok(quotient + u128::from(remainder.saturating_mul(2) >= count))
}

fn median_double(
    input: &Value,
    plan: &ReductionPlan,
    missing: MissingPolicy,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let complex_order = input.is_complex_numeric();
    let mut groups = complex64_groups("median", plan, context)?;
    visit_f64(input, "median", context, |index, value| {
        if missing == MissingPolicy::Omit && (value.re.is_nan() || value.im.is_nan()) {
            return Ok(());
        }
        let target = output_offset("median", index, plan)?;
        groups[target].push(value);
        Ok(())
    })?;
    let mut output = reserved_vec("median", groups.len())?;
    for (index, mut group) in groups.into_iter().enumerate() {
        check_cancelled_at(context, index)?;
        if missing == MissingPolicy::Include && complex64_group_has_missing(&group, context)? {
            output.push(ArrayComplex64::new(f64::NAN, 0.0));
            continue;
        }
        stable_sort_by("median", &mut group, context, |left, right| {
            compare_complex64(*left, *right, complex_order)
        })?;
        output.push(middle_complex64(&group));
    }
    complex_f64_output(plan.output_shape.clone(), output)
}

fn median_single(
    input: &Value,
    plan: &ReductionPlan,
    missing: MissingPolicy,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let complex_order = input.is_complex_numeric();
    let mut groups = complex32_groups("median", plan, context)?;
    visit_f32(input, "median", context, |index, value| {
        if missing == MissingPolicy::Omit && (value.re.is_nan() || value.im.is_nan()) {
            return Ok(());
        }
        let target = output_offset("median", index, plan)?;
        groups[target].push(value);
        Ok(())
    })?;
    let mut output = reserved_vec("median", groups.len())?;
    for (index, mut group) in groups.into_iter().enumerate() {
        check_cancelled_at(context, index)?;
        if missing == MissingPolicy::Include && complex32_group_has_missing(&group, context)? {
            output.push(ArrayComplex32::new(f32::NAN, 0.0));
            continue;
        }
        stable_sort_by("median", &mut group, context, |left, right| {
            compare_complex32(*left, *right, complex_order)
        })?;
        output.push(middle_complex32(&group));
    }
    crate::core_elementary::complex_f32_array_output(plan.output_shape.clone(), output)
}

fn median_logical(
    input: &Value,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let mut true_counts = filled_values("median", plan.output_length, 0_u64, context)?;
    let mut counts = filled_values("median", plan.output_length, 0_u64, context)?;
    visit_logical(input, "median", context, |index, value| {
        let target = output_offset("median", index, plan)?;
        true_counts[target] += u64::from(value);
        counts[target] += 1;
        Ok(())
    })?;
    let mut values = reserved_vec("median", true_counts.len())?;
    for (index, (true_count, count)) in true_counts.into_iter().zip(counts).enumerate() {
        check_cancelled_at(context, index)?;
        values.push(Logical::from(count != 0 && true_count >= count.div_ceil(2)));
    }
    DenseArray::from_vec(plan.output_shape.clone(), values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn median_char(
    input: &DenseArray<CharCodeUnit>,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if input.numel() == 0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`median` does not define a value for an empty char array",
        ));
    }
    let mut groups: Vec<Vec<u16>> = typed_groups("median", plan, context)?;
    for (index, value) in input.as_slice().iter().enumerate() {
        check_cancelled_at(context, index)?;
        let target = output_offset("median", index, plan)?;
        groups[target].push(value.get());
    }
    let odd_length = groups.first().is_some_and(|group| group.len() % 2 == 1);
    if odd_length {
        let mut values = reserved_vec("median", groups.len())?;
        for mut group in groups {
            stable_sort_by("median", &mut group, context, Ord::cmp)?;
            let value = group
                .get(group.len() / 2)
                .copied()
                .ok_or_else(|| mapping_error("median"))?;
            values.push(CharCodeUnit::new(value));
        }
        DenseArray::from_vec(plan.output_shape.clone(), values)
            .map(ArrayData::Char)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        let mut values = reserved_vec("median", groups.len())?;
        for mut group in groups {
            stable_sort_by("median", &mut group, context, Ord::cmp)?;
            let lower = group
                .get(group.len() / 2 - 1)
                .copied()
                .ok_or_else(|| mapping_error("median"))?;
            let upper = group
                .get(group.len() / 2)
                .copied()
                .ok_or_else(|| mapping_error("median"))?;
            values.push(f64::midpoint(f64::from(lower), f64::from(upper)));
        }
        real_f64_output(plan.output_shape.clone(), values)
    }
}

macro_rules! median_integer {
    ($array:expr, $plan:expr, $context:expr, $kind:ty, signed) => {{
        let mut groups: Vec<Vec<$kind>> = typed_groups("median", $plan, $context)?;
        for (index, value) in $array.as_slice().iter().copied().enumerate() {
            check_cancelled_at($context, index)?;
            let target = output_offset("median", index, $plan)?;
            groups[target].push(value);
        }
        let mut values = reserved_vec("median", groups.len())?;
        for mut group in groups {
            stable_sort_by("median", &mut group, $context, Ord::cmp)?;
            values.push(if group.is_empty() {
                0
            } else if group.len() % 2 == 1 {
                group[group.len() / 2]
            } else {
                let lower = i128::from(group[group.len() / 2 - 1]);
                let upper = i128::from(group[group.len() / 2]);
                let sum = lower + upper;
                let midpoint = sum / 2;
                let adjust = if sum % 2 == 0 || (sum < 0 && upper == 0) {
                    0
                } else if sum < 0 {
                    -1
                } else {
                    1
                };
                <$kind>::try_from(midpoint + adjust).map_err(|_| mapping_error("median"))?
            });
        }
        DenseArray::from_vec($plan.output_shape.clone(), values)
            .map(IntegerArrayData::from_typed)
            .map(ArrayData::Integer)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }};
    ($array:expr, $plan:expr, $context:expr, $kind:ty, unsigned) => {{
        let mut groups: Vec<Vec<$kind>> = typed_groups("median", $plan, $context)?;
        for (index, value) in $array.as_slice().iter().copied().enumerate() {
            check_cancelled_at($context, index)?;
            let target = output_offset("median", index, $plan)?;
            groups[target].push(value);
        }
        let mut values = reserved_vec("median", groups.len())?;
        for mut group in groups {
            stable_sort_by("median", &mut group, $context, Ord::cmp)?;
            values.push(if group.is_empty() {
                0
            } else if group.len() % 2 == 1 {
                group[group.len() / 2]
            } else {
                let lower = u128::from(group[group.len() / 2 - 1]);
                let upper = u128::from(group[group.len() / 2]);
                <$kind>::try_from(lower + (upper - lower).div_ceil(2))
                    .map_err(|_| mapping_error("median"))?
            });
        }
        DenseArray::from_vec($plan.output_shape.clone(), values)
            .map(IntegerArrayData::from_typed)
            .map(ArrayData::Integer)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }};
}

fn median_integer(
    input: &IntegerArrayData,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match input {
        IntegerArrayData::I8(array) => median_integer!(array, plan, context, i8, signed),
        IntegerArrayData::U8(array) => median_integer!(array, plan, context, u8, unsigned),
        IntegerArrayData::I16(array) => median_integer!(array, plan, context, i16, signed),
        IntegerArrayData::U16(array) => median_integer!(array, plan, context, u16, unsigned),
        IntegerArrayData::I32(array) => median_integer!(array, plan, context, i32, signed),
        IntegerArrayData::U32(array) => median_integer!(array, plan, context, u32, unsigned),
        IntegerArrayData::I64(array) => median_integer!(array, plan, context, i64, signed),
        IntegerArrayData::U64(array) => median_integer!(array, plan, context, u64, unsigned),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "`median` does not accept complex integer storage",
        )),
    }
}

#[allow(clippy::cast_precision_loss)]
fn variance_double(
    input: &Value,
    plan: &ReductionPlan,
    missing: MissingPolicy,
    normalized: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let mut means = filled_acc64("var", plan.output_length, context)?;
    visit_f64(input, "var", context, |index, value| {
        if missing == MissingPolicy::Omit && (value.re.is_nan() || value.im.is_nan()) {
            return Ok(());
        }
        let target = output_offset("var", index, plan)?;
        means[target].re += value.re;
        means[target].im += value.im;
        means[target].count += 1;
        Ok(())
    })?;
    for value in &mut means {
        if value.count != 0 {
            let count = value.count as f64;
            value.re /= count;
            value.im /= count;
        }
    }
    let mut squares = filled_values("var", plan.output_length, 0.0_f64, context)?;
    visit_f64(input, "var", context, |index, value| {
        if missing == MissingPolicy::Omit && (value.re.is_nan() || value.im.is_nan()) {
            return Ok(());
        }
        let target = output_offset("var", index, plan)?;
        let mean = means[target];
        let re = value.re - mean.re;
        let im = value.im - mean.im;
        squares[target] += re * re + im * im;
        Ok(())
    })?;
    let mut values = reserved_vec("var", squares.len())?;
    for (index, (square, mean)) in squares.into_iter().zip(means).enumerate() {
        check_cancelled_at(context, index)?;
        values.push(variance_finish_f64(square, mean.count, normalized));
    }
    real_f64_output(plan.output_shape.clone(), values)
}

#[allow(clippy::cast_precision_loss)]
fn variance_single(
    input: &Value,
    plan: &ReductionPlan,
    missing: MissingPolicy,
    normalized: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let mut means = filled_acc32("var", plan.output_length, context)?;
    visit_f32(input, "var", context, |index, value| {
        if missing == MissingPolicy::Omit && (value.re.is_nan() || value.im.is_nan()) {
            return Ok(());
        }
        let target = output_offset("var", index, plan)?;
        means[target].re += value.re;
        means[target].im += value.im;
        means[target].count += 1;
        Ok(())
    })?;
    for value in &mut means {
        if value.count != 0 {
            let count = value.count as f32;
            value.re /= count;
            value.im /= count;
        }
    }
    let mut squares = filled_values("var", plan.output_length, 0.0_f32, context)?;
    visit_f32(input, "var", context, |index, value| {
        if missing == MissingPolicy::Omit && (value.re.is_nan() || value.im.is_nan()) {
            return Ok(());
        }
        let target = output_offset("var", index, plan)?;
        let mean = means[target];
        let re = value.re - mean.re;
        let im = value.im - mean.im;
        squares[target] += re * re + im * im;
        Ok(())
    })?;
    let mut values = reserved_vec("var", squares.len())?;
    for (index, (square, mean)) in squares.into_iter().zip(means).enumerate() {
        check_cancelled_at(context, index)?;
        values.push(variance_finish_f32(square, mean.count, normalized));
    }
    DenseArray::from_vec(plan.output_shape.clone(), values)
        .map(ArrayData::F32)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_precision_loss)]
fn variance_finish_f64(square: f64, count: u64, normalized: bool) -> f64 {
    if count == 0 {
        f64::NAN
    } else {
        let denominator = if normalized {
            count
        } else {
            count.saturating_sub(1).max(1)
        };
        square / denominator as f64
    }
}

#[allow(clippy::cast_precision_loss)]
fn variance_finish_f32(square: f32, count: u64, normalized: bool) -> f32 {
    if count == 0 {
        f32::NAN
    } else {
        let denominator = if normalized {
            count
        } else {
            count.saturating_sub(1).max(1)
        };
        square / denominator as f32
    }
}

pub(super) fn visit_f64<F>(
    input: &Value,
    name: &str,
    context: &BuiltinContext<'_>,
    mut visit: F,
) -> Result<(), BuiltinError>
where
    F: FnMut(usize, ArrayComplex64) -> Result<(), BuiltinError>,
{
    macro_rules! visit_slice {
        ($slice:expr, $convert:expr) => {{
            for (index, value) in $slice.iter().enumerate() {
                check_cancelled_at(context, index)?;
                visit(index, $convert(value))?;
            }
        }};
    }
    match input {
        Value::Logical(value) => visit(0, ArrayComplex64::new(f64::from(*value), 0.0)),
        Value::Double(value) => visit(0, ArrayComplex64::new(*value, 0.0)),
        Value::Complex(value) => visit(0, ArrayComplex64::new(value.real, value.imaginary)),
        Value::Array(ArrayData::F32(array)) => {
            visit_slice!(array.as_slice(), |value: &f32| ArrayComplex64::new(
                f64::from(*value),
                0.0
            ));
            Ok(())
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            visit_slice!(array.as_slice(), |value: &ArrayComplex32| {
                ArrayComplex64::new(f64::from(value.re), f64::from(value.im))
            });
            Ok(())
        }
        Value::Array(ArrayData::Logical(array)) => {
            visit_slice!(array.as_slice(), |value: &Logical| ArrayComplex64::new(
                f64::from(value.get()),
                0.0
            ));
            Ok(())
        }
        Value::Array(ArrayData::F64(array)) => {
            visit_slice!(array.as_slice(), |value: &f64| ArrayComplex64::new(
                *value, 0.0
            ));
            Ok(())
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            visit_slice!(array.as_slice(), |value: &ArrayComplex64| *value);
            Ok(())
        }
        Value::Array(ArrayData::Char(array)) => {
            visit_slice!(array.as_slice(), |value: &openmat_array::CharCodeUnit| {
                ArrayComplex64::new(f64::from(value.get()), 0.0)
            });
            Ok(())
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                visit(
                    index,
                    ArrayComplex64::new(integer_component_f64(value.real_component()), 0.0),
                )?;
            }
            Ok(())
        }
        value => Err(type_error(
            name,
            1,
            "real or complex numeric, logical, char, or integer array",
            value,
        )),
    }
}

pub(super) fn visit_f32<F>(
    input: &Value,
    name: &str,
    context: &BuiltinContext<'_>,
    mut visit: F,
) -> Result<(), BuiltinError>
where
    F: FnMut(usize, ArrayComplex32) -> Result<(), BuiltinError>,
{
    match input {
        Value::Array(ArrayData::F32(array)) => {
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                visit(index, ArrayComplex32::new(value, 0.0))?;
            }
            Ok(())
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                visit(index, value)?;
            }
            Ok(())
        }
        value => Err(type_error(name, 1, "single array", value)),
    }
}

pub(super) fn visit_logical<F>(
    input: &Value,
    name: &str,
    context: &BuiltinContext<'_>,
    mut visit: F,
) -> Result<(), BuiltinError>
where
    F: FnMut(usize, bool) -> Result<(), BuiltinError>,
{
    match input {
        Value::Logical(value) => visit(0, *value),
        Value::Array(ArrayData::Logical(array)) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                visit(index, value.get())?;
            }
            Ok(())
        }
        value => Err(type_error(name, 1, "logical array", value)),
    }
}

pub(super) fn output_offset(
    name: &str,
    input_offset: usize,
    plan: &ReductionPlan,
) -> Result<usize, BuiltinError> {
    let mut remainder = u64::try_from(input_offset).map_err(|_| mapping_error(name))?;
    let mut output_offset = 0_u64;
    let mut output_stride = 1_u64;
    for (index, extent) in plan.input_dimensions.iter().copied().enumerate() {
        let coordinate = if extent == 0 { 0 } else { remainder % extent };
        if extent != 0 {
            remainder /= extent;
        }
        let dimension = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| mapping_error(name))?;
        let coordinate = if plan.selected.binary_search(&dimension).is_ok() {
            0
        } else {
            coordinate
        };
        output_offset = coordinate
            .checked_mul(output_stride)
            .and_then(|value| output_offset.checked_add(value))
            .ok_or_else(|| mapping_error(name))?;
        output_stride = output_stride
            .checked_mul(plan.output_shape.extent(index))
            .ok_or_else(|| mapping_error(name))?;
    }
    usize::try_from(output_offset).map_err(|_| mapping_error(name))
}

pub(super) fn numeric_dimensions<'a>(
    name: &str,
    value: &'a Value,
) -> Result<&'a [u64], BuiltinError> {
    match value {
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
            | ArrayData::Integer(_),
        ) => value
            .dimensions()
            .ok_or_else(|| type_error(name, 1, "numeric, logical, char, or integer array", value)),
        _ => Err(type_error(
            name,
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

fn dispersion_dimensions<'a>(name: &str, value: &'a Value) -> Result<&'a [u64], BuiltinError> {
    if matches!(value, Value::Array(ArrayData::Integer(_))) {
        return Err(type_error(
            name,
            1,
            "double, single, logical, or char array",
            value,
        ));
    }
    numeric_dimensions(name, value)
}

pub(super) fn dimension_values(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        return checked_integer_dimension(name, value).map(|value| vec![value]);
    }
    if let Some(value) = value.as_real_number() {
        return checked_float_dimension(name, value).map(|value| vec![value]);
    }
    if let Some(value) = value.as_real_single() {
        return checked_float_dimension(name, f64::from(value)).map(|value| vec![value]);
    }
    let length = value
        .numel()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| type_error(name, position, "positive integer dimension vector", value))?;
    if length == 0 {
        return Err(dimension_error(name));
    }
    let mut output = reserved_vec(name, length)?;
    macro_rules! push_dimensions {
        ($slice:expr, $convert:expr) => {{
            for (index, value) in $slice.iter().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(checked_float_dimension(name, $convert(value))?);
            }
        }};
    }
    match value {
        Value::Array(ArrayData::F64(array)) => push_dimensions!(array.as_slice(), |v: &f64| *v),
        Value::Array(ArrayData::F32(array)) => {
            push_dimensions!(array.as_slice(), |v: &f32| f64::from(*v));
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(checked_integer_dimension(name, value.real_component())?);
            }
        }
        _ => {
            return Err(type_error(
                name,
                position,
                "positive integer dimension scalar or vector",
                value,
            ));
        }
    }
    output.sort_unstable();
    if output.windows(2).any(|values| values[0] == values[1]) {
        return Err(dimension_error(name));
    }
    Ok(output)
}

fn checked_integer_dimension(name: &str, value: IntegerComponent) -> Result<u64, BuiltinError> {
    match value {
        IntegerComponent::Signed(value) => u64::try_from(value).ok(),
        IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
    }
    .filter(|value| *value > 0)
    .ok_or_else(|| dimension_error(name))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn checked_float_dimension(name: &str, value: f64) -> Result<u64, BuiltinError> {
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        Err(dimension_error(name))
    } else {
        Ok(value as u64)
    }
}

fn all_dimensions(value: &Value) -> Result<Vec<u64>, BuiltinError> {
    all_dimensions_from_slice(value.dimensions().ok_or_else(|| mapping_error("mean"))?)
}

fn all_dimensions_from_slice(dimensions: &[u64]) -> Result<Vec<u64>, BuiltinError> {
    let mut output = reserved_vec("reduction", dimensions.len())?;
    for index in 0..dimensions.len() {
        output.push(
            u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| mapping_error("reduction"))?,
        );
    }
    Ok(output)
}

fn default_dimension(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|index| u64::try_from(index).ok())
        .and_then(|index| index.checked_add(1))
        .unwrap_or(1)
}

pub(super) fn keyword(value: &Value) -> Option<String> {
    if let Some(value) = value.as_string_scalar()
        && !value.is_missing()
    {
        return Some(value.to_utf8_lossy().to_ascii_lowercase());
    }
    match value {
        Value::Array(ArrayData::Char(array))
            if array.shape().ndims() == 2 && array.shape().extent(0) == 1 =>
        {
            String::from_utf16(
                &array
                    .as_slice()
                    .iter()
                    .map(|value| value.get())
                    .collect::<Vec<_>>(),
            )
            .ok()
            .map(|value| value.to_ascii_lowercase())
        }
        _ => None,
    }
}

fn compare_complex64(left: ArrayComplex64, right: ArrayComplex64, complex_order: bool) -> Ordering {
    if complex_order {
        left.re
            .hypot(left.im)
            .total_cmp(&right.re.hypot(right.im))
            .then_with(|| left.im.atan2(left.re).total_cmp(&right.im.atan2(right.re)))
    } else {
        left.re.total_cmp(&right.re)
    }
}

fn compare_complex32(left: ArrayComplex32, right: ArrayComplex32, complex_order: bool) -> Ordering {
    if complex_order {
        left.re
            .hypot(left.im)
            .total_cmp(&right.re.hypot(right.im))
            .then_with(|| left.im.atan2(left.re).total_cmp(&right.im.atan2(right.re)))
    } else {
        left.re.total_cmp(&right.re)
    }
}

fn complex64_group_has_missing(
    values: &[ArrayComplex64],
    context: &BuiltinContext<'_>,
) -> Result<bool, BuiltinError> {
    for (index, value) in values.iter().enumerate() {
        check_cancelled_at(context, index)?;
        if value.re.is_nan() || value.im.is_nan() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn complex32_group_has_missing(
    values: &[ArrayComplex32],
    context: &BuiltinContext<'_>,
) -> Result<bool, BuiltinError> {
    for (index, value) in values.iter().enumerate() {
        check_cancelled_at(context, index)?;
        if value.re.is_nan() || value.im.is_nan() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn stable_sort_by<T, F>(
    name: &str,
    values: &mut Vec<T>,
    context: &BuiltinContext<'_>,
    mut compare: F,
) -> Result<(), BuiltinError>
where
    T: Copy,
    F: FnMut(&T, &T) -> Ordering,
{
    if values.len() < 2 {
        return Ok(());
    }
    let mut scratch = reserved_vec(name, values.len())?;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        scratch.push(value);
    }
    let mut width = 1_usize;
    while width < values.len() {
        let mut start = 0_usize;
        while start < values.len() {
            context.check_cancelled()?;
            let middle = start.saturating_add(width).min(values.len());
            let end = middle.saturating_add(width).min(values.len());
            let mut left = start;
            let mut right = middle;
            for (target, destination) in scratch.iter_mut().enumerate().take(end).skip(start) {
                check_cancelled_at(context, target)?;
                let take_left = right == end
                    || (left < middle
                        && compare(&values[left], &values[right]) != Ordering::Greater);
                let source = if take_left {
                    let source = left;
                    left += 1;
                    source
                } else {
                    let source = right;
                    right += 1;
                    source
                };
                *destination = values[source];
            }
            start = end;
        }
        std::mem::swap(values, &mut scratch);
        width = width.saturating_mul(2);
    }
    Ok(())
}

fn middle_complex64(values: &[ArrayComplex64]) -> ArrayComplex64 {
    if values.is_empty() {
        ArrayComplex64::new(f64::NAN, 0.0)
    } else if values.len() % 2 == 1 {
        values[values.len() / 2]
    } else {
        let left = values[values.len() / 2 - 1];
        let right = values[values.len() / 2];
        ArrayComplex64::new(
            left.re / 2.0 + right.re / 2.0,
            left.im / 2.0 + right.im / 2.0,
        )
    }
}

fn middle_complex32(values: &[ArrayComplex32]) -> ArrayComplex32 {
    if values.is_empty() {
        ArrayComplex32::new(f32::NAN, 0.0)
    } else if values.len() % 2 == 1 {
        values[values.len() / 2]
    } else {
        let left = values[values.len() / 2 - 1];
        let right = values[values.len() / 2];
        ArrayComplex32::new(
            left.re / 2.0 + right.re / 2.0,
            left.im / 2.0 + right.im / 2.0,
        )
    }
}

pub(super) fn complex_f64_output(
    shape: Shape,
    values: Vec<ArrayComplex64>,
) -> Result<Value, BuiltinError> {
    if shape.numel() == 1 {
        return values
            .first()
            .copied()
            .map(crate::core_elementary::complex_scalar_output)
            .ok_or_else(|| mapping_error("reduction"));
    }
    crate::core_elementary::complex_f64_array_output(shape, values)
}

pub(super) fn real_f64_output(shape: Shape, values: Vec<f64>) -> Result<Value, BuiltinError> {
    if shape.numel() == 1 {
        return values
            .first()
            .copied()
            .map(Value::Double)
            .ok_or_else(|| mapping_error("reduction"));
    }
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn complex64_groups(
    name: &str,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Vec<ArrayComplex64>>, BuiltinError> {
    typed_groups(name, plan, context)
}

fn complex32_groups(
    name: &str,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Vec<ArrayComplex32>>, BuiltinError> {
    typed_groups(name, plan, context)
}

fn typed_groups<T>(
    name: &str,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Vec<T>>, BuiltinError> {
    let mut groups = reserved_vec(name, plan.output_length)?;
    let group_capacity = plan
        .selected
        .iter()
        .try_fold(1_u64, |total, dimension| {
            let extent = usize::try_from(*dimension)
                .ok()
                .and_then(|dimension| dimension.checked_sub(1))
                .and_then(|dimension| plan.input_dimensions.get(dimension))
                .copied()
                .unwrap_or(1);
            total.checked_mul(extent)
        })
        .ok_or_else(|| mapping_error(name))?;
    let group_capacity = usize::try_from(group_capacity).map_err(|_| mapping_error(name))?;
    for index in 0..plan.output_length {
        check_cancelled_at(context, index)?;
        groups.push(reserved_vec(name, group_capacity)?);
    }
    Ok(groups)
}

fn filled_acc64(
    name: &str,
    length: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Acc64>, BuiltinError> {
    filled_values(name, length, Acc64::default(), context)
}

fn filled_acc32(
    name: &str,
    length: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<Acc32>, BuiltinError> {
    filled_values(name, length, Acc32::default(), context)
}

fn filled_values<T: Clone>(
    name: &str,
    length: usize,
    value: T,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_vec(name, length)?;
    while output.len() < length {
        context.check_cancelled()?;
        let target = output
            .len()
            .saturating_add(CANCELLATION_CHECK_INTERVAL)
            .min(length);
        output.resize(target, value.clone());
    }
    Ok(output)
}

fn map_reserved<T, U, F>(
    name: &str,
    input: &[T],
    context: &BuiltinContext<'_>,
    mut transform: F,
) -> Result<Vec<U>, BuiltinError>
where
    F: FnMut(&T) -> U,
{
    let mut output = reserved_vec(name, input.len())?;
    for (index, value) in input.iter().enumerate() {
        check_cancelled_at(context, index)?;
        output.push(transform(value));
    }
    Ok(output)
}

fn reserved_vec<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate {length} elements"),
        )
    })?;
    Ok(output)
}

#[allow(clippy::cast_precision_loss)]
fn integer_component_f64(value: IntegerComponent) -> f64 {
    match value {
        IntegerComponent::Signed(value) => value as f64,
        IntegerComponent::Unsigned(value) => value as f64,
    }
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn dimension_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("dimension inputs to `{name}` must be unique positive integers"),
    )
}

fn option_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` received an unsupported or repeated option"),
    )
}

fn normalization_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "the normalization input to `std` or `var` must be empty, zero, or one",
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` computed an invalid checked reduction mapping"),
    )
}

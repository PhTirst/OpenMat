use std::cmp::Ordering;

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::core_statistics::{
    ReductionPlan, dimension_values, keyword, numeric_dimensions, output_offset, real_f64_output,
    reduction_plan, visit_f32, visit_f64, visit_logical,
};
use crate::{array_error, expect_argument_count_range, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArithmeticReduction {
    Sum,
    Product,
}

impl ArithmeticReduction {
    const fn name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Product => "prod",
        }
    }

    const fn identity_f64(self) -> ArrayComplex64 {
        match self {
            Self::Sum => ArrayComplex64::ZERO,
            Self::Product => ArrayComplex64::new(1.0, 0.0),
        }
    }

    const fn identity_f32(self) -> ArrayComplex32 {
        match self {
            Self::Sum => ArrayComplex32::ZERO,
            Self::Product => ArrayComplex32::new(1.0, 0.0),
        }
    }

    fn combine_f64(self, accumulator: &mut ArrayComplex64, value: ArrayComplex64) {
        match self {
            Self::Sum => *accumulator += value,
            Self::Product => *accumulator = *accumulator * value,
        }
    }

    fn combine_f32(self, accumulator: &mut ArrayComplex32, value: ArrayComplex32) {
        match self {
            Self::Sum => {
                accumulator.re += value.re;
                accumulator.im += value.im;
            }
            Self::Product => {
                let real = accumulator
                    .re
                    .mul_add(value.re, -(accumulator.im * value.im));
                let imaginary = accumulator.re.mul_add(value.im, accumulator.im * value.re);
                *accumulator = ArrayComplex32::new(real, imaginary);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReductionOutput {
    Default,
    Double,
    Native,
}

struct ArithmeticOptions {
    selection: Option<Vec<u64>>,
    omit_nan: bool,
    output: ReductionOutput,
}

pub(super) fn sum_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    arithmetic_reduction_builtin(ArithmeticReduction::Sum, arguments, context)
}

pub(super) fn prod_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    arithmetic_reduction_builtin(ArithmeticReduction::Product, arguments, context)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Extremum {
    Minimum,
    Maximum,
}

impl Extremum {
    const fn name(self) -> &'static str {
        match self {
            Self::Minimum => "min",
            Self::Maximum => "max",
        }
    }

    fn prefers(self, ordering: Ordering) -> bool {
        match self {
            Self::Minimum => ordering == Ordering::Less,
            Self::Maximum => ordering == Ordering::Greater,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MissingPolicy {
    Omit,
    Include,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComparisonMethod {
    Auto,
    Real,
    Absolute,
}

struct ExtremumOptions {
    selection: Option<Vec<u64>>,
    all: bool,
    linear: bool,
    missing: MissingPolicy,
    comparison: ComparisonMethod,
}

pub(super) fn min_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    extremum_builtin(Extremum::Minimum, arguments, context)
}

pub(super) fn max_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    extremum_builtin(Extremum::Maximum, arguments, context)
}

fn extremum_builtin(
    extremum: Extremum,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = extremum.name();
    expect_argument_count_range(name, arguments, 1, 8)?;
    context.check_cancelled()?;
    let reduction = arguments.len() == 1
        || arguments
            .get(1)
            .is_some_and(|value| is_empty_numeric(name, value));
    if reduction {
        expect_max_outputs(name, context, 2)?;
        let option_start = if arguments.len() > 1 { 2 } else { 1 };
        let options =
            extremum_options(name, &arguments[option_start..], option_start + 1, context)?;
        let dimensions = numeric_dimensions(name, &arguments[0])?;
        let plan = extremum_plan(name, dimensions, options.selection.as_deref(), context)?;
        if context.requested_outputs() > 1
            && plan.selected.len() > 1
            && !options.all
            && !options.linear
        {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` requires the linear option for an index over a dimension vector"),
            ));
        }
        let (value, index) =
            reduce_extremum_value(extremum, &arguments[0], &plan, &options, context)?;
        let mut outputs = vec![value];
        if context.requested_outputs() > 1 {
            outputs.push(index);
        }
        context.check_cancelled()?;
        Ok(outputs)
    } else {
        expect_max_outputs(name, context, 1)?;
        let options = elementwise_extremum_options(name, &arguments[2..], context)?;
        let output =
            elementwise_extremum_value(extremum, &arguments[0], &arguments[1], options, context)?;
        context.check_cancelled()?;
        Ok(vec![output])
    }
}

fn extremum_options(
    name: &str,
    arguments: &[Value],
    first_position: usize,
    context: &BuiltinContext<'_>,
) -> Result<ExtremumOptions, BuiltinError> {
    let mut options = ExtremumOptions {
        selection: None,
        all: false,
        linear: false,
        missing: MissingPolicy::Omit,
        comparison: ComparisonMethod::Auto,
    };
    let mut offset = 0;
    while offset < arguments.len() {
        check_cancelled_at(context, offset)?;
        match keyword(&arguments[offset]).as_deref() {
            Some("all") => {
                set_selection(name, &mut options.selection, Vec::new())?;
                options.all = true;
                offset += 1;
            }
            Some("linear") => {
                options.linear = true;
                offset += 1;
            }
            Some("omitnan" | "omitmissing") => {
                options.missing = MissingPolicy::Omit;
                offset += 1;
            }
            Some("includenan" | "includemissing") => {
                options.missing = MissingPolicy::Include;
                offset += 1;
            }
            Some("comparisonmethod") => {
                let value = arguments
                    .get(offset + 1)
                    .ok_or_else(|| option_error(name))?;
                options.comparison = comparison_method(name, value)?;
                offset += 2;
            }
            Some(_) => return Err(option_error(name)),
            None => {
                let dimensions =
                    dimension_values(name, first_position + offset, &arguments[offset], context)?;
                set_selection(name, &mut options.selection, dimensions)?;
                offset += 1;
            }
        }
    }
    Ok(options)
}

fn elementwise_extremum_options(
    name: &str,
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<(MissingPolicy, ComparisonMethod), BuiltinError> {
    let mut missing = MissingPolicy::Omit;
    let mut comparison = ComparisonMethod::Auto;
    let mut offset = 0;
    while offset < arguments.len() {
        check_cancelled_at(context, offset)?;
        match keyword(&arguments[offset]).as_deref() {
            Some("omitnan" | "omitmissing") => {
                missing = MissingPolicy::Omit;
                offset += 1;
            }
            Some("includenan" | "includemissing") => {
                missing = MissingPolicy::Include;
                offset += 1;
            }
            Some("comparisonmethod") => {
                let value = arguments
                    .get(offset + 1)
                    .ok_or_else(|| option_error(name))?;
                comparison = comparison_method(name, value)?;
                offset += 2;
            }
            _ => return Err(option_error(name)),
        }
    }
    Ok((missing, comparison))
}

fn comparison_method(name: &str, value: &Value) -> Result<ComparisonMethod, BuiltinError> {
    match keyword(value).as_deref() {
        Some("auto") => Ok(ComparisonMethod::Auto),
        Some("real") => Ok(ComparisonMethod::Real),
        Some("abs") => Ok(ComparisonMethod::Absolute),
        _ => Err(option_error(name)),
    }
}

fn is_empty_numeric(name: &str, value: &Value) -> bool {
    value.numel() == Some(0) && numeric_dimensions(name, value).is_ok()
}

fn extremum_plan(
    name: &str,
    dimensions: &[u64],
    selection: Option<&[u64]>,
    context: &BuiltinContext<'_>,
) -> Result<ReductionPlan, BuiltinError> {
    let mut selected = match selection {
        Some([]) => all_dimensions(dimensions)?,
        Some(selection) => selection.to_vec(),
        None => vec![default_dimension(dimensions)],
    };
    selected.sort_unstable();
    let mut output_dimensions = reserved_values(name, dimensions.len())?;
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

fn all_dimensions(dimensions: &[u64]) -> Result<Vec<u64>, BuiltinError> {
    let mut selected = reserved_values("reduction", dimensions.len())?;
    for index in 0..dimensions.len() {
        selected.push(
            u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| mapping_error("reduction"))?,
        );
    }
    Ok(selected)
}

fn default_dimension(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|index| u64::try_from(index).ok())
        .and_then(|index| index.checked_add(1))
        .unwrap_or(1)
}

struct ReducedValues<T> {
    values: Vec<T>,
    indices: Vec<f64>,
}

fn reduce_typed<T: Copy>(
    extremum: Extremum,
    values: &[T],
    plan: &ReductionPlan,
    options: &ExtremumOptions,
    context: &BuiltinContext<'_>,
    is_missing: impl Fn(T) -> bool,
    compare: impl Fn(T, T, ComparisonMethod) -> Ordering,
) -> Result<ReducedValues<T>, BuiltinError> {
    let name = extremum.name();
    let mut selected = filled_values(name, plan.output_length, None::<(T, usize)>)?;
    let mut fallback = filled_values(name, plan.output_length, None::<(T, usize)>)?;
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let target = output_offset(name, index, plan)?;
        if is_missing(value) {
            if options.missing == MissingPolicy::Include {
                let slot = selected
                    .get_mut(target)
                    .ok_or_else(|| mapping_error(name))?;
                if slot.is_none_or(|(current, _)| !is_missing(current)) {
                    *slot = Some((value, index));
                }
            } else {
                let slot = fallback
                    .get_mut(target)
                    .ok_or_else(|| mapping_error(name))?;
                if slot.is_none() {
                    *slot = Some((value, index));
                }
            }
            continue;
        }
        let slot = selected
            .get_mut(target)
            .ok_or_else(|| mapping_error(name))?;
        if slot.is_none_or(|(current, _)| {
            !is_missing(current) && extremum.prefers(compare(value, current, options.comparison))
        }) {
            *slot = Some((value, index));
        }
    }
    let mut output = reserved_values(name, plan.output_length)?;
    let mut indices = reserved_values(name, plan.output_length)?;
    for (slot, fallback) in selected.into_iter().zip(fallback) {
        let (value, index) = slot.or(fallback).ok_or_else(|| mapping_error(name))?;
        output.push(value);
        indices.push(reduced_index(index, plan, options)?);
    }
    Ok(ReducedValues {
        values: output,
        indices,
    })
}

#[allow(clippy::cast_precision_loss)]
fn reduced_index(
    input_offset: usize,
    plan: &ReductionPlan,
    options: &ExtremumOptions,
) -> Result<f64, BuiltinError> {
    if options.linear || options.all || plan.selected.len() > 1 {
        return u64::try_from(input_offset)
            .ok()
            .and_then(|index| index.checked_add(1))
            .map(|index| index as f64)
            .ok_or_else(|| mapping_error("reduction"));
    }
    let dimension = plan.selected.first().copied().unwrap_or(1);
    let axis =
        usize::try_from(dimension.saturating_sub(1)).map_err(|_| mapping_error("reduction"))?;
    let stride = plan
        .input_dimensions
        .iter()
        .take(axis)
        .try_fold(1_u64, |stride, extent| stride.checked_mul(*extent))
        .ok_or_else(|| mapping_error("reduction"))?;
    let extent = plan.input_dimensions.get(axis).copied().unwrap_or(1);
    let offset = u64::try_from(input_offset).map_err(|_| mapping_error("reduction"))?;
    Ok(((offset / stride) % extent.max(1) + 1) as f64)
}

fn reduce_extremum_value(
    extremum: Extremum,
    input: &Value,
    plan: &ReductionPlan,
    options: &ExtremumOptions,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value), BuiltinError> {
    match input {
        Value::Double(value) => {
            reduce_double_extremum(extremum, &[*value], false, true, plan, options, context)
        }
        Value::Complex(value) => reduce_double_extremum(
            extremum,
            &[ArrayComplex64::new(value.real, value.imaginary)],
            true,
            true,
            plan,
            options,
            context,
        ),
        Value::Array(ArrayData::F64(array)) => reduce_double_extremum(
            extremum,
            array.as_slice(),
            false,
            false,
            plan,
            options,
            context,
        ),
        Value::Array(ArrayData::ComplexF64(array)) => reduce_double_extremum(
            extremum,
            array.as_slice(),
            true,
            false,
            plan,
            options,
            context,
        ),
        Value::Array(ArrayData::F32(array)) => {
            reduce_single_extremum(extremum, array.as_slice(), false, plan, options, context)
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            reduce_single_extremum(extremum, array.as_slice(), true, plan, options, context)
        }
        Value::Logical(value) => {
            reduce_logical_extremum(extremum, &[*value], true, plan, options, context)
        }
        Value::Array(ArrayData::Logical(array)) => {
            let values = map_values(extremum.name(), array.as_slice(), context, |value| {
                value.get()
            })?;
            reduce_logical_extremum(extremum, &values, false, plan, options, context)
        }
        Value::Array(ArrayData::Char(array)) => {
            reduce_char_extremum(extremum, array, plan, options, context)
        }
        Value::Array(ArrayData::Integer(array)) => {
            reduce_integer_extremum(extremum, array, plan, options, context)
        }
        value => Err(type_error(
            extremum.name(),
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

trait IntoComplex64 {
    fn into_complex64(self) -> ArrayComplex64;
}

impl IntoComplex64 for f64 {
    fn into_complex64(self) -> ArrayComplex64 {
        ArrayComplex64::new(self, 0.0)
    }
}

impl IntoComplex64 for ArrayComplex64 {
    fn into_complex64(self) -> ArrayComplex64 {
        self
    }
}

trait IntoComplex32 {
    fn into_complex32(self) -> ArrayComplex32;
}

impl IntoComplex32 for f32 {
    fn into_complex32(self) -> ArrayComplex32 {
        ArrayComplex32::new(self, 0.0)
    }
}

impl IntoComplex32 for ArrayComplex32 {
    fn into_complex32(self) -> ArrayComplex32 {
        self
    }
}

fn reduce_double_extremum<T: Copy + IntoComplex64>(
    extremum: Extremum,
    input: &[T],
    complex: bool,
    scalar_variant: bool,
    plan: &ReductionPlan,
    options: &ExtremumOptions,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value), BuiltinError> {
    let values = map_values(extremum.name(), input, context, |value| {
        (*value).into_complex64()
    })?;
    let reduced = reduce_typed(
        extremum,
        &values,
        plan,
        options,
        context,
        complex64_is_nan,
        |left, right, method| compare_complex64(left, right, method, complex),
    )?;
    Ok((
        reduced_double_output(plan, reduced.values, complex, scalar_variant)?,
        index_output(plan, reduced.indices)?,
    ))
}

fn reduce_single_extremum<T: Copy + IntoComplex32>(
    extremum: Extremum,
    input: &[T],
    complex: bool,
    plan: &ReductionPlan,
    options: &ExtremumOptions,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value), BuiltinError> {
    let values = map_values(extremum.name(), input, context, |value| {
        (*value).into_complex32()
    })?;
    let reduced = reduce_typed(
        extremum,
        &values,
        plan,
        options,
        context,
        complex32_is_nan,
        |left, right, method| compare_complex32(left, right, method, complex),
    )?;
    Ok((
        reduced_single_output(plan, reduced.values, complex)?,
        index_output(plan, reduced.indices)?,
    ))
}

fn reduce_logical_extremum(
    extremum: Extremum,
    input: &[bool],
    scalar_variant: bool,
    plan: &ReductionPlan,
    options: &ExtremumOptions,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value), BuiltinError> {
    require_automatic_comparison(extremum.name(), options.comparison)?;
    let reduced = reduce_typed(
        extremum,
        input,
        plan,
        options,
        context,
        |_| false,
        |left, right, _| left.cmp(&right),
    )?;
    Ok((
        reduced_logical_output(plan, reduced.values, scalar_variant)?,
        index_output(plan, reduced.indices)?,
    ))
}

fn reduce_char_extremum(
    extremum: Extremum,
    input: &DenseArray<CharCodeUnit>,
    plan: &ReductionPlan,
    options: &ExtremumOptions,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value), BuiltinError> {
    require_automatic_comparison(extremum.name(), options.comparison)?;
    let values = map_values(extremum.name(), input.as_slice(), context, |value| {
        f64::from(value.get())
    })?;
    reduce_double_extremum(extremum, &values, false, false, plan, options, context)
}

fn reduce_integer_extremum(
    extremum: Extremum,
    input: &IntegerArrayData,
    plan: &ReductionPlan,
    options: &ExtremumOptions,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value), BuiltinError> {
    require_automatic_comparison(extremum.name(), options.comparison)?;
    macro_rules! reduce_variant {
        ($array:expr) => {{
            let reduced = reduce_typed(
                extremum,
                $array.as_slice(),
                plan,
                options,
                context,
                |_| false,
                |left, right, _| left.cmp(&right),
            )?;
            let value = DenseArray::from_vec(plan.output_shape.clone(), reduced.values)
                .map(IntegerArrayData::from_typed)
                .map(ArrayData::Integer)
                .map(Value::Array)
                .map_err(|error| array_error(&error))?;
            Ok((value, index_output(plan, reduced.indices)?))
        }};
    }
    match input {
        IntegerArrayData::I8(array) => reduce_variant!(array),
        IntegerArrayData::U8(array) => reduce_variant!(array),
        IntegerArrayData::I16(array) => reduce_variant!(array),
        IntegerArrayData::U16(array) => reduce_variant!(array),
        IntegerArrayData::I32(array) => reduce_variant!(array),
        IntegerArrayData::U32(array) => reduce_variant!(array),
        IntegerArrayData::I64(array) => reduce_variant!(array),
        IntegerArrayData::U64(array) => reduce_variant!(array),
        _ => Err(type_error(
            extremum.name(),
            1,
            "real integer array",
            &Value::Array(ArrayData::Integer(input.clone())),
        )),
    }
}

fn reduced_double_output(
    plan: &ReductionPlan,
    values: Vec<ArrayComplex64>,
    complex: bool,
    scalar_variant: bool,
) -> Result<Value, BuiltinError> {
    if scalar_variant && plan.output_shape.numel() == 1 {
        let value = values
            .first()
            .copied()
            .ok_or_else(|| mapping_error("reduction"))?;
        return Ok(if complex {
            Value::Complex(openmat_value::Complex64::from(value))
        } else {
            Value::Double(value.re)
        });
    }
    if complex {
        DenseArray::from_vec(plan.output_shape.clone(), values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        DenseArray::from_vec(
            plan.output_shape.clone(),
            values.into_iter().map(|value| value.re).collect(),
        )
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
    }
}

fn reduced_single_output(
    plan: &ReductionPlan,
    values: Vec<ArrayComplex32>,
    complex: bool,
) -> Result<Value, BuiltinError> {
    let output = if complex {
        ArrayData::ComplexF32(
            DenseArray::from_vec(plan.output_shape.clone(), values)
                .map_err(|error| array_error(&error))?,
        )
    } else {
        ArrayData::F32(
            DenseArray::from_vec(
                plan.output_shape.clone(),
                values.into_iter().map(|value| value.re).collect(),
            )
            .map_err(|error| array_error(&error))?,
        )
    };
    Ok(Value::Array(output))
}

fn reduced_logical_output(
    plan: &ReductionPlan,
    values: Vec<bool>,
    scalar_variant: bool,
) -> Result<Value, BuiltinError> {
    if scalar_variant && plan.output_shape.numel() == 1 {
        return values
            .first()
            .copied()
            .map(Value::Logical)
            .ok_or_else(|| mapping_error("reduction"));
    }
    DenseArray::from_vec(
        plan.output_shape.clone(),
        values.into_iter().map(Logical::from).collect(),
    )
    .map(ArrayData::Logical)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

fn index_output(plan: &ReductionPlan, values: Vec<f64>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(plan.output_shape.clone(), values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn require_automatic_comparison(
    name: &str,
    comparison: ComparisonMethod,
) -> Result<(), BuiltinError> {
    if comparison == ComparisonMethod::Auto {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` comparison methods require floating-point input"),
        ))
    }
}

fn compare_complex64(
    left: ArrayComplex64,
    right: ArrayComplex64,
    method: ComparisonMethod,
    complex: bool,
) -> Ordering {
    if method == ComparisonMethod::Real || method == ComparisonMethod::Auto && !complex {
        return left.re.partial_cmp(&right.re).unwrap_or(Ordering::Equal);
    }
    left.re
        .hypot(left.im)
        .partial_cmp(&right.re.hypot(right.im))
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            left.im
                .atan2(left.re)
                .partial_cmp(&right.im.atan2(right.re))
                .unwrap_or(Ordering::Equal)
        })
}

fn compare_complex32(
    left: ArrayComplex32,
    right: ArrayComplex32,
    method: ComparisonMethod,
    complex: bool,
) -> Ordering {
    if method == ComparisonMethod::Real || method == ComparisonMethod::Auto && !complex {
        return left.re.partial_cmp(&right.re).unwrap_or(Ordering::Equal);
    }
    left.re
        .hypot(left.im)
        .partial_cmp(&right.re.hypot(right.im))
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            left.im
                .atan2(left.re)
                .partial_cmp(&right.im.atan2(right.re))
                .unwrap_or(Ordering::Equal)
        })
}

struct Float64Data {
    shape: Shape,
    values: Vec<ArrayComplex64>,
    complex: bool,
    scalar_variant: bool,
}

struct Float32Data {
    shape: Shape,
    values: Vec<ArrayComplex32>,
    complex: bool,
}

fn elementwise_extremum_value(
    extremum: Extremum,
    left: &Value,
    right: &Value,
    options: (MissingPolicy, ComparisonMethod),
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if matches!(
        left,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    ) || matches!(
        right,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    ) {
        return elementwise_single_extremum(extremum, left, right, options, context);
    }
    if is_double_numeric(left) && is_double_numeric(right) {
        return elementwise_double_extremum(extremum, left, right, options, context);
    }
    if is_logical_value(left) && is_logical_value(right) {
        let (left_shape, left_values, left_scalar) = logical_data(extremum.name(), left, context)?;
        let (right_shape, right_values, right_scalar) =
            logical_data(extremum.name(), right, context)?;
        return elementwise_logical_extremum(
            extremum,
            &left_values,
            left_shape.dimensions(),
            &right_values,
            right_shape.dimensions(),
            left_scalar && right_scalar,
            options,
            context,
        );
    }
    match (left, right) {
        (Value::Array(ArrayData::Char(left)), Value::Array(ArrayData::Char(right))) => {
            require_automatic_comparison(extremum.name(), options.1)?;
            let left_values = map_values(extremum.name(), left.as_slice(), context, |value| {
                f64::from(value.get())
            })?;
            let right_values = map_values(extremum.name(), right.as_slice(), context, |value| {
                f64::from(value.get())
            })?;
            let left = Float64Data {
                shape: left.shape().clone(),
                values: left_values
                    .into_iter()
                    .map(|value| ArrayComplex64::new(value, 0.0))
                    .collect(),
                complex: false,
                scalar_variant: false,
            };
            let right = Float64Data {
                shape: right.shape().clone(),
                values: right_values
                    .into_iter()
                    .map(|value| ArrayComplex64::new(value, 0.0))
                    .collect(),
                complex: false,
                scalar_variant: false,
            };
            elementwise_double_data(extremum, &left, &right, options, context)
        }
        (Value::Array(ArrayData::Integer(left)), Value::Array(ArrayData::Integer(right))) => {
            elementwise_integer_extremum(extremum, left, right, options, context)
        }
        (Value::Array(ArrayData::Integer(array)), Value::Double(scalar)) => {
            elementwise_integer_scalar_extremum(extremum, array, *scalar, false, options, context)
        }
        (Value::Double(scalar), Value::Array(ArrayData::Integer(array))) => {
            elementwise_integer_scalar_extremum(extremum, array, *scalar, true, options, context)
        }
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!(
                "inputs to `{}` have incompatible numeric classes",
                extremum.name()
            ),
        )),
    }
}

const fn is_double_numeric(value: &Value) -> bool {
    matches!(
        value,
        Value::Double(_)
            | Value::Complex(_)
            | Value::Array(ArrayData::F64(_) | ArrayData::ComplexF64(_))
    )
}

const fn is_logical_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Logical(_) | Value::Array(ArrayData::Logical(_))
    )
}

fn logical_data(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<(Shape, Vec<bool>, bool), BuiltinError> {
    match value {
        Value::Logical(value) => Ok((
            Shape::new([1, 1]).map_err(|error| array_error(&error))?,
            vec![*value],
            true,
        )),
        Value::Array(ArrayData::Logical(array)) => Ok((
            array.shape().clone(),
            map_values(name, array.as_slice(), context, |value| value.get())?,
            false,
        )),
        value => Err(type_error(name, 1, "logical array", value)),
    }
}

fn elementwise_double_extremum(
    extremum: Extremum,
    left: &Value,
    right: &Value,
    options: (MissingPolicy, ComparisonMethod),
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let left = float64_data(extremum.name(), left, context)?;
    let right = float64_data(extremum.name(), right, context)?;
    elementwise_double_data(extremum, &left, &right, options, context)
}

fn elementwise_double_data(
    extremum: Extremum,
    left: &Float64Data,
    right: &Float64Data,
    options: (MissingPolicy, ComparisonMethod),
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let plan = BroadcastPlan::new(
        extremum.name(),
        left.shape.dimensions(),
        right.shape.dimensions(),
    )?;
    let complex = left.complex || right.complex;
    let values = elementwise_typed(
        extremum,
        &left.values,
        &right.values,
        &plan,
        options.0,
        context,
        complex64_is_nan,
        |left, right| compare_complex64(left, right, options.1, complex),
    )?;
    elementwise_double_output(
        plan.shape,
        values,
        complex,
        left.scalar_variant && right.scalar_variant,
    )
}

fn elementwise_single_extremum(
    extremum: Extremum,
    left: &Value,
    right: &Value,
    options: (MissingPolicy, ComparisonMethod),
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let left = float32_data(extremum.name(), left, context)?;
    let right = float32_data(extremum.name(), right, context)?;
    let plan = BroadcastPlan::new(
        extremum.name(),
        left.shape.dimensions(),
        right.shape.dimensions(),
    )?;
    let complex = left.complex || right.complex;
    let values = elementwise_typed(
        extremum,
        &left.values,
        &right.values,
        &plan,
        options.0,
        context,
        complex32_is_nan,
        |left, right| compare_complex32(left, right, options.1, complex),
    )?;
    elementwise_single_output(plan.shape, values, complex)
}

#[allow(clippy::too_many_arguments)]
fn elementwise_logical_extremum(
    extremum: Extremum,
    left: &[bool],
    left_dimensions: &[u64],
    right: &[bool],
    right_dimensions: &[u64],
    scalar_variant: bool,
    options: (MissingPolicy, ComparisonMethod),
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    require_automatic_comparison(extremum.name(), options.1)?;
    let plan = BroadcastPlan::new(extremum.name(), left_dimensions, right_dimensions)?;
    let values = elementwise_typed(
        extremum,
        left,
        right,
        &plan,
        options.0,
        context,
        |_| false,
        |left, right| left.cmp(&right),
    )?;
    if scalar_variant && plan.shape.numel() == 1 {
        return values
            .first()
            .copied()
            .map(Value::Logical)
            .ok_or_else(|| mapping_error(extremum.name()));
    }
    DenseArray::from_vec(plan.shape, values.into_iter().map(Logical::from).collect())
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn elementwise_integer_extremum(
    extremum: Extremum,
    left: &IntegerArrayData,
    right: &IntegerArrayData,
    options: (MissingPolicy, ComparisonMethod),
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    require_automatic_comparison(extremum.name(), options.1)?;
    macro_rules! same_variant {
        ($left:expr, $right:expr) => {{
            let plan = BroadcastPlan::new(
                extremum.name(),
                $left.shape().dimensions(),
                $right.shape().dimensions(),
            )?;
            let values = elementwise_typed(
                extremum,
                $left.as_slice(),
                $right.as_slice(),
                &plan,
                options.0,
                context,
                |_| false,
                |left, right| left.cmp(&right),
            )?;
            DenseArray::from_vec(plan.shape, values)
                .map(IntegerArrayData::from_typed)
                .map(ArrayData::Integer)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }};
    }
    match (left, right) {
        (IntegerArrayData::I8(left), IntegerArrayData::I8(right)) => same_variant!(left, right),
        (IntegerArrayData::U8(left), IntegerArrayData::U8(right)) => same_variant!(left, right),
        (IntegerArrayData::I16(left), IntegerArrayData::I16(right)) => same_variant!(left, right),
        (IntegerArrayData::U16(left), IntegerArrayData::U16(right)) => same_variant!(left, right),
        (IntegerArrayData::I32(left), IntegerArrayData::I32(right)) => same_variant!(left, right),
        (IntegerArrayData::U32(left), IntegerArrayData::U32(right)) => same_variant!(left, right),
        (IntegerArrayData::I64(left), IntegerArrayData::I64(right)) => same_variant!(left, right),
        (IntegerArrayData::U64(left), IntegerArrayData::U64(right)) => same_variant!(left, right),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!(
                "inputs to `{}` must use the same real integer class",
                extremum.name()
            ),
        )),
    }
}

fn elementwise_integer_scalar_extremum(
    extremum: Extremum,
    array: &IntegerArrayData,
    scalar: f64,
    scalar_left: bool,
    options: (MissingPolicy, ComparisonMethod),
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    require_automatic_comparison(extremum.name(), options.1)?;
    macro_rules! signed_variant {
        ($array:expr, $kind:ty) => {{
            let scalar =
                rounded_signed_scalar(scalar, i128::from(<$kind>::MIN), i128::from(<$kind>::MAX));
            let scalar = <$kind>::try_from(scalar).map_err(|_| mapping_error(extremum.name()))?;
            integer_scalar_output(extremum, $array, scalar, scalar_left, options.0, context)
        }};
    }
    macro_rules! unsigned_variant {
        ($array:expr, $kind:ty) => {{
            let scalar = rounded_unsigned_scalar(scalar, u128::from(<$kind>::MAX));
            let scalar = <$kind>::try_from(scalar).map_err(|_| mapping_error(extremum.name()))?;
            integer_scalar_output(extremum, $array, scalar, scalar_left, options.0, context)
        }};
    }
    match array {
        IntegerArrayData::I8(array) => signed_variant!(array, i8),
        IntegerArrayData::U8(array) => unsigned_variant!(array, u8),
        IntegerArrayData::I16(array) => signed_variant!(array, i16),
        IntegerArrayData::U16(array) => unsigned_variant!(array, u16),
        IntegerArrayData::I32(array) => signed_variant!(array, i32),
        IntegerArrayData::U32(array) => unsigned_variant!(array, u32),
        IntegerArrayData::I64(array) => signed_variant!(array, i64),
        IntegerArrayData::U64(array) => unsigned_variant!(array, u64),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!(
                "`{}` does not accept complex integer input",
                extremum.name()
            ),
        )),
    }
}

fn integer_scalar_output<T: openmat_array::IntegerElement + Ord + Copy>(
    extremum: Extremum,
    array: &DenseArray<T>,
    scalar: T,
    scalar_left: bool,
    missing: MissingPolicy,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let scalar_shape = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    let plan = if scalar_left {
        BroadcastPlan::new(
            extremum.name(),
            scalar_shape.dimensions(),
            array.shape().dimensions(),
        )?
    } else {
        BroadcastPlan::new(
            extremum.name(),
            array.shape().dimensions(),
            scalar_shape.dimensions(),
        )?
    };
    let scalar_values = [scalar];
    let (left, right) = if scalar_left {
        (&scalar_values[..], array.as_slice())
    } else {
        (array.as_slice(), &scalar_values[..])
    };
    let values = elementwise_typed(
        extremum,
        left,
        right,
        &plan,
        missing,
        context,
        |_| false,
        |left, right| left.cmp(&right),
    )?;
    DenseArray::from_vec(plan.shape, values)
        .map(IntegerArrayData::from_typed)
        .map(ArrayData::Integer)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_possible_truncation)]
fn rounded_signed_scalar(value: f64, minimum: i128, maximum: i128) -> i128 {
    if value.is_nan() {
        return 0;
    }
    if value == f64::INFINITY {
        return maximum;
    }
    if value == f64::NEG_INFINITY {
        return minimum;
    }
    let value = value.round();
    #[allow(clippy::cast_precision_loss)]
    if value <= minimum as f64 {
        minimum
    } else {
        #[allow(clippy::cast_precision_loss)]
        if value >= maximum as f64 {
            maximum
        } else {
            value as i128
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rounded_unsigned_scalar(value: f64, maximum: u128) -> u128 {
    if value.is_nan() || value == f64::NEG_INFINITY || value <= 0.0 {
        return 0;
    }
    if value == f64::INFINITY {
        return maximum;
    }
    let value = value.round();
    #[allow(clippy::cast_precision_loss)]
    if value >= maximum as f64 {
        maximum
    } else {
        value as u128
    }
}

fn float64_data(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Float64Data, BuiltinError> {
    let scalar_shape = || Shape::new([1, 1]).map_err(|error| array_error(&error));
    match value {
        Value::Double(value) => Ok(Float64Data {
            shape: scalar_shape()?,
            values: vec![ArrayComplex64::new(*value, 0.0)],
            complex: false,
            scalar_variant: true,
        }),
        Value::Complex(value) => Ok(Float64Data {
            shape: scalar_shape()?,
            values: vec![ArrayComplex64::new(value.real, value.imaginary)],
            complex: true,
            scalar_variant: true,
        }),
        Value::Array(ArrayData::F64(array)) => Ok(Float64Data {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| {
                ArrayComplex64::new(*value, 0.0)
            })?,
            complex: false,
            scalar_variant: false,
        }),
        Value::Array(ArrayData::ComplexF64(array)) => Ok(Float64Data {
            shape: array.shape().clone(),
            values: clone_values(name, array.as_slice(), context)?,
            complex: true,
            scalar_variant: false,
        }),
        value => Err(type_error(name, 1, "double array", value)),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn float32_data(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Float32Data, BuiltinError> {
    let scalar_shape = || Shape::new([1, 1]).map_err(|error| array_error(&error));
    match value {
        Value::Double(value) => Ok(Float32Data {
            shape: scalar_shape()?,
            values: vec![ArrayComplex32::new(*value as f32, 0.0)],
            complex: false,
        }),
        Value::Complex(value) => Ok(Float32Data {
            shape: scalar_shape()?,
            values: vec![ArrayComplex32::new(
                value.real as f32,
                value.imaginary as f32,
            )],
            complex: true,
        }),
        Value::Array(ArrayData::F64(array)) => Ok(Float32Data {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| {
                ArrayComplex32::new(*value as f32, 0.0)
            })?,
            complex: false,
        }),
        Value::Array(ArrayData::ComplexF64(array)) => Ok(Float32Data {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| {
                ArrayComplex32::new(value.re as f32, value.im as f32)
            })?,
            complex: true,
        }),
        Value::Array(ArrayData::F32(array)) => Ok(Float32Data {
            shape: array.shape().clone(),
            values: map_values(name, array.as_slice(), context, |value| {
                ArrayComplex32::new(*value, 0.0)
            })?,
            complex: false,
        }),
        Value::Array(ArrayData::ComplexF32(array)) => Ok(Float32Data {
            shape: array.shape().clone(),
            values: clone_values(name, array.as_slice(), context)?,
            complex: true,
        }),
        value => Err(type_error(name, 1, "single or double array", value)),
    }
}

fn elementwise_double_output(
    shape: Shape,
    values: Vec<ArrayComplex64>,
    complex: bool,
    scalar_variant: bool,
) -> Result<Value, BuiltinError> {
    if scalar_variant && shape.numel() == 1 {
        let value = values
            .first()
            .copied()
            .ok_or_else(|| mapping_error("extremum"))?;
        return Ok(if complex {
            Value::Complex(openmat_value::Complex64::from(value))
        } else {
            Value::Double(value.re)
        });
    }
    if complex {
        DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        DenseArray::from_vec(shape, values.into_iter().map(|value| value.re).collect())
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

fn elementwise_single_output(
    shape: Shape,
    values: Vec<ArrayComplex32>,
    complex: bool,
) -> Result<Value, BuiltinError> {
    let output = if complex {
        ArrayData::ComplexF32(
            DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?,
        )
    } else {
        ArrayData::F32(
            DenseArray::from_vec(shape, values.into_iter().map(|value| value.re).collect())
                .map_err(|error| array_error(&error))?,
        )
    };
    Ok(Value::Array(output))
}

struct BroadcastPlan {
    shape: Shape,
    left_dimensions: Vec<u64>,
    right_dimensions: Vec<u64>,
}

impl BroadcastPlan {
    fn new(name: &str, left: &[u64], right: &[u64]) -> Result<Self, BuiltinError> {
        let rank = left.len().max(right.len()).max(2);
        let mut dimensions = reserved_values(name, rank)?;
        for axis in 0..rank {
            let left_extent = left.get(axis).copied().unwrap_or(1);
            let right_extent = right.get(axis).copied().unwrap_or(1);
            let extent = if left_extent == right_extent {
                left_extent
            } else if left_extent == 1 {
                right_extent
            } else if right_extent == 1 {
                left_extent
            } else {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("inputs to `{name}` have incompatible dimensions"),
                ));
            };
            dimensions.push(extent);
        }
        Ok(Self {
            shape: Shape::new(dimensions).map_err(|error| array_error(&error))?,
            left_dimensions: left.to_vec(),
            right_dimensions: right.to_vec(),
        })
    }

    fn offsets(&self, output: usize) -> Result<(usize, usize), BuiltinError> {
        Ok((
            broadcast_offset(output, self.shape.dimensions(), &self.left_dimensions)?,
            broadcast_offset(output, self.shape.dimensions(), &self.right_dimensions)?,
        ))
    }
}

fn broadcast_offset(
    output: usize,
    output_dimensions: &[u64],
    input_dimensions: &[u64],
) -> Result<usize, BuiltinError> {
    let mut remainder = u64::try_from(output).map_err(|_| mapping_error("extremum"))?;
    let mut input_offset = 0_u64;
    let mut input_stride = 1_u64;
    for (axis, output_extent) in output_dimensions.iter().copied().enumerate() {
        let coordinate = if output_extent == 0 {
            0
        } else {
            let coordinate = remainder % output_extent;
            remainder /= output_extent;
            coordinate
        };
        let input_extent = input_dimensions.get(axis).copied().unwrap_or(1);
        let input_coordinate = if input_extent == 1 { 0 } else { coordinate };
        input_offset = input_coordinate
            .checked_mul(input_stride)
            .and_then(|value| input_offset.checked_add(value))
            .ok_or_else(|| mapping_error("extremum"))?;
        input_stride = input_stride
            .checked_mul(input_extent)
            .ok_or_else(|| mapping_error("extremum"))?;
    }
    usize::try_from(input_offset).map_err(|_| mapping_error("extremum"))
}

#[allow(clippy::too_many_arguments)]
fn elementwise_typed<T: Copy>(
    extremum: Extremum,
    left: &[T],
    right: &[T],
    plan: &BroadcastPlan,
    missing: MissingPolicy,
    context: &BuiltinContext<'_>,
    is_missing: impl Fn(T) -> bool,
    compare: impl Fn(T, T) -> Ordering,
) -> Result<Vec<T>, BuiltinError> {
    let length = usize::try_from(plan.shape.numel()).map_err(|_| mapping_error(extremum.name()))?;
    let mut output = reserved_values(extremum.name(), length)?;
    for index in 0..length {
        check_cancelled_at(context, index)?;
        let (left_offset, right_offset) = plan.offsets(index)?;
        let left = *left
            .get(left_offset)
            .ok_or_else(|| mapping_error(extremum.name()))?;
        let right = *right
            .get(right_offset)
            .ok_or_else(|| mapping_error(extremum.name()))?;
        let left_missing = is_missing(left);
        let right_missing = is_missing(right);
        let value = if left_missing || right_missing {
            match missing {
                MissingPolicy::Include => {
                    if left_missing {
                        left
                    } else {
                        right
                    }
                }
                MissingPolicy::Omit => {
                    if left_missing {
                        right
                    } else {
                        left
                    }
                }
            }
        } else if extremum.prefers(compare(right, left)) {
            right
        } else {
            left
        };
        output.push(value);
    }
    Ok(output)
}

fn map_values<T, U>(
    name: &str,
    values: &[T],
    context: &BuiltinContext<'_>,
    mut map: impl FnMut(&T) -> U,
) -> Result<Vec<U>, BuiltinError> {
    let mut output = reserved_values(name, values.len())?;
    for (index, value) in values.iter().enumerate() {
        check_cancelled_at(context, index)?;
        output.push(map(value));
    }
    Ok(output)
}

fn clone_values<T: Clone>(
    name: &str,
    values: &[T],
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    map_values(name, values, context, Clone::clone)
}

fn arithmetic_reduction_builtin(
    reduction: ArithmeticReduction,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = reduction.name();
    expect_argument_count_range(name, arguments, 1, 4)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions(name, &arguments[0])?;
    let options = arithmetic_options(name, arguments, context)?;
    let plan = reduction_plan(name, dimensions, options.selection, context)?;
    let output = match (&arguments[0], options.output) {
        (Value::Array(ArrayData::Integer(array)), ReductionOutput::Native) => {
            native_integer_reduction(reduction, array, &plan, context)?
        }
        (Value::Logical(_) | Value::Array(ArrayData::Logical(_)), ReductionOutput::Native) => {
            native_logical_reduction(reduction, &arguments[0], &plan, context)?
        }
        (Value::Array(ArrayData::Char(_)), ReductionOutput::Native) => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` cannot use native accumulation for char input"),
            ));
        }
        (
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_)),
            ReductionOutput::Default | ReductionOutput::Native,
        ) => single_reduction(reduction, &arguments[0], &plan, options.omit_nan, context)?,
        _ => double_reduction(reduction, &arguments[0], &plan, options.omit_nan, context)?,
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn arithmetic_options(
    name: &str,
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<ArithmeticOptions, BuiltinError> {
    let mut selection = None;
    let mut omit_nan = false;
    let mut output = ReductionOutput::Default;
    for (offset, value) in arguments[1..].iter().enumerate() {
        check_cancelled_at(context, offset)?;
        match keyword(value).as_deref() {
            Some("all") => set_selection(name, &mut selection, Vec::new())?,
            Some("omitnan" | "omitmissing") => omit_nan = true,
            Some("includenan" | "includemissing") => omit_nan = false,
            Some("default") => output = ReductionOutput::Default,
            Some("double") => output = ReductionOutput::Double,
            Some("native") => output = ReductionOutput::Native,
            Some(_) => return Err(option_error(name)),
            None => {
                let dimensions = dimension_values(name, offset + 2, value, context)?;
                set_selection(name, &mut selection, dimensions)?;
            }
        }
    }
    Ok(ArithmeticOptions {
        selection,
        omit_nan,
        output,
    })
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

fn double_reduction(
    reduction: ArithmeticReduction,
    input: &Value,
    plan: &ReductionPlan,
    omit_nan: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = reduction.name();
    let mut output = filled_values(name, plan.output_length, reduction.identity_f64())?;
    visit_f64(input, name, context, |index, value| {
        if omit_nan && complex64_is_nan(value) {
            return Ok(());
        }
        let target = output_offset(name, index, plan)?;
        let accumulator = output.get_mut(target).ok_or_else(|| mapping_error(name))?;
        reduction.combine_f64(accumulator, value);
        Ok(())
    })?;
    double_output(plan, output, input.is_complex_numeric())
}

fn single_reduction(
    reduction: ArithmeticReduction,
    input: &Value,
    plan: &ReductionPlan,
    omit_nan: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = reduction.name();
    let mut output = filled_values(name, plan.output_length, reduction.identity_f32())?;
    visit_f32(input, name, context, |index, value| {
        if omit_nan && complex32_is_nan(value) {
            return Ok(());
        }
        let target = output_offset(name, index, plan)?;
        let accumulator = output.get_mut(target).ok_or_else(|| mapping_error(name))?;
        reduction.combine_f32(accumulator, value);
        Ok(())
    })?;
    single_output(plan, output, input.is_complex_numeric())
}

fn double_output(
    plan: &ReductionPlan,
    values: Vec<ArrayComplex64>,
    complex: bool,
) -> Result<Value, BuiltinError> {
    if complex {
        if plan.output_shape.numel() == 1 {
            return values
                .first()
                .copied()
                .map(openmat_value::Complex64::from)
                .map(Value::Complex)
                .ok_or_else(|| mapping_error("reduction"));
        }
        DenseArray::from_vec(plan.output_shape.clone(), values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        real_f64_output(
            plan.output_shape.clone(),
            values.into_iter().map(|value| value.re).collect(),
        )
    }
}

fn single_output(
    plan: &ReductionPlan,
    values: Vec<ArrayComplex32>,
    complex: bool,
) -> Result<Value, BuiltinError> {
    let output = if complex {
        ArrayData::ComplexF32(
            DenseArray::from_vec(plan.output_shape.clone(), values)
                .map_err(|error| array_error(&error))?,
        )
    } else {
        ArrayData::F32(
            DenseArray::from_vec(
                plan.output_shape.clone(),
                values.into_iter().map(|value| value.re).collect(),
            )
            .map_err(|error| array_error(&error))?,
        )
    };
    Ok(Value::Array(output))
}

fn native_logical_reduction(
    reduction: ArithmeticReduction,
    input: &Value,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = reduction.name();
    let identity = reduction == ArithmeticReduction::Product;
    let mut output = filled_values(name, plan.output_length, identity)?;
    visit_logical(input, name, context, |index, value| {
        let target = output_offset(name, index, plan)?;
        let accumulator = output.get_mut(target).ok_or_else(|| mapping_error(name))?;
        match reduction {
            ArithmeticReduction::Sum => *accumulator |= value,
            ArithmeticReduction::Product => *accumulator &= value,
        }
        Ok(())
    })?;
    logical_output(plan, output)
}

fn logical_output(plan: &ReductionPlan, values: Vec<bool>) -> Result<Value, BuiltinError> {
    if plan.output_shape.numel() == 1 {
        return values
            .first()
            .copied()
            .map(Value::Logical)
            .ok_or_else(|| mapping_error("reduction"));
    }
    let values = values.into_iter().map(Logical::from).collect();
    DenseArray::from_vec(plan.output_shape.clone(), values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn native_integer_reduction(
    reduction: ArithmeticReduction,
    input: &IntegerArrayData,
    plan: &ReductionPlan,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! reduce_variant {
        ($array:expr, $kind:ty) => {{
            let identity: $kind = match reduction {
                ArithmeticReduction::Sum => 0,
                ArithmeticReduction::Product => 1,
            };
            let mut output = filled_values(reduction.name(), plan.output_length, identity)?;
            for (index, value) in $array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                let target = output_offset(reduction.name(), index, plan)?;
                output[target] = match reduction {
                    ArithmeticReduction::Sum => output[target].saturating_add(value),
                    ArithmeticReduction::Product => output[target].saturating_mul(value),
                };
            }
            DenseArray::from_vec(plan.output_shape.clone(), output)
                .map(IntegerArrayData::from_typed)
                .map(ArrayData::Integer)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }};
    }
    match input {
        IntegerArrayData::I8(array) => reduce_variant!(array, i8),
        IntegerArrayData::U8(array) => reduce_variant!(array, u8),
        IntegerArrayData::I16(array) => reduce_variant!(array, i16),
        IntegerArrayData::U16(array) => reduce_variant!(array, u16),
        IntegerArrayData::I32(array) => reduce_variant!(array, i32),
        IntegerArrayData::U32(array) => reduce_variant!(array, u32),
        IntegerArrayData::I64(array) => reduce_variant!(array, i64),
        IntegerArrayData::U64(array) => reduce_variant!(array, u64),
        _ => Err(type_error(
            reduction.name(),
            1,
            "real integer array",
            &Value::Array(ArrayData::Integer(input.clone())),
        )),
    }
}

const fn complex64_is_nan(value: ArrayComplex64) -> bool {
    value.re.is_nan() || value.im.is_nan()
}

const fn complex32_is_nan(value: ArrayComplex32) -> bool {
    value.re.is_nan() || value.im.is_nan()
}

fn filled_values<T: Clone>(name: &str, length: usize, value: T) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(name))?;
    values.resize(length, value);
    Ok(values)
}

fn reserved_values<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(name))?;
    Ok(values)
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

fn option_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` received an unsupported or conflicting reduction option"),
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` computed an invalid checked reduction mapping"),
    )
}

fn allocation_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not reserve its checked output buffer"),
    )
}

use std::cmp::Ordering;

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData, Logical,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count_range,
    expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MissingPlacement {
    Auto,
    First,
    Last,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComparisonMethod {
    Auto,
    Real,
    Absolute,
}

#[derive(Clone, Copy)]
struct SortOptions {
    dimension: u64,
    direction: Direction,
    missing: MissingPlacement,
    comparison: ComparisonMethod,
}

#[derive(Clone)]
struct SortEntry<T> {
    value: T,
    original_axis_index: usize,
}

trait SortElement: Clone {
    fn is_missing(&self) -> bool {
        false
    }

    fn compare_value(&self, other: &Self, method: ComparisonMethod) -> Ordering;
}

pub(super) fn sort_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("sort", arguments, 1, 7)?;
    expect_max_outputs("sort", context, 2)?;
    context.check_cancelled()?;
    let dimensions = sort_dimensions(&arguments[0])?;
    let options = parse_options(arguments, dimensions)?;
    if context.requested_outputs() == 0 {
        return Ok(Vec::new());
    }
    let (sorted, indices) = sort_value(&arguments[0], options, context)?;
    let mut output = reserved_vec("sort", context.requested_outputs())?;
    output.push(sorted);
    if context.requested_outputs() > 1 {
        output.push(indices);
    }
    context.check_cancelled()?;
    Ok(output)
}

fn parse_options(arguments: &[Value], dimensions: &[u64]) -> Result<SortOptions, BuiltinError> {
    let mut index = 1;
    let dimension = if let Some(value) = arguments.get(index)
        && keyword(value).is_none()
    {
        index += 1;
        sort_dimension(value)?
    } else {
        default_dimension(dimensions)
    };
    let direction = match arguments.get(index).and_then(keyword).as_deref() {
        Some("ascend") => {
            index += 1;
            Direction::Ascending
        }
        Some("descend") => {
            index += 1;
            Direction::Descending
        }
        _ => Direction::Ascending,
    };
    let mut missing = MissingPlacement::Auto;
    let mut comparison = ComparisonMethod::Auto;
    while index < arguments.len() {
        let name = keyword(&arguments[index]).ok_or_else(option_error)?;
        let value = arguments
            .get(index + 1)
            .and_then(keyword)
            .ok_or_else(option_error)?;
        match name.as_str() {
            "missingplacement" => {
                missing = match value.as_str() {
                    "auto" => MissingPlacement::Auto,
                    "first" => MissingPlacement::First,
                    "last" => MissingPlacement::Last,
                    _ => return Err(option_error()),
                };
            }
            "comparisonmethod" => {
                comparison = match value.as_str() {
                    "auto" => ComparisonMethod::Auto,
                    "real" => ComparisonMethod::Real,
                    "abs" => ComparisonMethod::Absolute,
                    _ => return Err(option_error()),
                };
            }
            _ => return Err(option_error()),
        }
        index += 2;
    }
    Ok(SortOptions {
        dimension,
        direction,
        missing,
        comparison,
    })
}

fn sort_value(
    input: &Value,
    options: SortOptions,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value), BuiltinError> {
    if matches!(
        input,
        Value::Logical(_) | Value::Double(_) | Value::Complex(_)
    ) {
        return Ok((input.clone(), Value::Double(1.0)));
    }
    macro_rules! sort_dense_value {
        ($array:expr, $wrap:expr) => {{
            let (array, indices) = sort_dense($array, options, context)?;
            Ok(($wrap(array), indices_value(indices)?))
        }};
    }
    match input {
        Value::Array(ArrayData::F32(array)) => {
            sort_dense_value!(array, |array| Value::Array(ArrayData::F32(array)))
        }
        Value::Array(ArrayData::ComplexF32(array)) => sort_dense_value!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        }),
        Value::Array(ArrayData::Logical(array)) => {
            sort_dense_value!(array, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Array(ArrayData::F64(array)) => {
            sort_dense_value!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => sort_dense_value!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Value::Array(ArrayData::Char(array)) => {
            sort_dense_value!(array, |array| Value::Array(ArrayData::Char(array)))
        }
        Value::Array(ArrayData::Integer(array)) => sort_integer(array, options, context),
        value => Err(type_error(
            "sort",
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

fn sort_integer(
    input: &IntegerArrayData,
    options: SortOptions,
    context: &BuiltinContext<'_>,
) -> Result<(Value, Value), BuiltinError> {
    macro_rules! sort_integer_array {
        ($array:expr) => {{
            let (array, indices) = sort_dense($array, options, context)?;
            Ok((
                Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(array))),
                indices_value(indices)?,
            ))
        }};
    }
    match input {
        IntegerArrayData::I8(array) => sort_integer_array!(array),
        IntegerArrayData::U8(array) => sort_integer_array!(array),
        IntegerArrayData::I16(array) => sort_integer_array!(array),
        IntegerArrayData::U16(array) => sort_integer_array!(array),
        IntegerArrayData::I32(array) => sort_integer_array!(array),
        IntegerArrayData::U32(array) => sort_integer_array!(array),
        IntegerArrayData::I64(array) => sort_integer_array!(array),
        IntegerArrayData::U64(array) => sort_integer_array!(array),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "`sort` does not accept complex integer storage",
        )),
    }
}

#[allow(clippy::cast_precision_loss)]
fn sort_dense<T: SortElement>(
    input: &DenseArray<T>,
    options: SortOptions,
    context: &BuiltinContext<'_>,
) -> Result<(DenseArray<T>, DenseArray<f64>), BuiltinError> {
    let axis = usize::try_from(options.dimension - 1).map_err(|_| dimension_error())?;
    let extent = usize::try_from(input.shape().extent(axis)).map_err(|_| mapping_error())?;
    let mut values = clone_values(input.as_slice(), context)?;
    let mut indices = filled_values(input.as_slice().len(), 0.0_f64, context)?;
    if extent <= 1 || input.is_empty() {
        indices.fill(1.0);
        return build_dense_results(input, values, indices);
    }
    let stride = input
        .shape()
        .dimensions()
        .iter()
        .take(axis)
        .try_fold(1_u64, |stride, extent| stride.checked_mul(*extent))
        .and_then(|stride| usize::try_from(stride).ok())
        .ok_or_else(mapping_error)?;
    let block = stride.checked_mul(extent).ok_or_else(mapping_error)?;
    let outer = input.as_slice().len() / block;
    for outer_index in 0..outer {
        for inner in 0..stride {
            context.check_cancelled()?;
            let base = outer_index
                .checked_mul(block)
                .and_then(|value| value.checked_add(inner))
                .ok_or_else(mapping_error)?;
            let mut entries = reserved_vec("sort", extent)?;
            for coordinate in 0..extent {
                check_cancelled_at(context, coordinate)?;
                let offset = base
                    .checked_add(coordinate.checked_mul(stride).ok_or_else(mapping_error)?)
                    .ok_or_else(mapping_error)?;
                entries.push(SortEntry {
                    value: input.as_slice()[offset].clone(),
                    original_axis_index: coordinate,
                });
            }
            stable_merge_sort(&mut entries, options, context)?;
            for (coordinate, entry) in entries.into_iter().enumerate() {
                check_cancelled_at(context, coordinate)?;
                let offset = base
                    .checked_add(coordinate.checked_mul(stride).ok_or_else(mapping_error)?)
                    .ok_or_else(mapping_error)?;
                values[offset] = entry.value;
                indices[offset] = (entry.original_axis_index + 1) as f64;
            }
        }
    }
    build_dense_results(input, values, indices)
}

fn build_dense_results<T>(
    input: &DenseArray<T>,
    values: Vec<T>,
    indices: Vec<f64>,
) -> Result<(DenseArray<T>, DenseArray<f64>), BuiltinError> {
    Ok((
        DenseArray::from_vec(input.shape().clone(), values).map_err(|error| array_error(&error))?,
        DenseArray::from_vec(input.shape().clone(), indices)
            .map_err(|error| array_error(&error))?,
    ))
}

fn stable_merge_sort<T: SortElement>(
    values: &mut Vec<SortEntry<T>>,
    options: SortOptions,
    context: &BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    if values.len() < 2 {
        return Ok(());
    }
    let mut scratch = clone_entries(values, context)?;
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
                        && compare_entry(&values[left], &values[right], options)
                            != Ordering::Greater);
                let source = if take_left {
                    let source = left;
                    left += 1;
                    source
                } else {
                    let source = right;
                    right += 1;
                    source
                };
                *destination = values[source].clone();
            }
            start = end;
        }
        std::mem::swap(values, &mut scratch);
        width = width.saturating_mul(2);
    }
    Ok(())
}

fn compare_entry<T: SortElement>(
    left: &SortEntry<T>,
    right: &SortEntry<T>,
    options: SortOptions,
) -> Ordering {
    let left_missing = left.value.is_missing();
    let right_missing = right.value.is_missing();
    match (left_missing, right_missing) {
        (true, true) => Ordering::Equal,
        (true, false) => match effective_missing(options) {
            MissingPlacement::First => Ordering::Less,
            MissingPlacement::Last | MissingPlacement::Auto => Ordering::Greater,
        },
        (false, true) => match effective_missing(options) {
            MissingPlacement::First => Ordering::Greater,
            MissingPlacement::Last | MissingPlacement::Auto => Ordering::Less,
        },
        (false, false) => {
            let ordering = left.value.compare_value(&right.value, options.comparison);
            if options.direction == Direction::Descending {
                ordering.reverse()
            } else {
                ordering
            }
        }
    }
}

fn effective_missing(options: SortOptions) -> MissingPlacement {
    match (options.missing, options.direction) {
        (MissingPlacement::Auto, Direction::Ascending) => MissingPlacement::Last,
        (MissingPlacement::Auto, Direction::Descending) => MissingPlacement::First,
        (placement, _) => placement,
    }
}

macro_rules! impl_sort_ord {
    ($($kind:ty),+ $(,)?) => {$(
        impl SortElement for $kind {
            fn compare_value(&self, other: &Self, _method: ComparisonMethod) -> Ordering {
                self.cmp(other)
            }
        }
    )+};
}

impl_sort_ord!(i8, u8, i16, u16, i32, u32, i64, u64);

impl SortElement for Logical {
    fn compare_value(&self, other: &Self, _method: ComparisonMethod) -> Ordering {
        self.get().cmp(&other.get())
    }
}

impl SortElement for CharCodeUnit {
    fn compare_value(&self, other: &Self, _method: ComparisonMethod) -> Ordering {
        self.get().cmp(&other.get())
    }
}

macro_rules! impl_sort_float {
    ($kind:ty) => {
        impl SortElement for $kind {
            fn is_missing(&self) -> bool {
                self.is_nan()
            }

            fn compare_value(&self, other: &Self, method: ComparisonMethod) -> Ordering {
                match method {
                    ComparisonMethod::Absolute => self
                        .abs()
                        .partial_cmp(&other.abs())
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| {
                            (0.0 as $kind)
                                .atan2(*self)
                                .partial_cmp(&(0.0 as $kind).atan2(*other))
                                .unwrap_or(Ordering::Equal)
                        }),
                    ComparisonMethod::Auto | ComparisonMethod::Real => {
                        self.partial_cmp(other).unwrap_or(Ordering::Equal)
                    }
                }
            }
        }
    };
}

impl_sort_float!(f32);
impl_sort_float!(f64);

macro_rules! impl_sort_complex {
    ($kind:ty) => {
        impl SortElement for $kind {
            fn is_missing(&self) -> bool {
                self.re.is_nan() || self.im.is_nan()
            }

            fn compare_value(&self, other: &Self, method: ComparisonMethod) -> Ordering {
                match method {
                    ComparisonMethod::Real => self
                        .re
                        .partial_cmp(&other.re)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| self.im.partial_cmp(&other.im).unwrap_or(Ordering::Equal)),
                    ComparisonMethod::Auto | ComparisonMethod::Absolute => self
                        .re
                        .hypot(self.im)
                        .partial_cmp(&other.re.hypot(other.im))
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| {
                            self.im
                                .atan2(self.re)
                                .partial_cmp(&other.im.atan2(other.re))
                                .unwrap_or(Ordering::Equal)
                        }),
                }
            }
        }
    };
}

impl_sort_complex!(ArrayComplex32);
impl_sort_complex!(ArrayComplex64);

fn sort_dimension(value: &Value) -> Result<u64, BuiltinError> {
    if matches!(value, Value::Logical(_) | Value::Complex(_)) {
        return Err(dimension_error());
    }
    if let Some(value) = exact_real_integer_scalar(value) {
        return match value {
            openmat_array::IntegerComponent::Signed(value) => u64::try_from(value).ok(),
            openmat_array::IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
        }
        .filter(|value| *value > 0)
        .ok_or_else(dimension_error);
    }
    let value = match value {
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => Some(array.as_slice()[0]),
        _ => value.as_real_single().map(f64::from),
    }
    .ok_or_else(dimension_error)?;
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(dimension_error());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn sort_dimensions(value: &Value) -> Result<&[u64], BuiltinError> {
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
        ) => value.dimensions().ok_or_else(mapping_error),
        _ => Err(type_error(
            "sort",
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

fn default_dimension(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|index| u64::try_from(index).ok())
        .and_then(|index| index.checked_add(1))
        .unwrap_or(1)
}

fn keyword(value: &Value) -> Option<String> {
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

fn indices_value(array: DenseArray<f64>) -> Result<Value, BuiltinError> {
    if array.numel() == 1 {
        array
            .as_slice()
            .first()
            .copied()
            .map(Value::Double)
            .ok_or_else(mapping_error)
    } else {
        Ok(Value::Array(ArrayData::F64(array)))
    }
}

fn clone_values<T: Clone>(
    input: &[T],
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_vec("sort", input.len())?;
    for (index, value) in input.iter().enumerate() {
        check_cancelled_at(context, index)?;
        output.push(value.clone());
    }
    Ok(output)
}

fn clone_entries<T: Clone>(
    input: &[SortEntry<T>],
    context: &BuiltinContext<'_>,
) -> Result<Vec<SortEntry<T>>, BuiltinError> {
    let mut output = reserved_vec("sort", input.len())?;
    for (index, value) in input.iter().enumerate() {
        check_cancelled_at(context, index)?;
        output.push(value.clone());
    }
    Ok(output)
}

fn filled_values<T: Clone>(
    length: usize,
    value: T,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_vec("sort", length)?;
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

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn dimension_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "input 2 to `sort` must be a positive real integer scalar dimension",
    )
}

fn option_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`sort` received an unsupported direction or name-value option",
    )
}

fn mapping_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`sort` computed an invalid checked column-major mapping",
    )
}

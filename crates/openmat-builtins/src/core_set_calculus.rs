//! R2022b-compatible set, ordering, selection, and discrete-calculus built-ins.
use std::cmp::Ordering;

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32, Complex64 as ArrayComplex64, DType, DenseArray,
    IntegerArrayData, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count_range,
    expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Order {
    Sorted,
    Stable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetMode {
    Elements,
    Rows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetOperation {
    Intersect,
    Union,
    Difference,
    Xor,
}

impl SetOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::Intersect => "intersect",
            Self::Union => "union",
            Self::Difference => "setdiff",
            Self::Xor => "setxor",
        }
    }

    const fn max_outputs(self) -> usize {
        match self {
            Self::Difference => 2,
            Self::Intersect | Self::Union | Self::Xor => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Direction {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComparisonMethod {
    Auto,
    Real,
    Absolute,
}

trait OrderedElement: Clone {
    fn set_equal(&self, other: &Self) -> bool;
    fn compare_value(&self, other: &Self, method: ComparisonMethod) -> Ordering;
    fn is_missing(&self) -> bool {
        false
    }
}

macro_rules! impl_ordered_integer {
    ($($kind:ty),+ $(,)?) => {$(
        impl OrderedElement for $kind {
            fn set_equal(&self, other: &Self) -> bool {
                self == other
            }

            fn compare_value(&self, other: &Self, _method: ComparisonMethod) -> Ordering {
                self.cmp(other)
            }
        }
    )+};
}

impl_ordered_integer!(i8, u8, i16, u16, i32, u32, i64, u64);

impl OrderedElement for Logical {
    fn set_equal(&self, other: &Self) -> bool {
        self == other
    }

    fn compare_value(&self, other: &Self, _method: ComparisonMethod) -> Ordering {
        self.get().cmp(&other.get())
    }
}

impl OrderedElement for CharCodeUnit {
    fn set_equal(&self, other: &Self) -> bool {
        self == other
    }

    fn compare_value(&self, other: &Self, _method: ComparisonMethod) -> Ordering {
        self.get().cmp(&other.get())
    }
}

macro_rules! impl_ordered_float {
    ($kind:ty) => {
        impl OrderedElement for $kind {
            #[allow(clippy::float_cmp)]
            fn set_equal(&self, other: &Self) -> bool {
                self == other
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

            fn is_missing(&self) -> bool {
                self.is_nan()
            }
        }
    };
}

impl_ordered_float!(f32);
impl_ordered_float!(f64);

macro_rules! impl_ordered_complex {
    ($kind:ty) => {
        impl OrderedElement for $kind {
            #[allow(clippy::float_cmp)]
            fn set_equal(&self, other: &Self) -> bool {
                self.re == other.re && self.im == other.im
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

            fn is_missing(&self) -> bool {
                self.re.is_nan() || self.im.is_nan()
            }
        }
    };
}

impl_ordered_complex!(Complex32);
impl_ordered_complex!(ArrayComplex64);

#[derive(Clone)]
struct SetUnit<T> {
    values: Vec<T>,
    source: usize,
    index: usize,
}

struct SetResult<T> {
    values: DenseArray<T>,
    left_indices: Vec<f64>,
    right_indices: Vec<f64>,
}

pub(super) fn intersect_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    set_builtin(SetOperation::Intersect, arguments, context)
}

pub(super) fn union_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    set_builtin(SetOperation::Union, arguments, context)
}

pub(super) fn setdiff_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    set_builtin(SetOperation::Difference, arguments, context)
}

pub(super) fn setxor_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    set_builtin(SetOperation::Xor, arguments, context)
}

fn set_builtin(
    operation: SetOperation,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = operation.name();
    expect_argument_count_range(name, arguments, 2, 4)?;
    expect_max_outputs(name, context, operation.max_outputs())?;
    context.check_cancelled()?;
    let (mode, order) = parse_set_options(name, &arguments[2..])?;
    let outputs = dispatch_set(
        operation,
        &arguments[0],
        &arguments[1],
        mode,
        order,
        context,
    )?;
    context.check_cancelled()?;
    Ok(outputs)
}

fn parse_set_options(name: &str, values: &[Value]) -> Result<(SetMode, Order), BuiltinError> {
    let mut mode = SetMode::Elements;
    let mut order = Order::Sorted;
    let mut saw_mode = false;
    let mut saw_order = false;
    for value in values {
        match keyword(value).as_deref() {
            Some("rows") if !saw_mode => {
                mode = SetMode::Rows;
                saw_mode = true;
            }
            Some("sorted") if !saw_order => {
                order = Order::Sorted;
                saw_order = true;
            }
            Some("stable") if !saw_order => {
                order = Order::Stable;
                saw_order = true;
            }
            _ => return Err(option_error(name)),
        }
    }
    Ok((mode, order))
}

macro_rules! run_integer_set {
    ($kind:ty, $left:expr, $right:expr, $operation:expr, $mode:expr, $order:expr, $context:expr) => {{
        let left_array = dense_integer::<$kind>($left)
            .ok_or_else(|| set_type_error($operation.name(), 1, $left))?;
        let right_array = dense_integer::<$kind>($right)
            .ok_or_else(|| set_type_error($operation.name(), 2, $right))?;
        let result = set_dense(
            $operation,
            &left_array,
            &right_array,
            $mode,
            $order,
            $context,
        )?;
        let values = Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
            result.values.clone(),
        )));
        set_outputs($operation, values, result, $context.requested_outputs())
    }};
}

fn dispatch_set(
    operation: SetOperation,
    left: &Value,
    right: &Value,
    mode: SetMode,
    order: Order,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let left_dtype = left
        .dtype()
        .ok_or_else(|| set_type_error(operation.name(), 1, left))?;
    let right_dtype = right
        .dtype()
        .ok_or_else(|| set_type_error(operation.name(), 2, right))?;

    macro_rules! run {
        ($left_array:expr, $right_array:expr, $wrap:expr) => {{
            let left_array =
                $left_array.ok_or_else(|| set_type_error(operation.name(), 1, left))?;
            let right_array =
                $right_array.ok_or_else(|| set_type_error(operation.name(), 2, right))?;
            let result = set_dense(operation, &left_array, &right_array, mode, order, context)?;
            let values = $wrap(result.values.clone());
            set_outputs(operation, values, result, context.requested_outputs())
        }};
    }

    match (left_dtype, right_dtype) {
        (DType::F64, DType::F64) => run!(dense_f64(left), dense_f64(right), |array| {
            Value::Array(ArrayData::F64(array))
        }),
        (DType::ComplexF64, DType::F64 | DType::ComplexF64) | (DType::F64, DType::ComplexF64) => {
            run!(dense_complex64(left), dense_complex64(right), |array| {
                Value::Array(ArrayData::ComplexF64(array))
            })
        }
        (DType::F32, DType::F32) => run!(dense_f32(left), dense_f32(right), |array| {
            Value::Array(ArrayData::F32(array))
        }),
        (DType::ComplexF32, DType::F32 | DType::ComplexF32) | (DType::F32, DType::ComplexF32) => {
            run!(dense_complex32(left), dense_complex32(right), |array| {
                Value::Array(ArrayData::ComplexF32(array))
            })
        }
        (DType::F64 | DType::ComplexF64, DType::F32 | DType::ComplexF32)
        | (DType::F32 | DType::ComplexF32, DType::F64 | DType::ComplexF64) => {
            if left_dtype.is_complex() || right_dtype.is_complex() {
                run!(dense_complex32(left), dense_complex32(right), |array| {
                    Value::Array(ArrayData::ComplexF32(array))
                })
            } else {
                run!(dense_f32(left), dense_f32(right), |array| {
                    Value::Array(ArrayData::F32(array))
                })
            }
        }
        (DType::Logical, DType::Logical) => {
            run!(dense_logical(left), dense_logical(right), |array| {
                Value::Array(ArrayData::Logical(array))
            })
        }
        (DType::Logical, DType::F64) | (DType::F64, DType::Logical) => {
            run!(dense_f64(left), dense_f64(right), |array| {
                Value::Array(ArrayData::F64(array))
            })
        }
        (DType::Char | DType::F64, DType::Char) | (DType::Char, DType::F64) => {
            run!(dense_char(left), dense_char(right), |array| {
                Value::Array(ArrayData::Char(array))
            })
        }
        (DType::I8, DType::I8 | DType::F64) | (DType::F64, DType::I8) => {
            run_integer_set!(i8, left, right, operation, mode, order, context)
        }
        (DType::U8, DType::U8 | DType::F64) | (DType::F64, DType::U8) => {
            run_integer_set!(u8, left, right, operation, mode, order, context)
        }
        (DType::I16, DType::I16 | DType::F64) | (DType::F64, DType::I16) => {
            run_integer_set!(i16, left, right, operation, mode, order, context)
        }
        (DType::U16, DType::U16 | DType::F64) | (DType::F64, DType::U16) => {
            run_integer_set!(u16, left, right, operation, mode, order, context)
        }
        (DType::I32, DType::I32 | DType::F64) | (DType::F64, DType::I32) => {
            run_integer_set!(i32, left, right, operation, mode, order, context)
        }
        (DType::U32, DType::U32 | DType::F64) | (DType::F64, DType::U32) => {
            run_integer_set!(u32, left, right, operation, mode, order, context)
        }
        (DType::I64, DType::I64 | DType::F64) | (DType::F64, DType::I64) => {
            run_integer_set!(i64, left, right, operation, mode, order, context)
        }
        (DType::U64, DType::U64 | DType::F64) | (DType::F64, DType::U64) => {
            run_integer_set!(u64, left, right, operation, mode, order, context)
        }
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!(
                "`{}` requires compatible real numeric, logical, or char inputs",
                operation.name()
            ),
        )),
    }
}

fn set_dense<T: OrderedElement>(
    operation: SetOperation,
    left: &DenseArray<T>,
    right: &DenseArray<T>,
    mode: SetMode,
    order: Order,
    context: &BuiltinContext<'_>,
) -> Result<SetResult<T>, BuiltinError> {
    validate_set_shapes(operation.name(), left.shape(), right.shape(), mode)?;
    let left_units = unique_set_units(set_units(left, 0, mode, context)?, context)?;
    let right_units = unique_set_units(set_units(right, 1, mode, context)?, context)?;
    let mut selected = Vec::new();
    match operation {
        SetOperation::Intersect => {
            for unit in &left_units {
                if right_units.iter().any(|other| units_equal(unit, other)) {
                    selected.push(unit.clone());
                }
            }
        }
        SetOperation::Union => {
            selected.extend(left_units.iter().cloned());
            for unit in &right_units {
                if !left_units.iter().any(|other| units_equal(unit, other)) {
                    selected.push(unit.clone());
                }
            }
        }
        SetOperation::Difference => {
            for unit in &left_units {
                if !right_units.iter().any(|other| units_equal(unit, other)) {
                    selected.push(unit.clone());
                }
            }
        }
        SetOperation::Xor => {
            for unit in &left_units {
                if !right_units.iter().any(|other| units_equal(unit, other)) {
                    selected.push(unit.clone());
                }
            }
            for unit in &right_units {
                if !left_units.iter().any(|other| units_equal(unit, other)) {
                    selected.push(unit.clone());
                }
            }
        }
    }
    if order == Order::Sorted {
        selected.sort_by(|left, right| compare_units(left, right, ComparisonMethod::Auto));
    }
    let mut left_indices = Vec::new();
    let mut right_indices = Vec::new();
    if operation == SetOperation::Intersect {
        for unit in &selected {
            left_indices.push(index_as_f64(unit.index)?);
            let matching = right_units
                .iter()
                .find(|other| units_equal(unit, other))
                .ok_or_else(|| mapping_error(operation.name()))?;
            right_indices.push(index_as_f64(matching.index)?);
        }
    } else {
        for unit in &selected {
            if unit.source == 0 {
                left_indices.push(index_as_f64(unit.index)?);
            } else {
                right_indices.push(index_as_f64(unit.index)?);
            }
        }
    }
    let values = build_set_values(operation, left.shape(), right.shape(), mode, &selected)?;
    Ok(SetResult {
        values,
        left_indices,
        right_indices,
    })
}

fn set_units<T: OrderedElement>(
    input: &DenseArray<T>,
    source: usize,
    mode: SetMode,
    context: &BuiltinContext<'_>,
) -> Result<Vec<SetUnit<T>>, BuiltinError> {
    let count = match mode {
        SetMode::Elements => host_length("set operation", input.numel())?,
        SetMode::Rows => host_length("set operation", input.shape().extent(0))?,
    };
    let columns = host_length("set operation", input.shape().extent(1))?;
    let rows = host_length("set operation", input.shape().extent(0))?;
    let mut output = reserved_vec("set operation", count)?;
    for index in 0..count {
        check_cancelled_at(context, index)?;
        let values = match mode {
            SetMode::Elements => vec![input.as_slice()[index].clone()],
            SetMode::Rows => {
                let mut values = reserved_vec("set operation", columns)?;
                for column in 0..columns {
                    values.push(input.as_slice()[column * rows + index].clone());
                }
                values
            }
        };
        output.push(SetUnit {
            values,
            source,
            index,
        });
    }
    Ok(output)
}

fn unique_set_units<T: OrderedElement>(
    units: Vec<SetUnit<T>>,
    context: &BuiltinContext<'_>,
) -> Result<Vec<SetUnit<T>>, BuiltinError> {
    let mut output = Vec::new();
    for (index, unit) in units.into_iter().enumerate() {
        check_cancelled_at(context, index)?;
        if !output.iter().any(|other| units_equal(&unit, other)) {
            output.push(unit);
        }
    }
    Ok(output)
}

fn units_equal<T: OrderedElement>(left: &SetUnit<T>, right: &SetUnit<T>) -> bool {
    left.values.len() == right.values.len()
        && left
            .values
            .iter()
            .zip(&right.values)
            .all(|(left, right)| left.set_equal(right))
}

fn compare_units<T: OrderedElement>(
    left: &SetUnit<T>,
    right: &SetUnit<T>,
    method: ComparisonMethod,
) -> Ordering {
    left.values
        .iter()
        .zip(&right.values)
        .map(|(left, right)| compare_ordered(left, right, Direction::Ascending, method))
        .find(|order| *order != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

fn build_set_values<T: OrderedElement>(
    operation: SetOperation,
    left_shape: &Shape,
    right_shape: &Shape,
    mode: SetMode,
    selected: &[SetUnit<T>],
) -> Result<DenseArray<T>, BuiltinError> {
    let length = u64::try_from(selected.len()).map_err(|_| mapping_error(operation.name()))?;
    match mode {
        SetMode::Elements => {
            let row = match operation {
                SetOperation::Difference => left_shape.extent(0) == 1,
                SetOperation::Intersect | SetOperation::Union | SetOperation::Xor => {
                    left_shape.extent(0) == 1 && right_shape.extent(0) == 1
                }
            };
            let shape = if row {
                Shape::new([1, length])
            } else {
                Shape::new([length, 1])
            }
            .map_err(|error| array_error(&error))?;
            let values = selected
                .iter()
                .filter_map(|unit| unit.values.first().cloned())
                .collect();
            DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
        }
        SetMode::Rows => {
            let columns = left_shape.extent(1);
            let mut values = Vec::new();
            for column in 0..host_length(operation.name(), columns)? {
                for unit in selected {
                    values.push(unit.values[column].clone());
                }
            }
            DenseArray::from_vec(
                Shape::new([length, columns]).map_err(|error| array_error(&error))?,
                values,
            )
            .map_err(|error| array_error(&error))
        }
    }
}

fn set_outputs<T>(
    operation: SetOperation,
    values: Value,
    result: SetResult<T>,
    requested: usize,
) -> BuiltinResult {
    let mut outputs = reserved_vec(operation.name(), requested)?;
    if requested > 0 {
        outputs.push(values);
    }
    if requested > 1 {
        outputs.push(index_column(operation.name(), result.left_indices)?);
    }
    if requested > 2 {
        outputs.push(index_column(operation.name(), result.right_indices)?);
    }
    Ok(outputs)
}

fn validate_set_shapes(
    name: &str,
    left: &Shape,
    right: &Shape,
    mode: SetMode,
) -> Result<(), BuiltinError> {
    match mode {
        SetMode::Elements => {
            if !is_vector_shape(left) || !is_vector_shape(right) {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("`{name}` element form requires vector inputs"),
                ));
            }
        }
        SetMode::Rows => {
            if left.ndims() != 2 || right.ndims() != 2 || left.extent(1) != right.extent(1) {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("`{name}` rows form requires two matrices with equal column counts"),
                ));
            }
        }
    }
    Ok(())
}

fn is_vector_shape(shape: &Shape) -> bool {
    shape.ndims() == 2 && (shape.extent(0) <= 1 || shape.extent(1) <= 1)
}

pub(super) fn issorted_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("issorted", arguments, 1, 5)?;
    expect_max_outputs("issorted", context, 1)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions("issorted", &arguments[0], true)?;
    let options = parse_issorted_options(arguments, dimensions)?;
    let sorted = dispatch_issorted(&arguments[0], options, context)?;
    Ok(vec![Value::Logical(sorted)])
}

#[derive(Clone, Copy)]
struct IssortedOptions {
    axis: u64,
    rows: bool,
    direction: Direction,
    comparison: ComparisonMethod,
}

fn parse_issorted_options(
    arguments: &[Value],
    dimensions: &[u64],
) -> Result<IssortedOptions, BuiltinError> {
    let mut axis = default_axis(dimensions);
    let mut rows = false;
    let mut direction = Direction::Ascending;
    let mut comparison = ComparisonMethod::Auto;
    let mut index = 1;
    if let Some(value) = arguments.get(index) {
        if let Some(option) = keyword(value) {
            match option.as_str() {
                "rows" => {
                    rows = true;
                    index += 1;
                }
                "ascend" => index += 1,
                "descend" => {
                    direction = Direction::Descending;
                    index += 1;
                }
                _ => {}
            }
        } else {
            axis = positive_dimension("issorted", 2, value)?;
            index += 1;
        }
    }
    if rows {
        if index != arguments.len() {
            return Err(option_error("issorted"));
        }
        return Ok(IssortedOptions {
            axis: 1,
            rows,
            direction,
            comparison,
        });
    }
    if let Some(option) = arguments.get(index).and_then(keyword)
        && matches!(option.as_str(), "ascend" | "descend")
    {
        direction = if option == "descend" {
            Direction::Descending
        } else {
            Direction::Ascending
        };
        index += 1;
    }
    if index < arguments.len() {
        if arguments.len() - index != 2
            || keyword(&arguments[index]).as_deref() != Some("comparisonmethod")
        {
            return Err(option_error("issorted"));
        }
        comparison = comparison_method(&arguments[index + 1])?;
    }
    Ok(IssortedOptions {
        axis,
        rows,
        direction,
        comparison,
    })
}

fn dispatch_issorted(
    input: &Value,
    options: IssortedOptions,
    context: &BuiltinContext<'_>,
) -> Result<bool, BuiltinError> {
    macro_rules! run {
        ($array:expr) => {{
            let array = $array.ok_or_else(|| set_type_error("issorted", 1, input))?;
            issorted_dense(&array, options, context)
        }};
    }
    match input.dtype() {
        Some(openmat_array::DType::F64) => run!(dense_f64(input)),
        Some(openmat_array::DType::ComplexF64) => run!(dense_complex64(input)),
        Some(openmat_array::DType::F32) => run!(dense_f32(input)),
        Some(openmat_array::DType::ComplexF32) => run!(dense_complex32(input)),
        Some(openmat_array::DType::Logical) => run!(dense_logical(input)),
        Some(openmat_array::DType::Char) => run!(dense_char(input)),
        Some(openmat_array::DType::I8) => run!(dense_integer::<i8>(input)),
        Some(openmat_array::DType::U8) => run!(dense_integer::<u8>(input)),
        Some(openmat_array::DType::I16) => run!(dense_integer::<i16>(input)),
        Some(openmat_array::DType::U16) => run!(dense_integer::<u16>(input)),
        Some(openmat_array::DType::I32) => run!(dense_integer::<i32>(input)),
        Some(openmat_array::DType::U32) => run!(dense_integer::<u32>(input)),
        Some(openmat_array::DType::I64) => run!(dense_integer::<i64>(input)),
        Some(openmat_array::DType::U64) => run!(dense_integer::<u64>(input)),
        _ => Err(set_type_error("issorted", 1, input)),
    }
}

fn issorted_dense<T: OrderedElement>(
    input: &DenseArray<T>,
    options: IssortedOptions,
    context: &BuiltinContext<'_>,
) -> Result<bool, BuiltinError> {
    if options.rows {
        if input.shape().ndims() != 2 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`issorted(...,'rows')` requires a matrix",
            ));
        }
        let units = set_units(input, 0, SetMode::Rows, context)?;
        return Ok(units.windows(2).all(|pair| {
            compare_units(&pair[0], &pair[1], options.comparison) != Ordering::Greater
        }));
    }
    let axis = axis_index(options.axis, "issorted")?;
    let stride = axis_stride(input.shape(), axis)?;
    let extent = host_length("issorted", input.shape().extent(axis))?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error("issorted"))?;
    let outer = trailing_outer(input.shape(), axis)?;
    for outer_index in 0..outer {
        for inner in 0..stride {
            context.check_cancelled()?;
            let base = outer_index * block + inner;
            for coordinate in 1..extent {
                let left = &input.as_slice()[base + (coordinate - 1) * stride];
                let right = &input.as_slice()[base + coordinate * stride];
                if compare_ordered(left, right, options.direction, options.comparison)
                    == Ordering::Greater
                {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

pub(super) fn mink_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    selection_builtin(Direction::Ascending, arguments, context)
}

pub(super) fn maxk_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    selection_builtin(Direction::Descending, arguments, context)
}

fn selection_builtin(
    direction: Direction,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = if direction == Direction::Ascending {
        "mink"
    } else {
        "maxk"
    };
    expect_argument_count_range(name, arguments, 2, 5)?;
    expect_max_outputs(name, context, 2)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions(name, &arguments[0], true)?;
    let count = nonnegative_integer(name, 2, &arguments[1])?;
    let (axis, comparison) = parse_selection_options(name, &arguments[2..], dimensions)?;
    dispatch_selection(
        name,
        &arguments[0],
        count,
        axis,
        direction,
        comparison,
        context,
    )
}

fn parse_selection_options(
    name: &str,
    arguments: &[Value],
    dimensions: &[u64],
) -> Result<(u64, ComparisonMethod), BuiltinError> {
    let mut axis = default_axis(dimensions);
    let mut comparison = ComparisonMethod::Auto;
    let mut index = 0;
    if let Some(value) = arguments.first()
        && keyword(value).is_none()
    {
        axis = positive_dimension(name, 3, value)?;
        index += 1;
    }
    if index < arguments.len() {
        if arguments.len() - index != 2
            || keyword(&arguments[index]).as_deref() != Some("comparisonmethod")
        {
            return Err(option_error(name));
        }
        comparison = comparison_method(&arguments[index + 1])?;
    }
    Ok((axis, comparison))
}

macro_rules! run_integer_selection {
    ($kind:ty, $name:expr, $input:expr, $count:expr, $axis:expr, $direction:expr, $comparison:expr, $context:expr) => {{
        let array =
            dense_integer::<$kind>($input).ok_or_else(|| set_type_error($name, 1, $input))?;
        let (values, indices) = selection_dense(
            $name,
            &array,
            $count,
            $axis,
            $direction,
            $comparison,
            $context,
        )?;
        selection_outputs(
            Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(values))),
            indices,
            $context.requested_outputs(),
        )
    }};
}

fn dispatch_selection(
    name: &str,
    input: &Value,
    count: u64,
    axis: u64,
    direction: Direction,
    comparison: ComparisonMethod,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    macro_rules! run {
        ($array:expr, $wrap:expr) => {{
            let array = $array.ok_or_else(|| set_type_error(name, 1, input))?;
            let (values, indices) =
                selection_dense(name, &array, count, axis, direction, comparison, context)?;
            selection_outputs($wrap(values), indices, context.requested_outputs())
        }};
    }
    match input.dtype() {
        Some(openmat_array::DType::F64) => run!(dense_f64(input), |array| {
            Value::Array(ArrayData::F64(array))
        }),
        Some(openmat_array::DType::ComplexF64) => run!(dense_complex64(input), |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Some(openmat_array::DType::F32) => run!(dense_f32(input), |array| {
            Value::Array(ArrayData::F32(array))
        }),
        Some(openmat_array::DType::ComplexF32) => run!(dense_complex32(input), |array| {
            Value::Array(ArrayData::ComplexF32(array))
        }),
        Some(openmat_array::DType::Logical) => run!(dense_logical(input), |array| {
            Value::Array(ArrayData::Logical(array))
        }),
        Some(openmat_array::DType::Char) => run!(dense_char(input), |array| {
            Value::Array(ArrayData::Char(array))
        }),
        Some(openmat_array::DType::I8) => {
            run_integer_selection!(i8, name, input, count, axis, direction, comparison, context)
        }
        Some(openmat_array::DType::U8) => {
            run_integer_selection!(u8, name, input, count, axis, direction, comparison, context)
        }
        Some(openmat_array::DType::I16) => {
            run_integer_selection!(
                i16, name, input, count, axis, direction, comparison, context
            )
        }
        Some(openmat_array::DType::U16) => {
            run_integer_selection!(
                u16, name, input, count, axis, direction, comparison, context
            )
        }
        Some(openmat_array::DType::I32) => {
            run_integer_selection!(
                i32, name, input, count, axis, direction, comparison, context
            )
        }
        Some(openmat_array::DType::U32) => {
            run_integer_selection!(
                u32, name, input, count, axis, direction, comparison, context
            )
        }
        Some(openmat_array::DType::I64) => {
            run_integer_selection!(
                i64, name, input, count, axis, direction, comparison, context
            )
        }
        Some(openmat_array::DType::U64) => {
            run_integer_selection!(
                u64, name, input, count, axis, direction, comparison, context
            )
        }
        _ => Err(set_type_error(name, 1, input)),
    }
}

#[derive(Clone)]
struct SelectionEntry<T> {
    value: T,
    index: usize,
}

fn selection_dense<T: OrderedElement>(
    name: &str,
    input: &DenseArray<T>,
    count: u64,
    dimension: u64,
    direction: Direction,
    comparison: ComparisonMethod,
    context: &BuiltinContext<'_>,
) -> Result<(DenseArray<T>, DenseArray<f64>), BuiltinError> {
    let axis = axis_index(dimension, name)?;
    let extent = host_length(name, input.shape().extent(axis))?;
    let selected = extent.min(host_length(name, count)?);
    let mut dimensions = input.shape().dimensions().to_vec();
    if axis >= dimensions.len() {
        dimensions.resize(axis + 1, 1);
    }
    dimensions[axis] = u64::try_from(selected).map_err(|_| mapping_error(name))?;
    let output_shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let output_length = host_length(name, output_shape.numel())?;
    let mut values = reserved_vec(name, output_length)?;
    let mut indices = reserved_vec(name, output_length)?;
    let stride = axis_stride(input.shape(), axis)?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error(name))?;
    let outer = trailing_outer(input.shape(), axis)?;
    for outer_index in 0..outer {
        let mut lines = reserved_vec(name, stride)?;
        for inner in 0..stride {
            context.check_cancelled()?;
            let base = outer_index * block + inner;
            let mut entries = reserved_vec(name, extent)?;
            for coordinate in 0..extent {
                entries.push(SelectionEntry {
                    value: input.as_slice()[base + coordinate * stride].clone(),
                    index: coordinate,
                });
            }
            entries.sort_by(|left, right| {
                compare_selection(&left.value, &right.value, direction, comparison)
            });
            lines.push(entries);
        }
        for coordinate in 0..selected {
            for entries in &lines {
                values.push(entries[coordinate].value.clone());
                indices.push(index_as_f64(entries[coordinate].index)?);
            }
        }
    }
    Ok((
        DenseArray::from_vec(output_shape.clone(), values).map_err(|error| array_error(&error))?,
        DenseArray::from_vec(output_shape, indices).map_err(|error| array_error(&error))?,
    ))
}

fn selection_outputs(values: Value, indices: DenseArray<f64>, requested: usize) -> BuiltinResult {
    let mut outputs = reserved_vec("selection", requested)?;
    if requested > 0 {
        outputs.push(values);
    }
    if requested > 1 {
        outputs.push(Value::Array(ArrayData::F64(indices)));
    }
    Ok(outputs)
}

fn compare_ordered<T: OrderedElement>(
    left: &T,
    right: &T,
    direction: Direction,
    method: ComparisonMethod,
) -> Ordering {
    match (left.is_missing(), right.is_missing()) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if direction == Direction::Ascending {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, true) => {
            if direction == Direction::Ascending {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, false) => {
            let order = left.compare_value(right, method);
            if direction == Direction::Descending {
                order.reverse()
            } else {
                order
            }
        }
    }
}

fn compare_selection<T: OrderedElement>(
    left: &T,
    right: &T,
    direction: Direction,
    method: ComparisonMethod,
) -> Ordering {
    match (left.is_missing(), right.is_missing()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let order = left.compare_value(right, method);
            if direction == Direction::Descending {
                order.reverse()
            } else {
                order
            }
        }
    }
}

pub(super) fn trapz_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    trapezoid_builtin(false, arguments, context)
}

pub(super) fn cumtrapz_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    trapezoid_builtin(true, arguments, context)
}

fn trapezoid_builtin(
    cumulative: bool,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = if cumulative { "cumtrapz" } else { "trapz" };
    expect_argument_count_range(name, arguments, 1, 3)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let (coordinates, samples, dimension) = parse_trapezoid_arguments(name, arguments)?;
    let axis = dimension.unwrap_or(default_axis(numeric_dimensions(name, samples, true)?));
    let output = dispatch_trapezoid(name, cumulative, coordinates, samples, axis, context)?;
    Ok(vec![output])
}

fn parse_trapezoid_arguments<'a>(
    name: &str,
    arguments: &'a [Value],
) -> Result<(Option<&'a Value>, &'a Value, Option<u64>), BuiltinError> {
    match arguments {
        [samples] => Ok((None, samples, None)),
        [samples, dimension] if dimension.is_scalar() => {
            Ok((None, samples, Some(positive_dimension(name, 2, dimension)?)))
        }
        [coordinates, samples] => Ok((Some(coordinates), samples, None)),
        [coordinates, samples, dimension] => Ok((
            Some(coordinates),
            samples,
            Some(positive_dimension(name, 3, dimension)?),
        )),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` received an unsupported argument form"),
        )),
    }
}

trait FloatCalculus: OrderedElement + Copy {
    fn zero() -> Self;
    fn from_real(value: f64) -> Self;
    fn add(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
}

macro_rules! impl_real_calculus {
    ($kind:ty) => {
        impl FloatCalculus for $kind {
            fn zero() -> Self {
                0.0
            }
            #[allow(clippy::cast_possible_truncation)]
            fn from_real(value: f64) -> Self {
                value as Self
            }
            fn add(self, right: Self) -> Self {
                self + right
            }
            fn subtract(self, right: Self) -> Self {
                self - right
            }
            fn multiply(self, right: Self) -> Self {
                self * right
            }
            fn divide(self, right: Self) -> Self {
                self / right
            }
        }
    };
}

impl_real_calculus!(f32);
impl_real_calculus!(f64);

macro_rules! impl_complex_calculus {
    ($kind:ty, $component:ty) => {
        impl FloatCalculus for $kind {
            fn zero() -> Self {
                Self::new(0.0, 0.0)
            }
            #[allow(clippy::cast_possible_truncation)]
            fn from_real(value: f64) -> Self {
                Self::new(value as $component, 0.0)
            }
            fn add(self, right: Self) -> Self {
                Self::new(self.re + right.re, self.im + right.im)
            }
            fn subtract(self, right: Self) -> Self {
                Self::new(self.re - right.re, self.im - right.im)
            }
            fn multiply(self, right: Self) -> Self {
                Self::new(
                    self.re.mul_add(right.re, -(self.im * right.im)),
                    self.re.mul_add(right.im, self.im * right.re),
                )
            }
            fn divide(self, right: Self) -> Self {
                let denominator = right.re.mul_add(right.re, right.im * right.im);
                Self::new(
                    self.re.mul_add(right.re, self.im * right.im) / denominator,
                    self.im.mul_add(right.re, -(self.re * right.im)) / denominator,
                )
            }
        }
    };
}

impl_complex_calculus!(Complex32, f32);
impl_complex_calculus!(ArrayComplex64, f64);

trait CalculusConvert: FloatCalculus {
    fn dense_from_value(value: &Value) -> Option<DenseArray<Self>>;
}

impl CalculusConvert for f64 {
    fn dense_from_value(value: &Value) -> Option<DenseArray<Self>> {
        dense_f64(value)
    }
}

impl CalculusConvert for f32 {
    fn dense_from_value(value: &Value) -> Option<DenseArray<Self>> {
        dense_f32(value)
    }
}

impl CalculusConvert for ArrayComplex64 {
    fn dense_from_value(value: &Value) -> Option<DenseArray<Self>> {
        dense_complex64(value)
    }
}

impl CalculusConvert for Complex32 {
    fn dense_from_value(value: &Value) -> Option<DenseArray<Self>> {
        dense_complex32(value)
    }
}

fn dispatch_trapezoid(
    name: &str,
    cumulative: bool,
    coordinates: Option<&Value>,
    samples: &Value,
    axis: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let sample_dtype = samples
        .dtype()
        .ok_or_else(|| type_error(name, 1, "numeric array", samples))?;
    let coordinate_dtype = coordinates.and_then(Value::dtype);
    if cumulative && coordinates.is_none() {
        match sample_dtype {
            openmat_array::DType::I8 => {
                return cumulative_integer_trapezoid::<i8>(name, samples, axis, context);
            }
            openmat_array::DType::U8 => {
                return cumulative_integer_trapezoid::<u8>(name, samples, axis, context);
            }
            openmat_array::DType::I16 => {
                return cumulative_integer_trapezoid::<i16>(name, samples, axis, context);
            }
            openmat_array::DType::U16 => {
                return cumulative_integer_trapezoid::<u16>(name, samples, axis, context);
            }
            openmat_array::DType::I32 => {
                return cumulative_integer_trapezoid::<i32>(name, samples, axis, context);
            }
            openmat_array::DType::U32 => {
                return cumulative_integer_trapezoid::<u32>(name, samples, axis, context);
            }
            openmat_array::DType::I64 => {
                return cumulative_integer_trapezoid::<i64>(name, samples, axis, context);
            }
            openmat_array::DType::U64 => {
                return cumulative_integer_trapezoid::<u64>(name, samples, axis, context);
            }
            openmat_array::DType::Char => {
                return Err(type_error(name, 1, "numeric array", samples));
            }
            _ => {}
        }
    }
    if coordinates.is_some()
        && (sample_dtype.is_integer()
            || coordinate_dtype.is_some_and(openmat_array::DType::is_integer))
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("`{name}` explicit integer-coordinate forms are not supported"),
        ));
    }
    let use_single = is_single_dtype(sample_dtype) || coordinate_dtype.is_some_and(is_single_dtype);
    let use_complex =
        sample_dtype.is_complex() || coordinate_dtype.is_some_and(openmat_array::DType::is_complex);
    if use_single && use_complex {
        trapezoid_float::<Complex32>(name, cumulative, coordinates, samples, axis, context)
            .map(|array| Value::Array(ArrayData::ComplexF32(array)))
    } else if use_single {
        trapezoid_float::<f32>(name, cumulative, coordinates, samples, axis, context)
            .map(|array| Value::Array(ArrayData::F32(array)))
    } else if use_complex {
        trapezoid_float::<ArrayComplex64>(name, cumulative, coordinates, samples, axis, context)
            .map(|array| Value::Array(ArrayData::ComplexF64(array)))
    } else {
        trapezoid_float::<f64>(name, cumulative, coordinates, samples, axis, context)
            .map(|array| Value::Array(ArrayData::F64(array)))
    }
}

fn trapezoid_float<T: FloatCalculus + CalculusConvert>(
    name: &str,
    cumulative: bool,
    coordinates: Option<&Value>,
    samples: &Value,
    dimension: u64,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let samples = T::dense_from_value(samples)
        .ok_or_else(|| type_error(name, 1, "numeric array", samples))?;
    let axis = axis_index(dimension, name)?;
    let coordinates = coordinates
        .map(|value| {
            T::dense_from_value(value)
                .ok_or_else(|| type_error(name, 1, "numeric coordinate vector", value))
        })
        .transpose()?;
    if let Some(coordinates) = &coordinates
        && (coordinates.shape().ndims() != 2
            || !(coordinates.shape().extent(0) <= 1 || coordinates.shape().extent(1) <= 1)
            || coordinates.numel() != samples.shape().extent(axis))
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` coordinate vector length must match the integration dimension"),
        ));
    }
    if cumulative {
        cumulative_trapezoid_dense(name, &samples, coordinates.as_ref(), axis, context)
    } else {
        reducing_trapezoid_dense(name, &samples, coordinates.as_ref(), axis, context)
    }
}

fn reducing_trapezoid_dense<T: FloatCalculus>(
    name: &str,
    input: &DenseArray<T>,
    coordinates: Option<&DenseArray<T>>,
    axis: usize,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    if input.shape().dimensions() == [0, 0] {
        return scalar_dense(T::zero());
    }
    let mut dimensions = input.shape().dimensions().to_vec();
    if axis >= dimensions.len() {
        dimensions.resize(axis + 1, 1);
    }
    dimensions[axis] = 1;
    let output_shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let mut output = reserved_vec(name, host_length(name, output_shape.numel())?)?;
    let stride = axis_stride(input.shape(), axis)?;
    let extent = host_length(name, input.shape().extent(axis))?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error(name))?;
    let outer = trailing_outer(input.shape(), axis)?;
    let half = T::from_real(0.5);
    for outer_index in 0..outer {
        for inner in 0..stride {
            context.check_cancelled()?;
            let base = outer_index * block + inner;
            let mut sum = T::zero();
            for coordinate in 1..extent {
                let left = input.as_slice()[base + (coordinate - 1) * stride];
                let right = input.as_slice()[base + coordinate * stride];
                let width = coordinates.map_or_else(
                    || T::from_real(1.0),
                    |coordinates| {
                        coordinates.as_slice()[coordinate]
                            .subtract(coordinates.as_slice()[coordinate - 1])
                    },
                );
                sum = sum.add(left.add(right).multiply(width).multiply(half));
            }
            output.push(sum);
        }
    }
    DenseArray::from_vec(output_shape, output).map_err(|error| array_error(&error))
}

fn cumulative_trapezoid_dense<T: FloatCalculus>(
    name: &str,
    input: &DenseArray<T>,
    coordinates: Option<&DenseArray<T>>,
    axis: usize,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let mut output = vec![T::zero(); input.as_slice().len()];
    let stride = axis_stride(input.shape(), axis)?;
    let extent = host_length(name, input.shape().extent(axis))?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error(name))?;
    let outer = trailing_outer(input.shape(), axis)?;
    let half = T::from_real(0.5);
    for outer_index in 0..outer {
        for inner in 0..stride {
            context.check_cancelled()?;
            let base = outer_index * block + inner;
            let mut sum = T::zero();
            for coordinate in 1..extent {
                let left = input.as_slice()[base + (coordinate - 1) * stride];
                let right = input.as_slice()[base + coordinate * stride];
                let width = coordinates.map_or_else(
                    || T::from_real(1.0),
                    |coordinates| {
                        coordinates.as_slice()[coordinate]
                            .subtract(coordinates.as_slice()[coordinate - 1])
                    },
                );
                sum = sum.add(left.add(right).multiply(width).multiply(half));
                output[base + coordinate * stride] = sum;
            }
        }
    }
    DenseArray::from_vec(input.shape().clone(), output).map_err(|error| array_error(&error))
}

trait MatlabInteger: OrderedElement + Copy {
    fn saturating_add(self, right: Self) -> Self;
    fn saturating_sub(self, right: Self) -> Self;
    fn to_f64(self) -> f64;
    fn from_f64(value: f64) -> Self;
}

macro_rules! impl_matlab_integer {
    ($($kind:ty),+ $(,)?) => {$(
        impl MatlabInteger for $kind {
            fn saturating_add(self, right: Self) -> Self {
                self.saturating_add(right)
            }
            fn saturating_sub(self, right: Self) -> Self {
                self.saturating_sub(right)
            }
            #[allow(clippy::cast_precision_loss, clippy::cast_lossless)]
            fn to_f64(self) -> f64 {
                self as f64
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            fn from_f64(value: f64) -> Self {
                value.round() as Self
            }
        }
    )+};
}

impl_matlab_integer!(i8, u8, i16, u16, i32, u32, i64, u64);

fn cumulative_integer_trapezoid<T: MatlabInteger + MatlabCast + openmat_array::IntegerElement>(
    name: &str,
    samples: &Value,
    dimension: u64,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let input = dense_integer::<T>(samples)
        .ok_or_else(|| type_error(name, 1, "real integer array", samples))?;
    let axis = axis_index(dimension, name)?;
    let stride = axis_stride(input.shape(), axis)?;
    let extent = host_length(name, input.shape().extent(axis))?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error(name))?;
    let outer = trailing_outer(input.shape(), axis)?;
    let mut output = vec![T::from_f64(0.0); input.as_slice().len()];
    for outer_index in 0..outer {
        for inner in 0..stride {
            context.check_cancelled()?;
            let base = outer_index * block + inner;
            let mut doubled = T::from_f64(0.0);
            for coordinate in 1..extent {
                let pair = input.as_slice()[base + (coordinate - 1) * stride]
                    .saturating_add(input.as_slice()[base + coordinate * stride]);
                doubled = doubled.saturating_add(pair);
                output[base + coordinate * stride] = T::from_f64(doubled.to_f64() / 2.0);
            }
        }
    }
    DenseArray::from_vec(input.shape().clone(), output)
        .map(IntegerArrayData::from_typed)
        .map(ArrayData::Integer)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

pub(super) fn gradient_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("gradient", arguments, 1, usize::MAX)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions("gradient", &arguments[0], true)?;
    let vector = dimensions.len() == 2 && (dimensions[0] == 1 || dimensions[1] == 1);
    let max_outputs = if vector { 1 } else { dimensions.len() };
    expect_max_outputs("gradient", context, max_outputs.max(1))?;
    let axes = gradient_axes(dimensions, vector);
    let spacings = parse_gradient_spacings(&arguments[1..], &axes, dimensions)?;
    dispatch_gradient(
        &arguments[0],
        &axes,
        &spacings,
        context.requested_outputs().max(1),
        context,
    )
}

#[derive(Clone)]
enum Spacing {
    Scalar(Value),
    Vector(Value),
}

fn gradient_axes(dimensions: &[u64], vector: bool) -> Vec<usize> {
    if vector {
        return vec![
            dimensions
                .iter()
                .position(|extent| *extent > 1)
                .unwrap_or(0),
        ];
    }
    let mut axes = Vec::with_capacity(dimensions.len());
    if dimensions.len() >= 2 {
        axes.extend([1, 0]);
        axes.extend(2..dimensions.len());
    } else {
        axes.push(0);
    }
    axes
}

fn parse_gradient_spacings(
    values: &[Value],
    axes: &[usize],
    dimensions: &[u64],
) -> Result<Vec<Spacing>, BuiltinError> {
    if values.is_empty() {
        return Ok((0..axes.len())
            .map(|_| Spacing::Scalar(Value::Double(1.0)))
            .collect());
    }
    if values.len() == 1 && values[0].is_scalar() {
        return Ok((0..axes.len())
            .map(|_| Spacing::Scalar(values[0].clone()))
            .collect());
    }
    if values.len() != axes.len() {
        return Err(option_error("gradient"));
    }
    values
        .iter()
        .zip(axes)
        .map(|(value, axis)| {
            if value.is_scalar() {
                return Ok(Spacing::Scalar(value.clone()));
            }
            let value_dimensions = value.dimensions().ok_or_else(|| {
                type_error("gradient", 2, "numeric spacing scalar or vector", value)
            })?;
            if value_dimensions.len() != 2
                || !(value_dimensions[0] <= 1 || value_dimensions[1] <= 1)
                || value.numel() != Some(dimensions.get(*axis).copied().unwrap_or(1))
            {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`gradient` spacing vector length must match its dimension",
                ));
            }
            Ok(Spacing::Vector(value.clone()))
        })
        .collect()
}

fn dispatch_gradient(
    input: &Value,
    axes: &[usize],
    spacings: &[Spacing],
    requested: usize,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let dtype = input
        .dtype()
        .ok_or_else(|| type_error("gradient", 1, "numeric array", input))?;
    let spacing_complex = spacings.iter().any(|spacing| match spacing {
        Spacing::Scalar(value) | Spacing::Vector(value) => value.is_complex_numeric(),
    });
    macro_rules! run_float {
        ($kind:ty, $wrap:expr) => {{
            let array = <$kind as CalculusConvert>::dense_from_value(input)
                .ok_or_else(|| type_error("gradient", 1, "numeric array", input))?;
            let mut outputs = reserved_vec("gradient", requested)?;
            for output_index in 0..requested {
                let spacing = spacing_dense::<$kind>(&spacings[output_index])?;
                outputs.push($wrap(gradient_float_dense(
                    &array,
                    axes[output_index],
                    &spacing,
                    context,
                )?));
            }
            Ok(outputs)
        }};
    }
    match dtype {
        openmat_array::DType::F32 | openmat_array::DType::ComplexF32 => {
            if dtype.is_complex() || spacing_complex {
                run_float!(Complex32, |array| Value::Array(ArrayData::ComplexF32(
                    array
                )))
            } else {
                run_float!(f32, |array| Value::Array(ArrayData::F32(array)))
            }
        }
        openmat_array::DType::F64 | openmat_array::DType::ComplexF64 => {
            if dtype.is_complex() || spacing_complex {
                run_float!(ArrayComplex64, |array| Value::Array(ArrayData::ComplexF64(
                    array
                )))
            } else {
                run_float!(f64, |array| Value::Array(ArrayData::F64(array)))
            }
        }
        openmat_array::DType::Logical | openmat_array::DType::Char => {
            if spacing_complex {
                run_float!(ArrayComplex64, |array| Value::Array(ArrayData::ComplexF64(
                    array
                )))
            } else {
                run_float!(f64, |array| Value::Array(ArrayData::F64(array)))
            }
        }
        openmat_array::DType::I8 => {
            gradient_integer_outputs::<i8>(input, axes, spacings, requested, context)
        }
        openmat_array::DType::U8 => {
            gradient_integer_outputs::<u8>(input, axes, spacings, requested, context)
        }
        openmat_array::DType::I16 => {
            gradient_integer_outputs::<i16>(input, axes, spacings, requested, context)
        }
        openmat_array::DType::U16 => {
            gradient_integer_outputs::<u16>(input, axes, spacings, requested, context)
        }
        openmat_array::DType::I32 => {
            gradient_integer_outputs::<i32>(input, axes, spacings, requested, context)
        }
        openmat_array::DType::U32 => {
            gradient_integer_outputs::<u32>(input, axes, spacings, requested, context)
        }
        openmat_array::DType::I64 => {
            gradient_integer_outputs::<i64>(input, axes, spacings, requested, context)
        }
        openmat_array::DType::U64 => {
            gradient_integer_outputs::<u64>(input, axes, spacings, requested, context)
        }
        _ => Err(type_error("gradient", 1, "real numeric array", input)),
    }
}

fn spacing_dense<T: CalculusConvert>(spacing: &Spacing) -> Result<DenseArray<T>, BuiltinError> {
    let value = match spacing {
        Spacing::Scalar(value) | Spacing::Vector(value) => value,
    };
    T::dense_from_value(value)
        .ok_or_else(|| type_error("gradient", 2, "numeric spacing scalar or vector", value))
}

fn gradient_float_dense<T: FloatCalculus>(
    input: &DenseArray<T>,
    axis: usize,
    spacing: &DenseArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let stride = axis_stride(input.shape(), axis)?;
    let extent = host_length("gradient", input.shape().extent(axis))?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error("gradient"))?;
    let outer = trailing_outer(input.shape(), axis)?;
    let scalar_spacing = spacing.numel() == 1;
    let mut output = vec![T::zero(); input.as_slice().len()];
    for outer_index in 0..outer {
        for inner in 0..stride {
            context.check_cancelled()?;
            let base = outer_index * block + inner;
            for coordinate in 0..extent {
                let offset = base + coordinate * stride;
                output[offset] = if extent <= 1 {
                    T::zero()
                } else {
                    let (left, right) = gradient_neighbors(coordinate, extent);
                    let numerator = input.as_slice()[base + right * stride]
                        .subtract(input.as_slice()[base + left * stride]);
                    let denominator = if scalar_spacing {
                        let steps = if coordinate == 0 || coordinate + 1 == extent {
                            1.0
                        } else {
                            2.0
                        };
                        spacing.as_slice()[0].multiply(T::from_real(steps))
                    } else {
                        spacing.as_slice()[right].subtract(spacing.as_slice()[left])
                    };
                    numerator.divide(denominator)
                };
            }
        }
    }
    DenseArray::from_vec(input.shape().clone(), output).map_err(|error| array_error(&error))
}

fn gradient_integer_outputs<T: MatlabInteger + MatlabCast + openmat_array::IntegerElement>(
    input: &Value,
    axes: &[usize],
    spacings: &[Spacing],
    requested: usize,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let input_array = dense_integer::<T>(input)
        .ok_or_else(|| type_error("gradient", 1, "real integer array", input))?;
    let mut outputs = reserved_vec("gradient", requested)?;
    for output_index in 0..requested {
        let spacing_value = match &spacings[output_index] {
            Spacing::Scalar(value) | Spacing::Vector(value) => value,
        };
        let spacing = dense_f64(spacing_value).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Type,
                "integer `gradient` requires real spacing",
            )
        })?;
        let values = gradient_integer_dense(&input_array, axes[output_index], &spacing, context)?;
        outputs.push(Value::Array(ArrayData::Integer(
            IntegerArrayData::from_typed(values),
        )));
    }
    Ok(outputs)
}

fn gradient_integer_dense<T: MatlabInteger>(
    input: &DenseArray<T>,
    axis: usize,
    spacing: &DenseArray<f64>,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let stride = axis_stride(input.shape(), axis)?;
    let extent = host_length("gradient", input.shape().extent(axis))?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error("gradient"))?;
    let outer = trailing_outer(input.shape(), axis)?;
    let scalar_spacing = spacing.numel() == 1;
    let mut output = vec![T::from_f64(0.0); input.as_slice().len()];
    for outer_index in 0..outer {
        for inner in 0..stride {
            context.check_cancelled()?;
            let base = outer_index * block + inner;
            for coordinate in 0..extent {
                if extent <= 1 {
                    continue;
                }
                let (left, right) = gradient_neighbors(coordinate, extent);
                let difference = input.as_slice()[base + right * stride]
                    .saturating_sub(input.as_slice()[base + left * stride]);
                let width = if scalar_spacing {
                    spacing.as_slice()[0]
                        * if coordinate == 0 || coordinate + 1 == extent {
                            1.0
                        } else {
                            2.0
                        }
                } else {
                    spacing.as_slice()[right] - spacing.as_slice()[left]
                };
                output[base + coordinate * stride] = T::from_f64(difference.to_f64() / width);
            }
        }
    }
    DenseArray::from_vec(input.shape().clone(), output).map_err(|error| array_error(&error))
}

fn gradient_neighbors(coordinate: usize, extent: usize) -> (usize, usize) {
    if coordinate == 0 {
        (0, 1)
    } else if coordinate + 1 == extent {
        (extent - 2, extent - 1)
    } else {
        (coordinate - 1, coordinate + 1)
    }
}

#[allow(clippy::cast_precision_loss)]
fn dense_f64(value: &Value) -> Option<DenseArray<f64>> {
    match value {
        Value::Logical(value) => scalar_dense(f64::from(*value)).ok(),
        Value::Double(value) => scalar_dense(*value).ok(),
        Value::Array(ArrayData::F32(array)) => map_dense(array, |value| f64::from(*value)).ok(),
        Value::Array(ArrayData::F64(array)) => Some(array.clone()),
        Value::Array(ArrayData::Logical(array)) => {
            map_dense(array, |value| f64::from(value.get())).ok()
        }
        Value::Array(ArrayData::Char(array)) => {
            map_dense(array, |value| f64::from(value.get())).ok()
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            let values = array
                .elements()
                .map(|value| match value.real_component() {
                    openmat_array::IntegerComponent::Signed(value) => value as f64,
                    openmat_array::IntegerComponent::Unsigned(value) => value as f64,
                })
                .collect();
            DenseArray::from_vec(array.shape().clone(), values).ok()
        }
        _ => None,
    }
}

#[allow(clippy::cast_possible_truncation)]
fn dense_f32(value: &Value) -> Option<DenseArray<f32>> {
    match value {
        Value::Logical(value) => scalar_dense(f32::from(*value)).ok(),
        Value::Double(value) => scalar_dense(*value as f32).ok(),
        Value::Array(ArrayData::F32(array)) => Some(array.clone()),
        Value::Array(ArrayData::F64(array)) => map_dense(array, |value| *value as f32).ok(),
        Value::Array(ArrayData::Logical(array)) => {
            map_dense(array, |value| f32::from(value.get())).ok()
        }
        Value::Array(ArrayData::Char(array)) => {
            map_dense(array, |value| f32::from(value.get())).ok()
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            dense_f64(value).and_then(|array| map_dense(&array, |value| *value as f32).ok())
        }
        _ => None,
    }
}

fn dense_complex64(value: &Value) -> Option<DenseArray<ArrayComplex64>> {
    match value {
        Value::Complex(value) => {
            scalar_dense(ArrayComplex64::new(value.real, value.imaginary)).ok()
        }
        Value::Array(ArrayData::ComplexF64(array)) => Some(array.clone()),
        Value::Array(ArrayData::ComplexF32(array)) => map_dense(array, |value| {
            ArrayComplex64::new(f64::from(value.re), f64::from(value.im))
        })
        .ok(),
        _ => dense_f64(value)
            .and_then(|array| map_dense(&array, |value| ArrayComplex64::new(*value, 0.0)).ok()),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn dense_complex32(value: &Value) -> Option<DenseArray<Complex32>> {
    match value {
        Value::Complex(value) => {
            scalar_dense(Complex32::new(value.real as f32, value.imaginary as f32)).ok()
        }
        Value::Array(ArrayData::ComplexF64(array)) => map_dense(array, |value| {
            Complex32::new(value.re as f32, value.im as f32)
        })
        .ok(),
        Value::Array(ArrayData::ComplexF32(array)) => Some(array.clone()),
        _ => dense_f32(value)
            .and_then(|array| map_dense(&array, |value| Complex32::new(*value, 0.0)).ok()),
    }
}

fn dense_logical(value: &Value) -> Option<DenseArray<Logical>> {
    match value {
        Value::Logical(value) => scalar_dense(Logical::from(*value)).ok(),
        Value::Array(ArrayData::Logical(array)) => Some(array.clone()),
        _ => None,
    }
}

fn dense_char(value: &Value) -> Option<DenseArray<CharCodeUnit>> {
    match value {
        Value::Array(ArrayData::Char(array)) => Some(array.clone()),
        Value::Double(value) => scalar_dense(CharCodeUnit::new(matlab_cast::<u16>(*value))).ok(),
        Value::Array(ArrayData::F64(array)) => {
            map_dense(array, |value| CharCodeUnit::new(matlab_cast::<u16>(*value))).ok()
        }
        _ => None,
    }
}

trait MatlabCast: Sized {
    fn from_matlab_double(value: f64) -> Self;
}

macro_rules! impl_matlab_cast {
    ($($kind:ty),+ $(,)?) => {$(
        impl MatlabCast for $kind {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            fn from_matlab_double(value: f64) -> Self {
                value.round() as Self
            }
        }
    )+};
}

impl_matlab_cast!(i8, u8, i16, u16, i32, u32, i64, u64);

fn matlab_cast<T: MatlabCast>(value: f64) -> T {
    T::from_matlab_double(value)
}

fn dense_integer<T: openmat_array::IntegerElement + MatlabCast>(
    value: &Value,
) -> Option<DenseArray<T>> {
    match value {
        Value::Array(ArrayData::Integer(array)) => array.as_typed::<T>().cloned(),
        Value::Double(value) => scalar_dense(matlab_cast::<T>(*value)).ok(),
        Value::Array(ArrayData::F64(array)) => {
            map_dense(array, |value| matlab_cast::<T>(*value)).ok()
        }
        _ => None,
    }
}

fn map_dense<S, T>(
    input: &DenseArray<S>,
    map: impl FnMut(&S) -> T,
) -> Result<DenseArray<T>, BuiltinError> {
    DenseArray::from_vec(
        input.shape().clone(),
        input.as_slice().iter().map(map).collect(),
    )
    .map_err(|error| array_error(&error))
}

fn scalar_dense<T>(value: T) -> Result<DenseArray<T>, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map_err(|error| array_error(&error))
}

fn numeric_dimensions<'a>(
    name: &str,
    value: &'a Value,
    allow_char_logical: bool,
) -> Result<&'a [u64], BuiltinError> {
    let accepted = match value.dtype() {
        Some(dtype) => {
            !is_complex_integer_dtype(dtype)
                && (allow_char_logical
                    || !matches!(
                        dtype,
                        openmat_array::DType::Char | openmat_array::DType::Logical
                    ))
        }
        None => false,
    };
    if !accepted {
        return Err(type_error(name, 1, "numeric array", value));
    }
    value
        .dimensions()
        .ok_or_else(|| type_error(name, 1, "numeric array", value))
}

fn is_complex_integer_dtype(dtype: openmat_array::DType) -> bool {
    matches!(
        dtype,
        openmat_array::DType::ComplexI8
            | openmat_array::DType::ComplexU8
            | openmat_array::DType::ComplexI16
            | openmat_array::DType::ComplexU16
            | openmat_array::DType::ComplexI32
            | openmat_array::DType::ComplexU32
            | openmat_array::DType::ComplexI64
            | openmat_array::DType::ComplexU64
    )
}

fn is_single_dtype(dtype: openmat_array::DType) -> bool {
    matches!(
        dtype,
        openmat_array::DType::F32 | openmat_array::DType::ComplexF32
    )
}

fn positive_dimension(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    if matches!(value, Value::Logical(_) | Value::Complex(_)) {
        return Err(dimension_error(name, position));
    }
    if let Some(component) = exact_real_integer_scalar(value) {
        let dimension = match component {
            openmat_array::IntegerComponent::Signed(value) => u64::try_from(value).ok(),
            openmat_array::IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
        };
        return dimension
            .filter(|value| *value > 0)
            .ok_or_else(|| dimension_error(name, position));
    }
    let number = dense_f64(value)
        .filter(|array| array.numel() == 1)
        .and_then(|array| array.as_slice().first().copied())
        .ok_or_else(|| dimension_error(name, position))?;
    if !number.is_finite()
        || number < 1.0
        || number.fract() != 0.0
        || number >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(dimension_error(name, position));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as u64)
}

fn nonnegative_integer(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    if matches!(value, Value::Logical(_) | Value::Complex(_)) {
        return Err(domain_scalar_error(name, position));
    }
    if let Some(component) = exact_real_integer_scalar(value) {
        return match component {
            openmat_array::IntegerComponent::Signed(value) => u64::try_from(value).ok(),
            openmat_array::IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
        }
        .ok_or_else(|| domain_scalar_error(name, position));
    }
    let number = dense_f64(value)
        .filter(|array| array.numel() == 1)
        .and_then(|array| array.as_slice().first().copied())
        .ok_or_else(|| domain_scalar_error(name, position))?;
    if !number.is_finite()
        || number < 0.0
        || number.fract() != 0.0
        || number >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(domain_scalar_error(name, position));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as u64)
}

fn default_axis(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|axis| u64::try_from(axis + 1).ok())
        .unwrap_or(1)
}

fn axis_index(dimension: u64, name: &str) -> Result<usize, BuiltinError> {
    usize::try_from(dimension - 1).map_err(|_| mapping_error(name))
}

fn axis_stride(shape: &Shape, axis: usize) -> Result<usize, BuiltinError> {
    shape
        .dimensions()
        .iter()
        .take(axis)
        .try_fold(1_u64, |stride, extent| stride.checked_mul(*extent))
        .and_then(|stride| usize::try_from(stride).ok())
        .ok_or_else(|| mapping_error("axis mapping"))
}

fn trailing_outer(shape: &Shape, axis: usize) -> Result<usize, BuiltinError> {
    shape
        .dimensions()
        .iter()
        .skip(axis + 1)
        .try_fold(1_u64, |outer, extent| outer.checked_mul(*extent))
        .and_then(|outer| usize::try_from(outer).ok())
        .ok_or_else(|| mapping_error("axis mapping"))
}

fn comparison_method(value: &Value) -> Result<ComparisonMethod, BuiltinError> {
    match keyword(value).as_deref() {
        Some("auto") => Ok(ComparisonMethod::Auto),
        Some("real") => Ok(ComparisonMethod::Real),
        Some("abs") => Ok(ComparisonMethod::Absolute),
        _ => Err(option_error("comparison method")),
    }
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

fn index_column(name: &str, values: Vec<f64>) -> Result<Value, BuiltinError> {
    let length = u64::try_from(values.len()).map_err(|_| mapping_error(name))?;
    DenseArray::from_vec(
        Shape::new([length, 1]).map_err(|error| array_error(&error))?,
        values,
    )
    .map(ArrayData::F64)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_precision_loss)]
fn index_as_f64(index: usize) -> Result<f64, BuiltinError> {
    index
        .checked_add(1)
        .map(|value| value as f64)
        .ok_or_else(|| mapping_error("index"))
}

fn host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| mapping_error(name))
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
        context.check_cancelled()?;
    }
    Ok(())
}

fn set_type_error(name: &str, position: usize, value: &Value) -> BuiltinError {
    type_error(
        name,
        position,
        "compatible numeric, logical, char, or integer array",
        value,
    )
}

fn dimension_error(name: &str, position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `{name}` must be a positive real integer dimension"),
    )
}

fn domain_scalar_error(name: &str, position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `{name}` must be a nonnegative integer scalar"),
    )
}

fn option_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` received an unsupported option form"),
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` computed an invalid checked column-major mapping"),
    )
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use openmat_array::{ArrayData, DenseArray, IntegerArrayData, Shape};
    use openmat_runtime::{BuiltinContext, BuiltinErrorCategory, CancellationToken, VecOutput};
    use openmat_value::Value;

    use super::{
        cumtrapz_builtin, gradient_builtin, intersect_builtin, issorted_builtin, maxk_builtin,
        mink_builtin, setdiff_builtin, setxor_builtin, trapz_builtin, union_builtin,
    };

    type Builtin = for<'a> fn(
        &[Value],
        &mut BuiltinContext<'a>,
    ) -> Result<Vec<Value>, openmat_runtime::BuiltinError>;

    fn invoke(function: Builtin, arguments: &[Value], outputs: usize) -> Vec<Value> {
        let cancellation = CancellationToken::new();
        let mut output = VecOutput::new();
        let mut context = BuiltinContext::new(outputs, &cancellation, &mut output);
        function(arguments, &mut context).expect("focused built-in invocation")
    }

    fn invoke_error(
        function: Builtin,
        arguments: &[Value],
        outputs: usize,
    ) -> BuiltinErrorCategory {
        let cancellation = CancellationToken::new();
        let mut output = VecOutput::new();
        let mut context = BuiltinContext::new(outputs, &cancellation, &mut output);
        function(arguments, &mut context)
            .expect_err("focused invalid invocation")
            .category
    }

    fn real_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    fn single_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f32>) -> Value {
        Value::Array(ArrayData::F32(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    fn integer_array<T: openmat_array::IntegerElement>(
        dimensions: impl IntoIterator<Item = u64>,
        values: Vec<T>,
    ) -> Value {
        Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        )))
    }

    fn reals(value: &Value) -> (&[u64], &[f64]) {
        let Value::Array(ArrayData::F64(array)) = value else {
            panic!("expected double array")
        };
        (array.shape().dimensions(), array.as_slice())
    }

    #[test]
    fn set_operations_cover_sorted_stable_rows_nan_and_indices() {
        let left = real_array([1, 5], vec![3.0, f64::NAN, 1.0, 3.0, -0.0]);
        let right = real_array([1, 4], vec![1.0, 3.0, f64::NAN, 0.0]);
        let outputs = invoke(intersect_builtin, &[left.clone(), right.clone()], 3);
        let (shape, values) = reals(&outputs[0]);
        assert_eq!(shape, [1, 3]);
        assert_eq!(&values[..2], &[0.0, 1.0]);
        assert_eq!(values[2], 3.0);
        assert_eq!(reals(&outputs[1]).1, &[5.0, 3.0, 1.0]);
        assert_eq!(reals(&outputs[2]).1, &[4.0, 1.0, 2.0]);

        let outputs = invoke(union_builtin, &[left.clone(), right.clone()], 3);
        let (_, values) = reals(&outputs[0]);
        assert_eq!(values.len(), 5);
        assert_eq!(&values[..3], &[0.0, 1.0, 3.0]);
        assert!(values[3].is_nan());
        assert!(values[4].is_nan());
        assert_eq!(reals(&outputs[1]).1, &[5.0, 3.0, 1.0, 2.0]);
        assert_eq!(reals(&outputs[2]).1, &[3.0]);

        let stable = Value::from("stable");
        let stable_left = real_array([1, 4], vec![3.0, 1.0, 3.0, 2.0]);
        let stable_right = real_array([1, 4], vec![4.0, 3.0, 1.0, 4.0]);
        let outputs = invoke(
            setxor_builtin,
            &[stable_left, stable_right, stable.clone()],
            3,
        );
        assert_eq!(reals(&outputs[0]).1, &[2.0, 4.0]);
        assert_eq!(reals(&outputs[1]).1, &[4.0]);
        assert_eq!(reals(&outputs[2]).1, &[1.0]);

        let rows_left = real_array([3, 2], vec![3.0, 1.0, 3.0, 4.0, 2.0, 4.0]);
        let rows_right = real_array([2, 2], vec![5.0, 1.0, 6.0, 2.0]);
        let outputs = invoke(
            union_builtin,
            &[rows_left, rows_right, Value::from("rows")],
            3,
        );
        assert_eq!(reals(&outputs[0]).0, &[3, 2]);
        assert_eq!(reals(&outputs[0]).1, &[1.0, 3.0, 5.0, 2.0, 4.0, 6.0]);
        assert_eq!(reals(&outputs[1]).1, &[2.0, 1.0]);
        assert_eq!(reals(&outputs[2]).1, &[1.0]);

        let difference = invoke(setdiff_builtin, &[left, right, stable], 2);
        assert_eq!(reals(&difference[0]).1.len(), 1);
        assert!(reals(&difference[0]).1[0].is_nan());
        assert_eq!(reals(&difference[1]).1, &[2.0]);
    }

    #[test]
    fn set_mixed_double_integer_uses_r2022b_rounding_and_saturation() {
        let output = invoke(
            union_builtin,
            &[
                integer_array([1, 2], vec![1_u8, 2]),
                real_array([1, 3], vec![2.5, 300.0, -1.0]),
            ],
            1,
        );
        let Value::Array(ArrayData::Integer(IntegerArrayData::U8(array))) = &output[0] else {
            panic!("expected uint8 output")
        };
        assert_eq!(array.as_slice(), &[0, 1, 2, 3, 255]);
    }

    #[test]
    fn ordering_and_selection_cover_dimensions_ties_missing_and_classes() {
        assert_eq!(
            invoke(
                issorted_builtin,
                &[real_array([1, 3], vec![1.0, 2.0, f64::NAN])],
                1,
            ),
            vec![Value::Logical(true)]
        );
        assert_eq!(
            invoke(
                issorted_builtin,
                &[
                    real_array([1, 3], vec![f64::NAN, 2.0, 1.0]),
                    Value::from("descend"),
                ],
                1,
            ),
            vec![Value::Logical(true)]
        );

        let input = real_array(
            [1, 6],
            vec![3.0, f64::NAN, 1.0, 3.0, f64::NEG_INFINITY, f64::INFINITY],
        );
        let minimum = invoke(mink_builtin, &[input.clone(), Value::Double(3.0)], 2);
        assert_eq!(reals(&minimum[0]).1, &[f64::NEG_INFINITY, 1.0, 3.0]);
        assert_eq!(reals(&minimum[1]).1, &[5.0, 3.0, 1.0]);
        let maximum = invoke(maxk_builtin, &[input, Value::Double(3.0)], 2);
        assert_eq!(reals(&maximum[0]).1, &[f64::INFINITY, 3.0, 3.0]);
        assert_eq!(reals(&maximum[1]).1, &[6.0, 1.0, 4.0]);

        let matrix = real_array([2, 2], vec![3.0, 4.0, 1.0, 2.0]);
        let dim_two = invoke(
            mink_builtin,
            &[matrix, Value::Double(1.0), Value::Double(2.0)],
            2,
        );
        assert_eq!(reals(&dim_two[0]).0, &[2, 1]);
        assert_eq!(reals(&dim_two[0]).1, &[1.0, 2.0]);
        assert_eq!(reals(&dim_two[1]).1, &[2.0, 2.0]);

        let single = invoke(
            maxk_builtin,
            &[
                single_array([1, 3], vec![3.0, 1.0, 2.0]),
                Value::Double(2.0),
            ],
            1,
        );
        let Value::Array(ArrayData::F32(array)) = &single[0] else {
            panic!("expected single output")
        };
        assert_eq!(array.as_slice(), &[3.0, 2.0]);
    }

    #[test]
    fn trapezoids_cover_default_explicit_dimensions_coordinates_and_native_classes() {
        let matrix = real_array([2, 3], vec![1.0, 2.0, 3.0, 5.0, 6.0, 9.0]);
        let reduced = invoke(trapz_builtin, std::slice::from_ref(&matrix), 1);
        assert_eq!(reals(&reduced[0]).0, &[1, 3]);
        assert_eq!(reals(&reduced[0]).1, &[1.5, 4.0, 7.5]);
        let along_two = invoke(trapz_builtin, &[matrix.clone(), Value::Double(2.0)], 1);
        assert_eq!(reals(&along_two[0]).1, &[6.5, 10.5]);
        let cumulative = invoke(cumtrapz_builtin, &[matrix, Value::Double(2.0)], 1);
        assert_eq!(reals(&cumulative[0]).1, &[0.0, 0.0, 2.0, 3.5, 6.5, 10.5]);

        let coordinates = real_array([1, 3], vec![0.0, 2.0, 5.0]);
        let samples = real_array([1, 3], vec![1.0, 3.0, 6.0]);
        let coordinate_integral = invoke(trapz_builtin, &[coordinates, samples], 1);
        assert_eq!(reals(&coordinate_integral[0]).1, &[17.5]);

        let single = invoke(
            trapz_builtin,
            &[single_array([1, 3], vec![1.0, 3.0, 6.0])],
            1,
        );
        let Value::Array(ArrayData::F32(array)) = &single[0] else {
            panic!("expected single trapz output")
        };
        assert_eq!(array.as_slice(), &[6.5]);

        let integer = invoke(
            cumtrapz_builtin,
            &[integer_array([1, 3], vec![1_u8, 2, 4])],
            1,
        );
        let Value::Array(ArrayData::Integer(IntegerArrayData::U8(array))) = &integer[0] else {
            panic!("expected native integer cumtrapz output")
        };
        assert_eq!(array.as_slice(), &[0, 2, 5]);
    }

    #[test]
    fn gradient_covers_vector_matrix_nd_spacing_complex_nan_and_integer_paths() {
        let vector = invoke(
            gradient_builtin,
            &[real_array([1, 3], vec![1.0, 4.0, 9.0])],
            1,
        );
        assert_eq!(reals(&vector[0]).1, &[3.0, 4.0, 5.0]);
        let nonuniform = invoke(
            gradient_builtin,
            &[
                real_array([1, 3], vec![1.0, 4.0, 9.0]),
                real_array([1, 3], vec![0.0, 1.0, 3.0]),
            ],
            1,
        );
        assert_eq!(reals(&nonuniform[0]).1, &[3.0, 8.0 / 3.0, 2.5]);

        let matrix = real_array([2, 3], vec![1.0, 2.0, 3.0, 5.0, 6.0, 9.0]);
        let outputs = invoke(gradient_builtin, &[matrix], 2);
        assert_eq!(reals(&outputs[0]).1, &[2.0, 3.0, 2.5, 3.5, 3.0, 4.0]);
        assert_eq!(reals(&outputs[1]).1, &[1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);

        let integer = invoke(
            gradient_builtin,
            &[integer_array([1, 3], vec![1_u8, 2, 4])],
            1,
        );
        let Value::Array(ArrayData::Integer(IntegerArrayData::U8(array))) = &integer[0] else {
            panic!("expected native integer gradient output")
        };
        assert_eq!(array.as_slice(), &[1, 2, 2]);

        let special = invoke(
            gradient_builtin,
            &[real_array([1, 3], vec![1.0, f64::NAN, f64::INFINITY])],
            1,
        );
        let values = reals(&special[0]).1;
        assert!(values[0].is_nan());
        assert!(values[1].is_infinite());
        assert!(values[2].is_nan());

        let empty_matrix = invoke(gradient_builtin, &[real_array([0, 3], Vec::new())], 2);
        assert_eq!(reals(&empty_matrix[0]).0, &[0, 3]);
        assert_eq!(reals(&empty_matrix[1]).0, &[0, 3]);
    }

    #[test]
    fn invalid_forms_preserve_argument_type_domain_and_shape_categories() {
        assert_eq!(
            invoke_error(
                union_builtin,
                &[
                    integer_array([1, 1], vec![1_u8]),
                    integer_array([1, 1], vec![1_u16]),
                ],
                1,
            ),
            BuiltinErrorCategory::Type
        );
        assert_eq!(
            invoke_error(
                setdiff_builtin,
                &[
                    real_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]),
                    real_array([1, 3], vec![1.0, 2.0, 3.0]),
                    Value::from("rows"),
                ],
                1,
            ),
            BuiltinErrorCategory::Domain
        );
        assert_eq!(
            invoke_error(
                mink_builtin,
                &[real_array([1, 2], vec![1.0, 2.0]), Value::Double(-1.0),],
                1,
            ),
            BuiltinErrorCategory::Domain
        );
        assert_eq!(
            invoke_error(
                trapz_builtin,
                &[real_array([1, 2], vec![1.0, 2.0]), Value::Double(0.0),],
                1,
            ),
            BuiltinErrorCategory::Domain
        );
        assert_eq!(
            invoke_error(
                gradient_builtin,
                &[real_array([1, 3], vec![1.0, 2.0, 4.0])],
                2,
            ),
            BuiltinErrorCategory::ArgumentCount
        );
    }
}

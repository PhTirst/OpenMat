use openmat_array::{
    ArrayData, Complex32, Complex64 as ArrayComplex64, ComplexInteger, DenseArray,
    IntegerArrayData, IntegerComponent, IntegerElement, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{Complex64, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count_range,
    expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

trait MatlabArithmetic: Copy {
    fn zero() -> Self;
    fn one() -> Self;
    fn difference(self, previous: Self) -> Self;
    fn add(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn is_missing(self) -> bool {
        false
    }
}

macro_rules! impl_float_arithmetic {
    ($kind:ty) => {
        impl MatlabArithmetic for $kind {
            fn zero() -> Self {
                0.0
            }
            fn one() -> Self {
                1.0
            }
            fn difference(self, previous: Self) -> Self {
                self - previous
            }
            fn add(self, right: Self) -> Self {
                self + right
            }
            fn multiply(self, right: Self) -> Self {
                self * right
            }
            fn is_missing(self) -> bool {
                self.is_nan()
            }
        }
    };
}

impl_float_arithmetic!(f32);
impl_float_arithmetic!(f64);

macro_rules! impl_integer_arithmetic {
    ($($kind:ty),+ $(,)?) => {$(
        impl MatlabArithmetic for $kind {
            fn zero() -> Self { 0 }
            fn one() -> Self { 1 }
            fn difference(self, previous: Self) -> Self { self.saturating_sub(previous) }
            fn add(self, right: Self) -> Self { self.saturating_add(right) }
            fn multiply(self, right: Self) -> Self { self.saturating_mul(right) }
        }
    )+};
}

impl_integer_arithmetic!(i8, u8, i16, u16, i32, u32, i64, u64);

macro_rules! impl_complex_float_arithmetic {
    ($kind:ty, $component:ty) => {
        impl MatlabArithmetic for $kind {
            fn zero() -> Self {
                Self::new(0.0, 0.0)
            }
            fn one() -> Self {
                Self::new(1.0, 0.0)
            }
            fn difference(self, previous: Self) -> Self {
                Self::new(self.re - previous.re, self.im - previous.im)
            }
            fn add(self, right: Self) -> Self {
                Self::new(self.re + right.re, self.im + right.im)
            }
            fn multiply(self, right: Self) -> Self {
                Self::new(
                    self.re.mul_add(right.re, -(self.im * right.im)),
                    self.re.mul_add(right.im, self.im * right.re),
                )
            }
            fn is_missing(self) -> bool {
                self.re.is_nan() || self.im.is_nan()
            }
        }
    };
}

impl_complex_float_arithmetic!(Complex32, f32);
impl_complex_float_arithmetic!(ArrayComplex64, f64);

impl<T> MatlabArithmetic for ComplexInteger<T>
where
    T: MatlabArithmetic + IntegerElement,
    ComplexInteger<T>: IntegerElement,
{
    fn zero() -> Self {
        Self::new(T::zero(), T::zero())
    }

    fn one() -> Self {
        Self::new(T::one(), T::zero())
    }

    fn difference(self, previous: Self) -> Self {
        Self::new(
            self.re().difference(previous.re()),
            self.im().difference(previous.im()),
        )
    }

    fn add(self, right: Self) -> Self {
        Self::new(self.re().add(right.re()), self.im().add(right.im()))
    }

    fn multiply(self, right: Self) -> Self {
        let real = self
            .re()
            .multiply(right.re())
            .difference(self.im().multiply(right.im()));
        let imaginary = self
            .re()
            .multiply(right.im())
            .add(self.im().multiply(right.re()));
        Self::new(real, imaginary)
    }
}

pub(super) fn diff_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("diff", arguments, 1, 3)?;
    expect_max_outputs("diff", context, 1)?;
    context.check_cancelled()?;
    let order = arguments
        .get(1)
        .map_or(Ok(1_u64), |value| positive_integer("diff", 2, value))?;
    let axis = arguments
        .get(2)
        .map(|value| positive_integer("diff", 3, value));
    let output = difference_value(&arguments[0], order, axis.transpose()?, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CumulativeOperation {
    Sum,
    Product,
}

impl CumulativeOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::Sum => "cumsum",
            Self::Product => "cumprod",
        }
    }
}

pub(super) fn cumsum_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    cumulative_builtin(CumulativeOperation::Sum, arguments, context)
}

pub(super) fn cumprod_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    cumulative_builtin(CumulativeOperation::Product, arguments, context)
}

#[derive(Clone, Copy)]
struct CumulativeOptions {
    axis: u64,
    reverse: bool,
    omit_missing: bool,
}

fn cumulative_builtin(
    operation: CumulativeOperation,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = operation.name();
    expect_argument_count_range(name, arguments, 1, 4)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = arguments[0]
        .dimensions()
        .ok_or_else(|| type_error(name, 1, "numeric or logical array", &arguments[0]))?;
    let options = cumulative_options(name, arguments, dimensions)?;
    let output = cumulative_value(operation, &arguments[0], options, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn cumulative_options(
    name: &str,
    arguments: &[Value],
    dimensions: &[u64],
) -> Result<CumulativeOptions, BuiltinError> {
    let mut axis = default_axis(dimensions);
    let mut reverse = false;
    let mut omit_missing = false;
    let mut index = 1;
    if let Some(value) = arguments.get(index)
        && keyword(value).is_none()
    {
        axis = positive_integer(name, index + 1, value)?;
        index += 1;
    }
    while let Some(value) = arguments.get(index) {
        let option = keyword(value).ok_or_else(|| option_error(name))?;
        match option.as_str() {
            "forward" => reverse = false,
            "reverse" => reverse = true,
            "includenan" => omit_missing = false,
            "omitnan" => omit_missing = true,
            _ => return Err(option_error(name)),
        }
        index += 1;
    }
    Ok(CumulativeOptions {
        axis,
        reverse,
        omit_missing,
    })
}

fn difference_value(
    input: &Value,
    order: u64,
    axis: Option<u64>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! difference_dense_value {
        ($array:expr, $wrap:expr) => {{
            let array = difference_dense($array, order, axis, context)?;
            Ok($wrap(array))
        }};
    }
    match input {
        Value::Logical(value) => {
            let array = scalar_array(f64::from(*value))?;
            difference_dense_value!(&array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Double(value) => {
            let array = scalar_array(*value)?;
            difference_dense_value!(&array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Complex(value) => {
            let array = scalar_array(ArrayComplex64::new(value.real, value.imaginary))?;
            difference_dense_value!(&array, |array| Value::Array(ArrayData::ComplexF64(array)))
        }
        Value::Array(ArrayData::F32(array)) => {
            difference_dense_value!(array, |array| { Value::Array(ArrayData::F32(array)) })
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            difference_dense_value!(array, |array| Value::Array(ArrayData::ComplexF32(array)))
        }
        Value::Array(ArrayData::Logical(array)) => {
            let values = array
                .as_slice()
                .iter()
                .map(|value| f64::from(value.get()))
                .collect();
            let array = DenseArray::from_vec(array.shape().clone(), values)
                .map_err(|error| array_error(&error))?;
            difference_dense_value!(&array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::Char(array)) => {
            let values = array
                .as_slice()
                .iter()
                .map(|value| f64::from(value.get()))
                .collect();
            let array = DenseArray::from_vec(array.shape().clone(), values)
                .map_err(|error| array_error(&error))?;
            difference_dense_value!(&array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::F64(array)) => {
            difference_dense_value!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => difference_dense_value!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Value::Array(ArrayData::Integer(array)) => difference_integer(array, order, axis, context)
            .map(ArrayData::Integer)
            .map(Value::Array),
        value => Err(type_error(
            "diff",
            1,
            "numeric, logical, char, or integer array",
            value,
        )),
    }
}

fn cumulative_value(
    operation: CumulativeOperation,
    input: &Value,
    options: CumulativeOptions,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    macro_rules! cumulative_dense_value {
        ($array:expr, $wrap:expr) => {{
            let array = cumulative_dense(name, operation, $array, options, context)?;
            Ok($wrap(array))
        }};
    }
    match input {
        Value::Logical(value) => Ok(Value::Double(f64::from(*value))),
        Value::Double(value) => Ok(Value::Double(*value)),
        Value::Complex(value) => Ok(Value::Complex(Complex64::new(value.real, value.imaginary))),
        Value::Array(ArrayData::F32(array)) => {
            cumulative_dense_value!(array, |array| { Value::Array(ArrayData::F32(array)) })
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            cumulative_dense_value!(array, |array| Value::Array(ArrayData::ComplexF32(array)))
        }
        Value::Array(ArrayData::Logical(array)) => {
            let values = array
                .as_slice()
                .iter()
                .map(|value| f64::from(value.get()))
                .collect();
            let array = DenseArray::from_vec(array.shape().clone(), values)
                .map_err(|error| array_error(&error))?;
            cumulative_dense_value!(&array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::F64(array)) => {
            cumulative_dense_value!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => cumulative_dense_value!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Value::Array(ArrayData::Integer(array)) => {
            cumulative_integer(array, operation, options, context)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        value => Err(type_error(name, 1, "numeric or logical array", value)),
    }
}

fn difference_dense<T: MatlabArithmetic>(
    input: &DenseArray<T>,
    order: u64,
    explicit_axis: Option<u64>,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let mut current = input.clone();
    let mut remaining = order;
    while remaining > 0 {
        context.check_cancelled()?;
        let axis = if let Some(axis) = explicit_axis {
            usize::try_from(axis - 1).map_err(|_| mapping_error("diff"))?
        } else if let Some(axis) = current
            .shape()
            .dimensions()
            .iter()
            .position(|extent| *extent != 1)
        {
            axis
        } else {
            return DenseArray::from_vec(
                Shape::new([0, 0]).map_err(|error| array_error(&error))?,
                Vec::new(),
            )
            .map_err(|error| array_error(&error));
        };
        if current.shape().extent(axis) == 0 {
            return Ok(current);
        }
        current = difference_once(&current, axis, context)?;
        remaining -= 1;
        if explicit_axis.is_some() && current.shape().extent(axis) == 0 {
            return Ok(current);
        }
    }
    Ok(current)
}

fn difference_once<T: MatlabArithmetic>(
    input: &DenseArray<T>,
    axis: usize,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let extent = input.shape().extent(axis);
    let output_extent = extent.saturating_sub(1);
    let mut dimensions = input.shape().dimensions().to_vec();
    if axis >= dimensions.len() {
        dimensions.resize(axis + 1, 1);
    }
    dimensions[axis] = output_extent;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let output_length = host_length("diff", shape.numel())?;
    let stride = axis_stride(input.shape(), axis)?;
    let block = stride
        .checked_mul(usize::try_from(extent).map_err(|_| mapping_error("diff"))?)
        .ok_or_else(|| mapping_error("diff"))?;
    let outer = if block == 0 {
        0
    } else {
        input.as_slice().len() / block
    };
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_length)
        .map_err(|_| mapping_error("diff"))?;
    for outer_index in 0..outer {
        for coordinate in 0..usize::try_from(output_extent).map_err(|_| mapping_error("diff"))? {
            for inner in 0..stride {
                check_cancelled_at(context, output.len())?;
                let previous = outer_index * block + coordinate * stride + inner;
                let current = previous + stride;
                output.push(input.as_slice()[current].difference(input.as_slice()[previous]));
            }
        }
    }
    DenseArray::from_vec(shape, output).map_err(|error| array_error(&error))
}

fn cumulative_dense<T: MatlabArithmetic>(
    name: &str,
    operation: CumulativeOperation,
    input: &DenseArray<T>,
    options: CumulativeOptions,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let axis = usize::try_from(options.axis - 1).map_err(|_| mapping_error(name))?;
    let extent = usize::try_from(input.shape().extent(axis)).map_err(|_| mapping_error(name))?;
    let stride = axis_stride(input.shape(), axis)?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error(name))?;
    let outer = if block == 0 {
        0
    } else {
        input.as_slice().len() / block
    };
    let mut output = input.as_slice().to_vec();
    for outer_index in 0..outer {
        for inner in 0..stride {
            let mut accumulator = match operation {
                CumulativeOperation::Sum => T::zero(),
                CumulativeOperation::Product => T::one(),
            };
            for step in 0..extent {
                check_cancelled_at(context, step)?;
                let coordinate = if options.reverse {
                    extent - 1 - step
                } else {
                    step
                };
                let offset = outer_index * block + coordinate * stride + inner;
                let value = input.as_slice()[offset];
                if !(options.omit_missing && value.is_missing()) {
                    accumulator = match operation {
                        CumulativeOperation::Sum => accumulator.add(value),
                        CumulativeOperation::Product => accumulator.multiply(value),
                    };
                }
                output[offset] = accumulator;
            }
        }
    }
    DenseArray::from_vec(input.shape().clone(), output).map_err(|error| array_error(&error))
}

fn difference_integer(
    input: &IntegerArrayData,
    order: u64,
    axis: Option<u64>,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! variant {
        ($array:expr) => {
            difference_dense($array, order, axis, context).map(IntegerArrayData::from_typed)
        };
    }
    dispatch_integer!(input, variant)
}

fn cumulative_integer(
    input: &IntegerArrayData,
    operation: CumulativeOperation,
    options: CumulativeOptions,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! variant {
        ($array:expr) => {
            cumulative_dense(operation.name(), operation, $array, options, context)
                .map(IntegerArrayData::from_typed)
        };
    }
    dispatch_integer!(input, variant)
}

macro_rules! dispatch_integer {
    ($input:expr, $operation:ident) => {
        match $input {
            IntegerArrayData::I8(array) => $operation!(array),
            IntegerArrayData::ComplexI8(array) => $operation!(array),
            IntegerArrayData::U8(array) => $operation!(array),
            IntegerArrayData::ComplexU8(array) => $operation!(array),
            IntegerArrayData::I16(array) => $operation!(array),
            IntegerArrayData::ComplexI16(array) => $operation!(array),
            IntegerArrayData::U16(array) => $operation!(array),
            IntegerArrayData::ComplexU16(array) => $operation!(array),
            IntegerArrayData::I32(array) => $operation!(array),
            IntegerArrayData::ComplexI32(array) => $operation!(array),
            IntegerArrayData::U32(array) => $operation!(array),
            IntegerArrayData::ComplexU32(array) => $operation!(array),
            IntegerArrayData::I64(array) => $operation!(array),
            IntegerArrayData::ComplexI64(array) => $operation!(array),
            IntegerArrayData::U64(array) => $operation!(array),
            IntegerArrayData::ComplexU64(array) => $operation!(array),
        }
    };
}

use dispatch_integer;

fn scalar_array<T: Clone>(value: T) -> Result<DenseArray<T>, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map_err(|error| array_error(&error))
}

fn axis_stride(shape: &Shape, axis: usize) -> Result<usize, BuiltinError> {
    shape
        .dimensions()
        .iter()
        .take(axis)
        .try_fold(1_u64, |stride, extent| stride.checked_mul(*extent))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| mapping_error("array accumulation"))
}

fn default_axis(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|axis| u64::try_from(axis + 1).ok())
        .unwrap_or(1)
}

fn positive_integer(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        let value = match value {
            IntegerComponent::Signed(value) => u64::try_from(value).ok(),
            IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
        };
        return value
            .filter(|value| *value > 0)
            .ok_or_else(|| dimension_error(name));
    }
    let value = match value {
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            array.as_slice().first().copied()
        }
        _ => value.as_real_single().map(f64::from),
    }
    .ok_or_else(|| type_error(name, position, "positive integer scalar", value))?;
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(dimension_error(name));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn keyword(value: &Value) -> Option<String> {
    let units: Vec<u16> = match value {
        Value::String(value) => value.as_scalar()?.code_units().to_vec(),
        Value::Array(ArrayData::Char(array))
            if array.shape().ndims() == 2 && array.shape().extent(0) == 1 =>
        {
            array.as_slice().iter().map(|value| value.get()).collect()
        }
        _ => return None,
    };
    String::from_utf16(&units)
        .ok()
        .map(|value| value.to_ascii_lowercase())
}

fn host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| mapping_error(name))
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

fn dimension_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` requires positive integer order and dimension inputs"),
    )
}

fn option_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` accepts only forward, reverse, includenan, or omitnan options"),
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` could not map the requested array shape"),
    )
}

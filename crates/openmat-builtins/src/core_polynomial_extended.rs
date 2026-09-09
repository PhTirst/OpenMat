//! Polynomial, convolution, and first-stage one-dimensional interpolation built-ins.
//!
//! `interp1` intentionally implements only `linear` and `nearest`; spline-family
//! methods are rejected explicitly. Numerical kernels operate directly on
//! column-major storage, use checked allocation, and cooperate with cancellation.

use std::cmp::Ordering;

use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData, IntegerElement, Shape,
};
use openmat_linalg::EigRequest;
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistrationError, BuiltinResult,
};
use openmat_value::Value;

use crate::core_linalg::{
    check_cancelled_at, checked_host_length, filled_values, keyword, linalg_error, reserved_values,
    wrap_complex32, wrap_complex64,
};
use crate::{
    BuiltinRegistry, array_error, expect_argument_count, expect_argument_count_range,
    expect_max_outputs, type_error,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Precision {
    Double,
    Single,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VectorOrientation {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConvolutionShape {
    Full,
    Same,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InterpolationMethod {
    Linear,
    Nearest,
}

#[derive(Clone, Copy, Debug)]
enum Extrapolation<T: Real> {
    Missing,
    Method,
    Value(ComplexValue<T>),
}

trait Real: Copy + Default + PartialEq + PartialOrd + std::fmt::Debug + Send + Sync + 'static {
    const ZERO: Self;
    const ONE: Self;
    const NAN: Self;

    fn from_usize(value: usize) -> Self;
    fn add(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
    fn negate(self) -> Self;
    fn is_finite(self) -> bool;
    fn is_nan(self) -> bool;
}

macro_rules! impl_real {
    ($type:ty) => {
        impl Real for $type {
            const ZERO: Self = 0.0;
            const ONE: Self = 1.0;
            const NAN: Self = <$type>::NAN;

            #[allow(clippy::cast_precision_loss)]
            fn from_usize(value: usize) -> Self {
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
            fn negate(self) -> Self {
                -self
            }
            fn is_finite(self) -> bool {
                self.is_finite()
            }
            fn is_nan(self) -> bool {
                self.is_nan()
            }
        }
    };
}

impl_real!(f64);
impl_real!(f32);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ComplexValue<T: Real> {
    re: T,
    im: T,
}

impl<T: Real> ComplexValue<T> {
    const fn new(re: T, im: T) -> Self {
        Self { re, im }
    }
    fn zero() -> Self {
        Self::new(T::ZERO, T::ZERO)
    }
    fn one() -> Self {
        Self::new(T::ONE, T::ZERO)
    }
    fn nan() -> Self {
        Self::new(T::NAN, T::ZERO)
    }
    fn add(self, right: Self) -> Self {
        Self::new(self.re.add(right.re), self.im.add(right.im))
    }
    fn subtract(self, right: Self) -> Self {
        Self::new(self.re.subtract(right.re), self.im.subtract(right.im))
    }
    fn multiply(self, right: Self) -> Self {
        Self::new(
            self.re
                .multiply(right.re)
                .subtract(self.im.multiply(right.im)),
            self.re.multiply(right.im).add(self.im.multiply(right.re)),
        )
    }
    fn divide(self, right: Self) -> Self {
        let denominator = right.re.multiply(right.re).add(right.im.multiply(right.im));
        Self::new(
            self.re
                .multiply(right.re)
                .add(self.im.multiply(right.im))
                .divide(denominator),
            self.im
                .multiply(right.re)
                .subtract(self.re.multiply(right.im))
                .divide(denominator),
        )
    }
    fn negate(self) -> Self {
        Self::new(self.re.negate(), self.im.negate())
    }
    fn scale(self, factor: T) -> Self {
        Self::new(self.re.multiply(factor), self.im.multiply(factor))
    }
    fn is_zero(self) -> bool {
        self.re == T::ZERO && self.im == T::ZERO
    }
    fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
}

#[derive(Clone, Debug)]
struct NumericArray<T: Real> {
    shape: Shape,
    values: Vec<ComplexValue<T>>,
}

impl<T: Real> NumericArray<T> {
    fn is_complex(&self) -> bool {
        self.values.iter().any(|value| value.im != T::ZERO)
    }
}

#[derive(Clone, Copy)]
struct NumericPolicy {
    logical: bool,
    integer: bool,
}

const FLOAT_ONLY: NumericPolicy = NumericPolicy {
    logical: false,
    integer: false,
};
const FLOAT_OR_LOGICAL: NumericPolicy = NumericPolicy {
    logical: true,
    integer: false,
};
const CONVOLUTION_NUMERIC: NumericPolicy = NumericPolicy {
    logical: true,
    integer: true,
};

/// Registers this focused surface into a caller-provided registry.
///
/// The root registry is outside this task's owned paths. The integration task
/// only needs to call this function once from `register_minimal`.
#[allow(dead_code)]
pub(super) fn register_polynomial_extended(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("conv", conv_builtin)?;
    registry.register("conv2", conv2_builtin)?;
    registry.register("deconv", deconv_builtin)?;
    registry.register("poly", poly_builtin)?;
    registry.register("polyval", polyval_builtin)?;
    registry.register("polyder", polyder_builtin)?;
    registry.register("polyint", polyint_builtin)?;
    registry.register("roots", roots_builtin)?;
    registry.register("interp1", interp1_builtin)?;
    Ok(())
}

pub(super) fn conv_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("conv", arguments, 2, 3)?;
    expect_max_outputs("conv", context, 1)?;
    context.check_cancelled()?;
    let shape = arguments
        .get(2)
        .map_or(Ok(ConvolutionShape::Full), |value| {
            convolution_shape("conv", value)
        })?;
    let output = match precision_of(arguments) {
        Precision::Double => conv_typed::<f64>(arguments, shape, context)?,
        Precision::Single => conv_typed::<f32>(arguments, shape, context)?,
    };
    Ok(vec![output])
}

fn conv_typed<T: Real + NumericOutput>(
    arguments: &[Value],
    requested_shape: ConvolutionShape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError>
where
    NumericArray<T>: NumericInput<T>,
{
    let left = NumericArray::<T>::read("conv", 1, &arguments[0], CONVOLUTION_NUMERIC, context)?;
    let right = NumericArray::<T>::read("conv", 2, &arguments[1], CONVOLUTION_NUMERIC, context)?;
    let left_orientation = vector_orientation("conv", 1, &left.shape, false)?;
    let right_orientation = vector_orientation("conv", 2, &right.shape, false)?;
    let full = convolve_vectors("conv", &left.values, &right.values, context)?;
    let (start, length, orientation) = match requested_shape {
        ConvolutionShape::Full => (
            0,
            full.len(),
            if left_orientation == VectorOrientation::Row
                && right_orientation == VectorOrientation::Row
            {
                VectorOrientation::Row
            } else {
                VectorOrientation::Column
            },
        ),
        ConvolutionShape::Same => (right.values.len() / 2, left.values.len(), left_orientation),
        ConvolutionShape::Valid => (
            right.values.len().saturating_sub(1),
            if right.values.is_empty() {
                left.values.len()
            } else {
                left.values
                    .len()
                    .checked_sub(right.values.len())
                    .map_or(0, |difference| difference.saturating_add(1))
            },
            left_orientation,
        ),
    };
    let values = crop_values("conv", &full, start, length)?;
    let shape = vector_output_shape("conv", values.len(), orientation)?;
    T::output(shape, values, context)
}

pub(super) fn conv2_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("conv2", arguments, 2, 4)?;
    expect_max_outputs("conv2", context, 1)?;
    context.check_cancelled()?;
    let separable = arguments.len() >= 3 && keyword(&arguments[2]).is_none();
    let shape_position = if separable { 3 } else { 2 };
    let requested_shape = arguments
        .get(shape_position)
        .map_or(Ok(ConvolutionShape::Full), |value| {
            convolution_shape("conv2", value)
        })?;
    if !separable && arguments.len() > 3 {
        return Err(argument_form_error(
            "conv2",
            "A, B[, shape] or hcol, hrow, A[, shape]",
        ));
    }
    let output = match precision_of(arguments) {
        Precision::Double => conv2_typed::<f64>(arguments, separable, requested_shape, context)?,
        Precision::Single => conv2_typed::<f32>(arguments, separable, requested_shape, context)?,
    };
    Ok(vec![output])
}

fn conv2_typed<T: Real + NumericOutput>(
    arguments: &[Value],
    separable: bool,
    requested_shape: ConvolutionShape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError>
where
    NumericArray<T>: NumericInput<T>,
{
    let (input, kernel) = if separable {
        let column =
            NumericArray::<T>::read("conv2", 1, &arguments[0], CONVOLUTION_NUMERIC, context)?;
        let row = NumericArray::<T>::read("conv2", 2, &arguments[1], CONVOLUTION_NUMERIC, context)?;
        vector_orientation("conv2", 1, &column.shape, false)?;
        vector_orientation("conv2", 2, &row.shape, false)?;
        let input =
            NumericArray::<T>::read("conv2", 3, &arguments[2], CONVOLUTION_NUMERIC, context)?;
        ensure_matrix("conv2", 3, &input.shape)?;
        let kernel_shape = shape2(
            "conv2 separable kernel",
            host_u64("conv2 column kernel", column.values.len())?,
            host_u64("conv2 row kernel", row.values.len())?,
        )?;
        let kernel_length = checked_product(
            "conv2 separable kernel",
            column.values.len(),
            row.values.len(),
        )?;
        let mut values = reserved_values("conv2 separable kernel", kernel_length)?;
        for right in &row.values {
            for left in &column.values {
                values.push(left.multiply(*right));
            }
        }
        (
            input,
            NumericArray {
                shape: kernel_shape,
                values,
            },
        )
    } else {
        let input =
            NumericArray::<T>::read("conv2", 1, &arguments[0], CONVOLUTION_NUMERIC, context)?;
        let kernel =
            NumericArray::<T>::read("conv2", 2, &arguments[1], CONVOLUTION_NUMERIC, context)?;
        ensure_matrix("conv2", 1, &input.shape)?;
        ensure_matrix("conv2", 2, &kernel.shape)?;
        (input, kernel)
    };

    let full = convolve_matrices("conv2", &input, &kernel, context)?;
    let input_rows = checked_host_length("conv2 input rows", input.shape.extent(0))?;
    let input_columns = checked_host_length("conv2 input columns", input.shape.extent(1))?;
    let kernel_rows = checked_host_length("conv2 kernel rows", kernel.shape.extent(0))?;
    let kernel_columns = checked_host_length("conv2 kernel columns", kernel.shape.extent(1))?;
    let full_rows = checked_host_length("conv2 full rows", full.shape.extent(0))?;
    let full_columns = checked_host_length("conv2 full columns", full.shape.extent(1))?;
    let (row_start, column_start, rows, columns) = match requested_shape {
        ConvolutionShape::Full => (0, 0, full_rows, full_columns),
        ConvolutionShape::Same => (
            kernel_rows / 2,
            kernel_columns / 2,
            input_rows,
            input_columns,
        ),
        ConvolutionShape::Valid if kernel.values.is_empty() => (0, 0, input_rows, input_columns),
        ConvolutionShape::Valid => (
            kernel_rows.saturating_sub(1),
            kernel_columns.saturating_sub(1),
            input_rows
                .checked_sub(kernel_rows)
                .map_or(0, |difference| difference.saturating_add(1)),
            input_columns
                .checked_sub(kernel_columns)
                .map_or(0, |difference| difference.saturating_add(1)),
        ),
    };
    let values = crop_matrix(
        "conv2",
        &full.values,
        full_rows,
        row_start,
        column_start,
        rows,
        columns,
        context,
    )?;
    let shape = shape2(
        "conv2 output",
        host_u64("conv2 output rows", rows)?,
        host_u64("conv2 output columns", columns)?,
    )?;
    T::output(shape, values, context)
}

pub(super) fn deconv_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("deconv", arguments, 2)?;
    expect_max_outputs("deconv", context, 2)?;
    context.check_cancelled()?;
    match precision_of(arguments) {
        Precision::Double => deconv_typed::<f64>(arguments, context),
        Precision::Single => deconv_typed::<f32>(arguments, context),
    }
}

fn deconv_typed<T: Real + NumericOutput>(
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> BuiltinResult
where
    NumericArray<T>: NumericInput<T>,
{
    let dividend =
        NumericArray::<T>::read("deconv", 1, &arguments[0], CONVOLUTION_NUMERIC, context)?;
    let divisor =
        NumericArray::<T>::read("deconv", 2, &arguments[1], CONVOLUTION_NUMERIC, context)?;
    let dividend_orientation = vector_orientation("deconv", 1, &dividend.shape, true)?;
    vector_orientation("deconv", 2, &divisor.shape, true)?;
    let Some(first_divisor) = divisor.values.first().copied() else {
        return Err(domain_error(
            "deconv",
            "requires a divisor with a nonzero leading coefficient",
        ));
    };
    if first_divisor.is_zero() {
        return Err(domain_error(
            "deconv",
            "requires a divisor with a nonzero leading coefficient",
        ));
    }
    let quotient_length = dividend
        .values
        .len()
        .checked_sub(divisor.values.len())
        .map_or(1, |difference| difference.saturating_add(1));
    let mut quotient =
        filled_host_values("deconv quotient", quotient_length, ComplexValue::zero())?;
    let mut remainder = clone_checked("deconv remainder", &dividend.values)?;
    if dividend.values.len() >= divisor.values.len() {
        for index in 0..quotient_length {
            check_cancelled_at(context, index)?;
            let coefficient = remainder[index].divide(first_divisor);
            quotient[index] = coefficient;
            for (offset, divisor_value) in divisor.values.iter().copied().enumerate() {
                let remainder_index = index + offset;
                remainder[remainder_index] =
                    remainder[remainder_index].subtract(coefficient.multiply(divisor_value));
            }
        }
    }
    let quotient_shape =
        vector_output_shape("deconv quotient", quotient.len(), dividend_orientation)?;
    let quotient = T::output(quotient_shape, quotient, context)?;
    if context.requested_outputs().max(1) == 1 {
        return Ok(vec![quotient]);
    }
    let remainder = T::output(dividend.shape, remainder, context)?;
    Ok(vec![quotient, remainder])
}

pub(super) fn poly_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("poly", arguments, 1)?;
    expect_max_outputs("poly", context, 1)?;
    context.check_cancelled()?;
    if let Value::Array(ArrayData::Integer(integer)) = &arguments[0] {
        return Ok(vec![poly_integer(integer, context)?]);
    }
    if let Value::Array(ArrayData::Logical(array)) = &arguments[0]
        && array.numel() != 0
        && !is_vector_shape(array.shape())
    {
        return Err(type_error(
            "poly",
            1,
            "floating-point square matrix or numeric vector",
            &arguments[0],
        ));
    }
    let output = match precision_of(arguments) {
        Precision::Double => poly_typed_f64(&arguments[0], context)?,
        Precision::Single => poly_typed_f32(&arguments[0], context)?,
    };
    Ok(vec![output])
}

fn poly_typed_f64(value: &Value, context: &BuiltinContext<'_>) -> Result<Value, BuiltinError> {
    let input = NumericArray::<f64>::read("poly", 1, value, FLOAT_OR_LOGICAL, context)?;
    if is_vector_shape(&input.shape) || input.shape.numel() == 0 {
        let coefficients = polynomial_from_roots("poly", &input.values, context)?;
        return f64::output(
            vector_output_shape("poly", coefficients.len(), VectorOrientation::Row)?,
            coefficients,
            context,
        );
    }
    ensure_square_matrix("poly", 1, &input.shape)?;
    let real_input = !input.is_complex();
    let eigenvalues = if real_input {
        let mut values = reserved_values("poly real matrix", input.values.len())?;
        values.extend(input.values.iter().map(|value| value.re));
        let matrix = DenseArray::from_vec(input.shape.clone(), values)
            .map_err(|error| array_error(&error))?;
        let eigenvalues = context
            .linalg_provider()
            .eig_f64(EigRequest::new(&matrix).with_cancellation_flag(context.cancellation_flag()))
            .map_err(|error| linalg_error("poly", error))?
            .eigenvalues;
        copy_complex64_values("poly eigenvalues", eigenvalues.as_slice())
    } else {
        let matrix = dense_complex64(input.shape.clone(), &input.values)?;
        let eigenvalues = context
            .linalg_provider()
            .eig_complex64(
                EigRequest::new(&matrix).with_cancellation_flag(context.cancellation_flag()),
            )
            .map_err(|error| linalg_error("poly", error))?
            .eigenvalues;
        copy_complex64_values("poly eigenvalues", eigenvalues.as_slice())
    }?;
    let mut coefficients = polynomial_from_roots("poly", &eigenvalues, context)?;
    if real_input {
        for coefficient in &mut coefficients {
            coefficient.im = 0.0;
        }
    }
    f64::output(
        vector_output_shape("poly", coefficients.len(), VectorOrientation::Row)?,
        coefficients,
        context,
    )
}

fn poly_typed_f32(value: &Value, context: &BuiltinContext<'_>) -> Result<Value, BuiltinError> {
    let input = NumericArray::<f32>::read("poly", 1, value, FLOAT_OR_LOGICAL, context)?;
    if is_vector_shape(&input.shape) || input.shape.numel() == 0 {
        let coefficients = polynomial_from_roots("poly", &input.values, context)?;
        return f32::output(
            vector_output_shape("poly", coefficients.len(), VectorOrientation::Row)?,
            coefficients,
            context,
        );
    }
    ensure_square_matrix("poly", 1, &input.shape)?;
    let real_input = !input.is_complex();
    let eigenvalues = if real_input {
        let mut values = reserved_values("poly real single matrix", input.values.len())?;
        values.extend(input.values.iter().map(|value| value.re));
        let matrix = DenseArray::from_vec(input.shape.clone(), values)
            .map_err(|error| array_error(&error))?;
        let eigenvalues = context
            .linalg_provider()
            .eig_f32(EigRequest::new(&matrix).with_cancellation_flag(context.cancellation_flag()))
            .map_err(|error| linalg_error("poly", error))?
            .eigenvalues;
        copy_complex32_values("poly eigenvalues", eigenvalues.as_slice())
    } else {
        let matrix = dense_complex32(input.shape.clone(), &input.values)?;
        let eigenvalues = context
            .linalg_provider()
            .eig_complex32(
                EigRequest::new(&matrix).with_cancellation_flag(context.cancellation_flag()),
            )
            .map_err(|error| linalg_error("poly", error))?
            .eigenvalues;
        copy_complex32_values("poly eigenvalues", eigenvalues.as_slice())
    }?;
    let mut coefficients = polynomial_from_roots("poly", &eigenvalues, context)?;
    if real_input {
        for coefficient in &mut coefficients {
            coefficient.im = 0.0;
        }
    }
    f32::output(
        vector_output_shape("poly", coefficients.len(), VectorOrientation::Row)?,
        coefficients,
        context,
    )
}

pub(super) fn polyval_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("polyval", arguments, 2)?;
    expect_max_outputs("polyval", context, 1)?;
    context.check_cancelled()?;
    let output = match precision_of(arguments) {
        Precision::Double => polyval_typed::<f64>(arguments, context)?,
        Precision::Single => polyval_typed::<f32>(arguments, context)?,
    };
    Ok(vec![output])
}

fn polyval_typed<T: Real + NumericOutput>(
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError>
where
    NumericArray<T>: NumericInput<T>,
{
    let coefficients =
        NumericArray::<T>::read("polyval", 1, &arguments[0], FLOAT_OR_LOGICAL, context)?;
    vector_orientation("polyval", 1, &coefficients.shape, true)?;
    let points = NumericArray::<T>::read("polyval", 2, &arguments[1], FLOAT_OR_LOGICAL, context)?;
    let mut output = reserved_values("polyval output", points.values.len())?;
    for (index, point) in points.values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let mut value = ComplexValue::zero();
        for coefficient in &coefficients.values {
            value = value.multiply(point).add(*coefficient);
        }
        output.push(value);
    }
    T::output(points.shape, output, context)
}

pub(super) fn polyder_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("polyder", arguments, 1, 2)?;
    expect_max_outputs("polyder", context, if arguments.len() == 2 { 2 } else { 1 })?;
    context.check_cancelled()?;
    match precision_of(arguments) {
        Precision::Double => polyder_typed::<f64>(arguments, context),
        Precision::Single => polyder_typed::<f32>(arguments, context),
    }
}

fn polyder_typed<T: Real + NumericOutput>(
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> BuiltinResult
where
    NumericArray<T>: NumericInput<T>,
{
    let first = NumericArray::<T>::read("polyder", 1, &arguments[0], FLOAT_OR_LOGICAL, context)?;
    vector_orientation("polyder", 1, &first.shape, true)?;
    if arguments.len() == 1 {
        let derivative = polynomial_derivative("polyder", &first.values)?;
        let shape = vector_output_shape("polyder", derivative.len(), VectorOrientation::Row)?;
        return Ok(vec![T::output(shape, derivative, context)?]);
    }
    let second = NumericArray::<T>::read("polyder", 2, &arguments[1], FLOAT_OR_LOGICAL, context)?;
    vector_orientation("polyder", 2, &second.shape, true)?;
    if context.requested_outputs().max(1) == 1 {
        let product = convolve_vectors("polyder", &first.values, &second.values, context)?;
        let derivative = polynomial_derivative("polyder", &product)?;
        let shape = vector_output_shape("polyder", derivative.len(), VectorOrientation::Row)?;
        return Ok(vec![T::output(shape, derivative, context)?]);
    }
    let first_derivative = polynomial_derivative("polyder", &first.values)?;
    let second_derivative = polynomial_derivative("polyder", &second.values)?;
    let left = convolve_vectors("polyder", &first_derivative, &second.values, context)?;
    let right = convolve_vectors("polyder", &first.values, &second_derivative, context)?;
    let difference = polynomial_subtract("polyder", &left, &right)?;
    let numerator = trim_leading_zeros("polyder", &difference)?;
    let denominator = convolve_vectors("polyder", &second.values, &second.values, context)?;
    let numerator_shape =
        vector_output_shape("polyder numerator", numerator.len(), VectorOrientation::Row)?;
    let denominator_shape = vector_output_shape(
        "polyder denominator",
        denominator.len(),
        VectorOrientation::Row,
    )?;
    Ok(vec![
        T::output(numerator_shape, numerator, context)?,
        T::output(denominator_shape, denominator, context)?,
    ])
}

pub(super) fn polyint_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("polyint", arguments, 1, 2)?;
    expect_max_outputs("polyint", context, 1)?;
    context.check_cancelled()?;
    let output = match precision_of(arguments) {
        Precision::Double => polyint_typed::<f64>(arguments, context)?,
        Precision::Single => polyint_typed::<f32>(arguments, context)?,
    };
    Ok(vec![output])
}

fn polyint_typed<T: Real + NumericOutput>(
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError>
where
    NumericArray<T>: NumericInput<T>,
{
    let coefficients =
        NumericArray::<T>::read("polyint", 1, &arguments[0], FLOAT_OR_LOGICAL, context)?;
    let dimensions = coefficients.shape.dimensions();
    if coefficients.shape.numel() != 0
        && (dimensions.len() != 2 || coefficients.shape.extent(0) != 1)
    {
        return Err(domain_error(
            "polyint",
            "requires the coefficient input to be a row vector",
        ));
    }
    let constant = arguments.get(1).map_or(Ok(ComplexValue::zero()), |value| {
        numeric_scalar::<T>("polyint", 2, value, FLOAT_OR_LOGICAL, context)
    })?;
    let output_length = coefficients.values.len().saturating_add(1);
    let mut output = reserved_values("polyint output", output_length)?;
    for (index, coefficient) in coefficients.values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let degree = coefficients.values.len() - index;
        output.push(coefficient.scale(T::ONE.divide(T::from_usize(degree))));
    }
    output.push(constant);
    let shape = vector_output_shape("polyint", output.len(), VectorOrientation::Row)?;
    T::output(shape, output, context)
}

pub(super) fn roots_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("roots", arguments, 1)?;
    expect_max_outputs("roots", context, 1)?;
    context.check_cancelled()?;
    let output = match precision_of(arguments) {
        Precision::Double => roots_f64(&arguments[0], context)?,
        Precision::Single => roots_f32(&arguments[0], context)?,
    };
    Ok(vec![output])
}

fn roots_f64(value: &Value, context: &BuiltinContext<'_>) -> Result<Value, BuiltinError> {
    let coefficients = NumericArray::<f64>::read("roots", 1, value, FLOAT_ONLY, context)?;
    vector_orientation("roots", 1, &coefficients.shape, true)?;
    validate_finite_coefficients("roots", &coefficients.values)?;
    let prepared = prepare_roots("roots", &coefficients.values)?;
    let mut output = filled_host_values(
        "roots explicit zeros",
        prepared.zero_roots,
        ComplexValue::zero(),
    )?;
    if !prepared.coefficients.is_empty() {
        let order = prepared.coefficients.len() - 1;
        let shape = shape2(
            "roots companion matrix",
            host_u64("roots order", order)?,
            host_u64("roots order", order)?,
        )?;
        if prepared.coefficients.iter().all(|value| value.im == 0.0) {
            let mut matrix = filled_host_values(
                "roots companion matrix",
                checked_product("roots companion matrix", order, order)?,
                0.0,
            )?;
            fill_real_companion(&mut matrix, order, &prepared.coefficients);
            let matrix =
                DenseArray::from_vec(shape, matrix).map_err(|error| array_error(&error))?;
            let eigenvalues = context
                .linalg_provider()
                .eig_f64(
                    EigRequest::new(&matrix).with_cancellation_flag(context.cancellation_flag()),
                )
                .map_err(|error| linalg_error("roots", error))?
                .eigenvalues;
            output.extend(
                eigenvalues
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(value.re, value.im)),
            );
        } else {
            let mut matrix = filled_host_values(
                "roots complex companion matrix",
                checked_product("roots companion matrix", order, order)?,
                ComplexValue::zero(),
            )?;
            fill_complex_companion(&mut matrix, order, &prepared.coefficients);
            let matrix = dense_complex64(shape, &matrix)?;
            let eigenvalues = context
                .linalg_provider()
                .eig_complex64(
                    EigRequest::new(&matrix).with_cancellation_flag(context.cancellation_flag()),
                )
                .map_err(|error| linalg_error("roots", error))?
                .eigenvalues;
            output.extend(
                eigenvalues
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(value.re, value.im)),
            );
        }
    }
    f64::output(
        vector_output_shape("roots", output.len(), VectorOrientation::Column)?,
        output,
        context,
    )
}

fn roots_f32(value: &Value, context: &BuiltinContext<'_>) -> Result<Value, BuiltinError> {
    let coefficients = NumericArray::<f32>::read("roots", 1, value, FLOAT_ONLY, context)?;
    vector_orientation("roots", 1, &coefficients.shape, true)?;
    validate_finite_coefficients("roots", &coefficients.values)?;
    let prepared = prepare_roots("roots", &coefficients.values)?;
    let mut output = filled_host_values(
        "roots explicit zeros",
        prepared.zero_roots,
        ComplexValue::zero(),
    )?;
    if !prepared.coefficients.is_empty() {
        let order = prepared.coefficients.len() - 1;
        let shape = shape2(
            "roots companion matrix",
            host_u64("roots order", order)?,
            host_u64("roots order", order)?,
        )?;
        if prepared.coefficients.iter().all(|value| value.im == 0.0) {
            let mut matrix = filled_host_values(
                "roots companion matrix",
                checked_product("roots companion matrix", order, order)?,
                0.0_f32,
            )?;
            fill_real_companion(&mut matrix, order, &prepared.coefficients);
            let matrix =
                DenseArray::from_vec(shape, matrix).map_err(|error| array_error(&error))?;
            let eigenvalues = context
                .linalg_provider()
                .eig_f32(
                    EigRequest::new(&matrix).with_cancellation_flag(context.cancellation_flag()),
                )
                .map_err(|error| linalg_error("roots", error))?
                .eigenvalues;
            output.extend(
                eigenvalues
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(value.re, value.im)),
            );
        } else {
            let mut matrix = filled_host_values(
                "roots complex companion matrix",
                checked_product("roots companion matrix", order, order)?,
                ComplexValue::zero(),
            )?;
            fill_complex_companion(&mut matrix, order, &prepared.coefficients);
            let matrix = dense_complex32(shape, &matrix)?;
            let eigenvalues = context
                .linalg_provider()
                .eig_complex32(
                    EigRequest::new(&matrix).with_cancellation_flag(context.cancellation_flag()),
                )
                .map_err(|error| linalg_error("roots", error))?
                .eigenvalues;
            output.extend(
                eigenvalues
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(value.re, value.im)),
            );
        }
    }
    f32::output(
        vector_output_shape("roots", output.len(), VectorOrientation::Column)?,
        output,
        context,
    )
}

pub(super) fn interp1_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("interp1", arguments, 2, 5)?;
    expect_max_outputs("interp1", context, 1)?;
    context.check_cancelled()?;
    let parsed = parse_interp1_arguments(arguments)?;
    let mut precision_values = vec![parsed.y, parsed.query];
    if let Some(x) = parsed.x {
        precision_values.push(x);
    }
    if let Some(extrapolation) = parsed.extrapolation {
        precision_values.push(extrapolation);
    }
    let output = match precision_of(&precision_values) {
        Precision::Double => interp1_typed::<f64>(parsed, context)?,
        Precision::Single => interp1_typed::<f32>(parsed, context)?,
    };
    Ok(vec![output])
}

#[derive(Clone, Copy)]
struct ParsedInterp1<'a> {
    x: Option<&'a Value>,
    y: &'a Value,
    query: &'a Value,
    method: InterpolationMethod,
    extrapolation: Option<&'a Value>,
}

fn parse_interp1_arguments(arguments: &[Value]) -> Result<ParsedInterp1<'_>, BuiltinError> {
    let third_is_method = arguments.get(2).and_then(keyword).is_some();
    let (x, y, query, method_value, extrapolation) = match arguments.len() {
        2 => (None, &arguments[0], &arguments[1], None, None),
        3 if third_is_method => (
            None,
            &arguments[0],
            &arguments[1],
            Some(&arguments[2]),
            None,
        ),
        3 => (
            Some(&arguments[0]),
            &arguments[1],
            &arguments[2],
            None,
            None,
        ),
        4 if third_is_method => (
            None,
            &arguments[0],
            &arguments[1],
            Some(&arguments[2]),
            Some(&arguments[3]),
        ),
        4 => (
            Some(&arguments[0]),
            &arguments[1],
            &arguments[2],
            Some(&arguments[3]),
            None,
        ),
        5 => (
            Some(&arguments[0]),
            &arguments[1],
            &arguments[2],
            Some(&arguments[3]),
            Some(&arguments[4]),
        ),
        _ => unreachable!("argument count was checked"),
    };
    let method = method_value.map_or(Ok(InterpolationMethod::Linear), interp1_method)?;
    Ok(ParsedInterp1 {
        x,
        y,
        query,
        method,
        extrapolation,
    })
}

fn interp1_method(value: &Value) -> Result<InterpolationMethod, BuiltinError> {
    match keyword(value).as_deref() {
        Some("linear") => Ok(InterpolationMethod::Linear),
        Some("nearest") => Ok(InterpolationMethod::Nearest),
        Some("pchip" | "spline" | "cubic" | "makima") => Err(domain_error(
            "interp1",
            "does not implement pchip, spline, cubic, or makima interpolation",
        )),
        _ => Err(domain_error(
            "interp1",
            "requires interpolation method 'linear' or 'nearest'",
        )),
    }
}

fn interp1_typed<T: Real + NumericOutput>(
    parsed: ParsedInterp1<'_>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError>
where
    NumericArray<T>: NumericInput<T>,
{
    let y = NumericArray::<T>::read("interp1", 2, parsed.y, FLOAT_ONLY, context)?;
    ensure_matrix("interp1", 2, &y.shape)?;
    let (samples, channels, vector_y) = interpolation_y_layout(&y.shape)?;
    if samples < 2 {
        return Err(domain_error(
            "interp1",
            "requires at least two sample points",
        ));
    }
    let x = if let Some(value) = parsed.x {
        let x = NumericArray::<T>::read("interp1", 1, value, FLOAT_ONLY, context)?;
        vector_orientation("interp1", 1, &x.shape, false)?;
        if x.values.len() != samples || x.is_complex() {
            return Err(domain_error(
                "interp1",
                "requires X to be a real vector with one point per row of Y",
            ));
        }
        let mut values = reserved_values("interp1 X", samples)?;
        values.extend(x.values.iter().map(|value| value.re));
        values
    } else {
        let mut values = reserved_values("interp1 implicit X", samples)?;
        values.extend((0..samples).map(|index| T::from_usize(index.saturating_add(1))));
        values
    };
    if x.iter().any(|value| !value.is_finite()) {
        return Err(domain_error("interp1", "requires finite X sample points"));
    }
    let query = NumericArray::<T>::read("interp1", 3, parsed.query, FLOAT_ONLY, context)?;
    if query.is_complex() {
        return Err(domain_error("interp1", "requires real query points"));
    }
    let extrapolation = parse_extrapolation::<T>(parsed.extrapolation, context)?;
    let mut order = reserved_values("interp1 sort order", samples)?;
    order.extend(0..samples);
    order.sort_by(|left, right| x[*left].partial_cmp(&x[*right]).unwrap_or(Ordering::Equal));
    let mut sorted_x = reserved_values("interp1 sorted X", samples)?;
    for index in &order {
        sorted_x.push(x[*index]);
    }
    if sorted_x.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(domain_error("interp1", "requires unique X sample points"));
    }
    let output_shape = interpolation_output_shape(&query.shape, channels, vector_y)?;
    let output_length = checked_product("interp1 output", query.values.len(), channels)?;
    let mut output = reserved_values("interp1 output", output_length)?;
    for channel in 0..channels {
        let base = channel * samples;
        let mut sorted_y = reserved_values("interp1 sorted Y", samples)?;
        for index in &order {
            sorted_y.push(y.values[base + *index]);
        }
        for (query_index, query_value) in query.values.iter().enumerate() {
            check_cancelled_at(context, channel * query.values.len() + query_index)?;
            output.push(interpolate_one(
                &sorted_x,
                &sorted_y,
                query_value.re,
                parsed.method,
                extrapolation,
            ));
        }
    }
    T::output(output_shape, output, context)
}

fn parse_extrapolation<T: Real>(
    value: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> Result<Extrapolation<T>, BuiltinError>
where
    NumericArray<T>: NumericInput<T>,
{
    let Some(value) = value else {
        return Ok(Extrapolation::Missing);
    };
    if let Some(option) = keyword(value) {
        return if option == "extrap" {
            Ok(Extrapolation::Method)
        } else {
            Err(domain_error(
                "interp1",
                "requires extrapolation option 'extrap' or a numeric scalar",
            ))
        };
    }
    numeric_scalar::<T>("interp1", 5, value, CONVOLUTION_NUMERIC, context).map(Extrapolation::Value)
}

fn interpolate_one<T: Real>(
    x: &[T],
    y: &[ComplexValue<T>],
    query: T,
    method: InterpolationMethod,
    extrapolation: Extrapolation<T>,
) -> ComplexValue<T> {
    if query.is_nan() {
        return ComplexValue::nan();
    }
    if query < x[0] {
        return extrapolated_value(x, y, query, method, extrapolation, 0, 1);
    }
    let last = x.len() - 1;
    if query > x[last] {
        return extrapolated_value(x, y, query, method, extrapolation, last - 1, last);
    }
    match x.binary_search_by(|candidate| candidate.partial_cmp(&query).unwrap_or(Ordering::Less)) {
        Ok(index) => y[index],
        Err(upper) => interval_value(x, y, query, method, upper - 1, upper),
    }
}

fn extrapolated_value<T: Real>(
    x: &[T],
    y: &[ComplexValue<T>],
    query: T,
    method: InterpolationMethod,
    extrapolation: Extrapolation<T>,
    left: usize,
    right: usize,
) -> ComplexValue<T> {
    match extrapolation {
        Extrapolation::Missing => ComplexValue::nan(),
        Extrapolation::Value(value) => value,
        Extrapolation::Method if method == InterpolationMethod::Nearest => {
            if query < x[0] {
                y[0]
            } else {
                y[y.len() - 1]
            }
        }
        Extrapolation::Method => interval_value(x, y, query, method, left, right),
    }
}

fn interval_value<T: Real>(
    x: &[T],
    y: &[ComplexValue<T>],
    query: T,
    method: InterpolationMethod,
    left: usize,
    right: usize,
) -> ComplexValue<T> {
    if method == InterpolationMethod::Nearest {
        let left_distance = query.subtract(x[left]);
        let right_distance = x[right].subtract(query);
        return if left_distance < right_distance {
            y[left]
        } else {
            y[right]
        };
    }
    let fraction = query.subtract(x[left]).divide(x[right].subtract(x[left]));
    y[left].add(y[right].subtract(y[left]).scale(fraction))
}

fn interpolation_y_layout(shape: &Shape) -> Result<(usize, usize, bool), BuiltinError> {
    let rows = checked_host_length("interp1 Y rows", shape.extent(0))?;
    let columns = checked_host_length("interp1 Y columns", shape.extent(1))?;
    if rows == 1 || columns == 1 {
        return Ok((
            checked_host_length("interp1 Y length", shape.numel())?,
            1,
            true,
        ));
    }
    Ok((rows, columns, false))
}

fn interpolation_output_shape(
    query_shape: &Shape,
    channels: usize,
    vector_y: bool,
) -> Result<Shape, BuiltinError> {
    if vector_y || channels == 1 {
        return Ok(query_shape.clone());
    }
    if is_vector_shape(query_shape) {
        return shape2(
            "interp1 output",
            query_shape.numel(),
            host_u64("interp1 channels", channels)?,
        );
    }
    let mut dimensions = query_shape.dimensions().to_vec();
    dimensions.push(host_u64("interp1 channels", channels)?);
    Shape::new(dimensions).map_err(|error| array_error(&error))
}

struct PreparedRoots<T: Real> {
    coefficients: Vec<ComplexValue<T>>,
    zero_roots: usize,
}

fn prepare_roots<T: Real>(
    name: &str,
    coefficients: &[ComplexValue<T>],
) -> Result<PreparedRoots<T>, BuiltinError> {
    let first_nonzero = coefficients
        .iter()
        .position(|value| !value.is_zero())
        .unwrap_or(coefficients.len());
    let significant = &coefficients[first_nonzero..];
    if significant.len() <= 1 {
        return Ok(PreparedRoots {
            coefficients: Vec::new(),
            zero_roots: 0,
        });
    }
    let zero_roots = significant
        .iter()
        .rev()
        .take_while(|value| value.is_zero())
        .count();
    let nonzero_length = significant.len() - zero_roots;
    if nonzero_length <= 1 {
        return Ok(PreparedRoots {
            coefficients: Vec::new(),
            zero_roots,
        });
    }
    let coefficients = clone_checked(name, &significant[..nonzero_length])?;
    Ok(PreparedRoots {
        coefficients,
        zero_roots,
    })
}

fn validate_finite_coefficients<T: Real>(
    name: &str,
    coefficients: &[ComplexValue<T>],
) -> Result<(), BuiltinError> {
    if coefficients.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(domain_error(name, "requires finite coefficients"))
    }
}

fn fill_real_companion<T: Real>(matrix: &mut [T], order: usize, coefficients: &[ComplexValue<T>]) {
    let leading = coefficients[0].re;
    for column in 0..order {
        matrix[column * order] = coefficients[column + 1].re.divide(leading).negate();
    }
    for row in 1..order {
        matrix[(row - 1) * order + row] = T::ONE;
    }
}

fn fill_complex_companion<T: Real>(
    matrix: &mut [ComplexValue<T>],
    order: usize,
    coefficients: &[ComplexValue<T>],
) {
    let leading = coefficients[0];
    for column in 0..order {
        matrix[column * order] = coefficients[column + 1].divide(leading).negate();
    }
    for row in 1..order {
        matrix[(row - 1) * order + row] = ComplexValue::one();
    }
}

fn polynomial_from_roots<T: Real>(
    name: &str,
    roots: &[ComplexValue<T>],
    context: &BuiltinContext<'_>,
) -> Result<Vec<ComplexValue<T>>, BuiltinError> {
    let mut coefficients = vec![ComplexValue::one()];
    for (root_index, root) in roots.iter().copied().enumerate() {
        check_cancelled_at(context, root_index)?;
        let next_length = coefficients.len().checked_add(1).ok_or_else(|| {
            domain_error(name, "cannot represent the polynomial coefficient count")
        })?;
        let mut next = filled_host_values(name, next_length, ComplexValue::zero())?;
        for (index, coefficient) in coefficients.iter().copied().enumerate() {
            next[index] = next[index].add(coefficient);
            next[index + 1] = next[index + 1].subtract(coefficient.multiply(root));
        }
        coefficients = next;
    }
    Ok(coefficients)
}

fn polynomial_derivative<T: Real>(
    name: &str,
    coefficients: &[ComplexValue<T>],
) -> Result<Vec<ComplexValue<T>>, BuiltinError> {
    if coefficients.len() <= 1 {
        return Ok(vec![ComplexValue::zero()]);
    }
    let mut output = reserved_values(name, coefficients.len() - 1)?;
    output.extend(
        coefficients[..coefficients.len() - 1]
            .iter()
            .copied()
            .enumerate()
            .map(|(index, coefficient)| {
                coefficient.scale(T::from_usize(coefficients.len() - index - 1))
            }),
    );
    Ok(output)
}

fn polynomial_subtract<T: Real>(
    name: &str,
    left: &[ComplexValue<T>],
    right: &[ComplexValue<T>],
) -> Result<Vec<ComplexValue<T>>, BuiltinError> {
    let length = left.len().max(right.len());
    let mut output = filled_host_values(name, length, ComplexValue::zero())?;
    let left_offset = length - left.len();
    let right_offset = length - right.len();
    for (index, value) in left.iter().copied().enumerate() {
        output[left_offset + index] = output[left_offset + index].add(value);
    }
    for (index, value) in right.iter().copied().enumerate() {
        output[right_offset + index] = output[right_offset + index].subtract(value);
    }
    Ok(output)
}

fn trim_leading_zeros<T: Real>(
    name: &str,
    values: &[ComplexValue<T>],
) -> Result<Vec<ComplexValue<T>>, BuiltinError> {
    let first = values
        .iter()
        .position(|value| !value.is_zero())
        .unwrap_or_else(|| values.len().saturating_sub(1));
    clone_checked(name, &values[first..])
}

fn convolve_vectors<T: Real>(
    name: &str,
    left: &[ComplexValue<T>],
    right: &[ComplexValue<T>],
    context: &BuiltinContext<'_>,
) -> Result<Vec<ComplexValue<T>>, BuiltinError> {
    let length = convolution_extent(name, left.len(), right.len())?;
    let mut output = filled_host_values(name, length, ComplexValue::zero())?;
    for (left_index, left_value) in left.iter().copied().enumerate() {
        check_cancelled_at(context, left_index)?;
        for (right_index, right_value) in right.iter().copied().enumerate() {
            let output_index = left_index + right_index;
            output[output_index] = output[output_index].add(left_value.multiply(right_value));
        }
    }
    Ok(output)
}

fn convolve_matrices<T: Real>(
    name: &str,
    input: &NumericArray<T>,
    kernel: &NumericArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<NumericArray<T>, BuiltinError> {
    let input_rows = checked_host_length(name, input.shape.extent(0))?;
    let input_columns = checked_host_length(name, input.shape.extent(1))?;
    let kernel_rows = checked_host_length(name, kernel.shape.extent(0))?;
    let kernel_columns = checked_host_length(name, kernel.shape.extent(1))?;
    let output_rows = convolution_extent(name, input_rows, kernel_rows)?;
    let output_columns = convolution_extent(name, input_columns, kernel_columns)?;
    let output_length = checked_product(name, output_rows, output_columns)?;
    let mut values = filled_host_values(name, output_length, ComplexValue::zero())?;
    for input_column in 0..input_columns {
        for kernel_column in 0..kernel_columns {
            let output_column = input_column + kernel_column;
            for input_row in 0..input_rows {
                check_cancelled_at(context, input_column * input_rows + input_row)?;
                let input_value = input.values[input_column * input_rows + input_row];
                for kernel_row in 0..kernel_rows {
                    let output_row = input_row + kernel_row;
                    let output_index = output_column * output_rows + output_row;
                    let kernel_value = kernel.values[kernel_column * kernel_rows + kernel_row];
                    values[output_index] =
                        values[output_index].add(input_value.multiply(kernel_value));
                }
            }
        }
    }
    Ok(NumericArray {
        shape: shape2(
            name,
            host_u64(name, output_rows)?,
            host_u64(name, output_columns)?,
        )?,
        values,
    })
}

#[allow(clippy::too_many_arguments)]
fn crop_matrix<T: Real>(
    name: &str,
    full: &[ComplexValue<T>],
    full_rows: usize,
    row_start: usize,
    column_start: usize,
    rows: usize,
    columns: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<ComplexValue<T>>, BuiltinError> {
    let length = checked_product(name, rows, columns)?;
    let mut output = reserved_values(name, length)?;
    for column in 0..columns {
        for row in 0..rows {
            check_cancelled_at(context, column * rows + row)?;
            let source_row = row_start + row;
            let source_column = column_start + column;
            let source_index = source_column
                .checked_mul(full_rows)
                .and_then(|base| base.checked_add(source_row));
            output.push(
                source_index
                    .and_then(|index| full.get(index).copied())
                    .unwrap_or_else(ComplexValue::zero),
            );
        }
    }
    Ok(output)
}

fn crop_values<T: Real>(
    name: &str,
    full: &[ComplexValue<T>],
    start: usize,
    length: usize,
) -> Result<Vec<ComplexValue<T>>, BuiltinError> {
    let mut output = filled_host_values(name, length, ComplexValue::zero())?;
    for (offset, output_value) in output.iter_mut().enumerate() {
        if let Some(value) = full.get(start.saturating_add(offset)) {
            *output_value = *value;
        }
    }
    Ok(output)
}

fn convolution_extent(name: &str, left: usize, right: usize) -> Result<usize, BuiltinError> {
    if left == 0 || right == 0 {
        return Ok(left.max(right));
    }
    left.checked_add(right)
        .and_then(|sum| sum.checked_sub(1))
        .ok_or_else(|| domain_error(name, "cannot represent the convolution result size"))
}

fn convolution_shape(name: &str, value: &Value) -> Result<ConvolutionShape, BuiltinError> {
    match keyword(value).as_deref() {
        Some("full") => Ok(ConvolutionShape::Full),
        Some("same") => Ok(ConvolutionShape::Same),
        Some("valid") => Ok(ConvolutionShape::Valid),
        _ => Err(domain_error(
            name,
            "requires shape option 'full', 'same', or 'valid'",
        )),
    }
}

trait NumericInput<T: Real> {
    fn read(
        name: &str,
        position: usize,
        value: &Value,
        policy: NumericPolicy,
        context: &BuiltinContext<'_>,
    ) -> Result<NumericArray<T>, BuiltinError>;
}

impl NumericInput<f64> for NumericArray<f64> {
    #[allow(clippy::cast_precision_loss)]
    fn read(
        name: &str,
        position: usize,
        value: &Value,
        policy: NumericPolicy,
        context: &BuiltinContext<'_>,
    ) -> Result<Self, BuiltinError> {
        let scalar_shape = || shape2(name, 1, 1);
        match value {
            Value::Double(value) => Ok(Self {
                shape: scalar_shape()?,
                values: vec![ComplexValue::new(*value, 0.0)],
            }),
            Value::Complex(value) => Ok(Self {
                shape: scalar_shape()?,
                values: vec![ComplexValue::new(value.real, value.imaginary)],
            }),
            Value::Logical(value) if policy.logical => Ok(Self {
                shape: scalar_shape()?,
                values: vec![ComplexValue::new(f64::from(*value), 0.0)],
            }),
            Value::Array(ArrayData::F32(array)) => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(f64::from(*value), 0.0)),
                context,
            ),
            Value::Array(ArrayData::ComplexF32(array)) => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(f64::from(value.re), f64::from(value.im))),
                context,
            ),
            Value::Array(ArrayData::F64(array)) => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(*value, 0.0)),
                context,
            ),
            Value::Array(ArrayData::ComplexF64(array)) => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(value.re, value.im)),
                context,
            ),
            Value::Array(ArrayData::Logical(array)) if policy.logical => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(f64::from(value.get()), 0.0)),
                context,
            ),
            Value::Array(ArrayData::Integer(integer))
                if policy.integer && !integer.is_complex() =>
            {
                copy_numeric(
                    name,
                    integer.shape().clone(),
                    integer.elements().map(|value| {
                        let real = match value.real_component() {
                            openmat_array::IntegerComponent::Signed(value) => value as f64,
                            openmat_array::IntegerComponent::Unsigned(value) => value as f64,
                        };
                        ComplexValue::new(real, 0.0)
                    }),
                    context,
                )
            }
            _ => Err(type_error(
                name,
                position,
                numeric_expectation(policy),
                value,
            )),
        }
    }
}

impl NumericInput<f32> for NumericArray<f32> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn read(
        name: &str,
        position: usize,
        value: &Value,
        policy: NumericPolicy,
        context: &BuiltinContext<'_>,
    ) -> Result<Self, BuiltinError> {
        let scalar_shape = || shape2(name, 1, 1);
        match value {
            Value::Double(value) => Ok(Self {
                shape: scalar_shape()?,
                values: vec![ComplexValue::new(*value as f32, 0.0)],
            }),
            Value::Complex(value) => Ok(Self {
                shape: scalar_shape()?,
                values: vec![ComplexValue::new(value.real as f32, value.imaginary as f32)],
            }),
            Value::Logical(value) if policy.logical => Ok(Self {
                shape: scalar_shape()?,
                values: vec![ComplexValue::new(f32::from(*value), 0.0)],
            }),
            Value::Array(ArrayData::F32(array)) => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(*value, 0.0)),
                context,
            ),
            Value::Array(ArrayData::ComplexF32(array)) => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(value.re, value.im)),
                context,
            ),
            Value::Array(ArrayData::F64(array)) => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(*value as f32, 0.0)),
                context,
            ),
            Value::Array(ArrayData::ComplexF64(array)) => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(value.re as f32, value.im as f32)),
                context,
            ),
            Value::Array(ArrayData::Logical(array)) if policy.logical => copy_numeric(
                name,
                array.shape().clone(),
                array
                    .as_slice()
                    .iter()
                    .map(|value| ComplexValue::new(f32::from(value.get()), 0.0)),
                context,
            ),
            Value::Array(ArrayData::Integer(integer))
                if policy.integer && !integer.is_complex() =>
            {
                copy_numeric(
                    name,
                    integer.shape().clone(),
                    integer.elements().map(|value| {
                        let real = match value.real_component() {
                            openmat_array::IntegerComponent::Signed(value) => value as f32,
                            openmat_array::IntegerComponent::Unsigned(value) => value as f32,
                        };
                        ComplexValue::new(real, 0.0)
                    }),
                    context,
                )
            }
            _ => Err(type_error(
                name,
                position,
                numeric_expectation(policy),
                value,
            )),
        }
    }
}

fn copy_numeric<T: Real, I: Iterator<Item = ComplexValue<T>>>(
    name: &str,
    shape: Shape,
    values: I,
    context: &BuiltinContext<'_>,
) -> Result<NumericArray<T>, BuiltinError> {
    let length = checked_host_length(name, shape.numel())?;
    let mut output = reserved_values(name, length)?;
    for (index, value) in values.enumerate() {
        check_cancelled_at(context, index)?;
        output.push(value);
    }
    Ok(NumericArray {
        shape,
        values: output,
    })
}

fn numeric_scalar<T: Real>(
    name: &str,
    position: usize,
    value: &Value,
    policy: NumericPolicy,
    context: &BuiltinContext<'_>,
) -> Result<ComplexValue<T>, BuiltinError>
where
    NumericArray<T>: NumericInput<T>,
{
    let input = NumericArray::<T>::read(name, position, value, policy, context)?;
    if input.values.len() == 1 {
        Ok(input.values[0])
    } else {
        Err(type_error(name, position, "numeric scalar", value))
    }
}

trait NumericOutput: Real {
    fn output(
        shape: Shape,
        values: Vec<ComplexValue<Self>>,
        context: &BuiltinContext<'_>,
    ) -> Result<Value, BuiltinError>;
}

impl NumericOutput for f64 {
    fn output(
        shape: Shape,
        values: Vec<ComplexValue<Self>>,
        context: &BuiltinContext<'_>,
    ) -> Result<Value, BuiltinError> {
        wrap_complex64(dense_complex64(shape, &values)?, context)
    }
}

impl NumericOutput for f32 {
    fn output(
        shape: Shape,
        values: Vec<ComplexValue<Self>>,
        context: &BuiltinContext<'_>,
    ) -> Result<Value, BuiltinError> {
        wrap_complex32(dense_complex32(shape, &values)?, context)
    }
}

fn dense_complex64(
    shape: Shape,
    values: &[ComplexValue<f64>],
) -> Result<DenseArray<ArrayComplex64>, BuiltinError> {
    let mut output = reserved_values("complex double output", values.len())?;
    output.extend(
        values
            .iter()
            .map(|value| ArrayComplex64::new(value.re, value.im)),
    );
    DenseArray::from_vec(shape, output).map_err(|error| array_error(&error))
}

fn dense_complex32(
    shape: Shape,
    values: &[ComplexValue<f32>],
) -> Result<DenseArray<ArrayComplex32>, BuiltinError> {
    let mut output = reserved_values("complex single output", values.len())?;
    output.extend(
        values
            .iter()
            .map(|value| ArrayComplex32::new(value.re, value.im)),
    );
    DenseArray::from_vec(shape, output).map_err(|error| array_error(&error))
}

fn copy_complex64_values(
    name: &str,
    values: &[ArrayComplex64],
) -> Result<Vec<ComplexValue<f64>>, BuiltinError> {
    let mut output = reserved_values(name, values.len())?;
    output.extend(
        values
            .iter()
            .map(|value| ComplexValue::new(value.re, value.im)),
    );
    Ok(output)
}

fn copy_complex32_values(
    name: &str,
    values: &[ArrayComplex32],
) -> Result<Vec<ComplexValue<f32>>, BuiltinError> {
    let mut output = reserved_values(name, values.len())?;
    output.extend(
        values
            .iter()
            .map(|value| ComplexValue::new(value.re, value.im)),
    );
    Ok(output)
}

fn precision_of(arguments: &[impl std::borrow::Borrow<Value>]) -> Precision {
    if arguments.iter().any(|value| {
        matches!(
            value.borrow(),
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        )
    }) {
        Precision::Single
    } else {
        Precision::Double
    }
}

fn numeric_expectation(policy: NumericPolicy) -> &'static str {
    match (policy.logical, policy.integer) {
        (false, false) => "single or double floating-point numeric array",
        (true, false) => "single, double, or logical numeric array",
        (true, true) => "real integer, logical, single, or double numeric array",
        (false, true) => "real integer, single, or double numeric array",
    }
}

fn vector_orientation(
    name: &str,
    position: usize,
    shape: &Shape,
    allow_zero_by_zero: bool,
) -> Result<VectorOrientation, BuiltinError> {
    if shape.ndims() != 2 {
        return Err(domain_error(name, "requires two-dimensional vectors"));
    }
    let rows = shape.extent(0);
    let columns = shape.extent(1);
    if rows == 1 {
        Ok(VectorOrientation::Row)
    } else if columns == 1 {
        Ok(VectorOrientation::Column)
    } else if allow_zero_by_zero && rows == 0 && columns == 0 {
        Ok(VectorOrientation::Row)
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a vector"),
        ))
    }
}

fn is_vector_shape(shape: &Shape) -> bool {
    shape.ndims() == 2 && (shape.extent(0) == 1 || shape.extent(1) == 1)
}

fn ensure_matrix(name: &str, position: usize, shape: &Shape) -> Result<(), BuiltinError> {
    if shape.ndims() == 2 {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a two-dimensional matrix"),
        ))
    }
}

fn ensure_square_matrix(name: &str, position: usize, shape: &Shape) -> Result<(), BuiltinError> {
    ensure_matrix(name, position, shape)?;
    if shape.extent(0) == shape.extent(1) {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a vector or square matrix"),
        ))
    }
}

fn vector_output_shape(
    name: &str,
    length: usize,
    orientation: VectorOrientation,
) -> Result<Shape, BuiltinError> {
    let length = host_u64(name, length)?;
    match orientation {
        VectorOrientation::Row => shape2(name, 1, length),
        VectorOrientation::Column => shape2(name, length, 1),
    }
}

fn shape2(name: &str, rows: u64, columns: u64) -> Result<Shape, BuiltinError> {
    Shape::new([rows, columns]).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot construct output shape: {error}"),
        )
    })
}

fn checked_product(name: &str, left: usize, right: usize) -> Result<usize, BuiltinError> {
    left.checked_mul(right)
        .ok_or_else(|| domain_error(name, "cannot represent the requested array size"))
}

fn host_u64(name: &str, value: usize) -> Result<u64, BuiltinError> {
    u64::try_from(value).map_err(|_| domain_error(name, "cannot represent a host array length"))
}

fn filled_host_values<T: Clone>(
    name: &str,
    length: usize,
    value: T,
) -> Result<Vec<T>, BuiltinError> {
    filled_values(name, host_u64(name, length)?, value)
}

fn clone_checked<T: Clone>(name: &str, values: &[T]) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_values(name, values.len())?;
    output.extend_from_slice(values);
    Ok(output)
}

fn domain_error(name: &str, detail: &str) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Domain, format!("`{name}` {detail}"))
}

fn argument_form_error(name: &str, expected: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::ArgumentCount,
        format!("built-in `{name}` expects {expected}"),
    )
}

trait SaturatingInteger: IntegerElement + Copy + Default {
    fn zero() -> Self;
    fn one() -> Self;
    fn add_saturating(self, right: Self) -> Self;
    fn multiply_saturating(self, right: Self) -> Self;
    fn negate_saturating(self) -> Self;
}

macro_rules! impl_signed_saturating_integer {
    ($($type:ty),+ $(,)?) => {
        $(
            impl SaturatingInteger for $type {
                fn zero() -> Self { 0 }
                fn one() -> Self { 1 }
                fn add_saturating(self, right: Self) -> Self { self.saturating_add(right) }
                fn multiply_saturating(self, right: Self) -> Self { self.saturating_mul(right) }
                fn negate_saturating(self) -> Self { self.saturating_neg() }
            }
        )+
    };
}

macro_rules! impl_unsigned_saturating_integer {
    ($($type:ty),+ $(,)?) => {
        $(
            impl SaturatingInteger for $type {
                fn zero() -> Self { 0 }
                fn one() -> Self { 1 }
                fn add_saturating(self, right: Self) -> Self { self.saturating_add(right) }
                fn multiply_saturating(self, right: Self) -> Self { self.saturating_mul(right) }
                fn negate_saturating(self) -> Self { 0 }
            }
        )+
    };
}

impl_signed_saturating_integer!(i8, i16, i32, i64);
impl_unsigned_saturating_integer!(u8, u16, u32, u64);

fn poly_integer(
    integer: &IntegerArrayData,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! dispatch {
        ($array:expr) => {{
            vector_orientation("poly", 1, $array.shape(), true)?;
            poly_integer_typed($array, context)
        }};
    }
    match integer {
        IntegerArrayData::I8(array) => dispatch!(array),
        IntegerArrayData::U8(array) => dispatch!(array),
        IntegerArrayData::I16(array) => dispatch!(array),
        IntegerArrayData::U16(array) => dispatch!(array),
        IntegerArrayData::I32(array) => dispatch!(array),
        IntegerArrayData::U32(array) => dispatch!(array),
        IntegerArrayData::I64(array) => dispatch!(array),
        IntegerArrayData::U64(array) => dispatch!(array),
        _ => Err(domain_error(
            "poly",
            "does not accept complex integer roots",
        )),
    }
}

fn poly_integer_typed<T: SaturatingInteger>(
    roots: &DenseArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let mut coefficients = vec![T::one()];
    for (root_index, root) in roots.as_slice().iter().copied().enumerate() {
        check_cancelled_at(context, root_index)?;
        let next_length = coefficients
            .len()
            .checked_add(1)
            .ok_or_else(|| domain_error("poly", "cannot represent the coefficient count"))?;
        let mut next = filled_host_values("poly integer", next_length, T::zero())?;
        let negative_root = root.negate_saturating();
        for (index, coefficient) in coefficients.iter().copied().enumerate() {
            next[index] = next[index].add_saturating(coefficient);
            next[index + 1] =
                next[index + 1].add_saturating(coefficient.multiply_saturating(negative_root));
        }
        coefficients = next;
    }
    let shape = vector_output_shape("poly", coefficients.len(), VectorOrientation::Row)?;
    let array = DenseArray::from_vec(shape, coefficients).map_err(|error| array_error(&error))?;
    Ok(Value::Array(ArrayData::Integer(
        IntegerArrayData::from_typed(array),
    )))
}

#[cfg(test)]
mod tests {
    use openmat_array::{ArrayData, Complex64, DenseArray, IntegerArrayData, Logical, Shape};
    use openmat_linalg::ReferenceProvider;
    use openmat_runtime::{BuiltinInvocationError, CancellationToken, NullOutput};

    use super::*;

    fn array(dimensions: [u64; 2], values: Vec<f64>) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    fn complex_array(dimensions: [u64; 2], values: Vec<Complex64>) -> Value {
        Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    fn single_array(dimensions: [u64; 2], values: Vec<f32>) -> Value {
        Value::Array(ArrayData::F32(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    fn logical_array(dimensions: [u64; 2], values: Vec<bool>) -> Value {
        Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                Shape::new(dimensions).unwrap(),
                values.into_iter().map(Logical::from).collect(),
            )
            .unwrap(),
        ))
    }

    fn i16_array(dimensions: [u64; 2], values: Vec<i16>) -> Value {
        Value::Array(ArrayData::Integer(IntegerArrayData::I16(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        )))
    }

    fn invoke(name: &str, arguments: &[Value], outputs: usize) -> Result<Vec<Value>, BuiltinError> {
        let mut registry = BuiltinRegistry::new();
        register_polynomial_extended(&mut registry).unwrap();
        let handle = registry
            .handle_by_name(name)
            .expect("registered focused built-in");
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let provider = ReferenceProvider;
        let mut context =
            BuiltinContext::with_linalg_provider(outputs, &cancellation, &mut output, &provider);
        registry
            .invoke(handle, arguments, &mut context)
            .map_err(|error| match error {
                BuiltinInvocationError::Failed { error, .. } => error,
                BuiltinInvocationError::UnknownHandle(_) => panic!("focused handle must resolve"),
            })
    }

    fn f64_data(value: &Value) -> (&[f64], &[u64]) {
        let Value::Array(ArrayData::F64(array)) = value else {
            panic!("expected double array, received {value:?}");
        };
        (array.as_slice(), array.shape().dimensions())
    }

    #[test]
    fn focused_registration_exposes_all_nine_entries() {
        let mut registry = BuiltinRegistry::new();
        register_polynomial_extended(&mut registry).unwrap();
        for name in [
            "conv", "conv2", "deconv", "poly", "polyval", "polyder", "polyint", "roots", "interp1",
        ] {
            assert!(registry.handle_by_name(name).is_some(), "missing {name}");
        }
    }

    #[test]
    fn conv_preserves_matlab_orientation_shapes_and_precision() {
        let row = array([1, 2], vec![1.0, 2.0]);
        let column = array([2, 1], vec![3.0, 4.0]);
        let output = invoke("conv", &[row.clone(), column], 1).unwrap();
        let (values, shape) = f64_data(&output[0]);
        assert_eq!(shape, [3, 1]);
        assert_eq!(values, [3.0, 10.0, 8.0]);

        let same = invoke("conv", &[row.clone(), row.clone(), Value::from("same")], 1).unwrap();
        let (values, shape) = f64_data(&same[0]);
        assert_eq!(shape, [1, 2]);
        assert_eq!(values, [4.0, 4.0]);

        let single = invoke("conv", &[single_array([1, 2], vec![1.0, 2.0]), row], 1).unwrap();
        let Value::Array(ArrayData::F32(single)) = &single[0] else {
            panic!("single input must dominate conv output precision");
        };
        assert_eq!(single.as_slice(), [1.0, 4.0, 4.0]);

        let empty_kernel = invoke(
            "conv",
            &[
                array([1, 2], vec![1.0, 2.0]),
                array([1, 0], Vec::new()),
                Value::from("valid"),
            ],
            1,
        )
        .unwrap();
        let (values, shape) = f64_data(&empty_kernel[0]);
        assert_eq!(shape, [1, 2]);
        assert_eq!(values, [0.0, 0.0]);
    }

    #[test]
    fn conv2_and_deconv_follow_column_major_and_two_output_contracts() {
        let matrix = array([2, 2], vec![1.0, 3.0, 2.0, 4.0]);
        let kernel = array([2, 2], vec![1.0, 3.0, 2.0, 4.0]);
        let output = invoke("conv2", &[matrix, kernel], 1).unwrap();
        let (values, shape) = f64_data(&output[0]);
        assert_eq!(shape, [3, 3]);
        assert_eq!(values, [1.0, 6.0, 9.0, 4.0, 20.0, 24.0, 4.0, 16.0, 16.0]);

        let outputs = invoke(
            "deconv",
            &[
                array([1, 3], vec![1.0, 3.0, 2.0]),
                array([1, 2], vec![1.0, 1.0]),
            ],
            2,
        )
        .unwrap();
        let (quotient, quotient_shape) = f64_data(&outputs[0]);
        let (remainder, remainder_shape) = f64_data(&outputs[1]);
        assert_eq!(quotient_shape, [1, 2]);
        assert_eq!(quotient, [1.0, 2.0]);
        assert_eq!(remainder_shape, [1, 3]);
        assert_eq!(remainder, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn polynomial_family_covers_integer_saturation_complex_and_ratio_derivative() {
        let integer = invoke(
            "poly",
            &[Value::Array(ArrayData::Integer(IntegerArrayData::I8(
                DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![100_i8, 100]).unwrap(),
            )))],
            1,
        )
        .unwrap();
        let Value::Array(ArrayData::Integer(IntegerArrayData::I8(integer))) = &integer[0] else {
            panic!("poly must preserve integer class");
        };
        assert_eq!(integer.as_slice(), [1, -128, 127]);

        let values = invoke(
            "polyval",
            &[
                complex_array(
                    [1, 2],
                    vec![Complex64::new(1.0, 1.0), Complex64::new(2.0, 0.0)],
                ),
                array([1, 2], vec![1.0, 2.0]),
            ],
            1,
        )
        .unwrap();
        let Value::Array(ArrayData::ComplexF64(values)) = &values[0] else {
            panic!("complex coefficients must produce complex values");
        };
        assert_eq!(
            values.as_slice(),
            [Complex64::new(3.0, 1.0), Complex64::new(4.0, 2.0)]
        );

        let ratio = invoke(
            "polyder",
            &[array([1, 2], vec![1.0, 2.0]), array([1, 2], vec![3.0, 4.0])],
            2,
        )
        .unwrap();
        assert_eq!(ratio[0], Value::Double(-2.0));
        let (denominator, shape) = f64_data(&ratio[1]);
        assert_eq!(shape, [1, 3]);
        assert_eq!(denominator, [9.0, 24.0, 16.0]);

        let integral = invoke(
            "polyint",
            &[single_array([1, 2], vec![1.0, 2.0]), Value::Double(3.0)],
            1,
        )
        .unwrap();
        let Value::Array(ArrayData::F32(integral)) = &integral[0] else {
            panic!("single coefficient must produce single integral");
        };
        assert_eq!(integral.as_slice(), [0.5, 2.0, 3.0]);
    }

    #[test]
    fn roots_uses_provider_and_preserves_zero_roots_and_column_shape() {
        let output = invoke(
            "roots",
            &[array([1, 7], vec![0.0, 0.0, 1.0, -3.0, 2.0, 0.0, 0.0])],
            1,
        )
        .unwrap();
        let (values, shape) = f64_data(&output[0]);
        assert_eq!(shape, [4, 1]);
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        assert_eq!(&sorted[..2], [0.0, 0.0]);
        assert!((sorted[2] - 1.0).abs() < 1.0e-12);
        assert!((sorted[3] - 2.0).abs() < 1.0e-12);

        let nonfinite = invoke("roots", &[array([1, 2], vec![1.0, f64::NAN])], 1)
            .expect_err("nonfinite coefficients must be rejected");
        assert_eq!(nonfinite.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn interp1_sorts_x_interpolates_columns_and_controls_extrapolation() {
        let output = invoke(
            "interp1",
            &[
                array([1, 3], vec![3.0, 1.0, 2.0]),
                array([3, 2], vec![300.0, 10.0, 20.0, 30.0, 1.0, 2.0]),
                array([1, 2], vec![1.5, 2.5]),
            ],
            1,
        )
        .unwrap();
        let (values, shape) = f64_data(&output[0]);
        assert_eq!(shape, [2, 2]);
        assert_eq!(values, [15.0, 160.0, 1.5, 16.0]);

        let nearest = invoke(
            "interp1",
            &[
                array([1, 3], vec![1.0, 2.0, 3.0]),
                array([1, 3], vec![10.0, 20.0, 30.0]),
                array([1, 2], vec![0.0, 4.0]),
                Value::from("nearest"),
                Value::from("extrap"),
            ],
            1,
        )
        .unwrap();
        let (values, shape) = f64_data(&nearest[0]);
        assert_eq!(shape, [1, 2]);
        assert_eq!(values, [10.0, 30.0]);

        let duplicate = invoke(
            "interp1",
            &[
                array([1, 3], vec![1.0, 2.0, 2.0]),
                array([1, 3], vec![10.0, 20.0, 21.0]),
                Value::Double(2.0),
            ],
            1,
        )
        .expect_err("duplicate X points must be rejected");
        assert_eq!(duplicate.category, BuiltinErrorCategory::Domain);

        let pchip = invoke(
            "interp1",
            &[
                array([1, 3], vec![10.0, 20.0, 30.0]),
                Value::Double(1.5),
                Value::from("pchip"),
            ],
            1,
        )
        .expect_err("pchip must not be silently approximated");
        assert_eq!(pchip.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn class_boundaries_reject_unimplemented_numeric_coercions() {
        let polyval_integer = invoke(
            "polyval",
            &[i16_array([1, 2], vec![1, 2]), array([1, 2], vec![1.0, 2.0])],
            1,
        )
        .expect_err("polyval integer coefficients are unsupported by MATLAB");
        assert_eq!(polyval_integer.category, BuiltinErrorCategory::Type);

        let interp_logical = invoke(
            "interp1",
            &[
                array([1, 3], vec![1.0, 2.0, 3.0]),
                logical_array([1, 3], vec![true, false, true]),
                Value::Double(1.5),
            ],
            1,
        )
        .expect_err("interp1 logical Y is unsupported by MATLAB");
        assert_eq!(interp_logical.category, BuiltinErrorCategory::Type);

        let logical_matrix = logical_array([2, 2], vec![true, false, false, true]);
        let poly_logical_matrix = invoke("poly", &[logical_matrix], 1)
            .expect_err("poly logical matrices are rejected by MATLAB eig");
        assert_eq!(poly_logical_matrix.category, BuiltinErrorCategory::Type);
    }
}

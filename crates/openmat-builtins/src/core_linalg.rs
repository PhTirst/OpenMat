use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerComponent, Shape,
};
use openmat_linalg::{
    CholeskyRequest, CholeskyResult, CholeskyTriangle, FactorRequest, LuResult,
    MatrixFunctionRequest, QrRequest, QrResult, QrVectors, SolveRequest, SwapParity,
    matrix_sqrt_complex32, matrix_sqrt_complex64, matrix_sqrt_f32, matrix_sqrt_f64,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{Complex64 as ValueComplex64, Value};

use crate::{
    array_error, exact_real_integer_scalar, expect_argument_count, expect_argument_count_range,
    expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Precision {
    Double,
    Single,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PermutationOutput {
    Matrix,
    Vector,
}

#[derive(Clone, Copy, Debug)]
struct QrOptions {
    vectors: QrVectors,
    permutation: PermutationOutput,
}

trait FactorElement: Copy + Default {
    fn one() -> Self;
    fn multiply(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
    fn negate(self) -> Self;
    fn diagonal_is_real(self) -> bool;
}

impl FactorElement for f64 {
    fn one() -> Self {
        1.0
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn subtract(self, right: Self) -> Self {
        self - right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn negate(self) -> Self {
        -self
    }

    fn diagonal_is_real(self) -> bool {
        true
    }
}

impl FactorElement for f32 {
    fn one() -> Self {
        1.0
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn subtract(self, right: Self) -> Self {
        self - right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn negate(self) -> Self {
        -self
    }

    fn diagonal_is_real(self) -> bool {
        true
    }
}

impl FactorElement for ArrayComplex64 {
    fn one() -> Self {
        Self::new(1.0, 0.0)
    }

    fn multiply(self, right: Self) -> Self {
        Self::new(
            self.re * right.re - self.im * right.im,
            self.re * right.im + self.im * right.re,
        )
    }

    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }

    fn divide(self, right: Self) -> Self {
        if right.re.abs() >= right.im.abs() {
            let ratio = right.im / right.re;
            let denominator = right.re + right.im * ratio;
            Self::new(
                (self.re + self.im * ratio) / denominator,
                (self.im - self.re * ratio) / denominator,
            )
        } else {
            let ratio = right.re / right.im;
            let denominator = right.im + right.re * ratio;
            Self::new(
                (self.re * ratio + self.im) / denominator,
                (self.im * ratio - self.re) / denominator,
            )
        }
    }

    fn negate(self) -> Self {
        Self::new(-self.re, -self.im)
    }

    fn diagonal_is_real(self) -> bool {
        self.im == 0.0
    }
}

impl FactorElement for ArrayComplex32 {
    fn one() -> Self {
        Self::new(1.0, 0.0)
    }

    fn multiply(self, right: Self) -> Self {
        Self::new(
            self.re * right.re - self.im * right.im,
            self.re * right.im + self.im * right.re,
        )
    }

    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }

    fn divide(self, right: Self) -> Self {
        if right.re.abs() >= right.im.abs() {
            let ratio = right.im / right.re;
            let denominator = right.re + right.im * ratio;
            Self::new(
                (self.re + self.im * ratio) / denominator,
                (self.im - self.re * ratio) / denominator,
            )
        } else {
            let ratio = right.re / right.im;
            let denominator = right.im + right.re * ratio;
            Self::new(
                (self.re * ratio + self.im) / denominator,
                (self.im * ratio - self.re) / denominator,
            )
        }
    }

    fn negate(self) -> Self {
        Self::new(-self.re, -self.im)
    }

    fn diagonal_is_real(self) -> bool {
        self.im == 0.0
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn lu_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("lu", arguments, 1, 2)?;
    expect_max_outputs("lu", context, 3)?;
    context.check_cancelled()?;
    let permutation =
        arguments
            .get(1)
            .map_or(Ok(PermutationOutput::Matrix), |value| {
                match keyword(value).as_deref() {
                    Some("matrix") => Ok(PermutationOutput::Matrix),
                    Some("vector") => Ok(PermutationOutput::Vector),
                    _ => Err(option_error("lu", "'matrix' or 'vector'")),
                }
            })?;
    let requested_outputs = context.requested_outputs().max(1);
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            let result = context
                .linalg_provider()
                .factor_lu_f64(lu_request(&input, context))
                .map_err(|error| linalg_error("lu", error))?;
            lu_outputs(
                &input,
                result,
                requested_outputs,
                permutation,
                Precision::Double,
                wrap_f64,
                context,
            )
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let result = context
                .linalg_provider()
                .factor_lu_complex64(lu_request(&input, context))
                .map_err(|error| linalg_error("lu", error))?;
            lu_outputs(
                &input,
                result,
                requested_outputs,
                permutation,
                Precision::Double,
                wrap_complex64,
                context,
            )
        }
        Value::Array(ArrayData::F32(input)) => {
            let result = context
                .linalg_provider()
                .factor_lu_f32(lu_request(input, context))
                .map_err(|error| linalg_error("lu", error))?;
            lu_outputs(
                input,
                result,
                requested_outputs,
                permutation,
                Precision::Single,
                wrap_f32,
                context,
            )
        }
        Value::Array(ArrayData::ComplexF32(input)) => {
            let result = context
                .linalg_provider()
                .factor_lu_complex32(lu_request(input, context))
                .map_err(|error| linalg_error("lu", error))?;
            lu_outputs(
                input,
                result,
                requested_outputs,
                permutation,
                Precision::Single,
                wrap_complex32,
                context,
            )
        }
        Value::Array(ArrayData::F64(input)) => {
            let result = context
                .linalg_provider()
                .factor_lu_f64(lu_request(input, context))
                .map_err(|error| linalg_error("lu", error))?;
            lu_outputs(
                input,
                result,
                requested_outputs,
                permutation,
                Precision::Double,
                wrap_f64,
                context,
            )
        }
        Value::Array(ArrayData::ComplexF64(input)) => {
            let result = context
                .linalg_provider()
                .factor_lu_complex64(lu_request(input, context))
                .map_err(|error| linalg_error("lu", error))?;
            lu_outputs(
                input,
                result,
                requested_outputs,
                permutation,
                Precision::Double,
                wrap_complex64,
                context,
            )
        }
        value => Err(type_error(
            "lu",
            1,
            "real or complex double or single matrix",
            value,
        )),
    }
}

fn lu_request<'a, T>(
    input: &'a DenseArray<T>,
    context: &'a BuiltinContext<'_>,
) -> FactorRequest<'a, T> {
    FactorRequest::new(input).with_cancellation_flag(context.cancellation_flag())
}

fn lu_outputs<T, F>(
    input: &DenseArray<T>,
    result: LuResult<T>,
    requested_outputs: usize,
    permutation_output: PermutationOutput,
    precision: Precision,
    wrap: F,
    context: &BuiltinContext<'_>,
) -> BuiltinResult
where
    T: FactorElement,
    F: Fn(DenseArray<T>, &BuiltinContext<'_>) -> Result<Value, BuiltinError>,
{
    validate_lu_result(input, &result)?;
    if requested_outputs == 1 {
        return Ok(vec![wrap(result.packed_lu, context)?]);
    }
    let (mut lower, upper) = split_lu(&result.packed_lu, context)?;
    if requested_outputs == 2 {
        lower = fold_lu_row_permutation(&lower, &result.row_permutation_zero_based, context)?;
        return Ok(vec![wrap(lower, context)?, wrap(upper, context)?]);
    }
    let permutation = match permutation_output {
        PermutationOutput::Matrix => permutation_matrix(
            &result.row_permutation_zero_based,
            precision,
            PermutationOrientation::Rows,
            context,
        )?,
        PermutationOutput::Vector => {
            permutation_vector(&result.row_permutation_zero_based, context)?
        }
    };
    Ok(vec![
        wrap(lower, context)?,
        wrap(upper, context)?,
        permutation,
    ])
}

#[allow(clippy::too_many_lines)]
pub(super) fn inv_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("inv", arguments, 1)?;
    expect_max_outputs("inv", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            inverse_with(
                &input,
                &context
                    .linalg_provider()
                    .factor_lu_f64(lu_request(&input, context))
                    .map_err(|error| linalg_error("inv", error))?,
                wrap_f64,
                context,
            )?
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            inverse_with(
                &input,
                &context
                    .linalg_provider()
                    .factor_lu_complex64(lu_request(&input, context))
                    .map_err(|error| linalg_error("inv", error))?,
                wrap_complex64,
                context,
            )?
        }
        Value::Array(ArrayData::F32(input)) => inverse_with(
            input,
            &context
                .linalg_provider()
                .factor_lu_f32(lu_request(input, context))
                .map_err(|error| linalg_error("inv", error))?,
            wrap_f32,
            context,
        )?,
        Value::Array(ArrayData::ComplexF32(input)) => inverse_with(
            input,
            &context
                .linalg_provider()
                .factor_lu_complex32(lu_request(input, context))
                .map_err(|error| linalg_error("inv", error))?,
            wrap_complex32,
            context,
        )?,
        Value::Array(ArrayData::F64(input)) => inverse_with(
            input,
            &context
                .linalg_provider()
                .factor_lu_f64(lu_request(input, context))
                .map_err(|error| linalg_error("inv", error))?,
            wrap_f64,
            context,
        )?,
        Value::Array(ArrayData::ComplexF64(input)) => inverse_with(
            input,
            &context
                .linalg_provider()
                .factor_lu_complex64(lu_request(input, context))
                .map_err(|error| linalg_error("inv", error))?,
            wrap_complex64,
            context,
        )?,
        value => {
            return Err(type_error(
                "inv",
                1,
                "real or complex double or single square matrix",
                value,
            ));
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn sqrtm_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("sqrtm", arguments, 1)?;
    expect_max_outputs("sqrtm", context, 3)?;
    context.check_cancelled()?;
    let requested_outputs = context.requested_outputs().max(1);
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            sqrtm_f64_outputs(&input, requested_outputs, context)
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            sqrtm_complex64_outputs(&input, requested_outputs, context)
        }
        Value::Array(ArrayData::F64(input)) => sqrtm_f64_outputs(input, requested_outputs, context),
        Value::Array(ArrayData::ComplexF64(input)) => {
            sqrtm_complex64_outputs(input, requested_outputs, context)
        }
        Value::Array(ArrayData::F32(input)) => sqrtm_f32_outputs(input, requested_outputs, context),
        Value::Array(ArrayData::ComplexF32(input)) => {
            sqrtm_complex32_outputs(input, requested_outputs, context)
        }
        value => Err(
            type_error("sqrtm", 1, "real or complex double or single matrix", value)
                .with_identifier("MATLAB:sqrtm:inputType"),
        ),
    }
}

fn sqrtm_f64_outputs(
    input: &DenseArray<f64>,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    validate_sqrtm(input, |value| value.is_finite())?;
    let input_norm = real_one_norm(input);
    let result = matrix_sqrt_f64(
        context.linalg_provider(),
        MatrixFunctionRequest::new(input).with_cancellation_flag(context.cancellation_flag()),
    )
    .map_err(|error| linalg_error("sqrtm", error))?;
    sqrtm_outputs64(
        result.root,
        result.relative_residual_1,
        input_norm,
        requested_outputs,
        context,
    )
}

fn sqrtm_complex64_outputs(
    input: &DenseArray<ArrayComplex64>,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    validate_sqrtm(input, |value| value.re.is_finite() && value.im.is_finite())?;
    let input_norm = complex64_one_norm(input.as_slice(), input.shape().extent(0));
    let result = matrix_sqrt_complex64(
        context.linalg_provider(),
        MatrixFunctionRequest::new(input).with_cancellation_flag(context.cancellation_flag()),
    )
    .map_err(|error| linalg_error("sqrtm", error))?;
    sqrtm_outputs64(
        result.root,
        result.relative_residual_1,
        input_norm,
        requested_outputs,
        context,
    )
}

fn sqrtm_f32_outputs(
    input: &DenseArray<f32>,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    validate_sqrtm(input, |value| value.is_finite())?;
    let input_norm = real32_one_norm(input);
    let result = matrix_sqrt_f32(
        context.linalg_provider(),
        MatrixFunctionRequest::new(input).with_cancellation_flag(context.cancellation_flag()),
    )
    .map_err(|error| linalg_error("sqrtm", error))?;
    sqrtm_outputs32(
        result.root,
        result.relative_residual_1,
        input_norm,
        requested_outputs,
        context,
    )
}

fn sqrtm_complex32_outputs(
    input: &DenseArray<ArrayComplex32>,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    validate_sqrtm(input, |value| value.re.is_finite() && value.im.is_finite())?;
    let input_norm = complex32_one_norm(input.as_slice(), input.shape().extent(0));
    let result = matrix_sqrt_complex32(
        context.linalg_provider(),
        MatrixFunctionRequest::new(input).with_cancellation_flag(context.cancellation_flag()),
    )
    .map_err(|error| linalg_error("sqrtm", error))?;
    sqrtm_outputs32(
        result.root,
        result.relative_residual_1,
        input_norm,
        requested_outputs,
        context,
    )
}

fn validate_sqrtm<T>(
    input: &DenseArray<T>,
    finite: impl Fn(&T) -> bool,
) -> Result<(), BuiltinError> {
    if input.shape().ndims() != 2 || input.shape().extent(0) != input.shape().extent(1) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`sqrtm` requires a square matrix",
        )
        .with_identifier("MATLAB:sqrtm:inputMustBeSquare"));
    }
    if !input.as_slice().iter().all(finite) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`sqrtm` requires finite floating-point input",
        )
        .with_identifier("MATLAB:sqrtm:inputMustBeFinite"));
    }
    Ok(())
}

fn sqrtm_outputs64(
    root: DenseArray<ArrayComplex64>,
    residual: f64,
    input_norm: f64,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let mut outputs = Vec::with_capacity(requested_outputs);
    if requested_outputs == 3 {
        let order = root.shape().extent(0);
        let root_norm = complex64_one_norm(root.as_slice(), order);
        let alpha = stability_factor(root_norm, input_norm);
        let condition = sqrt_condition64(&root, input_norm, context)?;
        outputs.push(wrap_complex64(root, context)?);
        outputs.push(Value::Double(alpha));
        outputs.push(Value::Double(condition));
    } else {
        outputs.push(wrap_complex64(root, context)?);
        if requested_outputs == 2 {
            outputs.push(Value::Double(residual));
        }
    }
    Ok(outputs)
}

fn sqrtm_outputs32(
    root: DenseArray<ArrayComplex32>,
    residual: f32,
    input_norm: f32,
    requested_outputs: usize,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let mut outputs = Vec::with_capacity(requested_outputs);
    if requested_outputs == 3 {
        let order = root.shape().extent(0);
        let root_norm = complex32_one_norm(root.as_slice(), order);
        let alpha = stability_factor(root_norm, input_norm);
        let condition = sqrt_condition32(&root, input_norm, context)?;
        outputs.push(wrap_complex32(root, context)?);
        outputs.push(single_scalar(alpha)?);
        outputs.push(single_scalar(condition)?);
    } else {
        outputs.push(wrap_complex32(root, context)?);
        if requested_outputs == 2 {
            outputs.push(single_scalar(residual)?);
        }
    }
    Ok(outputs)
}

fn stability_factor<T>(root_norm: T, input_norm: T) -> T
where
    T: Copy + std::ops::Mul<Output = T> + std::ops::Div<Output = T>,
{
    root_norm * root_norm / input_norm
}

fn sqrt_condition64(
    root: &DenseArray<ArrayComplex64>,
    input_norm: f64,
    context: &BuiltinContext<'_>,
) -> Result<f64, BuiltinError> {
    let order = root.shape().extent(0);
    if order == 0 {
        return Ok(f64::NAN);
    }
    let operator = sqrt_frechet_operator64(root)?;
    let identity = complex_identity64(operator.shape().extent(0))?;
    let inverse = match context.linalg_provider().solve_complex64(
        SolveRequest::new(&operator, &identity).with_cancellation_flag(context.cancellation_flag()),
    ) {
        Ok(value) => value,
        Err(openmat_linalg::LinalgError::SingularMatrix { .. }) => return Ok(f64::INFINITY),
        Err(error) => return Err(linalg_error("sqrtm", error)),
    };
    let root_norm = complex64_one_norm(root.as_slice(), order);
    Ok(complex64_one_norm(inverse.as_slice(), order * order) * input_norm / root_norm)
}

fn sqrt_condition32(
    root: &DenseArray<ArrayComplex32>,
    input_norm: f32,
    context: &BuiltinContext<'_>,
) -> Result<f32, BuiltinError> {
    let order = root.shape().extent(0);
    if order == 0 {
        return Ok(f32::NAN);
    }
    let operator = sqrt_frechet_operator32(root)?;
    let identity = complex_identity32(operator.shape().extent(0))?;
    let inverse = match context.linalg_provider().solve_complex32(
        SolveRequest::new(&operator, &identity).with_cancellation_flag(context.cancellation_flag()),
    ) {
        Ok(value) => value,
        Err(openmat_linalg::LinalgError::SingularMatrix { .. }) => return Ok(f32::INFINITY),
        Err(error) => return Err(linalg_error("sqrtm", error)),
    };
    let root_norm = complex32_one_norm(root.as_slice(), order);
    Ok(complex32_one_norm(inverse.as_slice(), order * order) * input_norm / root_norm)
}

#[allow(clippy::cast_possible_truncation)]
fn sqrt_frechet_operator64(
    root: &DenseArray<ArrayComplex64>,
) -> Result<DenseArray<ArrayComplex64>, BuiltinError> {
    let order = root.shape().extent(0);
    let squared = order
        .checked_mul(order)
        .ok_or_else(|| host_length_error("sqrtm", u64::MAX))?;
    let length = squared
        .checked_mul(squared)
        .ok_or_else(|| host_length_error("sqrtm", u64::MAX))?;
    let mut values = filled_values("sqrtm condition operator", length, ArrayComplex64::ZERO)?;
    for column_block in 0..order {
        for column_row in 0..order {
            let column = column_row + column_block * order;
            for row_block in 0..order {
                for row_row in 0..order {
                    let row = row_row + row_block * order;
                    let mut value = ArrayComplex64::ZERO;
                    if row_block == column_block {
                        value += root.as_slice()[(column_row * order + row_row) as usize];
                    }
                    if row_row == column_row {
                        value += root.as_slice()[(row_block * order + column_block) as usize];
                    }
                    values[(column * squared + row) as usize] = value;
                }
            }
        }
    }
    DenseArray::from_vec(
        Shape::new([squared, squared]).map_err(|error| array_error(&error))?,
        values,
    )
    .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_possible_truncation)]
fn sqrt_frechet_operator32(
    root: &DenseArray<ArrayComplex32>,
) -> Result<DenseArray<ArrayComplex32>, BuiltinError> {
    let order = root.shape().extent(0);
    let squared = order
        .checked_mul(order)
        .ok_or_else(|| host_length_error("sqrtm", u64::MAX))?;
    let length = squared
        .checked_mul(squared)
        .ok_or_else(|| host_length_error("sqrtm", u64::MAX))?;
    let mut values = filled_values("sqrtm condition operator", length, ArrayComplex32::ZERO)?;
    for column_block in 0..order {
        for column_row in 0..order {
            let column = column_row + column_block * order;
            for row_block in 0..order {
                for row_row in 0..order {
                    let row = row_row + row_block * order;
                    let mut value = ArrayComplex32::ZERO;
                    if row_block == column_block {
                        let add = root.as_slice()[(column_row * order + row_row) as usize];
                        value = ArrayComplex32::new(value.re + add.re, value.im + add.im);
                    }
                    if row_row == column_row {
                        let add = root.as_slice()[(row_block * order + column_block) as usize];
                        value = ArrayComplex32::new(value.re + add.re, value.im + add.im);
                    }
                    values[(column * squared + row) as usize] = value;
                }
            }
        }
    }
    DenseArray::from_vec(
        Shape::new([squared, squared]).map_err(|error| array_error(&error))?,
        values,
    )
    .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_possible_truncation)]
fn complex_identity64(order: u64) -> Result<DenseArray<ArrayComplex64>, BuiltinError> {
    let length = order
        .checked_mul(order)
        .ok_or_else(|| host_length_error("sqrtm", u64::MAX))?;
    let mut values = filled_values("sqrtm condition identity", length, ArrayComplex64::ZERO)?;
    for index in 0..order {
        values[(index * order + index) as usize] = ArrayComplex64::new(1.0, 0.0);
    }
    DenseArray::from_vec(
        Shape::new([order, order]).map_err(|error| array_error(&error))?,
        values,
    )
    .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_possible_truncation)]
fn complex_identity32(order: u64) -> Result<DenseArray<ArrayComplex32>, BuiltinError> {
    let length = order
        .checked_mul(order)
        .ok_or_else(|| host_length_error("sqrtm", u64::MAX))?;
    let mut values = filled_values("sqrtm condition identity", length, ArrayComplex32::ZERO)?;
    for index in 0..order {
        values[(index * order + index) as usize] = ArrayComplex32::new(1.0, 0.0);
    }
    DenseArray::from_vec(
        Shape::new([order, order]).map_err(|error| array_error(&error))?,
        values,
    )
    .map_err(|error| array_error(&error))
}

#[allow(clippy::cast_possible_truncation)]
fn real_one_norm(input: &DenseArray<f64>) -> f64 {
    let order = input.shape().extent(0) as usize;
    input
        .as_slice()
        .chunks(order.max(1))
        .map(|column| column.iter().map(|value| value.abs()).sum())
        .fold(0.0, f64::max)
}

#[allow(clippy::cast_possible_truncation)]
fn real32_one_norm(input: &DenseArray<f32>) -> f32 {
    let order = input.shape().extent(0) as usize;
    input
        .as_slice()
        .chunks(order.max(1))
        .map(|column| column.iter().map(|value| value.abs()).sum())
        .fold(0.0, f32::max)
}

#[allow(clippy::cast_possible_truncation)]
fn complex64_one_norm(values: &[ArrayComplex64], order: u64) -> f64 {
    values
        .chunks((order as usize).max(1))
        .map(|column| column.iter().map(|value| value.re.hypot(value.im)).sum())
        .fold(0.0, f64::max)
}

#[allow(clippy::cast_possible_truncation)]
fn complex32_one_norm(values: &[ArrayComplex32], order: u64) -> f32 {
    values
        .chunks((order as usize).max(1))
        .map(|column| column.iter().map(|value| value.re.hypot(value.im)).sum())
        .fold(0.0, f32::max)
}

fn single_scalar(value: f32) -> Result<Value, BuiltinError> {
    Ok(Value::Array(ArrayData::F32(scalar_dense(value)?)))
}

fn inverse_with<T, F>(
    input: &DenseArray<T>,
    factor: &LuResult<T>,
    wrap: F,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError>
where
    T: FactorElement,
    F: Fn(DenseArray<T>, &BuiltinContext<'_>) -> Result<Value, BuiltinError>,
{
    ensure_square("inv", input)?;
    validate_lu_result_for("inv", input, factor)?;
    if factor.first_zero_pivot.is_some() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`inv` cannot yet reproduce R2022b partial inverse results for an exactly singular matrix",
        ));
    }
    let order = checked_host_length("inv", input.shape().extent(0))?;
    let mut output = filled_values("inv", input.numel(), T::default())?;
    let mut column = filled_values("inv", input.shape().extent(0), T::default())?;
    for right_hand_side in 0..order {
        context.check_cancelled()?;
        column.fill(T::default());
        for (factor_row, original_row) in factor
            .row_permutation_zero_based
            .iter()
            .copied()
            .enumerate()
        {
            if usize::try_from(original_row).ok() == Some(right_hand_side) {
                column[factor_row] = T::one();
            }
        }
        for row in 0..order {
            let mut value = column[row];
            for (preceding, preceding_value) in column.iter().copied().enumerate().take(row) {
                value = value.subtract(
                    factor.packed_lu.as_slice()[preceding * order + row].multiply(preceding_value),
                );
            }
            column[row] = value;
        }
        for row in (0..order).rev() {
            let mut value = column[row];
            for (following, following_value) in column
                .iter()
                .copied()
                .enumerate()
                .skip(row.saturating_add(1))
            {
                value = value.subtract(
                    factor.packed_lu.as_slice()[following * order + row].multiply(following_value),
                );
            }
            column[row] = value.divide(factor.packed_lu.as_slice()[row * order + row]);
        }
        output[right_hand_side * order..right_hand_side.saturating_add(1) * order]
            .copy_from_slice(&column);
    }
    wrap(
        DenseArray::from_vec(input.shape().clone(), output).map_err(|error| array_error(&error))?,
        context,
    )
}

fn validate_lu_result<T>(input: &DenseArray<T>, result: &LuResult<T>) -> Result<(), BuiltinError> {
    validate_lu_result_for("lu", input, result)
}

fn validate_lu_result_for<T>(
    name: &str,
    input: &DenseArray<T>,
    result: &LuResult<T>,
) -> Result<(), BuiltinError> {
    if result.packed_lu.shape() != input.shape() {
        return Err(provider_contract_error(name, "packed factor shape"));
    }
    validate_permutation(
        name,
        &result.row_permutation_zero_based,
        input.shape().extent(0),
    )?;
    if result
        .first_zero_pivot
        .is_some_and(|pivot| pivot >= input.shape().extent(0).min(input.shape().extent(1)))
    {
        return Err(provider_contract_error(name, "zero-pivot index"));
    }
    Ok(())
}

fn split_lu<T: FactorElement>(
    packed: &DenseArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<(DenseArray<T>, DenseArray<T>), BuiltinError> {
    let rows = packed.shape().extent(0);
    let columns = packed.shape().extent(1);
    let order = rows.min(columns);
    let lower_shape = Shape::new([rows, order]).map_err(|error| array_error(&error))?;
    let upper_shape = Shape::new([order, columns]).map_err(|error| array_error(&error))?;
    let mut lower = filled_values("lu", lower_shape.numel(), T::default())?;
    let mut upper = filled_values("lu", upper_shape.numel(), T::default())?;
    let rows = checked_host_length("lu", rows)?;
    let columns = checked_host_length("lu", columns)?;
    let order = checked_host_length("lu", order)?;
    for column in 0..order {
        check_cancelled_at(context, column)?;
        for row in column..rows {
            lower[column * rows + row] = if row == column {
                T::one()
            } else {
                packed.as_slice()[column * rows + row]
            };
        }
    }
    for column in 0..columns {
        check_cancelled_at(context, column)?;
        for row in 0..order.min(column.saturating_add(1)) {
            upper[column * order + row] = packed.as_slice()[column * rows + row];
        }
    }
    Ok((
        DenseArray::from_vec(lower_shape, lower).map_err(|error| array_error(&error))?,
        DenseArray::from_vec(upper_shape, upper).map_err(|error| array_error(&error))?,
    ))
}

fn fold_lu_row_permutation<T: FactorElement>(
    lower: &DenseArray<T>,
    permutation: &[u64],
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let rows = checked_host_length("lu", lower.shape().extent(0))?;
    let columns = checked_host_length("lu", lower.shape().extent(1))?;
    let mut folded = filled_values("lu", lower.numel(), T::default())?;
    for (factor_row, original_row) in permutation.iter().copied().enumerate() {
        check_cancelled_at(context, factor_row)?;
        let original_row = checked_host_length("lu", original_row)?;
        for column in 0..columns {
            folded[column * rows + original_row] = lower.as_slice()[column * rows + factor_row];
        }
    }
    DenseArray::from_vec(lower.shape().clone(), folded).map_err(|error| array_error(&error))
}

#[allow(clippy::too_many_lines)]
pub(super) fn qr_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("qr", arguments, 1, 3)?;
    expect_max_outputs("qr", context, 3)?;
    context.check_cancelled()?;
    let options = qr_options(arguments)?;
    let requested_outputs = context.requested_outputs().max(1);
    let pivot_columns = requested_outputs == 3;
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            let result = context
                .linalg_provider()
                .qr_f64(qr_request(&input, options.vectors, pivot_columns, context))
                .map_err(|error| linalg_error("qr", error))?;
            qr_outputs(
                &input,
                result,
                requested_outputs,
                options,
                Precision::Double,
                wrap_f64,
                context,
            )
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let result = context
                .linalg_provider()
                .qr_complex64(qr_request(&input, options.vectors, pivot_columns, context))
                .map_err(|error| linalg_error("qr", error))?;
            qr_outputs(
                &input,
                result,
                requested_outputs,
                options,
                Precision::Double,
                wrap_complex64,
                context,
            )
        }
        Value::Array(ArrayData::F32(input)) => {
            let result = context
                .linalg_provider()
                .qr_f32(qr_request(input, options.vectors, pivot_columns, context))
                .map_err(|error| linalg_error("qr", error))?;
            qr_outputs(
                input,
                result,
                requested_outputs,
                options,
                Precision::Single,
                wrap_f32,
                context,
            )
        }
        Value::Array(ArrayData::ComplexF32(input)) => {
            let result = context
                .linalg_provider()
                .qr_complex32(qr_request(input, options.vectors, pivot_columns, context))
                .map_err(|error| linalg_error("qr", error))?;
            qr_outputs(
                input,
                result,
                requested_outputs,
                options,
                Precision::Single,
                wrap_complex32,
                context,
            )
        }
        Value::Array(ArrayData::F64(input)) => {
            let result = context
                .linalg_provider()
                .qr_f64(qr_request(input, options.vectors, pivot_columns, context))
                .map_err(|error| linalg_error("qr", error))?;
            qr_outputs(
                input,
                result,
                requested_outputs,
                options,
                Precision::Double,
                wrap_f64,
                context,
            )
        }
        Value::Array(ArrayData::ComplexF64(input)) => {
            let result = context
                .linalg_provider()
                .qr_complex64(qr_request(input, options.vectors, pivot_columns, context))
                .map_err(|error| linalg_error("qr", error))?;
            qr_outputs(
                input,
                result,
                requested_outputs,
                options,
                Precision::Double,
                wrap_complex64,
                context,
            )
        }
        value => Err(type_error(
            "qr",
            1,
            "real or complex double or single matrix",
            value,
        )),
    }
}

fn qr_options(arguments: &[Value]) -> Result<QrOptions, BuiltinError> {
    if arguments.len() == 1 {
        return Ok(QrOptions {
            vectors: QrVectors::Full,
            permutation: PermutationOutput::Matrix,
        });
    }
    if is_numeric_zero_scalar(&arguments[1]) {
        if arguments.len() != 2 {
            return Err(option_error("qr", "a standalone economy flag 0"));
        }
        return Ok(QrOptions {
            vectors: QrVectors::Thin,
            permutation: PermutationOutput::Vector,
        });
    }
    let first = keyword(&arguments[1]);
    if arguments.len() == 2 {
        return match first.as_deref() {
            Some("econ") => Ok(QrOptions {
                vectors: QrVectors::Thin,
                permutation: PermutationOutput::Matrix,
            }),
            Some("matrix") => Ok(QrOptions {
                vectors: QrVectors::Full,
                permutation: PermutationOutput::Matrix,
            }),
            Some("vector") => Ok(QrOptions {
                vectors: QrVectors::Full,
                permutation: PermutationOutput::Vector,
            }),
            _ => Err(option_error("qr", "0, 'econ', 'matrix', or 'vector'")),
        };
    }
    if first.as_deref() != Some("econ") {
        return Err(option_error(
            "qr",
            "the combined form 'econ', 'matrix' or 'econ', 'vector'",
        ));
    }
    let permutation = match keyword(&arguments[2]).as_deref() {
        Some("matrix") => PermutationOutput::Matrix,
        Some("vector") => PermutationOutput::Vector,
        _ => {
            return Err(option_error(
                "qr",
                "the combined form 'econ', 'matrix' or 'econ', 'vector'",
            ));
        }
    };
    Ok(QrOptions {
        vectors: QrVectors::Thin,
        permutation,
    })
}

fn qr_request<'a, T>(
    input: &'a DenseArray<T>,
    vectors: QrVectors,
    pivot_columns: bool,
    context: &'a BuiltinContext<'_>,
) -> QrRequest<'a, T> {
    QrRequest::new(input, vectors)
        .with_column_pivoting(pivot_columns)
        .with_cancellation_flag(context.cancellation_flag())
}

fn qr_outputs<T, F>(
    input: &DenseArray<T>,
    result: QrResult<T>,
    requested_outputs: usize,
    options: QrOptions,
    precision: Precision,
    wrap: F,
    context: &BuiltinContext<'_>,
) -> BuiltinResult
where
    T: FactorElement,
    F: Fn(DenseArray<T>, &BuiltinContext<'_>) -> Result<Value, BuiltinError>,
{
    validate_qr_result(input, &result, options.vectors)?;
    if requested_outputs == 1 {
        return Ok(vec![wrap(result.r, context)?]);
    }
    if requested_outputs == 2 {
        return Ok(vec![wrap(result.q, context)?, wrap(result.r, context)?]);
    }
    let permutation = match options.permutation {
        PermutationOutput::Matrix => permutation_matrix(
            &result.column_permutation_zero_based,
            precision,
            PermutationOrientation::Columns,
            context,
        )?,
        PermutationOutput::Vector => {
            permutation_vector(&result.column_permutation_zero_based, context)?
        }
    };
    Ok(vec![
        wrap(result.q, context)?,
        wrap(result.r, context)?,
        permutation,
    ])
}

fn validate_qr_result<T>(
    input: &DenseArray<T>,
    result: &QrResult<T>,
    vectors: QrVectors,
) -> Result<(), BuiltinError> {
    let rows = input.shape().extent(0);
    let columns = input.shape().extent(1);
    let q_columns = match vectors {
        QrVectors::Full => rows,
        QrVectors::Thin => rows.min(columns),
    };
    if result.q.shape().dimensions() != [rows, q_columns]
        || result.r.shape().dimensions() != [q_columns, columns]
    {
        return Err(provider_contract_error("qr", "factor shapes"));
    }
    validate_permutation("qr", &result.column_permutation_zero_based, columns)?;
    if result.numerical_rank > rows.min(columns)
        || result
            .first_rank_deficient_diagonal
            .is_some_and(|diagonal| diagonal >= rows.min(columns))
    {
        return Err(provider_contract_error("qr", "rank metadata"));
    }
    Ok(())
}

pub(super) fn chol_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("chol", arguments, 1, 2)?;
    expect_max_outputs("chol", context, 2)?;
    context.check_cancelled()?;
    let triangle = arguments
        .get(1)
        .map_or(Ok(CholeskyTriangle::Upper), |value| {
            match keyword(value).as_deref() {
                Some("upper") => Ok(CholeskyTriangle::Upper),
                Some("lower") => Ok(CholeskyTriangle::Lower),
                _ => Err(option_error("chol", "'upper' or 'lower'")),
            }
        })?;
    let requested_outputs = context.requested_outputs().max(1);
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            let result = context
                .linalg_provider()
                .cholesky_f64(cholesky_request(&input, triangle, context))
                .map_err(|error| linalg_error("chol", error))?;
            chol_outputs(&input, result, requested_outputs, wrap_f64, context)
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let result = context
                .linalg_provider()
                .cholesky_complex64(cholesky_request(&input, triangle, context))
                .map_err(|error| linalg_error("chol", error))?;
            chol_outputs(&input, result, requested_outputs, wrap_complex64, context)
        }
        Value::Array(ArrayData::F32(input)) => {
            let result = context
                .linalg_provider()
                .cholesky_f32(cholesky_request(input, triangle, context))
                .map_err(|error| linalg_error("chol", error))?;
            chol_outputs(input, result, requested_outputs, wrap_f32, context)
        }
        Value::Array(ArrayData::ComplexF32(input)) => {
            let result = context
                .linalg_provider()
                .cholesky_complex32(cholesky_request(input, triangle, context))
                .map_err(|error| linalg_error("chol", error))?;
            chol_outputs(input, result, requested_outputs, wrap_complex32, context)
        }
        Value::Array(ArrayData::F64(input)) => {
            let result = context
                .linalg_provider()
                .cholesky_f64(cholesky_request(input, triangle, context))
                .map_err(|error| linalg_error("chol", error))?;
            chol_outputs(input, result, requested_outputs, wrap_f64, context)
        }
        Value::Array(ArrayData::ComplexF64(input)) => {
            let result = context
                .linalg_provider()
                .cholesky_complex64(cholesky_request(input, triangle, context))
                .map_err(|error| linalg_error("chol", error))?;
            chol_outputs(input, result, requested_outputs, wrap_complex64, context)
        }
        value => Err(type_error(
            "chol",
            1,
            "real or complex double or single square matrix",
            value,
        )),
    }
}

fn cholesky_request<'a, T>(
    input: &'a DenseArray<T>,
    triangle: CholeskyTriangle,
    context: &'a BuiltinContext<'_>,
) -> CholeskyRequest<'a, T> {
    CholeskyRequest::new(input, triangle).with_cancellation_flag(context.cancellation_flag())
}

fn chol_outputs<T, F>(
    input: &DenseArray<T>,
    result: CholeskyResult<T>,
    requested_outputs: usize,
    wrap: F,
    context: &BuiltinContext<'_>,
) -> BuiltinResult
where
    T: FactorElement,
    F: Fn(DenseArray<T>, &BuiltinContext<'_>) -> Result<Value, BuiltinError>,
{
    validate_cholesky_result(input, &result)?;
    let nonreal_diagonal = first_nonreal_diagonal(input, context)?;
    let failure = match (result.first_non_positive_minor, nonreal_diagonal) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    if requested_outputs == 1 {
        if failure.is_some() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`chol` requires a positive-definite matrix with a real diagonal",
            ));
        }
        return Ok(vec![wrap(result.factor, context)?]);
    }
    let (factor, status) = if let Some(failure) = failure {
        (
            leading_square(&result.factor, failure, context)?,
            Value::Double(one_based_index(failure)),
        )
    } else {
        (result.factor, Value::Double(0.0))
    };
    Ok(vec![wrap(factor, context)?, status])
}

fn validate_cholesky_result<T>(
    input: &DenseArray<T>,
    result: &CholeskyResult<T>,
) -> Result<(), BuiltinError> {
    if result.factor.shape() != input.shape() {
        return Err(provider_contract_error("chol", "factor shape"));
    }
    let order = input.shape().extent(0);
    if result
        .first_non_positive_minor
        .is_some_and(|minor| minor >= order)
    {
        return Err(provider_contract_error("chol", "failure-minor index"));
    }
    Ok(())
}

fn first_nonreal_diagonal<T: FactorElement>(
    input: &DenseArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<Option<u64>, BuiltinError> {
    let order = input.shape().extent(0);
    let host_order = checked_host_length("chol", order)?;
    for diagonal in 0..host_order {
        check_cancelled_at(context, diagonal)?;
        if !input.as_slice()[diagonal * host_order + diagonal].diagonal_is_real() {
            return Ok(Some(
                u64::try_from(diagonal).map_err(|_| host_length_error("chol", order))?,
            ));
        }
    }
    Ok(None)
}

fn leading_square<T: FactorElement>(
    factor: &DenseArray<T>,
    order: u64,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let shape = Shape::new([order, order]).map_err(|error| array_error(&error))?;
    let mut values = filled_values("chol", shape.numel(), T::default())?;
    let source_order = checked_host_length("chol", factor.shape().extent(0))?;
    let order = checked_host_length("chol", order)?;
    for column in 0..order {
        check_cancelled_at(context, column)?;
        for row in 0..order {
            values[column * order + row] = factor.as_slice()[column * source_order + row];
        }
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

pub(super) fn det_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("det", arguments, 1)?;
    expect_max_outputs("det", context, 1)?;
    context.check_cancelled()?;
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            let result = context
                .linalg_provider()
                .factor_lu_f64(lu_request(&input, context))
                .map_err(|error| linalg_error("det", error))?;
            determinant_output(&input, result, wrap_f64, context)
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let result = context
                .linalg_provider()
                .factor_lu_complex64(lu_request(&input, context))
                .map_err(|error| linalg_error("det", error))?;
            determinant_output(&input, result, wrap_complex64, context)
        }
        Value::Array(ArrayData::F32(input)) => {
            ensure_square("det", input)?;
            let result = context
                .linalg_provider()
                .factor_lu_f32(lu_request(input, context))
                .map_err(|error| linalg_error("det", error))?;
            determinant_output(input, result, wrap_f32, context)
        }
        Value::Array(ArrayData::ComplexF32(input)) => {
            ensure_square("det", input)?;
            let result = context
                .linalg_provider()
                .factor_lu_complex32(lu_request(input, context))
                .map_err(|error| linalg_error("det", error))?;
            determinant_output(input, result, wrap_complex32, context)
        }
        Value::Array(ArrayData::F64(input)) => {
            ensure_square("det", input)?;
            let result = context
                .linalg_provider()
                .factor_lu_f64(lu_request(input, context))
                .map_err(|error| linalg_error("det", error))?;
            determinant_output(input, result, wrap_f64, context)
        }
        Value::Array(ArrayData::ComplexF64(input)) => {
            ensure_square("det", input)?;
            let result = context
                .linalg_provider()
                .factor_lu_complex64(lu_request(input, context))
                .map_err(|error| linalg_error("det", error))?;
            determinant_output(input, result, wrap_complex64, context)
        }
        value => Err(type_error(
            "det",
            1,
            "real or complex double or single square matrix",
            value,
        )),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn determinant_output<T, F>(
    input: &DenseArray<T>,
    result: LuResult<T>,
    wrap: F,
    context: &BuiltinContext<'_>,
) -> BuiltinResult
where
    T: FactorElement,
    F: Fn(DenseArray<T>, &BuiltinContext<'_>) -> Result<Value, BuiltinError>,
{
    ensure_square("det", input)?;
    validate_lu_result(input, &result)?;
    let order = checked_host_length("det", input.shape().extent(0))?;
    let singular = result.first_zero_pivot.is_some();
    let mut determinant = if singular {
        T::default()
    } else {
        let mut value = T::one();
        for diagonal in 0..order {
            check_cancelled_at(context, diagonal)?;
            value = value.multiply(result.packed_lu.as_slice()[diagonal * order + diagonal]);
        }
        value
    };
    if !singular && result.swap_parity == SwapParity::Odd {
        determinant = determinant.negate();
    }
    let scalar = DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![determinant],
    )
    .map_err(|error| array_error(&error))?;
    Ok(vec![wrap(scalar, context)?])
}

pub(super) fn ensure_square<T>(name: &str, input: &DenseArray<T>) -> Result<(), BuiltinError> {
    if input.shape().ndims() != 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires a two-dimensional matrix"),
        ));
    }
    let rows = input.shape().extent(0);
    let columns = input.shape().extent(1);
    if rows != columns {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires a square matrix, received {rows}x{columns}"),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum PermutationOrientation {
    Rows,
    Columns,
}

fn permutation_matrix(
    permutation: &[u64],
    precision: Precision,
    orientation: PermutationOrientation,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let order = u64::try_from(permutation.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "permutation length does not fit the runtime shape model",
        )
    })?;
    let shape = Shape::new([order, order]).map_err(|error| array_error(&error))?;
    let length = checked_host_length("permutation", shape.numel())?;
    let host_order = permutation.len();
    match precision {
        Precision::Double => {
            let mut values = reserved_values("permutation", length)?;
            values.resize(length, 0.0_f64);
            for (position, original) in permutation.iter().copied().enumerate() {
                check_cancelled_at(context, position)?;
                let original = checked_host_length("permutation", original)?;
                let offset = match orientation {
                    PermutationOrientation::Rows => original * host_order + position,
                    PermutationOrientation::Columns => position * host_order + original,
                };
                values[offset] = 1.0;
            }
            wrap_f64(
                DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?,
                context,
            )
        }
        Precision::Single => {
            let mut values = reserved_values("permutation", length)?;
            values.resize(length, 0.0_f32);
            for (position, original) in permutation.iter().copied().enumerate() {
                check_cancelled_at(context, position)?;
                let original = checked_host_length("permutation", original)?;
                let offset = match orientation {
                    PermutationOrientation::Rows => original * host_order + position,
                    PermutationOrientation::Columns => position * host_order + original,
                };
                values[offset] = 1.0;
            }
            wrap_f32(
                DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?,
                context,
            )
        }
    }
}

fn permutation_vector(
    permutation: &[u64],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let length = u64::try_from(permutation.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "permutation length does not fit the runtime shape model",
        )
    })?;
    let mut values = reserved_values("permutation", permutation.len())?;
    for (index, value) in permutation.iter().enumerate() {
        check_cancelled_at(context, index)?;
        values.push(one_based_index(*value));
    }
    wrap_f64(
        DenseArray::from_vec(
            Shape::new([1, length]).map_err(|error| array_error(&error))?,
            values,
        )
        .map_err(|error| array_error(&error))?,
        context,
    )
}

fn validate_permutation(
    name: &str,
    permutation: &[u64],
    expected: u64,
) -> Result<(), BuiltinError> {
    let expected = checked_host_length(name, expected)?;
    if permutation.len() != expected {
        return Err(provider_contract_error(name, "permutation length"));
    }
    let mut seen = reserved_values(name, expected)?;
    seen.resize(expected, false);
    for value in permutation {
        let value = usize::try_from(*value)
            .ok()
            .filter(|value| *value < expected)
            .ok_or_else(|| provider_contract_error(name, "permutation entry"))?;
        if seen[value] {
            return Err(provider_contract_error(name, "permutation uniqueness"));
        }
        seen[value] = true;
    }
    Ok(())
}

fn is_numeric_zero_scalar(value: &Value) -> bool {
    if let Some(value) = exact_real_integer_scalar(value) {
        return match value {
            IntegerComponent::Signed(value) => value == 0,
            IntegerComponent::Unsigned(value) => value == 0,
        };
    }
    match value {
        Value::Double(value) => *value == 0.0,
        Value::Complex(value) => value.real == 0.0 && value.imaginary == 0.0,
        Value::Array(ArrayData::F32(value)) if value.numel() == 1 => value.as_slice()[0] == 0.0,
        Value::Array(ArrayData::ComplexF32(value)) if value.numel() == 1 => {
            value.as_slice()[0].re == 0.0 && value.as_slice()[0].im == 0.0
        }
        Value::Array(ArrayData::F64(value)) if value.numel() == 1 => value.as_slice()[0] == 0.0,
        Value::Array(ArrayData::ComplexF64(value)) if value.numel() == 1 => {
            value.as_slice()[0].re == 0.0 && value.as_slice()[0].im == 0.0
        }
        _ => false,
    }
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

#[allow(clippy::unnecessary_wraps)]
pub(super) fn wrap_f64(
    array: DenseArray<f64>,
    _context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if array.numel() == 1 {
        Ok(Value::Double(array.as_slice()[0]))
    } else {
        Ok(Value::Array(ArrayData::F64(array)))
    }
}

pub(super) fn wrap_complex64(
    array: DenseArray<ArrayComplex64>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if array.as_slice().iter().all(|value| value.im == 0.0) {
        let shape = array.shape().clone();
        let mut values = reserved_values("complex factor output", array.as_slice().len())?;
        for chunk in array.as_slice().chunks(CANCELLATION_CHECK_INTERVAL) {
            context.check_cancelled()?;
            values.extend(chunk.iter().map(|value| value.re));
        }
        return wrap_f64(
            DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?,
            context,
        );
    }
    if array.numel() == 1 {
        let value = array.as_slice()[0];
        Ok(Value::Complex(ValueComplex64::new(value.re, value.im)))
    } else {
        Ok(Value::Array(ArrayData::ComplexF64(array)))
    }
}

#[allow(clippy::unnecessary_wraps)]
pub(super) fn wrap_f32(
    array: DenseArray<f32>,
    _context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    Ok(Value::Array(ArrayData::F32(array)))
}

pub(super) fn wrap_complex32(
    array: DenseArray<ArrayComplex32>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if array.as_slice().iter().all(|value| value.im == 0.0) {
        let shape = array.shape().clone();
        let mut values = reserved_values("complex factor output", array.as_slice().len())?;
        for chunk in array.as_slice().chunks(CANCELLATION_CHECK_INTERVAL) {
            context.check_cancelled()?;
            values.extend(chunk.iter().map(|value| value.re));
        }
        return wrap_f32(
            DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?,
            context,
        );
    }
    Ok(Value::Array(ArrayData::ComplexF32(array)))
}

pub(super) fn scalar_dense<T>(value: T) -> Result<DenseArray<T>, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map_err(|error| array_error(&error))
}

pub(super) fn filled_values<T: Clone>(
    name: &str,
    length: u64,
    value: T,
) -> Result<Vec<T>, BuiltinError> {
    let length = checked_host_length(name, length)?;
    let mut output = reserved_values(name, length)?;
    output.resize(length, value);
    Ok(output)
}

pub(super) fn reserved_values<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {length} elements"),
        )
    })?;
    Ok(output)
}

pub(super) fn checked_host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| host_length_error(name, length))
}

#[allow(clippy::cast_precision_loss)]
fn one_based_index(zero_based: u64) -> f64 {
    zero_based.saturating_add(1) as f64
}

pub(super) fn check_cancelled_at(
    context: &BuiltinContext<'_>,
    index: usize,
) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

pub(super) fn option_error(name: &str, expected: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("options to `{name}` must be {expected}"),
    )
}

pub(super) fn provider_contract_error(name: &str, detail: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Other,
        format!("the selected numerical provider returned invalid {detail} for `{name}`"),
    )
}

fn host_length_error(name: &str, length: u64) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` array length {length} does not fit this host"),
    )
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn linalg_error(name: &str, error: openmat_linalg::LinalgError) -> BuiltinError {
    let category = match error {
        openmat_linalg::LinalgError::Cancelled { .. } => BuiltinErrorCategory::Cancelled,
        openmat_linalg::LinalgError::MatrixRequired { .. }
        | openmat_linalg::LinalgError::SquareMatrixRequired { .. }
        | openmat_linalg::LinalgError::DimensionMismatch { .. }
        | openmat_linalg::LinalgError::OutputShapeMismatch { .. }
        | openmat_linalg::LinalgError::Lp64DimensionOverflow { .. }
        | openmat_linalg::LinalgError::AllocationFailure { .. }
        | openmat_linalg::LinalgError::Array(_) => BuiltinErrorCategory::Domain,
        openmat_linalg::LinalgError::SingularMatrix { .. }
        | openmat_linalg::LinalgError::RankDeficient { .. }
        | openmat_linalg::LinalgError::NoConvergence { .. }
        | openmat_linalg::LinalgError::InvalidProviderArgument { .. }
        | openmat_linalg::LinalgError::ProviderFailure { .. } => BuiltinErrorCategory::Other,
    };
    BuiltinError::new(
        category,
        format!("the selected numerical provider could not complete `{name}`: {error}"),
    )
}

use std::cmp::Ordering;

use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerComponent, Shape,
};
use openmat_linalg::{
    EigRequest, EigResult, LinalgError, SolveRequest, SvdRequest, SvdResult, SvdVectors,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::core_linalg::{
    check_cancelled_at, checked_host_length, ensure_square, filled_values, keyword, linalg_error,
    option_error, provider_contract_error, scalar_dense, wrap_complex32, wrap_complex64, wrap_f32,
    wrap_f64,
};
use crate::{
    array_error, exact_real_integer_scalar, expect_argument_count_range, expect_max_outputs,
    type_error,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SvdShape {
    Full,
    Thin,
    LegacyZero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SvdValueOutput {
    Matrix,
    Vector,
}

#[derive(Clone, Copy, Debug)]
struct SvdOptions {
    shape: SvdShape,
    values: SvdValueOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EigValueOutput {
    Matrix,
    Vector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConditionNorm {
    One,
    Two,
    Infinity,
    Frobenius,
}

trait SpectralElement: Copy + Default {
    fn one() -> Self;
    fn conjugate(self) -> Self;
    fn is_finite(self) -> bool;
}

trait PseudoinverseElement<R>: SpectralElement {
    fn add(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn scale(self, factor: R) -> Self;
}

impl PseudoinverseElement<f64> for f64 {
    fn add(self, right: Self) -> Self {
        self + right
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn scale(self, factor: f64) -> Self {
        self * factor
    }
}

impl PseudoinverseElement<f32> for f32 {
    fn add(self, right: Self) -> Self {
        self + right
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn scale(self, factor: f32) -> Self {
        self * factor
    }
}

impl PseudoinverseElement<f64> for ArrayComplex64 {
    fn add(self, right: Self) -> Self {
        Self::new(self.re + right.re, self.im + right.im)
    }

    fn multiply(self, right: Self) -> Self {
        Self::new(
            self.re * right.re - self.im * right.im,
            self.re * right.im + self.im * right.re,
        )
    }

    fn scale(self, factor: f64) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }
}

impl PseudoinverseElement<f32> for ArrayComplex32 {
    fn add(self, right: Self) -> Self {
        Self::new(self.re + right.re, self.im + right.im)
    }

    fn multiply(self, right: Self) -> Self {
        Self::new(
            self.re * right.re - self.im * right.im,
            self.re * right.im + self.im * right.re,
        )
    }

    fn scale(self, factor: f32) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }
}

impl SpectralElement for f64 {
    fn one() -> Self {
        1.0
    }

    fn conjugate(self) -> Self {
        self
    }

    fn is_finite(self) -> bool {
        self.is_finite()
    }
}

impl SpectralElement for f32 {
    fn one() -> Self {
        1.0
    }

    fn conjugate(self) -> Self {
        self
    }

    fn is_finite(self) -> bool {
        self.is_finite()
    }
}

impl SpectralElement for ArrayComplex64 {
    fn one() -> Self {
        Self::new(1.0, 0.0)
    }

    fn conjugate(self) -> Self {
        self.conjugate()
    }

    fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
}

impl SpectralElement for ArrayComplex32 {
    fn one() -> Self {
        Self::new(1.0, 0.0)
    }

    fn conjugate(self) -> Self {
        self.conjugate()
    }

    fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
}

trait RealElement: Copy + Default + PartialEq + PartialOrd {
    fn one() -> Self;
    fn from_u64(value: u64) -> Self;
    fn maximum(self, right: Self) -> Self;
    fn infinity() -> Self;
    fn add(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
    fn hypot(self, right: Self) -> Self;
}

impl RealElement for f64 {
    fn one() -> Self {
        1.0
    }

    #[allow(clippy::cast_precision_loss)]
    fn from_u64(value: u64) -> Self {
        value as Self
    }

    fn maximum(self, right: Self) -> Self {
        self.max(right)
    }

    fn infinity() -> Self {
        Self::INFINITY
    }

    fn add(self, right: Self) -> Self {
        self + right
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn hypot(self, right: Self) -> Self {
        self.hypot(right)
    }
}

impl RealElement for f32 {
    fn one() -> Self {
        1.0
    }

    #[allow(clippy::cast_precision_loss)]
    fn from_u64(value: u64) -> Self {
        value as Self
    }

    fn maximum(self, right: Self) -> Self {
        self.max(right)
    }

    fn infinity() -> Self {
        Self::INFINITY
    }

    fn add(self, right: Self) -> Self {
        self + right
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn hypot(self, right: Self) -> Self {
        self.hypot(right)
    }
}

trait NormElement: SpectralElement {
    type Real: RealElement;
    fn magnitude(self) -> Self::Real;
}

impl NormElement for f64 {
    type Real = f64;

    fn magnitude(self) -> Self::Real {
        self.abs()
    }
}

impl NormElement for f32 {
    type Real = f32;

    fn magnitude(self) -> Self::Real {
        self.abs()
    }
}

impl NormElement for ArrayComplex64 {
    type Real = f64;

    fn magnitude(self) -> Self::Real {
        self.re.hypot(self.im)
    }
}

impl NormElement for ArrayComplex32 {
    type Real = f32;

    fn magnitude(self) -> Self::Real {
        self.re.hypot(self.im)
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn svd_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("svd", arguments, 1, 3)?;
    expect_max_outputs("svd", context, 3)?;
    context.check_cancelled()?;
    let options = svd_options(arguments)?;
    let requested_outputs = context.requested_outputs().max(1);
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            let vectors = svd_vectors(&input, options.shape, requested_outputs);
            let result = context
                .linalg_provider()
                .svd_f64(svd_request(&input, vectors, context))
                .map_err(|error| linalg_error("svd", error))?;
            svd_outputs(
                &input,
                result,
                vectors,
                options.values,
                requested_outputs,
                wrap_f64,
                wrap_f64,
                context,
            )
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let vectors = svd_vectors(&input, options.shape, requested_outputs);
            let result = context
                .linalg_provider()
                .svd_complex64(svd_request(&input, vectors, context))
                .map_err(|error| linalg_error("svd", error))?;
            svd_outputs(
                &input,
                result,
                vectors,
                options.values,
                requested_outputs,
                wrap_complex64,
                wrap_f64,
                context,
            )
        }
        Value::Array(ArrayData::F32(input)) => {
            let vectors = svd_vectors(input, options.shape, requested_outputs);
            let result = context
                .linalg_provider()
                .svd_f32(svd_request(input, vectors, context))
                .map_err(|error| linalg_error("svd", error))?;
            svd_outputs(
                input,
                result,
                vectors,
                options.values,
                requested_outputs,
                wrap_f32,
                wrap_f32,
                context,
            )
        }
        Value::Array(ArrayData::ComplexF32(input)) => {
            let vectors = svd_vectors(input, options.shape, requested_outputs);
            let result = context
                .linalg_provider()
                .svd_complex32(svd_request(input, vectors, context))
                .map_err(|error| linalg_error("svd", error))?;
            svd_outputs(
                input,
                result,
                vectors,
                options.values,
                requested_outputs,
                wrap_complex32,
                wrap_f32,
                context,
            )
        }
        Value::Array(ArrayData::F64(input)) => {
            let vectors = svd_vectors(input, options.shape, requested_outputs);
            let result = context
                .linalg_provider()
                .svd_f64(svd_request(input, vectors, context))
                .map_err(|error| linalg_error("svd", error))?;
            svd_outputs(
                input,
                result,
                vectors,
                options.values,
                requested_outputs,
                wrap_f64,
                wrap_f64,
                context,
            )
        }
        Value::Array(ArrayData::ComplexF64(input)) => {
            let vectors = svd_vectors(input, options.shape, requested_outputs);
            let result = context
                .linalg_provider()
                .svd_complex64(svd_request(input, vectors, context))
                .map_err(|error| linalg_error("svd", error))?;
            svd_outputs(
                input,
                result,
                vectors,
                options.values,
                requested_outputs,
                wrap_complex64,
                wrap_f64,
                context,
            )
        }
        value => Err(type_error(
            "svd",
            1,
            "real or complex double or single matrix",
            value,
        )),
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn pinv_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("pinv", arguments, 1, 2)?;
    expect_max_outputs("pinv", context, 1)?;
    context.check_cancelled()?;
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            pinv_f64(&input, arguments.get(1), context)
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            pinv_complex64(&input, arguments.get(1), context)
        }
        Value::Array(ArrayData::F32(input)) => pinv_f32(input, arguments.get(1), context),
        Value::Array(ArrayData::ComplexF32(input)) => {
            pinv_complex32(input, arguments.get(1), context)
        }
        Value::Array(ArrayData::F64(input)) => pinv_f64(input, arguments.get(1), context),
        Value::Array(ArrayData::ComplexF64(input)) => {
            pinv_complex64(input, arguments.get(1), context)
        }
        value => Err(type_error(
            "pinv",
            1,
            "real or complex double or single matrix",
            value,
        )),
    }
}

fn pinv_f64(
    input: &DenseArray<f64>,
    tolerance: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let tolerance = tolerance
        .map(|value| tolerance_f64_for("pinv", value))
        .transpose()?;
    let result = context
        .linalg_provider()
        .svd_f64(svd_request(input, SvdVectors::Thin, context))
        .map_err(|error| linalg_error("pinv", error))?;
    Ok(vec![wrap_f64(
        pseudoinverse(input, result, tolerance, eps_at_f64, context)?,
        context,
    )?])
}

fn pinv_complex64(
    input: &DenseArray<ArrayComplex64>,
    tolerance: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let tolerance = tolerance
        .map(|value| tolerance_f64_for("pinv", value))
        .transpose()?;
    let result = context
        .linalg_provider()
        .svd_complex64(svd_request(input, SvdVectors::Thin, context))
        .map_err(|error| linalg_error("pinv", error))?;
    Ok(vec![wrap_complex64(
        pseudoinverse(input, result, tolerance, eps_at_f64, context)?,
        context,
    )?])
}

fn pinv_f32(
    input: &DenseArray<f32>,
    tolerance: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let tolerance = tolerance
        .map(|value| tolerance_f32_for("pinv", value))
        .transpose()?;
    let result = context
        .linalg_provider()
        .svd_f32(svd_request(input, SvdVectors::Thin, context))
        .map_err(|error| linalg_error("pinv", error))?;
    Ok(vec![wrap_f32(
        pseudoinverse(input, result, tolerance, eps_at_f32, context)?,
        context,
    )?])
}

fn pinv_complex32(
    input: &DenseArray<ArrayComplex32>,
    tolerance: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    let tolerance = tolerance
        .map(|value| tolerance_f32_for("pinv", value))
        .transpose()?;
    let result = context
        .linalg_provider()
        .svd_complex32(svd_request(input, SvdVectors::Thin, context))
        .map_err(|error| linalg_error("pinv", error))?;
    Ok(vec![wrap_complex32(
        pseudoinverse(input, result, tolerance, eps_at_f32, context)?,
        context,
    )?])
}

fn pseudoinverse<T, R, E>(
    input: &DenseArray<T>,
    result: SvdResult<T, R>,
    explicit_tolerance: Option<R>,
    epsilon: E,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError>
where
    T: PseudoinverseElement<R>,
    R: Copy + Default + PartialOrd + RealElement,
    E: Fn(R) -> R,
{
    validate_svd_result(input, &result, SvdVectors::Thin)?;
    let rows = checked_host_length("pinv rows", input.shape().extent(0))?;
    let columns = checked_host_length("pinv columns", input.shape().extent(1))?;
    let order = rows.min(columns);
    let u = result
        .u
        .ok_or_else(|| provider_contract_error("pinv", "missing U factor"))?;
    let vh = result
        .vh
        .ok_or_else(|| provider_contract_error("pinv", "missing Vh factor"))?;
    let maximum = result
        .singular_values
        .as_slice()
        .iter()
        .copied()
        .fold(R::default(), RealElement::maximum);
    #[allow(clippy::cast_precision_loss)]
    let tolerance = explicit_tolerance.unwrap_or_else(|| {
        R::from_u64(input.shape().extent(0).max(input.shape().extent(1))).multiply(epsilon(maximum))
    });
    let shape = Shape::new([input.shape().extent(1), input.shape().extent(0)])
        .map_err(|error| array_error(&error))?;
    let mut values = filled_values("pinv", shape.numel(), T::default())?;
    for output_column in 0..rows {
        context.check_cancelled()?;
        for output_row in 0..columns {
            let mut value = T::default();
            for index in 0..order {
                let singular = result.singular_values.as_slice()[index];
                if singular > tolerance {
                    let v = vh.as_slice()[output_row * order + index].conjugate();
                    let uh = u.as_slice()[index * rows + output_column].conjugate();
                    value = value.add(v.multiply(uh).scale(R::one().divide(singular)));
                }
            }
            values[output_column * columns + output_row] = value;
        }
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn svd_options(arguments: &[Value]) -> Result<SvdOptions, BuiltinError> {
    let mut shape = SvdShape::Full;
    let mut values = SvdValueOutput::Matrix;
    let mut shape_seen = false;
    let mut values_seen = false;
    for (option_index, option) in arguments.iter().skip(1).enumerate() {
        if option_index == 0 && is_real_zero_scalar(option) {
            if shape_seen {
                return Err(option_error(
                    "svd",
                    "one economy/full selector and one 'matrix'/'vector' selector",
                ));
            }
            shape = SvdShape::LegacyZero;
            shape_seen = true;
            continue;
        }
        match keyword(option).as_deref() {
            Some("econ") if !shape_seen => {
                shape = SvdShape::Thin;
                shape_seen = true;
            }
            Some("matrix") if !values_seen => {
                values = SvdValueOutput::Matrix;
                values_seen = true;
            }
            Some("vector") if !values_seen => {
                values = SvdValueOutput::Vector;
                values_seen = true;
            }
            _ => {
                return Err(option_error(
                    "svd",
                    "numeric zero or non-repeated 'econ', 'matrix', and 'vector' selectors",
                ));
            }
        }
    }
    Ok(SvdOptions { shape, values })
}

fn is_real_zero_scalar(value: &Value) -> bool {
    if let Some(value) = exact_real_integer_scalar(value) {
        return match value {
            IntegerComponent::Signed(value) => value == 0,
            IntegerComponent::Unsigned(value) => value == 0,
        };
    }
    match value {
        Value::Double(value) => *value == 0.0,
        Value::Array(ArrayData::F32(value)) if value.numel() == 1 => value.as_slice()[0] == 0.0,
        Value::Array(ArrayData::F64(value)) if value.numel() == 1 => value.as_slice()[0] == 0.0,
        _ => false,
    }
}

fn svd_vectors<T>(input: &DenseArray<T>, shape: SvdShape, requested_outputs: usize) -> SvdVectors {
    if requested_outputs == 1 {
        return SvdVectors::None;
    }
    match shape {
        SvdShape::Full => SvdVectors::Full,
        SvdShape::Thin => SvdVectors::Thin,
        SvdShape::LegacyZero => {
            if input.shape().extent(0) > input.shape().extent(1) {
                SvdVectors::Thin
            } else {
                SvdVectors::Full
            }
        }
    }
}

fn svd_request<'a, T>(
    input: &'a DenseArray<T>,
    vectors: SvdVectors,
    context: &'a BuiltinContext<'_>,
) -> SvdRequest<'a, T> {
    SvdRequest::new(input, vectors).with_cancellation_flag(context.cancellation_flag())
}

#[allow(clippy::too_many_arguments)]
fn svd_outputs<T, R, FT, FR>(
    input: &DenseArray<T>,
    result: SvdResult<T, R>,
    vectors: SvdVectors,
    value_output: SvdValueOutput,
    requested_outputs: usize,
    wrap_factor: FT,
    wrap_real: FR,
    context: &BuiltinContext<'_>,
) -> BuiltinResult
where
    T: SpectralElement,
    R: Copy + Default,
    FT: Fn(DenseArray<T>, &BuiltinContext<'_>) -> Result<Value, BuiltinError>,
    FR: Fn(DenseArray<R>, &BuiltinContext<'_>) -> Result<Value, BuiltinError>,
{
    validate_svd_result(input, &result, vectors)?;
    if requested_outputs == 1 {
        return Ok(vec![wrap_real(result.singular_values, context)?]);
    }
    let rows = input.shape().extent(0);
    let columns = input.shape().extent(1);
    let order = rows.min(columns);
    let u = result
        .u
        .ok_or_else(|| provider_contract_error("svd", "missing U factor"))?;
    let vh = result
        .vh
        .ok_or_else(|| provider_contract_error("svd", "missing Vh factor"))?;
    let singular = match value_output {
        SvdValueOutput::Vector => result.singular_values,
        SvdValueOutput::Matrix => {
            let (s_rows, s_columns) = match vectors {
                SvdVectors::Thin => (order, order),
                SvdVectors::Full => (rows, columns),
                SvdVectors::None => unreachable!("multiple SVD outputs request vectors"),
            };
            diagonal_singular_values(&result.singular_values, s_rows, s_columns, context)?
        }
    };
    let v = conjugate_transpose(&vh, context)?;
    if requested_outputs == 2 {
        return Ok(vec![
            wrap_factor(u, context)?,
            wrap_real(singular, context)?,
        ]);
    }
    Ok(vec![
        wrap_factor(u, context)?,
        wrap_real(singular, context)?,
        wrap_factor(v, context)?,
    ])
}

fn validate_svd_result<T, R>(
    input: &DenseArray<T>,
    result: &SvdResult<T, R>,
    vectors: SvdVectors,
) -> Result<(), BuiltinError> {
    let rows = input.shape().extent(0);
    let columns = input.shape().extent(1);
    let order = rows.min(columns);
    if result.singular_values.shape().dimensions() != [order, 1] {
        return Err(provider_contract_error("svd", "singular-value shape"));
    }
    let expected = match vectors {
        SvdVectors::None => None,
        SvdVectors::Thin => Some(([rows, order], [order, columns])),
        SvdVectors::Full => Some(([rows, rows], [columns, columns])),
    };
    match (expected, &result.u, &result.vh) {
        (None, None, None) => Ok(()),
        (Some((u_shape, vh_shape)), Some(u), Some(vh))
            if u.shape().dimensions() == u_shape && vh.shape().dimensions() == vh_shape =>
        {
            Ok(())
        }
        _ => Err(provider_contract_error("svd", "singular-vector shapes")),
    }
}

fn diagonal_singular_values<R: Copy + Default>(
    singular_values: &DenseArray<R>,
    rows: u64,
    columns: u64,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<R>, BuiltinError> {
    let shape = Shape::new([rows, columns]).map_err(|error| array_error(&error))?;
    let mut values = filled_values("svd singular-value matrix", shape.numel(), R::default())?;
    let host_rows = checked_host_length("svd singular-value rows", rows)?;
    for (diagonal, value) in singular_values.as_slice().iter().copied().enumerate() {
        check_cancelled_at(context, diagonal)?;
        values[diagonal * host_rows + diagonal] = value;
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn conjugate_transpose<T: SpectralElement>(
    input: &DenseArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let rows = checked_host_length("conjugate-transpose rows", input.shape().extent(0))?;
    let columns = checked_host_length("conjugate-transpose columns", input.shape().extent(1))?;
    let mut values = filled_values("conjugate-transpose output", input.numel(), T::default())?;
    for column in 0..columns {
        for row in 0..rows {
            let input_offset = column * rows + row;
            check_cancelled_at(context, input_offset)?;
            values[row * columns + column] = input.as_slice()[input_offset].conjugate();
        }
    }
    DenseArray::from_vec(
        Shape::new([input.shape().extent(1), input.shape().extent(0)])
            .map_err(|error| array_error(&error))?,
        values,
    )
    .map_err(|error| array_error(&error))
}

#[allow(clippy::too_many_lines)]
pub(super) fn eig_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("eig", arguments, 1, 2)?;
    expect_max_outputs("eig", context, 3)?;
    context.check_cancelled()?;
    let value_output =
        arguments
            .get(1)
            .map_or(Ok(EigValueOutput::Matrix), |value| {
                match keyword(value).as_deref() {
                    Some("matrix") => Ok(EigValueOutput::Matrix),
                    Some("vector") => Ok(EigValueOutput::Vector),
                    _ => Err(option_error(
                        "eig",
                        "'matrix' or 'vector' for the ordinary single-matrix form",
                    )),
                }
            })?;
    let requested_outputs = context.requested_outputs().max(1);
    let right_vectors = requested_outputs >= 2;
    let left_vectors = requested_outputs >= 3;
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            let result = context
                .linalg_provider()
                .eig_f64(eig_request(&input, left_vectors, right_vectors, context))
                .map_err(|error| linalg_error("eig", error))?;
            eig_outputs(
                &input,
                result,
                requested_outputs,
                value_output,
                wrap_complex64,
                context,
            )
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let result = context
                .linalg_provider()
                .eig_complex64(eig_request(&input, left_vectors, right_vectors, context))
                .map_err(|error| linalg_error("eig", error))?;
            eig_outputs(
                &input,
                result,
                requested_outputs,
                value_output,
                wrap_complex64,
                context,
            )
        }
        Value::Array(ArrayData::F32(input)) => {
            let result = context
                .linalg_provider()
                .eig_f32(eig_request(input, left_vectors, right_vectors, context))
                .map_err(|error| linalg_error("eig", error))?;
            eig_outputs(
                input,
                result,
                requested_outputs,
                value_output,
                wrap_complex32,
                context,
            )
        }
        Value::Array(ArrayData::ComplexF32(input)) => {
            let result = context
                .linalg_provider()
                .eig_complex32(eig_request(input, left_vectors, right_vectors, context))
                .map_err(|error| linalg_error("eig", error))?;
            eig_outputs(
                input,
                result,
                requested_outputs,
                value_output,
                wrap_complex32,
                context,
            )
        }
        Value::Array(ArrayData::F64(input)) => {
            let result = context
                .linalg_provider()
                .eig_f64(eig_request(input, left_vectors, right_vectors, context))
                .map_err(|error| linalg_error("eig", error))?;
            eig_outputs(
                input,
                result,
                requested_outputs,
                value_output,
                wrap_complex64,
                context,
            )
        }
        Value::Array(ArrayData::ComplexF64(input)) => {
            let result = context
                .linalg_provider()
                .eig_complex64(eig_request(input, left_vectors, right_vectors, context))
                .map_err(|error| linalg_error("eig", error))?;
            eig_outputs(
                input,
                result,
                requested_outputs,
                value_output,
                wrap_complex64,
                context,
            )
        }
        value => Err(type_error(
            "eig",
            1,
            "real or complex double or single square matrix",
            value,
        )),
    }
}

fn eig_request<'a, T>(
    input: &'a DenseArray<T>,
    left_vectors: bool,
    right_vectors: bool,
    context: &'a BuiltinContext<'_>,
) -> EigRequest<'a, T> {
    EigRequest::new(input)
        .with_left_vectors(left_vectors)
        .with_right_vectors(right_vectors)
        .with_cancellation_flag(context.cancellation_flag())
}

fn eig_outputs<T, C, F>(
    input: &DenseArray<T>,
    result: EigResult<C>,
    requested_outputs: usize,
    value_output: EigValueOutput,
    wrap: F,
    context: &BuiltinContext<'_>,
) -> BuiltinResult
where
    C: SpectralElement,
    F: Fn(DenseArray<C>, &BuiltinContext<'_>) -> Result<Value, BuiltinError>,
{
    validate_eig_result(input, &result, requested_outputs)?;
    if requested_outputs == 1 {
        return Ok(vec![wrap(result.eigenvalues, context)?]);
    }
    let right = result
        .right_vectors
        .ok_or_else(|| provider_contract_error("eig", "missing right eigenvectors"))?;
    let values = match value_output {
        EigValueOutput::Vector => result.eigenvalues,
        EigValueOutput::Matrix => diagonal_eigenvalues(&result.eigenvalues, context)?,
    };
    if requested_outputs == 2 {
        return Ok(vec![wrap(right, context)?, wrap(values, context)?]);
    }
    let left = result
        .left_vectors
        .ok_or_else(|| provider_contract_error("eig", "missing left eigenvectors"))?;
    Ok(vec![
        wrap(right, context)?,
        wrap(values, context)?,
        wrap(left, context)?,
    ])
}

fn validate_eig_result<T, C>(
    input: &DenseArray<T>,
    result: &EigResult<C>,
    requested_outputs: usize,
) -> Result<(), BuiltinError> {
    let order = input.shape().extent(0);
    if result.eigenvalues.shape().dimensions() != [order, 1] {
        return Err(provider_contract_error("eig", "eigenvalue shape"));
    }
    let square = [order, order];
    let right_valid = if requested_outputs >= 2 {
        result
            .right_vectors
            .as_ref()
            .is_some_and(|value| value.shape().dimensions() == square)
    } else {
        result.right_vectors.is_none()
    };
    let left_valid = if requested_outputs >= 3 {
        result
            .left_vectors
            .as_ref()
            .is_some_and(|value| value.shape().dimensions() == square)
    } else {
        result.left_vectors.is_none()
    };
    if right_valid && left_valid {
        Ok(())
    } else {
        Err(provider_contract_error("eig", "eigenvector shapes"))
    }
}

fn diagonal_eigenvalues<C: SpectralElement>(
    eigenvalues: &DenseArray<C>,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<C>, BuiltinError> {
    let order = eigenvalues.shape().extent(0);
    let shape = Shape::new([order, order]).map_err(|error| array_error(&error))?;
    let mut values = filled_values("eig diagonal matrix", shape.numel(), C::default())?;
    let host_order = checked_host_length("eig order", order)?;
    for (diagonal, value) in eigenvalues.as_slice().iter().copied().enumerate() {
        check_cancelled_at(context, diagonal)?;
        values[diagonal * host_order + diagonal] = value;
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

pub(super) fn rank_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("rank", arguments, 1, 2)?;
    expect_max_outputs("rank", context, 1)?;
    context.check_cancelled()?;
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            rank_f64(&input, arguments.get(1), context)
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            rank_complex64(&input, arguments.get(1), context)
        }
        Value::Array(ArrayData::F32(input)) => rank_f32(input, arguments.get(1), context),
        Value::Array(ArrayData::ComplexF32(input)) => {
            rank_complex32(input, arguments.get(1), context)
        }
        Value::Array(ArrayData::F64(input)) => rank_f64(input, arguments.get(1), context),
        Value::Array(ArrayData::ComplexF64(input)) => {
            rank_complex64(input, arguments.get(1), context)
        }
        value => Err(type_error(
            "rank",
            1,
            "real or complex double or single matrix",
            value,
        )),
    }
}

fn rank_f64(
    input: &DenseArray<f64>,
    tolerance: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    ensure_rank_finite(input, context)?;
    let explicit = tolerance.map(tolerance_f64).transpose()?;
    let result = context
        .linalg_provider()
        .svd_f64(svd_request(input, SvdVectors::None, context))
        .map_err(|error| linalg_error("rank", error))?;
    validate_svd_result(input, &result, SvdVectors::None)?;
    Ok(vec![Value::Double(rank_from_f64(
        input,
        &result.singular_values,
        explicit,
        context,
    )?)])
}

fn rank_complex64(
    input: &DenseArray<ArrayComplex64>,
    tolerance: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    ensure_rank_finite(input, context)?;
    let explicit = tolerance.map(tolerance_f64).transpose()?;
    let result = context
        .linalg_provider()
        .svd_complex64(svd_request(input, SvdVectors::None, context))
        .map_err(|error| linalg_error("rank", error))?;
    validate_svd_result(input, &result, SvdVectors::None)?;
    Ok(vec![Value::Double(rank_from_f64(
        input,
        &result.singular_values,
        explicit,
        context,
    )?)])
}

fn rank_f32(
    input: &DenseArray<f32>,
    tolerance: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    ensure_rank_finite(input, context)?;
    let explicit = tolerance.map(tolerance_f32).transpose()?;
    let result = context
        .linalg_provider()
        .svd_f32(svd_request(input, SvdVectors::None, context))
        .map_err(|error| linalg_error("rank", error))?;
    validate_svd_result(input, &result, SvdVectors::None)?;
    Ok(vec![Value::Double(rank_from_f32(
        input,
        &result.singular_values,
        explicit,
        context,
    )?)])
}

fn rank_complex32(
    input: &DenseArray<ArrayComplex32>,
    tolerance: Option<&Value>,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    ensure_rank_finite(input, context)?;
    let explicit = tolerance.map(tolerance_f32).transpose()?;
    let result = context
        .linalg_provider()
        .svd_complex32(svd_request(input, SvdVectors::None, context))
        .map_err(|error| linalg_error("rank", error))?;
    validate_svd_result(input, &result, SvdVectors::None)?;
    Ok(vec![Value::Double(rank_from_f32(
        input,
        &result.singular_values,
        explicit,
        context,
    )?)])
}

fn ensure_rank_finite<T: SpectralElement>(
    input: &DenseArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    for (index, value) in input.as_slice().iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        if !value.is_finite() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`rank` requires a matrix whose elements are finite",
            ));
        }
    }
    Ok(())
}

fn rank_from_f64<T>(
    input: &DenseArray<T>,
    singular_values: &DenseArray<f64>,
    explicit: Option<f64>,
    context: &BuiltinContext<'_>,
) -> Result<f64, BuiltinError> {
    let maximum = singular_values
        .as_slice()
        .iter()
        .copied()
        .fold(0.0_f64, f64::max);
    #[allow(clippy::cast_precision_loss)]
    let tolerance = explicit.unwrap_or_else(|| {
        (input.shape().extent(0).max(input.shape().extent(1)) as f64) * eps_at_f64(maximum)
    });
    let mut rank = 0_u64;
    for (index, singular) in singular_values.as_slice().iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        if singular > tolerance {
            rank = rank.saturating_add(1);
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let rank = rank as f64;
    Ok(rank)
}

fn rank_from_f32<T>(
    input: &DenseArray<T>,
    singular_values: &DenseArray<f32>,
    explicit: Option<f32>,
    context: &BuiltinContext<'_>,
) -> Result<f64, BuiltinError> {
    let maximum = singular_values
        .as_slice()
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);
    #[allow(clippy::cast_precision_loss)]
    let tolerance = explicit.unwrap_or_else(|| {
        (input.shape().extent(0).max(input.shape().extent(1)) as f32) * eps_at_f32(maximum)
    });
    let mut rank = 0_u64;
    for (index, singular) in singular_values.as_slice().iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        if singular > tolerance {
            rank = rank.saturating_add(1);
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let rank = rank as f64;
    Ok(rank)
}

fn eps_at_f64(value: f64) -> f64 {
    let value = value.abs();
    if value.is_nan() || value.is_infinite() {
        return f64::NAN;
    }
    if value == f64::MAX {
        return value - f64::from_bits(value.to_bits() - 1);
    }
    f64::from_bits(value.to_bits() + 1) - value
}

fn eps_at_f32(value: f32) -> f32 {
    let value = value.abs();
    if value.is_nan() || value.is_infinite() {
        return f32::NAN;
    }
    if value == f32::MAX {
        return value - f32::from_bits(value.to_bits() - 1);
    }
    f32::from_bits(value.to_bits() + 1) - value
}

fn tolerance_f64(value: &Value) -> Result<f64, BuiltinError> {
    tolerance_f64_for("rank", value)
}

fn tolerance_f64_for(name: &str, value: &Value) -> Result<f64, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        #[allow(clippy::cast_precision_loss)]
        return Ok(match value {
            IntegerComponent::Signed(value) => value as f64,
            IntegerComponent::Unsigned(value) => value as f64,
        });
    }
    match value {
        Value::Double(value) => Ok(*value),
        Value::Array(ArrayData::F64(value)) if value.numel() == 1 => Ok(value.as_slice()[0]),
        Value::Array(ArrayData::F32(value)) if value.numel() == 1 => {
            Ok(f64::from(value.as_slice()[0]))
        }
        Value::Array(ArrayData::F64(_) | ArrayData::F32(_)) => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input 2 to `{name}` must be a real numeric scalar tolerance"),
        )),
        _ => Err(type_error(
            name,
            2,
            "real double, single, or integer scalar tolerance",
            value,
        )),
    }
}

fn tolerance_f32(value: &Value) -> Result<f32, BuiltinError> {
    tolerance_f32_for("rank", value)
}

fn tolerance_f32_for(name: &str, value: &Value) -> Result<f32, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        #[allow(clippy::cast_precision_loss)]
        return Ok(match value {
            IntegerComponent::Signed(value) => value as f32,
            IntegerComponent::Unsigned(value) => value as f32,
        });
    }
    match value {
        #[allow(clippy::cast_possible_truncation)]
        Value::Double(value) => Ok(*value as f32),
        #[allow(clippy::cast_possible_truncation)]
        Value::Array(ArrayData::F64(value)) if value.numel() == 1 => Ok(value.as_slice()[0] as f32),
        Value::Array(ArrayData::F32(value)) if value.numel() == 1 => Ok(value.as_slice()[0]),
        Value::Array(ArrayData::F64(_) | ArrayData::F32(_)) => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input 2 to `{name}` must be a real numeric scalar tolerance"),
        )),
        _ => Err(type_error(
            name,
            2,
            "real double, single, or integer scalar tolerance",
            value,
        )),
    }
}

pub(super) fn cond_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("cond", arguments, 1, 2)?;
    expect_max_outputs("cond", context, 1)?;
    context.check_cancelled()?;
    let norm = arguments
        .get(1)
        .map_or(Ok(ConditionNorm::Two), condition_norm)?;
    match &arguments[0] {
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            Ok(vec![Value::Double(cond_f64(&input, norm, context)?)])
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            Ok(vec![Value::Double(cond_complex64(&input, norm, context)?)])
        }
        Value::Array(ArrayData::F32(input)) => Ok(vec![wrap_f32(
            scalar_dense(cond_f32(input, norm, context)?)?,
            context,
        )?]),
        Value::Array(ArrayData::ComplexF32(input)) => Ok(vec![wrap_f32(
            scalar_dense(cond_complex32(input, norm, context)?)?,
            context,
        )?]),
        Value::Array(ArrayData::F64(input)) => {
            Ok(vec![Value::Double(cond_f64(input, norm, context)?)])
        }
        Value::Array(ArrayData::ComplexF64(input)) => {
            Ok(vec![Value::Double(cond_complex64(input, norm, context)?)])
        }
        value => Err(type_error(
            "cond",
            1,
            "real or complex double or single matrix",
            value,
        )),
    }
}

#[allow(clippy::float_cmp)]
fn condition_norm(value: &Value) -> Result<ConditionNorm, BuiltinError> {
    if let Some(keyword) = keyword(value) {
        return match keyword.as_str() {
            "inf" => Ok(ConditionNorm::Infinity),
            "fro" => Ok(ConditionNorm::Frobenius),
            _ => Err(option_error(
                "cond",
                "numeric 1, 2, Inf, or character/string 'inf' or 'fro'",
            )),
        };
    }
    let number = real_numeric_scalar(value).ok_or_else(|| {
        option_error(
            "cond",
            "numeric 1, 2, Inf, or character/string 'inf' or 'fro'",
        )
    })?;
    if number == 1.0 {
        Ok(ConditionNorm::One)
    } else if number == 2.0 {
        Ok(ConditionNorm::Two)
    } else if number == f64::INFINITY {
        Ok(ConditionNorm::Infinity)
    } else {
        Err(option_error(
            "cond",
            "numeric 1, 2, Inf, or character/string 'inf' or 'fro'",
        ))
    }
}

fn real_numeric_scalar(value: &Value) -> Option<f64> {
    if let Some(value) = exact_real_integer_scalar(value) {
        #[allow(clippy::cast_precision_loss)]
        return Some(match value {
            IntegerComponent::Signed(value) => value as f64,
            IntegerComponent::Unsigned(value) => value as f64,
        });
    }
    match value {
        Value::Double(value) => Some(*value),
        Value::Complex(value) if value.imaginary == 0.0 => Some(value.real),
        Value::Array(ArrayData::F64(value)) if value.numel() == 1 => Some(value.as_slice()[0]),
        Value::Array(ArrayData::ComplexF64(value))
            if value.numel() == 1 && value.as_slice()[0].im == 0.0 =>
        {
            Some(value.as_slice()[0].re)
        }
        Value::Array(ArrayData::F32(value)) if value.numel() == 1 => {
            Some(f64::from(value.as_slice()[0]))
        }
        Value::Array(ArrayData::ComplexF32(value))
            if value.numel() == 1 && value.as_slice()[0].im == 0.0 =>
        {
            Some(f64::from(value.as_slice()[0].re))
        }
        _ => None,
    }
}

fn cond_f64(
    input: &DenseArray<f64>,
    norm: ConditionNorm,
    context: &BuiltinContext<'_>,
) -> Result<f64, BuiltinError> {
    if norm == ConditionNorm::Two {
        let result = context
            .linalg_provider()
            .svd_f64(svd_request(input, SvdVectors::None, context))
            .map_err(|error| linalg_error("cond", error))?;
        validate_svd_result(input, &result, SvdVectors::None)?;
        return Ok(two_norm_condition(&result.singular_values));
    }
    ensure_square("cond", input)?;
    inverse_condition(input, norm, context, |identity| {
        context.linalg_provider().solve_f64(
            SolveRequest::new(input, identity).with_cancellation_flag(context.cancellation_flag()),
        )
    })
}

fn cond_complex64(
    input: &DenseArray<ArrayComplex64>,
    norm: ConditionNorm,
    context: &BuiltinContext<'_>,
) -> Result<f64, BuiltinError> {
    if norm == ConditionNorm::Two {
        let result = context
            .linalg_provider()
            .svd_complex64(svd_request(input, SvdVectors::None, context))
            .map_err(|error| linalg_error("cond", error))?;
        validate_svd_result(input, &result, SvdVectors::None)?;
        return Ok(two_norm_condition(&result.singular_values));
    }
    ensure_square("cond", input)?;
    inverse_condition(input, norm, context, |identity| {
        context.linalg_provider().solve_complex64(
            SolveRequest::new(input, identity).with_cancellation_flag(context.cancellation_flag()),
        )
    })
}

fn cond_f32(
    input: &DenseArray<f32>,
    norm: ConditionNorm,
    context: &BuiltinContext<'_>,
) -> Result<f32, BuiltinError> {
    if norm == ConditionNorm::Two {
        let result = context
            .linalg_provider()
            .svd_f32(svd_request(input, SvdVectors::None, context))
            .map_err(|error| linalg_error("cond", error))?;
        validate_svd_result(input, &result, SvdVectors::None)?;
        return Ok(two_norm_condition(&result.singular_values));
    }
    ensure_square("cond", input)?;
    inverse_condition(input, norm, context, |identity| {
        context.linalg_provider().solve_f32(
            SolveRequest::new(input, identity).with_cancellation_flag(context.cancellation_flag()),
        )
    })
}

fn cond_complex32(
    input: &DenseArray<ArrayComplex32>,
    norm: ConditionNorm,
    context: &BuiltinContext<'_>,
) -> Result<f32, BuiltinError> {
    if norm == ConditionNorm::Two {
        let result = context
            .linalg_provider()
            .svd_complex32(svd_request(input, SvdVectors::None, context))
            .map_err(|error| linalg_error("cond", error))?;
        validate_svd_result(input, &result, SvdVectors::None)?;
        return Ok(two_norm_condition(&result.singular_values));
    }
    ensure_square("cond", input)?;
    inverse_condition(input, norm, context, |identity| {
        context.linalg_provider().solve_complex32(
            SolveRequest::new(input, identity).with_cancellation_flag(context.cancellation_flag()),
        )
    })
}

fn two_norm_condition<R: RealElement>(singular_values: &DenseArray<R>) -> R {
    let Some(maximum) = singular_values.as_slice().first().copied() else {
        return R::default();
    };
    let minimum = singular_values
        .as_slice()
        .last()
        .copied()
        .expect("a nonempty singular-value column has a last element");
    if minimum == R::default() {
        R::infinity()
    } else {
        maximum.divide(minimum)
    }
}

fn inverse_condition<T, F>(
    input: &DenseArray<T>,
    norm: ConditionNorm,
    context: &BuiltinContext<'_>,
    solve: F,
) -> Result<T::Real, BuiltinError>
where
    T: NormElement,
    F: FnOnce(&DenseArray<T>) -> Result<DenseArray<T>, LinalgError>,
{
    let order = input.shape().extent(0);
    if order == 0 {
        return Ok(T::Real::default());
    }
    let identity = identity_matrix::<T>(order, context)?;
    let inverse = match solve(&identity) {
        Ok(value) => value,
        Err(LinalgError::SingularMatrix { .. }) => return Ok(T::Real::infinity()),
        Err(error) => return Err(linalg_error("cond", error)),
    };
    if inverse.shape() != input.shape() {
        return Err(provider_contract_error("cond", "inverse solve shape"));
    }
    let input_norm = matrix_norm(input, norm, context)?;
    let inverse_norm = matrix_norm(&inverse, norm, context)?;
    Ok(input_norm.multiply(inverse_norm))
}

fn identity_matrix<T: SpectralElement>(
    order: u64,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let shape = Shape::new([order, order]).map_err(|error| array_error(&error))?;
    let mut values = filled_values("cond identity", shape.numel(), T::default())?;
    let order = checked_host_length("cond identity order", order)?;
    for diagonal in 0..order {
        check_cancelled_at(context, diagonal)?;
        values[diagonal * order + diagonal] = T::one();
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn matrix_norm<T: NormElement>(
    input: &DenseArray<T>,
    norm: ConditionNorm,
    context: &BuiltinContext<'_>,
) -> Result<T::Real, BuiltinError> {
    let rows = checked_host_length("cond norm rows", input.shape().extent(0))?;
    let columns = checked_host_length("cond norm columns", input.shape().extent(1))?;
    match norm {
        ConditionNorm::One => {
            let mut maximum = T::Real::default();
            for column in 0..columns {
                let mut sum = T::Real::default();
                for row in 0..rows {
                    let offset = column * rows + row;
                    check_cancelled_at(context, offset)?;
                    sum = sum.add(input.as_slice()[offset].magnitude());
                }
                if matches!(sum.partial_cmp(&maximum), Some(Ordering::Greater) | None) {
                    maximum = sum;
                }
            }
            Ok(maximum)
        }
        ConditionNorm::Infinity => {
            let mut maximum = T::Real::default();
            for row in 0..rows {
                let mut sum = T::Real::default();
                for column in 0..columns {
                    let offset = column * rows + row;
                    check_cancelled_at(context, offset)?;
                    sum = sum.add(input.as_slice()[offset].magnitude());
                }
                if matches!(sum.partial_cmp(&maximum), Some(Ordering::Greater) | None) {
                    maximum = sum;
                }
            }
            Ok(maximum)
        }
        ConditionNorm::Frobenius => {
            let mut accumulated = T::Real::default();
            for (index, value) in input.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                accumulated = accumulated.hypot(value.magnitude());
            }
            Ok(accumulated)
        }
        ConditionNorm::Two => unreachable!("two-norm condition uses SVD"),
    }
}

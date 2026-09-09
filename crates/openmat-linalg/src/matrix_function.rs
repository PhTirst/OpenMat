use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::{Complex32, Complex64, DenseArray};

use crate::{LinalgError, LinalgProvider, SchurRequest, SchurResult, matrix_dimensions};

const MATRIX_FUNCTION_PROVIDER: &str = "matrix-function";
const MAX_SQUARE_ROOTS: usize = 32;
const MAX_SERIES_TERMS: usize = 256;

/// Borrowed matrix and cancellation state for a principal matrix function.
#[derive(Clone, Copy, Debug)]
pub struct MatrixFunctionRequest<'array, T> {
    /// Square matrix in contiguous column-major storage.
    pub matrix: &'array DenseArray<T>,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, T> MatrixFunctionRequest<'array, T> {
    /// Constructs a request without a cancellation flag.
    #[must_use]
    pub const fn new(matrix: &'array DenseArray<T>) -> Self {
        Self {
            matrix,
            cancellation: None,
        }
    }

    /// Attaches a cooperative cancellation flag.
    #[must_use]
    pub const fn with_cancellation_flag(mut self, cancellation: &'array AtomicBool) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    fn check_cancellation(&self) -> Result<(), LinalgError> {
        if self
            .cancellation
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            Err(LinalgError::Cancelled {
                operation: "principal matrix function",
            })
        } else {
            Ok(())
        }
    }
}

/// Principal matrix square root and its relative one-norm residual.
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixSquareRootResult<C, R> {
    /// Principal matrix square root.
    pub root: DenseArray<C>,
    /// `norm(root*root-matrix, 1) / norm(matrix, 1)`.
    pub relative_residual_1: R,
}

/// Computes the principal square root of a real binary64 matrix.
///
/// # Errors
///
/// Returns a shape, cancellation, allocation, Schur, or convergence error.
pub fn matrix_sqrt_f64(
    provider: &dyn LinalgProvider,
    request: MatrixFunctionRequest<'_, f64>,
) -> Result<MatrixSquareRootResult<Complex64, f64>, LinalgError> {
    let complex = convert_real64(request.matrix)?;
    matrix_sqrt_complex64(provider, forward_request(&request, &complex))
}

/// Computes the principal square root of a complex binary64 matrix.
///
/// # Errors
///
/// Returns a shape, cancellation, allocation, Schur, or convergence error.
pub fn matrix_sqrt_complex64(
    provider: &dyn LinalgProvider,
    request: MatrixFunctionRequest<'_, Complex64>,
) -> Result<MatrixSquareRootResult<Complex64, f64>, LinalgError> {
    validate_matrix_function(&request)?;
    request.check_cancellation()?;
    let schur = provider.schur_complex64(schur_request(&request))?;
    let order = host_order(request.matrix)?;
    let mut check = || request.check_cancellation();
    let (root, residual) = sqrt_from_schur(
        request.matrix.as_slice(),
        &schur,
        order,
        f64::EPSILON,
        &mut check,
    )?;
    Ok(MatrixSquareRootResult {
        root: DenseArray::from_vec(request.matrix.shape().clone(), root)?,
        relative_residual_1: residual,
    })
}

/// Computes the principal square root of a real binary32 matrix.
///
/// # Errors
///
/// Returns a shape, cancellation, allocation, Schur, or convergence error.
pub fn matrix_sqrt_f32(
    provider: &dyn LinalgProvider,
    request: MatrixFunctionRequest<'_, f32>,
) -> Result<MatrixSquareRootResult<Complex32, f32>, LinalgError> {
    let complex = convert_real32(request.matrix)?;
    matrix_sqrt_complex32(provider, forward_request(&request, &complex))
}

/// Computes the principal square root of a complex binary32 matrix.
///
/// The triangular function is accumulated in binary64 after the provider's
/// native binary32 Schur factorization, then rounded once into binary32 output.
///
/// # Errors
///
/// Returns a shape, cancellation, allocation, Schur, or convergence error.
pub fn matrix_sqrt_complex32(
    provider: &dyn LinalgProvider,
    request: MatrixFunctionRequest<'_, Complex32>,
) -> Result<MatrixSquareRootResult<Complex32, f32>, LinalgError> {
    validate_matrix_function(&request)?;
    request.check_cancellation()?;
    let schur = provider.schur_complex32(schur_request(&request))?;
    let input = widen_complex32(request.matrix.as_slice())?;
    let widened = widen_schur32(&schur)?;
    let order = host_order(request.matrix)?;
    let mut check = || request.check_cancellation();
    let (root, residual) =
        sqrt_from_schur(&input, &widened, order, f64::from(f32::EPSILON), &mut check)?;
    Ok(MatrixSquareRootResult {
        root: DenseArray::from_vec(request.matrix.shape().clone(), narrow_complex32(&root)?)?,
        relative_residual_1: narrow_f32(residual),
    })
}

/// Computes a principal real-input binary64 matrix power.
///
/// # Errors
///
/// Returns a shape, cancellation, allocation, Schur, branch, or convergence error.
pub fn matrix_power_f64(
    provider: &dyn LinalgProvider,
    request: MatrixFunctionRequest<'_, f64>,
    exponent: Complex64,
) -> Result<DenseArray<Complex64>, LinalgError> {
    let complex = convert_real64(request.matrix)?;
    matrix_power_complex64(provider, forward_request(&request, &complex), exponent)
}

/// Computes a principal complex binary64 matrix power.
///
/// # Errors
///
/// Returns a shape, cancellation, allocation, Schur, branch, or convergence error.
pub fn matrix_power_complex64(
    provider: &dyn LinalgProvider,
    request: MatrixFunctionRequest<'_, Complex64>,
    exponent: Complex64,
) -> Result<DenseArray<Complex64>, LinalgError> {
    validate_matrix_function(&request)?;
    request.check_cancellation()?;
    let schur = provider.schur_complex64(schur_request(&request))?;
    let order = host_order(request.matrix)?;
    let mut check = || request.check_cancellation();
    let values = power_from_schur(&schur, order, exponent, f64::EPSILON, &mut check)?;
    DenseArray::from_vec(request.matrix.shape().clone(), values).map_err(Into::into)
}

/// Computes a principal real-input binary32 matrix power.
///
/// # Errors
///
/// Returns a shape, cancellation, allocation, Schur, branch, or convergence error.
pub fn matrix_power_f32(
    provider: &dyn LinalgProvider,
    request: MatrixFunctionRequest<'_, f32>,
    exponent: Complex32,
) -> Result<DenseArray<Complex32>, LinalgError> {
    let complex = convert_real32(request.matrix)?;
    matrix_power_complex32(provider, forward_request(&request, &complex), exponent)
}

/// Computes a principal complex binary32 matrix power.
///
/// # Errors
///
/// Returns a shape, cancellation, allocation, Schur, branch, or convergence error.
pub fn matrix_power_complex32(
    provider: &dyn LinalgProvider,
    request: MatrixFunctionRequest<'_, Complex32>,
    exponent: Complex32,
) -> Result<DenseArray<Complex32>, LinalgError> {
    validate_matrix_function(&request)?;
    request.check_cancellation()?;
    let schur = provider.schur_complex32(schur_request(&request))?;
    let widened = widen_schur32(&schur)?;
    let order = host_order(request.matrix)?;
    let mut check = || request.check_cancellation();
    let values = power_from_schur(
        &widened,
        order,
        Complex64::new(f64::from(exponent.re), f64::from(exponent.im)),
        f64::from(f32::EPSILON),
        &mut check,
    )?;
    DenseArray::from_vec(request.matrix.shape().clone(), narrow_complex32(&values)?)
        .map_err(Into::into)
}

fn validate_matrix_function<T>(request: &MatrixFunctionRequest<'_, T>) -> Result<(), LinalgError> {
    let (rows, columns) = matrix_dimensions("matrix-function input", request.matrix)?;
    if rows != columns {
        return Err(LinalgError::SquareMatrixRequired {
            operand: "matrix-function input",
            rows,
            columns,
        });
    }
    Ok(())
}

fn host_order<T>(matrix: &DenseArray<T>) -> Result<usize, LinalgError> {
    usize::try_from(matrix.shape().extent(0)).map_err(|_| LinalgError::AllocationFailure {
        operation: "matrix-function host order",
        elements: matrix.shape().extent(0),
    })
}

fn forward_request<'a, T, U>(
    source: &MatrixFunctionRequest<'a, T>,
    matrix: &'a DenseArray<U>,
) -> MatrixFunctionRequest<'a, U> {
    let mut request = MatrixFunctionRequest::new(matrix);
    if let Some(flag) = source.cancellation {
        request = request.with_cancellation_flag(flag);
    }
    request
}

fn schur_request<'a, C>(request: &MatrixFunctionRequest<'a, C>) -> SchurRequest<'a, C> {
    let mut result = SchurRequest::new(request.matrix);
    if let Some(flag) = request.cancellation {
        result = result.with_cancellation_flag(flag);
    }
    result
}

fn convert_real64(matrix: &DenseArray<f64>) -> Result<DenseArray<Complex64>, LinalgError> {
    let values = try_map(matrix.as_slice(), |value| Complex64::new(*value, 0.0))?;
    Ok(DenseArray::from_vec(matrix.shape().clone(), values)?)
}

fn convert_real32(matrix: &DenseArray<f32>) -> Result<DenseArray<Complex32>, LinalgError> {
    let values = try_map(matrix.as_slice(), |value| Complex32::new(*value, 0.0))?;
    Ok(DenseArray::from_vec(matrix.shape().clone(), values)?)
}

fn widen_complex32(values: &[Complex32]) -> Result<Vec<Complex64>, LinalgError> {
    try_map(values, |value| {
        Complex64::new(f64::from(value.re), f64::from(value.im))
    })
}

fn narrow_complex32(values: &[Complex64]) -> Result<Vec<Complex32>, LinalgError> {
    #[allow(clippy::cast_possible_truncation)]
    try_map(values, |value| {
        Complex32::new(value.re as f32, value.im as f32)
    })
}

fn widen_schur32(result: &SchurResult<Complex32>) -> Result<SchurResult<Complex64>, LinalgError> {
    Ok(SchurResult {
        form: DenseArray::from_vec(
            result.form.shape().clone(),
            widen_complex32(result.form.as_slice())?,
        )?,
        vectors: DenseArray::from_vec(
            result.vectors.shape().clone(),
            widen_complex32(result.vectors.as_slice())?,
        )?,
    })
}

fn sqrt_from_schur(
    input: &[Complex64],
    schur: &SchurResult<Complex64>,
    order: usize,
    epsilon: f64,
    check: &mut impl FnMut() -> Result<(), LinalgError>,
) -> Result<(Vec<Complex64>, f64), LinalgError> {
    validate_schur_result(schur, order)?;
    let triangular = triangular_sqrt(schur.form.as_slice(), order, check)?;
    let root = undo_schur(&triangular, schur.vectors.as_slice(), order, check)?;
    let squared = multiply(&root, &root, order, check)?;
    let mut difference = try_copy("matrix-square-root residual", &squared)?;
    for (value, original) in difference.iter_mut().zip(input) {
        *value = c_sub(*value, *original);
    }
    let denominator = matrix_one_norm(input, order);
    let residual = matrix_one_norm(&difference, order) / denominator;
    let threshold = 64.0 * epsilon * count_as_f64(order.max(1));
    let root = clean_roundoff(root, threshold);
    Ok((root, residual))
}

fn power_from_schur(
    schur: &SchurResult<Complex64>,
    order: usize,
    exponent: Complex64,
    epsilon: f64,
    check: &mut impl FnMut() -> Result<(), LinalgError>,
) -> Result<Vec<Complex64>, LinalgError> {
    validate_schur_result(schur, order)?;
    if order == 0 {
        return Ok(Vec::new());
    }
    let function = if exponent.im == 0.0 && exponent.re.to_bits() == 0.5_f64.to_bits() {
        triangular_sqrt(schur.form.as_slice(), order, check)?
    } else if is_effectively_diagonal(schur.form.as_slice(), order, epsilon) {
        diagonal_power(schur.form.as_slice(), order, exponent)?
    } else {
        let logarithm = triangular_log(schur.form.as_slice(), order, epsilon, check)?;
        let scaled = try_map(&logarithm, |value| c_mul(*value, exponent))?;
        matrix_exp(&scaled, order, epsilon, check)?
    };
    let result = undo_schur(&function, schur.vectors.as_slice(), order, check)?;
    Ok(clean_roundoff(
        result,
        64.0 * epsilon * count_as_f64(order.max(1)),
    ))
}

fn validate_schur_result(result: &SchurResult<Complex64>, order: usize) -> Result<(), LinalgError> {
    let expected = order
        .checked_mul(order)
        .ok_or_else(|| allocation("Schur result dimensions", usize::MAX))?;
    if result.form.as_slice().len() != expected || result.vectors.as_slice().len() != expected {
        return Err(LinalgError::ProviderFailure {
            provider: MATRIX_FUNCTION_PROVIDER,
            operation: "Schur result validation",
            detail: "provider returned an invalid Schur shape".to_owned(),
        });
    }
    Ok(())
}

fn triangular_sqrt(
    matrix: &[Complex64],
    order: usize,
    check: &mut impl FnMut() -> Result<(), LinalgError>,
) -> Result<Vec<Complex64>, LinalgError> {
    let mut root = try_filled("triangular square root", matrix.len(), Complex64::ZERO)?;
    for index in 0..order {
        root[index * order + index] = c_sqrt(matrix[index * order + index]);
    }
    for gap in 1..order {
        check()?;
        for row in 0..order - gap {
            let column = row + gap;
            let mut numerator = matrix[column * order + row];
            for inner in row + 1..column {
                numerator = c_sub(
                    numerator,
                    c_mul(root[inner * order + row], root[column * order + inner]),
                );
            }
            let denominator = c_add(root[row * order + row], root[column * order + column]);
            root[column * order + row] = if c_abs(denominator) == 0.0 {
                if c_abs(numerator) == 0.0 {
                    Complex64::ZERO
                } else {
                    c_infinite_quotient(numerator)
                }
            } else {
                c_div(numerator, denominator)
            };
        }
    }
    Ok(root)
}

fn triangular_log(
    matrix: &[Complex64],
    order: usize,
    epsilon: f64,
    check: &mut impl FnMut() -> Result<(), LinalgError>,
) -> Result<Vec<Complex64>, LinalgError> {
    for index in 0..order {
        if c_abs(matrix[index * order + index]) == 0.0 {
            return Err(LinalgError::SingularMatrix {
                provider: MATRIX_FUNCTION_PROVIDER,
                operation: "principal matrix logarithm",
                pivot: u64::try_from(index + 1).unwrap_or(u64::MAX),
            });
        }
    }
    let mut reduced = try_copy("matrix logarithm Schur form", matrix)?;
    let identity = identity(order)?;
    let mut square_roots = 0_usize;
    while distance_from_identity(&reduced, order) > 0.25 {
        if square_roots == MAX_SQUARE_ROOTS {
            return Err(no_convergence(
                "matrix logarithm inverse scaling",
                square_roots,
            ));
        }
        reduced = triangular_sqrt(&reduced, order, check)?;
        square_roots += 1;
    }
    let mut x = try_copy("matrix logarithm displacement", &reduced)?;
    for (value, one) in x.iter_mut().zip(&identity) {
        *value = c_sub(*value, *one);
    }
    let mut term = try_copy("matrix logarithm term", &x)?;
    let mut sum = try_copy("matrix logarithm series", &x)?;
    let mut converged = matrix_one_norm(&term, order) == 0.0;
    for degree in 2..=MAX_SERIES_TERMS {
        check()?;
        term = multiply(&term, &x, order, check)?;
        let sign = if degree % 2 == 0 { -1.0 } else { 1.0 };
        let degree = count_as_f64(degree);
        let factor = sign / degree;
        for (value, contribution) in sum.iter_mut().zip(&term) {
            *value = c_add(*value, c_scale(*contribution, factor));
        }
        if matrix_one_norm(&term, order) / degree <= epsilon * matrix_one_norm(&sum, order).max(1.0)
        {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err(no_convergence(
            "matrix logarithm Taylor series",
            MAX_SERIES_TERMS,
        ));
    }
    let scale = 2.0_f64.powi(i32::try_from(square_roots).unwrap_or(i32::MAX));
    for value in &mut sum {
        *value = c_scale(*value, scale);
    }
    Ok(sum)
}

fn matrix_exp(
    matrix: &[Complex64],
    order: usize,
    epsilon: f64,
    check: &mut impl FnMut() -> Result<(), LinalgError>,
) -> Result<Vec<Complex64>, LinalgError> {
    let norm = matrix_one_norm(matrix, order);
    let mut square_count = 0_usize;
    if norm.is_finite() {
        let mut scaled_norm = norm;
        while scaled_norm > 0.5 {
            square_count += 1;
            if square_count > 1024 {
                return Err(no_convergence("matrix exponential scaling", square_count));
            }
            scaled_norm *= 0.5;
        }
    }
    let scale = 2.0_f64.powi(-i32::try_from(square_count).unwrap_or(i32::MAX));
    let scaled = try_map(matrix, |value| c_scale(*value, scale))?;
    let mut sum = identity(order)?;
    let mut term = identity(order)?;
    let mut converged = norm == 0.0;
    for degree in 1..=MAX_SERIES_TERMS {
        check()?;
        term = multiply(&term, &scaled, order, check)?;
        let inverse = 1.0 / count_as_f64(degree);
        for value in &mut term {
            *value = c_scale(*value, inverse);
        }
        for (value, contribution) in sum.iter_mut().zip(&term) {
            *value = c_add(*value, *contribution);
        }
        if matrix_one_norm(&term, order) <= epsilon * matrix_one_norm(&sum, order).max(1.0) {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err(no_convergence(
            "matrix exponential Taylor series",
            MAX_SERIES_TERMS,
        ));
    }
    for _ in 0..square_count {
        check()?;
        sum = multiply(&sum, &sum, order, check)?;
    }
    Ok(sum)
}

fn diagonal_power(
    matrix: &[Complex64],
    order: usize,
    exponent: Complex64,
) -> Result<Vec<Complex64>, LinalgError> {
    let mut result = try_filled("diagonal matrix power", matrix.len(), Complex64::ZERO)?;
    for index in 0..order {
        result[index * order + index] = c_pow(matrix[index * order + index], exponent);
    }
    Ok(result)
}

fn is_effectively_diagonal(matrix: &[Complex64], order: usize, epsilon: f64) -> bool {
    let threshold = 16.0 * epsilon * matrix_one_norm(matrix, order).max(1.0);
    for column in 0..order {
        for row in 0..order {
            if row != column && c_abs(matrix[column * order + row]) > threshold {
                return false;
            }
        }
    }
    true
}

fn undo_schur(
    function: &[Complex64],
    vectors: &[Complex64],
    order: usize,
    check: &mut impl FnMut() -> Result<(), LinalgError>,
) -> Result<Vec<Complex64>, LinalgError> {
    if is_identity(vectors, order) {
        return try_copy("matrix-function identity Schur reconstruction", function);
    }
    let product = multiply(vectors, function, order, check)?;
    let adjoint = conjugate_transpose(vectors, order)?;
    multiply(&product, &adjoint, order, check)
}

fn is_identity(matrix: &[Complex64], order: usize) -> bool {
    matrix.iter().enumerate().all(|(offset, value)| {
        let column = offset / order;
        let row = offset % order;
        let expected = if row == column {
            Complex64::new(1.0, 0.0)
        } else {
            Complex64::ZERO
        };
        c_abs(c_sub(*value, expected)) == 0.0
    })
}

fn multiply(
    left: &[Complex64],
    right: &[Complex64],
    order: usize,
    check: &mut impl FnMut() -> Result<(), LinalgError>,
) -> Result<Vec<Complex64>, LinalgError> {
    let length = order
        .checked_mul(order)
        .ok_or_else(|| allocation("matrix-function product", usize::MAX))?;
    let mut result = try_filled("matrix-function product", length, Complex64::ZERO)?;
    for column in 0..order {
        check()?;
        for inner in 0..order {
            let factor = right[column * order + inner];
            for row in 0..order {
                let index = column * order + row;
                result[index] = c_add(result[index], c_mul(left[inner * order + row], factor));
            }
        }
    }
    Ok(result)
}

fn conjugate_transpose(matrix: &[Complex64], order: usize) -> Result<Vec<Complex64>, LinalgError> {
    let mut result = try_filled(
        "matrix-function conjugate transpose",
        matrix.len(),
        Complex64::ZERO,
    )?;
    for column in 0..order {
        for row in 0..order {
            result[column * order + row] = matrix[row * order + column].conjugate();
        }
    }
    Ok(result)
}

fn identity(order: usize) -> Result<Vec<Complex64>, LinalgError> {
    let length = order
        .checked_mul(order)
        .ok_or_else(|| allocation("matrix-function identity", usize::MAX))?;
    let mut result = try_filled("matrix-function identity", length, Complex64::ZERO)?;
    for index in 0..order {
        result[index * order + index] = Complex64::new(1.0, 0.0);
    }
    Ok(result)
}

fn matrix_one_norm(matrix: &[Complex64], order: usize) -> f64 {
    let mut norm = 0.0_f64;
    for column in 0..order {
        let mut sum = 0.0_f64;
        for row in 0..order {
            sum += c_abs(matrix[column * order + row]);
        }
        if sum.is_nan() {
            return f64::NAN;
        }
        norm = norm.max(sum);
    }
    norm
}

fn distance_from_identity(matrix: &[Complex64], order: usize) -> f64 {
    let mut norm = 0.0_f64;
    for column in 0..order {
        let mut sum = 0.0_f64;
        for row in 0..order {
            let mut value = matrix[column * order + row];
            if row == column {
                value = c_sub(value, Complex64::new(1.0, 0.0));
            }
            sum += c_abs(value);
        }
        norm = norm.max(sum);
    }
    norm
}

fn clean_roundoff(mut values: Vec<Complex64>, relative: f64) -> Vec<Complex64> {
    let scale = values.iter().fold(0.0_f64, |current, value| {
        let magnitude = c_abs(*value);
        if magnitude.is_finite() {
            current.max(magnitude)
        } else {
            current
        }
    });
    let threshold = relative * scale.max(1.0);
    for value in &mut values {
        if value.re.is_finite() && value.re.abs() <= threshold {
            value.re = 0.0;
        }
        if value.im.is_finite() && value.im.abs() <= threshold {
            value.im = 0.0;
        }
    }
    values
}

fn c_add(left: Complex64, right: Complex64) -> Complex64 {
    Complex64::new(left.re + right.re, left.im + right.im)
}

fn c_sub(left: Complex64, right: Complex64) -> Complex64 {
    Complex64::new(left.re - right.re, left.im - right.im)
}

fn c_mul(left: Complex64, right: Complex64) -> Complex64 {
    Complex64::new(
        left.re.mul_add(right.re, -(left.im * right.im)),
        left.re.mul_add(right.im, left.im * right.re),
    )
}

fn c_scale(value: Complex64, scale: f64) -> Complex64 {
    Complex64::new(value.re * scale, value.im * scale)
}

fn c_abs(value: Complex64) -> f64 {
    value.re.hypot(value.im)
}

fn c_div(left: Complex64, right: Complex64) -> Complex64 {
    if right.re == 0.0 && right.im == 0.0 {
        return c_infinite_quotient(left);
    }
    if right.re.abs() >= right.im.abs() {
        let ratio = right.im / right.re;
        let denominator = right.re + right.im * ratio;
        Complex64::new(
            (left.re + left.im * ratio) / denominator,
            (left.im - left.re * ratio) / denominator,
        )
    } else {
        let ratio = right.re / right.im;
        let denominator = right.im + right.re * ratio;
        Complex64::new(
            (left.re * ratio + left.im) / denominator,
            (left.im * ratio - left.re) / denominator,
        )
    }
}

fn c_sqrt(value: Complex64) -> Complex64 {
    if value.im == 0.0 {
        if value.re < 0.0 {
            return Complex64::new(0.0, (-value.re).sqrt());
        }
        return Complex64::new(value.re.sqrt(), value.im);
    }
    let magnitude = c_abs(value);
    if value.re >= 0.0 {
        let real = ((magnitude + value.re) * 0.5).sqrt();
        Complex64::new(real, value.im / (2.0 * real))
    } else {
        let imaginary = ((magnitude - value.re) * 0.5).sqrt().copysign(value.im);
        Complex64::new(value.im / (2.0 * imaginary), imaginary)
    }
}

fn c_log(value: Complex64) -> Complex64 {
    Complex64::new(c_abs(value).ln(), value.im.atan2(value.re))
}

fn c_exp(value: Complex64) -> Complex64 {
    let magnitude = value.re.exp();
    Complex64::new(magnitude * value.im.cos(), magnitude * value.im.sin())
}

fn c_pow(value: Complex64, exponent: Complex64) -> Complex64 {
    if value.re == 0.0 && value.im == 0.0 {
        if exponent.im == 0.0 && exponent.re > 0.0 {
            return Complex64::ZERO;
        }
        if exponent.re == 0.0 && exponent.im == 0.0 {
            return Complex64::new(1.0, 0.0);
        }
    }
    c_exp(c_mul(exponent, c_log(value)))
}

fn c_infinite_quotient(numerator: Complex64) -> Complex64 {
    let real = if numerator.re == 0.0 {
        0.0
    } else {
        f64::INFINITY.copysign(numerator.re)
    };
    let imaginary = if numerator.im == 0.0 {
        0.0
    } else {
        f64::INFINITY.copysign(numerator.im)
    };
    Complex64::new(real, imaginary)
}

fn count_as_f64(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

#[allow(clippy::cast_possible_truncation)]
fn narrow_f32(value: f64) -> f32 {
    value as f32
}

fn try_map<T, U>(values: &[T], mut map: impl FnMut(&T) -> U) -> Result<Vec<U>, LinalgError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(values.len())
        .map_err(|_| allocation("matrix-function conversion", values.len()))?;
    result.extend(values.iter().map(&mut map));
    Ok(result)
}

fn try_copy<T: Copy>(operation: &'static str, values: &[T]) -> Result<Vec<T>, LinalgError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(values.len())
        .map_err(|_| allocation(operation, values.len()))?;
    result.extend_from_slice(values);
    Ok(result)
}

fn try_filled<T: Clone>(
    operation: &'static str,
    length: usize,
    value: T,
) -> Result<Vec<T>, LinalgError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| allocation(operation, length))?;
    result.resize(length, value);
    Ok(result)
}

fn allocation(operation: &'static str, length: usize) -> LinalgError {
    LinalgError::AllocationFailure {
        operation,
        elements: u64::try_from(length).unwrap_or(u64::MAX),
    }
}

fn no_convergence(operation: &'static str, iterations: usize) -> LinalgError {
    LinalgError::NoConvergence {
        provider: MATRIX_FUNCTION_PROVIDER,
        operation,
        iterations: Some(u64::try_from(iterations).unwrap_or(u64::MAX)),
        unconverged: None,
    }
}

#[cfg(test)]
mod tests {
    use openmat_array::{Complex64, DenseArray, Shape};

    use super::{MatrixFunctionRequest, matrix_power_f64, matrix_sqrt_f64};
    use crate::ReferenceProvider;

    #[test]
    fn schur_square_root_and_power_preserve_jordan_derivatives() {
        let matrix =
            DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![4.0, 0.0, 1.0, 4.0]).unwrap();
        let provider = ReferenceProvider;
        let root = matrix_sqrt_f64(&provider, MatrixFunctionRequest::new(&matrix)).unwrap();
        assert!((root.root.as_slice()[0].re - 2.0).abs() < 1.0e-12);
        assert!((root.root.as_slice()[2].re - 0.25).abs() < 1.0e-12);
        assert!(root.relative_residual_1 < 1.0e-12);

        let power = matrix_power_f64(
            &provider,
            MatrixFunctionRequest::new(&matrix),
            Complex64::new(1.5, 0.0),
        )
        .unwrap();
        assert!((power.as_slice()[0].re - 8.0).abs() < 1.0e-10);
        assert!((power.as_slice()[2].re - 3.0).abs() < 1.0e-10);
    }

    #[test]
    fn negative_spectrum_uses_the_principal_branch() {
        let matrix =
            DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![-1.0, 0.0, 0.0, 4.0]).unwrap();
        let root =
            matrix_sqrt_f64(&ReferenceProvider, MatrixFunctionRequest::new(&matrix)).unwrap();
        assert_eq!(root.root.as_slice()[0], Complex64::new(0.0, 1.0));
        assert_eq!(root.root.as_slice()[3], Complex64::new(2.0, 0.0));
    }

    #[test]
    fn nilpotent_jordan_root_preserves_the_single_infinite_entry() {
        let matrix =
            DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![0.0, 0.0, 1.0, 0.0]).unwrap();
        let root =
            matrix_sqrt_f64(&ReferenceProvider, MatrixFunctionRequest::new(&matrix)).unwrap();
        assert_eq!(root.root.as_slice()[0], Complex64::ZERO);
        assert_eq!(root.root.as_slice()[1], Complex64::ZERO);
        assert!(root.root.as_slice()[2].re.is_infinite());
        assert!(root.root.as_slice()[2].im.abs() <= f64::EPSILON);
        assert_eq!(root.root.as_slice()[3], Complex64::ZERO);
        assert!(root.relative_residual_1.is_nan());
    }
}

use openmat_array::{Complex32, Complex64, DenseArray, Shape};

use crate::{
    CholeskyRequest, CholeskyResult, EigRequest, EigResult, FactorRequest, LinalgError, LuResult,
    QrRequest, QrResult, RectangularSolveRequest, SchurRequest, SchurResult, SolveRequest,
    SvdRequest, SvdResult,
};

/// Matrix transformation applied before a GEMM operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatrixTranspose {
    /// Use the matrix as stored.
    None,
    /// Exchange rows and columns without conjugating values.
    Transpose,
    /// Exchange rows and columns and conjugate complex values.
    ConjugateTranspose,
}

/// Integer calling convention used by a numerical provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderIntegerAbi {
    /// Checked Rust `u64` dimensions; no external integer ABI is involved.
    RustU64,
    /// Signed 32-bit BLAS dimensions and leading dimensions.
    Lp64,
}

/// Scalar and transformation options for `C = alpha * op(A) * op(B) + beta * C`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GemmOptions<T> {
    /// Transformation applied to the left matrix.
    pub left_transpose: MatrixTranspose,
    /// Transformation applied to the right matrix.
    pub right_transpose: MatrixTranspose,
    /// Product multiplier.
    pub alpha: T,
    /// Existing-output multiplier.
    pub beta: T,
}

impl<T> GemmOptions<T> {
    /// Constructs GEMM options.
    #[must_use]
    pub const fn new(
        left_transpose: MatrixTranspose,
        right_transpose: MatrixTranspose,
        alpha: T,
        beta: T,
    ) -> Self {
        Self {
            left_transpose,
            right_transpose,
            alpha,
            beta,
        }
    }
}

/// Borrowed inputs and scalar options for a GEMM operation.
#[derive(Clone, Copy, Debug)]
pub struct GemmRequest<'array, T> {
    /// Left matrix in column-major storage.
    pub left: &'array DenseArray<T>,
    /// Right matrix in column-major storage.
    pub right: &'array DenseArray<T>,
    /// Scalar and transpose options.
    pub options: GemmOptions<T>,
}

impl<'array, T> GemmRequest<'array, T> {
    /// Constructs a borrowed GEMM request.
    #[must_use]
    pub const fn new(
        left: &'array DenseArray<T>,
        right: &'array DenseArray<T>,
        options: GemmOptions<T>,
    ) -> Self {
        Self {
            left,
            right,
            options,
        }
    }
}

/// Validated GEMM dimensions before conversion to a provider integer ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GemmDimensions {
    m: u64,
    n: u64,
    k: u64,
    left_rows: u64,
    right_rows: u64,
    output_rows: u64,
}

impl GemmDimensions {
    /// Rows in `op(A)` and `C`.
    #[must_use]
    pub const fn m(self) -> u64 {
        self.m
    }

    /// Columns in `op(B)` and `C`.
    #[must_use]
    pub const fn n(self) -> u64 {
        self.n
    }

    /// Shared inner dimension.
    #[must_use]
    pub const fn k(self) -> u64 {
        self.k
    }

    /// Physical row count, and thus column-major leading dimension, of `A`.
    #[must_use]
    pub fn left_leading_dimension(self) -> u64 {
        self.left_rows.max(1)
    }

    /// Physical row count, and thus column-major leading dimension, of `B`.
    #[must_use]
    pub fn right_leading_dimension(self) -> u64 {
        self.right_rows.max(1)
    }

    /// Physical row count, and thus column-major leading dimension, of `C`.
    #[must_use]
    pub fn output_leading_dimension(self) -> u64 {
        self.output_rows.max(1)
    }
}

/// Returns the two physical dimensions of an array accepted as a matrix.
///
/// Canonical trailing singleton dimensions have already been removed by
/// [`Shape`], so a rank above two here means a significant higher dimension.
///
/// # Errors
///
/// Returns [`LinalgError::MatrixRequired`] if `array` is not two-dimensional.
pub fn matrix_dimensions<T>(
    operand: &'static str,
    array: &DenseArray<T>,
) -> Result<(u64, u64), LinalgError> {
    if array.shape().ndims() != 2 {
        return Err(LinalgError::MatrixRequired {
            operand,
            dimensions: array.shape().dimensions().to_vec(),
        });
    }
    Ok((array.shape().extent(0), array.shape().extent(1)))
}

fn transformed_dimensions(rows: u64, columns: u64, transpose: MatrixTranspose) -> (u64, u64) {
    match transpose {
        MatrixTranspose::None => (rows, columns),
        MatrixTranspose::Transpose | MatrixTranspose::ConjugateTranspose => (columns, rows),
    }
}

/// Validates matrix ranks, inner dimensions, and output shape for GEMM.
///
/// The returned dimensions remain `u64`. An LP64 provider must additionally
/// convert them through [`crate::Lp64GemmDimensions`] before entering CBLAS or
/// LAPACKE.
///
/// # Errors
///
/// Returns a matrix-rank, dimension-mismatch, or output-shape error.
pub fn validate_gemm<T>(
    request: &GemmRequest<'_, T>,
    output: &DenseArray<T>,
) -> Result<GemmDimensions, LinalgError> {
    let (left_rows, left_columns) = matrix_dimensions("left GEMM operand", request.left)?;
    let (right_rows, right_columns) = matrix_dimensions("right GEMM operand", request.right)?;
    let (output_rows, output_columns) = matrix_dimensions("GEMM output", output)?;
    let (m, left_inner) =
        transformed_dimensions(left_rows, left_columns, request.options.left_transpose);
    let (right_inner, n) =
        transformed_dimensions(right_rows, right_columns, request.options.right_transpose);
    if left_inner != right_inner {
        return Err(LinalgError::DimensionMismatch {
            operation: "GEMM inner dimensions",
            left: left_inner,
            right: right_inner,
        });
    }
    if output_rows != m || output_columns != n {
        return Err(LinalgError::OutputShapeMismatch {
            expected_rows: m,
            expected_columns: n,
            actual_rows: output_rows,
            actual_columns: output_columns,
        });
    }
    Ok(GemmDimensions {
        m,
        n,
        k: left_inner,
        left_rows,
        right_rows,
        output_rows,
    })
}

/// Provider abstraction for the linear algebra operations used by release one.
///
/// Native providers can implement this trait behind their own narrow FFI module.
/// Implementations reporting [`ProviderIntegerAbi::Lp64`] must reject dimensions
/// using [`crate::Lp64GemmDimensions`] before calling external code.
pub trait LinalgProvider: Send + Sync {
    /// Stable diagnostic name of this provider implementation.
    fn name(&self) -> &'static str;

    /// Integer convention used by the implementation boundary.
    fn integer_abi(&self) -> ProviderIntegerAbi;

    /// Computes real binary64 GEMM into `output`.
    ///
    /// # Errors
    ///
    /// Returns a validation or provider execution failure.
    fn gemm_f64(
        &self,
        request: GemmRequest<'_, f64>,
        output: &mut DenseArray<f64>,
    ) -> Result<(), LinalgError>;

    /// Computes real binary32 GEMM into `output`.
    ///
    /// Providers predating the binary32 contract receive a structured failure
    /// from this default implementation. Native binary32 providers must
    /// override it without widening values through binary64 storage.
    ///
    /// # Errors
    ///
    /// Returns a validation or provider execution failure.
    fn gemm_f32(
        &self,
        request: GemmRequest<'_, f32>,
        output: &mut DenseArray<f32>,
    ) -> Result<(), LinalgError> {
        let _ = (request, output);
        Err(binary32_not_implemented(self.name(), "f32 GEMM"))
    }

    /// Computes complex binary64 GEMM into `output`.
    ///
    /// # Errors
    ///
    /// Returns a validation or provider execution failure.
    fn gemm_complex64(
        &self,
        request: GemmRequest<'_, Complex64>,
        output: &mut DenseArray<Complex64>,
    ) -> Result<(), LinalgError>;

    /// Computes complex binary32 GEMM into `output`.
    ///
    /// Providers predating the binary32 contract receive a structured failure
    /// from this default implementation. Native binary32 providers must
    /// override it without widening values through binary64 storage.
    ///
    /// # Errors
    ///
    /// Returns a validation or provider execution failure.
    fn gemm_complex32(
        &self,
        request: GemmRequest<'_, Complex32>,
        output: &mut DenseArray<Complex32>,
    ) -> Result<(), LinalgError> {
        let _ = (request, output);
        Err(binary32_not_implemented(self.name(), "complex32 GEMM"))
    }

    /// Computes the unconjugated real dot product of equal-length arrays.
    ///
    /// # Errors
    ///
    /// Returns [`LinalgError::DimensionMismatch`] when element counts differ.
    fn dot_f64(&self, left: &DenseArray<f64>, right: &DenseArray<f64>) -> Result<f64, LinalgError>;

    /// Computes a scaled Euclidean norm of all real elements.
    ///
    /// # Errors
    ///
    /// Returns a provider execution failure if the backend cannot compute the
    /// operation.
    fn norm2_f64(&self, input: &DenseArray<f64>) -> Result<f64, LinalgError>;

    /// Solves the real binary64 general system `A * X = B`.
    ///
    /// Implementations must not modify or share result storage with either
    /// input. The coefficient matrix must be square and all operands use
    /// column-major storage.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, singularity, or
    /// provider execution error.
    fn solve_f64(&self, request: SolveRequest<'_, f64>) -> Result<DenseArray<f64>, LinalgError>;

    /// Solves the real binary32 general system `A * X = B`.
    ///
    /// Implementations must not modify or share result storage with either
    /// input and must not implement this operation by widening to binary64.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, singularity, or
    /// provider execution error.
    fn solve_f32(&self, request: SolveRequest<'_, f32>) -> Result<DenseArray<f32>, LinalgError> {
        let _ = request;
        Err(binary32_not_implemented(self.name(), "f32 general solve"))
    }

    /// Solves the complex binary64 general system `A * X = B`.
    ///
    /// Implementations must not modify or share result storage with either
    /// input. The coefficient matrix must be square and all operands use
    /// column-major storage.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, singularity, or
    /// provider execution error.
    fn solve_complex64(
        &self,
        request: SolveRequest<'_, Complex64>,
    ) -> Result<DenseArray<Complex64>, LinalgError>;

    /// Solves the complex binary32 general system `A * X = B`.
    ///
    /// Implementations must not modify or share result storage with either
    /// input and must not implement this operation by widening to binary64.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, singularity, or
    /// provider execution error.
    fn solve_complex32(
        &self,
        request: SolveRequest<'_, Complex32>,
    ) -> Result<DenseArray<Complex32>, LinalgError> {
        let _ = request;
        Err(binary32_not_implemented(
            self.name(),
            "complex32 general solve",
        ))
    }

    /// Solves a real binary64 square, overdetermined, or underdetermined
    /// full-rank system under the rectangular-solve contract.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, rank-deficiency, or
    /// provider execution error.
    fn solve_rectangular_f64(
        &self,
        request: RectangularSolveRequest<'_, f64>,
    ) -> Result<DenseArray<f64>, LinalgError>;

    /// Solves a real binary32 square, overdetermined, or underdetermined
    /// full-rank system under the rectangular-solve contract.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, rank-deficiency, or
    /// provider execution error.
    fn solve_rectangular_f32(
        &self,
        request: RectangularSolveRequest<'_, f32>,
    ) -> Result<DenseArray<f32>, LinalgError> {
        let _ = request;
        Err(binary32_not_implemented(
            self.name(),
            "f32 rectangular solve",
        ))
    }

    /// Solves a complex binary64 square, overdetermined, or underdetermined
    /// full-rank system under the rectangular-solve contract.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, rank-deficiency, or
    /// provider execution error.
    fn solve_rectangular_complex64(
        &self,
        request: RectangularSolveRequest<'_, Complex64>,
    ) -> Result<DenseArray<Complex64>, LinalgError>;

    /// Solves a complex binary32 square, overdetermined, or underdetermined
    /// full-rank system under the rectangular-solve contract.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, rank-deficiency, or
    /// provider execution error.
    fn solve_rectangular_complex32(
        &self,
        request: RectangularSolveRequest<'_, Complex32>,
    ) -> Result<DenseArray<Complex32>, LinalgError> {
        let _ = request;
        Err(binary32_not_implemented(
            self.name(),
            "complex32 rectangular solve",
        ))
    }

    /// Computes a rectangular real binary32 LU factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn factor_lu_f32(&self, request: FactorRequest<'_, f32>) -> Result<LuResult<f32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "f32 LU factorization",
        ))
    }

    /// Computes a rectangular real binary64 LU factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn factor_lu_f64(&self, request: FactorRequest<'_, f64>) -> Result<LuResult<f64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "f64 LU factorization",
        ))
    }

    /// Computes a rectangular complex binary32 LU factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn factor_lu_complex32(
        &self,
        request: FactorRequest<'_, Complex32>,
    ) -> Result<LuResult<Complex32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex32 LU factorization",
        ))
    }

    /// Computes a rectangular complex binary64 LU factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn factor_lu_complex64(
        &self,
        request: FactorRequest<'_, Complex64>,
    ) -> Result<LuResult<Complex64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex64 LU factorization",
        ))
    }

    /// Computes an explicit real binary32 QR factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn qr_f32(&self, request: QrRequest<'_, f32>) -> Result<QrResult<f32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "f32 QR factorization",
        ))
    }

    /// Computes an explicit real binary64 QR factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn qr_f64(&self, request: QrRequest<'_, f64>) -> Result<QrResult<f64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "f64 QR factorization",
        ))
    }

    /// Computes an explicit complex binary32 QR factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn qr_complex32(
        &self,
        request: QrRequest<'_, Complex32>,
    ) -> Result<QrResult<Complex32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex32 QR factorization",
        ))
    }

    /// Computes an explicit complex binary64 QR factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn qr_complex64(
        &self,
        request: QrRequest<'_, Complex64>,
    ) -> Result<QrResult<Complex64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex64 QR factorization",
        ))
    }

    /// Computes a real binary32 Cholesky factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn cholesky_f32(
        &self,
        request: CholeskyRequest<'_, f32>,
    ) -> Result<CholeskyResult<f32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "f32 Cholesky factorization",
        ))
    }

    /// Computes a real binary64 Cholesky factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn cholesky_f64(
        &self,
        request: CholeskyRequest<'_, f64>,
    ) -> Result<CholeskyResult<f64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "f64 Cholesky factorization",
        ))
    }

    /// Computes a complex binary32 Cholesky factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn cholesky_complex32(
        &self,
        request: CholeskyRequest<'_, Complex32>,
    ) -> Result<CholeskyResult<Complex32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex32 Cholesky factorization",
        ))
    }

    /// Computes a complex binary64 Cholesky factorization.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, or provider error.
    fn cholesky_complex64(
        &self,
        request: CholeskyRequest<'_, Complex64>,
    ) -> Result<CholeskyResult<Complex64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex64 Cholesky factorization",
        ))
    }

    /// Computes a real binary32 singular value decomposition.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn svd_f32(&self, request: SvdRequest<'_, f32>) -> Result<SvdResult<f32, f32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(self.name(), "f32 SVD"))
    }

    /// Computes a real binary64 singular value decomposition.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn svd_f64(&self, request: SvdRequest<'_, f64>) -> Result<SvdResult<f64, f64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(self.name(), "f64 SVD"))
    }

    /// Computes a complex binary32 singular value decomposition.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn svd_complex32(
        &self,
        request: SvdRequest<'_, Complex32>,
    ) -> Result<SvdResult<Complex32, f32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(self.name(), "complex32 SVD"))
    }

    /// Computes a complex binary64 singular value decomposition.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn svd_complex64(
        &self,
        request: SvdRequest<'_, Complex64>,
    ) -> Result<SvdResult<Complex64, f64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(self.name(), "complex64 SVD"))
    }

    /// Computes real binary32 general eigenvalues and optional eigenvectors.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn eig_f32(&self, request: EigRequest<'_, f32>) -> Result<EigResult<Complex32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "f32 general eig",
        ))
    }

    /// Computes real binary64 general eigenvalues and optional eigenvectors.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn eig_f64(&self, request: EigRequest<'_, f64>) -> Result<EigResult<Complex64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "f64 general eig",
        ))
    }

    /// Computes complex binary32 general eigenvalues and optional eigenvectors.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn eig_complex32(
        &self,
        request: EigRequest<'_, Complex32>,
    ) -> Result<EigResult<Complex32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex32 general eig",
        ))
    }

    /// Computes complex binary64 general eigenvalues and optional eigenvectors.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn eig_complex64(
        &self,
        request: EigRequest<'_, Complex64>,
    ) -> Result<EigResult<Complex64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex64 general eig",
        ))
    }

    /// Computes a complex binary32 Schur factorization `A = Q*T*Q^H`.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn schur_complex32(
        &self,
        request: SchurRequest<'_, Complex32>,
    ) -> Result<SchurResult<Complex32>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex32 Schur decomposition",
        ))
    }

    /// Computes a complex binary64 Schur factorization `A = Q*T*Q^H`.
    ///
    /// # Errors
    ///
    /// Returns a validation, cancellation, allocation, convergence, or provider error.
    fn schur_complex64(
        &self,
        request: SchurRequest<'_, Complex64>,
    ) -> Result<SchurResult<Complex64>, LinalgError> {
        let _ = request;
        Err(decomposition_not_implemented(
            self.name(),
            "complex64 Schur decomposition",
        ))
    }
}

fn binary32_not_implemented(provider: &'static str, operation: &'static str) -> LinalgError {
    LinalgError::ProviderFailure {
        provider,
        operation,
        detail: "provider does not implement the native binary32 contract".to_owned(),
    }
}

fn decomposition_not_implemented(provider: &'static str, operation: &'static str) -> LinalgError {
    LinalgError::ProviderFailure {
        provider,
        operation,
        detail: "provider does not implement the decomposition contract".to_owned(),
    }
}

/// Allocates and computes an ordinary real binary32 matrix product.
///
/// # Errors
///
/// Returns a shape, allocation, validation, or provider failure.
pub fn matrix_multiply_f32(
    provider: &dyn LinalgProvider,
    left: &DenseArray<f32>,
    right: &DenseArray<f32>,
) -> Result<DenseArray<f32>, LinalgError> {
    let (m, inner) = matrix_dimensions("left matrix-multiply operand", left)?;
    let (right_inner, n) = matrix_dimensions("right matrix-multiply operand", right)?;
    if inner != right_inner {
        return Err(LinalgError::DimensionMismatch {
            operation: "matrix multiplication inner dimensions",
            left: inner,
            right: right_inner,
        });
    }
    let mut output = DenseArray::from_elem(Shape::new([m, n])?, 0.0_f32)?;
    let request = GemmRequest::new(
        left,
        right,
        GemmOptions::new(MatrixTranspose::None, MatrixTranspose::None, 1.0, 0.0),
    );
    provider.gemm_f32(request, &mut output)?;
    Ok(output)
}

/// Allocates and computes an ordinary complex binary32 matrix product.
///
/// # Errors
///
/// Returns a shape, allocation, validation, or provider failure.
pub fn matrix_multiply_complex32(
    provider: &dyn LinalgProvider,
    left: &DenseArray<Complex32>,
    right: &DenseArray<Complex32>,
) -> Result<DenseArray<Complex32>, LinalgError> {
    let (m, inner) = matrix_dimensions("left matrix-multiply operand", left)?;
    let (right_inner, n) = matrix_dimensions("right matrix-multiply operand", right)?;
    if inner != right_inner {
        return Err(LinalgError::DimensionMismatch {
            operation: "matrix multiplication inner dimensions",
            left: inner,
            right: right_inner,
        });
    }
    let mut output = DenseArray::from_elem(Shape::new([m, n])?, Complex32::ZERO)?;
    let request = GemmRequest::new(
        left,
        right,
        GemmOptions::new(
            MatrixTranspose::None,
            MatrixTranspose::None,
            Complex32::new(1.0, 0.0),
            Complex32::ZERO,
        ),
    );
    provider.gemm_complex32(request, &mut output)?;
    Ok(output)
}

/// Allocates and computes an ordinary real matrix product.
///
/// # Errors
///
/// Returns a shape, allocation, validation, or provider failure.
pub fn matrix_multiply_f64(
    provider: &dyn LinalgProvider,
    left: &DenseArray<f64>,
    right: &DenseArray<f64>,
) -> Result<DenseArray<f64>, LinalgError> {
    let (m, inner) = matrix_dimensions("left matrix-multiply operand", left)?;
    let (right_inner, n) = matrix_dimensions("right matrix-multiply operand", right)?;
    if inner != right_inner {
        return Err(LinalgError::DimensionMismatch {
            operation: "matrix multiplication inner dimensions",
            left: inner,
            right: right_inner,
        });
    }
    let mut output = DenseArray::from_elem(Shape::new([m, n])?, 0.0)?;
    let request = GemmRequest::new(
        left,
        right,
        GemmOptions::new(MatrixTranspose::None, MatrixTranspose::None, 1.0, 0.0),
    );
    provider.gemm_f64(request, &mut output)?;
    Ok(output)
}

/// Allocates and computes an ordinary complex matrix product.
///
/// # Errors
///
/// Returns a shape, allocation, validation, or provider failure.
pub fn matrix_multiply_complex64(
    provider: &dyn LinalgProvider,
    left: &DenseArray<Complex64>,
    right: &DenseArray<Complex64>,
) -> Result<DenseArray<Complex64>, LinalgError> {
    let (m, inner) = matrix_dimensions("left matrix-multiply operand", left)?;
    let (right_inner, n) = matrix_dimensions("right matrix-multiply operand", right)?;
    if inner != right_inner {
        return Err(LinalgError::DimensionMismatch {
            operation: "matrix multiplication inner dimensions",
            left: inner,
            right: right_inner,
        });
    }
    let mut output = DenseArray::from_elem(Shape::new([m, n])?, Complex64::ZERO)?;
    let request = GemmRequest::new(
        left,
        right,
        GemmOptions::new(
            MatrixTranspose::None,
            MatrixTranspose::None,
            Complex64::new(1.0, 0.0),
            Complex64::ZERO,
        ),
    );
    provider.gemm_complex64(request, &mut output)?;
    Ok(output)
}

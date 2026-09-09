use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::{Complex32, Complex64, DenseArray};

use crate::{LinalgError, LinalgProvider, matrix_dimensions};

/// Borrowed matrix and optional cancellation for LU factorization.
#[derive(Clone, Copy, Debug)]
pub struct FactorRequest<'array, T> {
    /// Matrix to factor, in contiguous column-major storage.
    pub matrix: &'array DenseArray<T>,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, T> FactorRequest<'array, T> {
    /// Constructs an LU request without a cancellation flag.
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

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }

    /// Reports a pre-existing or cooperatively observed cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`LinalgError::Cancelled`] when the attached flag is set.
    pub fn check_cancellation(&self) -> Result<(), LinalgError> {
        check_cancellation(self.is_cancelled(), "LU factorization")
    }
}

/// Parity of the row interchanges performed during LU factorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapParity {
    /// An even number of row interchanges, including zero.
    Even,
    /// An odd number of row interchanges.
    Odd,
}

impl SwapParity {
    pub(crate) fn toggle(&mut self) {
        *self = match *self {
            Self::Even => Self::Odd,
            Self::Odd => Self::Even,
        };
    }
}

/// Owned, provider-neutral LU factorization result.
///
/// `packed_lu` has the input shape. Its strict lower trapezoid stores unit-
/// diagonal `L` multipliers and its upper trapezoid stores `U`.
#[derive(Clone, Debug, PartialEq)]
pub struct LuResult<T> {
    /// Rectangular packed LU factors in column-major storage.
    pub packed_lu: DenseArray<T>,
    /// Final row order in `P * A = L * U`, expressed as zero-based original rows.
    pub row_permutation_zero_based: Vec<u64>,
    /// Parity of the row permutation.
    pub swap_parity: SwapParity,
    /// First exact zero pivot, expressed as a zero-based diagonal index.
    pub first_zero_pivot: Option<u64>,
}

/// Whether a QR request materializes a full or economy-size orthogonal factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QrVectors {
    /// Materialize `Q` with shape `m x m` and `R` with shape `m x n`.
    Full,
    /// Materialize `Q` with shape `m x min(m,n)` and matching thin `R`.
    Thin,
}

/// Borrowed matrix and options for QR factorization.
#[derive(Clone, Copy, Debug)]
pub struct QrRequest<'array, T> {
    /// Matrix to factor, in contiguous column-major storage.
    pub matrix: &'array DenseArray<T>,
    /// Requested shape of the explicit orthogonal/unitary factor.
    pub vectors: QrVectors,
    /// Whether to select columns by rank-revealing column pivoting.
    pub pivot_columns: bool,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, T> QrRequest<'array, T> {
    /// Constructs an unpivoted QR request without a cancellation flag.
    #[must_use]
    pub const fn new(matrix: &'array DenseArray<T>, vectors: QrVectors) -> Self {
        Self {
            matrix,
            vectors,
            pivot_columns: false,
            cancellation: None,
        }
    }

    /// Selects whether QR uses column pivoting.
    #[must_use]
    pub const fn with_column_pivoting(mut self, pivot_columns: bool) -> Self {
        self.pivot_columns = pivot_columns;
        self
    }

    /// Attaches a cooperative cancellation flag.
    #[must_use]
    pub const fn with_cancellation_flag(mut self, cancellation: &'array AtomicBool) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }

    /// Reports a pre-existing or cooperatively observed cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`LinalgError::Cancelled`] when the attached flag is set.
    pub fn check_cancellation(&self) -> Result<(), LinalgError> {
        check_cancellation(self.is_cancelled(), "QR factorization")
    }
}

/// Owned, provider-neutral explicit QR factorization result.
#[derive(Clone, Debug, PartialEq)]
pub struct QrResult<T> {
    /// Full or thin explicit orthogonal/unitary factor.
    pub q: DenseArray<T>,
    /// Matching upper trapezoidal factor.
    pub r: DenseArray<T>,
    /// Zero-based original column at each factorized position.
    ///
    /// This is the identity permutation for an unpivoted request and otherwise
    /// satisfies `A(:, permutation) = Q * R`.
    pub column_permutation_zero_based: Vec<u64>,
    /// Numerical rank estimated from the factor diagonal in the input precision.
    pub numerical_rank: u64,
    /// First diagonal below the same numerical-rank threshold, as a zero-based index.
    pub first_rank_deficient_diagonal: Option<u64>,
}

/// Triangle selected for Cholesky input and output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CholeskyTriangle {
    /// Read the upper triangle and return an upper-triangular factor `R`.
    Upper,
    /// Read the lower triangle and return a lower-triangular factor `L`.
    Lower,
}

/// Borrowed matrix and options for Cholesky factorization.
#[derive(Clone, Copy, Debug)]
pub struct CholeskyRequest<'array, T> {
    /// Square matrix to factor, in contiguous column-major storage.
    pub matrix: &'array DenseArray<T>,
    /// Triangle that contains the Hermitian input and receives the factor.
    pub triangle: CholeskyTriangle,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, T> CholeskyRequest<'array, T> {
    /// Constructs a Cholesky request without a cancellation flag.
    #[must_use]
    pub const fn new(matrix: &'array DenseArray<T>, triangle: CholeskyTriangle) -> Self {
        Self {
            matrix,
            triangle,
            cancellation: None,
        }
    }

    /// Attaches a cooperative cancellation flag.
    #[must_use]
    pub const fn with_cancellation_flag(mut self, cancellation: &'array AtomicBool) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }

    /// Reports a pre-existing or cooperatively observed cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`LinalgError::Cancelled`] when the attached flag is set.
    pub fn check_cancellation(&self) -> Result<(), LinalgError> {
        check_cancellation(self.is_cancelled(), "Cholesky factorization")
    }
}

/// Owned Cholesky factor plus non-positive-definite data status.
#[derive(Clone, Debug, PartialEq)]
pub struct CholeskyResult<T> {
    /// Requested triangular factor. Entries outside the successful leading
    /// block and outside the selected triangle are exact zeros.
    pub factor: DenseArray<T>,
    /// First non-positive-definite leading minor, as a zero-based index.
    pub first_non_positive_minor: Option<u64>,
}

/// Validated dimensions for rectangular LU or QR factorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactorDimensions {
    rows: u64,
    columns: u64,
}

impl FactorDimensions {
    /// Matrix row count.
    #[must_use]
    pub const fn rows(self) -> u64 {
        self.rows
    }

    /// Matrix column count.
    #[must_use]
    pub const fn columns(self) -> u64 {
        self.columns
    }

    /// Number of diagonal pivots or Householder reflectors.
    #[must_use]
    pub const fn order(self) -> u64 {
        if self.rows < self.columns {
            self.rows
        } else {
            self.columns
        }
    }

    /// Column-major leading dimension of the matrix.
    #[must_use]
    pub fn leading_dimension(self) -> u64 {
        self.rows.max(1)
    }

    /// Explicit `Q` column count for the requested vector shape.
    #[must_use]
    pub const fn q_columns(self, vectors: QrVectors) -> u64 {
        match vectors {
            QrVectors::Full => self.rows,
            QrVectors::Thin => self.order(),
        }
    }
}

/// Validated dimensions for square Cholesky factorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CholeskyDimensions {
    order: u64,
}

impl CholeskyDimensions {
    /// Matrix order.
    #[must_use]
    pub const fn order(self) -> u64 {
        self.order
    }

    /// Column-major leading dimension.
    #[must_use]
    pub fn leading_dimension(self) -> u64 {
        self.order.max(1)
    }
}

/// Validates an LU factorization input.
///
/// # Errors
///
/// Returns [`LinalgError::MatrixRequired`] for a significant rank above two.
pub fn validate_factor<T>(request: &FactorRequest<'_, T>) -> Result<FactorDimensions, LinalgError> {
    let (rows, columns) = matrix_dimensions("LU factorization matrix", request.matrix)?;
    Ok(FactorDimensions { rows, columns })
}

/// Validates a QR factorization input.
///
/// # Errors
///
/// Returns [`LinalgError::MatrixRequired`] for a significant rank above two.
pub fn validate_qr<T>(request: &QrRequest<'_, T>) -> Result<FactorDimensions, LinalgError> {
    let (rows, columns) = matrix_dimensions("QR factorization matrix", request.matrix)?;
    Ok(FactorDimensions { rows, columns })
}

/// Validates a square Cholesky factorization input.
///
/// # Errors
///
/// Returns a matrix-rank error or [`LinalgError::SquareMatrixRequired`].
pub fn validate_cholesky<T>(
    request: &CholeskyRequest<'_, T>,
) -> Result<CholeskyDimensions, LinalgError> {
    let (rows, columns) = matrix_dimensions("Cholesky factorization matrix", request.matrix)?;
    if rows != columns {
        return Err(LinalgError::SquareMatrixRequired {
            operand: "Cholesky factorization matrix",
            rows,
            columns,
        });
    }
    Ok(CholeskyDimensions { order: rows })
}

macro_rules! provider_helper {
    ($name:ident, $method:ident, $request:ident, $result:ident, $scalar:ty) => {
        #[doc = concat!("Runs provider-neutral `", stringify!($method), "` dispatch.")]
        ///
        /// # Errors
        ///
        /// Returns a validation, cancellation, allocation, or provider error.
        pub fn $name(
            provider: &dyn LinalgProvider,
            request: $request<'_, $scalar>,
        ) -> Result<$result<$scalar>, LinalgError> {
            provider.$method(request)
        }
    };
}

provider_helper!(factor_lu_f32, factor_lu_f32, FactorRequest, LuResult, f32);
provider_helper!(factor_lu_f64, factor_lu_f64, FactorRequest, LuResult, f64);
provider_helper!(
    factor_lu_complex32,
    factor_lu_complex32,
    FactorRequest,
    LuResult,
    Complex32
);
provider_helper!(
    factor_lu_complex64,
    factor_lu_complex64,
    FactorRequest,
    LuResult,
    Complex64
);
provider_helper!(qr_f32, qr_f32, QrRequest, QrResult, f32);
provider_helper!(qr_f64, qr_f64, QrRequest, QrResult, f64);
provider_helper!(qr_complex32, qr_complex32, QrRequest, QrResult, Complex32);
provider_helper!(qr_complex64, qr_complex64, QrRequest, QrResult, Complex64);
provider_helper!(
    cholesky_f32,
    cholesky_f32,
    CholeskyRequest,
    CholeskyResult,
    f32
);
provider_helper!(
    cholesky_f64,
    cholesky_f64,
    CholeskyRequest,
    CholeskyResult,
    f64
);
provider_helper!(
    cholesky_complex32,
    cholesky_complex32,
    CholeskyRequest,
    CholeskyResult,
    Complex32
);
provider_helper!(
    cholesky_complex64,
    cholesky_complex64,
    CholeskyRequest,
    CholeskyResult,
    Complex64
);

fn check_cancellation(cancelled: bool, operation: &'static str) -> Result<(), LinalgError> {
    if cancelled {
        Err(LinalgError::Cancelled { operation })
    } else {
        Ok(())
    }
}

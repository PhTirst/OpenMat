use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::{Complex32, Complex64, DenseArray};

use crate::{LinalgError, LinalgProvider, matrix_dimensions};

/// Singular-vector materialization requested from an SVD provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvdVectors {
    /// Return singular values without allocating singular vectors.
    None,
    /// Return economy-size `U` and `Vh`.
    Thin,
    /// Return square `U` and `Vh` factors.
    Full,
}

/// Borrowed matrix and options for singular value decomposition.
#[derive(Clone, Copy, Debug)]
pub struct SvdRequest<'array, T> {
    /// Matrix in contiguous column-major storage.
    pub matrix: &'array DenseArray<T>,
    /// Singular-vector shape to materialize.
    pub vectors: SvdVectors,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, T> SvdRequest<'array, T> {
    /// Constructs an SVD request without a cancellation flag.
    #[must_use]
    pub const fn new(matrix: &'array DenseArray<T>, vectors: SvdVectors) -> Self {
        Self {
            matrix,
            vectors,
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
        check_cancellation(self.is_cancelled(), "singular value decomposition")
    }
}

/// Owned provider-neutral singular value decomposition.
#[derive(Clone, Debug, PartialEq)]
pub struct SvdResult<T, R> {
    /// Descending nonnegative singular values, shaped `min(m,n) x 1`.
    pub singular_values: DenseArray<R>,
    /// Optional left singular vectors.
    pub u: Option<DenseArray<T>>,
    /// Optional conjugate-transposed right singular vectors.
    pub vh: Option<DenseArray<T>>,
}

/// Borrowed matrix and vector options for a general eigenvalue decomposition.
#[derive(Clone, Copy, Debug)]
pub struct EigRequest<'array, T> {
    /// Square matrix in contiguous column-major storage.
    pub matrix: &'array DenseArray<T>,
    /// Whether to return left eigenvectors in columns.
    pub left_vectors: bool,
    /// Whether to return right eigenvectors in columns.
    pub right_vectors: bool,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, T> EigRequest<'array, T> {
    /// Constructs a values-only eig request.
    #[must_use]
    pub const fn new(matrix: &'array DenseArray<T>) -> Self {
        Self {
            matrix,
            left_vectors: false,
            right_vectors: false,
            cancellation: None,
        }
    }

    /// Selects whether left eigenvectors are materialized.
    #[must_use]
    pub const fn with_left_vectors(mut self, enabled: bool) -> Self {
        self.left_vectors = enabled;
        self
    }

    /// Selects whether right eigenvectors are materialized.
    #[must_use]
    pub const fn with_right_vectors(mut self, enabled: bool) -> Self {
        self.right_vectors = enabled;
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
        check_cancellation(self.is_cancelled(), "general eigenvalue decomposition")
    }
}

/// Owned provider-neutral general eigenvalue decomposition.
#[derive(Clone, Debug, PartialEq)]
pub struct EigResult<C> {
    /// Complex eigenvalues, shaped `n x 1`.
    pub eigenvalues: DenseArray<C>,
    /// Optional left eigenvectors in columns, satisfying `u^H A = lambda u^H`.
    pub left_vectors: Option<DenseArray<C>>,
    /// Optional right eigenvectors in columns, satisfying `A v = lambda v`.
    pub right_vectors: Option<DenseArray<C>>,
}

/// Checked matrix dimensions shared by SVD and eig providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpectralDimensions {
    rows: u64,
    columns: u64,
}

impl SpectralDimensions {
    pub(crate) const fn square(order: u64) -> Self {
        Self {
            rows: order,
            columns: order,
        }
    }

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
    /// Number of singular values.
    #[must_use]
    pub const fn order(self) -> u64 {
        if self.rows < self.columns {
            self.rows
        } else {
            self.columns
        }
    }
    /// Column-major leading dimension.
    #[must_use]
    pub fn leading_dimension(self) -> u64 {
        self.rows.max(1)
    }
}

/// Validates a matrix accepted by SVD.
///
/// # Errors
///
/// Returns [`LinalgError::MatrixRequired`] for a significant rank above two.
pub fn validate_svd<T>(request: &SvdRequest<'_, T>) -> Result<SpectralDimensions, LinalgError> {
    let (rows, columns) = matrix_dimensions("SVD matrix", request.matrix)?;
    Ok(SpectralDimensions { rows, columns })
}

/// Validates a square matrix accepted by general eig.
///
/// # Errors
///
/// Returns a matrix-rank error or [`LinalgError::SquareMatrixRequired`].
pub fn validate_eig<T>(request: &EigRequest<'_, T>) -> Result<SpectralDimensions, LinalgError> {
    let (rows, columns) = matrix_dimensions("eigenvalue matrix", request.matrix)?;
    if rows != columns {
        return Err(LinalgError::SquareMatrixRequired {
            operand: "eigenvalue matrix",
            rows,
            columns,
        });
    }
    Ok(SpectralDimensions { rows, columns })
}

macro_rules! svd_helper {
    ($name:ident, $method:ident, $scalar:ty, $real:ty) => {
        #[doc = concat!("Runs provider-neutral `", stringify!($method), "` dispatch.")]
        ///
        /// # Errors
        ///
        /// Returns a validation, cancellation, allocation, convergence, or provider error.
        pub fn $name(
            provider: &dyn LinalgProvider,
            request: SvdRequest<'_, $scalar>,
        ) -> Result<SvdResult<$scalar, $real>, LinalgError> {
            provider.$method(request)
        }
    };
}
macro_rules! eig_helper {
    ($name:ident, $method:ident, $input:ty, $complex:ty) => {
        #[doc = concat!("Runs provider-neutral `", stringify!($method), "` dispatch.")]
        ///
        /// # Errors
        ///
        /// Returns a validation, cancellation, allocation, convergence, or provider error.
        pub fn $name(
            provider: &dyn LinalgProvider,
            request: EigRequest<'_, $input>,
        ) -> Result<EigResult<$complex>, LinalgError> {
            provider.$method(request)
        }
    };
}
svd_helper!(svd_f32, svd_f32, f32, f32);
svd_helper!(svd_f64, svd_f64, f64, f64);
svd_helper!(svd_complex32, svd_complex32, Complex32, f32);
svd_helper!(svd_complex64, svd_complex64, Complex64, f64);
eig_helper!(eig_f32, eig_f32, f32, Complex32);
eig_helper!(eig_f64, eig_f64, f64, Complex64);
eig_helper!(eig_complex32, eig_complex32, Complex32, Complex32);
eig_helper!(eig_complex64, eig_complex64, Complex64, Complex64);

fn check_cancellation(cancelled: bool, operation: &'static str) -> Result<(), LinalgError> {
    if cancelled {
        Err(LinalgError::Cancelled { operation })
    } else {
        Ok(())
    }
}

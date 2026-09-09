use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::{Complex32, Complex64, DenseArray};

use crate::{LinalgError, LinalgProvider, SpectralDimensions, matrix_dimensions};

/// Borrowed square complex matrix for a Schur decomposition.
#[derive(Clone, Copy, Debug)]
pub struct SchurRequest<'array, C> {
    /// Matrix in contiguous column-major storage.
    pub matrix: &'array DenseArray<C>,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, C> SchurRequest<'array, C> {
    /// Constructs a Schur request without a cancellation flag.
    #[must_use]
    pub const fn new(matrix: &'array DenseArray<C>) -> Self {
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

    /// Reports a pre-existing or cooperatively observed cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`LinalgError::Cancelled`] when the attached flag is set.
    pub fn check_cancellation(&self) -> Result<(), LinalgError> {
        if self
            .cancellation
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            Err(LinalgError::Cancelled {
                operation: "complex Schur decomposition",
            })
        } else {
            Ok(())
        }
    }
}

/// Owned complex Schur factorization `A = Q*T*Q^H`.
#[derive(Clone, Debug, PartialEq)]
pub struct SchurResult<C> {
    /// Upper-triangular Schur form `T`.
    pub form: DenseArray<C>,
    /// Unitary Schur vectors `Q`.
    pub vectors: DenseArray<C>,
}

/// Validates a square complex matrix accepted by Schur providers.
///
/// # Errors
///
/// Returns a matrix-rank error or [`LinalgError::SquareMatrixRequired`].
pub fn validate_schur<C>(request: &SchurRequest<'_, C>) -> Result<SpectralDimensions, LinalgError> {
    let (rows, columns) = matrix_dimensions("Schur matrix", request.matrix)?;
    if rows != columns {
        return Err(LinalgError::SquareMatrixRequired {
            operand: "Schur matrix",
            rows,
            columns,
        });
    }
    Ok(SpectralDimensions::square(rows))
}

/// Runs provider-neutral complex binary32 Schur dispatch.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, convergence, or provider error.
pub fn schur_complex32(
    provider: &dyn LinalgProvider,
    request: SchurRequest<'_, Complex32>,
) -> Result<SchurResult<Complex32>, LinalgError> {
    provider.schur_complex32(request)
}

/// Runs provider-neutral complex binary64 Schur dispatch.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, convergence, or provider error.
pub fn schur_complex64(
    provider: &dyn LinalgProvider,
    request: SchurRequest<'_, Complex64>,
) -> Result<SchurResult<Complex64>, LinalgError> {
    provider.schur_complex64(request)
}

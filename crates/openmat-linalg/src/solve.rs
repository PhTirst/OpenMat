use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::{Complex32, Complex64, DenseArray};

use crate::{LinalgError, LinalgProvider, matrix_dimensions};

/// Borrowed inputs and optional cooperative cancellation for `A * X = B`.
///
/// Both operands use contiguous column-major storage. Providers must treat the
/// inputs as immutable and return an independently owned result.
#[derive(Clone, Copy, Debug)]
pub struct SolveRequest<'array, T> {
    /// Square coefficient matrix `A`.
    pub coefficients: &'array DenseArray<T>,
    /// Right-hand-side matrix `B`.
    pub right_hand_side: &'array DenseArray<T>,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, T> SolveRequest<'array, T> {
    /// Constructs a solve request without a cancellation flag.
    #[must_use]
    pub const fn new(
        coefficients: &'array DenseArray<T>,
        right_hand_side: &'array DenseArray<T>,
    ) -> Self {
        Self {
            coefficients,
            right_hand_side,
            cancellation: None,
        }
    }

    /// Attaches a cooperative cancellation flag to this request.
    ///
    /// The reference provider checks the flag during factorization and
    /// substitution. Native providers check it immediately before and after a
    /// blocking native call, because LAPACK itself is not cooperatively
    /// interruptible through this interface.
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

    /// Converts an observed cancellation request to a structured error.
    ///
    /// # Errors
    ///
    /// Returns [`LinalgError::Cancelled`] when the attached flag is set.
    pub fn check_cancellation(&self) -> Result<(), LinalgError> {
        if self.is_cancelled() {
            Err(LinalgError::Cancelled {
                operation: "general linear solve",
            })
        } else {
            Ok(())
        }
    }
}

/// Validated dimensions for a square general linear solve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolveDimensions {
    order: u64,
    right_hand_sides: u64,
}

impl SolveDimensions {
    /// Order `n` of the square coefficient matrix.
    #[must_use]
    pub const fn order(self) -> u64 {
        self.order
    }

    /// Number of right-hand sides (`nrhs`).
    #[must_use]
    pub const fn right_hand_sides(self) -> u64 {
        self.right_hand_sides
    }

    /// Column-major leading dimension of the coefficient matrix.
    #[must_use]
    pub fn coefficient_leading_dimension(self) -> u64 {
        self.order.max(1)
    }

    /// Column-major leading dimension of the right-hand-side matrix.
    #[must_use]
    pub fn right_hand_side_leading_dimension(self) -> u64 {
        self.order.max(1)
    }
}

/// Validates the matrix ranks and shapes for `A * X = B`.
///
/// # Errors
///
/// Returns a matrix-rank error, [`LinalgError::SquareMatrixRequired`], or a
/// row-count [`LinalgError::DimensionMismatch`].
pub fn validate_solve<T>(request: &SolveRequest<'_, T>) -> Result<SolveDimensions, LinalgError> {
    let (coefficient_rows, coefficient_columns) =
        matrix_dimensions("linear solve coefficient matrix", request.coefficients)?;
    let (right_hand_side_rows, right_hand_sides) =
        matrix_dimensions("linear solve right-hand side", request.right_hand_side)?;
    if coefficient_rows != coefficient_columns {
        return Err(LinalgError::SquareMatrixRequired {
            operand: "linear solve coefficient matrix",
            rows: coefficient_rows,
            columns: coefficient_columns,
        });
    }
    if coefficient_rows != right_hand_side_rows {
        return Err(LinalgError::DimensionMismatch {
            operation: "linear solve coefficient and right-hand-side rows",
            left: coefficient_rows,
            right: right_hand_side_rows,
        });
    }
    Ok(SolveDimensions {
        order: coefficient_rows,
        right_hand_sides,
    })
}

/// Solves the real binary64 system `A * X = B` using `provider`.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, singularity, or provider
/// execution error.
pub fn solve_f64(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<f64>,
    right_hand_side: &DenseArray<f64>,
) -> Result<DenseArray<f64>, LinalgError> {
    provider.solve_f64(SolveRequest::new(coefficients, right_hand_side))
}

/// Solves the real binary32 system `A * X = B` using `provider`.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, singularity, or provider
/// execution error.
pub fn solve_f32(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<f32>,
    right_hand_side: &DenseArray<f32>,
) -> Result<DenseArray<f32>, LinalgError> {
    provider.solve_f32(SolveRequest::new(coefficients, right_hand_side))
}

/// Solves the complex binary64 system `A * X = B` using `provider`.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, singularity, or provider
/// execution error.
pub fn solve_complex64(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<Complex64>,
    right_hand_side: &DenseArray<Complex64>,
) -> Result<DenseArray<Complex64>, LinalgError> {
    provider.solve_complex64(SolveRequest::new(coefficients, right_hand_side))
}

/// Solves the complex binary32 system `A * X = B` using `provider`.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, singularity, or provider
/// execution error.
pub fn solve_complex32(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<Complex32>,
    right_hand_side: &DenseArray<Complex32>,
) -> Result<DenseArray<Complex32>, LinalgError> {
    provider.solve_complex32(SolveRequest::new(coefficients, right_hand_side))
}

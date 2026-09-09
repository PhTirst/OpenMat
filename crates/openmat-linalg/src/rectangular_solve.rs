use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::{Complex32, Complex64, DenseArray};

use crate::{LinalgError, LinalgProvider, matrix_dimensions};

/// Shape-selected meaning of a rectangular solve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectangularSolveKind {
    /// `m == n`: the unique solution of a full-rank square system.
    Square,
    /// `m > n`: the full-column-rank solution minimizing `||A * X - B||_2`.
    OverdeterminedLeastSquares,
    /// `m < n`: the minimum-Euclidean-norm solution of a full-row-rank system.
    UnderdeterminedMinimumNorm,
}

/// Borrowed immutable inputs for a full-rank rectangular solve.
///
/// `A` is `m x n`, `B` is `m x nrhs`, and the independently owned result is
/// always `n x nrhs`. The relative values of `m` and `n` select the precise
/// semantics described by [`RectangularSolveKind`].
#[derive(Clone, Copy, Debug)]
pub struct RectangularSolveRequest<'array, T> {
    /// Coefficient matrix `A` in contiguous column-major storage.
    pub coefficients: &'array DenseArray<T>,
    /// Right-hand-side matrix `B` in contiguous column-major storage.
    pub right_hand_side: &'array DenseArray<T>,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, T> RectangularSolveRequest<'array, T> {
    /// Constructs a request without a cancellation flag.
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

    /// Attaches a cooperative cancellation flag.
    ///
    /// Reference code checks during factorization and substitution. Native
    /// providers check immediately before and after a blocking LAPACK call.
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
                operation: "rectangular linear solve",
            })
        } else {
            Ok(())
        }
    }
}

/// Provider-neutral dimensions and shape semantics for a rectangular solve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectangularSolveDimensions {
    coefficient_rows: u64,
    coefficient_columns: u64,
    right_hand_sides: u64,
    kind: RectangularSolveKind,
}

impl RectangularSolveDimensions {
    /// Rows `m` in `A` and `B`.
    #[must_use]
    pub const fn coefficient_rows(self) -> u64 {
        self.coefficient_rows
    }

    /// Columns `n` in `A` and rows in the result.
    #[must_use]
    pub const fn coefficient_columns(self) -> u64 {
        self.coefficient_columns
    }

    /// Number of right-hand sides (`nrhs`).
    #[must_use]
    pub const fn right_hand_sides(self) -> u64 {
        self.right_hand_sides
    }

    /// Shape-selected mathematical meaning of the request.
    #[must_use]
    pub const fn kind(self) -> RectangularSolveKind {
        self.kind
    }

    /// Full rank required by this contract (`min(m, n)`).
    #[must_use]
    pub const fn required_rank(self) -> u64 {
        if self.coefficient_rows < self.coefficient_columns {
            self.coefficient_rows
        } else {
            self.coefficient_columns
        }
    }

    /// Column-major leading dimension of `A`.
    #[must_use]
    pub fn coefficient_leading_dimension(self) -> u64 {
        self.coefficient_rows.max(1)
    }

    /// Column-major leading dimension of the LAPACK-compatible padded `B`
    /// workspace.
    #[must_use]
    pub fn workspace_leading_dimension(self) -> u64 {
        self.coefficient_rows.max(self.coefficient_columns).max(1)
    }
}

/// Validates matrix ranks and the shared `m` dimension, then classifies shape.
///
/// # Errors
///
/// Returns a matrix-rank error or a row-count
/// [`LinalgError::DimensionMismatch`].
pub fn validate_rectangular_solve<T>(
    request: &RectangularSolveRequest<'_, T>,
) -> Result<RectangularSolveDimensions, LinalgError> {
    let (coefficient_rows, coefficient_columns) =
        matrix_dimensions("rectangular solve coefficient matrix", request.coefficients)?;
    let (right_hand_side_rows, right_hand_sides) =
        matrix_dimensions("rectangular solve right-hand side", request.right_hand_side)?;
    if coefficient_rows != right_hand_side_rows {
        return Err(LinalgError::DimensionMismatch {
            operation: "rectangular solve coefficient and right-hand-side rows",
            left: coefficient_rows,
            right: right_hand_side_rows,
        });
    }
    let kind = match coefficient_rows.cmp(&coefficient_columns) {
        std::cmp::Ordering::Equal => RectangularSolveKind::Square,
        std::cmp::Ordering::Greater => RectangularSolveKind::OverdeterminedLeastSquares,
        std::cmp::Ordering::Less => RectangularSolveKind::UnderdeterminedMinimumNorm,
    };
    Ok(RectangularSolveDimensions {
        coefficient_rows,
        coefficient_columns,
        right_hand_sides,
        kind,
    })
}

/// Solves a real full-rank rectangular system using `provider`.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, rank-deficiency, or
/// provider execution error.
pub fn solve_rectangular_f64(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<f64>,
    right_hand_side: &DenseArray<f64>,
) -> Result<DenseArray<f64>, LinalgError> {
    provider.solve_rectangular_f64(RectangularSolveRequest::new(coefficients, right_hand_side))
}

/// Solves a real binary32 full-rank rectangular system using `provider`.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, rank-deficiency, or
/// provider execution error.
pub fn solve_rectangular_f32(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<f32>,
    right_hand_side: &DenseArray<f32>,
) -> Result<DenseArray<f32>, LinalgError> {
    provider.solve_rectangular_f32(RectangularSolveRequest::new(coefficients, right_hand_side))
}

/// Solves a complex full-rank rectangular system using `provider`.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, rank-deficiency, or
/// provider execution error.
pub fn solve_rectangular_complex64(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<Complex64>,
    right_hand_side: &DenseArray<Complex64>,
) -> Result<DenseArray<Complex64>, LinalgError> {
    provider
        .solve_rectangular_complex64(RectangularSolveRequest::new(coefficients, right_hand_side))
}

/// Solves a complex binary32 full-rank rectangular system using `provider`.
///
/// # Errors
///
/// Returns a validation, cancellation, allocation, rank-deficiency, or
/// provider execution error.
pub fn solve_rectangular_complex32(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<Complex32>,
    right_hand_side: &DenseArray<Complex32>,
) -> Result<DenseArray<Complex32>, LinalgError> {
    provider
        .solve_rectangular_complex32(RectangularSolveRequest::new(coefficients, right_hand_side))
}

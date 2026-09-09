use crate::{
    CholeskyDimensions, FactorDimensions, GemmDimensions, LinalgError, RectangularSolveDimensions,
    SolveDimensions, SpectralDimensions,
};

/// Largest nonnegative dimension representable by the signed LP64 BLAS ABI.
pub const LP64_MAX_DIMENSION: u64 = i32::MAX as u64;

/// Converts a checked runtime dimension to an LP64 BLAS integer.
///
/// # Errors
///
/// Returns [`LinalgError::Lp64DimensionOverflow`] if `value` exceeds
/// [`LP64_MAX_DIMENSION`].
pub fn checked_lp64_dimension(parameter: &'static str, value: u64) -> Result<i32, LinalgError> {
    i32::try_from(value).map_err(|_| LinalgError::Lp64DimensionOverflow { parameter, value })
}

/// GEMM sizes converted for a future LP64 CBLAS call.
///
/// This structure is the explicit boundary between `u64` runtime dimensions
/// and signed 32-bit BLAS parameters. Construct it before passing any sizes to
/// an LP64 backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lp64GemmDimensions {
    /// Rows in `op(A)` and `C`.
    pub m: i32,
    /// Columns in `op(B)` and `C`.
    pub n: i32,
    /// Shared inner dimension.
    pub k: i32,
    /// Physical column-major leading dimension of `A`.
    pub lda: i32,
    /// Physical column-major leading dimension of `B`.
    pub ldb: i32,
    /// Physical column-major leading dimension of `C`.
    pub ldc: i32,
}

impl TryFrom<&GemmDimensions> for Lp64GemmDimensions {
    type Error = LinalgError;

    fn try_from(dimensions: &GemmDimensions) -> Result<Self, Self::Error> {
        Ok(Self {
            m: checked_lp64_dimension("m", dimensions.m())?,
            n: checked_lp64_dimension("n", dimensions.n())?,
            k: checked_lp64_dimension("k", dimensions.k())?,
            lda: checked_lp64_dimension("lda", dimensions.left_leading_dimension())?,
            ldb: checked_lp64_dimension("ldb", dimensions.right_leading_dimension())?,
            ldc: checked_lp64_dimension("ldc", dimensions.output_leading_dimension())?,
        })
    }
}

/// General-solve sizes converted for the LP64 LAPACKE ABI.
///
/// Construct this value before allocating pivots or passing any size to a
/// native backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lp64SolveDimensions {
    /// Order of the square coefficient matrix.
    pub n: i32,
    /// Number of right-hand sides.
    pub nrhs: i32,
    /// Column-major leading dimension of the coefficient matrix.
    pub lda: i32,
    /// Column-major leading dimension of the right-hand-side matrix.
    pub ldb: i32,
}

impl TryFrom<&SolveDimensions> for Lp64SolveDimensions {
    type Error = LinalgError;

    fn try_from(dimensions: &SolveDimensions) -> Result<Self, Self::Error> {
        Ok(Self {
            n: checked_lp64_dimension("n", dimensions.order())?,
            nrhs: checked_lp64_dimension("nrhs", dimensions.right_hand_sides())?,
            lda: checked_lp64_dimension("lda", dimensions.coefficient_leading_dimension())?,
            ldb: checked_lp64_dimension("ldb", dimensions.right_hand_side_leading_dimension())?,
        })
    }
}

/// Rectangular-solve sizes converted for the LP64 LAPACKE ABI.
///
/// `ldb` describes the padded LAPACK working right-hand side, whose physical
/// row count is `max(m, n)`, rather than the immutable input's `m` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lp64RectangularSolveDimensions {
    /// Rows in the coefficient matrix.
    pub m: i32,
    /// Columns in the coefficient matrix and rows in the result.
    pub n: i32,
    /// Number of right-hand sides.
    pub nrhs: i32,
    /// Column-major leading dimension of the coefficient matrix.
    pub lda: i32,
    /// Column-major leading dimension of the padded LAPACK right-hand side.
    pub ldb: i32,
}

impl TryFrom<&RectangularSolveDimensions> for Lp64RectangularSolveDimensions {
    type Error = LinalgError;

    fn try_from(dimensions: &RectangularSolveDimensions) -> Result<Self, Self::Error> {
        Ok(Self {
            m: checked_lp64_dimension("m", dimensions.coefficient_rows())?,
            n: checked_lp64_dimension("n", dimensions.coefficient_columns())?,
            nrhs: checked_lp64_dimension("nrhs", dimensions.right_hand_sides())?,
            lda: checked_lp64_dimension("lda", dimensions.coefficient_leading_dimension())?,
            ldb: checked_lp64_dimension("ldb", dimensions.workspace_leading_dimension())?,
        })
    }
}

/// Rectangular factorization sizes converted for the LP64 LAPACKE ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lp64FactorDimensions {
    /// Matrix row count.
    pub m: i32,
    /// Matrix column count.
    pub n: i32,
    /// Number of diagonal pivots or reflectors.
    pub k: i32,
    /// Column-major leading dimension of the matrix.
    pub lda: i32,
}

impl TryFrom<&FactorDimensions> for Lp64FactorDimensions {
    type Error = LinalgError;

    fn try_from(dimensions: &FactorDimensions) -> Result<Self, Self::Error> {
        Ok(Self {
            m: checked_lp64_dimension("m", dimensions.rows())?,
            n: checked_lp64_dimension("n", dimensions.columns())?,
            k: checked_lp64_dimension("k", dimensions.order())?,
            lda: checked_lp64_dimension("lda", dimensions.leading_dimension())?,
        })
    }
}

/// Cholesky sizes converted for the LP64 LAPACKE ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lp64CholeskyDimensions {
    /// Matrix order.
    pub n: i32,
    /// Column-major leading dimension.
    pub lda: i32,
}

impl TryFrom<&CholeskyDimensions> for Lp64CholeskyDimensions {
    type Error = LinalgError;

    fn try_from(dimensions: &CholeskyDimensions) -> Result<Self, Self::Error> {
        Ok(Self {
            n: checked_lp64_dimension("n", dimensions.order())?,
            lda: checked_lp64_dimension("lda", dimensions.leading_dimension())?,
        })
    }
}

/// SVD/eig sizes converted for the LP64 LAPACKE ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lp64SpectralDimensions {
    /// Matrix row count.
    pub m: i32,
    /// Matrix column count.
    pub n: i32,
    /// Minimum of the row and column counts.
    pub k: i32,
    /// Column-major leading dimension.
    pub lda: i32,
}

impl TryFrom<&SpectralDimensions> for Lp64SpectralDimensions {
    type Error = LinalgError;

    fn try_from(dimensions: &SpectralDimensions) -> Result<Self, Self::Error> {
        Ok(Self {
            m: checked_lp64_dimension("m", dimensions.rows())?,
            n: checked_lp64_dimension("n", dimensions.columns())?,
            k: checked_lp64_dimension("k", dimensions.order())?,
            lda: checked_lp64_dimension("lda", dimensions.leading_dimension())?,
        })
    }
}

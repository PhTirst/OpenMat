use std::error::Error;
use std::fmt;

use openmat_array::ArrayError;

/// Errors reported by provider-independent linear algebra operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinalgError {
    /// An array construction or checked-indexing operation failed.
    Array(ArrayError),
    /// A matrix operation received an array with a significant dimension above
    /// the second dimension.
    MatrixRequired {
        /// Human-readable operand name.
        operand: &'static str,
        /// Canonical dimensions of the received array.
        dimensions: Vec<u64>,
    },
    /// Two dimensions that must agree differ.
    DimensionMismatch {
        /// Operation or constraint being validated.
        operation: &'static str,
        /// Left-hand dimension.
        left: u64,
        /// Right-hand dimension.
        right: u64,
    },
    /// A caller-provided GEMM output matrix has the wrong shape.
    OutputShapeMismatch {
        /// Required row count.
        expected_rows: u64,
        /// Required column count.
        expected_columns: u64,
        /// Actual row count.
        actual_rows: u64,
        /// Actual column count.
        actual_columns: u64,
    },
    /// A coefficient matrix for a linear solve is not square.
    SquareMatrixRequired {
        /// Human-readable operand name.
        operand: &'static str,
        /// Actual row count.
        rows: u64,
        /// Actual column count.
        columns: u64,
    },
    /// A dimension cannot be passed to an LP64 BLAS integer parameter.
    Lp64DimensionOverflow {
        /// BLAS/LAPACK parameter name such as `m`, `n`, `nrhs`, or `lda`.
        parameter: &'static str,
        /// Rejected unsigned dimension.
        value: u64,
    },
    /// A fallible allocation required by a numerical operation failed.
    AllocationFailure {
        /// Buffer or operation that requested the allocation.
        operation: &'static str,
        /// Number of elements requested, when representable by the shape model.
        elements: u64,
    },
    /// Cooperative cancellation was observed outside native code or at a
    /// reference-provider checkpoint.
    Cancelled {
        /// Operation that observed cancellation.
        operation: &'static str,
    },
    /// A provider reported a singular coefficient matrix.
    SingularMatrix {
        /// Provider that detected singularity.
        provider: &'static str,
        /// Operation requested from the provider.
        operation: &'static str,
        /// One-based pivot at which factorization found an exact zero.
        pivot: u64,
    },
    /// A least-squares provider determined that the coefficient matrix does
    /// not have the full rank required by the rectangular-solve contract.
    RankDeficient {
        /// Provider that detected the deficient numerical rank.
        provider: &'static str,
        /// Operation requested from the provider.
        operation: &'static str,
        /// One-based triangular diagonal found numerically zero.
        deficient_diagonal: u64,
        /// Rank required by the full-rank contract (`min(m, n)`).
        required_rank: u64,
    },
    /// An iterative or LAPACK provider failed to converge.
    NoConvergence {
        /// Provider that detected the failure.
        provider: &'static str,
        /// Operation requested from the provider.
        operation: &'static str,
        /// Iterations completed by a reference algorithm, when available.
        iterations: Option<u64>,
        /// Provider-reported unconverged count or index, when available.
        unconverged: Option<u64>,
    },
    /// A LAPACK-style provider rejected one of the arguments assembled by this
    /// layer.
    InvalidProviderArgument {
        /// Provider that rejected the call.
        provider: &'static str,
        /// Operation requested from the provider.
        operation: &'static str,
        /// One-based argument position reported by the provider.
        argument: u32,
    },
    /// A numerical backend reported a provider-specific execution failure.
    ProviderFailure {
        /// Provider that failed.
        provider: &'static str,
        /// Operation requested from the provider.
        operation: &'static str,
        /// Provider-owned detail that is safe to surface internally.
        detail: String,
    },
}

impl fmt::Display for LinalgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Array(error) => write!(formatter, "array error: {error}"),
            Self::MatrixRequired {
                operand,
                dimensions,
            } => write!(
                formatter,
                "{operand} must be a matrix, received dimensions {dimensions:?}"
            ),
            Self::DimensionMismatch {
                operation,
                left,
                right,
            } => write!(
                formatter,
                "dimension mismatch for {operation}: {left} does not equal {right}"
            ),
            Self::OutputShapeMismatch {
                expected_rows,
                expected_columns,
                actual_rows,
                actual_columns,
            } => write!(
                formatter,
                "GEMM output must be {expected_rows}x{expected_columns}, received {actual_rows}x{actual_columns}"
            ),
            Self::SquareMatrixRequired {
                operand,
                rows,
                columns,
            } => write!(
                formatter,
                "{operand} must be square, received {rows}x{columns}"
            ),
            Self::Lp64DimensionOverflow { parameter, value } => write!(
                formatter,
                "BLAS/LAPACK parameter {parameter}={value} exceeds the LP64 integer range"
            ),
            Self::AllocationFailure {
                operation,
                elements,
            } => write!(
                formatter,
                "allocation for {operation} failed for {elements} elements"
            ),
            Self::Cancelled { operation } => {
                write!(formatter, "operation {operation} was cancelled")
            }
            Self::SingularMatrix {
                provider,
                operation,
                pivot,
            } => write!(
                formatter,
                "provider {provider} found a singular matrix during {operation} at pivot {pivot}"
            ),
            Self::RankDeficient {
                provider,
                operation,
                deficient_diagonal,
                required_rank,
            } => write!(
                formatter,
                "provider {provider} detected rank deficiency at triangular diagonal {deficient_diagonal} during {operation}, which requires rank {required_rank}"
            ),
            Self::NoConvergence {
                provider,
                operation,
                iterations,
                unconverged,
            } => write!(
                formatter,
                "provider {provider} did not converge during {operation} (iterations: {iterations:?}, unconverged: {unconverged:?})"
            ),
            Self::InvalidProviderArgument {
                provider,
                operation,
                argument,
            } => write!(
                formatter,
                "provider {provider} rejected argument {argument} during {operation}"
            ),
            Self::ProviderFailure {
                provider,
                operation,
                detail,
            } => write!(
                formatter,
                "provider {provider} failed {operation}: {detail}"
            ),
        }
    }
}

impl Error for LinalgError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Array(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ArrayError> for LinalgError {
    fn from(value: ArrayError) -> Self {
        Self::Array(value)
    }
}

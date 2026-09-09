use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::Complex64;

/// Signed integer width accepted by a sparse numerical provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparseIndexWidth {
    /// Signed 32-bit indices, commonly used by non-long C APIs.
    I32,
    /// Signed 64-bit indices, commonly used by long-index C APIs.
    I64,
}

impl SparseIndexWidth {
    const fn maximum(self) -> u64 {
        match self {
            Self::I32 => i32::MAX as u64,
            Self::I64 => i64::MAX as u64,
        }
    }
}

/// Provider execution phase used for cancellation and error reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparsePhase {
    /// Pattern inspection and ordering.
    Symbolic,
    /// Value-dependent factorization.
    Numeric,
    /// Applying an operator or reusable factor.
    Execution,
}

/// Operation kind represented by a reusable sparse factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparseFactorKind {
    /// General square LU factorization.
    Lu,
    /// Hermitian/symmetric positive-definite Cholesky factorization.
    Cholesky,
    /// Tall least-squares QR factorization.
    Qr,
}

/// Deterministic provider work measurements.
///
/// Work counters are provider-defined elementary loop counts. They deliberately
/// exclude elapsed time, so identical inputs to a deterministic provider can be
/// compared across runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SparseMetrics {
    /// Work performed while inspecting the sparsity pattern.
    pub symbolic_work: u64,
    /// Value-dependent or execution work.
    pub numeric_work: u64,
    /// Stored entries read from all sparse inputs.
    pub input_nnz: u64,
    /// Stored entries produced, if any.
    pub output_nnz: u64,
}

impl SparseMetrics {
    /// Adds two metric records using saturating counters.
    #[must_use]
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            symbolic_work: self.symbolic_work.saturating_add(other.symbolic_work),
            numeric_work: self.numeric_work.saturating_add(other.numeric_work),
            input_nnz: self.input_nnz.saturating_add(other.input_nnz),
            output_nnz: self.output_nnz.saturating_add(other.output_nnz),
        }
    }
}

/// Sparse provider failures independent of a specific implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SparseError {
    /// A CSC structural invariant was violated.
    InvalidCsc {
        /// Short stable name for the violated invariant.
        invariant: &'static str,
        /// Column related to the failure, when available.
        column: Option<u64>,
    },
    /// A provider's signed index ABI cannot represent an input value.
    IndexWidthOverflow {
        /// Provider index ABI.
        width: SparseIndexWidth,
        /// Name of the rejected dimension or count.
        parameter: &'static str,
        /// Rejected value.
        value: u64,
    },
    /// Matrix or dense operand dimensions are incompatible.
    DimensionMismatch {
        /// Operation or constraint being validated.
        operation: &'static str,
        /// Expected element count or dimension.
        expected: u64,
        /// Received element count or dimension.
        actual: u64,
    },
    /// An operation requires a square matrix.
    SquareMatrixRequired {
        /// Actual row count.
        rows: u64,
        /// Actual column count.
        columns: u64,
    },
    /// Cooperative cancellation was observed at a provider checkpoint.
    Cancelled {
        /// Operation observing cancellation.
        operation: &'static str,
        /// Phase observing cancellation.
        phase: SparsePhase,
    },
    /// A general factorization encountered an exact zero pivot.
    Singular {
        /// Provider reporting the failure.
        provider: &'static str,
        /// One-based pivot index.
        pivot: u64,
    },
    /// Cholesky encountered a non-positive diagonal.
    NotPositiveDefinite {
        /// Provider reporting the failure.
        provider: &'static str,
        /// One-based leading minor that failed.
        minor: u64,
    },
    /// Cholesky input is not symmetric/Hermitian within provider tolerance.
    NotHermitian {
        /// Provider reporting the failure.
        provider: &'static str,
        /// One-based row of the first mismatch.
        row: u64,
        /// One-based column of the first mismatch.
        column: u64,
    },
    /// A rectangular solve detected deficient numerical rank.
    RankDeficient {
        /// Provider reporting the failure.
        provider: &'static str,
        /// Measured numerical rank.
        rank: u64,
        /// Full rank required by the operation.
        required_rank: u64,
    },
    /// A provider deliberately does not implement an operation.
    Unsupported {
        /// Provider reporting the boundary.
        provider: &'static str,
        /// Unsupported operation.
        operation: &'static str,
    },
    /// A numeric factor does not correspond to the supplied symbolic pattern.
    SymbolicPatternMismatch,
    /// A checked allocation or platform-size conversion failed.
    AllocationFailure {
        /// Buffer being constructed.
        operation: &'static str,
        /// Requested element count.
        elements: u64,
    },
    /// Provider-specific internal failure safe for internal diagnostics.
    ProviderFailure {
        /// Provider reporting the failure.
        provider: &'static str,
        /// Operation that failed.
        operation: &'static str,
        /// Provider-owned diagnostic detail.
        detail: String,
    },
}

impl fmt::Display for SparseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCsc { invariant, column } => {
                write!(formatter, "invalid CSC ({invariant}, column {column:?})")
            }
            Self::IndexWidthOverflow {
                width,
                parameter,
                value,
            } => {
                write!(
                    formatter,
                    "{parameter}={value} exceeds {width:?} sparse index range"
                )
            }
            Self::DimensionMismatch {
                operation,
                expected,
                actual,
            } => {
                write!(
                    formatter,
                    "{operation} expected {expected}, received {actual}"
                )
            }
            Self::SquareMatrixRequired { rows, columns } => {
                write!(
                    formatter,
                    "sparse factorization requires a square matrix, received {rows}x{columns}"
                )
            }
            Self::Cancelled { operation, phase } => {
                write!(formatter, "{operation} cancelled during {phase:?}")
            }
            Self::Singular { provider, pivot } => {
                write!(formatter, "{provider} found a singular pivot at {pivot}")
            }
            Self::NotPositiveDefinite { provider, minor } => {
                write!(
                    formatter,
                    "{provider} found non-positive leading minor {minor}"
                )
            }
            Self::NotHermitian {
                provider,
                row,
                column,
            } => write!(
                formatter,
                "{provider} found non-Hermitian entries at ({row}, {column})"
            ),
            Self::RankDeficient {
                provider,
                rank,
                required_rank,
            } => {
                write!(
                    formatter,
                    "{provider} measured rank {rank}, required {required_rank}"
                )
            }
            Self::Unsupported {
                provider,
                operation,
            } => {
                write!(formatter, "{provider} does not support {operation}")
            }
            Self::SymbolicPatternMismatch => write!(formatter, "symbolic factor pattern mismatch"),
            Self::AllocationFailure {
                operation,
                elements,
            } => {
                write!(
                    formatter,
                    "allocation for {operation} failed for {elements} elements"
                )
            }
            Self::ProviderFailure {
                provider,
                operation,
                detail,
            } => {
                write!(formatter, "{provider} failed {operation}: {detail}")
            }
        }
    }
}

impl Error for SparseError {}

/// Borrowed, canonical CSC pattern.
#[derive(Clone, Copy, Debug)]
pub struct CscPatternRef<'a> {
    rows: u64,
    columns: u64,
    col_offsets: &'a [u64],
    row_indices: &'a [u64],
}

impl<'a> CscPatternRef<'a> {
    /// Validates and borrows a canonical zero-based CSC pattern.
    ///
    /// # Errors
    ///
    /// Returns [`SparseError::InvalidCsc`] when any canonical CSC invariant fails.
    pub fn new(
        rows: u64,
        columns: u64,
        col_offsets: &'a [u64],
        row_indices: &'a [u64],
    ) -> Result<Self, SparseError> {
        let expected_offsets = columns.checked_add(1).ok_or(SparseError::InvalidCsc {
            invariant: "column count plus sentinel must be representable",
            column: None,
        })?;
        if u64::try_from(col_offsets.len()).ok() != Some(expected_offsets) {
            return Err(SparseError::InvalidCsc {
                invariant: "column offset length",
                column: None,
            });
        }
        if col_offsets.first() != Some(&0) {
            return Err(SparseError::InvalidCsc {
                invariant: "first column offset is zero",
                column: Some(0),
            });
        }
        let nnz = u64::try_from(row_indices.len()).map_err(|_| SparseError::InvalidCsc {
            invariant: "stored entry count is representable",
            column: None,
        })?;
        if col_offsets.last() != Some(&nnz) {
            return Err(SparseError::InvalidCsc {
                invariant: "last column offset equals nnz",
                column: Some(columns),
            });
        }
        for (column, offsets) in col_offsets.windows(2).enumerate() {
            if offsets[0] > offsets[1] || offsets[1] > nnz {
                return Err(SparseError::InvalidCsc {
                    invariant: "monotone in-range column offsets",
                    column: u64::try_from(column).ok(),
                });
            }
            let start = usize::try_from(offsets[0]).map_err(|_| SparseError::InvalidCsc {
                invariant: "column offset fits platform size",
                column: u64::try_from(column).ok(),
            })?;
            let end = usize::try_from(offsets[1]).map_err(|_| SparseError::InvalidCsc {
                invariant: "column offset fits platform size",
                column: u64::try_from(column).ok(),
            })?;
            let mut previous = None;
            for &row in &row_indices[start..end] {
                if row >= rows || previous.is_some_and(|value| value >= row) {
                    return Err(SparseError::InvalidCsc {
                        invariant: "strictly increasing in-range row indices",
                        column: u64::try_from(column).ok(),
                    });
                }
                previous = Some(row);
            }
        }
        Ok(Self {
            rows,
            columns,
            col_offsets,
            row_indices,
        })
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

    /// Number of structurally stored entries.
    #[must_use]
    pub fn nnz(self) -> u64 {
        self.row_indices.len() as u64
    }

    /// Canonical column offsets, including the final sentinel.
    #[must_use]
    pub const fn col_offsets(self) -> &'a [u64] {
        self.col_offsets
    }

    /// Canonical zero-based row indices.
    #[must_use]
    pub const fn row_indices(self) -> &'a [u64] {
        self.row_indices
    }

    /// Checks every dimension and structural index for a provider ABI.
    ///
    /// # Errors
    ///
    /// Returns [`SparseError::IndexWidthOverflow`] when the selected signed ABI
    /// cannot represent a dimension or stored-entry count.
    pub fn check_index_width(self, width: SparseIndexWidth) -> Result<(), SparseError> {
        for (parameter, value) in [
            ("rows", self.rows),
            ("columns", self.columns),
            ("nnz", self.nnz()),
        ] {
            if value > width.maximum() {
                return Err(SparseError::IndexWidthOverflow {
                    width,
                    parameter,
                    value,
                });
            }
        }
        Ok(())
    }

    /// Stable deterministic fingerprint for symbolic/numeric matching.
    #[must_use]
    pub fn fingerprint(self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for value in [self.rows, self.columns]
            .into_iter()
            .chain(self.col_offsets.iter().copied())
            .chain(self.row_indices.iter().copied())
        {
            for byte in value.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }
}

/// Borrowed canonical CSC matrix.
#[derive(Clone, Copy, Debug)]
pub struct CscMatrixRef<'a, T> {
    pattern: CscPatternRef<'a>,
    values: &'a [T],
}

impl<'a, T> CscMatrixRef<'a, T> {
    /// Validates and borrows canonical CSC parts.
    ///
    /// # Errors
    ///
    /// Returns [`SparseError::InvalidCsc`] for invalid structure or a value-count
    /// mismatch.
    pub fn new(
        rows: u64,
        columns: u64,
        col_offsets: &'a [u64],
        row_indices: &'a [u64],
        values: &'a [T],
    ) -> Result<Self, SparseError> {
        let pattern = CscPatternRef::new(rows, columns, col_offsets, row_indices)?;
        if values.len() != row_indices.len() {
            return Err(SparseError::InvalidCsc {
                invariant: "value count equals row-index count",
                column: None,
            });
        }
        Ok(Self { pattern, values })
    }

    /// Borrows the validated pattern.
    #[must_use]
    pub const fn pattern(self) -> CscPatternRef<'a> {
        self.pattern
    }

    /// Stored values aligned with row indices.
    #[must_use]
    pub const fn values(self) -> &'a [T] {
        self.values
    }
}

/// Provider-neutral owned CSC result.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedCscMatrix<T> {
    rows: u64,
    columns: u64,
    col_offsets: Vec<u64>,
    row_indices: Vec<u64>,
    values: Vec<T>,
}

impl<T> OwnedCscMatrix<T> {
    /// Validates and takes deterministic ownership of CSC buffers.
    ///
    /// # Errors
    ///
    /// Returns [`SparseError::InvalidCsc`] if the supplied buffers are not a
    /// canonical CSC matrix.
    pub fn new(
        rows: u64,
        columns: u64,
        col_offsets: Vec<u64>,
        row_indices: Vec<u64>,
        values: Vec<T>,
    ) -> Result<Self, SparseError> {
        CscMatrixRef::new(rows, columns, &col_offsets, &row_indices, &values)?;
        Ok(Self {
            rows,
            columns,
            col_offsets,
            row_indices,
            values,
        })
    }

    /// Borrows this matrix through the provider boundary.
    #[must_use]
    pub fn as_ref(&self) -> CscMatrixRef<'_, T> {
        CscMatrixRef {
            pattern: CscPatternRef {
                rows: self.rows,
                columns: self.columns,
                col_offsets: &self.col_offsets,
                row_indices: &self.row_indices,
            },
            values: &self.values,
        }
    }

    /// Takes the owned CSC buffers apart.
    #[must_use]
    pub fn into_parts(self) -> (u64, u64, Vec<u64>, Vec<u64>, Vec<T>) {
        (
            self.rows,
            self.columns,
            self.col_offsets,
            self.row_indices,
            self.values,
        )
    }
}

/// Cooperative cancellation view. Providers never retain this borrow.
#[derive(Clone, Copy, Debug, Default)]
pub struct SparseCancellation<'a> {
    flag: Option<&'a AtomicBool>,
}

impl<'a> SparseCancellation<'a> {
    /// A token that never cancels.
    #[must_use]
    pub const fn never() -> Self {
        Self { flag: None }
    }

    /// Borrows a caller-owned atomic cancellation flag.
    #[must_use]
    pub const fn from_atomic(flag: &'a AtomicBool) -> Self {
        Self { flag: Some(flag) }
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(self) -> bool {
        self.flag.is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// Emits the common cancellation error at an explicit checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`SparseError::Cancelled`] when the borrowed flag is set.
    pub fn checkpoint(
        self,
        operation: &'static str,
        phase: SparsePhase,
    ) -> Result<(), SparseError> {
        if self.is_cancelled() {
            Err(SparseError::Cancelled { operation, phase })
        } else {
            Ok(())
        }
    }
}

/// Capability declaration used for deterministic provider selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SparseCapabilities {
    /// Sparse matrix-vector multiply.
    pub spmv: bool,
    /// Sparse matrix-matrix multiply.
    pub spgemm: bool,
    /// General square LU and solve.
    pub lu: bool,
    /// Positive-definite Cholesky and solve.
    pub cholesky: bool,
    /// Rectangular least-squares solve.
    pub least_squares: bool,
    /// Left-to-right basic solve for full-row-rank wide matrices.
    pub underdetermined_basic: bool,
}

/// Immutable provider-owned symbolic analysis handle.
pub trait SparseSymbolicFactor: fmt::Debug + Send + Sync {
    /// Factorization family.
    fn kind(&self) -> SparseFactorKind;
    /// Number of factor unknowns (square order, or QR column count).
    fn order(&self) -> u64;
    /// Fingerprint of the exact canonical input pattern.
    fn pattern_fingerprint(&self) -> u64;
    /// Deterministic work measured during analysis.
    fn metrics(&self) -> SparseMetrics;
}

/// Reusable numeric factor handle.
pub trait SparseNumericFactor<T>: fmt::Debug + Send + Sync {
    /// Symbolic analysis associated with the values.
    fn symbolic(&self) -> &dyn SparseSymbolicFactor;
    /// Deterministic work measured while constructing this factor.
    fn metrics(&self) -> SparseMetrics;
    /// Solves one or more column-major right-hand sides without consuming the factor.
    ///
    /// # Errors
    ///
    /// Returns a structured [`SparseError`] for invalid dimensions, cancellation,
    /// or provider execution failure.
    fn solve(
        &self,
        right_hand_side: &[T],
        right_hand_sides: u64,
        cancellation: SparseCancellation<'_>,
    ) -> Result<SparseSolveResult<T>, SparseError>;
}

/// Marker for a reusable general LU factor.
pub trait SparseLuFactor<T>: SparseNumericFactor<T> {}

/// Marker for a reusable positive-definite Cholesky factor.
pub trait SparseCholeskyFactor<T>: SparseNumericFactor<T> {}

/// Marker for a reusable tall least-squares QR factor.
pub trait SparseQrFactor<T>: SparseNumericFactor<T> {}

/// Owned dense result from a sparse solve, in column-major order.
#[derive(Clone, Debug, PartialEq)]
pub struct SparseSolveResult<T> {
    /// Matrix order / result row count.
    pub rows: u64,
    /// Number of solved right-hand sides.
    pub columns: u64,
    /// Column-major solution values.
    pub values: Vec<T>,
    /// Deterministic work for this solve application.
    pub metrics: SparseMetrics,
}

/// Internal sparse numerical provider boundary.
///
/// Rust implementations are selected and linked in-process at build time. This
/// trait is not an external ABI; process or plugin providers require a separately
/// versioned C ABI or serialized protocol adapter.
pub trait SparseProvider: Send + Sync {
    /// Concrete LU symbolic handle.
    type LuSymbolic: SparseSymbolicFactor;
    /// Concrete Cholesky symbolic handle.
    type CholeskySymbolic: SparseSymbolicFactor;
    /// Reusable real LU factor.
    type LuF64: SparseLuFactor<f64>;
    /// Reusable complex-double LU factor.
    type LuComplex64: SparseLuFactor<Complex64>;
    /// Reusable real Cholesky factor.
    type CholeskyF64: SparseCholeskyFactor<f64>;
    /// Reusable complex-double Cholesky factor.
    type CholeskyComplex64: SparseCholeskyFactor<Complex64>;
    /// Concrete QR symbolic handle.
    type QrSymbolic: SparseSymbolicFactor;
    /// Reusable real tall QR factor.
    type QrF64: SparseQrFactor<f64>;
    /// Reusable complex-double tall QR factor.
    type QrComplex64: SparseQrFactor<Complex64>;

    /// Stable internal provider name.
    fn name(&self) -> &'static str;
    /// Signed structural index width accepted by this provider.
    fn index_width(&self) -> SparseIndexWidth;
    /// Operations implemented by this provider.
    fn capabilities(&self) -> SparseCapabilities;

    /// Computes `A*x` for real double values.
    ///
    /// # Errors
    ///
    /// Returns a structured error for incompatible dimensions, cancellation,
    /// allocation, or provider failure.
    fn spmv_f64(
        &self,
        matrix: CscMatrixRef<'_, f64>,
        vector: &[f64],
        cancellation: SparseCancellation<'_>,
    ) -> Result<(Vec<f64>, SparseMetrics), SparseError>;

    /// Computes `A*x` for complex-double values.
    ///
    /// # Errors
    ///
    /// Returns a structured error for incompatible dimensions, cancellation,
    /// allocation, or provider failure.
    fn spmv_complex64(
        &self,
        matrix: CscMatrixRef<'_, Complex64>,
        vector: &[Complex64],
        cancellation: SparseCancellation<'_>,
    ) -> Result<(Vec<Complex64>, SparseMetrics), SparseError>;

    /// Computes real-double sparse `A*B`.
    ///
    /// # Errors
    ///
    /// Returns a structured error for incompatible dimensions, cancellation,
    /// allocation, or provider failure.
    fn spgemm_f64(
        &self,
        left: CscMatrixRef<'_, f64>,
        right: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<(OwnedCscMatrix<f64>, SparseMetrics), SparseError>;

    /// Computes complex-double sparse `A*B`.
    ///
    /// # Errors
    ///
    /// Returns a structured error for incompatible dimensions, cancellation,
    /// allocation, or provider failure.
    fn spgemm_complex64(
        &self,
        left: CscMatrixRef<'_, Complex64>,
        right: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<(OwnedCscMatrix<Complex64>, SparseMetrics), SparseError>;

    /// Analyzes a general square pattern independently of numeric values.
    ///
    /// # Errors
    ///
    /// Returns a structured error for non-square input, index overflow,
    /// cancellation, allocation, or provider failure.
    fn analyze_lu(
        &self,
        pattern: CscPatternRef<'_>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::LuSymbolic, SparseError>;

    /// Produces a reusable real numeric LU factor.
    ///
    /// # Errors
    ///
    /// Returns a structured error for pattern mismatch, cancellation, singularity,
    /// allocation, or provider failure.
    fn factor_lu_f64(
        &self,
        symbolic: &Self::LuSymbolic,
        matrix: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::LuF64, SparseError>;

    /// Produces a reusable complex-double numeric LU factor.
    ///
    /// # Errors
    ///
    /// Returns a structured error for pattern mismatch, cancellation, singularity,
    /// allocation, or provider failure.
    fn factor_lu_complex64(
        &self,
        symbolic: &Self::LuSymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::LuComplex64, SparseError>;

    /// Analyzes an SPD/Hermitian-positive-definite pattern.
    ///
    /// # Errors
    ///
    /// Returns a structured error for non-square input, index overflow,
    /// cancellation, allocation, or provider failure.
    fn analyze_cholesky(
        &self,
        pattern: CscPatternRef<'_>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskySymbolic, SparseError>;

    /// Produces a reusable real Cholesky factor.
    ///
    /// # Errors
    ///
    /// Returns a structured error for pattern mismatch, cancellation, a
    /// non-positive-definite matrix, allocation, or provider failure.
    fn factor_cholesky_f64(
        &self,
        symbolic: &Self::CholeskySymbolic,
        matrix: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskyF64, SparseError>;

    /// Produces a reusable complex-double Cholesky factor.
    ///
    /// # Errors
    ///
    /// Returns a structured error for pattern mismatch, cancellation, a
    /// non-positive-definite matrix, allocation, or provider failure.
    fn factor_cholesky_complex64(
        &self,
        symbolic: &Self::CholeskySymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskyComplex64, SparseError>;

    /// Analyzes a tall least-squares pattern independently of numeric values.
    ///
    /// # Errors
    ///
    /// Returns a structured error for unsupported shapes, index overflow,
    /// cancellation, allocation, or provider failure.
    fn analyze_qr(
        &self,
        pattern: CscPatternRef<'_>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::QrSymbolic, SparseError>;

    /// Produces a reusable real tall QR factor.
    ///
    /// # Errors
    ///
    /// Returns a structured error for pattern mismatch, cancellation, rank
    /// deficiency, allocation, or provider failure.
    fn factor_qr_f64(
        &self,
        symbolic: &Self::QrSymbolic,
        matrix: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::QrF64, SparseError>;

    /// Produces a reusable complex-double tall QR factor.
    ///
    /// # Errors
    ///
    /// Returns a structured error for pattern mismatch, cancellation, rank
    /// deficiency, allocation, or provider failure.
    fn factor_qr_complex64(
        &self,
        symbolic: &Self::QrSymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::QrComplex64, SparseError>;

    /// Solves a real rectangular least-squares problem when supported.
    ///
    /// # Errors
    ///
    /// Returns a structured error for invalid dimensions, cancellation, rank
    /// deficiency, unsupported shapes, allocation, or provider failure.
    fn least_squares_f64(
        &self,
        matrix: CscMatrixRef<'_, f64>,
        right_hand_side: &[f64],
        right_hand_sides: u64,
        cancellation: SparseCancellation<'_>,
    ) -> Result<SparseSolveResult<f64>, SparseError> {
        let symbolic = self.analyze_qr(matrix.pattern(), cancellation)?;
        let factor = self.factor_qr_f64(&symbolic, matrix, cancellation)?;
        factor.solve(right_hand_side, right_hand_sides, cancellation)
    }

    /// Solves a complex rectangular least-squares problem when supported.
    ///
    /// # Errors
    ///
    /// Returns a structured error for invalid dimensions, cancellation, rank
    /// deficiency, unsupported shapes, allocation, or provider failure.
    fn least_squares_complex64(
        &self,
        matrix: CscMatrixRef<'_, Complex64>,
        right_hand_side: &[Complex64],
        right_hand_sides: u64,
        cancellation: SparseCancellation<'_>,
    ) -> Result<SparseSolveResult<Complex64>, SparseError> {
        let symbolic = self.analyze_qr(matrix.pattern(), cancellation)?;
        let factor = self.factor_qr_complex64(&symbolic, matrix, cancellation)?;
        factor.solve(right_hand_side, right_hand_sides, cancellation)
    }

    /// Computes the sparse-language basic solution for a full-row-rank wide
    /// real matrix.
    ///
    /// Columns are considered from left to right. The first numerically
    /// independent `rows` columns form a square system; its solution is placed
    /// in those result rows and every other result row is exactly zero. This is
    /// intentionally distinct from a minimum-norm solve and from the
    /// column-norm pivoting used by the dense-language helper.
    ///
    /// # Errors
    ///
    /// Returns a structured error for a non-wide shape, incompatible right-hand
    /// sides, cancellation, deficient row rank, allocation, or provider failure.
    fn underdetermined_basic_f64(
        &self,
        matrix: CscMatrixRef<'_, f64>,
        right_hand_side: &[f64],
        right_hand_sides: u64,
        cancellation: SparseCancellation<'_>,
    ) -> Result<SparseSolveResult<f64>, SparseError>
    where
        Self: Sized,
    {
        sparse_underdetermined_basic_f64(
            self,
            matrix,
            right_hand_side,
            right_hand_sides,
            cancellation,
        )
    }

    /// Computes the sparse-language basic solution for a full-row-rank wide
    /// complex-double matrix.
    ///
    /// Column selection, zero fill, result shape, and structured failures match
    /// [`SparseProvider::underdetermined_basic_f64`]. Independence testing uses
    /// the conjugate inner product.
    ///
    /// # Errors
    ///
    /// Returns a structured error for a non-wide shape, incompatible right-hand
    /// sides, cancellation, deficient row rank, allocation, or provider failure.
    fn underdetermined_basic_complex64(
        &self,
        matrix: CscMatrixRef<'_, Complex64>,
        right_hand_side: &[Complex64],
        right_hand_sides: u64,
        cancellation: SparseCancellation<'_>,
    ) -> Result<SparseSolveResult<Complex64>, SparseError>
    where
        Self: Sized,
    {
        sparse_underdetermined_basic_complex64(
            self,
            matrix,
            right_hand_side,
            right_hand_sides,
            cancellation,
        )
    }
}

fn sparse_underdetermined_basic_f64<P: SparseProvider>(
    provider: &P,
    matrix: CscMatrixRef<'_, f64>,
    right_hand_side: &[f64],
    right_hand_sides: u64,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<f64>, SparseError> {
    let (selected, output_rows, selected_columns, selection_metrics) =
        select_leftmost_independent_columns(
            provider.name(),
            provider.index_width(),
            matrix,
            cancellation,
        )?;
    let symbolic = provider.analyze_lu(selected.as_ref().pattern(), cancellation)?;
    let factor = provider.factor_lu_f64(&symbolic, selected.as_ref(), cancellation)?;
    let mut solved = factor.solve(right_hand_side, right_hand_sides, cancellation)?;
    solved.metrics = selection_metrics
        .saturating_add(symbolic.metrics())
        .saturating_add(factor.metrics())
        .saturating_add(solved.metrics);
    expand_basic_solution(&solved, output_rows, &selected_columns, cancellation)
}

fn sparse_underdetermined_basic_complex64<P: SparseProvider>(
    provider: &P,
    matrix: CscMatrixRef<'_, Complex64>,
    right_hand_side: &[Complex64],
    right_hand_sides: u64,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<Complex64>, SparseError> {
    let (selected, output_rows, selected_columns, selection_metrics) =
        select_leftmost_independent_columns(
            provider.name(),
            provider.index_width(),
            matrix,
            cancellation,
        )?;
    let symbolic = provider.analyze_lu(selected.as_ref().pattern(), cancellation)?;
    let factor = provider.factor_lu_complex64(&symbolic, selected.as_ref(), cancellation)?;
    let mut solved = factor.solve(right_hand_side, right_hand_sides, cancellation)?;
    solved.metrics = selection_metrics
        .saturating_add(symbolic.metrics())
        .saturating_add(factor.metrics())
        .saturating_add(solved.metrics);
    expand_basic_solution(&solved, output_rows, &selected_columns, cancellation)
}

fn select_leftmost_independent_columns<T: SparseBasicScalar>(
    provider: &'static str,
    index_width: SparseIndexWidth,
    matrix: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<(OwnedCscMatrix<T>, u64, Vec<usize>, SparseMetrics), SparseError> {
    const OPERATION: &str = "sparse underdetermined basic solve";

    cancellation.checkpoint(OPERATION, SparsePhase::Numeric)?;
    let pattern = matrix.pattern();
    pattern.check_index_width(index_width)?;
    let row_count = pattern.rows();
    let column_count = pattern.columns();
    if row_count >= column_count {
        return Err(SparseError::Unsupported {
            provider,
            operation: "non-wide sparse basic solve",
        });
    }
    let rows = host_size("sparse basic-solve rows", row_count)?;
    let columns = host_size("sparse basic-solve columns", column_count)?;
    let mut basis = try_filled_buffer(
        "sparse basic-solve orthogonal basis",
        rows.checked_mul(rows)
            .ok_or(SparseError::AllocationFailure {
                operation: "sparse basic-solve orthogonal basis",
                elements: u64::MAX,
            })?,
        T::zero(),
    )?;
    let mut residual = try_filled_buffer("sparse basic-solve column residual", rows, T::zero())?;
    let mut selected_columns = try_reserved_buffer("sparse basic-solve selected columns", rows)?;

    let mut scale = 0.0_f64;
    for column in 0..columns {
        cancellation.checkpoint(OPERATION, SparsePhase::Numeric)?;
        scale = scale.max(sparse_column_norm(matrix, column)?);
    }
    let dimension = u32::try_from(rows.max(columns)).map_or(f64::from(u32::MAX), f64::from);
    let tolerance = scale * f64::EPSILON * dimension;
    let mut numeric_work = pattern.nnz();

    for column in 0..columns {
        cancellation.checkpoint(OPERATION, SparsePhase::Numeric)?;
        residual.fill(T::zero());
        let (start, end) = column_range(pattern, column)?;
        for position in start..end {
            let row = host_size(
                "sparse basic-solve row index",
                pattern.row_indices()[position],
            )?;
            residual[row] = matrix.values()[position];
        }

        for basis_column in 0..selected_columns.len() {
            for _ in 0..2 {
                cancellation.checkpoint(OPERATION, SparsePhase::Numeric)?;
                let mut projection = T::zero();
                for row in 0..rows {
                    projection = projection.add(
                        basis[basis_column * rows + row]
                            .conjugate()
                            .multiply(residual[row]),
                    );
                    numeric_work = numeric_work.saturating_add(1);
                }
                for row in 0..rows {
                    residual[row] = residual[row]
                        .subtract(basis[basis_column * rows + row].multiply(projection));
                    numeric_work = numeric_work.saturating_add(1);
                }
            }
        }

        let norm = dense_vector_norm(&residual);
        if norm > tolerance {
            let basis_column = selected_columns.len();
            for row in 0..rows {
                basis[basis_column * rows + row] = residual[row].scale(norm.recip());
            }
            selected_columns.push(column);
            if selected_columns.len() == rows {
                break;
            }
        }
    }

    if selected_columns.len() != rows {
        return Err(SparseError::RankDeficient {
            provider,
            rank: u64::try_from(selected_columns.len()).unwrap_or(u64::MAX),
            required_rank: row_count,
        });
    }

    let selected = copy_selected_columns(matrix, &selected_columns, cancellation)?;
    Ok((
        selected,
        column_count,
        selected_columns,
        SparseMetrics {
            symbolic_work: 0,
            numeric_work,
            input_nnz: pattern.nnz(),
            output_nnz: 0,
        },
    ))
}

fn copy_selected_columns<T: Copy>(
    matrix: CscMatrixRef<'_, T>,
    columns: &[usize],
    cancellation: SparseCancellation<'_>,
) -> Result<OwnedCscMatrix<T>, SparseError> {
    const OPERATION: &str = "sparse basic-solve selected matrix";
    let pattern = matrix.pattern();
    let mut selected_nnz = 0_usize;
    for &column in columns {
        let (start, end) = column_range(pattern, column)?;
        selected_nnz =
            selected_nnz
                .checked_add(end - start)
                .ok_or(SparseError::AllocationFailure {
                    operation: OPERATION,
                    elements: u64::MAX,
                })?;
    }

    let mut col_offsets = try_reserved_buffer(OPERATION, columns.len().saturating_add(1))?;
    let mut row_indices = try_reserved_buffer(OPERATION, selected_nnz)?;
    let mut values = try_reserved_buffer(OPERATION, selected_nnz)?;
    col_offsets.push(0);
    for &column in columns {
        cancellation.checkpoint(OPERATION, SparsePhase::Numeric)?;
        let (start, end) = column_range(pattern, column)?;
        row_indices.extend_from_slice(&pattern.row_indices()[start..end]);
        values.extend_from_slice(&matrix.values()[start..end]);
        col_offsets.push(u64::try_from(row_indices.len()).map_err(|_| {
            SparseError::AllocationFailure {
                operation: OPERATION,
                elements: u64::MAX,
            }
        })?);
    }
    OwnedCscMatrix::new(
        pattern.rows(),
        pattern.rows(),
        col_offsets,
        row_indices,
        values,
    )
}

fn expand_basic_solution<T: SparseBasicScalar>(
    selected: &SparseSolveResult<T>,
    output_rows: u64,
    selected_columns: &[usize],
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<T>, SparseError> {
    const OPERATION: &str = "sparse underdetermined basic-solution expansion";
    let rows = host_size(OPERATION, output_rows)?;
    let columns = host_size(OPERATION, selected.columns)?;
    let output_len = rows
        .checked_mul(columns)
        .ok_or(SparseError::AllocationFailure {
            operation: OPERATION,
            elements: u64::MAX,
        })?;
    let mut values = try_filled_buffer(OPERATION, output_len, T::zero())?;
    for rhs_column in 0..columns {
        cancellation.checkpoint(OPERATION, SparsePhase::Execution)?;
        for (selected_row, &output_row) in selected_columns.iter().enumerate() {
            values[rhs_column * rows + output_row] =
                selected.values[rhs_column * selected_columns.len() + selected_row];
        }
    }
    Ok(SparseSolveResult {
        rows: output_rows,
        columns: selected.columns,
        values,
        metrics: selected.metrics,
    })
}

fn sparse_column_norm<T: SparseBasicScalar>(
    matrix: CscMatrixRef<'_, T>,
    column: usize,
) -> Result<f64, SparseError> {
    let (start, end) = column_range(matrix.pattern(), column)?;
    Ok(matrix.values()[start..end]
        .iter()
        .fold(0.0_f64, |norm, value| norm.hypot(value.magnitude())))
}

fn dense_vector_norm<T: SparseBasicScalar>(values: &[T]) -> f64 {
    values
        .iter()
        .fold(0.0_f64, |norm, value| norm.hypot(value.magnitude()))
}

fn column_range(pattern: CscPatternRef<'_>, column: usize) -> Result<(usize, usize), SparseError> {
    let start = pattern
        .col_offsets()
        .get(column)
        .copied()
        .ok_or(SparseError::InvalidCsc {
            invariant: "requested column exists",
            column: u64::try_from(column).ok(),
        })?;
    let end = pattern
        .col_offsets()
        .get(column.saturating_add(1))
        .copied()
        .ok_or(SparseError::InvalidCsc {
            invariant: "requested column sentinel exists",
            column: u64::try_from(column).ok(),
        })?;
    Ok((
        host_size("sparse column start", start)?,
        host_size("sparse column end", end)?,
    ))
}

fn host_size(operation: &'static str, value: u64) -> Result<usize, SparseError> {
    usize::try_from(value).map_err(|_| SparseError::AllocationFailure {
        operation,
        elements: value,
    })
}

fn try_filled_buffer<T: Clone>(
    operation: &'static str,
    length: usize,
    value: T,
) -> Result<Vec<T>, SparseError> {
    let mut output = try_reserved_buffer(operation, length)?;
    output.resize(length, value);
    Ok(output)
}

fn try_reserved_buffer<T>(operation: &'static str, length: usize) -> Result<Vec<T>, SparseError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| SparseError::AllocationFailure {
            operation,
            elements: u64::try_from(length).unwrap_or(u64::MAX),
        })?;
    Ok(output)
}

trait SparseBasicScalar: Copy {
    fn zero() -> Self;
    fn magnitude(self) -> f64;
    fn conjugate(self) -> Self;
    fn add(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn scale(self, factor: f64) -> Self;
}

impl SparseBasicScalar for f64 {
    fn zero() -> Self {
        0.0
    }

    fn magnitude(self) -> f64 {
        self.abs()
    }

    fn conjugate(self) -> Self {
        self
    }

    fn add(self, right: Self) -> Self {
        self + right
    }

    fn subtract(self, right: Self) -> Self {
        self - right
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn scale(self, factor: f64) -> Self {
        self * factor
    }
}

impl SparseBasicScalar for Complex64 {
    fn zero() -> Self {
        Self::ZERO
    }

    fn magnitude(self) -> f64 {
        self.re.hypot(self.im)
    }

    fn conjugate(self) -> Self {
        self.conjugate()
    }

    fn add(self, right: Self) -> Self {
        Self::new(self.re + right.re, self.im + right.im)
    }

    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn scale(self, factor: f64) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }
}

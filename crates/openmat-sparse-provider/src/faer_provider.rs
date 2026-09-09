use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use faer::dyn_stack::{MemBuffer, MemStack};
use faer::linalg::solvers::{Solve, SolveLstsq};
use faer::perm::PermRef;
use faer::sparse::linalg::lu::{
    LuSymbolicParams, SymbolicLu as DiagnosticSymbolicLu, factorize_symbolic_lu,
    simplicial as simplicial_lu,
};
use faer::sparse::linalg::matmul::{sparse_dense_matmul, sparse_sparse_matmul};
use faer::sparse::linalg::qr::{
    QrSymbolicParams, SymbolicQr as DiagnosticSymbolicQr, factorize_symbolic_qr,
};
use faer::sparse::linalg::solvers::{Llt, Qr, SymbolicLlt, SymbolicQr};
use faer::sparse::linalg::{LltError, LuError, SupernodalThreshold};
use faer::sparse::{CreationError, FaerError, SparseColMat, Triplet};
use faer::{Accum, Conj, Mat, Par, Side, Spec, c64};
use openmat_array::Complex64;
use openmat_linalg::{
    CscMatrixRef, CscPatternRef, OwnedCscMatrix, SparseCancellation, SparseCapabilities,
    SparseCholeskyFactor, SparseError, SparseFactorKind, SparseIndexWidth, SparseLuFactor,
    SparseMetrics, SparseNumericFactor, SparsePhase, SparseProvider, SparseQrFactor,
    SparseSolveResult, SparseSymbolicFactor,
};

const PROVIDER_NAME: &str = "faer-0.24.4";

/// Sparse operation implementation selected by [`FaerSparseProvider`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaerOperationBackend {
    /// The operation is executed by faer 0.24.4.
    Faer,
}

/// In-process sparse provider backed by faer 0.24.4.
#[derive(Clone, Copy, Debug, Default)]
pub struct FaerSparseProvider;

impl FaerSparseProvider {
    /// Reports the implementation used for a provider operation.
    #[must_use]
    pub const fn operation_backend(&self) -> FaerOperationBackend {
        FaerOperationBackend::Faer
    }
}

#[derive(Clone, Debug)]
struct PatternOwnership {
    rows: u64,
    columns: u64,
    fingerprint: u64,
    col_offsets: Vec<u64>,
    row_indices: Vec<u64>,
    metrics: SparseMetrics,
}

/// Reusable faer LU symbolic analysis.
#[derive(Clone, Debug)]
pub struct FaerLuSymbolic {
    pattern: PatternOwnership,
    inner: DiagnosticSymbolicLu<usize>,
}

/// Reusable faer Cholesky symbolic analysis.
#[derive(Clone, Debug)]
pub struct FaerCholeskySymbolic {
    pattern: PatternOwnership,
    inner: SymbolicLlt<usize>,
}

/// Reusable faer tall-QR symbolic analysis.
#[derive(Clone, Debug)]
pub struct FaerQrSymbolic {
    pattern: PatternOwnership,
    inner: SymbolicQr<usize>,
    diagnostic: Arc<DiagnosticSymbolicQr<usize>>,
}

macro_rules! impl_symbolic_factor {
    ($type:ty, $kind:expr) => {
        impl SparseSymbolicFactor for $type {
            fn kind(&self) -> SparseFactorKind {
                $kind
            }

            fn order(&self) -> u64 {
                self.pattern.columns
            }

            fn pattern_fingerprint(&self) -> u64 {
                self.pattern.fingerprint
            }

            fn metrics(&self) -> SparseMetrics {
                self.pattern.metrics
            }
        }
    };
}

impl_symbolic_factor!(FaerLuSymbolic, SparseFactorKind::Lu);
impl_symbolic_factor!(FaerCholeskySymbolic, SparseFactorKind::Cholesky);
impl_symbolic_factor!(FaerQrSymbolic, SparseFactorKind::Qr);

#[derive(Clone, Debug)]
struct FaerLuCore<T> {
    symbolic: FaerLuSymbolic,
    numeric: simplicial_lu::SimplicialLu<usize, T>,
    row_perm: Vec<usize>,
    row_perm_inv: Vec<usize>,
    metrics: SparseMetrics,
}

#[derive(Clone, Debug)]
struct FaerCholeskyCore<T> {
    symbolic: FaerCholeskySymbolic,
    numeric: Llt<usize, T>,
    metrics: SparseMetrics,
}

#[derive(Clone, Debug)]
struct FaerQrCore<T> {
    symbolic: FaerQrSymbolic,
    numeric: Qr<usize, T>,
    metrics: SparseMetrics,
}

/// Reusable faer real LU factor.
#[derive(Clone, Debug)]
pub struct FaerLuF64Factor(FaerLuCore<f64>);
/// Reusable faer complex-double LU factor.
#[derive(Clone, Debug)]
pub struct FaerLuComplex64Factor(FaerLuCore<c64>);
/// Reusable faer real Cholesky factor.
#[derive(Clone, Debug)]
pub struct FaerCholeskyF64Factor(FaerCholeskyCore<f64>);
/// Reusable faer complex-double Cholesky factor.
#[derive(Clone, Debug)]
pub struct FaerCholeskyComplex64Factor(FaerCholeskyCore<c64>);
/// Reusable faer real tall-QR factor.
#[derive(Clone, Debug)]
pub struct FaerQrF64Factor(FaerQrCore<f64>);
/// Reusable faer complex-double tall-QR factor.
#[derive(Clone, Debug)]
pub struct FaerQrComplex64Factor(FaerQrCore<c64>);

trait BridgeScalar: Copy + fmt::Debug + Send + Sync + 'static {
    type Faer: faer::traits::ComplexField + fmt::Debug + Send + Sync;

    fn to_faer(self) -> Self::Faer;
    fn from_faer(value: &Self::Faer) -> Self;
    fn faer_one() -> Self::Faer;
    fn faer_norm(value: &Self::Faer) -> f64;
    fn faer_is_finite(value: &Self::Faer) -> bool;
    fn is_zero(value: &Self::Faer) -> bool;
}

impl BridgeScalar for f64 {
    type Faer = f64;

    fn to_faer(self) -> Self::Faer {
        self
    }
    fn from_faer(value: &Self::Faer) -> Self {
        *value
    }
    fn faer_one() -> Self::Faer {
        1.0
    }
    fn faer_norm(value: &Self::Faer) -> f64 {
        value.abs()
    }
    fn faer_is_finite(value: &Self::Faer) -> bool {
        value.is_finite()
    }
    fn is_zero(value: &Self::Faer) -> bool {
        *value == 0.0
    }
}

impl BridgeScalar for Complex64 {
    type Faer = c64;

    fn to_faer(self) -> Self::Faer {
        c64::new(self.re, self.im)
    }
    fn from_faer(value: &Self::Faer) -> Self {
        Self::new(value.re, value.im)
    }
    fn faer_one() -> Self::Faer {
        c64::new(1.0, 0.0)
    }
    fn faer_norm(value: &Self::Faer) -> f64 {
        value.re.hypot(value.im)
    }
    fn faer_is_finite(value: &Self::Faer) -> bool {
        value.re.is_finite() && value.im.is_finite()
    }
    fn is_zero(value: &Self::Faer) -> bool {
        value.re == 0.0 && value.im == 0.0
    }
}

macro_rules! impl_numeric_factor {
    ($wrapper:ty, $scalar:ty, $solver:ident, $marker:ident) => {
        impl SparseNumericFactor<$scalar> for $wrapper {
            fn symbolic(&self) -> &dyn SparseSymbolicFactor {
                &self.0.symbolic
            }

            fn metrics(&self) -> SparseMetrics {
                self.0.metrics
            }

            fn solve(
                &self,
                right_hand_side: &[$scalar],
                right_hand_sides: u64,
                cancellation: SparseCancellation<'_>,
            ) -> Result<SparseSolveResult<$scalar>, SparseError> {
                $solver(&self.0, right_hand_side, right_hand_sides, cancellation)
            }
        }

        impl $marker<$scalar> for $wrapper {}
    };
}

impl_numeric_factor!(FaerLuF64Factor, f64, solve_lu, SparseLuFactor);
impl_numeric_factor!(FaerLuComplex64Factor, Complex64, solve_lu, SparseLuFactor);
impl_numeric_factor!(
    FaerCholeskyF64Factor,
    f64,
    solve_cholesky,
    SparseCholeskyFactor
);
impl_numeric_factor!(
    FaerCholeskyComplex64Factor,
    Complex64,
    solve_cholesky,
    SparseCholeskyFactor
);
impl_numeric_factor!(FaerQrF64Factor, f64, solve_qr, SparseQrFactor);
impl_numeric_factor!(FaerQrComplex64Factor, Complex64, solve_qr, SparseQrFactor);

impl SparseProvider for FaerSparseProvider {
    type LuSymbolic = FaerLuSymbolic;
    type CholeskySymbolic = FaerCholeskySymbolic;
    type LuF64 = FaerLuF64Factor;
    type LuComplex64 = FaerLuComplex64Factor;
    type CholeskyF64 = FaerCholeskyF64Factor;
    type CholeskyComplex64 = FaerCholeskyComplex64Factor;
    type QrSymbolic = FaerQrSymbolic;
    type QrF64 = FaerQrF64Factor;
    type QrComplex64 = FaerQrComplex64Factor;

    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }

    fn index_width(&self) -> SparseIndexWidth {
        if usize::BITS >= 64 {
            SparseIndexWidth::I64
        } else {
            SparseIndexWidth::I32
        }
    }

    fn capabilities(&self) -> SparseCapabilities {
        SparseCapabilities {
            spmv: true,
            spgemm: true,
            lu: true,
            cholesky: true,
            least_squares: true,
            underdetermined_basic: true,
        }
    }

    fn spmv_f64(
        &self,
        matrix: CscMatrixRef<'_, f64>,
        vector: &[f64],
        cancellation: SparseCancellation<'_>,
    ) -> Result<(Vec<f64>, SparseMetrics), SparseError> {
        spmv(matrix, vector, cancellation)
    }

    fn spmv_complex64(
        &self,
        matrix: CscMatrixRef<'_, Complex64>,
        vector: &[Complex64],
        cancellation: SparseCancellation<'_>,
    ) -> Result<(Vec<Complex64>, SparseMetrics), SparseError> {
        spmv(matrix, vector, cancellation)
    }

    fn spgemm_f64(
        &self,
        left: CscMatrixRef<'_, f64>,
        right: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<(OwnedCscMatrix<f64>, SparseMetrics), SparseError> {
        spgemm(left, right, cancellation)
    }

    fn spgemm_complex64(
        &self,
        left: CscMatrixRef<'_, Complex64>,
        right: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<(OwnedCscMatrix<Complex64>, SparseMetrics), SparseError> {
        spgemm(left, right, cancellation)
    }

    fn analyze_lu(
        &self,
        pattern: CscPatternRef<'_>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::LuSymbolic, SparseError> {
        analyze_lu(pattern, self.index_width(), cancellation)
    }

    fn factor_lu_f64(
        &self,
        symbolic: &Self::LuSymbolic,
        matrix: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::LuF64, SparseError> {
        factor_lu(symbolic, matrix, cancellation).map(FaerLuF64Factor)
    }

    fn factor_lu_complex64(
        &self,
        symbolic: &Self::LuSymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::LuComplex64, SparseError> {
        factor_lu(symbolic, matrix, cancellation).map(FaerLuComplex64Factor)
    }

    fn analyze_cholesky(
        &self,
        pattern: CscPatternRef<'_>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskySymbolic, SparseError> {
        analyze_cholesky(pattern, self.index_width(), cancellation)
    }

    fn factor_cholesky_f64(
        &self,
        symbolic: &Self::CholeskySymbolic,
        matrix: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskyF64, SparseError> {
        validate_symmetric(matrix, cancellation)?;
        factor_cholesky(symbolic, matrix, cancellation).map(FaerCholeskyF64Factor)
    }

    fn factor_cholesky_complex64(
        &self,
        symbolic: &Self::CholeskySymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskyComplex64, SparseError> {
        validate_hermitian(matrix, cancellation)?;
        factor_cholesky(symbolic, matrix, cancellation).map(FaerCholeskyComplex64Factor)
    }

    fn analyze_qr(
        &self,
        pattern: CscPatternRef<'_>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::QrSymbolic, SparseError> {
        analyze_qr(pattern, self.index_width(), cancellation)
    }

    fn factor_qr_f64(
        &self,
        symbolic: &Self::QrSymbolic,
        matrix: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::QrF64, SparseError> {
        factor_qr(symbolic, matrix, cancellation).map(FaerQrF64Factor)
    }

    fn factor_qr_complex64(
        &self,
        symbolic: &Self::QrSymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::QrComplex64, SparseError> {
        factor_qr(symbolic, matrix, cancellation).map(FaerQrComplex64Factor)
    }
}

fn faer_call<R>(operation: &'static str, callback: impl FnOnce() -> R) -> Result<R, SparseError> {
    catch_unwind(AssertUnwindSafe(callback)).map_err(|_| SparseError::ProviderFailure {
        provider: PROVIDER_NAME,
        operation,
        detail: "faer panicked after validated input crossed the adapter".to_string(),
    })
}

fn allocation_error(operation: &'static str, elements: usize) -> SparseError {
    SparseError::AllocationFailure {
        operation,
        elements: u64::try_from(elements).unwrap_or(u64::MAX),
    }
}

fn try_filled<T: Clone>(
    operation: &'static str,
    length: usize,
    value: T,
) -> Result<Vec<T>, SparseError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(operation, length))?;
    output.resize(length, value);
    Ok(output)
}

fn copy_u64(operation: &'static str, source: &[u64]) -> Result<Vec<u64>, SparseError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(source.len())
        .map_err(|_| allocation_error(operation, source.len()))?;
    output.extend_from_slice(source);
    Ok(output)
}

fn checked_pattern(
    pattern: CscPatternRef<'_>,
    width: SparseIndexWidth,
) -> Result<(usize, usize), SparseError> {
    pattern.check_index_width(width)?;
    let rows = usize::try_from(pattern.rows()).map_err(|_| SparseError::IndexWidthOverflow {
        width,
        parameter: "rows",
        value: pattern.rows(),
    })?;
    let columns =
        usize::try_from(pattern.columns()).map_err(|_| SparseError::IndexWidthOverflow {
            width,
            parameter: "columns",
            value: pattern.columns(),
        })?;
    Ok((rows, columns))
}

fn own_pattern(
    pattern: CscPatternRef<'_>,
    width: SparseIndexWidth,
) -> Result<PatternOwnership, SparseError> {
    checked_pattern(pattern, width)?;
    Ok(PatternOwnership {
        rows: pattern.rows(),
        columns: pattern.columns(),
        fingerprint: pattern.fingerprint(),
        col_offsets: copy_u64("faer symbolic column offsets", pattern.col_offsets())?,
        row_indices: copy_u64("faer symbolic row indices", pattern.row_indices())?,
        metrics: SparseMetrics {
            symbolic_work: pattern.columns().saturating_add(pattern.nnz()),
            numeric_work: 0,
            input_nnz: pattern.nnz(),
            output_nnz: 0,
        },
    })
}

fn pattern_matches(
    owned: &PatternOwnership,
    pattern: CscPatternRef<'_>,
) -> Result<(), SparseError> {
    if owned.rows != pattern.rows()
        || owned.columns != pattern.columns()
        || owned.fingerprint != pattern.fingerprint()
        || owned.col_offsets != pattern.col_offsets()
        || owned.row_indices != pattern.row_indices()
    {
        return Err(SparseError::SymbolicPatternMismatch);
    }
    Ok(())
}

fn to_faer_matrix<T: BridgeScalar>(
    matrix: CscMatrixRef<'_, T>,
    width: SparseIndexWidth,
    operation: &'static str,
    phase: SparsePhase,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseColMat<usize, T::Faer>, SparseError> {
    let pattern = matrix.pattern();
    let (rows, columns) = checked_pattern(pattern, width)?;
    let mut triplets = Vec::new();
    let nnz = usize::try_from(pattern.nnz()).map_err(|_| SparseError::IndexWidthOverflow {
        width,
        parameter: "nnz",
        value: pattern.nnz(),
    })?;
    triplets
        .try_reserve_exact(nnz)
        .map_err(|_| allocation_error(operation, nnz))?;
    for column in 0..columns {
        cancellation.checkpoint(operation, phase)?;
        let start = usize::try_from(pattern.col_offsets()[column]).map_err(|_| {
            SparseError::IndexWidthOverflow {
                width,
                parameter: "column offset",
                value: pattern.col_offsets()[column],
            }
        })?;
        let end = usize::try_from(pattern.col_offsets()[column + 1]).map_err(|_| {
            SparseError::IndexWidthOverflow {
                width,
                parameter: "column offset",
                value: pattern.col_offsets()[column + 1],
            }
        })?;
        for index in start..end {
            let row = usize::try_from(pattern.row_indices()[index]).map_err(|_| {
                SparseError::IndexWidthOverflow {
                    width,
                    parameter: "row index",
                    value: pattern.row_indices()[index],
                }
            })?;
            triplets.push(Triplet::new(row, column, matrix.values()[index].to_faer()));
        }
    }
    cancellation.checkpoint(operation, phase)?;
    let created = faer_call(operation, || {
        SparseColMat::try_new_from_triplets(rows, columns, &triplets)
    })?;
    cancellation.checkpoint(operation, phase)?;
    created.map_err(|error| map_creation_error(error, operation, pattern))
}

fn pattern_matrix(
    pattern: CscPatternRef<'_>,
    width: SparseIndexWidth,
    operation: &'static str,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseColMat<usize, f64>, SparseError> {
    let values = try_filled(
        "faer symbolic dummy values",
        pattern.row_indices().len(),
        1.0,
    )?;
    let matrix = CscMatrixRef::new(
        pattern.rows(),
        pattern.columns(),
        pattern.col_offsets(),
        pattern.row_indices(),
        &values,
    )?;
    to_faer_matrix(
        matrix,
        width,
        operation,
        SparsePhase::Symbolic,
        cancellation,
    )
}

fn map_creation_error(
    error: CreationError,
    operation: &'static str,
    pattern: CscPatternRef<'_>,
) -> SparseError {
    match error {
        CreationError::Generic(error) => map_faer_error(error, operation, pattern),
        CreationError::OutOfBounds { .. } => SparseError::InvalidCsc {
            invariant: "faer rejected a validated CSC coordinate",
            column: None,
        },
    }
}

fn map_faer_error(
    error: FaerError,
    operation: &'static str,
    pattern: CscPatternRef<'_>,
) -> SparseError {
    match error {
        FaerError::IndexOverflow => SparseError::IndexWidthOverflow {
            width: if usize::BITS >= 64 {
                SparseIndexWidth::I64
            } else {
                SparseIndexWidth::I32
            },
            parameter: "faer sparse structure",
            value: pattern.rows().max(pattern.columns()).max(pattern.nnz()),
        },
        FaerError::OutOfMemory => SparseError::AllocationFailure {
            operation,
            elements: pattern.nnz(),
        },
        _ => SparseError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: format!("unrecognized faer error: {error:?}"),
        },
    }
}

fn analyze_lu(
    pattern: CscPatternRef<'_>,
    width: SparseIndexWidth,
    cancellation: SparseCancellation<'_>,
) -> Result<FaerLuSymbolic, SparseError> {
    cancellation.checkpoint("faer LU symbolic analysis", SparsePhase::Symbolic)?;
    if pattern.rows() != pattern.columns() {
        return Err(SparseError::SquareMatrixRequired {
            rows: pattern.rows(),
            columns: pattern.columns(),
        });
    }
    let matrix = pattern_matrix(pattern, width, "faer LU CSC conversion", cancellation)?;
    let params = LuSymbolicParams {
        supernodal_flop_ratio_threshold: SupernodalThreshold::FORCE_SIMPLICIAL,
        ..Default::default()
    };
    cancellation.checkpoint("faer LU symbolic analysis", SparsePhase::Symbolic)?;
    let result = faer_call("faer LU symbolic analysis", || {
        factorize_symbolic_lu(matrix.symbolic(), params)
    })?;
    cancellation.checkpoint("faer LU symbolic analysis", SparsePhase::Symbolic)?;
    Ok(FaerLuSymbolic {
        pattern: own_pattern(pattern, width)?,
        inner: result
            .map_err(|error| map_faer_error(error, "faer LU symbolic analysis", pattern))?,
    })
}

fn analyze_cholesky(
    pattern: CscPatternRef<'_>,
    width: SparseIndexWidth,
    cancellation: SparseCancellation<'_>,
) -> Result<FaerCholeskySymbolic, SparseError> {
    cancellation.checkpoint("faer Cholesky symbolic analysis", SparsePhase::Symbolic)?;
    if pattern.rows() != pattern.columns() {
        return Err(SparseError::SquareMatrixRequired {
            rows: pattern.rows(),
            columns: pattern.columns(),
        });
    }
    let matrix = pattern_matrix(pattern, width, "faer Cholesky CSC conversion", cancellation)?;
    cancellation.checkpoint("faer Cholesky symbolic analysis", SparsePhase::Symbolic)?;
    let result = faer_call("faer Cholesky symbolic analysis", || {
        SymbolicLlt::try_new(matrix.symbolic(), Side::Lower)
    })?;
    cancellation.checkpoint("faer Cholesky symbolic analysis", SparsePhase::Symbolic)?;
    Ok(FaerCholeskySymbolic {
        pattern: own_pattern(pattern, width)?,
        inner: result
            .map_err(|error| map_faer_error(error, "faer Cholesky symbolic analysis", pattern))?,
    })
}

fn analyze_qr(
    pattern: CscPatternRef<'_>,
    width: SparseIndexWidth,
    cancellation: SparseCancellation<'_>,
) -> Result<FaerQrSymbolic, SparseError> {
    cancellation.checkpoint("faer QR symbolic analysis", SparsePhase::Symbolic)?;
    if pattern.rows() < pattern.columns() {
        return Err(SparseError::Unsupported {
            provider: PROVIDER_NAME,
            operation: "underdetermined sparse least squares",
        });
    }
    let matrix = pattern_matrix(pattern, width, "faer QR CSC conversion", cancellation)?;
    cancellation.checkpoint("faer QR symbolic analysis", SparsePhase::Symbolic)?;
    let inner = faer_call("faer QR symbolic analysis", || {
        SymbolicQr::try_new(matrix.symbolic())
    })?;
    cancellation.checkpoint("faer QR symbolic analysis", SparsePhase::Symbolic)?;
    let params = QrSymbolicParams {
        supernodal_flop_ratio_threshold: SupernodalThreshold::FORCE_SIMPLICIAL,
        ..Default::default()
    };
    cancellation.checkpoint("faer QR rank symbolic analysis", SparsePhase::Symbolic)?;
    let diagnostic = faer_call("faer QR rank symbolic analysis", || {
        factorize_symbolic_qr(matrix.symbolic(), params)
    })?;
    cancellation.checkpoint("faer QR symbolic analysis", SparsePhase::Symbolic)?;
    Ok(FaerQrSymbolic {
        pattern: own_pattern(pattern, width)?,
        inner: inner
            .map_err(|error| map_faer_error(error, "faer QR symbolic analysis", pattern))?,
        diagnostic: diagnostic
            .map(Arc::new)
            .map_err(|error| map_faer_error(error, "faer QR rank symbolic analysis", pattern))?,
    })
}

fn factor_lu<T: BridgeScalar>(
    symbolic: &FaerLuSymbolic,
    matrix: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<FaerLuCore<T::Faer>, SparseError> {
    cancellation.checkpoint("faer LU factorization", SparsePhase::Numeric)?;
    pattern_matches(&symbolic.pattern, matrix.pattern())?;
    let faer_matrix = to_faer_matrix(
        matrix,
        provider_index_width(),
        "faer LU CSC conversion",
        SparsePhase::Numeric,
        cancellation,
    )?;
    let order = usize::try_from(symbolic.pattern.columns)
        .map_err(|_| allocation_error("faer LU order", usize::MAX))?;
    let mut row_perm = try_filled("faer LU row permutation", order, 0_usize)?;
    let mut row_perm_inv = try_filled("faer LU inverse row permutation", order, 0_usize)?;
    let mut numeric = simplicial_lu::SimplicialLu::new();
    let req =
        simplicial_lu::factorize_simplicial_numeric_lu_scratch::<usize, T::Faer>(order, order);
    let mut memory =
        MemBuffer::try_new(req).map_err(|_| allocation_error("faer LU workspace", order))?;
    cancellation.checkpoint("faer LU factorization", SparsePhase::Numeric)?;
    let result = faer_call("faer LU factorization", || {
        simplicial_lu::factorize_simplicial_numeric_lu(
            &mut row_perm,
            &mut row_perm_inv,
            &mut numeric,
            faer_matrix.as_ref(),
            symbolic.inner.col_perm(),
            MemStack::new(&mut memory),
        )
    })?;
    cancellation.checkpoint("faer LU factorization", SparsePhase::Numeric)?;
    result.map_err(map_lu_error)?;
    validate_lu_diagonal::<T>(&numeric)?;
    Ok(FaerLuCore {
        symbolic: symbolic.clone(),
        numeric,
        row_perm,
        row_perm_inv,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: 0,
            input_nnz: matrix.pattern().nnz(),
            output_nnz: 0,
        },
    })
}

fn validate_lu_diagonal<T: BridgeScalar>(
    numeric: &simplicial_lu::SimplicialLu<usize, T::Faer>,
) -> Result<(), SparseError> {
    let upper = numeric.u_factor_unsorted();
    for column in 0..upper.ncols() {
        let diagonal = upper
            .row_idx_of_col(column)
            .zip(upper.val_of_col(column))
            .find_map(|(row, value)| (row == column).then_some(value));
        if diagonal.is_none_or(|value| T::is_zero(value) || !T::faer_is_finite(value)) {
            return Err(SparseError::Singular {
                provider: PROVIDER_NAME,
                pivot: u64::try_from(column).unwrap_or(u64::MAX).saturating_add(1),
            });
        }
    }
    Ok(())
}

fn map_lu_error(error: LuError) -> SparseError {
    match error {
        LuError::SymbolicSingular { index } => SparseError::Singular {
            provider: PROVIDER_NAME,
            pivot: u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1),
        },
        LuError::Generic(error) => map_factor_faer_error(error, "faer LU factorization"),
    }
}

fn factor_cholesky<T: BridgeScalar>(
    symbolic: &FaerCholeskySymbolic,
    matrix: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<FaerCholeskyCore<T::Faer>, SparseError> {
    cancellation.checkpoint("faer Cholesky factorization", SparsePhase::Numeric)?;
    pattern_matches(&symbolic.pattern, matrix.pattern())?;
    let faer_matrix = to_faer_matrix(
        matrix,
        provider_index_width(),
        "faer Cholesky CSC conversion",
        SparsePhase::Numeric,
        cancellation,
    )?;
    cancellation.checkpoint("faer Cholesky factorization", SparsePhase::Numeric)?;
    let result = faer_call("faer Cholesky factorization", || {
        Llt::try_new_with_symbolic(symbolic.inner.clone(), faer_matrix.as_ref(), Side::Lower)
    })?;
    cancellation.checkpoint("faer Cholesky factorization", SparsePhase::Numeric)?;
    let numeric = result.map_err(map_llt_error)?;
    Ok(FaerCholeskyCore {
        symbolic: symbolic.clone(),
        numeric,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: 0,
            input_nnz: matrix.pattern().nnz(),
            output_nnz: 0,
        },
    })
}

fn map_llt_error(error: LltError) -> SparseError {
    match error {
        LltError::Numeric(faer::linalg::cholesky::llt::factor::LltError::NonPositivePivot {
            index,
        }) => SparseError::NotPositiveDefinite {
            provider: PROVIDER_NAME,
            minor: u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1),
        },
        LltError::Generic(error) => map_factor_faer_error(error, "faer Cholesky factorization"),
    }
}

fn map_factor_faer_error(error: FaerError, operation: &'static str) -> SparseError {
    match error {
        FaerError::OutOfMemory => SparseError::AllocationFailure {
            operation,
            elements: 0,
        },
        FaerError::IndexOverflow => SparseError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: "faer reported index overflow after adapter checks".to_string(),
        },
        _ => SparseError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: format!("unrecognized faer error: {error:?}"),
        },
    }
}

fn factor_qr<T: BridgeScalar>(
    symbolic: &FaerQrSymbolic,
    matrix: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<FaerQrCore<T::Faer>, SparseError> {
    cancellation.checkpoint("faer QR factorization", SparsePhase::Numeric)?;
    pattern_matches(&symbolic.pattern, matrix.pattern())?;
    let faer_matrix = to_faer_matrix(
        matrix,
        provider_index_width(),
        "faer QR CSC conversion",
        SparsePhase::Numeric,
        cancellation,
    )?;
    diagnose_qr_rank::<T>(symbolic, &faer_matrix, matrix.values(), cancellation)?;
    cancellation.checkpoint("faer QR factorization", SparsePhase::Numeric)?;
    let result = faer_call("faer QR factorization", || {
        Qr::try_new_with_symbolic(symbolic.inner.clone(), faer_matrix.as_ref())
    })?;
    cancellation.checkpoint("faer QR factorization", SparsePhase::Numeric)?;
    let numeric =
        result.map_err(|error| map_faer_error(error, "faer QR factorization", matrix.pattern()))?;
    Ok(FaerQrCore {
        symbolic: symbolic.clone(),
        numeric,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: 0,
            input_nnz: matrix.pattern().nnz(),
            output_nnz: 0,
        },
    })
}

fn diagnose_qr_rank<T: BridgeScalar>(
    symbolic: &FaerQrSymbolic,
    matrix: &SparseColMat<usize, T::Faer>,
    original_values: &[T],
    cancellation: SparseCancellation<'_>,
) -> Result<(), SparseError> {
    let mut indices = try_filled(
        "faer QR rank indices",
        symbolic.diagnostic.len_idx(),
        0_usize,
    )?;
    let mut values = try_filled(
        "faer QR rank values",
        symbolic.diagnostic.len_val(),
        T::faer_one(),
    )?;
    let req = symbolic
        .diagnostic
        .factorize_numeric_qr_scratch::<T::Faer>(Par::Seq, Spec::default());
    let mut memory = MemBuffer::try_new(req)
        .map_err(|_| allocation_error("faer QR rank workspace", values.len()))?;
    cancellation.checkpoint("faer QR rank factorization", SparsePhase::Numeric)?;
    faer_call("faer QR rank factorization", || {
        let _factor = symbolic.diagnostic.factorize_numeric_qr(
            &mut indices,
            &mut values,
            matrix.as_ref(),
            Par::Seq,
            MemStack::new(&mut memory),
            Spec::default(),
        );
    })?;
    cancellation.checkpoint("faer QR rank factorization", SparsePhase::Numeric)?;
    let columns = usize::try_from(symbolic.pattern.columns)
        .map_err(|_| allocation_error("faer QR columns", usize::MAX))?;
    let norm = original_values
        .iter()
        .map(|value| T::faer_norm(&value.to_faer()).powi(2))
        .sum::<f64>()
        .sqrt();
    let scale = f64::from(
        u32::try_from(symbolic.pattern.rows.max(symbolic.pattern.columns)).unwrap_or(u32::MAX),
    );
    let tolerance = f64::EPSILON * scale * norm.max(1.0);
    let offsets = indices
        .get(..=columns)
        .ok_or_else(|| SparseError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation: "faer QR rank factorization",
            detail: "faer returned an invalid simplicial R offset layout".to_string(),
        })?;
    let r_nnz = offsets[columns];
    let row_start = columns.saturating_add(1);
    let row_end = row_start.saturating_add(r_nnz);
    let rows = indices
        .get(row_start..row_end)
        .ok_or_else(|| SparseError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation: "faer QR rank factorization",
            detail: "faer returned an invalid simplicial R layout".to_string(),
        })?;
    let r_values = values
        .get(..r_nnz)
        .ok_or_else(|| SparseError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation: "faer QR rank factorization",
            detail: "faer returned an invalid simplicial R value layout".to_string(),
        })?;
    let mut rank = 0_u64;
    for column in 0..columns {
        let start = offsets[column];
        let end = offsets[column + 1];
        let column_rows = rows
            .get(start..end)
            .ok_or_else(|| SparseError::ProviderFailure {
                provider: PROVIDER_NAME,
                operation: "faer QR rank factorization",
                detail: "faer returned out-of-range simplicial R offsets".to_string(),
            })?;
        let column_values =
            r_values
                .get(start..end)
                .ok_or_else(|| SparseError::ProviderFailure {
                    provider: PROVIDER_NAME,
                    operation: "faer QR rank factorization",
                    detail: "faer returned out-of-range simplicial R values".to_string(),
                })?;
        let diagonal = column_rows
            .iter()
            .zip(column_values)
            .find_map(|(&row, value)| (row == column).then_some(value));
        if diagonal.is_some_and(|value| T::faer_is_finite(value) && T::faer_norm(value) > tolerance)
        {
            rank = rank.saturating_add(1);
        }
    }
    if rank != symbolic.pattern.columns {
        return Err(SparseError::RankDeficient {
            provider: PROVIDER_NAME,
            rank,
            required_rank: symbolic.pattern.columns,
        });
    }
    Ok(())
}

fn solve_lu<T: BridgeScalar>(
    factor: &FaerLuCore<T::Faer>,
    right_hand_side: &[T],
    right_hand_sides: u64,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<T>, SparseError> {
    cancellation.checkpoint("faer LU solve", SparsePhase::Execution)?;
    let order = factor.symbolic.pattern.columns;
    let (mut rhs, rhs_columns) = dense_rhs::<T>(
        order,
        right_hand_side,
        right_hand_sides,
        "faer LU right-hand side",
    )?;
    cancellation.checkpoint("faer LU solve", SparsePhase::Execution)?;
    let n = usize::try_from(order).map_err(|_| allocation_error("faer LU order", usize::MAX))?;
    let req = simplicial_lu::solve_in_place_scratch::<usize, T::Faer>(n, rhs_columns, Par::Seq);
    let mut memory =
        MemBuffer::try_new(req).map_err(|_| allocation_error("faer LU solve workspace", n))?;
    faer_call("faer LU solve", || {
        factor.numeric.solve_in_place_with_conj(
            PermRef::new_checked(&factor.row_perm, &factor.row_perm_inv, n),
            factor.symbolic.inner.col_perm(),
            Conj::No,
            rhs.as_mut(),
            Par::Seq,
            MemStack::new(&mut memory),
        );
    })?;
    cancellation.checkpoint("faer LU solve", SparsePhase::Execution)?;
    dense_solution::<T>(&rhs, order, right_hand_sides, "faer LU solve")
}

fn solve_cholesky<T: BridgeScalar>(
    factor: &FaerCholeskyCore<T::Faer>,
    right_hand_side: &[T],
    right_hand_sides: u64,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<T>, SparseError> {
    cancellation.checkpoint("faer Cholesky solve", SparsePhase::Execution)?;
    let order = factor.symbolic.pattern.columns;
    let (rhs, _) = dense_rhs::<T>(
        order,
        right_hand_side,
        right_hand_sides,
        "faer Cholesky right-hand side",
    )?;
    cancellation.checkpoint("faer Cholesky solve", SparsePhase::Execution)?;
    let result = faer_call("faer Cholesky solve", || factor.numeric.solve(&rhs))?;
    cancellation.checkpoint("faer Cholesky solve", SparsePhase::Execution)?;
    dense_solution::<T>(&result, order, right_hand_sides, "faer Cholesky solve")
}

fn solve_qr<T: BridgeScalar>(
    factor: &FaerQrCore<T::Faer>,
    right_hand_side: &[T],
    right_hand_sides: u64,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<T>, SparseError> {
    cancellation.checkpoint("faer QR solve", SparsePhase::Execution)?;
    let (rhs, _) = dense_rhs::<T>(
        factor.symbolic.pattern.rows,
        right_hand_side,
        right_hand_sides,
        "faer QR right-hand side",
    )?;
    cancellation.checkpoint("faer QR solve", SparsePhase::Execution)?;
    let result = faer_call("faer QR solve", || factor.numeric.solve_lstsq(&rhs))?;
    cancellation.checkpoint("faer QR solve", SparsePhase::Execution)?;
    dense_solution::<T>(
        &result,
        factor.symbolic.pattern.columns,
        right_hand_sides,
        "faer QR solve",
    )
}

fn dense_rhs<T: BridgeScalar>(
    rows: u64,
    right_hand_side: &[T],
    right_hand_sides: u64,
    operation: &'static str,
) -> Result<(Mat<T::Faer>, usize), SparseError> {
    let expected = rows
        .checked_mul(right_hand_sides)
        .ok_or(SparseError::AllocationFailure {
            operation,
            elements: u64::MAX,
        })?;
    if u64::try_from(right_hand_side.len()).ok() != Some(expected) {
        return Err(SparseError::DimensionMismatch {
            operation,
            expected,
            actual: u64::try_from(right_hand_side.len()).unwrap_or(u64::MAX),
        });
    }
    let rows = usize::try_from(rows).map_err(|_| allocation_error(operation, usize::MAX))?;
    let columns =
        usize::try_from(right_hand_sides).map_err(|_| allocation_error(operation, usize::MAX))?;
    let matrix = faer_call(operation, || {
        Mat::from_fn(rows, columns, |row, column| {
            right_hand_side[row + column * rows].to_faer()
        })
    })?;
    Ok((matrix, columns))
}

fn dense_solution<T: BridgeScalar>(
    matrix: &Mat<T::Faer>,
    rows: u64,
    columns: u64,
    operation: &'static str,
) -> Result<SparseSolveResult<T>, SparseError> {
    let rows_usize = usize::try_from(rows).map_err(|_| allocation_error(operation, usize::MAX))?;
    let columns_usize =
        usize::try_from(columns).map_err(|_| allocation_error(operation, usize::MAX))?;
    let length = rows_usize
        .checked_mul(columns_usize)
        .ok_or_else(|| allocation_error(operation, usize::MAX))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(operation, length))?;
    for column in 0..columns_usize {
        for row in 0..rows_usize {
            let value = &matrix[(row, column)];
            if !T::faer_is_finite(value) {
                return Err(SparseError::ProviderFailure {
                    provider: PROVIDER_NAME,
                    operation,
                    detail: "faer produced a non-finite solution".to_string(),
                });
            }
            values.push(T::from_faer(value));
        }
    }
    Ok(SparseSolveResult {
        rows,
        columns,
        values,
        metrics: SparseMetrics::default(),
    })
}

fn spmv<T: BridgeScalar>(
    matrix: CscMatrixRef<'_, T>,
    vector: &[T],
    cancellation: SparseCancellation<'_>,
) -> Result<(Vec<T>, SparseMetrics), SparseError> {
    let pattern = matrix.pattern();
    if u64::try_from(vector.len()).ok() != Some(pattern.columns()) {
        return Err(SparseError::DimensionMismatch {
            operation: "faer SpMV vector length",
            expected: pattern.columns(),
            actual: u64::try_from(vector.len()).unwrap_or(u64::MAX),
        });
    }
    let faer_matrix = to_faer_matrix(
        matrix,
        provider_index_width(),
        "faer SpMV CSC conversion",
        SparsePhase::Execution,
        cancellation,
    )?;
    let rows =
        usize::try_from(pattern.rows()).map_err(|_| allocation_error("faer SpMV", usize::MAX))?;
    let columns = vector.len();
    let rhs = faer_call("faer SpMV input", || {
        Mat::from_fn(columns, 1, |row, _| vector[row].to_faer())
    })?;
    let mut output = faer_call("faer SpMV output", || Mat::<T::Faer>::zeros(rows, 1))?;
    cancellation.checkpoint("faer SpMV", SparsePhase::Execution)?;
    faer_call("faer SpMV", || {
        sparse_dense_matmul(
            output.as_mut(),
            Accum::Replace,
            faer_matrix.as_ref(),
            rhs.as_ref(),
            T::faer_one(),
            Par::Seq,
        );
    })?;
    cancellation.checkpoint("faer SpMV", SparsePhase::Execution)?;
    let values = (0..rows)
        .map(|row| T::from_faer(&output[(row, 0)]))
        .collect();
    Ok((
        values,
        SparseMetrics {
            symbolic_work: 0,
            numeric_work: 0,
            input_nnz: pattern.nnz(),
            output_nnz: 0,
        },
    ))
}

fn spgemm<T: BridgeScalar>(
    left: CscMatrixRef<'_, T>,
    right: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<(OwnedCscMatrix<T>, SparseMetrics), SparseError> {
    if left.pattern().columns() != right.pattern().rows() {
        return Err(SparseError::DimensionMismatch {
            operation: "faer SpGEMM inner dimension",
            expected: left.pattern().columns(),
            actual: right.pattern().rows(),
        });
    }
    let left_faer = to_faer_matrix(
        left,
        provider_index_width(),
        "faer SpGEMM left CSC conversion",
        SparsePhase::Execution,
        cancellation,
    )?;
    let right_faer = to_faer_matrix(
        right,
        provider_index_width(),
        "faer SpGEMM right CSC conversion",
        SparsePhase::Execution,
        cancellation,
    )?;
    cancellation.checkpoint("faer SpGEMM", SparsePhase::Execution)?;
    let result = faer_call("faer SpGEMM", || {
        sparse_sparse_matmul(
            left_faer.as_ref(),
            right_faer.as_ref(),
            T::faer_one(),
            Par::Seq,
        )
    })?;
    cancellation.checkpoint("faer SpGEMM", SparsePhase::Execution)?;
    let result = result.map_err(|error| map_faer_error(error, "faer SpGEMM", left.pattern()))?;
    let output = from_faer_csc::<T>(&result)?;
    let output_nnz = output.as_ref().pattern().nnz();
    Ok((
        output,
        SparseMetrics {
            symbolic_work: left.pattern().nnz().saturating_add(right.pattern().nnz()),
            numeric_work: 0,
            input_nnz: left.pattern().nnz().saturating_add(right.pattern().nnz()),
            output_nnz,
        },
    ))
}

fn from_faer_csc<T: BridgeScalar>(
    matrix: &SparseColMat<usize, T::Faer>,
) -> Result<OwnedCscMatrix<T>, SparseError> {
    let symbolic = matrix.symbolic();
    let mut col_offsets = Vec::new();
    let mut row_indices = Vec::new();
    let mut values = Vec::new();
    col_offsets
        .try_reserve_exact(symbolic.ncols().saturating_add(1))
        .map_err(|_| {
            allocation_error(
                "faer SpGEMM output offsets",
                symbolic.ncols().saturating_add(1),
            )
        })?;
    col_offsets.push(0);
    for column in 0..symbolic.ncols() {
        for (row, value) in symbolic
            .row_idx_of_col(column)
            .zip(matrix.val_of_col(column))
        {
            if T::is_zero(value) {
                continue;
            }
            row_indices.try_reserve(1).map_err(|_| {
                allocation_error(
                    "faer SpGEMM output rows",
                    row_indices.len().saturating_add(1),
                )
            })?;
            values.try_reserve(1).map_err(|_| {
                allocation_error("faer SpGEMM output values", values.len().saturating_add(1))
            })?;
            row_indices.push(
                u64::try_from(row)
                    .map_err(|_| allocation_error("faer SpGEMM row index", usize::MAX))?,
            );
            values.push(T::from_faer(value));
        }
        col_offsets.push(
            u64::try_from(row_indices.len())
                .map_err(|_| allocation_error("faer SpGEMM output nnz", usize::MAX))?,
        );
    }
    OwnedCscMatrix::new(
        u64::try_from(symbolic.nrows())
            .map_err(|_| allocation_error("faer SpGEMM output rows", usize::MAX))?,
        u64::try_from(symbolic.ncols())
            .map_err(|_| allocation_error("faer SpGEMM output columns", usize::MAX))?,
        col_offsets,
        row_indices,
        values,
    )
}

fn provider_index_width() -> SparseIndexWidth {
    if usize::BITS >= 64 {
        SparseIndexWidth::I64
    } else {
        SparseIndexWidth::I32
    }
}

fn find_value<T: Copy>(matrix: CscMatrixRef<'_, T>, row: u64, column: u64) -> Option<T> {
    let pattern = matrix.pattern();
    let column = usize::try_from(column).ok()?;
    let start = usize::try_from(pattern.col_offsets()[column]).ok()?;
    let end = usize::try_from(pattern.col_offsets()[column + 1]).ok()?;
    let relative = pattern.row_indices()[start..end].binary_search(&row).ok()?;
    Some(matrix.values()[start + relative])
}

fn validate_symmetric(
    matrix: CscMatrixRef<'_, f64>,
    cancellation: SparseCancellation<'_>,
) -> Result<(), SparseError> {
    let pattern = matrix.pattern();
    for column in 0..pattern.columns() {
        cancellation.checkpoint("faer symmetry check", SparsePhase::Numeric)?;
        let start = usize::try_from(pattern.col_offsets()[usize::try_from(column).unwrap_or(0)])
            .map_err(|_| allocation_error("faer symmetry check", usize::MAX))?;
        let end = usize::try_from(pattern.col_offsets()[usize::try_from(column).unwrap_or(0) + 1])
            .map_err(|_| allocation_error("faer symmetry check", usize::MAX))?;
        for index in start..end {
            let row = pattern.row_indices()[index];
            let value = matrix.values()[index];
            let counterpart = find_value(matrix, column, row).unwrap_or(0.0);
            let scale = value.abs().max(counterpart.abs()).max(1.0);
            if (value - counterpart).abs() > f64::EPSILON * scale {
                return Err(SparseError::NotHermitian {
                    provider: PROVIDER_NAME,
                    row: row.saturating_add(1),
                    column: column.saturating_add(1),
                });
            }
        }
    }
    Ok(())
}

fn validate_hermitian(
    matrix: CscMatrixRef<'_, Complex64>,
    cancellation: SparseCancellation<'_>,
) -> Result<(), SparseError> {
    let pattern = matrix.pattern();
    for column in 0..pattern.columns() {
        cancellation.checkpoint("faer Hermitian check", SparsePhase::Numeric)?;
        let column_usize = usize::try_from(column)
            .map_err(|_| allocation_error("faer Hermitian check", usize::MAX))?;
        let start = usize::try_from(pattern.col_offsets()[column_usize])
            .map_err(|_| allocation_error("faer Hermitian check", usize::MAX))?;
        let end = usize::try_from(pattern.col_offsets()[column_usize + 1])
            .map_err(|_| allocation_error("faer Hermitian check", usize::MAX))?;
        for index in start..end {
            let row = pattern.row_indices()[index];
            let value = matrix.values()[index];
            let counterpart = find_value(matrix, column, row)
                .unwrap_or(Complex64::ZERO)
                .conjugate();
            let difference = (value.re - counterpart.re).hypot(value.im - counterpart.im);
            let scale = value
                .re
                .hypot(value.im)
                .max(counterpart.re.hypot(counterpart.im))
                .max(1.0);
            if difference > f64::EPSILON * scale {
                return Err(SparseError::NotHermitian {
                    provider: PROVIDER_NAME,
                    row: row.saturating_add(1),
                    column: column.saturating_add(1),
                });
            }
        }
    }
    Ok(())
}

use openmat_array::Complex64;
use openmat_linalg::{
    CscMatrixRef, CscPatternRef, OwnedCscMatrix, SparseCancellation, SparseCapabilities,
    SparseCholeskyFactor, SparseError, SparseFactorKind, SparseIndexWidth, SparseLuFactor,
    SparseMetrics, SparseNumericFactor, SparsePhase, SparseProvider, SparseQrFactor,
    SparseSolveResult, SparseSymbolicFactor,
};

use crate::scalar::ReferenceScalar;

const PROVIDER_NAME: &str = "openmat-reference-sparse";

/// Deterministic correctness-first provider for small sparse problems.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReferenceSparseProvider;

/// Owned pattern analysis reusable across numeric values with exactly the same CSC structure.
#[derive(Clone, Debug)]
pub struct ReferenceSymbolic {
    kind: SparseFactorKind,
    rows: u64,
    order: u64,
    fingerprint: u64,
    col_offsets: Vec<u64>,
    row_indices: Vec<u64>,
    metrics: SparseMetrics,
}

impl SparseSymbolicFactor for ReferenceSymbolic {
    fn kind(&self) -> SparseFactorKind {
        self.kind
    }
    fn order(&self) -> u64 {
        self.order
    }
    fn pattern_fingerprint(&self) -> u64 {
        self.fingerprint
    }
    fn metrics(&self) -> SparseMetrics {
        self.metrics
    }
}

/// Reusable dense-backed LU factor produced by the reference sparse provider.
#[derive(Clone, Debug)]
pub struct ReferenceLuFactor<T> {
    symbolic: ReferenceSymbolic,
    factors: Vec<T>,
    pivots: Vec<usize>,
    metrics: SparseMetrics,
}

/// Reusable dense-backed lower Cholesky factor produced by the reference provider.
#[derive(Clone, Debug)]
pub struct ReferenceCholeskyFactor<T> {
    symbolic: ReferenceSymbolic,
    lower: Vec<T>,
    metrics: SparseMetrics,
}

/// Reusable dense-backed tall QR factor produced by the reference provider.
#[derive(Clone, Debug)]
pub struct ReferenceQrFactor<T> {
    symbolic: ReferenceSymbolic,
    q: Vec<T>,
    r: Vec<T>,
    metrics: SparseMetrics,
}

impl<T: ReferenceScalar> SparseNumericFactor<T> for ReferenceLuFactor<T> {
    fn symbolic(&self) -> &dyn SparseSymbolicFactor {
        &self.symbolic
    }
    fn metrics(&self) -> SparseMetrics {
        self.metrics
    }

    fn solve(
        &self,
        right_hand_side: &[T],
        right_hand_sides: u64,
        cancellation: SparseCancellation<'_>,
    ) -> Result<SparseSolveResult<T>, SparseError> {
        solve_lu(self, right_hand_side, right_hand_sides, cancellation)
    }
}

impl<T: ReferenceScalar> SparseLuFactor<T> for ReferenceLuFactor<T> {}

impl<T: ReferenceScalar> SparseNumericFactor<T> for ReferenceCholeskyFactor<T> {
    fn symbolic(&self) -> &dyn SparseSymbolicFactor {
        &self.symbolic
    }
    fn metrics(&self) -> SparseMetrics {
        self.metrics
    }

    fn solve(
        &self,
        right_hand_side: &[T],
        right_hand_sides: u64,
        cancellation: SparseCancellation<'_>,
    ) -> Result<SparseSolveResult<T>, SparseError> {
        solve_cholesky(self, right_hand_side, right_hand_sides, cancellation)
    }
}

impl<T: ReferenceScalar> SparseCholeskyFactor<T> for ReferenceCholeskyFactor<T> {}

impl<T: ReferenceScalar> SparseNumericFactor<T> for ReferenceQrFactor<T> {
    fn symbolic(&self) -> &dyn SparseSymbolicFactor {
        &self.symbolic
    }

    fn metrics(&self) -> SparseMetrics {
        self.metrics
    }

    fn solve(
        &self,
        right_hand_side: &[T],
        right_hand_sides: u64,
        cancellation: SparseCancellation<'_>,
    ) -> Result<SparseSolveResult<T>, SparseError> {
        solve_qr(self, right_hand_side, right_hand_sides, cancellation)
    }
}

impl<T: ReferenceScalar> SparseQrFactor<T> for ReferenceQrFactor<T> {}

impl SparseProvider for ReferenceSparseProvider {
    type LuSymbolic = ReferenceSymbolic;
    type CholeskySymbolic = ReferenceSymbolic;
    type LuF64 = ReferenceLuFactor<f64>;
    type LuComplex64 = ReferenceLuFactor<Complex64>;
    type CholeskyF64 = ReferenceCholeskyFactor<f64>;
    type CholeskyComplex64 = ReferenceCholeskyFactor<Complex64>;
    type QrSymbolic = ReferenceSymbolic;
    type QrF64 = ReferenceQrFactor<f64>;
    type QrComplex64 = ReferenceQrFactor<Complex64>;

    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }
    fn index_width(&self) -> SparseIndexWidth {
        SparseIndexWidth::I64
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
        analyze(
            pattern,
            SparseFactorKind::Lu,
            self.index_width(),
            cancellation,
        )
    }

    fn factor_lu_f64(
        &self,
        symbolic: &Self::LuSymbolic,
        matrix: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::LuF64, SparseError> {
        factor_lu(symbolic, matrix, cancellation)
    }

    fn factor_lu_complex64(
        &self,
        symbolic: &Self::LuSymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::LuComplex64, SparseError> {
        factor_lu(symbolic, matrix, cancellation)
    }

    fn analyze_cholesky(
        &self,
        pattern: CscPatternRef<'_>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskySymbolic, SparseError> {
        analyze(
            pattern,
            SparseFactorKind::Cholesky,
            self.index_width(),
            cancellation,
        )
    }

    fn factor_cholesky_f64(
        &self,
        symbolic: &Self::CholeskySymbolic,
        matrix: CscMatrixRef<'_, f64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskyF64, SparseError> {
        factor_cholesky(symbolic, matrix, cancellation)
    }

    fn factor_cholesky_complex64(
        &self,
        symbolic: &Self::CholeskySymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::CholeskyComplex64, SparseError> {
        factor_cholesky(symbolic, matrix, cancellation)
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
        factor_qr(symbolic, matrix, cancellation)
    }

    fn factor_qr_complex64(
        &self,
        symbolic: &Self::QrSymbolic,
        matrix: CscMatrixRef<'_, Complex64>,
        cancellation: SparseCancellation<'_>,
    ) -> Result<Self::QrComplex64, SparseError> {
        factor_qr(symbolic, matrix, cancellation)
    }
}

fn to_usize(operation: &'static str, value: u64) -> Result<usize, SparseError> {
    usize::try_from(value).map_err(|_| SparseError::AllocationFailure {
        operation,
        elements: value,
    })
}

fn product_usize(operation: &'static str, left: u64, right: u64) -> Result<usize, SparseError> {
    let elements = left
        .checked_mul(right)
        .ok_or(SparseError::AllocationFailure {
            operation,
            elements: u64::MAX,
        })?;
    to_usize(operation, elements)
}

fn filled<T: Clone>(
    operation: &'static str,
    length: usize,
    value: T,
) -> Result<Vec<T>, SparseError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| SparseError::AllocationFailure {
            operation,
            elements: u64::try_from(length).unwrap_or(u64::MAX),
        })?;
    output.resize(length, value);
    Ok(output)
}

fn copy_slice<T: Copy>(operation: &'static str, source: &[T]) -> Result<Vec<T>, SparseError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(source.len())
        .map_err(|_| SparseError::AllocationFailure {
            operation,
            elements: u64::try_from(source.len()).unwrap_or(u64::MAX),
        })?;
    output.extend_from_slice(source);
    Ok(output)
}

fn reserve_sparse_entry<T>(
    row_indices: &mut Vec<u64>,
    values: &mut Vec<T>,
) -> Result<(), SparseError> {
    row_indices
        .try_reserve(1)
        .map_err(|_| SparseError::AllocationFailure {
            operation: "sparse matrix-matrix row indices",
            elements: u64::try_from(row_indices.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        })?;
    values
        .try_reserve(1)
        .map_err(|_| SparseError::AllocationFailure {
            operation: "sparse matrix-matrix values",
            elements: u64::try_from(values.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        })
}

fn check_pattern(
    symbolic: &ReferenceSymbolic,
    pattern: CscPatternRef<'_>,
    kind: SparseFactorKind,
) -> Result<(), SparseError> {
    if symbolic.kind != kind
        || symbolic.rows != pattern.rows()
        || symbolic.order != pattern.columns()
        || symbolic.fingerprint != pattern.fingerprint()
        || symbolic.col_offsets != pattern.col_offsets()
        || symbolic.row_indices != pattern.row_indices()
    {
        return Err(SparseError::SymbolicPatternMismatch);
    }
    Ok(())
}

fn analyze(
    pattern: CscPatternRef<'_>,
    kind: SparseFactorKind,
    width: SparseIndexWidth,
    cancellation: SparseCancellation<'_>,
) -> Result<ReferenceSymbolic, SparseError> {
    cancellation.checkpoint("sparse symbolic analysis", SparsePhase::Symbolic)?;
    if pattern.rows() != pattern.columns() {
        return Err(SparseError::SquareMatrixRequired {
            rows: pattern.rows(),
            columns: pattern.columns(),
        });
    }
    pattern.check_index_width(width)?;
    let col_offsets = copy_slice("sparse symbolic column offsets", pattern.col_offsets())?;
    let row_indices = copy_slice("sparse symbolic row indices", pattern.row_indices())?;
    cancellation.checkpoint("sparse symbolic analysis", SparsePhase::Symbolic)?;
    Ok(ReferenceSymbolic {
        kind,
        rows: pattern.rows(),
        order: pattern.columns(),
        fingerprint: pattern.fingerprint(),
        col_offsets,
        row_indices,
        metrics: SparseMetrics {
            symbolic_work: pattern.columns().saturating_add(pattern.nnz()),
            numeric_work: 0,
            input_nnz: pattern.nnz(),
            output_nnz: 0,
        },
    })
}

fn analyze_qr(
    pattern: CscPatternRef<'_>,
    width: SparseIndexWidth,
    cancellation: SparseCancellation<'_>,
) -> Result<ReferenceSymbolic, SparseError> {
    cancellation.checkpoint("sparse QR symbolic analysis", SparsePhase::Symbolic)?;
    if pattern.rows() < pattern.columns() {
        return Err(SparseError::Unsupported {
            provider: PROVIDER_NAME,
            operation: "underdetermined sparse least squares",
        });
    }
    pattern.check_index_width(width)?;
    let col_offsets = copy_slice("sparse QR symbolic column offsets", pattern.col_offsets())?;
    let row_indices = copy_slice("sparse QR symbolic row indices", pattern.row_indices())?;
    cancellation.checkpoint("sparse QR symbolic analysis", SparsePhase::Symbolic)?;
    Ok(ReferenceSymbolic {
        kind: SparseFactorKind::Qr,
        rows: pattern.rows(),
        order: pattern.columns(),
        fingerprint: pattern.fingerprint(),
        col_offsets,
        row_indices,
        metrics: SparseMetrics {
            symbolic_work: pattern.columns().saturating_add(pattern.nnz()),
            numeric_work: 0,
            input_nnz: pattern.nnz(),
            output_nnz: 0,
        },
    })
}

fn spmv<T: ReferenceScalar>(
    matrix: CscMatrixRef<'_, T>,
    vector: &[T],
    cancellation: SparseCancellation<'_>,
) -> Result<(Vec<T>, SparseMetrics), SparseError> {
    let pattern = matrix.pattern();
    if u64::try_from(vector.len()).ok() != Some(pattern.columns()) {
        return Err(SparseError::DimensionMismatch {
            operation: "sparse matrix-vector multiply vector length",
            expected: pattern.columns(),
            actual: u64::try_from(vector.len()).unwrap_or(u64::MAX),
        });
    }
    let rows = to_usize("sparse matrix-vector result", pattern.rows())?;
    let mut output = filled("sparse matrix-vector result", rows, T::zero())?;
    let mut work = 0_u64;
    for (column, &vector_value) in vector.iter().enumerate() {
        cancellation.checkpoint("sparse matrix-vector multiply", SparsePhase::Execution)?;
        let start = to_usize("sparse column offset", pattern.col_offsets()[column])?;
        let end = to_usize("sparse column offset", pattern.col_offsets()[column + 1])?;
        for index in start..end {
            let row = to_usize("sparse row index", pattern.row_indices()[index])?;
            output[row] = output[row].add(matrix.values()[index].multiply(vector_value));
            work = work.saturating_add(2);
        }
    }
    Ok((
        output,
        SparseMetrics {
            symbolic_work: 0,
            numeric_work: work,
            input_nnz: pattern.nnz(),
            output_nnz: 0,
        },
    ))
}

fn spgemm<T: ReferenceScalar>(
    left: CscMatrixRef<'_, T>,
    right: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<(OwnedCscMatrix<T>, SparseMetrics), SparseError> {
    let left_pattern = left.pattern();
    let right_pattern = right.pattern();
    if left_pattern.columns() != right_pattern.rows() {
        return Err(SparseError::DimensionMismatch {
            operation: "sparse matrix-matrix multiply inner dimension",
            expected: left_pattern.columns(),
            actual: right_pattern.rows(),
        });
    }
    let rows = to_usize("sparse matrix-matrix accumulator", left_pattern.rows())?;
    let columns = to_usize("sparse matrix-matrix columns", right_pattern.columns())?;
    let mut accumulator = filled("sparse matrix-matrix accumulator", rows, T::zero())?;
    let mut touched = filled("sparse matrix-matrix touched rows", rows, false)?;
    let mut col_offsets = Vec::new();
    col_offsets
        .try_reserve_exact(columns.saturating_add(1))
        .map_err(|_| SparseError::AllocationFailure {
            operation: "sparse matrix-matrix column offsets",
            elements: right_pattern.columns().saturating_add(1),
        })?;
    col_offsets.push(0);
    let mut row_indices = Vec::new();
    let mut values = Vec::new();
    let mut work = 0_u64;

    for column in 0..columns {
        cancellation.checkpoint("sparse matrix-matrix multiply", SparsePhase::Execution)?;
        let right_start = to_usize("sparse column offset", right_pattern.col_offsets()[column])?;
        let right_end = to_usize(
            "sparse column offset",
            right_pattern.col_offsets()[column + 1],
        )?;
        for right_index in right_start..right_end {
            let shared = to_usize("sparse row index", right_pattern.row_indices()[right_index])?;
            let left_start = to_usize("sparse column offset", left_pattern.col_offsets()[shared])?;
            let left_end = to_usize(
                "sparse column offset",
                left_pattern.col_offsets()[shared + 1],
            )?;
            for left_index in left_start..left_end {
                let row = to_usize("sparse row index", left_pattern.row_indices()[left_index])?;
                accumulator[row] = accumulator[row]
                    .add(left.values()[left_index].multiply(right.values()[right_index]));
                touched[row] = true;
                work = work.saturating_add(2);
            }
        }
        for row in 0..rows {
            if touched[row] {
                if !accumulator[row].is_zero() {
                    reserve_sparse_entry(&mut row_indices, &mut values)?;
                    row_indices.push(u64::try_from(row).map_err(|_| {
                        SparseError::AllocationFailure {
                            operation: "sparse matrix-matrix row index",
                            elements: u64::MAX,
                        }
                    })?);
                    values.push(accumulator[row]);
                }
                accumulator[row] = T::zero();
                touched[row] = false;
            }
        }
        col_offsets.push(u64::try_from(row_indices.len()).map_err(|_| {
            SparseError::AllocationFailure {
                operation: "sparse matrix-matrix stored entries",
                elements: u64::MAX,
            }
        })?);
    }
    let output_nnz = u64::try_from(values.len()).unwrap_or(u64::MAX);
    let result = OwnedCscMatrix::new(
        left_pattern.rows(),
        right_pattern.columns(),
        col_offsets,
        row_indices,
        values,
    )?;
    Ok((
        result,
        SparseMetrics {
            symbolic_work: left_pattern.nnz().saturating_add(right_pattern.nnz()),
            numeric_work: work,
            input_nnz: left_pattern.nnz().saturating_add(right_pattern.nnz()),
            output_nnz,
        },
    ))
}

fn dense_from_csc<T: ReferenceScalar>(matrix: CscMatrixRef<'_, T>) -> Result<Vec<T>, SparseError> {
    let pattern = matrix.pattern();
    let rows = to_usize("sparse factor dense rows", pattern.rows())?;
    let columns = to_usize("sparse factor dense columns", pattern.columns())?;
    let mut dense = filled(
        "sparse factor dense workspace",
        product_usize(
            "sparse factor dense workspace",
            pattern.rows(),
            pattern.columns(),
        )?,
        T::zero(),
    )?;
    for column in 0..columns {
        let start = to_usize("sparse column offset", pattern.col_offsets()[column])?;
        let end = to_usize("sparse column offset", pattern.col_offsets()[column + 1])?;
        for index in start..end {
            let row = to_usize("sparse row index", pattern.row_indices()[index])?;
            dense[row + column * rows] = matrix.values()[index];
        }
    }
    Ok(dense)
}

fn factor_lu<T: ReferenceScalar>(
    symbolic: &ReferenceSymbolic,
    matrix: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<ReferenceLuFactor<T>, SparseError> {
    cancellation.checkpoint("sparse LU factorization", SparsePhase::Numeric)?;
    check_pattern(symbolic, matrix.pattern(), SparseFactorKind::Lu)?;
    let n = to_usize("sparse LU order", symbolic.order)?;
    let mut factors = dense_from_csc(matrix)?;
    let mut pivots = Vec::new();
    pivots
        .try_reserve_exact(n)
        .map_err(|_| SparseError::AllocationFailure {
            operation: "sparse LU pivots",
            elements: symbolic.order,
        })?;
    let mut work = 0_u64;
    for pivot_column in 0..n {
        cancellation.checkpoint("sparse LU factorization", SparsePhase::Numeric)?;
        let mut pivot_row = pivot_column;
        let mut pivot_norm = factors[pivot_column + pivot_column * n].norm_squared();
        for row in (pivot_column + 1)..n {
            let candidate = factors[row + pivot_column * n].norm_squared();
            if candidate > pivot_norm {
                pivot_norm = candidate;
                pivot_row = row;
            }
        }
        if pivot_norm == 0.0 || pivot_norm.is_nan() {
            return Err(SparseError::Singular {
                provider: PROVIDER_NAME,
                pivot: u64::try_from(pivot_column)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
            });
        }
        pivots.push(pivot_row);
        if pivot_row != pivot_column {
            for column in 0..n {
                factors.swap(pivot_column + column * n, pivot_row + column * n);
            }
        }
        let diagonal = factors[pivot_column + pivot_column * n];
        for row in (pivot_column + 1)..n {
            let multiplier_index = row + pivot_column * n;
            factors[multiplier_index] = factors[multiplier_index].divide(diagonal);
            let multiplier = factors[multiplier_index];
            work = work.saturating_add(1);
            for column in (pivot_column + 1)..n {
                let index = row + column * n;
                factors[index] = factors[index]
                    .subtract(multiplier.multiply(factors[pivot_column + column * n]));
                work = work.saturating_add(2);
            }
        }
    }
    Ok(ReferenceLuFactor {
        symbolic: symbolic.clone(),
        factors,
        pivots,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: work,
            input_nnz: matrix.pattern().nnz(),
            output_nnz: 0,
        },
    })
}

fn solve_lu<T: ReferenceScalar>(
    factor: &ReferenceLuFactor<T>,
    right_hand_side: &[T],
    right_hand_sides: u64,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<T>, SparseError> {
    let n = to_usize("sparse LU solve order", factor.symbolic.order)?;
    let columns = to_usize("sparse LU right-hand sides", right_hand_sides)?;
    let expected = factor.symbolic.order.checked_mul(right_hand_sides).ok_or(
        SparseError::AllocationFailure {
            operation: "sparse LU right-hand side",
            elements: u64::MAX,
        },
    )?;
    if u64::try_from(right_hand_side.len()).ok() != Some(expected) {
        return Err(SparseError::DimensionMismatch {
            operation: "sparse LU right-hand side length",
            expected,
            actual: u64::try_from(right_hand_side.len()).unwrap_or(u64::MAX),
        });
    }
    let mut solution = copy_slice("sparse LU solution", right_hand_side)?;
    let mut work = 0_u64;
    for rhs in 0..columns {
        cancellation.checkpoint("sparse LU solve", SparsePhase::Execution)?;
        let base = rhs * n;
        for (row, &pivot) in factor.pivots.iter().enumerate() {
            if row != pivot {
                solution.swap(base + row, base + pivot);
            }
        }
        for row in 0..n {
            let mut value = solution[base + row];
            for column in 0..row {
                value = value
                    .subtract(factor.factors[row + column * n].multiply(solution[base + column]));
                work = work.saturating_add(2);
            }
            solution[base + row] = value;
        }
        for row in (0..n).rev() {
            let mut value = solution[base + row];
            for column in (row + 1)..n {
                value = value
                    .subtract(factor.factors[row + column * n].multiply(solution[base + column]));
                work = work.saturating_add(2);
            }
            solution[base + row] = value.divide(factor.factors[row + row * n]);
            work = work.saturating_add(1);
        }
    }
    Ok(SparseSolveResult {
        rows: factor.symbolic.order,
        columns: right_hand_sides,
        values: solution,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: work,
            input_nnz: 0,
            output_nnz: 0,
        },
    })
}

fn factor_cholesky<T: ReferenceScalar>(
    symbolic: &ReferenceSymbolic,
    matrix: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<ReferenceCholeskyFactor<T>, SparseError> {
    cancellation.checkpoint("sparse Cholesky factorization", SparsePhase::Numeric)?;
    check_pattern(symbolic, matrix.pattern(), SparseFactorKind::Cholesky)?;
    let n = to_usize("sparse Cholesky order", symbolic.order)?;
    let dense = dense_from_csc(matrix)?;
    check_hermitian(&dense, n)?;
    let mut lower = filled(
        "sparse Cholesky factor",
        product_usize("sparse Cholesky factor", symbolic.order, symbolic.order)?,
        T::zero(),
    )?;
    let mut work = 0_u64;
    for column in 0..n {
        cancellation.checkpoint("sparse Cholesky factorization", SparsePhase::Numeric)?;
        for row in column..n {
            let mut value = dense[row + column * n];
            for inner in 0..column {
                value = value.subtract(
                    lower[row + inner * n].multiply(lower[column + inner * n].conjugate()),
                );
                work = work.saturating_add(2);
            }
            if row == column {
                let scale = dense[column + column * n].norm_squared().sqrt().max(1.0);
                if value.imaginary().abs() > f64::EPSILON * scale
                    || value.real() <= 0.0
                    || !value.real().is_finite()
                {
                    return Err(SparseError::NotPositiveDefinite {
                        provider: PROVIDER_NAME,
                        minor: u64::try_from(column).unwrap_or(u64::MAX).saturating_add(1),
                    });
                }
                lower[row + column * n] = T::from_real(value.real().sqrt());
            } else {
                lower[row + column * n] = value.divide(lower[column + column * n]);
                work = work.saturating_add(1);
            }
        }
    }
    Ok(ReferenceCholeskyFactor {
        symbolic: symbolic.clone(),
        lower,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: work,
            input_nnz: matrix.pattern().nnz(),
            output_nnz: 0,
        },
    })
}

fn check_hermitian<T: ReferenceScalar>(dense: &[T], order: usize) -> Result<(), SparseError> {
    let tolerance_scale = f64::from(u32::try_from(order).unwrap_or(u32::MAX));
    for column in 0..order {
        for row in column..order {
            let lower = dense[row + column * order];
            let upper = dense[column + row * order].conjugate();
            let difference = lower.subtract(upper).norm_squared().sqrt();
            let scale = lower
                .norm_squared()
                .sqrt()
                .max(upper.norm_squared().sqrt())
                .max(1.0);
            if !difference.is_finite() || difference > f64::EPSILON * tolerance_scale * scale {
                return Err(SparseError::NotHermitian {
                    provider: PROVIDER_NAME,
                    row: u64::try_from(row).unwrap_or(u64::MAX).saturating_add(1),
                    column: u64::try_from(column).unwrap_or(u64::MAX).saturating_add(1),
                });
            }
        }
    }
    Ok(())
}

fn solve_cholesky<T: ReferenceScalar>(
    factor: &ReferenceCholeskyFactor<T>,
    right_hand_side: &[T],
    right_hand_sides: u64,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<T>, SparseError> {
    let n = to_usize("sparse Cholesky solve order", factor.symbolic.order)?;
    let columns = to_usize("sparse Cholesky right-hand sides", right_hand_sides)?;
    let expected = factor.symbolic.order.checked_mul(right_hand_sides).ok_or(
        SparseError::AllocationFailure {
            operation: "sparse Cholesky right-hand side",
            elements: u64::MAX,
        },
    )?;
    if u64::try_from(right_hand_side.len()).ok() != Some(expected) {
        return Err(SparseError::DimensionMismatch {
            operation: "sparse Cholesky right-hand side length",
            expected,
            actual: u64::try_from(right_hand_side.len()).unwrap_or(u64::MAX),
        });
    }
    let mut solution = copy_slice("sparse Cholesky solution", right_hand_side)?;
    let mut work = 0_u64;
    for rhs in 0..columns {
        cancellation.checkpoint("sparse Cholesky solve", SparsePhase::Execution)?;
        let base = rhs * n;
        for row in 0..n {
            let mut value = solution[base + row];
            for column in 0..row {
                value = value
                    .subtract(factor.lower[row + column * n].multiply(solution[base + column]));
                work = work.saturating_add(2);
            }
            solution[base + row] = value.divide(factor.lower[row + row * n]);
            work = work.saturating_add(1);
        }
        for row in (0..n).rev() {
            let mut value = solution[base + row];
            for column in (row + 1)..n {
                value = value.subtract(
                    factor.lower[column + row * n]
                        .conjugate()
                        .multiply(solution[base + column]),
                );
                work = work.saturating_add(2);
            }
            solution[base + row] = value.divide(factor.lower[row + row * n].conjugate());
            work = work.saturating_add(1);
        }
    }
    Ok(SparseSolveResult {
        rows: factor.symbolic.order,
        columns: right_hand_sides,
        values: solution,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: work,
            input_nnz: 0,
            output_nnz: 0,
        },
    })
}

fn factor_qr<T: ReferenceScalar>(
    symbolic: &ReferenceSymbolic,
    matrix: CscMatrixRef<'_, T>,
    cancellation: SparseCancellation<'_>,
) -> Result<ReferenceQrFactor<T>, SparseError> {
    let pattern = matrix.pattern();
    cancellation.checkpoint("sparse QR factorization", SparsePhase::Numeric)?;
    check_pattern(symbolic, pattern, SparseFactorKind::Qr)?;
    let rows = to_usize("sparse least-squares rows", pattern.rows())?;
    let columns = to_usize("sparse least-squares columns", pattern.columns())?;
    let dense = dense_from_csc(matrix)?;
    let mut q = filled(
        "sparse least-squares Q",
        product_usize("sparse least-squares Q", pattern.rows(), pattern.columns())?,
        T::zero(),
    )?;
    let mut r = filled(
        "sparse least-squares R",
        product_usize(
            "sparse least-squares R",
            pattern.columns(),
            pattern.columns(),
        )?,
        T::zero(),
    )?;
    let matrix_norm = dense
        .iter()
        .map(|value| value.norm_squared())
        .sum::<f64>()
        .sqrt();
    let dimension_scale = f64::from(u32::try_from(rows.max(columns)).unwrap_or(u32::MAX));
    let tolerance = f64::EPSILON * dimension_scale * matrix_norm.max(1.0);
    let mut work = 0_u64;
    for column in 0..columns {
        cancellation.checkpoint("sparse QR factorization", SparsePhase::Numeric)?;
        for row in 0..rows {
            q[row + column * rows] = dense[row + column * rows];
        }
        for previous in 0..column {
            let mut projection = T::zero();
            for row in 0..rows {
                projection = projection.add(
                    q[row + previous * rows]
                        .conjugate()
                        .multiply(q[row + column * rows]),
                );
                work = work.saturating_add(2);
            }
            r[previous + column * columns] = projection;
            for row in 0..rows {
                q[row + column * rows] =
                    q[row + column * rows].subtract(q[row + previous * rows].multiply(projection));
                work = work.saturating_add(2);
            }
        }
        let norm = q[column * rows..(column + 1) * rows]
            .iter()
            .map(|value| value.norm_squared())
            .sum::<f64>()
            .sqrt();
        if !norm.is_finite() || norm <= tolerance {
            return Err(SparseError::RankDeficient {
                provider: PROVIDER_NAME,
                rank: u64::try_from(column).unwrap_or(u64::MAX),
                required_rank: symbolic.order,
            });
        }
        r[column + column * columns] = T::from_real(norm);
        for row in 0..rows {
            q[row + column * rows] = q[row + column * rows].divide(T::from_real(norm));
            work = work.saturating_add(1);
        }
    }

    Ok(ReferenceQrFactor {
        symbolic: symbolic.clone(),
        q,
        r,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: work,
            input_nnz: pattern.nnz(),
            output_nnz: 0,
        },
    })
}

fn solve_qr<T: ReferenceScalar>(
    factor: &ReferenceQrFactor<T>,
    right_hand_side: &[T],
    right_hand_sides: u64,
    cancellation: SparseCancellation<'_>,
) -> Result<SparseSolveResult<T>, SparseError> {
    let rows = to_usize("sparse least-squares rows", factor.symbolic.rows)?;
    let columns = to_usize("sparse least-squares columns", factor.symbolic.order)?;
    let rhs_columns = to_usize("sparse least-squares right-hand sides", right_hand_sides)?;
    let expected_rhs = factor.symbolic.rows.checked_mul(right_hand_sides).ok_or(
        SparseError::AllocationFailure {
            operation: "sparse least-squares right-hand side",
            elements: u64::MAX,
        },
    )?;
    if u64::try_from(right_hand_side.len()).ok() != Some(expected_rhs) {
        return Err(SparseError::DimensionMismatch {
            operation: "sparse least-squares right-hand side length",
            expected: expected_rhs,
            actual: u64::try_from(right_hand_side.len()).unwrap_or(u64::MAX),
        });
    }

    let solution_length = product_usize(
        "sparse least-squares solution",
        factor.symbolic.order,
        right_hand_sides,
    )?;
    let mut solution = filled("sparse least-squares solution", solution_length, T::zero())?;
    let mut work = 0_u64;
    for rhs in 0..rhs_columns {
        cancellation.checkpoint("sparse least squares", SparsePhase::Execution)?;
        for column in 0..columns {
            let mut value = T::zero();
            for row in 0..rows {
                value = value.add(
                    factor.q[row + column * rows]
                        .conjugate()
                        .multiply(right_hand_side[row + rhs * rows]),
                );
                work = work.saturating_add(2);
            }
            solution[column + rhs * columns] = value;
        }
        for row in (0..columns).rev() {
            let mut value = solution[row + rhs * columns];
            for column in (row + 1)..columns {
                value = value.subtract(
                    factor.r[row + column * columns].multiply(solution[column + rhs * columns]),
                );
                work = work.saturating_add(2);
            }
            solution[row + rhs * columns] = value.divide(factor.r[row + row * columns]);
            work = work.saturating_add(1);
        }
    }
    Ok(SparseSolveResult {
        rows: factor.symbolic.order,
        columns: right_hand_sides,
        values: solution,
        metrics: SparseMetrics {
            symbolic_work: 0,
            numeric_work: work,
            input_nnz: 0,
            output_nnz: 0,
        },
    })
}

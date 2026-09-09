use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::{Complex32, Complex64, DenseArray, Shape};

use crate::{LinalgError, LinalgProvider, SolveRequest, matrix_dimensions};

const OPERATION: &str = "underdetermined basic solve";
const RANK_PROVIDER: &str = "openmat-linalg";

/// Computes a real binary64 column-pivoted basic solution of `A * X = B`.
///
/// `A` must be an underdetermined `m x n` matrix (`m < n`) and `B` must be an
/// `m x nrhs` matrix. The result has shape `n x nrhs`. The helper selects `m`
/// linearly independent columns using deterministic column-pivoted modified
/// Gram-Schmidt with reorthogonalization, solves the selected square system
/// through `provider`, and leaves every non-pivot variable exactly zero.
///
/// # Errors
///
/// Returns a matrix-rank, dimension-mismatch, cancellation, allocation,
/// rank-deficiency, square-solve, or provider execution error.
pub fn solve_underdetermined_basic_f64(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<f64>,
    right_hand_side: &DenseArray<f64>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<f64>, LinalgError> {
    solve_underdetermined_basic(provider, coefficients, right_hand_side, cancellation)
}

/// Computes a real binary32 column-pivoted basic solution of `A * X = B`.
///
/// Shape, pivot selection, zero-fill, cancellation, and error semantics match
/// [`solve_underdetermined_basic_f64`]. Norms, thresholds,
/// orthogonalization, and the selected square solve remain binary32.
///
/// # Errors
///
/// Returns a matrix-rank, dimension-mismatch, cancellation, allocation,
/// rank-deficiency, square-solve, or provider execution error.
pub fn solve_underdetermined_basic_f32(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<f32>,
    right_hand_side: &DenseArray<f32>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<f32>, LinalgError> {
    solve_underdetermined_basic(provider, coefficients, right_hand_side, cancellation)
}

/// Computes a complex binary64 column-pivoted basic solution of `A * X = B`.
///
/// Shape, pivot selection, zero-fill, cancellation, and error semantics match
/// [`solve_underdetermined_basic_f64`]. Complex orthogonalization uses the
/// conjugate inner product.
///
/// # Errors
///
/// Returns a matrix-rank, dimension-mismatch, cancellation, allocation,
/// rank-deficiency, square-solve, or provider execution error.
pub fn solve_underdetermined_basic_complex64(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<Complex64>,
    right_hand_side: &DenseArray<Complex64>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex64>, LinalgError> {
    solve_underdetermined_basic(provider, coefficients, right_hand_side, cancellation)
}

/// Computes a complex binary32 column-pivoted basic solution of `A * X = B`.
///
/// Shape, pivot selection, zero-fill, cancellation, and error semantics match
/// [`solve_underdetermined_basic_complex64`]. All numerical work remains in
/// binary32 and complex projections use the conjugate inner product.
///
/// # Errors
///
/// Returns a matrix-rank, dimension-mismatch, cancellation, allocation,
/// rank-deficiency, square-solve, or provider execution error.
pub fn solve_underdetermined_basic_complex32(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<Complex32>,
    right_hand_side: &DenseArray<Complex32>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex32>, LinalgError> {
    solve_underdetermined_basic(provider, coefficients, right_hand_side, cancellation)
}

fn solve_underdetermined_basic<T: BasicSolveScalar>(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<T>,
    right_hand_side: &DenseArray<T>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<T>, LinalgError> {
    let (row_count, column_count) = matrix_dimensions(
        "underdetermined basic solve coefficient matrix",
        coefficients,
    )?;
    let (right_hand_side_rows, right_hand_side_count) = matrix_dimensions(
        "underdetermined basic solve right-hand side",
        right_hand_side,
    )?;
    if row_count != right_hand_side_rows {
        return Err(LinalgError::DimensionMismatch {
            operation: "underdetermined basic solve coefficient and right-hand-side rows",
            left: row_count,
            right: right_hand_side_rows,
        });
    }
    if row_count >= column_count {
        return Err(LinalgError::DimensionMismatch {
            operation: "underdetermined basic solve requires fewer rows than columns",
            left: row_count,
            right: column_count,
        });
    }

    check_cancellation(cancellation)?;
    let rows = checked_host_dimension("underdetermined basic solve row dimension", row_count)?;
    let columns =
        checked_host_dimension("underdetermined basic solve column dimension", column_count)?;
    let right_hand_sides = checked_host_dimension(
        "underdetermined basic solve right-hand-side dimension",
        right_hand_side_count,
    )?;
    let output_len = checked_buffer_length(
        "underdetermined basic solve result dimensions",
        columns,
        right_hand_sides,
    )?;

    if rows == 0 {
        let output = try_filled_buffer(
            "underdetermined basic solve zero-row result",
            output_len,
            T::zero(),
        )?;
        check_cancellation(cancellation)?;
        return DenseArray::from_vec(Shape::new([column_count, right_hand_side_count])?, output)
            .map_err(Into::into);
    }

    let pivots = select_pivot_columns(coefficients, rows, columns, cancellation)?;
    check_cancellation(cancellation)?;
    let selected_len = checked_buffer_length(
        "underdetermined basic solve selected matrix dimensions",
        rows,
        rows,
    )?;
    let mut selected = try_filled_buffer(
        "underdetermined basic solve selected matrix",
        selected_len,
        T::zero(),
    )?;
    for (selected_column, &source_column) in pivots.iter().enumerate() {
        check_cancellation(cancellation)?;
        for row in 0..rows {
            check_cancellation(cancellation)?;
            selected[selected_column * rows + row] =
                coefficients.as_slice()[source_column * rows + row];
        }
    }
    let selected = DenseArray::from_vec(Shape::new([row_count, row_count])?, selected)?;

    check_cancellation(cancellation)?;
    let mut request = SolveRequest::new(&selected, right_hand_side);
    if let Some(flag) = cancellation {
        request = request.with_cancellation_flag(flag);
    }
    let selected_solution = T::solve_selected(provider, request)?;
    check_cancellation(cancellation)?;

    let mut output =
        try_filled_buffer("underdetermined basic solve result", output_len, T::zero())?;
    for right_hand_side_column in 0..right_hand_sides {
        check_cancellation(cancellation)?;
        for (selected_row, &output_row) in pivots.iter().enumerate() {
            check_cancellation(cancellation)?;
            output[right_hand_side_column * columns + output_row] =
                selected_solution.as_slice()[right_hand_side_column * rows + selected_row];
        }
    }
    check_cancellation(cancellation)?;
    DenseArray::from_vec(Shape::new([column_count, right_hand_side_count])?, output)
        .map_err(Into::into)
}

fn select_pivot_columns<T: BasicSolveScalar>(
    coefficients: &DenseArray<T>,
    rows: usize,
    columns: usize,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<usize>, LinalgError> {
    let residual_len = checked_buffer_length(
        "underdetermined basic solve orthogonalization dimensions",
        rows,
        columns,
    )?;
    let mut residuals = try_filled_buffer(
        "underdetermined basic solve orthogonalization workspace",
        residual_len,
        T::zero(),
    )?;
    for column in 0..columns {
        check_cancellation(cancellation)?;
        for row in 0..rows {
            check_cancellation(cancellation)?;
            residuals[column * rows + row] = coefficients.as_slice()[column * rows + row];
        }
    }

    let mut permutation = try_index_buffer(columns)?;
    let mut scale = T::Real::zero();
    for column in 0..columns {
        check_cancellation(cancellation)?;
        scale = scale.maximum(column_norm(&residuals, rows, column, cancellation)?);
    }
    let dimension_scale = T::Real::from_dimension(rows.max(columns));
    let rank_tolerance = scale.multiply(T::Real::epsilon()).multiply(dimension_scale);

    for pivot_index in 0..rows {
        check_cancellation(cancellation)?;
        let mut pivot_column = pivot_index;
        let mut pivot_norm = column_norm(&residuals, rows, pivot_index, cancellation)?;
        for candidate in (pivot_index + 1)..columns {
            check_cancellation(cancellation)?;
            let candidate_norm = column_norm(&residuals, rows, candidate, cancellation)?;
            let ordering = candidate_norm.total_compare(pivot_norm);
            if ordering.is_gt()
                || (ordering.is_eq() && permutation[candidate] < permutation[pivot_column])
            {
                pivot_column = candidate;
                pivot_norm = candidate_norm;
            }
        }

        if pivot_norm.less_than_or_equal(rank_tolerance) {
            return Err(LinalgError::RankDeficient {
                provider: RANK_PROVIDER,
                operation: OPERATION,
                deficient_diagonal: one_based_u64(pivot_index),
                required_rank: u64::try_from(rows).unwrap_or(u64::MAX),
            });
        }

        if pivot_column != pivot_index {
            permutation.swap(pivot_index, pivot_column);
            for row in 0..rows {
                check_cancellation(cancellation)?;
                residuals.swap(pivot_index * rows + row, pivot_column * rows + row);
            }
        }

        for row in 0..rows {
            check_cancellation(cancellation)?;
            let offset = pivot_index * rows + row;
            residuals[offset] = residuals[offset].scale(pivot_norm.reciprocal());
        }

        for trailing_column in (pivot_index + 1)..columns {
            check_cancellation(cancellation)?;
            // A second pass suppresses the loss of orthogonality characteristic
            // of a single modified Gram-Schmidt update.
            for _ in 0..2 {
                let mut projection = T::zero();
                for row in 0..rows {
                    check_cancellation(cancellation)?;
                    projection = projection.add(
                        residuals[pivot_index * rows + row]
                            .conjugate()
                            .multiply(residuals[trailing_column * rows + row]),
                    );
                }
                for row in 0..rows {
                    check_cancellation(cancellation)?;
                    let trailing_offset = trailing_column * rows + row;
                    residuals[trailing_offset] = residuals[trailing_offset]
                        .subtract(residuals[pivot_index * rows + row].multiply(projection));
                }
            }
        }
    }

    permutation.truncate(rows);
    Ok(permutation)
}

fn column_norm<T: BasicSolveScalar>(
    matrix: &[T],
    rows: usize,
    column: usize,
    cancellation: Option<&AtomicBool>,
) -> Result<T::Real, LinalgError> {
    let mut norm = T::Real::zero();
    for row in 0..rows {
        check_cancellation(cancellation)?;
        norm = norm.hypotenuse(matrix[column * rows + row].magnitude());
    }
    Ok(norm)
}

fn check_cancellation(cancellation: Option<&AtomicBool>) -> Result<(), LinalgError> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(LinalgError::Cancelled {
            operation: OPERATION,
        })
    } else {
        Ok(())
    }
}

fn checked_host_dimension(operation: &'static str, value: u64) -> Result<usize, LinalgError> {
    usize::try_from(value).map_err(|_| LinalgError::AllocationFailure {
        operation,
        elements: value,
    })
}

fn checked_buffer_length(
    operation: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, LinalgError> {
    left.checked_mul(right)
        .ok_or(LinalgError::AllocationFailure {
            operation,
            elements: u64::MAX,
        })
}

fn try_filled_buffer<T: Copy>(
    operation: &'static str,
    length: usize,
    value: T,
) -> Result<Vec<T>, LinalgError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(length)
        .map_err(|_| LinalgError::AllocationFailure {
            operation,
            elements: u64::try_from(length).unwrap_or(u64::MAX),
        })?;
    buffer.resize(length, value);
    Ok(buffer)
}

fn try_index_buffer(length: usize) -> Result<Vec<usize>, LinalgError> {
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(length)
        .map_err(|_| LinalgError::AllocationFailure {
            operation: "underdetermined basic solve pivot permutation",
            elements: u64::try_from(length).unwrap_or(u64::MAX),
        })?;
    indices.extend(0..length);
    Ok(indices)
}

fn one_based_u64(index: usize) -> u64 {
    u64::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .unwrap_or(u64::MAX)
}

trait BasicReal: Copy {
    fn zero() -> Self;
    fn epsilon() -> Self;
    fn from_dimension(value: usize) -> Self;
    fn maximum(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn reciprocal(self) -> Self;
    fn hypotenuse(self, right: Self) -> Self;
    fn total_compare(self, right: Self) -> std::cmp::Ordering;
    fn less_than_or_equal(self, right: Self) -> bool;
}

impl BasicReal for f64 {
    fn zero() -> Self {
        0.0
    }

    fn epsilon() -> Self {
        Self::EPSILON
    }

    fn from_dimension(value: usize) -> Self {
        u32::try_from(value).map_or(f64::from(u32::MAX), f64::from)
    }

    fn maximum(self, right: Self) -> Self {
        self.max(right)
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn reciprocal(self) -> Self {
        self.recip()
    }

    fn hypotenuse(self, right: Self) -> Self {
        self.hypot(right)
    }

    fn total_compare(self, right: Self) -> std::cmp::Ordering {
        self.total_cmp(&right)
    }

    fn less_than_or_equal(self, right: Self) -> bool {
        self <= right
    }
}

impl BasicReal for f32 {
    fn zero() -> Self {
        0.0
    }

    fn epsilon() -> Self {
        Self::EPSILON
    }

    #[allow(clippy::cast_precision_loss)]
    fn from_dimension(value: usize) -> Self {
        value as Self
    }

    fn maximum(self, right: Self) -> Self {
        self.max(right)
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn reciprocal(self) -> Self {
        self.recip()
    }

    fn hypotenuse(self, right: Self) -> Self {
        self.hypot(right)
    }

    fn total_compare(self, right: Self) -> std::cmp::Ordering {
        self.total_cmp(&right)
    }

    fn less_than_or_equal(self, right: Self) -> bool {
        self <= right
    }
}

trait BasicSolveScalar: Copy {
    type Real: BasicReal;

    fn zero() -> Self;
    fn magnitude(self) -> Self::Real;
    fn conjugate(self) -> Self;
    fn add(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn scale(self, factor: Self::Real) -> Self;
    fn solve_selected(
        provider: &dyn LinalgProvider,
        request: SolveRequest<'_, Self>,
    ) -> Result<DenseArray<Self>, LinalgError>;
}

impl BasicSolveScalar for f64 {
    type Real = f64;

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

    fn solve_selected(
        provider: &dyn LinalgProvider,
        request: SolveRequest<'_, Self>,
    ) -> Result<DenseArray<Self>, LinalgError> {
        provider.solve_f64(request)
    }
}

impl BasicSolveScalar for f32 {
    type Real = f32;

    fn zero() -> Self {
        0.0
    }

    fn magnitude(self) -> Self::Real {
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

    fn scale(self, factor: Self::Real) -> Self {
        self * factor
    }

    fn solve_selected(
        provider: &dyn LinalgProvider,
        request: SolveRequest<'_, Self>,
    ) -> Result<DenseArray<Self>, LinalgError> {
        provider.solve_f32(request)
    }
}

impl BasicSolveScalar for Complex64 {
    type Real = f64;

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
        self + right
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

    fn solve_selected(
        provider: &dyn LinalgProvider,
        request: SolveRequest<'_, Self>,
    ) -> Result<DenseArray<Self>, LinalgError> {
        provider.solve_complex64(request)
    }
}

impl BasicSolveScalar for Complex32 {
    type Real = f32;

    fn zero() -> Self {
        Self::ZERO
    }

    fn magnitude(self) -> Self::Real {
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
        Self::new(
            self.re * right.re - self.im * right.im,
            self.re * right.im + self.im * right.re,
        )
    }

    fn scale(self, factor: Self::Real) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }

    fn solve_selected(
        provider: &dyn LinalgProvider,
        request: SolveRequest<'_, Self>,
    ) -> Result<DenseArray<Self>, LinalgError> {
        provider.solve_complex32(request)
    }
}

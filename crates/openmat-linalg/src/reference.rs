use openmat_array::{Complex32, Complex64, DenseArray, Shape};

use crate::{
    CholeskyRequest, CholeskyResult, CholeskyTriangle, EigRequest, EigResult, FactorRequest,
    GemmRequest, LinalgError, LinalgProvider, LuResult, MatrixTranspose, ProviderIntegerAbi,
    QrRequest, QrResult, QrVectors, RectangularSolveKind, RectangularSolveRequest, SchurRequest,
    SchurResult, SolveRequest, SvdRequest, SvdResult, SwapParity, validate_cholesky,
    validate_factor, validate_gemm, validate_qr, validate_rectangular_solve, validate_solve,
};

/// Deterministic, dependency-free numerical provider used for tests and as a
/// correctness fallback.
///
/// The implementation is intentionally simple and does not claim optimized
/// BLAS performance.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReferenceProvider;

impl LinalgProvider for ReferenceProvider {
    fn name(&self) -> &'static str {
        "reference"
    }

    fn integer_abi(&self) -> ProviderIntegerAbi {
        ProviderIntegerAbi::RustU64
    }

    fn gemm_f64(
        &self,
        request: GemmRequest<'_, f64>,
        output: &mut DenseArray<f64>,
    ) -> Result<(), LinalgError> {
        let dimensions = validate_gemm(&request, output)?;
        for column in 0..dimensions.n() {
            for row in 0..dimensions.m() {
                let mut sum = 0.0;
                for inner in 0..dimensions.k() {
                    let left =
                        real_value(request.left, request.options.left_transpose, row, inner)?;
                    let right = real_value(
                        request.right,
                        request.options.right_transpose,
                        inner,
                        column,
                    )?;
                    sum = left.mul_add(right, sum);
                }
                let subscripts = [row + 1, column + 1];
                let existing = *output.get_subscripts(&subscripts)?;
                *output.get_mut_subscripts(&subscripts)? = request
                    .options
                    .alpha
                    .mul_add(sum, request.options.beta * existing);
            }
        }
        Ok(())
    }

    fn gemm_f32(
        &self,
        request: GemmRequest<'_, f32>,
        output: &mut DenseArray<f32>,
    ) -> Result<(), LinalgError> {
        let dimensions = validate_gemm(&request, output)?;
        for column in 0..dimensions.n() {
            for row in 0..dimensions.m() {
                let mut sum = 0.0_f32;
                for inner in 0..dimensions.k() {
                    let left =
                        real32_value(request.left, request.options.left_transpose, row, inner)?;
                    let right = real32_value(
                        request.right,
                        request.options.right_transpose,
                        inner,
                        column,
                    )?;
                    sum = left.mul_add(right, sum);
                }
                let subscripts = [row + 1, column + 1];
                let existing = *output.get_subscripts(&subscripts)?;
                *output.get_mut_subscripts(&subscripts)? = request
                    .options
                    .alpha
                    .mul_add(sum, request.options.beta * existing);
            }
        }
        Ok(())
    }

    fn gemm_complex64(
        &self,
        request: GemmRequest<'_, Complex64>,
        output: &mut DenseArray<Complex64>,
    ) -> Result<(), LinalgError> {
        let dimensions = validate_gemm(&request, output)?;
        for column in 0..dimensions.n() {
            for row in 0..dimensions.m() {
                let mut sum = Complex64::ZERO;
                for inner in 0..dimensions.k() {
                    let left =
                        complex_value(request.left, request.options.left_transpose, row, inner)?;
                    let right = complex_value(
                        request.right,
                        request.options.right_transpose,
                        inner,
                        column,
                    )?;
                    sum += left * right;
                }
                let subscripts = [row + 1, column + 1];
                let existing = *output.get_subscripts(&subscripts)?;
                *output.get_mut_subscripts(&subscripts)? =
                    request.options.alpha * sum + request.options.beta * existing;
            }
        }
        Ok(())
    }

    fn gemm_complex32(
        &self,
        request: GemmRequest<'_, Complex32>,
        output: &mut DenseArray<Complex32>,
    ) -> Result<(), LinalgError> {
        let dimensions = validate_gemm(&request, output)?;
        for column in 0..dimensions.n() {
            for row in 0..dimensions.m() {
                let mut sum = Complex32::ZERO;
                for inner in 0..dimensions.k() {
                    let left =
                        complex32_value(request.left, request.options.left_transpose, row, inner)?;
                    let right = complex32_value(
                        request.right,
                        request.options.right_transpose,
                        inner,
                        column,
                    )?;
                    sum = complex32_add(sum, complex32_multiply(left, right));
                }
                let subscripts = [row + 1, column + 1];
                let existing = *output.get_subscripts(&subscripts)?;
                *output.get_mut_subscripts(&subscripts)? = complex32_add(
                    complex32_multiply(request.options.alpha, sum),
                    complex32_multiply(request.options.beta, existing),
                );
            }
        }
        Ok(())
    }

    fn dot_f64(&self, left: &DenseArray<f64>, right: &DenseArray<f64>) -> Result<f64, LinalgError> {
        if left.numel() != right.numel() {
            return Err(LinalgError::DimensionMismatch {
                operation: "real dot product element counts",
                left: left.numel(),
                right: right.numel(),
            });
        }
        Ok(left
            .as_slice()
            .iter()
            .zip(right.as_slice())
            .fold(0.0, |sum, (&left, &right)| left.mul_add(right, sum)))
    }

    fn norm2_f64(&self, input: &DenseArray<f64>) -> Result<f64, LinalgError> {
        let mut scale = 0.0;
        let mut sum_of_squares = 1.0;
        let mut has_infinite = false;
        for &element in input.as_slice() {
            let absolute = element.abs();
            if absolute.is_nan() {
                // NaN has precedence even if an infinity occurred earlier.
                return Ok(f64::NAN);
            }
            if absolute.is_infinite() {
                // Keep scanning so a later NaN still propagates. Infinite
                // values must not enter the scaled sum as `inf / inf`.
                has_infinite = true;
                continue;
            }
            if absolute <= 0.0 {
                continue;
            }
            if scale < absolute {
                let ratio = scale / absolute;
                sum_of_squares = 1.0 + sum_of_squares * ratio * ratio;
                scale = absolute;
            } else {
                let ratio = absolute / scale;
                sum_of_squares += ratio * ratio;
            }
        }
        if has_infinite {
            Ok(f64::INFINITY)
        } else if scale > 0.0 {
            Ok(scale * sum_of_squares.sqrt())
        } else {
            Ok(0.0)
        }
    }

    fn solve_f64(&self, request: SolveRequest<'_, f64>) -> Result<DenseArray<f64>, LinalgError> {
        reference_solve(request, self.name())
    }

    fn solve_f32(&self, request: SolveRequest<'_, f32>) -> Result<DenseArray<f32>, LinalgError> {
        reference_solve(request, self.name())
    }

    fn solve_complex64(
        &self,
        request: SolveRequest<'_, Complex64>,
    ) -> Result<DenseArray<Complex64>, LinalgError> {
        reference_solve(request, self.name())
    }

    fn solve_complex32(
        &self,
        request: SolveRequest<'_, Complex32>,
    ) -> Result<DenseArray<Complex32>, LinalgError> {
        reference_solve(request, self.name())
    }

    fn solve_rectangular_f64(
        &self,
        request: RectangularSolveRequest<'_, f64>,
    ) -> Result<DenseArray<f64>, LinalgError> {
        reference_rectangular_solve(request, self.name())
    }

    fn solve_rectangular_f32(
        &self,
        request: RectangularSolveRequest<'_, f32>,
    ) -> Result<DenseArray<f32>, LinalgError> {
        reference_rectangular_solve(request, self.name())
    }

    fn solve_rectangular_complex64(
        &self,
        request: RectangularSolveRequest<'_, Complex64>,
    ) -> Result<DenseArray<Complex64>, LinalgError> {
        reference_rectangular_solve(request, self.name())
    }

    fn solve_rectangular_complex32(
        &self,
        request: RectangularSolveRequest<'_, Complex32>,
    ) -> Result<DenseArray<Complex32>, LinalgError> {
        reference_rectangular_solve(request, self.name())
    }

    fn factor_lu_f32(&self, request: FactorRequest<'_, f32>) -> Result<LuResult<f32>, LinalgError> {
        reference_lu(request)
    }

    fn factor_lu_f64(&self, request: FactorRequest<'_, f64>) -> Result<LuResult<f64>, LinalgError> {
        reference_lu(request)
    }

    fn factor_lu_complex32(
        &self,
        request: FactorRequest<'_, Complex32>,
    ) -> Result<LuResult<Complex32>, LinalgError> {
        reference_lu(request)
    }

    fn factor_lu_complex64(
        &self,
        request: FactorRequest<'_, Complex64>,
    ) -> Result<LuResult<Complex64>, LinalgError> {
        reference_lu(request)
    }

    fn qr_f32(&self, request: QrRequest<'_, f32>) -> Result<QrResult<f32>, LinalgError> {
        reference_qr(request)
    }

    fn qr_f64(&self, request: QrRequest<'_, f64>) -> Result<QrResult<f64>, LinalgError> {
        reference_qr(request)
    }

    fn qr_complex32(
        &self,
        request: QrRequest<'_, Complex32>,
    ) -> Result<QrResult<Complex32>, LinalgError> {
        reference_qr(request)
    }

    fn qr_complex64(
        &self,
        request: QrRequest<'_, Complex64>,
    ) -> Result<QrResult<Complex64>, LinalgError> {
        reference_qr(request)
    }

    fn cholesky_f32(
        &self,
        request: CholeskyRequest<'_, f32>,
    ) -> Result<CholeskyResult<f32>, LinalgError> {
        reference_cholesky(request)
    }

    fn cholesky_f64(
        &self,
        request: CholeskyRequest<'_, f64>,
    ) -> Result<CholeskyResult<f64>, LinalgError> {
        reference_cholesky(request)
    }

    fn cholesky_complex32(
        &self,
        request: CholeskyRequest<'_, Complex32>,
    ) -> Result<CholeskyResult<Complex32>, LinalgError> {
        reference_cholesky(request)
    }

    fn cholesky_complex64(
        &self,
        request: CholeskyRequest<'_, Complex64>,
    ) -> Result<CholeskyResult<Complex64>, LinalgError> {
        reference_cholesky(request)
    }

    fn svd_f32(&self, request: SvdRequest<'_, f32>) -> Result<SvdResult<f32, f32>, LinalgError> {
        crate::reference_spectral::svd(request)
    }

    fn svd_f64(&self, request: SvdRequest<'_, f64>) -> Result<SvdResult<f64, f64>, LinalgError> {
        crate::reference_spectral::svd(request)
    }

    fn svd_complex32(
        &self,
        request: SvdRequest<'_, Complex32>,
    ) -> Result<SvdResult<Complex32, f32>, LinalgError> {
        crate::reference_spectral::svd(request)
    }

    fn svd_complex64(
        &self,
        request: SvdRequest<'_, Complex64>,
    ) -> Result<SvdResult<Complex64, f64>, LinalgError> {
        crate::reference_spectral::svd(request)
    }

    fn eig_f32(&self, request: EigRequest<'_, f32>) -> Result<EigResult<Complex32>, LinalgError> {
        crate::reference_spectral::eig(request)
    }

    fn eig_f64(&self, request: EigRequest<'_, f64>) -> Result<EigResult<Complex64>, LinalgError> {
        crate::reference_spectral::eig(request)
    }

    fn eig_complex32(
        &self,
        request: EigRequest<'_, Complex32>,
    ) -> Result<EigResult<Complex32>, LinalgError> {
        crate::reference_spectral::eig(request)
    }

    fn eig_complex64(
        &self,
        request: EigRequest<'_, Complex64>,
    ) -> Result<EigResult<Complex64>, LinalgError> {
        crate::reference_spectral::eig(request)
    }

    fn schur_complex32(
        &self,
        request: SchurRequest<'_, Complex32>,
    ) -> Result<SchurResult<Complex32>, LinalgError> {
        crate::reference_spectral::schur(request)
    }

    fn schur_complex64(
        &self,
        request: SchurRequest<'_, Complex64>,
    ) -> Result<SchurResult<Complex64>, LinalgError> {
        crate::reference_spectral::schur(request)
    }
}

fn reference_lu<T: LuScalar>(request: FactorRequest<'_, T>) -> Result<LuResult<T>, LinalgError> {
    let dimensions = validate_factor(&request)?;
    request.check_cancellation()?;
    let rows = checked_host_dimension("reference LU row dimension", dimensions.rows())?;
    let columns = checked_host_dimension("reference LU column dimension", dimensions.columns())?;
    let mut packed_lu = try_copy_buffer("reference LU matrix copy", request.matrix.as_slice())?;
    let status = lu_factor_in_place(&mut packed_lu, rows, columns, None, || {
        request.check_cancellation()
    })?;
    request.check_cancellation()?;
    Ok(LuResult {
        packed_lu: DenseArray::from_vec(request.matrix.shape().clone(), packed_lu)?,
        row_permutation_zero_based: status.row_permutation_zero_based,
        swap_parity: status.swap_parity,
        first_zero_pivot: status.first_zero_pivot.map(saturating_u64),
    })
}

fn reference_qr<T: QrScalar>(request: QrRequest<'_, T>) -> Result<QrResult<T>, LinalgError> {
    let dimensions = validate_qr(&request)?;
    request.check_cancellation()?;
    let rows = checked_host_dimension("reference QR row dimension", dimensions.rows())?;
    let columns = checked_host_dimension("reference QR column dimension", dimensions.columns())?;
    let order = rows.min(columns);
    let q_columns = match request.vectors {
        QrVectors::Full => rows,
        QrVectors::Thin => order,
    };
    let mut factor = try_copy_buffer("reference QR matrix copy", request.matrix.as_slice())?;
    let householder =
        householder_factor_in_place(&mut factor, rows, columns, request.pivot_columns, || {
            request.check_cancellation()
        })?;

    let q_length = rows
        .checked_mul(q_columns)
        .ok_or(LinalgError::AllocationFailure {
            operation: "reference QR Q dimensions",
            elements: u64::MAX,
        })?;
    let mut q = try_filled_buffer("reference QR Q", q_length, T::zero())?;
    for diagonal in 0..q_columns.min(rows) {
        q[diagonal * rows + diagonal] = T::from_real(T::Real::one());
    }
    apply_reflectors_core(
        &factor,
        rows,
        &householder.reflectors,
        &mut q,
        q_columns,
        ReflectorOrder::Reverse,
        || request.check_cancellation(),
    )?;

    let r_rows = q_columns;
    let r_length = r_rows
        .checked_mul(columns)
        .ok_or(LinalgError::AllocationFailure {
            operation: "reference QR R dimensions",
            elements: u64::MAX,
        })?;
    let mut r = try_filled_buffer("reference QR R", r_length, T::zero())?;
    for column in 0..columns {
        for row in 0..r_rows.min(column.saturating_add(1)) {
            r[column * r_rows + row] = factor[column * rows + row];
        }
    }
    request.check_cancellation()?;
    Ok(QrResult {
        q: DenseArray::from_vec(
            Shape::new([dimensions.rows(), saturating_u64(q_columns)])?,
            q,
        )?,
        r: DenseArray::from_vec(
            Shape::new([saturating_u64(r_rows), dimensions.columns()])?,
            r,
        )?,
        column_permutation_zero_based: householder.column_permutation_zero_based,
        numerical_rank: saturating_u64(householder.numerical_rank),
        first_rank_deficient_diagonal: householder
            .first_rank_deficient_diagonal
            .map(saturating_u64),
    })
}

fn reference_cholesky<T: QrScalar>(
    request: CholeskyRequest<'_, T>,
) -> Result<CholeskyResult<T>, LinalgError> {
    let dimensions = validate_cholesky(&request)?;
    request.check_cancellation()?;
    let order = checked_host_dimension("reference Cholesky order dimension", dimensions.order())?;
    let mut factor = try_filled_buffer(
        "reference Cholesky factor",
        request.matrix.as_slice().len(),
        T::zero(),
    )?;
    let mut first_non_positive_minor = None;

    for column in 0..order {
        request.check_cancellation()?;
        let mut diagonal = request.matrix.as_slice()[column * order + column].real_part();
        for inner in 0..column {
            let value = match request.triangle {
                CholeskyTriangle::Upper => factor[column * order + inner],
                CholeskyTriangle::Lower => factor[inner * order + column],
            };
            let magnitude = value.magnitude();
            diagonal = diagonal.subtract(magnitude.multiply(magnitude));
        }
        if !diagonal.is_strictly_positive() {
            first_non_positive_minor = Some(saturating_u64(column));
            break;
        }
        let diagonal_scalar = T::from_real(diagonal.square_root());
        factor[column * order + column] = diagonal_scalar;

        for trailing in (column + 1)..order {
            request.check_cancellation()?;
            let mut value = match request.triangle {
                CholeskyTriangle::Upper => request.matrix.as_slice()[trailing * order + column],
                CholeskyTriangle::Lower => request.matrix.as_slice()[column * order + trailing],
            };
            for inner in 0..column {
                let product = match request.triangle {
                    CholeskyTriangle::Upper => factor[column * order + inner]
                        .conjugate()
                        .multiply(factor[trailing * order + inner]),
                    CholeskyTriangle::Lower => factor[inner * order + trailing]
                        .multiply(factor[inner * order + column].conjugate()),
                };
                value = value.subtract(product);
            }
            let value = value.divide(diagonal_scalar);
            match request.triangle {
                CholeskyTriangle::Upper => factor[trailing * order + column] = value,
                CholeskyTriangle::Lower => factor[column * order + trailing] = value,
            }
        }
    }
    if let Some(minor) = first_non_positive_minor {
        let successful_order = usize::try_from(minor).unwrap_or(0).min(order);
        for column in 0..order {
            for row in 0..order {
                if row >= successful_order || column >= successful_order {
                    factor[column * order + row] = T::zero();
                }
            }
        }
    }
    request.check_cancellation()?;
    Ok(CholeskyResult {
        factor: DenseArray::from_vec(request.matrix.shape().clone(), factor)?,
        first_non_positive_minor,
    })
}

fn reference_rectangular_solve<T: QrScalar>(
    request: RectangularSolveRequest<'_, T>,
    provider: &'static str,
) -> Result<DenseArray<T>, LinalgError> {
    let dimensions = validate_rectangular_solve(&request)?;
    request.check_cancellation()?;
    let rows = checked_host_dimension(
        "reference rectangular solve row dimension",
        dimensions.coefficient_rows(),
    )?;
    let columns = checked_host_dimension(
        "reference rectangular solve column dimension",
        dimensions.coefficient_columns(),
    )?;
    let right_hand_sides = checked_host_dimension(
        "reference rectangular solve right-hand-side dimension",
        dimensions.right_hand_sides(),
    )?;
    let output_len =
        columns
            .checked_mul(right_hand_sides)
            .ok_or(LinalgError::AllocationFailure {
                operation: "reference rectangular solve result dimensions",
                elements: u64::MAX,
            })?;

    let solution = if dimensions.required_rank() == 0 {
        try_filled_buffer(
            "reference rectangular solve empty-rank result",
            output_len,
            T::zero(),
        )?
    } else {
        match dimensions.kind() {
            RectangularSolveKind::Square | RectangularSolveKind::OverdeterminedLeastSquares => {
                reference_least_squares(&request, rows, columns, right_hand_sides, provider)?
            }
            RectangularSolveKind::UnderdeterminedMinimumNorm => {
                reference_minimum_norm(&request, rows, columns, right_hand_sides, provider)?
            }
        }
    };
    request.check_cancellation()?;
    DenseArray::from_vec(
        Shape::new([
            dimensions.coefficient_columns(),
            dimensions.right_hand_sides(),
        ])?,
        solution,
    )
    .map_err(Into::into)
}

fn reference_least_squares<T: QrScalar>(
    request: &RectangularSolveRequest<'_, T>,
    rows: usize,
    columns: usize,
    right_hand_sides: usize,
    provider: &'static str,
) -> Result<Vec<T>, LinalgError> {
    let mut factor = try_copy_buffer(
        "reference rectangular solve coefficient copy",
        request.coefficients.as_slice(),
    )?;
    let reflectors = factor_householder(&mut factor, rows, columns, request, provider)?;
    let mut transformed_rhs = try_copy_buffer(
        "reference rectangular solve right-hand-side copy",
        request.right_hand_side.as_slice(),
    )?;
    apply_reflectors(
        &factor,
        rows,
        &reflectors,
        &mut transformed_rhs,
        right_hand_sides,
        ReflectorOrder::Forward,
        request,
    )?;

    let output_len =
        columns
            .checked_mul(right_hand_sides)
            .ok_or(LinalgError::AllocationFailure {
                operation: "reference rectangular solve result dimensions",
                elements: u64::MAX,
            })?;
    let mut solution =
        try_filled_buffer("reference rectangular solve result", output_len, T::zero())?;
    for right_hand_side in 0..right_hand_sides {
        for row in (0..columns).rev() {
            request.check_cancellation()?;
            let mut value = transformed_rhs[right_hand_side * rows + row];
            for column in (row + 1)..columns {
                value = value.subtract(
                    factor[column * rows + row]
                        .multiply(solution[right_hand_side * columns + column]),
                );
            }
            solution[right_hand_side * columns + row] = value.divide(factor[row * rows + row]);
        }
    }
    Ok(solution)
}

fn reference_minimum_norm<T: QrScalar>(
    request: &RectangularSolveRequest<'_, T>,
    rows: usize,
    columns: usize,
    right_hand_sides: usize,
    provider: &'static str,
) -> Result<Vec<T>, LinalgError> {
    let factor_len = columns
        .checked_mul(rows)
        .ok_or(LinalgError::AllocationFailure {
            operation: "reference minimum-norm factor dimensions",
            elements: u64::MAX,
        })?;
    let mut factor = try_filled_buffer(
        "reference minimum-norm conjugate transpose",
        factor_len,
        T::zero(),
    )?;
    for factor_column in 0..rows {
        for factor_row in 0..columns {
            factor[factor_column * columns + factor_row] =
                request.coefficients.as_slice()[factor_row * rows + factor_column].conjugate();
        }
    }
    let reflectors = factor_householder(&mut factor, columns, rows, request, provider)?;

    let output_len =
        columns
            .checked_mul(right_hand_sides)
            .ok_or(LinalgError::AllocationFailure {
                operation: "reference minimum-norm result dimensions",
                elements: u64::MAX,
            })?;
    let mut solution = try_filled_buffer("reference minimum-norm result", output_len, T::zero())?;
    for right_hand_side in 0..right_hand_sides {
        for row in 0..rows {
            request.check_cancellation()?;
            let mut value = request.right_hand_side.as_slice()[right_hand_side * rows + row];
            for column in 0..row {
                value = value.subtract(
                    factor[row * columns + column]
                        .conjugate()
                        .multiply(solution[right_hand_side * columns + column]),
                );
            }
            solution[right_hand_side * columns + row] =
                value.divide(factor[row * columns + row].conjugate());
        }
    }
    apply_reflectors(
        &factor,
        columns,
        &reflectors,
        &mut solution,
        right_hand_sides,
        ReflectorOrder::Reverse,
        request,
    )?;
    Ok(solution)
}

fn factor_householder<T: QrScalar>(
    matrix: &mut [T],
    rows: usize,
    columns: usize,
    request: &RectangularSolveRequest<'_, T>,
    provider: &'static str,
) -> Result<Vec<T::Real>, LinalgError> {
    debug_assert!(rows >= columns);
    let factorization = householder_factor_in_place(matrix, rows, columns, false, || {
        request.check_cancellation()
    })?;
    if let Some(deficient_diagonal) = factorization.first_rank_deficient_diagonal {
        return Err(LinalgError::RankDeficient {
            provider,
            operation: "rectangular least-squares solve",
            deficient_diagonal: saturating_u64(deficient_diagonal).saturating_add(1),
            required_rank: saturating_u64(columns),
        });
    }
    Ok(factorization.reflectors)
}

struct HouseholderFactor<R> {
    reflectors: Vec<R>,
    column_permutation_zero_based: Vec<u64>,
    numerical_rank: usize,
    first_rank_deficient_diagonal: Option<usize>,
}

fn householder_factor_in_place<T: QrScalar>(
    matrix: &mut [T],
    rows: usize,
    columns: usize,
    pivot_columns: bool,
    mut check_cancellation: impl FnMut() -> Result<(), LinalgError>,
) -> Result<HouseholderFactor<T::Real>, LinalgError> {
    let order = rows.min(columns);
    let scale = matrix.iter().fold(T::Real::zero(), |scale, value| {
        scale.maximum(value.magnitude())
    });
    let dimension_scale = T::Real::from_dimension(rows.max(columns));
    let rank_tolerance = scale.multiply(T::Real::epsilon()).multiply(dimension_scale);
    let mut reflectors = try_filled_buffer(
        "reference QR reflector coefficients",
        order,
        T::Real::zero(),
    )?;
    let mut column_permutation_zero_based = Vec::new();
    column_permutation_zero_based
        .try_reserve_exact(columns)
        .map_err(|_| LinalgError::AllocationFailure {
            operation: "reference QR column permutation",
            elements: saturating_u64(columns),
        })?;
    for column in 0..columns {
        column_permutation_zero_based.push(saturating_u64(column));
    }
    let mut numerical_rank = 0;
    let mut first_rank_deficient_diagonal = None;

    for column in 0..order {
        check_cancellation()?;
        if pivot_columns {
            let mut pivot_column = column;
            let mut pivot_norm = T::Real::zero();
            for candidate in column..columns {
                let norm = (column..rows).fold(T::Real::zero(), |norm, row| {
                    norm.hypotenuse(matrix[candidate * rows + row].magnitude())
                });
                if candidate == column || norm.greater_than(pivot_norm) {
                    pivot_column = candidate;
                    pivot_norm = norm;
                }
            }
            if pivot_column != column {
                for row in 0..rows {
                    matrix.swap(column * rows + row, pivot_column * rows + row);
                }
                column_permutation_zero_based.swap(column, pivot_column);
            }
        }
        let norm = (column..rows).fold(T::Real::zero(), |norm, row| {
            norm.hypotenuse(matrix[column * rows + row].magnitude())
        });
        if !norm.less_than_or_equal(rank_tolerance) {
            numerical_rank += 1;
        } else if first_rank_deficient_diagonal.is_none() {
            first_rank_deficient_diagonal = Some(column);
        }
        if norm.less_than_or_equal(T::Real::zero()) {
            continue;
        }

        let diagonal_offset = column * rows + column;
        let diagonal = matrix[diagonal_offset].householder_diagonal(norm);
        let divisor = matrix[diagonal_offset].subtract(diagonal);
        let mut squared_norm = T::Real::one();
        for row in (column + 1)..rows {
            let offset = column * rows + row;
            matrix[offset] = matrix[offset].divide(divisor);
            let magnitude = matrix[offset].magnitude();
            squared_norm = magnitude.mul_add(magnitude, squared_norm);
        }
        let tau = T::Real::two().divide(squared_norm);
        reflectors[column] = tau;

        for trailing_column in (column + 1)..columns {
            let mut product = matrix[trailing_column * rows + column];
            for row in (column + 1)..rows {
                product = product.add(
                    matrix[column * rows + row]
                        .conjugate()
                        .multiply(matrix[trailing_column * rows + row]),
                );
            }
            let scaled_product = product.scale(tau);
            let leading_offset = trailing_column * rows + column;
            matrix[leading_offset] = matrix[leading_offset].subtract(scaled_product);
            for row in (column + 1)..rows {
                let offset = trailing_column * rows + row;
                matrix[offset] =
                    matrix[offset].subtract(matrix[column * rows + row].multiply(scaled_product));
            }
        }
        matrix[diagonal_offset] = diagonal;
    }
    Ok(HouseholderFactor {
        reflectors,
        column_permutation_zero_based,
        numerical_rank,
        first_rank_deficient_diagonal,
    })
}

#[derive(Clone, Copy)]
enum ReflectorOrder {
    Forward,
    Reverse,
}

fn apply_reflectors<T: QrScalar>(
    factor: &[T],
    rows: usize,
    reflectors: &[T::Real],
    matrix: &mut [T],
    matrix_columns: usize,
    order: ReflectorOrder,
    request: &RectangularSolveRequest<'_, T>,
) -> Result<(), LinalgError> {
    apply_reflectors_core(
        factor,
        rows,
        reflectors,
        matrix,
        matrix_columns,
        order,
        || request.check_cancellation(),
    )
}

fn apply_reflectors_core<T: QrScalar>(
    factor: &[T],
    rows: usize,
    reflectors: &[T::Real],
    matrix: &mut [T],
    matrix_columns: usize,
    order: ReflectorOrder,
    mut check_cancellation: impl FnMut() -> Result<(), LinalgError>,
) -> Result<(), LinalgError> {
    let apply = |column: usize, matrix: &mut [T]| {
        for matrix_column in 0..matrix_columns {
            let mut product = matrix[matrix_column * rows + column];
            for row in (column + 1)..rows {
                product = product.add(
                    factor[column * rows + row]
                        .conjugate()
                        .multiply(matrix[matrix_column * rows + row]),
                );
            }
            let scaled_product = product.scale(reflectors[column]);
            let leading_offset = matrix_column * rows + column;
            matrix[leading_offset] = matrix[leading_offset].subtract(scaled_product);
            for row in (column + 1)..rows {
                let offset = matrix_column * rows + row;
                matrix[offset] =
                    matrix[offset].subtract(factor[column * rows + row].multiply(scaled_product));
            }
        }
    };

    match order {
        ReflectorOrder::Forward => {
            for column in 0..reflectors.len() {
                check_cancellation()?;
                apply(column, matrix);
            }
        }
        ReflectorOrder::Reverse => {
            for column in (0..reflectors.len()).rev() {
                check_cancellation()?;
                apply(column, matrix);
            }
        }
    }
    Ok(())
}

fn checked_host_dimension(operation: &'static str, value: u64) -> Result<usize, LinalgError> {
    usize::try_from(value).map_err(|_| LinalgError::AllocationFailure {
        operation,
        elements: value,
    })
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
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

trait QrReal: Copy {
    fn zero() -> Self;
    fn one() -> Self;
    fn two() -> Self;
    fn epsilon() -> Self;
    fn from_dimension(value: usize) -> Self;
    fn maximum(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
    fn mul_add(self, multiplier: Self, addend: Self) -> Self;
    fn hypotenuse(self, right: Self) -> Self;
    fn less_than_or_equal(self, right: Self) -> bool;
    fn greater_than(self, right: Self) -> bool;
    fn square_root(self) -> Self;
    fn is_strictly_positive(self) -> bool;
}

impl QrReal for f64 {
    fn zero() -> Self {
        0.0
    }

    fn one() -> Self {
        1.0
    }

    fn two() -> Self {
        2.0
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

    fn subtract(self, right: Self) -> Self {
        self - right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn mul_add(self, multiplier: Self, addend: Self) -> Self {
        self.mul_add(multiplier, addend)
    }

    fn hypotenuse(self, right: Self) -> Self {
        self.hypot(right)
    }

    fn less_than_or_equal(self, right: Self) -> bool {
        self <= right
    }

    fn greater_than(self, right: Self) -> bool {
        self > right
    }

    fn square_root(self) -> Self {
        self.sqrt()
    }

    fn is_strictly_positive(self) -> bool {
        self > 0.0
    }
}

impl QrReal for f32 {
    fn zero() -> Self {
        0.0
    }

    fn one() -> Self {
        1.0
    }

    fn two() -> Self {
        2.0
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

    fn subtract(self, right: Self) -> Self {
        self - right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn mul_add(self, multiplier: Self, addend: Self) -> Self {
        self.mul_add(multiplier, addend)
    }

    fn hypotenuse(self, right: Self) -> Self {
        self.hypot(right)
    }

    fn less_than_or_equal(self, right: Self) -> bool {
        self <= right
    }

    fn greater_than(self, right: Self) -> bool {
        self > right
    }

    fn square_root(self) -> Self {
        self.sqrt()
    }

    fn is_strictly_positive(self) -> bool {
        self > 0.0
    }
}

trait QrScalar: Copy {
    type Real: QrReal;

    fn zero() -> Self;
    fn magnitude(self) -> Self::Real;
    fn conjugate(self) -> Self;
    fn add(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
    fn scale(self, factor: Self::Real) -> Self;
    fn householder_diagonal(self, norm: Self::Real) -> Self;
    fn from_real(value: Self::Real) -> Self;
    fn real_part(self) -> Self::Real;
}

impl QrScalar for f64 {
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

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn scale(self, factor: f64) -> Self {
        self * factor
    }

    fn householder_diagonal(self, norm: f64) -> Self {
        if self >= 0.0 { -norm } else { norm }
    }

    fn from_real(value: Self::Real) -> Self {
        value
    }

    fn real_part(self) -> Self::Real {
        self
    }
}

impl QrScalar for f32 {
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

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn scale(self, factor: Self::Real) -> Self {
        self * factor
    }

    fn householder_diagonal(self, norm: Self::Real) -> Self {
        if self >= 0.0 { -norm } else { norm }
    }

    fn from_real(value: Self::Real) -> Self {
        value
    }

    fn real_part(self) -> Self::Real {
        self
    }
}

impl QrScalar for Complex64 {
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

    fn divide(self, right: Self) -> Self {
        if right.re.abs() >= right.im.abs() {
            let ratio = right.im / right.re;
            let denominator = right.re + right.im * ratio;
            Self::new(
                (self.re + self.im * ratio) / denominator,
                (self.im - self.re * ratio) / denominator,
            )
        } else {
            let ratio = right.re / right.im;
            let denominator = right.im + right.re * ratio;
            Self::new(
                (self.re * ratio + self.im) / denominator,
                (self.im * ratio - self.re) / denominator,
            )
        }
    }

    fn scale(self, factor: f64) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }

    fn householder_diagonal(self, norm: f64) -> Self {
        let magnitude = QrScalar::magnitude(self);
        if magnitude == 0.0 {
            Self::new(-norm, 0.0)
        } else {
            self.scale(-norm / magnitude)
        }
    }

    fn from_real(value: Self::Real) -> Self {
        Self::new(value, 0.0)
    }

    fn real_part(self) -> Self::Real {
        self.re
    }
}

impl QrScalar for Complex32 {
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
        complex32_multiply(self, right)
    }

    fn divide(self, right: Self) -> Self {
        complex32_divide(self, right)
    }

    fn scale(self, factor: Self::Real) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }

    fn householder_diagonal(self, norm: Self::Real) -> Self {
        let magnitude = QrScalar::magnitude(self);
        if magnitude == 0.0 {
            Self::new(-norm, 0.0)
        } else {
            self.scale(-norm / magnitude)
        }
    }

    fn from_real(value: Self::Real) -> Self {
        Self::new(value, 0.0)
    }

    fn real_part(self) -> Self::Real {
        self.re
    }
}

fn reference_solve<T: LuScalar>(
    request: SolveRequest<'_, T>,
    provider: &'static str,
) -> Result<DenseArray<T>, LinalgError> {
    let dimensions = validate_solve(&request)?;
    request.check_cancellation()?;

    let mut coefficients = try_copy_buffer(
        "reference solve coefficient copy",
        request.coefficients.as_slice(),
    )?;
    let mut solution = try_copy_buffer(
        "reference solve right-hand-side copy",
        request.right_hand_side.as_slice(),
    )?;
    request.check_cancellation()?;

    let order =
        usize::try_from(dimensions.order()).map_err(|_| LinalgError::AllocationFailure {
            operation: "reference solve dimensions",
            elements: dimensions.order(),
        })?;
    let right_hand_sides = usize::try_from(dimensions.right_hand_sides()).map_err(|_| {
        LinalgError::AllocationFailure {
            operation: "reference solve dimensions",
            elements: dimensions.right_hand_sides(),
        }
    })?;

    lu_solve_in_place(
        &mut coefficients,
        &mut solution,
        order,
        right_hand_sides,
        &request,
        provider,
    )?;
    DenseArray::from_vec(request.right_hand_side.shape().clone(), solution).map_err(Into::into)
}

fn try_copy_buffer<T: Copy>(operation: &'static str, source: &[T]) -> Result<Vec<T>, LinalgError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(source.len())
        .map_err(|_| LinalgError::AllocationFailure {
            operation,
            elements: u64::try_from(source.len()).unwrap_or(u64::MAX),
        })?;
    copy.extend_from_slice(source);
    Ok(copy)
}

fn lu_solve_in_place<T: LuScalar>(
    coefficients: &mut [T],
    right_hand_side: &mut [T],
    order: usize,
    right_hand_sides: usize,
    request: &SolveRequest<'_, T>,
    provider: &'static str,
) -> Result<(), LinalgError> {
    let factorization = lu_factor_in_place(
        coefficients,
        order,
        order,
        Some((right_hand_side, right_hand_sides)),
        || request.check_cancellation(),
    )?;
    if let Some(pivot) = factorization.first_zero_pivot {
        return Err(LinalgError::SingularMatrix {
            provider,
            operation: "general linear solve",
            pivot: saturating_u64(pivot).saturating_add(1),
        });
    }

    for row in (0..order).rev() {
        request.check_cancellation()?;
        let diagonal = coefficients[row * order + row];
        for right_hand_side_column in 0..right_hand_sides {
            let offset = right_hand_side_column * order + row;
            let mut value = right_hand_side[offset];
            for column in (row + 1)..order {
                value = value.subtract(
                    coefficients[column * order + row]
                        .multiply(right_hand_side[right_hand_side_column * order + column]),
                );
            }
            right_hand_side[offset] = value.divide(diagonal);
        }
    }
    Ok(())
}

struct LuFactorStatus {
    row_permutation_zero_based: Vec<u64>,
    swap_parity: SwapParity,
    first_zero_pivot: Option<usize>,
}

fn lu_factor_in_place<T: LuScalar>(
    coefficients: &mut [T],
    rows: usize,
    columns: usize,
    mut right_hand_side: Option<(&mut [T], usize)>,
    mut check_cancellation: impl FnMut() -> Result<(), LinalgError>,
) -> Result<LuFactorStatus, LinalgError> {
    let mut row_permutation_zero_based = Vec::new();
    row_permutation_zero_based
        .try_reserve_exact(rows)
        .map_err(|_| LinalgError::AllocationFailure {
            operation: "reference LU row permutation",
            elements: saturating_u64(rows),
        })?;
    for row in 0..rows {
        row_permutation_zero_based.push(saturating_u64(row));
    }
    let mut swap_parity = SwapParity::Even;
    let mut first_zero_pivot = None;

    for pivot_column in 0..rows.min(columns) {
        check_cancellation()?;
        let pivot_row = select_pivot(coefficients, rows, pivot_column);
        if coefficients[pivot_column * rows + pivot_row].is_zero() {
            if first_zero_pivot.is_none() {
                first_zero_pivot = Some(pivot_column);
            }
            continue;
        }

        if pivot_row != pivot_column {
            swap_rows(coefficients, rows, columns, pivot_column, pivot_row);
            row_permutation_zero_based.swap(pivot_column, pivot_row);
            swap_parity.toggle();
            if let Some((right_hand_side, right_hand_sides)) = right_hand_side.as_mut() {
                swap_rows(
                    right_hand_side,
                    rows,
                    *right_hand_sides,
                    pivot_column,
                    pivot_row,
                );
            }
        }

        let diagonal = coefficients[pivot_column * rows + pivot_column];
        for row in (pivot_column + 1)..rows {
            let pivot_offset = pivot_column * rows + row;
            let multiplier = coefficients[pivot_offset].divide(diagonal);
            coefficients[pivot_offset] = multiplier;
            for column in (pivot_column + 1)..columns {
                let offset = column * rows + row;
                let pivot_value = coefficients[column * rows + pivot_column];
                coefficients[offset] =
                    coefficients[offset].subtract(multiplier.multiply(pivot_value));
            }
            if let Some((right_hand_side, right_hand_sides)) = right_hand_side.as_mut() {
                for right_hand_side_column in 0..*right_hand_sides {
                    let offset = right_hand_side_column * rows + row;
                    let pivot_value = right_hand_side[right_hand_side_column * rows + pivot_column];
                    right_hand_side[offset] =
                        right_hand_side[offset].subtract(multiplier.multiply(pivot_value));
                }
            }
        }
    }
    Ok(LuFactorStatus {
        row_permutation_zero_based,
        swap_parity,
        first_zero_pivot,
    })
}

fn select_pivot<T: LuScalar>(coefficients: &[T], rows: usize, column: usize) -> usize {
    let mut pivot_row = column;
    let mut pivot = coefficients[column * rows + column];
    for row in (column + 1)..rows {
        let candidate = coefficients[column * rows + row];
        if candidate.has_greater_magnitude(pivot) {
            pivot_row = row;
            pivot = candidate;
        }
    }
    pivot_row
}

fn swap_rows<T>(matrix: &mut [T], rows: usize, columns: usize, left: usize, right: usize) {
    for column in 0..columns {
        matrix.swap(column * rows + left, column * rows + right);
    }
}

trait LuScalar: Copy {
    fn has_greater_magnitude(self, right: Self) -> bool;
    fn is_zero(self) -> bool;
    fn subtract(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
}

impl LuScalar for f64 {
    fn has_greater_magnitude(self, right: Self) -> bool {
        self.abs() > right.abs()
    }

    fn is_zero(self) -> bool {
        self == 0.0
    }

    fn subtract(self, right: Self) -> Self {
        self - right
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }
}

impl LuScalar for f32 {
    fn has_greater_magnitude(self, right: Self) -> bool {
        self.abs() > right.abs()
    }

    fn is_zero(self) -> bool {
        self == 0.0
    }

    fn subtract(self, right: Self) -> Self {
        self - right
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }
}

impl LuScalar for Complex64 {
    fn has_greater_magnitude(self, right: Self) -> bool {
        self.re.hypot(self.im) > right.re.hypot(right.im)
    }

    fn is_zero(self) -> bool {
        self.re == 0.0 && self.im == 0.0
    }

    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn divide(self, right: Self) -> Self {
        if right.re.abs() >= right.im.abs() {
            let ratio = right.im / right.re;
            let denominator = right.re + right.im * ratio;
            Self::new(
                (self.re + self.im * ratio) / denominator,
                (self.im - self.re * ratio) / denominator,
            )
        } else {
            let ratio = right.re / right.im;
            let denominator = right.im + right.re * ratio;
            Self::new(
                (self.re * ratio + self.im) / denominator,
                (self.im * ratio - self.re) / denominator,
            )
        }
    }
}

impl LuScalar for Complex32 {
    fn has_greater_magnitude(self, right: Self) -> bool {
        self.re.hypot(self.im) > right.re.hypot(right.im)
    }

    fn is_zero(self) -> bool {
        self.re == 0.0 && self.im == 0.0
    }

    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }

    fn multiply(self, right: Self) -> Self {
        complex32_multiply(self, right)
    }

    fn divide(self, right: Self) -> Self {
        complex32_divide(self, right)
    }
}

fn real_value(
    array: &DenseArray<f64>,
    transpose: MatrixTranspose,
    row: u64,
    column: u64,
) -> Result<f64, LinalgError> {
    let subscripts = match transpose {
        MatrixTranspose::None => [row + 1, column + 1],
        MatrixTranspose::Transpose | MatrixTranspose::ConjugateTranspose => [column + 1, row + 1],
    };
    Ok(*array.get_subscripts(&subscripts)?)
}

fn real32_value(
    array: &DenseArray<f32>,
    transpose: MatrixTranspose,
    row: u64,
    column: u64,
) -> Result<f32, LinalgError> {
    let subscripts = match transpose {
        MatrixTranspose::None => [row + 1, column + 1],
        MatrixTranspose::Transpose | MatrixTranspose::ConjugateTranspose => [column + 1, row + 1],
    };
    Ok(*array.get_subscripts(&subscripts)?)
}

fn complex_value(
    array: &DenseArray<Complex64>,
    transpose: MatrixTranspose,
    row: u64,
    column: u64,
) -> Result<Complex64, LinalgError> {
    let (subscripts, conjugate) = match transpose {
        MatrixTranspose::None => ([row + 1, column + 1], false),
        MatrixTranspose::Transpose => ([column + 1, row + 1], false),
        MatrixTranspose::ConjugateTranspose => ([column + 1, row + 1], true),
    };
    let value = *array.get_subscripts(&subscripts)?;
    Ok(if conjugate { value.conjugate() } else { value })
}

fn complex32_value(
    array: &DenseArray<Complex32>,
    transpose: MatrixTranspose,
    row: u64,
    column: u64,
) -> Result<Complex32, LinalgError> {
    let (subscripts, conjugate) = match transpose {
        MatrixTranspose::None => ([row + 1, column + 1], false),
        MatrixTranspose::Transpose => ([column + 1, row + 1], false),
        MatrixTranspose::ConjugateTranspose => ([column + 1, row + 1], true),
    };
    let value = *array.get_subscripts(&subscripts)?;
    Ok(if conjugate { value.conjugate() } else { value })
}

fn complex32_add(left: Complex32, right: Complex32) -> Complex32 {
    Complex32::new(left.re + right.re, left.im + right.im)
}

fn complex32_multiply(left: Complex32, right: Complex32) -> Complex32 {
    Complex32::new(
        left.re * right.re - left.im * right.im,
        left.re * right.im + left.im * right.re,
    )
}

fn complex32_divide(left: Complex32, right: Complex32) -> Complex32 {
    if right.re.abs() >= right.im.abs() {
        let ratio = right.im / right.re;
        let denominator = right.re + right.im * ratio;
        Complex32::new(
            (left.re + left.im * ratio) / denominator,
            (left.im - left.re * ratio) / denominator,
        )
    } else {
        let ratio = right.re / right.im;
        let denominator = right.im + right.re * ratio;
        Complex32::new(
            (left.re * ratio + left.im) / denominator,
            (left.im * ratio - left.re) / denominator,
        )
    }
}

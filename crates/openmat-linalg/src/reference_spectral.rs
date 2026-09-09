use openmat_array::{Complex32, Complex64, DenseArray, Shape};

use crate::{
    EigRequest, EigResult, LinalgError, SchurRequest, SchurResult, SvdRequest, SvdResult,
    SvdVectors, validate_eig, validate_schur, validate_svd,
};

const REFERENCE_PROVIDER: &str = "reference";

pub(crate) fn svd<T: SpectralScalar>(
    request: SvdRequest<'_, T>,
) -> Result<SvdResult<T, T::Real>, LinalgError> {
    let dimensions = validate_svd(&request)?;
    request.check_cancellation()?;
    let rows = host_dimension("reference SVD rows", dimensions.rows())?;
    let columns = host_dimension("reference SVD columns", dimensions.columns())?;
    let result = if rows >= columns {
        tall_svd(
            request.matrix.as_slice(),
            rows,
            columns,
            request.vectors,
            || request.check_cancellation(),
        )?
    } else {
        let transposed = conjugate_transpose(request.matrix.as_slice(), rows, columns)?;
        let transposed_result = tall_svd(&transposed, columns, rows, request.vectors, || {
            request.check_cancellation()
        })?;
        TallSvd {
            singular_values: transposed_result.singular_values,
            u: transposed_result
                .vh
                .as_deref()
                .map(|vh| conjugate_transpose(vh, rows, rows))
                .transpose()?,
            vh: transposed_result
                .u
                .as_deref()
                .map(|u| conjugate_transpose(u, columns, transposed_result.u_columns))
                .transpose()?,
            u_columns: rows,
            vh_rows: transposed_result.u_columns,
        }
    };
    request.check_cancellation()?;
    Ok(SvdResult {
        singular_values: DenseArray::from_vec(
            Shape::new([dimensions.order(), 1])?,
            result.singular_values,
        )?,
        u: result
            .u
            .map(|values| {
                DenseArray::from_vec(
                    Shape::new([
                        dimensions.rows(),
                        u64::try_from(result.u_columns).unwrap_or(u64::MAX),
                    ])?,
                    values,
                )
            })
            .transpose()?,
        vh: result
            .vh
            .map(|values| {
                DenseArray::from_vec(
                    Shape::new([
                        u64::try_from(result.vh_rows).unwrap_or(u64::MAX),
                        dimensions.columns(),
                    ])?,
                    values,
                )
            })
            .transpose()?,
    })
}

pub(crate) fn eig<T: EigInput>(
    request: EigRequest<'_, T>,
) -> Result<EigResult<T::Complex>, LinalgError> {
    let dimensions = validate_eig(&request)?;
    request.check_cancellation()?;
    let order = host_dimension("reference eig order", dimensions.rows())?;
    let mut matrix = try_vec(
        "reference eig matrix copy",
        order.saturating_mul(order),
        T::Complex::zero(),
    )?;
    for (destination, &source) in matrix.iter_mut().zip(request.matrix.as_slice()) {
        *destination = source.to_complex();
    }
    let result = complex_eig(
        matrix,
        order,
        request.left_vectors,
        request.right_vectors,
        default_eig_iterations(order),
        || request.check_cancellation(),
    )?;
    request.check_cancellation()?;
    Ok(EigResult {
        eigenvalues: DenseArray::from_vec(Shape::new([dimensions.rows(), 1])?, result.values)?,
        left_vectors: result
            .left
            .map(|values| {
                DenseArray::from_vec(Shape::new([dimensions.rows(), dimensions.rows()])?, values)
            })
            .transpose()?,
        right_vectors: result
            .right
            .map(|values| {
                DenseArray::from_vec(Shape::new([dimensions.rows(), dimensions.rows()])?, values)
            })
            .transpose()?,
    })
}

pub(crate) fn schur<C: SpectralScalar>(
    request: SchurRequest<'_, C>,
) -> Result<SchurResult<C>, LinalgError> {
    let dimensions = validate_schur(&request)?;
    request.check_cancellation()?;
    let order = host_dimension("reference Schur order", dimensions.rows())?;
    let matrix = try_copy("reference Schur matrix copy", request.matrix.as_slice())?;
    let mut result = complex_schur(
        matrix,
        order,
        true,
        default_eig_iterations(order),
        "complex Schur decomposition",
        || request.check_cancellation(),
    )?;
    for value in &mut result.form {
        *value = value.scale(result.scale);
    }
    request.check_cancellation()?;
    let shape = Shape::new([dimensions.rows(), dimensions.rows()])?;
    Ok(SchurResult {
        form: DenseArray::from_vec(shape.clone(), result.form)?,
        vectors: DenseArray::from_vec(
            shape,
            result.vectors.expect("Schur vectors were requested"),
        )?,
    })
}

struct TallSvd<T: SpectralScalar> {
    singular_values: Vec<T::Real>,
    u: Option<Vec<T>>,
    vh: Option<Vec<T>>,
    u_columns: usize,
    vh_rows: usize,
}

#[allow(clippy::too_many_lines, clippy::needless_range_loop)]
fn tall_svd<T: SpectralScalar>(
    input: &[T],
    rows: usize,
    columns: usize,
    vectors: SvdVectors,
    mut check_cancellation: impl FnMut() -> Result<(), LinalgError>,
) -> Result<TallSvd<T>, LinalgError> {
    debug_assert!(rows >= columns);
    let mut matrix = try_copy("reference SVD workspace", input)?;
    let scale = matrix.iter().fold(T::Real::zero(), |current, value| {
        current.maximum(value.magnitude())
    });
    if scale.greater_than(T::Real::zero()) {
        let inverse = T::Real::one().divide(scale);
        for value in &mut matrix {
            *value = value.scale(inverse);
        }
    }
    let wants_vectors = vectors != SvdVectors::None;
    let mut right = wants_vectors.then(|| identity::<T>(columns)).transpose()?;
    if columns > 1 && scale.greater_than(T::Real::zero()) {
        let max_sweeps = 64_usize.saturating_add(columns.saturating_mul(4));
        let mut converged = false;
        for _sweep in 0..max_sweeps {
            check_cancellation()?;
            let mut rotated = false;
            for left in 0..columns - 1 {
                for right_column in left + 1..columns {
                    let (alpha, beta, gamma) = column_gram(&matrix, rows, left, right_column);
                    let gamma_magnitude = gamma.magnitude();
                    let threshold = alpha
                        .multiply(beta)
                        .square_root()
                        .multiply(T::Real::epsilon())
                        .multiply(T::Real::from_usize(rows.max(columns)));
                    if !gamma_magnitude.greater_than(threshold) {
                        continue;
                    }
                    rotated = true;
                    let zeta = beta
                        .subtract(alpha)
                        .divide(T::Real::two().multiply(gamma_magnitude));
                    let sign = if zeta.less_than(T::Real::zero()) {
                        T::Real::zero().subtract(T::Real::one())
                    } else {
                        T::Real::one()
                    };
                    let tangent = sign.divide(
                        zeta.absolute()
                            .add(T::Real::one().add(zeta.multiply(zeta)).square_root()),
                    );
                    let cosine = T::Real::one()
                        .divide(T::Real::one().add(tangent.multiply(tangent)).square_root());
                    let sine = cosine.multiply(tangent);
                    let phase = gamma.scale(T::Real::one().divide(gamma_magnitude));
                    rotate_columns(&mut matrix, rows, left, right_column, cosine, sine, phase);
                    if let Some(right_vectors) = &mut right {
                        rotate_columns(
                            right_vectors,
                            columns,
                            left,
                            right_column,
                            cosine,
                            sine,
                            phase,
                        );
                    }
                }
            }
            if !rotated {
                converged = true;
                break;
            }
        }
        if !converged {
            return Err(LinalgError::NoConvergence {
                provider: REFERENCE_PROVIDER,
                operation: "singular value decomposition",
                iterations: Some(u64::try_from(max_sweeps).unwrap_or(u64::MAX)),
                unconverged: None,
            });
        }
    }

    let mut singular_values = try_vec("reference SVD singular values", columns, T::Real::zero())?;
    for column in 0..columns {
        singular_values[column] = column_norm(&matrix, rows, column).multiply(scale);
    }
    let mut order = try_vec("reference SVD sort order", columns, 0_usize)?;
    for (index, value) in order.iter_mut().enumerate() {
        *value = index;
    }
    order.sort_by(|&left, &right_index| {
        singular_values[right_index]
            .partial_compare(singular_values[left])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    reorder_columns(&mut matrix, rows, &order)?;
    if let Some(right_vectors) = &mut right {
        reorder_columns(right_vectors, columns, &order)?;
    }
    let mut sorted = try_vec(
        "reference SVD sorted singular values",
        columns,
        T::Real::zero(),
    )?;
    for (destination, &source) in sorted.iter_mut().zip(&order) {
        *destination = singular_values[source];
    }
    singular_values = sorted;

    if !wants_vectors {
        return Ok(TallSvd {
            singular_values,
            u: None,
            vh: None,
            u_columns: 0,
            vh_rows: 0,
        });
    }
    for column in 0..columns {
        let norm = column_norm(&matrix, rows, column);
        if norm.greater_than(T::Real::zero()) {
            let inverse = T::Real::one().divide(norm);
            for row in 0..rows {
                matrix[column * rows + row] = matrix[column * rows + row].scale(inverse);
            }
        } else {
            fill_orthogonal_column(&mut matrix, rows, column)?;
        }
    }
    let u_columns = match vectors {
        SvdVectors::Full => rows,
        SvdVectors::Thin => columns,
        SvdVectors::None => unreachable!(),
    };
    if u_columns > columns {
        let mut full = try_vec(
            "reference SVD full U",
            rows.saturating_mul(u_columns),
            T::zero(),
        )?;
        full[..rows * columns].copy_from_slice(&matrix);
        for column in columns..u_columns {
            fill_orthogonal_column(&mut full, rows, column)?;
        }
        matrix = full;
    }
    let right = right.expect("vector request allocated right singular vectors");
    let vh = conjugate_transpose(&right, columns, columns)?;
    Ok(TallSvd {
        singular_values,
        u: Some(matrix),
        vh: Some(vh),
        u_columns,
        vh_rows: columns,
    })
}

fn column_gram<T: SpectralScalar>(
    matrix: &[T],
    rows: usize,
    left: usize,
    right: usize,
) -> (T::Real, T::Real, T) {
    let mut alpha = T::Real::zero();
    let mut beta = T::Real::zero();
    let mut gamma = T::zero();
    for row in 0..rows {
        let p = matrix[left * rows + row];
        let q = matrix[right * rows + row];
        alpha = p.magnitude().mul_add(p.magnitude(), alpha);
        beta = q.magnitude().mul_add(q.magnitude(), beta);
        gamma = gamma.add(p.conjugate().multiply(q));
    }
    (alpha, beta, gamma)
}

fn rotate_columns<T: SpectralScalar>(
    matrix: &mut [T],
    rows: usize,
    left: usize,
    right: usize,
    cosine: T::Real,
    sine: T::Real,
    phase: T,
) {
    for row in 0..rows {
        let p = matrix[left * rows + row];
        let q = matrix[right * rows + row];
        matrix[left * rows + row] = p
            .scale(cosine)
            .subtract(q.multiply(phase.conjugate()).scale(sine));
        matrix[right * rows + row] = p.multiply(phase).scale(sine).add(q.scale(cosine));
    }
}

fn column_norm<T: SpectralScalar>(matrix: &[T], rows: usize, column: usize) -> T::Real {
    let mut norm = T::Real::zero();
    for row in 0..rows {
        norm = norm.hypotenuse(matrix[column * rows + row].magnitude());
    }
    norm
}

fn fill_orthogonal_column<T: SpectralScalar>(
    matrix: &mut [T],
    rows: usize,
    column: usize,
) -> Result<(), LinalgError> {
    let tolerance = T::Real::epsilon().multiply(T::Real::from_usize(rows.max(1)));
    for candidate in 0..rows {
        for row in 0..rows {
            matrix[column * rows + row] = if row == candidate {
                T::one()
            } else {
                T::zero()
            };
        }
        for previous in 0..column {
            let mut product = T::zero();
            for row in 0..rows {
                product = product.add(
                    matrix[previous * rows + row]
                        .conjugate()
                        .multiply(matrix[column * rows + row]),
                );
            }
            for row in 0..rows {
                matrix[column * rows + row] = matrix[column * rows + row]
                    .subtract(matrix[previous * rows + row].multiply(product));
            }
        }
        let norm = column_norm(matrix, rows, column);
        if norm.greater_than(tolerance) {
            let inverse = T::Real::one().divide(norm);
            for row in 0..rows {
                matrix[column * rows + row] = matrix[column * rows + row].scale(inverse);
            }
            return Ok(());
        }
    }
    Err(LinalgError::ProviderFailure {
        provider: REFERENCE_PROVIDER,
        operation: "singular vector completion",
        detail: "could not construct a finite orthogonal complement".to_owned(),
    })
}

struct ComplexEig<C> {
    values: Vec<C>,
    left: Option<Vec<C>>,
    right: Option<Vec<C>>,
}

struct ScaledSchur<C: SpectralScalar> {
    form: Vec<C>,
    vectors: Option<Vec<C>>,
    scale: C::Real,
}

#[allow(clippy::too_many_lines)]
fn complex_eig<C: SpectralScalar>(
    matrix: Vec<C>,
    order: usize,
    left_vectors: bool,
    right_vectors: bool,
    max_iterations: usize,
    mut check_cancellation: impl FnMut() -> Result<(), LinalgError>,
) -> Result<ComplexEig<C>, LinalgError> {
    if order == 0 {
        return Ok(ComplexEig {
            values: Vec::new(),
            left: left_vectors.then(Vec::new),
            right: right_vectors.then(Vec::new),
        });
    }
    let wants_vectors = left_vectors || right_vectors;
    let result = complex_schur(
        matrix,
        order,
        wants_vectors,
        max_iterations,
        "general eigenvalue decomposition",
        &mut check_cancellation,
    )?;
    let mut values = try_vec("reference eig values", order, C::zero())?;
    for (index, value) in values.iter_mut().enumerate() {
        *value = result.form[index * order + index].scale(result.scale);
    }
    let right = if right_vectors {
        Some(schur_eigenvectors(
            &result.form,
            order,
            result.vectors.as_ref().expect("requested Schur vectors"),
            false,
        )?)
    } else {
        None
    };
    let left = if left_vectors {
        Some(schur_eigenvectors(
            &result.form,
            order,
            result.vectors.as_ref().expect("requested Schur vectors"),
            true,
        )?)
    } else {
        None
    };
    Ok(ComplexEig {
        values,
        left,
        right,
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn complex_schur<C: SpectralScalar>(
    mut matrix: Vec<C>,
    order: usize,
    wants_vectors: bool,
    max_iterations: usize,
    operation: &'static str,
    mut check_cancellation: impl FnMut() -> Result<(), LinalgError>,
) -> Result<ScaledSchur<C>, LinalgError> {
    if order == 0 {
        return Ok(ScaledSchur {
            form: Vec::new(),
            vectors: wants_vectors.then(Vec::new),
            scale: C::Real::one(),
        });
    }
    let scale = matrix.iter().fold(C::Real::zero(), |current, value| {
        current.maximum(value.magnitude())
    });
    if scale.greater_than(C::Real::zero()) {
        let inverse = C::Real::one().divide(scale);
        for value in &mut matrix {
            *value = value.scale(inverse);
        }
    }
    let mut schur_vectors = wants_vectors.then(|| identity::<C>(order)).transpose()?;
    hessenberg_reduce(
        &mut matrix,
        order,
        schur_vectors.as_mut(),
        &mut check_cancellation,
    )?;
    let tolerance = C::Real::epsilon()
        .multiply(C::Real::from_usize(order.max(1)))
        .multiply(C::Real::from_usize(8));
    let mut active = order;
    let mut iterations = 0_usize;
    while active > 1 {
        check_cancellation()?;
        while active > 1 && deflatable(&matrix, order, active - 1, tolerance) {
            matrix[(active - 2) * order + active - 1] = C::zero();
            active -= 1;
        }
        if active <= 1 {
            break;
        }
        if iterations >= max_iterations {
            return Err(LinalgError::NoConvergence {
                provider: REFERENCE_PROVIDER,
                operation,
                iterations: Some(u64::try_from(iterations).unwrap_or(u64::MAX)),
                unconverged: Some(u64::try_from(active).unwrap_or(u64::MAX)),
            });
        }
        let mut start = active - 1;
        while start > 0 && !deflatable(&matrix, order, start, tolerance) {
            start -= 1;
        }
        if start > 0 {
            matrix[(start - 1) * order + start] = C::zero();
        }
        let block = active - start;
        let shift = trailing_shift(&matrix, order, active);
        let mut shifted = try_vec(
            "reference eig shifted block",
            block.saturating_mul(block),
            C::zero(),
        )?;
        for column in 0..block {
            for row in 0..block {
                shifted[column * block + row] = matrix[(start + column) * order + start + row];
            }
            shifted[column * block + column] = shifted[column * block + column].subtract(shift);
        }
        let (q, _) = householder_qr(shifted, block)?;
        apply_similarity(&mut matrix, order, start, active, &q)?;
        if let Some(vectors) = &mut schur_vectors {
            multiply_columns(vectors, order, start, active, &q)?;
        }
        for column in 0..active {
            for row in column.saturating_add(2)..active {
                let local_scale = matrix[column * order + column]
                    .magnitude()
                    .add(matrix[row * order + row].magnitude())
                    .add(C::Real::one());
                if matrix[column * order + row]
                    .magnitude()
                    .less_than_or_equal(tolerance.multiply(local_scale))
                {
                    matrix[column * order + row] = C::zero();
                }
            }
        }
        iterations += 1;
    }
    Ok(ScaledSchur {
        form: matrix,
        vectors: schur_vectors,
        scale,
    })
}

#[allow(clippy::needless_range_loop)]
fn hessenberg_reduce<C: SpectralScalar>(
    matrix: &mut [C],
    order: usize,
    mut vectors: Option<&mut Vec<C>>,
    check_cancellation: &mut impl FnMut() -> Result<(), LinalgError>,
) -> Result<(), LinalgError> {
    for column in 0..order.saturating_sub(2) {
        check_cancellation()?;
        let length = order - column - 1;
        let mut reflector = try_vec("reference eig Hessenberg reflector", length, C::zero())?;
        for row in 0..length {
            reflector[row] = matrix[column * order + column + 1 + row];
        }
        let Some(beta) = make_reflector(&mut reflector) else {
            continue;
        };
        for trailing_column in column..order {
            let mut product = C::zero();
            for index in 0..length {
                product = product.add(
                    reflector[index]
                        .conjugate()
                        .multiply(matrix[trailing_column * order + column + 1 + index]),
                );
            }
            product = product.scale(beta);
            for index in 0..length {
                let offset = trailing_column * order + column + 1 + index;
                matrix[offset] = matrix[offset].subtract(reflector[index].multiply(product));
            }
        }
        for row in 0..order {
            let mut product = C::zero();
            for index in 0..length {
                product = product
                    .add(matrix[(column + 1 + index) * order + row].multiply(reflector[index]));
            }
            product = product.scale(beta);
            for index in 0..length {
                let offset = (column + 1 + index) * order + row;
                matrix[offset] =
                    matrix[offset].subtract(product.multiply(reflector[index].conjugate()));
            }
        }
        if let Some(schur) = vectors.as_deref_mut() {
            apply_reflector_right(schur, order, column + 1, &reflector, beta);
        }
        for row in column + 2..order {
            matrix[column * order + row] = C::zero();
        }
    }
    Ok(())
}

fn make_reflector<C: SpectralScalar>(vector: &mut [C]) -> Option<C::Real> {
    let norm = vector.iter().fold(C::Real::zero(), |current, value| {
        current.hypotenuse(value.magnitude())
    });
    if !norm.greater_than(C::Real::zero()) {
        return None;
    }
    let first = vector[0];
    let phase = if first.magnitude().greater_than(C::Real::zero()) {
        first.scale(C::Real::one().divide(first.magnitude()))
    } else {
        C::one()
    };
    vector[0] = vector[0].add(phase.scale(norm));
    let squared = vector.iter().fold(C::Real::zero(), |sum, value| {
        value.magnitude().mul_add(value.magnitude(), sum)
    });
    Some(C::Real::two().divide(squared))
}

#[allow(clippy::needless_range_loop)]
fn apply_reflector_right<C: SpectralScalar>(
    matrix: &mut [C],
    rows: usize,
    start: usize,
    reflector: &[C],
    beta: C::Real,
) {
    for row in 0..rows {
        let mut product = C::zero();
        for index in 0..reflector.len() {
            product = product.add(matrix[(start + index) * rows + row].multiply(reflector[index]));
        }
        product = product.scale(beta);
        for index in 0..reflector.len() {
            let offset = (start + index) * rows + row;
            matrix[offset] =
                matrix[offset].subtract(product.multiply(reflector[index].conjugate()));
        }
    }
}

#[allow(clippy::needless_range_loop)]
fn householder_qr<C: SpectralScalar>(
    mut matrix: Vec<C>,
    order: usize,
) -> Result<(Vec<C>, Vec<C>), LinalgError> {
    let mut q = identity::<C>(order)?;
    for column in 0..order {
        let length = order - column;
        let mut reflector = try_vec("reference eig QR reflector", length, C::zero())?;
        for row in 0..length {
            reflector[row] = matrix[column * order + column + row];
        }
        let Some(beta) = make_reflector(&mut reflector) else {
            continue;
        };
        for trailing_column in column..order {
            let mut product = C::zero();
            for index in 0..length {
                product = product.add(
                    reflector[index]
                        .conjugate()
                        .multiply(matrix[trailing_column * order + column + index]),
                );
            }
            product = product.scale(beta);
            for index in 0..length {
                let offset = trailing_column * order + column + index;
                matrix[offset] = matrix[offset].subtract(reflector[index].multiply(product));
            }
        }
        apply_reflector_right(&mut q, order, column, &reflector, beta);
    }
    Ok((q, matrix))
}

fn apply_similarity<C: SpectralScalar>(
    matrix: &mut [C],
    order: usize,
    start: usize,
    end: usize,
    q: &[C],
) -> Result<(), LinalgError> {
    let block = end - start;
    let mut temporary = try_vec(
        "reference eig similarity workspace",
        order.saturating_mul(block),
        C::zero(),
    )?;
    for column in 0..block {
        for row in 0..order {
            for inner in 0..block {
                temporary[column * order + row] = temporary[column * order + row]
                    .add(matrix[(start + inner) * order + row].multiply(q[column * block + inner]));
            }
        }
    }
    for column in 0..block {
        matrix[(start + column) * order..(start + column + 1) * order]
            .copy_from_slice(&temporary[column * order..(column + 1) * order]);
    }
    let mut left = try_vec(
        "reference eig left similarity workspace",
        block.saturating_mul(order),
        C::zero(),
    )?;
    for column in 0..order {
        for row in 0..block {
            for inner in 0..block {
                left[column * block + row] = left[column * block + row].add(
                    q[row * block + inner]
                        .conjugate()
                        .multiply(matrix[column * order + start + inner]),
                );
            }
        }
    }
    for column in 0..order {
        for row in 0..block {
            matrix[column * order + start + row] = left[column * block + row];
        }
    }
    Ok(())
}

fn multiply_columns<C: SpectralScalar>(
    matrix: &mut [C],
    rows: usize,
    start: usize,
    end: usize,
    q: &[C],
) -> Result<(), LinalgError> {
    let block = end - start;
    let mut result = try_vec(
        "reference eig Schur-vector workspace",
        rows.saturating_mul(block),
        C::zero(),
    )?;
    for column in 0..block {
        for row in 0..rows {
            for inner in 0..block {
                result[column * rows + row] = result[column * rows + row]
                    .add(matrix[(start + inner) * rows + row].multiply(q[column * block + inner]));
            }
        }
    }
    for column in 0..block {
        matrix[(start + column) * rows..(start + column + 1) * rows]
            .copy_from_slice(&result[column * rows..(column + 1) * rows]);
    }
    Ok(())
}

fn deflatable<C: SpectralScalar>(
    matrix: &[C],
    order: usize,
    row: usize,
    tolerance: C::Real,
) -> bool {
    let subdiagonal = matrix[(row - 1) * order + row].magnitude();
    let scale = matrix[(row - 1) * order + row - 1]
        .magnitude()
        .add(matrix[row * order + row].magnitude())
        .add(C::Real::one());
    subdiagonal.less_than_or_equal(tolerance.multiply(scale))
}

fn trailing_shift<C: SpectralScalar>(matrix: &[C], order: usize, active: usize) -> C {
    if active < 2 {
        return matrix[0];
    }
    let a = matrix[(active - 2) * order + active - 2];
    let b = matrix[(active - 1) * order + active - 2];
    let c = matrix[(active - 2) * order + active - 1];
    let d = matrix[(active - 1) * order + active - 1];
    let half = C::Real::one().divide(C::Real::two());
    let trace = a.add(d).scale(half);
    let delta = a.subtract(d).scale(half);
    let root = complex_sqrt(delta.multiply(delta).add(b.multiply(c)));
    let first = trace.add(root);
    let second = trace.subtract(root);
    if first
        .subtract(d)
        .magnitude()
        .less_than(second.subtract(d).magnitude())
    {
        first
    } else {
        second
    }
}

fn complex_sqrt<C: SpectralScalar>(value: C) -> C {
    let magnitude = value.magnitude();
    if !magnitude.greater_than(C::Real::zero()) {
        return C::zero();
    }
    let half = C::Real::one().divide(C::Real::two());
    let real = magnitude
        .add(value.real_part())
        .multiply(half)
        .maximum(C::Real::zero())
        .square_root();
    let imaginary_magnitude = magnitude
        .subtract(value.real_part())
        .multiply(half)
        .maximum(C::Real::zero())
        .square_root();
    let imaginary = if value.imaginary_part().less_than(C::Real::zero()) {
        C::Real::zero().subtract(imaginary_magnitude)
    } else {
        imaginary_magnitude
    };
    C::from_parts(real, imaginary)
}

fn schur_eigenvectors<C: SpectralScalar>(
    matrix: &[C],
    order: usize,
    schur: &[C],
    left: bool,
) -> Result<Vec<C>, LinalgError> {
    let mut result = try_vec(
        "reference eig eigenvectors",
        order.saturating_mul(order),
        C::zero(),
    )?;
    let epsilon = C::Real::epsilon().multiply(C::Real::from_usize(order.max(1)));
    for eigen_index in 0..order {
        let lambda = matrix[eigen_index * order + eigen_index];
        let mut vector = try_vec("reference eig triangular vector", order, C::zero())?;
        vector[eigen_index] = C::one();
        if left {
            for row in eigen_index + 1..order {
                let mut sum = C::zero();
                for inner in eigen_index..row {
                    sum = sum.add(
                        matrix[row * order + inner]
                            .conjugate()
                            .multiply(vector[inner]),
                    );
                }
                let denominator = matrix[row * order + row]
                    .conjugate()
                    .subtract(lambda.conjugate());
                vector[row] = regularized_divide(C::zero().subtract(sum), denominator, epsilon);
            }
        } else {
            for row in (0..eigen_index).rev() {
                let mut sum = C::zero();
                for inner in row + 1..=eigen_index {
                    sum = sum.add(matrix[inner * order + row].multiply(vector[inner]));
                }
                let denominator = matrix[row * order + row].subtract(lambda);
                vector[row] = regularized_divide(C::zero().subtract(sum), denominator, epsilon);
            }
        }
        for row in 0..order {
            for inner in 0..order {
                result[eigen_index * order + row] = result[eigen_index * order + row]
                    .add(schur[inner * order + row].multiply(vector[inner]));
            }
        }
        let norm = column_norm(&result, order, eigen_index);
        if norm.greater_than(C::Real::zero()) {
            let inverse = C::Real::one().divide(norm);
            for row in 0..order {
                result[eigen_index * order + row] =
                    result[eigen_index * order + row].scale(inverse);
            }
        }
    }
    Ok(result)
}

fn regularized_divide<C: SpectralScalar>(numerator: C, denominator: C, epsilon: C::Real) -> C {
    if denominator.magnitude().greater_than(epsilon) {
        numerator.divide(denominator)
    } else {
        numerator.divide(C::from_real(epsilon))
    }
}

fn reorder_columns<T: Copy>(
    matrix: &mut [T],
    rows: usize,
    order: &[usize],
) -> Result<(), LinalgError> {
    let original = try_copy("reference spectral reorder workspace", matrix)?;
    for (destination, &source) in order.iter().enumerate() {
        matrix[destination * rows..(destination + 1) * rows]
            .copy_from_slice(&original[source * rows..(source + 1) * rows]);
    }
    Ok(())
}

fn conjugate_transpose<T: SpectralScalar>(
    matrix: &[T],
    rows: usize,
    columns: usize,
) -> Result<Vec<T>, LinalgError> {
    let mut result = try_vec(
        "reference conjugate transpose",
        rows.saturating_mul(columns),
        T::zero(),
    )?;
    for column in 0..columns {
        for row in 0..rows {
            result[row * columns + column] = matrix[column * rows + row].conjugate();
        }
    }
    Ok(result)
}

fn identity<T: SpectralScalar>(order: usize) -> Result<Vec<T>, LinalgError> {
    let mut result = try_vec(
        "reference identity matrix",
        order.saturating_mul(order),
        T::zero(),
    )?;
    for index in 0..order {
        result[index * order + index] = T::one();
    }
    Ok(result)
}

fn host_dimension(operation: &'static str, value: u64) -> Result<usize, LinalgError> {
    usize::try_from(value).map_err(|_| LinalgError::AllocationFailure {
        operation,
        elements: value,
    })
}

fn try_vec<T: Copy>(
    operation: &'static str,
    length: usize,
    value: T,
) -> Result<Vec<T>, LinalgError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| LinalgError::AllocationFailure {
            operation,
            elements: u64::try_from(length).unwrap_or(u64::MAX),
        })?;
    result.resize(length, value);
    Ok(result)
}

fn try_copy<T: Copy>(operation: &'static str, source: &[T]) -> Result<Vec<T>, LinalgError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(source.len())
        .map_err(|_| LinalgError::AllocationFailure {
            operation,
            elements: u64::try_from(source.len()).unwrap_or(u64::MAX),
        })?;
    result.extend_from_slice(source);
    Ok(result)
}

fn default_eig_iterations(order: usize) -> usize {
    128_usize
        .saturating_mul(order.max(1))
        .saturating_mul(order.max(1))
}

pub(crate) trait SpectralReal: Copy {
    fn zero() -> Self;
    fn one() -> Self;
    fn two() -> Self;
    fn epsilon() -> Self;
    fn from_usize(value: usize) -> Self;
    fn add(self, rhs: Self) -> Self;
    fn subtract(self, rhs: Self) -> Self;
    fn multiply(self, rhs: Self) -> Self;
    fn divide(self, rhs: Self) -> Self;
    fn mul_add(self, multiplier: Self, addend: Self) -> Self;
    fn square_root(self) -> Self;
    fn hypotenuse(self, rhs: Self) -> Self;
    fn maximum(self, rhs: Self) -> Self;
    fn absolute(self) -> Self;
    fn less_than(self, rhs: Self) -> bool;
    fn less_than_or_equal(self, rhs: Self) -> bool;
    fn greater_than(self, rhs: Self) -> bool;
    fn partial_compare(self, rhs: Self) -> Option<std::cmp::Ordering>;
}

macro_rules! real_impl {
    ($real:ty) => {
        impl SpectralReal for $real {
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
            fn from_usize(value: usize) -> Self {
                value as Self
            }
            fn add(self, rhs: Self) -> Self {
                self + rhs
            }
            fn subtract(self, rhs: Self) -> Self {
                self - rhs
            }
            fn multiply(self, rhs: Self) -> Self {
                self * rhs
            }
            fn divide(self, rhs: Self) -> Self {
                self / rhs
            }
            fn mul_add(self, multiplier: Self, addend: Self) -> Self {
                self.mul_add(multiplier, addend)
            }
            fn square_root(self) -> Self {
                self.sqrt()
            }
            fn hypotenuse(self, rhs: Self) -> Self {
                self.hypot(rhs)
            }
            fn maximum(self, rhs: Self) -> Self {
                self.max(rhs)
            }
            fn absolute(self) -> Self {
                self.abs()
            }
            fn less_than(self, rhs: Self) -> bool {
                self < rhs
            }
            fn less_than_or_equal(self, rhs: Self) -> bool {
                self <= rhs
            }
            fn greater_than(self, rhs: Self) -> bool {
                self > rhs
            }
            fn partial_compare(self, rhs: Self) -> Option<std::cmp::Ordering> {
                self.partial_cmp(&rhs)
            }
        }
    };
}
real_impl!(f32);
real_impl!(f64);

pub(crate) trait SpectralScalar: Copy {
    type Real: SpectralReal;
    fn zero() -> Self;
    fn one() -> Self;
    fn from_real(value: Self::Real) -> Self;
    fn from_parts(real: Self::Real, imaginary: Self::Real) -> Self;
    fn real_part(self) -> Self::Real;
    fn imaginary_part(self) -> Self::Real;
    fn magnitude(self) -> Self::Real;
    fn conjugate(self) -> Self;
    fn add(self, rhs: Self) -> Self;
    fn subtract(self, rhs: Self) -> Self;
    fn multiply(self, rhs: Self) -> Self;
    fn divide(self, rhs: Self) -> Self;
    fn scale(self, factor: Self::Real) -> Self;
}

macro_rules! real_scalar_impl {
    ($real:ty) => {
        impl SpectralScalar for $real {
            type Real = $real;
            fn zero() -> Self {
                0.0
            }
            fn one() -> Self {
                1.0
            }
            fn from_real(value: Self) -> Self {
                value
            }
            fn from_parts(real: Self, _imaginary: Self) -> Self {
                real
            }
            fn real_part(self) -> Self {
                self
            }
            fn imaginary_part(self) -> Self {
                0.0
            }
            fn magnitude(self) -> Self {
                self.abs()
            }
            fn conjugate(self) -> Self {
                self
            }
            fn add(self, rhs: Self) -> Self {
                self + rhs
            }
            fn subtract(self, rhs: Self) -> Self {
                self - rhs
            }
            fn multiply(self, rhs: Self) -> Self {
                self * rhs
            }
            fn divide(self, rhs: Self) -> Self {
                self / rhs
            }
            fn scale(self, factor: Self) -> Self {
                self * factor
            }
        }
    };
}
real_scalar_impl!(f32);
real_scalar_impl!(f64);

macro_rules! complex_scalar_impl {
    ($complex:ty, $real:ty) => {
        impl SpectralScalar for $complex {
            type Real = $real;
            fn zero() -> Self {
                Self::ZERO
            }
            fn one() -> Self {
                Self::new(1.0, 0.0)
            }
            fn from_real(value: $real) -> Self {
                Self::new(value, 0.0)
            }
            fn from_parts(real: $real, imaginary: $real) -> Self {
                Self::new(real, imaginary)
            }
            fn real_part(self) -> $real {
                self.re
            }
            fn imaginary_part(self) -> $real {
                self.im
            }
            fn magnitude(self) -> $real {
                self.re.hypot(self.im)
            }
            fn conjugate(self) -> Self {
                self.conjugate()
            }
            fn add(self, rhs: Self) -> Self {
                Self::new(self.re + rhs.re, self.im + rhs.im)
            }
            fn subtract(self, rhs: Self) -> Self {
                Self::new(self.re - rhs.re, self.im - rhs.im)
            }
            fn multiply(self, rhs: Self) -> Self {
                Self::new(
                    self.re.mul_add(rhs.re, -(self.im * rhs.im)),
                    self.re.mul_add(rhs.im, self.im * rhs.re),
                )
            }
            fn divide(self, rhs: Self) -> Self {
                if rhs.re.abs() >= rhs.im.abs() {
                    let ratio = rhs.im / rhs.re;
                    let denominator = rhs.re + rhs.im * ratio;
                    Self::new(
                        (self.re + self.im * ratio) / denominator,
                        (self.im - self.re * ratio) / denominator,
                    )
                } else {
                    let ratio = rhs.re / rhs.im;
                    let denominator = rhs.im + rhs.re * ratio;
                    Self::new(
                        (self.re * ratio + self.im) / denominator,
                        (self.im * ratio - self.re) / denominator,
                    )
                }
            }
            fn scale(self, factor: $real) -> Self {
                Self::new(self.re * factor, self.im * factor)
            }
        }
    };
}
complex_scalar_impl!(Complex32, f32);
complex_scalar_impl!(Complex64, f64);

pub(crate) trait EigInput: Copy {
    type Complex: SpectralScalar;
    fn to_complex(self) -> Self::Complex;
}
impl EigInput for f32 {
    type Complex = Complex32;
    fn to_complex(self) -> Complex32 {
        Complex32::new(self, 0.0)
    }
}
impl EigInput for f64 {
    type Complex = Complex64;
    fn to_complex(self) -> Complex64 {
        Complex64::new(self, 0.0)
    }
}
impl EigInput for Complex32 {
    type Complex = Complex32;
    fn to_complex(self) -> Complex32 {
        self
    }
}
impl EigInput for Complex64 {
    type Complex = Complex64;
    fn to_complex(self) -> Complex64 {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_iteration_eig_reports_no_convergence() {
        let matrix = vec![
            Complex64::new(0.0, 0.0),
            Complex64::new(1.0, 0.0),
            Complex64::new(-1.0, 0.0),
            Complex64::new(0.0, 0.0),
        ];
        assert!(matches!(
            complex_eig(matrix, 2, false, false, 0, || Ok(())),
            Err(LinalgError::NoConvergence { .. })
        ));
    }
}

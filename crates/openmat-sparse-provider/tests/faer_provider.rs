use std::sync::atomic::AtomicBool;

use openmat_array::Complex64;
use openmat_linalg::{
    CscMatrixRef, OwnedCscMatrix, SparseCancellation, SparseError, SparseIndexWidth,
    SparseNumericFactor, SparseProvider,
};
use openmat_sparse_provider::{FaerOperationBackend, FaerSparseProvider, ReferenceSparseProvider};

fn real_csc(
    rows: u64,
    columns: u64,
    col_offsets: &[u64],
    row_indices: &[u64],
    values: &[f64],
) -> OwnedCscMatrix<f64> {
    OwnedCscMatrix::new(
        rows,
        columns,
        col_offsets.to_vec(),
        row_indices.to_vec(),
        values.to_vec(),
    )
    .unwrap()
}

fn complex_csc(
    rows: u64,
    columns: u64,
    col_offsets: &[u64],
    row_indices: &[u64],
    values: &[Complex64],
) -> OwnedCscMatrix<Complex64> {
    OwnedCscMatrix::new(
        rows,
        columns,
        col_offsets.to_vec(),
        row_indices.to_vec(),
        values.to_vec(),
    )
    .unwrap()
}

fn assert_real_close(actual: &[f64], expected: &[f64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len());
    for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        let scale = expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tolerance * scale,
            "entry {index}: {actual} != {expected}"
        );
    }
}

fn assert_complex_close(actual: &[Complex64], expected: &[Complex64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        let error = (actual.re - expected.re).hypot(actual.im - expected.im);
        let scale = expected.re.hypot(expected.im).max(1.0);
        assert!(
            error <= tolerance * scale,
            "entry {index}: {actual:?} != {expected:?}"
        );
    }
}

#[test]
fn faer_capabilities_are_explicit_and_do_not_claim_a_fallback() {
    let provider = FaerSparseProvider;
    assert_eq!(provider.name(), "faer-0.24.4");
    assert_eq!(provider.operation_backend(), FaerOperationBackend::Faer);
    let capabilities = provider.capabilities();
    assert!(capabilities.spmv);
    assert!(capabilities.spgemm);
    assert!(capabilities.lu);
    assert!(capabilities.cholesky);
    assert!(capabilities.least_squares);
    assert!(capabilities.underdetermined_basic);
}

#[test]
fn faer_spmv_and_spgemm_match_reference_for_real_and_complex_csc() {
    let faer = FaerSparseProvider;
    let reference = ReferenceSparseProvider;
    let real = real_csc(
        3,
        3,
        &[0, 2, 4, 6],
        &[0, 1, 0, 1, 1, 2],
        &[4.0, 2.0, 1.0, 3.0, 5.0, 6.0],
    );
    let vector = [1.0, 2.0, 3.0];
    let (actual, metrics) = faer
        .spmv_f64(real.as_ref(), &vector, SparseCancellation::never())
        .unwrap();
    let expected = reference
        .spmv_f64(real.as_ref(), &vector, SparseCancellation::never())
        .unwrap()
        .0;
    assert_real_close(&actual, &expected, 1.0e-12);
    assert_eq!(metrics.input_nnz, 6);
    let repeated_metrics = faer
        .spmv_f64(real.as_ref(), &vector, SparseCancellation::never())
        .unwrap()
        .1;
    assert_eq!(metrics, repeated_metrics);

    let left = real_csc(2, 3, &[0, 1, 2, 4], &[0, 1, 0, 1], &[1.0, 3.0, 2.0, 4.0]);
    let right = real_csc(3, 2, &[0, 2, 4], &[0, 2, 1, 2], &[5.0, 7.0, 6.0, 8.0]);
    let (actual, actual_metrics) = faer
        .spgemm_f64(left.as_ref(), right.as_ref(), SparseCancellation::never())
        .unwrap();
    let (expected, _) = reference
        .spgemm_f64(left.as_ref(), right.as_ref(), SparseCancellation::never())
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual_metrics.output_nnz, 4);

    let complex = complex_csc(
        2,
        2,
        &[0, 1, 3],
        &[0, 0, 1],
        &[
            Complex64::new(1.0, 1.0),
            Complex64::new(2.0, -1.0),
            Complex64::new(3.0, 0.0),
        ],
    );
    let complex_vector = [Complex64::new(1.0, -1.0), Complex64::new(2.0, 1.0)];
    let actual = faer
        .spmv_complex64(
            complex.as_ref(),
            &complex_vector,
            SparseCancellation::never(),
        )
        .unwrap()
        .0;
    let expected = reference
        .spmv_complex64(
            complex.as_ref(),
            &complex_vector,
            SparseCancellation::never(),
        )
        .unwrap()
        .0;
    assert_complex_close(&actual, &expected, 1.0e-12);

    let actual = faer
        .spgemm_complex64(
            complex.as_ref(),
            complex.as_ref(),
            SparseCancellation::never(),
        )
        .unwrap()
        .0;
    let expected = reference
        .spgemm_complex64(
            complex.as_ref(),
            complex.as_ref(),
            SparseCancellation::never(),
        )
        .unwrap()
        .0;
    let (actual_rows, actual_columns, actual_offsets, actual_indices, actual_values) =
        actual.into_parts();
    let (expected_rows, expected_columns, expected_offsets, expected_indices, expected_values) =
        expected.into_parts();
    assert_eq!(
        (actual_rows, actual_columns),
        (expected_rows, expected_columns)
    );
    assert_eq!(actual_offsets, expected_offsets);
    assert_eq!(actual_indices, expected_indices);
    assert_complex_close(&actual_values, &expected_values, 1.0e-12);
}

#[test]
fn faer_spgemm_prunes_exact_cancellation_to_canonical_empty_column() {
    let provider = FaerSparseProvider;
    let left = real_csc(1, 2, &[0, 1, 2], &[0, 0], &[1.0, 1.0]);
    let right = real_csc(2, 1, &[0, 2], &[0, 1], &[1.0, -1.0]);
    let zero = provider
        .spgemm_f64(left.as_ref(), right.as_ref(), SparseCancellation::never())
        .unwrap()
        .0;
    assert_eq!(zero.into_parts(), (1, 1, vec![0, 0], vec![], vec![]));
}

#[test]
fn faer_lu_reuses_real_and_complex_factors_and_matches_reference() {
    let faer = FaerSparseProvider;
    let reference = ReferenceSparseProvider;
    let real = real_csc(
        3,
        3,
        &[0, 2, 4, 7],
        &[0, 2, 0, 1, 0, 1, 2],
        &[4.0, 2.0, 1.0, 3.0, 2.0, -1.0, 5.0],
    );
    let expected = [1.0, 2.0, -1.0, -2.0, 1.0, 3.0];
    let mut rhs = Vec::new();
    for solution in expected.chunks(3) {
        rhs.extend(
            reference
                .spmv_f64(real.as_ref(), solution, SparseCancellation::never())
                .unwrap()
                .0,
        );
    }
    let symbolic = faer
        .analyze_lu(real.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = faer
        .factor_lu_f64(&symbolic, real.as_ref(), SparseCancellation::never())
        .unwrap();
    let first = factor.solve(&rhs, 2, SparseCancellation::never()).unwrap();
    let second = factor
        .solve(&rhs[..3], 1, SparseCancellation::never())
        .unwrap();
    assert_real_close(&first.values, &expected, 1.0e-11);
    assert_real_close(&second.values, &expected[..3], 1.0e-11);
    assert_eq!(factor.metrics().input_nnz, real.as_ref().pattern().nnz());

    let complex = complex_csc(
        2,
        2,
        &[0, 2, 4],
        &[0, 1, 0, 1],
        &[
            Complex64::new(2.0, 1.0),
            Complex64::new(1.0, -1.0),
            Complex64::new(1.0, 0.0),
            Complex64::new(3.0, 2.0),
        ],
    );
    let expected = [
        Complex64::new(1.0, 2.0),
        Complex64::new(-1.0, 0.5),
        Complex64::new(0.5, -2.0),
        Complex64::new(3.0, 1.0),
    ];
    let mut rhs = Vec::new();
    for solution in expected.chunks(2) {
        rhs.extend(
            reference
                .spmv_complex64(complex.as_ref(), solution, SparseCancellation::never())
                .unwrap()
                .0,
        );
    }
    let symbolic = faer
        .analyze_lu(complex.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = faer
        .factor_lu_complex64(&symbolic, complex.as_ref(), SparseCancellation::never())
        .unwrap();
    let actual = factor.solve(&rhs, 2, SparseCancellation::never()).unwrap();
    assert_complex_close(&actual.values, &expected, 1.0e-11);
    for (solution, expected_rhs) in actual.values.chunks(2).zip(rhs.chunks(2)) {
        let residual = faer
            .spmv_complex64(complex.as_ref(), solution, SparseCancellation::never())
            .unwrap()
            .0;
        assert_complex_close(&residual, expected_rhs, 1.0e-11);
    }
}

#[test]
fn faer_lu_reports_singular_and_pattern_mismatch_without_panicking() {
    let provider = FaerSparseProvider;
    let singular = real_csc(2, 2, &[0, 2, 4], &[0, 1, 0, 1], &[1.0, 2.0, 2.0, 4.0]);
    let symbolic = provider
        .analyze_lu(singular.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    assert!(matches!(
        provider.factor_lu_f64(&symbolic, singular.as_ref(), SparseCancellation::never()),
        Err(SparseError::Singular { pivot: 2, .. })
    ));

    let other = real_csc(2, 2, &[0, 1, 2], &[0, 1], &[1.0, 1.0]);
    assert!(matches!(
        provider.factor_lu_f64(&symbolic, other.as_ref(), SparseCancellation::never()),
        Err(SparseError::SymbolicPatternMismatch)
    ));
}

#[test]
fn faer_cholesky_matches_reference_and_rejects_invalid_inputs() {
    let faer = FaerSparseProvider;
    let reference = ReferenceSparseProvider;
    let real = real_csc(2, 2, &[0, 2, 4], &[0, 1, 0, 1], &[4.0, 1.0, 1.0, 3.0]);
    let symbolic = faer
        .analyze_cholesky(real.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = faer
        .factor_cholesky_f64(&symbolic, real.as_ref(), SparseCancellation::never())
        .unwrap();
    let rhs = [6.0, 7.0, 2.0, 5.0];
    let actual = factor.solve(&rhs, 2, SparseCancellation::never()).unwrap();
    let expected = reference
        .least_squares_f64(real.as_ref(), &rhs, 2, SparseCancellation::never())
        .unwrap();
    assert_real_close(&actual.values, &expected.values, 1.0e-11);

    let complex = complex_csc(
        2,
        2,
        &[0, 2, 4],
        &[0, 1, 0, 1],
        &[
            Complex64::new(4.0, 0.0),
            Complex64::new(1.0, -1.0),
            Complex64::new(1.0, 1.0),
            Complex64::new(3.0, 0.0),
        ],
    );
    let expected = [Complex64::new(1.0, 1.0), Complex64::new(2.0, -1.0)];
    let rhs = reference
        .spmv_complex64(complex.as_ref(), &expected, SparseCancellation::never())
        .unwrap()
        .0;
    let symbolic = faer
        .analyze_cholesky(complex.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = faer
        .factor_cholesky_complex64(&symbolic, complex.as_ref(), SparseCancellation::never())
        .unwrap();
    let actual = factor.solve(&rhs, 1, SparseCancellation::never()).unwrap();
    assert_complex_close(&actual.values, &expected, 1.0e-11);

    let indefinite = real_csc(2, 2, &[0, 2, 4], &[0, 1, 0, 1], &[1.0, 2.0, 2.0, 1.0]);
    let symbolic = faer
        .analyze_cholesky(indefinite.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    assert!(matches!(
        faer.factor_cholesky_f64(&symbolic, indefinite.as_ref(), SparseCancellation::never()),
        Err(SparseError::NotPositiveDefinite { .. })
    ));

    let asymmetric = real_csc(2, 2, &[0, 1, 3], &[0, 0, 1], &[2.0, 1.0, 2.0]);
    let symbolic = faer
        .analyze_cholesky(asymmetric.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    assert!(matches!(
        faer.factor_cholesky_f64(&symbolic, asymmetric.as_ref(), SparseCancellation::never()),
        Err(SparseError::NotHermitian { .. })
    ));

    let non_hermitian = complex_csc(1, 1, &[0, 1], &[0], &[Complex64::new(2.0, 1.0)]);
    let symbolic = faer
        .analyze_cholesky(
            non_hermitian.as_ref().pattern(),
            SparseCancellation::never(),
        )
        .unwrap();
    assert!(matches!(
        faer.factor_cholesky_complex64(
            &symbolic,
            non_hermitian.as_ref(),
            SparseCancellation::never()
        ),
        Err(SparseError::NotHermitian { .. })
    ));
}

#[test]
fn faer_tall_qr_reuses_factors_and_reports_rank_deficiency() {
    let faer = FaerSparseProvider;
    let reference = ReferenceSparseProvider;
    let real = real_csc(
        4,
        2,
        &[0, 4, 7],
        &[0, 1, 2, 3, 0, 1, 3],
        &[1.0, 1.0, 1.0, 1.0, 1.0, 2.0, -1.0],
    );
    let expected = [2.0, -1.0, -3.0, 4.0];
    let mut rhs = Vec::new();
    for solution in expected.chunks(2) {
        rhs.extend(
            reference
                .spmv_f64(real.as_ref(), solution, SparseCancellation::never())
                .unwrap()
                .0,
        );
    }
    let symbolic = faer
        .analyze_qr(real.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = faer
        .factor_qr_f64(&symbolic, real.as_ref(), SparseCancellation::never())
        .unwrap();
    let actual = factor.solve(&rhs, 2, SparseCancellation::never()).unwrap();
    assert_real_close(&actual.values, &expected, 1.0e-10);
    let repeated = factor
        .solve(&rhs[..4], 1, SparseCancellation::never())
        .unwrap();
    assert_real_close(&repeated.values, &expected[..2], 1.0e-10);

    let complex = complex_csc(
        3,
        2,
        &[0, 2, 4],
        &[0, 2, 1, 2],
        &[
            Complex64::new(1.0, 0.0),
            Complex64::new(1.0, 0.0),
            Complex64::new(1.0, 0.0),
            Complex64::new(0.0, 1.0),
        ],
    );
    let expected = [Complex64::new(1.0, 1.0), Complex64::new(2.0, -1.0)];
    let rhs = reference
        .spmv_complex64(complex.as_ref(), &expected, SparseCancellation::never())
        .unwrap()
        .0;
    let symbolic = faer
        .analyze_qr(complex.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = faer
        .factor_qr_complex64(&symbolic, complex.as_ref(), SparseCancellation::never())
        .unwrap();
    let actual = factor.solve(&rhs, 1, SparseCancellation::never()).unwrap();
    assert_complex_close(&actual.values, &expected, 1.0e-10);

    let deficient = real_csc(
        3,
        2,
        &[0, 3, 6],
        &[0, 1, 2, 0, 1, 2],
        &[1.0, 2.0, 3.0, 1.0, 2.0, 3.0],
    );
    let symbolic = faer
        .analyze_qr(deficient.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    assert!(matches!(
        faer.factor_qr_f64(&symbolic, deficient.as_ref(), SparseCancellation::never()),
        Err(SparseError::RankDeficient {
            rank: 1,
            required_rank: 2,
            ..
        })
    ));

    let underdetermined = real_csc(1, 2, &[0, 1, 2], &[0, 0], &[1.0, 2.0]);
    assert!(matches!(
        faer.analyze_qr(
            underdetermined.as_ref().pattern(),
            SparseCancellation::never()
        ),
        Err(SparseError::Unsupported {
            provider: "faer-0.24.4",
            operation: "underdetermined sparse least squares",
        })
    ));
}

#[test]
fn faer_sparse_basic_solve_uses_leftmost_independent_columns_for_multiple_rhs() {
    let faer = FaerSparseProvider;
    let reference = ReferenceSparseProvider;
    let real = real_csc(2, 3, &[0, 1, 2, 4], &[0, 1, 0, 1], &[1.0, 1.0, 1.0, 1.0]);
    let rhs = [1.0, 2.0, 2.0, 3.0];
    let expected = [1.0, 2.0, 0.0, 2.0, 3.0, 0.0];
    let actual = faer
        .underdetermined_basic_f64(real.as_ref(), &rhs, 2, SparseCancellation::never())
        .unwrap();
    let oracle = reference
        .underdetermined_basic_f64(real.as_ref(), &rhs, 2, SparseCancellation::never())
        .unwrap();
    assert_eq!((actual.rows, actual.columns), (3, 2));
    assert_real_close(&actual.values, &expected, 1.0e-11);
    assert_real_close(&actual.values, &oracle.values, 1.0e-11);

    let complex = complex_csc(
        2,
        3,
        &[0, 1, 2, 4],
        &[0, 1, 0, 1],
        &[
            Complex64::new(1.0, 1.0),
            Complex64::new(2.0, -1.0),
            Complex64::new(1.0, 0.0),
            Complex64::new(1.0, 0.0),
        ],
    );
    let expected = [
        Complex64::new(1.0, 2.0),
        Complex64::new(-1.0, 0.5),
        Complex64::ZERO,
        Complex64::new(2.0, -1.0),
        Complex64::new(3.0, 2.0),
        Complex64::ZERO,
    ];
    let mut rhs = Vec::new();
    for solution in expected.chunks(3) {
        rhs.extend(
            faer.spmv_complex64(complex.as_ref(), solution, SparseCancellation::never())
                .unwrap()
                .0,
        );
    }
    let actual = faer
        .underdetermined_basic_complex64(complex.as_ref(), &rhs, 2, SparseCancellation::never())
        .unwrap();
    assert_complex_close(&actual.values, &expected, 1.0e-11);

    let deficient = real_csc(2, 3, &[0, 2, 4, 4], &[0, 1, 0, 1], &[1.0, 2.0, 2.0, 4.0]);
    assert!(matches!(
        faer.underdetermined_basic_f64(
            deficient.as_ref(),
            &[1.0, 2.0],
            1,
            SparseCancellation::never()
        ),
        Err(SparseError::RankDeficient {
            rank: 1,
            required_rank: 2,
            ..
        })
    ));
}

#[test]
fn faer_checks_index_width_and_cancellation_before_user_data_reaches_faer() {
    let provider = FaerSparseProvider;
    let (width, maximum) = match provider.index_width() {
        SparseIndexWidth::I32 => (SparseIndexWidth::I32, i32::MAX as u64),
        SparseIndexWidth::I64 => (SparseIndexWidth::I64, i64::MAX as u64),
    };
    let huge = CscMatrixRef::new(maximum + 1, 0, &[0], &[], &[] as &[f64]).unwrap();
    assert_eq!(
        provider.spmv_f64(huge, &[], SparseCancellation::never()),
        Err(SparseError::IndexWidthOverflow {
            width,
            parameter: "rows",
            value: maximum + 1,
        })
    );

    let matrix = real_csc(2, 2, &[0, 1, 2], &[0, 1], &[2.0, 3.0]);
    let symbolic = provider
        .analyze_lu(matrix.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = provider
        .factor_lu_f64(&symbolic, matrix.as_ref(), SparseCancellation::never())
        .unwrap();
    let flag = AtomicBool::new(true);
    assert!(matches!(
        provider.spmv_f64(
            matrix.as_ref(),
            &[1.0, 1.0],
            SparseCancellation::from_atomic(&flag)
        ),
        Err(SparseError::Cancelled { .. })
    ));
    assert!(matches!(
        provider.analyze_lu(
            matrix.as_ref().pattern(),
            SparseCancellation::from_atomic(&flag)
        ),
        Err(SparseError::Cancelled { .. })
    ));
    assert!(matches!(
        provider.factor_lu_f64(
            &symbolic,
            matrix.as_ref(),
            SparseCancellation::from_atomic(&flag)
        ),
        Err(SparseError::Cancelled { .. })
    ));
    assert!(matches!(
        factor.solve(&[2.0, 3.0], 1, SparseCancellation::from_atomic(&flag)),
        Err(SparseError::Cancelled { .. })
    ));
}

#[test]
fn moderately_sized_sparse_cholesky_smoke_has_residual_and_structural_metrics() {
    let provider = FaerSparseProvider;
    let order = 256_usize;
    let mut offsets = Vec::with_capacity(order + 1);
    let mut rows = Vec::with_capacity(order * 3 - 2);
    let mut values = Vec::with_capacity(order * 3 - 2);
    offsets.push(0_u64);
    for column in 0..order {
        if column > 0 {
            rows.push((column - 1) as u64);
            values.push(-1.0);
        }
        rows.push(column as u64);
        values.push(4.0);
        if column + 1 < order {
            rows.push((column + 1) as u64);
            values.push(-1.0);
        }
        offsets.push(rows.len() as u64);
    }
    let matrix = OwnedCscMatrix::new(order as u64, order as u64, offsets, rows, values).unwrap();
    let expected: Vec<_> = (0..order)
        .map(|index| f64::from(u32::try_from(index % 13).unwrap()) - 6.0)
        .collect();
    let (rhs, spmv_metrics) = provider
        .spmv_f64(matrix.as_ref(), &expected, SparseCancellation::never())
        .unwrap();
    let symbolic = provider
        .analyze_cholesky(matrix.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = provider
        .factor_cholesky_f64(&symbolic, matrix.as_ref(), SparseCancellation::never())
        .unwrap();
    let actual = factor.solve(&rhs, 1, SparseCancellation::never()).unwrap();
    assert_real_close(&actual.values, &expected, 1.0e-10);
    let residual = provider
        .spmv_f64(matrix.as_ref(), &actual.values, SparseCancellation::never())
        .unwrap()
        .0;
    assert_real_close(&residual, &rhs, 1.0e-10);
    assert_eq!(spmv_metrics.input_nnz, matrix.as_ref().pattern().nnz());
    assert_eq!(factor.metrics().input_nnz, matrix.as_ref().pattern().nnz());
}

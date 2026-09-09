use std::sync::atomic::AtomicBool;

use openmat_array::Complex64;
use openmat_linalg::{
    OwnedCscMatrix, SparseCancellation, SparseError, SparseNumericFactor, SparseProvider,
};
use openmat_sparse_provider::ReferenceSparseProvider;

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

fn assert_real_close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() <= 1.0e-11,
            "{actual} != {expected}"
        );
    }
}

fn assert_complex_close(actual: &[Complex64], expected: &[Complex64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        let error = (actual.re - expected.re).hypot(actual.im - expected.im);
        assert!(error <= 1.0e-11, "{actual:?} != {expected:?}");
    }
}

#[test]
fn spmv_and_spgemm_produce_canonical_real_csc() {
    let provider = ReferenceSparseProvider;
    let matrix = real_csc(
        3,
        3,
        &[0, 2, 4, 6],
        &[0, 1, 0, 1, 1, 2],
        &[4.0, 2.0, 1.0, 3.0, 5.0, 6.0],
    );
    let (product, metrics) = provider
        .spmv_f64(
            matrix.as_ref(),
            &[1.0, 2.0, 3.0],
            SparseCancellation::never(),
        )
        .unwrap();
    assert_real_close(&product, &[6.0, 23.0, 18.0]);
    assert_eq!(metrics.input_nnz, 6);
    assert!(metrics.numeric_work > 0);

    let left = real_csc(2, 3, &[0, 1, 2, 4], &[0, 1, 0, 1], &[1.0, 3.0, 2.0, 4.0]);
    let right = real_csc(3, 2, &[0, 2, 4], &[0, 2, 1, 2], &[5.0, 7.0, 6.0, 8.0]);
    let (result, metrics) = provider
        .spgemm_f64(left.as_ref(), right.as_ref(), SparseCancellation::never())
        .unwrap();
    let (_, _, offsets, rows, values) = result.into_parts();
    assert_eq!(offsets, vec![0, 2, 4]);
    assert_eq!(rows, vec![0, 1, 0, 1]);
    assert_real_close(&values, &[19.0, 28.0, 16.0, 50.0]);
    assert_eq!(metrics.output_nnz, 4);
}

#[test]
fn complex_spmv_and_spgemm_preserve_complex_arithmetic() {
    let provider = ReferenceSparseProvider;
    let matrix = complex_csc(
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
    let vector = [Complex64::new(1.0, -1.0), Complex64::new(2.0, 1.0)];
    let (actual, _) = provider
        .spmv_complex64(matrix.as_ref(), &vector, SparseCancellation::never())
        .unwrap();
    assert_complex_close(
        &actual,
        &[Complex64::new(7.0, 0.0), Complex64::new(6.0, 3.0)],
    );

    let (squared, _) = provider
        .spgemm_complex64(
            matrix.as_ref(),
            matrix.as_ref(),
            SparseCancellation::never(),
        )
        .unwrap();
    let (_, _, offsets, rows, values) = squared.into_parts();
    assert_eq!(offsets, vec![0, 1, 3]);
    assert_eq!(rows, vec![0, 0, 1]);
    assert_complex_close(
        &values,
        &[
            Complex64::new(0.0, 2.0),
            Complex64::new(9.0, -2.0),
            Complex64::new(9.0, 0.0),
        ],
    );
}

#[test]
fn reusable_lu_solves_multiple_rhs_with_small_residuals() {
    let provider = ReferenceSparseProvider;
    let matrix = real_csc(
        3,
        3,
        &[0, 2, 4, 7],
        &[0, 2, 0, 1, 0, 1, 2],
        &[4.0, 2.0, 1.0, 3.0, 2.0, -1.0, 5.0],
    );
    let symbolic = provider
        .analyze_lu(matrix.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = provider
        .factor_lu_f64(&symbolic, matrix.as_ref(), SparseCancellation::never())
        .unwrap();
    let expected = [1.0, 2.0, -1.0, -2.0, 1.0, 3.0];
    let mut rhs = Vec::new();
    for solution in expected.chunks(3) {
        rhs.extend(
            provider
                .spmv_f64(matrix.as_ref(), solution, SparseCancellation::never())
                .unwrap()
                .0,
        );
    }
    let actual = factor.solve(&rhs, 2, SparseCancellation::never()).unwrap();
    assert_real_close(&actual.values, &expected);
    for (solution, expected_rhs) in actual.values.chunks(3).zip(rhs.chunks(3)) {
        let residual = provider
            .spmv_f64(matrix.as_ref(), solution, SparseCancellation::never())
            .unwrap()
            .0;
        assert_real_close(&residual, expected_rhs);
    }
    assert!(factor.metrics().numeric_work > 0);
}

#[test]
fn lu_reports_singular_matrix_and_symbolic_pattern_mismatch() {
    let provider = ReferenceSparseProvider;
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
fn complex_lu_solution_has_small_residual() {
    let provider = ReferenceSparseProvider;
    let matrix = complex_csc(
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
    let expected = [Complex64::new(1.0, 2.0), Complex64::new(-1.0, 0.5)];
    let rhs = provider
        .spmv_complex64(matrix.as_ref(), &expected, SparseCancellation::never())
        .unwrap()
        .0;
    let symbolic = provider
        .analyze_lu(matrix.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = provider
        .factor_lu_complex64(&symbolic, matrix.as_ref(), SparseCancellation::never())
        .unwrap();
    let actual = factor.solve(&rhs, 1, SparseCancellation::never()).unwrap();
    assert_complex_close(&actual.values, &expected);
    let residual = provider
        .spmv_complex64(matrix.as_ref(), &actual.values, SparseCancellation::never())
        .unwrap()
        .0;
    assert_complex_close(&residual, &rhs);
}

#[test]
fn real_and_complex_cholesky_solutions_have_small_residuals() {
    let provider = ReferenceSparseProvider;
    let real = real_csc(2, 2, &[0, 2, 4], &[0, 1, 0, 1], &[4.0, 1.0, 1.0, 3.0]);
    let symbolic = provider
        .analyze_cholesky(real.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = provider
        .factor_cholesky_f64(&symbolic, real.as_ref(), SparseCancellation::never())
        .unwrap();
    let solved = factor
        .solve(&[6.0, 7.0], 1, SparseCancellation::never())
        .unwrap();
    assert_real_close(&solved.values, &[1.0, 2.0]);

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
    let rhs = provider
        .spmv_complex64(complex.as_ref(), &expected, SparseCancellation::never())
        .unwrap()
        .0;
    let symbolic = provider
        .analyze_cholesky(complex.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    let factor = provider
        .factor_cholesky_complex64(&symbolic, complex.as_ref(), SparseCancellation::never())
        .unwrap();
    let actual = factor.solve(&rhs, 1, SparseCancellation::never()).unwrap();
    assert_complex_close(&actual.values, &expected);
    let residual = provider
        .spmv_complex64(
            complex.as_ref(),
            &actual.values,
            SparseCancellation::never(),
        )
        .unwrap()
        .0;
    assert_complex_close(&residual, &rhs);
}

#[test]
fn cholesky_rejects_non_positive_definite_input() {
    let provider = ReferenceSparseProvider;
    let matrix = real_csc(2, 2, &[0, 2, 4], &[0, 1, 0, 1], &[1.0, 2.0, 2.0, 1.0]);
    let symbolic = provider
        .analyze_cholesky(matrix.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    assert!(matches!(
        provider.factor_cholesky_f64(&symbolic, matrix.as_ref(), SparseCancellation::never()),
        Err(SparseError::NotPositiveDefinite { minor: 2, .. })
    ));

    let asymmetric = real_csc(2, 2, &[0, 1, 3], &[0, 0, 1], &[2.0, 1.0, 2.0]);
    let symbolic = provider
        .analyze_cholesky(asymmetric.as_ref().pattern(), SparseCancellation::never())
        .unwrap();
    assert!(matches!(
        provider.factor_cholesky_f64(&symbolic, asymmetric.as_ref(), SparseCancellation::never()),
        Err(SparseError::NotHermitian {
            row: 2,
            column: 1,
            ..
        })
    ));
}

#[test]
fn tall_least_squares_solves_and_reports_rank_deficiency() {
    let provider = ReferenceSparseProvider;
    let matrix = real_csc(
        3,
        2,
        &[0, 3, 5],
        &[0, 1, 2, 1, 2],
        &[1.0, 1.0, 1.0, 1.0, 2.0],
    );
    let solved = provider
        .least_squares_f64(
            matrix.as_ref(),
            &[2.0, 5.0, 8.0],
            1,
            SparseCancellation::never(),
        )
        .unwrap();
    assert_real_close(&solved.values, &[2.0, 3.0]);
    let fitted = provider
        .spmv_f64(matrix.as_ref(), &solved.values, SparseCancellation::never())
        .unwrap()
        .0;
    assert_real_close(&fitted, &[2.0, 5.0, 8.0]);

    let deficient = real_csc(
        3,
        2,
        &[0, 3, 6],
        &[0, 1, 2, 0, 1, 2],
        &[1.0, 2.0, 3.0, 1.0, 2.0, 3.0],
    );
    assert_eq!(
        provider.least_squares_f64(
            deficient.as_ref(),
            &[1.0, 2.0, 3.0],
            1,
            SparseCancellation::never()
        ),
        Err(SparseError::RankDeficient {
            provider: "openmat-reference-sparse",
            rank: 1,
            required_rank: 2,
        })
    );

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
    let rhs = provider
        .spmv_complex64(complex.as_ref(), &expected, SparseCancellation::never())
        .unwrap()
        .0;
    let solved = provider
        .least_squares_complex64(complex.as_ref(), &rhs, 1, SparseCancellation::never())
        .unwrap();
    assert_complex_close(&solved.values, &expected);
}

#[test]
fn underdetermined_least_squares_is_an_explicit_boundary() {
    let provider = ReferenceSparseProvider;
    let matrix = real_csc(1, 2, &[0, 1, 2], &[0, 0], &[1.0, 2.0]);
    assert_eq!(
        provider.least_squares_f64(matrix.as_ref(), &[1.0], 1, SparseCancellation::never()),
        Err(SparseError::Unsupported {
            provider: "openmat-reference-sparse",
            operation: "underdetermined sparse least squares",
        })
    );

    let basic = real_csc(2, 3, &[0, 1, 2, 4], &[0, 1, 0, 1], &[1.0, 1.0, 1.0, 1.0]);
    let solved = provider
        .underdetermined_basic_f64(
            basic.as_ref(),
            &[1.0, 2.0, 2.0, 3.0],
            2,
            SparseCancellation::never(),
        )
        .unwrap();
    assert_eq!((solved.rows, solved.columns), (3, 2));
    assert_real_close(&solved.values, &[1.0, 2.0, 0.0, 2.0, 3.0, 0.0]);
}

#[test]
fn cancellation_is_observed_and_metrics_are_deterministic() {
    let provider = ReferenceSparseProvider;
    let matrix = real_csc(2, 2, &[0, 1, 2], &[0, 1], &[2.0, 3.0]);
    let flag = AtomicBool::new(true);
    assert!(matches!(
        provider.spmv_f64(
            matrix.as_ref(),
            &[1.0, 1.0],
            SparseCancellation::from_atomic(&flag)
        ),
        Err(SparseError::Cancelled { .. })
    ));
    let first = provider
        .spmv_f64(matrix.as_ref(), &[1.0, 1.0], SparseCancellation::never())
        .unwrap()
        .1;
    let second = provider
        .spmv_f64(matrix.as_ref(), &[1.0, 1.0], SparseCancellation::never())
        .unwrap()
        .1;
    assert_eq!(first, second);
}

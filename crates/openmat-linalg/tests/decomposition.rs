use std::sync::atomic::AtomicBool;

use openmat_array::{Complex32, Complex64, DenseArray, Shape};
use openmat_linalg::{
    CholeskyRequest, CholeskyTriangle, FactorRequest, GemmOptions, GemmRequest, LinalgError,
    LinalgProvider, Lp64FactorDimensions, MatrixTranspose, QrRequest, QrVectors, ReferenceProvider,
    SwapParity, cholesky_complex32, cholesky_complex64, cholesky_f32, cholesky_f64,
    factor_lu_complex32, factor_lu_complex64, factor_lu_f32, factor_lu_f64,
    matrix_multiply_complex32, matrix_multiply_complex64, matrix_multiply_f64, qr_complex32,
    qr_complex64, qr_f32, qr_f64,
};

fn array<T>(shape: [u64; 2], values: Vec<T>) -> DenseArray<T> {
    DenseArray::from_vec(Shape::new(shape).unwrap(), values).unwrap()
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = 2.0e-11 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, received {actual}"
    );
}

fn assert_close32(actual: f32, expected: f32) {
    let tolerance = 5.0e-5_f32 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, received {actual}"
    );
}

fn assert_f64_array(actual: &DenseArray<f64>, expected: &DenseArray<f64>) {
    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_close(actual, expected);
    }
}

fn assert_f32_array(actual: &DenseArray<f32>, expected: &DenseArray<f32>) {
    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_close32(actual, expected);
    }
}

fn assert_complex64_array(actual: &DenseArray<Complex64>, expected: &DenseArray<Complex64>) {
    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_close(actual.re, expected.re);
        assert_close(actual.im, expected.im);
    }
}

fn assert_complex32_array(actual: &DenseArray<Complex32>, expected: &DenseArray<Complex32>) {
    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_close32(actual.re, expected.re);
        assert_close32(actual.im, expected.im);
    }
}

#[test]
fn reference_rectangular_lu_reconstructs_real_single_and_double_with_zero_based_rows() {
    let double = array([3, 2], vec![0.0_f64, 2.0, 4.0, 1.0, 3.0, 5.0]);
    let alias = double.clone();
    let result = factor_lu_f64(&ReferenceProvider, FactorRequest::new(&double)).unwrap();
    assert_eq!(result.row_permutation_zero_based, vec![2, 0, 1]);
    assert_eq!(result.swap_parity, SwapParity::Even);
    assert_eq!(result.first_zero_pivot, None);
    assert!(double.shares_storage_with(&alias));
    assert!(!result.packed_lu.shares_storage_with(&double));

    let packed = result.packed_lu.as_slice();
    let l = array([3, 2], vec![1.0, packed[1], packed[2], 0.0, 1.0, packed[5]]);
    let u = array([2, 2], vec![packed[0], 0.0, packed[3], packed[4]]);
    let reconstructed = matrix_multiply_f64(&ReferenceProvider, &l, &u).unwrap();
    let permuted = array(
        [3, 2],
        vec![
            double.as_slice()[2],
            double.as_slice()[0],
            double.as_slice()[1],
            double.as_slice()[5],
            double.as_slice()[3],
            double.as_slice()[4],
        ],
    );
    assert_f64_array(&reconstructed, &permuted);

    let single = array([2, 3], vec![0.0_f32, 2.0, 1.0, 3.0, 4.0, 5.0]);
    let result = factor_lu_f32(&ReferenceProvider, FactorRequest::new(&single)).unwrap();
    assert_eq!(result.packed_lu.shape().dimensions(), &[2, 3]);
    assert_eq!(result.row_permutation_zero_based, vec![1, 0]);
    assert_eq!(result.swap_parity, SwapParity::Odd);
    assert_eq!(result.first_zero_pivot, None);
}

#[test]
fn reference_lu_complex_precisions_and_zero_pivot_are_data() {
    let single = array(
        [2, 2],
        vec![
            Complex32::ZERO,
            Complex32::new(2.0, 1.0),
            Complex32::new(1.0, -1.0),
            Complex32::new(3.0, 0.5),
        ],
    );
    let single_result =
        factor_lu_complex32(&ReferenceProvider, FactorRequest::new(&single)).unwrap();
    assert_eq!(single_result.row_permutation_zero_based, vec![1, 0]);
    assert_eq!(single_result.first_zero_pivot, None);

    let singular = array(
        [2, 2],
        vec![
            Complex64::new(1.0, 1.0),
            Complex64::new(2.0, 2.0),
            Complex64::new(2.0, 0.0),
            Complex64::new(4.0, 0.0),
        ],
    );
    let result = factor_lu_complex64(&ReferenceProvider, FactorRequest::new(&singular)).unwrap();
    assert_eq!(result.first_zero_pivot, Some(1));
    assert_eq!(result.packed_lu.shape(), singular.shape());
}

#[test]
fn reference_qr_reconstructs_thin_tall_and_full_pivoted_wide_matrices() {
    let tall = array([3, 2], vec![1.0_f64, 1.0, 1.0, 0.0, 1.0, 2.0]);
    let result = qr_f64(&ReferenceProvider, QrRequest::new(&tall, QrVectors::Thin)).unwrap();
    assert_eq!(result.q.shape().dimensions(), &[3, 2]);
    assert_eq!(result.r.shape().dimensions(), &[2, 2]);
    assert_eq!(result.column_permutation_zero_based, vec![0, 1]);
    assert_eq!(result.numerical_rank, 2);
    assert!(!result.q.shares_storage_with(&tall));
    assert!(!result.r.shares_storage_with(&tall));
    assert_f64_array(
        &matrix_multiply_f64(&ReferenceProvider, &result.q, &result.r).unwrap(),
        &tall,
    );
    let mut gram = array([2, 2], vec![0.0_f64; 4]);
    ReferenceProvider
        .gemm_f64(
            GemmRequest::new(
                &result.q,
                &result.q,
                GemmOptions::new(MatrixTranspose::Transpose, MatrixTranspose::None, 1.0, 0.0),
            ),
            &mut gram,
        )
        .unwrap();
    assert_f64_array(&gram, &array([2, 2], vec![1.0, 0.0, 0.0, 1.0]));

    let wide = array(
        [2, 3],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::ZERO,
            Complex32::new(8.0, 0.0),
            Complex32::new(1.0, -2.0),
            Complex32::new(0.5, 0.0),
            Complex32::new(-1.0, 1.0),
        ],
    );
    let result = qr_complex32(
        &ReferenceProvider,
        QrRequest::new(&wide, QrVectors::Full).with_column_pivoting(true),
    )
    .unwrap();
    assert_eq!(result.q.shape().dimensions(), &[2, 2]);
    assert_eq!(result.r.shape().dimensions(), &[2, 3]);
    assert_eq!(result.column_permutation_zero_based[0], 1);
    let permutation = &result.column_permutation_zero_based;
    let permuted = array(
        [2, 3],
        permutation
            .iter()
            .flat_map(|&column| {
                let start = usize::try_from(column).unwrap() * 2;
                wide.as_slice()[start..start + 2].iter().copied()
            })
            .collect(),
    );
    assert_complex32_array(
        &matrix_multiply_complex32(&ReferenceProvider, &result.q, &result.r).unwrap(),
        &permuted,
    );
}

#[test]
fn reference_qr_uses_native_precision_rank_policy_and_complex64_unitarity() {
    let single = array([2, 2], vec![1.0_f32, 0.0, 0.0, 1.0e-8]);
    let double = array([2, 2], vec![1.0_f64, 0.0, 0.0, 1.0e-8]);
    let single_result =
        qr_f32(&ReferenceProvider, QrRequest::new(&single, QrVectors::Thin)).unwrap();
    assert_eq!(single_result.numerical_rank, 1);
    assert_eq!(single_result.first_rank_deficient_diagonal, Some(1));
    assert_eq!(
        qr_f64(&ReferenceProvider, QrRequest::new(&double, QrVectors::Thin))
            .unwrap()
            .numerical_rank,
        2
    );

    let complex = array(
        [2, 2],
        vec![
            Complex64::new(1.0, 1.0),
            Complex64::new(2.0, -1.0),
            Complex64::new(0.5, 3.0),
            Complex64::new(-1.0, 0.5),
        ],
    );
    let result = qr_complex64(
        &ReferenceProvider,
        QrRequest::new(&complex, QrVectors::Full),
    )
    .unwrap();
    assert_complex64_array(
        &matrix_multiply_complex64(&ReferenceProvider, &result.q, &result.r).unwrap(),
        &complex,
    );
    let mut gram = array([2, 2], vec![Complex64::ZERO; 4]);
    ReferenceProvider
        .gemm_complex64(
            GemmRequest::new(
                &result.q,
                &result.q,
                GemmOptions::new(
                    MatrixTranspose::ConjugateTranspose,
                    MatrixTranspose::None,
                    Complex64::new(1.0, 0.0),
                    Complex64::ZERO,
                ),
            ),
            &mut gram,
        )
        .unwrap();
    assert_complex64_array(
        &gram,
        &array(
            [2, 2],
            vec![
                Complex64::new(1.0, 0.0),
                Complex64::ZERO,
                Complex64::ZERO,
                Complex64::new(1.0, 0.0),
            ],
        ),
    );
}

#[test]
fn reference_cholesky_handles_triangles_precisions_and_non_positive_status() {
    let real = array([2, 2], vec![4.0_f64, 2.0, 2.0, 3.0]);
    let upper = cholesky_f64(
        &ReferenceProvider,
        CholeskyRequest::new(&real, CholeskyTriangle::Upper),
    )
    .unwrap();
    assert_eq!(upper.first_non_positive_minor, None);
    assert!(!upper.factor.shares_storage_with(&real));
    assert_close(upper.factor.as_slice()[0], 2.0);
    assert_close(upper.factor.as_slice()[2], 1.0);
    assert_close(upper.factor.as_slice()[3], 2.0_f64.sqrt());
    assert_close(upper.factor.as_slice()[1], 0.0);

    let single = array([2, 2], vec![4.0_f32, 2.0, 2.0, 3.0]);
    let lower = cholesky_f32(
        &ReferenceProvider,
        CholeskyRequest::new(&single, CholeskyTriangle::Lower),
    )
    .unwrap();
    assert_eq!(lower.first_non_positive_minor, None);
    assert_f32_array(
        &lower.factor,
        &array([2, 2], vec![2.0_f32, 1.0, 0.0, 2.0_f32.sqrt()]),
    );

    let indefinite = array([2, 2], vec![1.0_f64, 2.0, 2.0, 1.0]);
    let failed = cholesky_f64(
        &ReferenceProvider,
        CholeskyRequest::new(&indefinite, CholeskyTriangle::Upper),
    )
    .unwrap();
    assert_eq!(failed.first_non_positive_minor, Some(1));
    assert_eq!(failed.factor.as_slice(), &[1.0, 0.0, 0.0, 0.0]);
}

#[test]
fn reference_complex_cholesky_reconstructs_single_and_double() {
    let single = array(
        [2, 2],
        vec![
            Complex32::new(4.0, 0.0),
            Complex32::new(2.0, 2.0),
            Complex32::new(2.0, -2.0),
            Complex32::new(5.0, 0.0),
        ],
    );
    let lower = cholesky_complex32(
        &ReferenceProvider,
        CholeskyRequest::new(&single, CholeskyTriangle::Lower),
    )
    .unwrap();
    let mut reconstructed = array([2, 2], vec![Complex32::ZERO; 4]);
    ReferenceProvider
        .gemm_complex32(
            GemmRequest::new(
                &lower.factor,
                &lower.factor,
                GemmOptions::new(
                    MatrixTranspose::None,
                    MatrixTranspose::ConjugateTranspose,
                    Complex32::new(1.0, 0.0),
                    Complex32::ZERO,
                ),
            ),
            &mut reconstructed,
        )
        .unwrap();
    assert_complex32_array(&reconstructed, &single);

    let double = array(
        [2, 2],
        single
            .as_slice()
            .iter()
            .map(|value| Complex64::new(f64::from(value.re), f64::from(value.im)))
            .collect(),
    );
    let upper = cholesky_complex64(
        &ReferenceProvider,
        CholeskyRequest::new(&double, CholeskyTriangle::Upper),
    )
    .unwrap();
    let mut reconstructed = array([2, 2], vec![Complex64::ZERO; 4]);
    ReferenceProvider
        .gemm_complex64(
            GemmRequest::new(
                &upper.factor,
                &upper.factor,
                GemmOptions::new(
                    MatrixTranspose::ConjugateTranspose,
                    MatrixTranspose::None,
                    Complex64::new(1.0, 0.0),
                    Complex64::ZERO,
                ),
            ),
            &mut reconstructed,
        )
        .unwrap();
    assert_complex64_array(&reconstructed, &double);
}

#[test]
fn decomposition_empty_shapes_and_cancellation_are_structured() {
    let empty = array::<f64>([0, 3], vec![]);
    let lu = factor_lu_f64(&ReferenceProvider, FactorRequest::new(&empty)).unwrap();
    assert_eq!(lu.packed_lu.shape().dimensions(), &[0, 3]);
    assert!(lu.row_permutation_zero_based.is_empty());

    let qr = qr_f64(
        &ReferenceProvider,
        QrRequest::new(&empty, QrVectors::Full).with_column_pivoting(true),
    )
    .unwrap();
    assert_eq!(qr.q.shape().dimensions(), &[0, 0]);
    assert_eq!(qr.r.shape().dimensions(), &[0, 3]);
    assert_eq!(qr.column_permutation_zero_based, vec![0, 1, 2]);
    assert_eq!(qr.numerical_rank, 0);
    assert_eq!(qr.first_rank_deficient_diagonal, None);

    let zero_columns = array::<f64>([3, 0], vec![]);
    let qr = qr_f64(
        &ReferenceProvider,
        QrRequest::new(&zero_columns, QrVectors::Full),
    )
    .unwrap();
    assert_f64_array(
        &qr.q,
        &array([3, 3], vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
    );
    assert_eq!(qr.r.shape().dimensions(), &[3, 0]);

    let chol = cholesky_f64(
        &ReferenceProvider,
        CholeskyRequest::new(&array::<f64>([0, 0], vec![]), CholeskyTriangle::Upper),
    )
    .unwrap();
    assert_eq!(chol.factor.shape().dimensions(), &[0, 0]);
    assert_eq!(chol.first_non_positive_minor, None);

    let cancellation = AtomicBool::new(true);
    let matrix = array([1, 1], vec![1.0_f64]);
    assert_eq!(
        ReferenceProvider
            .factor_lu_f64(FactorRequest::new(&matrix).with_cancellation_flag(&cancellation))
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "LU factorization"
        }
    );
    assert_eq!(
        ReferenceProvider
            .qr_f64(QrRequest::new(&matrix, QrVectors::Full).with_cancellation_flag(&cancellation))
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "QR factorization"
        }
    );
    assert_eq!(
        ReferenceProvider
            .cholesky_f64(
                CholeskyRequest::new(&matrix, CholeskyTriangle::Upper)
                    .with_cancellation_flag(&cancellation)
            )
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "Cholesky factorization"
        }
    );
}

#[test]
fn decomposition_lp64_conversion_precedes_empty_native_shortcuts() {
    let oversized = openmat_linalg::LP64_MAX_DIMENSION + 1;
    let matrix = array::<f64>([oversized, 0], vec![]);
    let dimensions = openmat_linalg::validate_factor(&FactorRequest::new(&matrix)).unwrap();
    assert!(matches!(
        Lp64FactorDimensions::try_from(&dimensions),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "m",
            value,
        }) if value == oversized
    ));
}

#[test]
fn decomposition_shape_and_fallible_allocation_errors_remain_structured() {
    let higher_rank =
        DenseArray::from_vec(Shape::new([1, 1, 2]).unwrap(), vec![1.0_f64, 2.0]).unwrap();
    assert!(matches!(
        factor_lu_f64(&ReferenceProvider, FactorRequest::new(&higher_rank)),
        Err(LinalgError::MatrixRequired {
            operand: "LU factorization matrix",
            ..
        })
    ));
    assert!(matches!(
        cholesky_f64(
            &ReferenceProvider,
            CholeskyRequest::new(&array([2, 3], vec![0.0_f64; 6]), CholeskyTriangle::Upper,),
        ),
        Err(LinalgError::SquareMatrixRequired {
            operand: "Cholesky factorization matrix",
            rows: 2,
            columns: 3,
        })
    ));

    let enormous_empty = array::<f64>([u64::MAX, 0], vec![]);
    assert!(matches!(
        factor_lu_f64(&ReferenceProvider, FactorRequest::new(&enormous_empty)),
        Err(LinalgError::AllocationFailure {
            operation: "reference LU row permutation",
            elements: u64::MAX,
        })
    ));
}

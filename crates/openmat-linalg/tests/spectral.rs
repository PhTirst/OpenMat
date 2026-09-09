use std::sync::atomic::AtomicBool;

use openmat_array::{Complex32, Complex64, DenseArray, Shape};
use openmat_linalg::{
    EigRequest, LinalgError, LinalgProvider, Lp64SpectralDimensions, ReferenceProvider, SvdRequest,
    SvdVectors, validate_eig, validate_svd,
};

fn array_f64(rows: u64, columns: u64, values: Vec<f64>) -> DenseArray<f64> {
    DenseArray::from_vec(Shape::new([rows, columns]).unwrap(), values).unwrap()
}

fn array_f32(rows: u64, columns: u64, values: Vec<f32>) -> DenseArray<f32> {
    DenseArray::from_vec(Shape::new([rows, columns]).unwrap(), values).unwrap()
}

fn array_c64(rows: u64, columns: u64, values: Vec<Complex64>) -> DenseArray<Complex64> {
    DenseArray::from_vec(Shape::new([rows, columns]).unwrap(), values).unwrap()
}

fn array_c32(rows: u64, columns: u64, values: Vec<Complex32>) -> DenseArray<Complex32> {
    DenseArray::from_vec(Shape::new([rows, columns]).unwrap(), values).unwrap()
}

fn reconstruct_f64(u: &DenseArray<f64>, s: &DenseArray<f64>, vh: &DenseArray<f64>) -> Vec<f64> {
    let rows = usize::try_from(u.shape().extent(0)).unwrap();
    let order = usize::try_from(s.shape().extent(0)).unwrap();
    let columns = usize::try_from(vh.shape().extent(1)).unwrap();
    let mut result = vec![0.0; rows * columns];
    for column in 0..columns {
        for row in 0..rows {
            for inner in 0..order {
                result[column * rows + row] += u.as_slice()[inner * rows + row]
                    * s.as_slice()[inner]
                    * vh.as_slice()[column * order + inner];
            }
        }
    }
    result
}

fn multiply_c64(left: Complex64, right: Complex64) -> Complex64 {
    Complex64::new(
        left.re * right.re - left.im * right.im,
        left.re * right.im + left.im * right.re,
    )
}

fn subtract_c64(left: Complex64, right: Complex64) -> Complex64 {
    Complex64::new(left.re - right.re, left.im - right.im)
}

fn eig_residual_c64(
    matrix: &DenseArray<Complex64>,
    values: &DenseArray<Complex64>,
    vectors: &DenseArray<Complex64>,
) -> f64 {
    let order = usize::try_from(matrix.shape().extent(0)).unwrap();
    let mut maximum: f64 = 0.0;
    for column in 0..order {
        for row in 0..order {
            let mut product = Complex64::ZERO;
            for inner in 0..order {
                let term = multiply_c64(
                    matrix.as_slice()[inner * order + row],
                    vectors.as_slice()[column * order + inner],
                );
                product = Complex64::new(product.re + term.re, product.im + term.im);
            }
            let expected = multiply_c64(
                values.as_slice()[column],
                vectors.as_slice()[column * order + row],
            );
            let difference = subtract_c64(product, expected);
            maximum = maximum.max(difference.re.hypot(difference.im));
        }
    }
    maximum
}

fn left_eig_residual_c64(
    matrix: &DenseArray<Complex64>,
    values: &DenseArray<Complex64>,
    vectors: &DenseArray<Complex64>,
) -> f64 {
    let order = usize::try_from(matrix.shape().extent(0)).unwrap();
    let mut maximum: f64 = 0.0;
    for column in 0..order {
        for row in 0..order {
            let mut product = Complex64::ZERO;
            for inner in 0..order {
                let term = multiply_c64(
                    matrix.as_slice()[row * order + inner].conjugate(),
                    vectors.as_slice()[column * order + inner],
                );
                product = Complex64::new(product.re + term.re, product.im + term.im);
            }
            let expected = multiply_c64(
                values.as_slice()[column].conjugate(),
                vectors.as_slice()[column * order + row],
            );
            let difference = subtract_c64(product, expected);
            maximum = maximum.max(difference.re.hypot(difference.im));
        }
    }
    maximum
}

fn gram_error_c64(matrix: &DenseArray<Complex64>) -> f64 {
    let rows = usize::try_from(matrix.shape().extent(0)).unwrap();
    let columns = usize::try_from(matrix.shape().extent(1)).unwrap();
    let mut maximum: f64 = 0.0;
    for left in 0..columns {
        for right in 0..columns {
            let mut product = Complex64::ZERO;
            for row in 0..rows {
                let term = multiply_c64(
                    matrix.as_slice()[left * rows + row].conjugate(),
                    matrix.as_slice()[right * rows + row],
                );
                product = Complex64::new(product.re + term.re, product.im + term.im);
            }
            let expected = if left == right { 1.0 } else { 0.0 };
            maximum = maximum.max((product.re - expected).hypot(product.im));
        }
    }
    maximum
}

#[test]
fn reference_real_svd_reconstructs_tall_thin_and_preserves_cow() {
    let matrix = array_f64(3, 2, vec![3.0, 0.0, 0.0, 1.0, 2.0, 2.0]);
    let shared = matrix.clone();
    let result = ReferenceProvider
        .svd_f64(SvdRequest::new(&matrix, SvdVectors::Thin))
        .unwrap();
    assert_eq!(result.singular_values.shape().dimensions(), &[2, 1]);
    assert_eq!(result.u.as_ref().unwrap().shape().dimensions(), &[3, 2]);
    assert_eq!(result.vh.as_ref().unwrap().shape().dimensions(), &[2, 2]);
    assert!(result.singular_values.as_slice()[0] >= result.singular_values.as_slice()[1]);
    for (actual, expected) in reconstruct_f64(
        result.u.as_ref().unwrap(),
        &result.singular_values,
        result.vh.as_ref().unwrap(),
    )
    .iter()
    .zip(matrix.as_slice())
    {
        assert!(
            (actual - expected).abs() < 1.0e-10,
            "{actual} != {expected}"
        );
    }
    assert!(matrix.shares_storage_with(&shared));
}

#[test]
fn reference_real_single_svd_is_native_and_handles_wide_full() {
    let matrix = array_f32(2, 3, vec![1.0, 0.0, 0.0, 2.0, 2.0, 1.0]);
    let result = ReferenceProvider
        .svd_f32(SvdRequest::new(&matrix, SvdVectors::Full))
        .unwrap();
    assert_eq!(result.u.unwrap().shape().dimensions(), &[2, 2]);
    assert_eq!(result.vh.unwrap().shape().dimensions(), &[3, 3]);
    assert_eq!(result.singular_values.shape().dimensions(), &[2, 1]);
}

#[test]
fn reference_complex_svds_cover_both_precisions_and_values_only() {
    let double = array_c64(
        2,
        2,
        vec![
            Complex64::new(1.0, 1.0),
            Complex64::new(0.0, 0.0),
            Complex64::new(2.0, -1.0),
            Complex64::new(1.0, 0.0),
        ],
    );
    let single = array_c32(
        2,
        2,
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::new(0.0, 0.0),
            Complex32::new(2.0, -1.0),
            Complex32::new(1.0, 0.0),
        ],
    );
    let double_result = ReferenceProvider
        .svd_complex64(SvdRequest::new(&double, SvdVectors::Thin))
        .unwrap();
    assert!(
        double_result.singular_values.as_slice()[0] >= double_result.singular_values.as_slice()[1]
    );
    assert!(gram_error_c64(double_result.u.as_ref().unwrap()) < 1.0e-10);
    let vh = double_result.vh.as_ref().unwrap();
    let v = array_c64(
        2,
        2,
        (0..4)
            .map(|index| {
                let row = index % 2;
                let column = index / 2;
                vh.as_slice()[row * 2 + column].conjugate()
            })
            .collect(),
    );
    assert!(gram_error_c64(&v) < 1.0e-10);
    let single_result = ReferenceProvider
        .svd_complex32(SvdRequest::new(&single, SvdVectors::None))
        .unwrap();
    assert!(single_result.u.is_none());
    assert!(single_result.vh.is_none());
}

#[test]
fn reference_svd_empty_shapes_are_exact() {
    let empty = array_f64(0, 3, Vec::new());
    let thin = ReferenceProvider
        .svd_f64(SvdRequest::new(&empty, SvdVectors::Thin))
        .unwrap();
    assert_eq!(thin.singular_values.shape().dimensions(), &[0, 1]);
    assert_eq!(thin.u.unwrap().shape().dimensions(), &[0, 0]);
    assert_eq!(thin.vh.unwrap().shape().dimensions(), &[0, 3]);
    let full = ReferenceProvider
        .svd_f64(SvdRequest::new(&empty, SvdVectors::Full))
        .unwrap();
    assert_eq!(full.u.unwrap().shape().dimensions(), &[0, 0]);
    assert_eq!(full.vh.unwrap().shape().dimensions(), &[3, 3]);
}

#[test]
fn reference_real_eig_normalizes_conjugate_pair_and_right_residuals() {
    let real = array_f64(2, 2, vec![0.0, 1.0, -1.0, 0.0]);
    let result = ReferenceProvider
        .eig_f64(EigRequest::new(&real).with_right_vectors(true))
        .unwrap();
    assert!((result.eigenvalues.as_slice()[0].re).abs() < 1.0e-10);
    assert!((result.eigenvalues.as_slice()[0].im.abs() - 1.0).abs() < 1.0e-9);
    assert!(
        (result.eigenvalues.as_slice()[1].im + result.eigenvalues.as_slice()[0].im).abs() < 1.0e-9
    );
    let complex = array_c64(
        2,
        2,
        real.as_slice()
            .iter()
            .map(|&value| Complex64::new(value, 0.0))
            .collect(),
    );
    assert!(
        eig_residual_c64(
            &complex,
            &result.eigenvalues,
            result.right_vectors.as_ref().unwrap()
        ) < 1.0e-8
    );
}

#[test]
fn reference_complex_nonnormal_eig_returns_left_and_right_vectors() {
    let matrix = array_c64(
        3,
        3,
        vec![
            Complex64::new(1.0, 1.0),
            Complex64::new(-1.0, 0.5),
            Complex64::new(0.0, 0.5),
            Complex64::new(2.0, -0.5),
            Complex64::new(2.0, -1.0),
            Complex64::new(-2.0, 0.0),
            Complex64::new(0.5, 0.0),
            Complex64::new(1.0, 1.0),
            Complex64::new(3.0, 0.5),
        ],
    );
    let result = ReferenceProvider
        .eig_complex64(
            EigRequest::new(&matrix)
                .with_left_vectors(true)
                .with_right_vectors(true),
        )
        .unwrap();
    assert!(
        eig_residual_c64(
            &matrix,
            &result.eigenvalues,
            result.right_vectors.as_ref().unwrap()
        ) < 1.0e-8
    );
    assert!(
        left_eig_residual_c64(
            &matrix,
            &result.eigenvalues,
            result.left_vectors.as_ref().unwrap()
        ) < 1.0e-8
    );
}

#[test]
fn reference_single_eig_methods_remain_binary32() {
    let real = array_f32(2, 2, vec![2.0, 0.0, 1.0, 3.0]);
    let complex = array_c32(
        2,
        2,
        vec![
            Complex32::new(2.0, 0.0),
            Complex32::ZERO,
            Complex32::new(1.0, 1.0),
            Complex32::new(3.0, 0.0),
        ],
    );
    assert_eq!(
        ReferenceProvider
            .eig_f32(EigRequest::new(&real))
            .unwrap()
            .eigenvalues
            .as_slice()
            .len(),
        2
    );
    assert_eq!(
        ReferenceProvider
            .eig_complex32(EigRequest::new(&complex))
            .unwrap()
            .eigenvalues
            .as_slice()
            .len(),
        2
    );
}

#[test]
fn reference_spectral_operations_observe_pre_cancelled_requests() {
    let matrix = array_f64(2, 2, vec![1.0, 0.0, 0.0, 1.0]);
    let cancellation = AtomicBool::new(true);
    assert!(matches!(
        ReferenceProvider.svd_f64(
            SvdRequest::new(&matrix, SvdVectors::Thin).with_cancellation_flag(&cancellation)
        ),
        Err(LinalgError::Cancelled { .. })
    ));
    assert!(matches!(
        ReferenceProvider.eig_f64(EigRequest::new(&matrix).with_cancellation_flag(&cancellation)),
        Err(LinalgError::Cancelled { .. })
    ));
}

#[test]
fn spectral_validation_and_lp64_conversion_are_checked_before_allocation() {
    let oversized = i32::MAX as u64 + 1;
    let empty = array_f64(oversized, 0, Vec::new());
    let dimensions = validate_svd(&SvdRequest::new(&empty, SvdVectors::None)).unwrap();
    assert!(matches!(
        Lp64SpectralDimensions::try_from(&dimensions),
        Err(LinalgError::Lp64DimensionOverflow { parameter: "m", value }) if value == oversized
    ));

    let rectangular = array_f64(2, 3, vec![0.0; 6]);
    assert!(matches!(
        validate_eig(&EigRequest::new(&rectangular)),
        Err(LinalgError::SquareMatrixRequired {
            rows: 2,
            columns: 3,
            ..
        })
    ));
}

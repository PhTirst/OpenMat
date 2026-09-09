#![cfg(windows)]

use std::{path::PathBuf, sync::atomic::AtomicBool};

use openmat_array::{Complex32, Complex64, DenseArray, Shape};
use openmat_linalg::{
    CholeskyRequest, CholeskyTriangle, EigRequest, FactorRequest, GemmOptions, GemmRequest,
    LP64_MAX_DIMENSION, LinalgError, LinalgProvider, MatrixTranspose, ProviderIntegerAbi,
    QrRequest, QrVectors, RectangularSolveRequest, ReferenceProvider, SchurRequest, SolveRequest,
    SvdRequest, SvdVectors, matrix_multiply_complex32, matrix_multiply_complex64,
    matrix_multiply_f32, matrix_multiply_f64, solve_complex32, solve_complex64, solve_f32,
    solve_f64, solve_rectangular_complex32, solve_rectangular_complex64, solve_rectangular_f32,
    solve_rectangular_f64,
};
use openmat_openblas::{OpenBlasError, OpenBlasIntegerAbi, OpenBlasProvider};

const TEST_DLL_ENV: &str = "OPENMAT_TEST_OPENBLAS_DLL";

fn array<T>(dimensions: [u64; 2], data: Vec<T>) -> DenseArray<T> {
    DenseArray::from_vec(Shape::new(dimensions).unwrap(), data).unwrap()
}

fn configured_provider() -> Option<OpenBlasProvider> {
    let Some(path) = std::env::var_os(TEST_DLL_ENV) else {
        eprintln!("skipping real OpenBLAS test: {TEST_DLL_ENV} is not set");
        return None;
    };
    let provider = OpenBlasProvider::load(PathBuf::from(path))
        .unwrap_or_else(|error| panic!("{TEST_DLL_ENV} names an unusable DLL: {error}"));
    eprintln!(
        "OpenBLAS test provider: path={}, version={}, core={}, ABI={}, config={}",
        provider.info().path().display(),
        provider.info().version(),
        provider.info().core_name(),
        provider.info().integer_abi(),
        provider.info().config()
    );
    Some(provider)
}

fn assert_close(actual: f64, expected: f64) {
    if expected.is_nan() {
        assert!(actual.is_nan(), "expected NaN, received {actual}");
        return;
    }
    if expected.is_infinite() {
        assert!(actual.is_infinite());
        assert_eq!(actual.is_sign_positive(), expected.is_sign_positive());
        return;
    }
    let tolerance = 2.0e-12 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, received {actual}"
    );
}

fn assert_complex_close(actual: Complex64, expected: Complex64) {
    assert_close(actual.re, expected.re);
    assert_close(actual.im, expected.im);
}

fn assert_f64_arrays_close(actual: &DenseArray<f64>, expected: &DenseArray<f64>) {
    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_close(actual, expected);
    }
}

fn assert_complex_arrays_close(actual: &DenseArray<Complex64>, expected: &DenseArray<Complex64>) {
    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_complex_close(actual, expected);
    }
}

fn assert_close32(actual: f32, expected: f32) {
    let tolerance = 5.0e-5_f32 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, received {actual}"
    );
}

fn assert_complex_close32(actual: Complex32, expected: Complex32) {
    assert_close32(actual.re, expected.re);
    assert_close32(actual.im, expected.im);
}

fn assert_f32_arrays_close(actual: &DenseArray<f32>, expected: &DenseArray<f32>) {
    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_close32(actual, expected);
    }
}

fn assert_complex32_arrays_close(actual: &DenseArray<Complex32>, expected: &DenseArray<Complex32>) {
    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_complex_close32(actual, expected);
    }
}

#[test]
fn loading_requires_an_absolute_path() {
    assert!(matches!(
        OpenBlasProvider::load("libopenblas.dll"),
        Err(OpenBlasError::PathNotAbsolute { .. })
    ));
}

#[test]
fn nonexistent_absolute_path_is_a_typed_load_error() {
    let missing = std::env::temp_dir().join(format!(
        "openmat-openblas-missing-{}-{}.dll",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    assert!(matches!(
        OpenBlasProvider::load(&missing),
        Err(OpenBlasError::LibraryLoad { path, .. }) if path == missing
    ));
}

#[test]
fn non_openblas_dll_reports_the_missing_identity_symbol() {
    let Some(system_root) = std::env::var_os("SystemRoot") else {
        return;
    };
    let kernel32 = PathBuf::from(system_root)
        .join("System32")
        .join("kernel32.dll");
    assert!(matches!(
        OpenBlasProvider::load(&kernel32),
        Err(OpenBlasError::MissingSymbol {
            symbol: "openblas_get_config",
            ..
        })
    ));
}

#[test]
fn actual_provider_reports_lp64_identity() {
    let Some(provider) = configured_provider() else {
        return;
    };
    assert_eq!(provider.name(), "openblas");
    assert_eq!(provider.integer_abi(), ProviderIntegerAbi::Lp64);
    assert_eq!(provider.info().integer_abi(), OpenBlasIntegerAbi::Lp64);
    assert!(!provider.info().version().is_empty());
    assert!(!provider.info().core_name().is_empty());
}

#[test]
fn actual_f64_gemm_matches_reference_for_transpose_scalars_and_cow() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;
    let left = array([2, 3], vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    let right = array([1, 2], vec![10.0, 20.0]);
    let initial = array([3, 1], vec![1.0, -2.0, 3.0]);
    let mut actual = initial.clone();
    let mut expected = initial.deep_copy();
    let options = GemmOptions::new(
        MatrixTranspose::Transpose,
        MatrixTranspose::ConjugateTranspose,
        2.0,
        -0.5,
    );

    provider
        .gemm_f64(GemmRequest::new(&left, &right, options), &mut actual)
        .unwrap();
    reference
        .gemm_f64(GemmRequest::new(&left, &right, options), &mut expected)
        .unwrap();

    assert!(!actual.shares_storage_with(&initial));
    assert_eq!(initial.as_slice(), &[1.0, -2.0, 3.0]);
    assert_f64_arrays_close(&actual, &expected);
}

#[test]
fn actual_complex_gemm_matches_reference_with_explicit_packing() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;
    let left = array(
        [2, 2],
        vec![
            Complex64::new(1.0, 2.0),
            Complex64::new(-3.0, 0.5),
            Complex64::new(4.0, -1.0),
            Complex64::new(2.0, 3.0),
        ],
    );
    let right = array(
        [2, 2],
        vec![
            Complex64::new(0.5, -2.0),
            Complex64::new(1.0, 1.5),
            Complex64::new(-4.0, 1.0),
            Complex64::new(3.0, -0.25),
        ],
    );
    let initial = array(
        [2, 2],
        vec![
            Complex64::new(1.0, 0.0),
            Complex64::new(0.0, 1.0),
            Complex64::new(-1.0, 2.0),
            Complex64::new(3.0, -4.0),
        ],
    );
    let mut actual = initial.clone();
    let mut expected = initial.deep_copy();
    let options = GemmOptions::new(
        MatrixTranspose::ConjugateTranspose,
        MatrixTranspose::Transpose,
        Complex64::new(0.75, -0.5),
        Complex64::new(-0.25, 0.5),
    );

    provider
        .gemm_complex64(GemmRequest::new(&left, &right, options), &mut actual)
        .unwrap();
    reference
        .gemm_complex64(GemmRequest::new(&left, &right, options), &mut expected)
        .unwrap();

    assert!(!actual.shares_storage_with(&initial));
    assert_eq!(initial.as_slice()[0], Complex64::new(1.0, 0.0));
    assert_complex_arrays_close(&actual, &expected);
}

#[test]
fn actual_f32_gemm_matches_native_reference_for_transpose_scalars_and_cow() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;
    let left = array([2, 3], vec![1.0_f32, 4.0, 2.0, 5.0, 3.0, 6.0]);
    let right = array([1, 2], vec![10.0_f32, 20.0]);
    let initial = array([3, 1], vec![1.0_f32, -2.0, 3.0]);
    let mut actual = initial.clone();
    let mut expected = initial.deep_copy();
    let options = GemmOptions::new(
        MatrixTranspose::Transpose,
        MatrixTranspose::ConjugateTranspose,
        2.0_f32,
        -0.5_f32,
    );

    provider
        .gemm_f32(GemmRequest::new(&left, &right, options), &mut actual)
        .unwrap();
    reference
        .gemm_f32(GemmRequest::new(&left, &right, options), &mut expected)
        .unwrap();

    assert!(!actual.shares_storage_with(&initial));
    assert_eq!(initial.as_slice(), &[1.0, -2.0, 3.0]);
    assert_f32_arrays_close(&actual, &expected);
}

#[test]
fn actual_complex32_gemm_matches_reference_with_explicit_f32_pairs() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;
    let left = array(
        [2, 2],
        vec![
            Complex32::new(1.0, 2.0),
            Complex32::new(-3.0, 0.5),
            Complex32::new(4.0, -1.0),
            Complex32::new(2.0, 3.0),
        ],
    );
    let right = array(
        [2, 2],
        vec![
            Complex32::new(0.5, -2.0),
            Complex32::new(1.0, 1.5),
            Complex32::new(-4.0, 1.0),
            Complex32::new(3.0, -0.25),
        ],
    );
    let initial = array([2, 2], vec![Complex32::new(1.0, -1.0); 4]);
    let mut actual = initial.clone();
    let mut expected = initial.deep_copy();
    let options = GemmOptions::new(
        MatrixTranspose::ConjugateTranspose,
        MatrixTranspose::Transpose,
        Complex32::new(0.75, -0.5),
        Complex32::new(-0.25, 0.5),
    );

    provider
        .gemm_complex32(GemmRequest::new(&left, &right, options), &mut actual)
        .unwrap();
    reference
        .gemm_complex32(GemmRequest::new(&left, &right, options), &mut expected)
        .unwrap();

    assert!(!actual.shares_storage_with(&initial));
    assert_complex32_arrays_close(&actual, &expected);
}

#[test]
fn actual_empty_gemm_and_zero_inner_dimension_match_reference() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;

    let empty_left = array::<f64>([2, 0], vec![]);
    let empty_right = array::<f64>([0, 3], vec![]);
    let initial = array([2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let mut actual = initial.clone();
    let mut expected = initial.deep_copy();
    let options = GemmOptions::new(MatrixTranspose::None, MatrixTranspose::None, 7.0, -0.5);
    provider
        .gemm_f64(
            GemmRequest::new(&empty_left, &empty_right, options),
            &mut actual,
        )
        .unwrap();
    reference
        .gemm_f64(
            GemmRequest::new(&empty_left, &empty_right, options),
            &mut expected,
        )
        .unwrap();
    assert!(!actual.shares_storage_with(&initial));
    assert_f64_arrays_close(&actual, &expected);

    let zero_rows_left = array::<f64>([0, 2], vec![]);
    let right = array([2, 3], vec![1.0; 6]);
    let mut zero_rows_output = array::<f64>([0, 3], vec![]);
    provider
        .gemm_f64(
            GemmRequest::new(
                &zero_rows_left,
                &right,
                GemmOptions::new(MatrixTranspose::None, MatrixTranspose::None, 1.0, 0.0),
            ),
            &mut zero_rows_output,
        )
        .unwrap();
    assert!(zero_rows_output.is_empty());
}

#[test]
fn actual_dot_and_stable_norm_match_reference() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;
    let left = array([1, 4], vec![1.0e100, 2.0, -3.0, -1.0e100]);
    let right = array([4, 1], vec![1.0e-100, 4.0, 5.0, 1.0e-100]);
    assert_close(
        provider.dot_f64(&left, &right).unwrap(),
        reference.dot_f64(&left, &right).unwrap(),
    );

    for input in [
        array([1, 2], vec![3.0e200, 4.0e200]),
        array([1, 3], vec![f64::MIN_POSITIVE, 0.0, -f64::MIN_POSITIVE]),
        array([1, 2], vec![f64::INFINITY, f64::INFINITY]),
        array([1, 2], vec![f64::INFINITY, f64::NAN]),
        array::<f64>([0, 2], vec![]),
    ] {
        assert_close(
            provider.norm2_f64(&input).unwrap(),
            reference.norm2_f64(&input).unwrap(),
        );
    }

    let empty = array::<f64>([0, 2], vec![]);
    assert_close(provider.dot_f64(&empty, &empty).unwrap(), 0.0);
}

#[test]
fn actual_provider_checks_lp64_before_empty_gemm_shortcuts() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let oversized_rows = LP64_MAX_DIMENSION + 1;
    let left = array::<f64>([oversized_rows, 0], vec![]);
    let right = array::<f64>([0, 0], vec![]);
    let mut output = array::<f64>([oversized_rows, 0], vec![]);
    let error = provider
        .gemm_f64(
            GemmRequest::new(
                &left,
                &right,
                GemmOptions::new(MatrixTranspose::None, MatrixTranspose::None, 1.0, 0.0),
            ),
            &mut output,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        LinalgError::Lp64DimensionOverflow {
            parameter: "m",
            value,
        } if value == oversized_rows
    ));

    let left = array::<f32>([oversized_rows, 0], vec![]);
    let right = array::<f32>([0, 0], vec![]);
    let mut output = array::<f32>([oversized_rows, 0], vec![]);
    assert!(matches!(
        provider.gemm_f32(
            GemmRequest::new(
                &left,
                &right,
                GemmOptions::new(MatrixTranspose::None, MatrixTranspose::None, 1.0, 0.0),
            ),
            &mut output,
        ),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "m",
            value,
        }) if value == oversized_rows
    ));
}

#[test]
fn actual_f64_solve_matches_reference_with_pivoting_multiple_rhs_and_cow() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let coefficients = array([3, 3], vec![0.0, 1.0, 2.0, 2.0, -2.0, 3.0, 1.0, -3.0, 1.0]);
    let right_hand_side = array([3, 2], vec![3.0, 0.0, 7.0, -1.0, -1.0, 5.0]);
    let original_coefficients = coefficients.deep_copy();
    let original_right_hand_side = right_hand_side.deep_copy();

    let actual = solve_f64(&provider, &coefficients, &right_hand_side).unwrap();
    let expected = array([3, 2], vec![1.0, 2.0, -1.0, 4.0, -2.0, 3.0]);

    assert_f64_arrays_close(&actual, &expected);
    assert!(!actual.shares_storage_with(&coefficients));
    assert!(!actual.shares_storage_with(&right_hand_side));
    assert_eq!(coefficients, original_coefficients);
    assert_eq!(right_hand_side, original_right_hand_side);
}

#[test]
fn actual_complex_solve_matches_reference_with_explicit_packing() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;
    let coefficients = array(
        [2, 2],
        vec![
            Complex64::new(1.0, 1.0),
            Complex64::new(0.0, 3.0),
            Complex64::new(2.0, 0.0),
            Complex64::new(4.0, -1.0),
        ],
    );
    let expected = array(
        [2, 2],
        vec![
            Complex64::new(1.0, -2.0),
            Complex64::new(3.0, 0.5),
            Complex64::new(-1.0, 1.0),
            Complex64::new(2.0, -3.0),
        ],
    );
    let right_hand_side =
        openmat_linalg::matrix_multiply_complex64(&reference, &coefficients, &expected).unwrap();
    let original_coefficients = coefficients.deep_copy();
    let original_right_hand_side = right_hand_side.deep_copy();

    let actual = solve_complex64(&provider, &coefficients, &right_hand_side).unwrap();

    assert_complex_arrays_close(&actual, &expected);
    assert_eq!(coefficients, original_coefficients);
    assert_eq!(right_hand_side, original_right_hand_side);
    assert!(!actual.shares_storage_with(&coefficients));
    assert!(!actual.shares_storage_with(&right_hand_side));
}

#[test]
fn actual_f32_solve_matches_reference_with_multiple_rhs_and_cow() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let coefficients = array(
        [3, 3],
        vec![0.0_f32, 1.0, 2.0, 2.0, -2.0, 3.0, 1.0, -3.0, 1.0],
    );
    let right_hand_side = array([3, 2], vec![3.0_f32, 0.0, 7.0, -1.0, -1.0, 5.0]);
    let original_coefficients = coefficients.deep_copy();
    let original_right_hand_side = right_hand_side.deep_copy();

    let actual = solve_f32(&provider, &coefficients, &right_hand_side).unwrap();
    let expected = solve_f32(&ReferenceProvider, &coefficients, &right_hand_side).unwrap();

    assert_f32_arrays_close(&actual, &expected);
    assert_eq!(coefficients, original_coefficients);
    assert_eq!(right_hand_side, original_right_hand_side);
    assert!(!actual.shares_storage_with(&coefficients));
    assert!(!actual.shares_storage_with(&right_hand_side));
}

#[test]
fn actual_complex32_solve_matches_reference_with_explicit_f32_pairs() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let coefficients = array(
        [2, 2],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::new(0.0, 3.0),
            Complex32::new(2.0, 0.0),
            Complex32::new(4.0, -1.0),
        ],
    );
    let expected = array(
        [2, 2],
        vec![
            Complex32::new(1.0, -2.0),
            Complex32::new(3.0, 0.5),
            Complex32::new(-1.0, 1.0),
            Complex32::new(2.0, -3.0),
        ],
    );
    let right_hand_side =
        matrix_multiply_complex32(&ReferenceProvider, &coefficients, &expected).unwrap();

    let actual = solve_complex32(&provider, &coefficients, &right_hand_side).unwrap();

    assert_complex32_arrays_close(&actual, &expected);
    assert!(!actual.shares_storage_with(&coefficients));
    assert!(!actual.shares_storage_with(&right_hand_side));
}

#[test]
fn actual_solve_reports_singular_matrix_like_reference() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let error = solve_f64(
        &provider,
        &array([2, 2], vec![1.0, 2.0, 2.0, 4.0]),
        &array([2, 1], vec![3.0, 6.0]),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LinalgError::SingularMatrix {
            provider: "openblas",
            operation: "f64 general solve",
            pivot: 2,
        }
    ));
}

#[test]
fn actual_zero_order_solve_returns_independent_empty_results() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let coefficients = array::<f64>([0, 0], vec![]);
    for right_hand_side in [array::<f64>([0, 0], vec![]), array::<f64>([0, 4], vec![])] {
        let actual = solve_f64(&provider, &coefficients, &right_hand_side).unwrap();
        assert_eq!(actual.shape(), right_hand_side.shape());
        assert!(actual.is_empty());
        assert!(!actual.shares_storage_with(&right_hand_side));
    }
}

#[test]
fn actual_provider_checks_lp64_before_empty_solve_shortcuts() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let oversized_right_hand_sides = LP64_MAX_DIMENSION + 1;
    let error = solve_f64(
        &provider,
        &array::<f64>([0, 0], vec![]),
        &array::<f64>([0, oversized_right_hand_sides], vec![]),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LinalgError::Lp64DimensionOverflow {
            parameter: "nrhs",
            value,
        } if value == oversized_right_hand_sides
    ));

    assert!(matches!(
        solve_f32(
            &provider,
            &array::<f32>([0, 0], vec![]),
            &array::<f32>([0, oversized_right_hand_sides], vec![]),
        ),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "nrhs",
            value,
        }) if value == oversized_right_hand_sides
    ));

    let oversized_columns = LP64_MAX_DIMENSION + 1;
    assert!(matches!(
        solve_rectangular_f32(
            &provider,
            &array::<f32>([0, oversized_columns], vec![]),
            &array::<f32>([0, 0], vec![]),
        ),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "n",
            value,
        }) if value == oversized_columns
    ));
}

#[test]
fn actual_single_solvers_observe_preexisting_cancellation_before_native_calls() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let cancellation = AtomicBool::new(true);
    let coefficients = array([1, 1], vec![2.0_f32]);
    let right_hand_side = array([1, 1], vec![4.0_f32]);
    assert_eq!(
        provider
            .solve_f32(
                SolveRequest::new(&coefficients, &right_hand_side)
                    .with_cancellation_flag(&cancellation),
            )
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "general linear solve",
        }
    );
    assert_eq!(
        provider
            .solve_rectangular_f32(
                RectangularSolveRequest::new(&coefficients, &right_hand_side)
                    .with_cancellation_flag(&cancellation),
            )
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "rectangular linear solve",
        }
    );
}

#[test]
fn actual_f64_overdetermined_solve_matches_reference_and_residual() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;
    let coefficients = array([3, 2], vec![1.0, 1.0, 1.0, 0.0, 1.0, 2.0]);
    let right_hand_side = array([3, 2], vec![1.0, 2.0, 2.0, 2.0, 0.0, 1.0]);
    let original_coefficients = coefficients.deep_copy();
    let original_right_hand_side = right_hand_side.deep_copy();

    let actual = solve_rectangular_f64(&provider, &coefficients, &right_hand_side).unwrap();
    let expected = solve_rectangular_f64(&reference, &coefficients, &right_hand_side).unwrap();
    assert_f64_arrays_close(&actual, &expected);
    let fitted = matrix_multiply_f64(&reference, &coefficients, &actual).unwrap();
    for rhs in 0..2 {
        let residual = [
            fitted.as_slice()[rhs * 3] - right_hand_side.as_slice()[rhs * 3],
            fitted.as_slice()[rhs * 3 + 1] - right_hand_side.as_slice()[rhs * 3 + 1],
            fitted.as_slice()[rhs * 3 + 2] - right_hand_side.as_slice()[rhs * 3 + 2],
        ];
        assert_close(residual.iter().sum(), 0.0);
        assert_close(residual[1] + 2.0 * residual[2], 0.0);
    }
    assert_eq!(coefficients, original_coefficients);
    assert_eq!(right_hand_side, original_right_hand_side);
    assert!(!actual.shares_storage_with(&coefficients));
    assert!(!actual.shares_storage_with(&right_hand_side));
}

#[test]
fn actual_complex_underdetermined_solve_matches_reference_and_minimum_norm() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let reference = ReferenceProvider;
    let coefficients = array(
        [2, 3],
        vec![
            Complex64::new(1.0, 0.0),
            Complex64::ZERO,
            Complex64::ZERO,
            Complex64::new(1.0, 0.0),
            Complex64::new(1.0, 0.0),
            Complex64::new(0.0, 1.0),
        ],
    );
    let right_hand_side = array(
        [2, 1],
        vec![Complex64::new(1.0, 0.0), Complex64::new(3.0, -1.0)],
    );

    let actual = solve_rectangular_complex64(&provider, &coefficients, &right_hand_side).unwrap();
    let expected =
        solve_rectangular_complex64(&reference, &coefficients, &right_hand_side).unwrap();
    assert_complex_arrays_close(&actual, &expected);
    let reproduced = matrix_multiply_complex64(&reference, &coefficients, &actual).unwrap();
    assert_complex_arrays_close(&reproduced, &right_hand_side);
    let x = actual.as_slice();
    assert_complex_close(
        Complex64::new(-x[0].re, -x[0].im) + Complex64::new(0.0, 1.0) * x[1] + x[2],
        Complex64::ZERO,
    );
}

#[test]
fn actual_f32_overdetermined_solve_matches_native_reference() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let coefficients = array([3, 2], vec![1.0_f32, 1.0, 1.0, 0.0, 1.0, 2.0]);
    let right_hand_side = array([3, 2], vec![1.0_f32, 2.0, 2.0, 2.0, 0.0, 1.0]);
    let actual = solve_rectangular_f32(&provider, &coefficients, &right_hand_side).unwrap();
    let expected =
        solve_rectangular_f32(&ReferenceProvider, &coefficients, &right_hand_side).unwrap();
    assert_f32_arrays_close(&actual, &expected);
    assert!(!actual.shares_storage_with(&coefficients));
    assert!(!actual.shares_storage_with(&right_hand_side));
}

#[test]
fn actual_complex32_underdetermined_solve_matches_native_reference() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let coefficients = array(
        [2, 3],
        vec![
            Complex32::new(1.0, 0.0),
            Complex32::ZERO,
            Complex32::ZERO,
            Complex32::new(1.0, 0.0),
            Complex32::new(1.0, 0.0),
            Complex32::new(0.0, 1.0),
        ],
    );
    let right_hand_side = array(
        [2, 1],
        vec![Complex32::new(1.0, 0.0), Complex32::new(3.0, -1.0)],
    );
    let actual = solve_rectangular_complex32(&provider, &coefficients, &right_hand_side).unwrap();
    let expected =
        solve_rectangular_complex32(&ReferenceProvider, &coefficients, &right_hand_side).unwrap();
    assert_complex32_arrays_close(&actual, &expected);
    let reproduced = matrix_multiply_complex32(&ReferenceProvider, &coefficients, &actual).unwrap();
    assert_complex32_arrays_close(&reproduced, &right_hand_side);
}

#[test]
#[allow(clippy::similar_names, clippy::too_many_lines)]
fn actual_decomposition_entry_points_cover_every_precision_when_explicitly_enabled() {
    let Some(provider) = configured_provider() else {
        return;
    };

    let real32 = array([2, 2], vec![0.0_f32, 2.0, 1.0, 3.0]);
    let real64 = array([2, 2], vec![0.0_f64, 2.0, 1.0, 3.0]);
    let complex32 = array(
        [2, 2],
        vec![
            Complex32::ZERO,
            Complex32::new(2.0, 1.0),
            Complex32::new(1.0, -1.0),
            Complex32::new(3.0, 0.5),
        ],
    );
    let complex64 = array(
        [2, 2],
        complex32
            .as_slice()
            .iter()
            .map(|value| Complex64::new(f64::from(value.re), f64::from(value.im)))
            .collect(),
    );
    assert_eq!(
        provider
            .factor_lu_f32(FactorRequest::new(&real32))
            .unwrap()
            .row_permutation_zero_based,
        vec![1, 0]
    );
    assert_eq!(
        provider
            .factor_lu_f64(FactorRequest::new(&real64))
            .unwrap()
            .row_permutation_zero_based,
        vec![1, 0]
    );
    assert_eq!(
        provider
            .factor_lu_complex32(FactorRequest::new(&complex32))
            .unwrap()
            .first_zero_pivot,
        None
    );
    assert_eq!(
        provider
            .factor_lu_complex64(FactorRequest::new(&complex64))
            .unwrap()
            .first_zero_pivot,
        None
    );

    let qr32 = provider
        .qr_f32(QrRequest::new(&real32, QrVectors::Full))
        .unwrap();
    assert_f32_arrays_close(
        &matrix_multiply_f32(&ReferenceProvider, &qr32.q, &qr32.r).unwrap(),
        &real32,
    );
    let qr64 = provider
        .qr_f64(QrRequest::new(&real64, QrVectors::Thin))
        .unwrap();
    assert_f64_arrays_close(
        &matrix_multiply_f64(&ReferenceProvider, &qr64.q, &qr64.r).unwrap(),
        &real64,
    );
    let qrc32 = provider
        .qr_complex32(QrRequest::new(&complex32, QrVectors::Full).with_column_pivoting(true))
        .unwrap();
    let permuted32 = array(
        [2, 2],
        qrc32
            .column_permutation_zero_based
            .iter()
            .flat_map(|&column| {
                let start = usize::try_from(column).unwrap() * 2;
                complex32.as_slice()[start..start + 2].iter().copied()
            })
            .collect(),
    );
    assert_complex32_arrays_close(
        &matrix_multiply_complex32(&ReferenceProvider, &qrc32.q, &qrc32.r).unwrap(),
        &permuted32,
    );
    let qrc64 = provider
        .qr_complex64(QrRequest::new(&complex64, QrVectors::Thin))
        .unwrap();
    assert_complex_arrays_close(
        &matrix_multiply_complex64(&ReferenceProvider, &qrc64.q, &qrc64.r).unwrap(),
        &complex64,
    );

    let spd32 = array([2, 2], vec![4.0_f32, 2.0, 2.0, 3.0]);
    let spd64 = array([2, 2], vec![4.0_f64, 2.0, 2.0, 3.0]);
    assert_eq!(
        provider
            .cholesky_f32(CholeskyRequest::new(&spd32, CholeskyTriangle::Upper))
            .unwrap()
            .first_non_positive_minor,
        None
    );
    assert_eq!(
        provider
            .cholesky_f64(CholeskyRequest::new(&spd64, CholeskyTriangle::Lower))
            .unwrap()
            .first_non_positive_minor,
        None
    );
    let h32 = array(
        [2, 2],
        vec![
            Complex32::new(4.0, 0.0),
            Complex32::new(2.0, 2.0),
            Complex32::new(2.0, -2.0),
            Complex32::new(5.0, 0.0),
        ],
    );
    let h64 = array(
        [2, 2],
        h32.as_slice()
            .iter()
            .map(|value| Complex64::new(f64::from(value.re), f64::from(value.im)))
            .collect(),
    );
    assert_eq!(
        provider
            .cholesky_complex32(CholeskyRequest::new(&h32, CholeskyTriangle::Lower))
            .unwrap()
            .first_non_positive_minor,
        None
    );
    assert_eq!(
        provider
            .cholesky_complex64(CholeskyRequest::new(&h64, CholeskyTriangle::Upper))
            .unwrap()
            .first_non_positive_minor,
        None
    );
}

#[test]
fn actual_decomposition_checks_lp64_and_preexisting_cancellation_before_native_calls() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let oversized = LP64_MAX_DIMENSION + 1;
    let empty = array::<f32>([oversized, 0], vec![]);
    assert!(matches!(
        provider.factor_lu_f32(FactorRequest::new(&empty)),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "m",
            value,
        }) if value == oversized
    ));
    assert!(matches!(
        provider.qr_f32(QrRequest::new(&empty, QrVectors::Full)),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "m",
            value,
        }) if value == oversized
    ));

    let cancellation = AtomicBool::new(true);
    let matrix = array([1, 1], vec![1.0_f32]);
    assert_eq!(
        provider
            .factor_lu_f32(FactorRequest::new(&matrix).with_cancellation_flag(&cancellation))
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "LU factorization"
        }
    );
    assert_eq!(
        provider
            .qr_f32(QrRequest::new(&matrix, QrVectors::Full).with_cancellation_flag(&cancellation))
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "QR factorization"
        }
    );
    assert_eq!(
        provider
            .cholesky_f32(
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
fn actual_spectral_provider_covers_four_precisions_when_explicitly_enabled() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let real32 = array([2, 2], vec![3.0_f32, 0.0, 0.0, 2.0]);
    let real64 = array([2, 2], vec![3.0_f64, 0.0, 0.0, 2.0]);
    let complex32 = array(
        [2, 2],
        vec![
            Complex32::new(3.0, 0.0),
            Complex32::ZERO,
            Complex32::ZERO,
            Complex32::new(2.0, 1.0),
        ],
    );
    let complex64 = array(
        [2, 2],
        vec![
            Complex64::new(3.0, 0.0),
            Complex64::ZERO,
            Complex64::ZERO,
            Complex64::new(2.0, 1.0),
        ],
    );
    assert_close32(
        provider
            .svd_f32(SvdRequest::new(&real32, SvdVectors::Thin))
            .unwrap()
            .singular_values
            .as_slice()[0],
        3.0,
    );
    assert_close(
        provider
            .svd_f64(SvdRequest::new(&real64, SvdVectors::Full))
            .unwrap()
            .singular_values
            .as_slice()[0],
        3.0,
    );
    assert_eq!(
        provider
            .svd_complex32(SvdRequest::new(&complex32, SvdVectors::None))
            .unwrap()
            .singular_values
            .as_slice()
            .len(),
        2
    );
    assert_eq!(
        provider
            .svd_complex64(SvdRequest::new(&complex64, SvdVectors::Thin))
            .unwrap()
            .singular_values
            .as_slice()
            .len(),
        2
    );

    assert_eq!(
        provider
            .eig_f32(EigRequest::new(&real32))
            .unwrap()
            .eigenvalues
            .as_slice()
            .len(),
        2
    );
    assert_eq!(
        provider
            .eig_f64(EigRequest::new(&real64).with_right_vectors(true))
            .unwrap()
            .right_vectors
            .unwrap()
            .shape()
            .dimensions(),
        &[2, 2]
    );
    assert_eq!(
        provider
            .eig_complex32(EigRequest::new(&complex32).with_left_vectors(true))
            .unwrap()
            .left_vectors
            .unwrap()
            .shape()
            .dimensions(),
        &[2, 2]
    );
    assert_eq!(
        provider
            .eig_complex64(
                EigRequest::new(&complex64)
                    .with_left_vectors(true)
                    .with_right_vectors(true)
            )
            .unwrap()
            .eigenvalues
            .as_slice()
            .len(),
        2
    );
}

#[test]
fn actual_complex_schur_reconstructs_non_normal_matrices() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let complex32 = array(
        [2, 2],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::new(0.0, 3.0),
            Complex32::new(2.0, -1.0),
            Complex32::new(4.0, -1.0),
        ],
    );
    let complex64 = array(
        [2, 2],
        vec![
            Complex64::new(1.0, 1.0),
            Complex64::new(0.0, 3.0),
            Complex64::new(2.0, -1.0),
            Complex64::new(4.0, -1.0),
        ],
    );

    let schur32 = provider
        .schur_complex32(SchurRequest::new(&complex32))
        .unwrap();
    assert_close32(schur32.form.as_slice()[1].re, 0.0);
    assert_close32(schur32.form.as_slice()[1].im, 0.0);
    let qt32 = matrix_multiply_complex32(&provider, &schur32.vectors, &schur32.form).unwrap();
    let mut reconstructed32 = array([2, 2], vec![Complex32::ZERO; 4]);
    provider
        .gemm_complex32(
            GemmRequest::new(
                &qt32,
                &schur32.vectors,
                GemmOptions::new(
                    MatrixTranspose::None,
                    MatrixTranspose::ConjugateTranspose,
                    Complex32::new(1.0, 0.0),
                    Complex32::ZERO,
                ),
            ),
            &mut reconstructed32,
        )
        .unwrap();
    assert_complex32_arrays_close(&reconstructed32, &complex32);

    let schur64 = provider
        .schur_complex64(SchurRequest::new(&complex64))
        .unwrap();
    assert_complex_close(schur64.form.as_slice()[1], Complex64::ZERO);
    let qt64 = matrix_multiply_complex64(&provider, &schur64.vectors, &schur64.form).unwrap();
    let mut reconstructed64 = array([2, 2], vec![Complex64::ZERO; 4]);
    provider
        .gemm_complex64(
            GemmRequest::new(
                &qt64,
                &schur64.vectors,
                GemmOptions::new(
                    MatrixTranspose::None,
                    MatrixTranspose::ConjugateTranspose,
                    Complex64::new(1.0, 0.0),
                    Complex64::ZERO,
                ),
            ),
            &mut reconstructed64,
        )
        .unwrap();
    assert_complex_arrays_close(&reconstructed64, &complex64);
}

#[test]
fn actual_spectral_checks_lp64_and_cancellation_before_native_calls() {
    let Some(provider) = configured_provider() else {
        return;
    };
    let oversized = LP64_MAX_DIMENSION + 1;
    let empty = array::<f32>([oversized, 0], vec![]);
    assert!(matches!(
        provider.svd_f32(SvdRequest::new(&empty, SvdVectors::None)),
        Err(LinalgError::Lp64DimensionOverflow { parameter: "m", value }) if value == oversized
    ));
    let cancellation = AtomicBool::new(true);
    let matrix = array([1, 1], vec![1.0_f32]);
    assert!(matches!(
        provider.svd_f32(
            SvdRequest::new(&matrix, SvdVectors::Thin).with_cancellation_flag(&cancellation)
        ),
        Err(LinalgError::Cancelled {
            operation: "singular value decomposition"
        })
    ));
    assert!(matches!(
        provider.eig_f32(EigRequest::new(&matrix).with_cancellation_flag(&cancellation)),
        Err(LinalgError::Cancelled {
            operation: "general eigenvalue decomposition"
        })
    ));
    let complex = array([1, 1], vec![Complex32::new(1.0, 0.0)]);
    assert_eq!(
        provider
            .schur_complex32(SchurRequest::new(&complex).with_cancellation_flag(&cancellation))
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "complex Schur decomposition"
        }
    );
}

use std::sync::atomic::AtomicBool;

use openmat_array::{Complex64, DenseArray, Shape};
use openmat_linalg::{
    GemmOptions, GemmRequest, LP64_MAX_DIMENSION, LinalgError, LinalgProvider, Lp64GemmDimensions,
    Lp64RectangularSolveDimensions, Lp64SolveDimensions, MatrixTranspose, ProviderIntegerAbi,
    RectangularSolveKind, RectangularSolveRequest, ReferenceProvider, SolveRequest,
    checked_lp64_dimension, matrix_multiply_complex64, matrix_multiply_f64, solve_complex64,
    solve_f64, solve_rectangular_complex64, solve_rectangular_f64, validate_gemm,
    validate_rectangular_solve, validate_solve,
};

fn array<T>(dimensions: [u64; 2], data: Vec<T>) -> DenseArray<T> {
    DenseArray::from_vec(Shape::new(dimensions).unwrap(), data).unwrap()
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = 1.0e-12 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, received {actual}"
    );
}

fn assert_complex_close(actual: Complex64, expected: Complex64) {
    assert_close(actual.re, expected.re);
    assert_close(actual.im, expected.im);
}

fn complex_subtract(left: Complex64, right: Complex64) -> Complex64 {
    Complex64::new(left.re - right.re, left.im - right.im)
}

#[test]
fn reference_gemm_uses_column_major_storage() {
    let provider = ReferenceProvider;
    let left = array([2, 3], vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    let right = array([3, 2], vec![7.0, 9.0, 11.0, 8.0, 10.0, 12.0]);

    let result = matrix_multiply_f64(&provider, &left, &right).unwrap();

    assert_eq!(result.shape().dimensions(), &[2, 2]);
    for (actual, expected) in result.as_slice().iter().zip([58.0, 139.0, 64.0, 154.0]) {
        assert_close(*actual, expected);
    }
}

#[test]
fn one_by_one_and_zero_inner_dimension_products_work() {
    let provider = ReferenceProvider;
    let scalar = matrix_multiply_f64(
        &provider,
        &array([1, 1], vec![6.0]),
        &array([1, 1], vec![7.0]),
    )
    .unwrap();
    assert_close(scalar.as_slice()[0], 42.0);

    let empty_left = array::<f64>([2, 0], vec![]);
    let empty_right = array::<f64>([0, 3], vec![]);
    let zeros = matrix_multiply_f64(&provider, &empty_left, &empty_right).unwrap();
    assert_eq!(zeros.shape().dimensions(), &[2, 3]);
    assert_eq!(zeros.as_slice(), &[0.0; 6]);

    let zero_rows = matrix_multiply_f64(
        &provider,
        &array::<f64>([0, 2], vec![]),
        &array([2, 3], vec![1.0; 6]),
    )
    .unwrap();
    assert_eq!(zero_rows.shape().dimensions(), &[0, 3]);
    assert!(zero_rows.is_empty());
}

#[test]
fn gemm_supports_transpose_alpha_beta_and_cow_output() {
    let provider = ReferenceProvider;
    let left = array([2, 3], vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    let right = array([2, 1], vec![10.0, 20.0]);
    let mut output = array([3, 1], vec![1.0, 1.0, 1.0]);
    let original_output = output.clone();
    let request = GemmRequest::new(
        &left,
        &right,
        GemmOptions::new(MatrixTranspose::Transpose, MatrixTranspose::None, 2.0, 3.0),
    );

    provider.gemm_f64(request, &mut output).unwrap();

    assert!(!output.shares_storage_with(&original_output));
    assert_eq!(original_output.as_slice(), &[1.0, 1.0, 1.0]);
    for (actual, expected) in output.as_slice().iter().zip([183.0, 243.0, 303.0]) {
        assert_close(*actual, expected);
    }
}

#[test]
fn complex_gemm_honors_conjugate_transpose() {
    let provider = ReferenceProvider;
    let vector = array(
        [2, 1],
        vec![Complex64::new(1.0, 1.0), Complex64::new(2.0, -1.0)],
    );
    let mut output = array([1, 1], vec![Complex64::ZERO]);
    let request = GemmRequest::new(
        &vector,
        &vector,
        GemmOptions::new(
            MatrixTranspose::ConjugateTranspose,
            MatrixTranspose::None,
            Complex64::new(1.0, 0.0),
            Complex64::ZERO,
        ),
    );

    provider.gemm_complex64(request, &mut output).unwrap();
    assert_complex_close(output.as_slice()[0], Complex64::new(7.0, 0.0));

    let ordinary = matrix_multiply_complex64(
        &provider,
        &array([1, 1], vec![Complex64::new(1.0, 2.0)]),
        &array([1, 1], vec![Complex64::new(3.0, -1.0)]),
    )
    .unwrap();
    assert_complex_close(ordinary.as_slice()[0], Complex64::new(5.0, 5.0));
}

#[test]
fn reference_dot_and_scaled_norm_cover_basic_operations() {
    let provider = ReferenceProvider;
    let left = array([1, 3], vec![1.0, 2.0, 3.0]);
    let right = array([3, 1], vec![4.0, 5.0, 6.0]);
    assert_close(provider.dot_f64(&left, &right).unwrap(), 32.0);
    assert_close(
        provider
            .norm2_f64(&array([1, 2], vec![3.0e200, 4.0e200]))
            .unwrap(),
        5.0e200,
    );
}

#[test]
fn reference_norm_handles_infinity_without_losing_nan_propagation() {
    let provider = ReferenceProvider;

    let repeated_infinity = provider
        .norm2_f64(&array([1, 2], vec![f64::INFINITY, f64::INFINITY]))
        .unwrap();
    assert!(repeated_infinity.is_infinite() && repeated_infinity.is_sign_positive());

    let infinity_and_finite = provider
        .norm2_f64(&array([1, 2], vec![f64::INFINITY, 3.0]))
        .unwrap();
    assert!(infinity_and_finite.is_infinite() && infinity_and_finite.is_sign_positive());

    let nan_before_infinity = provider
        .norm2_f64(&array([1, 2], vec![f64::NAN, f64::INFINITY]))
        .unwrap();
    assert!(nan_before_infinity.is_nan());

    let nan_after_infinity = provider
        .norm2_f64(&array([1, 2], vec![f64::INFINITY, f64::NAN]))
        .unwrap();
    assert!(nan_after_infinity.is_nan());
}

#[test]
fn dimension_and_output_errors_are_typed() {
    let provider = ReferenceProvider;
    let mismatch = matrix_multiply_f64(
        &provider,
        &array([2, 3], vec![0.0; 6]),
        &array([4, 2], vec![0.0; 8]),
    )
    .unwrap_err();
    assert!(matches!(
        mismatch,
        LinalgError::DimensionMismatch {
            operation: "matrix multiplication inner dimensions",
            left: 3,
            right: 4,
        }
    ));

    let left = array([2, 2], vec![0.0; 4]);
    let right = array([2, 2], vec![0.0; 4]);
    let mut wrong_output = array([1, 4], vec![0.0; 4]);
    let request = GemmRequest::new(
        &left,
        &right,
        GemmOptions::new(MatrixTranspose::None, MatrixTranspose::None, 1.0, 0.0),
    );
    assert!(matches!(
        provider.gemm_f64(request, &mut wrong_output),
        Err(LinalgError::OutputShapeMismatch {
            expected_rows: 2,
            expected_columns: 2,
            actual_rows: 1,
            actual_columns: 4,
        })
    ));

    let higher_rank = DenseArray::from_vec(Shape::new([1, 1, 2]).unwrap(), vec![0.0; 2]).unwrap();
    assert!(matches!(
        matrix_multiply_f64(&provider, &higher_rank, &array([2, 1], vec![0.0; 2])),
        Err(LinalgError::MatrixRequired { .. })
    ));
}

#[test]
fn dot_rejects_different_element_counts() {
    let provider = ReferenceProvider;
    let error = provider
        .dot_f64(
            &array([1, 2], vec![1.0, 2.0]),
            &array([1, 3], vec![1.0, 2.0, 3.0]),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        LinalgError::DimensionMismatch {
            operation: "real dot product element counts",
            left: 2,
            right: 3,
        }
    ));
}

#[test]
fn lp64_conversion_checks_every_dimension_before_native_calls() {
    assert_eq!(
        checked_lp64_dimension("m", LP64_MAX_DIMENSION).unwrap(),
        i32::MAX
    );
    assert_eq!(
        checked_lp64_dimension("n", LP64_MAX_DIMENSION + 1).unwrap_err(),
        LinalgError::Lp64DimensionOverflow {
            parameter: "n",
            value: LP64_MAX_DIMENSION + 1,
        }
    );

    let oversized_rows = LP64_MAX_DIMENSION + 1;
    let left = array::<f64>([oversized_rows, 0], vec![]);
    let right = array::<f64>([0, 0], vec![]);
    let output = array::<f64>([oversized_rows, 0], vec![]);
    let request = GemmRequest::new(
        &left,
        &right,
        GemmOptions::new(MatrixTranspose::None, MatrixTranspose::None, 1.0, 0.0),
    );
    let dimensions = validate_gemm(&request, &output).unwrap();
    assert!(matches!(
        Lp64GemmDimensions::try_from(&dimensions),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "m",
            value,
        }) if value == oversized_rows
    ));
}

#[test]
fn reference_provider_does_not_claim_an_external_blas_abi() {
    let provider = ReferenceProvider;
    assert_eq!(provider.name(), "reference");
    assert_eq!(provider.integer_abi(), ProviderIntegerAbi::RustU64);
}

#[test]
fn reference_f64_solve_pivots_and_supports_multiple_right_hand_sides() {
    let provider = ReferenceProvider;
    let coefficients = array([3, 3], vec![0.0, 1.0, 2.0, 2.0, -2.0, 3.0, 1.0, -3.0, 1.0]);
    let right_hand_side = array([3, 2], vec![3.0, 0.0, 7.0, -1.0, -1.0, 5.0]);
    let coefficient_alias = coefficients.clone();
    let right_hand_side_alias = right_hand_side.clone();
    let original_coefficients = coefficients.deep_copy();
    let original_right_hand_side = right_hand_side.deep_copy();

    let solution = solve_f64(&provider, &coefficients, &right_hand_side).unwrap();

    assert_eq!(solution.shape().dimensions(), &[3, 2]);
    for (actual, expected) in solution
        .as_slice()
        .iter()
        .zip([1.0, 2.0, -1.0, 4.0, -2.0, 3.0])
    {
        assert_close(*actual, expected);
    }
    assert!(coefficients.shares_storage_with(&coefficient_alias));
    assert!(right_hand_side.shares_storage_with(&right_hand_side_alias));
    assert!(!solution.shares_storage_with(&coefficients));
    assert!(!solution.shares_storage_with(&right_hand_side));
    assert_eq!(coefficients, original_coefficients);
    assert_eq!(right_hand_side, original_right_hand_side);
}

#[test]
fn reference_complex_solve_matches_known_multiple_right_hand_sides() {
    let provider = ReferenceProvider;
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
    let right_hand_side = matrix_multiply_complex64(&provider, &coefficients, &expected).unwrap();
    let original_coefficients = coefficients.deep_copy();
    let original_right_hand_side = right_hand_side.deep_copy();

    let actual = solve_complex64(&provider, &coefficients, &right_hand_side).unwrap();

    assert_eq!(actual.shape(), expected.shape());
    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_complex_close(actual, expected);
    }
    assert_eq!(coefficients, original_coefficients);
    assert_eq!(right_hand_side, original_right_hand_side);
    assert!(!actual.shares_storage_with(&coefficients));
    assert!(!actual.shares_storage_with(&right_hand_side));
}

#[test]
fn reference_solve_supports_zero_order_with_zero_or_many_right_hand_sides() {
    let provider = ReferenceProvider;
    for right_hand_side in [array::<f64>([0, 0], vec![]), array::<f64>([0, 4], vec![])] {
        let solution =
            solve_f64(&provider, &array::<f64>([0, 0], vec![]), &right_hand_side).unwrap();
        assert_eq!(solution.shape(), right_hand_side.shape());
        assert!(solution.is_empty());
        assert!(!solution.shares_storage_with(&right_hand_side));
    }
}

#[test]
fn reference_solve_reports_singularity_and_shape_errors() {
    let provider = ReferenceProvider;
    let singular = solve_f64(
        &provider,
        &array([2, 2], vec![1.0, 2.0, 2.0, 4.0]),
        &array([2, 1], vec![3.0, 6.0]),
    )
    .unwrap_err();
    assert!(matches!(
        singular,
        LinalgError::SingularMatrix {
            provider: "reference",
            operation: "general linear solve",
            pivot: 2,
        }
    ));

    let non_square = solve_f64(
        &provider,
        &array([2, 3], vec![0.0; 6]),
        &array([2, 1], vec![0.0; 2]),
    )
    .unwrap_err();
    assert!(matches!(
        non_square,
        LinalgError::SquareMatrixRequired {
            rows: 2,
            columns: 3,
            ..
        }
    ));

    let wrong_rows = solve_f64(
        &provider,
        &array([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
        &array([3, 1], vec![0.0; 3]),
    )
    .unwrap_err();
    assert!(matches!(
        wrong_rows,
        LinalgError::DimensionMismatch {
            operation: "linear solve coefficient and right-hand-side rows",
            left: 2,
            right: 3,
        }
    ));

    let higher_rank = DenseArray::from_vec(Shape::new([1, 1, 2]).unwrap(), vec![1.0, 1.0]).unwrap();
    assert!(matches!(
        solve_f64(&provider, &higher_rank, &array([1, 1], vec![1.0])),
        Err(LinalgError::MatrixRequired { .. })
    ));
}

#[test]
fn reference_solve_observes_preexisting_cancellation() {
    let provider = ReferenceProvider;
    let cancellation = AtomicBool::new(true);
    let coefficients = array([1, 1], vec![2.0]);
    let right_hand_side = array([1, 1], vec![4.0]);
    let error = provider
        .solve_f64(
            SolveRequest::new(&coefficients, &right_hand_side)
                .with_cancellation_flag(&cancellation),
        )
        .unwrap_err();
    assert_eq!(
        error,
        LinalgError::Cancelled {
            operation: "general linear solve"
        }
    );
}

#[test]
fn lp64_solve_conversion_checks_empty_dimensions_before_native_shortcuts() {
    let coefficients = array::<f64>([0, 0], vec![]);
    let oversized_right_hand_sides = LP64_MAX_DIMENSION + 1;
    let right_hand_side = array::<f64>([0, oversized_right_hand_sides], vec![]);
    let dimensions = validate_solve(&SolveRequest::new(&coefficients, &right_hand_side)).unwrap();

    assert!(matches!(
        Lp64SolveDimensions::try_from(&dimensions),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "nrhs",
            value,
        }) if value == oversized_right_hand_sides
    ));
}

#[test]
fn rectangular_contract_classifies_all_three_shapes_and_validates_rhs_rows() {
    for (shape, expected) in [
        ([2, 2], RectangularSolveKind::Square),
        ([3, 2], RectangularSolveKind::OverdeterminedLeastSquares),
        ([2, 3], RectangularSolveKind::UnderdeterminedMinimumNorm),
    ] {
        let coefficient_len = usize::try_from(shape[0] * shape[1]).unwrap();
        let right_hand_side_len = usize::try_from(shape[0] * 4).unwrap();
        let coefficients = array::<f64>(shape, vec![0.0; coefficient_len]);
        let right_hand_side = array::<f64>([shape[0], 4], vec![0.0; right_hand_side_len]);
        let dimensions = validate_rectangular_solve(&RectangularSolveRequest::new(
            &coefficients,
            &right_hand_side,
        ))
        .unwrap();
        assert_eq!(dimensions.kind(), expected);
        assert_eq!(dimensions.required_rank(), shape[0].min(shape[1]));
        assert_eq!(dimensions.coefficient_columns(), shape[1]);
        assert_eq!(dimensions.right_hand_sides(), 4);
    }

    let coefficients = array::<f64>([2, 3], vec![0.0; 6]);
    let right_hand_side = array::<f64>([3, 1], vec![0.0; 3]);
    assert!(matches!(
        validate_rectangular_solve(&RectangularSolveRequest::new(
            &coefficients,
            &right_hand_side,
        )),
        Err(LinalgError::DimensionMismatch {
            operation: "rectangular solve coefficient and right-hand-side rows",
            left: 2,
            right: 3,
        })
    ));
}

#[test]
fn reference_real_overdetermined_solution_has_orthogonal_residuals() {
    let provider = ReferenceProvider;
    let coefficients = array([3, 2], vec![1.0, 1.0, 1.0, 0.0, 1.0, 2.0]);
    let right_hand_side = array([3, 2], vec![1.0, 2.0, 2.0, 2.0, 0.0, 1.0]);
    let original_coefficients = coefficients.deep_copy();
    let original_right_hand_side = right_hand_side.deep_copy();

    let solution = solve_rectangular_f64(&provider, &coefficients, &right_hand_side).unwrap();
    let expected = [7.0 / 6.0, 0.5, 1.5, -0.5];
    assert_eq!(solution.shape().dimensions(), &[2, 2]);
    for (&actual, expected) in solution.as_slice().iter().zip(expected) {
        assert_close(actual, expected);
    }

    let fitted = matrix_multiply_f64(&provider, &coefficients, &solution).unwrap();
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
    assert!(!solution.shares_storage_with(&coefficients));
    assert!(!solution.shares_storage_with(&right_hand_side));
}

#[test]
fn reference_real_underdetermined_solution_is_minimum_norm() {
    let provider = ReferenceProvider;
    let coefficients = array([2, 3], vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    let right_hand_side = array([2, 2], vec![1.0, 2.0, 2.0, -1.0]);

    let solution = solve_rectangular_f64(&provider, &coefficients, &right_hand_side).unwrap();
    let expected = [0.0, 1.0, 1.0, 5.0 / 3.0, -4.0 / 3.0, 1.0 / 3.0];
    assert_eq!(solution.shape().dimensions(), &[3, 2]);
    for (&actual, expected) in solution.as_slice().iter().zip(expected) {
        assert_close(actual, expected);
    }
    let reproduced = matrix_multiply_f64(&provider, &coefficients, &solution).unwrap();
    for (&actual, &expected) in reproduced.as_slice().iter().zip(right_hand_side.as_slice()) {
        assert_close(actual, expected);
    }
    for rhs in 0..2 {
        let x = &solution.as_slice()[rhs * 3..rhs * 3 + 3];
        assert_close(-x[0] - x[1] + x[2], 0.0);
    }
}

#[test]
fn reference_complex_rectangular_paths_cover_residual_and_minimum_norm() {
    let provider = ReferenceProvider;
    let over_coefficients = array(
        [3, 2],
        vec![
            Complex64::new(1.0, 0.0),
            Complex64::new(0.0, 1.0),
            Complex64::ZERO,
            Complex64::ZERO,
            Complex64::ZERO,
            Complex64::new(2.0, 0.0),
        ],
    );
    let over_rhs = array(
        [3, 1],
        vec![
            Complex64::new(1.0, 3.0),
            Complex64::new(-1.0, 1.0),
            Complex64::new(-1.0, 2.0),
        ],
    );
    let over_solution =
        solve_rectangular_complex64(&provider, &over_coefficients, &over_rhs).unwrap();
    assert_complex_close(over_solution.as_slice()[0], Complex64::new(1.0, 2.0));
    assert_complex_close(over_solution.as_slice()[1], Complex64::new(-0.5, 1.0));
    let fitted = matrix_multiply_complex64(&provider, &over_coefficients, &over_solution).unwrap();
    let residual = [
        complex_subtract(fitted.as_slice()[0], over_rhs.as_slice()[0]),
        complex_subtract(fitted.as_slice()[1], over_rhs.as_slice()[1]),
        complex_subtract(fitted.as_slice()[2], over_rhs.as_slice()[2]),
    ];
    assert_complex_close(
        residual[0] + Complex64::new(0.0, -1.0) * residual[1],
        Complex64::ZERO,
    );
    assert_complex_close(Complex64::new(2.0, 0.0) * residual[2], Complex64::ZERO);

    let under_coefficients = array(
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
    let under_rhs = array(
        [2, 1],
        vec![Complex64::new(1.0, 0.0), Complex64::new(3.0, -1.0)],
    );
    let under_solution =
        solve_rectangular_complex64(&provider, &under_coefficients, &under_rhs).unwrap();
    for (&actual, expected) in under_solution.as_slice().iter().zip([
        Complex64::new(1.0, 1.0),
        Complex64::new(2.0, -1.0),
        Complex64::new(0.0, -1.0),
    ]) {
        assert_complex_close(actual, expected);
    }
    let reproduced =
        matrix_multiply_complex64(&provider, &under_coefficients, &under_solution).unwrap();
    for (&actual, &expected) in reproduced.as_slice().iter().zip(under_rhs.as_slice()) {
        assert_complex_close(actual, expected);
    }
    let x = under_solution.as_slice();
    assert_complex_close(
        Complex64::new(-x[0].re, -x[0].im) + Complex64::new(0.0, 1.0) * x[1] + x[2],
        Complex64::ZERO,
    );
}

#[test]
fn rectangular_reference_reports_rank_deficiency_and_cancellation() {
    let provider = ReferenceProvider;
    let coefficients = array([3, 2], vec![1.0, 2.0, 3.0, 2.0, 4.0, 6.0]);
    let right_hand_side = array([3, 1], vec![1.0, 2.0, 3.0]);
    assert!(matches!(
        solve_rectangular_f64(&provider, &coefficients, &right_hand_side),
        Err(LinalgError::RankDeficient {
            provider: "reference",
            operation: "rectangular least-squares solve",
            deficient_diagonal: 2,
            required_rank: 2,
        })
    ));

    let cancellation = AtomicBool::new(true);
    let error = provider
        .solve_rectangular_f64(
            RectangularSolveRequest::new(&coefficients, &right_hand_side)
                .with_cancellation_flag(&cancellation),
        )
        .unwrap_err();
    assert_eq!(
        error,
        LinalgError::Cancelled {
            operation: "rectangular linear solve"
        }
    );
}

#[test]
fn rectangular_zero_rank_semantics_and_lp64_conversion_are_explicit() {
    let provider = ReferenceProvider;
    let under_coefficients = array::<f64>([0, 3], vec![]);
    let under_rhs = array::<f64>([0, 2], vec![]);
    let solution = solve_rectangular_f64(&provider, &under_coefficients, &under_rhs).unwrap();
    assert_eq!(solution.shape().dimensions(), &[3, 2]);
    assert_eq!(solution.as_slice(), &[0.0; 6]);

    let oversized_columns = LP64_MAX_DIMENSION + 1;
    let coefficients = array::<f64>([0, oversized_columns], vec![]);
    let right_hand_side = array::<f64>([0, 0], vec![]);
    let dimensions = validate_rectangular_solve(&RectangularSolveRequest::new(
        &coefficients,
        &right_hand_side,
    ))
    .unwrap();
    assert!(matches!(
        Lp64RectangularSolveDimensions::try_from(&dimensions),
        Err(LinalgError::Lp64DimensionOverflow {
            parameter: "n",
            value,
        }) if value == oversized_columns
    ));
}

use std::sync::atomic::AtomicBool;

use openmat_array::{Complex64, DenseArray, Shape};
use openmat_linalg::{
    LinalgError, ReferenceProvider, solve_underdetermined_basic_complex64,
    solve_underdetermined_basic_f64,
};

fn array<T>(shape: [u64; 2], values: Vec<T>) -> DenseArray<T> {
    DenseArray::from_vec(Shape::new(shape).unwrap(), values).unwrap()
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

#[test]
fn real_basic_solution_reorders_pivots_and_differs_from_minimum_norm() {
    let provider = ReferenceProvider;
    let coefficients = array([2, 3], vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    let right_hand_side = array([2, 2], vec![1.0, 2.0, 2.0, -1.0]);
    let coefficient_alias = coefficients.clone();
    let right_hand_side_alias = right_hand_side.clone();
    let original_coefficients = coefficients.deep_copy();
    let original_right_hand_side = right_hand_side.deep_copy();

    let solution =
        solve_underdetermined_basic_f64(&provider, &coefficients, &right_hand_side, None).unwrap();

    assert_eq!(solution.shape().dimensions(), &[3, 2]);
    for (&actual, expected) in solution
        .as_slice()
        .iter()
        .zip([-1.0, 0.0, 2.0, 3.0, 0.0, -1.0])
    {
        assert_close(actual, expected);
    }
    assert_ne!(&solution.as_slice()[..3], &[0.0, 1.0, 1.0]);
    assert_eq!(solution.as_slice()[1].to_bits(), 0.0_f64.to_bits());
    assert_eq!(solution.as_slice()[4].to_bits(), 0.0_f64.to_bits());

    assert_eq!(coefficients, original_coefficients);
    assert_eq!(right_hand_side, original_right_hand_side);
    assert!(coefficients.shares_storage_with(&coefficient_alias));
    assert!(right_hand_side.shares_storage_with(&right_hand_side_alias));
    assert!(!solution.shares_storage_with(&coefficients));
    assert!(!solution.shares_storage_with(&right_hand_side));
}

#[test]
fn equal_norm_pivot_ties_choose_the_smaller_original_column() {
    let solution = solve_underdetermined_basic_f64(
        &ReferenceProvider,
        &array([1, 2], vec![1.0, -1.0]),
        &array([1, 1], vec![2.0]),
        None,
    )
    .unwrap();

    assert_eq!(solution.as_slice(), &[2.0, 0.0]);
}

#[test]
fn complex_basic_solution_uses_conjugate_orthogonalization_and_multiple_rhs() {
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
        [2, 2],
        vec![
            Complex64::new(1.0, 1.0),
            Complex64::new(-2.0, -1.0),
            Complex64::new(3.0, -1.0),
            Complex64::new(1.0, 2.0),
        ],
    );

    let solution = solve_underdetermined_basic_complex64(
        &ReferenceProvider,
        &coefficients,
        &right_hand_side,
        None,
    )
    .unwrap();

    let expected = [
        Complex64::new(2.0, -1.0),
        Complex64::ZERO,
        Complex64::new(-1.0, 2.0),
        Complex64::new(1.0, 0.0),
        Complex64::ZERO,
        Complex64::new(2.0, -1.0),
    ];
    assert_eq!(solution.shape().dimensions(), &[3, 2]);
    for (&actual, expected) in solution.as_slice().iter().zip(expected) {
        assert_complex_close(actual, expected);
    }
    assert_eq!(solution.as_slice()[1], Complex64::ZERO);
    assert_eq!(solution.as_slice()[4], Complex64::ZERO);
}

#[test]
fn zero_row_system_returns_exact_independently_owned_zeros() {
    let coefficients = array::<f64>([0, 3], vec![]);
    let right_hand_side = array::<f64>([0, 2], vec![]);

    let solution =
        solve_underdetermined_basic_f64(&ReferenceProvider, &coefficients, &right_hand_side, None)
            .unwrap();

    assert_eq!(solution.shape().dimensions(), &[3, 2]);
    assert_eq!(solution.as_slice(), &[0.0; 6]);
    assert!(!solution.shares_storage_with(&coefficients));
    assert!(!solution.shares_storage_with(&right_hand_side));
}

#[test]
fn shape_validation_rejects_non_matrices_wrong_rows_and_non_underdetermined_inputs() {
    let higher_rank = DenseArray::from_vec(Shape::new([1, 1, 2]).unwrap(), vec![1.0, 2.0]).unwrap();
    assert!(matches!(
        solve_underdetermined_basic_f64(
            &ReferenceProvider,
            &higher_rank,
            &array([1, 1], vec![1.0]),
            None,
        ),
        Err(LinalgError::MatrixRequired { .. })
    ));

    assert!(matches!(
        solve_underdetermined_basic_f64(
            &ReferenceProvider,
            &array([2, 3], vec![0.0; 6]),
            &array([3, 1], vec![0.0; 3]),
            None,
        ),
        Err(LinalgError::DimensionMismatch {
            operation: "underdetermined basic solve coefficient and right-hand-side rows",
            left: 2,
            right: 3,
        })
    ));

    assert!(matches!(
        solve_underdetermined_basic_f64(
            &ReferenceProvider,
            &array([2, 2], vec![0.0; 4]),
            &array([2, 1], vec![0.0; 2]),
            None,
        ),
        Err(LinalgError::DimensionMismatch {
            operation: "underdetermined basic solve requires fewer rows than columns",
            left: 2,
            right: 2,
        })
    ));
}

#[test]
fn rank_deficiency_and_cancellation_are_structured() {
    let coefficients = array([2, 3], vec![1.0, 2.0, 2.0, 4.0, 3.0, 6.0]);
    let right_hand_side = array([2, 1], vec![1.0, 2.0]);
    assert_eq!(
        solve_underdetermined_basic_f64(&ReferenceProvider, &coefficients, &right_hand_side, None,)
            .unwrap_err(),
        LinalgError::RankDeficient {
            provider: "openmat-linalg",
            operation: "underdetermined basic solve",
            deficient_diagonal: 2,
            required_rank: 2,
        }
    );

    let cancellation = AtomicBool::new(true);
    assert_eq!(
        solve_underdetermined_basic_f64(
            &ReferenceProvider,
            &array([1, 2], vec![1.0, 0.0]),
            &array([1, 1], vec![1.0]),
            Some(&cancellation),
        )
        .unwrap_err(),
        LinalgError::Cancelled {
            operation: "underdetermined basic solve",
        }
    );
}

#[test]
fn scale_relative_threshold_rejects_a_numerically_short_second_pivot() {
    let coefficients = array([2, 3], vec![1.0e100, 0.0, 0.0, 1.0e70, 1.0e100, 1.0e70]);
    let right_hand_side = array([2, 1], vec![1.0e100, 1.0e70]);

    assert!(matches!(
        solve_underdetermined_basic_f64(&ReferenceProvider, &coefficients, &right_hand_side, None,),
        Err(LinalgError::RankDeficient {
            deficient_diagonal: 2,
            required_rank: 2,
            ..
        })
    ));
}

#[test]
fn zero_row_result_dimension_overflow_is_an_allocation_error() {
    let coefficients = array::<f64>([0, u64::MAX], vec![]);
    let right_hand_side = array::<f64>([0, 2], vec![]);

    assert!(matches!(
        solve_underdetermined_basic_f64(&ReferenceProvider, &coefficients, &right_hand_side, None,),
        Err(LinalgError::AllocationFailure {
            operation: "underdetermined basic solve result dimensions",
            elements: u64::MAX,
        })
    ));
}

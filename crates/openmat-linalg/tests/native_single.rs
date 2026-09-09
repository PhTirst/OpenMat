use std::sync::atomic::AtomicBool;

use openmat_array::{Complex32, DenseArray, Shape};
use openmat_linalg::{
    GemmOptions, GemmRequest, LinalgError, LinalgProvider, MatrixTranspose,
    RectangularSolveRequest, ReferenceProvider, SolveRequest, matrix_multiply_complex32,
    matrix_multiply_f32, solve_complex32, solve_f32, solve_rectangular_complex32,
    solve_rectangular_f32, solve_underdetermined_basic_complex32, solve_underdetermined_basic_f32,
};

fn array<T>(dimensions: [u64; 2], data: Vec<T>) -> DenseArray<T> {
    DenseArray::from_vec(Shape::new(dimensions).unwrap(), data).unwrap()
}

fn assert_close(actual: f32, expected: f32) {
    let tolerance = 3.0e-5_f32 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, received {actual}"
    );
}

fn assert_complex_close(actual: Complex32, expected: Complex32) {
    assert_close(actual.re, expected.re);
    assert_close(actual.im, expected.im);
}

#[test]
fn reference_f32_gemm_is_column_major_binary32_and_cow_safe() {
    let provider = ReferenceProvider;
    let left = array([1, 3], vec![1.0e10_f32, 1.0, -1.0e10_f32]);
    let right = array([3, 1], vec![1.0_f32; 3]);

    let product = matrix_multiply_f32(&provider, &left, &right).unwrap();

    // A binary64 compatibility bridge produces 1.0 here. Sequential f32 FMA
    // accumulation loses the middle one before the final cancellation.
    assert_eq!(product.as_slice()[0].to_bits(), 0.0_f32.to_bits());

    let left = array([2, 3], vec![1.0_f32, 4.0, 2.0, 5.0, 3.0, 6.0]);
    let right = array([2, 1], vec![10.0_f32, 20.0]);
    let initial = array([3, 1], vec![1.0_f32, 1.0, 1.0]);
    let mut output = initial.clone();
    provider
        .gemm_f32(
            GemmRequest::new(
                &left,
                &right,
                GemmOptions::new(MatrixTranspose::Transpose, MatrixTranspose::None, 2.0, 3.0),
            ),
            &mut output,
        )
        .unwrap();
    assert!(!output.shares_storage_with(&initial));
    assert_eq!(initial.as_slice(), &[1.0, 1.0, 1.0]);
    for (&actual, expected) in output.as_slice().iter().zip([183.0, 243.0, 303.0]) {
        assert_close(actual, expected);
    }
}

#[test]
fn reference_complex32_gemm_honors_conjugation_and_empty_shapes() {
    let provider = ReferenceProvider;
    let vector = array(
        [2, 1],
        vec![Complex32::new(1.0, 1.0), Complex32::new(2.0, -1.0)],
    );
    let mut output = array([1, 1], vec![Complex32::ZERO]);
    provider
        .gemm_complex32(
            GemmRequest::new(
                &vector,
                &vector,
                GemmOptions::new(
                    MatrixTranspose::ConjugateTranspose,
                    MatrixTranspose::None,
                    Complex32::new(1.0, 0.0),
                    Complex32::ZERO,
                ),
            ),
            &mut output,
        )
        .unwrap();
    assert_complex_close(output.as_slice()[0], Complex32::new(7.0, 0.0));

    let product = matrix_multiply_complex32(
        &provider,
        &array::<Complex32>([2, 0], vec![]),
        &array::<Complex32>([0, 3], vec![]),
    )
    .unwrap();
    assert_eq!(product.shape().dimensions(), &[2, 3]);
    assert_eq!(product.as_slice(), &[Complex32::ZERO; 6]);
}

#[test]
fn reference_f32_square_solve_supports_pivoting_multiple_rhs_and_cow() {
    let coefficients = array(
        [3, 3],
        vec![0.0_f32, 1.0, 2.0, 2.0, -2.0, 3.0, 1.0, -3.0, 1.0],
    );
    let right_hand_side = array([3, 2], vec![3.0_f32, 0.0, 7.0, -1.0, -1.0, 5.0]);
    let coefficient_alias = coefficients.clone();
    let right_hand_side_alias = right_hand_side.clone();

    let solution = solve_f32(&ReferenceProvider, &coefficients, &right_hand_side).unwrap();

    assert_eq!(solution.shape().dimensions(), &[3, 2]);
    for (&actual, expected) in solution
        .as_slice()
        .iter()
        .zip([1.0, 2.0, -1.0, 4.0, -2.0, 3.0])
    {
        assert_close(actual, expected);
    }
    assert!(coefficients.shares_storage_with(&coefficient_alias));
    assert!(right_hand_side.shares_storage_with(&right_hand_side_alias));
    assert!(!solution.shares_storage_with(&coefficients));
    assert!(!solution.shares_storage_with(&right_hand_side));
}

#[test]
fn reference_complex32_square_solve_stays_in_complex_binary32() {
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

    let actual = solve_complex32(&ReferenceProvider, &coefficients, &right_hand_side).unwrap();

    for (&actual, &expected) in actual.as_slice().iter().zip(expected.as_slice()) {
        assert_complex_close(actual, expected);
    }
    assert!(!actual.shares_storage_with(&coefficients));
    assert!(!actual.shares_storage_with(&right_hand_side));
}

#[test]
fn reference_single_solve_empty_singular_and_cancellation_boundaries_match_double() {
    let empty_rhs = array::<f32>([0, 4], vec![]);
    let empty = solve_f32(
        &ReferenceProvider,
        &array::<f32>([0, 0], vec![]),
        &empty_rhs,
    )
    .unwrap();
    assert_eq!(empty.shape(), empty_rhs.shape());
    assert!(!empty.shares_storage_with(&empty_rhs));

    assert!(matches!(
        solve_f32(
            &ReferenceProvider,
            &array([2, 2], vec![1.0_f32, 2.0, 2.0, 4.0]),
            &array([2, 1], vec![3.0_f32, 6.0]),
        ),
        Err(LinalgError::SingularMatrix {
            provider: "reference",
            pivot: 2,
            ..
        })
    ));

    let cancellation = AtomicBool::new(true);
    let coefficients = array([1, 1], vec![2.0_f32]);
    let right_hand_side = array([1, 1], vec![4.0_f32]);
    assert_eq!(
        ReferenceProvider
            .solve_f32(
                SolveRequest::new(&coefficients, &right_hand_side)
                    .with_cancellation_flag(&cancellation),
            )
            .unwrap_err(),
        LinalgError::Cancelled {
            operation: "general linear solve",
        }
    );
}

#[test]
fn reference_f32_rectangular_paths_use_binary32_rank_policy() {
    let coefficients = array([3, 2], vec![1.0_f32, 1.0, 1.0, 0.0, 1.0, 2.0]);
    let right_hand_side = array([3, 2], vec![1.0_f32, 2.0, 2.0, 2.0, 0.0, 1.0]);
    let solution =
        solve_rectangular_f32(&ReferenceProvider, &coefficients, &right_hand_side).unwrap();
    for (&actual, expected) in solution
        .as_slice()
        .iter()
        .zip([7.0_f32 / 6.0, 0.5, 1.5, -0.5])
    {
        assert_close(actual, expected);
    }
    assert!(!solution.shares_storage_with(&coefficients));
    assert!(!solution.shares_storage_with(&right_hand_side));

    // This second diagonal is well above binary64 epsilon but below the
    // binary32-scaled full-rank threshold. A widened QR path accepts it.
    let short = array([3, 2], vec![1.0_f32, 0.0, 0.0, 0.0, 1.0e-8, 0.0]);
    assert!(matches!(
        solve_rectangular_f32(
            &ReferenceProvider,
            &short,
            &array([3, 1], vec![1.0_f32, 1.0e-8, 0.0]),
        ),
        Err(LinalgError::RankDeficient {
            deficient_diagonal: 2,
            required_rank: 2,
            ..
        })
    ));

    let zero_rank = solve_rectangular_f32(
        &ReferenceProvider,
        &array::<f32>([0, 3], vec![]),
        &array::<f32>([0, 2], vec![]),
    )
    .unwrap();
    assert_eq!(zero_rank.shape().dimensions(), &[3, 2]);
    assert_eq!(zero_rank.as_slice(), &[0.0_f32; 6]);
}

#[test]
fn reference_complex32_rectangular_and_cancellation_paths_work() {
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
    let solution =
        solve_rectangular_complex32(&ReferenceProvider, &coefficients, &right_hand_side).unwrap();
    for (&actual, expected) in solution.as_slice().iter().zip([
        Complex32::new(1.0, 1.0),
        Complex32::new(2.0, -1.0),
        Complex32::new(0.0, -1.0),
    ]) {
        assert_complex_close(actual, expected);
    }

    let cancellation = AtomicBool::new(true);
    assert_eq!(
        ReferenceProvider
            .solve_rectangular_complex32(
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
fn native_single_basic_solve_supports_real_complex_multi_rhs_and_zero_rows() {
    let real = solve_underdetermined_basic_f32(
        &ReferenceProvider,
        &array([2, 3], vec![1.0_f32, 0.0, 0.0, 1.0, 1.0, 1.0]),
        &array([2, 2], vec![1.0_f32, 2.0, 2.0, -1.0]),
        None,
    )
    .unwrap();
    for (&actual, expected) in real
        .as_slice()
        .iter()
        .zip([-1.0_f32, 0.0, 2.0, 3.0, 0.0, -1.0])
    {
        assert_close(actual, expected);
    }
    assert_eq!(real.as_slice()[1].to_bits(), 0.0_f32.to_bits());

    let complex = solve_underdetermined_basic_complex32(
        &ReferenceProvider,
        &array(
            [1, 2],
            vec![Complex32::new(1.0, 1.0), Complex32::new(0.0, 1.0)],
        ),
        &array(
            [1, 2],
            vec![Complex32::new(2.0, 2.0), Complex32::new(-1.0, 1.0)],
        ),
        None,
    )
    .unwrap();
    assert_eq!(complex.shape().dimensions(), &[2, 2]);
    assert_eq!(complex.as_slice()[1], Complex32::ZERO);
    assert_eq!(complex.as_slice()[3], Complex32::ZERO);

    let zero = solve_underdetermined_basic_f32(
        &ReferenceProvider,
        &array::<f32>([0, 3], vec![]),
        &array::<f32>([0, 2], vec![]),
        None,
    )
    .unwrap();
    assert_eq!(zero.shape().dimensions(), &[3, 2]);
    assert_eq!(zero.as_slice(), &[0.0_f32; 6]);
}

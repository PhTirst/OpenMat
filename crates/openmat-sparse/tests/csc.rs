use std::sync::atomic::{AtomicBool, Ordering};

use openmat_array::{ArrayData, Complex64, Logical};
use openmat_sparse::{CooEntry, CscMatrix, SparseArrayData, SparseError, SparseInvariant};

#[test]
fn coo_normalizes_column_order_duplicates_and_explicit_zero() {
    let sparse = SparseArrayData::try_from_f64_coo(
        3,
        2,
        vec![
            CooEntry::new(1, 0, 4.0),
            CooEntry::new(0, 0, 5.0),
            CooEntry::new(1, 0, -4.0),
            CooEntry::new(2, 1, f64::INFINITY),
            CooEntry::new(2, 1, f64::NEG_INFINITY),
        ],
        5,
        None,
    )
    .unwrap();
    let matrix = sparse.as_f64().unwrap();
    assert_eq!(matrix.col_offsets(), &[0, 1, 2]);
    assert_eq!(matrix.row_indices(), &[0, 2]);
    assert!((matrix.values()[0] - 5.0).abs() < f64::EPSILON);
    assert!(matrix.values()[1].is_nan());
    assert_eq!(matrix.nzmax(), 5);
}

#[test]
fn duplicate_floating_values_sum_in_matlab_input_order() {
    let cancelled = SparseArrayData::try_from_f64_coo(
        1,
        1,
        vec![
            CooEntry::new(0, 0, 1.0e16),
            CooEntry::new(0, 0, 1.0),
            CooEntry::new(0, 0, -1.0e16),
        ],
        1,
        None,
    )
    .unwrap();
    assert_eq!(cancelled.nnz(), 0);

    let retained = SparseArrayData::try_from_f64_coo(
        1,
        1,
        vec![
            CooEntry::new(0, 0, 1.0e16),
            CooEntry::new(0, 0, -1.0e16),
            CooEntry::new(0, 0, 1.0),
        ],
        1,
        None,
    )
    .unwrap();
    assert_eq!(retained.as_f64().unwrap().values(), &[1.0]);
}

#[test]
fn logical_duplicates_use_boolean_combination_and_false_is_removed() {
    let sparse = SparseArrayData::try_from_logical_coo(
        2,
        2,
        vec![
            CooEntry::new(0, 0, Logical::FALSE),
            CooEntry::new(0, 0, Logical::TRUE),
            CooEntry::new(0, 0, Logical::FALSE),
        ],
        3,
        None,
    )
    .unwrap();
    let matrix = sparse.as_logical().unwrap();
    assert_eq!(matrix.nnz(), 1);
    assert!(matrix.values()[0].get());
}

#[test]
fn complex_duplicate_that_becomes_real_is_canonicalized() {
    let sparse = SparseArrayData::try_from_complex_f64_coo(
        2,
        2,
        vec![
            CooEntry::new(1, 0, Complex64::new(1.0, 2.0)),
            CooEntry::new(1, 0, Complex64::new(1.0, -2.0)),
        ],
        2,
        None,
    )
    .unwrap();
    assert!(!sparse.is_complex());
    assert_eq!(sparse.as_f64().unwrap().values(), &[2.0]);
}

#[test]
fn empty_rectangles_and_spalloc_accounting_are_exact() {
    for (rows, columns) in [(0, 0), (0, 3), (3, 0)] {
        let sparse = SparseArrayData::try_spalloc(rows, columns, 0, None).unwrap();
        assert_eq!(sparse.shape().dimensions(), &[rows, columns]);
        assert_eq!(sparse.nnz(), 0);
        assert_eq!(sparse.nzmax(), 1);
    }
    let sparse = SparseArrayData::try_spalloc(4, 5, 9, None).unwrap();
    assert_eq!(sparse.payload_bytes(), Some(192));
}

#[test]
fn transpose_and_conjugate_transpose_preserve_csc() {
    let sparse = SparseArrayData::try_from_complex_f64_coo(
        2,
        3,
        vec![
            CooEntry::new(0, 1, Complex64::new(1.0, 2.0)),
            CooEntry::new(1, 0, Complex64::new(3.0, -4.0)),
        ],
        2,
        None,
    )
    .unwrap();
    let transpose = sparse.try_transpose(false, None).unwrap();
    let conjugate = sparse.try_transpose(true, None).unwrap();
    assert_eq!(transpose.shape().dimensions(), &[3, 2]);
    assert_eq!(
        transpose
            .as_complex_f64()
            .unwrap()
            .get_subscripts_one_based(2, 1)
            .unwrap(),
        Some(&Complex64::new(1.0, 2.0))
    );
    assert_eq!(
        conjugate
            .as_complex_f64()
            .unwrap()
            .get_subscripts_one_based(2, 1)
            .unwrap(),
        Some(&Complex64::new(1.0, -2.0))
    );
}

#[test]
fn scalar_indexing_is_sparse_and_absent_complex_becomes_real() {
    let sparse = SparseArrayData::try_from_complex_f64_coo(
        2,
        2,
        vec![CooEntry::new(0, 0, Complex64::new(1.0, 2.0))],
        1,
        None,
    )
    .unwrap();
    let present = sparse.try_index_linear_one_based(1, None).unwrap();
    let absent = sparse.try_index_linear_one_based(2, None).unwrap();
    assert!(present.is_complex());
    assert!(!absent.is_complex());
    assert_eq!(present.shape().dimensions(), &[1, 1]);
    assert_eq!(absent.nnz(), 0);
}

#[test]
fn full_round_trip_preserves_nan_inf_and_logical_class() {
    let dense = ArrayData::Logical(
        openmat_array::DenseArray::from_vec(
            openmat_array::Shape::new([2, 2]).unwrap(),
            vec![Logical::FALSE, Logical::TRUE, Logical::TRUE, Logical::FALSE],
        )
        .unwrap(),
    );
    let sparse = SparseArrayData::try_from_dense(&dense, None).unwrap();
    assert_eq!(sparse.dtype(), openmat_array::DType::Logical);
    assert_eq!(sparse.try_to_dense(None).unwrap(), dense);
}

#[test]
fn canonical_parts_reject_unsorted_rows_and_explicit_zero() {
    let error =
        CscMatrix::try_from_canonical_parts(3, 1, vec![0, 2], vec![2, 1], vec![1.0, 2.0], 2, None)
            .unwrap_err();
    assert_eq!(error, SparseError::InvalidCsc(SparseInvariant::RowOrder));
    let error = CscMatrix::try_from_canonical_parts(1, 1, vec![0, 1], vec![0], vec![0.0], 1, None)
        .unwrap_err();
    assert_eq!(
        error,
        SparseError::InvalidCsc(SparseInvariant::ExplicitZero)
    );
}

#[test]
fn checked_allocation_and_cancellation_fail_without_partial_value() {
    let cancelled = AtomicBool::new(true);
    let error = SparseArrayData::try_spalloc(3, 3, 4, Some(&cancelled)).unwrap_err();
    assert_eq!(error, SparseError::Cancelled);
    cancelled.store(false, Ordering::Release);
    let error = SparseArrayData::try_spalloc(0, u64::MAX, 0, None).unwrap_err();
    assert!(matches!(
        error,
        SparseError::HostLengthOverflow { .. } | SparseError::Allocation { .. }
    ));
}

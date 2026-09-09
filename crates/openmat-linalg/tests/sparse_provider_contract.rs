use std::sync::atomic::AtomicBool;

use openmat_linalg::{
    CscMatrixRef, CscPatternRef, OwnedCscMatrix, SparseCancellation, SparseError, SparseIndexWidth,
    SparsePhase,
};

#[test]
fn csc_views_reject_noncanonical_structure() {
    assert!(matches!(
        CscPatternRef::new(3, 1, &[0, 2], &[1, 1]),
        Err(SparseError::InvalidCsc {
            invariant: "strictly increasing in-range row indices",
            column: Some(0)
        })
    ));
    assert!(matches!(
        CscMatrixRef::new(2, 1, &[0, 1], &[0], &[1.0, 2.0]),
        Err(SparseError::InvalidCsc {
            invariant: "value count equals row-index count",
            column: None
        })
    ));
}

#[test]
fn signed_provider_index_width_is_checked_before_attachment() {
    let pattern = CscPatternRef::new(i32::MAX as u64 + 1, 0, &[0], &[]).unwrap();
    assert_eq!(
        pattern.check_index_width(SparseIndexWidth::I32),
        Err(SparseError::IndexWidthOverflow {
            width: SparseIndexWidth::I32,
            parameter: "rows",
            value: i32::MAX as u64 + 1,
        })
    );
    assert_eq!(pattern.check_index_width(SparseIndexWidth::I64), Ok(()));
}

#[test]
fn owned_results_reborrow_without_changing_csc_order() {
    let matrix =
        OwnedCscMatrix::new(3, 2, vec![0, 1, 3], vec![2, 0, 1], vec![4.0, 5.0, 6.0]).unwrap();
    let borrowed = matrix.as_ref();
    assert_eq!(borrowed.pattern().col_offsets(), &[0, 1, 3]);
    assert_eq!(borrowed.pattern().row_indices(), &[2, 0, 1]);
    assert_eq!(borrowed.values(), &[4.0, 5.0, 6.0]);
}

#[test]
fn cancellation_reports_the_observed_phase() {
    let flag = AtomicBool::new(true);
    assert_eq!(
        SparseCancellation::from_atomic(&flag).checkpoint("test operation", SparsePhase::Numeric),
        Err(SparseError::Cancelled {
            operation: "test operation",
            phase: SparsePhase::Numeric,
        })
    );
    assert_eq!(
        SparseCancellation::never().checkpoint("test operation", SparsePhase::Execution),
        Ok(())
    );
}

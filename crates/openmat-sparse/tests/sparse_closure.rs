use openmat_array::{ArrayData, Complex64, DenseArray, Shape};
use openmat_sparse::{CooEntry, SparseArrayData};

fn real_sparse(rows: u64, columns: u64, entries: &[(u64, u64, f64)]) -> SparseArrayData {
    SparseArrayData::try_from_f64_coo(
        rows,
        columns,
        entries
            .iter()
            .map(|&(row, column, value)| CooEntry::new(row, column, value))
            .collect(),
        entries.len(),
        None,
    )
    .unwrap()
}

fn dense(rows: u64, columns: u64, values: Vec<f64>) -> ArrayData {
    ArrayData::F64(DenseArray::from_vec(Shape::new([rows, columns]).unwrap(), values).unwrap())
}

fn full_real(value: &SparseArrayData) -> Vec<f64> {
    let ArrayData::F64(array) = value.try_to_dense(None).unwrap() else {
        panic!("expected real double sparse data")
    };
    array.as_slice().to_vec()
}

#[test]
fn sparse_sparse_arithmetic_preserves_csc_and_ieee_division() {
    let lhs = real_sparse(2, 2, &[(0, 0, 1.0), (1, 1, 2.0)]);
    let rhs = real_sparse(2, 2, &[(1, 0, 5.0), (0, 1, 4.0)]);

    let sum = lhs.try_add_sparse(&rhs, None).unwrap();
    assert_eq!(sum.nnz(), 4);
    assert_eq!(full_real(&sum), vec![1.0, 5.0, 4.0, 2.0]);

    let product = lhs.try_element_multiply_sparse(&rhs, None).unwrap();
    assert_eq!(product.nnz(), 0);

    let zeros = SparseArrayData::try_spalloc(2, 2, 0, None).unwrap();
    let quotient = zeros.try_element_divide_sparse(&zeros, None).unwrap();
    assert_eq!(quotient.nnz(), 4);
    assert!(full_real(&quotient).into_iter().all(f64::is_nan));
}

#[test]
fn complex_sparse_division_by_zero_matches_measured_components() {
    let numerator = SparseArrayData::try_from_complex_f64_coo(
        1,
        1,
        vec![CooEntry::new(0, 0, Complex64::new(1.0, 2.0))],
        1,
        None,
    )
    .unwrap();
    let zero = SparseArrayData::try_spalloc(1, 1, 0, None).unwrap();
    let result = numerator.try_element_divide_sparse(&zero, None).unwrap();
    let value = result.as_complex_f64().unwrap().values()[0];
    assert!(value.re.is_infinite() && value.re.is_sign_positive());
    assert!(value.im.is_infinite() && value.im.is_sign_positive());
}

#[test]
fn sparse_dense_storage_rules_match_measured_r2022b_behavior() {
    let sparse = real_sparse(2, 2, &[(0, 0, 1.0), (1, 1, 2.0)]);
    let full = dense(2, 2, vec![3.0, 5.0, 4.0, 6.0]);

    let added = sparse.try_add_dense(&full, true, None).unwrap();
    let ArrayData::F64(added) = added else {
        panic!("addition must materialize full double storage")
    };
    assert_eq!(added.as_slice(), &[4.0, 5.0, 4.0, 8.0]);

    let multiplied = sparse
        .try_element_multiply_dense(&full, true, None)
        .unwrap();
    assert_eq!(multiplied.nnz(), 2);
    assert_eq!(full_real(&multiplied), vec![3.0, 0.0, 0.0, 12.0]);

    let divided = sparse.try_element_divide_by_dense(&full, None).unwrap();
    assert_eq!(divided.nnz(), 2);
    assert_eq!(full_real(&divided), vec![1.0 / 3.0, 0.0, 0.0, 1.0 / 3.0]);

    let reverse = sparse.try_dense_element_divide_by(&full, None).unwrap();
    let ArrayData::F64(reverse) = reverse else {
        panic!("full divided by sparse must stay full")
    };
    assert!((reverse.as_slice()[0] - 3.0).abs() < f64::EPSILON);
    assert!(reverse.as_slice()[1].is_infinite());
    assert!(reverse.as_slice()[2].is_infinite());
    assert!((reverse.as_slice()[3] - 3.0).abs() < f64::EPSILON);
}

#[test]
fn sparse_matrix_product_is_canonical_and_cancels_duplicate_products() {
    let lhs = real_sparse(2, 3, &[(0, 0, 1.0), (1, 0, 2.0), (0, 1, -1.0), (0, 2, 3.0)]);
    let rhs = real_sparse(3, 2, &[(0, 0, 4.0), (1, 0, 4.0), (2, 1, 5.0)]);
    let result = lhs.try_matrix_multiply(&rhs, None).unwrap();
    let matrix = result.as_f64().unwrap();
    assert_eq!(matrix.col_offsets(), &[0, 1, 2]);
    assert_eq!(matrix.row_indices(), &[1, 0]);
    assert_eq!(matrix.values(), &[8.0, 15.0]);
    assert_eq!(full_real(&result), vec![0.0, 8.0, 15.0, 0.0]);
}

#[test]
fn gather_assignment_reshape_and_concat_preserve_column_major_order() {
    let source = real_sparse(2, 3, &[(0, 0, 10.0), (1, 1, 50.0), (0, 2, 30.0)]);
    let gathered = source
        .try_gather_offsets(&[5, 0, 2], &Shape::new([1, 3]).unwrap(), None)
        .unwrap();
    assert_eq!(full_real(&gathered), vec![0.0, 10.0, 0.0]);

    let supplied = real_sparse(1, 3, &[(0, 1, 7.0), (0, 2, 9.0)]);
    let assigned = source
        .try_assign_offsets(&[0, 3, 5], &supplied, None)
        .unwrap();
    assert_eq!(full_real(&assigned), vec![0.0, 0.0, 0.0, 7.0, 30.0, 9.0]);

    let reshaped = source.try_reshape_2d(3, 2, None).unwrap();
    assert_eq!(full_real(&reshaped), vec![10.0, 0.0, 0.0, 50.0, 30.0, 0.0]);

    let tail = SparseArrayData::try_from_dense(&dense(2, 1, vec![1.0, 2.0]), None).unwrap();
    let concatenated = SparseArrayData::try_concatenate_2d(&[source, tail], 1, None).unwrap();
    assert_eq!(concatenated.shape().dimensions(), &[2, 4]);
    assert_eq!(
        full_real(&concatenated),
        vec![10.0, 0.0, 0.0, 50.0, 30.0, 0.0, 1.0, 2.0]
    );
}

#[test]
fn measured_linear_row_and_column_deletions_remap_surviving_entries() {
    let source = real_sparse(2, 3, &[(0, 0, 10.0), (1, 1, 50.0), (0, 2, 30.0)]);
    let linear = source
        .try_delete_linear_offsets(&[1, 4], &Shape::new([1, 4]).unwrap(), None)
        .unwrap();
    assert_eq!(linear.shape().dimensions(), &[1, 4]);
    assert_eq!(full_real(&linear), vec![10.0, 0.0, 50.0, 0.0]);

    let rows = source.try_delete_rows(&[1], None).unwrap();
    assert_eq!(rows.shape().dimensions(), &[1, 3]);
    assert_eq!(full_real(&rows), vec![10.0, 0.0, 30.0]);

    let columns = source.try_delete_columns(&[1], None).unwrap();
    assert_eq!(columns.shape().dimensions(), &[2, 2]);
    assert_eq!(full_real(&columns), vec![10.0, 0.0, 30.0, 0.0]);
}

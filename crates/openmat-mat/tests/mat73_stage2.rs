use openmat_array::{ArrayData, Complex64, DenseArray, Logical, Shape};
use openmat_mat::{
    Mat73Hyperslab, MatErrorKind, MatLimits, MatVariable, MatVersion, decode, encode,
    read_v73_dense_hyperslab, write_v73_dense_hyperslab,
};
use openmat_value::{
    CellArray, CooEntry, FieldName, SparseArrayData, StringValue, StructArray, Value,
};

fn assert_sparse_semantics(actual: &SparseArrayData, expected: &SparseArrayData) {
    assert_eq!(actual.shape(), expected.shape());
    assert_eq!(actual.dtype(), expected.dtype());
    assert_eq!(actual.nnz(), expected.nnz());
    for position in 0..actual.nnz() {
        assert_eq!(
            actual.stored_entry(position),
            expected.stored_entry(position)
        );
    }
}

fn sparse_fixtures() -> Vec<MatVariable> {
    let real = SparseArrayData::try_from_f64_coo(
        3,
        5,
        vec![
            CooEntry::new(0, 0, 1.5),
            CooEntry::new(2, 1, -2.0),
            CooEntry::new(1, 3, 9.0),
        ],
        8,
        None,
    )
    .unwrap();
    let complex = SparseArrayData::try_from_complex_f64_coo(
        4,
        3,
        vec![
            CooEntry::new(1, 0, Complex64::new(3.0, 4.0)),
            CooEntry::new(3, 2, Complex64::new(-5.0, 2.0)),
        ],
        7,
        None,
    )
    .unwrap();
    let logical = SparseArrayData::try_from_logical_coo(
        2,
        3,
        vec![
            CooEntry::new(0, 0, Logical::TRUE),
            CooEntry::new(1, 1, Logical::TRUE),
            CooEntry::new(0, 2, Logical::TRUE),
        ],
        3,
        None,
    )
    .unwrap();
    let empty_rows = SparseArrayData::try_spalloc(0, 4, 9, None).unwrap();
    let empty_columns = SparseArrayData::try_spalloc(3, 0, 5, None).unwrap();
    let allocated_empty = SparseArrayData::try_spalloc(5, 6, 11, None).unwrap();
    let logical_empty = SparseArrayData::try_from_logical_coo(2, 0, Vec::new(), 0, None).unwrap();
    let nested = CellArray::from_values(
        Shape::new([2, 1]).unwrap(),
        vec![Value::Sparse(real.clone()), Value::Sparse(complex.clone())],
    )
    .unwrap();
    let nested_struct = StructArray::from_columns(
        Shape::new([1, 1]).unwrap(),
        vec![
            FieldName::new("left").unwrap(),
            FieldName::new("right").unwrap(),
        ],
        vec![
            vec![Value::Sparse(real.clone())],
            vec![Value::Sparse(logical.clone())],
        ],
    )
    .unwrap();
    vec![
        MatVariable::new("real", Value::Sparse(real)),
        MatVariable::new("complex", Value::Sparse(complex)),
        MatVariable::new("logical", Value::Sparse(logical)),
        MatVariable::new("empty_rows", Value::Sparse(empty_rows)),
        MatVariable::new("empty_columns", Value::Sparse(empty_columns)),
        MatVariable::new("allocated_empty", Value::Sparse(allocated_empty)),
        MatVariable::new("logical_empty", Value::Sparse(logical_empty)),
        MatVariable::new("nested", Value::Cell(nested)),
        MatVariable::new("nested_struct", Value::Struct(nested_struct)),
    ]
}

#[test]
fn sparse_real_complex_logical_empty_and_nested_round_trip() {
    let fixtures = sparse_fixtures();
    let image = encode(&fixtures, MatVersion::V73).unwrap();
    let decoded = decode(&image, MatLimits::default()).unwrap();

    assert_eq!(decoded.len(), fixtures.len());
    for actual in &decoded {
        let expected = fixtures
            .iter()
            .find(|expected| expected.name == actual.name)
            .unwrap();
        let (Value::Sparse(actual), Value::Sparse(expected)) = (&actual.value, &expected.value)
        else {
            if actual.name == "nested" {
                let (Value::Cell(actual), Value::Cell(expected)) = (&actual.value, &expected.value)
                else {
                    unreachable!();
                };
                assert_eq!(actual.shape(), expected.shape());
                for (actual, expected) in actual.values().iter().zip(expected.values()) {
                    let (Value::Sparse(actual), Value::Sparse(expected)) = (actual, expected)
                    else {
                        panic!("expected nested sparse values");
                    };
                    assert_sparse_semantics(actual, expected);
                }
                continue;
            }
            if actual.name == "nested_struct" {
                let (Value::Struct(actual), Value::Struct(expected)) =
                    (&actual.value, &expected.value)
                else {
                    unreachable!();
                };
                assert_eq!(actual.shape(), expected.shape());
                assert_eq!(actual.field_names(), expected.field_names());
                for field in 0..actual.field_count() {
                    let Value::Sparse(actual) = &actual.field_values(field).unwrap()[0] else {
                        panic!("expected actual struct sparse field");
                    };
                    let Value::Sparse(expected) = &expected.field_values(field).unwrap()[0] else {
                        panic!("expected expected struct sparse field");
                    };
                    assert_sparse_semantics(actual, expected);
                }
                continue;
            }
            panic!("expected sparse value for {}", actual.name);
        };
        assert_sparse_semantics(actual, expected);
    }
}

#[test]
fn dense_hyperslab_read_and_write_preserve_column_major_order() {
    let shape = Shape::new([3, 4]).unwrap();
    let dense = DenseArray::from_vec(shape, (1..=12).map(f64::from).collect()).unwrap();
    let image = encode(
        &[MatVariable::new("a", Value::Array(ArrayData::F64(dense)))],
        MatVersion::V73,
    )
    .unwrap();
    let selection = Mat73Hyperslab::new(vec![1, 1], vec![2, 2]).unwrap();

    let partial =
        read_v73_dense_hyperslab(&image, "a", &selection, MatLimits::default(), None).unwrap();
    let Value::Array(ArrayData::F64(partial)) = partial else {
        panic!("expected dense double hyperslab");
    };
    assert_eq!(partial.shape().dimensions(), &[2, 2]);
    assert_eq!(partial.as_slice(), &[5.0, 6.0, 8.0, 9.0]);

    let replacement =
        DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![50.0, 60.0, 80.0, 90.0]).unwrap();
    let updated = write_v73_dense_hyperslab(
        &image,
        "a",
        &selection,
        &Value::Array(ArrayData::F64(replacement)),
        MatLimits::default(),
        None,
    )
    .unwrap();
    let decoded = decode(&updated, MatLimits::default()).unwrap();
    let Value::Array(ArrayData::F64(updated)) = &decoded[0].value else {
        panic!("expected updated double array");
    };
    assert_eq!(
        updated.as_slice(),
        &[
            1.0, 2.0, 3.0, 4.0, 50.0, 60.0, 7.0, 80.0, 90.0, 10.0, 11.0, 12.0
        ]
    );
}

#[test]
fn hyperslab_bounds_and_storage_type_are_checked() {
    let image = encode(
        &[MatVariable::new("a", Value::Double(1.0))],
        MatVersion::V73,
    )
    .unwrap();
    let outside = Mat73Hyperslab::new(vec![0, 1], vec![1, 1]).unwrap();
    let error =
        read_v73_dense_hyperslab(&image, "a", &outside, MatLimits::default(), None).unwrap_err();
    assert_eq!(error.kind(), MatErrorKind::InvalidValue);

    let scalar = Mat73Hyperslab::new(vec![0, 0], vec![1, 1]).unwrap();
    let error = write_v73_dense_hyperslab(
        &image,
        "a",
        &scalar,
        &Value::Logical(true),
        MatLimits::default(),
        None,
    )
    .unwrap_err();
    assert_eq!(error.kind(), MatErrorKind::InvalidValue);
}

#[test]
fn object_backed_string_writes_are_structurally_unsupported() {
    let error = encode(
        &[MatVariable::new(
            "text",
            Value::String(StringValue::scalar("exact UTF-16 runtime string")),
        )],
        MatVersion::V73,
    )
    .unwrap_err();
    assert_eq!(error.kind(), MatErrorKind::Unsupported);
    assert!(error.message().contains("string"));
}

#[test]
#[ignore = "requires OPENMAT_MATLAB_FIXTURE_DIR from the local R2022b probe"]
fn matlab_r2022b_stage2_fixture_decodes_sparse_and_rejects_mcos_families() {
    let directory = std::env::var_os("OPENMAT_MATLAB_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .expect("OPENMAT_MATLAB_FIXTURE_DIR must name the probe directory");
    let sparse = std::fs::read(directory.join("matlab-stage2-sparse-v73.mat")).unwrap();
    let decoded = decode(&sparse, MatLimits::default()).unwrap();
    assert_eq!(decoded.len(), 8);
    assert_eq!(
        decoded
            .iter()
            .filter(|variable| matches!(variable.value, Value::Sparse(_)))
            .count(),
        7
    );
    let Value::Struct(structure) = &decoded
        .iter()
        .find(|variable| variable.name == "structSparse")
        .unwrap()
        .value
    else {
        panic!("expected MATLAB-authored scalar struct");
    };
    assert_eq!(structure.shape().dimensions(), &[1, 1]);
    assert!(matches!(
        structure.field_values(0).unwrap()[0],
        Value::Sparse(_)
    ));
    assert!(matches!(
        structure.field_values(1).unwrap()[0],
        Value::Sparse(_)
    ));
    let openmat = encode(&sparse_fixtures(), MatVersion::V73).unwrap();
    std::fs::write(directory.join("openmat-stage2-sparse-v73.mat"), openmat).unwrap();

    let image = std::fs::read(directory.join("matlab-stage2-v73.mat")).unwrap();
    let error = decode(&image, MatLimits::default()).unwrap_err();
    assert_eq!(error.kind(), MatErrorKind::Unsupported, "{error}");
    assert!(error.message().contains("string"));

    let unsupported = std::fs::read(directory.join("matlab-stage2-unsupported-v73.mat")).unwrap();
    let error = decode(&unsupported, MatLimits::default()).unwrap_err();
    assert_eq!(error.kind(), MatErrorKind::Unsupported);
}

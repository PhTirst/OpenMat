use openmat_array::{ArrayData, CharCodeUnit, Complex32, ComplexInteger, DType, DenseArray, Shape};
use openmat_value::{StringArray, StringElement, StringValue, Value, ValueKind};

#[test]
fn string_elements_preserve_exact_utf16_and_missing_independently() {
    let isolated = StringElement::from_code_units(vec![0xD83D]);
    let non_bmp = StringElement::from_code_units(vec![0xD83D, 0xDE42]);
    let empty = StringElement::from_utf8("");
    let missing = StringElement::missing();

    assert_eq!(isolated.code_units(), &[0xD83D]);
    assert_eq!(isolated.code_unit_len(), 1);
    assert_eq!(isolated.to_utf8_lossy(), "\u{FFFD}");
    assert_eq!(non_bmp.code_units(), &[0xD83D, 0xDE42]);
    assert_eq!(non_bmp.code_unit_len(), 2);
    assert_eq!(non_bmp.to_utf8_lossy(), "🙂");

    assert!(!empty.is_missing());
    assert!(empty.is_payload_empty());
    assert!(missing.is_missing());
    assert!(missing.is_payload_empty());
    assert_ne!(empty, missing);
}

#[test]
fn empty_string_missing_string_and_empty_string_array_are_distinct() {
    let empty_scalar = StringValue::scalar("");
    let missing_scalar = StringValue::missing();
    let empty_array = StringValue::array(
        StringArray::from_elements(Shape::new([0, 2]).unwrap(), Vec::new()).unwrap(),
    );

    assert_eq!(empty_scalar.numel(), 1);
    assert_eq!(empty_scalar.dimensions(), &[1, 1]);
    assert!(!empty_scalar.as_scalar().unwrap().is_missing());
    assert_eq!(empty_scalar.utf16_code_unit_len(), 0);

    assert_eq!(missing_scalar.numel(), 1);
    assert!(missing_scalar.as_scalar().unwrap().is_missing());
    assert_eq!(missing_scalar.utf16_code_unit_len(), 0);

    assert_eq!(empty_array.numel(), 0);
    assert_eq!(empty_array.dimensions(), &[0, 2]);
    assert!(empty_array.as_scalar().is_none());
    assert_ne!(empty_scalar, missing_scalar);
    assert_ne!(empty_scalar, empty_array);
}

#[test]
fn string_array_copy_on_write_preserves_utf16_and_missing() {
    let array = StringArray::from_elements(
        Shape::new([1, 2]).unwrap(),
        vec![
            StringElement::from_code_units(vec![0xD83D]),
            StringElement::missing(),
        ],
    )
    .unwrap();
    let original = Value::from(array);
    let mut assigned = original.clone();

    assert!(original.shares_array_storage_with(&assigned));
    assigned
        .as_string_array_mut()
        .unwrap()
        .replace_linear(1, StringElement::from_utf8("changed"))
        .unwrap();
    assert!(!original.shares_array_storage_with(&assigned));
    assert_eq!(
        original.as_string_array().unwrap().as_slice()[0].code_units(),
        &[0xD83D]
    );
    assert!(original.as_string_array().unwrap().as_slice()[1].is_missing());
    assert_eq!(
        assigned.as_string_array().unwrap().as_slice()[0].code_units(),
        "changed".encode_utf16().collect::<Vec<_>>()
    );
}

#[test]
fn value_reports_exact_char_and_integer_metadata_without_scalar_variants() {
    let char_value = Value::from(ArrayData::from(
        DenseArray::from_vec(
            Shape::new([1, 2]).unwrap(),
            vec![CharCodeUnit::new(0xD83D), CharCodeUnit::new(0xDE42)],
        )
        .unwrap(),
    ));
    assert_eq!(char_value.kind(), ValueKind::Char);
    assert_eq!(char_value.class_name(), "char");
    assert_eq!(char_value.dtype(), Some(DType::Char));
    assert_eq!(char_value.dimensions(), Some([1, 2].as_slice()));
    assert_eq!(char_value.numel(), Some(2));
    assert!(!char_value.is_complex_numeric());

    let uint64_scalar = Value::from(ArrayData::from(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![u64::MAX]).unwrap(),
    ));
    assert!(matches!(uint64_scalar, Value::Array(_)));
    assert_eq!(uint64_scalar.kind(), ValueKind::Integer);
    assert_eq!(uint64_scalar.class_name(), "uint64");
    assert_eq!(uint64_scalar.dtype(), Some(DType::U64));
    assert_eq!(uint64_scalar.dimensions(), Some([1, 1].as_slice()));
    assert!(uint64_scalar.is_scalar());
    assert_eq!(uint64_scalar.as_real_number(), None);

    let complex_integer = Value::from(ArrayData::from(
        DenseArray::from_vec(
            Shape::new([0, 3]).unwrap(),
            Vec::<ComplexInteger<i8>>::new(),
        )
        .unwrap(),
    ));
    assert_eq!(complex_integer.kind(), ValueKind::Integer);
    assert_eq!(complex_integer.class_name(), "int8");
    assert_eq!(complex_integer.dtype(), Some(DType::ComplexI8));
    assert_eq!(complex_integer.dimensions(), Some([0, 3].as_slice()));
    assert_eq!(complex_integer.numel(), Some(0));
    assert!(complex_integer.is_complex_numeric());
    assert_eq!(complex_integer.as_complex_number(), None);
}

#[test]
fn char_value_clone_detaches_without_changing_class_or_shape() {
    let original = Value::from(ArrayData::from(
        DenseArray::from_vec(
            Shape::new([1, 2]).unwrap(),
            vec![CharCodeUnit::new(65), CharCodeUnit::new(66)],
        )
        .unwrap(),
    ));
    let mut assigned = original.clone();
    assert!(original.shares_array_storage_with(&assigned));

    *assigned
        .as_array_mut()
        .unwrap()
        .as_typed_mut::<CharCodeUnit>()
        .unwrap()
        .get_mut_linear(2)
        .unwrap() = CharCodeUnit::new(90);

    assert!(!original.shares_array_storage_with(&assigned));
    assert_eq!(assigned.kind(), ValueKind::Char);
    assert_eq!(assigned.class_name(), "char");
    assert_eq!(assigned.dimensions(), Some([1, 2].as_slice()));
    assert_eq!(
        original
            .as_array()
            .unwrap()
            .as_typed::<CharCodeUnit>()
            .unwrap()
            .as_slice()[1]
            .get(),
        66
    );
    assert_eq!(
        assigned
            .as_array()
            .unwrap()
            .as_typed::<CharCodeUnit>()
            .unwrap()
            .as_slice()[1]
            .get(),
        90
    );
}

#[test]
fn value_reports_exact_real_and_complex_single_metadata() {
    let scalar = Value::from(ArrayData::from(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![-0.0_f32]).unwrap(),
    ));
    assert!(matches!(scalar, Value::Array(ArrayData::F32(_))));
    assert_eq!(scalar.kind(), ValueKind::Single);
    assert_eq!(scalar.class_name(), "single");
    assert_eq!(scalar.dtype(), Some(DType::F32));
    assert_eq!(scalar.dimensions(), Some([1, 1].as_slice()));
    assert_eq!(scalar.numel(), Some(1));
    assert!(scalar.is_scalar());
    assert!(!scalar.is_complex_numeric());
    assert_eq!(
        scalar.as_real_single().unwrap().to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(scalar.as_real_number(), None);
    let array = scalar.as_array().unwrap();
    assert_eq!(
        array.as_typed::<f32>().unwrap().as_slice()[0].to_bits(),
        (-0.0_f32).to_bits()
    );
    assert!(array.as_typed::<f64>().is_none());
    assert!(array.as_typed::<Complex32>().is_none());
    assert!(
        Value::from(ArrayData::from(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![1.0_f64]).unwrap()
        ))
        .as_single()
        .is_none()
    );

    let complex = Value::from(ArrayData::from(
        DenseArray::from_vec(
            Shape::new([2, 2]).unwrap(),
            vec![
                Complex32::new(1.0, -2.0),
                Complex32::new(f32::NAN, 0.0),
                Complex32::new(f32::INFINITY, f32::NEG_INFINITY),
                Complex32::new(-0.0, 0.0),
            ],
        )
        .unwrap(),
    ));
    assert_eq!(complex.kind(), ValueKind::Single);
    assert_eq!(complex.class_name(), "single");
    assert_eq!(complex.dtype(), Some(DType::ComplexF32));
    assert_eq!(complex.dimensions(), Some([2, 2].as_slice()));
    assert_eq!(complex.numel(), Some(4));
    assert!(complex.is_complex_numeric());
    assert_eq!(complex.as_complex_single(), None);
    let array = complex.as_array().unwrap();
    assert_eq!(
        array.as_typed::<Complex32>().unwrap().as_slice()[0],
        Complex32::new(1.0, -2.0)
    );
    assert!(array.as_typed::<f32>().is_none());

    let empty = Value::from(ArrayData::from(
        DenseArray::<f32>::from_vec(Shape::new([0, 3]).unwrap(), Vec::new()).unwrap(),
    ));
    assert_eq!(empty.dimensions(), Some([0, 3].as_slice()));
    assert_eq!(empty.numel(), Some(0));
    assert!(!empty.is_scalar());
}

#[test]
fn single_value_clone_shares_then_detaches_exact_storage() {
    let original = Value::from(ArrayData::from(
        DenseArray::from_vec(Shape::new([2, 1]).unwrap(), vec![1.25_f32, -2.5]).unwrap(),
    ));
    let mut assigned = original.clone();
    assert!(original.shares_array_storage_with(&assigned));

    *assigned
        .as_array_mut()
        .unwrap()
        .as_typed_mut::<f32>()
        .unwrap()
        .get_mut_linear(2)
        .unwrap() = -7.75;

    assert!(!original.shares_array_storage_with(&assigned));
    assert_eq!(
        original.as_single().unwrap().as_f32().unwrap().as_slice(),
        &[1.25, -2.5]
    );
    assert_eq!(
        assigned.as_single().unwrap().as_f32().unwrap().as_slice(),
        &[1.25, -7.75]
    );
}

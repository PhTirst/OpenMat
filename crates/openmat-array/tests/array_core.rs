use openmat_array::{
    ArrayData, ArrayError, CharCodeUnit, Complex32, Complex64, ComplexInteger, Contiguity, DType,
    DenseArray, IndexRange, IndexSelection, IntegerArrayData, IntegerComponent, Logical,
    SelectionStrategy, Shape,
};

#[test]
fn shape_is_at_least_two_dimensional_and_trims_trailing_singletons() {
    assert_eq!(Shape::new([]).unwrap().dimensions(), &[1, 1]);
    assert_eq!(Shape::new([7]).unwrap().dimensions(), &[7, 1]);
    assert_eq!(Shape::new([2, 3, 1, 1]).unwrap().dimensions(), &[2, 3]);
    assert_eq!(Shape::new([2, 1, 4, 1]).unwrap().dimensions(), &[2, 1, 4]);
    assert_eq!(Shape::new([1, 1]).unwrap().ndims(), 2);
}

#[test]
fn shape_checks_dimension_products() {
    assert_eq!(
        Shape::new([u64::MAX, 2]).unwrap_err(),
        ArrayError::SizeOverflow {
            dimension: 1,
            partial: u64::MAX,
            extent: 2,
        }
    );
}

#[test]
fn matlab_column_major_offsets_round_trip() {
    let shape = Shape::new([2, 3, 4]).unwrap();
    assert_eq!(shape.stride(0), 1);
    assert_eq!(shape.stride(1), 2);
    assert_eq!(shape.stride(2), 6);
    assert_eq!(shape.offset_for_subscripts(&[2, 3, 4]).unwrap(), 23);
    assert_eq!(shape.linear_index_for_subscripts(&[2, 3, 4]).unwrap(), 24);
    assert_eq!(shape.subscripts_for_linear_index(24, 3).unwrap(), [2, 3, 4]);
}

#[test]
fn indexing_collapses_dimensions_and_extends_trailing_singletons() {
    let shape = Shape::new([2, 3, 4]).unwrap();
    assert_eq!(shape.effective_dimensions(2).unwrap(), [2, 12]);
    assert_eq!(shape.offset_for_subscripts(&[2, 12]).unwrap(), 23);
    assert_eq!(shape.subscripts_for_linear_index(24, 2).unwrap(), [2, 12]);
    assert_eq!(shape.offset_for_subscripts(&[2, 3, 4, 1]).unwrap(), 23);
    assert!(matches!(
        shape.offset_for_subscripts(&[2, 3, 4, 2]),
        Err(ArrayError::IndexOutOfBounds {
            dimension: 3,
            extent: 1,
            ..
        })
    ));
}

#[test]
fn one_by_one_and_empty_arrays_have_checked_indexing() {
    let scalar = DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![42.0_f64]).unwrap();
    assert_eq!(
        (*scalar.get_linear(1).unwrap()).to_bits(),
        42.0_f64.to_bits()
    );
    assert!(matches!(
        scalar.get_linear(0),
        Err(ArrayError::LinearIndexOutOfBounds { .. })
    ));

    let empty = DenseArray::<f64>::from_vec(Shape::new([0, 3]).unwrap(), vec![]).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.shape().dimensions(), &[0, 3]);
    assert!(matches!(
        empty.get_linear(1),
        Err(ArrayError::LinearIndexOutOfBounds { index: 1, numel: 0 })
    ));
}

#[test]
fn range_and_colon_descriptors_resolve_without_materializing() {
    let ascending = IndexRange::new(2, 3, 8).unwrap();
    assert_eq!(
        IndexSelection::Range(ascending)
            .resolve(8)
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [2, 5, 8]
    );
    let descending = IndexRange::new(5, -2, 1).unwrap();
    assert_eq!(
        IndexSelection::Range(descending)
            .resolve(5)
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [5, 3, 1]
    );
    assert_eq!(IndexSelection::Colon.resolve(0).unwrap().len(), 0);
    assert!(
        IndexSelection::Range(IndexRange::new(9, 1, 1).unwrap())
            .resolve(3)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        IndexRange::new(1, 0, 3).unwrap_err(),
        ArrayError::ZeroRangeStep
    );
}

#[test]
fn nonempty_selection_checks_both_range_ends() {
    assert!(matches!(
        IndexSelection::Range(IndexRange::new(1, 2, 7).unwrap()).resolve(6),
        Err(ArrayError::IndexOutOfBounds {
            index: 7,
            extent: 6,
            ..
        })
    ));
    assert!(matches!(
        IndexSelection::Scalar(0).resolve(4),
        Err(ArrayError::IndexOutOfBounds { index: 0, .. })
    ));
}

#[test]
fn clone_shares_then_first_write_detaches() {
    let original = DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![1, 2, 3, 4]).unwrap();
    let mut clone = original.clone();

    assert!(original.shares_storage_with(&clone));
    assert_eq!(original.storage_strong_count(), 2);
    assert_eq!(*clone.get_subscripts(&[1, 2]).unwrap(), 3);
    assert!(original.shares_storage_with(&clone));

    *clone.get_mut_subscripts(&[1, 2]).unwrap() = 30;
    assert!(!original.shares_storage_with(&clone));
    assert_eq!(*original.get_subscripts(&[1, 2]).unwrap(), 3);
    assert_eq!(*clone.get_subscripts(&[1, 2]).unwrap(), 30);
}

#[test]
fn dense_storage_policy_and_supported_dtypes_are_explicit() {
    let real = DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![1.0]).unwrap();
    assert_eq!(real.dtype(), DType::F64);
    assert_eq!(real.contiguity(), Contiguity::ColumnMajor);
    assert_eq!(
        real.selection_strategy(),
        SelectionStrategy::MaterializeCopy
    );

    let complex =
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![Complex64::new(1.0, -2.0)]).unwrap();
    let logical = DenseArray::from_vec(
        Shape::new([1, 2]).unwrap(),
        vec![Logical::TRUE, Logical::FALSE],
    )
    .unwrap();
    assert_eq!(complex.dtype(), DType::ComplexF64);
    assert_eq!(logical.dtype(), DType::Logical);
    assert!(logical.as_slice()[0].get());

    let tagged = ArrayData::ComplexF64(complex);
    assert_eq!(tagged.dtype(), DType::ComplexF64);
    assert_eq!(tagged.numel(), 1);
}

#[test]
fn mismatched_buffers_are_rejected() {
    assert_eq!(
        DenseArray::<f64>::from_vec(Shape::new([2, 2]).unwrap(), vec![0.0; 3]).unwrap_err(),
        ArrayError::DataLengthMismatch {
            expected: 4,
            actual: 3,
        }
    );
}

#[test]
fn dtype_metadata_covers_all_supported_storage_classes() {
    let expected = [
        (DType::F32, "single", false, 4, 4),
        (DType::ComplexF32, "single", true, 4, 8),
        (DType::F64, "double", false, 8, 8),
        (DType::ComplexF64, "double", true, 8, 16),
        (DType::Logical, "logical", false, 1, 1),
        (DType::Char, "char", false, 2, 2),
        (DType::I8, "int8", false, 1, 1),
        (DType::ComplexI8, "int8", true, 1, 2),
        (DType::U8, "uint8", false, 1, 1),
        (DType::ComplexU8, "uint8", true, 1, 2),
        (DType::I16, "int16", false, 2, 2),
        (DType::ComplexI16, "int16", true, 2, 4),
        (DType::U16, "uint16", false, 2, 2),
        (DType::ComplexU16, "uint16", true, 2, 4),
        (DType::I32, "int32", false, 4, 4),
        (DType::ComplexI32, "int32", true, 4, 8),
        (DType::U32, "uint32", false, 4, 4),
        (DType::ComplexU32, "uint32", true, 4, 8),
        (DType::I64, "int64", false, 8, 8),
        (DType::ComplexI64, "int64", true, 8, 16),
        (DType::U64, "uint64", false, 8, 8),
        (DType::ComplexU64, "uint64", true, 8, 16),
    ];

    for (dtype, class, complex, component_width, element_width) in expected {
        assert_eq!(dtype.class_name(), class);
        assert_eq!(dtype.is_complex(), complex);
        assert_eq!(dtype.component_width_bytes(), component_width);
        assert_eq!(dtype.element_width_bytes(), element_width);
        assert_eq!(
            dtype.is_integer(),
            !matches!(
                dtype,
                DType::F32
                    | DType::ComplexF32
                    | DType::F64
                    | DType::ComplexF64
                    | DType::Logical
                    | DType::Char
            )
        );
    }
}

#[test]
fn single_storage_is_exact_column_major_and_copy_on_write() {
    let values = vec![
        -0.0_f32,
        1.5_f32,
        -2.25_f32,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(0x7fc0_1234),
    ];
    let original =
        ArrayData::from(DenseArray::from_vec(Shape::new([2, 3]).unwrap(), values.clone()).unwrap());
    let mut assigned = original.clone();

    assert_eq!(original.dtype(), DType::F32);
    assert_eq!(original.class_name(), "single");
    assert!(!original.is_complex());
    assert_eq!(original.shape().dimensions(), &[2, 3]);
    assert_eq!(original.numel(), 6);
    assert_eq!(
        original
            .as_f32()
            .unwrap()
            .get_subscripts(&[1, 2])
            .unwrap()
            .to_bits(),
        values[2].to_bits()
    );
    assert_eq!(
        original.as_f32().unwrap().as_slice()[0].to_bits(),
        (-0.0_f32).to_bits()
    );
    assert!(original.as_f32().unwrap().as_slice()[5].is_nan());
    assert!(original.shares_storage_with(&assigned));

    *assigned
        .as_f32_mut()
        .unwrap()
        .get_mut_subscripts(&[1, 2])
        .unwrap() = 99.0;
    assert!(!original.shares_storage_with(&assigned));
    assert_eq!(
        original.as_f32().unwrap().as_slice()[2].to_bits(),
        (-2.25_f32).to_bits()
    );
    assert_eq!(
        assigned.as_f32().unwrap().as_slice()[2].to_bits(),
        99.0_f32.to_bits()
    );
}

#[test]
fn complex_single_storage_preserves_components_and_empty_shape() {
    let empty = ArrayData::from(
        DenseArray::<Complex32>::from_vec(Shape::new([0, 4]).unwrap(), Vec::new()).unwrap(),
    );
    assert_eq!(empty.dtype(), DType::ComplexF32);
    assert_eq!(empty.shape().dimensions(), &[0, 4]);
    assert_eq!(empty.numel(), 0);
    assert!(empty.is_complex());
    assert!(empty.as_typed::<Complex32>().unwrap().as_slice().is_empty());
    assert!(empty.as_typed::<f32>().is_none());

    let values = vec![
        Complex32::new(-0.0, 0.0),
        Complex32::new(f32::NAN, f32::INFINITY),
    ];
    let complex =
        ArrayData::from(DenseArray::from_vec(Shape::new([2, 1]).unwrap(), values).unwrap());
    let stored = complex.as_complex_f32().unwrap().as_slice();
    assert_eq!(stored[0].re.to_bits(), (-0.0_f32).to_bits());
    assert_eq!(stored[0].im.to_bits(), 0.0_f32.to_bits());
    assert!(stored[1].re.is_nan());
    assert_eq!(stored[1].im.to_bits(), f32::INFINITY.to_bits());
    assert_eq!(stored[0].conjugate().im.to_bits(), (-0.0_f32).to_bits());

    let mut assigned = complex.clone();
    assert!(complex.shares_storage_with(&assigned));
    assert!(assigned.as_typed_mut::<f32>().is_none());
    assert!(complex.shares_storage_with(&assigned));
    assigned.as_typed_mut::<Complex32>().unwrap().as_mut_slice()[0].im = -3.25;
    assert!(!complex.shares_storage_with(&assigned));
    assert_eq!(
        complex.as_complex_f32().unwrap().as_slice()[0].im.to_bits(),
        0.0_f32.to_bits()
    );
    assert_eq!(
        assigned.as_complex_f32().unwrap().as_slice()[0]
            .im
            .to_bits(),
        (-3.25_f32).to_bits()
    );
}

#[test]
fn char_preserves_utf16_code_units_and_remains_distinct_from_uint16() {
    let shape = Shape::new([1, 3]).unwrap();
    let code_units = [0xD83D, 0xDE42, 0xD83D].map(CharCodeUnit::new);
    let char_data =
        ArrayData::from(DenseArray::from_vec(shape.clone(), Vec::from(code_units)).unwrap());
    let uint16_data = ArrayData::from(DenseArray::from_vec(shape, vec![0xD83D_u16; 3]).unwrap());

    assert_eq!(char_data.dtype(), DType::Char);
    assert_eq!(char_data.class_name(), "char");
    assert_eq!(uint16_data.dtype(), DType::U16);
    assert_eq!(uint16_data.class_name(), "uint16");
    assert_eq!(
        char_data
            .as_typed::<CharCodeUnit>()
            .unwrap()
            .as_slice()
            .iter()
            .copied()
            .map(CharCodeUnit::get)
            .collect::<Vec<_>>(),
        [0xD83D, 0xDE42, 0xD83D]
    );
    assert!(char_data.as_typed::<u16>().is_none());
}

#[test]
fn integer_storage_preserves_zero_extent_shape_and_exact_components() {
    let empty = IntegerArrayData::from(
        DenseArray::<i32>::from_vec(Shape::new([0, 7]).unwrap(), Vec::new()).unwrap(),
    );
    assert_eq!(empty.shape().dimensions(), &[0, 7]);
    assert_eq!(empty.numel(), 0);
    assert_eq!(empty.dtype(), DType::I32);

    let maximum =
        ArrayData::from(DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![u64::MAX]).unwrap());
    assert_eq!(maximum.dtype(), DType::U64);
    assert_eq!(maximum.as_typed::<u64>().unwrap().as_slice(), &[u64::MAX]);
    assert_eq!(
        maximum
            .as_integer()
            .unwrap()
            .element(0)
            .unwrap()
            .real_component(),
        IntegerComponent::Unsigned(u128::from(u64::MAX))
    );

    let components = vec![
        ComplexInteger::new(i16::MIN, i16::MAX),
        ComplexInteger::new(-7_i16, 9_i16),
    ];
    let complex = ArrayData::from(
        DenseArray::from_vec(Shape::new([2, 1]).unwrap(), components.clone()).unwrap(),
    );
    assert_eq!(complex.dtype(), DType::ComplexI16);
    assert_eq!(complex.class_name(), "int16");
    assert!(complex.is_complex());
    assert_eq!(
        complex
            .as_typed::<ComplexInteger<i16>>()
            .unwrap()
            .as_slice(),
        components
    );
    assert_eq!(components[0].into_parts(), (i16::MIN, i16::MAX));
}

#[test]
fn dynamic_typed_access_preserves_integer_copy_on_write() {
    let original = ArrayData::from(
        DenseArray::from_vec(Shape::new([2, 1]).unwrap(), vec![1_u64, u64::MAX]).unwrap(),
    );
    let mut assigned = original.clone();

    assert!(original.shares_storage_with(&assigned));
    assert!(assigned.as_typed::<i64>().is_none());
    *assigned
        .as_typed_mut::<u64>()
        .unwrap()
        .get_mut_linear(1)
        .unwrap() = 99;

    assert!(!original.shares_storage_with(&assigned));
    assert_eq!(
        original.as_typed::<u64>().unwrap().as_slice(),
        &[1, u64::MAX]
    );
    assert_eq!(
        assigned.as_typed::<u64>().unwrap().as_slice(),
        &[99, u64::MAX]
    );
}

macro_rules! assert_dynamic_integer_storage {
    ($value:expr, $dtype:expr, $real:expr, $imaginary:expr) => {{
        let storage = IntegerArrayData::from(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![$value]).unwrap(),
        );
        assert_eq!(storage.dtype(), $dtype);
        let element = storage.element(0).unwrap();
        let expected_imaginary: Option<IntegerComponent> = $imaginary;
        assert_eq!(element.real_component(), $real);
        assert_eq!(element.imaginary_component(), expected_imaginary);
        assert_eq!(element.is_complex(), expected_imaginary.is_some());
        assert_eq!(storage.elements().collect::<Vec<_>>(), vec![element]);
        assert_eq!(storage.elements().len(), 1);
        assert!(storage.element(1).is_none());

        let decimal = storage.element_decimal(0).unwrap();
        assert_eq!(decimal.real_component(), $real.canonical_decimal());
        assert_eq!(
            decimal.imaginary_component(),
            expected_imaginary
                .map_or_else(|| String::from("0"), IntegerComponent::canonical_decimal)
        );
    }};
}

#[test]
fn dynamic_integer_view_and_decimal_cover_signed_storage() {
    assert_dynamic_integer_storage!(i8::MIN, DType::I8, IntegerComponent::Signed(-128), None);
    assert_dynamic_integer_storage!(
        ComplexInteger::new(i8::MIN, i8::MAX),
        DType::ComplexI8,
        IntegerComponent::Signed(-128),
        Some(IntegerComponent::Signed(127))
    );
    assert_dynamic_integer_storage!(
        i16::MIN,
        DType::I16,
        IntegerComponent::Signed(-32_768),
        None
    );
    assert_dynamic_integer_storage!(
        ComplexInteger::new(i16::MIN, i16::MAX),
        DType::ComplexI16,
        IntegerComponent::Signed(-32_768),
        Some(IntegerComponent::Signed(32_767))
    );
    assert_dynamic_integer_storage!(
        i32::MIN,
        DType::I32,
        IntegerComponent::Signed(i128::from(i32::MIN)),
        None
    );
    assert_dynamic_integer_storage!(
        ComplexInteger::new(i32::MIN, i32::MAX),
        DType::ComplexI32,
        IntegerComponent::Signed(i128::from(i32::MIN)),
        Some(IntegerComponent::Signed(i128::from(i32::MAX)))
    );
    assert_dynamic_integer_storage!(
        i64::MIN,
        DType::I64,
        IntegerComponent::Signed(i128::from(i64::MIN)),
        None
    );
    assert_dynamic_integer_storage!(
        ComplexInteger::new(i64::MIN, i64::MAX),
        DType::ComplexI64,
        IntegerComponent::Signed(i128::from(i64::MIN)),
        Some(IntegerComponent::Signed(i128::from(i64::MAX)))
    );
}

#[test]
fn dynamic_integer_view_and_decimal_cover_unsigned_storage() {
    assert_dynamic_integer_storage!(u8::MAX, DType::U8, IntegerComponent::Unsigned(255), None);
    assert_dynamic_integer_storage!(
        ComplexInteger::new(u8::MAX, 0_u8),
        DType::ComplexU8,
        IntegerComponent::Unsigned(255),
        Some(IntegerComponent::Unsigned(0))
    );
    assert_dynamic_integer_storage!(
        u16::MAX,
        DType::U16,
        IntegerComponent::Unsigned(65_535),
        None
    );
    assert_dynamic_integer_storage!(
        ComplexInteger::new(u16::MAX, 1_u16),
        DType::ComplexU16,
        IntegerComponent::Unsigned(65_535),
        Some(IntegerComponent::Unsigned(1))
    );
    assert_dynamic_integer_storage!(
        u32::MAX,
        DType::U32,
        IntegerComponent::Unsigned(u128::from(u32::MAX)),
        None
    );
    assert_dynamic_integer_storage!(
        ComplexInteger::new(u32::MAX, 1_u32),
        DType::ComplexU32,
        IntegerComponent::Unsigned(u128::from(u32::MAX)),
        Some(IntegerComponent::Unsigned(1))
    );
    assert_dynamic_integer_storage!(
        u64::MAX,
        DType::U64,
        IntegerComponent::Unsigned(u128::from(u64::MAX)),
        None
    );
    assert_dynamic_integer_storage!(
        ComplexInteger::new(u64::MAX, u64::MAX - 1),
        DType::ComplexU64,
        IntegerComponent::Unsigned(u128::from(u64::MAX)),
        Some(IntegerComponent::Unsigned(u128::from(u64::MAX - 1)))
    );
}

use openmat_array::{ArrayData, CharCodeUnit, DenseArray, IntegerArrayData, Shape};
use openmat_builtins::minimal_registry;
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult, CancellationToken,
    OutputEvent, VecOutput,
};
use openmat_value::{
    CellArray, Complex64, FieldName, ObjectHandle, StringArray, StringElement, StringValue, Value,
};

fn invoke(name: &str, arguments: &[Value]) -> BuiltinResult {
    invoke_with_outputs(name, arguments, 1)
}

fn invoke_with_outputs(name: &str, arguments: &[Value], requested_outputs: usize) -> BuiltinResult {
    invoke_capturing(name, arguments, requested_outputs).0
}

fn invoke_capturing(
    name: &str,
    arguments: &[Value],
    requested_outputs: usize,
) -> (BuiltinResult, VecOutput) {
    let registry = minimal_registry().expect("fresh registry must register aggregate built-ins");
    let handle = registry
        .handle_by_name(name)
        .expect("requested aggregate built-in must be registered");
    let cancellation = CancellationToken::new();
    let mut output = VecOutput::new();
    let mut context = BuiltinContext::new(requested_outputs, &cancellation, &mut output);
    let result = registry
        .invoke(handle, arguments, &mut context)
        .map_err(|error| match error {
            openmat_runtime::BuiltinInvocationError::Failed { error, .. } => error,
            openmat_runtime::BuiltinInvocationError::UnknownHandle(_) => {
                panic!("looked-up built-in handle must resolve")
            }
        });
    (result, output)
}

fn only_output(result: BuiltinResult) -> Value {
    let mut outputs = result.expect("built-in invocation must succeed");
    assert_eq!(outputs.len(), 1);
    outputs.remove(0)
}

fn assert_error(result: BuiltinResult, category: BuiltinErrorCategory) -> BuiltinError {
    let error = result.expect_err("built-in invocation must fail");
    assert_eq!(error.category, category);
    assert!(!error.message.is_empty());
    error
}

fn real_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn char_array(dimensions: impl IntoIterator<Item = u64>, code_units: &[u16]) -> Value {
    Value::Array(ArrayData::Char(
        DenseArray::from_vec(
            Shape::new(dimensions).unwrap(),
            code_units.iter().copied().map(CharCodeUnit::new).collect(),
        )
        .unwrap(),
    ))
}

fn char_row(value: &str) -> Value {
    let code_units = value.encode_utf16().collect::<Vec<_>>();
    char_array([1, u64::try_from(code_units.len()).unwrap()], &code_units)
}

fn uint64_scalar(value: u64) -> Value {
    Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![value]).unwrap(),
    )))
}

fn cell(dimensions: impl IntoIterator<Item = u64>, values: Vec<Value>) -> Value {
    Value::Cell(CellArray::from_values(Shape::new(dimensions).unwrap(), values).unwrap())
}

fn assert_empty_double(value: &Value) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("cell default must be a real double array, found {value:?}");
    };
    assert_eq!(array.shape().dimensions(), &[0, 0]);
    assert!(array.as_slice().is_empty());
}

#[test]
fn cell_constructor_covers_default_zero_exact_and_higher_dimensions() {
    let Value::Cell(empty) = only_output(invoke("cell", &[])) else {
        panic!("cell() must return a cell array");
    };
    assert_eq!(empty.shape().dimensions(), &[0, 0]);

    let Value::Cell(square) = only_output(invoke("cell", &[Value::Double(2.0)])) else {
        panic!("cell(2) must return a cell array");
    };
    assert_eq!(square.shape().dimensions(), &[2, 2]);
    assert_eq!(square.values().len(), 4);
    square.values().iter().for_each(assert_empty_double);

    let Value::Cell(shaped_empty) = only_output(invoke(
        "cell",
        &[uint64_scalar(2), uint64_scalar(0), uint64_scalar(4)],
    )) else {
        panic!("exact integer dimensions must construct a cell array");
    };
    assert_eq!(shaped_empty.shape().dimensions(), &[2, 0, 4]);
    assert!(shaped_empty.values().is_empty());

    let Value::Cell(higher) = only_output(invoke(
        "cell",
        &[Value::Double(1.0), Value::Double(2.0), Value::Double(3.0)],
    )) else {
        panic!("three dimensions must construct a cell array");
    };
    assert_eq!(higher.shape().dimensions(), &[1, 2, 3]);
    assert_eq!(higher.numel(), 6);
}

#[test]
fn cell_constructor_rejects_invalid_overloads_and_checked_overflow() {
    for invalid in [
        Value::Double(-1.0),
        Value::Double(1.5),
        Value::Double(f64::INFINITY),
    ] {
        assert_error(invoke("cell", &[invalid]), BuiltinErrorCategory::Domain);
    }
    assert_error(
        invoke("cell", &[Value::Complex(Complex64::new(2.0, 1.0))]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("cell", &[real_array([1, 2], vec![2.0, 3.0])]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("cell", &[Value::empty_double()]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke("cell", &[uint64_scalar(u64::MAX), uint64_scalar(2)]),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke_with_outputs("cell", &[], 2),
        BuiltinErrorCategory::ArgumentCount,
    );
}

#[test]
fn struct_constructor_preserves_empty_states_and_ordered_scalar_fields() {
    let Value::Struct(scalar) = only_output(invoke("struct", &[])) else {
        panic!("struct() must return a struct array");
    };
    assert_eq!(scalar.shape().dimensions(), &[1, 1]);
    assert_eq!(scalar.numel(), 1);
    assert!(scalar.field_names().is_empty());

    let Value::Struct(empty) = only_output(invoke("struct", &[Value::empty_double()])) else {
        panic!("struct([]) must return a struct array");
    };
    assert_eq!(empty.shape().dimensions(), &[0, 0]);
    assert!(empty.field_names().is_empty());

    let Value::Struct(ordered) = only_output(invoke(
        "struct",
        &[
            char_row("beta"),
            Value::Double(2.0),
            Value::from("alpha"),
            Value::Logical(true),
        ],
    )) else {
        panic!("name/value pairs must return a struct array");
    };
    assert_eq!(ordered.shape().dimensions(), &[1, 1]);
    assert_eq!(
        ordered
            .field_names()
            .iter()
            .map(FieldName::as_str)
            .collect::<Vec<_>>(),
        ["beta", "alpha"]
    );
    assert_eq!(ordered.value_at_name("beta", 0), Some(&Value::Double(2.0)));
    assert_eq!(
        ordered.value_at_name("alpha", 0),
        Some(&Value::Logical(true))
    );
}

#[test]
fn struct_infers_cell_shape_and_applies_scalar_and_complete_value_expansion() {
    let records = cell([1, 2], vec![Value::Double(10.0), Value::Double(20.0)]);
    let expanded = cell([1, 1], vec![Value::from("same")]);
    let complete = real_array([1, 2], vec![7.0, 8.0]);
    let Value::Struct(mut structure) = only_output(invoke(
        "struct",
        &[
            char_row("record"),
            records,
            Value::from("expanded"),
            expanded,
            char_row("complete"),
            complete.clone(),
        ],
    )) else {
        panic!("cell-valued fields must construct a struct array");
    };
    assert_eq!(structure.shape().dimensions(), &[1, 2]);
    assert_eq!(
        structure.field_values(0),
        Some([Value::Double(10.0), Value::Double(20.0)].as_slice())
    );
    assert_eq!(
        structure.field_values(1),
        Some([Value::from("same"), Value::from("same")].as_slice())
    );
    for value in structure.field_values(2).unwrap() {
        assert!(complete.shares_array_storage_with(value));
        let Value::Array(ArrayData::F64(array)) = value else {
            panic!("non-cell field must remain one complete array value");
        };
        assert_eq!(array.as_slice(), &[7.0, 8.0]);
    }

    let mut changed = structure.value_at(2, 0).unwrap().clone();
    let Value::Array(ArrayData::F64(array)) = &mut changed else {
        unreachable!();
    };
    *array.get_mut_linear(1).unwrap() = 99.0;
    structure.replace_at(2, 0, changed).unwrap();
    let Value::Array(ArrayData::F64(source)) = &complete else {
        unreachable!();
    };
    assert_eq!(source.as_slice(), &[7.0, 8.0]);
    let Value::Array(ArrayData::F64(second)) = structure.value_at(2, 1).unwrap() else {
        unreachable!();
    };
    assert_eq!(second.as_slice(), &[7.0, 8.0]);
}

#[test]
fn struct_preserves_shaped_empty_schema_and_rejects_mismatched_cell_shapes() {
    let Value::Struct(empty) = only_output(invoke(
        "struct",
        &[
            char_row("beta"),
            cell([0, 3], Vec::new()),
            Value::from("alpha"),
            cell([1, 1], vec![Value::Double(9.0)]),
        ],
    )) else {
        panic!("a shaped empty cell must determine struct shape");
    };
    assert_eq!(empty.shape().dimensions(), &[0, 3]);
    assert_eq!(empty.numel(), 0);
    assert_eq!(
        empty
            .field_names()
            .iter()
            .map(FieldName::as_str)
            .collect::<Vec<_>>(),
        ["beta", "alpha"]
    );
    assert_eq!(empty.field_values(0), Some(&[] as &[Value]));
    assert_eq!(empty.field_values(1), Some(&[] as &[Value]));

    assert_error(
        invoke(
            "struct",
            &[
                char_row("left"),
                cell([1, 2], vec![Value::Double(1.0), Value::Double(2.0)]),
                char_row("right"),
                cell([2, 1], vec![Value::Double(3.0), Value::Double(4.0)]),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
}

#[test]
fn struct_rejects_duplicate_invalid_missing_and_unsupported_names_or_forms() {
    assert_error(
        invoke(
            "struct",
            &[
                char_row("same"),
                Value::Double(1.0),
                Value::from("same"),
                Value::Double(2.0),
            ],
        ),
        BuiltinErrorCategory::Domain,
    );
    for invalid in [char_row(""), char_row("_bad"), char_row("bad-name")] {
        assert_error(
            invoke("struct", &[invalid, Value::Double(1.0)]),
            BuiltinErrorCategory::Domain,
        );
    }
    assert_error(
        invoke(
            "struct",
            &[char_array([1, 1], &[0x03b1]), Value::Double(1.0)],
        ),
        BuiltinErrorCategory::Domain,
    );
    assert_error(
        invoke(
            "struct",
            &[Value::String(StringValue::missing()), Value::Double(1.0)],
        ),
        BuiltinErrorCategory::Domain,
    );
    let string_names = Value::from(
        StringArray::from_elements(
            Shape::new([1, 2]).unwrap(),
            vec![StringElement::from("a"), StringElement::from("b")],
        )
        .unwrap(),
    );
    assert_error(
        invoke("struct", &[string_names, Value::Double(1.0)]),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke(
            "struct",
            &[
                char_array([2, 1], &[u16::from(b'a'), u16::from(b'b')]),
                Value::Double(1.0),
            ],
        ),
        BuiltinErrorCategory::Type,
    );
    assert_error(
        invoke(
            "struct",
            &[char_row("field"), Value::Double(1.0), char_row("dangling")],
        ),
        BuiltinErrorCategory::ArgumentCount,
    );
    assert_error(
        invoke("struct", &[Value::Double(1.0)]),
        BuiltinErrorCategory::Other,
    );
    assert_error(
        invoke("struct", &[real_array([0, 3], Vec::new())]),
        BuiltinErrorCategory::Other,
    );
}

#[test]
fn struct_nested_exact_values_keep_cow_and_use_the_language_copy_boundary() {
    let exact_integer = uint64_scalar(u64::MAX);
    let exact_char = char_array([1, 1], &[0xd83d]);
    let missing = Value::String(StringValue::missing());
    let nested = cell(
        [1, 3],
        vec![exact_integer.clone(), exact_char.clone(), missing.clone()],
    );
    let holder = cell([1, 1], vec![nested.clone()]);
    let Value::Struct(mut structure) =
        only_output(invoke("struct", &[char_row("payload"), holder]))
    else {
        panic!("nested exact values must construct a struct");
    };
    let stored = structure.value_at_name("payload", 0).unwrap();
    assert_eq!(stored, &nested);
    assert!(stored.shares_array_storage_with(&nested));

    let Value::Cell(mut changed) = stored.clone() else {
        unreachable!();
    };
    changed.replace_at_offset(0, Value::Double(9.0)).unwrap();
    structure.replace_at(0, 0, Value::Cell(changed)).unwrap();
    let Value::Cell(original) = &nested else {
        unreachable!();
    };
    assert_eq!(original.value_at_offset(0), Some(&exact_integer));
    assert_eq!(original.value_at_offset(1), Some(&exact_char));
    assert_eq!(original.value_at_offset(2), Some(&missing));

    assert_error(
        invoke(
            "struct",
            &[char_row("object"), Value::Object(ObjectHandle::new(7))],
        ),
        BuiltinErrorCategory::LanguageCopy,
    );
    assert_error(
        invoke(
            "struct",
            &[
                char_row("object"),
                cell([1, 1], vec![Value::Object(ObjectHandle::new(7))]),
            ],
        ),
        BuiltinErrorCategory::LanguageCopy,
    );
}

#[test]
fn aggregate_metadata_display_and_type_errors_remain_exact() {
    let cell_value = only_output(invoke("cell", &[Value::Double(0.0), Value::Double(3.0)]));
    assert_eq!(
        invoke("class", std::slice::from_ref(&cell_value)),
        Ok(vec![Value::from("cell")])
    );
    assert_eq!(
        invoke("isa", &[cell_value.clone(), char_row("cell")]),
        Ok(vec![Value::Logical(true)])
    );
    let size = only_output(invoke("size", std::slice::from_ref(&cell_value)));
    let Value::Array(ArrayData::F64(size)) = size else {
        panic!("size must return a real vector");
    };
    assert_eq!(size.as_slice(), &[0.0, 3.0]);
    assert_eq!(
        invoke("numel", std::slice::from_ref(&cell_value)),
        Ok(vec![Value::Double(0.0)])
    );
    assert_eq!(
        invoke("ndims", std::slice::from_ref(&cell_value)),
        Ok(vec![Value::Double(2.0)])
    );
    assert_eq!(
        invoke("length", std::slice::from_ref(&cell_value)),
        Ok(vec![Value::Double(0.0)])
    );
    assert_eq!(
        invoke("isempty", std::slice::from_ref(&cell_value)),
        Ok(vec![Value::Logical(true)])
    );

    let structure = only_output(invoke(
        "struct",
        &[char_row("field"), cell([0, 3], Vec::new())],
    ));
    assert_eq!(
        invoke("class", std::slice::from_ref(&structure)),
        Ok(vec![Value::from("struct")])
    );
    let (display, output) = invoke_capturing("disp", std::slice::from_ref(&structure), 0);
    assert_eq!(display, Ok(Vec::new()));
    let [OutputEvent::Display(displayed)] = output.events() else {
        panic!("disp must emit exactly one display event");
    };
    assert_eq!(displayed, &structure);
    assert!(displayed.shares_array_storage_with(&structure));

    assert_eq!(
        invoke("strcmp", &[cell_value.clone(), cell_value.clone()]),
        Ok(vec![Value::Array(ArrayData::Logical(
            DenseArray::from_vec(Shape::new([0, 3]).unwrap(), Vec::new()).unwrap()
        ))])
    );

    for (name, arguments) in [
        ("double", vec![cell_value.clone()]),
        ("string", vec![structure.clone()]),
        (
            "reshape",
            vec![structure, Value::Double(1.0), Value::Double(0.0)],
        ),
    ] {
        assert_error(invoke(name, &arguments), BuiltinErrorCategory::Type);
    }
}

#[test]
fn deal_matches_r2022b_arity_and_copies_every_returned_value() {
    assert_eq!(invoke_with_outputs("deal", &[], 0), Ok(Vec::new()));
    assert_eq!(
        invoke_with_outputs("deal", &[Value::Double(5.0)], 0),
        Ok(Vec::new())
    );
    assert_error(
        invoke_with_outputs("deal", &[Value::Double(1.0), Value::Double(2.0)], 0),
        BuiltinErrorCategory::ArgumentCount,
    );
    assert_error(
        invoke_with_outputs("deal", &[], 1),
        BuiltinErrorCategory::ArgumentCount,
    );

    let source = cell([1, 1], vec![Value::Double(7.0)]);
    let outputs = invoke_with_outputs("deal", std::slice::from_ref(&source), 3).unwrap();
    assert_eq!(outputs.len(), 3);
    assert!(
        outputs
            .iter()
            .all(|value| value.shares_array_storage_with(&source))
    );
    let Value::Cell(mut changed) = outputs[0].clone() else {
        unreachable!();
    };
    changed.replace_at_offset(0, Value::Double(9.0)).unwrap();
    let Value::Cell(source_cell) = &source else {
        unreachable!();
    };
    assert_eq!(source_cell.value_at_offset(0), Some(&Value::Double(7.0)));
    assert_eq!(outputs[1], source);

    assert_eq!(
        invoke_with_outputs("deal", &[Value::Double(1.0), Value::from("two")], 2),
        Ok(vec![Value::Double(1.0), Value::from("two")])
    );
    assert_error(
        invoke_with_outputs("deal", &[Value::Double(1.0), Value::Double(2.0)], 1),
        BuiltinErrorCategory::ArgumentCount,
    );
    assert_error(
        invoke_with_outputs("deal", &[Value::Double(1.0), Value::Double(2.0)], 3),
        BuiltinErrorCategory::ArgumentCount,
    );
    assert_error(
        invoke_with_outputs("deal", &[Value::Object(ObjectHandle::new(9))], 2),
        BuiltinErrorCategory::LanguageCopy,
    );
    assert_error(
        invoke_with_outputs(
            "deal",
            &[Value::Double(1.0), Value::Object(ObjectHandle::new(9))],
            2,
        ),
        BuiltinErrorCategory::LanguageCopy,
    );
}

#[test]
fn exact_char_integer_and_string_constructors_remain_registered() {
    let character = only_output(invoke("char", &[uint64_scalar(65)]));
    let Value::Array(ArrayData::Char(character_data)) = &character else {
        panic!("char must preserve exact storage");
    };
    assert_eq!(character_data.as_slice()[0].get(), 65);

    let round_trip = only_output(invoke("uint64", &[character]));
    let Value::Array(ArrayData::Integer(integer)) = round_trip else {
        panic!("uint64 must preserve exact integer storage");
    };
    assert_eq!(
        integer
            .element(0)
            .unwrap()
            .canonical_decimal()
            .real_component(),
        "65"
    );

    let strings = only_output(invoke("strings", &[uint64_scalar(2)]));
    let array = strings.as_string_array().unwrap();
    assert_eq!(array.shape().dimensions(), &[2, 2]);
    assert!(array.as_slice().iter().all(StringElement::is_payload_empty));

    let complex = only_output(invoke(
        "complex",
        &[real_array([1, 1], vec![1.0]), Value::Double(2.0)],
    ));
    assert_eq!(complex.as_complex_number(), Some(Complex64::new(1.0, 2.0)));
}

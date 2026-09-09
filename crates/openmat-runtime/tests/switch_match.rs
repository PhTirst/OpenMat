use openmat_array::{
    ArrayData, CharCodeUnit, Complex64 as ArrayComplex64, ComplexInteger, DenseArray,
    IntegerArrayData, IntegerElement, Logical, Shape,
};
use openmat_bytecode::{
    BytecodeModule, Function, FunctionId, Instruction, InstructionKind, LocalSlot, Register,
    SourceLocation,
};
use openmat_runtime::{ArrayRuntimeError, Interpreter, RuntimeError, RuntimeErrorKind};
use openmat_value::{
    BuiltinHandle, CellArray, Complex64, FunctionHandle, StringArray, StringElement, StringValue,
    Value, ValueKind,
};

const SWITCH_LOCATION: SourceLocation = SourceLocation::new(17, 4, 26);
const I64_MIN_AS_F64: f64 = -9_223_372_036_854_775_808.0;
const U64_EXCLUSIVE_LIMIT_AS_F64: f64 = 18_446_744_073_709_551_616.0;

fn switch_interpreter() -> Interpreter {
    let function = Function {
        name: "switch_match".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 2,
        persistent_slot_count: 0,
        parameter_count: 2,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            Instruction::new(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            Instruction::new(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(1),
            }),
            Instruction::located(
                InstructionKind::SwitchMatch {
                    dst: Register::new(2),
                    selector: Register::new(0),
                    case_value: Register::new(1),
                },
                SWITCH_LOCATION,
            ),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    Interpreter::new(BytecodeModule::new(vec![function], FunctionId::new(0)))
        .expect("switch-match bytecode should verify")
}

fn matched(interpreter: &mut Interpreter, selector: Value, case_value: Value) -> bool {
    let returned = interpreter
        .execute_entry(&[selector, case_value])
        .expect("supported switch values should execute");
    assert_eq!(returned.len(), 1);
    let Value::Logical(matched) = returned[0] else {
        panic!("SwitchMatch must write one logical scalar")
    };
    matched
}

fn switch_error(interpreter: &mut Interpreter, selector: Value, case_value: Value) -> RuntimeError {
    interpreter
        .execute_entry(&[selector, case_value])
        .expect_err("unsupported switch values must return a runtime error")
}

fn integer_scalar<T: IntegerElement>(value: T) -> Value {
    let array = DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![value]).unwrap();
    Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(array)))
}

fn real_array(dimensions: [u64; 2], values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn logical_array(dimensions: [u64; 2], values: Vec<bool>) -> Value {
    Value::Array(ArrayData::Logical(
        DenseArray::from_vec(
            Shape::new(dimensions).unwrap(),
            values.into_iter().map(Logical::from).collect(),
        )
        .unwrap(),
    ))
}

fn char_array(dimensions: [u64; 2], values: Vec<u16>) -> Value {
    Value::Array(ArrayData::Char(
        DenseArray::from_vec(
            Shape::new(dimensions).unwrap(),
            values.into_iter().map(CharCodeUnit::new).collect(),
        )
        .unwrap(),
    ))
}

fn string_array(dimensions: [u64; 2], values: Vec<StringElement>) -> Value {
    Value::String(StringValue::Array(
        StringArray::from_elements(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn cell_case(values: Vec<Value>) -> Value {
    let length = u64::try_from(values.len()).unwrap();
    Value::Cell(CellArray::from_values(Shape::new([1, length]).unwrap(), values).unwrap())
}

#[test]
fn executes_r2022b_numeric_scalar_matching_without_array_equality() {
    let mut interpreter = switch_interpreter();

    assert!(matched(
        &mut interpreter,
        Value::Double(1.0),
        Value::Logical(true)
    ));
    assert!(matched(
        &mut interpreter,
        Value::Complex(Complex64::new(1.0, 0.0)),
        Value::Double(1.0)
    ));
    assert!(matched(
        &mut interpreter,
        real_array([1, 1], vec![1.0]),
        Value::Double(1.0)
    ));
    assert!(!matched(
        &mut interpreter,
        Value::Double(f64::NAN),
        Value::Double(f64::NAN)
    ));
    assert!(!matched(
        &mut interpreter,
        Value::Complex(Complex64::new(1.0, 2.0)),
        Value::Complex(Complex64::new(1.0, -2.0))
    ));
    assert!(matched(
        &mut interpreter,
        Value::Double(-0.0),
        Value::Double(0.0)
    ));

    assert!(!matched(
        &mut interpreter,
        Value::Double(1.0),
        real_array([1, 2], vec![1.0, 2.0])
    ));
    let error = switch_error(
        &mut interpreter,
        real_array([1, 2], vec![1.0, 2.0]),
        real_array([1, 2], vec![1.0, 2.0]),
    );
    assert_switch_operand_error(&error, "switch selector", ValueKind::Array);
    let error = switch_error(
        &mut interpreter,
        logical_array([0, 0], Vec::new()),
        logical_array([0, 0], Vec::new()),
    );
    assert_switch_operand_error(&error, "switch selector", ValueKind::Array);
}

#[test]
fn matches_all_exact_integer_classes_and_complex_extremes() {
    let mut interpreter = switch_interpreter();
    let real_classes = vec![
        integer_scalar(7_i8),
        integer_scalar(7_u8),
        integer_scalar(7_i16),
        integer_scalar(7_u16),
        integer_scalar(7_i32),
        integer_scalar(7_u32),
        integer_scalar(7_i64),
        integer_scalar(7_u64),
    ];
    for value in real_classes {
        assert!(matched(&mut interpreter, value, Value::Double(7.0)));
    }

    let u64_max = integer_scalar(u64::MAX);
    assert!(matched(&mut interpreter, u64_max.clone(), u64_max.clone()));
    assert!(!matched(
        &mut interpreter,
        u64_max,
        Value::Double(U64_EXCLUSIVE_LIMIT_AS_F64)
    ));
    assert!(matched(
        &mut interpreter,
        integer_scalar(i64::MIN),
        Value::Double(I64_MIN_AS_F64)
    ));
    assert!(!matched(
        &mut interpreter,
        integer_scalar(i64::MAX),
        Value::Double(9_223_372_036_854_775_808.0)
    ));

    let complex_classes = vec![
        integer_scalar(ComplexInteger::new(7_i8, 2_i8)),
        integer_scalar(ComplexInteger::new(7_u8, 2_u8)),
        integer_scalar(ComplexInteger::new(7_i16, 2_i16)),
        integer_scalar(ComplexInteger::new(7_u16, 2_u16)),
        integer_scalar(ComplexInteger::new(7_i32, 2_i32)),
        integer_scalar(ComplexInteger::new(7_u32, 2_u32)),
        integer_scalar(ComplexInteger::new(7_i64, 2_i64)),
        integer_scalar(ComplexInteger::new(7_u64, 2_u64)),
    ];
    for value in complex_classes {
        assert!(matched(
            &mut interpreter,
            value,
            Value::Complex(Complex64::new(7.0, 2.0))
        ));
    }

    let complex_min_max = integer_scalar(ComplexInteger::new(i64::MIN, i64::MAX));
    assert!(matched(
        &mut interpreter,
        complex_min_max.clone(),
        complex_min_max
    ));
    assert!(!matched(
        &mut interpreter,
        integer_scalar(ComplexInteger::new(u64::MAX, u64::MAX)),
        Value::Complex(Complex64::new(
            U64_EXCLUSIVE_LIMIT_AS_F64,
            U64_EXCLUSIVE_LIMIT_AS_F64,
        ))
    ));
}

#[test]
fn matches_exact_utf16_char_and_string_values_including_empty_and_missing() {
    let mut interpreter = switch_interpreter();
    let surrogate = char_array([1, 1], vec![0xD800]);
    assert!(matched(&mut interpreter, surrogate.clone(), surrogate));
    assert!(!matched(
        &mut interpreter,
        char_array([1, 1], vec![0xD800]),
        char_array([1, 1], vec![0xD801])
    ));
    assert!(matched(
        &mut interpreter,
        char_array([1, 1], vec![u16::from(b'A')]),
        Value::Double(65.0)
    ));

    let exact_text = vec![0xD800, u16::from(b'x')];
    assert!(matched(
        &mut interpreter,
        char_array([1, 2], exact_text.clone()),
        Value::from(StringElement::from_code_units(exact_text))
    ));
    assert!(matched(
        &mut interpreter,
        char_array([0, 0], Vec::new()),
        Value::from("")
    ));
    assert!(!matched(
        &mut interpreter,
        Value::from(StringElement::missing()),
        Value::from(StringElement::missing())
    ));
    assert!(!matched(
        &mut interpreter,
        Value::from(StringElement::missing()),
        Value::from("")
    ));

    let strings = string_array(
        [1, 2],
        vec![
            StringElement::from_code_units([0xD800].as_slice()),
            StringElement::from_utf8("tail"),
        ],
    );
    assert!(matched(&mut interpreter, strings.clone(), strings.clone()));
    assert!(!matched(
        &mut interpreter,
        strings,
        string_array(
            [2, 1],
            vec![
                StringElement::from_code_units([0xD800].as_slice()),
                StringElement::from_utf8("tail"),
            ],
        )
    ));
    let with_missing = string_array(
        [1, 2],
        vec![StringElement::missing(), StringElement::from_utf8("tail")],
    );
    assert!(!matched(
        &mut interpreter,
        with_missing.clone(),
        with_missing
    ));
}

#[test]
fn matches_any_element_of_a_cell_valued_case_list() {
    let mut interpreter = switch_interpreter();

    assert!(matched(
        &mut interpreter,
        Value::from("beta"),
        cell_case(vec![Value::from("alpha"), Value::from("beta")]),
    ));
    assert!(matched(
        &mut interpreter,
        Value::Double(7.0),
        cell_case(vec![Value::Double(2.0), integer_scalar(7_i16)]),
    ));
    assert!(!matched(
        &mut interpreter,
        Value::from("missing"),
        cell_case(vec![Value::from("alpha"), Value::from("beta")]),
    ));
    assert!(!matched(
        &mut interpreter,
        Value::Double(1.0),
        cell_case(Vec::new()),
    ));
}

#[test]
fn reports_stable_errors_with_location_and_preserves_cow_storage() {
    let mut interpreter = switch_interpreter();
    let error = switch_error(&mut interpreter, Value::Double(1.0), Value::from("1"));
    assert_switch_operand_error(&error, "switch case value", ValueKind::String);
    assert_eq!(error.location, Some(SWITCH_LOCATION));

    let error = switch_error(&mut interpreter, Value::Nothing, Value::Nothing);
    assert_switch_operand_error(&error, "switch selector", ValueKind::Nothing);
    let error = switch_error(
        &mut interpreter,
        Value::Function(FunctionHandle::Builtin(BuiltinHandle::new(91))),
        Value::Function(FunctionHandle::Builtin(BuiltinHandle::new(91))),
    );
    assert_switch_operand_error(&error, "switch selector", ValueKind::Function);

    let selector = char_array([1, 3], vec![0xD800, u16::from(b'a'), u16::from(b'b')]);
    let alias = selector.clone();
    assert!(selector.shares_array_storage_with(&alias));
    assert!(matched(&mut interpreter, selector.clone(), alias.clone()));
    assert!(selector.shares_array_storage_with(&alias));

    let integer = integer_scalar(ComplexInteger::new(u64::MAX, 7_u64));
    let integer_alias = integer.clone();
    assert!(integer.shares_array_storage_with(&integer_alias));
    assert!(matched(
        &mut interpreter,
        integer.clone(),
        integer_alias.clone()
    ));
    assert!(integer.shares_array_storage_with(&integer_alias));
}

fn assert_switch_operand_error(
    error: &RuntimeError,
    expected_operation: &'static str,
    expected_kind: ValueKind,
) {
    let RuntimeErrorKind::InvalidExecutionState {
        array: Some(detail),
        ..
    } = &error.kind
    else {
        panic!("expected structured switch operand error, found {error:?}");
    };
    assert_eq!(
        detail.as_ref(),
        &ArrayRuntimeError::InvalidOperand {
            operation: expected_operation,
            actual: expected_kind,
        }
    );
}

#[test]
fn complex_double_array_scalar_uses_both_components() {
    let mut interpreter = switch_interpreter();
    let selector = Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(
            Shape::new([1, 1]).unwrap(),
            vec![ArrayComplex64::new(3.0, 4.0)],
        )
        .unwrap(),
    ));
    assert!(matched(
        &mut interpreter,
        selector,
        Value::Complex(Complex64::new(3.0, 4.0))
    ));
}

#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::float_cmp
)]

use openmat_array::{ArrayData, CharCodeUnit, DenseArray, Shape};
use openmat_builtins::minimal_registry;
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinResult, CancellationToken, VecOutput};
use openmat_value::{CellArray, StringArray, StringElement, StringValue, Value};

fn invoke(name: &str, arguments: &[Value]) -> BuiltinResult {
    let registry = minimal_registry().unwrap();
    let handle = registry.handle_by_name(name).unwrap();
    let cancellation = CancellationToken::new();
    let mut output = VecOutput::new();
    let mut context = BuiltinContext::new(1, &cancellation, &mut output);
    registry
        .invoke(handle, arguments, &mut context)
        .map_err(|error| match error {
            openmat_runtime::BuiltinInvocationError::Failed { error, .. } => error,
            openmat_runtime::BuiltinInvocationError::UnknownHandle(_) => unreachable!(),
        })
}

fn only(result: BuiltinResult) -> Value {
    let mut values = result.unwrap();
    assert_eq!(values.len(), 1);
    values.remove(0)
}

fn chars(units: &[u16]) -> Value {
    let shape = if units.is_empty() {
        [0, 0]
    } else {
        [1, units.len() as u64]
    };
    char_array(shape, units)
}

fn char_array(shape: impl IntoIterator<Item = u64>, units: &[u16]) -> Value {
    Value::Array(ArrayData::Char(
        DenseArray::from_vec(
            Shape::new(shape).unwrap(),
            units.iter().copied().map(CharCodeUnit::new).collect(),
        )
        .unwrap(),
    ))
}

fn string_array(shape: impl IntoIterator<Item = u64>, values: Vec<StringElement>) -> Value {
    Value::String(StringValue::array(
        StringArray::from_elements(Shape::new(shape).unwrap(), values).unwrap(),
    ))
}

fn cell(shape: impl IntoIterator<Item = u64>, values: Vec<Value>) -> Value {
    Value::Cell(CellArray::from_values(Shape::new(shape).unwrap(), values).unwrap())
}

fn assert_logicals(value: &Value, shape: &[u64], expected: &[bool]) {
    let Value::Array(ArrayData::Logical(array)) = value else {
        panic!("expected logical array: {value:?}")
    };
    assert_eq!(array.shape().dimensions(), shape);
    assert_eq!(
        array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
        expected
    );
}

fn assert_doubles(value: &Value, shape: &[u64], expected: &[f64]) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected double array: {value:?}")
    };
    assert_eq!(array.shape().dimensions(), shape);
    for (actual, expected) in array.as_slice().iter().zip(expected) {
        assert!((actual.is_nan() && expected.is_nan()) || actual == expected);
    }
}

fn char_units(value: &Value) -> (&[u64], Vec<u16>) {
    let Value::Array(ArrayData::Char(array)) = value else {
        panic!("expected char: {value:?}")
    };
    (
        array.shape().dimensions(),
        array.as_slice().iter().map(|unit| unit.get()).collect(),
    )
}

#[test]
fn registers_the_complete_high_frequency_string_family() {
    let registry = minimal_registry().unwrap();
    for name in [
        "strcmpi",
        "strncmp",
        "strncmpi",
        "strlength",
        "upper",
        "lower",
        "reverse",
        "strip",
        "strtrim",
        "deblank",
        "contains",
        "startsWith",
        "endsWith",
        "strfind",
        "strrep",
        "erase",
        "replace",
    ] {
        assert!(registry.handle_by_name(name).is_some(), "missing {name}");
    }
}

#[test]
fn comparisons_preserve_shapes_missing_and_utf16_prefix_indices() {
    assert_eq!(
        only(invoke("strcmpi", &[chars(&[0x00e9]), chars(&[0x00c9])])),
        Value::Logical(true)
    );
    assert_eq!(
        only(invoke(
            "strcmpi",
            &[chars(&[0xd801, 0xdc28]), chars(&[0xd801, 0xdc00])]
        )),
        Value::Logical(false)
    );
    assert_eq!(
        only(invoke(
            "strncmp",
            &[
                chars(&[b'x' as u16, 0xd83d, 0xde00, b'z' as u16]),
                chars(&[b'x' as u16, 0xd83d, 0xde00, b'q' as u16]),
                Value::Double(3.0)
            ]
        )),
        Value::Logical(true)
    );
    assert_eq!(
        only(invoke(
            "strncmp",
            &[
                chars(&[b'a' as u16]),
                chars(&[b'b' as u16]),
                Value::Double(-1.0)
            ]
        )),
        Value::Logical(true)
    );

    let left = string_array(
        [2, 2],
        vec![
            StringElement::from("A"),
            StringElement::from("b"),
            StringElement::missing(),
            StringElement::from(""),
        ],
    );
    let compared = only(invoke(
        "strcmpi",
        &[left, Value::String(StringValue::scalar("a"))],
    ));
    assert_logicals(&compared, &[2, 2], &[true, false, false, false]);

    let cells = cell(
        [1, 3],
        vec![chars(&[b'A' as u16]), Value::Double(1.0), chars(&[])],
    );
    let compared = only(invoke("strcmpi", &[cells, chars(&[b'a' as u16])]));
    assert_logicals(&compared, &[1, 3], &[true, false, false]);
}

#[test]
fn length_case_reverse_and_trim_match_r2022b_class_and_shape_rules() {
    assert_eq!(
        only(invoke(
            "strlength",
            &[chars(&[b'a' as u16, 0xd83d, 0xde00])]
        )),
        Value::Double(3.0)
    );
    let lengths = only(invoke(
        "strlength",
        &[string_array(
            [1, 3],
            vec![
                StringElement::from("a"),
                StringElement::from_code_units(vec![0xd83d, 0xde00]),
                StringElement::missing(),
            ],
        )],
    ));
    assert_doubles(&lengths, &[1, 3], &[1.0, 2.0, f64::NAN]);

    let matrix = char_array([2, 2], &[b'a' as u16, 0x00e9, 0x00df, b'z' as u16]);
    let upper = only(invoke("upper", &[matrix]));
    assert_eq!(
        char_units(&upper),
        (&[2, 2][..], vec![b'A' as u16, 0x00c9, 0x00df, b'Z' as u16])
    );
    let upper = only(invoke("upper", &[Value::String(StringValue::scalar("aß"))]));
    assert_eq!(
        upper.as_string_scalar().unwrap().code_units(),
        &[b'A' as u16, b'S' as u16, b'S' as u16]
    );
    let upper_cell = only(invoke(
        "upper",
        &[cell([1, 1], vec![chars(&[b'a' as u16, 0x00df])])],
    ));
    let Value::Cell(upper_cell) = upper_cell else {
        panic!("expected cellstr output")
    };
    assert_eq!(
        char_units(&upper_cell.values()[0]).1,
        vec![b'A' as u16, 0x00df]
    );

    let reversed = only(invoke(
        "reverse",
        &[chars(&[b'a' as u16, 0xd83d, 0xde00, b'b' as u16])],
    ));
    assert_eq!(
        char_units(&reversed).1,
        vec![b'b' as u16, 0xde00, 0xd83d, b'a' as u16]
    );

    let stripped = only(invoke(
        "strip",
        &[chars(&[b' ' as u16, b'a' as u16, b' ' as u16])],
    ));
    assert_eq!(char_units(&stripped), (&[1, 1][..], vec![b'a' as u16]));
    let trimmed = only(invoke(
        "strtrim",
        &[char_array(
            [2, 4],
            &[
                b' ' as u16,
                b'b' as u16,
                b'a' as u16,
                b'c' as u16,
                b' ' as u16,
                b' ' as u16,
                b' ' as u16,
                b' ' as u16,
            ],
        )],
    ));
    assert_eq!(
        char_units(&trimmed),
        (
            &[2, 2][..],
            vec![b' ' as u16, b'b' as u16, b'a' as u16, b'c' as u16]
        )
    );
    let deblanked = only(invoke(
        "deblank",
        &[chars(&[b'a' as u16, b'\t' as u16, b' ' as u16, 0])],
    ));
    assert_eq!(char_units(&deblanked), (&[1, 1][..], vec![b'a' as u16]));
}

#[test]
fn predicates_and_strfind_use_pattern_sets_and_utf16_positions() {
    let input = string_array(
        [2, 1],
        vec![StringElement::from("Abc"), StringElement::missing()],
    );
    let patterns = string_array(
        [1, 2],
        vec![StringElement::from("b"), StringElement::from("z")],
    );
    let result = only(invoke(
        "contains",
        &[
            input,
            patterns,
            chars(&"IgnoreCase".encode_utf16().collect::<Vec<_>>()),
            Value::Logical(true),
        ],
    ));
    assert_logicals(&result, &[2, 1], &[true, false]);
    assert_eq!(
        only(invoke("startsWith", &[chars(&[b'a' as u16]), chars(&[])])),
        Value::Logical(true)
    );
    assert_eq!(
        only(invoke(
            "endsWith",
            &[chars(&[b'a' as u16, b'b' as u16]), chars(&[b'b' as u16])]
        )),
        Value::Logical(true)
    );

    let found = only(invoke(
        "strfind",
        &[
            chars(&[b'a' as u16, 0xd83d, 0xde00, b'b' as u16, 0xd83d, 0xde00]),
            chars(&[0xd83d, 0xde00]),
        ],
    ));
    assert_doubles(&found, &[1, 2], &[2.0, 5.0]);
    let cell_found = only(invoke(
        "strfind",
        &[
            cell(
                [1, 2],
                vec![
                    chars(&[b'a' as u16, b'b' as u16]),
                    chars(&[b'b' as u16, b'a' as u16]),
                ],
            ),
            chars(&[b'a' as u16]),
        ],
    ));
    let Value::Cell(cell_found) = cell_found else {
        panic!("expected cell")
    };
    assert_doubles(&cell_found.values()[0], &[1, 1], &[1.0]);
    assert_doubles(&cell_found.values()[1], &[1, 1], &[2.0]);
}

#[test]
fn replacement_family_preserves_input_class_missing_and_scalar_expansion() {
    let replaced = only(invoke(
        "strrep",
        &[
            chars(&[b'a' as u16, 0xd83d, 0xde00, b'a' as u16]),
            chars(&[b'a' as u16]),
            chars(&[b'x' as u16, b'y' as u16]),
        ],
    ));
    assert_eq!(
        char_units(&replaced).1,
        vec![
            b'x' as u16,
            b'y' as u16,
            0xd83d,
            0xde00,
            b'x' as u16,
            b'y' as u16
        ]
    );

    let erased = only(invoke(
        "erase",
        &[
            chars(&[
                b'a' as u16,
                b'b' as u16,
                b'c' as u16,
                b'a' as u16,
                b'b' as u16,
                b'c' as u16,
            ]),
            cell([1, 2], vec![chars(&[b'a' as u16]), chars(&[b'c' as u16])]),
        ],
    ));
    assert_eq!(char_units(&erased).1, vec![b'b' as u16, b'b' as u16]);

    let input = string_array(
        [1, 2],
        vec![StringElement::from("abc"), StringElement::missing()],
    );
    let patterns = string_array(
        [1, 2],
        vec![StringElement::from("a"), StringElement::from("b")],
    );
    let replaced = only(invoke(
        "replace",
        &[input, patterns, Value::String(StringValue::scalar("X"))],
    ));
    let Value::String(StringValue::Array(replaced)) = replaced else {
        panic!("expected string array")
    };
    assert_eq!(
        replaced.as_slice()[0].code_units(),
        &[b'X' as u16, b'X' as u16, b'c' as u16]
    );
    assert!(replaced.as_slice()[1].is_missing());

    let inserted = only(invoke(
        "replace",
        &[
            chars(&[b'a' as u16, b'b' as u16]),
            chars(&[]),
            chars(&[b'x' as u16]),
        ],
    ));
    assert_eq!(
        char_units(&inserted).1,
        vec![
            b'x' as u16,
            b'a' as u16,
            b'x' as u16,
            b'b' as u16,
            b'x' as u16
        ]
    );
}

#[test]
fn rejects_r2022b_error_boundaries() {
    assert!(matches!(
        invoke("strlength", &[char_array([2, 2], &[b'a' as u16; 4])]),
        Err(BuiltinError { .. })
    ));
    assert!(invoke("reverse", &[char_array([2, 2], &[b'a' as u16; 4])]).is_err());
    assert!(
        invoke(
            "contains",
            &[
                chars(&[b'a' as u16]),
                chars(&[b'a' as u16]),
                chars(&[b'x' as u16]),
                Value::Logical(true)
            ]
        )
        .is_err()
    );
    assert!(
        invoke(
            "replace",
            &[
                chars(&[b'a' as u16]),
                cell([1, 2], vec![chars(&[b'a' as u16]), chars(&[b'b' as u16])]),
                cell([2, 1], vec![chars(&[b'x' as u16]), chars(&[b'y' as u16])])
            ]
        )
        .is_err()
    );
}

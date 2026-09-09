use openmat_array::{ArrayData, DenseArray, Shape};
use openmat_builtins::minimal_registry;
use openmat_runtime::{BuiltinContext, CancellationToken, VecOutput};
use openmat_value::{CellArray, FieldName, StructArray, Value};

fn invoke(name: &str, arguments: &[Value], outputs: usize) -> Result<Vec<Value>, String> {
    let registry = minimal_registry().unwrap();
    let mut output = VecOutput::new();
    let cancellation = CancellationToken::new();
    let mut context = BuiltinContext::new(outputs, &cancellation, &mut output);
    registry
        .invoke(
            registry.handle_by_name(name).unwrap(),
            arguments,
            &mut context,
        )
        .map_err(|e| e.to_string())
}

#[test]
fn argument_validators_are_callable_and_reject_output_requests() {
    for name in [
        "mustBeNumeric",
        "mustBeFloat",
        "mustBeReal",
        "mustBeFinite",
        "mustBeNonNan",
        "mustBePositive",
        "mustBeNonnegative",
        "mustBeNonzero",
        "mustBeInteger",
        "mustBeNonempty",
    ] {
        assert!(
            invoke(name, &[Value::Double(2.0)], 0).unwrap().is_empty(),
            "{name}"
        );
        assert!(invoke(name, &[Value::Double(2.0)], 1).is_err(), "{name}");
        assert!(invoke(name, &[], 0).is_err(), "{name}");
    }
    for (name, value) in [
        ("mustBePositive", 0.0),
        ("mustBeNegative", 0.0),
        ("mustBeNonnegative", -1.0),
        ("mustBeNonpositive", 1.0),
        ("mustBeFinite", f64::INFINITY),
        ("mustBeNonNan", f64::NAN),
        ("mustBeInteger", 1.5),
        ("mustBeNonzero", 0.0),
    ] {
        assert!(invoke(name, &[Value::Double(value)], 0).is_err(), "{name}");
    }
}

#[test]
fn argument_validation_preserves_empty_and_complex_rules() {
    for name in [
        "mustBePositive",
        "mustBeInteger",
        "mustBeFinite",
        "mustBeNonzero",
    ] {
        invoke(name, &[Value::empty_double()], 0).unwrap();
    }
    assert!(invoke("mustBeNonempty", &[Value::empty_double()], 0).is_err());
    let complex = Value::Complex(openmat_value::Complex64::new(0.0, 1.0));
    invoke("mustBeNonzero", std::slice::from_ref(&complex), 0).unwrap();
    invoke("mustBeFinite", std::slice::from_ref(&complex), 0).unwrap();
    assert!(invoke("mustBeReal", std::slice::from_ref(&complex), 0).is_err());
    assert!(invoke("mustBePositive", &[complex], 0).is_err());
}

#[test]
fn option_fields_are_reordered_without_creating_missing_fields() {
    let field = |name: &str| FieldName::new(name).unwrap();
    let structure = Value::Struct(
        StructArray::from_columns(
            Shape::new([1, 1]).unwrap(),
            vec![field("B"), field("A")],
            vec![vec![Value::Double(2.0)], vec![Value::Double(1.0)]],
        )
        .unwrap(),
    );
    let order = Value::Cell(
        CellArray::from_values(
            Shape::new([1, 3]).unwrap(),
            vec![Value::from("A"), Value::from("B"), Value::from("C")],
        )
        .unwrap(),
    );
    let output = invoke("__openmat_order_argument_fields", &[structure, order], 1).unwrap();
    let Value::Struct(value) = &output[0] else {
        panic!("struct expected")
    };
    assert_eq!(value.field_names(), &[field("A"), field("B")]);
    assert_eq!(
        invoke("isfield", &[output[0].clone(), Value::from("A")], 1).unwrap(),
        vec![Value::Logical(true)]
    );
    assert_eq!(
        invoke("isfield", &[output[0].clone(), Value::from("C")], 1).unwrap(),
        vec![Value::Logical(false)]
    );
}

#[test]
fn argument_dimensions_reject_invalid_extents_and_do_not_reshape_matrices() {
    let vector = |values: Vec<f64>| {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([1, values.len() as u64]).unwrap(), values).unwrap(),
        ))
    };
    for specification in [
        vec![1.0, -2.0],
        vec![1.0, 1.5],
        vec![1.0, f64::NAN],
        vec![1.0, f64::INFINITY],
        vec![1.0],
    ] {
        assert!(
            invoke(
                "__openmat_validate_argument_size",
                &[Value::Double(1.0), vector(specification)],
                1
            )
            .is_err()
        );
    }
    assert!(
        invoke(
            "__openmat_validate_argument_size",
            &[vector(vec![1.0, 2.0, 3.0]), vector(vec![2.0, 3.0])],
            1
        )
        .is_err()
    );
}

#[test]
fn range_validators_enforce_arity_output_counts_and_scalar_bounds() {
    for name in [
        "mustBeGreaterThan",
        "mustBeGreaterThanOrEqual",
        "mustBeLessThan",
        "mustBeLessThanOrEqual",
    ] {
        assert!(invoke(name, &[], 0).is_err());
        assert!(invoke(name, &[Value::Double(1.0)], 0).is_err());
        assert!(invoke(name, &[Value::Double(1.0), Value::Double(2.0)], 1).is_err());
        assert!(invoke(name, &[Value::empty_double(), Value::empty_double()], 0).is_err());
        assert!(invoke(name, &[Value::Double(f64::NAN), Value::Double(0.0)], 0).is_err());
        assert!(invoke(name, &[Value::from("text"), Value::Double(0.0)], 0).is_err());
        invoke(name, &[Value::empty_double(), Value::Double(f64::NAN)], 0).unwrap();
    }
    assert!(invoke("mustBeInRange", &[const { Value::Double(0.0) }; 2], 0).is_err());
    assert!(invoke("mustBeInRange", &[const { Value::Double(0.0) }; 6], 0).is_err());
    assert!(invoke("mustBeInRange", &[const { Value::Double(0.0) }; 3], 1).is_err());
}

#[test]
fn range_validators_preserve_int64_and_uint64_precision() {
    use openmat_array::IntegerArrayData;
    let u = |n| {
        Value::Array(ArrayData::Integer(IntegerArrayData::U64(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![n]).unwrap(),
        )))
    };
    let i = |n| {
        Value::Array(ArrayData::Integer(IntegerArrayData::I64(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![n]).unwrap(),
        )))
    };
    invoke("mustBeGreaterThan", &[u(u64::MAX), u(u64::MAX - 1)], 0).unwrap();
    assert!(invoke("mustBeGreaterThan", &[u(u64::MAX - 1), u(u64::MAX)], 0).is_err());
    invoke("mustBeLessThan", &[i(i64::MIN), i(i64::MIN + 1)], 0).unwrap();
    invoke("mustBeInRange", &[u(u64::MAX), u(u64::MAX), u(u64::MAX)], 0).unwrap();
    invoke(
        "mustBeGreaterThan",
        &[
            u(9_007_199_254_740_993),
            Value::Double(9_007_199_254_740_992.0),
        ],
        0,
    )
    .unwrap();
    invoke(
        "mustBeLessThan",
        &[u(u64::MAX), Value::Double(18_446_744_073_709_551_616.0)],
        0,
    )
    .unwrap();
    invoke("mustBeGreaterThan", &[u(u64::MAX), i(-1)], 0).unwrap();
    invoke("mustBeLessThan", &[i(-1), Value::Double(-0.5)], 0).unwrap();
    invoke("mustBeGreaterThan", &[i(-1), Value::Double(-1.5)], 0).unwrap();
    invoke(
        "mustBeLessThan",
        &[i(i64::MIN), Value::Double(f64::INFINITY)],
        0,
    )
    .unwrap();
    invoke(
        "mustBeGreaterThan",
        &[i(i64::MIN), Value::Double(f64::NEG_INFINITY)],
        0,
    )
    .unwrap();
}

#[test]
fn range_flags_match_r2022b_prefix_and_combination_rules() {
    for flags in [
        vec![],
        vec!["inclusive"],
        vec!["exclusive"],
        vec!["exclude-lower"],
        vec!["exclude-upper"],
        vec!["exclude-lower", "exclude-upper"],
        vec!["exclude-upper", "exclude-lower"],
        vec!["INC"],
        vec!["in"],
    ] {
        let mut arguments = vec![Value::Double(0.5), Value::Double(0.0), Value::Double(1.0)];
        arguments.extend(flags.iter().map(|flag| Value::from(*flag)));
        invoke("mustBeInRange", &arguments, 0).unwrap();
    }
    for flags in [
        vec![""],
        vec!["exc"],
        vec!["bogus"],
        vec!["inclusive", "exclusive"],
        vec!["exclusive", "exclusive"],
        vec!["inclusive", "inclusive"],
        vec!["exclude-lower", "inclusive"],
        vec!["exclude-lower", "exclude-lower"],
    ] {
        let mut arguments = vec![Value::Double(0.5), Value::Double(0.0), Value::Double(1.0)];
        arguments.extend(flags.iter().map(|flag| Value::from(*flag)));
        assert!(invoke("mustBeInRange", &arguments, 0).is_err(), "{flags:?}");
    }
}

#[test]
fn range_validators_check_cancellation_even_for_empty_inputs() {
    let registry = minimal_registry().unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut output = VecOutput::new();
    let mut context = BuiltinContext::new(0, &cancellation, &mut output);
    assert!(
        registry
            .invoke(
                registry.handle_by_name("mustBeGreaterThan").unwrap(),
                &[Value::empty_double(), Value::Double(0.0)],
                &mut context
            )
            .is_err()
    );
}

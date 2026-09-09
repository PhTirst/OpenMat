use openmat_array::{ArrayData, Complex64 as ArrayComplex64, DenseArray};
use openmat_builtins::{
    minimal_registry,
    quadrature::{AdaptiveQuadratureOptions, adaptive_gauss_kronrod_callable},
};
use openmat_bytecode::{
    ApplyArgument, BinaryOperator, BytecodeModule, Constant, ConstantId, Function, FunctionId,
    Instruction, InstructionKind, LocalSlot, Register,
};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistry, Interpreter,
    RuntimeErrorKind,
};
use openmat_value::Value;

fn quadrature_registry() -> BuiltinRegistry {
    let mut registry = minimal_registry().unwrap();
    registry
        .register(
            "test_adaptive_quadrature",
            |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                let integrated = adaptive_gauss_kronrod_callable(
                    context,
                    &arguments[0],
                    0.0,
                    1.0,
                    AdaptiveQuadratureOptions {
                        absolute_tolerance: 1.0e-12,
                        relative_tolerance: 1.0e-12,
                        maximum_intervals: 100,
                    },
                )?;
                Ok(vec![Value::Double(integrated.value.re)])
            },
        )
        .unwrap();
    registry
}

fn entry(callback_name: &str) -> Function {
    Function {
        name: "quadrature_entry".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String(callback_name.to_owned()),
            Constant::String("test_adaptive_quadrature".to_owned()),
        ],
        instructions: vec![
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(1),
                name: ConstantId::new(1),
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(1),
                arguments: vec![ApplyArgument::Value(Register::new(0))],
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    }
}

fn square_callback() -> Function {
    Function {
        name: "square_callback".to_owned(),
        register_count: 2,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: Vec::new(),
        instructions: vec![
            Instruction::new(InstructionKind::LoadLocal {
                dst: Register::new(0),
                local: LocalSlot::new(0),
            }),
            Instruction::new(InstructionKind::Binary {
                operator: BinaryOperator::ElementMultiply,
                dst: Register::new(1),
                lhs: Register::new(0),
                rhs: Register::new(0),
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(1)],
            }),
        ],
        exception_handlers: Vec::new(),
    }
}

fn quadgk_entry(callback_name: &str) -> Function {
    Function {
        name: "quadgk_entry".to_owned(),
        register_count: 10,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String(callback_name.to_owned()),
            Constant::String("quadgk".to_owned()),
            Constant::Double(0.0),
            Constant::Double(1.0),
            Constant::String("AbsTol".to_owned()),
            Constant::Double(1.0e-12),
            Constant::String("RelTol".to_owned()),
            Constant::Double(0.0),
        ],
        instructions: vec![
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(1),
                name: ConstantId::new(1),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(3),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(4),
                constant: ConstantId::new(4),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(5),
                constant: ConstantId::new(5),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(6),
                constant: ConstantId::new(6),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(7),
                constant: ConstantId::new(7),
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(8), Register::new(9)],
                target: Register::new(1),
                arguments: vec![
                    ApplyArgument::Value(Register::new(0)),
                    ApplyArgument::Value(Register::new(2)),
                    ApplyArgument::Value(Register::new(3)),
                    ApplyArgument::Value(Register::new(4)),
                    ApplyArgument::Value(Register::new(5)),
                    ApplyArgument::Value(Register::new(6)),
                    ApplyArgument::Value(Register::new(7)),
                ],
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(8), Register::new(9)],
            }),
        ],
        exception_handlers: Vec::new(),
    }
}

fn improper_quadgk_entry(callback_name: &str, start: f64, end: f64, waypoint: f64) -> Function {
    Function {
        name: "improper_quadgk_entry".to_owned(),
        register_count: 8,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String(callback_name.to_owned()),
            Constant::String("quadgk".to_owned()),
            Constant::Double(start),
            Constant::Double(end),
            Constant::String("Waypoints".to_owned()),
            Constant::Double(waypoint),
        ],
        instructions: vec![
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(1),
                name: ConstantId::new(1),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(3),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(4),
                constant: ConstantId::new(4),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(5),
                constant: ConstantId::new(5),
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(6), Register::new(7)],
                target: Register::new(1),
                arguments: vec![
                    ApplyArgument::Value(Register::new(0)),
                    ApplyArgument::Value(Register::new(2)),
                    ApplyArgument::Value(Register::new(3)),
                    ApplyArgument::Value(Register::new(4)),
                    ApplyArgument::Value(Register::new(5)),
                ],
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(6), Register::new(7)],
            }),
        ],
        exception_handlers: Vec::new(),
    }
}

#[allow(clippy::too_many_lines)]
fn contour_quadgk_entry(
    callback_name: &str,
    start: (f64, f64),
    end: (f64, f64),
    waypoints: &[(f64, f64)],
) -> Function {
    let mut constants = vec![
        Constant::String(callback_name.to_owned()),
        Constant::String("quadgk".to_owned()),
        Constant::Complex {
            real: start.0,
            imaginary: start.1,
        },
        Constant::Complex {
            real: end.0,
            imaginary: end.1,
        },
        Constant::String("Waypoints".to_owned()),
    ];
    constants.extend(waypoints.iter().map(|waypoint| Constant::Complex {
        real: waypoint.0,
        imaginary: waypoint.1,
    }));
    let matrix_register = u32::try_from(constants.len()).unwrap();
    let output_register = matrix_register + 1;
    let error_register = matrix_register + 2;
    let mut instructions = vec![
        Instruction::new(InstructionKind::LoadFunctionHandle {
            dst: Register::new(0),
            name: ConstantId::new(0),
        }),
        Instruction::new(InstructionKind::LoadFunctionHandle {
            dst: Register::new(1),
            name: ConstantId::new(1),
        }),
    ];
    for index in 2..matrix_register {
        instructions.push(Instruction::new(InstructionKind::LoadConstant {
            dst: Register::new(index),
            constant: ConstantId::new(index),
        }));
    }
    instructions.push(Instruction::new(InstructionKind::BuildMatrix {
        dst: Register::new(matrix_register),
        rows: vec![(5..matrix_register).map(Register::new).collect()],
    }));
    instructions.push(Instruction::new(InstructionKind::Apply {
        outputs: vec![
            Register::new(output_register),
            Register::new(error_register),
        ],
        target: Register::new(1),
        arguments: vec![
            ApplyArgument::Value(Register::new(0)),
            ApplyArgument::Value(Register::new(2)),
            ApplyArgument::Value(Register::new(3)),
            ApplyArgument::Value(Register::new(4)),
            ApplyArgument::Value(Register::new(matrix_register)),
        ],
    }));
    instructions.push(Instruction::new(InstructionKind::Return {
        values: vec![
            Register::new(output_register),
            Register::new(error_register),
        ],
    }));
    Function {
        name: "contour_quadgk_entry".to_owned(),
        register_count: error_register + 1,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants,
        instructions,
        exception_handlers: Vec::new(),
    }
}

#[allow(clippy::too_many_lines)]
fn warning_quadgk_entry(
    callback_name: &str,
    start: f64,
    end: f64,
    absolute_tolerance: f64,
    maximum_intervals: f64,
    waypoint: f64,
) -> Function {
    Function {
        name: "limited_quadgk_entry".to_owned(),
        register_count: 17,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String(callback_name.to_owned()),
            Constant::String("quadgk".to_owned()),
            Constant::Double(start),
            Constant::Double(end),
            Constant::String("AbsTol".to_owned()),
            Constant::Double(absolute_tolerance),
            Constant::String("RelTol".to_owned()),
            Constant::Double(0.0),
            Constant::String("MaxIntervalCount".to_owned()),
            Constant::Double(maximum_intervals),
            Constant::String("Waypoints".to_owned()),
            Constant::Double(waypoint),
            Constant::String("lastwarn".to_owned()),
        ],
        instructions: vec![
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(1),
                name: ConstantId::new(1),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(2),
                constant: ConstantId::new(2),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(3),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(4),
                constant: ConstantId::new(4),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(5),
                constant: ConstantId::new(5),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(6),
                constant: ConstantId::new(6),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(7),
                constant: ConstantId::new(7),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(8),
                constant: ConstantId::new(8),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(9),
                constant: ConstantId::new(9),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(10),
                constant: ConstantId::new(10),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(11),
                constant: ConstantId::new(11),
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(12), Register::new(13)],
                target: Register::new(1),
                arguments: vec![
                    ApplyArgument::Value(Register::new(0)),
                    ApplyArgument::Value(Register::new(2)),
                    ApplyArgument::Value(Register::new(3)),
                    ApplyArgument::Value(Register::new(4)),
                    ApplyArgument::Value(Register::new(5)),
                    ApplyArgument::Value(Register::new(6)),
                    ApplyArgument::Value(Register::new(7)),
                    ApplyArgument::Value(Register::new(8)),
                    ApplyArgument::Value(Register::new(9)),
                    ApplyArgument::Value(Register::new(10)),
                    ApplyArgument::Value(Register::new(11)),
                ],
            }),
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(14),
                name: ConstantId::new(12),
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(15), Register::new(16)],
                target: Register::new(14),
                arguments: Vec::new(),
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(12), Register::new(13), Register::new(16)],
            }),
        ],
        exception_handlers: Vec::new(),
    }
}

#[test]
fn quadgk_builtin_parses_tolerances_and_returns_value_with_error_bound() {
    let module = BytecodeModule::new(
        vec![quadgk_entry("square_callback"), square_callback()],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, minimal_registry().unwrap())
        .unwrap()
        .execute_entry(&[])
        .expect("quadgk should call the vectorized language integrand");
    let [Value::Double(value), Value::Double(error)] = result.as_slice() else {
        panic!("expected the integral and its absolute error estimate")
    };
    assert!((*value - 1.0 / 3.0).abs() <= 1.0e-12);
    assert!(*error <= 1.0e-12);
}

#[test]
fn quadgk_builtin_preserves_complex_integrand_results() {
    let mut registry = minimal_registry().unwrap();
    registry
        .register(
            "complex_exponential_callback",
            |arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                let Value::Array(ArrayData::F64(points)) = &arguments[0] else {
                    panic!("quadgk should supply a double sample array")
                };
                let values = points
                    .as_slice()
                    .iter()
                    .map(|point| ArrayComplex64::new(point.cos(), point.sin()))
                    .collect();
                let output = DenseArray::from_vec(points.shape().clone(), values).unwrap();
                Ok(vec![Value::Array(ArrayData::ComplexF64(output))])
            },
        )
        .unwrap();
    let module = BytecodeModule::new(
        vec![quadgk_entry("complex_exponential_callback")],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect("quadgk should preserve a complex callback result");
    let [Value::Complex(value), Value::Double(error)] = result.as_slice() else {
        panic!("expected a complex integral and a real error estimate")
    };
    assert!((value.real - 1.0_f64.sin()).abs() <= 1.0e-12);
    assert!((value.imaginary - (1.0 - 1.0_f64.cos())).abs() <= 1.0e-12);
    assert!(*error <= 1.0e-12);
}

#[test]
fn quadgk_builtin_maps_straight_complex_paths_and_supplies_complex_samples() {
    let mut registry = minimal_registry().unwrap();
    registry
        .register(
            "identity_contour_callback",
            |arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                let Value::Array(ArrayData::ComplexF64(points)) = &arguments[0] else {
                    panic!("complex contours must supply a complex double sample array")
                };
                assert!(points.as_slice().iter().any(|point| point.im != 0.0));
                Ok(vec![arguments[0].clone()])
            },
        )
        .unwrap();
    let module = BytecodeModule::new(
        vec![contour_quadgk_entry(
            "identity_contour_callback",
            (0.0, 0.0),
            (1.0, 1.0),
            &[(0.0, 0.0)],
        )],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect("quadgk should integrate a straight complex path");
    let [Value::Complex(value), Value::Double(error)] = result.as_slice() else {
        panic!("expected a complex contour integral and its error estimate")
    };
    assert!(value.real.abs() <= 1.0e-12);
    assert!((value.imaginary - 1.0).abs() <= 1.0e-12);
    assert!(*error <= 1.0e-12);
}

#[test]
fn quadgk_builtin_preserves_complex_path_orientation() {
    let mut registry = minimal_registry().unwrap();
    registry
        .register(
            "reverse_identity_contour_callback",
            |arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                let Value::Array(ArrayData::ComplexF64(_)) = &arguments[0] else {
                    panic!("complex contours must supply a complex double sample array")
                };
                Ok(vec![arguments[0].clone()])
            },
        )
        .unwrap();
    let module = BytecodeModule::new(
        vec![contour_quadgk_entry(
            "reverse_identity_contour_callback",
            (1.0, 1.0),
            (0.0, 0.0),
            &[],
        )],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect("quadgk should preserve complex path orientation");
    let [Value::Complex(value), Value::Double(error)] = result.as_slice() else {
        panic!("expected a complex contour integral and its error estimate")
    };
    assert!(value.real.abs() <= 1.0e-12);
    assert!((value.imaginary + 1.0).abs() <= 1.0e-12);
    assert!(*error <= 1.0e-12);
}

#[test]
fn quadgk_builtin_integrates_an_ordered_closed_contour() {
    let mut registry = minimal_registry().unwrap();
    registry
        .register(
            "complex_reciprocal_callback",
            |arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                let Value::Array(ArrayData::ComplexF64(points)) = &arguments[0] else {
                    panic!("complex contours must supply a complex double sample array")
                };
                let values = points
                    .as_slice()
                    .iter()
                    .map(|point| {
                        let denominator = point.re.mul_add(point.re, point.im * point.im);
                        ArrayComplex64::new(point.re / denominator, -point.im / denominator)
                    })
                    .collect();
                let output = DenseArray::from_vec(points.shape().clone(), values).unwrap();
                Ok(vec![Value::Array(ArrayData::ComplexF64(output))])
            },
        )
        .unwrap();
    let module = BytecodeModule::new(
        vec![contour_quadgk_entry(
            "complex_reciprocal_callback",
            (1.0, 0.0),
            (1.0, 0.0),
            &[(0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)],
        )],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect("quadgk should preserve the supplied contour order");
    let [Value::Complex(value), Value::Double(error)] = result.as_slice() else {
        panic!("expected a complex contour integral and its error estimate")
    };
    assert!(value.real.abs() <= 1.0e-10);
    assert!((value.imaginary - 2.0 * std::f64::consts::PI).abs() <= 1.0e-10);
    assert!(*error <= 1.0e-8);
}

#[test]
fn quadgk_builtin_integrates_infinite_domains_with_waypoints_and_orientation() {
    let cases = [
        (0.0, f64::INFINITY, 1.0, 1.0),
        (f64::NEG_INFINITY, 0.0, -1.0, 1.0),
        (f64::NEG_INFINITY, f64::INFINITY, 0.0, 2.0),
        (f64::INFINITY, 0.0, 1.0, -1.0),
        (0.0, f64::NEG_INFINITY, -1.0, -1.0),
        (f64::INFINITY, f64::NEG_INFINITY, 0.0, -2.0),
    ];
    for (start, end, waypoint, expected) in cases {
        let mut registry = minimal_registry().unwrap();
        registry
            .register(
                "two_sided_decay_callback",
                |arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                    let Value::Array(ArrayData::F64(points)) = &arguments[0] else {
                        panic!("quadgk should supply a double sample array")
                    };
                    assert!(points.as_slice().iter().all(|point| point.is_finite()));
                    let values = points
                        .as_slice()
                        .iter()
                        .map(|point| (-point.abs()).exp())
                        .collect();
                    let output = DenseArray::from_vec(points.shape().clone(), values).unwrap();
                    Ok(vec![Value::Array(ArrayData::F64(output))])
                },
            )
            .unwrap();
        let module = BytecodeModule::new(
            vec![improper_quadgk_entry(
                "two_sided_decay_callback",
                start,
                end,
                waypoint,
            )],
            FunctionId::new(0),
        );
        let result = Interpreter::with_registry(module, registry)
            .unwrap()
            .execute_entry(&[])
            .expect("quadgk should integrate a transformed infinite domain");
        let [Value::Double(value), Value::Double(error)] = result.as_slice() else {
            panic!("expected an improper integral and its error estimate")
        };
        assert!((*value - expected).abs() <= 2.0e-8);
        assert!(*error <= 2.0e-6);
    }

    for endpoint in [f64::NEG_INFINITY, f64::INFINITY] {
        let mut registry = minimal_registry().unwrap();
        registry
            .register(
                "unreachable_callback",
                |_arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                    panic!("an indeterminate infinite interval must not invoke its integrand")
                },
            )
            .unwrap();
        let module = BytecodeModule::new(
            vec![improper_quadgk_entry(
                "unreachable_callback",
                endpoint,
                endpoint,
                0.0,
            )],
            FunctionId::new(0),
        );
        let result = Interpreter::with_registry(module, registry)
            .unwrap()
            .execute_entry(&[])
            .expect("equal infinite endpoints should return indeterminate outputs");
        assert!(matches!(
            result.as_slice(),
            [Value::Double(value), Value::Double(error)] if value.is_nan() && error.is_nan()
        ));
    }
}

#[test]
fn quadgk_builtin_integrates_reciprocal_square_tail() {
    let mut registry = minimal_registry().unwrap();
    registry
        .register(
            "reciprocal_square_callback",
            |arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                let Value::Array(ArrayData::F64(points)) = &arguments[0] else {
                    panic!("quadgk should supply a double sample array")
                };
                let values = points
                    .as_slice()
                    .iter()
                    .map(|point| 1.0 / (point * point))
                    .collect();
                let output = DenseArray::from_vec(points.shape().clone(), values).unwrap();
                Ok(vec![Value::Array(ArrayData::F64(output))])
            },
        )
        .unwrap();
    let module = BytecodeModule::new(
        vec![improper_quadgk_entry(
            "reciprocal_square_callback",
            1.0,
            f64::INFINITY,
            2.0,
        )],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect("the convergent reciprocal-square tail should integrate");
    let [Value::Double(value), Value::Double(error)] = result.as_slice() else {
        panic!("expected an improper integral and its error estimate")
    };
    assert!((*value - 1.0).abs() <= 1.0e-10);
    assert!(*error <= 1.0e-6);
}

#[test]
fn quadgk_interval_limit_returns_the_best_estimate_and_records_a_warning() {
    let mut registry = minimal_registry().unwrap();
    registry
        .register(
            "localized_absolute_callback",
            |arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                let Value::Array(ArrayData::F64(points)) = &arguments[0] else {
                    panic!("quadgk should supply a double sample array")
                };
                let values = points
                    .as_slice()
                    .iter()
                    .map(|point| (point - 0.123_456_789).abs())
                    .collect();
                let output = DenseArray::from_vec(points.shape().clone(), values).unwrap();
                Ok(vec![Value::Array(ArrayData::F64(output))])
            },
        )
        .unwrap();
    let module = BytecodeModule::new(
        vec![warning_quadgk_entry(
            "localized_absolute_callback",
            0.0,
            1.0,
            1.0e-14,
            1.0,
            0.75,
        )],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect("the interval limit must return its best estimate instead of failing");
    let [
        Value::Double(value),
        Value::Double(error),
        Value::Array(ArrayData::Char(identifier)),
    ] = result.as_slice()
    else {
        panic!("expected the estimate, error bound, and last-warning identifier")
    };
    assert!(value.is_finite());
    assert!(*error > 1.0e-14);
    let identifier = String::from_utf16(
        &identifier
            .as_slice()
            .iter()
            .map(|unit| unit.get())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(identifier, "MATLAB:quadgk:MaxIntervalCountReached");
}

#[test]
fn quadgk_minimum_step_returns_the_best_estimate_and_records_a_warning() {
    let mut registry = minimal_registry().unwrap();
    registry
        .register(
            "constant_callback",
            |arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                let Value::Array(ArrayData::F64(points)) = &arguments[0] else {
                    panic!("quadgk should supply a double sample array")
                };
                let output = DenseArray::from_vec(
                    points.shape().clone(),
                    vec![1.0; points.as_slice().len()],
                )
                .unwrap();
                Ok(vec![Value::Array(ArrayData::F64(output))])
            },
        )
        .unwrap();
    let start = 1.0_f64;
    let adjacent = f64::from_bits(start.to_bits() + 1);
    let module = BytecodeModule::new(
        vec![warning_quadgk_entry(
            "constant_callback",
            start,
            adjacent,
            f64::MIN_POSITIVE,
            650.0,
            start,
        )],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect("floating-point stagnation must return its best estimate instead of failing");
    let [
        Value::Double(value),
        Value::Double(error),
        Value::Array(ArrayData::Char(identifier)),
    ] = result.as_slice()
    else {
        panic!("expected the estimate, error bound, and last-warning identifier")
    };
    assert!((*value - (adjacent - start)).abs() <= f64::EPSILON);
    assert!(*error > f64::MIN_POSITIVE);
    let identifier = String::from_utf16(
        &identifier
            .as_slice()
            .iter()
            .map(|unit| unit.get())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(identifier, "MATLAB:quadgk:MinStepSize");
}

#[test]
fn adaptive_core_evaluates_a_vectorized_language_function_handle() {
    let module = BytecodeModule::new(
        vec![entry("square_callback"), square_callback()],
        FunctionId::new(0),
    );
    let result = Interpreter::with_registry(module, quadrature_registry())
        .unwrap()
        .execute_entry(&[])
        .expect("adaptive quadrature should invoke the language callback");
    let [Value::Double(value)] = result.as_slice() else {
        panic!("expected one real quadrature result")
    };
    assert!((*value - 1.0 / 3.0).abs() <= 1.0e-12);
}

#[test]
fn adaptive_core_preserves_the_original_callback_exception() {
    let mut registry = quadrature_registry();
    registry
        .register(
            "failing_callback",
            |_arguments: &[Value], _context: &mut BuiltinContext<'_>| {
                Err(
                    BuiltinError::new(BuiltinErrorCategory::Domain, "integrand exploded")
                        .with_identifier("Test:IntegrandFailure"),
                )
            },
        )
        .unwrap();
    let module = BytecodeModule::new(vec![quadgk_entry("failing_callback")], FunctionId::new(0));
    let error = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect_err("the callback exception must escape the adaptive core");
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Builtin {
            ref name,
            category: BuiltinErrorCategory::Domain,
            identifier: Some(ref identifier),
            ref message,
        } if name == "failing_callback"
            && identifier == "Test:IntegrandFailure"
            && message == "integrand exploded"
    ));
}

#[test]
fn adaptive_core_preserves_callback_cancellation() {
    let mut registry = quadrature_registry();
    registry
        .register(
            "cancelling_callback",
            |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                context.cancellation().cancel();
                Ok(vec![arguments[0].clone()])
            },
        )
        .unwrap();
    let module = BytecodeModule::new(
        vec![quadgk_entry("cancelling_callback")],
        FunctionId::new(0),
    );
    let error = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect_err("callback cancellation must stop adaptive quadrature");
    assert_eq!(error.kind, RuntimeErrorKind::Cancelled);
}

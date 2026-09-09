#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32, Complex64, ComplexInteger, DenseArray, IntegerArrayData,
    Logical, Shape,
};
use openmat_bytecode::{
    ApplyArgument, BytecodeModule, Constant, ConstantId, Function, FunctionId, Instruction,
    InstructionKind, Register,
};
use openmat_oex::{OexLoadError, OexPlugin};
use openmat_runtime::{
    BuiltinContext, BuiltinErrorCategory, BuiltinInvocationError, BuiltinRegistry,
    CancellationToken, Interpreter, NullOutput, RuntimeErrorKind,
};
use openmat_value::{SparseArrayData, Value};

fn bridge_dll() -> PathBuf {
    let executable = std::env::current_exe().expect("locate test executable");
    let deps = executable.parent().expect("test executable directory");
    let profile = deps.parent().expect("target profile directory");
    for candidate in [
        deps.join("openmat_oex.dll"),
        profile.join("openmat_oex.dll"),
    ] {
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("cargo did not build the openmat_oex.dll bridge");
}

#[test]
fn real_c_container_plugin_preserves_encoding_nested_values_and_atomicity() {
    if Command::new("gcc").arg("--version").output().is_err() {
        eprintln!("skipping real C container test because gcc is unavailable");
        return;
    }
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let directory = repository.join("target/oex-container-c-test");
    std::fs::create_dir_all(&directory).unwrap();
    let bridge = bridge_dll();
    std::fs::copy(&bridge, directory.join("openmat_oex.dll")).unwrap();
    let dll = directory.join("containers.oex.dll");
    compile_fixture(
        &repository.join("crates/openmat-oex/tests/fixtures/container_plugin.c"),
        &dll,
        repository,
        &bridge,
    );
    // SAFETY: the plugin is compiled above from the checked-in fixture.
    let plugin = unsafe { OexPlugin::load(&dll) }.unwrap();
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let outputs = invoke(&registry, "oex_container_test", &[]).unwrap();
    let Value::Table(table) = &outputs[0] else {
        panic!("expected a nested table");
    };
    assert_eq!(table.row_count(), 2);
    assert_eq!(table.variable_count(), 2);
    assert_eq!(
        table.variable(0).unwrap().dimensions(),
        Some([2, 2].as_slice())
    );
}

#[test]
fn real_cpp_container_plugin_uses_exceptions_and_owning_values() {
    if Command::new("g++").arg("--version").output().is_err() {
        eprintln!("skipping real C++ container test because g++ is unavailable");
        return;
    }
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let directory = repository.join("target/oex-container-cpp-test");
    std::fs::create_dir_all(&directory).unwrap();
    let bridge = bridge_dll();
    std::fs::copy(&bridge, directory.join("openmat_oex.dll")).unwrap();
    let dll = directory.join("containers.oex.dll");
    let result = Command::new("g++")
        .args([
            "-shared",
            "-std=c++20",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-static-libgcc",
            "-static-libstdc++",
            "-static",
            "-I",
        ])
        .arg(repository.join("include"))
        .arg(repository.join("crates/openmat-oex/tests/fixtures/container_plugin.cpp"))
        .arg(&bridge)
        .arg("-o")
        .arg(&dll)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // SAFETY: the plugin is compiled above from the checked-in C++ fixture.
    let plugin = unsafe { OexPlugin::load(&dll) }.unwrap();
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    assert!(matches!(
        invoke(&registry, "oex_cpp_container_test", &[]).unwrap()[0],
        Value::Table(_)
    ));
}

fn compile_fixture(source: &Path, output: &Path, repository: &Path, bridge: &Path) {
    compile_fixture_with_arguments(source, output, repository, bridge, &[]);
}

fn compile_fixture_with_arguments(
    source: &Path,
    output: &Path,
    repository: &Path,
    bridge: &Path,
    arguments: &[&str],
) {
    let mut command = Command::new("gcc");
    command
        .arg("-shared")
        .arg("-std=c11")
        .arg("-O2")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror");
    command.args(arguments);
    let status = command
        .arg("-I")
        .arg(repository.join("include"))
        .arg(source)
        .arg(bridge)
        .arg("-o")
        .arg(output)
        .status()
        .expect("run gcc for OEX fixture");
    assert!(status.success(), "C OEX fixture must compile cleanly");
}

fn compiled_fixtures() -> Option<(PathBuf, PathBuf)> {
    if Command::new("gcc").arg("--version").output().is_err() {
        eprintln!("skipping real C OEX test because gcc is unavailable");
        return None;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest.parent()?.parent()?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("openmat-oex-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create OEX fixture directory");
    let bridge = bridge_dll();
    std::fs::copy(&bridge, directory.join("openmat_oex.dll"))
        .expect("copy OEX bridge beside plugin");
    let arithmetic = directory.join("arithmetic.oex.dll");
    let invalid = directory.join("invalid.oex.dll");
    compile_fixture(
        &manifest.join("tests/fixtures/arithmetic_plugin.c"),
        &arithmetic,
        repository,
        &bridge,
    );
    compile_fixture(
        &manifest.join("tests/fixtures/invalid_descriptor_plugin.c"),
        &invalid,
        repository,
        &bridge,
    );
    Some((arithmetic, invalid))
}

fn invoke(
    registry: &BuiltinRegistry,
    name: &str,
    arguments: &[Value],
) -> Result<Vec<Value>, BuiltinInvocationError> {
    let handle = registry
        .handle_by_name(name)
        .expect("registered OEX function");
    let cancellation = CancellationToken::new();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(1, &cancellation, &mut output);
    registry.invoke(handle, arguments, &mut context)
}

fn invoke_owned(
    registry: &BuiltinRegistry,
    name: &str,
    arguments: Vec<Value>,
) -> Result<Vec<Value>, BuiltinInvocationError> {
    let handle = registry
        .handle_by_name(name)
        .expect("registered OEX function");
    let cancellation = CancellationToken::new();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(1, &cancellation, &mut output);
    registry.invoke_owned(handle, arguments, &mut context)
}

fn invoke_with_output_count(
    registry: &BuiltinRegistry,
    name: &str,
    arguments: &[Value],
    output_count: usize,
) -> Result<Vec<Value>, BuiltinInvocationError> {
    let handle = registry
        .handle_by_name(name)
        .expect("registered OEX function");
    let cancellation = CancellationToken::new();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(output_count, &cancellation, &mut output);
    registry.invoke(handle, arguments, &mut context)
}

fn f64_matrix(values: &[f64]) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new([2, 2]).unwrap(), values.to_vec()).unwrap(),
    ))
}

fn dense_f64(value: &Value) -> &DenseArray<f64> {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected dense f64 array");
    };
    array
}

fn sparse_f64(value: &Value) -> &openmat_value::CscMatrix<f64> {
    let Value::Sparse(SparseArrayData::F64(matrix)) = value else {
        panic!("expected sparse f64 array");
    };
    matrix
}

#[test]
fn direct_c_api_supports_scalar_builder_and_taken_mutation() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load direct-API plugin");
    assert_eq!(plugin.name(), "OpenMat OEX test plugin");
    assert_eq!(plugin.version(), "1.0.0");
    assert_eq!(
        plugin.function_names().collect::<Vec<_>>(),
        [
            "add2",
            "scale2_builder",
            "scale2_taken",
            "copy_dense",
            "fail_with_error",
            "missing_output",
            "invoke_callback",
            "destroyed_count",
            "sparse_triplet",
            "destroyed_receiver_count"
        ]
    );
    assert_eq!(
        plugin.class_names().collect::<Vec<_>>(),
        ["NativeCounter", "NativeReceiver"]
    );
    let mut registry = BuiltinRegistry::new();
    plugin
        .register_into(&mut registry)
        .expect("register all static descriptors");

    let scalar = invoke(&registry, "add2", &[Value::Double(2.0), Value::Double(3.5)]).unwrap();
    assert_eq!(scalar, [Value::Double(5.5)]);

    let original = f64_matrix(&[1.0, 2.0, 3.0, 4.0]);
    let built = invoke(&registry, "scale2_builder", std::slice::from_ref(&original)).unwrap();
    assert_eq!(dense_f64(&built[0]).as_slice(), &[2.0, 4.0, 6.0, 8.0]);
    assert_eq!(dense_f64(&original).as_slice(), &[1.0, 2.0, 3.0, 4.0]);
    assert!(!dense_f64(&built[0]).shares_storage_with(dense_f64(&original)));

    let taken = invoke(&registry, "scale2_taken", std::slice::from_ref(&original)).unwrap();
    assert_eq!(dense_f64(&taken[0]).as_slice(), &[2.0, 4.0, 6.0, 8.0]);
    assert_eq!(dense_f64(&original).as_slice(), &[1.0, 2.0, 3.0, 4.0]);
    assert!(!dense_f64(&taken[0]).shares_storage_with(dense_f64(&original)));

    let donated = f64_matrix(&[1.0, 2.0, 3.0, 4.0]);
    let donated_pointer = dense_f64(&donated).as_slice().as_ptr();
    let donated = invoke_owned(&registry, "scale2_taken", vec![donated]).unwrap();
    assert_eq!(dense_f64(&donated[0]).as_slice(), &[2.0, 4.0, 6.0, 8.0]);
    assert_eq!(dense_f64(&donated[0]).as_slice().as_ptr(), donated_pointer);

    let shape = Shape::new([2, 1]).unwrap();
    let typed_values = [
        Value::Array(ArrayData::Logical(
            DenseArray::from_vec(shape.clone(), vec![Logical::FALSE, Logical::TRUE]).unwrap(),
        )),
        Value::Array(ArrayData::Char(
            DenseArray::from_vec(
                shape.clone(),
                vec![CharCodeUnit::new(0x0041), CharCodeUnit::new(0xd800)],
            )
            .unwrap(),
        )),
        Value::Array(ArrayData::ComplexF32(
            DenseArray::from_vec(
                shape.clone(),
                vec![Complex32::new(1.5, -2.0), Complex32::new(-3.0, 4.25)],
            )
            .unwrap(),
        )),
        Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI16(
            DenseArray::from_vec(
                shape.clone(),
                vec![ComplexInteger::new(-7, 8), ComplexInteger::new(9, -10)],
            )
            .unwrap(),
        ))),
        Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                shape,
                vec![Complex64::new(1.0, -0.5), Complex64::new(-2.0, 3.0)],
            )
            .unwrap(),
        )),
    ];
    for value in typed_values {
        let copied = invoke(&registry, "copy_dense", std::slice::from_ref(&value)).unwrap();
        assert_eq!(copied, [value]);
    }

    let sparse = invoke(&registry, "sparse_triplet", &[]).unwrap();
    let sparse = sparse_f64(&sparse[0]);
    assert_eq!(sparse.shape().dimensions(), &[3, 3]);
    assert_eq!(sparse.col_offsets(), &[0, 2, 3, 3]);
    assert_eq!(sparse.row_indices(), &[0, 2, 0]);
    assert_eq!(sparse.values(), &[5.0, 3.0, 2.0]);
    assert_eq!(sparse.nzmax(), 5);

    let failure = invoke(&registry, "fail_with_error", &[]).unwrap_err();
    let BuiltinInvocationError::Failed { error, .. } = failure else {
        panic!("expected plugin error");
    };
    assert_eq!(error.category, BuiltinErrorCategory::Other);
    assert_eq!(error.identifier.as_deref(), Some("oex:testFailure"));
    assert_eq!(error.message, "intentional plugin failure");
}

#[test]
fn callback_output_contract_is_enforced() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load callback-contract plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();

    let failure = invoke(&registry, "missing_output", &[]).unwrap_err();
    let BuiltinInvocationError::Failed { error, .. } = failure else {
        panic!("expected missing-output failure");
    };
    assert_eq!(error.category, BuiltinErrorCategory::Output);

    let failure = invoke_with_output_count(
        &registry,
        "add2",
        &[Value::Double(2.0), Value::Double(3.0)],
        0,
    )
    .unwrap_err();
    let BuiltinInvocationError::Failed { error, .. } = failure else {
        panic!("expected output-count failure");
    };
    assert_eq!(error.category, BuiltinErrorCategory::ArgumentCount);
}

fn callback_module(callback_instructions: Vec<Instruction>) -> BytecodeModule {
    let entry = Function {
        name: "plugin_callback_entry".to_owned(),
        register_count: 5,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("invoke_callback".to_owned()),
            Constant::String("callback_target".to_owned()),
            Constant::Double(8.0),
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
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(3), Register::new(4)],
                target: Register::new(0),
                arguments: vec![
                    ApplyArgument::Value(Register::new(1)),
                    ApplyArgument::Value(Register::new(2)),
                ],
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(3), Register::new(4)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let callback = Function {
        name: "callback_target".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![
            Constant::Double(2.0),
            Constant::String("cancel_callback".to_owned()),
        ],
        instructions: callback_instructions,
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![entry, callback], FunctionId::new(0))
}

#[test]
fn c_plugin_can_invoke_language_function_handles_with_multiple_outputs() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load callback plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let module = callback_module(vec![
        Instruction::new(InstructionKind::LoadLocal {
            dst: Register::new(0),
            local: openmat_bytecode::LocalSlot::new(0),
        }),
        Instruction::new(InstructionKind::LoadConstant {
            dst: Register::new(1),
            constant: ConstantId::new(0),
        }),
        Instruction::new(InstructionKind::Binary {
            operator: openmat_bytecode::BinaryOperator::Multiply,
            dst: Register::new(2),
            lhs: Register::new(0),
            rhs: Register::new(1),
        }),
        Instruction::new(InstructionKind::Return {
            values: vec![Register::new(0), Register::new(2)],
        }),
    ]);
    let result = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect("C callback should reenter the language runtime");
    assert_eq!(result, [Value::Double(8.0), Value::Double(16.0)]);
}

#[test]
fn c_plugin_callback_preserves_the_original_language_failure() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load callback plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let module = callback_module(vec![
        Instruction::new(InstructionKind::LoadConstant {
            dst: Register::new(0),
            constant: ConstantId::new(0),
        }),
        Instruction::new(InstructionKind::LoadLocal {
            dst: Register::new(1),
            local: openmat_bytecode::LocalSlot::new(0),
        }),
        Instruction::new(InstructionKind::Apply {
            outputs: vec![Register::new(2)],
            target: Register::new(0),
            arguments: vec![ApplyArgument::Value(Register::new(1))],
        }),
        Instruction::new(InstructionKind::Return {
            values: vec![Register::new(2)],
        }),
    ]);
    let error = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect_err("the language callback must fail");
    assert!(error.index_out_of_bounds().is_some(), "{error:?}");
    assert!(
        error
            .stack
            .iter()
            .any(|frame| frame.name == "callback_target")
    );
}

#[test]
fn c_plugin_callback_propagates_cooperative_cancellation() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load callback plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    registry
        .register(
            "cancel_callback",
            |_arguments: &[Value], context: &mut BuiltinContext<'_>| {
                context.cancellation().cancel();
                Ok(Vec::new())
            },
        )
        .unwrap();
    let module = callback_module(vec![
        Instruction::new(InstructionKind::LoadFunctionHandle {
            dst: Register::new(0),
            name: ConstantId::new(1),
        }),
        Instruction::new(InstructionKind::Apply {
            outputs: Vec::new(),
            target: Register::new(0),
            arguments: Vec::new(),
        }),
        Instruction::new(InstructionKind::Return { values: Vec::new() }),
    ]);
    let error = Interpreter::with_registry(module, registry)
        .unwrap()
        .execute_entry(&[])
        .expect_err("callback cancellation must abort the plugin invocation");
    assert_eq!(error.kind, RuntimeErrorKind::Cancelled);
    assert!(
        error
            .stack
            .iter()
            .any(|frame| frame.name == "callback_target")
    );
}

fn native_counter_module() -> BytecodeModule {
    let function = Function {
        name: "native_counter".to_owned(),
        register_count: 11,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("NativeCounter".to_owned()),
            Constant::Double(10.0),
            Constant::Double(2.5),
            Constant::String("add".to_owned()),
            Constant::String("value".to_owned()),
            Constant::String("assemble".to_owned()),
            Constant::Double(2.0),
            Constant::Double(-3.0),
        ],
        instructions: vec![
            Instruction::new(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(1),
                constant: ConstantId::new(1),
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(2),
            }),
            Instruction::new(InstructionKind::ApplyField {
                outputs: vec![Register::new(4)],
                object: Register::new(2),
                name: ConstantId::new(3),
                arguments: vec![ApplyArgument::Value(Register::new(3))],
            }),
            Instruction::new(InstructionKind::ApplyField {
                outputs: vec![Register::new(5)],
                object: Register::new(2),
                name: ConstantId::new(4),
                arguments: Vec::new(),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(6),
                constant: ConstantId::new(6),
            }),
            Instruction::new(InstructionKind::ApplyField {
                outputs: vec![Register::new(7)],
                object: Register::new(2),
                name: ConstantId::new(5),
                arguments: vec![ApplyArgument::Value(Register::new(6))],
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(8),
                constant: ConstantId::new(7),
            }),
            Instruction::new(InstructionKind::ApplyField {
                outputs: vec![Register::new(9)],
                object: Register::new(2),
                name: ConstantId::new(5),
                arguments: vec![ApplyArgument::Value(Register::new(8))],
            }),
            Instruction::new(InstructionKind::Move {
                dst: Register::new(10),
                src: Register::new(2),
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![
                    Register::new(10),
                    Register::new(4),
                    Register::new(5),
                    Register::new(7),
                    Register::new(9),
                ],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![function], FunctionId::new(0))
}

fn destroyed_count_module() -> BytecodeModule {
    let function = Function {
        name: "check_destroyed".to_owned(),
        register_count: 4,
        pack_register_count: 0,
        local_count: 0,
        persistent_slot_count: 0,
        parameter_count: 0,
        argument_layout: None,
        constants: vec![
            Constant::String("destroyed_count".to_owned()),
            Constant::String("destroyed_receiver_count".to_owned()),
        ],
        instructions: vec![
            Instruction::new(InstructionKind::LoadGlobal {
                dst: Register::new(0),
                name: ConstantId::new(0),
                construct_if_class: false,
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(1)],
                target: Register::new(0),
                arguments: Vec::new(),
            }),
            Instruction::new(InstructionKind::LoadGlobal {
                dst: Register::new(2),
                name: ConstantId::new(1),
                construct_if_class: false,
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(3)],
                target: Register::new(2),
                arguments: Vec::new(),
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(1), Register::new(3)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    BytecodeModule::new(vec![function], FunctionId::new(0))
}

fn native_property_module() -> BytecodeModule {
    BytecodeModule::new(
        vec![Function {
            name: "native_property".to_owned(),
            register_count: 7,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("NativeCounter".to_owned()),
                Constant::Double(7.0),
                Constant::String("Value".to_owned()),
                Constant::Double(11.5),
            ],
            instructions: vec![
                Instruction::new(InstructionKind::LoadGlobal {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                    construct_if_class: false,
                }),
                Instruction::new(InstructionKind::LoadConstant {
                    dst: Register::new(1),
                    constant: ConstantId::new(1),
                }),
                Instruction::new(InstructionKind::Apply {
                    outputs: vec![Register::new(2)],
                    target: Register::new(0),
                    arguments: vec![ApplyArgument::Value(Register::new(1))],
                }),
                Instruction::new(InstructionKind::GetField {
                    dst: Register::new(3),
                    object: Register::new(2),
                    name: ConstantId::new(2),
                }),
                Instruction::new(InstructionKind::LoadConstant {
                    dst: Register::new(4),
                    constant: ConstantId::new(3),
                }),
                Instruction::new(InstructionKind::SetField {
                    dst: Register::new(5),
                    object: Register::new(2),
                    name: ConstantId::new(2),
                    value: Register::new(4),
                }),
                Instruction::new(InstructionKind::GetField {
                    dst: Register::new(6),
                    object: Register::new(5),
                    name: ConstantId::new(2),
                }),
                Instruction::new(InstructionKind::Return {
                    values: vec![
                        Register::new(2),
                        Register::new(5),
                        Register::new(3),
                        Register::new(6),
                    ],
                }),
            ],
            exception_handlers: Vec::new(),
        }],
        FunctionId::new(0),
    )
}

fn native_property_access_error_module(read: bool) -> BytecodeModule {
    let mut instructions = vec![
        Instruction::new(InstructionKind::LoadGlobal {
            dst: Register::new(0),
            name: ConstantId::new(0),
            construct_if_class: false,
        }),
        Instruction::new(InstructionKind::LoadConstant {
            dst: Register::new(1),
            constant: ConstantId::new(1),
        }),
        Instruction::new(InstructionKind::Apply {
            outputs: vec![Register::new(2)],
            target: Register::new(0),
            arguments: vec![ApplyArgument::Value(Register::new(1))],
        }),
    ];
    if read {
        instructions.push(Instruction::new(InstructionKind::GetField {
            dst: Register::new(3),
            object: Register::new(2),
            name: ConstantId::new(2),
        }));
    } else {
        instructions.extend([
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(3),
                constant: ConstantId::new(3),
            }),
            Instruction::new(InstructionKind::SetField {
                dst: Register::new(4),
                object: Register::new(2),
                name: ConstantId::new(2),
                value: Register::new(3),
            }),
        ]);
    }
    instructions.push(Instruction::new(InstructionKind::Return {
        values: Vec::new(),
    }));
    BytecodeModule::new(
        vec![Function {
            name: "native_property_access_error".to_owned(),
            register_count: 5,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("NativeCounter".to_owned()),
                Constant::Double(7.0),
                Constant::String(if read {
                    "WriteOnly".to_owned()
                } else {
                    "ReadOnly".to_owned()
                }),
                Constant::Double(11.5),
            ],
            instructions,
            exception_handlers: Vec::new(),
        }],
        FunctionId::new(0),
    )
}

#[test]
fn native_class_uses_openmat_object_identity_methods_and_destruction() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load native-class plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let mut interpreter = Interpreter::with_registry(native_counter_module(), registry).unwrap();
    let values = interpreter.execute_entry(&[]).unwrap();
    assert_eq!(interpreter.value_class_name(&values[0]), "NativeCounter");
    assert_eq!(values[1], Value::Double(12.5));
    assert_eq!(values[2], Value::Double(12.5));
    let first = sparse_f64(&values[3]);
    assert_eq!(first.col_offsets(), &[0, 1, 2, 3]);
    assert_eq!(first.row_indices(), &[0, 1, 2]);
    assert_eq!(first.values(), &[2.0, -2.0, 4.0]);
    assert_eq!(first.nzmax(), 4);
    let second = sparse_f64(&values[4]);
    assert_eq!(second.col_offsets(), &[0, 1, 2, 3]);
    assert_eq!(second.row_indices(), &[0, 1, 2]);
    assert_eq!(second.values(), &[-3.0, 3.0, -6.0]);
    assert_eq!(second.nzmax(), 4);
    assert_eq!(interpreter.object_count(), 1);

    interpreter.clear_session();
    interpreter
        .replace_module(destroyed_count_module())
        .unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        [Value::Double(1.0), Value::Double(0.0)]
    );
}

#[test]
fn native_properties_use_getters_setters_and_handle_identity() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load native-property plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let mut interpreter = Interpreter::with_registry(native_property_module(), registry).unwrap();
    let values = interpreter.execute_entry(&[]).unwrap();
    assert_eq!(interpreter.value_class_name(&values[0]), "NativeCounter");
    assert_eq!(values[0], values[1]);
    assert_eq!(values[2], Value::Double(7.0));
    assert_eq!(values[3], Value::Double(11.5));
    assert_eq!(interpreter.object_count(), 1);
}

#[test]
fn native_property_access_modes_are_enforced_before_callbacks() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load native-property plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();

    let mut interpreter =
        Interpreter::with_registry(native_property_access_error_module(true), registry).unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Object {
            operation: "property read",
            ..
        }
    ));

    interpreter
        .replace_module(native_property_access_error_module(false))
        .unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Object {
            operation: "property write",
            ..
        }
    ));
}

fn native_object_bridge_module(wrong_class: bool) -> BytecodeModule {
    let mut instructions = vec![
        Instruction::new(InstructionKind::LoadGlobal {
            dst: Register::new(0),
            name: ConstantId::new(0),
            construct_if_class: false,
        }),
        Instruction::new(InstructionKind::LoadConstant {
            dst: Register::new(1),
            constant: ConstantId::new(1),
        }),
        Instruction::new(InstructionKind::Apply {
            outputs: vec![Register::new(2)],
            target: Register::new(0),
            arguments: vec![ApplyArgument::Value(Register::new(1))],
        }),
        Instruction::new(InstructionKind::LoadGlobal {
            dst: Register::new(3),
            name: ConstantId::new(2),
            construct_if_class: false,
        }),
        Instruction::new(InstructionKind::Apply {
            outputs: vec![Register::new(4)],
            target: Register::new(3),
            arguments: vec![ApplyArgument::Value(Register::new(2))],
        }),
    ];
    if wrong_class {
        instructions.extend([
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(5)],
                target: Register::new(3),
                arguments: vec![ApplyArgument::Value(Register::new(4))],
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(5)],
            }),
        ]);
    } else {
        instructions.extend([
            Instruction::new(InstructionKind::ApplyField {
                outputs: vec![Register::new(5)],
                object: Register::new(4),
                name: ConstantId::new(3),
                arguments: Vec::new(),
            }),
            Instruction::new(InstructionKind::LoadConstant {
                dst: Register::new(6),
                constant: ConstantId::new(4),
            }),
            Instruction::new(InstructionKind::ApplyField {
                outputs: vec![Register::new(7)],
                object: Register::new(2),
                name: ConstantId::new(5),
                arguments: vec![ApplyArgument::Value(Register::new(6))],
            }),
            Instruction::new(InstructionKind::ApplyField {
                outputs: vec![Register::new(8)],
                object: Register::new(7),
                name: ConstantId::new(3),
                arguments: Vec::new(),
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![
                    Register::new(2),
                    Register::new(4),
                    Register::new(7),
                    Register::new(5),
                    Register::new(8),
                ],
            }),
        ]);
    }
    BytecodeModule::new(
        vec![Function {
            name: "native_object_bridge".to_owned(),
            register_count: 9,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("NativeCounter".to_owned()),
                Constant::Double(7.0),
                Constant::String("NativeReceiver".to_owned()),
                Constant::String("value".to_owned()),
                Constant::Double(3.0),
                Constant::String("cloneScaled".to_owned()),
            ],
            instructions,
            exception_handlers: Vec::new(),
        }],
        FunctionId::new(0),
    )
}

fn native_object_creation_failure_module() -> BytecodeModule {
    BytecodeModule::new(
        vec![Function {
            name: "native_object_creation_failure".to_owned(),
            register_count: 3,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("NativeCounter".to_owned()),
                Constant::Double(5.0),
                Constant::String("cloneFail".to_owned()),
            ],
            instructions: vec![
                Instruction::new(InstructionKind::LoadGlobal {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                    construct_if_class: false,
                }),
                Instruction::new(InstructionKind::LoadConstant {
                    dst: Register::new(1),
                    constant: ConstantId::new(1),
                }),
                Instruction::new(InstructionKind::Apply {
                    outputs: vec![Register::new(2)],
                    target: Register::new(0),
                    arguments: vec![ApplyArgument::Value(Register::new(1))],
                }),
                Instruction::new(InstructionKind::ApplyField {
                    outputs: Vec::new(),
                    object: Register::new(2),
                    name: ConstantId::new(2),
                    arguments: Vec::new(),
                }),
                Instruction::new(InstructionKind::Return { values: Vec::new() }),
            ],
            exception_handlers: Vec::new(),
        }],
        FunctionId::new(0),
    )
}

#[test]
fn native_objects_cross_the_value_bridge_in_both_directions() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load native-object bridge plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let mut interpreter =
        Interpreter::with_registry(native_object_bridge_module(false), registry).unwrap();
    let values = interpreter.execute_entry(&[]).unwrap();
    assert_eq!(interpreter.value_class_name(&values[0]), "NativeCounter");
    assert_eq!(interpreter.value_class_name(&values[1]), "NativeReceiver");
    assert_eq!(interpreter.value_class_name(&values[2]), "NativeCounter");
    assert_eq!(values[3], Value::Double(7.0));
    assert_eq!(values[4], Value::Double(21.0));
    assert_eq!(interpreter.object_count(), 3);

    interpreter.clear_session();
    interpreter
        .replace_module(destroyed_count_module())
        .unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        [Value::Double(2.0), Value::Double(1.0)]
    );
}

#[test]
fn native_object_borrow_rejects_the_wrong_registered_class() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load native-object bridge plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let mut interpreter =
        Interpreter::with_registry(native_object_bridge_module(true), registry).unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Builtin {
            category: BuiltinErrorCategory::Type,
            ..
        }
    ));
}

#[test]
fn failed_callback_destroys_an_uncommitted_native_object() {
    let Some((arithmetic, _)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: The fixture is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(arithmetic) }.expect("load native-object bridge plugin");
    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let mut interpreter =
        Interpreter::with_registry(native_object_creation_failure_module(), registry).unwrap();
    let error = interpreter.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::Builtin {
            category: BuiltinErrorCategory::Other,
            ..
        }
    ));

    interpreter.clear_session();
    interpreter
        .replace_module(destroyed_count_module())
        .unwrap();
    assert_eq!(
        interpreter.execute_entry(&[]).unwrap(),
        [Value::Double(2.0), Value::Double(0.0)]
    );
}

#[test]
fn static_descriptor_is_rejected_atomically_before_registration() {
    let Some((_, invalid)) = compiled_fixtures() else {
        return;
    };
    // SAFETY: This checked-in fixture deliberately returns a readable but incompatible descriptor.
    let error = unsafe { OexPlugin::load(invalid) }.unwrap_err();
    let OexLoadError::InvalidRegistration { message, .. } = error else {
        panic!("expected descriptor validation error");
    };
    assert!(message.contains("requires OEX ABI 2.0"));
}

#[test]
fn complete_checked_in_example_compiles_loads_and_executes() {
    if Command::new("gcc").arg("--version").output().is_err() {
        eprintln!("skipping complete OEX example test because gcc is unavailable");
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest
        .parent()
        .and_then(Path::parent)
        .expect("OEX crate belongs to the repository");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "openmat-oex-example-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let bridge = bridge_dll();
    std::fs::copy(&bridge, directory.join("openmat_oex.dll")).unwrap();
    let output = directory.join("oex_v1_plugin.oex.dll");
    compile_fixture(
        &repository.join("examples/oex-v1-plugin/oex_v1_plugin.c"),
        &output,
        repository,
        &bridge,
    );

    // SAFETY: The example is compiled from checked-in C against this checkout's header.
    let plugin = unsafe { OexPlugin::load(output) }.expect("load complete OEX example");
    assert_eq!(
        plugin.function_names().collect::<Vec<_>>(),
        [
            "oex_scale_in_place",
            "oex_apply1",
            "oex_triplet_demo",
            "oex_assembler_scale"
        ]
    );
    assert_eq!(
        plugin.class_names().collect::<Vec<_>>(),
        ["OexPatternAssembler"]
    );

    let mut registry = BuiltinRegistry::new();
    plugin.register_into(&mut registry).unwrap();
    let original = f64_matrix(&[1.0, 2.0, 3.0, 4.0]);
    let scaled = invoke(
        &registry,
        "oex_scale_in_place",
        &[original.clone(), Value::Double(2.5)],
    )
    .unwrap();
    assert_eq!(dense_f64(&scaled[0]).as_slice(), &[2.5, 5.0, 7.5, 10.0]);
    assert_eq!(dense_f64(&original).as_slice(), &[1.0, 2.0, 3.0, 4.0]);

    let sparse = invoke(&registry, "oex_triplet_demo", &[]).unwrap();
    let sparse = sparse_f64(&sparse[0]);
    assert_eq!(sparse.col_offsets(), &[0, 1, 2, 3]);
    assert_eq!(sparse.row_indices(), &[0, 1, 2]);
    assert_eq!(sparse.values(), &[5.0, 4.0, 5.0]);
}

#[test]
fn malformed_v1_descriptors_are_rejected_during_atomic_preflight() {
    if Command::new("gcc").arg("--version").output().is_err() {
        eprintln!("skipping invalid OEX descriptor tests because gcc is unavailable");
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest
        .parent()
        .and_then(Path::parent)
        .expect("OEX crate belongs to the repository");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "openmat-oex-invalid-contract-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let bridge = bridge_dll();
    std::fs::copy(&bridge, directory.join("openmat_oex.dll")).unwrap();
    let source = manifest.join("tests/fixtures/invalid_contract_plugin.c");

    for (case, expected) in [
        (1, "function descriptor flags must be zero"),
        (2, "has no getter or setter"),
        (3, "conflicts with a method"),
        (4, "null callback"),
        (5, "function descriptor is"),
    ] {
        let output = directory.join(format!("invalid-contract-{case}.oex.dll"));
        let define = format!("-DOEX_INVALID_CASE={case}");
        compile_fixture_with_arguments(&source, &output, repository, &bridge, &[&define]);
        // SAFETY: Each checked-in fixture exposes readable static descriptors and no callback runs.
        let error = unsafe { OexPlugin::load(output) }.unwrap_err();
        let OexLoadError::InvalidRegistration { message, .. } = error else {
            panic!("expected invalid-registration error for case {case}");
        };
        assert!(
            message.contains(expected),
            "case {case} returned unexpected error: {message}"
        );
    }
}

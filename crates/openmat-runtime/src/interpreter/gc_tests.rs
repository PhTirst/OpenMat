use std::{collections::BTreeMap, sync::Arc};

use openmat_array::Shape;
use openmat_bytecode::{
    BytecodeModule, Constant, ConstantId, Function, FunctionId, Instruction, InstructionKind,
    LocalSlot, PersistentSlot, Register,
};
use openmat_object::{
    AccessContext, ClassDefinition, ClassId, EventDescriptor, HandleState, MethodDescriptor,
    PropertyAccess, PropertyDescriptor, PropertyResolution,
};
use openmat_value::{BytecodeFunctionHandle, CellArray, FunctionHandle, ObjectHandle, Value};

use super::{
    CapturedBinding, GC_ALLOCATION_THRESHOLD, Interpreter, PersistentBindingKey,
    REGISTERED_FUNCTION_HANDLE_BASE, RuntimeClassCode, RuntimeErrorKind, RuntimeFunctionCode,
    RuntimeMethodCode, SafepointReason, ScopeAddress, ScopeBindingChange, Workspace,
};

fn interpreter() -> Interpreter {
    let mut entry = Function::new("<gc-test>", 0, 0, 0);
    entry
        .instructions
        .push(Instruction::new(InstructionKind::Return {
            values: Vec::new(),
        }));
    Interpreter::new(BytecodeModule::new(vec![entry], FunctionId::new(0))).unwrap()
}

fn register_node(runtime: &mut Interpreter) -> (ClassId, openmat_object::PropertyKey) {
    let class = runtime
        .classes
        .register_class(
            ClassDefinition::handle("GcNode")
                .with_property(PropertyDescriptor::stored("next", Some(Value::Nothing))),
        )
        .unwrap();
    let PropertyResolution::Stored { key, .. } = runtime
        .classes
        .resolve_property(
            class,
            "next",
            PropertyAccess::Set,
            AccessContext::external(),
        )
        .unwrap()
    else {
        panic!("next must be stored");
    };
    (class, key)
}

fn allocate(runtime: &mut Interpreter, class: ClassId) -> ObjectHandle {
    let reference = runtime.objects.allocate(&runtime.classes, class).unwrap();
    runtime.objects.begin_construction(&reference).unwrap();
    runtime.objects.finish_construction(&reference).unwrap();
    let handle = ObjectHandle::new(reference.id().get());
    runtime.object_references.insert(handle, reference);
    handle
}

fn set_next(
    runtime: &mut Interpreter,
    object: ObjectHandle,
    key: &openmat_object::PropertyKey,
    value: Value,
) {
    let reference = runtime.object_references.get(&object).unwrap();
    *runtime.objects.slot_mut(reference, key).unwrap() = value;
}

#[test]
fn interpreter_roots_preserve_nested_cycles_and_reclaim_unreachable_objects() {
    let mut runtime = interpreter();
    let (class, key) = register_node(&mut runtime);
    let first = allocate(&mut runtime, class);
    let second = allocate(&mut runtime, class);
    let isolated = allocate(&mut runtime, class);
    set_next(&mut runtime, first, &key, Value::Object(second));
    set_next(&mut runtime, second, &key, Value::Object(first));
    set_next(&mut runtime, isolated, &key, Value::Object(isolated));

    let nested_root = Value::Cell(
        CellArray::from_values(
            Shape::new([1, 1]).unwrap(),
            vec![Value::Cell(
                CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![Value::Object(first)])
                    .unwrap(),
            )],
        )
        .unwrap(),
    );
    runtime.workspace.insert("root", nested_root);

    let first_collection = runtime.collect_garbage_for_tests();
    assert_eq!(first_collection.reclaimed_objects, 1);
    assert_eq!(runtime.object_count(), 2);
    let isolated_reference = runtime.object_references.get(&isolated).unwrap();
    assert_eq!(
        runtime.objects.handle_state(isolated_reference).unwrap(),
        HandleState::Invalid
    );

    runtime.workspace.clear();
    let second_collection = runtime.collect_garbage_for_tests();
    assert_eq!(second_collection.reclaimed_objects, 2);
    assert_eq!(runtime.object_count(), 0);
}

#[test]
fn persistent_closure_class_and_transient_runtime_roots_are_traced() {
    let mut runtime = interpreter();
    let (class, _) = register_node(&mut runtime);

    let persistent = allocate(&mut runtime, class);
    runtime.persistent_bindings.insert(
        PersistentBindingKey {
            function_name: "persistent_owner".to_owned(),
            caller_class: None,
            slot: PersistentSlot::new(0),
        },
        Value::Object(persistent),
    );

    let captured = allocate(&mut runtime, class);
    let closure = BytecodeFunctionHandle::new(REGISTERED_FUNCTION_HANDLE_BASE + 7);
    runtime.function_handles.insert(
        closure,
        RuntimeFunctionCode {
            access_context: AccessContext::external(),
            module: Arc::clone(&runtime.module),
            function: FunctionId::new(0),
            captures: Arc::new(BTreeMap::from([(
                "captured".to_owned(),
                CapturedBinding::Value(Value::Object(captured)),
            )])),
        },
    );
    runtime.workspace.insert(
        "closure",
        Value::Function(FunctionHandle::Bytecode(closure)),
    );

    let class_default = allocate(&mut runtime, class);
    runtime
        .classes
        .register_class(ClassDefinition::handle("DefaultRoot").with_property(
            PropertyDescriptor::stored("value", Some(Value::Object(class_default))),
        ))
        .unwrap();

    let frame_root = allocate(&mut runtime, class);
    runtime
        .active_frame_roots
        .push(vec![Value::Object(frame_root)]);
    let pending = allocate(&mut runtime, class);
    runtime.pending_scope_bindings.push(ScopeBindingChange {
        target: ScopeAddress::Base,
        name: "pending".to_owned(),
        value: Value::Object(pending),
    });
    let builtin = allocate(&mut runtime, class);
    let mut builtin_workspace = Workspace::new();
    builtin_workspace.insert("builtin", Value::Object(builtin));
    runtime.builtin_workspace = Some(builtin_workspace);

    let collection = runtime.collect_garbage_for_tests();
    assert_eq!(collection.reclaimed_objects, 0);
    assert_eq!(runtime.object_count(), 6);

    runtime.persistent_bindings.clear();
    runtime.workspace.clear();
    runtime.active_frame_roots.clear();
    runtime.pending_scope_bindings.clear();
    runtime.builtin_workspace = None;
    let collection = runtime.collect_garbage_for_tests();
    assert_eq!(collection.reclaimed_objects, 5);
    assert_eq!(
        runtime.object_count(),
        1,
        "the class default remains a root"
    );
}

#[test]
fn kernels_collect_independently_even_when_local_ids_match() {
    let mut first_kernel = interpreter();
    let mut second_kernel = interpreter();
    let (first_class, _) = register_node(&mut first_kernel);
    let (second_class, _) = register_node(&mut second_kernel);
    let first = allocate(&mut first_kernel, first_class);
    let second = allocate(&mut second_kernel, second_class);
    assert_eq!(first.identifier(), second.identifier());

    first_kernel.workspace.insert("root", Value::Object(first));
    first_kernel.collect_garbage_for_tests();
    second_kernel.collect_garbage_for_tests();

    assert_eq!(first_kernel.object_count(), 1);
    assert_eq!(second_kernel.object_count(), 0);
}

#[test]
fn exception_metadata_does_not_root_an_unreachable_exception_object() {
    let mut runtime = interpreter();
    let (class, _) = register_node(&mut runtime);
    let exception = allocate(&mut runtime, class);
    let error = runtime.error(RuntimeErrorKind::Cancelled, None);
    runtime.exception_errors.insert(exception, error);

    let collection = runtime.collect_garbage_for_tests();
    assert_eq!(collection.reclaimed_objects, 1);
    assert!(!runtime.exception_errors.contains_key(&exception));
    let exception_reference = runtime.object_references.get(&exception).unwrap();
    assert_eq!(
        runtime.objects.handle_state(exception_reference).unwrap(),
        HandleState::Invalid
    );
}

#[test]
fn rollback_removal_updates_live_metrics_without_reusing_identity() {
    let mut runtime = interpreter();
    let (class, _) = register_node(&mut runtime);
    let reference = runtime.objects.allocate(&runtime.classes, class).unwrap();
    runtime.objects.begin_construction(&reference).unwrap();
    let failed = ObjectHandle::new(reference.id().get());
    runtime.object_references.insert(failed, reference);
    runtime.rollback_object(failed);
    assert_eq!(runtime.gc_metrics().live_objects, 0);

    let next = allocate(&mut runtime, class);
    assert!(next.identifier() > failed.identifier());
    runtime.collect_garbage_for_tests();
    assert_eq!(runtime.object_count(), 0);
}

#[test]
fn allocation_debt_is_deferred_until_an_explicit_interpreter_safepoint() {
    let mut runtime = interpreter();
    let (class, _) = register_node(&mut runtime);
    let allocation_count = (GC_ALLOCATION_THRESHOLD / 2).saturating_add(1);
    let mut first = None;
    for _ in 0..allocation_count {
        let handle = allocate(&mut runtime, class);
        first.get_or_insert(handle);
    }
    assert!(runtime.objects.collection_due(GC_ALLOCATION_THRESHOLD));
    assert_eq!(runtime.object_count(), allocation_count as usize);

    runtime
        .active_frame_roots
        .push(vec![Value::Object(first.unwrap())]);
    let collection = runtime
        .collect_garbage_at_safepoint(
            std::iter::empty::<&Value>(),
            SafepointReason::AllocationDebt,
            None,
        )
        .expect("allocation debt must request collection at the safe point");
    assert_eq!(collection.remaining_objects, 1);
    assert_eq!(runtime.object_count(), 1);
    assert_eq!(runtime.gc_metrics().allocation_debt, 0);
}

#[test]
fn command_boundary_collects_unreachable_objects_without_a_timer() {
    let mut runtime = interpreter();
    let (class, _) = register_node(&mut runtime);
    allocate(&mut runtime, class);
    assert_eq!(runtime.gc_metrics().collections, 0);

    runtime.execute_entry(&[]).unwrap();

    assert_eq!(runtime.object_count(), 0);
    assert_eq!(runtime.gc_metrics().collections, 1);
}

#[test]
fn replacing_the_last_workspace_root_requests_a_language_safepoint() {
    let mut runtime = interpreter();
    let (class, _) = register_node(&mut runtime);
    let object = allocate(&mut runtime, class);
    runtime.replace_workspace_binding("root".to_owned(), Value::Object(object));
    assert!(!runtime.take_root_removal_safepoint_for_tests());

    runtime.replace_workspace_binding("root".to_owned(), Value::Double(1.0));
    assert!(runtime.take_root_removal_safepoint_for_tests());
    assert!(!runtime.take_root_removal_safepoint_for_tests());

    runtime.replace_workspace_binding("root".to_owned(), Value::Double(2.0));
    assert!(!runtime.take_root_removal_safepoint_for_tests());
}

#[test]
fn scalar_and_colon_assignment_can_grow_an_initially_empty_row_set() {
    let runtime = interpreter();
    let shape = Shape::new([0, 2]).unwrap();
    let arguments = [
        super::IndexInput::Value(Value::Double(1.0)),
        super::IndexInput::Colon,
    ];
    let (selection, grown) = runtime
        .selection_with_scalar_growth(&shape, &arguments, "test growth", None)
        .unwrap();

    assert_eq!(grown.unwrap().dimensions(), &[1, 2]);
    assert_eq!(selection.offsets, vec![0, 1]);
    assert_eq!(selection.shape.dimensions(), &[1, 2]);
}

#[test]
fn object_free_instruction_stream_does_not_add_allocation_debt_collections() {
    let mut entry = Function::new("<object-free-hot-loop>", 2, 0, 0);
    entry.constants.push(Constant::Double(1.0));
    entry
        .instructions
        .push(Instruction::new(InstructionKind::LoadConstant {
            dst: Register::new(0),
            constant: ConstantId::new(0),
        }));
    for index in 0..1_024 {
        let (dst, operand) = if index % 2 == 0 {
            (Register::new(1), Register::new(0))
        } else {
            (Register::new(0), Register::new(1))
        };
        entry
            .instructions
            .push(Instruction::new(InstructionKind::Transpose {
                dst,
                operand,
                conjugate: false,
            }));
    }
    entry
        .instructions
        .push(Instruction::new(InstructionKind::Return {
            values: Vec::new(),
        }));
    let mut runtime =
        Interpreter::new(BytecodeModule::new(vec![entry], FunctionId::new(0))).unwrap();

    runtime.execute_entry(&[]).unwrap();

    assert_eq!(runtime.object_count(), 0);
    assert_eq!(runtime.gc_metrics().collections, 1);
}

#[test]
fn finalizer_reads_properties_only_through_the_controlled_receiver() {
    let mut runtime = interpreter();
    let class = runtime
        .classes
        .register_class(
            ClassDefinition::handle("GcFinalizer")
                .with_property(PropertyDescriptor::stored(
                    "marker",
                    Some(Value::Double(0.0)),
                ))
                .with_method(MethodDescriptor::instance("delete")),
        )
        .unwrap();
    let PropertyResolution::Stored { key, .. } = runtime
        .classes
        .resolve_property(
            class,
            "marker",
            PropertyAccess::Set,
            AccessContext::external(),
        )
        .unwrap()
    else {
        panic!("marker must be stored")
    };

    let mut delete = Function::new("GcFinalizer.delete", 2, 1, 1);
    delete.constants.extend([
        Constant::String("marker".to_owned()),
        Constant::String("observed".to_owned()),
    ]);
    delete.instructions.extend([
        Instruction::new(InstructionKind::LoadLocal {
            dst: Register::new(0),
            local: LocalSlot::new(0),
        }),
        Instruction::new(InstructionKind::GetField {
            dst: Register::new(1),
            object: Register::new(0),
            name: ConstantId::new(0),
        }),
        Instruction::new(InstructionKind::StoreGlobal {
            name: ConstantId::new(1),
            src: Register::new(1),
        }),
        Instruction::new(InstructionKind::Return { values: Vec::new() }),
    ]);
    let delete_module = Arc::new(BytecodeModule::new(vec![delete], FunctionId::new(0)));
    runtime.class_code.insert(
        class,
        RuntimeClassCode {
            methods: BTreeMap::from([(
                "delete".to_owned(),
                RuntimeMethodCode {
                    module: delete_module,
                    function: FunctionId::new(0),
                    constructor_output: None,
                },
            )]),
            ..RuntimeClassCode::default()
        },
    );
    let object = allocate(&mut runtime, class);
    let reference = runtime.object_references.get(&object).unwrap();
    *runtime.objects.slot_mut(reference, &key).unwrap() = Value::Double(42.0);

    let collection = runtime.collect_garbage_for_tests();

    assert_eq!(collection.reclaimed_objects, 1);
    assert_eq!(
        runtime.workspace.get("observed"),
        Some(&Value::Double(42.0))
    );
    let reference = runtime.object_references.get(&object).unwrap();
    assert_eq!(
        runtime.objects.handle_state(reference).unwrap(),
        HandleState::Invalid
    );
}

#[test]
fn listener_identity_and_bidirectional_roots_follow_handle_lifecycle() {
    let mut runtime = interpreter();
    let empty_listeners = CellArray::from_values(Shape::new([0, 0]).unwrap(), Vec::new()).unwrap();
    let source_class = runtime
        .classes
        .register_class(
            ClassDefinition::handle("GcEventSource")
                .with_property(PropertyDescriptor::stored(
                    super::EVENT_SOURCE_LISTENERS_PROPERTY,
                    Some(Value::Cell(empty_listeners)),
                ))
                .with_event(EventDescriptor::new("Pulse")),
        )
        .unwrap();
    let source_key = runtime
        .classes
        .stored_properties(source_class)
        .unwrap()
        .into_iter()
        .find_map(|(key, property)| {
            (property.name() == super::EVENT_SOURCE_LISTENERS_PROPERTY).then_some(key)
        })
        .unwrap();
    runtime.event_source_slots.insert(source_class, source_key);
    let source = allocate(&mut runtime, source_class);
    let event = runtime.char_row_value("Pulse", "event name", None).unwrap();
    let callback = Value::Function(FunctionHandle::Bytecode(BytecodeFunctionHandle::new(0)));
    let module = Arc::clone(&runtime.module);
    let first = super::frames::run(async |stack| {
        runtime
            .add_event_listener(
                stack,
                &module,
                &[Value::Object(source), event.clone(), callback.clone()],
                1,
                AccessContext::external(),
                None,
            )
            .await
    })
    .unwrap();
    let second = super::frames::run(async |stack| {
        runtime
            .add_event_listener(
                stack,
                &module,
                &[Value::Object(source), event, callback],
                1,
                AccessContext::external(),
                None,
            )
            .await
    })
    .unwrap();
    let first_handle = super::object_handle(&first).unwrap();
    let second_handle = super::object_handle(&second).unwrap();
    assert_ne!(first_handle, second_handle);
    assert_eq!(runtime.value_class_name(&first), "event.listener");
    assert_eq!(runtime.object_count(), 3);

    super::frames::run(async |stack| {
        runtime
            .begin_explicit_handle_finalization(stack, second_handle, None)
            .await
    })
    .unwrap();
    super::frames::run(async |stack| {
        runtime
            .drain_requested_finalizers(stack, SafepointReason::ExplicitDelete)
            .await;
    });
    assert!(!runtime.event_listeners.contains_key(&second_handle));
    assert_eq!(
        runtime
            .objects
            .handle_state(runtime.object_references.get(&second_handle).unwrap())
            .unwrap(),
        HandleState::Invalid
    );

    runtime.workspace.insert("listener", first);
    let rooted = runtime.collect_garbage_for_tests();
    assert_eq!(rooted.reclaimed_objects, 0);
    assert_eq!(runtime.object_count(), 2);

    runtime.workspace.clear();
    let unreachable_cycle = runtime.collect_garbage_for_tests();
    assert_eq!(unreachable_cycle.reclaimed_objects, 2);
    assert_eq!(runtime.object_count(), 0);
    assert!(runtime.event_listeners.is_empty());
    assert!(runtime.event_listener_order.is_empty());
}

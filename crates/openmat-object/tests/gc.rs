use openmat_object::{
    AccessContext, ClassDefinition, ClassRegistry, HandleState, ObjectRef, ObjectStore,
    PropertyAccess, PropertyDescriptor, PropertyResolution,
};
use openmat_value::{ObjectHandle, Value};

fn finish(store: &mut ObjectStore<Value>, object: &ObjectRef) {
    store.begin_construction(object).unwrap();
    store.finish_construction(object).unwrap();
}

fn handle(object: &ObjectRef) -> ObjectHandle {
    ObjectHandle::new(object.id().get())
}

#[test]
fn mark_sweep_reclaims_cycles_and_preserves_reachable_edges() {
    let mut registry = ClassRegistry::new();
    let class = registry
        .register_class(
            ClassDefinition::handle("Node")
                .with_property(PropertyDescriptor::stored("next", Some(Value::Nothing))),
        )
        .unwrap();
    let PropertyResolution::Stored { key, .. } = registry
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

    let mut store = ObjectStore::new();
    let first = store.allocate(&registry, class).unwrap();
    let second = store.allocate(&registry, class).unwrap();
    let isolated = store.allocate(&registry, class).unwrap();
    finish(&mut store, &first);
    finish(&mut store, &second);
    finish(&mut store, &isolated);

    *store.slot_mut(&first, &key).unwrap() = Value::Object(handle(&second));
    *store.slot_mut(&second, &key).unwrap() = Value::Object(handle(&first));
    *store.slot_mut(&isolated, &key).unwrap() = Value::Object(handle(&isolated));

    let first_pass = store.collect_with([handle(&first)], |slot, visit| {
        slot.trace_object_handles(&mut |handle| visit(handle));
    });
    assert_eq!(first_pass.reclaimed_objects, 1);
    assert_eq!(store.len(), 2);
    assert!(store.state(&first).is_ok());
    assert!(store.state(&second).is_ok());
    assert!(store.state(&isolated).is_err());

    let second_pass = store.collect_with([], |slot, visit| {
        slot.trace_object_handles(&mut |handle| visit(handle));
    });
    assert_eq!(second_pass.reclaimed_objects, 2);
    assert!(store.is_empty());
}

#[test]
fn collection_metrics_and_object_ids_remain_monotonic() {
    let mut registry = ClassRegistry::<Value>::new();
    let class = registry
        .register_class(ClassDefinition::handle("Leaf"))
        .unwrap();
    let mut store = ObjectStore::new();
    let first = store.allocate(&registry, class).unwrap();
    finish(&mut store, &first);

    let before = store.gc_metrics();
    assert_eq!(before.live_objects, 1);
    assert_eq!(before.total_allocated_objects, 1);
    assert!(before.allocation_debt > 0);

    let collected = store.collect_with([], |slot, visit| {
        slot.trace_object_handles(&mut |handle| visit(handle));
    });
    assert_eq!(collected.reclaimed_objects, 1);
    let after = store.gc_metrics();
    assert_eq!(after.collections, 1);
    assert_eq!(after.total_reclaimed_objects, 1);
    assert_eq!(after.allocation_debt, 0);

    let second = store.allocate(&registry, class).unwrap();
    assert!(second.id().get() > first.id().get());
}

#[test]
fn two_phase_collection_retains_cycle_slots_and_orders_candidates_deterministically() {
    let mut registry = ClassRegistry::new();
    let class = registry
        .register_class(
            ClassDefinition::handle("FinalNode")
                .with_property(PropertyDescriptor::stored("peer", Some(Value::Nothing))),
        )
        .unwrap();
    let PropertyResolution::Stored { key, .. } = registry
        .resolve_property(
            class,
            "peer",
            PropertyAccess::Set,
            AccessContext::external(),
        )
        .unwrap()
    else {
        panic!("peer must be stored");
    };
    let mut store = ObjectStore::new();
    let first = store.allocate(&registry, class).unwrap();
    let second = store.allocate(&registry, class).unwrap();
    finish(&mut store, &first);
    finish(&mut store, &second);
    *store.slot_mut(&first, &key).unwrap() = Value::Object(handle(&second));
    *store.slot_mut(&second, &key).unwrap() = Value::Object(handle(&first));

    let plan = store.begin_collection_with(
        [],
        |slot, visit| slot.trace_object_handles(&mut |edge| visit(edge)),
        |candidate_class| candidate_class == class,
    );
    assert_eq!(
        plan.candidates()
            .iter()
            .map(openmat_object::FinalizationCandidate::id)
            .collect::<Vec<_>>(),
        vec![second.id(), first.id()]
    );
    assert_eq!(store.len(), 2);
    assert_eq!(store.handle_state(&first), Ok(HandleState::Finalizing));
    assert_eq!(
        store.finalizer_slot(&plan.candidates()[0], &key),
        Ok(&Value::Object(handle(&first)))
    );

    for candidate in plan.candidates() {
        store.finish_finalization(candidate).unwrap();
    }
    assert_eq!(store.gc_metrics().total_reclaimed_objects, 2);

    let collection = store.finish_collection(plan);
    assert_eq!(collection.reclaimed_objects, 2);
    assert_eq!(store.gc_metrics().total_reclaimed_objects, 2);
    assert_eq!(store.handle_state(&first), Ok(HandleState::Invalid));
    assert_eq!(store.handle_state(&second), Ok(HandleState::Invalid));
}

#[test]
fn classes_without_destructors_use_the_immediate_collection_path() {
    let mut registry = ClassRegistry::<Value>::new();
    let class = registry
        .register_class(ClassDefinition::handle("PlainHandle"))
        .unwrap();
    let mut store = ObjectStore::new();
    let object = store.allocate(&registry, class).unwrap();
    finish(&mut store, &object);

    let collection = store.collect_with([], |slot, visit| {
        slot.trace_object_handles(&mut |edge| visit(edge));
    });
    assert_eq!(collection.reclaimed_objects, 1);
    assert_eq!(store.handle_state(&object), Ok(HandleState::Invalid));
    assert!(store.is_empty());
}

#[test]
fn cancelled_collection_discovery_rolls_back_before_invalidation() {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Cancelled;

    let mut registry = ClassRegistry::<Value>::new();
    let class = registry
        .register_class(ClassDefinition::handle("KeptAlive"))
        .unwrap();
    let mut store = ObjectStore::new();
    let object = store.allocate(&registry, class).unwrap();
    finish(&mut store, &object);

    let result = store.try_begin_collection_with(
        [],
        |_slot, _visit| Ok::<(), Cancelled>(()),
        |_class| Err(Cancelled),
    );
    assert_eq!(result, Err(Cancelled));
    assert_eq!(store.handle_state(&object), Ok(HandleState::Alive));
    assert_eq!(store.len(), 1);
}

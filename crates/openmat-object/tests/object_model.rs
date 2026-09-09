use openmat_object::{
    Access, AccessContext, AttributeOwner, ClassDefinition, ClassRegistry, ClassSemantics,
    ConstructionState, EnumerationMemberDescriptor, EventAccess, EventDescriptor, HandleState,
    MethodDescriptor, MethodKind, ObjectArrayDescriptor, ObjectError, ObjectRef, ObjectStore,
    Operator, PropertyAccess, PropertyDescriptor, PropertyResolution,
};

fn finish<S>(store: &mut ObjectStore<S>, object: &ObjectRef) {
    store.begin_construction(object).unwrap();
    store.finish_construction(object).unwrap();
}

#[test]
fn inheritance_propagates_handle_semantics_and_overrides_methods() {
    let mut registry = ClassRegistry::<i32>::new();
    let base = registry
        .register_class(
            ClassDefinition::handle("Base")
                .with_property(PropertyDescriptor::stored("base_value", Some(1)))
                .with_method(MethodDescriptor::instance("describe"))
                .with_method(MethodDescriptor::instance("base_only")),
        )
        .unwrap();
    let child = registry
        .register_class(
            ClassDefinition::value("Child")
                .with_superclass(base)
                .with_property(PropertyDescriptor::stored("child_value", Some(2)))
                .with_method(MethodDescriptor::instance("describe")),
        )
        .unwrap();

    assert_eq!(
        registry.class(child).unwrap().declared_semantics(),
        ClassSemantics::Value
    );
    assert_eq!(registry.semantics(child).unwrap(), ClassSemantics::Handle);
    assert!(registry.is_subclass_of(child, base).unwrap());

    let overridden = registry
        .resolve_method(
            child,
            "describe",
            MethodKind::Instance,
            AccessContext::external(),
        )
        .unwrap();
    assert_eq!(overridden.declaring_class(), child);

    let inherited = registry
        .resolve_method(
            child,
            "base_only",
            MethodKind::Instance,
            AccessContext::external(),
        )
        .unwrap();
    assert_eq!(inherited.declaring_class(), base);

    let slots = registry.stored_properties(child).unwrap();
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].0.declaring_class(), base);
    assert_eq!(slots[1].0.declaring_class(), child);
}

#[test]
fn ordered_multiple_superclasses_contribute_ancestry_members_and_storage() {
    let mut registry = ClassRegistry::<i32>::new();
    let left = registry
        .register_class(
            ClassDefinition::value("Left")
                .with_property(
                    PropertyDescriptor::stored("shared_value", Some(1))
                        .with_get_access(Access::Private),
                )
                .with_method(MethodDescriptor::instance("from_left"))
                .with_method(MethodDescriptor::instance("shared_method")),
        )
        .unwrap();
    let right = registry
        .register_class(
            ClassDefinition::handle("Right")
                .with_property(
                    PropertyDescriptor::stored("shared_value", Some(2))
                        .with_get_access(Access::Private),
                )
                .with_method(MethodDescriptor::instance("from_right"))
                .with_method(MethodDescriptor::instance("shared_method")),
        )
        .unwrap();
    let child = registry
        .register_class(
            ClassDefinition::value("Child")
                .with_superclass(left)
                .with_additional_superclass(right)
                .with_property(PropertyDescriptor::stored("child_value", Some(3))),
        )
        .unwrap();

    assert_eq!(
        registry
            .class(child)
            .unwrap()
            .superclasses()
            .collect::<Vec<_>>(),
        vec![left, right]
    );
    assert!(registry.is_subclass_of(child, left).unwrap());
    assert!(registry.is_subclass_of(child, right).unwrap());
    assert_eq!(registry.semantics(child).unwrap(), ClassSemantics::Handle);

    let right_method = registry
        .resolve_method(
            child,
            "from_right",
            MethodKind::Instance,
            AccessContext::external(),
        )
        .unwrap();
    assert_eq!(right_method.declaring_class(), right);
    let shared_method = registry
        .resolve_method(
            child,
            "shared_method",
            MethodKind::Instance,
            AccessContext::external(),
        )
        .unwrap();
    assert_eq!(shared_method.declaring_class(), left);
    let effective_shared = registry
        .effective_methods(child)
        .unwrap()
        .into_iter()
        .find(|(_, method)| method.name() == "shared_method")
        .expect("shared method should remain visible");
    assert_eq!(effective_shared.0, left);

    let right_property = registry
        .resolve_property(
            child,
            "shared_value",
            PropertyAccess::Get,
            AccessContext::class(right),
        )
        .unwrap();
    let PropertyResolution::Stored { key, .. } = right_property else {
        panic!("right parent property should use stored-slot resolution");
    };
    assert_eq!(key.declaring_class(), right);

    let slots = registry.stored_properties(child).unwrap();
    assert_eq!(slots.len(), 3);
    assert_eq!(slots[0].0.declaring_class(), left);
    assert_eq!(slots[1].0.declaring_class(), right);
    assert_eq!(slots[2].0.declaring_class(), child);
}

#[test]
fn transitive_ancestry_and_direct_constructor_access_keep_classes_distinct() {
    let mut registry = ClassRegistry::<()>::new();
    let base = registry
        .register_class(
            ClassDefinition::value("Base")
                .with_method(MethodDescriptor::instance("Base").with_access(Access::Protected)),
        )
        .unwrap();
    let middle = registry
        .register_class(ClassDefinition::value("Middle").with_superclass(base))
        .unwrap();
    let leaf = registry
        .register_class(ClassDefinition::value("Leaf").with_superclass(middle))
        .unwrap();
    let unrelated_handle = registry
        .register_class(ClassDefinition::handle("UnrelatedHandle"))
        .unwrap();

    assert_eq!(registry.class(leaf).unwrap().superclass(), Some(middle));
    assert!(registry.is_subclass_of(leaf, leaf).unwrap());
    assert!(registry.is_subclass_of(leaf, middle).unwrap());
    assert!(registry.is_subclass_of(leaf, base).unwrap());
    assert!(!registry.is_subclass_of(leaf, unrelated_handle).unwrap());
    assert_eq!(registry.semantics(leaf).unwrap(), ClassSemantics::Value);
    assert_eq!(
        registry.semantics(unrelated_handle).unwrap(),
        ClassSemantics::Handle
    );

    let constructor = registry
        .resolve_constructor(base, AccessContext::class(middle))
        .expect("a subclass may call a protected direct-base constructor")
        .expect("base constructor");
    assert_eq!(constructor.declaring_class(), base);
}

#[test]
fn public_protected_and_private_access_use_declaring_class() {
    let mut registry = ClassRegistry::<()>::new();
    let base = registry
        .register_class(
            ClassDefinition::value("Base")
                .with_method(MethodDescriptor::instance("public_method"))
                .with_method(
                    MethodDescriptor::instance("protected_method").with_access(Access::Protected),
                )
                .with_method(
                    MethodDescriptor::instance("private_method").with_access(Access::Private),
                )
                .with_property(
                    PropertyDescriptor::stored("secret", None)
                        .with_get_access(Access::Private)
                        .with_set_access(Access::Protected),
                ),
        )
        .unwrap();
    let child = registry
        .register_class(ClassDefinition::value("Child").with_superclass(base))
        .unwrap();
    let outsider = registry
        .register_class(ClassDefinition::value("Outsider"))
        .unwrap();

    registry
        .resolve_method(
            child,
            "public_method",
            MethodKind::Instance,
            AccessContext::external(),
        )
        .unwrap();
    registry
        .resolve_method(
            child,
            "protected_method",
            MethodKind::Instance,
            AccessContext::class(child),
        )
        .unwrap();
    assert!(matches!(
        registry.resolve_method(
            child,
            "protected_method",
            MethodKind::Instance,
            AccessContext::class(outsider)
        ),
        Err(ObjectError::AccessDenied {
            required: Access::Protected,
            ..
        })
    ));
    registry
        .resolve_method(
            child,
            "private_method",
            MethodKind::Instance,
            AccessContext::class(base),
        )
        .unwrap();
    assert!(matches!(
        registry.resolve_method(
            child,
            "private_method",
            MethodKind::Instance,
            AccessContext::class(child)
        ),
        Err(ObjectError::AccessDenied {
            required: Access::Private,
            ..
        })
    ));

    registry
        .resolve_property(
            child,
            "secret",
            PropertyAccess::Set,
            AccessContext::class(child),
        )
        .unwrap();
    assert!(matches!(
        registry.resolve_property(
            child,
            "secret",
            PropertyAccess::Get,
            AccessContext::class(child)
        ),
        Err(ObjectError::AccessDenied {
            required: Access::Private,
            ..
        })
    ));
}

#[test]
fn static_constant_and_dependent_members_resolve_by_convention() {
    let mut registry = ClassRegistry::<i32>::new();
    let class = registry
        .register_class(
            ClassDefinition::value("Measured")
                .with_property(PropertyDescriptor::constant("Version", 7))
                .with_property(PropertyDescriptor::dependent("Magnitude"))
                .with_method(
                    MethodDescriptor::instance("get.Magnitude").with_access(Access::Private),
                )
                .with_method(MethodDescriptor::instance("set.Magnitude"))
                .with_method(MethodDescriptor::static_method("create")),
        )
        .unwrap();

    let constant = registry
        .resolve_property(
            class,
            "Version",
            PropertyAccess::Get,
            AccessContext::external(),
        )
        .unwrap();
    match constant {
        PropertyResolution::Constant { descriptor } => {
            assert_eq!(descriptor.default_value(), Some(&7));
        }
        other => panic!("unexpected resolution: {other:?}"),
    }
    assert!(matches!(
        registry.resolve_property(
            class,
            "Version",
            PropertyAccess::Set,
            AccessContext::external()
        ),
        Err(ObjectError::PropertyReadOnly { .. })
    ));

    let getter = registry
        .resolve_property(
            class,
            "Magnitude",
            PropertyAccess::Get,
            AccessContext::external(),
        )
        .unwrap();
    match getter {
        PropertyResolution::Accessor { method, .. } => {
            assert_eq!(method.descriptor().name(), "get.Magnitude");
        }
        other => panic!("unexpected resolution: {other:?}"),
    }

    registry
        .resolve_method(
            class,
            "create",
            MethodKind::Static,
            AccessContext::external(),
        )
        .unwrap();
    assert!(matches!(
        registry.resolve_method(
            class,
            "create",
            MethodKind::Instance,
            AccessContext::external()
        ),
        Err(ObjectError::MethodKindMismatch { .. })
    ));
    assert!(registry.stored_properties(class).unwrap().is_empty());
}

#[test]
fn dependent_properties_require_accessors_and_preserve_read_write_modes() {
    let mut registry = ClassRegistry::<i32>::new();
    let missing = registry
        .register_class(
            ClassDefinition::value("MissingGetter")
                .with_property(PropertyDescriptor::dependent("Data")),
        )
        .unwrap();
    assert!(matches!(
        registry.resolve_property(
            missing,
            "Data",
            PropertyAccess::Get,
            AccessContext::external()
        ),
        Err(ObjectError::MissingAccessor { accessor, .. }) if accessor == "get.Data"
    ));

    let read_only = registry
        .register_class(
            ClassDefinition::value("ReadOnly")
                .with_property(PropertyDescriptor::dependent("Data").read_only())
                .with_method(MethodDescriptor::instance("get.Data")),
        )
        .unwrap();
    assert!(matches!(
        registry.resolve_property(
            read_only,
            "Data",
            PropertyAccess::Set,
            AccessContext::external()
        ),
        Err(ObjectError::PropertyReadOnly { .. })
    ));

    let write_only = registry
        .register_class(
            ClassDefinition::value("WriteOnly")
                .with_property(PropertyDescriptor::dependent("Data").write_only())
                .with_method(MethodDescriptor::instance("set.Data")),
        )
        .unwrap();
    assert!(matches!(
        registry.resolve_property(
            write_only,
            "Data",
            PropertyAccess::Get,
            AccessContext::external()
        ),
        Err(ObjectError::PropertyWriteOnly { .. })
    ));
}

#[test]
fn constructors_are_direct_instance_methods_with_checked_access() {
    let mut registry = ClassRegistry::<i32>::new();
    let implicit = registry
        .register_class(ClassDefinition::value("Implicit"))
        .unwrap();
    assert!(
        registry
            .resolve_constructor(implicit, AccessContext::external())
            .unwrap()
            .is_none()
    );

    let constructible =
        registry
            .register_class(ClassDefinition::value("Constructible").with_method(
                MethodDescriptor::instance("Constructible").with_access(Access::Private),
            ))
            .unwrap();
    assert!(matches!(
        registry.resolve_constructor(constructible, AccessContext::external()),
        Err(ObjectError::AccessDenied { .. })
    ));
    let constructor = registry
        .resolve_constructor(constructible, AccessContext::class(constructible))
        .unwrap()
        .unwrap();
    assert_eq!(constructor.descriptor().name(), "Constructible");

    let packaged = registry
        .register_class(
            ClassDefinition::value("alpha.Box").with_method(MethodDescriptor::instance("Box")),
        )
        .unwrap();
    let constructor = registry
        .resolve_constructor(packaged, AccessContext::external())
        .unwrap()
        .unwrap();
    assert_eq!(constructor.descriptor().name(), "Box");
}

#[test]
fn value_assignment_copies_slots_and_requires_construction() {
    let mut registry = ClassRegistry::<i32>::new();
    let class = registry
        .register_class(
            ClassDefinition::value("Point")
                .with_property(PropertyDescriptor::stored("x", Some(3)))
                .with_property(PropertyDescriptor::stored("y", None)),
        )
        .unwrap();
    let mut store = ObjectStore::new();
    let original = store
        .allocate_with(&registry, class, |property| {
            if property.name() == "y" {
                Ok(4)
            } else {
                Err(ObjectError::MissingPropertyInitializer {
                    class_id: class,
                    property: property.name().to_owned(),
                })
            }
        })
        .unwrap();

    assert!(matches!(
        store.assignment_copy(&original),
        Err(ObjectError::ObjectNotConstructed {
            state: ConstructionState::Allocated,
            ..
        })
    ));
    finish(&mut store, &original);
    let copied = store.assignment_copy(&original).unwrap();
    assert!(!original.aliases(&copied));
    assert_eq!(store.len(), 2);

    let x_key = match registry
        .resolve_property(class, "x", PropertyAccess::Set, AccessContext::external())
        .unwrap()
    {
        PropertyResolution::Stored { key, .. } => key,
        other => panic!("unexpected resolution: {other:?}"),
    };
    *store.slot_mut(&copied, &x_key).unwrap() = 9;
    assert_eq!(store.slot(&original, &x_key).unwrap(), &3);
    assert_eq!(store.slot(&copied, &x_key).unwrap(), &9);

    let descriptor =
        ObjectArrayDescriptor::new(&registry, class, vec![1, 2], &[&original, &copied]).unwrap();
    assert_eq!(descriptor.element_ids(), &[original.id(), copied.id()]);
    assert!(matches!(
        ObjectArrayDescriptor::new(&registry, class, vec![1, 2], &[&original, &original]),
        Err(ObjectError::ValueIdentityAliased { .. })
    ));
}

#[test]
fn fresh_value_assignment_copies_can_be_rolled_back_without_removing_handles() {
    let mut registry = ClassRegistry::<i32>::new();
    let value_class = registry
        .register_class(ClassDefinition::value("ValueItem"))
        .unwrap();
    let handle_class = registry
        .register_class(ClassDefinition::handle("HandleItem"))
        .unwrap();
    let mut store = ObjectStore::new();
    let value = store.allocate(&registry, value_class).unwrap();
    finish(&mut store, &value);
    let copied = store.assignment_copy(&value).unwrap();
    assert_eq!(store.len(), 2);
    store.discard_value_assignment_copy(&copied).unwrap();
    assert_eq!(store.len(), 1);
    assert!(matches!(
        store.state(&copied),
        Err(ObjectError::UnknownObject { .. })
    ));

    let handle = store.allocate(&registry, handle_class).unwrap();
    finish(&mut store, &handle);
    let alias = store.assignment_copy(&handle).unwrap();
    assert!(matches!(
        store.discard_value_assignment_copy(&alias),
        Err(ObjectError::ObjectSemanticsMismatch {
            expected: ClassSemantics::Value,
            actual: ClassSemantics::Handle,
            ..
        })
    ));
    assert_eq!(store.state(&handle), Ok(ConstructionState::Constructed));
}

#[test]
fn handle_assignment_aliases_slots_and_arrays_allow_repeated_identity() {
    let mut registry = ClassRegistry::<i32>::new();
    let handle_base = registry
        .register_class(
            ClassDefinition::handle("Counter")
                .with_property(PropertyDescriptor::stored("count", Some(0))),
        )
        .unwrap();
    let child = registry
        .register_class(ClassDefinition::value("NamedCounter").with_superclass(handle_base))
        .unwrap();
    let mut store = ObjectStore::new();
    let original = store.allocate(&registry, child).unwrap();
    finish(&mut store, &original);
    let alias = store.assignment_copy(&original).unwrap();

    assert!(original.aliases(&alias));
    assert_eq!(store.len(), 1);
    let count_key = match registry
        .resolve_property(
            child,
            "count",
            PropertyAccess::Set,
            AccessContext::external(),
        )
        .unwrap()
    {
        PropertyResolution::Stored { key, .. } => key,
        other => panic!("unexpected resolution: {other:?}"),
    };
    *store.slot_mut(&alias, &count_key).unwrap() = 12;
    assert_eq!(store.slot(&original, &count_key).unwrap(), &12);

    ObjectArrayDescriptor::new(&registry, child, vec![2, 1], &[&original, &alias]).unwrap();
}

#[test]
fn construction_state_rejects_invalid_transitions() {
    let mut registry = ClassRegistry::<i32>::new();
    let class = registry
        .register_class(ClassDefinition::value("Simple"))
        .unwrap();
    let mut store = ObjectStore::new();
    let object = store.allocate(&registry, class).unwrap();

    assert!(matches!(
        store.finish_construction(&object),
        Err(ObjectError::InvalidConstructionTransition {
            from: ConstructionState::Allocated,
            to: ConstructionState::Constructed,
            ..
        })
    ));
    store.begin_construction(&object).unwrap();
    store.fail_construction(&object).unwrap();
    assert_eq!(store.state(&object).unwrap(), ConstructionState::Failed);
    assert!(matches!(
        store.begin_construction(&object),
        Err(ObjectError::InvalidConstructionTransition {
            from: ConstructionState::Failed,
            ..
        })
    ));
}

#[test]
fn handle_finalization_invalidates_aliases_once_but_preserves_internal_reads() {
    let mut registry = ClassRegistry::<i32>::new();
    let class = registry
        .register_class(
            ClassDefinition::handle("Resource")
                .with_property(PropertyDescriptor::stored("token", Some(41))),
        )
        .unwrap();
    let PropertyResolution::Stored { key, .. } = registry
        .resolve_property(
            class,
            "token",
            PropertyAccess::Get,
            AccessContext::external(),
        )
        .unwrap()
    else {
        panic!("token must be stored");
    };
    let mut store = ObjectStore::new();
    let original = store.allocate(&registry, class).unwrap();
    finish(&mut store, &original);
    let alias = store.assignment_copy(&original).unwrap();

    let candidate = store.begin_finalization(&alias, true).unwrap().unwrap();
    assert_eq!(store.handle_state(&original), Ok(HandleState::Finalizing));
    assert!(matches!(
        store.slot(&original, &key),
        Err(ObjectError::HandleNotAlive {
            state: HandleState::Finalizing,
            ..
        })
    ));
    assert!(matches!(
        store.state(&alias),
        Err(ObjectError::HandleNotAlive {
            state: HandleState::Finalizing,
            ..
        })
    ));
    assert!(matches!(
        store.slot_mut(&alias, &key),
        Err(ObjectError::HandleNotAlive {
            state: HandleState::Finalizing,
            ..
        })
    ));
    assert!(matches!(
        store.assignment_copy(&original),
        Err(ObjectError::HandleNotAlive {
            state: HandleState::Finalizing,
            ..
        })
    ));
    assert_eq!(candidate.semantics(), ClassSemantics::Handle);
    assert_eq!(store.finalizer_slot(&candidate, &key), Ok(&41));
    assert!(store.begin_finalization(&original, true).unwrap().is_none());

    let reclaimed_before = store.gc_metrics().total_reclaimed_objects;
    assert_eq!(store.finish_finalization(&candidate), Ok(true));
    assert_eq!(
        store.gc_metrics().total_reclaimed_objects,
        reclaimed_before + 1
    );
    assert_eq!(store.finish_finalization(&candidate), Ok(false));
    assert_eq!(
        store.gc_metrics().total_reclaimed_objects,
        reclaimed_before + 1
    );
    assert_eq!(store.handle_state(&alias), Ok(HandleState::Invalid));
    assert!(store.begin_finalization(&alias, true).unwrap().is_none());
}

#[test]
fn constructor_failure_can_finalize_and_incomplete_rollback_invalidates_handles() {
    let mut registry = ClassRegistry::<i32>::new();
    let class = registry
        .register_class(
            ClassDefinition::handle("Partial")
                .with_property(PropertyDescriptor::stored("progress", Some(7))),
        )
        .unwrap();
    let PropertyResolution::Stored { key, .. } = registry
        .resolve_property(
            class,
            "progress",
            PropertyAccess::Get,
            AccessContext::external(),
        )
        .unwrap()
    else {
        panic!("progress must be stored");
    };
    let mut store = ObjectStore::new();
    let failed = store.allocate(&registry, class).unwrap();
    store.begin_construction(&failed).unwrap();
    store.fail_construction(&failed).unwrap();

    let candidate = store.begin_finalization(&failed, true).unwrap().unwrap();
    assert_eq!(candidate.construction_state(), ConstructionState::Failed);
    assert_eq!(store.finalizer_slot(&candidate, &key), Ok(&7));
    store.finish_finalization(&candidate).unwrap();
    assert_eq!(store.handle_state(&failed), Ok(HandleState::Invalid));

    let rolled_back = store.allocate(&registry, class).unwrap();
    store.discard_incomplete(&rolled_back).unwrap();
    assert_eq!(store.handle_state(&rolled_back), Ok(HandleState::Invalid));

    let no_destructor = store.allocate(&registry, class).unwrap();
    finish(&mut store, &no_destructor);
    let reclaimed_before = store.gc_metrics().total_reclaimed_objects;
    assert!(
        store
            .begin_finalization(&no_destructor, false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.gc_metrics().total_reclaimed_objects,
        reclaimed_before + 1
    );
    assert_eq!(store.handle_state(&no_destructor), Ok(HandleState::Invalid));
}

#[test]
fn bad_class_definitions_are_structured_and_registration_is_atomic() {
    let mut registry = ClassRegistry::<()>::new();
    let cycle = registry.register_classes([
        ClassDefinition::value("A").with_superclass("B"),
        ClassDefinition::value("B").with_superclass("A"),
    ]);
    assert!(matches!(cycle, Err(ObjectError::InheritanceCycle { .. })));
    assert!(registry.is_empty());

    let base = registry
        .register_class(
            ClassDefinition::value("Base")
                .with_property(PropertyDescriptor::stored("data", None))
                .with_method(MethodDescriptor::instance("run")),
        )
        .unwrap();
    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("DuplicateBase")
                .with_superclass(base)
                .with_additional_superclass(base)
        ),
        Err(ObjectError::DuplicateSuperclass { superclass, .. }) if superclass == base
    ));
    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("BadProperty")
                .with_superclass(base)
                .with_property(PropertyDescriptor::stored("data", None))
        ),
        Err(ObjectError::InheritedPropertyConflict { .. })
    ));
    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("BadOverride")
                .with_superclass(base)
                .with_method(MethodDescriptor::static_method("run"))
        ),
        Err(ObjectError::OverrideKindMismatch { .. })
    ));
    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("UnknownAttribute").with_property(
                PropertyDescriptor::stored("x", None).with_unsupported_attribute("Transient")
            )
        ),
        Err(ObjectError::UnsupportedAttribute {
            owner: AttributeOwner::Property(name),
            attribute,
        }) if name == "x" && attribute == "Transient"
    ));
    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("BadConstructor")
                .with_method(MethodDescriptor::static_method("BadConstructor"))
        ),
        Err(ObjectError::InvalidConstructorKind { .. })
    ));
    assert!(matches!(
        registry.register_class(ClassDefinition::value("WritableConstant").with_property(
            PropertyDescriptor::constant("Answer", ()).with_set_access(Access::Public)
        )),
        Err(ObjectError::InvalidPropertyDefinition { .. })
    ));
    assert_eq!(registry.len(), 1);
}

#[test]
fn operators_map_to_conventional_override_names() {
    assert_eq!(Operator::Add.method_name(), "plus");
    assert_eq!(Operator::MatrixMultiply.method_name(), "mtimes");
    assert_eq!(Operator::Equal.method_name(), "eq");
    assert_eq!(Operator::LessThanOrEqual.method_name(), "le");

    let mut registry = ClassRegistry::<()>::new();
    let base = registry
        .register_class(
            ClassDefinition::value("Number")
                .with_method(MethodDescriptor::instance(Operator::Add.method_name())),
        )
        .unwrap();
    let child = registry
        .register_class(
            ClassDefinition::value("SpecialNumber")
                .with_superclass(base)
                .with_method(MethodDescriptor::instance(Operator::Add.method_name())),
        )
        .unwrap();

    let selected = registry
        .resolve_operator(child, Operator::Add, AccessContext::external())
        .unwrap();
    assert_eq!(selected.declaring_class(), child);
}

#[test]
fn homogeneous_arrays_reject_subclass_elements_and_bad_shape() {
    let mut registry = ClassRegistry::<i32>::new();
    let base = registry
        .register_class(ClassDefinition::value("Base"))
        .unwrap();
    let child = registry
        .register_class(ClassDefinition::value("Child").with_superclass(base))
        .unwrap();
    let mut store = ObjectStore::new();
    let base_object = store.allocate(&registry, base).unwrap();
    finish(&mut store, &base_object);
    let child_object = store.allocate(&registry, child).unwrap();
    finish(&mut store, &child_object);

    assert!(matches!(
        ObjectArrayDescriptor::new(&registry, base, vec![1, 2], &[&base_object, &child_object]),
        Err(ObjectError::HeterogeneousArrayElement { index: 1, .. })
    ));
    assert!(matches!(
        ObjectArrayDescriptor::new(&registry, base, vec![3, 1], &[&base_object]),
        Err(ObjectError::ArrayElementCountMismatch {
            expected: 3,
            actual: 1
        })
    ));
    assert!(matches!(
        ObjectArrayDescriptor::new(&registry, base, vec![u64::MAX, 2], &[]),
        Err(ObjectError::ShapeOverflow)
    ));
}

#[test]
fn abstract_slots_link_across_layers_and_gate_instantiation() {
    let mut registry = ClassRegistry::<()>::new();
    let base = registry
        .register_class(
            ClassDefinition::value("AbstractBase")
                .with_abstract(true)
                .with_method(MethodDescriptor::instance("f").abstract_method())
                .with_method(MethodDescriptor::instance("g").abstract_method()),
        )
        .unwrap();
    let middle = registry
        .register_class(
            ClassDefinition::value("AbstractMiddle")
                .with_superclass(base)
                .with_method(MethodDescriptor::instance("f"))
                .with_method(
                    MethodDescriptor::instance("h")
                        .with_access(Access::Protected)
                        .abstract_method(),
                ),
        )
        .unwrap();
    let missing = registry
        .register_class(
            ClassDefinition::value("MissingLeaf")
                .with_superclass(middle)
                .with_method(MethodDescriptor::instance("g")),
        )
        .unwrap();
    assert!(registry.class(missing).unwrap().is_abstract());
    assert!(matches!(
        registry.ensure_instantiable(missing),
        Err(ObjectError::AbstractClassInstantiation { .. })
    ));

    let leaf = registry
        .register_class(
            ClassDefinition::value("ConcreteLeaf")
                .with_superclass(middle)
                .with_method(MethodDescriptor::instance("g"))
                .with_method(MethodDescriptor::instance("h").with_access(Access::Protected)),
        )
        .unwrap();
    assert!(!registry.class(leaf).unwrap().is_abstract());
    assert_eq!(registry.ensure_instantiable(leaf), Ok(()));
    let protected = registry
        .resolve_method_any(leaf, "h", AccessContext::class(middle))
        .expect("the class that declares a protected abstract slot may dispatch its override");
    assert_eq!(protected.declaring_class(), leaf);
    assert!(matches!(
        registry.resolve_method_any(base, "f", AccessContext::external()),
        Err(ObjectError::AbstractMethodCall { .. })
    ));
}

#[test]
fn protected_virtual_hooks_keep_access_from_the_base_declaration() {
    // Locally authored MATLAB R2022b probe: Base.run -> obj.setup dispatches the
    // protected Derived.setup override and returns 7. The caller remains Base.
    let mut registry = ClassRegistry::<()>::new();
    let base = registry
        .register_class(
            ClassDefinition::handle("UiBase")
                .with_method(MethodDescriptor::instance("setup").with_access(Access::Protected)),
        )
        .unwrap();
    let derived = registry
        .register_class(
            ClassDefinition::handle("UiDerived")
                .with_superclass(base)
                .with_method(MethodDescriptor::instance("setup").with_access(Access::Protected))
                .with_method(
                    MethodDescriptor::instance("unrelated").with_access(Access::Protected),
                ),
        )
        .unwrap();
    assert_eq!(
        registry
            .resolve_method_any(derived, "setup", AccessContext::class(base))
            .unwrap()
            .declaring_class(),
        derived
    );
    assert!(
        registry
            .resolve_method_any(derived, "setup", AccessContext::external())
            .is_err()
    );
    assert!(
        registry
            .resolve_method_any(derived, "unrelated", AccessContext::class(base))
            .is_err()
    );
}

#[test]
fn sealed_and_abstract_override_constraints_are_structured() {
    let mut registry = ClassRegistry::<()>::new();
    let sealed = registry
        .register_class(ClassDefinition::value("Final").sealed())
        .unwrap();
    assert!(matches!(
        registry.register_class(ClassDefinition::value("Nope").with_superclass(sealed)),
        Err(ObjectError::SealedSuperclass { .. })
    ));

    let abstract_base = registry
        .register_class(
            ClassDefinition::value("ProtectedContract").with_method(
                MethodDescriptor::instance("run")
                    .with_access(Access::Protected)
                    .abstract_method(),
            ),
        )
        .unwrap();
    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("BadAccess")
                .with_superclass(abstract_base)
                .with_method(MethodDescriptor::instance("run"))
        ),
        Err(ObjectError::AbstractOverrideAccessMismatch { .. })
    ));
    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("ExplicitConcrete")
                .with_abstract(false)
                .with_superclass(abstract_base)
        ),
        Err(ObjectError::ExplicitConcreteWithAbstractMethods { .. })
    ));
}

#[test]
fn abstract_slot_implementation_matches_by_name_even_when_static() {
    let mut registry = ClassRegistry::<()>::new();
    let base = registry
        .register_class(
            ClassDefinition::value("KindContract")
                .with_method(MethodDescriptor::instance("convert").abstract_method()),
        )
        .unwrap();
    let leaf = registry
        .register_class(
            ClassDefinition::value("KindImplementation")
                .with_superclass(base)
                .with_method(MethodDescriptor::static_method("convert")),
        )
        .unwrap();

    assert!(!registry.class(leaf).unwrap().is_abstract());
    assert_eq!(
        registry
            .resolve_method_any(leaf, "convert", AccessContext::external())
            .unwrap()
            .descriptor()
            .kind(),
        MethodKind::Static
    );
}

#[test]
fn enumeration_and_event_metadata_enforce_lifecycle_boundaries() {
    let mut registry = ClassRegistry::<i32>::new();
    let enumeration = registry
        .register_class(
            ClassDefinition::handle("State")
                .with_enumeration_base("handle")
                .with_enumeration_member(EnumerationMemberDescriptor::new("Ready", vec![1]))
                .with_enumeration_member(EnumerationMemberDescriptor::new("Done", vec![2]))
                .with_event(
                    EventDescriptor::new("Changed")
                        .with_listen_access(Access::Protected)
                        .with_notify_access(Access::Private)
                        .hidden(),
                ),
        )
        .unwrap();
    let descriptor = registry.class(enumeration).unwrap();
    assert!(descriptor.is_enumeration());
    assert!(!descriptor.is_sealed());
    assert_eq!(descriptor.enumeration_base(), Some("handle"));
    assert_eq!(
        descriptor.enumeration_members()[1].constructor_arguments(),
        [2]
    );
    assert_eq!(descriptor.events()[0].notify_access(), Access::Private);
    assert!(descriptor.events()[0].is_hidden());
    assert!(matches!(
        registry.ensure_instantiable(enumeration),
        Err(ObjectError::EnumerationDirectConstruction { .. })
    ));
    assert!(matches!(
        registry.register_class(ClassDefinition::value("StateChild").with_superclass(enumeration)),
        Err(ObjectError::SealedSuperclass { .. })
    ));

    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("ValueEvents").with_event(EventDescriptor::new("Changed"))
        ),
        Err(ObjectError::InvalidPropertyDefinition { .. })
    ));
}

#[test]
fn inherited_event_resolution_uses_the_declaring_class_access_boundary() {
    let mut registry = ClassRegistry::<()>::new();
    let base = registry
        .register_class(
            ClassDefinition::handle("EventBase")
                .with_event(
                    EventDescriptor::new("Changed")
                        .with_listen_access(Access::Protected)
                        .with_notify_access(Access::Private),
                )
                .with_event(EventDescriptor::new("PublicEvent")),
        )
        .unwrap();
    let child = registry
        .register_class(ClassDefinition::handle("EventChild").with_superclass(base))
        .unwrap();

    let inherited = registry
        .resolve_event(
            child,
            "Changed",
            EventAccess::Listen,
            AccessContext::class(child),
        )
        .unwrap();
    assert_eq!(inherited.receiver_class(), child);
    assert_eq!(inherited.declaring_class(), base);
    assert!(matches!(
        registry.resolve_event(
            child,
            "Changed",
            EventAccess::Listen,
            AccessContext::external(),
        ),
        Err(ObjectError::AccessDenied {
            required: Access::Protected,
            ..
        })
    ));
    assert!(matches!(
        registry.resolve_event(
            child,
            "Changed",
            EventAccess::Notify,
            AccessContext::class(child),
        ),
        Err(ObjectError::AccessDenied {
            required: Access::Private,
            ..
        })
    ));
    assert!(
        registry
            .resolve_event(
                child,
                "Changed",
                EventAccess::Notify,
                AccessContext::class(base),
            )
            .is_ok()
    );
    assert!(
        registry
            .resolve_event(
                child,
                "PublicEvent",
                EventAccess::Listen,
                AccessContext::external(),
            )
            .is_ok()
    );
    assert_eq!(
        registry
            .effective_events(child)
            .unwrap()
            .into_iter()
            .map(|(declaring, event)| (declaring, event.name().to_owned()))
            .collect::<Vec<_>>(),
        vec![
            (base, "Changed".to_owned()),
            (base, "PublicEvent".to_owned())
        ]
    );
}

#[test]
fn abstract_properties_and_sealed_methods_link_across_inheritance() {
    let mut registry = ClassRegistry::<()>::new();
    let base = registry
        .register_class(
            ClassDefinition::value("PropertyContract")
                .with_property(
                    PropertyDescriptor::dependent("Value")
                        .with_set_access(Access::Protected)
                        .abstract_slot(),
                )
                .with_method(MethodDescriptor::instance("run").abstract_method().sealed()),
        )
        .unwrap();
    assert!(registry.class(base).unwrap().is_abstract());

    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("BadPropertyAccess")
                .with_superclass(base)
                .with_property(PropertyDescriptor::stored("Value", None))
        ),
        Err(ObjectError::AbstractPropertyOverrideAccessMismatch { .. })
    ));
    assert!(matches!(
        registry.register_class(
            ClassDefinition::value("BadSealedOverride")
                .with_superclass(base)
                .with_property(
                    PropertyDescriptor::stored("Value", None).with_set_access(Access::Protected)
                )
                .with_method(MethodDescriptor::instance("run"))
        ),
        Err(ObjectError::SealedMethodOverride { .. })
    ));

    let concrete_property = registry
        .register_class(
            ClassDefinition::value("PropertyLeaf")
                .with_superclass(base)
                .with_property(
                    PropertyDescriptor::stored("Value", None).with_set_access(Access::Protected),
                ),
        )
        .unwrap();
    assert!(registry.class(concrete_property).unwrap().is_abstract());
}

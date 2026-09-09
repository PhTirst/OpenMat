use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Access, ClassDefinition, ClassDescriptor, ClassId, ClassSemantics, EventDescriptor,
    MethodDescriptor, MethodKind, ObjectError, ObjectResult, Operator, PropertyDescriptor,
    PropertyKey, PropertyKind, Superclass, descriptor::validate_definition,
};

type AbstractPropertyAccess = (Option<Access>, Option<Access>);
type AbstractPropertySet = BTreeMap<String, AbstractPropertyAccess>;

/// The class, if any, whose method body is performing a member access.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AccessContext {
    caller_class: Option<ClassId>,
}

impl AccessContext {
    #[must_use]
    pub const fn external() -> Self {
        Self { caller_class: None }
    }

    #[must_use]
    pub const fn class(caller_class: ClassId) -> Self {
        Self {
            caller_class: Some(caller_class),
        }
    }

    #[must_use]
    pub const fn caller_class(self) -> Option<ClassId> {
        self.caller_class
    }
}

/// The operation being performed on a property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertyAccess {
    Get,
    Set,
}

/// The access channel used for an event declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventAccess {
    Listen,
    Notify,
}

/// An inherited event selected after visibility checking.
#[derive(Clone, Copy, Debug)]
pub struct EventSelection<'a> {
    receiver_class: ClassId,
    declaring_class: ClassId,
    descriptor: &'a EventDescriptor,
}

impl<'a> EventSelection<'a> {
    #[must_use]
    pub const fn receiver_class(self) -> ClassId {
        self.receiver_class
    }

    #[must_use]
    pub const fn declaring_class(self) -> ClassId {
        self.declaring_class
    }

    #[must_use]
    pub const fn descriptor(self) -> &'a EventDescriptor {
        self.descriptor
    }
}

/// A method selected after virtual lookup and access checking.
#[derive(Clone, Copy, Debug)]
pub struct MethodSelection<'a> {
    receiver_class: ClassId,
    declaring_class: ClassId,
    descriptor: &'a MethodDescriptor,
}

impl<'a> MethodSelection<'a> {
    #[must_use]
    pub const fn receiver_class(self) -> ClassId {
        self.receiver_class
    }

    #[must_use]
    pub const fn declaring_class(self) -> ClassId {
        self.declaring_class
    }

    #[must_use]
    pub const fn descriptor(self) -> &'a MethodDescriptor {
        self.descriptor
    }
}

/// A checked property operation, separated from actual object storage.
#[derive(Clone, Debug)]
pub enum PropertyResolution<'a, S> {
    Stored {
        key: PropertyKey,
        descriptor: &'a PropertyDescriptor<S>,
    },
    Constant {
        descriptor: &'a PropertyDescriptor<S>,
    },
    Accessor {
        descriptor: &'a PropertyDescriptor<S>,
        method: MethodSelection<'a>,
    },
}

/// Registry for validated class metadata and release-one member dispatch.
#[derive(Clone, Debug)]
pub struct ClassRegistry<S = ()> {
    classes: BTreeMap<ClassId, ClassDescriptor<S>>,
    names: BTreeMap<String, ClassId>,
    next_id: u64,
}

impl<S> Default for ClassRegistry<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> ClassRegistry<S> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            classes: BTreeMap::new(),
            names: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Registers one class atomically.
    ///
    /// # Errors
    ///
    /// Returns a structured definition, inheritance, member, or identity error.
    pub fn register_class(&mut self, definition: ClassDefinition<S>) -> ObjectResult<ClassId> {
        let mut ids = self.register_classes([definition])?;
        ids.pop().ok_or(ObjectError::IdExhausted { kind: "class" })
    }

    /// Atomically validates and registers a group of classes. Superclasses may
    /// refer to another definition in the group by name, enabling cycle checks
    /// without partially mutating the registry.
    ///
    /// # Errors
    ///
    /// Returns a structured definition, inheritance, member, or identity error;
    /// the registry is unchanged on failure.
    #[allow(clippy::too_many_lines)]
    pub fn register_classes<I>(&mut self, definitions: I) -> ObjectResult<Vec<ClassId>>
    where
        I: IntoIterator<Item = ClassDefinition<S>>,
    {
        let definitions: Vec<_> = definitions.into_iter().collect();
        if definitions.is_empty() {
            return Ok(Vec::new());
        }

        let mut batch_names = BTreeMap::new();
        let mut batch_ids = Vec::with_capacity(definitions.len());
        let mut next_id = self.next_id;
        for definition in &definitions {
            validate_definition(definition)?;
            if self.names.contains_key(definition.name())
                || batch_names.contains_key(definition.name())
            {
                return Err(ObjectError::DuplicateClass {
                    name: definition.name().to_owned(),
                });
            }
            let id = ClassId::new(next_id);
            next_id = next_id
                .checked_add(1)
                .ok_or(ObjectError::IdExhausted { kind: "class" })?;
            batch_names.insert(definition.name().to_owned(), id);
            batch_ids.push(id);
        }

        let mut batch_indices = BTreeMap::new();
        for (index, id) in batch_ids.iter().copied().enumerate() {
            batch_indices.insert(id, index);
        }

        let mut superclasses = Vec::with_capacity(definitions.len());
        let mut additional_superclasses = Vec::with_capacity(definitions.len());
        for definition in &definitions {
            let superclass = match definition.superclass() {
                None => None,
                Some(Superclass::Id(id)) => {
                    if self.classes.contains_key(id) || batch_indices.contains_key(id) {
                        Some(*id)
                    } else {
                        return Err(ObjectError::UnknownClass { id: *id });
                    }
                }
                Some(Superclass::Name(name)) => self
                    .names
                    .get(name)
                    .or_else(|| batch_names.get(name))
                    .copied()
                    .ok_or_else(|| ObjectError::UnknownClassName { name: name.clone() })
                    .map(Some)?,
            };
            let mut additional = Vec::new();
            additional
                .try_reserve_exact(definition.additional_superclasses().len())
                .map_err(|_| ObjectError::CapacityExceeded {
                    resource: "direct superclasses",
                })?;
            let mut seen = BTreeSet::new();
            if let Some(superclass) = superclass {
                seen.insert(superclass);
            }
            for superclass in definition.additional_superclasses().iter().copied() {
                if !self.classes.contains_key(&superclass) {
                    return Err(ObjectError::UnknownClass { id: superclass });
                }
                if !seen.insert(superclass) {
                    return Err(ObjectError::DuplicateSuperclass {
                        class_name: definition.name().to_owned(),
                        superclass,
                    });
                }
                additional.push(superclass);
            }
            superclasses.push(superclass);
            additional_superclasses.push(additional);
        }

        for (index, definition) in definitions.iter().enumerate() {
            if definition.is_enumeration() && superclasses[index].is_some() {
                return Err(ObjectError::InvalidEnumerationDefinition {
                    class_name: definition.name().to_owned(),
                    reason: "enumeration base metadata cannot be a linked user superclass",
                });
            }
        }

        self.validate_sealed_superclasses(&definitions, &batch_indices, &superclasses)?;
        for (definition, superclasses) in definitions.iter().zip(&additional_superclasses) {
            for superclass in superclasses {
                if self.combined_class_sealed(*superclass, &definitions, &batch_indices)? {
                    return Err(ObjectError::SealedSuperclass {
                        class_name: definition.name().to_owned(),
                        superclass: *superclass,
                    });
                }
            }
        }

        for id in &batch_ids {
            self.check_combined_cycle(*id, &batch_indices, &superclasses)?;
        }

        let mut effective_semantics = Vec::with_capacity(definitions.len());
        for (index, definition) in definitions.iter().enumerate() {
            let effective = if definition.declared_semantics() == ClassSemantics::Handle
                || additional_superclasses[index]
                    .iter()
                    .any(|superclass| self.semantics(*superclass) == Ok(ClassSemantics::Handle))
            {
                ClassSemantics::Handle
            } else {
                self.inherited_semantics(
                    superclasses[index],
                    &definitions,
                    &batch_indices,
                    &superclasses,
                )?
            };
            effective_semantics.push(effective);
        }

        let effective_abstract =
            self.effective_abstractness(&definitions, &batch_ids, &batch_indices, &superclasses)?;

        for (definition, semantics) in definitions.iter().zip(&effective_semantics) {
            if !definition.events().is_empty() && *semantics != ClassSemantics::Handle {
                return Err(ObjectError::InvalidPropertyDefinition {
                    class_name: definition.name().to_owned(),
                    property: "<events>".to_owned(),
                    reason: "events require handle-class semantics",
                });
            }
        }

        for (index, definition) in definitions.iter().enumerate() {
            self.validate_inherited_members(
                definition,
                superclasses[index],
                &definitions,
                &batch_indices,
                &superclasses,
            )?;
        }

        for (((((definition, id), superclass), additional), semantics), abstract_) in definitions
            .into_iter()
            .zip(batch_ids.iter().copied())
            .zip(superclasses)
            .zip(additional_superclasses)
            .zip(effective_semantics)
            .zip(effective_abstract)
        {
            let descriptor = ClassDescriptor::from_definition(
                id, definition, superclass, additional, semantics, abstract_,
            );
            self.names.insert(descriptor.name().to_owned(), id);
            self.classes.insert(id, descriptor);
        }
        self.next_id = next_id;
        Ok(batch_ids)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.classes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    /// Iterates over every explicit stored-property default and constant value.
    ///
    /// Runtimes whose class metadata can contain heap references use these
    /// values as permanent class-definition roots. Properties without an
    /// explicit default and dependent properties contribute no value.
    pub fn default_values(&self) -> impl Iterator<Item = &S> {
        self.classes.values().flat_map(|class| {
            class
                .properties()
                .iter()
                .filter_map(PropertyDescriptor::default_value)
        })
    }

    /// # Errors
    ///
    /// Returns [`ObjectError::UnknownClass`] when `id` is not registered.
    pub fn class(&self, id: ClassId) -> ObjectResult<&ClassDescriptor<S>> {
        self.classes
            .get(&id)
            .ok_or(ObjectError::UnknownClass { id })
    }

    /// # Errors
    ///
    /// Returns [`ObjectError::UnknownClassName`] when `name` is not registered.
    pub fn class_named(&self, name: &str) -> ObjectResult<&ClassDescriptor<S>> {
        let id = self
            .names
            .get(name)
            .copied()
            .ok_or_else(|| ObjectError::UnknownClassName {
                name: name.to_owned(),
            })?;
        self.class(id)
    }

    /// # Errors
    ///
    /// Returns [`ObjectError::UnknownClassName`] when `name` is not registered.
    pub fn class_id(&self, name: &str) -> ObjectResult<ClassId> {
        self.names
            .get(name)
            .copied()
            .ok_or_else(|| ObjectError::UnknownClassName {
                name: name.to_owned(),
            })
    }

    /// # Errors
    ///
    /// Returns [`ObjectError::UnknownClass`] for an unregistered class.
    pub fn semantics(&self, class_id: ClassId) -> ObjectResult<ClassSemantics> {
        Ok(self.class(class_id)?.effective_semantics())
    }

    /// Rejects direct allocation of an effectively abstract class.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectError::AbstractClassInstantiation`] for an abstract
    /// class, or [`ObjectError::UnknownClass`] for an unknown id.
    pub fn ensure_instantiable(&self, class_id: ClassId) -> ObjectResult<()> {
        let class = self.class(class_id)?;
        if class.is_enumeration() {
            Err(ObjectError::EnumerationDirectConstruction {
                class_name: class.name().to_owned(),
            })
        } else if class.is_abstract() {
            Err(ObjectError::AbstractClassInstantiation {
                class_name: class.name().to_owned(),
            })
        } else {
            Ok(())
        }
    }

    /// Returns true for the class itself and for any transitive subclass.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectError::UnknownClass`] if either class is unregistered.
    pub fn is_subclass_of(&self, class_id: ClassId, ancestor: ClassId) -> ObjectResult<bool> {
        self.class(ancestor)?;
        Ok(self
            .ancestry_preorder(class_id)?
            .into_iter()
            .any(|id| id == ancestor))
    }

    /// Performs override selection, invocation-kind validation, and access checking.
    ///
    /// # Errors
    ///
    /// Returns an error when the class or method is unknown, the method kind is
    /// wrong, or the caller lacks access.
    pub fn resolve_method(
        &self,
        receiver_class: ClassId,
        name: &str,
        expected_kind: MethodKind,
        context: AccessContext,
    ) -> ObjectResult<MethodSelection<'_>> {
        let selection = self.resolve_method_any(receiver_class, name, context)?;
        if selection.descriptor.kind() != expected_kind {
            return Err(ObjectError::MethodKindMismatch {
                method: name.to_owned(),
                expected: expected_kind,
                actual: selection.descriptor.kind(),
            });
        }
        Ok(selection)
    }

    /// Performs virtual lookup and access checking without constraining whether
    /// the selected concrete implementation is instance or static. MATLAB uses
    /// this form for object-qualified calls that implement inherited abstract
    /// slots with a static method.
    ///
    /// # Errors
    ///
    /// Returns a structured lookup, access, or unresolved-abstract-slot error.
    pub fn resolve_method_any(
        &self,
        receiver_class: ClassId,
        name: &str,
        context: AccessContext,
    ) -> ObjectResult<MethodSelection<'_>> {
        let selection = self.select_method(receiver_class, name)?;
        let access_declaring_class = self.method_access_declaring_class(
            receiver_class,
            name,
            selection.declaring_class,
            selection.descriptor.access(),
            selection.descriptor.kind(),
        )?;
        self.check_access(
            access_declaring_class,
            selection.descriptor.access(),
            context,
            name,
        )?;
        if selection.descriptor.is_abstract() {
            let class_name = self.class(selection.declaring_class)?.name().to_owned();
            return Err(ObjectError::AbstractMethodCall {
                class_name,
                method: name.to_owned(),
            });
        }
        Ok(selection)
    }

    /// Resolves an operator's conventional instance method.
    ///
    /// # Errors
    ///
    /// Returns the same lookup, kind, and access errors as [`Self::resolve_method`].
    pub fn resolve_operator(
        &self,
        receiver_class: ClassId,
        operator: Operator,
        context: AccessContext,
    ) -> ObjectResult<MethodSelection<'_>> {
        self.resolve_method(
            receiver_class,
            operator.method_name(),
            MethodKind::Instance,
            context,
        )
    }

    /// Resolves only a constructor declared directly by the class. `None`
    /// represents the implicit default constructor.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown class, a static constructor declaration,
    /// or inaccessible constructor.
    pub fn resolve_constructor(
        &self,
        class_id: ClassId,
        context: AccessContext,
    ) -> ObjectResult<Option<MethodSelection<'_>>> {
        let class = self.class(class_id)?;
        let constructor_name = class.name().rsplit('.').next().unwrap_or(class.name());
        let Some(method) = class.local_method(constructor_name) else {
            return Ok(None);
        };
        if method.kind() != MethodKind::Instance {
            return Err(ObjectError::InvalidConstructorKind {
                class_name: class.name().to_owned(),
            });
        }
        self.check_access(class_id, method.access(), context, method.name())?;
        Ok(Some(MethodSelection {
            receiver_class: class_id,
            declaring_class: class_id,
            descriptor: method,
        }))
    }

    /// Resolves a stored slot, constant, or conventional dependent accessor.
    /// Accessor method visibility is an implementation detail; property access
    /// is checked before the accessor is returned.
    ///
    /// # Errors
    ///
    /// Returns an error when the class or property is unknown, access is denied,
    /// a write targets a read-only property, or a dependent accessor is missing.
    pub fn resolve_property(
        &self,
        receiver_class: ClassId,
        name: &str,
        operation: PropertyAccess,
        context: AccessContext,
    ) -> ObjectResult<PropertyResolution<'_, S>> {
        let (declaring_class, property) = self.lookup_property(receiver_class, name, context)?;
        let required_access = match operation {
            PropertyAccess::Get => {
                property
                    .get_access()
                    .ok_or_else(|| ObjectError::PropertyWriteOnly {
                        class_id: receiver_class,
                        property: name.to_owned(),
                    })?
            }
            PropertyAccess::Set => {
                property
                    .set_access()
                    .ok_or_else(|| ObjectError::PropertyReadOnly {
                        class_id: receiver_class,
                        property: name.to_owned(),
                    })?
            }
        };
        self.check_access(declaring_class, required_access, context, name)?;

        match property.kind() {
            PropertyKind::Stored => Ok(PropertyResolution::Stored {
                key: PropertyKey::new(declaring_class, name),
                descriptor: property,
            }),
            PropertyKind::Constant => Ok(PropertyResolution::Constant {
                descriptor: property,
            }),
            PropertyKind::Dependent => {
                let prefix = match operation {
                    PropertyAccess::Get => "get",
                    PropertyAccess::Set => "set",
                };
                let accessor = format!("{prefix}.{name}");
                let method = self
                    .select_method(receiver_class, &accessor)
                    .map_err(|error| {
                        if matches!(error, ObjectError::MethodNotFound { .. }) {
                            ObjectError::MissingAccessor {
                                class_id: receiver_class,
                                property: name.to_owned(),
                                accessor: accessor.clone(),
                            }
                        } else {
                            error
                        }
                    })?;
                if method.descriptor.kind() != MethodKind::Instance {
                    return Err(ObjectError::MethodKindMismatch {
                        method: accessor,
                        expected: MethodKind::Instance,
                        actual: method.descriptor.kind(),
                    });
                }
                Ok(PropertyResolution::Accessor {
                    descriptor: property,
                    method,
                })
            }
        }
    }

    /// Resolves an inherited event and checks its listen or notify access
    /// against the declaring class.
    ///
    /// # Errors
    ///
    /// Returns an unknown-event, unknown-class, or access-denied error.
    pub fn resolve_event(
        &self,
        receiver_class: ClassId,
        name: &str,
        operation: EventAccess,
        context: AccessContext,
    ) -> ObjectResult<EventSelection<'_>> {
        for id in self.ancestry_preorder(receiver_class)? {
            let class = self.class(id)?;
            if let Some(event) = class.events().iter().find(|event| event.name() == name) {
                let required = match operation {
                    EventAccess::Listen => event.listen_access(),
                    EventAccess::Notify => event.notify_access(),
                };
                self.check_access(id, required, context, name)?;
                return Ok(EventSelection {
                    receiver_class,
                    declaring_class: id,
                    descriptor: event,
                });
            }
        }
        Err(ObjectError::EventNotFound {
            class_id: receiver_class,
            event: name.to_owned(),
        })
    }

    /// Returns inherited events in stable base-to-derived declaration order.
    /// Same-named derived declarations replace the inherited slot in place.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectError::UnknownClass`] for an unregistered class.
    pub fn effective_events(
        &self,
        class_id: ClassId,
    ) -> ObjectResult<Vec<(ClassId, &EventDescriptor)>> {
        let priorities = self
            .ancestry_preorder(class_id)?
            .into_iter()
            .enumerate()
            .map(|(priority, id)| (id, priority))
            .collect::<BTreeMap<_, _>>();
        let mut events = Vec::new();
        let mut indices = BTreeMap::new();
        let mut selected_priorities = BTreeMap::new();
        for id in self.ancestry_postorder(class_id)? {
            let priority = priorities[&id];
            for event in self.class(id)?.events() {
                if let Some(index) = indices.get(event.name()).copied() {
                    if priority < selected_priorities[event.name()] {
                        events[index] = (id, event);
                        selected_priorities.insert(event.name().to_owned(), priority);
                    }
                } else {
                    indices.insert(event.name().to_owned(), events.len());
                    selected_priorities.insert(event.name().to_owned(), priority);
                    events.push((id, event));
                }
            }
        }
        Ok(events)
    }

    /// Returns inherited stored properties in base-to-derived initialization
    /// order. Constant and dependent properties have no per-object slot.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectError::UnknownClass`] for an unregistered class.
    pub fn stored_properties(
        &self,
        class_id: ClassId,
    ) -> ObjectResult<Vec<(PropertyKey, &PropertyDescriptor<S>)>> {
        let mut properties = Vec::new();
        for id in self.ancestry_postorder(class_id)? {
            let class = self.class(id)?;
            for property in class.properties() {
                if property.kind() == PropertyKind::Stored {
                    properties.push((PropertyKey::new(id, property.name()), property));
                }
            }
        }
        Ok(properties)
    }

    /// Returns the effective inherited method view in stable base-to-derived
    /// order, with overrides replacing the corresponding inherited slot.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectError::UnknownClass`] for an unregistered class.
    pub fn effective_methods(
        &self,
        class_id: ClassId,
    ) -> ObjectResult<Vec<(ClassId, &MethodDescriptor)>> {
        let priorities = self
            .ancestry_preorder(class_id)?
            .into_iter()
            .enumerate()
            .map(|(priority, id)| (id, priority))
            .collect::<BTreeMap<_, _>>();
        let mut methods = Vec::new();
        let mut indices = BTreeMap::new();
        let mut selected_priorities = BTreeMap::new();
        for id in self.ancestry_postorder(class_id)? {
            let priority = priorities[&id];
            for method in self.class(id)?.methods() {
                if let Some(index) = indices.get(method.name()).copied() {
                    if priority < selected_priorities[method.name()] {
                        methods[index] = (id, method);
                        selected_priorities.insert(method.name().to_owned(), priority);
                    }
                } else {
                    indices.insert(method.name().to_owned(), methods.len());
                    selected_priorities.insert(method.name().to_owned(), priority);
                    methods.push((id, method));
                }
            }
        }
        Ok(methods)
    }

    fn lookup_property(
        &self,
        receiver_class: ClassId,
        name: &str,
        context: AccessContext,
    ) -> ObjectResult<(ClassId, &PropertyDescriptor<S>)> {
        if let Some(caller) = context.caller_class()
            && self.is_subclass_of(receiver_class, caller)?
            && let Some(property) = self.class(caller)?.local_property(name)
        {
            return Ok((caller, property));
        }
        for id in self.ancestry_preorder(receiver_class)? {
            let class = self.class(id)?;
            if let Some(property) = class.local_property(name) {
                return Ok((id, property));
            }
        }
        Err(ObjectError::PropertyNotFound {
            class_id: receiver_class,
            property: name.to_owned(),
        })
    }

    fn select_method(
        &self,
        receiver_class: ClassId,
        name: &str,
    ) -> ObjectResult<MethodSelection<'_>> {
        for id in self.ancestry_preorder(receiver_class)? {
            let class = self.class(id)?;
            if let Some(method) = class.local_method(name) {
                return Ok(MethodSelection {
                    receiver_class,
                    declaring_class: id,
                    descriptor: method,
                });
            }
        }
        Err(ObjectError::MethodNotFound {
            class_id: receiver_class,
            method: name.to_owned(),
        })
    }

    fn method_access_declaring_class(
        &self,
        receiver_class: ClassId,
        name: &str,
        implementation_class: ClassId,
        access: Access,
        kind: MethodKind,
    ) -> ObjectResult<ClassId> {
        let mut owner = implementation_class;
        for id in self.ancestry_preorder(receiver_class)? {
            let class = self.class(id)?;
            if let Some(method) = class.local_method(name)
                && method.access() == access
                && (method.is_abstract() || (access == Access::Protected && method.kind() == kind))
            {
                owner = id;
            }
        }
        Ok(owner)
    }

    fn ancestry_preorder(&self, class_id: ClassId) -> ObjectResult<Vec<ClassId>> {
        let mut ancestry = Vec::new();
        let mut visited = BTreeSet::new();
        self.collect_ancestry_preorder(class_id, &mut visited, &mut ancestry)?;
        Ok(ancestry)
    }

    fn collect_ancestry_preorder(
        &self,
        class_id: ClassId,
        visited: &mut BTreeSet<ClassId>,
        ancestry: &mut Vec<ClassId>,
    ) -> ObjectResult<()> {
        if !visited.insert(class_id) {
            return Ok(());
        }
        let class = self.class(class_id)?;
        ancestry.push(class_id);
        for superclass in class.superclasses() {
            self.collect_ancestry_preorder(superclass, visited, ancestry)?;
        }
        Ok(())
    }

    fn ancestry_postorder(&self, class_id: ClassId) -> ObjectResult<Vec<ClassId>> {
        let mut ancestry = Vec::new();
        let mut visited = BTreeSet::new();
        self.collect_ancestry_postorder(class_id, &mut visited, &mut ancestry)?;
        Ok(ancestry)
    }

    fn collect_ancestry_postorder(
        &self,
        class_id: ClassId,
        visited: &mut BTreeSet<ClassId>,
        ancestry: &mut Vec<ClassId>,
    ) -> ObjectResult<()> {
        if !visited.insert(class_id) {
            return Ok(());
        }
        let class = self.class(class_id)?;
        for superclass in class.superclasses() {
            self.collect_ancestry_postorder(superclass, visited, ancestry)?;
        }
        ancestry.push(class_id);
        Ok(())
    }

    fn check_access(
        &self,
        declaring_class: ClassId,
        required: Access,
        context: AccessContext,
        member: &str,
    ) -> ObjectResult<()> {
        let allowed = match required {
            Access::Public => true,
            Access::Private => context.caller_class == Some(declaring_class),
            Access::Protected => match context.caller_class {
                Some(caller) => self.is_subclass_of(caller, declaring_class)?,
                None => false,
            },
        };
        if allowed {
            Ok(())
        } else {
            Err(ObjectError::AccessDenied {
                member: member.to_owned(),
                declaring_class,
                required,
                caller: context.caller_class,
            })
        }
    }

    fn check_combined_cycle(
        &self,
        start: ClassId,
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<()> {
        let mut visited = BTreeSet::new();
        let mut current = Some(start);
        while let Some(id) = current {
            if !visited.insert(id) {
                return Err(ObjectError::InheritanceCycle { class_id: id });
            }
            current = self.combined_superclass(id, batch_indices, superclasses)?;
        }
        Ok(())
    }

    fn combined_superclass(
        &self,
        id: ClassId,
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<Option<ClassId>> {
        if let Some(index) = batch_indices.get(&id).copied() {
            superclasses
                .get(index)
                .copied()
                .ok_or(ObjectError::UnknownClass { id })
        } else {
            Ok(self.class(id)?.superclass())
        }
    }

    fn inherited_semantics(
        &self,
        mut superclass: Option<ClassId>,
        definitions: &[ClassDefinition<S>],
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<ClassSemantics> {
        while let Some(id) = superclass {
            if let Some(index) = batch_indices.get(&id).copied() {
                let definition = definitions
                    .get(index)
                    .ok_or(ObjectError::UnknownClass { id })?;
                if definition.declared_semantics() == ClassSemantics::Handle {
                    return Ok(ClassSemantics::Handle);
                }
                superclass = superclasses
                    .get(index)
                    .copied()
                    .ok_or(ObjectError::UnknownClass { id })?;
            } else {
                let descriptor = self.class(id)?;
                if descriptor.effective_semantics() == ClassSemantics::Handle {
                    return Ok(ClassSemantics::Handle);
                }
                superclass = descriptor.superclass();
            }
        }
        Ok(ClassSemantics::Value)
    }

    fn validate_inherited_members(
        &self,
        definition: &ClassDefinition<S>,
        superclass: Option<ClassId>,
        definitions: &[ClassDefinition<S>],
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<()> {
        if let Some(constructor) = definition
            .methods()
            .iter()
            .find(|method| method.name() == definition.name())
            && constructor.kind() != MethodKind::Instance
        {
            return Err(ObjectError::InvalidConstructorKind {
                class_name: definition.name().to_owned(),
            });
        }

        for property in definition.properties() {
            if let Some((inherited_from, inherited)) = self.find_combined_property(
                superclass,
                property.name(),
                definitions,
                batch_indices,
                superclasses,
            )? {
                if inherited.is_abstract() {
                    if inherited.get_access() != property.get_access()
                        || inherited.set_access() != property.set_access()
                    {
                        return Err(ObjectError::AbstractPropertyOverrideAccessMismatch {
                            class_name: definition.name().to_owned(),
                            property: property.name().to_owned(),
                        });
                    }
                } else {
                    return Err(ObjectError::InheritedPropertyConflict {
                        class_name: definition.name().to_owned(),
                        property: property.name().to_owned(),
                        inherited_from,
                    });
                }
            }
        }

        for method in definition.methods() {
            if let Some((_, inherited)) = self.find_combined_method(
                superclass,
                method.name(),
                definitions,
                batch_indices,
                superclasses,
            )? {
                if inherited.is_sealed() {
                    return Err(ObjectError::SealedMethodOverride {
                        class_name: definition.name().to_owned(),
                        method: method.name().to_owned(),
                    });
                }
                if inherited.kind() != method.kind() && !inherited.is_abstract() {
                    return Err(ObjectError::OverrideKindMismatch {
                        class_name: definition.name().to_owned(),
                        method: method.name().to_owned(),
                        inherited_kind: inherited.kind(),
                        overriding_kind: method.kind(),
                    });
                }
            }
        }
        Ok(())
    }

    fn combined_class_sealed(
        &self,
        id: ClassId,
        definitions: &[ClassDefinition<S>],
        batch_indices: &BTreeMap<ClassId, usize>,
    ) -> ObjectResult<bool> {
        if let Some(index) = batch_indices.get(&id).copied() {
            definitions
                .get(index)
                .map(|definition| definition.is_sealed() || definition.is_enumeration())
                .ok_or(ObjectError::UnknownClass { id })
        } else {
            let class = self.class(id)?;
            Ok(class.is_sealed() || class.is_enumeration())
        }
    }

    fn validate_sealed_superclasses(
        &self,
        definitions: &[ClassDefinition<S>],
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<()> {
        for (index, definition) in definitions.iter().enumerate() {
            if let Some(superclass) = superclasses[index]
                && self.combined_class_sealed(superclass, definitions, batch_indices)?
            {
                return Err(ObjectError::SealedSuperclass {
                    class_name: definition.name().to_owned(),
                    superclass,
                });
            }
        }
        Ok(())
    }

    fn effective_abstractness(
        &self,
        definitions: &[ClassDefinition<S>],
        batch_ids: &[ClassId],
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<Vec<bool>> {
        let mut effective = Vec::with_capacity(definitions.len());
        for (index, definition) in definitions.iter().enumerate() {
            let unresolved = self.combined_abstract_methods(
                batch_ids[index],
                definitions,
                batch_indices,
                superclasses,
            )?;
            let unresolved_properties = self.combined_abstract_properties(
                batch_ids[index],
                definitions,
                batch_indices,
                superclasses,
            )?;
            if definition.declared_abstract() == Some(false)
                && (!unresolved.is_empty() || !unresolved_properties.is_empty())
            {
                let mut members = unresolved.into_keys().collect::<Vec<_>>();
                members.extend(unresolved_properties.into_keys());
                members.sort();
                return Err(ObjectError::ExplicitConcreteWithAbstractMethods {
                    class_name: definition.name().to_owned(),
                    methods: members,
                });
            }
            effective.push(
                definition.declared_abstract() == Some(true)
                    || !unresolved.is_empty()
                    || !unresolved_properties.is_empty(),
            );
        }
        Ok(effective)
    }

    fn combined_abstract_methods(
        &self,
        class_id: ClassId,
        definitions: &[ClassDefinition<S>],
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<BTreeMap<String, (Access, ClassId)>> {
        let mut ancestry = Vec::new();
        let mut current = Some(class_id);
        while let Some(id) = current {
            ancestry.push(id);
            current = self.combined_superclass(id, batch_indices, superclasses)?;
        }
        ancestry.reverse();

        let mut unresolved = BTreeMap::new();
        for id in ancestry {
            let (class_name, methods) = if let Some(index) = batch_indices.get(&id).copied() {
                let definition = definitions
                    .get(index)
                    .ok_or(ObjectError::UnknownClass { id })?;
                (definition.name(), definition.methods())
            } else {
                let descriptor = self.class(id)?;
                (descriptor.name(), descriptor.methods())
            };
            for method in methods {
                if let Some((inherited_access, _)) = unresolved.get(method.name()).copied()
                    && inherited_access != method.access()
                {
                    return Err(ObjectError::AbstractOverrideAccessMismatch {
                        class_name: class_name.to_owned(),
                        method: method.name().to_owned(),
                        inherited_access,
                        overriding_access: method.access(),
                    });
                }
                if method.is_abstract() {
                    unresolved.insert(method.name().to_owned(), (method.access(), id));
                } else {
                    unresolved.remove(method.name());
                }
            }
        }
        Ok(unresolved)
    }

    fn combined_abstract_properties(
        &self,
        class_id: ClassId,
        definitions: &[ClassDefinition<S>],
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<AbstractPropertySet> {
        let mut ancestry = Vec::new();
        let mut current = Some(class_id);
        while let Some(id) = current {
            ancestry.push(id);
            current = self.combined_superclass(id, batch_indices, superclasses)?;
        }
        ancestry.reverse();

        let mut unresolved = BTreeMap::new();
        for id in ancestry {
            let (class_name, properties) = if let Some(index) = batch_indices.get(&id).copied() {
                let definition = definitions
                    .get(index)
                    .ok_or(ObjectError::UnknownClass { id })?;
                (definition.name(), definition.properties())
            } else {
                let descriptor = self.class(id)?;
                (descriptor.name(), descriptor.properties())
            };
            for property in properties {
                if let Some((get_access, set_access)) = unresolved.get(property.name()).copied()
                    && (get_access != property.get_access() || set_access != property.set_access())
                {
                    return Err(ObjectError::AbstractPropertyOverrideAccessMismatch {
                        class_name: class_name.to_owned(),
                        property: property.name().to_owned(),
                    });
                }
                if property.is_abstract() {
                    unresolved.insert(
                        property.name().to_owned(),
                        (property.get_access(), property.set_access()),
                    );
                } else {
                    unresolved.remove(property.name());
                }
            }
        }
        Ok(unresolved)
    }

    fn find_combined_property<'a>(
        &'a self,
        mut current: Option<ClassId>,
        name: &str,
        definitions: &'a [ClassDefinition<S>],
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<Option<(ClassId, &'a PropertyDescriptor<S>)>> {
        while let Some(id) = current {
            if let Some(index) = batch_indices.get(&id).copied() {
                let definition = definitions
                    .get(index)
                    .ok_or(ObjectError::UnknownClass { id })?;
                if let Some(property) = definition
                    .properties()
                    .iter()
                    .find(|property| property.name() == name)
                {
                    return Ok(Some((id, property)));
                }
                current = superclasses
                    .get(index)
                    .copied()
                    .ok_or(ObjectError::UnknownClass { id })?;
            } else {
                let class = self.class(id)?;
                if let Some(property) = class.local_property(name) {
                    return Ok(Some((id, property)));
                }
                current = class.superclass();
            }
        }
        Ok(None)
    }

    fn find_combined_method<'a>(
        &'a self,
        mut current: Option<ClassId>,
        name: &str,
        definitions: &'a [ClassDefinition<S>],
        batch_indices: &BTreeMap<ClassId, usize>,
        superclasses: &[Option<ClassId>],
    ) -> ObjectResult<Option<(ClassId, &'a MethodDescriptor)>> {
        while let Some(id) = current {
            if let Some(index) = batch_indices.get(&id).copied() {
                let definition = definitions
                    .get(index)
                    .ok_or(ObjectError::UnknownClass { id })?;
                if let Some(method) = definition
                    .methods()
                    .iter()
                    .find(|method| method.name() == name)
                {
                    return Ok(Some((id, method)));
                }
                current = superclasses
                    .get(index)
                    .copied()
                    .ok_or(ObjectError::UnknownClass { id })?;
            } else {
                let class = self.class(id)?;
                if let Some(method) = class.local_method(name) {
                    return Ok(Some((id, method)));
                }
                current = class.superclass();
            }
        }
        Ok(None)
    }
}

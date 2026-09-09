use crate::{ClassId, ObjectError, ObjectResult};

/// Access declared for a method or property operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Access {
    #[default]
    Public,
    Protected,
    Private,
}

/// Whether instances use copy-on-assignment or persistent handle identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ClassSemantics {
    #[default]
    Value,
    Handle,
}

/// A superclass reference accepted by atomic class registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Superclass {
    Id(ClassId),
    Name(String),
}

impl From<ClassId> for Superclass {
    fn from(id: ClassId) -> Self {
        Self::Id(id)
    }
}

impl From<&str> for Superclass {
    fn from(name: &str) -> Self {
        Self::Name(name.to_owned())
    }
}

impl From<String> for Superclass {
    fn from(name: String) -> Self {
        Self::Name(name)
    }
}

/// The supported release-one property storage forms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertyKind {
    Stored,
    Constant,
    Dependent,
}

/// A property declaration and its optional generic default slot value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyDescriptor<S = ()> {
    name: String,
    kind: PropertyKind,
    get_access: Option<Access>,
    set_access: Option<Access>,
    default_value: Option<S>,
    is_abstract: bool,
    unsupported_attributes: Vec<String>,
}

impl<S> PropertyDescriptor<S> {
    /// Declares a stored property. A missing default is supplied by the runtime
    /// slot factory when the object is allocated.
    pub fn stored(name: impl Into<String>, default_value: Option<S>) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Stored,
            get_access: Some(Access::Public),
            set_access: Some(Access::Public),
            default_value,
            is_abstract: false,
            unsupported_attributes: Vec::new(),
        }
    }

    /// Declares a class-wide read-only constant.
    pub fn constant(name: impl Into<String>, value: S) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Constant,
            get_access: Some(Access::Public),
            set_access: None,
            default_value: Some(value),
            is_abstract: false,
            unsupported_attributes: Vec::new(),
        }
    }

    /// Declares a property backed by conventional `get.<name>` and
    /// `set.<name>` instance accessor methods rather than an object slot.
    pub fn dependent(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Dependent,
            get_access: Some(Access::Public),
            set_access: Some(Access::Public),
            default_value: None,
            is_abstract: false,
            unsupported_attributes: Vec::new(),
        }
    }

    /// Declares a body-less property slot that a concrete subclass must
    /// implement with matching read/write access.
    pub fn abstract_property(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Stored,
            get_access: Some(Access::Public),
            set_access: Some(Access::Public),
            default_value: None,
            is_abstract: true,
            unsupported_attributes: Vec::new(),
        }
    }

    /// Declares a body-less constant property slot. A concrete subclass may
    /// discharge it with a same-named property carrying identical access.
    #[must_use]
    pub fn abstract_constant(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Constant,
            get_access: Some(Access::Public),
            set_access: None,
            default_value: None,
            is_abstract: true,
            unsupported_attributes: Vec::new(),
        }
    }

    /// Declares a body-less dependent property slot. Storage kind does not
    /// participate in abstract-slot completion, but retaining it preserves
    /// source metadata for reflection and diagnostics.
    #[must_use]
    pub fn abstract_dependent(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Dependent,
            get_access: Some(Access::Public),
            set_access: Some(Access::Public),
            default_value: None,
            is_abstract: true,
            unsupported_attributes: Vec::new(),
        }
    }

    /// Marks an existing stored or dependent declaration as abstract.
    #[must_use]
    pub fn abstract_slot(mut self) -> Self {
        self.is_abstract = true;
        self.default_value = None;
        self
    }

    #[must_use]
    pub const fn with_get_access(mut self, access: Access) -> Self {
        self.get_access = Some(access);
        self
    }

    #[must_use]
    pub const fn with_set_access(mut self, access: Access) -> Self {
        self.set_access = Some(access);
        self
    }

    #[must_use]
    pub const fn read_only(mut self) -> Self {
        self.set_access = None;
        self
    }

    /// Marks a dependent property as writable but not readable. Registration
    /// rejects this state for stored and constant properties.
    #[must_use]
    pub const fn write_only(mut self) -> Self {
        self.get_access = None;
        self
    }

    /// Preserves an attribute the core object model does not understand so
    /// registration can return a structured frontend-facing diagnostic.
    #[must_use]
    pub fn with_unsupported_attribute(mut self, attribute: impl Into<String>) -> Self {
        self.unsupported_attributes.push(attribute.into());
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> PropertyKind {
        self.kind
    }

    #[must_use]
    pub const fn get_access(&self) -> Option<Access> {
        self.get_access
    }

    #[must_use]
    pub const fn set_access(&self) -> Option<Access> {
        self.set_access
    }

    #[must_use]
    pub const fn default_value(&self) -> Option<&S> {
        self.default_value.as_ref()
    }

    #[must_use]
    pub const fn is_abstract(&self) -> bool {
        self.is_abstract
    }

    pub(crate) fn unsupported_attributes(&self) -> &[String] {
        &self.unsupported_attributes
    }
}

/// Whether a method is invoked with an instance receiver or through its class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MethodKind {
    Instance,
    Static,
}

/// Dispatch metadata. The runtime owns executable code and can key it by the
/// selected declaring class and method name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MethodDescriptor {
    name: String,
    kind: MethodKind,
    access: Access,
    is_abstract: bool,
    is_sealed: bool,
    unsupported_attributes: Vec<String>,
}

impl MethodDescriptor {
    #[must_use]
    pub fn instance(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: MethodKind::Instance,
            access: Access::Public,
            is_abstract: false,
            is_sealed: false,
            unsupported_attributes: Vec::new(),
        }
    }

    #[must_use]
    pub fn static_method(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: MethodKind::Static,
            access: Access::Public,
            is_abstract: false,
            is_sealed: false,
            unsupported_attributes: Vec::new(),
        }
    }

    #[must_use]
    pub const fn with_access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    /// Marks this method as a body-less abstract dispatch slot.
    #[must_use]
    pub const fn abstract_method(mut self) -> Self {
        self.is_abstract = true;
        self
    }

    /// Forbids subclasses from overriding this concrete method.
    #[must_use]
    pub const fn sealed(mut self) -> Self {
        self.is_sealed = true;
        self
    }

    #[must_use]
    pub fn with_unsupported_attribute(mut self, attribute: impl Into<String>) -> Self {
        self.unsupported_attributes.push(attribute.into());
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> MethodKind {
        self.kind
    }

    #[must_use]
    pub const fn access(&self) -> Access {
        self.access
    }

    #[must_use]
    pub const fn is_abstract(&self) -> bool {
        self.is_abstract
    }

    #[must_use]
    pub const fn is_sealed(&self) -> bool {
        self.is_sealed
    }

    pub(crate) fn unsupported_attributes(&self) -> &[String] {
        &self.unsupported_attributes
    }
}

/// Identifies where an unsupported class attribute appeared.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttributeOwner {
    Class(String),
    Property(String),
    Method(String),
}

/// One declaration-time enumeration member. Runtime construction will replace
/// the generic constructor arguments with language values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumerationMemberDescriptor<S = ()> {
    name: String,
    constructor_arguments: Vec<S>,
}

impl<S> EnumerationMemberDescriptor<S> {
    #[must_use]
    pub fn new(name: impl Into<String>, constructor_arguments: Vec<S>) -> Self {
        Self {
            name: name.into(),
            constructor_arguments,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn constructor_arguments(&self) -> &[S] {
        &self.constructor_arguments
    }
}

/// Listener and notifier visibility retained independently of listener state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventDescriptor {
    name: String,
    listen_access: Access,
    notify_access: Access,
    hidden: bool,
}

impl EventDescriptor {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            listen_access: Access::Public,
            notify_access: Access::Public,
            hidden: false,
        }
    }

    #[must_use]
    pub const fn with_listen_access(mut self, access: Access) -> Self {
        self.listen_access = access;
        self
    }

    #[must_use]
    pub const fn with_notify_access(mut self, access: Access) -> Self {
        self.notify_access = access;
        self
    }

    #[must_use]
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn listen_access(&self) -> Access {
        self.listen_access
    }

    #[must_use]
    pub const fn notify_access(&self) -> Access {
        self.notify_access
    }

    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }
}

/// An unregistered class definition. Definitions may refer to batch peers by
/// name, which permits deterministic cycle detection before registry mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassDefinition<S = ()> {
    name: String,
    superclass: Option<Superclass>,
    additional_superclasses: Vec<ClassId>,
    declared_semantics: ClassSemantics,
    declared_abstract: Option<bool>,
    sealed: bool,
    enumeration: bool,
    enumeration_base: Option<String>,
    enumeration_members: Vec<EnumerationMemberDescriptor<S>>,
    events: Vec<EventDescriptor>,
    properties: Vec<PropertyDescriptor<S>>,
    methods: Vec<MethodDescriptor>,
    unsupported_attributes: Vec<String>,
}

type ClassDefinitionParts<S> = (
    String,
    ClassSemantics,
    Option<bool>,
    bool,
    bool,
    Option<String>,
    Vec<EnumerationMemberDescriptor<S>>,
    Vec<EventDescriptor>,
    Vec<PropertyDescriptor<S>>,
    Vec<MethodDescriptor>,
);

impl<S> ClassDefinition<S> {
    #[must_use]
    pub fn value(name: impl Into<String>) -> Self {
        Self::new(name, ClassSemantics::Value)
    }

    #[must_use]
    pub fn handle(name: impl Into<String>) -> Self {
        Self::new(name, ClassSemantics::Handle)
    }

    #[must_use]
    pub fn enumeration(name: impl Into<String>) -> Self {
        let mut definition = Self::new(name, ClassSemantics::Value);
        definition.enumeration = true;
        definition
    }

    fn new(name: impl Into<String>, declared_semantics: ClassSemantics) -> Self {
        Self {
            name: name.into(),
            superclass: None,
            additional_superclasses: Vec::new(),
            declared_semantics,
            declared_abstract: None,
            sealed: false,
            enumeration: false,
            enumeration_base: None,
            enumeration_members: Vec::new(),
            events: Vec::new(),
            properties: Vec::new(),
            methods: Vec::new(),
            unsupported_attributes: Vec::new(),
        }
    }

    /// Preflights storage for source-declared members before registration work
    /// begins, so hostile member counts fail as a structured object error.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ObjectError::CapacityExceeded`] if either member list
    /// cannot be represented by the host allocator.
    pub fn try_reserve_members(
        &mut self,
        property_count: usize,
        method_count: usize,
    ) -> crate::ObjectResult<()> {
        self.properties.try_reserve(property_count).map_err(|_| {
            crate::ObjectError::CapacityExceeded {
                resource: "class properties",
            }
        })?;
        self.methods.try_reserve(method_count).map_err(|_| {
            crate::ObjectError::CapacityExceeded {
                resource: "class methods",
            }
        })?;
        Ok(())
    }

    /// Preflights enum-member and event metadata independently of the legacy
    /// property/method reservation API.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ObjectError::CapacityExceeded`] when either feature
    /// list cannot be represented by the host allocator.
    pub fn try_reserve_features(
        &mut self,
        enumeration_count: usize,
        event_count: usize,
    ) -> crate::ObjectResult<()> {
        self.enumeration_members
            .try_reserve(enumeration_count)
            .map_err(|_| crate::ObjectError::CapacityExceeded {
                resource: "enumeration members",
            })?;
        self.events
            .try_reserve(event_count)
            .map_err(|_| crate::ObjectError::CapacityExceeded { resource: "events" })?;
        Ok(())
    }

    #[must_use]
    pub fn with_superclass(mut self, superclass: impl Into<Superclass>) -> Self {
        self.superclass = Some(superclass.into());
        self
    }

    /// Adds another already-registered direct superclass after the primary one.
    ///
    /// The primary superclass remains available through [`Self::with_superclass`]
    /// for source forms that support only one direct base. Additional bases are
    /// ordered and are used by MATLAB's legacy tagged-struct class constructor.
    #[must_use]
    pub fn with_additional_superclass(mut self, superclass: ClassId) -> Self {
        self.additional_superclasses.push(superclass);
        self
    }

    /// Records an explicit MATLAB `Abstract` class attribute value.
    #[must_use]
    pub const fn with_abstract(mut self, value: bool) -> Self {
        self.declared_abstract = Some(value);
        self
    }

    /// Records the MATLAB `Sealed` class attribute.
    #[must_use]
    pub const fn sealed(mut self) -> Self {
        self.sealed = true;
        self
    }

    #[must_use]
    pub fn with_enumeration_base(mut self, base: impl Into<String>) -> Self {
        self.enumeration = true;
        self.enumeration_base = Some(base.into());
        self
    }

    #[must_use]
    pub fn with_enumeration_member(mut self, member: EnumerationMemberDescriptor<S>) -> Self {
        self.enumeration = true;
        self.enumeration_members.push(member);
        self
    }

    #[must_use]
    pub fn with_event(mut self, event: EventDescriptor) -> Self {
        self.events.push(event);
        self
    }

    #[must_use]
    pub fn with_property(mut self, property: PropertyDescriptor<S>) -> Self {
        self.properties.push(property);
        self
    }

    #[must_use]
    pub fn with_method(mut self, method: MethodDescriptor) -> Self {
        self.methods.push(method);
        self
    }

    #[must_use]
    pub fn with_unsupported_attribute(mut self, attribute: impl Into<String>) -> Self {
        self.unsupported_attributes.push(attribute.into());
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn superclass(&self) -> Option<&Superclass> {
        self.superclass.as_ref()
    }

    pub(crate) fn additional_superclasses(&self) -> &[ClassId] {
        &self.additional_superclasses
    }

    pub(crate) const fn declared_semantics(&self) -> ClassSemantics {
        self.declared_semantics
    }

    pub(crate) const fn declared_abstract(&self) -> Option<bool> {
        self.declared_abstract
    }

    pub(crate) const fn is_sealed(&self) -> bool {
        self.sealed
    }

    pub(crate) fn enumeration_base(&self) -> Option<&str> {
        self.enumeration_base.as_deref()
    }

    pub(crate) fn enumeration_members(&self) -> &[EnumerationMemberDescriptor<S>] {
        &self.enumeration_members
    }

    pub(crate) fn events(&self) -> &[EventDescriptor] {
        &self.events
    }

    pub(crate) fn is_enumeration(&self) -> bool {
        self.enumeration
    }

    pub(crate) fn properties(&self) -> &[PropertyDescriptor<S>] {
        &self.properties
    }

    pub(crate) fn methods(&self) -> &[MethodDescriptor] {
        &self.methods
    }

    pub(crate) fn unsupported_attributes(&self) -> &[String] {
        &self.unsupported_attributes
    }

    pub(crate) fn into_parts(self) -> ClassDefinitionParts<S> {
        (
            self.name,
            self.declared_semantics,
            self.declared_abstract,
            self.sealed,
            self.enumeration,
            self.enumeration_base,
            self.enumeration_members,
            self.events,
            self.properties,
            self.methods,
        )
    }
}

/// Validated metadata stored in a [`crate::ClassRegistry`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassDescriptor<S = ()> {
    id: ClassId,
    name: String,
    superclass: Option<ClassId>,
    additional_superclasses: Vec<ClassId>,
    declared_semantics: ClassSemantics,
    effective_semantics: ClassSemantics,
    declared_abstract: Option<bool>,
    effective_abstract: bool,
    sealed: bool,
    enumeration: bool,
    enumeration_base: Option<String>,
    enumeration_members: Vec<EnumerationMemberDescriptor<S>>,
    events: Vec<EventDescriptor>,
    properties: Vec<PropertyDescriptor<S>>,
    methods: Vec<MethodDescriptor>,
}

impl<S> ClassDescriptor<S> {
    pub(crate) fn from_definition(
        id: ClassId,
        definition: ClassDefinition<S>,
        superclass: Option<ClassId>,
        additional_superclasses: Vec<ClassId>,
        effective_semantics: ClassSemantics,
        effective_abstract: bool,
    ) -> Self {
        let (
            name,
            declared_semantics,
            declared_abstract,
            sealed,
            enumeration,
            enumeration_base,
            enumeration_members,
            events,
            properties,
            methods,
        ) = definition.into_parts();
        Self {
            id,
            name,
            superclass,
            additional_superclasses,
            declared_semantics,
            effective_semantics,
            declared_abstract,
            effective_abstract,
            sealed,
            enumeration,
            enumeration_base,
            enumeration_members,
            events,
            properties,
            methods,
        }
    }

    #[must_use]
    pub const fn id(&self) -> ClassId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn superclass(&self) -> Option<ClassId> {
        self.superclass
    }

    /// Returns direct superclasses in declared precedence order.
    #[must_use]
    pub fn superclasses(&self) -> impl DoubleEndedIterator<Item = ClassId> + '_ {
        self.superclass
            .iter()
            .copied()
            .chain(self.additional_superclasses.iter().copied())
    }

    #[must_use]
    pub const fn declared_semantics(&self) -> ClassSemantics {
        self.declared_semantics
    }

    #[must_use]
    pub const fn effective_semantics(&self) -> ClassSemantics {
        self.effective_semantics
    }

    /// Returns the source-declared `Abstract` value, if one was present.
    #[must_use]
    pub const fn declared_abstract(&self) -> Option<bool> {
        self.declared_abstract
    }

    /// Returns MATLAB-compatible effective abstractness after inherited slots
    /// have been linked.
    #[must_use]
    pub const fn is_abstract(&self) -> bool {
        self.effective_abstract
    }

    /// Returns whether this class forbids derivation.
    #[must_use]
    pub const fn is_sealed(&self) -> bool {
        self.sealed
    }

    #[must_use]
    pub fn is_enumeration(&self) -> bool {
        self.enumeration
    }

    #[must_use]
    pub fn enumeration_base(&self) -> Option<&str> {
        self.enumeration_base.as_deref()
    }

    #[must_use]
    pub fn enumeration_members(&self) -> &[EnumerationMemberDescriptor<S>] {
        &self.enumeration_members
    }

    #[must_use]
    pub fn events(&self) -> &[EventDescriptor] {
        &self.events
    }

    #[must_use]
    pub fn properties(&self) -> &[PropertyDescriptor<S>] {
        &self.properties
    }

    #[must_use]
    pub fn methods(&self) -> &[MethodDescriptor] {
        &self.methods
    }

    pub(crate) fn local_property(&self, name: &str) -> Option<&PropertyDescriptor<S>> {
        self.properties
            .iter()
            .find(|property| property.name() == name)
    }

    pub(crate) fn local_method(&self, name: &str) -> Option<&MethodDescriptor> {
        self.methods.iter().find(|method| method.name() == name)
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn validate_definition<S>(definition: &ClassDefinition<S>) -> ObjectResult<()> {
    if definition.name().is_empty() {
        return Err(ObjectError::EmptyName { kind: "class" });
    }
    if let Some(attribute) = definition.unsupported_attributes().first() {
        return Err(ObjectError::UnsupportedAttribute {
            owner: AttributeOwner::Class(definition.name().to_owned()),
            attribute: attribute.clone(),
        });
    }
    if definition.is_enumeration()
        && (definition.declared_abstract() == Some(true)
            || definition
                .properties()
                .iter()
                .any(PropertyDescriptor::is_abstract)
            || definition
                .methods()
                .iter()
                .any(MethodDescriptor::is_abstract))
    {
        return Err(ObjectError::InvalidEnumerationDefinition {
            class_name: definition.name().to_owned(),
            reason: "enumeration classes cannot be abstract",
        });
    }
    if definition.is_enumeration() && definition.enumeration_members().is_empty() {
        return Err(ObjectError::InvalidEnumerationDefinition {
            class_name: definition.name().to_owned(),
            reason: "enumeration classes must declare at least one member",
        });
    }
    if definition.enumeration_base().is_some_and(str::is_empty) {
        return Err(ObjectError::InvalidEnumerationDefinition {
            class_name: definition.name().to_owned(),
            reason: "enumeration base name cannot be empty",
        });
    }

    let mut feature_names = std::collections::BTreeSet::new();
    for member in definition.enumeration_members() {
        if member.name().is_empty() || !feature_names.insert(member.name()) {
            return Err(ObjectError::DuplicateEnumerationMember {
                class_name: definition.name().to_owned(),
                member: member.name().to_owned(),
            });
        }
    }
    for event in definition.events() {
        if event.name().is_empty() || !feature_names.insert(event.name()) {
            return Err(ObjectError::DuplicateEvent {
                class_name: definition.name().to_owned(),
                event: event.name().to_owned(),
            });
        }
    }

    let mut property_names = std::collections::BTreeSet::new();
    for property in definition.properties() {
        if property.name().is_empty() {
            return Err(ObjectError::EmptyName { kind: "property" });
        }
        if !property_names.insert(property.name()) {
            return Err(ObjectError::DuplicateProperty {
                class_name: definition.name().to_owned(),
                property: property.name().to_owned(),
            });
        }
        if feature_names.contains(property.name()) {
            return Err(ObjectError::InvalidPropertyDefinition {
                class_name: definition.name().to_owned(),
                property: property.name().to_owned(),
                reason: "property name conflicts with an enumeration member or event",
            });
        }
        if property.is_abstract() && property.default_value().is_some() {
            return Err(ObjectError::InvalidPropertyDefinition {
                class_name: definition.name().to_owned(),
                property: property.name().to_owned(),
                reason: "abstract property cannot carry a default value",
            });
        }
        let valid_access = match property.kind() {
            PropertyKind::Stored => {
                property.get_access().is_some() && property.set_access().is_some()
            }
            PropertyKind::Constant => {
                property.get_access().is_some() && property.set_access().is_none()
            }
            PropertyKind::Dependent => {
                property.get_access().is_some() || property.set_access().is_some()
            }
        };
        if !valid_access {
            return Err(ObjectError::InvalidPropertyDefinition {
                class_name: definition.name().to_owned(),
                property: property.name().to_owned(),
                reason: "property kind and readable/writable access metadata disagree",
            });
        }
        if let Some(attribute) = property.unsupported_attributes().first() {
            return Err(ObjectError::UnsupportedAttribute {
                owner: AttributeOwner::Property(property.name().to_owned()),
                attribute: attribute.clone(),
            });
        }
    }

    let mut method_names = std::collections::BTreeSet::new();
    for method in definition.methods() {
        if method.name().is_empty() {
            return Err(ObjectError::EmptyName { kind: "method" });
        }
        if !method_names.insert(method.name()) {
            return Err(ObjectError::DuplicateMethod {
                class_name: definition.name().to_owned(),
                method: method.name().to_owned(),
            });
        }
        if feature_names.contains(method.name()) {
            return Err(ObjectError::DuplicateMethod {
                class_name: definition.name().to_owned(),
                method: method.name().to_owned(),
            });
        }
        if let Some(attribute) = method.unsupported_attributes().first() {
            return Err(ObjectError::UnsupportedAttribute {
                owner: AttributeOwner::Method(method.name().to_owned()),
                attribute: attribute.clone(),
            });
        }
    }
    Ok(())
}

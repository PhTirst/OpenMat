use std::{error::Error, fmt};

use crate::{Access, AttributeOwner, ClassId, ClassSemantics, HandleState, MethodKind, ObjectId};

pub type ObjectResult<T> = Result<T, ObjectError>;

/// Structured failures produced by class validation, dispatch, and storage.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ObjectError {
    EmptyName {
        kind: &'static str,
    },
    DuplicateClass {
        name: String,
    },
    DuplicateSuperclass {
        class_name: String,
        superclass: ClassId,
    },
    DuplicateProperty {
        class_name: String,
        property: String,
    },
    DuplicateMethod {
        class_name: String,
        method: String,
    },
    InvalidPropertyDefinition {
        class_name: String,
        property: String,
        reason: &'static str,
    },
    InheritedPropertyConflict {
        class_name: String,
        property: String,
        inherited_from: ClassId,
    },
    OverrideKindMismatch {
        class_name: String,
        method: String,
        inherited_kind: MethodKind,
        overriding_kind: MethodKind,
    },
    AbstractOverrideAccessMismatch {
        class_name: String,
        method: String,
        inherited_access: Access,
        overriding_access: Access,
    },
    AbstractPropertyOverrideAccessMismatch {
        class_name: String,
        property: String,
    },
    SealedMethodOverride {
        class_name: String,
        method: String,
    },
    SealedSuperclass {
        class_name: String,
        superclass: ClassId,
    },
    ExplicitConcreteWithAbstractMethods {
        class_name: String,
        methods: Vec<String>,
    },
    InvalidEnumerationDefinition {
        class_name: String,
        reason: &'static str,
    },
    DuplicateEnumerationMember {
        class_name: String,
        member: String,
    },
    DuplicateEvent {
        class_name: String,
        event: String,
    },
    EventNotFound {
        class_id: ClassId,
        event: String,
    },
    InvalidConstructorKind {
        class_name: String,
    },
    UnsupportedAttribute {
        owner: AttributeOwner,
        attribute: String,
    },
    UnknownClass {
        id: ClassId,
    },
    UnknownClassName {
        name: String,
    },
    InheritanceCycle {
        class_id: ClassId,
    },
    IdExhausted {
        kind: &'static str,
    },
    CapacityExceeded {
        resource: &'static str,
    },
    MethodNotFound {
        class_id: ClassId,
        method: String,
    },
    MethodKindMismatch {
        method: String,
        expected: MethodKind,
        actual: MethodKind,
    },
    AbstractMethodCall {
        class_name: String,
        method: String,
    },
    AbstractClassInstantiation {
        class_name: String,
    },
    EnumerationDirectConstruction {
        class_name: String,
    },
    PropertyNotFound {
        class_id: ClassId,
        property: String,
    },
    PropertyReadOnly {
        class_id: ClassId,
        property: String,
    },
    PropertyWriteOnly {
        class_id: ClassId,
        property: String,
    },
    MissingAccessor {
        class_id: ClassId,
        property: String,
        accessor: String,
    },
    AccessDenied {
        member: String,
        declaring_class: ClassId,
        required: Access,
        caller: Option<ClassId>,
    },
    UnknownObject {
        id: ObjectId,
    },
    ObjectReferenceMismatch {
        id: ObjectId,
    },
    ObjectClassMismatch {
        id: ObjectId,
        expected: ClassId,
        actual: ClassId,
    },
    ObjectSemanticsMismatch {
        id: ObjectId,
        expected: ClassSemantics,
        actual: ClassSemantics,
    },
    HandleNotAlive {
        id: ObjectId,
        state: HandleState,
    },
    FinalizationCandidateMismatch {
        id: ObjectId,
    },
    InvalidConstructionTransition {
        id: ObjectId,
        from: crate::ConstructionState,
        to: crate::ConstructionState,
    },
    ObjectNotConstructed {
        id: ObjectId,
        state: crate::ConstructionState,
    },
    MissingPropertyInitializer {
        class_id: ClassId,
        property: String,
    },
    PropertySlotNotFound {
        id: ObjectId,
        property: String,
    },
    ShapeOverflow,
    ArrayElementCountMismatch {
        expected: u64,
        actual: usize,
    },
    HeterogeneousArrayElement {
        index: usize,
        expected: ClassId,
        actual: ClassId,
    },
    ValueIdentityAliased {
        id: ObjectId,
    },
}

impl fmt::Display for ObjectError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName { kind } => write!(formatter, "{kind} name cannot be empty"),
            Self::DuplicateClass { name } => write!(formatter, "class `{name}` is already defined"),
            Self::DuplicateSuperclass {
                class_name,
                superclass,
            } => write!(
                formatter,
                "class `{class_name}` names {superclass} as a direct superclass more than once"
            ),
            Self::DuplicateProperty {
                class_name,
                property,
            } => write!(
                formatter,
                "class `{class_name}` declares property `{property}` more than once"
            ),
            Self::DuplicateMethod { class_name, method } => write!(
                formatter,
                "class `{class_name}` declares method `{method}` more than once"
            ),
            Self::InvalidPropertyDefinition {
                class_name,
                property,
                reason,
            } => write!(
                formatter,
                "invalid property `{property}` on class `{class_name}`: {reason}"
            ),
            Self::InheritedPropertyConflict {
                class_name,
                property,
                inherited_from,
            } => write!(
                formatter,
                "class `{class_name}` redeclares inherited property `{property}` from {inherited_from}"
            ),
            Self::OverrideKindMismatch {
                class_name,
                method,
                inherited_kind,
                overriding_kind,
            } => write!(
                formatter,
                "class `{class_name}` changes method `{method}` from {inherited_kind:?} to {overriding_kind:?}"
            ),
            Self::AbstractOverrideAccessMismatch {
                class_name,
                method,
                inherited_access,
                overriding_access,
            } => write!(
                formatter,
                "class `{class_name}` implements abstract method `{method}` with {overriding_access:?} access instead of {inherited_access:?}"
            ),
            Self::AbstractPropertyOverrideAccessMismatch {
                class_name,
                property,
            } => write!(
                formatter,
                "class `{class_name}` implements abstract property `{property}` with incompatible access"
            ),
            Self::SealedMethodOverride { class_name, method } => write!(
                formatter,
                "class `{class_name}` overrides sealed method `{method}`"
            ),
            Self::SealedSuperclass {
                class_name,
                superclass,
            } => write!(
                formatter,
                "class `{class_name}` cannot derive from sealed {superclass}"
            ),
            Self::ExplicitConcreteWithAbstractMethods {
                class_name,
                methods,
            } => write!(
                formatter,
                "class `{class_name}` explicitly disables `Abstract` but leaves abstract methods unresolved: {}",
                methods.join(", ")
            ),
            Self::InvalidEnumerationDefinition { class_name, reason } => write!(
                formatter,
                "invalid enumeration class `{class_name}`: {reason}"
            ),
            Self::DuplicateEnumerationMember { class_name, member } => write!(
                formatter,
                "enumeration class `{class_name}` declares member `{member}` more than once"
            ),
            Self::DuplicateEvent { class_name, event } => write!(
                formatter,
                "class `{class_name}` declares event `{event}` more than once"
            ),
            Self::EventNotFound { class_id, event } => {
                write!(formatter, "event `{event}` was not found on {class_id}")
            }
            Self::InvalidConstructorKind { class_name } => {
                write!(
                    formatter,
                    "constructor `{class_name}` must be an instance method"
                )
            }
            Self::CapacityExceeded { resource } => {
                write!(formatter, "{resource} exceed host capacity")
            }
            Self::UnsupportedAttribute { owner, attribute } => {
                write!(
                    formatter,
                    "unsupported attribute `{attribute}` on {owner:?}"
                )
            }
            Self::UnknownClass { id } => write!(formatter, "unknown {id}"),
            Self::UnknownClassName { name } => write!(formatter, "unknown class `{name}`"),
            Self::InheritanceCycle { class_id } => {
                write!(formatter, "inheritance cycle containing {class_id}")
            }
            Self::IdExhausted { kind } => write!(formatter, "{kind} identity space exhausted"),
            Self::MethodNotFound { class_id, method } => {
                write!(formatter, "method `{method}` was not found on {class_id}")
            }
            Self::MethodKindMismatch {
                method,
                expected,
                actual,
            } => write!(
                formatter,
                "method `{method}` is {actual:?}, but {expected:?} dispatch was requested"
            ),
            Self::AbstractMethodCall { class_name, method } => write!(
                formatter,
                "abstract method `{class_name}.{method}` has no executable body"
            ),
            Self::AbstractClassInstantiation { class_name } => {
                write!(
                    formatter,
                    "abstract class `{class_name}` cannot be instantiated"
                )
            }
            Self::EnumerationDirectConstruction { class_name } => write!(
                formatter,
                "enumeration class `{class_name}` can only be constructed through a declared member"
            ),
            Self::PropertyNotFound { class_id, property } => {
                write!(
                    formatter,
                    "property `{property}` was not found on {class_id}"
                )
            }
            Self::PropertyReadOnly { class_id, property } => {
                write!(
                    formatter,
                    "property `{property}` on {class_id} is read-only"
                )
            }
            Self::PropertyWriteOnly { class_id, property } => {
                write!(
                    formatter,
                    "property `{property}` on {class_id} is write-only"
                )
            }
            Self::MissingAccessor {
                class_id,
                property,
                accessor,
            } => write!(
                formatter,
                "dependent property `{property}` on {class_id} has no `{accessor}` accessor"
            ),
            Self::AccessDenied {
                member,
                declaring_class,
                required,
                caller,
            } => write!(
                formatter,
                "access to `{member}` declared by {declaring_class} requires {required:?}; caller is {caller:?}"
            ),
            Self::UnknownObject { id } => write!(formatter, "unknown {id}"),
            Self::ObjectReferenceMismatch { id } => {
                write!(formatter, "reference metadata does not match stored {id}")
            }
            Self::ObjectClassMismatch {
                id,
                expected,
                actual,
            } => write!(
                formatter,
                "{id} has class {actual}, but class {expected} was required"
            ),
            Self::ObjectSemanticsMismatch {
                id,
                expected,
                actual,
            } => write!(
                formatter,
                "{id} has {actual:?} semantics, but {expected:?} semantics were required"
            ),
            Self::HandleNotAlive { id, state } => {
                write!(formatter, "{id} is an invalid handle in state {state:?}")
            }
            Self::FinalizationCandidateMismatch { id } => {
                write!(formatter, "finalization capability does not match {id}")
            }
            Self::InvalidConstructionTransition { id, from, to } => write!(
                formatter,
                "invalid construction transition for {id}: {from:?} to {to:?}"
            ),
            Self::ObjectNotConstructed { id, state } => {
                write!(formatter, "{id} is {state:?}, not fully constructed")
            }
            Self::MissingPropertyInitializer { class_id, property } => write!(
                formatter,
                "stored property `{property}` on {class_id} has no initializer"
            ),
            Self::PropertySlotNotFound { id, property } => {
                write!(
                    formatter,
                    "{id} has no stored slot for property `{property}`"
                )
            }
            Self::ShapeOverflow => write!(formatter, "object-array shape product overflowed"),
            Self::ArrayElementCountMismatch { expected, actual } => write!(
                formatter,
                "object-array shape requires {expected} elements, but received {actual}"
            ),
            Self::HeterogeneousArrayElement {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "object-array element {index} has class {actual}, expected exact class {expected}"
            ),
            Self::ValueIdentityAliased { id } => {
                write!(
                    formatter,
                    "value-class array aliases {id} in multiple elements"
                )
            }
        }
    }
}

impl Error for ObjectError {}

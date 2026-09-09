#![doc = "Core `classdef` metadata, object identity, and dispatch semantics."]

mod descriptor;
mod error;
mod id;
mod object;
mod operator;
mod registry;

pub use descriptor::{
    Access, AttributeOwner, ClassDefinition, ClassDescriptor, ClassSemantics,
    EnumerationMemberDescriptor, EventDescriptor, MethodDescriptor, MethodKind, PropertyDescriptor,
    PropertyKind, Superclass,
};
pub use error::{ObjectError, ObjectResult};
pub use id::{ClassId, ObjectId, PropertyKey};
pub use object::{
    ConstructionState, FinalizationCandidate, GcCollection, GcMetrics, GcPlan, HandleState,
    ObjectArrayDescriptor, ObjectRef, ObjectStore,
};
pub use operator::Operator;
pub use registry::{
    AccessContext, ClassRegistry, EventAccess, EventSelection, MethodSelection, PropertyAccess,
    PropertyResolution,
};

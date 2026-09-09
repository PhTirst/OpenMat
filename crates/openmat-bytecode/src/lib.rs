#![doc = "Versioned register-oriented bytecode definitions and validation."]

mod model;
mod verify;

pub use model::{
    AbstractPropertyDefinition, Access, ApplyArgument, ArgumentLayout, AssignmentMode,
    BinaryOperator, BindingTarget, BytecodeModule, BytecodeVersion, CURRENT_BYTECODE_VERSION,
    ClassDefinition, ClassDefinitionId, ClassFeatureDefinition, ClassKind, ClassSemantics,
    Constant, ConstantId, EnumMemberDefinition, EventDefinition, ExceptionHandler,
    ExceptionHandlerKind, FieldOperand, Function, FunctionId, Instruction, InstructionIndex,
    InstructionKind, LocalSlot, MIN_SUPPORTED_BYTECODE_VERSION, MethodDefinition, MethodKind,
    NamedBindingKind, PackApplyTarget, PackRegister, PersistentSlot, PlaceStep, PropertyDefinition,
    PropertyKind, Register, SharedCaptureSource, SourceLocation, StatementResultTarget,
    ValueSource,
};
pub use verify::{VerificationError, VerificationErrorKind, verify};

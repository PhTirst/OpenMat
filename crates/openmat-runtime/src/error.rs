use std::{error::Error, fmt};

use openmat_bytecode::{
    BinaryOperator, FunctionId, InstructionIndex, SourceLocation, VerificationError,
};
use openmat_object::Access;
use openmat_value::{FunctionHandle, ValueKind};

use crate::{BuiltinErrorCategory, linalg_ops::RuntimeLinalgError};

/// The structured reason an indexing argument was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexErrorKind {
    /// At least one subscript is required.
    NoSubscripts,
    /// The argument is not numeric or logical.
    NonNumeric,
    /// A numeric index is NaN or infinite.
    NonFinite,
    /// A numeric index is not an integer.
    NonInteger,
    /// MATLAB indices must be positive and one-based.
    NonPositive,
    /// A logical mask length does not match the indexed extent.
    LogicalLength { expected: u64, actual: u64 },
}

/// The structured reason a colon range could not be materialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeErrorKind {
    /// Each operand must be a real numeric scalar.
    NonScalarReal,
    /// Range operands must be finite.
    NonFinite,
    /// The step cannot be zero.
    ZeroStep,
    /// The result cannot be represented by the host or array shape model.
    TooLarge,
}

/// Structured array, range, indexing, concatenation, and numerical failure detail.
///
/// This detail is nested under the existing stable
/// [`RuntimeErrorKind::InvalidExecutionState`] category so higher-level crates
/// compiled against the original exhaustive top-level category set remain
/// source-compatible while callers can still inspect precise array failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArrayRuntimeError {
    /// A dynamic-value linear-algebra bridge or provider operation failed.
    ///
    /// This retains the original provider-independent or provider-owned error
    /// fields instead of requiring callers to recover them from display text.
    LinearAlgebra { error: RuntimeLinalgError },
    /// Array operands have incompatible shapes for the operation.
    ShapeMismatch {
        operation: &'static str,
        lhs: Vec<u64>,
        rhs: Vec<u64>,
    },
    /// An operation supports arrays but not the supplied value category.
    InvalidOperand {
        operation: &'static str,
        actual: ValueKind,
    },
    /// Matrix literal concatenation found incompatible row or column extents.
    ConcatenationMismatch {
        axis: usize,
        expected: u64,
        actual: u64,
    },
    /// A range expression used invalid scalar operands.
    InvalidRange { reason: RangeErrorKind },
    /// An indexing argument is structurally invalid.
    InvalidIndex {
        argument: usize,
        reason: IndexErrorKind,
    },
    /// A one-based index is outside the effective indexed extent.
    IndexOutOfBounds {
        argument: usize,
        index: u64,
        extent: u64,
    },
    /// Indexed assignment cannot expand the right-hand value to the selection.
    AssignmentSizeMismatch { selected: u64, supplied: u64 },
    /// Object-array linear growth skipped elements whose values cannot be
    /// synthesized without invoking class-specific construction semantics.
    ObjectArrayGrowthGap { current: u64, requested: u64 },
    /// Parenthesized indexing cannot produce the requested number of outputs.
    IndexOutputArity { requested: usize },
    /// A colon descriptor was supplied to a callable instead of an array.
    ColonCallArgument { argument: usize },
    /// A checked host or array shape boundary was exceeded.
    SizeLimit,
    /// Cell-literal rows have different widths after pack expansion.
    RaggedCellRows {
        /// Zero-based source row containing the mismatch.
        row: usize,
        /// Width established by the first row.
        expected: usize,
        /// Width produced by this row.
        actual: usize,
    },
    /// A dynamic or static struct field name is not accepted by the aggregate contract.
    InvalidStructFieldName { name: String },
    /// A requested field is absent from a struct schema.
    MissingStructField { name: String },
    /// Struct assignment requires equal field sets.
    StructSchemaMismatch {
        /// Destination field names in observable order.
        destination: Vec<String>,
        /// Source field names in observable order.
        source: Vec<String>,
    },
    /// Aggregate deletion is not a whole supported dimension or linear-vector deletion.
    InvalidDeletionShape {
        /// Shape of the aggregate before deletion.
        dimensions: Vec<u64>,
        /// Number of supplied indexing arguments.
        arguments: usize,
    },
    /// Struct member application is currently defined only for scalar receivers.
    StructMemberApplyRequiresScalar { numel: u64 },
}

/// Read-only details for a one-based index that exceeded its effective extent.
///
/// Obtain this from [`RuntimeError::index_out_of_bounds`] or
/// [`RuntimeErrorKind::index_out_of_bounds`]. The helper keeps the existing
/// top-level error enum source-compatible while allowing kernel and protocol
/// adapters to assign a stable category without parsing diagnostic text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndexOutOfBoundsDetails {
    /// Zero-based position of the indexing argument.
    pub argument: usize,
    /// Rejected one-based language index.
    pub index: u64,
    /// Effective extent for this indexing argument.
    pub extent: u64,
}

/// A validated module could not be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBuildError {
    /// Bytecode verifier result.
    pub verification: VerificationError,
}

impl fmt::Display for RuntimeBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid bytecode: {}", self.verification)
    }
}

impl Error for RuntimeBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.verification)
    }
}

/// One execution frame captured in a runtime error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackTraceFrame {
    /// Bytecode function table index.
    pub function: FunctionId,
    /// Diagnostic function name.
    pub name: String,
    /// Current instruction index.
    pub instruction: InstructionIndex,
    /// Current source provenance.
    pub location: Option<SourceLocation>,
}

/// A classdef constraint preserved independently of object-error display text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassConstraintKind {
    /// Construction targeted a declared or effectively abstract class.
    AbstractClassInstantiation,
    /// A class named a sealed direct superclass.
    SealedSuperclass,
    /// A class explicitly disabled `Abstract` while retaining abstract slots.
    ExplicitConcreteWithAbstractMethods,
    /// A concrete override changed the access of an inherited abstract slot.
    AbstractOverrideAccessMismatch,
    /// Dispatch reached an abstract method that has no executable body.
    AbstractMethodUnavailable,
}

/// A structured runtime failure category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeErrorKind {
    /// Cooperative cancellation was observed.
    Cancelled,
    /// A workspace name and built-in name were both absent.
    UndefinedGlobal { name: String },
    /// A value used as a callee was not callable.
    NotCallable { actual: ValueKind },
    /// A callable handle cannot be resolved in the current module or registry.
    InvalidFunctionHandle { handle: FunctionHandle },
    /// A named function-handle target was absent from the linked module and registry.
    UnknownFunctionHandleTarget { name: String },
    /// A bytecode function received the wrong number of input arguments.
    InputArity {
        function: String,
        expected: usize,
        actual: usize,
    },
    /// A call requested more outputs than the callee returned.
    MissingOutputs { requested: usize, returned: usize },
    /// Scalar operands do not support the requested operator.
    InvalidOperands {
        operator: BinaryOperator,
        lhs: ValueKind,
        rhs: ValueKind,
    },
    /// A scalar bytecode operator received a non-scalar array and therefore
    /// cannot choose matrix versus element-wise language semantics.
    UnsupportedArrayOperator { operator: BinaryOperator },
    /// A value cannot be used as a scalar control-flow condition.
    InvalidCondition { actual: ValueKind },
    /// A registered built-in returned a structured failure.
    Builtin {
        name: String,
        category: BuiltinErrorCategory,
        /// Exact language identifier requested by the built-in, when any.
        identifier: Option<String>,
        message: String,
    },
    /// Class registration, object storage, or member dispatch failed.
    Object {
        operation: &'static str,
        message: String,
    },
    /// Class linking, instantiation, or abstract dispatch violated a classdef constraint.
    ClassConstraint {
        operation: &'static str,
        constraint: ClassConstraintKind,
        message: String,
    },
    /// A class member was resolved, but its declared access level rejects the caller.
    AccessViolation {
        operation: &'static str,
        member: String,
        required: Access,
    },
    /// A parsed class feature is intentionally deferred to a later tranche.
    UnsupportedClassFeature { feature: String },
    /// The configured recursion limit was reached.
    CallDepthExceeded { maximum: usize },
    /// Execution reached outside a function's instruction stream.
    InstructionPointerOutOfBounds {
        function: FunctionId,
        instruction: InstructionIndex,
    },
    /// A verifier invariant or operation-specific runtime boundary failed.
    InvalidExecutionState {
        message: String,
        /// Precise array failure detail, when this came from array semantics.
        array: Option<Box<ArrayRuntimeError>>,
    },
}

/// A runtime failure with source and complete bytecode call-stack context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    /// Structured failure category.
    pub kind: RuntimeErrorKind,
    /// Source location of the failing instruction.
    pub location: Option<SourceLocation>,
    /// Call stack ordered from entry frame to failing frame.
    pub stack: Vec<StackTraceFrame>,
}

impl RuntimeError {
    /// Classifies and returns structured indexing-bound details, if present.
    #[must_use]
    pub fn index_out_of_bounds(&self) -> Option<IndexOutOfBoundsDetails> {
        self.kind.index_out_of_bounds()
    }

    /// Returns the structured linear-algebra bridge failure, if present.
    #[must_use]
    pub fn linalg_error(&self) -> Option<&RuntimeLinalgError> {
        self.kind.linalg_error()
    }
}

impl RuntimeErrorKind {
    /// Returns the stable `OpenMat` identifier used by a language-level caught
    /// exception, or `None` when this category is outside the catchable
    /// runtime boundary.
    ///
    /// Keeping this conversion exhaustive makes catchability independent of
    /// display text. Verifier/internal failures and cancellation deliberately
    /// never become language exception objects.
    pub(crate) fn catchable_identifier(&self) -> Option<&str> {
        match self {
            Self::Cancelled
            | Self::InstructionPointerOutOfBounds { .. }
            | Self::InvalidExecutionState { array: None, .. } => None,
            Self::UndefinedGlobal { .. } => Some("OpenMat:UndefinedName"),
            Self::NotCallable { .. } => Some("OpenMat:NotCallable"),
            Self::InvalidFunctionHandle { .. } => Some("OpenMat:InvalidFunctionHandle"),
            Self::UnknownFunctionHandleTarget { .. } => Some("OpenMat:UnknownFunctionHandleTarget"),
            Self::InputArity { .. } => Some("OpenMat:InputArity"),
            Self::MissingOutputs { .. } => Some("OpenMat:MissingOutputs"),
            Self::InvalidOperands { .. } | Self::UnsupportedArrayOperator { .. } => {
                Some("OpenMat:InvalidOperands")
            }
            Self::InvalidCondition { .. } => Some("OpenMat:InvalidCondition"),
            Self::Builtin { identifier, .. } => {
                Some(identifier.as_deref().unwrap_or("OpenMat:BuiltinError"))
            }
            Self::Object { .. } => Some("OpenMat:ObjectError"),
            Self::ClassConstraint { constraint, .. } => Some(match constraint {
                ClassConstraintKind::AbstractMethodUnavailable => "OpenMat:UndefinedName",
                ClassConstraintKind::AbstractClassInstantiation
                | ClassConstraintKind::SealedSuperclass
                | ClassConstraintKind::ExplicitConcreteWithAbstractMethods
                | ClassConstraintKind::AbstractOverrideAccessMismatch => "OpenMat:ClassConstraint",
            }),
            Self::AccessViolation { .. } => Some("OpenMat:AccessViolation"),
            Self::UnsupportedClassFeature { .. } => Some("OpenMat:UnsupportedClassFeature"),
            Self::CallDepthExceeded { .. } => Some("OpenMat:CallDepthExceeded"),
            Self::InvalidExecutionState {
                array: Some(array), ..
            } => Some(array.catchable_identifier()),
        }
    }

    /// Classifies a nested array failure as index-out-of-bounds without
    /// inspecting its display message.
    #[must_use]
    pub fn index_out_of_bounds(&self) -> Option<IndexOutOfBoundsDetails> {
        let Self::InvalidExecutionState {
            array: Some(array), ..
        } = self
        else {
            return None;
        };
        let ArrayRuntimeError::IndexOutOfBounds {
            argument,
            index,
            extent,
        } = array.as_ref()
        else {
            return None;
        };
        Some(IndexOutOfBoundsDetails {
            argument: *argument,
            index: *index,
            extent: *extent,
        })
    }

    /// Returns the structured linear-algebra bridge failure, if present.
    #[must_use]
    pub fn linalg_error(&self) -> Option<&RuntimeLinalgError> {
        let Self::InvalidExecutionState {
            array: Some(array), ..
        } = self
        else {
            return None;
        };
        let ArrayRuntimeError::LinearAlgebra { error } = array.as_ref() else {
            return None;
        };
        Some(error)
    }
}

impl ArrayRuntimeError {
    const fn catchable_identifier(&self) -> &'static str {
        match self {
            Self::LinearAlgebra { .. } => "OpenMat:LinearAlgebraError",
            Self::ShapeMismatch { .. } => "OpenMat:ArrayShapeMismatch",
            Self::InvalidOperand { .. } => "OpenMat:ArrayInvalidOperand",
            Self::ConcatenationMismatch { .. } => "OpenMat:ArrayConcatenationMismatch",
            Self::InvalidRange { .. } => "OpenMat:InvalidRange",
            Self::InvalidIndex { .. } => "OpenMat:InvalidIndex",
            Self::IndexOutOfBounds { .. } => "OpenMat:IndexOutOfBounds",
            Self::AssignmentSizeMismatch { .. } => "OpenMat:AssignmentSizeMismatch",
            Self::ObjectArrayGrowthGap { .. } => "OpenMat:ObjectArrayGrowthGap",
            Self::IndexOutputArity { .. } => "OpenMat:IndexOutputArity",
            Self::ColonCallArgument { .. } => "OpenMat:ColonCallArgument",
            Self::SizeLimit => "OpenMat:ArraySizeLimit",
            Self::RaggedCellRows { .. } => "OpenMat:RaggedCellRows",
            Self::InvalidStructFieldName { .. } => "OpenMat:InvalidStructFieldName",
            Self::MissingStructField { .. } => "OpenMat:MissingStructField",
            Self::StructSchemaMismatch { .. } => "OpenMat:StructSchemaMismatch",
            Self::InvalidDeletionShape { .. } => "OpenMat:InvalidDeletionShape",
            Self::StructMemberApplyRequiresScalar { .. } => {
                "OpenMat:StructMemberApplyRequiresScalar"
            }
        }
    }
}

impl fmt::Display for RuntimeError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            RuntimeErrorKind::Cancelled => formatter.write_str("execution was cancelled"),
            RuntimeErrorKind::UndefinedGlobal { name } => {
                write!(formatter, "workspace name `{name}` is undefined")
            }
            RuntimeErrorKind::NotCallable { actual } => {
                write!(formatter, "a value of class `{actual}` is not callable")
            }
            RuntimeErrorKind::InvalidFunctionHandle { handle } => {
                write!(
                    formatter,
                    "function handle {handle:?} is not valid in this runtime"
                )
            }
            RuntimeErrorKind::UnknownFunctionHandleTarget { name } => {
                write!(formatter, "function-handle target `{name}` is unknown")
            }
            RuntimeErrorKind::InputArity {
                function,
                expected,
                actual,
            } => write!(
                formatter,
                "function `{function}` expects {expected} inputs but received {actual}"
            ),
            RuntimeErrorKind::MissingOutputs {
                requested,
                returned,
            } => write!(
                formatter,
                "call requested {requested} outputs but the callee returned {returned}"
            ),
            RuntimeErrorKind::InvalidOperands { operator, lhs, rhs } => write!(
                formatter,
                "operator `{operator}` does not support `{lhs}` and `{rhs}` operands"
            ),
            RuntimeErrorKind::UnsupportedArrayOperator { operator } => write!(
                formatter,
                "scalar bytecode operator `{operator}` does not encode non-scalar array semantics"
            ),
            RuntimeErrorKind::InvalidCondition { actual } => write!(
                formatter,
                "a scalar value of class `{actual}` cannot be used as a condition"
            ),
            RuntimeErrorKind::Builtin {
                name,
                category,
                identifier: _,
                message,
            } => write!(
                formatter,
                "built-in `{name}` failed in category {category:?}: {message}"
            ),
            RuntimeErrorKind::Object { operation, message } => {
                write!(formatter, "object {operation} failed: {message}")
            }
            RuntimeErrorKind::ClassConstraint {
                operation,
                constraint,
                message,
            } => write!(
                formatter,
                "classdef {operation} failed ({constraint:?}): {message}"
            ),
            RuntimeErrorKind::AccessViolation {
                operation,
                member,
                required,
            } => write!(
                formatter,
                "{operation} denied for {required:?} member `{member}`"
            ),
            RuntimeErrorKind::UnsupportedClassFeature { feature } => {
                write!(formatter, "unsupported class feature: {feature}")
            }
            RuntimeErrorKind::CallDepthExceeded { maximum } => {
                write!(formatter, "maximum call depth {maximum} was exceeded")
            }
            RuntimeErrorKind::InstructionPointerOutOfBounds {
                function,
                instruction,
            } => write!(
                formatter,
                "function {} reached invalid instruction {}",
                function.get(),
                instruction.get()
            ),
            RuntimeErrorKind::InvalidExecutionState {
                message,
                array: Some(array),
            } => write!(formatter, "{message}: {array}"),
            RuntimeErrorKind::InvalidExecutionState {
                message,
                array: None,
            } => formatter.write_str(message),
        }
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.linalg_error()
            .map(|error| error as &(dyn Error + 'static))
    }
}

impl fmt::Display for ArrayRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LinearAlgebra { error } => write!(formatter, "linear algebra failed: {error}"),
            Self::ShapeMismatch {
                operation,
                lhs,
                rhs,
            } => write!(
                formatter,
                "array shapes {lhs:?} and {rhs:?} are incompatible for {operation}"
            ),
            Self::InvalidOperand { operation, actual } => write!(
                formatter,
                "{operation} does not support a value of class `{actual}`"
            ),
            Self::ConcatenationMismatch {
                axis,
                expected,
                actual,
            } => write!(
                formatter,
                "matrix concatenation axis {axis} expected extent {expected} but found {actual}"
            ),
            Self::InvalidRange { reason } => write!(formatter, "invalid colon range: {reason:?}"),
            Self::InvalidIndex { argument, reason } => {
                write!(formatter, "invalid index argument {argument}: {reason:?}")
            }
            Self::IndexOutOfBounds {
                argument,
                index,
                extent,
            } => write!(
                formatter,
                "index {index} in argument {argument} exceeds extent {extent}"
            ),
            Self::AssignmentSizeMismatch { selected, supplied } => write!(
                formatter,
                "indexed assignment selects {selected} elements but supplies {supplied}"
            ),
            Self::ObjectArrayGrowthGap { current, requested } => write!(
                formatter,
                "object-array growth from {current} elements to index {requested} would leave uninitialized elements"
            ),
            Self::IndexOutputArity { requested } => {
                write!(
                    formatter,
                    "indexing cannot produce {requested} output values"
                )
            }
            Self::ColonCallArgument { argument } => write!(
                formatter,
                "colon index argument {argument} cannot be passed to a function call"
            ),
            Self::SizeLimit => {
                formatter.write_str("array result exceeds the checked size boundary")
            }
            Self::RaggedCellRows {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "cell row {row} has width {actual} after pack expansion, expected {expected}"
            ),
            Self::InvalidStructFieldName { name } => {
                write!(formatter, "invalid struct field name {name:?}")
            }
            Self::MissingStructField { name } => {
                write!(formatter, "struct field {name:?} does not exist")
            }
            Self::StructSchemaMismatch {
                destination,
                source,
            } => write!(
                formatter,
                "struct assignment field sets differ: destination {destination:?}, source {source:?}"
            ),
            Self::InvalidDeletionShape {
                dimensions,
                arguments,
            } => write!(
                formatter,
                "aggregate deletion with {arguments} subscripts is invalid for shape {dimensions:?}"
            ),
            Self::StructMemberApplyRequiresScalar { numel } => write!(
                formatter,
                "struct member application requires a scalar receiver, found {numel} records"
            ),
        }
    }
}

pub(crate) fn array_error(array: ArrayRuntimeError) -> RuntimeErrorKind {
    RuntimeErrorKind::InvalidExecutionState {
        message: "array operation failed".to_owned(),
        array: Some(Box::new(array)),
    }
}

impl From<RuntimeLinalgError> for RuntimeErrorKind {
    fn from(error: RuntimeLinalgError) -> Self {
        if matches!(
            error,
            RuntimeLinalgError::Linalg(crate::LinalgError::Cancelled { .. })
        ) {
            Self::Cancelled
        } else {
            array_error(ArrayRuntimeError::LinearAlgebra { error })
        }
    }
}

/// Result type for interpreter execution.
pub type RuntimeResult<T> = Result<T, RuntimeError>;

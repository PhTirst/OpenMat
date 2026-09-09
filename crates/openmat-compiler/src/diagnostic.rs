use std::{error::Error, fmt};

use openmat_bytecode::VerificationError;
use openmat_hir::{DeclarationContext, DeclarationForm, DeclarationKind};
use openmat_source::{Diagnostic, SourceId, TextRange};

/// Result returned by checked HIR-to-bytecode compilation.
pub type CompileResult<T = openmat_bytecode::BytecodeModule> = Result<T, CompileError>;

/// A compilation failure containing every diagnostic found in deterministic order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileError {
    diagnostics: Vec<CompileDiagnostic>,
}

impl CompileError {
    pub(crate) fn new(diagnostics: Vec<CompileDiagnostic>) -> Self {
        Self { diagnostics }
    }

    /// Returns the diagnostics that prevented bytecode emission.
    #[must_use]
    pub fn diagnostics(&self) -> &[CompileDiagnostic] {
        &self.diagnostics
    }

    /// Consumes the error and returns its diagnostics.
    #[must_use]
    pub fn into_diagnostics(self) -> Vec<CompileDiagnostic> {
        self.diagnostics
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.diagnostics.as_slice() {
            [] => formatter.write_str("compilation failed without a diagnostic"),
            [diagnostic] => write!(formatter, "compilation failed: {diagnostic}"),
            [first, rest @ ..] => write!(
                formatter,
                "compilation failed with {} diagnostics; first: {first}",
                rest.len() + 1
            ),
        }
    }
}

impl Error for CompileError {}

/// A structured compiler diagnostic tied to one source byte range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileDiagnostic {
    /// Source buffer containing the diagnosed HIR.
    pub source_id: SourceId,
    /// Half-open UTF-8 byte range of the diagnosed HIR node.
    pub range: TextRange,
    /// Structured failure category.
    pub kind: CompileDiagnosticKind,
}

impl CompileDiagnostic {
    pub(crate) const fn new(
        source_id: SourceId,
        range: TextRange,
        kind: CompileDiagnosticKind,
    ) -> Self {
        Self {
            source_id,
            range,
            kind,
        }
    }

    /// Returns a stable compiler diagnostic code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self.kind {
            CompileDiagnosticKind::MalformedHir { .. } => "OMC0001",
            CompileDiagnosticKind::DuplicateFunction { .. } => "OMC0002",
            CompileDiagnosticKind::DuplicateParameter { .. } => "OMC0003",
            CompileDiagnosticKind::DuplicateOutput { .. } => "OMC0004",
            CompileDiagnosticKind::InvalidNumericLiteral { .. } => "OMC0005",
            CompileDiagnosticKind::InvalidStringLiteral { .. } => "OMC0006",
            CompileDiagnosticKind::Unsupported { .. } => "OMC0007",
            CompileDiagnosticKind::LoopControlOutsideLoop { .. } => "OMC0008",
            CompileDiagnosticKind::ResourceLimit { .. } => "OMC0009",
            CompileDiagnosticKind::InvalidClassMember { .. } => "OMC0010",
            CompileDiagnosticKind::InvalidBindingDeclaration { .. } => "OMC0011",
            CompileDiagnosticKind::UnsupportedBindingOperation { .. } => "OMC0012",
            CompileDiagnosticKind::InvalidClassAttribute { .. } => "OMC0013",
            CompileDiagnosticKind::Internal { .. } => "OMC9999",
        }
    }

    /// Converts this compiler-specific diagnostic to the shared source API.
    #[must_use]
    pub fn to_source_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.source_id, self.range, self.to_string()).with_code(self.code())
    }
}

impl fmt::Display for CompileDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: ", self.code())?;
        match &self.kind {
            CompileDiagnosticKind::MalformedHir { detail } => {
                write!(formatter, "malformed HIR: {detail}")
            }
            CompileDiagnosticKind::DuplicateFunction { name } => {
                write!(formatter, "duplicate top-level function `{name}`")
            }
            CompileDiagnosticKind::DuplicateParameter { name } => {
                write!(formatter, "duplicate function parameter `{name}`")
            }
            CompileDiagnosticKind::DuplicateOutput { name } => {
                write!(formatter, "duplicate function output `{name}`")
            }
            CompileDiagnosticKind::InvalidNumericLiteral { literal } => {
                write!(formatter, "invalid numeric literal `{literal}`")
            }
            CompileDiagnosticKind::InvalidStringLiteral { literal } => {
                write!(formatter, "invalid quoted string literal `{literal}`")
            }
            CompileDiagnosticKind::Unsupported { feature } => {
                write!(
                    formatter,
                    "unsupported in the first bytecode compiler: {feature}"
                )
            }
            CompileDiagnosticKind::LoopControlOutsideLoop { keyword } => {
                write!(formatter, "`{keyword}` appears outside a loop")
            }
            CompileDiagnosticKind::ResourceLimit { resource } => {
                write!(
                    formatter,
                    "{resource} exceeds the bytecode u32 format boundary"
                )
            }
            CompileDiagnosticKind::InvalidClassMember {
                class,
                member,
                problem,
            } => write!(
                formatter,
                "invalid class member `{class}.{member}`: {problem}"
            ),
            CompileDiagnosticKind::InvalidBindingDeclaration {
                kind,
                name,
                problem,
            } => {
                let declaration = declaration_name(*kind);
                if let Some(name) = name {
                    write!(
                        formatter,
                        "invalid `{declaration}` binding declaration for `{name}`: {problem}"
                    )
                } else {
                    write!(
                        formatter,
                        "invalid `{declaration}` binding declaration: {problem}"
                    )
                }
            }
            CompileDiagnosticKind::UnsupportedBindingOperation {
                name,
                storage,
                operation,
            } => write!(
                formatter,
                "binding `{name}` uses {storage}, which cannot be used for {operation}"
            ),
            CompileDiagnosticKind::InvalidClassAttribute {
                class,
                attribute,
                problem,
            } => write!(
                formatter,
                "invalid class attribute `{class}.{attribute}`: {problem}"
            ),
            CompileDiagnosticKind::Internal { error } => {
                write!(formatter, "internal compiler error: {error}")
            }
        }
    }
}

/// Structured reasons why compilation failed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompileDiagnosticKind {
    /// The HIR contains a missing, empty, or explicit error node.
    MalformedHir { detail: String },
    /// More than one top-level function has the same name.
    DuplicateFunction { name: String },
    /// A function declares the same input more than once.
    DuplicateParameter { name: String },
    /// A function declares the same output more than once.
    DuplicateOutput { name: String },
    /// A numeric HIR token cannot be represented by a bytecode scalar constant.
    InvalidNumericLiteral { literal: String },
    /// A quoted HIR token is missing matching delimiters.
    InvalidStringLiteral { literal: String },
    /// The HIR is valid but needs bytecode/runtime support not present yet.
    Unsupported { feature: UnsupportedFeature },
    /// `break` or `continue` does not have an enclosing loop.
    LoopControlOutsideLoop { keyword: &'static str },
    /// A table or frame size cannot be represented by the bytecode format.
    ResourceLimit { resource: CompilerResource },
    /// A valid class member declaration violates an executable class contract.
    InvalidClassMember {
        class: String,
        member: String,
        problem: ClassMemberProblem,
    },
    /// A `global` or `persistent` declaration violates the static binding rules.
    InvalidBindingDeclaration {
        kind: DeclarationKind,
        name: Option<String>,
        problem: BindingDeclarationProblem,
    },
    /// A binding was resolved explicitly, but the bytecode has no safe form for
    /// the requested storage/operation combination.
    UnsupportedBindingOperation {
        name: String,
        storage: BindingStorageClass,
        operation: BindingOperation,
    },
    /// A recognized class attribute has an invalid value or is repeated.
    InvalidClassAttribute {
        class: String,
        attribute: String,
        problem: ClassAttributeProblem,
    },
    /// Compiler-produced bytecode or patch state violated an internal invariant.
    Internal { error: InternalCompilerError },
}

/// Static reasons why a `global` or `persistent` declaration is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BindingDeclarationProblem {
    InvalidForm(DeclarationForm),
    InvalidContext(DeclarationContext),
    EmptyName,
    DuplicatePersistent,
    ConflictsWithGlobal,
    ConflictsWithPersistent,
    ConflictsWithInput,
    ConflictsWithOutput,
    AssignedBeforeDeclaration,
    UsedBeforeDeclaration,
}

impl fmt::Display for BindingDeclarationProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidForm(form) => write!(formatter, "invalid declaration form {form:?}"),
            Self::InvalidContext(context) => {
                write!(formatter, "not valid in {context:?} lexical context")
            }
            Self::EmptyName => formatter.write_str("the declaration name is empty"),
            Self::DuplicatePersistent => {
                formatter.write_str("the persistent name is declared more than once")
            }
            Self::ConflictsWithGlobal => formatter.write_str("the name is also declared global"),
            Self::ConflictsWithPersistent => {
                formatter.write_str("the name is also declared persistent")
            }
            Self::ConflictsWithInput => {
                formatter.write_str("a persistent name cannot be a function input")
            }
            Self::ConflictsWithOutput => {
                formatter.write_str("a persistent name cannot be a function output")
            }
            Self::AssignedBeforeDeclaration => {
                formatter.write_str("the persistent name is assigned before its declaration")
            }
            Self::UsedBeforeDeclaration => {
                formatter.write_str("the persistent name is used before its declaration")
            }
        }
    }
}

/// Compiler-visible storage classes used in binding diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BindingStorageClass {
    Local,
    WorkspaceGlobal,
    Persistent,
}

impl fmt::Display for BindingStorageClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Local => "local storage",
            Self::WorkspaceGlobal => "workspace-global storage",
            Self::Persistent => "persistent storage",
        })
    }
}

/// Operations whose bytecode support depends on binding storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BindingOperation {
    ConstructorOutput,
    NamedClear,
    StatementResult,
}

impl fmt::Display for BindingOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConstructorOutput => "a class-constructor output object",
            Self::NamedClear => "named `clear`",
            Self::StatementResult => "the implicit statement result `ans`",
        })
    }
}

const fn declaration_name(kind: DeclarationKind) -> &'static str {
    match kind {
        DeclarationKind::Global => "global",
        DeclarationKind::Persistent => "persistent",
    }
}

/// Structured semantic failures for executable class properties and methods.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ClassMemberProblem {
    DuplicateOrConflictingDeclaration {
        previous_kind: &'static str,
    },
    ConflictingPropertyKinds,
    AbstractPropertyInitializer,
    EmptyEnumeration,
    EnumerationSuperclass {
        superclass: String,
    },
    DependentPropertyInitializer,
    MissingDependentAccessors,
    AccessorTargetNotDependent,
    MethodMustBeInstance,
    ConcreteMethodInAbstractBlock,
    AbstractConstructor,
    EventsRequireHandleClass,
    InvalidSignature {
        expected_inputs: usize,
        actual_inputs: usize,
        expected_outputs: usize,
        actual_outputs: usize,
    },
}

impl fmt::Display for ClassMemberProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateOrConflictingDeclaration { previous_kind } => write!(
                formatter,
                "the name is already declared as a {previous_kind} in this class"
            ),
            Self::ConflictingPropertyKinds => {
                formatter.write_str("`Constant` and `Dependent` cannot be combined")
            }
            Self::AbstractPropertyInitializer => {
                formatter.write_str("an abstract property cannot declare an initializer")
            }
            Self::EmptyEnumeration => {
                formatter.write_str("an enumeration block must declare at least one member")
            }
            Self::EnumerationSuperclass { superclass } => write!(
                formatter,
                "class cannot derive from enumeration superclass `{superclass}`"
            ),
            Self::DependentPropertyInitializer => {
                formatter.write_str("a dependent property cannot declare a stored initializer")
            }
            Self::MissingDependentAccessors => formatter
                .write_str("a dependent property must declare a conventional getter or setter"),
            Self::AccessorTargetNotDependent => {
                formatter.write_str("the accessor does not name a dependent property")
            }
            Self::MethodMustBeInstance => {
                formatter.write_str("the accessor or operator must be an instance method")
            }
            Self::ConcreteMethodInAbstractBlock => formatter
                .write_str("an abstract methods block may contain only body-less signatures"),
            Self::AbstractConstructor => {
                formatter.write_str("a constructor cannot be declared abstract")
            }
            Self::EventsRequireHandleClass => {
                formatter.write_str("events require handle-class semantics")
            }
            Self::InvalidSignature {
                expected_inputs,
                actual_inputs,
                expected_outputs,
                actual_outputs,
            } => write!(
                formatter,
                "expected {expected_inputs} inputs and {expected_outputs} outputs, found {actual_inputs} inputs and {actual_outputs} outputs"
            ),
        }
    }
}

/// Structured problems for recognized MATLAB class attributes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ClassAttributeProblem {
    Duplicate,
    InvalidBooleanValue,
    ExplicitConcreteWithAbstractMethods,
    EnumerationCannotBeAbstract,
}

impl fmt::Display for ClassAttributeProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Duplicate => "the attribute is specified more than once",
            Self::InvalidBooleanValue => {
                "the attribute value must be the logical name `true` or `false`"
            }
            Self::ExplicitConcreteWithAbstractMethods => {
                "`Abstract = false` conflicts with declared abstract methods"
            }
            Self::EnumerationCannotBeAbstract => "an enumeration class cannot be declared abstract",
        })
    }
}

/// Valid HIR constructs deliberately rejected by the first compiler version.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum UnsupportedFeature {
    MatrixLiteral,
    CellLiteral,
    AllIndex,
    EndOutsideIndex,
    EndInMemberApply,
    NestedPlaceEnd,
    UnequalCellSourceRows,
    PackInSuperclassConstructor,
    MultipleAggregateAssignmentTargets,
    AggregateAssignmentOutputArity,
    DynamicApplyOrIndex,
    FieldAccess,
    FunctionHandle,
    RangeExpression,
    Transpose,
    ForLoop,
    SwitchStatement,
    ClassDefinition,
    ClassAttribute(String),
    PropertyAttribute(String),
    MethodAttribute(String),
    EnumerationAttribute(String),
    EventAttribute(String),
    UserSuperclass(String),
    OperatorOverload(String),
    DottedAccessor(String),
    NestedFunction,
    ClearArguments,
    ClearImportScope,
    AssignmentTarget,
    MultipleAssignmentValue,
    Operator(String),
}

impl fmt::Display for UnsupportedFeature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::MatrixLiteral => "matrix literals",
            Self::CellLiteral => "cell literals",
            Self::AllIndex => "the all-elements index `:`",
            Self::EndOutsideIndex => "`end` outside an indexing context",
            Self::EndInMemberApply => {
                "`end` in a static member application whose property target is runtime-resolved"
            }
            Self::NestedPlaceEnd => {
                "`end` bound to a non-root step of an aggregate assignment place"
            }
            Self::UnequalCellSourceRows => "cell literal rows with unequal encoded source widths",
            Self::PackInSuperclassConstructor => {
                "comma-separated-list expansion in an explicit superclass constructor call"
            }
            Self::MultipleAggregateAssignmentTargets => {
                "multiple distinct aggregate places in one bracketed assignment target"
            }
            Self::AggregateAssignmentOutputArity => {
                "a bracketed aggregate assignment whose output count is not statically known"
            }
            Self::DynamicApplyOrIndex => "dynamic parenthesized apply/index",
            Self::FieldAccess => "field access",
            Self::FunctionHandle => "function handles",
            Self::RangeExpression => "range expressions",
            Self::Transpose => "transpose expressions",
            Self::ForLoop => "for loops",
            Self::SwitchStatement => "switch statements",
            Self::ClassDefinition => "classdef",
            Self::ClassAttribute(attribute) => {
                return write!(formatter, "class attribute `{attribute}`");
            }
            Self::PropertyAttribute(attribute) => {
                return write!(formatter, "property attribute `{attribute}`");
            }
            Self::MethodAttribute(attribute) => {
                return write!(formatter, "method attribute `{attribute}`");
            }
            Self::EnumerationAttribute(attribute) => {
                return write!(formatter, "enumeration attribute `{attribute}`");
            }
            Self::EventAttribute(attribute) => {
                return write!(formatter, "event attribute `{attribute}`");
            }
            Self::UserSuperclass(superclass) => {
                return write!(formatter, "user superclass `{superclass}`");
            }
            Self::OperatorOverload(method) => {
                return write!(formatter, "operator overload method `{method}`");
            }
            Self::DottedAccessor(method) => {
                return write!(formatter, "dependent property accessor `{method}`");
            }
            Self::NestedFunction => "nested function definitions",
            Self::ClearArguments => "`clear` options, patterns, or function-form arguments",
            Self::ClearImportScope => "`clear import` outside the base command workspace",
            Self::AssignmentTarget => "this assignment target",
            Self::MultipleAssignmentValue => "a non-call multiple-assignment value",
            Self::Operator(operator) => return write!(formatter, "operator `{operator}`"),
        };
        formatter.write_str(description)
    }
}

/// Bytecode tables whose indices are restricted to `u32`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompilerResource {
    Functions,
    Classes,
    Registers,
    PackRegisters,
    Locals,
    PersistentSlots,
    Parameters,
    Constants,
    Instructions,
}

impl fmt::Display for CompilerResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Functions => "function table",
            Self::Classes => "class-definition table",
            Self::Registers => "register file",
            Self::PackRegisters => "pack-register file",
            Self::Locals => "local-slot table",
            Self::PersistentSlots => "persistent-slot table",
            Self::Parameters => "parameter list",
            Self::Constants => "constant pool",
            Self::Instructions => "instruction stream",
        };
        formatter.write_str(name)
    }
}

/// Failures that indicate a compiler implementation bug rather than bad input.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InternalCompilerError {
    BytecodeVerification(VerificationError),
    InvalidJumpPatch,
    InvalidAnonymousFunctionReservation,
}

impl fmt::Display for InternalCompilerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BytecodeVerification(error) => {
                write!(formatter, "bytecode verification failed: {error}")
            }
            Self::InvalidJumpPatch => {
                formatter.write_str("a jump patch did not refer to a jump instruction")
            }
            Self::InvalidAnonymousFunctionReservation => {
                formatter.write_str("a reserved anonymous-function slot disappeared")
            }
        }
    }
}

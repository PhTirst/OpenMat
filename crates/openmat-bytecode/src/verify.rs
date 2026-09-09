use std::{collections::BTreeSet, error::Error, fmt};

use crate::{
    ApplyArgument, AssignmentMode, BindingTarget, BytecodeModule, BytecodeVersion,
    CURRENT_BYTECODE_VERSION, ClassDefinitionId, ClassKind, Constant, ConstantId, ExceptionHandler,
    ExceptionHandlerKind, FieldOperand, Function, FunctionId, Instruction, InstructionIndex,
    InstructionKind, LocalSlot, MIN_SUPPORTED_BYTECODE_VERSION, NamedBindingKind, PackApplyTarget,
    PackRegister, PersistentSlot, PlaceStep, Register, SharedCaptureSource, SourceLocation,
    StatementResultTarget, ValueSource,
};

/// A precise reason that bytecode validation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationErrorKind {
    /// Argument binding metadata requires v28 and a valid, fully covered signature.
    InvalidArgumentLayout,
    /// The module uses an unsupported format version.
    UnsupportedVersion {
        found: BytecodeVersion,
        supported: BytecodeVersion,
    },
    /// An older module carries an import or generalized package instruction
    /// introduced by the current bytecode version.
    PackageInstructionsRequireCurrentVersion { found: BytecodeVersion },
    /// Version 24 cannot carry mutable base-workspace import state.
    ImportClearRequiresCurrentVersion { found: BytecodeVersion },
    /// Versions before 26 cannot preserve argument-sensitive named calls.
    CallTargetRequiresCurrentVersion { found: BytecodeVersion },
    /// Versions before 27 cannot carry dynamic-output or checked-read instructions.
    DynamicOutputsRequireCurrentVersion { found: BytecodeVersion },
    /// Version 24 cannot defer a class method body to an `@Class` file.
    ExternalMethodRequiresCurrentVersion {
        found: BytecodeVersion,
        class: String,
        method: String,
    },
    /// One method cannot be both abstract and externally implemented.
    ExternalMethodIsAbstract { class: String, method: String },
    /// The module's entry function does not exist.
    InvalidEntry { entry: FunctionId },
    /// A function has more parameters than local slots.
    ParametersExceedLocals { parameters: u32, locals: u32 },
    /// A variadic-input marker appears anywhere other than instruction zero.
    VariadicInputsMustBeFirst,
    /// A function contains more than one variadic-input marker.
    DuplicateVariadicInputs,
    /// A register operand is outside the frame's register file.
    InvalidRegister { register: Register, count: u32 },
    /// A pack-register operand is outside the frame's pack-register file.
    InvalidPackRegister { register: PackRegister, count: u32 },
    /// A local operand is outside the frame's local-slot array.
    InvalidLocal { local: LocalSlot, count: u32 },
    /// A persistent operand is outside the function-owned persistent-slot array.
    InvalidPersistentSlot { slot: PersistentSlot, count: u32 },
    /// A constant operand is outside the function's constant pool.
    InvalidConstant { constant: ConstantId, count: usize },
    /// A global-name operand does not refer to a string constant.
    GlobalNameIsNotString { constant: ConstantId },
    /// A static aggregate-field operand does not refer to a string constant.
    FieldNameIsNotString { constant: ConstantId },
    /// Encoded cell-construction rows do not all have the same width.
    RaggedCellRows {
        row: usize,
        expected: usize,
        actual: usize,
    },
    /// An aggregate assignment carries no selector steps.
    EmptyPlacePath,
    /// Aggregate deletion attempts to use an expanded value pack as its token.
    DeleteSourceMustBeOne,
    /// An `end` resolver declares zero indexing arguments.
    InvalidEndArgumentCount,
    /// An `end` resolver selects an argument outside its declared argument count.
    InvalidEndArgumentIndex {
        argument_index: u32,
        argument_count: u32,
    },
    /// A clear-workspace instruction has no encoded names.
    EmptyClearNames,
    /// A function constant does not refer to a module function.
    InvalidFunctionConstant { function: FunctionId },
    /// A closure constructor does not refer to a module function.
    InvalidClosureFunction { function: FunctionId },
    /// A closure constructor encodes the same free-variable name twice.
    DuplicateClosureCapture { name: String },
    /// A class-table operand is outside the module class table.
    InvalidClassDefinition {
        class: ClassDefinitionId,
        count: usize,
    },
    /// A class or class member has an empty language-visible name.
    EmptyClassName { member: Option<String> },
    /// A class has an empty direct-superclass name.
    EmptySuperclassName { class: String },
    /// A class directly names itself as its superclass.
    SelfSuperclass { class: String },
    /// A property default or method does not refer to a module function.
    InvalidClassFunction {
        class: String,
        member: String,
        function: FunctionId,
    },
    /// A property default function has an invalid input signature.
    InvalidPropertyDefaultSignature {
        class: String,
        property: String,
        parameters: u32,
    },
    /// A constant property has no initializer function.
    MissingConstantInitializer { class: String, property: String },
    /// A property kind disagrees with its encoded readable/writable access.
    InvalidPropertyWriteAccess {
        class: String,
        property: String,
        kind: crate::PropertyKind,
    },
    /// A dependent property incorrectly carries a stored initializer.
    DependentPropertyHasInitializer { class: String, property: String },
    /// A readable or writable dependent property has no encoded accessor method.
    MissingDependentAccessor {
        class: String,
        property: String,
        accessor: String,
    },
    /// A dotted accessor method does not match dependent-property metadata.
    InvalidDependentAccessorTarget { class: String, accessor: String },
    /// A dependent accessor or supported operator has an invalid invocation kind.
    SpecialMethodMustBeInstance { class: String, method: String },
    /// A dependent accessor or supported operator has an invalid input/output signature.
    InvalidSpecialMethodSignature {
        class: String,
        method: String,
        expected_parameters: u32,
        actual_parameters: u32,
        expected_outputs: u32,
        actual_outputs: Option<u32>,
    },
    /// A static method carries constructor-only receiver seeding metadata.
    StaticMethodHasConstructorOutput { class: String, method: String },
    /// An abstract method carries constructor-only receiver seeding metadata.
    AbstractMethodHasConstructorOutput { class: String, method: String },
    /// An abstract method uses the class name reserved for constructors.
    AbstractConstructor { class: String },
    /// A class explicitly encoded `Abstract = false` while declaring slots.
    ExplicitConcreteHasAbstractMethod { class: String, method: String },
    /// Enum, event, abstract-property, or sealed-method metadata is malformed.
    InvalidClassFeature {
        class: String,
        feature: String,
        reason: &'static str,
    },
    /// A constructor seed slot is outside its function's locals.
    InvalidConstructorOutput {
        class: String,
        method: String,
        local: LocalSlot,
        count: u32,
    },
    /// A superclass-constructor instruction is not owned by exactly one constructor.
    SuperclassConstructorOutsideConstructor { superclass: String },
    /// A superclass-constructor instruction does not target its owning class's direct base.
    InvalidSuperclassConstructorTarget {
        class: String,
        expected: Option<String>,
        actual: String,
    },
    /// A superclass-constructor instruction does not use the constructor output slot.
    InvalidSuperclassConstructorObject {
        class: String,
        expected: LocalSlot,
        actual: LocalSlot,
    },
    /// A jump target is outside the instruction stream.
    InvalidJump {
        target: InstructionIndex,
        count: usize,
    },
    /// A protected interval is empty or has an endpoint outside the instruction stream.
    InvalidExceptionProtectedRange {
        start: InstructionIndex,
        end: InstructionIndex,
        count: usize,
    },
    /// A catch entry point is outside the instruction stream.
    InvalidExceptionHandler {
        handler: InstructionIndex,
        count: usize,
    },
    /// A try continuation is outside the instruction stream.
    InvalidExceptionExit {
        exit: InstructionIndex,
        count: usize,
    },
    /// A catch entry or continuation does not follow the protected body in
    /// the representable structured layout.
    InvalidExceptionCatchOrder {
        protected_end: InstructionIndex,
        handler: InstructionIndex,
        exit: InstructionIndex,
    },
    /// A catch-bearing protected body is not followed by `Jump exit`.
    InvalidExceptionCatchSkip {
        instruction: InstructionIndex,
        exit: InstructionIndex,
    },
    /// A no-catch handler does not continue at the end of its protected body.
    InvalidExceptionSwallowExit {
        protected_end: InstructionIndex,
        exit: InstructionIndex,
    },
    /// Two exception records protect the exact same interval, making nearest
    /// handler selection ambiguous.
    DuplicateExceptionRange {
        start: InstructionIndex,
        end: InstructionIndex,
    },
    /// Two protected intervals overlap without one containing the other.
    CrossingExceptionRanges {
        first_start: InstructionIndex,
        first_end: InstructionIndex,
        second_start: InstructionIndex,
        second_end: InstructionIndex,
    },
    /// A nested handler's catch/continuation region escapes the enclosing
    /// protected interval.
    NestedExceptionHandlerEscapes {
        outer_start: InstructionIndex,
        outer_end: InstructionIndex,
        inner_start: InstructionIndex,
        inner_end: InstructionIndex,
        inner_exit: InstructionIndex,
    },
}

/// A bytecode validation error with function, instruction, and source context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationError {
    /// Function containing the invalid operand, when applicable.
    pub function: Option<FunctionId>,
    /// Instruction containing the invalid operand, when applicable.
    pub instruction: Option<InstructionIndex>,
    /// Source provenance copied from the invalid instruction or exception record.
    pub location: Option<SourceLocation>,
    /// Validation failure category.
    pub kind: VerificationErrorKind,
}

impl fmt::Display for VerificationError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(function) = self.function {
            write!(formatter, "function {}", function.get())?;
            if let Some(instruction) = self.instruction {
                write!(formatter, ", instruction {}", instruction.get())?;
            }
            formatter.write_str(": ")?;
        }

        match &self.kind {
            VerificationErrorKind::UnsupportedVersion { found, supported } => {
                write!(
                    formatter,
                    "unsupported bytecode version {found}; supported version is {supported}"
                )
            }
            VerificationErrorKind::PackageInstructionsRequireCurrentVersion { found } => write!(
                formatter,
                "bytecode version {found} cannot carry version-24 import and package-member instructions"
            ),
            VerificationErrorKind::ImportClearRequiresCurrentVersion { found } => write!(
                formatter,
                "bytecode version {found} cannot carry version-25 mutable-import instructions"
            ),
            VerificationErrorKind::InvalidArgumentLayout => {
                write!(formatter, "invalid or unsupported argument binding layout")
            }
            VerificationErrorKind::DynamicOutputsRequireCurrentVersion { found } => write!(
                formatter,
                "bytecode version {found} cannot carry version-27 dynamic-output instructions"
            ),
            VerificationErrorKind::CallTargetRequiresCurrentVersion { found } => write!(
                formatter,
                "bytecode version {found} cannot carry version-26 named-call target instructions"
            ),
            VerificationErrorKind::ExternalMethodRequiresCurrentVersion {
                found,
                class,
                method,
            } => write!(
                formatter,
                "bytecode version {found} cannot declare external method `{class}.{method}`"
            ),
            VerificationErrorKind::ExternalMethodIsAbstract { class, method } => write!(
                formatter,
                "class `{class}` method `{method}` cannot be both abstract and external"
            ),
            VerificationErrorKind::InvalidEntry { entry } => {
                write!(formatter, "entry function {} does not exist", entry.get())
            }
            VerificationErrorKind::ParametersExceedLocals { parameters, locals } => write!(
                formatter,
                "parameter count {parameters} exceeds local count {locals}"
            ),
            VerificationErrorKind::VariadicInputsMustBeFirst => {
                formatter.write_str("variadic-input loading must be the first instruction")
            }
            VerificationErrorKind::DuplicateVariadicInputs => {
                formatter.write_str("function contains more than one variadic-input load")
            }
            VerificationErrorKind::InvalidRegister { register, count } => write!(
                formatter,
                "register {} is outside register file of size {count}",
                register.get()
            ),
            VerificationErrorKind::InvalidPackRegister { register, count } => write!(
                formatter,
                "pack register {} is outside pack-register file of size {count}",
                register.get()
            ),
            VerificationErrorKind::InvalidLocal { local, count } => write!(
                formatter,
                "local {} is outside local array of size {count}",
                local.get()
            ),
            VerificationErrorKind::InvalidPersistentSlot { slot, count } => write!(
                formatter,
                "persistent slot {} is outside persistent-slot array of size {count}",
                slot.get()
            ),
            VerificationErrorKind::InvalidConstant { constant, count } => write!(
                formatter,
                "constant {} is outside constant pool of size {count}",
                constant.get()
            ),
            VerificationErrorKind::GlobalNameIsNotString { constant } => write!(
                formatter,
                "constant {} used as a global name is not a string",
                constant.get()
            ),
            VerificationErrorKind::FieldNameIsNotString { constant } => write!(
                formatter,
                "constant {} used as an aggregate field name is not a string",
                constant.get()
            ),
            VerificationErrorKind::RaggedCellRows {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "cell row {row} has width {actual}, but encoded row width is {expected}"
            ),
            VerificationErrorKind::EmptyPlacePath => {
                formatter.write_str("aggregate assignment path has no selector steps")
            }
            VerificationErrorKind::DeleteSourceMustBeOne => {
                formatter.write_str("aggregate deletion source must be one ordinary value register")
            }
            VerificationErrorKind::InvalidEndArgumentCount => {
                formatter.write_str("end resolver has zero indexing arguments")
            }
            VerificationErrorKind::InvalidEndArgumentIndex {
                argument_index,
                argument_count,
            } => write!(
                formatter,
                "end resolver argument {argument_index} is outside argument count {argument_count}"
            ),
            VerificationErrorKind::EmptyClearNames => {
                formatter.write_str("clear-workspace instruction has no names")
            }
            VerificationErrorKind::InvalidFunctionConstant { function } => write!(
                formatter,
                "function constant {} does not exist in the module",
                function.get()
            ),
            VerificationErrorKind::InvalidClosureFunction { function } => write!(
                formatter,
                "closure function {} does not exist in the module",
                function.get()
            ),
            VerificationErrorKind::DuplicateClosureCapture { name } => {
                write!(formatter, "closure captures `{name}` more than once")
            }
            kind @ (VerificationErrorKind::InvalidClassDefinition { .. }
            | VerificationErrorKind::EmptyClassName { .. }
            | VerificationErrorKind::EmptySuperclassName { .. }
            | VerificationErrorKind::SelfSuperclass { .. }
            | VerificationErrorKind::InvalidClassFunction { .. }
            | VerificationErrorKind::InvalidPropertyDefaultSignature { .. }
            | VerificationErrorKind::MissingConstantInitializer { .. }
            | VerificationErrorKind::InvalidPropertyWriteAccess { .. }
            | VerificationErrorKind::DependentPropertyHasInitializer { .. }
            | VerificationErrorKind::MissingDependentAccessor { .. }
            | VerificationErrorKind::InvalidDependentAccessorTarget { .. }
            | VerificationErrorKind::SpecialMethodMustBeInstance { .. }
            | VerificationErrorKind::InvalidSpecialMethodSignature { .. }
            | VerificationErrorKind::StaticMethodHasConstructorOutput { .. }
            | VerificationErrorKind::AbstractMethodHasConstructorOutput { .. }
            | VerificationErrorKind::AbstractConstructor { .. }
            | VerificationErrorKind::ExplicitConcreteHasAbstractMethod { .. }
            | VerificationErrorKind::InvalidClassFeature { .. }
            | VerificationErrorKind::InvalidConstructorOutput { .. }
            | VerificationErrorKind::SuperclassConstructorOutsideConstructor { .. }
            | VerificationErrorKind::InvalidSuperclassConstructorTarget { .. }
            | VerificationErrorKind::InvalidSuperclassConstructorObject { .. }) => {
                format_class_error(kind, formatter)
            }
            VerificationErrorKind::InvalidJump { target, count } => write!(
                formatter,
                "jump target {} is outside instruction stream of size {count}",
                target.get()
            ),
            VerificationErrorKind::InvalidExceptionProtectedRange { start, end, count } => write!(
                formatter,
                "exception protected range {}..{} is empty or outside instruction stream of size {count}",
                start.get(),
                end.get()
            ),
            VerificationErrorKind::InvalidExceptionHandler { handler, count } => write!(
                formatter,
                "exception handler target {} is outside instruction stream of size {count}",
                handler.get()
            ),
            VerificationErrorKind::InvalidExceptionExit { exit, count } => write!(
                formatter,
                "exception exit target {} is outside instruction stream of size {count}",
                exit.get()
            ),
            VerificationErrorKind::InvalidExceptionCatchOrder {
                protected_end,
                handler,
                exit,
            } => write!(
                formatter,
                "catch target {} must immediately follow protected-end instruction {}, and exit {} must not precede the catch target",
                handler.get(),
                protected_end.get(),
                exit.get()
            ),
            VerificationErrorKind::InvalidExceptionCatchSkip { instruction, exit } => write!(
                formatter,
                "instruction {} after the protected body must jump to exception exit {}",
                instruction.get(),
                exit.get()
            ),
            VerificationErrorKind::InvalidExceptionSwallowExit {
                protected_end,
                exit,
            } => write!(
                formatter,
                "no-catch exception exit {} must equal protected end {}",
                exit.get(),
                protected_end.get()
            ),
            VerificationErrorKind::DuplicateExceptionRange { start, end } => write!(
                formatter,
                "exception protected range {}..{} has more than one handler",
                start.get(),
                end.get()
            ),
            VerificationErrorKind::CrossingExceptionRanges {
                first_start,
                first_end,
                second_start,
                second_end,
            } => write!(
                formatter,
                "exception protected ranges {}..{} and {}..{} cross instead of nesting",
                first_start.get(),
                first_end.get(),
                second_start.get(),
                second_end.get()
            ),
            VerificationErrorKind::NestedExceptionHandlerEscapes {
                outer_start,
                outer_end,
                inner_start,
                inner_end,
                inner_exit,
            } => write!(
                formatter,
                "nested exception region {}..{} exits at {}, outside enclosing protected range {}..{}",
                inner_start.get(),
                inner_end.get(),
                inner_exit.get(),
                outer_start.get(),
                outer_end.get()
            ),
        }
    }
}

#[allow(clippy::too_many_lines)]
fn format_class_error(
    kind: &VerificationErrorKind,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match kind {
        VerificationErrorKind::InvalidClassDefinition { class, count } => write!(
            formatter,
            "class definition {} is outside class table of size {count}",
            class.get()
        ),
        VerificationErrorKind::EmptyClassName { member: None } => {
            formatter.write_str("class definition has an empty name")
        }
        VerificationErrorKind::EmptyClassName {
            member: Some(member),
        } => write!(formatter, "class member `{member}` has an empty name"),
        VerificationErrorKind::EmptySuperclassName { class } => {
            write!(formatter, "class `{class}` has an empty superclass name")
        }
        VerificationErrorKind::SelfSuperclass { class } => {
            write!(formatter, "class `{class}` names itself as its superclass")
        }
        VerificationErrorKind::InvalidClassFunction {
            class,
            member,
            function,
        } => write!(
            formatter,
            "class `{class}` member `{member}` refers to missing function {}",
            function.get()
        ),
        VerificationErrorKind::InvalidPropertyDefaultSignature {
            class,
            property,
            parameters,
        } => write!(
            formatter,
            "class `{class}` property `{property}` default function requires {parameters} inputs"
        ),
        VerificationErrorKind::MissingConstantInitializer { class, property } => write!(
            formatter,
            "class `{class}` constant property `{property}` has no initializer"
        ),
        VerificationErrorKind::InvalidPropertyWriteAccess {
            class,
            property,
            kind,
        } => write!(
            formatter,
            "class `{class}` property `{property}` has write access inconsistent with {kind:?} storage"
        ),
        special @ (VerificationErrorKind::DependentPropertyHasInitializer { .. }
        | VerificationErrorKind::MissingDependentAccessor { .. }
        | VerificationErrorKind::InvalidDependentAccessorTarget { .. }
        | VerificationErrorKind::SpecialMethodMustBeInstance { .. }
        | VerificationErrorKind::InvalidSpecialMethodSignature { .. }) => {
            format_special_class_error(special, formatter)
        }
        constraint @ (VerificationErrorKind::StaticMethodHasConstructorOutput { .. }
        | VerificationErrorKind::AbstractMethodHasConstructorOutput { .. }
        | VerificationErrorKind::AbstractConstructor { .. }
        | VerificationErrorKind::ExplicitConcreteHasAbstractMethod { .. }) => {
            format_class_method_constraint(constraint, formatter)
        }
        VerificationErrorKind::InvalidConstructorOutput {
            class,
            method,
            local,
            count,
        } => write!(
            formatter,
            "class `{class}` constructor `{method}` output local {} is outside local table of size {count}",
            local.get()
        ),
        VerificationErrorKind::InvalidClassFeature {
            class,
            feature,
            reason,
        } => write!(
            formatter,
            "class `{class}` feature `{feature}` is invalid: {reason}"
        ),
        VerificationErrorKind::SuperclassConstructorOutsideConstructor { superclass } => write!(
            formatter,
            "direct superclass constructor `{superclass}` is invoked outside a unique instance constructor"
        ),
        VerificationErrorKind::InvalidSuperclassConstructorTarget {
            class,
            expected,
            actual,
        } => write!(
            formatter,
            "class `{class}` invokes superclass constructor `{actual}`, but its direct superclass is {expected:?}"
        ),
        VerificationErrorKind::InvalidSuperclassConstructorObject {
            class,
            expected,
            actual,
        } => write!(
            formatter,
            "class `{class}` invokes its superclass constructor on local {}, but constructor output local {} is required",
            actual.get(),
            expected.get()
        ),
        _ => formatter.write_str("invalid class bytecode"),
    }
}

fn format_class_method_constraint(
    kind: &VerificationErrorKind,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match kind {
        VerificationErrorKind::StaticMethodHasConstructorOutput { class, method } => write!(
            formatter,
            "class `{class}` static method `{method}` carries constructor output metadata"
        ),
        VerificationErrorKind::AbstractMethodHasConstructorOutput { class, method } => write!(
            formatter,
            "class `{class}` abstract method `{method}` carries constructor output metadata"
        ),
        VerificationErrorKind::AbstractConstructor { class } => {
            write!(
                formatter,
                "class `{class}` declares an abstract constructor"
            )
        }
        VerificationErrorKind::ExplicitConcreteHasAbstractMethod { class, method } => write!(
            formatter,
            "class `{class}` encodes `Abstract = false` but declares abstract method `{method}`"
        ),
        _ => formatter.write_str("invalid class method constraint"),
    }
}

fn format_special_class_error(
    kind: &VerificationErrorKind,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match kind {
        VerificationErrorKind::DependentPropertyHasInitializer { class, property } => write!(
            formatter,
            "dependent property `{class}.{property}` cannot carry an initializer"
        ),
        VerificationErrorKind::MissingDependentAccessor {
            class,
            property,
            accessor,
        } => write!(
            formatter,
            "dependent property `{class}.{property}` has no `{accessor}` method"
        ),
        VerificationErrorKind::InvalidDependentAccessorTarget { class, accessor } => write!(
            formatter,
            "class `{class}` accessor `{accessor}` does not match dependent-property metadata"
        ),
        VerificationErrorKind::SpecialMethodMustBeInstance { class, method } => write!(
            formatter,
            "class `{class}` accessor or operator `{method}` must be an instance method"
        ),
        VerificationErrorKind::InvalidSpecialMethodSignature {
            class,
            method,
            expected_parameters,
            actual_parameters,
            expected_outputs,
            actual_outputs,
        } => write!(
            formatter,
            "class `{class}` method `{method}` expects {expected_parameters} inputs and {expected_outputs} outputs, encoded function has {actual_parameters} inputs and {actual_outputs:?} outputs"
        ),
        _ => formatter.write_str("invalid dependent-property or operator bytecode"),
    }
}

impl Error for VerificationError {}

/// Validates all structural operands in a bytecode module.
///
/// # Errors
///
/// Returns the first unsupported version, invalid table index, invalid frame
/// operand, or invalid jump target encountered.
pub fn verify(module: &BytecodeModule) -> Result<(), VerificationError> {
    if module.version.minor != 0
        || module.version.major < MIN_SUPPORTED_BYTECODE_VERSION.major
        || module.version.major > CURRENT_BYTECODE_VERSION.major
    {
        return Err(VerificationError {
            function: None,
            instruction: None,
            location: None,
            kind: VerificationErrorKind::UnsupportedVersion {
                found: module.version,
                supported: CURRENT_BYTECODE_VERSION,
            },
        });
    }
    if get(&module.functions, module.entry.get()).is_none() {
        return Err(VerificationError {
            function: None,
            instruction: None,
            location: None,
            kind: VerificationErrorKind::InvalidEntry {
                entry: module.entry,
            },
        });
    }

    verify_classes(module)?;

    for (function_index, function) in module.functions.iter().enumerate() {
        let function_id = FunctionId::new(index_to_u32(function_index));
        verify_function(module, function_id, function)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_classes(module: &BytecodeModule) -> Result<(), VerificationError> {
    for class in &module.classes {
        if class.name.is_empty() {
            return Err(VerificationError {
                function: None,
                instruction: None,
                location: Some(class.location),
                kind: VerificationErrorKind::EmptyClassName { member: None },
            });
        }
        verify_superclass(class)?;
        if class.declared_abstract == Some(false)
            && let Some(method) = class.methods.iter().find(|method| method.is_abstract)
        {
            return Err(VerificationError {
                function: Some(method.function),
                instruction: None,
                location: Some(method.location),
                kind: VerificationErrorKind::ExplicitConcreteHasAbstractMethod {
                    class: class.name.clone(),
                    method: method.name.clone(),
                },
            });
        }
        for property in &class.properties {
            if property.name.is_empty() {
                return Err(VerificationError {
                    function: None,
                    instruction: None,
                    location: Some(property.location),
                    kind: VerificationErrorKind::EmptyClassName {
                        member: Some("property".to_owned()),
                    },
                });
            }
            let valid_write_access = match property.kind {
                crate::PropertyKind::Stored => {
                    property.get_access.is_some() && property.set_access.is_some()
                }
                crate::PropertyKind::Constant => {
                    property.get_access.is_some() && property.set_access.is_none()
                }
                crate::PropertyKind::Dependent => {
                    property.get_access.is_some() || property.set_access.is_some()
                }
            };
            if !valid_write_access {
                return Err(VerificationError {
                    function: None,
                    instruction: None,
                    location: Some(property.location),
                    kind: VerificationErrorKind::InvalidPropertyWriteAccess {
                        class: class.name.clone(),
                        property: property.name.clone(),
                        kind: property.kind,
                    },
                });
            }
            if property.kind == crate::PropertyKind::Constant && property.default.is_none() {
                return Err(VerificationError {
                    function: None,
                    instruction: None,
                    location: Some(property.location),
                    kind: VerificationErrorKind::MissingConstantInitializer {
                        class: class.name.clone(),
                        property: property.name.clone(),
                    },
                });
            }
            if property.kind == crate::PropertyKind::Dependent && property.default.is_some() {
                return Err(VerificationError {
                    function: None,
                    instruction: None,
                    location: Some(property.location),
                    kind: VerificationErrorKind::DependentPropertyHasInitializer {
                        class: class.name.clone(),
                        property: property.name.clone(),
                    },
                });
            }
            if let Some(function_id) = property.default {
                let Some(function) = get(&module.functions, function_id.get()) else {
                    return Err(VerificationError {
                        function: None,
                        instruction: None,
                        location: Some(property.location),
                        kind: VerificationErrorKind::InvalidClassFunction {
                            class: class.name.clone(),
                            member: property.name.clone(),
                            function: function_id,
                        },
                    });
                };
                if function.parameter_count != 0 {
                    return Err(VerificationError {
                        function: Some(function_id),
                        instruction: None,
                        location: Some(property.location),
                        kind: VerificationErrorKind::InvalidPropertyDefaultSignature {
                            class: class.name.clone(),
                            property: property.name.clone(),
                            parameters: function.parameter_count,
                        },
                    });
                }
            }
        }
        for method in &class.methods {
            if method.name.is_empty() {
                return Err(VerificationError {
                    function: None,
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::EmptyClassName {
                        member: Some("method".to_owned()),
                    },
                });
            }
            let Some(function) = get(&module.functions, method.function.get()) else {
                return Err(VerificationError {
                    function: None,
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::InvalidClassFunction {
                        class: class.name.clone(),
                        member: method.name.clone(),
                        function: method.function,
                    },
                });
            };
            if method.is_external && module.version.major < 25 {
                return Err(VerificationError {
                    function: Some(method.function),
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::ExternalMethodRequiresCurrentVersion {
                        found: module.version,
                        class: class.name.clone(),
                        method: method.name.clone(),
                    },
                });
            }
            if method.is_external && method.is_abstract {
                return Err(VerificationError {
                    function: Some(method.function),
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::ExternalMethodIsAbstract {
                        class: class.name.clone(),
                        method: method.name.clone(),
                    },
                });
            }
            if method.kind == crate::MethodKind::Static && method.constructor_output.is_some() {
                return Err(VerificationError {
                    function: Some(method.function),
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::StaticMethodHasConstructorOutput {
                        class: class.name.clone(),
                        method: method.name.clone(),
                    },
                });
            }
            if method.is_abstract && method.constructor_output.is_some() {
                return Err(VerificationError {
                    function: Some(method.function),
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::AbstractMethodHasConstructorOutput {
                        class: class.name.clone(),
                        method: method.name.clone(),
                    },
                });
            }
            let constructor_name = class.name.rsplit('.').next().unwrap_or(&class.name);
            if method.is_abstract && method.name == constructor_name {
                return Err(VerificationError {
                    function: Some(method.function),
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::AbstractConstructor {
                        class: class.name.clone(),
                    },
                });
            }
            if method.name == "plus" {
                verify_special_method_signature(module, class, method, 2, 1)?;
            }
            if let Some((kind, property_name)) = accessor_parts(&method.name) {
                let valid_target = class
                    .properties
                    .iter()
                    .find(|property| property.name == property_name)
                    .map_or_else(
                        || class.superclass.is_some(),
                        |property| {
                            property.kind == crate::PropertyKind::Dependent
                                && match kind {
                                    "get" => property.get_access.is_some(),
                                    "set" => property.set_access.is_some(),
                                    _ => false,
                                }
                        },
                    );
                if !valid_target {
                    return Err(VerificationError {
                        function: Some(method.function),
                        instruction: None,
                        location: Some(method.location),
                        kind: VerificationErrorKind::InvalidDependentAccessorTarget {
                            class: class.name.clone(),
                            accessor: method.name.clone(),
                        },
                    });
                }
                let expected_parameters = if kind == "get" { 1 } else { 2 };
                verify_special_method_signature(module, class, method, expected_parameters, 1)?;
            } else if method.name.contains('.') {
                return Err(VerificationError {
                    function: Some(method.function),
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::InvalidDependentAccessorTarget {
                        class: class.name.clone(),
                        accessor: method.name.clone(),
                    },
                });
            }
            if let Some(local) = method.constructor_output
                && local.get() >= function.local_count
            {
                return Err(VerificationError {
                    function: Some(method.function),
                    instruction: None,
                    location: Some(method.location),
                    kind: VerificationErrorKind::InvalidConstructorOutput {
                        class: class.name.clone(),
                        method: method.name.clone(),
                        local,
                        count: function.local_count,
                    },
                });
            }
        }
        for property in class
            .properties
            .iter()
            .filter(|property| property.kind == crate::PropertyKind::Dependent)
        {
            if property.get_access.is_some() {
                verify_dependent_accessor(class, property, "get")?;
            }
            if property.set_access.is_some() {
                verify_dependent_accessor(class, property, "set")?;
            }
        }
    }
    verify_class_features(module)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_class_features(module: &BytecodeModule) -> Result<(), VerificationError> {
    let mut feature_classes = BTreeSet::new();
    for feature in &module.class_features {
        let Some(class) = get(&module.classes, feature.class.get()) else {
            return Err(VerificationError {
                function: None,
                instruction: None,
                location: Some(feature.location),
                kind: VerificationErrorKind::InvalidClassDefinition {
                    class: feature.class,
                    count: module.classes.len(),
                },
            });
        };
        let invalid =
            |name: String, reason: &'static str, location: SourceLocation| VerificationError {
                function: None,
                instruction: None,
                location: Some(location),
                kind: VerificationErrorKind::InvalidClassFeature {
                    class: class.name.clone(),
                    feature: name,
                    reason,
                },
            };
        if !feature_classes.insert(feature.class) {
            return Err(invalid(
                "<class>".to_owned(),
                "class has more than one feature record",
                feature.location,
            ));
        }
        if feature.kind == ClassKind::Ordinary && !feature.enumeration_members.is_empty() {
            return Err(invalid(
                "enumeration".to_owned(),
                "ordinary class carries enumeration members",
                feature.location,
            ));
        }
        if feature.kind == ClassKind::Enumeration && feature.enumeration_members.is_empty() {
            return Err(invalid(
                "enumeration".to_owned(),
                "enumeration class has no members",
                feature.location,
            ));
        }
        if feature.kind == ClassKind::Enumeration
            && (class.declared_abstract == Some(true)
                || class.methods.iter().any(|method| method.is_abstract)
                || !feature.abstract_properties.is_empty())
        {
            return Err(invalid(
                "enumeration".to_owned(),
                "enumeration class is abstract",
                feature.location,
            ));
        }
        if feature.kind == ClassKind::Ordinary && feature.enumeration_base.is_some() {
            return Err(invalid(
                "enumeration".to_owned(),
                "ordinary class carries an enumeration base",
                feature.location,
            ));
        }
        if feature
            .enumeration_base
            .as_ref()
            .is_some_and(|base| base.is_empty() || base == &class.name)
        {
            return Err(invalid(
                "enumeration".to_owned(),
                "enumeration base is empty or self-referential",
                feature.location,
            ));
        }

        let mut names = BTreeSet::new();
        for member in &feature.enumeration_members {
            if member.name.is_empty() || !names.insert(member.name.as_str()) {
                return Err(invalid(
                    member.name.clone(),
                    "enumeration member name is empty or duplicated",
                    member.location,
                ));
            }
            if class.properties.iter().any(|item| item.name == member.name)
                || class.methods.iter().any(|item| item.name == member.name)
            {
                return Err(invalid(
                    member.name.clone(),
                    "enumeration member conflicts with a property or method",
                    member.location,
                ));
            }
            let Some(function) = get(&module.functions, member.argument_initializer.get()) else {
                return Err(VerificationError {
                    function: None,
                    instruction: None,
                    location: Some(member.location),
                    kind: VerificationErrorKind::InvalidClassFunction {
                        class: class.name.clone(),
                        member: member.name.clone(),
                        function: member.argument_initializer,
                    },
                });
            };
            let mut saw_return = false;
            let returns_match = function.parameter_count == 0
                && function.instructions.iter().all(|instruction| {
                    if let InstructionKind::Return { values } = &instruction.kind {
                        saw_return = true;
                        u32::try_from(values.len()) == Ok(member.argument_count)
                    } else {
                        true
                    }
                })
                && saw_return;
            if !returns_match {
                return Err(invalid(
                    member.name.clone(),
                    "constructor-argument initializer signature does not match its arity",
                    member.location,
                ));
            }
        }

        for event in &feature.events {
            if event.name.is_empty() || !names.insert(event.name.as_str()) {
                return Err(invalid(
                    event.name.clone(),
                    "event name is empty or conflicts with another class feature",
                    event.location,
                ));
            }
            if class.properties.iter().any(|item| item.name == event.name)
                || class.methods.iter().any(|item| item.name == event.name)
            {
                return Err(invalid(
                    event.name.clone(),
                    "event name conflicts with a property or method",
                    event.location,
                ));
            }
        }

        for property in &feature.abstract_properties {
            if property.name.is_empty() || !names.insert(property.name.as_str()) {
                return Err(invalid(
                    property.name.clone(),
                    "abstract property name is empty or conflicts with another class feature",
                    property.location,
                ));
            }
            let valid_access = match property.kind {
                crate::PropertyKind::Stored => {
                    property.get_access.is_some() && property.set_access.is_some()
                }
                crate::PropertyKind::Constant => {
                    property.get_access.is_some() && property.set_access.is_none()
                }
                crate::PropertyKind::Dependent => {
                    property.get_access.is_some() || property.set_access.is_some()
                }
            };
            if !valid_access {
                return Err(invalid(
                    property.name.clone(),
                    "abstract property kind and access metadata disagree",
                    property.location,
                ));
            }
            if class
                .properties
                .iter()
                .any(|item| item.name == property.name)
                || class.methods.iter().any(|item| item.name == property.name)
            {
                return Err(invalid(
                    property.name.clone(),
                    "abstract property conflicts with a concrete property or method",
                    property.location,
                ));
            }
        }

        let mut sealed = BTreeSet::new();
        for method in &feature.sealed_methods {
            let target = class
                .methods
                .iter()
                .find(|candidate| candidate.name == *method);
            if method.is_empty() || !sealed.insert(method.as_str()) {
                return Err(invalid(
                    method.clone(),
                    "sealed method name is empty or duplicated",
                    feature.location,
                ));
            }
            if target.is_none() {
                return Err(invalid(
                    method.clone(),
                    "sealed method must name a local method",
                    feature.location,
                ));
            }
        }
    }
    Ok(())
}

fn verify_dependent_accessor(
    class: &crate::ClassDefinition,
    property: &crate::PropertyDefinition,
    kind: &str,
) -> Result<(), VerificationError> {
    let accessor = format!("{kind}.{}", property.name);
    if class.methods.iter().any(|method| method.name == accessor) {
        return Ok(());
    }
    Err(VerificationError {
        function: None,
        instruction: None,
        location: Some(property.location),
        kind: VerificationErrorKind::MissingDependentAccessor {
            class: class.name.clone(),
            property: property.name.clone(),
            accessor,
        },
    })
}

fn accessor_parts(name: &str) -> Option<(&str, &str)> {
    let (kind, property) = name.split_once('.')?;
    (matches!(kind, "get" | "set") && !property.is_empty() && !property.contains('.'))
        .then_some((kind, property))
}

fn verify_special_method_signature(
    module: &BytecodeModule,
    class: &crate::ClassDefinition,
    method: &crate::MethodDefinition,
    expected_parameters: u32,
    expected_outputs: u32,
) -> Result<(), VerificationError> {
    if method.kind != crate::MethodKind::Instance {
        return Err(VerificationError {
            function: Some(method.function),
            instruction: None,
            location: Some(method.location),
            kind: VerificationErrorKind::SpecialMethodMustBeInstance {
                class: class.name.clone(),
                method: method.name.clone(),
            },
        });
    }
    let Some(function) = get(&module.functions, method.function.get()) else {
        return Ok(());
    };
    let actual_outputs = function.instructions.iter().find_map(|instruction| {
        let InstructionKind::Return { values } = &instruction.kind else {
            return None;
        };
        u32::try_from(values.len()).ok()
    });
    let outputs_match = actual_outputs == Some(expected_outputs)
        && function.instructions.iter().all(|instruction| {
            let InstructionKind::Return { values } = &instruction.kind else {
                return true;
            };
            u32::try_from(values.len()) == Ok(expected_outputs)
        });
    if function.parameter_count != expected_parameters || !outputs_match {
        return Err(VerificationError {
            function: Some(method.function),
            instruction: None,
            location: Some(method.location),
            kind: VerificationErrorKind::InvalidSpecialMethodSignature {
                class: class.name.clone(),
                method: method.name.clone(),
                expected_parameters,
                actual_parameters: function.parameter_count,
                expected_outputs,
                actual_outputs,
            },
        });
    }
    Ok(())
}

fn verify_superclass(class: &crate::ClassDefinition) -> Result<(), VerificationError> {
    let Some(superclass) = &class.superclass else {
        return Ok(());
    };
    let kind = if superclass.is_empty() {
        Some(VerificationErrorKind::EmptySuperclassName {
            class: class.name.clone(),
        })
    } else if superclass == &class.name {
        Some(VerificationErrorKind::SelfSuperclass {
            class: class.name.clone(),
        })
    } else {
        None
    };
    if let Some(kind) = kind {
        Err(VerificationError {
            function: None,
            instruction: None,
            location: Some(class.location),
            kind,
        })
    } else {
        Ok(())
    }
}

fn verify_function(
    module: &BytecodeModule,
    function_id: FunctionId,
    function: &Function,
) -> Result<(), VerificationError> {
    if let Some(layout) = &function.argument_layout {
        let mut fields = BTreeSet::new();
        let mut slots = BTreeSet::new();
        let positional_end = layout.positional_count.checked_add(layout.repeating_count);
        let valid = module.version.major >= 28
            && (layout.repeating_count == 0 || module.version.major >= 29)
            && layout.required_count <= layout.positional_count
            && positional_end.is_some_and(|end| end <= function.parameter_count)
            && layout.named.iter().all(|(slot, name)| {
                slots.insert(slot.get());
                positional_end.is_some_and(|end| slot.get() >= end)
                    && slot.get() < function.parameter_count
                    && !name.is_empty()
                    && fields.insert(name.clone())
            })
            && slots.len() == (function.parameter_count - positional_end.unwrap_or(0)) as usize
            && !function.instructions.iter().any(|instruction| {
                matches!(instruction.kind, InstructionKind::LoadVariadicInputs { .. })
            });
        if !valid {
            return Err(function_error(
                function_id,
                VerificationErrorKind::InvalidArgumentLayout,
            ));
        }
    }
    if function.parameter_count > function.local_count {
        return Err(function_error(
            function_id,
            VerificationErrorKind::ParametersExceedLocals {
                parameters: function.parameter_count,
                locals: function.local_count,
            },
        ));
    }

    for constant in &function.constants {
        if let Constant::Function(referenced) = constant
            && get(&module.functions, referenced.get()).is_none()
        {
            return Err(function_error(
                function_id,
                VerificationErrorKind::InvalidFunctionConstant {
                    function: *referenced,
                },
            ));
        }
    }

    verify_exception_handlers(function_id, function)?;

    let mut variadic_inputs =
        function
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instruction)| {
                matches!(instruction.kind, InstructionKind::LoadVariadicInputs { .. })
            });
    if let Some((index, _)) = variadic_inputs.next() {
        let first_executable = function
            .instructions
            .iter()
            .position(|instruction| {
                !matches!(
                    instruction.kind,
                    InstructionKind::DeclareNamedBindings { .. }
                        | InstructionKind::DeclareImports { .. }
                )
            })
            .unwrap_or(0);
        if index != first_executable {
            return Err(function_error(
                function_id,
                VerificationErrorKind::VariadicInputsMustBeFirst,
            ));
        }
        if variadic_inputs.next().is_some() {
            return Err(function_error(
                function_id,
                VerificationErrorKind::DuplicateVariadicInputs,
            ));
        }
    }

    for (instruction_index, instruction) in function.instructions.iter().enumerate() {
        verify_instruction(
            module,
            function_id,
            InstructionIndex::new(index_to_u32(instruction_index)),
            function,
            instruction,
        )?;
    }
    Ok(())
}

fn verify_exception_handlers(
    function_id: FunctionId,
    function: &Function,
) -> Result<(), VerificationError> {
    for handler in &function.exception_handlers {
        verify_exception_handler(function_id, function, handler)?;
    }
    verify_exception_range_nesting(function_id, &function.exception_handlers)
}

fn verify_exception_handler(
    function_id: FunctionId,
    function: &Function,
    handler: &ExceptionHandler,
) -> Result<(), VerificationError> {
    let fail = |kind| exception_handler_error(function_id, handler, kind);
    let instruction_count = function.instructions.len();
    let start = handler.protected_start;
    let end = handler.protected_end;
    if start >= end
        || get(&function.instructions, start.get()).is_none()
        || get(&function.instructions, end.get()).is_none()
    {
        return Err(fail(
            VerificationErrorKind::InvalidExceptionProtectedRange {
                start,
                end,
                count: instruction_count,
            },
        ));
    }
    if get(&function.instructions, handler.exit.get()).is_none() {
        return Err(fail(VerificationErrorKind::InvalidExceptionExit {
            exit: handler.exit,
            count: instruction_count,
        }));
    }

    match handler.kind {
        ExceptionHandlerKind::Catch {
            handler: catch,
            error_local,
        } => {
            if get(&function.instructions, catch.get()).is_none() {
                return Err(fail(VerificationErrorKind::InvalidExceptionHandler {
                    handler: catch,
                    count: instruction_count,
                }));
            }
            let expected_catch = end.get().checked_add(1).map(InstructionIndex::new);
            if expected_catch != Some(catch) || catch > handler.exit {
                return Err(fail(VerificationErrorKind::InvalidExceptionCatchOrder {
                    protected_end: end,
                    handler: catch,
                    exit: handler.exit,
                }));
            }
            if let Some(local) = error_local {
                check_local(function, local).map_err(fail)?;
            }
            let end_instruction = get(&function.instructions, end.get())
                .expect("protected-end instruction checked above");
            if !matches!(
                &end_instruction.kind,
                InstructionKind::Jump { target } if *target == handler.exit
            ) {
                return Err(fail(VerificationErrorKind::InvalidExceptionCatchSkip {
                    instruction: end,
                    exit: handler.exit,
                }));
            }
        }
        ExceptionHandlerKind::Swallow => {
            if end != handler.exit {
                return Err(fail(VerificationErrorKind::InvalidExceptionSwallowExit {
                    protected_end: end,
                    exit: handler.exit,
                }));
            }
        }
    }

    Ok(())
}

fn verify_exception_range_nesting(
    function_id: FunctionId,
    handlers: &[ExceptionHandler],
) -> Result<(), VerificationError> {
    let fail =
        |handler: &ExceptionHandler, kind| exception_handler_error(function_id, handler, kind);

    for (first_index, first) in handlers.iter().enumerate() {
        for second in &handlers[first_index + 1..] {
            let first_start = first.protected_start;
            let first_end = first.protected_end;
            let second_start = second.protected_start;
            let second_end = second.protected_end;
            if first_start == second_start && first_end == second_end {
                return Err(fail(
                    second,
                    VerificationErrorKind::DuplicateExceptionRange {
                        start: second_start,
                        end: second_end,
                    },
                ));
            }
            let overlaps = first_start < second_end && second_start < first_end;
            let first_contains_second = first_start <= second_start && second_end <= first_end;
            let second_contains_first = second_start <= first_start && first_end <= second_end;
            if overlaps && !first_contains_second && !second_contains_first {
                return Err(fail(
                    second,
                    VerificationErrorKind::CrossingExceptionRanges {
                        first_start,
                        first_end,
                        second_start,
                        second_end,
                    },
                ));
            }
            let nested = if first_contains_second {
                Some((first, second))
            } else if second_contains_first {
                Some((second, first))
            } else {
                None
            };
            if let Some((outer, inner)) = nested
                && inner.exit > outer.protected_end
            {
                return Err(fail(
                    inner,
                    VerificationErrorKind::NestedExceptionHandlerEscapes {
                        outer_start: outer.protected_start,
                        outer_end: outer.protected_end,
                        inner_start: inner.protected_start,
                        inner_end: inner.protected_end,
                        inner_exit: inner.exit,
                    },
                ));
            }
        }
    }

    Ok(())
}

fn exception_handler_error(
    function: FunctionId,
    handler: &ExceptionHandler,
    kind: VerificationErrorKind,
) -> VerificationError {
    VerificationError {
        function: Some(function),
        instruction: None,
        location: handler.location,
        kind,
    }
}

#[allow(clippy::too_many_lines)]
fn verify_instruction(
    module: &BytecodeModule,
    function_id: FunctionId,
    instruction_index: InstructionIndex,
    function: &Function,
    instruction: &Instruction,
) -> Result<(), VerificationError> {
    let fail = |kind| VerificationError {
        function: Some(function_id),
        instruction: Some(instruction_index),
        location: instruction.location,
        kind,
    };
    let register = |operand| check_register(function, operand).map_err(fail);
    let pack_register = |operand| check_pack_register(function, operand).map_err(fail);
    let local = |operand| check_local(function, operand).map_err(fail);
    let persistent = |operand| check_persistent_slot(function, operand).map_err(fail);
    let constant = |operand| check_constant(function, operand).map_err(fail);
    let global_name = |operand| check_global_name(function, operand).map_err(fail);
    let field_name = |operand| check_field_name(function, operand).map_err(fail);
    let jump = |operand| check_jump(function, operand).map_err(fail);

    if module.version.major < 24
        && matches!(
            instruction.kind,
            InstructionKind::DeclareImports { .. }
                | InstructionKind::LoadQualifiedTarget { .. }
                | InstructionKind::LoadFunctionHandleCandidates { .. }
                | InstructionKind::ApplyQualified { .. }
                | InstructionKind::GetQualifiedPack { .. }
                | InstructionKind::StatementApplyQualified { .. }
        )
    {
        return Err(fail(
            VerificationErrorKind::PackageInstructionsRequireCurrentVersion {
                found: module.version,
            },
        ));
    }
    if module.version.major < 25 && matches!(instruction.kind, InstructionKind::ClearImports) {
        return Err(fail(
            VerificationErrorKind::ImportClearRequiresCurrentVersion {
                found: module.version,
            },
        ));
    }
    if module.version.major < 26
        && matches!(instruction.kind, InstructionKind::LoadCallTarget { .. })
    {
        return Err(fail(
            VerificationErrorKind::CallTargetRequiresCurrentVersion {
                found: module.version,
            },
        ));
    }

    if module.version.major < 27
        && matches!(
            instruction.kind,
            InstructionKind::RequireDefined { .. }
                | InstructionKind::RequireSinglePack { .. }
                | InstructionKind::CountPlaceOutputs { .. }
                | InstructionKind::ApplyOutputPack { .. }
                | InstructionKind::SlicePack { .. }
        )
    {
        return Err(fail(
            VerificationErrorKind::DynamicOutputsRequireCurrentVersion {
                found: module.version,
            },
        ));
    }

    match &instruction.kind {
        InstructionKind::RequireSinglePack { pack } => pack_register(*pack)?,
        InstructionKind::RequireDefined { value, name } => {
            register(*value)?;
            global_name(*name)?;
        }
        InstructionKind::CountPlaceOutputs { dst, root, path } => {
            register(*dst)?;
            register(*root)?;
            if path.is_empty() {
                return Err(fail(VerificationErrorKind::EmptyPlacePath));
            }
            verify_place_path(path, &register, &pack_register, &field_name)?;
        }
        InstructionKind::ApplyOutputPack {
            dst_pack,
            count,
            target,
            arguments,
        } => {
            pack_register(*dst_pack)?;
            register(*count)?;
            match target {
                PackApplyTarget::Value(value) => register(*value)?,
                PackApplyTarget::Field { object, name } => {
                    register(*object)?;
                    field_name(*name)?;
                }
                PackApplyTarget::Qualified {
                    target,
                    unresolved_suffix,
                    members,
                } => {
                    register(*target)?;
                    register(*unresolved_suffix)?;
                    for member in members {
                        field_name(*member)?;
                    }
                }
            }
            verify_apply_arguments(arguments, &register, &pack_register)?;
        }
        InstructionKind::SlicePack {
            dst_pack,
            source,
            start,
            count,
        } => {
            pack_register(*dst_pack)?;
            pack_register(*source)?;
            register(*start)?;
            register(*count)?;
        }
        InstructionKind::DeclareNamedBindings { bindings } => {
            for (name, kind) in bindings {
                global_name(*name)?;
                match kind {
                    NamedBindingKind::Local(slot) => local(*slot)?,
                    NamedBindingKind::Persistent(slot) => persistent(*slot)?,
                    NamedBindingKind::Capture | NamedBindingKind::Workspace => {}
                }
            }
        }
        InstructionKind::DeclareImports { imports } => {
            for import in imports {
                global_name(*import)?;
            }
        }
        InstructionKind::ClearImports | InstructionKind::ClearGlobalAll => {}
        InstructionKind::LoadConstant { dst, constant: id } => {
            register(*dst)?;
            constant(*id)?;
        }
        InstructionKind::Move { dst, src } => {
            register(*dst)?;
            register(*src)?;
        }
        InstructionKind::Binary { dst, lhs, rhs, .. } => {
            register(*dst)?;
            register(*lhs)?;
            register(*rhs)?;
        }
        InstructionKind::SwitchMatch {
            dst,
            selector,
            case_value,
        } => {
            register(*dst)?;
            register(*selector)?;
            register(*case_value)?;
        }
        InstructionKind::BuildMatrix { dst, rows } => {
            register(*dst)?;
            for element in rows.iter().flatten() {
                register(*element)?;
            }
        }
        InstructionKind::BuildCell { dst, rows } => {
            register(*dst)?;
            if let Some(first) = rows.first() {
                let expected = first.len();
                for (row, values) in rows.iter().enumerate().skip(1) {
                    if values.len() != expected {
                        return Err(fail(VerificationErrorKind::RaggedCellRows {
                            row,
                            expected,
                            actual: values.len(),
                        }));
                    }
                }
            }
            for source in rows.iter().flatten() {
                verify_value_source(*source, &register, &pack_register)?;
            }
        }
        InstructionKind::Range {
            dst,
            start,
            step,
            end,
        } => {
            register(*dst)?;
            register(*start)?;
            register(*step)?;
            register(*end)?;
        }
        InstructionKind::Transpose { dst, operand, .. } => {
            register(*dst)?;
            register(*operand)?;
        }
        InstructionKind::LoadLocal { dst, local: slot } => {
            register(*dst)?;
            local(*slot)?;
        }
        InstructionKind::StoreLocal { local: slot, src } => {
            local(*slot)?;
            register(*src)?;
        }
        InstructionKind::DeclareGlobal { name } => global_name(*name)?,
        InstructionKind::DeclarePersistent { slot } => persistent(*slot)?,
        InstructionKind::LoadPersistent { dst, slot } => {
            register(*dst)?;
            persistent(*slot)?;
        }
        InstructionKind::StorePersistent { slot, src } => {
            persistent(*slot)?;
            register(*src)?;
        }
        InstructionKind::LoadCallInputCount { dst }
        | InstructionKind::LoadCallOutputCount { dst }
        | InstructionKind::LoadVariadicInputs { dst } => register(*dst)?,
        InstructionKind::LoadGlobal { dst, name, .. }
        | InstructionKind::LoadCallTarget { dst, name }
        | InstructionKind::LoadFunctionHandle { dst, name }
        | InstructionKind::LoadGlobalOrNothing { dst, name }
        | InstructionKind::LoadCapture { dst, name } => {
            register(*dst)?;
            global_name(*name)?;
        }
        InstructionKind::LoadFunctionHandleCandidates { dst, names } => {
            register(*dst)?;
            for name in names {
                global_name(*name)?;
            }
        }
        InstructionKind::LoadQualifiedTarget {
            dst,
            root_found,
            unresolved_suffix,
            root,
            qualified,
            ..
        } => {
            register(*dst)?;
            register(*root_found)?;
            register(*unresolved_suffix)?;
            global_name(*root)?;
            for candidate in qualified {
                global_name(*candidate)?;
            }
        }
        InstructionKind::MakeClosure {
            dst,
            function: closure,
            captures,
        } => {
            register(*dst)?;
            if get(&module.functions, closure.get()).is_none() {
                return Err(fail(VerificationErrorKind::InvalidClosureFunction {
                    function: *closure,
                }));
            }
            let mut names = BTreeSet::new();
            for (name, value, _) in captures {
                global_name(*name)?;
                if let Some(value) = value {
                    register(*value)?;
                }
                let Constant::String(name) = &function.constants[name.get() as usize] else {
                    unreachable!("global-name validation accepted a non-string constant");
                };
                if !names.insert(name.clone()) {
                    return Err(fail(VerificationErrorKind::DuplicateClosureCapture {
                        name: name.clone(),
                    }));
                }
            }
        }
        InstructionKind::MakeSharedClosure {
            dst,
            function: closure,
            captures,
        } => {
            register(*dst)?;
            if get(&module.functions, closure.get()).is_none() {
                return Err(fail(VerificationErrorKind::InvalidClosureFunction {
                    function: *closure,
                }));
            }
            let mut names = BTreeSet::new();
            for (name, source) in captures {
                global_name(*name)?;
                if let SharedCaptureSource::Local(slot) = source {
                    local(*slot)?;
                }
                let Constant::String(name) = &function.constants[name.get() as usize] else {
                    unreachable!("global-name validation accepted a non-string constant");
                };
                if !names.insert(name.clone()) {
                    return Err(fail(VerificationErrorKind::DuplicateClosureCapture {
                        name: name.clone(),
                    }));
                }
            }
        }
        InstructionKind::StoreCapture { name, src }
        | InstructionKind::StoreGlobal { name, src }
        | InstructionKind::Display { name, src } => {
            global_name(*name)?;
            register(*src)?;
        }
        InstructionKind::ClearGlobal { names } => {
            if names.is_empty() {
                return Err(fail(VerificationErrorKind::EmptyClearNames));
            }
            for name in names {
                global_name(*name)?;
            }
        }
        InstructionKind::ClearLocal { locals } => {
            for slot in locals {
                local(*slot)?;
            }
        }
        InstructionKind::ClearCapture { names }
        | InstructionKind::ClearDynamicBindings { names, .. } => {
            for name in names {
                global_name(*name)?;
            }
        }
        InstructionKind::RegisterClass { class } => {
            if get(&module.classes, class.get()).is_none() {
                return Err(fail(VerificationErrorKind::InvalidClassDefinition {
                    class: *class,
                    count: module.classes.len(),
                }));
            }
        }
        InstructionKind::InvokeSuperclassConstructor {
            superclass,
            object,
            arguments,
        } => {
            local(*object)?;
            global_name(*superclass)?;
            for argument in arguments {
                register(*argument)?;
            }
            verify_superclass_constructor_context(
                module,
                function_id,
                function,
                *superclass,
                *object,
            )
            .map_err(fail)?;
        }
        InstructionKind::GetField { dst, object, name } => {
            register(*dst)?;
            register(*object)?;
            global_name(*name)?;
        }
        InstructionKind::SetField {
            dst,
            object,
            name,
            value,
        } => {
            register(*dst)?;
            register(*object)?;
            global_name(*name)?;
            register(*value)?;
        }
        InstructionKind::ApplyField {
            outputs,
            object,
            name,
            arguments,
        } => {
            register(*object)?;
            global_name(*name)?;
            for output in outputs {
                register(*output)?;
            }
            verify_apply_arguments(arguments, &register, &pack_register)?;
        }
        InstructionKind::ApplyQualified {
            outputs,
            target,
            unresolved_suffix,
            members,
            arguments,
        } => {
            register(*target)?;
            register(*unresolved_suffix)?;
            for member in members {
                field_name(*member)?;
            }
            for output in outputs {
                register(*output)?;
            }
            verify_apply_arguments(arguments, &register, &pack_register)?;
        }
        InstructionKind::GetQualifiedPack {
            dst_pack,
            target,
            root_found,
            unresolved_suffix,
            members,
        } => {
            pack_register(*dst_pack)?;
            register(*target)?;
            register(*root_found)?;
            register(*unresolved_suffix)?;
            for member in members {
                field_name(*member)?;
            }
        }
        InstructionKind::BraceApply {
            dst_pack,
            target,
            arguments,
        } => {
            pack_register(*dst_pack)?;
            register(*target)?;
            verify_apply_arguments(arguments, &register, &pack_register)?;
        }
        InstructionKind::GetAggregateField {
            dst_pack,
            target,
            field,
        } => {
            pack_register(*dst_pack)?;
            register(*target)?;
            verify_field_operand(*field, &register, &field_name)?;
        }
        InstructionKind::AssignPlace {
            dst,
            root,
            path,
            source,
            mode,
        } => {
            register(*dst)?;
            register(*root)?;
            if path.is_empty() {
                return Err(fail(VerificationErrorKind::EmptyPlacePath));
            }
            verify_place_path(path, &register, &pack_register, &field_name)?;
            verify_value_source(*source, &register, &pack_register)?;
            if *mode == AssignmentMode::Delete && matches!(source, ValueSource::Expand(_)) {
                return Err(fail(VerificationErrorKind::DeleteSourceMustBeOne));
            }
        }
        InstructionKind::AssignBindingPlace {
            result,
            binding,
            root,
            path,
            source,
            mode,
        } => {
            if let Some(result) = result {
                register(*result)?;
            }
            match binding {
                BindingTarget::Local(slot) => local(*slot)?,
                BindingTarget::Persistent(slot) => persistent(*slot)?,
                BindingTarget::Capture(name) | BindingTarget::Workspace(name) => {
                    global_name(*name)?;
                }
            }
            register(*root)?;
            if path.is_empty() {
                return Err(fail(VerificationErrorKind::EmptyPlacePath));
            }
            verify_place_path(path, &register, &pack_register, &field_name)?;
            verify_value_source(*source, &register, &pack_register)?;
            if *mode == AssignmentMode::Delete && matches!(source, ValueSource::Expand(_)) {
                return Err(fail(VerificationErrorKind::DeleteSourceMustBeOne));
            }
        }
        InstructionKind::ResolveEnd {
            dst,
            target,
            argument_index,
            argument_count,
        } => {
            register(*dst)?;
            register(*target)?;
            if *argument_count == 0 {
                return Err(fail(VerificationErrorKind::InvalidEndArgumentCount));
            }
            if *argument_index >= *argument_count {
                return Err(fail(VerificationErrorKind::InvalidEndArgumentIndex {
                    argument_index: *argument_index,
                    argument_count: *argument_count,
                }));
            }
        }
        InstructionKind::Unpack { outputs, pack } => {
            for output in outputs {
                register(*output)?;
            }
            pack_register(*pack)?;
        }
        InstructionKind::Jump { target } => jump(*target)?,
        InstructionKind::JumpIfFalse { condition, target } => {
            register(*condition)?;
            jump(*target)?;
        }
        InstructionKind::Call {
            outputs,
            callee,
            arguments,
        } => {
            register(*callee)?;
            for output in outputs {
                register(*output)?;
            }
            for argument in arguments {
                register(*argument)?;
            }
        }
        InstructionKind::Apply {
            outputs,
            target,
            arguments,
        }
        | InstructionKind::ApplyBinding {
            outputs,
            target,
            arguments,
        } => {
            register(*target)?;
            for output in outputs {
                register(*output)?;
            }
            verify_apply_arguments(arguments, &register, &pack_register)?;
        }
        InstructionKind::StatementApply {
            target,
            arguments,
            result,
            ..
        } => {
            register(*target)?;
            verify_apply_arguments(arguments, &register, &pack_register)?;
            match result {
                StatementResultTarget::Global(name) => global_name(*name)?,
                StatementResultTarget::Local(slot) => local(*slot)?,
            }
        }
        InstructionKind::StatementApplyField {
            object,
            name,
            arguments,
            result,
            ..
        } => {
            register(*object)?;
            field_name(*name)?;
            verify_apply_arguments(arguments, &register, &pack_register)?;
            match result {
                StatementResultTarget::Global(name) => global_name(*name)?,
                StatementResultTarget::Local(slot) => local(*slot)?,
            }
        }
        InstructionKind::StatementApplyQualified {
            target,
            unresolved_suffix,
            members,
            arguments,
            result,
            ..
        } => {
            register(*target)?;
            register(*unresolved_suffix)?;
            for member in members {
                field_name(*member)?;
            }
            verify_apply_arguments(arguments, &register, &pack_register)?;
            match result {
                StatementResultTarget::Global(name) => global_name(*name)?,
                StatementResultTarget::Local(slot) => local(*slot)?,
            }
        }
        InstructionKind::StatementPack { pack, result, .. } => {
            pack_register(*pack)?;
            match result {
                StatementResultTarget::Global(name) => global_name(*name)?,
                StatementResultTarget::Local(slot) => local(*slot)?,
            }
        }
        InstructionKind::StatementValue { src, result, .. } => {
            register(*src)?;
            match result {
                StatementResultTarget::Global(name) => global_name(*name)?,
                StatementResultTarget::Local(slot) => local(*slot)?,
            }
        }
        InstructionKind::ReturnApply { target, arguments } => {
            register(*target)?;
            verify_apply_arguments(arguments, &register, &pack_register)?;
        }
        InstructionKind::ReturnApplyField {
            object,
            name,
            arguments,
        } => {
            register(*object)?;
            field_name(*name)?;
            verify_apply_arguments(arguments, &register, &pack_register)?;
        }
        InstructionKind::IndexAssign {
            dst,
            target,
            arguments,
            value,
        } => {
            register(*dst)?;
            register(*target)?;
            register(*value)?;
            verify_apply_arguments(arguments, &register, &pack_register)?;
        }
        InstructionKind::ForEach {
            iterable,
            index,
            dst,
            exit,
        } => {
            register(*iterable)?;
            register(*index)?;
            register(*dst)?;
            jump(*exit)?;
        }
        InstructionKind::Return { values } => {
            for value in values {
                register(*value)?;
            }
        }
        InstructionKind::ReturnVariadic { fixed, variadic } => {
            for value in fixed {
                register(*value)?;
            }
            register(*variadic)?;
        }
    }
    Ok(())
}

fn verify_superclass_constructor_context(
    module: &BytecodeModule,
    function_id: FunctionId,
    function: &Function,
    superclass: ConstantId,
    object: LocalSlot,
) -> Result<(), VerificationErrorKind> {
    let actual = match get(&function.constants, superclass.get()) {
        Some(Constant::String(name)) => name.clone(),
        Some(_) => {
            return Err(VerificationErrorKind::GlobalNameIsNotString {
                constant: superclass,
            });
        }
        None => {
            return Err(VerificationErrorKind::InvalidConstant {
                constant: superclass,
                count: function.constants.len(),
            });
        }
    };
    let mut owners = module.classes.iter().flat_map(|class| {
        class
            .methods
            .iter()
            .filter(move |method| method.function == function_id)
            .map(move |method| (class, method))
    });
    let Some((class, method)) = owners.next() else {
        return Err(
            VerificationErrorKind::SuperclassConstructorOutsideConstructor { superclass: actual },
        );
    };
    if owners.next().is_some()
        || method.name != class.name.rsplit('.').next().unwrap_or(&class.name)
        || method.kind != crate::MethodKind::Instance
        || method.constructor_output.is_none()
    {
        return Err(
            VerificationErrorKind::SuperclassConstructorOutsideConstructor { superclass: actual },
        );
    }
    if class.superclass.as_deref() != Some(actual.as_str()) {
        return Err(VerificationErrorKind::InvalidSuperclassConstructorTarget {
            class: class.name.clone(),
            expected: class.superclass.clone(),
            actual,
        });
    }
    let expected = method
        .constructor_output
        .expect("constructor output checked above");
    if object != expected {
        return Err(VerificationErrorKind::InvalidSuperclassConstructorObject {
            class: class.name.clone(),
            expected,
            actual: object,
        });
    }
    Ok(())
}

fn verify_apply_arguments(
    arguments: &[ApplyArgument],
    register: &impl Fn(Register) -> Result<(), VerificationError>,
    pack_register: &impl Fn(PackRegister) -> Result<(), VerificationError>,
) -> Result<(), VerificationError> {
    for argument in arguments {
        match argument {
            ApplyArgument::Value(argument) => register(*argument)?,
            ApplyArgument::Expand(argument) => pack_register(*argument)?,
            ApplyArgument::Colon => {}
        }
    }
    Ok(())
}

fn verify_value_source(
    source: ValueSource,
    register: &impl Fn(Register) -> Result<(), VerificationError>,
    pack_register: &impl Fn(PackRegister) -> Result<(), VerificationError>,
) -> Result<(), VerificationError> {
    match source {
        ValueSource::One(source) => register(source),
        ValueSource::Expand(source) => pack_register(source),
    }
}

fn verify_field_operand(
    field: FieldOperand,
    register: &impl Fn(Register) -> Result<(), VerificationError>,
    field_name: &impl Fn(ConstantId) -> Result<(), VerificationError>,
) -> Result<(), VerificationError> {
    match field {
        FieldOperand::Static(field) => field_name(field),
        FieldOperand::Dynamic(field) => register(field),
    }
}

fn verify_place_path(
    path: &[PlaceStep],
    register: &impl Fn(Register) -> Result<(), VerificationError>,
    pack_register: &impl Fn(PackRegister) -> Result<(), VerificationError>,
    field_name: &impl Fn(ConstantId) -> Result<(), VerificationError>,
) -> Result<(), VerificationError> {
    for step in path {
        match step {
            PlaceStep::Paren(arguments) | PlaceStep::Brace(arguments) => {
                verify_apply_arguments(arguments, register, pack_register)?;
            }
            PlaceStep::Field(field) => verify_field_operand(*field, register, field_name)?,
        }
    }
    Ok(())
}

fn check_register(function: &Function, register: Register) -> Result<(), VerificationErrorKind> {
    if register.get() < function.register_count {
        Ok(())
    } else {
        Err(VerificationErrorKind::InvalidRegister {
            register,
            count: function.register_count,
        })
    }
}

fn check_pack_register(
    function: &Function,
    register: PackRegister,
) -> Result<(), VerificationErrorKind> {
    if register.get() < function.pack_register_count {
        Ok(())
    } else {
        Err(VerificationErrorKind::InvalidPackRegister {
            register,
            count: function.pack_register_count,
        })
    }
}

fn check_local(function: &Function, local: LocalSlot) -> Result<(), VerificationErrorKind> {
    if local.get() < function.local_count {
        Ok(())
    } else {
        Err(VerificationErrorKind::InvalidLocal {
            local,
            count: function.local_count,
        })
    }
}

fn check_persistent_slot(
    function: &Function,
    slot: PersistentSlot,
) -> Result<(), VerificationErrorKind> {
    if slot.get() < function.persistent_slot_count {
        Ok(())
    } else {
        Err(VerificationErrorKind::InvalidPersistentSlot {
            slot,
            count: function.persistent_slot_count,
        })
    }
}

fn check_constant(function: &Function, constant: ConstantId) -> Result<(), VerificationErrorKind> {
    if get(&function.constants, constant.get()).is_some() {
        Ok(())
    } else {
        Err(VerificationErrorKind::InvalidConstant {
            constant,
            count: function.constants.len(),
        })
    }
}

fn check_global_name(
    function: &Function,
    constant: ConstantId,
) -> Result<(), VerificationErrorKind> {
    match get(&function.constants, constant.get()) {
        None => Err(VerificationErrorKind::InvalidConstant {
            constant,
            count: function.constants.len(),
        }),
        Some(Constant::String(_)) => Ok(()),
        Some(_) => Err(VerificationErrorKind::GlobalNameIsNotString { constant }),
    }
}

fn check_field_name(
    function: &Function,
    constant: ConstantId,
) -> Result<(), VerificationErrorKind> {
    match get(&function.constants, constant.get()) {
        None => Err(VerificationErrorKind::InvalidConstant {
            constant,
            count: function.constants.len(),
        }),
        Some(Constant::String(_)) => Ok(()),
        Some(_) => Err(VerificationErrorKind::FieldNameIsNotString { constant }),
    }
}

fn check_jump(function: &Function, target: InstructionIndex) -> Result<(), VerificationErrorKind> {
    if get(&function.instructions, target.get()).is_some() {
        Ok(())
    } else {
        Err(VerificationErrorKind::InvalidJump {
            target,
            count: function.instructions.len(),
        })
    }
}

fn function_error(function: FunctionId, kind: VerificationErrorKind) -> VerificationError {
    VerificationError {
        function: Some(function),
        instruction: None,
        location: None,
        kind,
    }
}

fn get<T>(items: &[T], index: u32) -> Option<&T> {
    usize::try_from(index)
        .ok()
        .and_then(|converted| items.get(converted))
}

fn index_to_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Access, ApplyArgument, AssignmentMode, BinaryOperator, ClassDefinition, ClassDefinitionId,
        ClassFeatureDefinition, ClassKind, ClassSemantics, EnumMemberDefinition, EventDefinition,
        FieldOperand, InstructionKind, MethodDefinition, MethodKind, PackRegister, PlaceStep,
        PropertyDefinition, PropertyKind, ValueSource,
    };

    fn valid_function() -> Function {
        Function {
            name: "main".to_owned(),
            register_count: 3,
            pack_register_count: 2,
            local_count: 1,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![Constant::Double(1.0), Constant::String("x".to_owned())],
            instructions: vec![
                Instruction::new(InstructionKind::LoadConstant {
                    dst: Register::new(0),
                    constant: ConstantId::new(0),
                }),
                Instruction::new(InstructionKind::Binary {
                    operator: BinaryOperator::Add,
                    dst: Register::new(1),
                    lhs: Register::new(0),
                    rhs: Register::new(0),
                }),
                Instruction::new(InstructionKind::Return {
                    values: vec![Register::new(1)],
                }),
            ],
            exception_handlers: Vec::new(),
        }
    }

    fn verify_kind(kind: InstructionKind) -> Result<(), VerificationError> {
        let mut function = valid_function();
        function.instructions = vec![
            Instruction::new(kind),
            Instruction::new(InstructionKind::Return { values: Vec::new() }),
        ];
        verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
    }

    #[test]
    fn accepts_well_formed_module() {
        let module = BytecodeModule::new(vec![valid_function()], FunctionId::new(0));
        assert_eq!(verify(&module), Ok(()));
        assert_eq!(module.version, BytecodeVersion::new(29, 0));
        assert_eq!(module.clone(), module);
        assert_eq!(module.functions[0].pack_register_count, 2);
        assert_eq!(Function::new("empty", 0, 0, 0).persistent_slot_count, 0);
    }

    #[test]
    fn verifies_v16_left_divide_operator_surface_and_explicit_discriminants() {
        let location = SourceLocation::new(23, 8, 17);
        let mut function = valid_function();
        function.instructions = vec![
            Instruction::located(
                InstructionKind::Binary {
                    operator: BinaryOperator::LeftDivide,
                    dst: Register::new(1),
                    lhs: Register::new(0),
                    rhs: Register::new(0),
                },
                location,
            ),
            Instruction::located(
                InstructionKind::Binary {
                    operator: BinaryOperator::ElementLeftDivide,
                    dst: Register::new(2),
                    lhs: Register::new(0),
                    rhs: Register::new(1),
                },
                location,
            ),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ];
        let module = BytecodeModule::new(vec![function], FunctionId::new(0));

        assert_eq!(verify(&module), Ok(()));
        assert_eq!(BinaryOperator::Divide.logical_discriminant(), 4);
        assert_eq!(BinaryOperator::ElementDivide.logical_discriminant(), 5);
        assert_eq!(BinaryOperator::LeftDivide.logical_discriminant(), 14);
        assert_eq!(BinaryOperator::ElementLeftDivide.logical_discriminant(), 15);
        for discriminant in 0..=15 {
            let operator = BinaryOperator::from_logical_discriminant(discriminant)
                .expect("assigned v16 binary-operator discriminant");
            assert_eq!(operator.logical_discriminant(), discriminant);
        }
        assert_eq!(BinaryOperator::from_logical_discriminant(16), None);
        assert_eq!(BinaryOperator::LeftDivide.to_string(), "\\");
        assert_eq!(BinaryOperator::ElementLeftDivide.to_string(), ".\\");
    }

    #[test]
    fn verifies_v15_global_and_persistent_instruction_surface() {
        let location = SourceLocation::new(17, 4, 19);
        let mut function = Function::new("stateful", 2, 0, 0).with_persistent_slot_count(2);
        function.constants = vec![Constant::String("shared".to_owned())];
        function.instructions = vec![
            Instruction::located(
                InstructionKind::DeclareGlobal {
                    name: ConstantId::new(0),
                },
                location,
            ),
            Instruction::located(
                InstructionKind::DeclarePersistent {
                    slot: PersistentSlot::new(1),
                },
                location,
            ),
            Instruction::new(InstructionKind::LoadPersistent {
                dst: Register::new(0),
                slot: PersistentSlot::new(1),
            }),
            Instruction::new(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(1),
                src: Register::new(0),
            }),
            Instruction::new(InstructionKind::Return { values: Vec::new() }),
        ];
        let module = BytecodeModule::new(vec![function], FunctionId::new(0));

        assert_eq!(verify(&module), Ok(()));
        assert_eq!(module.functions[0].persistent_slot_count, 2);
        assert_eq!(module.functions[0].instructions[0].location, Some(location));
        assert_eq!(module.functions[0].instructions[1].location, Some(location));
    }

    #[test]
    fn rejects_non_string_or_missing_declare_global_name() {
        let mut function = Function::new("global", 0, 0, 0);
        function.constants = vec![Constant::Double(1.0)];
        function.instructions = vec![Instruction::new(InstructionKind::DeclareGlobal {
            name: ConstantId::new(0),
        })];
        let error = verify(&BytecodeModule::new(
            vec![function.clone()],
            FunctionId::new(0),
        ))
        .expect_err("global declaration names must be strings");
        assert_eq!(
            error.kind,
            VerificationErrorKind::GlobalNameIsNotString {
                constant: ConstantId::new(0),
            }
        );

        function.instructions[0] = Instruction::new(InstructionKind::DeclareGlobal {
            name: ConstantId::new(1),
        });
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("global declaration names must exist");
        assert_eq!(
            error.kind,
            VerificationErrorKind::InvalidConstant {
                constant: ConstantId::new(1),
                count: 1,
            }
        );
    }

    #[test]
    fn rejects_every_out_of_range_persistent_operand() {
        let operations = [
            InstructionKind::DeclarePersistent {
                slot: PersistentSlot::new(1),
            },
            InstructionKind::LoadPersistent {
                dst: Register::new(0),
                slot: PersistentSlot::new(1),
            },
            InstructionKind::StorePersistent {
                slot: PersistentSlot::new(1),
                src: Register::new(0),
            },
        ];

        for operation in operations {
            let mut function = Function::new("stateful", 1, 0, 0).with_persistent_slot_count(1);
            function.instructions = vec![Instruction::new(operation)];
            let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
                .expect_err("persistent operands must be function-bounded");
            assert_eq!(
                error.kind,
                VerificationErrorKind::InvalidPersistentSlot {
                    slot: PersistentSlot::new(1),
                    count: 1,
                }
            );
        }
    }

    #[test]
    fn verifies_persistent_load_and_store_registers() {
        for operation in [
            InstructionKind::LoadPersistent {
                dst: Register::new(3),
                slot: PersistentSlot::new(0),
            },
            InstructionKind::StorePersistent {
                slot: PersistentSlot::new(0),
                src: Register::new(3),
            },
        ] {
            let mut function = valid_function();
            function.persistent_slot_count = 1;
            function.instructions = vec![Instruction::new(operation)];
            let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
                .expect_err("persistent load/store registers must be frame-bounded");
            assert_eq!(
                error.kind,
                VerificationErrorKind::InvalidRegister {
                    register: Register::new(3),
                    count: 3,
                }
            );
        }
    }

    #[test]
    fn verifies_v19_direct_binding_indexing_instructions() {
        for operation in [
            InstructionKind::ApplyBinding {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            },
            InstructionKind::AssignBindingPlace {
                result: Some(Register::new(2)),
                binding: BindingTarget::Workspace(ConstantId::new(1)),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    1,
                ))])],
                source: ValueSource::One(Register::new(0)),
                mode: AssignmentMode::Store,
            },
        ] {
            verify_kind(operation).expect("well-formed direct binding indexing must verify");
        }

        let empty = verify_kind(InstructionKind::AssignBindingPlace {
            result: None,
            binding: BindingTarget::Workspace(ConstantId::new(1)),
            root: Register::new(2),
            path: Vec::new(),
            source: ValueSource::One(Register::new(0)),
            mode: AssignmentMode::Store,
        })
        .expect_err("direct binding assignment must select a place");
        assert_eq!(empty.kind, VerificationErrorKind::EmptyPlacePath);
    }

    #[test]
    fn verifies_v12_aggregate_instruction_surface() {
        let mut function = valid_function();
        function.instructions = vec![
            Instruction::new(InstructionKind::BuildCell {
                dst: Register::new(0),
                rows: Vec::new(),
            }),
            Instruction::new(InstructionKind::BuildCell {
                dst: Register::new(0),
                rows: vec![
                    vec![
                        ValueSource::One(Register::new(0)),
                        ValueSource::Expand(PackRegister::new(0)),
                    ],
                    vec![
                        ValueSource::One(Register::new(1)),
                        ValueSource::Expand(PackRegister::new(1)),
                    ],
                ],
            }),
            Instruction::new(InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                arguments: vec![
                    ApplyArgument::Value(Register::new(1)),
                    ApplyArgument::Colon,
                    ApplyArgument::Expand(PackRegister::new(1)),
                ],
            }),
            Instruction::new(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                field: FieldOperand::Static(ConstantId::new(1)),
            }),
            Instruction::new(InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(1),
                target: Register::new(0),
                field: FieldOperand::Dynamic(Register::new(1)),
            }),
            Instruction::new(InstructionKind::AssignPlace {
                dst: Register::new(2),
                root: Register::new(0),
                path: vec![
                    PlaceStep::Paren(vec![
                        ApplyArgument::Value(Register::new(1)),
                        ApplyArgument::Expand(PackRegister::new(0)),
                    ]),
                    PlaceStep::Brace(vec![ApplyArgument::Colon]),
                    PlaceStep::Field(FieldOperand::Static(ConstantId::new(1))),
                    PlaceStep::Field(FieldOperand::Dynamic(Register::new(2))),
                ],
                source: ValueSource::Expand(PackRegister::new(1)),
                mode: AssignmentMode::Store,
            }),
            Instruction::new(InstructionKind::AssignPlace {
                dst: Register::new(2),
                root: Register::new(0),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(
                    1,
                ))])],
                source: ValueSource::One(Register::new(1)),
                mode: AssignmentMode::Delete,
            }),
            Instruction::new(InstructionKind::ResolveEnd {
                dst: Register::new(1),
                target: Register::new(0),
                argument_index: 1,
                argument_count: 2,
            }),
            Instruction::new(InstructionKind::Unpack {
                outputs: vec![Register::new(1), Register::new(2)],
                pack: PackRegister::new(0),
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Expand(PackRegister::new(1))],
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ];

        assert_eq!(
            verify(&BytecodeModule::new(vec![function], FunctionId::new(0))),
            Ok(())
        );
    }

    #[test]
    fn rejects_pack_register_bounds_in_outputs_arguments_paths_and_sources() {
        let invalid = PackRegister::new(2);
        let instructions = vec![
            InstructionKind::BuildCell {
                dst: Register::new(0),
                rows: vec![vec![ValueSource::Expand(invalid)]],
            },
            InstructionKind::BraceApply {
                dst_pack: invalid,
                target: Register::new(0),
                arguments: Vec::new(),
            },
            InstructionKind::GetAggregateField {
                dst_pack: invalid,
                target: Register::new(0),
                field: FieldOperand::Static(ConstantId::new(1)),
            },
            InstructionKind::Apply {
                outputs: vec![Register::new(0)],
                target: Register::new(1),
                arguments: vec![ApplyArgument::Expand(invalid)],
            },
            InstructionKind::AssignPlace {
                dst: Register::new(0),
                root: Register::new(1),
                path: vec![PlaceStep::Brace(vec![ApplyArgument::Expand(invalid)])],
                source: ValueSource::One(Register::new(2)),
                mode: AssignmentMode::Store,
            },
            InstructionKind::AssignPlace {
                dst: Register::new(0),
                root: Register::new(1),
                path: vec![PlaceStep::Paren(Vec::new())],
                source: ValueSource::Expand(invalid),
                mode: AssignmentMode::Store,
            },
            InstructionKind::Unpack {
                outputs: vec![Register::new(0)],
                pack: invalid,
            },
        ];

        for instruction in instructions {
            let error = verify_kind(instruction).expect_err("pack bound must be verified");
            assert!(matches!(
                error.kind,
                VerificationErrorKind::InvalidPackRegister { register, count: 2 }
                    if register == invalid
            ));
        }
    }

    #[test]
    fn rejects_regular_register_bounds_in_nested_aggregate_operands() {
        let invalid = Register::new(3);
        let instructions = vec![
            InstructionKind::BuildCell {
                dst: Register::new(0),
                rows: vec![vec![ValueSource::One(invalid)]],
            },
            InstructionKind::BraceApply {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(invalid)],
            },
            InstructionKind::GetAggregateField {
                dst_pack: PackRegister::new(0),
                target: Register::new(0),
                field: FieldOperand::Dynamic(invalid),
            },
            InstructionKind::AssignPlace {
                dst: Register::new(0),
                root: Register::new(1),
                path: vec![PlaceStep::Paren(vec![ApplyArgument::Value(invalid)])],
                source: ValueSource::One(Register::new(2)),
                mode: AssignmentMode::Store,
            },
            InstructionKind::AssignPlace {
                dst: Register::new(0),
                root: Register::new(1),
                path: vec![PlaceStep::Brace(Vec::new())],
                source: ValueSource::One(invalid),
                mode: AssignmentMode::Store,
            },
            InstructionKind::ResolveEnd {
                dst: Register::new(0),
                target: invalid,
                argument_index: 0,
                argument_count: 1,
            },
            InstructionKind::Unpack {
                outputs: vec![invalid],
                pack: PackRegister::new(0),
            },
        ];

        for instruction in instructions {
            let error =
                verify_kind(instruction).expect_err("value-register bound must be verified");
            assert!(matches!(
                error.kind,
                VerificationErrorKind::InvalidRegister { register, count: 3 }
                    if register == invalid
            ));
        }
    }

    #[test]
    fn rejects_ragged_cell_rows_with_instruction_location() {
        let location = SourceLocation::new(31, 7, 19);
        let mut function = valid_function();
        function.instructions[0] = Instruction::located(
            InstructionKind::BuildCell {
                dst: Register::new(0),
                rows: vec![vec![ValueSource::One(Register::new(0))], Vec::new()],
            },
            location,
        );
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("ragged encoded cell rows must fail verification");

        assert_eq!(error.location, Some(location));
        assert_eq!(
            error.kind,
            VerificationErrorKind::RaggedCellRows {
                row: 1,
                expected: 1,
                actual: 0,
            }
        );
    }

    #[test]
    fn rejects_empty_place_paths_and_pack_delete_sources() {
        let empty = verify_kind(InstructionKind::AssignPlace {
            dst: Register::new(0),
            root: Register::new(1),
            path: Vec::new(),
            source: ValueSource::One(Register::new(2)),
            mode: AssignmentMode::Store,
        })
        .expect_err("empty aggregate place must fail verification");
        assert_eq!(empty.kind, VerificationErrorKind::EmptyPlacePath);

        let delete = verify_kind(InstructionKind::AssignPlace {
            dst: Register::new(0),
            root: Register::new(1),
            path: vec![PlaceStep::Paren(Vec::new())],
            source: ValueSource::Expand(PackRegister::new(0)),
            mode: AssignmentMode::Delete,
        })
        .expect_err("aggregate deletion cannot consume a pack");
        assert_eq!(delete.kind, VerificationErrorKind::DeleteSourceMustBeOne);
    }

    #[test]
    fn rejects_invalid_end_metadata() {
        let count = verify_kind(InstructionKind::ResolveEnd {
            dst: Register::new(0),
            target: Register::new(1),
            argument_index: 0,
            argument_count: 0,
        })
        .expect_err("zero argument count must fail verification");
        assert_eq!(count.kind, VerificationErrorKind::InvalidEndArgumentCount);

        let index = verify_kind(InstructionKind::ResolveEnd {
            dst: Register::new(0),
            target: Register::new(1),
            argument_index: 2,
            argument_count: 2,
        })
        .expect_err("out-of-range end argument must fail verification");
        assert_eq!(
            index.kind,
            VerificationErrorKind::InvalidEndArgumentIndex {
                argument_index: 2,
                argument_count: 2,
            }
        );
    }

    #[test]
    fn rejects_invalid_static_and_nested_field_operands() {
        let missing = verify_kind(InstructionKind::GetAggregateField {
            dst_pack: PackRegister::new(0),
            target: Register::new(0),
            field: FieldOperand::Static(ConstantId::new(9)),
        })
        .expect_err("missing static field constant must fail verification");
        assert!(matches!(
            missing.kind,
            VerificationErrorKind::InvalidConstant { constant, count: 2 }
                if constant == ConstantId::new(9)
        ));

        let wrong_type = verify_kind(InstructionKind::GetAggregateField {
            dst_pack: PackRegister::new(0),
            target: Register::new(0),
            field: FieldOperand::Static(ConstantId::new(0)),
        })
        .expect_err("non-string static field constant must fail verification");
        assert_eq!(
            wrong_type.kind,
            VerificationErrorKind::FieldNameIsNotString {
                constant: ConstantId::new(0),
            }
        );

        let nested = verify_kind(InstructionKind::AssignPlace {
            dst: Register::new(0),
            root: Register::new(1),
            path: vec![PlaceStep::Field(FieldOperand::Static(ConstantId::new(0)))],
            source: ValueSource::One(Register::new(2)),
            mode: AssignmentMode::Store,
        })
        .expect_err("nested non-string static field must fail verification");
        assert!(matches!(
            nested.kind,
            VerificationErrorKind::FieldNameIsNotString { constant }
                if constant == ConstantId::new(0)
        ));
    }

    #[test]
    fn verifies_switch_match_registers_with_source_location() {
        let location = SourceLocation::new(23, 14, 27);
        let switch_match = |dst, selector, case_value| {
            Instruction::located(
                InstructionKind::SwitchMatch {
                    dst: Register::new(dst),
                    selector: Register::new(selector),
                    case_value: Register::new(case_value),
                },
                location,
            )
        };

        let mut valid = valid_function();
        valid.instructions[1] = switch_match(2, 0, 1);
        assert_eq!(
            verify(&BytecodeModule::new(vec![valid], FunctionId::new(0))),
            Ok(())
        );

        for (dst, selector, case_value) in [(3, 0, 1), (2, 3, 1), (2, 0, 3)] {
            let mut invalid = valid_function();
            invalid.instructions[1] = switch_match(dst, selector, case_value);
            let error = verify(&BytecodeModule::new(vec![invalid], FunctionId::new(0)))
                .expect_err("every switch-match register must be verified");
            assert_eq!(error.location, Some(location));
            assert!(matches!(
                error.kind,
                VerificationErrorKind::InvalidRegister { register, count: 3 }
                    if register == Register::new(3)
            ));
        }
    }

    #[test]
    fn verifies_closure_function_capture_names_and_registers() {
        let mut entry = valid_function();
        entry.instructions[0] = Instruction::new(InstructionKind::MakeClosure {
            dst: Register::new(2),
            function: FunctionId::new(1),
            captures: vec![(ConstantId::new(1), Some(Register::new(0)), true)],
        });
        let closure = valid_function();
        assert_eq!(
            verify(&BytecodeModule::new(
                vec![entry.clone(), closure.clone()],
                FunctionId::new(0)
            )),
            Ok(())
        );

        let location = SourceLocation::new(12, 3, 18);
        entry.instructions[0] = Instruction::located(
            InstructionKind::MakeClosure {
                dst: Register::new(2),
                function: FunctionId::new(2),
                captures: Vec::new(),
            },
            location,
        );
        let error = verify(&BytecodeModule::new(
            vec![entry.clone(), closure.clone()],
            FunctionId::new(0),
        ))
        .expect_err("closure target must exist");
        assert_eq!(error.location, Some(location));
        assert!(matches!(
            error.kind,
            VerificationErrorKind::InvalidClosureFunction { function }
                if function == FunctionId::new(2)
        ));

        entry.instructions[0] = Instruction::new(InstructionKind::MakeClosure {
            dst: Register::new(2),
            function: FunctionId::new(1),
            captures: vec![
                (ConstantId::new(1), None, true),
                (ConstantId::new(1), Some(Register::new(0)), false),
            ],
        });
        let error = verify(&BytecodeModule::new(
            vec![entry, closure],
            FunctionId::new(0),
        ))
        .expect_err("closure capture names must be unique");
        assert!(matches!(
            error.kind,
            VerificationErrorKind::DuplicateClosureCapture { ref name } if name == "x"
        ));
    }

    #[test]
    fn verifies_named_function_handle_register_and_string_name() {
        let mut function = valid_function();
        function.instructions[0] = Instruction::new(InstructionKind::LoadFunctionHandle {
            dst: Register::new(2),
            name: ConstantId::new(1),
        });
        assert_eq!(
            verify(&BytecodeModule::new(
                vec![function.clone()],
                FunctionId::new(0)
            )),
            Ok(())
        );

        let location = SourceLocation::new(11, 4, 17);
        function.instructions[0] = Instruction::located(
            InstructionKind::LoadFunctionHandle {
                dst: Register::new(2),
                name: ConstantId::new(0),
            },
            location,
        );
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("function-handle name must be a string constant");
        assert_eq!(error.location, Some(location));
        assert!(matches!(
            error.kind,
            VerificationErrorKind::GlobalNameIsNotString { constant }
                if constant == ConstantId::new(0)
        ));
    }

    #[test]
    fn rejects_bad_register_with_source_location() {
        let mut function = valid_function();
        let location = SourceLocation::new(7, 10, 12);
        function.instructions[0] = Instruction::located(
            InstructionKind::Move {
                dst: Register::new(3),
                src: Register::new(0),
            },
            location,
        );
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("bad register must be rejected");
        assert_eq!(error.location, Some(location));
        assert!(matches!(
            error.kind,
            VerificationErrorKind::InvalidRegister { register, count: 3 }
                if register == Register::new(3)
        ));
    }

    #[test]
    fn rejects_bad_jump() {
        let mut function = valid_function();
        function.instructions[0] = Instruction::new(InstructionKind::Jump {
            target: InstructionIndex::new(99),
        });
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("bad jump must be rejected");
        assert!(matches!(
            error.kind,
            VerificationErrorKind::InvalidJump { target, count: 3 }
                if target == InstructionIndex::new(99)
        ));
    }

    #[test]
    fn rejects_bad_constant_index() {
        let mut function = valid_function();
        function.instructions[0] = Instruction::new(InstructionKind::LoadConstant {
            dst: Register::new(0),
            constant: ConstantId::new(9),
        });
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("bad constant must be rejected");
        assert!(matches!(
            error.kind,
            VerificationErrorKind::InvalidConstant { constant, count: 2 }
                if constant == ConstantId::new(9)
        ));
    }

    #[test]
    fn rejects_bad_register_nested_in_apply_arguments() {
        let mut function = valid_function();
        function.instructions[0] = Instruction::new(InstructionKind::Apply {
            outputs: vec![Register::new(0)],
            target: Register::new(1),
            arguments: vec![ApplyArgument::Colon, ApplyArgument::Value(Register::new(3))],
        });
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("bad apply register must be rejected");
        assert!(matches!(
            error.kind,
            VerificationErrorKind::InvalidRegister { register, count: 3 }
                if register == Register::new(3)
        ));
    }

    #[test]
    fn rejects_bad_for_each_exit_target() {
        let mut function = valid_function();
        function.instructions[0] = Instruction::new(InstructionKind::ForEach {
            iterable: Register::new(0),
            index: Register::new(1),
            dst: Register::new(2),
            exit: InstructionIndex::new(3),
        });
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("bad for exit must be rejected");
        assert!(matches!(
            error.kind,
            VerificationErrorKind::InvalidJump { target, count: 3 }
                if target == InstructionIndex::new(3)
        ));
    }

    #[test]
    fn verifies_call_metadata_and_ordered_clear_operands() {
        let mut function = valid_function();
        function.instructions = vec![
            Instruction::new(InstructionKind::LoadCallInputCount {
                dst: Register::new(0),
            }),
            Instruction::new(InstructionKind::LoadCallOutputCount {
                dst: Register::new(1),
            }),
            Instruction::new(InstructionKind::ClearGlobal {
                names: vec![ConstantId::new(1), ConstantId::new(1)],
            }),
            Instruction::new(InstructionKind::Return { values: Vec::new() }),
        ];

        assert_eq!(
            verify(&BytecodeModule::new(vec![function], FunctionId::new(0))),
            Ok(())
        );
    }

    #[test]
    fn verifies_variadic_input_and_output_operands_and_marker_position() {
        let mut function = valid_function();
        function.instructions = vec![
            Instruction::new(InstructionKind::LoadVariadicInputs {
                dst: Register::new(0),
            }),
            Instruction::new(InstructionKind::ReturnVariadic {
                fixed: vec![Register::new(1)],
                variadic: Register::new(2),
            }),
        ];
        assert_eq!(
            verify(&BytecodeModule::new(
                vec![function.clone()],
                FunctionId::new(0)
            )),
            Ok(())
        );

        function.instructions.swap(0, 1);
        assert_eq!(
            verify(&BytecodeModule::new(
                vec![function.clone()],
                FunctionId::new(0)
            ))
            .expect_err("late variadic marker must fail")
            .kind,
            VerificationErrorKind::VariadicInputsMustBeFirst
        );

        function.instructions = vec![
            Instruction::new(InstructionKind::LoadVariadicInputs {
                dst: Register::new(0),
            }),
            Instruction::new(InstructionKind::LoadVariadicInputs {
                dst: Register::new(1),
            }),
            Instruction::new(InstructionKind::Return { values: Vec::new() }),
        ];
        assert_eq!(
            verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
                .expect_err("duplicate variadic marker must fail")
                .kind,
            VerificationErrorKind::DuplicateVariadicInputs
        );
    }

    #[test]
    fn rejects_empty_clear_name_lists() {
        let mut function = valid_function();
        function.instructions[0] =
            Instruction::new(InstructionKind::ClearGlobal { names: Vec::new() });
        let error = verify(&BytecodeModule::new(vec![function], FunctionId::new(0)))
            .expect_err("empty clear list must fail verification");

        assert_eq!(error.kind, VerificationErrorKind::EmptyClearNames);
    }

    #[test]
    fn verifies_class_definition_functions_and_registration() {
        let location = SourceLocation::new(4, 0, 12);
        let mut entry = valid_function();
        entry.instructions[0] = Instruction::located(
            InstructionKind::RegisterClass {
                class: ClassDefinitionId::new(0),
            },
            location,
        );
        let default = Function {
            name: "Point.<default:x>".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![Constant::Double(1.0)],
            instructions: vec![
                Instruction::new(InstructionKind::LoadConstant {
                    dst: Register::new(0),
                    constant: ConstantId::new(0),
                }),
                Instruction::new(InstructionKind::Return {
                    values: vec![Register::new(0)],
                }),
            ],
            exception_handlers: Vec::new(),
        };
        let method = Function {
            name: "Point.read".to_owned(),
            register_count: 0,
            pack_register_count: 0,
            local_count: 1,
            persistent_slot_count: 0,
            parameter_count: 1,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: Vec::new(),
            })],
            exception_handlers: Vec::new(),
        };
        let class = ClassDefinition {
            name: "Point".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: Some("Shape".to_owned()),
            declared_abstract: None,
            sealed: false,
            properties: vec![PropertyDefinition {
                name: "x".to_owned(),
                kind: PropertyKind::Stored,
                default: Some(FunctionId::new(1)),
                get_access: Some(Access::Protected),
                set_access: Some(Access::Private),
                location,
            }],
            methods: vec![MethodDefinition {
                name: "read".to_owned(),
                function: FunctionId::new(2),
                kind: MethodKind::Instance,
                access: Access::Protected,
                is_abstract: false,
                is_external: false,
                constructor_output: None,
                location,
            }],
            location,
        };
        let module = BytecodeModule::new(vec![entry, default, method], FunctionId::new(0))
            .with_classes(vec![class]);

        assert_eq!(verify(&module), Ok(()));
    }

    #[test]
    fn verifies_abstract_class_metadata_and_rejects_invalid_slots() {
        let location = SourceLocation::new(5, 2, 18);
        let signature = Function {
            name: "AbstractBase.transform".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 2,
            persistent_slot_count: 0,
            parameter_count: 2,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: vec![Register::new(0)],
            })],
            exception_handlers: Vec::new(),
        };
        let class = ClassDefinition {
            name: "AbstractBase".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: None,
            declared_abstract: Some(true),
            sealed: true,
            properties: Vec::new(),
            methods: vec![MethodDefinition {
                name: "transform".to_owned(),
                function: FunctionId::new(1),
                kind: MethodKind::Static,
                access: Access::Protected,
                is_abstract: true,
                is_external: false,
                constructor_output: None,
                location,
            }],
            location,
        };
        let module = BytecodeModule::new(vec![valid_function(), signature], FunctionId::new(0))
            .with_classes(vec![class]);

        assert_eq!(verify(&module), Ok(()));
        assert_eq!(verify(&module.clone()), Ok(()));

        let mut explicit_concrete = module.clone();
        explicit_concrete.classes[0].declared_abstract = Some(false);
        assert!(matches!(
            verify(&explicit_concrete).expect_err("explicit concrete class must reject slots").kind,
            VerificationErrorKind::ExplicitConcreteHasAbstractMethod { class, method }
                if class == "AbstractBase" && method == "transform"
        ));

        let mut constructor = module.clone();
        constructor.classes[0].methods[0].name = "AbstractBase".to_owned();
        assert!(matches!(
            verify(&constructor).expect_err("abstract constructor must fail").kind,
            VerificationErrorKind::AbstractConstructor { class }
                if class == "AbstractBase"
        ));

        let mut seeded = module;
        seeded.classes[0].methods[0].kind = MethodKind::Instance;
        seeded.classes[0].methods[0].constructor_output = Some(LocalSlot::new(0));
        assert!(matches!(
            verify(&seeded).expect_err("abstract slot cannot seed an object").kind,
            VerificationErrorKind::AbstractMethodHasConstructorOutput { class, method }
                if class == "AbstractBase" && method == "transform"
        ));
    }

    #[test]
    fn rejects_missing_class_method_function_with_member_location() {
        let location = SourceLocation::new(9, 20, 30);
        let class = ClassDefinition {
            name: "Broken".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: Vec::new(),
            methods: vec![MethodDefinition {
                name: "run".to_owned(),
                function: FunctionId::new(99),
                kind: MethodKind::Instance,
                access: Access::Public,
                is_abstract: false,
                is_external: false,
                constructor_output: None,
                location,
            }],
            location,
        };
        let error = verify(
            &BytecodeModule::new(vec![valid_function()], FunctionId::new(0))
                .with_classes(vec![class]),
        )
        .expect_err("missing class method function must fail verification");

        assert_eq!(error.location, Some(location));
        assert!(matches!(
            error.kind,
            VerificationErrorKind::InvalidClassFunction { function, .. }
                if function == FunctionId::new(99)
        ));
    }

    #[test]
    fn verifies_constant_and_static_member_contracts() {
        let location = SourceLocation::new(12, 4, 24);
        let initializer = Function {
            name: "Scale.<constant:Factor>".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![Constant::Double(3.0)],
            instructions: vec![
                Instruction::new(InstructionKind::LoadConstant {
                    dst: Register::new(0),
                    constant: ConstantId::new(0),
                }),
                Instruction::new(InstructionKind::Return {
                    values: vec![Register::new(0)],
                }),
            ],
            exception_handlers: Vec::new(),
        };
        let method = Function {
            name: "Scale.scale".to_owned(),
            register_count: 0,
            pack_register_count: 0,
            local_count: 1,
            persistent_slot_count: 0,
            parameter_count: 1,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: Vec::new(),
            })],
            exception_handlers: Vec::new(),
        };
        let class = ClassDefinition {
            name: "Scale".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: vec![PropertyDefinition {
                name: "Factor".to_owned(),
                kind: PropertyKind::Constant,
                default: Some(FunctionId::new(1)),
                get_access: Some(Access::Public),
                set_access: None,
                location,
            }],
            methods: vec![MethodDefinition {
                name: "scale".to_owned(),
                function: FunctionId::new(2),
                kind: MethodKind::Static,
                access: Access::Public,
                is_abstract: false,
                is_external: false,
                constructor_output: None,
                location,
            }],
            location,
        };
        let module = BytecodeModule::new(
            vec![valid_function(), initializer, method],
            FunctionId::new(0),
        )
        .with_classes(vec![class]);

        assert_eq!(verify(&module), Ok(()));
    }

    #[test]
    fn verifies_dependent_accessors_and_plus_signatures() {
        let location = SourceLocation::new(14, 3, 30);
        let special_function = |name: &str, parameters: u32| Function {
            name: name.to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: parameters,
            persistent_slot_count: 0,
            parameter_count: parameters,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: vec![Register::new(0)],
            })],
            exception_handlers: Vec::new(),
        };
        let method = |name: &str, function: u32| MethodDefinition {
            name: name.to_owned(),
            function: FunctionId::new(function),
            kind: MethodKind::Instance,
            access: Access::Public,
            is_abstract: false,
            is_external: false,
            constructor_output: None,
            location,
        };
        let class = ClassDefinition {
            name: "Box".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: vec![PropertyDefinition {
                name: "Twice".to_owned(),
                kind: PropertyKind::Dependent,
                default: None,
                get_access: Some(Access::Public),
                set_access: Some(Access::Public),
                location,
            }],
            methods: vec![
                method("get.Twice", 1),
                method("set.Twice", 2),
                method("plus", 3),
            ],
            location,
        };
        let module = BytecodeModule::new(
            vec![
                valid_function(),
                special_function("Box.get.Twice", 1),
                special_function("Box.set.Twice", 2),
                special_function("Box.plus", 2),
            ],
            FunctionId::new(0),
        )
        .with_classes(vec![class]);

        assert_eq!(verify(&module), Ok(()));
    }

    #[test]
    fn rejects_missing_or_malformed_dependent_accessors() {
        let location = SourceLocation::new(15, 5, 35);
        let getter = MethodDefinition {
            name: "get.Value".to_owned(),
            function: FunctionId::new(1),
            kind: MethodKind::Instance,
            access: Access::Public,
            is_abstract: false,
            is_external: false,
            constructor_output: None,
            location,
        };
        let class = |methods| ClassDefinition {
            name: "BrokenDependent".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: vec![PropertyDefinition {
                name: "Value".to_owned(),
                kind: PropertyKind::Dependent,
                default: None,
                get_access: Some(Access::Public),
                set_access: Some(Access::Public),
                location,
            }],
            methods,
            location,
        };
        let bad_getter = Function {
            name: "BrokenDependent.get.Value".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: vec![Register::new(0)],
            })],
            exception_handlers: Vec::new(),
        };

        let malformed = verify(
            &BytecodeModule::new(vec![valid_function(), bad_getter], FunctionId::new(0))
                .with_classes(vec![class(vec![getter.clone()])]),
        )
        .expect_err("getter input arity must be verified");
        assert!(matches!(
            malformed.kind,
            VerificationErrorKind::InvalidSpecialMethodSignature {
                expected_parameters: 1,
                actual_parameters: 0,
                ..
            }
        ));

        let valid_getter = Function {
            name: "BrokenDependent.get.Value".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 1,
            persistent_slot_count: 0,
            parameter_count: 1,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: vec![Register::new(0)],
            })],
            exception_handlers: Vec::new(),
        };
        let missing = verify(
            &BytecodeModule::new(vec![valid_function(), valid_getter], FunctionId::new(0))
                .with_classes(vec![class(vec![getter])]),
        )
        .expect_err("writable dependent property must encode a setter");
        assert!(matches!(
            missing.kind,
            VerificationErrorKind::MissingDependentAccessor { accessor, .. }
                if accessor == "set.Value"
        ));
    }

    #[test]
    fn rejects_constant_without_initializer_or_with_write_access() {
        let location = SourceLocation::new(13, 2, 9);
        let class = |default, set_access| ClassDefinition {
            name: "BrokenConstant".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: vec![PropertyDefinition {
                name: "Factor".to_owned(),
                kind: PropertyKind::Constant,
                default,
                get_access: Some(Access::Public),
                set_access,
                location,
            }],
            methods: Vec::new(),
            location,
        };

        let missing = verify(
            &BytecodeModule::new(vec![valid_function()], FunctionId::new(0))
                .with_classes(vec![class(None, None)]),
        )
        .expect_err("constant initializer is required");
        assert!(matches!(
            missing.kind,
            VerificationErrorKind::MissingConstantInitializer { .. }
        ));

        let writable = verify(
            &BytecodeModule::new(vec![valid_function()], FunctionId::new(0))
                .with_classes(vec![class(None, Some(Access::Public))]),
        )
        .expect_err("constant write access is forbidden");
        assert!(matches!(
            writable.kind,
            VerificationErrorKind::InvalidPropertyWriteAccess {
                kind: PropertyKind::Constant,
                ..
            }
        ));
    }

    #[test]
    fn rejects_empty_and_self_superclass_names() {
        let location = SourceLocation::new(11, 2, 18);
        let class = |superclass: &str| ClassDefinition {
            name: "Loop".to_owned(),
            semantics: ClassSemantics::Value,
            superclass: Some(superclass.to_owned()),
            declared_abstract: None,
            sealed: false,
            properties: Vec::new(),
            methods: Vec::new(),
            location,
        };
        let module = |class| {
            BytecodeModule::new(vec![valid_function()], FunctionId::new(0))
                .with_classes(vec![class])
        };

        let empty = verify(&module(class(""))).expect_err("empty superclass must fail");
        assert_eq!(empty.location, Some(location));
        assert!(matches!(
            empty.kind,
            VerificationErrorKind::EmptySuperclassName { class } if class == "Loop"
        ));

        let cyclic = verify(&module(class("Loop"))).expect_err("self superclass must fail");
        assert_eq!(cyclic.location, Some(location));
        assert!(matches!(
            cyclic.kind,
            VerificationErrorKind::SelfSuperclass { class } if class == "Loop"
        ));
    }

    #[test]
    fn accepts_v24_packages_v25_imports_and_reserves_call_targets_for_v26() {
        let location = SourceLocation::new(31, 4, 40);
        let class = ClassDefinition {
            name: "Signals".to_owned(),
            semantics: ClassSemantics::Handle,
            superclass: None,
            declared_abstract: None,
            sealed: false,
            properties: Vec::new(),
            methods: Vec::new(),
            location,
        };
        let initializer = Function {
            name: "Signals.<enumeration:Ready>".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: vec![Register::new(0)],
            })],
            exception_handlers: Vec::new(),
        };
        let features = vec![ClassFeatureDefinition {
            class: ClassDefinitionId::new(0),
            kind: ClassKind::Enumeration,
            enumeration_base: Some("handle".to_owned()),
            enumeration_members: vec![EnumMemberDefinition {
                name: "Ready".to_owned(),
                argument_initializer: FunctionId::new(1),
                argument_count: 1,
                location,
            }],
            events: vec![EventDefinition {
                name: "Changed".to_owned(),
                listen_access: Access::Protected,
                notify_access: Access::Private,
                hidden: true,
                location,
            }],
            abstract_properties: Vec::new(),
            sealed_methods: Vec::new(),
            location,
        }];
        let module = BytecodeModule::new(vec![valid_function(), initializer], FunctionId::new(0))
            .with_classes(vec![class])
            .with_class_features(features);
        assert_eq!(verify(&module), Ok(()));

        let mut legacy_with_class_features = module;
        legacy_with_class_features.version = MIN_SUPPORTED_BYTECODE_VERSION;
        assert_eq!(verify(&legacy_with_class_features), Ok(()));

        let mut mutable_imports = valid_function();
        mutable_imports.instructions[0] = Instruction::new(InstructionKind::ClearImports);
        let mut invalid_legacy = BytecodeModule::new(vec![mutable_imports], FunctionId::new(0));
        invalid_legacy.version = MIN_SUPPORTED_BYTECODE_VERSION;
        assert!(matches!(
            verify(&invalid_legacy).unwrap_err().kind,
            VerificationErrorKind::ImportClearRequiresCurrentVersion { .. }
        ));

        let mut call_target = valid_function();
        call_target.instructions[0] = Instruction::new(InstructionKind::LoadCallTarget {
            dst: Register::new(0),
            name: ConstantId::new(1),
        });
        let mut invalid_v25 = BytecodeModule::new(vec![call_target], FunctionId::new(0));
        invalid_v25.version = BytecodeVersion::new(25, 0);
        assert!(matches!(
            verify(&invalid_v25).unwrap_err().kind,
            VerificationErrorKind::CallTargetRequiresCurrentVersion { .. }
        ));
    }

    fn superclass_constructor_module(superclass: &str, object: LocalSlot) -> BytecodeModule {
        let location = SourceLocation::new(12, 30, 55);
        let constructor = Function {
            name: "Derived.Derived".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 2,
            persistent_slot_count: 0,
            parameter_count: 1,
            argument_layout: None,
            constants: vec![Constant::String(superclass.to_owned())],
            instructions: vec![
                Instruction::located(
                    InstructionKind::InvokeSuperclassConstructor {
                        superclass: ConstantId::new(0),
                        object,
                        arguments: vec![Register::new(0)],
                    },
                    location,
                ),
                Instruction::new(InstructionKind::Return { values: Vec::new() }),
            ],
            exception_handlers: Vec::new(),
        };
        BytecodeModule::new(vec![valid_function(), constructor], FunctionId::new(0)).with_classes(
            vec![ClassDefinition {
                name: "Derived".to_owned(),
                semantics: ClassSemantics::Value,
                superclass: Some("Base".to_owned()),
                declared_abstract: None,
                sealed: false,
                properties: Vec::new(),
                methods: vec![MethodDefinition {
                    name: "Derived".to_owned(),
                    function: FunctionId::new(1),
                    kind: MethodKind::Instance,
                    access: Access::Public,
                    is_abstract: false,
                    is_external: false,
                    constructor_output: Some(LocalSlot::new(1)),
                    location,
                }],
                location,
            }],
        )
    }

    #[test]
    fn verifies_direct_superclass_constructor_context_target_and_object() {
        assert_eq!(
            verify(&superclass_constructor_module("Base", LocalSlot::new(1))),
            Ok(())
        );

        let target = verify(&superclass_constructor_module(
            "NotTheDirectBase",
            LocalSlot::new(1),
        ))
        .expect_err("non-direct superclass constructor must fail");
        assert!(matches!(
            target.kind,
            VerificationErrorKind::InvalidSuperclassConstructorTarget {
                class,
                expected: Some(expected),
                actual,
            } if class == "Derived" && expected == "Base" && actual == "NotTheDirectBase"
        ));

        let object = verify(&superclass_constructor_module("Base", LocalSlot::new(0)))
            .expect_err("constructor must use its seeded object output");
        assert!(matches!(
            object.kind,
            VerificationErrorKind::InvalidSuperclassConstructorObject {
                class,
                expected,
                actual,
            } if class == "Derived"
                && expected == LocalSlot::new(1)
                && actual == LocalSlot::new(0)
        ));
    }

    #[test]
    fn rejects_superclass_constructor_instruction_outside_constructor() {
        let mut module = superclass_constructor_module("Base", LocalSlot::new(1));
        module.classes[0].methods[0].name = "ordinaryMethod".to_owned();
        module.classes[0].methods[0].constructor_output = None;

        let error = verify(&module).expect_err("ordinary method must not invoke a constructor");
        assert!(matches!(
            error.kind,
            VerificationErrorKind::SuperclassConstructorOutsideConstructor { superclass }
                if superclass == "Base"
        ));
    }
}

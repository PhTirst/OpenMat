use std::fmt;

/// The bytecode format version emitted and accepted by this release.
pub const CURRENT_BYTECODE_VERSION: BytecodeVersion = BytecodeVersion::new(29, 0);

/// Oldest logical module schema accepted by the in-memory compatibility path.
///
/// Version 24 modules predate mutable base-workspace imports, external
/// class-folder method bodies, and argument-sensitive named-call dispatch.
pub const MIN_SUPPORTED_BYTECODE_VERSION: BytecodeVersion = BytecodeVersion::new(24, 0);

/// A version for the serialized bytecode contract.
///
/// Bytecode serialization is intentionally not defined by Rust's in-memory
/// layout. A future protocol encoder must write these two fields explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BytecodeVersion {
    /// Incompatible format generation.
    pub major: u16,
    /// Backwards-compatible revision within a generation.
    pub minor: u16,
}

impl BytecodeVersion {
    /// Creates a bytecode version.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl fmt::Display for BytecodeVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

macro_rules! index_type {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            /// Creates an index from its format-level integer representation.
            #[must_use]
            pub const fn new(index: u32) -> Self {
                Self(index)
            }

            /// Returns the format-level integer representation.
            #[must_use]
            pub const fn get(self) -> u32 {
                self.0
            }
        }

        impl From<u32> for $name {
            fn from(index: u32) -> Self {
                Self::new(index)
            }
        }
    };
}

index_type!(Register, "A register in a function execution frame.");
index_type!(
    PackRegister,
    "A value-pack register in a function execution frame."
);
index_type!(
    LocalSlot,
    "A local-variable slot in a function execution frame."
);
index_type!(
    PersistentSlot,
    "A function-owned persistent-storage slot, bounded by its function's declared slot count."
);
index_type!(ConstantId, "An index into a function's constant pool.");
index_type!(
    FunctionId,
    "An index into a bytecode module's function table."
);
index_type!(
    InstructionIndex,
    "An index into a function's instruction stream."
);
index_type!(
    ClassDefinitionId,
    "An index into a bytecode module's class-definition table."
);

/// A source identifier and half-open byte range associated with an instruction.
///
/// This deliberately mirrors, rather than depends on, the source crate's
/// eventual identifier type. The HIR-to-bytecode adapter is responsible for the
/// checked conversion once that shared interface is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    /// Compiler-assigned source identifier.
    pub source_id: u32,
    /// Inclusive UTF-8 byte offset.
    pub start: u32,
    /// Exclusive UTF-8 byte offset.
    pub end: u32,
}

impl SourceLocation {
    /// Creates a source location.
    #[must_use]
    pub const fn new(source_id: u32, start: u32, end: u32) -> Self {
        Self {
            source_id,
            start,
            end,
        }
    }
}

/// A value embedded in a function's constant pool.
#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    /// `OpenMat`'s internal no-value marker.
    Nothing,
    /// A scalar logical value.
    Logical(bool),
    /// A scalar IEEE-754 double.
    Double(f64),
    /// A scalar complex double, stored as real and imaginary parts.
    Complex { real: f64, imaginary: f64 },
    /// A MATLAB character literal stored as exact UTF-16 code units.
    ///
    /// An empty payload denotes the `0 x 0` empty char array; every non-empty
    /// payload denotes a `1 x N` row vector.
    Char(Vec<u16>),
    /// A scalar string value.
    String(String),
    /// A reference to a function in the containing bytecode module.
    Function(FunctionId),
}

/// A binary arithmetic or comparison operation.
///
/// The logical serialization code is defined by [`Self::logical_discriminant`]
/// and is deliberately independent of this Rust enum's declaration order and
/// in-memory discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    /// Scalar addition.
    Add,
    /// Scalar subtraction.
    Subtract,
    /// Scalar multiplication.
    Multiply,
    /// Element-wise multiplication with scalar expansion.
    ElementMultiply,
    /// Scalar right division.
    Divide,
    /// Scalar or matrix left division.
    LeftDivide,
    /// Element-wise right division with scalar expansion.
    ElementDivide,
    /// Element-wise left division with scalar expansion.
    ElementLeftDivide,
    /// Scalar power.
    Power,
    /// Element-wise power with scalar expansion.
    ElementPower,
    /// Equality comparison.
    Equal,
    /// Inequality comparison.
    NotEqual,
    /// Less-than comparison.
    LessThan,
    /// Less-than-or-equal comparison.
    LessThanOrEqual,
    /// Greater-than comparison.
    GreaterThan,
    /// Greater-than-or-equal comparison.
    GreaterThanOrEqual,
}

impl BinaryOperator {
    /// Returns the explicit format-level discriminant for this operator.
    ///
    /// Existing v15 operators retain codes `0` through `13`; bytecode v16 adds
    /// left division as codes `14` and `15`. Encoders must call this method
    /// rather than cast the Rust enum.
    #[must_use]
    pub const fn logical_discriminant(self) -> u8 {
        match self {
            Self::Add => 0,
            Self::Subtract => 1,
            Self::Multiply => 2,
            Self::ElementMultiply => 3,
            Self::Divide => 4,
            Self::ElementDivide => 5,
            Self::Power => 6,
            Self::ElementPower => 7,
            Self::Equal => 8,
            Self::NotEqual => 9,
            Self::LessThan => 10,
            Self::LessThanOrEqual => 11,
            Self::GreaterThan => 12,
            Self::GreaterThanOrEqual => 13,
            Self::LeftDivide => 14,
            Self::ElementLeftDivide => 15,
        }
    }

    /// Decodes an explicit format-level operator discriminant.
    #[must_use]
    pub const fn from_logical_discriminant(discriminant: u8) -> Option<Self> {
        match discriminant {
            0 => Some(Self::Add),
            1 => Some(Self::Subtract),
            2 => Some(Self::Multiply),
            3 => Some(Self::ElementMultiply),
            4 => Some(Self::Divide),
            5 => Some(Self::ElementDivide),
            6 => Some(Self::Power),
            7 => Some(Self::ElementPower),
            8 => Some(Self::Equal),
            9 => Some(Self::NotEqual),
            10 => Some(Self::LessThan),
            11 => Some(Self::LessThanOrEqual),
            12 => Some(Self::GreaterThan),
            13 => Some(Self::GreaterThanOrEqual),
            14 => Some(Self::LeftDivide),
            15 => Some(Self::ElementLeftDivide),
            _ => None,
        }
    }
}

impl fmt::Display for BinaryOperator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::ElementMultiply => ".*",
            Self::Divide => "/",
            Self::LeftDivide => "\\",
            Self::ElementDivide => "./",
            Self::ElementLeftDivide => ".\\",
            Self::Power => "^",
            Self::ElementPower => ".^",
            Self::Equal => "==",
            Self::NotEqual => "~=",
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
        };
        formatter.write_str(text)
    }
}

/// One argument to unresolved MATLAB parenthesized application syntax.
///
/// A colon is represented in bytecode instead of as a dynamic runtime value,
/// because it is an indexing descriptor rather than an ordinary value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplyArgument {
    /// An ordinary evaluated argument.
    Value(Register),
    /// All values produced in a comma-separated value pack.
    Expand(PackRegister),
    /// MATLAB's all-elements index (`:`).
    Colon,
}

/// One scalar value or an expanded comma-separated value pack.
///
/// Value packs are deliberately not ordinary language values. Only operands
/// which explicitly accept this type can consume a [`PackRegister`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueSource {
    /// One ordinary language value.
    One(Register),
    /// All values in a comma-separated value pack.
    Expand(PackRegister),
}

/// Storage used for the implicit result of an expression-statement application.
///
/// MATLAB calls an unassigned function with zero requested outputs, but still
/// binds the first value actually returned to `ans`. Keeping this destination
/// in the instruction avoids conflating caller `nargout` with returned-value
/// capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementResultTarget {
    /// The workspace `ans` binding, named by a string constant.
    Global(ConstantId),
    /// A function-workspace `ans` local.
    Local(LocalSlot),
}

/// A statically resolved mutable language binding.
///
/// The operand identifies where an aggregate assignment must commit its final
/// value. Keeping this distinction in bytecode lets the runtime consume a
/// captured root without losing local, persistent, closure, or workspace
/// identity and without retaining a VM-only array alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingTarget {
    /// A frame-local slot.
    Local(LocalSlot),
    /// A function-owned persistent slot.
    Persistent(PersistentSlot),
    /// A shared lexical capture named by a string constant.
    Capture(ConstantId),
    /// An interpreter-session workspace binding named by a string constant.
    Workspace(ConstantId),
}

/// Storage identity exposed to current-scope built-ins such as load and save.
///
/// The corresponding name is carried by the declaring instruction as a
/// string constant. This metadata does not itself create or initialize a
/// language binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedBindingKind {
    /// A frame-local slot.
    Local(LocalSlot),
    /// A function-owned persistent slot.
    Persistent(PersistentSlot),
    /// A shared lexical capture with the declared name.
    Capture,
    /// An interpreter-session workspace global with the declared name.
    Workspace,
}

/// A static or dynamically evaluated aggregate field name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldOperand {
    /// A field name stored as a string constant.
    Static(ConstantId),
    /// A field name evaluated into an ordinary value register.
    Dynamic(Register),
}

/// One selector in a name-rooted aggregate assignment path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceStep {
    /// Parenthesized indexing or unresolved application syntax.
    Paren(Vec<ApplyArgument>),
    /// Cell-content indexing syntax.
    Brace(Vec<ApplyArgument>),
    /// Static or dynamic field selection.
    Field(FieldOperand),
}

/// Application target for a dynamically sized output pack (bytecode v27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackApplyTarget {
    /// Ordinary unresolved call or indexing.
    Value(Register),
    /// Method dispatch or property indexing.
    Field { object: Register, name: ConstantId },
    /// Package-qualified call or member traversal.
    Qualified {
        target: Register,
        unresolved_suffix: Register,
        members: Vec<ConstantId>,
    },
}

/// The update performed by [`InstructionKind::AssignPlace`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssignmentMode {
    /// Store one value or an expanded value pack into the selected place.
    Store,
    /// Delete the selected aggregate elements.
    ///
    /// Deletion uses one ordinary register as its deletion-token source; a
    /// pack source is structurally invalid bytecode.
    Delete,
}

/// Language assignment semantics declared by a bytecode class definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassSemantics {
    /// Instances are copied when assigned or passed as arguments.
    Value,
    /// Instances retain identity and alias their shared state.
    Handle,
}

/// Language access level serialized for class members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    /// Accessible from every caller.
    Public,
    /// Accessible from the declaring class and its subclasses.
    Protected,
    /// Accessible only from the declaring class.
    Private,
}

/// Invocation form of a class method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodKind {
    /// A method invoked with an object receiver.
    Instance,
    /// A class-wide method invoked without constructing an instance.
    Static,
}

/// Storage form of a property carried by a versioned class definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyKind {
    /// A per-instance, writable storage slot.
    Stored,
    /// A class-wide, read-only value initialized once when the class registers.
    Constant,
    /// A per-instance virtual property implemented by conventional accessor
    /// methods and carrying no object storage slot.
    Dependent,
}

/// One stored, constant, or dependent property in a versioned class definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyDefinition {
    /// Language-visible property name.
    pub name: String,
    /// Per-instance storage or class-wide constant semantics.
    pub kind: PropertyKind,
    /// Zero-argument function used to evaluate an explicit default value.
    pub default: Option<FunctionId>,
    /// Visibility of reads of this property. A write-only dependent property
    /// encodes `None` because it has no getter.
    pub get_access: Option<Access>,
    /// Visibility of writes to a stored or dependent property. Constants and
    /// read-only dependent properties encode `None`.
    pub set_access: Option<Access>,
    /// Source range of the property declaration.
    pub location: SourceLocation,
}

/// One method in a versioned class definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDefinition {
    /// Language-visible method name.
    pub name: String,
    /// Executable function body in the containing module.
    pub function: FunctionId,
    /// Instance or class-wide invocation form.
    pub kind: MethodKind,
    /// Visibility of calls to this method.
    pub access: Access,
    /// Whether this entry declares a body-less abstract dispatch slot. The
    /// referenced function carries only signature metadata and is never run.
    pub is_abstract: bool,
    /// Whether the executable body is supplied by a sibling method file in the
    /// owning `@Class` directory. The compiler emits signature metadata first;
    /// the kernel linker must replace `function` before runtime registration.
    pub is_external: bool,
    /// Constructor output slot initially seeded with the allocated object.
    pub constructor_output: Option<LocalSlot>,
    /// Source range of the method declaration.
    pub location: SourceLocation,
}

/// A runtime-registerable class description carried by bytecode.
///
/// This is a format-level contract and must be serialized field-by-field by a
/// future protocol encoder; its Rust layout is not an ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDefinition {
    /// Language-visible class name.
    pub name: String,
    /// Value or handle assignment behavior.
    pub semantics: ClassSemantics,
    /// Direct user-defined superclass name. Direct derivation from the built-in
    /// `handle` class is represented by [`ClassSemantics::Handle`] instead.
    pub superclass: Option<String>,
    /// Explicit `Abstract` value, or `None` when effective abstractness is
    /// inferred from unresolved slots at class-link time.
    pub declared_abstract: Option<bool>,
    /// Whether subclasses are forbidden.
    pub sealed: bool,
    /// Stored properties in source order.
    pub properties: Vec<PropertyDefinition>,
    /// Methods in source order.
    pub methods: Vec<MethodDefinition>,
    /// Source range of the complete class definition.
    pub location: SourceLocation,
}

/// Whether a class uses ordinary construction or declares enumeration members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassKind {
    Ordinary,
    Enumeration,
}

/// One enumeration member and the zero-input function that evaluates its
/// constructor arguments in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumMemberDefinition {
    pub name: String,
    pub argument_initializer: FunctionId,
    pub argument_count: u32,
    pub location: SourceLocation,
}

/// Event access metadata required by future `notify` and `addlistener`
/// execution. Callback storage and listener identity remain runtime-owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDefinition {
    pub name: String,
    pub listen_access: Access,
    pub notify_access: Access,
    pub hidden: bool,
    pub location: SourceLocation,
}

/// A body-less abstract property slot linked by name at class-registration
/// time. At least one of the two access entries must be present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbstractPropertyDefinition {
    pub name: String,
    pub kind: PropertyKind,
    pub get_access: Option<Access>,
    pub set_access: Option<Access>,
    pub location: SourceLocation,
}

/// Version-22 class metadata kept outside [`ClassDefinition`] so version-21
/// in-memory producers can continue constructing the old record unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassFeatureDefinition {
    pub class: ClassDefinitionId,
    pub kind: ClassKind,
    /// Raw explicit base named by an enumeration class. It is not linked as an
    /// ordinary user superclass by the version-21 runtime class table.
    pub enumeration_base: Option<String>,
    pub enumeration_members: Vec<EnumMemberDefinition>,
    pub events: Vec<EventDefinition>,
    pub abstract_properties: Vec<AbstractPropertyDefinition>,
    /// Names of local methods whose overrides are forbidden, including legal
    /// abstract sealed slots.
    pub sealed_methods: Vec<String>,
    pub location: SourceLocation,
}

/// The action taken after an error reaches a protected instruction range.
///
/// This is a format-level discriminated union. Serializers must encode the
/// variants explicitly instead of exposing the Rust enum layout as an ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExceptionHandlerKind {
    /// Enter a MATLAB `catch` body.
    Catch {
        /// First instruction in the catch body.
        handler: InstructionIndex,
        /// Surrounding-scope local which receives the structured error value.
        /// `None` implements plain `catch`; `Some` implements `catch name`.
        error_local: Option<LocalSlot>,
    },
    /// Discard the error and continue, implementing `try ... end` without
    /// a catch clause.
    Swallow,
}

/// One function-owned protected instruction range and its error continuation.
///
/// `protected_start..protected_end` is half-open. The runtime selects the
/// unique innermost range containing the instruction which raised the error.
/// For an error propagated from a called function, that instruction is the
/// suspended call/apply instruction in this function.
///
/// The compiler emission layout and all structural restrictions are enforced
/// by [`crate::verify`]. The optional location describes the complete source
/// `try` statement and is diagnostic metadata, not part of handler selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExceptionHandler {
    /// First protected instruction, inclusive.
    pub protected_start: InstructionIndex,
    /// First instruction after the protected body, exclusive.
    pub protected_end: InstructionIndex,
    /// Catch or no-catch behavior on an error.
    pub kind: ExceptionHandlerKind,
    /// First instruction after the complete `try` construct.
    pub exit: InstructionIndex,
    /// Source location of the complete `try` statement, when available.
    pub location: Option<SourceLocation>,
}

impl ExceptionHandler {
    /// Creates metadata for `try` followed by `catch` or `catch name`.
    #[must_use]
    pub const fn catch(
        protected_start: InstructionIndex,
        protected_end: InstructionIndex,
        handler: InstructionIndex,
        exit: InstructionIndex,
        error_local: Option<LocalSlot>,
    ) -> Self {
        Self {
            protected_start,
            protected_end,
            kind: ExceptionHandlerKind::Catch {
                handler,
                error_local,
            },
            exit,
            location: None,
        }
    }

    /// Creates metadata for no-catch `try ... end` error swallowing.
    #[must_use]
    pub const fn swallow(
        protected_start: InstructionIndex,
        protected_end: InstructionIndex,
        exit: InstructionIndex,
    ) -> Self {
        Self {
            protected_start,
            protected_end,
            kind: ExceptionHandlerKind::Swallow,
            exit,
            location: None,
        }
    }

    /// Attaches the source location of the complete `try` statement.
    #[must_use]
    pub const fn with_location(mut self, location: SourceLocation) -> Self {
        self.location = Some(location);
        self
    }
}

/// The executable part of an instruction.
///
/// Variants and operands form a versioned logical contract. A future encoder
/// must serialize them explicitly; this Rust enum's layout is not an ABI.
#[derive(Debug, Clone, PartialEq)]
pub enum InstructionKind {
    /// Rejects an undefined value at an actual language read, not when a frame
    /// is created. Raw local loads remain available for existence tests,
    /// capture construction, output collection, and indexed initialization.
    RequireDefined { value: Register, name: ConstantId },
    /// Requires a scalar intermediate value when traversing an assignment
    /// prefix. Unlike Unpack, a comma-separated list is not truncated.
    RequireSinglePack { pack: PackRegister },
    /// Counts comma-separated assignment destinations without reading the
    /// final field/content. Selectors have already been evaluated exactly once.
    CountPlaceOutputs {
        dst: Register,
        root: Register,
        path: Vec<PlaceStep>,
    },
    /// Requests a runtime-sized number of outputs and stores them in a pack.
    ApplyOutputPack {
        dst_pack: PackRegister,
        count: Register,
        target: PackApplyTarget,
        arguments: Vec<ApplyArgument>,
    },
    /// Copies a zero-based slice of a pack; count and start are nonnegative
    /// integer scalar registers. Too few source values are a language error.
    SlicePack {
        dst_pack: PackRegister,
        source: PackRegister,
        start: Register,
        count: Register,
    },
    /// Declares the statically known names in this function workspace.
    ///
    /// This bytecode-v20 metadata lets current-scope built-ins inspect and
    /// transactionally update locals without embedding compiler tables in the
    /// runtime. It must precede ordinary function body instructions.
    DeclareNamedBindings {
        bindings: Vec<(ConstantId, NamedBindingKind)>,
    },
    /// Declares the ordered package imports visible throughout one lexical scope.
    /// Entries are full names such as `alpha.Box` or wildcard package names such
    /// as `alpha.*`. Entry-scope imports persist with the base workspace.
    DeclareImports { imports: Vec<ConstantId> },
    /// Clears the import list attached to the current workspace. This is valid
    /// only for the base command workspace; ordinary value bindings are kept.
    ClearImports,
    /// Copies a constant-pool value to a register.
    LoadConstant { dst: Register, constant: ConstantId },
    /// Copies a register value.
    Move { dst: Register, src: Register },
    /// Applies a scalar arithmetic or comparison operator.
    Binary {
        operator: BinaryOperator,
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    /// Applies MATLAB `switch` matching semantics to a selector and one case value.
    ///
    /// This is deliberately distinct from [`BinaryOperator::Equal`]: switch
    /// matching has its own scalar, array, character, string, and integer rules.
    SwitchMatch {
        dst: Register,
        selector: Register,
        case_value: Register,
    },
    /// Concatenates evaluated matrix-literal elements by source rows.
    BuildMatrix {
        dst: Register,
        rows: Vec<Vec<Register>>,
    },
    /// Constructs a cell array from source rows.
    ///
    /// An empty outer vector denotes the `0 x 0` cell literal. Non-empty rows
    /// may encode shaped zero-width cells, but every encoded row must have the
    /// same width. Runtime expansion validates widths after packs are spliced.
    BuildCell {
        dst: Register,
        rows: Vec<Vec<ValueSource>>,
    },
    /// Materializes an inclusive MATLAB colon range.
    Range {
        dst: Register,
        start: Register,
        step: Register,
        end: Register,
    },
    /// Applies conjugate or non-conjugate transpose.
    Transpose {
        dst: Register,
        operand: Register,
        conjugate: bool,
    },
    /// Loads a frame-local slot.
    LoadLocal { dst: Register, local: LocalSlot },
    /// Stores a frame-local slot.
    StoreLocal { local: LocalSlot, src: Register },
    /// Executes a global declaration at this instruction's source position.
    ///
    /// `name` is a string constant naming the interpreter-session workspace
    /// binding. Its bytecode-v15 logical opcode discriminant is `41`; an
    /// encoder must write that value explicitly and must not derive it from
    /// this variant's Rust enum position.
    DeclareGlobal { name: ConstantId },
    /// Executes a persistent declaration at this instruction's source position.
    ///
    /// `slot` belongs to the containing function. Its bytecode-v15 logical
    /// opcode discriminant is `42`; an encoder must write that value explicitly
    /// and must not derive it from this variant's Rust enum position.
    DeclarePersistent { slot: PersistentSlot },
    /// Loads one function-owned persistent slot.
    ///
    /// Its bytecode-v15 logical opcode discriminant is `43`; an encoder must
    /// write that value explicitly and must not derive it from this variant's
    /// Rust enum position.
    LoadPersistent { dst: Register, slot: PersistentSlot },
    /// Stores one function-owned persistent slot.
    ///
    /// Its bytecode-v15 logical opcode discriminant is `44`; an encoder must
    /// write that value explicitly and must not derive it from this variant's
    /// Rust enum position.
    StorePersistent { slot: PersistentSlot, src: Register },
    /// Loads the actual number of arguments supplied to this invocation.
    LoadCallInputCount { dst: Register },
    /// Loads the number of outputs requested by this invocation's call site.
    LoadCallOutputCount { dst: Register },
    /// Packs inputs after the function's fixed parameter count into a `1 x N` cell array,
    /// or a `0 x 0` cell array when there are no variadic inputs.
    ///
    /// The presence of this instruction declares the containing function variadic. A
    /// verifier accepts it only as the first executable instruction and at most once.
    /// `parameter_count` locates the variadic tail, not a minimum input arity.
    LoadVariadicInputs { dst: Register },
    /// Loads a workspace global or registered callable by string constant.
    /// A syntactically bare value name may request implicit zero-input
    /// invocation if resolution selects a callable rather than a workspace
    /// value. The historical field name is retained in bytecode v17 because
    /// its encoded Boolean slot already carried this contextual distinction.
    LoadGlobal {
        dst: Register,
        name: ConstantId,
        construct_if_class: bool,
    },
    /// Loads a bare call target while preserving workspace-value precedence.
    ///
    /// If no dynamic value binding exists, the runtime produces a named
    /// callable whose application can select an old-style `@Class` method from
    /// the actual arguments before falling back to an ordinary function.
    LoadCallTarget { dst: Register, name: ConstantId },
    /// Resolves the root of an unresolved static dotted call before its arguments.
    ///
    /// When `root` is a workspace value, class, or registered namespace, `dst`
    /// receives that value and `root_found` is true. Otherwise the ordered
    /// `qualified` candidates are searched from their full name toward their
    /// longest callable prefix. `unresolved_suffix` records how many trailing
    /// components still require member traversal.
    LoadQualifiedTarget {
        dst: Register,
        root_found: Register,
        unresolved_suffix: Register,
        root: ConstantId,
        qualified: Vec<ConstantId>,
        member_count: u32,
    },
    /// Applies and consumes a shallow snapshot of a statically resolved value
    /// binding. The compiler captures `target` before evaluating arguments,
    /// matching MATLAB order, while consuming it prevents a VM-only array alias
    /// from surviving until a later indexed assignment.
    ApplyBinding {
        outputs: Vec<Register>,
        target: Register,
        arguments: Vec<ApplyArgument>,
    },
    /// Resolves a named function without reading or invoking a workspace value.
    ///
    /// Source-backed functions are normally rewritten to [`Constant::Function`]
    /// by the kernel linker. The runtime resolves registered built-ins and
    /// reports an unknown-target error for names that remain unresolved.
    LoadFunctionHandle { dst: Register, name: ConstantId },
    /// Resolves a named function handle from ordered import-expanded candidates
    /// without consulting workspace values.
    LoadFunctionHandleCandidates {
        dst: Register,
        names: Vec<ConstantId>,
    },
    /// Constructs a bytecode closure and copies the current values of its
    /// encoded lexical captures into runtime-owned storage.
    MakeClosure {
        dst: Register,
        function: FunctionId,
        /// Ordered `(free-variable name, optional enclosing register,
        /// tombstone-if-missing)` tuples.
        /// A missing register asks the runtime to copy an existing enclosing
        /// closure/workspace binding. The final flag distinguishes a missing
        /// value reference, which must stay missing for the closure lifetime,
        /// from a call target that should continue through callable resolution.
        captures: Vec<(ConstantId, Option<Register>, bool)>,
    },
    /// Constructs a nested-function closure whose lexical captures share storage.
    MakeSharedClosure {
        dst: Register,
        function: FunctionId,
        /// Ordered `(capture name, enclosing source)` pairs.
        captures: Vec<(ConstantId, SharedCaptureSource)>,
    },
    /// Loads one shared lexical capture by name.
    LoadCapture { dst: Register, name: ConstantId },
    /// Stores one shared lexical capture by name.
    StoreCapture { name: ConstantId, src: Register },
    /// Loads a workspace global or produces [`Constant::Nothing`] when the
    /// binding does not exist. This is reserved for indexed-assignment targets,
    /// where the runtime may grow a newly introduced array.
    LoadGlobalOrNothing { dst: Register, name: ConstantId },
    /// Stores a workspace global named by a string constant.
    StoreGlobal { name: ConstantId, src: Register },
    /// Removes workspace globals named by string constants, in encoded order.
    ClearGlobal { names: Vec<ConstantId> },
    /// Removes every value binding from the current interpreter workspace.
    ClearGlobalAll,
    /// Resets selected function-local slots to the internal no-value marker.
    ClearLocal { locals: Vec<LocalSlot> },
    /// Resets selected shared lexical captures to the internal no-value marker.
    ClearCapture { names: Vec<ConstantId> },
    /// Removes dynamically introduced function-workspace bindings.
    ///
    /// An empty name list with except false clears all dynamic bindings.
    /// With except true, every dynamic binding except the named set is removed.
    ClearDynamicBindings {
        names: Vec<ConstantId>,
        except: bool,
    },
    /// Atomically registers one class definition from the module table.
    RegisterClass { class: ClassDefinitionId },
    /// Executes the named direct superclass constructor on the current
    /// constructor's already-allocated output object.
    InvokeSuperclassConstructor {
        superclass: ConstantId,
        object: LocalSlot,
        arguments: Vec<Register>,
    },
    /// Reads a public object property.
    GetField {
        dst: Register,
        object: Register,
        name: ConstantId,
    },
    /// Produces the receiver after assigning one public object property.
    SetField {
        dst: Register,
        object: Register,
        name: ConstantId,
        value: Register,
    },
    /// Resolves `object.member(args)` at runtime as method dispatch or property indexing.
    ApplyField {
        outputs: Vec<Register>,
        object: Register,
        name: ConstantId,
        arguments: Vec<ApplyArgument>,
    },
    /// Applies a target produced by [`InstructionKind::LoadQualifiedTarget`].
    /// A resolved root traverses `members` as ordinary member access; a missing
    /// root applies the package callable stored in `target` directly.
    ApplyQualified {
        outputs: Vec<Register>,
        target: Register,
        unresolved_suffix: Register,
        members: Vec<ConstantId>,
        arguments: Vec<ApplyArgument>,
    },
    /// Resolves a dotted value expression. A workspace root is read as an
    /// ordinary member chain; a fully resolved package callable is invoked with
    /// zero inputs; a callable prefix exposes its remaining member value.
    GetQualifiedPack {
        dst_pack: PackRegister,
        target: Register,
        root_found: Register,
        unresolved_suffix: Register,
        members: Vec<ConstantId>,
    },
    /// Applies cell-content indexing and produces a comma-separated value pack.
    BraceApply {
        dst_pack: PackRegister,
        target: Register,
        arguments: Vec<ApplyArgument>,
    },
    /// Reads a struct field and produces its comma-separated value pack.
    GetAggregateField {
        dst_pack: PackRegister,
        target: Register,
        field: FieldOperand,
    },
    /// Transactionally updates a complete name-rooted aggregate place.
    AssignPlace {
        dst: Register,
        root: Register,
        path: Vec<PlaceStep>,
        source: ValueSource,
        mode: AssignmentMode,
    },
    /// Transactionally updates a statically resolved aggregate binding from
    /// the captured `root`. The compiler evaluates LHS selectors, loads this
    /// shallow snapshot, then evaluates the RHS, matching MATLAB assignment
    /// order. The interpreter consumes the snapshot so VM-only aliases do not
    /// force array copies. A result is needed only for unsuppressed display.
    AssignBindingPlace {
        result: Option<Register>,
        binding: BindingTarget,
        root: Register,
        path: Vec<PlaceStep>,
        source: ValueSource,
        mode: AssignmentMode,
    },
    /// Resolves `end` for one argument of an indexing target.
    ResolveEnd {
        dst: Register,
        target: Register,
        argument_index: u32,
        argument_count: u32,
    },
    /// Consumes a value pack into the requested ordinary output registers.
    Unpack {
        outputs: Vec<Register>,
        pack: PackRegister,
    },
    /// Transfers control unconditionally.
    Jump { target: InstructionIndex },
    /// Transfers control when the condition is false.
    JumpIfFalse {
        condition: Register,
        target: InstructionIndex,
    },
    /// Calls a bytecode function or registered built-in.
    ///
    /// Returned values are assigned left-to-right. Unrequested trailing return
    /// values are ignored; too few returned values are a runtime error.
    Call {
        outputs: Vec<Register>,
        callee: Register,
        arguments: Vec<Register>,
    },
    /// Resolves MATLAB `f(x)` at runtime as either a call or indexing operation.
    Apply {
        outputs: Vec<Register>,
        target: Register,
        arguments: Vec<ApplyArgument>,
    },
    /// Evaluates unresolved application syntax as an expression statement.
    ///
    /// Function targets observe zero requested outputs. If evaluation actually
    /// returns a value, its first value is assigned to `ans`; indexing targets
    /// are evaluated for one value. A semicolon suppresses only `display`.
    StatementApply {
        target: Register,
        arguments: Vec<ApplyArgument>,
        result: StatementResultTarget,
        display: bool,
    },
    /// Evaluates `object.member(args)` with expression-statement `ans` semantics.
    StatementApplyField {
        object: Register,
        name: ConstantId,
        arguments: Vec<ApplyArgument>,
        result: StatementResultTarget,
        display: bool,
    },
    /// Statement-form counterpart of [`InstructionKind::ApplyQualified`].
    StatementApplyQualified {
        target: Register,
        unresolved_suffix: Register,
        members: Vec<ConstantId>,
        arguments: Vec<ApplyArgument>,
        result: StatementResultTarget,
        display: bool,
    },
    /// Assigns each value in a comma-separated pack to `ans` in order.
    StatementPack {
        pack: PackRegister,
        result: StatementResultTarget,
        display: bool,
    },
    /// Stores and optionally displays an already evaluated statement value.
    /// The internal no-value marker leaves the existing `ans` binding untouched.
    StatementValue {
        src: Register,
        result: StatementResultTarget,
        display: bool,
    },
    /// Emits MATLAB's automatic named value display.
    Display { name: ConstantId, src: Register },
    /// Tail-evaluates unresolved MATLAB application syntax and returns exactly
    /// the current invocation's dynamically requested number of outputs.
    ReturnApply {
        target: Register,
        arguments: Vec<ApplyArgument>,
    },
    /// Tail-evaluates member application with the caller's requested outputs.
    ReturnApplyField {
        object: Register,
        name: ConstantId,
        arguments: Vec<ApplyArgument>,
    },
    /// Produces a copy-on-write indexed assignment result.
    IndexAssign {
        dst: Register,
        target: Register,
        arguments: Vec<ApplyArgument>,
        value: Register,
    },
    /// Advances a MATLAB `for` iteration by one array column.
    ///
    /// `index` is a compiler-owned zero-based counter register. When iteration
    /// is exhausted, execution jumps to `exit`; otherwise `dst` receives the
    /// next column and the counter is incremented.
    ForEach {
        iterable: Register,
        index: Register,
        dst: Register,
        exit: InstructionIndex,
    },
    /// Returns zero or more values to the caller.
    Return { values: Vec<Register> },
    /// Returns fixed outputs followed by as many `varargout` cell elements as requested.
    ReturnVariadic {
        fixed: Vec<Register>,
        variadic: Register,
    },
}

/// Source of one shared lexical binding when a nested closure is constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedCaptureSource {
    /// Promote a local in the current frame to shared storage.
    Local(LocalSlot),
    /// Reuse a shared capture inherited by the current frame under the same name.
    Enclosing,
}

/// An instruction together with optional source provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct Instruction {
    /// Executable operation.
    pub kind: InstructionKind,
    /// Source range used for runtime diagnostics.
    pub location: Option<SourceLocation>,
}

impl Instruction {
    /// Creates an instruction without source provenance.
    #[must_use]
    pub const fn new(kind: InstructionKind) -> Self {
        Self {
            kind,
            location: None,
        }
    }

    /// Creates an instruction with source provenance.
    #[must_use]
    pub const fn located(kind: InstructionKind, location: SourceLocation) -> Self {
        Self {
            kind,
            location: Some(location),
        }
    }
}

/// Input binding metadata for MATLAB argument blocks (v28; repeating slots v29).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArgumentLayout {
    /// Leading signature slots occupied by ordinary positional inputs.
    pub positional_count: u32,
    /// Required prefix, before positional defaults.
    pub required_count: u32,
    /// Version-29 repeated group width. These cell-valued signature slots follow
    /// the fixed positional slots and precede the name-value owner slots.
    /// Zero preserves version-28 binding semantics.
    pub repeating_count: u32,
    /// Option fields and their owning signature slots, in declaration order.
    pub named: Vec<(LocalSlot, String)>,
}

/// A register-bytecode function.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// Diagnostic function name.
    pub name: String,
    /// Number of registers allocated by each execution frame.
    pub register_count: u32,
    /// Number of value-pack registers allocated by each execution frame.
    pub pack_register_count: u32,
    /// Number of addressable local slots.
    pub local_count: u32,
    /// Number of addressable function-owned persistent slots.
    pub persistent_slot_count: u32,
    /// Number of leading input signature slots. With `argument_layout`, these
    /// include fixed inputs, repeated cells and name-value owner structures.
    /// Without a layout, this is the number of declared fixed input arguments.
    ///
    /// This is the maximum arity unless `argument_layout` is present or the
    /// instruction stream begins with [`InstructionKind::LoadVariadicInputs`]. Omitted inputs stay undefined;
    /// an actual read is checked with [`InstructionKind::RequireDefined`].
    pub parameter_count: u32,
    /// Version-28/29 argument-block binding. Defaults and validators are executable
    /// instructions, not serialized source or callbacks in this metadata.
    pub argument_layout: Option<ArgumentLayout>,
    /// Per-function constant pool.
    pub constants: Vec<Constant>,
    /// Instruction stream.
    pub instructions: Vec<Instruction>,
    /// Function-owned exception table. Table order does not affect handler
    /// selection; serializers and linkers must nevertheless preserve it.
    pub exception_handlers: Vec<ExceptionHandler>,
}

impl Function {
    /// Creates an empty function definition with the supplied frame sizes.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        register_count: u32,
        local_count: u32,
        parameter_count: u32,
    ) -> Self {
        Self {
            name: name.into(),
            register_count,
            pack_register_count: 0,
            local_count,
            persistent_slot_count: 0,
            parameter_count,
            argument_layout: None,
            constants: Vec::new(),
            instructions: Vec::new(),
            exception_handlers: Vec::new(),
        }
    }

    /// Sets the number of value-pack registers allocated by each frame.
    #[must_use]
    pub const fn with_pack_register_count(mut self, pack_register_count: u32) -> Self {
        self.pack_register_count = pack_register_count;
        self
    }

    /// Sets the number of function-owned persistent slots.
    #[must_use]
    pub const fn with_persistent_slot_count(mut self, persistent_slot_count: u32) -> Self {
        self.persistent_slot_count = persistent_slot_count;
        self
    }

    /// Sets the function-owned exception table.
    #[must_use]
    pub fn with_exception_handlers(mut self, exception_handlers: Vec<ExceptionHandler>) -> Self {
        self.exception_handlers = exception_handlers;
        self
    }
}

/// A self-contained unit of executable bytecode.
#[derive(Debug, Clone, PartialEq)]
pub struct BytecodeModule {
    /// Format version.
    pub version: BytecodeVersion,
    /// Module function table.
    pub functions: Vec<Function>,
    /// Runtime-registerable class definitions.
    pub classes: Vec<ClassDefinition>,
    /// Optional version-22 semantic extensions keyed by class-table index.
    pub class_features: Vec<ClassFeatureDefinition>,
    /// Function invoked when executing the module entry point.
    pub entry: FunctionId,
}

impl BytecodeModule {
    /// Creates a module using the current bytecode version.
    #[must_use]
    pub fn new(functions: Vec<Function>, entry: FunctionId) -> Self {
        Self {
            version: CURRENT_BYTECODE_VERSION,
            functions,
            classes: Vec::new(),
            class_features: Vec::new(),
            entry,
        }
    }

    /// Adds versioned class definitions to a module.
    #[must_use]
    pub fn with_classes(mut self, classes: Vec<ClassDefinition>) -> Self {
        self.classes = classes;
        self
    }

    /// Adds versioned enum, event, abstract-property, and sealed-method metadata.
    #[must_use]
    pub fn with_class_features(mut self, features: Vec<ClassFeatureDefinition>) -> Self {
        self.class_features = features;
        self
    }
}

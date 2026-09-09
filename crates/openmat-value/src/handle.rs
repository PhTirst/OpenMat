macro_rules! opaque_handle {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// The integer is stable only within the owning runtime/session table.
        /// It is not a Rust ABI or a persistent file-format identity.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Creates a handle from a runtime/session table identifier.
            #[must_use]
            pub const fn new(identifier: u64) -> Self {
                Self(identifier)
            }

            /// Returns the runtime/session table identifier.
            #[must_use]
            pub const fn identifier(self) -> u64 {
                self.0
            }
        }
    };
}

opaque_handle!(
    ArrayHandle,
    "An opaque handle reserved for identity-bearing external array adapters; owned dense `Value` arrays do not use it."
);
opaque_handle!(
    ObjectHandle,
    "An opaque adapter handle for an object owned by an object table."
);
opaque_handle!(
    ClassHandle,
    "An opaque handle for a class registered in one runtime session."
);
opaque_handle!(
    BuiltinHandle,
    "An opaque handle for an entry in a runtime built-in registry."
);

/// A function index in the currently executing bytecode module.
///
/// This remains separate from the bytecode crate's index type so the value
/// crate does not depend on compiler layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BytecodeFunctionHandle(u32);

impl BytecodeFunctionHandle {
    /// Creates a bytecode function handle.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the module function-table index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// A callable value resolved by the interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FunctionHandle {
    /// A function in the current bytecode module.
    Bytecode(BytecodeFunctionHandle),
    /// A function in the interpreter's built-in registry.
    Builtin(BuiltinHandle),
    /// A constructor for a class registered in the current runtime session.
    Class(ClassHandle),
    /// A runtime intrinsic whose semantics require session object metadata.
    Intrinsic(IntrinsicFunction),
}

/// Intrinsics resolved directly by the runtime rather than the host registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntrinsicFunction {
    /// Publishes bounded UI snapshots or component metadata to the host.
    OpenMatUi,
    /// Returns the session-owned low-level foreign-function namespace.
    Ffi,
    /// Returns the language-visible class name.
    Class,
    /// Tests class identity or ancestry.
    Isa,
    /// Tests whether a value is a classdef object.
    IsObject,
    /// Invokes a dynamically supplied function name or callable handle.
    Feval,
    /// Parses and executes source text in the caller's current workspace.
    Eval,
    /// Captures command-window text while evaluating source in the current workspace.
    EvalC,
    /// Parses and executes source text in a named caller or base workspace.
    EvalIn,
    /// Assigns one value into a named caller or base workspace.
    AssignIn,
    /// Creates a lazy named function handle from text.
    Str2Func,
    /// Returns the textual identity of a callable handle.
    Func2Str,
    /// Starts a session-owned elapsed-time measurement.
    Tic,
    /// Reads a session-owned elapsed-time measurement.
    Toc,
    /// Reads or replaces the session's legacy message-and-identifier error state.
    LastErr,
    /// Reads or replaces the session's structured legacy error state.
    LastError,
    /// Throws an `MException` from the current language frame.
    Throw,
    /// Throws an `MException` while omitting the current language frame.
    ThrowAsCaller,
    /// Rethrows a caught runtime exception without replacing its origin.
    Rethrow,
    /// Returns a value-copy of an `MException` with one appended cause.
    AddCause,
    /// Formats an `MException` as OpenMat-owned report text.
    GetReport,
}

#![doc = "Semantic analysis and checked lowering from HIR to bytecode."]
#![forbid(unsafe_code)]

mod diagnostic;
mod lowering;

pub use diagnostic::{
    BindingDeclarationProblem, BindingOperation, BindingStorageClass, ClassAttributeProblem,
    ClassMemberProblem, CompileDiagnostic, CompileDiagnosticKind, CompileError, CompileResult,
    CompilerResource, InternalCompilerError, UnsupportedFeature,
};
pub use lowering::{compile, compile_with_imports};

/// Compiler crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

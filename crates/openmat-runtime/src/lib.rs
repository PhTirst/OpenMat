#![doc = "Sequential bytecode execution, workspaces, calls, errors, and cancellation."]
#![forbid(unsafe_code)]

mod arguments;
mod array_ops;
mod builtin;
mod cancellation;
mod display;
mod error;
mod eval;
mod filesystem;
mod interpreter;
mod linalg_ops;
mod module_loader;
mod output;
mod random;
mod ui_sources;
mod workspace;
pub use ui_sources::is_builtin_source;

pub use builtin::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinFunction, BuiltinInvocationError,
    BuiltinRegistrationError, BuiltinRegistry, BuiltinResult, GraphicsService, NativeClass,
    NativeClassToken, NativeInstance, NativeMethod, NativeObjectService, NativeProperty,
    ObjectLifecycleRequest, ObjectLifecycleResult, ObjectLifecycleService,
};
pub use cancellation::CancellationToken;
pub use display::format_display_value_with;
pub use error::{
    ArrayRuntimeError, ClassConstraintKind, IndexErrorKind, IndexOutOfBoundsDetails,
    RangeErrorKind, RuntimeBuildError, RuntimeError, RuntimeErrorKind, RuntimeResult,
    StackTraceFrame,
};
pub use filesystem::{
    DEFAULT_MAX_FILE_BYTES, FileOpenAccess, FileOpenMode, FileSeekOrigin, FileSystemError,
    FileSystemErrorCategory, FileSystemMetadata, FileSystemService, LocalFileSystem,
    MatlabSourceResolver, OpenFileInfo, ResolvedMatlabSource, SearchPath, SearchPathPosition,
    SearchPathSnapshot, WorkingDirectory, WorkingDirectorySnapshot,
};
pub use interpreter::{ExecutionFrame, Interpreter, RuntimeConfig};
pub use linalg_ops::{
    RuntimeLinalgError, linalg_matrix_left_divide, linalg_matrix_multiply,
    linalg_matrix_right_divide, linalg_square_solve,
};
pub use openmat_fft::{FftError, FftProvider};
pub use openmat_graphics_model::{
    GraphicsClass, GraphicsDelta, GraphicsExecution, GraphicsHandle, GraphicsNotice,
    GraphicsNoticeKind, GraphicsRequest, GraphicsResponse, GraphicsSession,
};
pub use openmat_linalg::{LinalgError, LinalgProvider};
pub use output::{
    DisplayFormat, LineSpacing, NullOutput, NumericFormat, OutputError, OutputEvent, OutputSink,
    VecOutput,
};
pub use random::{RandomService, RandomSession, RandomSnapshot};
pub use workspace::Workspace;

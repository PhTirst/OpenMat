use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, RwLock},
    time::Instant,
};

use openmat_array::{ArrayData, CharCodeUnit, DenseArray, IntegerArrayData, Shape};
use openmat_bytecode::{
    Access as BytecodeAccess, ApplyArgument, AssignmentMode, BinaryOperator, BindingTarget,
    BytecodeModule, ClassDefinitionId, ClassKind, ClassSemantics as BytecodeClassSemantics,
    Constant, ExceptionHandler, ExceptionHandlerKind, FieldOperand, Function, FunctionId,
    InstructionIndex, InstructionKind, LocalSlot, MethodKind as BytecodeMethodKind,
    NamedBindingKind, PackApplyTarget, PackRegister, PersistentSlot, PlaceStep,
    PropertyKind as BytecodePropertyKind, Register, SharedCaptureSource, SourceLocation,
    StatementResultTarget, ValueSource, verify,
};
use openmat_fft::{FftProvider, RustFftProvider};
use openmat_linalg::{LinalgProvider, ReferenceProvider};
use openmat_object::{
    Access as ObjectAccess, AccessContext, ClassDefinition, ClassId, ClassRegistry, ClassSemantics,
    ConstructionState, EnumerationMemberDescriptor, EventAccess, EventDescriptor,
    FinalizationCandidate, GcCollection, GcMetrics, HandleState, MethodDescriptor, MethodKind,
    ObjectArrayDescriptor, ObjectError, ObjectRef, ObjectStore, Operator, PropertyAccess,
    PropertyDescriptor, PropertyKey, PropertyResolution,
};
use openmat_source::SourceId;
use openmat_text::{FancyRegexProvider, RegexProvider};
use openmat_value::{
    BuiltinHandle, BytecodeFunctionHandle, CellArray, ClassHandle, Complex64, FieldName,
    FunctionHandle, GraphicsHandleArray, IntrinsicFunction, ObjectArray, ObjectHandle, StructArray,
    TableArray, TableRowName, TableVariableName, Value,
};

use crate::{
    ArrayRuntimeError, BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinFunction,
    BuiltinRegistry, CancellationToken, ClassConstraintKind, DisplayFormat, FileOpenMode,
    FileSeekOrigin, FileSystemError, FileSystemMetadata, FileSystemService, GraphicsExecution,
    GraphicsRequest, GraphicsService, GraphicsSession, IndexErrorKind, NativeClass,
    NativeClassToken, NativeInstance, NativeObjectService, NullOutput, ObjectLifecycleRequest,
    ObjectLifecycleResult, ObjectLifecycleService, OpenFileInfo, OutputError, OutputEvent,
    OutputSink, RandomService, RandomSnapshot, ResolvedMatlabSource, RuntimeBuildError,
    RuntimeError, RuntimeErrorKind, RuntimeResult, SearchPathPosition, SearchPathSnapshot,
    StackTraceFrame, Workspace,
    array_ops::{self, IndexInput},
    builtin::{LanguageService, WarningState},
    error::array_error,
    module_loader::{ModuleLoadErrorKind, ModuleLoader},
};

mod ffi;
mod finalization;
mod frames;
mod sparse;
mod ui;

use finalization::{FinalizationSafepoints, FinalizerQueue, SafepointReason};
use frames::FrameStack;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeTarget {
    Current,
    Caller,
    Base,
}

#[derive(Clone, Debug, PartialEq)]
struct ScopeContext {
    address: ScopeAddress,
    workspace: Workspace,
    imports: Vec<String>,
    caller: Option<Arc<Self>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeAddress {
    Frame(u64),
    Base,
}

struct ScopeBindingChange {
    target: ScopeAddress,
    name: String,
    value: Value,
}

struct SharedOutputSink {
    output: Arc<Mutex<Option<Box<dyn OutputSink>>>>,
}

impl SharedOutputSink {
    fn new(output: Arc<Mutex<Option<Box<dyn OutputSink>>>>) -> Self {
        Self { output }
    }
}

impl OutputSink for SharedOutputSink {
    fn emit(&mut self, event: OutputEvent) -> Result<(), OutputError> {
        let mut output = self
            .output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        output
            .as_deref_mut()
            .ok_or_else(|| OutputError::new("the reentrant runtime output sink is unavailable"))?
            .emit(event)
    }
}

struct SharedGraphicsService {
    graphics: Arc<Mutex<GraphicsSession>>,
}

impl GraphicsService for SharedGraphicsService {
    fn current_colormap_length(&self) -> Option<usize> {
        self.graphics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current_colormap_length()
    }

    fn execute(
        &mut self,
        request: GraphicsRequest,
    ) -> Result<GraphicsExecution, openmat_graphics_model::GraphicsError> {
        self.graphics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .execute(request)
    }
}

struct SharedRandomService {
    random: Arc<Mutex<crate::RandomSession>>,
}

impl RandomService for SharedRandomService {
    fn next_uniform(&mut self) -> f64 {
        self.random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_uniform()
    }

    fn next_normal(&mut self) -> f64 {
        self.random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_normal()
    }

    fn reseed(&mut self, seed: u32) {
        self.random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reseed(seed);
    }

    fn shuffle(&mut self) {
        self.random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .shuffle();
    }

    fn snapshot(&self) -> RandomSnapshot {
        self.random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot()
    }
}

struct SharedFileSystemService {
    file_system: Arc<Mutex<Box<dyn FileSystemService>>>,
}

impl SharedFileSystemService {
    fn lock(&self) -> MutexGuard<'_, Box<dyn FileSystemService>> {
        self.file_system
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl FileSystemService for SharedFileSystemService {
    fn working_directory(&self) -> Result<PathBuf, FileSystemError> {
        self.lock().working_directory()
    }

    fn change_working_directory(&mut self, path: &str) -> Result<PathBuf, FileSystemError> {
        self.lock().change_working_directory(path)
    }

    fn search_path(&self) -> Result<SearchPathSnapshot, FileSystemError> {
        self.lock().search_path()
    }

    fn replace_search_path(
        &mut self,
        paths: &[String],
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        self.lock().replace_search_path(paths)
    }

    fn add_search_path(
        &mut self,
        paths: &[String],
        position: SearchPathPosition,
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        self.lock().add_search_path(paths, position)
    }

    fn remove_search_path(
        &mut self,
        paths: &[String],
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        self.lock().remove_search_path(paths)
    }

    fn generate_search_path(&mut self, root: &str) -> Result<Vec<PathBuf>, FileSystemError> {
        self.lock().generate_search_path(root)
    }

    fn resolve_matlab_source(
        &mut self,
        caller: Option<&str>,
        name: &str,
    ) -> Result<Option<ResolvedMatlabSource>, FileSystemError> {
        self.lock().resolve_matlab_source(caller, name)
    }

    fn read_file(&mut self, path: &str, maximum_bytes: u64) -> Result<Vec<u8>, FileSystemError> {
        self.lock().read_file(path, maximum_bytes)
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), FileSystemError> {
        self.lock().write_file(path, contents)
    }

    fn metadata(&mut self, path: &str) -> Result<FileSystemMetadata, FileSystemError> {
        self.lock().metadata(path)
    }

    fn directory_entries(
        &mut self,
        path: &str,
    ) -> Result<Vec<FileSystemMetadata>, FileSystemError> {
        self.lock().directory_entries(path)
    }

    fn create_directory(&mut self, path: &str) -> Result<(), FileSystemError> {
        self.lock().create_directory(path)
    }

    fn remove_directory(&mut self, path: &str, recursive: bool) -> Result<(), FileSystemError> {
        self.lock().remove_directory(path, recursive)
    }

    fn remove_file(&mut self, path: &str) -> Result<(), FileSystemError> {
        self.lock().remove_file(path)
    }

    fn copy_path(
        &mut self,
        source: &str,
        destination: &str,
        force: bool,
    ) -> Result<(), FileSystemError> {
        self.lock().copy_path(source, destination, force)
    }

    fn move_path(
        &mut self,
        source: &str,
        destination: &str,
        force: bool,
    ) -> Result<(), FileSystemError> {
        self.lock().move_path(source, destination, force)
    }

    fn open_file(
        &mut self,
        path: &str,
        mode: FileOpenMode,
        machine_format: &str,
        encoding: &str,
    ) -> Result<u32, FileSystemError> {
        self.lock().open_file(path, mode, machine_format, encoding)
    }

    fn close_open_file(&mut self, identifier: u32) -> bool {
        self.lock().close_open_file(identifier)
    }

    fn close_all_open_files(&mut self) {
        self.lock().close_all_open_files();
    }

    fn open_file_identifiers(&self) -> Vec<u32> {
        self.lock().open_file_identifiers()
    }

    fn open_file_info(&self, identifier: u32) -> Option<OpenFileInfo> {
        self.lock().open_file_info(identifier)
    }

    fn read_open_file(
        &mut self,
        identifier: u32,
        maximum_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FileSystemError> {
        self.lock().read_open_file(identifier, maximum_bytes)
    }

    fn write_open_file(
        &mut self,
        identifier: u32,
        contents: &[u8],
    ) -> Result<Option<usize>, FileSystemError> {
        self.lock().write_open_file(identifier, contents)
    }

    fn seek_open_file(
        &mut self,
        identifier: u32,
        offset: i64,
        origin: FileSeekOrigin,
    ) -> Result<Option<u64>, FileSystemError> {
        self.lock().seek_open_file(identifier, offset, origin)
    }

    fn tell_open_file(&mut self, identifier: u32) -> Result<Option<u64>, FileSystemError> {
        self.lock().tell_open_file(identifier)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct LastErrorState {
    message: String,
    identifier: String,
    stack: Vec<LastErrorFrame>,
}

#[derive(Clone, Debug, PartialEq)]
struct LastErrorFrame {
    file: String,
    name: String,
    line: f64,
}

#[derive(Clone, Debug)]
struct SourceMetadata {
    name: String,
    line_starts: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacyClassMetadata {
    constructor_source: PathBuf,
    own_fields: Vec<String>,
    superclasses: Vec<ClassId>,
}

impl SourceMetadata {
    fn new(name: String, source: &str) -> Self {
        let mut line_starts = Vec::new();
        line_starts.push(0);
        for (offset, byte) in source.bytes().enumerate() {
            if byte == b'\n'
                && let Ok(start) = u32::try_from(offset.saturating_add(1))
            {
                line_starts.push(start);
            }
        }
        Self { name, line_starts }
    }

    fn line_number(&self, offset: u32) -> f64 {
        let lines = self
            .line_starts
            .partition_point(|line_start| *line_start <= offset);
        f64::from(u32::try_from(lines).unwrap_or(u32::MAX))
    }
}

#[derive(Clone)]
struct EvalCaptureOutput {
    events: Arc<Mutex<Vec<OutputEvent>>>,
}

impl OutputSink for EvalCaptureOutput {
    fn emit(&mut self, event: OutputEvent) -> Result<(), crate::OutputError> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
        Ok(())
    }
}

/// Tunable interpreter safety limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// Maximum number of simultaneously active bytecode frames.
    ///
    /// Frames and their return continuations live on the heap. This is a
    /// language resource limit, not the host thread's native stack size.
    /// Synchronous native-to-m reentry has a separate ceiling of 16.
    pub maximum_call_depth: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            maximum_call_depth: 2048,
        }
    }
}

/// Registers, locals, and program counter for one function invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionFrame {
    scope_address: ScopeAddress,
    caller_scope: Option<Arc<ScopeContext>>,
    function: FunctionId,
    instruction: InstructionIndex,
    registers: Vec<Value>,
    pack_registers: Vec<Vec<Value>>,
    locals: Vec<Value>,
    named_bindings: BTreeMap<String, NamedBindingKind>,
    dynamic_locals: BTreeMap<String, Value>,
    shared_locals: BTreeMap<LocalSlot, SharedBinding>,
    call_inputs: Vec<Value>,
    input_count: u32,
    requested_output_count: u32,
    access_context: AccessContext,
    captures: Arc<BTreeMap<String, CapturedBinding>>,
    imports: Vec<String>,
}

impl ExecutionFrame {
    #[allow(clippy::too_many_arguments)]
    fn new(
        scope_address: ScopeAddress,
        caller_scope: Option<Arc<ScopeContext>>,
        function: FunctionId,
        definition: &Function,
        arguments: &[Value],
        requested_outputs: usize,
        seeded_local: Option<(LocalSlot, Value)>,
        access_context: AccessContext,
        captures: Arc<BTreeMap<String, CapturedBinding>>,
    ) -> Option<Self> {
        let register_count = usize::try_from(definition.register_count).ok()?;
        let pack_register_count = usize::try_from(definition.pack_register_count).ok()?;
        let local_count = usize::try_from(definition.local_count).ok()?;
        let input_count = u32::try_from(arguments.len()).ok()?;
        let requested_output_count = u32::try_from(requested_outputs).ok()?;
        let mut registers = Vec::new();
        registers.try_reserve_exact(register_count).ok()?;
        registers.resize(register_count, Value::Nothing);
        let mut pack_registers = Vec::new();
        pack_registers.try_reserve_exact(pack_register_count).ok()?;
        pack_registers.resize_with(pack_register_count, Vec::new);
        let mut locals = Vec::new();
        locals.try_reserve_exact(local_count).ok()?;
        locals.resize(local_count, Value::Nothing);
        let fixed_input_count = usize::try_from(definition.parameter_count).ok()?;
        for (slot, argument) in locals
            .iter_mut()
            .zip(arguments.iter().take(fixed_input_count))
        {
            *slot = argument.clone();
        }
        if let Some((slot, value)) = seeded_local {
            *get_mut(&mut locals, slot.get())? = value;
        }
        Some(Self {
            scope_address,
            caller_scope,
            function,
            instruction: InstructionIndex::new(0),
            registers,
            pack_registers,
            locals,
            named_bindings: BTreeMap::new(),
            dynamic_locals: BTreeMap::new(),
            shared_locals: BTreeMap::new(),
            call_inputs: arguments.to_vec(),
            input_count,
            requested_output_count,
            access_context,
            captures,
            imports: Vec::new(),
        })
    }

    /// Returns the bytecode function table index.
    #[must_use]
    pub const fn function(&self) -> FunctionId {
        self.function
    }

    /// Returns the next instruction index.
    #[must_use]
    pub const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    /// Returns the frame register file.
    #[must_use]
    pub fn registers(&self) -> &[Value] {
        &self.registers
    }

    /// Returns the number of separately allocated VM value-pack registers.
    #[must_use]
    pub fn pack_register_count(&self) -> usize {
        self.pack_registers.len()
    }

    /// Returns the frame local-slot array.
    #[must_use]
    pub fn locals(&self) -> &[Value] {
        &self.locals
    }

    /// Returns the actual number of inputs supplied to this invocation.
    #[must_use]
    pub const fn input_count(&self) -> u32 {
        self.input_count
    }

    /// Returns the number of outputs requested by this invocation's call site.
    #[must_use]
    pub const fn requested_output_count(&self) -> u32 {
        self.requested_output_count
    }

    fn read_register(&self, register: Register) -> Option<&Value> {
        get(&self.registers, register.get())
    }

    fn write_register(&mut self, register: Register, value: Value) -> bool {
        let Some(target) = get_mut(&mut self.registers, register.get()) else {
            return false;
        };
        *target = value;
        true
    }

    fn take_register(&mut self, register: Register) -> Option<Value> {
        get_mut(&mut self.registers, register.get())
            .map(|value| std::mem::replace(value, Value::Nothing))
    }

    fn read_pack_register(&self, register: PackRegister) -> Option<&[Value]> {
        get(&self.pack_registers, register.get()).map(Vec::as_slice)
    }

    fn write_pack_register(&mut self, register: PackRegister, values: Vec<Value>) -> bool {
        let Some(target) = get_mut(&mut self.pack_registers, register.get()) else {
            return false;
        };
        *target = values;
        true
    }

    fn take_pack_register(&mut self, register: PackRegister) -> Option<Vec<Value>> {
        get_mut(&mut self.pack_registers, register.get()).map(std::mem::take)
    }

    fn gc_roots(&self) -> Vec<Value> {
        let mut roots = Vec::new();
        roots.extend(self.registers.iter().cloned());
        roots.extend(self.pack_registers.iter().flatten().cloned());
        roots.extend(self.locals.iter().cloned());
        roots.extend(self.dynamic_locals.values().cloned());
        roots.extend(self.call_inputs.iter().cloned());
        roots.extend(self.shared_locals.values().map(SharedBinding::gc_value));
        roots.extend(self.captures.values().filter_map(CapturedBinding::gc_value));
        roots
    }

    fn language_gc_roots(&self) -> Vec<Value> {
        let mut roots = Vec::new();
        roots.extend(self.locals.iter().cloned());
        roots.extend(self.dynamic_locals.values().cloned());
        roots.extend(self.shared_locals.values().map(SharedBinding::gc_value));
        roots.extend(self.captures.values().filter_map(CapturedBinding::gc_value));
        roots
    }
}

#[derive(Clone)]
enum ResolvedPlaceStep {
    Paren(Vec<IndexInput>),
    Brace(Vec<IndexInput>),
    Field(String),
}

struct AssignmentSource {
    values: Vec<Value>,
    expanded: bool,
}

struct Invocation {
    requested_outputs: usize,
    seeded_local: Option<(LocalSlot, Value)>,
    copy_arguments: bool,
    access_context: AccessContext,
    captures: Arc<BTreeMap<String, CapturedBinding>>,
    scope: InvocationScope,
}

enum InvocationScope {
    Base,
    New,
    Reuse(Arc<ScopeContext>),
}

#[derive(Clone, Copy)]
struct LifecycleObjectSnapshot {
    semantics: ClassSemantics,
    valid: bool,
}

#[derive(Default)]
struct BuiltinObjectLifecycle {
    objects: BTreeMap<ObjectHandle, LifecycleObjectSnapshot>,
    accepted_deletions: Vec<Value>,
}

impl BuiltinObjectLifecycle {
    fn handles(value: &Value) -> Result<&[ObjectHandle], BuiltinError> {
        match value {
            Value::Object(handle) => Ok(std::slice::from_ref(handle)),
            Value::ObjectArray(array) => Ok(array.as_slice()),
            _ => Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                "object lifecycle requires an object scalar or homogeneous object array",
            )),
        }
    }

    fn semantics(&self, handles: &[ObjectHandle]) -> Result<ClassSemantics, BuiltinError> {
        let Some(first) = handles.first() else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "an empty object array has no lifecycle receiver semantics",
            ));
        };
        let semantics = self
            .objects
            .get(first)
            .map(|object| object.semantics)
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "unknown object lifecycle receiver",
                )
            })?;
        if handles.iter().all(|handle| {
            self.objects
                .get(handle)
                .is_some_and(|object| object.semantics == semantics)
        }) {
            Ok(semantics)
        } else {
            Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "object array lifecycle receivers must have homogeneous semantics",
            ))
        }
    }
}

impl ObjectLifecycleService for BuiltinObjectLifecycle {
    fn request(
        &mut self,
        request: ObjectLifecycleRequest<'_>,
    ) -> Result<ObjectLifecycleResult, BuiltinError> {
        let value = match request {
            ObjectLifecycleRequest::IsValid(value) | ObjectLifecycleRequest::Delete(value) => value,
        };
        let handles = Self::handles(value)?;
        let semantics = self.semantics(handles)?;
        match request {
            ObjectLifecycleRequest::IsValid(_) => {
                if semantics != ClassSemantics::Handle {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "`isvalid` requires a handle-class receiver",
                    ));
                }
                Ok(ObjectLifecycleResult::Validity(
                    handles
                        .iter()
                        .map(|handle| self.objects.get(handle).is_some_and(|object| object.valid))
                        .collect(),
                ))
            }
            ObjectLifecycleRequest::Delete(_) => {
                self.accepted_deletions.push(value.clone());
                if semantics == ClassSemantics::Handle {
                    Ok(ObjectLifecycleResult::HandleDeleteRequested {
                        elements: handles.len(),
                    })
                } else {
                    Ok(ObjectLifecycleResult::ValueDeleteMethodRequested)
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PersistentBindingKey {
    function_name: String,
    caller_class: Option<ClassId>,
    slot: PersistentSlot,
}

#[derive(Clone)]
enum RuntimeBindingTarget {
    Local(LocalSlot),
    Persistent(PersistentBindingKey),
    Shared(SharedBinding),
    Workspace(String),
}

/// A sequential interpreter for one validated bytecode module and workspace.
pub struct Interpreter {
    module: Arc<BytecodeModule>,
    workspace: Workspace,
    base_imports: Vec<String>,
    graphics: Arc<Mutex<GraphicsSession>>,
    random: Arc<Mutex<crate::RandomSession>>,
    file_system: Option<Arc<Mutex<Box<dyn crate::FileSystemService>>>>,
    persistent_bindings: BTreeMap<PersistentBindingKey, Value>,
    builtins: BuiltinRegistry,
    linalg_provider: Arc<dyn LinalgProvider>,
    fft_provider: Arc<dyn FftProvider>,
    regex_provider: Arc<dyn RegexProvider>,
    cancellation: CancellationToken,
    output: Box<dyn OutputSink>,
    display_format: DisplayFormat,
    warning_state: WarningState,
    config: RuntimeConfig,
    call_stack: Vec<StackTraceFrame>,
    classes: ClassRegistry<Value>,
    objects: ObjectStore<Value>,
    object_references: BTreeMap<ObjectHandle, ObjectRef>,
    exception_errors: BTreeMap<ObjectHandle, RuntimeError>,
    exception_class: Option<ClassId>,
    pending_exception: Option<ObjectHandle>,
    last_error: Option<LastErrorState>,
    source_metadata: BTreeMap<u32, SourceMetadata>,
    class_code: BTreeMap<ClassId, RuntimeClassCode>,
    legacy_classes: BTreeMap<ClassId, LegacyClassMetadata>,
    native_classes: BTreeMap<ClassId, Arc<dyn NativeClass>>,
    native_instances: BTreeMap<ObjectHandle, Arc<NativeObjectInstance>>,
    class_handles: BTreeMap<ClassHandle, ClassId>,
    enumeration_members: BTreeMap<(ClassId, String), Value>,
    enumeration_members_loading: BTreeSet<(ClassId, String)>,
    event_source_slots: BTreeMap<ClassId, PropertyKey>,
    event_listeners: BTreeMap<ObjectHandle, RuntimeListener>,
    event_listener_order: BTreeMap<(ObjectHandle, String), Vec<ObjectHandle>>,
    active_event_listeners: BTreeSet<ObjectHandle>,
    destroying_event_sources: BTreeSet<ObjectHandle>,
    event_listener_class: Option<RuntimeListenerClass>,
    event_dispatch_depth: usize,
    pending_event_errors: Vec<RuntimeError>,
    function_handles: BTreeMap<BytecodeFunctionHandle, RuntimeFunctionCode>,
    named_function_handles: BTreeMap<BytecodeFunctionHandle, String>,
    named_function_fallbacks: BTreeMap<BytecodeFunctionHandle, RuntimeFunctionCode>,
    module_loader: ModuleLoader,
    next_function_handle: Option<u32>,
    timer_handles: BTreeMap<u64, Instant>,
    next_timer_handle: Option<u64>,
    default_timer: Option<Instant>,
    language_callback_depth: usize,
    builtin_workspace: Option<Workspace>,
    builtin_scope_context: Option<Arc<ScopeContext>>,
    pending_scope_bindings: Vec<ScopeBindingChange>,
    next_scope_id: Option<u64>,
    displaced_workspaces: Vec<Workspace>,
    active_frame_roots: Vec<Vec<Value>>,
    finalization_safepoints: FinalizationSafepoints,
    finalizer_queue: FinalizerQueue<FinalizationCandidate>,
    active_finalizer: Option<FinalizationCandidate>,
    pending_finalizer_errors: Vec<RuntimeError>,
    ffi: ffi::FfiSession,
}

struct InterpreterLanguageService<'a> {
    interpreter: &'a mut Interpreter,
    module: Arc<BytecodeModule>,
    location: Option<SourceLocation>,
    created: &'a mut Vec<ObjectHandle>,
    copy_failure: &'a mut Option<BuiltinErrorCategory>,
    callback_failure: &'a mut Option<RuntimeError>,
}

// Synchronous native callbacks necessarily keep the native caller alive. This
// bound is independent of the (heap-backed) m call-depth budget and must not
// increase when a host raises RuntimeConfig::maximum_call_depth.
const MAXIMUM_NATIVE_CALLBACK_DEPTH: usize = 16;

impl InterpreterLanguageService<'_> {
    fn callback_error(error: &RuntimeError) -> BuiltinError {
        let category = if matches!(error.kind, RuntimeErrorKind::Cancelled) {
            BuiltinErrorCategory::Cancelled
        } else {
            BuiltinErrorCategory::Other
        };
        let identifier = error.kind.catchable_identifier().map(ToOwned::to_owned);
        let message = identifier.as_deref().map_or_else(
            || error.to_string(),
            |identifier| Interpreter::language_error_message(error, identifier),
        );
        let builtin = BuiltinError::new(category, message);
        if let Some(identifier) = identifier {
            builtin.with_identifier(identifier)
        } else {
            builtin
        }
    }
}

impl LanguageService for InterpreterLanguageService<'_> {
    fn class_name(&self, value: &Value) -> String {
        self.interpreter.value_class_name(value)
    }

    fn copy(&mut self, value: &Value) -> Result<Value, BuiltinError> {
        if self.copy_failure.is_some() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::LanguageCopy,
                "the built-in language-copy transaction has already failed",
            ));
        }
        match self.interpreter.language_copy_tracked(value) {
            Ok((value, mut handles)) => {
                if self.created.try_reserve(handles.len()).is_err() {
                    self.interpreter.rollback_assignment_copies(&handles);
                    self.interpreter.rollback_assignment_copies(self.created);
                    self.created.clear();
                    *self.copy_failure = Some(BuiltinErrorCategory::LanguageCopy);
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::LanguageCopy,
                        "the built-in language-copy transaction exceeded runtime limits",
                    ));
                }
                self.created.append(&mut handles);
                Ok(value)
            }
            Err(error) => {
                self.interpreter.rollback_assignment_copies(self.created);
                self.created.clear();
                let category = if matches!(error.kind, RuntimeErrorKind::Cancelled) {
                    BuiltinErrorCategory::Cancelled
                } else {
                    BuiltinErrorCategory::LanguageCopy
                };
                *self.copy_failure = Some(category);
                Err(BuiltinError::new(
                    category,
                    "the runtime could not complete the built-in language copy",
                ))
            }
        }
    }

    fn invoke(
        &mut self,
        callable: &Value,
        arguments: Vec<Value>,
        requested_outputs: usize,
    ) -> crate::BuiltinResult {
        if let Some(error) = self.callback_failure.as_ref() {
            return Err(Self::callback_error(error));
        }
        let maximum = self
            .interpreter
            .config
            .maximum_call_depth
            .min(MAXIMUM_NATIVE_CALLBACK_DEPTH);
        if self.interpreter.language_callback_depth >= maximum {
            let error = self.interpreter.error(
                RuntimeErrorKind::CallDepthExceeded { maximum },
                self.location,
            );
            let builtin = Self::callback_error(&error);
            *self.callback_failure = Some(error);
            return Err(builtin);
        }
        let previous_depth = self.interpreter.language_callback_depth;
        self.interpreter.language_callback_depth = previous_depth + 1;
        let result = self.interpreter.call_value_owned(
            &self.module,
            callable,
            arguments,
            requested_outputs,
            AccessContext::external(),
            self.location,
        );
        self.interpreter.language_callback_depth = previous_depth;
        match result {
            Ok(values) => Ok(values),
            Err(error) => {
                let builtin = Self::callback_error(&error);
                *self.callback_failure = Some(error);
                Err(builtin)
            }
        }
    }

    fn display_format(&self) -> DisplayFormat {
        self.interpreter.display_format
    }

    fn set_display_format(&mut self, format: DisplayFormat) {
        self.interpreter.display_format = format;
    }

    fn warning_enabled(&self, identifier: &str) -> bool {
        self.interpreter.warning_state.enabled(identifier)
    }

    fn set_warning_enabled(&mut self, identifier: Option<&str>, enabled: bool) -> bool {
        self.interpreter
            .warning_state
            .set_enabled(identifier, enabled)
    }

    fn last_warning(&self) -> (&str, &str) {
        self.interpreter.warning_state.last()
    }

    fn set_last_warning(&mut self, message: String, identifier: String) {
        self.interpreter.warning_state.set_last(message, identifier);
    }
}

#[derive(Clone)]
struct RuntimeFunctionCode {
    module: Arc<BytecodeModule>,
    function: FunctionId,
    captures: Arc<BTreeMap<String, CapturedBinding>>,
    access_context: AccessContext,
}

#[derive(Clone, Debug, PartialEq)]
enum CapturedBinding {
    Value(Value),
    Shared(SharedBinding),
    MissingValue,
    ResolverOnly,
}

#[derive(Clone, Debug)]
struct SharedBinding(Arc<RwLock<Value>>);

impl SharedBinding {
    fn new(value: Value) -> Self {
        Self(Arc::new(RwLock::new(value)))
    }

    fn read(&self) -> Option<Value> {
        self.0.read().ok().map(|value| value.clone())
    }

    fn replace(&self, value: Value) -> Option<Value> {
        let Ok(mut target) = self.0.write() else {
            return None;
        };
        Some(std::mem::replace(&mut *target, value))
    }

    fn gc_value(&self) -> Value {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl CapturedBinding {
    fn gc_value(&self) -> Option<Value> {
        match self {
            Self::Value(value) => Some(value.clone()),
            Self::Shared(shared) => Some(shared.gc_value()),
            Self::MissingValue | Self::ResolverOnly => None,
        }
    }
}

impl PartialEq for SharedBinding {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        self.read()
            .zip(other.read())
            .is_some_and(|(left, right)| left == right)
    }
}

const REGISTERED_FUNCTION_HANDLE_BASE: u32 = 1 << 31;
const GC_ALLOCATION_THRESHOLD: u64 = 256;

#[derive(Clone)]
struct RuntimeMethodCode {
    module: Arc<BytecodeModule>,
    function: FunctionId,
    constructor_output: Option<LocalSlot>,
}

#[derive(Clone)]
struct RuntimeEnumMemberCode {
    module: Arc<BytecodeModule>,
    argument_initializer: FunctionId,
    argument_count: usize,
}

#[derive(Clone, Default)]
struct RuntimeClassCode {
    methods: BTreeMap<String, RuntimeMethodCode>,
    enumeration_members: BTreeMap<String, RuntimeEnumMemberCode>,
}

struct NativeObjectInstance {
    class: Arc<dyn NativeClass>,
    instance: NativeInstance,
}

impl Drop for NativeObjectInstance {
    fn drop(&mut self) {
        self.class.destroy(self.instance);
    }
}

#[derive(Clone)]
struct NativeObjectSnapshot {
    native: Arc<NativeObjectInstance>,
    valid: bool,
}

struct PendingNativeObject {
    temporary: ObjectHandle,
    class_id: ClassId,
    native: Arc<NativeObjectInstance>,
}

struct BuiltinNativeObjects {
    objects: BTreeMap<ObjectHandle, NativeObjectSnapshot>,
    classes: BTreeMap<NativeClassToken, (ClassId, Arc<dyn NativeClass>)>,
    pending: Vec<PendingNativeObject>,
    next_identifier: Option<u64>,
}

impl NativeObjectService for BuiltinNativeObjects {
    fn borrow_instance(
        &mut self,
        value: &Value,
        expected_class: NativeClassToken,
    ) -> Result<NativeInstance, BuiltinError> {
        let Value::Object(handle) = value else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                "a scalar native object is required",
            ));
        };
        let Some(native) = self.objects.get(handle) else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                "the object is not backed by a native class",
            ));
        };
        if !native.valid {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "the native object is no longer valid",
            ));
        }
        if native.native.class.token() != expected_class {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                "the native object has a different class",
            ));
        }
        Ok(native.native.instance)
    }

    fn create_instance(
        &mut self,
        class: NativeClassToken,
        instance: NativeInstance,
    ) -> Result<Value, BuiltinError> {
        let Some((class_id, implementation)) = self.classes.get(&class) else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                "the native class token is not registered in this session",
            ));
        };
        let identifier = self.next_identifier.ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                "the runtime object identifier space is exhausted",
            )
        })?;
        let next = identifier.checked_add(1).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                "the runtime object identifier space is exhausted",
            )
        })?;
        let temporary = ObjectHandle::new(identifier);
        if self.objects.contains_key(&temporary) {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::HostService,
                "the native-object transaction predicted a duplicate object identifier",
            ));
        }
        self.pending.try_reserve(1).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                "the native-object transaction exceeded host capacity",
            )
        })?;
        let native = Arc::new(NativeObjectInstance {
            class: Arc::clone(implementation),
            instance,
        });
        self.pending.push(PendingNativeObject {
            temporary,
            class_id: *class_id,
            native: Arc::clone(&native),
        });
        self.objects.insert(
            temporary,
            NativeObjectSnapshot {
                native: Arc::clone(&native),
                valid: true,
            },
        );
        self.next_identifier = Some(next);
        Ok(Value::Object(temporary))
    }
}

struct NativeConstructionCall {
    class: Arc<dyn NativeClass>,
    result: Arc<Mutex<Option<NativeInstance>>>,
}

impl BuiltinFunction for NativeConstructionCall {
    fn call(&self, arguments: &[Value], context: &mut BuiltinContext<'_>) -> crate::BuiltinResult {
        self.call_owned(arguments.to_vec(), context)
    }

    fn call_owned(
        &self,
        arguments: Vec<Value>,
        context: &mut BuiltinContext<'_>,
    ) -> crate::BuiltinResult {
        let instance = self.class.construct(arguments, context)?;
        *self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(instance);
        Ok(Vec::new())
    }
}

struct NativeMethodCall {
    class: Arc<dyn NativeClass>,
    method: Arc<str>,
    instance: NativeInstance,
}

struct NativePropertyGetCall {
    class: Arc<dyn NativeClass>,
    property: Arc<str>,
    instance: NativeInstance,
}

impl BuiltinFunction for NativePropertyGetCall {
    fn call(&self, _arguments: &[Value], context: &mut BuiltinContext<'_>) -> crate::BuiltinResult {
        self.class
            .get_property(&self.property, self.instance, context)
            .map(|value| vec![value])
    }
}

struct NativePropertySetCall {
    class: Arc<dyn NativeClass>,
    property: Arc<str>,
    instance: NativeInstance,
}

impl BuiltinFunction for NativePropertySetCall {
    fn call(&self, arguments: &[Value], context: &mut BuiltinContext<'_>) -> crate::BuiltinResult {
        self.call_owned(arguments.to_vec(), context)
    }

    fn call_owned(
        &self,
        arguments: Vec<Value>,
        context: &mut BuiltinContext<'_>,
    ) -> crate::BuiltinResult {
        let mut arguments = arguments.into_iter();
        let value = arguments.next().ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Other,
                "native property setter received no value",
            )
        })?;
        if arguments.next().is_some() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Other,
                "native property setter received more than one value",
            ));
        }
        self.class
            .set_property(&self.property, self.instance, value, context)?;
        Ok(Vec::new())
    }
}

impl BuiltinFunction for NativeMethodCall {
    fn call(&self, arguments: &[Value], context: &mut BuiltinContext<'_>) -> crate::BuiltinResult {
        self.call_owned(arguments.to_vec(), context)
    }

    fn call_owned(
        &self,
        arguments: Vec<Value>,
        context: &mut BuiltinContext<'_>,
    ) -> crate::BuiltinResult {
        self.class
            .invoke(&self.method, self.instance, arguments, context)
    }
}

#[derive(Clone)]
struct RuntimeListener {
    module: Arc<BytecodeModule>,
    source: ObjectHandle,
    event: String,
    callback: Value,
}

#[derive(Clone)]
struct RuntimeListenerClass {
    class_id: ClassId,
    source_key: PropertyKey,
}

struct ObjectValueParts {
    class_id: ClassId,
    shape: Shape,
    handles: Vec<ObjectHandle>,
}

const EXCEPTION_CLASS_NAME: &str = "MException";
const EXCEPTION_PROPERTIES: [&str; 4] = ["identifier", "message", "stack", "cause"];
const EVENT_LISTENER_CLASS_NAME: &str = "event.listener";
const EVENT_SOURCE_LISTENERS_PROPERTY: &str = "$openmat_event_listeners";
const EVENT_LISTENER_SOURCE_PROPERTY: &str = "$openmat_event_source";
const EVENT_LISTENER_CALLBACK_PROPERTY: &str = "$openmat_event_callback";

impl Interpreter {
    /// Creates an interpreter with an empty built-in registry and no output.
    ///
    /// # Errors
    ///
    /// Returns the bytecode verifier's first structural error.
    pub fn new(module: BytecodeModule) -> Result<Self, RuntimeBuildError> {
        Self::with_registry(module, BuiltinRegistry::new())
    }

    /// Creates an interpreter with a supplied built-in registry and no output.
    ///
    /// # Errors
    ///
    /// Returns the bytecode verifier's first structural error.
    pub fn with_registry(
        module: BytecodeModule,
        builtins: BuiltinRegistry,
    ) -> Result<Self, RuntimeBuildError> {
        Self::with_components(module, builtins, Box::new(NullOutput))
    }

    /// Creates an interpreter with supplied built-ins and structured output.
    ///
    /// # Errors
    ///
    /// Returns the bytecode verifier's first structural error.
    pub fn with_components(
        module: BytecodeModule,
        builtins: BuiltinRegistry,
        output: Box<dyn OutputSink>,
    ) -> Result<Self, RuntimeBuildError> {
        Self::with_components_and_linalg_provider(
            module,
            builtins,
            output,
            Arc::new(ReferenceProvider),
        )
    }

    /// Creates an interpreter with a supplied linear-algebra provider, an
    /// empty built-in registry, and no output.
    ///
    /// The interpreter retains a cloneable shared owner for the provider and
    /// borrows it for each supported numerical operation.
    ///
    /// # Errors
    ///
    /// Returns the bytecode verifier's first structural error.
    pub fn with_linalg_provider(
        module: BytecodeModule,
        linalg_provider: Arc<dyn LinalgProvider>,
    ) -> Result<Self, RuntimeBuildError> {
        Self::with_components_and_linalg_provider(
            module,
            BuiltinRegistry::new(),
            Box::new(NullOutput),
            linalg_provider,
        )
    }

    /// Creates an interpreter with supplied built-ins, structured output, and
    /// a shared linear-algebra provider.
    ///
    /// # Errors
    ///
    /// Returns the bytecode verifier's first structural error.
    pub fn with_components_and_linalg_provider(
        module: BytecodeModule,
        builtins: BuiltinRegistry,
        output: Box<dyn OutputSink>,
        linalg_provider: Arc<dyn LinalgProvider>,
    ) -> Result<Self, RuntimeBuildError> {
        verify(&module).map_err(|verification| RuntimeBuildError { verification })?;
        let mut interpreter = Self {
            module: Arc::new(module),
            workspace: Workspace::new(),
            base_imports: Vec::new(),
            graphics: Arc::new(Mutex::new(GraphicsSession::new())),
            random: Arc::new(Mutex::new(crate::RandomSession::new())),
            file_system: None,
            persistent_bindings: BTreeMap::new(),
            builtins,
            linalg_provider,
            fft_provider: Arc::new(RustFftProvider::default()),
            regex_provider: Arc::new(FancyRegexProvider),
            cancellation: CancellationToken::new(),
            output,
            display_format: DisplayFormat::default(),
            warning_state: WarningState::default(),
            config: RuntimeConfig::default(),
            call_stack: Vec::new(),
            classes: ClassRegistry::new(),
            objects: ObjectStore::new(),
            object_references: BTreeMap::new(),
            exception_errors: BTreeMap::new(),
            exception_class: None,
            pending_exception: None,
            last_error: None,
            source_metadata: BTreeMap::new(),
            class_code: BTreeMap::new(),
            legacy_classes: BTreeMap::new(),
            native_classes: BTreeMap::new(),
            native_instances: BTreeMap::new(),
            class_handles: BTreeMap::new(),
            enumeration_members: BTreeMap::new(),
            enumeration_members_loading: BTreeSet::new(),
            event_source_slots: BTreeMap::new(),
            event_listeners: BTreeMap::new(),
            event_listener_order: BTreeMap::new(),
            active_event_listeners: BTreeSet::new(),
            destroying_event_sources: BTreeSet::new(),
            event_listener_class: None,
            event_dispatch_depth: 0,
            pending_event_errors: Vec::new(),
            function_handles: BTreeMap::new(),
            named_function_handles: BTreeMap::new(),
            named_function_fallbacks: BTreeMap::new(),
            module_loader: ModuleLoader::new(),
            next_function_handle: Some(REGISTERED_FUNCTION_HANDLE_BASE),
            timer_handles: BTreeMap::new(),
            next_timer_handle: Some(1),
            default_timer: None,
            language_callback_depth: 0,
            builtin_workspace: None,
            builtin_scope_context: None,
            pending_scope_bindings: Vec::new(),
            next_scope_id: Some(1),
            displaced_workspaces: Vec::new(),
            active_frame_roots: Vec::new(),
            finalization_safepoints: FinalizationSafepoints::default(),
            finalizer_queue: FinalizerQueue::default(),
            active_finalizer: None,
            pending_finalizer_errors: Vec::new(),
            ffi: ffi::FfiSession::default(),
        };
        interpreter.install_exception_class();
        interpreter.install_native_classes();
        Ok(interpreter)
    }

    /// Replaces the request module while retaining workspace, classes, and objects.
    ///
    /// # Errors
    ///
    /// Returns the verifier's first structural error without changing the active module.
    pub fn replace_module(&mut self, module: BytecodeModule) -> Result<(), RuntimeBuildError> {
        verify(&module).map_err(|verification| RuntimeBuildError { verification })?;
        self.module = Arc::new(module);
        Ok(())
    }

    /// Clears all language-visible and class/object session state.
    pub fn clear_session(&mut self) {
        if let Some(file_system) = &self.file_system {
            file_system
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .close_all_open_files();
        }
        self.workspace = Workspace::new();
        self.base_imports.clear();
        *self
            .graphics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = GraphicsSession::new();
        *self
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = crate::RandomSession::new();
        self.persistent_bindings.clear();
        self.classes = ClassRegistry::new();
        self.objects = ObjectStore::new();
        self.object_references.clear();
        self.exception_errors.clear();
        self.exception_class = None;
        self.pending_exception = None;
        self.last_error = None;
        self.source_metadata.clear();
        self.class_code.clear();
        self.legacy_classes.clear();
        self.native_instances.clear();
        self.native_classes.clear();
        self.class_handles.clear();
        self.enumeration_members.clear();
        self.enumeration_members_loading.clear();
        self.event_source_slots.clear();
        self.event_listeners.clear();
        self.event_listener_order.clear();
        self.active_event_listeners.clear();
        self.destroying_event_sources.clear();
        self.event_listener_class = None;
        self.event_dispatch_depth = 0;
        self.pending_event_errors.clear();
        self.function_handles.clear();
        self.named_function_handles.clear();
        self.named_function_fallbacks.clear();
        self.module_loader.clear();
        self.next_function_handle = Some(REGISTERED_FUNCTION_HANDLE_BASE);
        self.timer_handles.clear();
        self.next_timer_handle = Some(1);
        self.default_timer = None;
        self.language_callback_depth = 0;
        self.builtin_workspace = None;
        self.builtin_scope_context = None;
        self.pending_scope_bindings.clear();
        self.next_scope_id = Some(1);
        self.displaced_workspaces.clear();
        self.call_stack.clear();
        self.active_frame_roots.clear();
        self.finalization_safepoints = FinalizationSafepoints::default();
        self.finalizer_queue = FinalizerQueue::default();
        self.active_finalizer = None;
        self.pending_finalizer_errors.clear();
        self.ffi = ffi::FfiSession::default();
        self.install_exception_class();
        self.install_native_classes();
        self.display_format = DisplayFormat::default();
        self.warning_state = WarningState::default();
    }

    /// Returns the language-visible class name, including registered objects.
    #[must_use]
    pub fn value_class_name(&self, value: &Value) -> String {
        if let Some(name) = self.ffi_value_class_name(value) {
            return name.to_owned();
        }
        if let Ok(class_id) = self.object_value_class_id(value)
            && let Ok(class) = self.classes.class(class_id)
        {
            return class.name().to_owned();
        }
        value.class_name().to_owned()
    }

    /// Returns the number of object records retained by this runtime session.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Returns read-only per-kernel object collector metrics.
    #[must_use]
    pub const fn gc_metrics(&self) -> GcMetrics {
        self.objects.gc_metrics()
    }

    /// Returns the fixed allocation-pressure threshold in object-slot units.
    ///
    /// Collection is deferred until an interpreter safe point even after this
    /// threshold is crossed.
    #[must_use]
    pub const fn gc_allocation_threshold(&self) -> u64 {
        GC_ALLOCATION_THRESHOLD
    }

    /// Forces a collection between executions for runtime integration tests.
    ///
    /// This is a Rust host/test hook only; no language-visible `gc` built-in is
    /// registered by the runtime.
    #[doc(hidden)]
    pub fn collect_garbage_for_tests(&mut self) -> GcCollection {
        frames::run(async |stack| self.collect_garbage_for_tests_on_stack(stack).await)
    }

    async fn collect_garbage_for_tests_on_stack(&mut self, stack: &mut FrameStack) -> GcCollection {
        stack
            .run(|stack| {
                self.collect_garbage_at_safepoint_on_stack(
                    stack,
                    std::iter::empty::<&Value>(),
                    SafepointReason::CommandBoundary,
                    None,
                )
            })
            .await
            .expect("a forced collection always runs")
    }

    /// Returns the number of classes registered in this runtime session.
    #[must_use]
    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    /// Returns the persistent workspace.
    #[must_use]
    pub const fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Returns the ordered package imports retained by the base workspace.
    #[must_use]
    pub fn workspace_imports(&self) -> &[String] {
        &self.base_imports
    }

    /// Returns mutable access to the persistent workspace between executions.
    pub const fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    fn replace_workspace_binding(&mut self, name: impl Into<String>, value: Value) {
        if let Some(previous) = self.workspace.insert(name, value) {
            self.note_root_removal(&previous);
        }
    }

    fn remove_workspace_binding(&mut self, name: &str) {
        if let Some(previous) = self.workspace.remove(name) {
            self.note_root_removal(&previous);
        }
    }

    fn clear_workspace_bindings(&mut self) {
        let previous = std::mem::take(&mut self.workspace);
        for (_, value) in previous.iter() {
            self.note_root_removal(value);
        }
    }

    fn base_workspace(&self) -> &Workspace {
        self.displaced_workspaces.first().unwrap_or(&self.workspace)
    }

    fn apply_base_scope_binding(&mut self, name: String, value: Value) {
        let previous = if let Some(base) = self.displaced_workspaces.first_mut() {
            if matches!(value, Value::Nothing) {
                base.remove(&name)
            } else {
                base.insert(name, value)
            }
        } else if matches!(value, Value::Nothing) {
            self.workspace.remove(&name)
        } else {
            self.workspace.insert(name, value)
        };
        if let Some(previous) = previous {
            self.note_root_removal(&previous);
        }
    }

    fn enter_dynamic_workspace(
        &mut self,
        workspace: Workspace,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        self.displaced_workspaces
            .try_reserve(1)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let previous = std::mem::replace(&mut self.workspace, workspace);
        self.displaced_workspaces.push(previous);
        Ok(())
    }

    fn leave_dynamic_workspace(&mut self) -> RuntimeResult<Workspace> {
        let previous = self
            .displaced_workspaces
            .pop()
            .ok_or_else(|| self.invalid_state("dynamic workspace stack underflowed"))?;
        Ok(std::mem::replace(&mut self.workspace, previous))
    }

    fn note_root_removals(&mut self, values: &[Value]) {
        if values.iter().any(|value| self.value_roots_object(value)) {
            self.finalization_safepoints.note_root_removal();
        }
    }

    fn note_root_removal(&mut self, value: &Value) {
        if self.value_roots_object(value) {
            self.finalization_safepoints.note_root_removal();
        }
    }

    fn value_roots_object(&self, value: &Value) -> bool {
        let mut found = false;
        let mut visited_functions = BTreeSet::new();
        trace_gc_value(
            value,
            &self.function_handles,
            &mut visited_functions,
            &mut |_| found = true,
        );
        found
    }

    #[cfg(test)]
    fn take_root_removal_safepoint_for_tests(&mut self) -> bool {
        self.finalization_safepoints.take_root_removal()
    }

    /// Returns the per-session command-window display settings.
    #[must_use]
    pub const fn display_format(&self) -> DisplayFormat {
        self.display_format
    }

    /// Returns the session-owned graphics hierarchy.
    pub fn graphics_session(&self) -> MutexGuard<'_, GraphicsSession> {
        self.graphics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns mutable graphics access between source executions.
    pub fn graphics_session_mut(&mut self) -> MutexGuard<'_, GraphicsSession> {
        self.graphics_session()
    }

    /// Clones the session-owned graphics synchronization handle for a host
    /// adapter. Each graphics built-in remains the atomic mutation boundary.
    #[must_use]
    pub fn graphics_session_handle(&self) -> Arc<Mutex<GraphicsSession>> {
        Arc::clone(&self.graphics)
    }

    /// Returns the number of function-owned persistent bindings in this session.
    ///
    /// This is read-only host introspection. Persistent values remain an
    /// interpreter implementation detail and are not exposed as a process ABI.
    #[must_use]
    pub fn persistent_binding_count(&self) -> usize {
        self.persistent_bindings.len()
    }

    /// Returns the built-in registry.
    #[must_use]
    pub const fn builtins(&self) -> &BuiltinRegistry {
        &self.builtins
    }

    /// Returns mutable registry access between executions.
    pub const fn builtins_mut(&mut self) -> &mut BuiltinRegistry {
        &mut self.builtins
    }

    /// Replaces the language-level filesystem boundary used by file built-ins.
    pub fn set_file_system_service(&mut self, service: Box<dyn crate::FileSystemService>) {
        self.file_system = Some(Arc::new(Mutex::new(service)));
    }

    /// Returns the shared linear-algebra provider selected for this interpreter.
    #[must_use]
    pub const fn linalg_provider(&self) -> &Arc<dyn LinalgProvider> {
        &self.linalg_provider
    }

    /// Replaces the Fourier-transform provider used by later built-in calls.
    pub fn set_fft_provider(&mut self, provider: Arc<dyn FftProvider>) {
        self.fft_provider = provider;
    }

    /// Returns the shared Fourier-transform provider selected for this interpreter.
    #[must_use]
    pub const fn fft_provider(&self) -> &Arc<dyn FftProvider> {
        &self.fft_provider
    }

    /// Replaces the regular-expression provider used by later built-in calls.
    pub fn set_regex_provider(&mut self, provider: Arc<dyn RegexProvider>) {
        self.regex_provider = provider;
    }

    /// Returns the shared regular-expression provider selected for this interpreter.
    #[must_use]
    pub const fn regex_provider(&self) -> &Arc<dyn RegexProvider> {
        &self.regex_provider
    }

    /// Returns a shareable cancellation token for the current interpreter.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Returns the current runtime configuration.
    #[must_use]
    pub const fn config(&self) -> RuntimeConfig {
        self.config
    }

    /// Replaces runtime safety limits between executions.
    pub const fn set_config(&mut self, config: RuntimeConfig) {
        self.config = config;
    }

    /// Registers source text used to render language-visible error stack frames.
    ///
    /// Re-registering an identifier replaces its previous metadata. Hosts should
    /// call this after allocating the [`SourceId`] used to parse a source unit.
    pub fn register_source_metadata(
        &mut self,
        source_id: SourceId,
        source_name: impl Into<String>,
        source: &str,
    ) {
        self.source_metadata.insert(
            source_id.raw(),
            SourceMetadata::new(source_name.into(), source),
        );
    }

    /// Executes the module entry function with MATLAB input arity semantics.
    ///
    /// The cancellation flag is not reset implicitly; a host must reset the
    /// token before starting new work after an interrupt.
    ///
    /// # Errors
    ///
    /// Returns a structured runtime error with source and call-stack context.
    pub fn execute_entry(&mut self, arguments: &[Value]) -> RuntimeResult<Vec<Value>> {
        frames::run(async |stack| self.execute_entry_on_stack(stack, arguments).await)
    }

    async fn execute_entry_on_stack(
        &mut self,
        stack: &mut FrameStack,
        arguments: &[Value],
    ) -> RuntimeResult<Vec<Value>> {
        self.call_stack.clear();
        let module = Arc::clone(&self.module);
        let result = stack
            .run(|stack| {
                self.execute_function(
                    stack,
                    &module,
                    module.entry,
                    arguments,
                    Invocation {
                        requested_outputs: 0,
                        seeded_local: None,
                        copy_arguments: true,
                        access_context: AccessContext::external(),
                        captures: Arc::new(BTreeMap::new()),
                        scope: InvocationScope::Base,
                    },
                )
            })
            .await;
        self.pending_exception = None;
        let returned = result.as_deref().unwrap_or(&[]);
        stack
            .run(|stack| {
                self.collect_garbage_at_safepoint_on_stack(
                    stack,
                    arguments.iter().chain(returned),
                    SafepointReason::CommandBoundary,
                    None,
                )
            })
            .await;
        if let Err(error) = &result {
            self.record_last_error(error);
        }
        result
    }

    #[allow(clippy::too_many_lines)]
    async fn execute_function(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        function_id: FunctionId,
        arguments: &[Value],
        invocation: Invocation,
    ) -> RuntimeResult<Vec<Value>> {
        if self.call_stack.len() >= self.config.maximum_call_depth {
            return Err(self.error(
                RuntimeErrorKind::CallDepthExceeded {
                    maximum: self.config.maximum_call_depth,
                },
                self.call_stack.last().and_then(|frame| frame.location),
            ));
        }

        let Some(function) = get(&module.functions, function_id.get()).cloned() else {
            return Err(self.error(
                RuntimeErrorKind::InvalidFunctionHandle {
                    handle: FunctionHandle::Bytecode(BytecodeFunctionHandle::new(
                        function_id.get(),
                    )),
                },
                self.call_stack.last().and_then(|frame| frame.location),
            ));
        };
        let prepared_scope = self.builtin_scope_context.take();
        self.builtin_workspace = None;
        let (scope_address, caller_scope) = match invocation.scope {
            InvocationScope::Base => (ScopeAddress::Base, None),
            InvocationScope::New => {
                let scope_id = self.next_scope_id.ok_or_else(|| {
                    self.invalid_state("language scope identifier registry is exhausted")
                })?;
                self.next_scope_id = scope_id.checked_add(1);
                (ScopeAddress::Frame(scope_id), prepared_scope)
            }
            InvocationScope::Reuse(scope) => (scope.address, scope.caller.clone()),
        };
        let expected = usize::try_from(function.parameter_count).unwrap_or(usize::MAX);
        self.call_stack.push(StackTraceFrame {
            function: function_id,
            name: function.name.clone(),
            instruction: InstructionIndex::new(0),
            location: function
                .instructions
                .first()
                .and_then(|instruction| instruction.location),
        });

        let variadic = function
            .instructions
            .iter()
            .find(|instruction| {
                !matches!(
                    instruction.kind,
                    InstructionKind::DeclareNamedBindings { .. }
                        | InstructionKind::DeclareImports { .. }
                )
            })
            .is_some_and(|instruction| {
                matches!(instruction.kind, InstructionKind::LoadVariadicInputs { .. })
            });
        // Omitted fixed inputs stay uninitialized. MATLAB checks them when
        // their value is read, allowing nargin/exists guards inside the body.
        let arity_matches =
            function.argument_layout.is_some() || variadic || arguments.len() <= expected;

        let result = if arity_matches {
            let prepared_arguments = if invocation.copy_arguments {
                self.language_copy_values(arguments)
            } else {
                Ok(arguments.to_vec())
            };
            let prepared_arguments = prepared_arguments.and_then(|values| {
                if function.argument_layout.is_none() {
                    return Ok((values, None));
                }
                crate::arguments::bind(&function, &values)
                    .map(|bound| (bound, Some(values)))
                    .map_err(|message| {
                        self.intrinsic_builtin_error(
                            "arguments",
                            BuiltinErrorCategory::Domain,
                            Some("OpenMat:arguments:InvalidInput"),
                            &message,
                            None,
                        )
                    })
            });
            match prepared_arguments {
                Err(error) => Err(error),
                Ok((prepared_arguments, original_inputs)) => {
                    if let Some(mut frame) = ExecutionFrame::new(
                        scope_address,
                        caller_scope,
                        function_id,
                        &function,
                        &prepared_arguments,
                        invocation.requested_outputs,
                        invocation.seeded_local,
                        invocation.access_context,
                        Arc::clone(&invocation.captures),
                    ) {
                        // MATLAB counts supplied positional inputs, excluding
                        // name-value pairs and inserted defaults. Keep the
                        // original language-copied inputs as frame roots.
                        frame.input_count = u32::try_from(
                            function
                                .argument_layout
                                .as_ref()
                                .map_or(arguments.len(), |layout| {
                                    crate::arguments::positional_inputs(layout, arguments)
                                }),
                        )
                        .unwrap_or(u32::MAX);
                        if let Some(original_inputs) = original_inputs {
                            frame.call_inputs = original_inputs;
                        }
                        self.active_frame_roots.push(frame.gc_roots());
                        let result = stack
                            .run(|stack| {
                                self.run_frame_with_handlers(stack, module, &function, &mut frame)
                            })
                            .await;
                        let scope_had_objects = frame
                            .gc_roots()
                            .iter()
                            .any(|value| self.value_roots_object(value));
                        self.active_frame_roots.pop();
                        if !self.active_frame_roots.is_empty()
                            && (scope_had_objects
                                || self.finalization_safepoints.root_removal_pending())
                        {
                            let returned = result.as_deref().unwrap_or(&[]);
                            stack
                                .run(|stack| {
                                    self.collect_garbage_at_safepoint_on_stack(
                                        stack,
                                        arguments.iter().chain(returned),
                                        SafepointReason::ScopeExit,
                                        None,
                                    )
                                })
                                .await;
                        }
                        result
                    } else {
                        Err(self.error(
                            RuntimeErrorKind::InvalidExecutionState {
                                message: "frame dimensions cannot be represented on this platform"
                                    .to_owned(),
                                array: None,
                            },
                            None,
                        ))
                    }
                }
            }
        } else {
            Err(self.error(
                RuntimeErrorKind::InputArity {
                    function: function.name.clone(),
                    expected,
                    actual: arguments.len(),
                },
                None,
            ))
        };
        self.call_stack.pop();
        result
    }

    async fn run_frame_with_handlers(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        function: &Function,
        frame: &mut ExecutionFrame,
    ) -> RuntimeResult<Vec<Value>> {
        loop {
            match stack
                .run(|stack| self.run_frame(stack, module, function, frame))
                .await
            {
                Ok(values) => return Ok(values),
                Err(error) => {
                    self.builtin_workspace = None;
                    self.builtin_scope_context = None;
                    self.pending_scope_bindings
                        .retain(|change| change.target != frame.scope_address);
                    let failed_instruction =
                        InstructionIndex::new(frame.instruction.get().saturating_sub(1));
                    self.unwind_frame_error(function, frame, failed_instruction, error)?;
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn run_frame(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        function: &Function,
        frame: &mut ExecutionFrame,
    ) -> RuntimeResult<Vec<Value>> {
        loop {
            self.refresh_current_frame_roots(frame);
            if !self.finalization_safepoints.is_draining()
                && self.finalization_safepoints.take_root_removal()
            {
                stack
                    .run(|stack| {
                        self.collect_garbage_at_safepoint_on_stack(
                            stack,
                            std::iter::empty::<&Value>(),
                            SafepointReason::RootRemoval,
                            Some(frame),
                        )
                    })
                    .await;
            } else if self.objects.collection_due(GC_ALLOCATION_THRESHOLD) {
                stack
                    .run(|stack| {
                        self.collect_garbage_at_safepoint_on_stack(
                            stack,
                            std::iter::empty::<&Value>(),
                            SafepointReason::AllocationDebt,
                            None,
                        )
                    })
                    .await;
            }
            let instruction_index = frame.instruction;
            let Some(instruction) = get(&function.instructions, instruction_index.get()).cloned()
            else {
                return Err(self.error(
                    RuntimeErrorKind::InstructionPointerOutOfBounds {
                        function: frame.function,
                        instruction: instruction_index,
                    },
                    None,
                ));
            };
            self.update_stack(instruction_index, instruction.location);
            self.check_cancelled(instruction.location)?;

            frame.instruction = InstructionIndex::new(instruction_index.get().saturating_add(1));

            match instruction.kind {
                InstructionKind::RequireSinglePack { pack } => {
                    let count = self.read_pack_register(frame, pack)?.len();
                    if count != 1 {
                        return Err(self.error(
                            array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                                selected: u64::try_from(count).unwrap_or(u64::MAX),
                                supplied: 1,
                            }),
                            instruction.location,
                        ));
                    }
                }
                InstructionKind::RequireDefined { value, name } => {
                    if matches!(self.read_register(frame, value)?, Value::Nothing) {
                        let name = global_name(function, name)
                            .ok_or_else(|| self.invalid_state("checked name escaped validation"))?;
                        let missing_input = matches!(frame.named_bindings.get(name),
                            Some(NamedBindingKind::Local(slot)) if slot.get() < function.parameter_count
                                && slot.get() >= frame.input_count);
                        let kind = if missing_input {
                            RuntimeErrorKind::InputArity {
                                function: function.name.clone(),
                                expected: function.parameter_count as usize,
                                actual: frame.input_count as usize,
                            }
                        } else {
                            RuntimeErrorKind::UndefinedGlobal {
                                name: name.to_owned(),
                            }
                        };
                        return Err(self.error(kind, instruction.location));
                    }
                }
                InstructionKind::CountPlaceOutputs { dst, root, path } => {
                    let root = self.take_register(frame, root)?;
                    let path = self.resolve_place_path(function, frame, &path)?;
                    let count = stack
                        .run(|stack| {
                            self.count_place_outputs(
                                stack,
                                &root,
                                &path,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                    let count = u32::try_from(count).map_err(|_| {
                        self.error(
                            array_error(ArrayRuntimeError::SizeLimit),
                            instruction.location,
                        )
                    })?;
                    self.write_register(frame, dst, Value::Double(f64::from(count)))?;
                }
                InstructionKind::ApplyOutputPack {
                    dst_pack,
                    count,
                    target,
                    arguments,
                } => {
                    let count = self.read_pack_count(frame, count)?;
                    let arguments = self.read_apply_arguments(frame, &arguments)?;
                    self.prepare_builtin_scope(module, function, frame)?;
                    let returned = stack
                        .run(|stack| {
                            self.apply_output_pack(
                                stack,
                                module,
                                function,
                                frame,
                                target,
                                arguments,
                                count,
                                instruction.location,
                            )
                        })
                        .await;
                    self.apply_pending_scope_bindings(module, function, frame)?;
                    let mut returned = returned?;
                    if returned.len() < count {
                        return Err(self.error(
                            RuntimeErrorKind::MissingOutputs {
                                requested: count,
                                returned: returned.len(),
                            },
                            instruction.location,
                        ));
                    }
                    returned.truncate(count);
                    self.write_pack_register(frame, dst_pack, returned)?;
                }
                InstructionKind::SlicePack {
                    dst_pack,
                    source,
                    start,
                    count,
                } => {
                    let start = self.read_pack_count(frame, start)?;
                    let count = self.read_pack_count(frame, count)?;
                    let end = start.checked_add(count).ok_or_else(|| {
                        self.error(
                            array_error(ArrayRuntimeError::SizeLimit),
                            instruction.location,
                        )
                    })?;
                    let values = self.read_pack_register(frame, source)?;
                    let values = values
                        .get(start..end)
                        .ok_or_else(|| {
                            self.error(
                                RuntimeErrorKind::MissingOutputs {
                                    requested: end,
                                    returned: values.len(),
                                },
                                instruction.location,
                            )
                        })?
                        .to_vec();
                    self.write_pack_register(frame, dst_pack, values)?;
                }
                InstructionKind::DeclareNamedBindings { bindings } => {
                    for (name, kind) in bindings {
                        let name = global_name(function, name)
                            .ok_or_else(|| self.invalid_state("named binding escaped validation"))?
                            .to_owned();
                        frame.named_bindings.insert(name, kind);
                    }
                }
                InstructionKind::DeclareImports { imports } => {
                    for import in imports {
                        let import = global_name(function, import)
                            .ok_or_else(|| self.invalid_state("import name escaped validation"))?
                            .to_owned();
                        if !frame.imports.contains(&import) {
                            frame.imports.push(import);
                        }
                    }
                    if frame.scope_address == ScopeAddress::Base {
                        self.base_imports.clone_from(&frame.imports);
                    }
                }
                InstructionKind::ClearImports => {
                    if frame.scope_address != ScopeAddress::Base {
                        return Err(self.invalid_state(
                            "clear import escaped compiler base-workspace validation",
                        ));
                    }
                    frame.imports.clear();
                    self.base_imports.clear();
                }
                InstructionKind::LoadConstant { dst, constant } => {
                    let constant = get(&function.constants, constant.get())
                        .cloned()
                        .ok_or_else(|| self.invalid_state("constant index escaped validation"))?;
                    let value = self.constant_value(module, &constant, instruction.location)?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::Move { dst, src } => {
                    let value = self.read_register(frame, src)?.clone();
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::Binary {
                    operator,
                    dst,
                    lhs,
                    rhs,
                } => {
                    let lhs = self.read_register(frame, lhs)?.clone();
                    let rhs = self.read_register(frame, rhs)?.clone();
                    let value = stack
                        .run(|stack| {
                            self.evaluate_binary(
                                stack,
                                operator,
                                &lhs,
                                &rhs,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::SwitchMatch {
                    dst,
                    selector,
                    case_value,
                } => {
                    let selector = self.read_register(frame, selector)?;
                    let case_value = self.read_register(frame, case_value)?;
                    let matched = array_ops::switch_match(selector, case_value, &self.cancellation)
                        .map_err(|kind| self.error(kind, instruction.location))?;
                    self.write_register(frame, dst, Value::Logical(matched))?;
                }
                InstructionKind::BuildMatrix { dst, rows } => {
                    let rows = rows
                        .iter()
                        .map(|row| {
                            row.iter()
                                .map(|register| self.read_register(frame, *register).cloned())
                                .collect::<Result<Vec<_>, _>>()
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let has_tables = rows
                        .iter()
                        .flatten()
                        .any(|value| matches!(value, Value::Table(_)));
                    let has_objects = rows.iter().flatten().any(is_object_value);
                    let value = if has_tables {
                        self.build_table_matrix(&rows, instruction.location)?
                    } else if has_objects {
                        self.build_object_matrix(&rows, instruction.location)?
                    } else if let Some(value) = sparse::try_build_matrix(&rows, &self.cancellation)
                    {
                        value.map_err(|kind| self.error(kind, instruction.location))?
                    } else {
                        array_ops::build_matrix(&rows, &self.cancellation)
                            .map_err(|kind| self.error(kind, instruction.location))?
                    };
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::BuildCell { dst, rows } => {
                    let value = self.build_cell(frame, &rows, instruction.location)?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::Range {
                    dst,
                    start,
                    step,
                    end,
                } => {
                    let start = self.read_register(frame, start)?.clone();
                    let step = self.read_register(frame, step)?.clone();
                    let end = self.read_register(frame, end)?.clone();
                    let value = array_ops::make_range(&start, &step, &end, &self.cancellation)
                        .map_err(|kind| self.error(kind, instruction.location))?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::Transpose {
                    dst,
                    operand,
                    conjugate,
                } => {
                    let operand = self.read_register(frame, operand)?.clone();
                    let value = array_ops::transpose(&operand, conjugate, &self.cancellation)
                        .map_err(|kind| self.error(kind, instruction.location))?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::LoadLocal { dst, local } => {
                    let value = if let Some(shared) = frame.shared_locals.get(&local) {
                        shared
                            .read()
                            .ok_or_else(|| self.invalid_state("shared local lock is poisoned"))?
                    } else {
                        get(&frame.locals, local.get())
                            .cloned()
                            .ok_or_else(|| self.invalid_state("local index escaped validation"))?
                    };
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::StoreLocal { local, src } => {
                    let source = self.read_register(frame, src)?.clone();
                    let value = self.language_copy(&source)?;
                    if let Some(shared) = frame.shared_locals.get(&local) {
                        let previous = shared
                            .replace(value)
                            .ok_or_else(|| self.invalid_state("shared local lock is poisoned"))?;
                        self.note_root_removal(&previous);
                        continue;
                    }
                    let Some(target) = get_mut(&mut frame.locals, local.get()) else {
                        return Err(self.invalid_state("local index escaped validation"));
                    };
                    let previous = std::mem::replace(target, value);
                    self.note_root_removal(&previous);
                }
                InstructionKind::LoadCallInputCount { dst } => {
                    self.write_register(frame, dst, Value::Double(f64::from(frame.input_count)))?;
                }
                InstructionKind::LoadCallOutputCount { dst } => {
                    self.write_register(
                        frame,
                        dst,
                        Value::Double(f64::from(frame.requested_output_count)),
                    )?;
                }
                InstructionKind::LoadVariadicInputs { dst } => {
                    let start = usize::try_from(function.parameter_count).map_err(|_| {
                        self.error(
                            array_error(ArrayRuntimeError::SizeLimit),
                            instruction.location,
                        )
                    })?;
                    let count = frame.call_inputs.len().saturating_sub(start);
                    let columns = u64::try_from(count).map_err(|_| {
                        self.error(
                            array_error(ArrayRuntimeError::SizeLimit),
                            instruction.location,
                        )
                    })?;
                    let rows = u64::from(count != 0);
                    let shape = Shape::new([rows, columns]).map_err(|_| {
                        self.error(
                            array_error(ArrayRuntimeError::SizeLimit),
                            instruction.location,
                        )
                    })?;
                    let values = frame.call_inputs.get(start..).unwrap_or(&[]).to_vec();
                    let packed = CellArray::from_values(shape, values)
                        .map(Value::Cell)
                        .map_err(|error| {
                            self.aggregate_value_error(
                                "variadic input construction",
                                error,
                                instruction.location,
                            )
                        })?;
                    self.write_register(frame, dst, packed)?;
                }
                InstructionKind::LoadGlobal {
                    dst,
                    name,
                    construct_if_class,
                } => {
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("global name escaped validation"))?;
                    let resolver_only = matches!(
                        frame.captures.get(name),
                        Some(CapturedBinding::ResolverOnly)
                    );
                    let value = if let Some(value) = frame.dynamic_locals.get(name) {
                        value.clone()
                    } else if let Some(CapturedBinding::Value(value)) = frame.captures.get(name) {
                        value.clone()
                    } else if let Some(CapturedBinding::Shared(value)) = frame.captures.get(name) {
                        value
                            .read()
                            .ok_or_else(|| self.invalid_state("shared capture lock is poisoned"))?
                    } else if matches!(
                        frame.captures.get(name),
                        Some(CapturedBinding::MissingValue)
                    ) {
                        return Err(self.error(
                            RuntimeErrorKind::UndefinedGlobal {
                                name: name.to_owned(),
                            },
                            instruction.location,
                        ));
                    } else if !resolver_only {
                        if let Some(value) = self.workspace.get(name) {
                            value.clone()
                        } else if let Some(function) = module_function_named(module, name) {
                            let callable = self.constant_value(
                                module,
                                &Constant::Function(function),
                                instruction.location,
                            )?;
                            stack
                                .run(|stack| {
                                    self.load_named_callable_value(
                                        stack,
                                        module,
                                        callable,
                                        construct_if_class,
                                        frame.access_context,
                                        instruction.location,
                                    )
                                })
                                .await?
                        } else if let Ok(class) = self.classes.class_named(name) {
                            let handle = ClassHandle::new(class.id().get());
                            stack
                                .run(|stack| {
                                    self.load_named_callable_value(
                                        stack,
                                        module,
                                        Value::Function(FunctionHandle::Class(handle)),
                                        construct_if_class,
                                        frame.access_context,
                                        instruction.location,
                                    )
                                })
                                .await?
                        } else if event_runtime_named(name) {
                            let callable = Value::Function(FunctionHandle::Bytecode(
                                self.register_named_function_handle(name, instruction.location)?,
                            ));
                            stack
                                .run(|stack| {
                                    self.load_named_callable_value(
                                        stack,
                                        module,
                                        callable,
                                        construct_if_class,
                                        frame.access_context,
                                        instruction.location,
                                    )
                                })
                                .await?
                        } else if let Some(intrinsic) = intrinsic_named(name) {
                            stack
                                .run(|stack| {
                                    self.load_named_callable_value(
                                        stack,
                                        module,
                                        Value::Function(FunctionHandle::Intrinsic(intrinsic)),
                                        construct_if_class,
                                        frame.access_context,
                                        instruction.location,
                                    )
                                })
                                .await?
                        } else if let Some(handle) = self.builtins.handle_by_name(name) {
                            stack
                                .run(|stack| {
                                    self.load_named_callable_value(
                                        stack,
                                        module,
                                        Value::Function(FunctionHandle::Builtin(handle)),
                                        construct_if_class,
                                        frame.access_context,
                                        instruction.location,
                                    )
                                })
                                .await?
                        } else if let Some(callable) =
                            self.resolve_dynamic_file_callable(name, instruction.location)?
                        {
                            stack
                                .run(|stack| {
                                    self.load_named_callable_value(
                                        stack,
                                        module,
                                        callable,
                                        construct_if_class,
                                        frame.access_context,
                                        instruction.location,
                                    )
                                })
                                .await?
                        } else {
                            return Err(self.error(
                                RuntimeErrorKind::UndefinedGlobal {
                                    name: name.to_owned(),
                                },
                                instruction.location,
                            ));
                        }
                    } else if let Some(function) = module_function_named(module, name) {
                        let callable = self.constant_value(
                            module,
                            &Constant::Function(function),
                            instruction.location,
                        )?;
                        stack
                            .run(|stack| {
                                self.load_named_callable_value(
                                    stack,
                                    module,
                                    callable,
                                    construct_if_class,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    } else if let Ok(class) = self.classes.class_named(name) {
                        let handle = ClassHandle::new(class.id().get());
                        stack
                            .run(|stack| {
                                self.load_named_callable_value(
                                    stack,
                                    module,
                                    Value::Function(FunctionHandle::Class(handle)),
                                    construct_if_class,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    } else if event_runtime_named(name) {
                        let callable = Value::Function(FunctionHandle::Bytecode(
                            self.register_named_function_handle(name, instruction.location)?,
                        ));
                        stack
                            .run(|stack| {
                                self.load_named_callable_value(
                                    stack,
                                    module,
                                    callable,
                                    construct_if_class,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    } else if let Some(intrinsic) = intrinsic_named(name) {
                        stack
                            .run(|stack| {
                                self.load_named_callable_value(
                                    stack,
                                    module,
                                    Value::Function(FunctionHandle::Intrinsic(intrinsic)),
                                    construct_if_class,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    } else if let Some(handle) = self.builtins.handle_by_name(name) {
                        stack
                            .run(|stack| {
                                self.load_named_callable_value(
                                    stack,
                                    module,
                                    Value::Function(FunctionHandle::Builtin(handle)),
                                    construct_if_class,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    } else if let Some(callable) =
                        self.resolve_dynamic_file_callable(name, instruction.location)?
                    {
                        stack
                            .run(|stack| {
                                self.load_named_callable_value(
                                    stack,
                                    module,
                                    callable,
                                    construct_if_class,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    } else {
                        return Err(self.error(
                            RuntimeErrorKind::UndefinedGlobal {
                                name: name.to_owned(),
                            },
                            instruction.location,
                        ));
                    };
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::LoadCallTarget { dst, name } => {
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("call-target name escaped validation"))?;
                    let value = self.load_call_target(frame, name, instruction.location)?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::LoadQualifiedTarget {
                    dst,
                    root_found,
                    unresolved_suffix,
                    root,
                    qualified,
                    member_count,
                } => {
                    let root = global_name(function, root).ok_or_else(|| {
                        self.invalid_state("qualified-call root name escaped validation")
                    })?;
                    let candidates = qualified
                        .into_iter()
                        .map(|candidate| {
                            global_name(function, candidate)
                                .map(str::to_owned)
                                .ok_or_else(|| {
                                    self.invalid_state(
                                        "qualified-call candidate escaped validation",
                                    )
                                })
                        })
                        .collect::<RuntimeResult<Vec<_>>>()?;
                    let candidates = candidates
                        .into_iter()
                        .filter(|candidate| {
                            qualified_candidate_visible(
                                root,
                                candidate,
                                member_count,
                                &frame.imports,
                            )
                        })
                        .collect::<Vec<_>>();
                    let (target, found, suffix) = if let Some(value) = self
                        .resolve_global_name_if_present(module, frame, root, instruction.location)?
                    {
                        (value, true, member_count)
                    } else if !candidates.is_empty() {
                        let (value, suffix) = self.resolve_qualified_callable_candidates(
                            module,
                            &candidates,
                            member_count,
                            instruction.location,
                        )?;
                        (value, false, suffix)
                    } else {
                        return Err(self.error(
                            RuntimeErrorKind::UndefinedGlobal {
                                name: root.to_owned(),
                            },
                            instruction.location,
                        ));
                    };
                    self.write_register(frame, dst, target)?;
                    self.write_register(frame, root_found, Value::Logical(found))?;
                    self.write_register(
                        frame,
                        unresolved_suffix,
                        Value::Double(f64::from(suffix)),
                    )?;
                }
                InstructionKind::LoadFunctionHandle { dst, name } => {
                    let name = global_name(function, name).ok_or_else(|| {
                        self.invalid_state("function-handle name escaped validation")
                    })?;
                    let value = if let Some(target) = module_function_named(module, name) {
                        if function_shares_source(module, target, instruction.location) {
                            self.constant_value(
                                module,
                                &Constant::Function(target),
                                instruction.location,
                            )?
                        } else {
                            let code = RuntimeFunctionCode {
                                module: Arc::clone(module),
                                function: target,
                                captures: Arc::new(BTreeMap::new()),
                                access_context: AccessContext::external(),
                            };
                            Value::Function(FunctionHandle::Bytecode(
                                self.register_bound_named_function_handle(
                                    name,
                                    code,
                                    instruction.location,
                                )?,
                            ))
                        }
                    } else if let Ok(class) = self.classes.class_named(name) {
                        Value::Function(FunctionHandle::Class(ClassHandle::new(class.id().get())))
                    } else if let Some(intrinsic) = intrinsic_named(name) {
                        Value::Function(FunctionHandle::Intrinsic(intrinsic))
                    } else if let Some(handle) = self.builtins.handle_by_name(name) {
                        Value::Function(FunctionHandle::Builtin(handle))
                    } else {
                        Value::Function(FunctionHandle::Bytecode(
                            self.register_named_function_handle(name, instruction.location)?,
                        ))
                    };
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::LoadFunctionHandleCandidates { dst, names } => {
                    let names = names
                        .into_iter()
                        .map(|name| {
                            global_name(function, name)
                                .map(str::to_owned)
                                .ok_or_else(|| {
                                    self.invalid_state(
                                        "imported function-handle name escaped validation",
                                    )
                                })
                        })
                        .collect::<RuntimeResult<Vec<_>>>()?;
                    let leaf = names
                        .first()
                        .and_then(|name| name.rsplit('.').next())
                        .unwrap_or_default()
                        .to_owned();
                    let active_names = names
                        .iter()
                        .filter(|name| {
                            frame
                                .imports
                                .iter()
                                .any(|import| import_candidate_matches(import, name, &leaf))
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let mut resolved = None;
                    for name in &active_names {
                        if let Some(value) = self.resolve_named_callable_if_present(
                            module,
                            name,
                            instruction.location,
                        )? {
                            resolved = Some(value);
                            break;
                        }
                    }
                    if resolved.is_none() && active_names.is_empty() && !leaf.is_empty() {
                        resolved = self.resolve_named_callable_if_present(
                            module,
                            &leaf,
                            instruction.location,
                        )?;
                        if resolved.is_none() {
                            resolved = Some(Value::Function(FunctionHandle::Bytecode(
                                self.register_named_function_handle(&leaf, instruction.location)?,
                            )));
                        }
                    }
                    let value = resolved.ok_or_else(|| {
                        self.error(
                            RuntimeErrorKind::UnknownFunctionHandleTarget {
                                name: active_names.first().cloned().unwrap_or(leaf),
                            },
                            instruction.location,
                        )
                    })?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::MakeClosure {
                    dst,
                    function: closure,
                    captures,
                } => {
                    let mut captured = BTreeMap::new();
                    for (name, value, tombstone_if_missing) in captures {
                        let name = global_name(function, name)
                            .ok_or_else(|| self.invalid_state("closure name escaped validation"))?
                            .to_owned();
                        let binding = if let Some(register) = value {
                            let value = self.read_register(frame, register)?.clone();
                            if matches!(value, Value::Nothing) {
                                if tombstone_if_missing {
                                    CapturedBinding::MissingValue
                                } else {
                                    CapturedBinding::ResolverOnly
                                }
                            } else {
                                CapturedBinding::Value(self.language_copy(&value)?)
                            }
                        } else if let Some(binding) = frame.captures.get(&name) {
                            match binding {
                                CapturedBinding::Value(value) => {
                                    CapturedBinding::Value(self.language_copy(value)?)
                                }
                                CapturedBinding::Shared(value) => {
                                    let value = value.read().ok_or_else(|| {
                                        self.invalid_state("shared capture lock is poisoned")
                                    })?;
                                    CapturedBinding::Value(self.language_copy(&value)?)
                                }
                                CapturedBinding::MissingValue => CapturedBinding::MissingValue,
                                CapturedBinding::ResolverOnly => CapturedBinding::ResolverOnly,
                            }
                        } else if let Some(value) = self.workspace.get(&name).cloned() {
                            CapturedBinding::Value(self.language_copy(&value)?)
                        } else if tombstone_if_missing {
                            CapturedBinding::MissingValue
                        } else {
                            CapturedBinding::ResolverOnly
                        };
                        captured.insert(name, binding);
                    }
                    let handle = self.register_function_code(
                        RuntimeFunctionCode {
                            module: Arc::clone(module),
                            function: closure,
                            captures: Arc::new(captured),
                            access_context: frame.access_context,
                        },
                        instruction.location,
                    )?;
                    self.write_register(
                        frame,
                        dst,
                        Value::Function(FunctionHandle::Bytecode(handle)),
                    )?;
                }
                InstructionKind::MakeSharedClosure {
                    dst,
                    function: closure,
                    captures,
                } => {
                    let mut captured = BTreeMap::new();
                    for (name, source) in captures {
                        let name = global_name(function, name)
                            .ok_or_else(|| {
                                self.invalid_state("shared closure name escaped validation")
                            })?
                            .to_owned();
                        let shared = match source {
                            SharedCaptureSource::Local(local) => {
                                if let Some(shared) = frame.shared_locals.get(&local) {
                                    shared.clone()
                                } else {
                                    let value = get(&frame.locals, local.get())
                                        .cloned()
                                        .ok_or_else(|| {
                                            self.invalid_state("shared local escaped validation")
                                        })?;
                                    let shared = SharedBinding::new(value);
                                    frame.shared_locals.insert(local, shared.clone());
                                    shared
                                }
                            }
                            SharedCaptureSource::Enclosing => {
                                let Some(CapturedBinding::Shared(shared)) =
                                    frame.captures.get(&name)
                                else {
                                    return Err(self.invalid_state(
                                        "nested closure inherited a non-shared capture",
                                    ));
                                };
                                shared.clone()
                            }
                        };
                        captured.insert(name, CapturedBinding::Shared(shared));
                    }
                    let handle = self.register_function_code(
                        RuntimeFunctionCode {
                            module: Arc::clone(module),
                            function: closure,
                            captures: Arc::new(captured),
                            access_context: frame.access_context,
                        },
                        instruction.location,
                    )?;
                    self.write_register(
                        frame,
                        dst,
                        Value::Function(FunctionHandle::Bytecode(handle)),
                    )?;
                }
                InstructionKind::LoadCapture { dst, name } => {
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("capture name escaped validation"))?;
                    let Some(CapturedBinding::Shared(shared)) = frame.captures.get(name) else {
                        return Err(self.invalid_state("shared capture is missing"));
                    };
                    let value = shared
                        .read()
                        .ok_or_else(|| self.invalid_state("shared capture lock is poisoned"))?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::StoreCapture { name, src } => {
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("capture name escaped validation"))?;
                    let source = self.read_register(frame, src)?.clone();
                    let value = self.language_copy(&source)?;
                    let Some(CapturedBinding::Shared(shared)) = frame.captures.get(name) else {
                        return Err(self.invalid_state("shared capture is missing"));
                    };
                    let previous = shared
                        .replace(value)
                        .ok_or_else(|| self.invalid_state("shared capture lock is poisoned"))?;
                    self.note_root_removal(&previous);
                }
                InstructionKind::LoadGlobalOrNothing { dst, name } => {
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("global name escaped validation"))?;
                    let value = frame
                        .dynamic_locals
                        .get(name)
                        .or_else(|| self.workspace.get(name))
                        .cloned()
                        .unwrap_or(Value::Nothing);
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::StoreGlobal { name, src } => {
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("global name escaped validation"))?
                        .to_owned();
                    let source = self.read_register(frame, src)?.clone();
                    let value = self.language_copy(&source)?;
                    self.replace_workspace_binding(name, value);
                }
                InstructionKind::ClearGlobal { names } => {
                    for name in names {
                        let name = global_name(function, name)
                            .ok_or_else(|| self.invalid_state("clear name escaped validation"))?
                            .to_owned();
                        self.remove_workspace_binding(&name);
                    }
                }
                InstructionKind::ClearGlobalAll => self.clear_workspace_bindings(),
                InstructionKind::ClearLocal { locals } => {
                    for local in locals {
                        if let Some(shared) = frame.shared_locals.get(&local) {
                            let previous = shared.replace(Value::Nothing).ok_or_else(|| {
                                self.invalid_state("shared local lock is poisoned")
                            })?;
                            self.note_root_removal(&previous);
                        } else {
                            let Some(value) = get_mut(&mut frame.locals, local.get()) else {
                                return Err(self.invalid_state("local index escaped validation"));
                            };
                            let previous = std::mem::replace(value, Value::Nothing);
                            self.note_root_removal(&previous);
                        }
                    }
                }
                InstructionKind::ClearCapture { names } => {
                    for name in names {
                        let name = global_name(function, name).ok_or_else(|| {
                            self.invalid_state("capture clear name escaped validation")
                        })?;
                        let Some(CapturedBinding::Shared(shared)) = frame.captures.get(name) else {
                            return Err(self.invalid_state("shared capture is missing"));
                        };
                        let previous = shared
                            .replace(Value::Nothing)
                            .ok_or_else(|| self.invalid_state("shared capture lock is poisoned"))?;
                        self.note_root_removal(&previous);
                    }
                }
                InstructionKind::ClearDynamicBindings { names, except } => {
                    let names = names
                        .into_iter()
                        .map(|name| {
                            global_name(function, name)
                                .map(str::to_owned)
                                .ok_or_else(|| {
                                    self.invalid_state("dynamic clear name escaped validation")
                                })
                        })
                        .collect::<RuntimeResult<std::collections::BTreeSet<_>>>()?;
                    if except {
                        let mut removed = Vec::new();
                        frame.dynamic_locals.retain(|name, value| {
                            if names.contains(name) {
                                true
                            } else {
                                removed.push(value.clone());
                                false
                            }
                        });
                        self.note_root_removals(&removed);
                    } else if names.is_empty() {
                        let removed = std::mem::take(&mut frame.dynamic_locals)
                            .into_values()
                            .collect::<Vec<_>>();
                        self.note_root_removals(&removed);
                    } else {
                        for name in names {
                            if let Some(previous) = frame.dynamic_locals.remove(&name) {
                                self.note_root_removal(&previous);
                            }
                        }
                    }
                }
                InstructionKind::RegisterClass { class } => {
                    stack
                        .run(|stack| {
                            self.register_class(stack, module, class, instruction.location)
                        })
                        .await?;
                }
                InstructionKind::InvokeSuperclassConstructor {
                    superclass,
                    object,
                    arguments,
                } => {
                    let superclass = global_name(function, superclass)
                        .ok_or_else(|| self.invalid_state("superclass name escaped validation"))?
                        .to_owned();
                    let object = get(&frame.locals, object.get()).cloned().ok_or_else(|| {
                        self.invalid_state("constructor output local escaped validation")
                    })?;
                    let arguments = arguments
                        .iter()
                        .map(|register| self.read_register(frame, *register).cloned())
                        .collect::<Result<Vec<_>, _>>()?;
                    stack
                        .run(|stack| {
                            self.invoke_superclass_constructor(
                                stack,
                                &superclass,
                                &object,
                                &arguments,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                }
                InstructionKind::GetField { dst, object, name } => {
                    let object = self.read_register(frame, object)?.clone();
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("field name escaped validation"))?;
                    let value = if name == "__openmat_internal:class_metadata" {
                        self.class_metadata_value(&object, instruction.location)?
                    } else {
                        stack
                            .run(|stack| {
                                self.get_field_on_stack(
                                    stack,
                                    &object,
                                    name,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    };
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::SetField {
                    dst,
                    object,
                    name,
                    value,
                } => {
                    let object = self.read_register(frame, object)?.clone();
                    let value = self.read_register(frame, value)?.clone();
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("field name escaped validation"))?;
                    let assigned = stack
                        .run(|stack| {
                            self.set_field_on_stack(
                                stack,
                                &object,
                                name,
                                &value,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                    self.write_register(frame, dst, assigned)?;
                }
                InstructionKind::ApplyField {
                    outputs,
                    object,
                    name,
                    arguments,
                } => {
                    let object = self.read_register(frame, object)?.clone();
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("member name escaped validation"))?;
                    let native = self.is_native_object(&object);
                    let arguments = if native {
                        self.take_apply_arguments(frame, function, instruction_index, &arguments)?
                    } else {
                        self.read_apply_arguments(frame, &arguments)?
                    };
                    if native {
                        self.refresh_current_frame_roots(frame);
                    }
                    let returned = stack
                        .run(|stack| {
                            self.apply_field(
                                stack,
                                module,
                                &object,
                                name,
                                arguments,
                                outputs.len(),
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                    if returned.len() < outputs.len() {
                        return Err(self.error(
                            RuntimeErrorKind::MissingOutputs {
                                requested: outputs.len(),
                                returned: returned.len(),
                            },
                            instruction.location,
                        ));
                    }
                    for (register, value) in outputs.into_iter().zip(returned) {
                        self.write_register(frame, register, value)?;
                    }
                }
                InstructionKind::ApplyQualified {
                    outputs,
                    target,
                    unresolved_suffix,
                    members,
                    arguments,
                } => {
                    let target = self.read_register(frame, target)?.clone();
                    let all_members = members
                        .into_iter()
                        .map(|member| {
                            global_name(function, member)
                                .map(str::to_owned)
                                .ok_or_else(|| {
                                    self.invalid_state("qualified-call member escaped validation")
                                })
                        })
                        .collect::<RuntimeResult<Vec<_>>>()?;
                    let members =
                        self.qualified_suffix_members(frame, unresolved_suffix, &all_members)?;
                    let returned = if members.is_empty() {
                        let arguments = self.take_apply_arguments(
                            frame,
                            function,
                            instruction_index,
                            &arguments,
                        )?;
                        self.refresh_current_frame_roots(frame);
                        self.prepare_builtin_scope(module, function, frame)?;
                        let returned = stack
                            .run(|stack| {
                                self.apply_value_owned(
                                    stack,
                                    module,
                                    &target,
                                    arguments,
                                    outputs.len(),
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await;
                        self.apply_pending_scope_bindings(module, function, frame)?;
                        returned?
                    } else {
                        let (object, member) = stack
                            .run(|stack| {
                                self.resolve_qualified_member_receiver(
                                    stack,
                                    target,
                                    members,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?;
                        let native = self.is_native_object(&object);
                        let arguments = if native {
                            self.take_apply_arguments(
                                frame,
                                function,
                                instruction_index,
                                &arguments,
                            )?
                        } else {
                            self.read_apply_arguments(frame, &arguments)?
                        };
                        if native {
                            self.refresh_current_frame_roots(frame);
                        }
                        stack
                            .run(|stack| {
                                self.apply_field(
                                    stack,
                                    module,
                                    &object,
                                    member,
                                    arguments,
                                    outputs.len(),
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    };
                    if returned.len() < outputs.len() {
                        return Err(self.error(
                            RuntimeErrorKind::MissingOutputs {
                                requested: outputs.len(),
                                returned: returned.len(),
                            },
                            instruction.location,
                        ));
                    }
                    for (register, value) in outputs.into_iter().zip(returned) {
                        self.write_register(frame, register, value)?;
                    }
                }
                InstructionKind::GetQualifiedPack {
                    dst_pack,
                    target,
                    root_found,
                    unresolved_suffix,
                    members,
                } => {
                    let target = self.read_register(frame, target)?.clone();
                    let root_found = match self.read_register(frame, root_found)? {
                        Value::Logical(value) => *value,
                        _ => {
                            return Err(self.invalid_state(
                                "qualified-value root resolution flag is not logical",
                            ));
                        }
                    };
                    let all_members = members
                        .into_iter()
                        .map(|member| {
                            global_name(function, member)
                                .map(str::to_owned)
                                .ok_or_else(|| {
                                    self.invalid_state("qualified-value member escaped validation")
                                })
                        })
                        .collect::<RuntimeResult<Vec<_>>>()?;
                    let members = if root_found {
                        all_members.as_slice()
                    } else {
                        self.qualified_suffix_members(frame, unresolved_suffix, &all_members)?
                    };
                    let values = if !root_found && members.is_empty() {
                        self.prepare_builtin_scope(module, function, frame)?;
                        let returned = stack
                            .run(|stack| {
                                self.call_value(
                                    stack,
                                    module,
                                    &target,
                                    &[],
                                    1,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await;
                        self.apply_pending_scope_bindings(module, function, frame)?;
                        returned?
                    } else {
                        stack
                            .run(|stack| {
                                self.resolve_qualified_member_pack(
                                    stack,
                                    target,
                                    members,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    };
                    self.write_pack_register(frame, dst_pack, values)?;
                }
                InstructionKind::BraceApply {
                    dst_pack,
                    target,
                    arguments,
                } => {
                    let target = self.read_register(frame, target)?.clone();
                    let arguments = self.read_apply_arguments(frame, &arguments)?;
                    let values = self.brace_apply(&target, &arguments, instruction.location)?;
                    self.write_pack_register(frame, dst_pack, values)?;
                }
                InstructionKind::GetAggregateField {
                    dst_pack,
                    target,
                    field,
                } => {
                    let target = self.read_register(frame, target)?.clone();
                    let field = self.resolve_field_operand(function, frame, field)?;
                    let values = stack
                        .run(|stack| {
                            self.aggregate_field_pack(
                                stack,
                                &target,
                                &field,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                    self.write_pack_register(frame, dst_pack, values)?;
                }
                InstructionKind::AssignPlace {
                    dst,
                    root,
                    path,
                    source,
                    mode,
                } => {
                    let root = self.read_register(frame, root)?.clone();
                    let path = self.resolve_place_path(function, frame, &path)?;
                    let source = self.read_assignment_source(frame, source)?;
                    let assigned = stack
                        .run(|stack| {
                            self.assign_place(
                                stack,
                                &root,
                                &path,
                                &source,
                                mode,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                    self.write_register(frame, dst, assigned)?;
                }
                InstructionKind::AssignBindingPlace {
                    result,
                    binding,
                    root,
                    path,
                    source,
                    mode,
                } => {
                    let binding = self.resolve_binding_target(function, frame, binding)?;
                    let scalar_operands = match (&path[..], source, mode) {
                        (
                            [PlaceStep::Paren(arguments)],
                            ValueSource::One(value),
                            AssignmentMode::Store,
                        ) => match &arguments[..] {
                            [ApplyArgument::Value(index)] => Some((*index, value)),
                            _ => None,
                        },
                        _ => None,
                    };
                    let scalar_values = scalar_operands
                        .map(|(index, value)| {
                            Ok::<_, RuntimeError>((
                                self.read_register(frame, index)?.clone(),
                                self.read_register(frame, value)?.clone(),
                            ))
                        })
                        .transpose()?;
                    let (path, source, root) = if let Some((index, value)) = scalar_values {
                        let mut root = self.take_register(frame, root)?;
                        let mut previous_binding = self.take_binding(frame, &binding)?;
                        let restore_root_on_fallback = previous_binding
                            .as_ref()
                            .is_some_and(|previous| previous.shares_storage_with(&root));
                        if restore_root_on_fallback {
                            previous_binding = None;
                        }
                        match array_ops::try_index_assign_scalar_in_place(
                            &mut root,
                            &index,
                            &value,
                            &self.cancellation,
                        ) {
                            Ok(true) => {
                                if let Some(result) = result {
                                    self.commit_binding(frame, &binding, root.clone())?;
                                    self.write_register(frame, result, root)?;
                                } else {
                                    self.commit_binding(frame, &binding, root)?;
                                }
                                continue;
                            }
                            Ok(false) => {
                                if restore_root_on_fallback {
                                    self.commit_binding(frame, &binding, root.clone())?;
                                } else {
                                    self.restore_binding(frame, &binding, previous_binding)?;
                                }
                                (
                                    vec![ResolvedPlaceStep::Paren(vec![IndexInput::Value(index)])],
                                    AssignmentSource {
                                        values: vec![value],
                                        expanded: false,
                                    },
                                    root,
                                )
                            }
                            Err(kind) => {
                                if restore_root_on_fallback {
                                    self.commit_binding(frame, &binding, root)?;
                                } else {
                                    self.restore_binding(frame, &binding, previous_binding)?;
                                }
                                return Err(self.error(kind, instruction.location));
                            }
                        }
                    } else {
                        (
                            self.resolve_place_path(function, frame, &path)?,
                            self.read_assignment_source(frame, source)?,
                            self.take_register(frame, root)?,
                        )
                    };
                    let assigned = stack
                        .run(|stack| {
                            self.assign_place(
                                stack,
                                &root,
                                &path,
                                &source,
                                mode,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                    let stored = self.language_copy(&assigned)?;
                    self.commit_binding(frame, &binding, stored)?;
                    if let Some(result) = result {
                        self.write_register(frame, result, assigned)?;
                    }
                }
                InstructionKind::ResolveEnd {
                    dst,
                    target,
                    argument_index,
                    argument_count,
                } => {
                    let target = self.read_register(frame, target)?;
                    let value = self.resolve_end(
                        target,
                        argument_index,
                        argument_count,
                        instruction.location,
                    )?;
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::Unpack { outputs, pack } => {
                    let values = self.read_pack_register(frame, pack)?;
                    if values.len() < outputs.len() {
                        return Err(self.error(
                            RuntimeErrorKind::MissingOutputs {
                                requested: outputs.len(),
                                returned: values.len(),
                            },
                            instruction.location,
                        ));
                    }
                    let mut consumed = Vec::new();
                    consumed.try_reserve_exact(outputs.len()).map_err(|_| {
                        self.error(
                            array_error(ArrayRuntimeError::SizeLimit),
                            instruction.location,
                        )
                    })?;
                    consumed.extend_from_slice(&values[..outputs.len()]);
                    for (register, value) in outputs.into_iter().zip(consumed) {
                        self.write_register(frame, register, value)?;
                    }
                }
                InstructionKind::Jump { target } => frame.instruction = target,
                InstructionKind::JumpIfFalse { condition, target } => {
                    let condition = self.read_register(frame, condition)?;
                    let take_fallthrough =
                        array_ops::evaluate_condition(condition, &self.cancellation)
                            .map_err(|kind| self.error(kind, instruction.location))?;
                    if !take_fallthrough {
                        frame.instruction = target;
                    }
                }
                InstructionKind::Call {
                    outputs,
                    callee,
                    arguments,
                } => {
                    let callee = self.read_register(frame, callee)?.clone();
                    let arguments =
                        self.take_call_arguments(frame, function, instruction_index, &arguments)?;
                    self.refresh_current_frame_roots(frame);
                    self.prepare_builtin_scope(module, function, frame)?;
                    let returned = stack
                        .run(|stack| {
                            self.call_value_owned_on_stack(
                                stack,
                                module,
                                &callee,
                                arguments,
                                outputs.len(),
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await;
                    self.apply_pending_scope_bindings(module, function, frame)?;
                    let returned = returned?;
                    if returned.len() < outputs.len() {
                        return Err(self.error(
                            RuntimeErrorKind::MissingOutputs {
                                requested: outputs.len(),
                                returned: returned.len(),
                            },
                            instruction.location,
                        ));
                    }
                    for (register, value) in outputs.into_iter().zip(returned) {
                        self.write_register(frame, register, value)?;
                    }
                }
                InstructionKind::Apply {
                    outputs,
                    target,
                    arguments,
                } => {
                    let target = self.read_register(frame, target)?.clone();
                    let arguments = if matches!(target, Value::Function(_)) {
                        self.take_apply_arguments(frame, function, instruction_index, &arguments)?
                    } else {
                        self.read_apply_arguments(frame, &arguments)?
                    };
                    self.refresh_current_frame_roots(frame);
                    self.prepare_builtin_scope(module, function, frame)?;
                    let returned = stack
                        .run(|stack| {
                            self.apply_value_owned(
                                stack,
                                module,
                                &target,
                                arguments,
                                outputs.len(),
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await;
                    self.apply_pending_scope_bindings(module, function, frame)?;
                    let returned = returned?;
                    if returned.len() < outputs.len() {
                        return Err(self.error(
                            RuntimeErrorKind::MissingOutputs {
                                requested: outputs.len(),
                                returned: returned.len(),
                            },
                            instruction.location,
                        ));
                    }
                    for (register, value) in outputs.into_iter().zip(returned) {
                        self.write_register(frame, register, value)?;
                    }
                }
                InstructionKind::ApplyBinding {
                    outputs,
                    target,
                    arguments,
                } => {
                    let (target, arguments) = if outputs.len() == 1
                        && let [ApplyArgument::Value(index)] = &arguments[..]
                    {
                        let index = self.read_register(frame, *index)?.clone();
                        let target = self.take_register(frame, target)?;
                        match array_ops::try_index_real_scalar(&target, &index, &self.cancellation)
                        {
                            Ok(Some(value)) => {
                                self.write_register(frame, outputs[0], value)?;
                                continue;
                            }
                            Ok(None) => (target, vec![IndexInput::Value(index)]),
                            Err(kind) => return Err(self.error(kind, instruction.location)),
                        }
                    } else {
                        let arguments =
                            if matches!(self.read_register(frame, target)?, Value::Function(_)) {
                                self.take_apply_arguments(
                                    frame,
                                    function,
                                    instruction_index,
                                    &arguments,
                                )?
                            } else {
                                self.read_apply_arguments(frame, &arguments)?
                            };
                        let target = self.take_register(frame, target)?;
                        (target, arguments)
                    };
                    self.refresh_current_frame_roots(frame);
                    self.prepare_builtin_scope(module, function, frame)?;
                    let returned = stack
                        .run(|stack| {
                            self.apply_value_owned(
                                stack,
                                module,
                                &target,
                                arguments,
                                outputs.len(),
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await;
                    self.apply_pending_scope_bindings(module, function, frame)?;
                    let returned = returned?;
                    if returned.len() < outputs.len() {
                        return Err(self.error(
                            RuntimeErrorKind::MissingOutputs {
                                requested: outputs.len(),
                                returned: returned.len(),
                            },
                            instruction.location,
                        ));
                    }
                    for (register, value) in outputs.into_iter().zip(returned) {
                        self.write_register(frame, register, value)?;
                    }
                }
                InstructionKind::StatementApply {
                    target,
                    arguments,
                    result,
                    display,
                } => {
                    let target = self.read_register(frame, target)?.clone();
                    let arguments = if matches!(target, Value::Function(_)) {
                        self.take_apply_arguments(frame, function, instruction_index, &arguments)?
                    } else {
                        self.read_apply_arguments(frame, &arguments)?
                    };
                    self.refresh_current_frame_roots(frame);
                    let requested_outputs = usize::from(!matches!(target, Value::Function(_)));
                    self.prepare_builtin_scope(module, function, frame)?;
                    let returned = stack
                        .run(|stack| {
                            self.apply_value_owned(
                                stack,
                                module,
                                &target,
                                arguments,
                                requested_outputs,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await;
                    self.apply_pending_scope_bindings(module, function, frame)?;
                    let returned = returned?;
                    self.finish_statement_result(
                        function,
                        frame,
                        result,
                        display,
                        returned.into_iter().next(),
                        instruction.location,
                    )?;
                }
                InstructionKind::StatementApplyField {
                    object,
                    name,
                    arguments,
                    result,
                    display,
                } => {
                    let object = self.read_register(frame, object)?.clone();
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("member name escaped validation"))?;
                    let native = self.is_native_object(&object);
                    let arguments = if native {
                        self.take_apply_arguments(frame, function, instruction_index, &arguments)?
                    } else {
                        self.read_apply_arguments(frame, &arguments)?
                    };
                    if native {
                        self.refresh_current_frame_roots(frame);
                    }
                    let requested_outputs =
                        self.statement_field_output_count(&object, name, frame.access_context);
                    let returned = stack
                        .run(|stack| {
                            self.apply_field(
                                stack,
                                module,
                                &object,
                                name,
                                arguments,
                                requested_outputs,
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await?;
                    self.finish_statement_result(
                        function,
                        frame,
                        result,
                        display,
                        returned.into_iter().next(),
                        instruction.location,
                    )?;
                }
                InstructionKind::StatementApplyQualified {
                    target,
                    unresolved_suffix,
                    members,
                    arguments,
                    result,
                    display,
                } => {
                    let target = self.read_register(frame, target)?.clone();
                    let all_members = members
                        .into_iter()
                        .map(|member| {
                            global_name(function, member)
                                .map(str::to_owned)
                                .ok_or_else(|| {
                                    self.invalid_state("qualified-call member escaped validation")
                                })
                        })
                        .collect::<RuntimeResult<Vec<_>>>()?;
                    let members =
                        self.qualified_suffix_members(frame, unresolved_suffix, &all_members)?;
                    let returned = if members.is_empty() {
                        let arguments = self.take_apply_arguments(
                            frame,
                            function,
                            instruction_index,
                            &arguments,
                        )?;
                        self.refresh_current_frame_roots(frame);
                        self.prepare_builtin_scope(module, function, frame)?;
                        let returned = stack
                            .run(|stack| {
                                self.apply_value_owned(
                                    stack,
                                    module,
                                    &target,
                                    arguments,
                                    0,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await;
                        self.apply_pending_scope_bindings(module, function, frame)?;
                        returned?
                    } else {
                        let (object, member) = stack
                            .run(|stack| {
                                self.resolve_qualified_member_receiver(
                                    stack,
                                    target,
                                    members,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?;
                        let native = self.is_native_object(&object);
                        let arguments = if native {
                            self.take_apply_arguments(
                                frame,
                                function,
                                instruction_index,
                                &arguments,
                            )?
                        } else {
                            self.read_apply_arguments(frame, &arguments)?
                        };
                        if native {
                            self.refresh_current_frame_roots(frame);
                        }
                        let requested_outputs = self.statement_field_output_count(
                            &object,
                            member,
                            frame.access_context,
                        );
                        stack
                            .run(|stack| {
                                self.apply_field(
                                    stack,
                                    module,
                                    &object,
                                    member,
                                    arguments,
                                    requested_outputs,
                                    frame.access_context,
                                    instruction.location,
                                )
                            })
                            .await?
                    };
                    self.finish_statement_result(
                        function,
                        frame,
                        result,
                        display,
                        returned.into_iter().next(),
                        instruction.location,
                    )?;
                }
                InstructionKind::StatementPack {
                    pack,
                    result,
                    display,
                } => {
                    let values = self.read_pack_register(frame, pack)?.to_vec();
                    for value in values {
                        self.finish_statement_result(
                            function,
                            frame,
                            result,
                            display,
                            Some(value),
                            instruction.location,
                        )?;
                    }
                }
                InstructionKind::StatementValue {
                    src,
                    result,
                    display,
                } => {
                    let value = self.read_register(frame, src)?.clone();
                    if !matches!(value, Value::Nothing) {
                        self.finish_statement_result(
                            function,
                            frame,
                            result,
                            display,
                            Some(value),
                            instruction.location,
                        )?;
                    }
                }
                InstructionKind::Display { name, src } => {
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("display name escaped validation"))?;
                    let value = self.read_register(frame, src)?.clone();
                    self.emit_named_display(name, value, instruction.location)?;
                }
                InstructionKind::ReturnApply { target, arguments } => {
                    let target = self.read_register(frame, target)?.clone();
                    let arguments = if matches!(target, Value::Function(_)) {
                        self.take_apply_arguments(frame, function, instruction_index, &arguments)?
                    } else {
                        self.read_apply_arguments(frame, &arguments)?
                    };
                    self.refresh_current_frame_roots(frame);
                    self.prepare_builtin_scope(module, function, frame)?;
                    let returned = stack
                        .run(|stack| {
                            self.apply_value_owned(
                                stack,
                                module,
                                &target,
                                arguments,
                                usize::try_from(frame.requested_output_count).unwrap_or(usize::MAX),
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await;
                    self.apply_pending_scope_bindings(module, function, frame)?;
                    let returned = returned?;
                    return Ok(returned);
                }
                InstructionKind::ReturnApplyField {
                    object,
                    name,
                    arguments,
                } => {
                    let object = self.read_register(frame, object)?.clone();
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("member name escaped validation"))?;
                    let native = self.is_native_object(&object);
                    let arguments = if native {
                        self.take_apply_arguments(frame, function, instruction_index, &arguments)?
                    } else {
                        self.read_apply_arguments(frame, &arguments)?
                    };
                    if native {
                        self.refresh_current_frame_roots(frame);
                    }
                    return stack
                        .run(|stack| {
                            self.apply_field(
                                stack,
                                module,
                                &object,
                                name,
                                arguments,
                                usize::try_from(frame.requested_output_count).unwrap_or(usize::MAX),
                                frame.access_context,
                                instruction.location,
                            )
                        })
                        .await;
                }
                InstructionKind::IndexAssign {
                    dst,
                    target,
                    arguments,
                    value,
                } => {
                    let target = self.read_register(frame, target)?.clone();
                    let arguments = self.read_apply_arguments(frame, &arguments)?;
                    let value = self.read_register(frame, value)?.clone();
                    let assigned =
                        self.index_assign_value(&target, &arguments, &value, instruction.location)?;
                    self.write_register(frame, dst, assigned)?;
                }
                InstructionKind::ForEach {
                    iterable,
                    index,
                    dst,
                    exit,
                } => {
                    let iterable = self.read_register(frame, iterable)?.clone();
                    let index_value = self
                        .read_register(frame, index)?
                        .as_real_number()
                        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
                        .ok_or_else(|| self.invalid_state("for index register was corrupted"))?;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let iteration = index_value as u64;
                    let value = array_ops::for_iteration(&iterable, iteration, &self.cancellation)
                        .map_err(|kind| self.error(kind, instruction.location))?;
                    if let Some(value) = value {
                        self.write_register(frame, dst, value)?;
                        self.write_register(frame, index, Value::Double(index_value + 1.0))?;
                    } else {
                        frame.instruction = exit;
                    }
                }
                InstructionKind::DeclareGlobal { name } => {
                    let name = global_name(function, name)
                        .ok_or_else(|| self.invalid_state("global name escaped validation"))?
                        .to_owned();
                    if !self.workspace.contains(&name) {
                        self.workspace.insert(name, Value::empty_double());
                    }
                }
                InstructionKind::DeclarePersistent { slot } => {
                    let key = persistent_binding_key(function, frame, slot);
                    self.persistent_bindings
                        .entry(key)
                        .or_insert_with(Value::empty_double);
                }
                InstructionKind::LoadPersistent { dst, slot } => {
                    let key = persistent_binding_key(function, frame, slot);
                    let value = self
                        .persistent_bindings
                        .entry(key)
                        .or_insert_with(Value::empty_double)
                        .clone();
                    self.write_register(frame, dst, value)?;
                }
                InstructionKind::StorePersistent { slot, src } => {
                    let source = self.read_register(frame, src)?.clone();
                    let value = self.language_copy(&source)?;
                    let key = persistent_binding_key(function, frame, slot);
                    if let Some(previous) = self.persistent_bindings.insert(key, value) {
                        self.note_root_removal(&previous);
                    }
                }
                InstructionKind::Return { values } => {
                    return values
                        .iter()
                        .map(|register| self.read_register(frame, *register).cloned())
                        .collect();
                }
                InstructionKind::ReturnVariadic { fixed, variadic } => {
                    let mut returned = fixed
                        .iter()
                        .map(|register| self.read_register(frame, *register).cloned())
                        .collect::<RuntimeResult<Vec<_>>>()?;
                    let requested =
                        usize::try_from(frame.requested_output_count).map_err(|_| {
                            self.error(
                                array_error(ArrayRuntimeError::SizeLimit),
                                instruction.location,
                            )
                        })?;
                    let extra = requested.saturating_sub(returned.len());
                    if extra > 0 {
                        let value = self.read_register(frame, variadic)?;
                        let Value::Cell(values) = value else {
                            return Err(self.aggregate_type_error(
                                "varargout return",
                                "cell",
                                value,
                                instruction.location,
                            ));
                        };
                        returned.extend(values.values().iter().take(extra).cloned());
                    }
                    return Ok(returned);
                }
            }
        }
    }

    fn resolve_binding_target(
        &self,
        function: &Function,
        frame: &ExecutionFrame,
        target: BindingTarget,
    ) -> RuntimeResult<RuntimeBindingTarget> {
        match target {
            BindingTarget::Local(local) => Ok(RuntimeBindingTarget::Local(local)),
            BindingTarget::Persistent(slot) => Ok(RuntimeBindingTarget::Persistent(
                persistent_binding_key(function, frame, slot),
            )),
            BindingTarget::Capture(name) => {
                let name = global_name(function, name)
                    .ok_or_else(|| self.invalid_state("capture name escaped validation"))?;
                let Some(CapturedBinding::Shared(shared)) = frame.captures.get(name) else {
                    return Err(self.invalid_state("shared capture is missing"));
                };
                Ok(RuntimeBindingTarget::Shared(shared.clone()))
            }
            BindingTarget::Workspace(name) => global_name(function, name)
                .map(str::to_owned)
                .map(RuntimeBindingTarget::Workspace)
                .ok_or_else(|| self.invalid_state("global name escaped validation")),
        }
    }

    fn take_binding(
        &mut self,
        frame: &mut ExecutionFrame,
        target: &RuntimeBindingTarget,
    ) -> RuntimeResult<Option<Value>> {
        match target {
            RuntimeBindingTarget::Local(local) => {
                if let Some(shared) = frame.shared_locals.get(local) {
                    return shared
                        .replace(Value::Nothing)
                        .map(Some)
                        .ok_or_else(|| self.invalid_state("shared local lock is poisoned"));
                }
                get_mut(&mut frame.locals, local.get())
                    .map(|value| Some(std::mem::replace(value, Value::Nothing)))
                    .ok_or_else(|| self.invalid_state("local index escaped validation"))
            }
            RuntimeBindingTarget::Persistent(key) => Ok(self.persistent_bindings.remove(key)),
            RuntimeBindingTarget::Shared(shared) => shared
                .replace(Value::Nothing)
                .map(Some)
                .ok_or_else(|| self.invalid_state("shared capture lock is poisoned")),
            RuntimeBindingTarget::Workspace(name) => Ok(self.workspace.remove(name)),
        }
    }

    fn restore_binding(
        &mut self,
        frame: &mut ExecutionFrame,
        target: &RuntimeBindingTarget,
        previous: Option<Value>,
    ) -> RuntimeResult<()> {
        if let Some(previous) = previous {
            return self.commit_binding(frame, target, previous);
        }
        match target {
            RuntimeBindingTarget::Persistent(key) => {
                self.persistent_bindings.remove(key);
                Ok(())
            }
            RuntimeBindingTarget::Workspace(name) => {
                self.workspace.remove(name);
                Ok(())
            }
            RuntimeBindingTarget::Local(_) | RuntimeBindingTarget::Shared(_) => {
                Err(self.invalid_state("owned frame binding unexpectedly disappeared"))
            }
        }
    }

    fn commit_binding(
        &mut self,
        frame: &mut ExecutionFrame,
        target: &RuntimeBindingTarget,
        value: Value,
    ) -> RuntimeResult<()> {
        match target {
            RuntimeBindingTarget::Local(local) => {
                if let Some(shared) = frame.shared_locals.get(local) {
                    let previous = shared
                        .replace(value)
                        .ok_or_else(|| self.invalid_state("shared local lock is poisoned"))?;
                    self.note_root_removal(&previous);
                    return Ok(());
                }
                let Some(target) = get_mut(&mut frame.locals, local.get()) else {
                    return Err(self.invalid_state("local index escaped validation"));
                };
                let previous = std::mem::replace(target, value);
                self.note_root_removal(&previous);
                Ok(())
            }
            RuntimeBindingTarget::Persistent(key) => {
                if let Some(previous) = self.persistent_bindings.insert(key.clone(), value) {
                    self.note_root_removal(&previous);
                }
                Ok(())
            }
            RuntimeBindingTarget::Shared(shared) => {
                let previous = shared
                    .replace(value)
                    .ok_or_else(|| self.invalid_state("shared capture lock is poisoned"))?;
                self.note_root_removal(&previous);
                Ok(())
            }
            RuntimeBindingTarget::Workspace(name) => {
                self.replace_workspace_binding(name.clone(), value);
                Ok(())
            }
        }
    }

    fn unwind_frame_error(
        &mut self,
        function: &Function,
        frame: &mut ExecutionFrame,
        failed_instruction: InstructionIndex,
        error: RuntimeError,
    ) -> RuntimeResult<()> {
        self.record_last_error(&error);
        let Some(identifier) = error.kind.catchable_identifier() else {
            return Err(error);
        };
        let Some(handler) = innermost_exception_handler(function, failed_instruction) else {
            return Err(error);
        };

        // Registers and packs are expression temporaries. Locals represent the
        // surrounding language scope and therefore survive the abandoned
        // protected computation.
        frame.registers.fill(Value::Nothing);
        for pack in &mut frame.pack_registers {
            pack.clear();
        }

        match handler.kind {
            ExceptionHandlerKind::Catch {
                handler,
                error_local,
            } => {
                if let Some(error_local) = error_local {
                    if get(&frame.locals, error_local.get()).is_none() {
                        return Err(Self::exception_binding_error(
                            &error,
                            "verified catch local is absent from its frame",
                        ));
                    }
                    let exception = self.exception_object(&error, identifier)?;
                    let target =
                        get_mut(&mut frame.locals, error_local.get()).ok_or_else(|| {
                            Self::exception_binding_error(
                                &error,
                                "verified catch local is absent from its frame",
                            )
                        })?;
                    *target = exception;
                } else {
                    self.pending_exception = None;
                }
                frame.instruction = handler;
            }
            ExceptionHandlerKind::Swallow => {
                self.pending_exception = None;
                frame.instruction = handler.exit;
            }
        }
        Ok(())
    }

    fn exception_object(&mut self, error: &RuntimeError, identifier: &str) -> RuntimeResult<Value> {
        if let Some(handle) = self.pending_exception.take()
            && self.exception_errors.get(&handle) == Some(error)
        {
            return Ok(Value::Object(handle));
        }
        let stack = self
            .exception_stack_value(error)
            .map_err(|message| Self::exception_binding_error(error, &message))?;
        let cause = Self::empty_exception_cause()
            .map_err(|message| Self::exception_binding_error(error, &message))?;
        let message = Self::language_error_message(error, identifier);
        let identifier = self
            .last_error_text_value(identifier, "exception identifier", error.location)
            .map_err(|failure| {
                Self::exception_binding_error(
                    error,
                    &format!("identifier creation failed: {failure}"),
                )
            })?;
        let message = self
            .last_error_text_value(&message, "exception message", error.location)
            .map_err(|failure| {
                Self::exception_binding_error(error, &format!("message creation failed: {failure}"))
            })?;
        let handle = self
            .allocate_exception([identifier, message, stack, cause])
            .map_err(|detail| Self::exception_binding_error(error, &detail))?;
        self.exception_errors.insert(handle, error.clone());
        Ok(Value::Object(handle))
    }

    fn exception_stack_value(&self, error: &RuntimeError) -> Result<Value, String> {
        self.stack_frames(error)
            .and_then(|frames| Self::stack_value(&frames))
    }

    fn language_error_message(error: &RuntimeError, identifier: &str) -> String {
        match &error.kind {
            RuntimeErrorKind::Builtin {
                identifier: Some(_),
                message,
                ..
            } => message.clone(),
            _ => format!("OpenMat runtime error ({identifier}): {error}"),
        }
    }

    fn record_last_error(&mut self, error: &RuntimeError) {
        let Some(identifier) = error.kind.catchable_identifier() else {
            return;
        };
        let stack = self.stack_frames(error).unwrap_or_default();
        self.last_error = Some(LastErrorState {
            message: Self::language_error_message(error, identifier),
            identifier: identifier.to_owned(),
            stack,
        });
    }

    fn stack_frames(&self, error: &RuntimeError) -> Result<Vec<LastErrorFrame>, String> {
        let mut stack = Vec::new();
        stack
            .try_reserve_exact(error.stack.len())
            .map_err(|_| "exception stack exceeds host capacity".to_owned())?;
        for frame in error.stack.iter().rev() {
            let Some(location) = frame.location else {
                continue;
            };
            let (file, line) = self.source_metadata.get(&location.source_id).map_or_else(
                || (String::new(), 0.0),
                |source| (source.name.clone(), source.line_number(location.start)),
            );
            stack.push(LastErrorFrame {
                file,
                name: frame.name.clone(),
                line,
            });
        }
        Ok(stack)
    }

    fn last_error_text_value(
        &self,
        text: &str,
        label: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if !text.is_empty() {
            return self.char_row_value(text, label, location);
        }
        let shape = Shape::new([0, 0])
            .map_err(|_| self.invalid_state("empty error text shape is invalid"))?;
        DenseArray::from_vec(shape, Vec::new())
            .map(ArrayData::Char)
            .map(Value::Array)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))
    }

    fn last_error_stack_value(
        &self,
        stack: &[LastErrorFrame],
        _location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        Self::stack_value(stack).map_err(|message| self.invalid_state(&message))
    }

    fn stack_value(stack: &[LastErrorFrame]) -> Result<Value, String> {
        let length = u64::try_from(stack.len())
            .map_err(|_| "exception stack exceeds language limits".to_owned())?;
        let shape =
            Shape::new([length, 1]).map_err(|_| "exception stack shape is invalid".to_owned())?;
        let mut files = Vec::with_capacity(stack.len());
        let mut names = Vec::with_capacity(stack.len());
        let mut lines = Vec::with_capacity(stack.len());
        for frame in stack {
            files.push(Self::exception_text_value(&frame.file)?);
            names.push(Self::exception_text_value(&frame.name)?);
            lines.push(Value::Double(frame.line));
        }
        StructArray::from_columns(
            shape,
            ["file", "name", "line"]
                .into_iter()
                .map(FieldName::new)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "exception stack schema is invalid".to_owned())?,
            vec![files, names, lines],
        )
        .map(Value::Struct)
        .map_err(|_| "exception stack value is invalid".to_owned())
    }

    fn exception_text_value(text: &str) -> Result<Value, String> {
        let values = text
            .encode_utf16()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        let dimensions = if values.is_empty() {
            [0, 0]
        } else {
            [1, u64::try_from(values.len()).unwrap_or(u64::MAX)]
        };
        let shape =
            Shape::new(dimensions).map_err(|_| "exception text shape is invalid".to_owned())?;
        DenseArray::from_vec(shape, values)
            .map(ArrayData::Char)
            .map(Value::Array)
            .map_err(|_| "exception text value is invalid".to_owned())
    }

    fn last_error_struct_value(
        &self,
        state: &LastErrorState,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let message = self.last_error_text_value(&state.message, "error message", location)?;
        let identifier =
            self.last_error_text_value(&state.identifier, "error identifier", location)?;
        let stack = self.last_error_stack_value(&state.stack, location)?;
        StructArray::from_columns(
            Shape::new([1, 1])
                .map_err(|_| self.invalid_state("last-error state shape is invalid"))?,
            ["message", "identifier", "stack"]
                .into_iter()
                .map(FieldName::new)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| self.invalid_state("last-error state schema is invalid"))?,
            vec![vec![message], vec![identifier], vec![stack]],
        )
        .map(Value::Struct)
        .map_err(|_| self.invalid_state("last-error state value is invalid"))
    }

    fn last_error_input_text(
        &self,
        operation: &'static str,
        role: &'static str,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<String> {
        let code_units = match value {
            Value::String(value) => value
                .as_scalar()
                .filter(|value| !value.is_missing())
                .map(|value| value.code_units().to_vec()),
            Value::Array(ArrayData::Char(array))
                if array.shape().dimensions() == [0, 0]
                    || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
            {
                Some(array.as_slice().iter().map(|value| value.get()).collect())
            }
            _ => None,
        }
        .ok_or_else(|| {
            self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Type,
                Some(match operation {
                    "lasterr" => "OpenMat:lasterr:InvalidText",
                    _ => "OpenMat:lasterror:InvalidState",
                }),
                format!("{role} must be a char row vector or string scalar"),
                location,
            )
        })?;
        String::from_utf16(&code_units).map_err(|_| {
            self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Type,
                Some(match operation {
                    "lasterr" => "OpenMat:lasterr:InvalidText",
                    _ => "OpenMat:lasterror:InvalidState",
                }),
                format!("{role} must contain valid UTF-16 text"),
                location,
            )
        })
    }

    fn last_error_state_from_value(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<LastErrorState> {
        let Value::Struct(state) = value else {
            return Err(self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::Type,
                Some("OpenMat:lasterror:InvalidState"),
                "lasterror state must be a scalar struct",
                location,
            ));
        };
        if state.shape().dimensions() != [1, 1] {
            return Err(self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::Domain,
                Some("OpenMat:lasterror:InvalidState"),
                "lasterror state must be a scalar struct",
                location,
            ));
        }
        let message = state
            .field_index("message")
            .and_then(|field| state.value_at(field, 0))
            .map(|value| self.last_error_input_text("lasterror", "message", value, location))
            .transpose()?
            .unwrap_or_default();
        let identifier = state
            .field_index("identifier")
            .and_then(|field| state.value_at(field, 0))
            .map(|value| self.last_error_input_text("lasterror", "identifier", value, location))
            .transpose()?
            .unwrap_or_default();
        let stack = state
            .field_index("stack")
            .and_then(|field| state.value_at(field, 0))
            .map(|value| self.last_error_stack_from_value(value, location))
            .transpose()?
            .unwrap_or_default();
        Ok(LastErrorState {
            message,
            identifier,
            stack,
        })
    }

    fn last_error_stack_from_value(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<LastErrorFrame>> {
        let Value::Struct(stack) = value else {
            return Err(self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::Type,
                Some("OpenMat:lasterror:InvalidState"),
                "lasterror stack must be a column struct array",
                location,
            ));
        };
        if stack.shape().ndims() != 2 || stack.shape().extent(1) != 1 {
            return Err(self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::Domain,
                Some("OpenMat:lasterror:InvalidState"),
                "lasterror stack must be a column struct array",
                location,
            ));
        }
        let length = usize::try_from(stack.shape().extent(0))
            .map_err(|_| self.invalid_state("last-error stack exceeds host limits"))?;
        if length == 0 {
            return Ok(Vec::new());
        }
        let file_field = stack.field_index("file").ok_or_else(|| {
            self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::Type,
                Some("OpenMat:lasterror:InvalidState"),
                "nonempty lasterror stack requires file, name, and line fields",
                location,
            )
        })?;
        let name_field = stack.field_index("name").ok_or_else(|| {
            self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::Type,
                Some("OpenMat:lasterror:InvalidState"),
                "nonempty lasterror stack requires file, name, and line fields",
                location,
            )
        })?;
        let line_field = stack.field_index("line").ok_or_else(|| {
            self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::Type,
                Some("OpenMat:lasterror:InvalidState"),
                "nonempty lasterror stack requires file, name, and line fields",
                location,
            )
        })?;
        let mut frames = Vec::with_capacity(length);
        for offset in 0..length {
            let file = self.last_error_input_text(
                "lasterror",
                "stack file",
                stack
                    .value_at(file_field, offset)
                    .ok_or_else(|| self.invalid_state("last-error file field is incomplete"))?,
                location,
            )?;
            let name = self.last_error_input_text(
                "lasterror",
                "stack name",
                stack
                    .value_at(name_field, offset)
                    .ok_or_else(|| self.invalid_state("last-error name field is incomplete"))?,
                location,
            )?;
            let line = match stack.value_at(line_field, offset) {
                Some(Value::Double(line)) => *line,
                _ => {
                    return Err(self.intrinsic_builtin_error(
                        "lasterror",
                        BuiltinErrorCategory::Type,
                        Some("OpenMat:lasterror:InvalidState"),
                        "stack line values must be real double scalars",
                        location,
                    ));
                }
            };
            frames.push(LastErrorFrame { file, name, line });
        }
        Ok(frames)
    }

    fn empty_exception_cause() -> Result<Value, String> {
        let shape =
            Shape::new([0, 0]).map_err(|_| "empty exception cause shape is invalid".to_owned())?;
        CellArray::from_values(shape, Vec::new())
            .map(Value::Cell)
            .map_err(|_| "empty exception cause value is invalid".to_owned())
    }

    fn allocate_exception(&mut self, values: [Value; 4]) -> Result<ObjectHandle, String> {
        let class_id = self
            .exception_class
            .ok_or_else(|| "MException class is not installed".to_owned())?;
        let reference = self
            .objects
            .allocate_with(&self.classes, class_id, |_| Ok(Value::Nothing))
            .map_err(|detail| format!("exception object allocation failed: {detail}"))?;
        let initialization = (|| -> Result<(), String> {
            self.objects
                .begin_construction(&reference)
                .map_err(|detail| detail.to_string())?;
            for (name, value) in EXCEPTION_PROPERTIES.into_iter().zip(values) {
                let resolution = self
                    .classes
                    .resolve_property(
                        class_id,
                        name,
                        PropertyAccess::Get,
                        AccessContext::external(),
                    )
                    .map_err(|detail| detail.to_string())?;
                let PropertyResolution::Stored { key, .. } = resolution else {
                    return Err(format!("MException property `{name}` is not stored"));
                };
                *self
                    .objects
                    .slot_mut(&reference, &key)
                    .map_err(|detail| detail.to_string())? = value;
            }
            self.objects
                .finish_construction(&reference)
                .map_err(|detail| detail.to_string())
        })();
        if let Err(detail) = initialization {
            let _ = self.objects.fail_construction(&reference);
            let _ = self.objects.discard_incomplete(&reference);
            return Err(format!("exception object initialization failed: {detail}"));
        }
        let handle = ObjectHandle::new(reference.id().get());
        self.object_references.insert(handle, reference);
        Ok(handle)
    }

    fn exception_binding_error(original: &RuntimeError, message: &str) -> RuntimeError {
        RuntimeError {
            kind: RuntimeErrorKind::InvalidExecutionState {
                message: message.to_owned(),
                array: None,
            },
            location: original.location,
            stack: original.stack.clone(),
        }
    }

    fn build_cell(
        &mut self,
        frame: &ExecutionFrame,
        rows: &[Vec<ValueSource>],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if rows.is_empty() {
            let shape = Shape::new([0, 0])
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            return CellArray::from_values(shape, Vec::new())
                .map(Value::Cell)
                .map_err(|error| self.aggregate_value_error("cell construction", error, location));
        }

        let mut expanded_rows = Vec::new();
        expanded_rows
            .try_reserve_exact(rows.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for row in rows {
            let mut expanded = Vec::new();
            expanded
                .try_reserve(row.len())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            for source in row {
                match source {
                    ValueSource::One(register) => {
                        expanded.push(self.read_register(frame, *register)?.clone());
                    }
                    ValueSource::Expand(register) => {
                        let pack = self.read_pack_register(frame, *register)?;
                        expanded.try_reserve(pack.len()).map_err(|_| {
                            self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                        })?;
                        expanded.extend_from_slice(pack);
                    }
                }
            }
            expanded_rows.push(expanded);
        }
        let columns = expanded_rows.first().map_or(0, Vec::len);
        for (row, values) in expanded_rows.iter().enumerate().skip(1) {
            if values.len() != columns {
                return Err(self.error(
                    array_error(ArrayRuntimeError::RaggedCellRows {
                        row,
                        expected: columns,
                        actual: values.len(),
                    }),
                    location,
                ));
            }
        }

        let row_count = u64::try_from(expanded_rows.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let column_count = u64::try_from(columns)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let total = expanded_rows
            .len()
            .checked_mul(columns)
            .ok_or_else(|| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let mut source_order = Vec::new();
        source_order
            .try_reserve_exact(total)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for row in &expanded_rows {
            source_order.extend_from_slice(row);
        }
        let mut column_major = Vec::new();
        column_major
            .try_reserve_exact(total)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let shape = Shape::new([row_count, column_count])
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let copied = self.language_copy_values(&source_order)?;
        for column in 0..columns {
            for row in 0..expanded_rows.len() {
                let source = row
                    .checked_mul(columns)
                    .and_then(|value| value.checked_add(column))
                    .ok_or_else(|| {
                        self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                    })?;
                column_major.push(copied.get(source).cloned().ok_or_else(|| {
                    self.invalid_state("cell construction ordering escaped validation")
                })?);
            }
        }
        CellArray::from_values(shape, column_major)
            .map(Value::Cell)
            .map_err(|error| self.aggregate_value_error("cell construction", error, location))
    }

    fn brace_apply(
        &mut self,
        target: &Value,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if let Value::Table(table) = target {
            return self.table_brace_apply(table, arguments, location);
        }
        let Value::Cell(cell) = target else {
            return Err(self.aggregate_type_error("cell brace indexing", "cell", target, location));
        };
        let selection = array_ops::resolve_index_selection(
            cell.shape().dimensions(),
            arguments,
            &self.cancellation,
        )
        .map_err(|kind| self.error(kind, location))?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(selection.offsets.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for offset in selection.offsets {
            raw.push(
                cell.value_at_offset(offset)
                    .cloned()
                    .ok_or_else(|| self.invalid_state("cell brace offset escaped validation"))?,
            );
        }
        self.language_copy_values(&raw)
    }

    async fn aggregate_field_pack(
        &mut self,
        stack: &mut FrameStack,
        target: &Value,
        field: &str,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        match target {
            Value::Table(table) => self
                .get_table_field(table, field, location)
                .map(|value| vec![value]),
            Value::Struct(structure) => {
                let field_name = self.validate_struct_field_name(field, location)?;
                let field = structure.field_index(field_name.as_str()).ok_or_else(|| {
                    self.error(
                        array_error(ArrayRuntimeError::MissingStructField {
                            name: field_name.to_string(),
                        }),
                        location,
                    )
                })?;
                let values = structure.field_values(field).ok_or_else(|| {
                    self.invalid_state("struct field index escaped validated schema")
                })?;
                self.language_copy_values(values)
            }
            Value::Object(_) | Value::ObjectArray(_) => stack
                .run(|stack| self.get_field_on_stack(stack, target, field, context, location))
                .await
                .map(|value| vec![value]),
            Value::Graphics(_) | Value::GraphicsArray(_) => {
                stack
                    .run(|stack| self.get_graphics_field(stack, target, field, location))
                    .await
            }
            Value::Function(FunctionHandle::Class(handle)) => stack
                .run(|stack| self.get_class_field(stack, *handle, field, context, location))
                .await
                .map(|value| vec![value]),
            Value::Function(FunctionHandle::Intrinsic(IntrinsicFunction::Ffi)) => stack
                .run(|stack| self.get_field_on_stack(stack, target, field, context, location))
                .await
                .map(|value| vec![value]),
            _ => Err(self.aggregate_type_error(
                "aggregate field read",
                "struct or object",
                target,
                location,
            )),
        }
    }

    fn get_table_field(
        &mut self,
        table: &TableArray,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if name == "Properties" {
            return self.table_properties_value(table, location);
        }
        let variable = table.variable_by_name(name).ok_or_else(|| {
            self.table_error(
                "table variable read",
                format!("variable `{name}` does not exist"),
                location,
            )
        })?;
        self.language_copy(variable)
    }

    fn table_properties_value(
        &self,
        table: &TableArray,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let mut names = Vec::new();
        names
            .try_reserve_exact(table.variable_count())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for name in table.variable_names() {
            let code_units = name
                .as_str()
                .encode_utf16()
                .map(CharCodeUnit::new)
                .collect::<Vec<_>>();
            let width = u64::try_from(code_units.len())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let shape = Shape::new([1, width])
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let value = DenseArray::from_vec(shape, code_units)
                .map(ArrayData::Char)
                .map(Value::Array)
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            names.push(value);
        }
        let width = u64::try_from(names.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let variable_names = CellArray::from_values(
            Shape::new([1, width])
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
            names,
        )
        .map(Value::Cell)
        .map_err(|error| {
            self.aggregate_value_error("table Properties.VariableNames", error, location)
        })?;
        let row_names = if let Some(names) = table.row_names() {
            let mut values = Vec::with_capacity(names.len());
            for name in names {
                values.push(self.table_name_value(name.as_str(), location)?);
            }
            CellArray::from_values(
                Shape::new([names.len() as u64, 1])
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
                values,
            )
        } else {
            CellArray::from_values(
                Shape::new([0, 0])
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
                Vec::new(),
            )
        }
        .map(Value::Cell)
        .map_err(|error| {
            self.aggregate_value_error("table Properties.RowNames", error, location)
        })?;
        let variable_names_field = FieldName::new("VariableNames")
            .map_err(|_| self.invalid_state("static table Properties field is invalid"))?;
        let row_names_field = FieldName::new("RowNames")
            .map_err(|_| self.invalid_state("static table Properties field is invalid"))?;
        StructArray::from_columns(
            Shape::new([1, 1])
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
            vec![variable_names_field, row_names_field],
            vec![vec![variable_names], vec![row_names]],
        )
        .map(Value::Struct)
        .map_err(|error| self.aggregate_value_error("table Properties", error, location))
    }

    fn table_name_value(
        &self,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let code_units = name
            .encode_utf16()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        let width = u64::try_from(code_units.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        DenseArray::from_vec(
            Shape::new([1, width])
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
            code_units,
        )
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))
    }

    fn table_parenthesis_index(
        &mut self,
        table: &TableArray,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let (rows, variables) =
            self.table_subscripts(arguments, "table parenthesis indexing", location)?;
        let row_offsets = self.table_row_offsets(table, rows, location)?;
        let normalized_rows = self.table_row_index_input(&row_offsets, location)?;
        let variable_offsets = self.table_variable_offsets(table, variables, location)?;
        let row_count = u64::try_from(row_offsets.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let mut names = Vec::new();
        let mut values = Vec::new();
        names
            .try_reserve_exact(variable_offsets.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        values
            .try_reserve_exact(variable_offsets.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for (progress, offset) in variable_offsets.into_iter().enumerate() {
            self.check_loop_cancelled(progress, location)?;
            let name = table.variable_names().get(offset).cloned().ok_or_else(|| {
                self.invalid_state("table variable selection escaped schema validation")
            })?;
            let variable = table.variable(offset).ok_or_else(|| {
                self.invalid_state("table variable selection escaped storage validation")
            })?;
            names.push(name);
            values.push(self.index_table_variable_rows(variable, &normalized_rows, location)?);
        }
        let row_names = table.row_names().map(|names| {
            row_offsets
                .iter()
                .filter_map(|offset| names.get(*offset).cloned())
                .collect::<Vec<_>>()
        });
        TableArray::from_parts_with_row_names(row_count, names, values, row_names)
            .map(Value::Table)
            .map_err(|error| {
                self.table_error("table parenthesis indexing", error.to_string(), location)
            })
    }

    fn table_brace_apply(
        &mut self,
        table: &TableArray,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let (rows, variables) =
            self.table_subscripts(arguments, "table brace indexing", location)?;
        let row_offsets = self.table_row_offsets(table, rows, location)?;
        let normalized_rows = self.table_row_index_input(&row_offsets, location)?;
        let variable_offsets = self.table_variable_offsets(table, variables, location)?;
        if variable_offsets.is_empty() {
            let row_count = u64::try_from(row_offsets.len())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let empty = DenseArray::from_vec(
                Shape::new([row_count, 0])
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
                Vec::new(),
            )
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            return Ok(vec![empty]);
        }

        let mut selected = Vec::new();
        selected
            .try_reserve_exact(variable_offsets.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for (progress, offset) in variable_offsets.into_iter().enumerate() {
            self.check_loop_cancelled(progress, location)?;
            let variable = table.variable(offset).ok_or_else(|| {
                self.invalid_state("table brace selection escaped storage validation")
            })?;
            selected.push(self.index_table_variable_rows(variable, &normalized_rows, location)?);
        }
        if selected.len() == 1 {
            return self.language_copy(&selected[0]).map(|value| vec![value]);
        }
        let concatenated = array_ops::build_matrix(&[selected.clone()], &self.cancellation)
            .map_err(|kind| {
                let mut error = self.error(kind, location);
                if let RuntimeErrorKind::InvalidExecutionState { message, .. } = &mut error.kind {
                    "table brace indexing cannot horizontally concatenate selected variables"
                        .clone_into(message);
                }
                error
            })?;
        let result_class = concatenated.class_name();
        let lossless = selected.iter().all(|value| {
            value.class_name() == result_class
                || value.numel() == Some(0)
                || (value.class_name() == "logical" && result_class == "double")
        });
        if !lossless {
            let classes = selected
                .iter()
                .map(Value::class_name)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(self.table_error(
                "table brace indexing",
                format!(
                    "selected variable classes [{classes}] require a lossy conversion to `{result_class}`"
                ),
                location,
            ));
        }
        Ok(vec![concatenated])
    }

    fn table_subscripts<'a>(
        &self,
        arguments: &'a [IndexInput],
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(&'a IndexInput, &'a IndexInput)> {
        let [rows, variables] = arguments else {
            return Err(self.table_error(
                operation,
                format!(
                    "exactly two subscripts are required, received {}",
                    arguments.len()
                ),
                location,
            ));
        };
        Ok((rows, variables))
    }

    fn table_row_index_input(
        &self,
        offsets: &[usize],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<IndexInput> {
        let values = offsets
            .iter()
            .map(|offset| {
                offset
                    .checked_add(1)
                    .and_then(|value| u64::try_from(value).ok())
                    .ok_or_else(|| self.error(array_error(ArrayRuntimeError::SizeLimit), location))
            })
            .collect::<RuntimeResult<Vec<_>>>()?;
        DenseArray::from_vec(
            Shape::new([values.len() as u64, 1])
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
            values,
        )
        .map(IntegerArrayData::U64)
        .map(ArrayData::Integer)
        .map(Value::Array)
        .map(IndexInput::Value)
        .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))
    }

    #[allow(clippy::too_many_lines)]
    fn assign_table_parentheses(
        &mut self,
        table: &TableArray,
        arguments: &[IndexInput],
        supplied: Option<&Value>,
        mode: AssignmentMode,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let (rows, variables) =
            self.table_subscripts(arguments, "table parenthesized assignment", location)?;
        let variable_offsets = self.table_variable_offsets(table, variables, location)?;
        if mode == AssignmentMode::Delete {
            if matches!(rows, IndexInput::Colon) && variable_offsets.len() != table.variable_count()
            {
                let mut assigned = table.clone();
                for &offset in variable_offsets.iter().rev() {
                    assigned.remove_variable(offset).map_err(|error| {
                        self.table_error("table variable deletion", error.to_string(), location)
                    })?;
                }
                return Ok(Value::Table(assigned));
            }
            if variable_offsets != (0..table.variable_count()).collect::<Vec<_>>() {
                return Err(self.table_error(
                    "table row deletion",
                    "row deletion requires selecting every table variable",
                    location,
                ));
            }
            let deleted = self.table_row_offsets(table, rows, location)?;
            let deleted = deleted.into_iter().collect::<BTreeSet<_>>();
            let kept = (0..usize::try_from(table.row_count())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?)
                .filter(|offset| !deleted.contains(offset))
                .collect::<Vec<_>>();
            let keep_rows = self.table_row_index_input(&kept, location)?;
            let mut values = Vec::with_capacity(table.variable_count());
            for index in 0..table.variable_count() {
                let variable = table.variable(index).ok_or_else(|| {
                    self.invalid_state("table row deletion escaped storage validation")
                })?;
                values.push(self.index_table_variable_rows(variable, &keep_rows, location)?);
            }
            let names = table.variable_names().to_vec();
            let row_names = table.row_names().map(|names| {
                kept.iter()
                    .filter_map(|offset| names.get(*offset).cloned())
                    .collect::<Vec<_>>()
            });
            return TableArray::from_parts_with_row_names(
                kept.len() as u64,
                names,
                values,
                row_names,
            )
            .map(Value::Table)
            .map_err(|error| self.table_error("table row deletion", error.to_string(), location));
        }

        let supplied = supplied.ok_or_else(|| {
            self.invalid_state("table assignment source disappeared after validation")
        })?;
        let one_based_row = if matches!(rows, IndexInput::Colon) {
            return Err(self.table_error(
                "table row assignment",
                "the current implementation requires one positive integer row index",
                location,
            ));
        } else {
            array_ops::scalar_linear_index(std::slice::from_ref(rows), &self.cancellation).map_err(
                |_| {
                    self.table_error(
                        "table row assignment",
                        "the current implementation requires one positive integer row index",
                        location,
                    )
                },
            )?
        };
        if one_based_row > table.row_count().saturating_add(1) {
            return Err(self.table_error(
                "table row assignment",
                "row growth may append only the next table row",
                location,
            ));
        }
        let row = usize::try_from(one_based_row - 1)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let source_values = match supplied {
            Value::Cell(cell)
                if cell.shape().dimensions() == [1, variable_offsets.len() as u64] =>
            {
                cell.values().to_vec()
            }
            Value::Table(source)
                if source.row_count() == 1 && source.variable_count() == variable_offsets.len() =>
            {
                (0..source.variable_count())
                    .map(|index| {
                        source.variable(index).cloned().ok_or_else(|| {
                            self.invalid_state("table assignment source storage is incomplete")
                        })
                    })
                    .collect::<RuntimeResult<Vec<_>>>()?
            }
            _ => {
                return Err(self.table_error(
                    "table row assignment",
                    "the right side must be a one-row cell or table with one value per selected variable",
                    location,
                ));
            }
        };
        let append = row == usize::try_from(table.row_count()).unwrap_or(usize::MAX);
        if append && variable_offsets != (0..table.variable_count()).collect::<Vec<_>>() {
            return Err(self.table_error(
                "table row assignment",
                "row growth requires selecting every table variable",
                location,
            ));
        }
        if append && table.row_names().is_some() {
            return Err(self.table_error(
                "table row assignment",
                "appending to a table with row names requires an explicit new row name",
                location,
            ));
        }
        let mut values = (0..table.variable_count())
            .map(|index| {
                table.variable(index).cloned().ok_or_else(|| {
                    self.invalid_state("table assignment escaped storage validation")
                })
            })
            .collect::<RuntimeResult<Vec<_>>>()?;
        for (&variable, source) in variable_offsets.iter().zip(&source_values) {
            let target = values.get(variable).cloned().ok_or_else(|| {
                self.invalid_state("table assignment variable escaped validation")
            })?;
            values[variable] = if append {
                self.append_table_variable_row(&target, source, location)?
            } else {
                self.replace_table_variable_row(&target, row, source, location)?
            };
        }
        let row_count = table.row_count() + u64::from(append);
        TableArray::from_parts_with_row_names(
            row_count,
            table.variable_names().to_vec(),
            values,
            table.row_names().map(<[TableRowName]>::to_vec),
        )
        .map(Value::Table)
        .map_err(|error| self.table_error("table row assignment", error.to_string(), location))
    }

    fn replace_table_variable_row(
        &mut self,
        target: &Value,
        row: usize,
        source: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if let Value::Cell(cell) = target {
            let rows = usize::try_from(cell.shape().extent(0))
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let columns = usize::try_from(cell.shape().extent(1))
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let source_values = if columns == 1 {
                vec![source.clone()]
            } else if let Value::Cell(source) = source
                && source.shape().dimensions() == [1, columns as u64]
            {
                source.values().to_vec()
            } else {
                return Err(self.table_error(
                    "table row assignment",
                    "a multi-column cell variable requires a matching one-row cell value",
                    location,
                ));
            };
            let mut assigned = cell.clone();
            for (column, value) in source_values.into_iter().enumerate() {
                let offset = column
                    .checked_mul(rows)
                    .and_then(|offset| offset.checked_add(row))
                    .ok_or_else(|| {
                        self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                    })?;
                assigned.replace_at_offset(offset, value).map_err(|error| {
                    self.aggregate_value_error("table cell row assignment", error, location)
                })?;
            }
            return Ok(Value::Cell(assigned));
        }
        let dimensions = target.dimensions().ok_or_else(|| {
            self.table_error(
                "table row assignment",
                format!(
                    "a variable of class `{}` has no indexable shape",
                    target.class_name()
                ),
                location,
            )
        })?;
        let row = row
            .checked_add(1)
            .and_then(|row| u64::try_from(row).ok())
            .ok_or_else(|| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let row = DenseArray::from_vec(
            Shape::new([1, 1])
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
            vec![row],
        )
        .map(IntegerArrayData::U64)
        .map(ArrayData::Integer)
        .map(Value::Array)
        .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let mut arguments = vec![IndexInput::Value(row)];
        arguments.resize(dimensions.len(), IndexInput::Colon);
        self.index_assign_value(target, &arguments, source, location)
    }

    fn append_table_variable_row(
        &mut self,
        target: &Value,
        source: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if let Value::Cell(cell) = target {
            let rows = usize::try_from(cell.shape().extent(0))
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let columns = usize::try_from(cell.shape().extent(1))
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let source_values = if columns == 1 {
                vec![source.clone()]
            } else if let Value::Cell(source) = source
                && source.shape().dimensions() == [1, columns as u64]
            {
                source.values().to_vec()
            } else {
                return Err(self.table_error(
                    "table row growth",
                    "a multi-column cell variable requires a matching one-row cell value",
                    location,
                ));
            };
            let mut values = Vec::with_capacity((rows + 1).saturating_mul(columns));
            for (column, appended) in source_values.into_iter().enumerate() {
                let start = column.saturating_mul(rows);
                let end = start.saturating_add(rows);
                values.extend_from_slice(cell.values().get(start..end).ok_or_else(|| {
                    self.invalid_state("table cell row growth escaped storage validation")
                })?);
                values.push(appended);
            }
            return CellArray::from_values(
                Shape::new([(rows + 1) as u64, columns as u64])
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
                values,
            )
            .map(Value::Cell)
            .map_err(|error| self.aggregate_value_error("table cell row growth", error, location));
        }
        array_ops::build_matrix(
            &[vec![target.clone()], vec![source.clone()]],
            &self.cancellation,
        )
        .map_err(|kind| self.error(kind, location))
    }

    fn table_row_offsets(
        &self,
        table: &TableArray,
        rows: &IndexInput,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<usize>> {
        let names = match rows {
            IndexInput::Value(Value::Array(ArrayData::Char(array))) => {
                Some(vec![self.table_name_from_char(
                    array,
                    "table row selection",
                    location,
                )?])
            }
            IndexInput::Value(Value::String(strings)) => {
                let mut names = Vec::new();
                for offset in 0..usize::try_from(strings.numel())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?
                {
                    let element = strings.element(offset).ok_or_else(|| {
                        self.invalid_state("string table row selector escaped validation")
                    })?;
                    if element.is_missing() {
                        return Err(self.table_error(
                            "table row selection",
                            "missing strings cannot select table rows",
                            location,
                        ));
                    }
                    names.push(String::from_utf16(element.code_units()).map_err(|_| {
                        self.table_error(
                            "table row selection",
                            "a row selector contains invalid UTF-16",
                            location,
                        )
                    })?);
                }
                Some(names)
            }
            IndexInput::Value(Value::Cell(cell)) => {
                let mut names = Vec::with_capacity(cell.values().len());
                for value in cell.values() {
                    let Value::Array(ArrayData::Char(array)) = value else {
                        return Err(self.table_error(
                            "table row selection",
                            "a row-name selector must contain only char row vectors",
                            location,
                        ));
                    };
                    names.push(self.table_name_from_char(
                        array,
                        "table row selection",
                        location,
                    )?);
                }
                Some(names)
            }
            _ => None,
        };
        if let Some(names) = names {
            if table.row_names().is_none() {
                return Err(self.table_error(
                    "table row selection",
                    "the table has no row names",
                    location,
                ));
            }
            return names
                .into_iter()
                .map(|name| {
                    table.row_name_index(&name).ok_or_else(|| {
                        self.table_error(
                            "table row selection",
                            format!("row name `{name}` does not exist"),
                            location,
                        )
                    })
                })
                .collect();
        }
        if let IndexInput::Value(value) = rows
            && !is_table_numeric_index(value)
        {
            return Err(self.error(
                array_error(ArrayRuntimeError::InvalidIndex {
                    argument: 0,
                    reason: IndexErrorKind::NonNumeric,
                }),
                location,
            ));
        }
        array_ops::resolve_index_selection(
            &[table.row_count(), 1],
            std::slice::from_ref(rows),
            &self.cancellation,
        )
        .map(|selection| selection.offsets)
        .map_err(|kind| self.error(kind, location))
    }

    fn table_variable_offsets(
        &self,
        table: &TableArray,
        variables: &IndexInput,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<usize>> {
        match variables {
            IndexInput::Colon => Ok((0..table.variable_count()).collect()),
            IndexInput::Value(Value::Array(ArrayData::Char(array))) => {
                let name =
                    self.table_name_from_char(array, "table variable selection", location)?;
                self.table_variable_name_offsets(table, [name], location)
            }
            IndexInput::Value(Value::String(strings)) => {
                let count = usize::try_from(strings.numel())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                let mut names = Vec::new();
                names
                    .try_reserve_exact(count)
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                for offset in 0..count {
                    let element = strings.element(offset).ok_or_else(|| {
                        self.invalid_state("string table selector escaped storage validation")
                    })?;
                    if element.is_missing() {
                        return Err(self.table_error(
                            "table variable selection",
                            "missing strings cannot name table variables",
                            location,
                        ));
                    }
                    names.push(String::from_utf16(element.code_units()).map_err(|_| {
                        self.table_error(
                            "table variable selection",
                            "a string selector contains invalid UTF-16",
                            location,
                        )
                    })?);
                }
                self.table_variable_name_offsets(table, names, location)
            }
            IndexInput::Value(Value::Cell(cell)) => {
                let mut names = Vec::new();
                names
                    .try_reserve_exact(cell.values().len())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                for value in cell.values() {
                    let Value::Array(ArrayData::Char(array)) = value else {
                        return Err(self.table_error(
                            "table variable selection",
                            "a variable-name cell selector must contain only char row vectors",
                            location,
                        ));
                    };
                    names.push(self.table_name_from_char(
                        array,
                        "table variable selection",
                        location,
                    )?);
                }
                self.table_variable_name_offsets(table, names, location)
            }
            IndexInput::Value(value) if is_table_numeric_index(value) => {
                let width = table.shape().dimensions()[1];
                array_ops::resolve_index_selection(
                    &[width, 1],
                    std::slice::from_ref(variables),
                    &self.cancellation,
                )
                .map(|selection| selection.offsets)
                .map_err(|kind| self.table_index_error(kind, 1, location))
            }
            IndexInput::Value(_) => Err(self.error(
                array_error(ArrayRuntimeError::InvalidIndex {
                    argument: 1,
                    reason: IndexErrorKind::NonNumeric,
                }),
                location,
            )),
        }
    }

    fn table_variable_name_offsets(
        &self,
        table: &TableArray,
        names: impl IntoIterator<Item = String>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<usize>> {
        let names = names.into_iter();
        let mut offsets = Vec::new();
        offsets
            .try_reserve(names.size_hint().0)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for name in names {
            let offset = table.variable_index(&name).ok_or_else(|| {
                self.table_error(
                    "table variable selection",
                    format!("variable `{name}` does not exist"),
                    location,
                )
            })?;
            offsets.push(offset);
        }
        Ok(offsets)
    }

    fn table_name_from_char(
        &self,
        array: &DenseArray<CharCodeUnit>,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<String> {
        if array.shape().dimensions() != [0, 0]
            && (array.shape().ndims() != 2 || array.shape().extent(0) != 1)
        {
            return Err(self.table_error(
                operation,
                "a char variable selector must be a row vector",
                location,
            ));
        }
        let code_units = array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>();
        String::from_utf16(&code_units).map_err(|_| {
            self.table_error(
                operation,
                "a char variable selector contains invalid UTF-16",
                location,
            )
        })
    }

    fn index_table_variable_rows(
        &mut self,
        variable: &Value,
        rows: &IndexInput,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let dimensions = variable.dimensions().ok_or_else(|| {
            self.table_error(
                "table row indexing",
                format!(
                    "a variable of class `{}` has no indexable shape",
                    variable.class_name()
                ),
                location,
            )
        })?;
        let mut arguments = Vec::new();
        arguments
            .try_reserve_exact(dimensions.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        arguments.push(rows.clone());
        arguments.resize(dimensions.len(), IndexInput::Colon);
        match variable {
            Value::Cell(_) | Value::Struct(_) => {
                self.index_aggregate_value(variable, &arguments, false, location)
            }
            Value::Table(table) => {
                let nested = [rows.clone(), IndexInput::Colon];
                self.table_parenthesis_index(table, &nested, location)
            }
            Value::Object(_) | Value::ObjectArray(_) => {
                self.index_object_value(variable, &arguments, location)
            }
            Value::Graphics(_) | Value::GraphicsArray(_) => {
                self.index_table_graphics_variable(variable, &arguments, location)
            }
            _ => array_ops::index(variable, &arguments, &self.cancellation)
                .map_err(|kind| self.error(kind, location)),
        }
    }

    fn index_table_graphics_variable(
        &self,
        variable: &Value,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let dimensions = variable
            .dimensions()
            .ok_or_else(|| self.invalid_state("graphics table variable lost its shape"))?;
        let selection =
            array_ops::resolve_index_selection(dimensions, arguments, &self.cancellation)
                .map_err(|kind| self.error(kind, location))?;
        let (class, source): (_, &[openmat_graphics_model::GraphicsHandle]) = match variable {
            Value::Graphics(handle) => (handle.class(), std::slice::from_ref(handle)),
            Value::GraphicsArray(array) => (array.class(), array.as_slice()),
            _ => return Err(self.invalid_state("graphics table indexing received another class")),
        };
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(selection.offsets.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for offset in selection.offsets {
            handles.push(*source.get(offset).ok_or_else(|| {
                self.invalid_state("graphics table index escaped selection validation")
            })?);
        }
        if let [handle] = handles.as_slice() {
            return Ok(Value::Graphics(*handle));
        }
        GraphicsHandleArray::column(class, handles)
            .map(Value::GraphicsArray)
            .map_err(|error| self.table_error("table row indexing", error.to_string(), location))
    }

    fn table_index_error(
        &self,
        mut kind: RuntimeErrorKind,
        argument: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        if let RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } = &mut kind
        {
            match detail.as_mut() {
                ArrayRuntimeError::InvalidIndex {
                    argument: current, ..
                }
                | ArrayRuntimeError::IndexOutOfBounds {
                    argument: current, ..
                } => *current = argument,
                _ => {}
            }
        }
        self.error(kind, location)
    }

    fn table_error(
        &self,
        operation: &'static str,
        message: impl Into<String>,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        self.error(
            RuntimeErrorKind::Object {
                operation,
                message: message.into(),
            },
            location,
        )
    }

    fn resolve_end(
        &self,
        target: &Value,
        argument_index: u32,
        argument_count: u32,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let dimensions = target.dimensions().ok_or_else(|| {
            self.error(
                array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "end resolution",
                    actual: target.kind(),
                }),
                location,
            )
        })?;
        let count = usize::try_from(argument_count)
            .map_err(|_| self.invalid_state("end argument count exceeds host capacity"))?;
        let index = usize::try_from(argument_index)
            .map_err(|_| self.invalid_state("end argument index exceeds host capacity"))?;
        if count == 0 || index >= count {
            return Err(self.invalid_state("end metadata escaped bytecode validation"));
        }
        let shape = Shape::new(dimensions.iter().copied())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let effective = shape
            .effective_dimensions(count)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let extent = effective
            .get(index)
            .copied()
            .ok_or_else(|| self.invalid_state("end dimension escaped validation"))?;
        #[allow(clippy::cast_precision_loss)]
        Ok(Value::Double(extent as f64))
    }

    fn resolve_field_operand(
        &self,
        function: &Function,
        frame: &ExecutionFrame,
        field: FieldOperand,
    ) -> RuntimeResult<String> {
        match field {
            FieldOperand::Static(constant) => global_name(function, constant)
                .map(str::to_owned)
                .ok_or_else(|| self.invalid_state("field constant escaped validation")),
            FieldOperand::Dynamic(register) => {
                self.dynamic_field_name(self.read_register(frame, register)?)
            }
        }
    }

    fn resolve_place_path(
        &self,
        function: &Function,
        frame: &ExecutionFrame,
        path: &[PlaceStep],
    ) -> RuntimeResult<Vec<ResolvedPlaceStep>> {
        let mut resolved = Vec::new();
        resolved
            .try_reserve_exact(path.len())
            .map_err(|_| self.invalid_state("aggregate place path exceeds host capacity"))?;
        for step in path {
            resolved.push(match step {
                PlaceStep::Paren(arguments) => {
                    ResolvedPlaceStep::Paren(self.read_apply_arguments(frame, arguments)?)
                }
                PlaceStep::Brace(arguments) => {
                    ResolvedPlaceStep::Brace(self.read_apply_arguments(frame, arguments)?)
                }
                PlaceStep::Field(field) => {
                    ResolvedPlaceStep::Field(self.resolve_field_operand(function, frame, *field)?)
                }
            });
        }
        Ok(resolved)
    }

    fn read_assignment_source(
        &self,
        frame: &ExecutionFrame,
        source: ValueSource,
    ) -> RuntimeResult<AssignmentSource> {
        match source {
            ValueSource::One(register) => Ok(AssignmentSource {
                values: vec![self.read_register(frame, register)?.clone()],
                expanded: false,
            }),
            ValueSource::Expand(register) => {
                let pack = self.read_pack_register(frame, register)?;
                let mut values = Vec::new();
                values.try_reserve_exact(pack.len()).map_err(|_| {
                    self.invalid_state("assignment value pack exceeds host capacity")
                })?;
                values.extend_from_slice(pack);
                Ok(AssignmentSource {
                    values,
                    expanded: true,
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn assign_place(
        &mut self,
        stack: &mut FrameStack,
        root: &Value,
        path: &[ResolvedPlaceStep],
        source: &AssignmentSource,
        mode: AssignmentMode,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if path.is_empty() {
            return Err(self.invalid_state("empty aggregate place escaped bytecode validation"));
        }
        if mode == AssignmentMode::Delete {
            if source.expanded || source.values.len() != 1 {
                return Err(self.invalid_state("aggregate delete source escaped validation"));
            }
            if !source.values.first().is_some_and(is_empty_double) {
                let actual = source
                    .values
                    .first()
                    .map_or(openmat_value::ValueKind::Nothing, Value::kind);
                return Err(self.error(
                    array_error(ArrayRuntimeError::InvalidOperand {
                        operation: "aggregate deletion token",
                        actual,
                    }),
                    location,
                ));
            }
        }
        stack
            .run(|stack| self.assign_path(stack, root, path, source, mode, context, location))
            .await
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn assign_path(
        &mut self,
        stack: &mut FrameStack,
        target: &Value,
        path: &[ResolvedPlaceStep],
        source: &AssignmentSource,
        mode: AssignmentMode,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let (step, tail) = path
            .split_first()
            .ok_or_else(|| self.invalid_state("aggregate path recursion exhausted early"))?;
        if tail.is_empty() {
            return match step {
                ResolvedPlaceStep::Paren(arguments) => {
                    self.assign_final_paren(target, arguments, source, mode, location)
                }
                ResolvedPlaceStep::Brace(arguments) => {
                    self.assign_final_brace(target, arguments, source, mode, location)
                }
                ResolvedPlaceStep::Field(field) => {
                    stack
                        .run(|stack| {
                            self.assign_final_field(
                                stack, target, field, source, mode, context, location,
                            )
                        })
                        .await
                }
            };
        }

        match step {
            ResolvedPlaceStep::Paren(arguments) => {
                if matches!(target, Value::Table(_)) {
                    return Err(self.table_error(
                        "table content assignment",
                        "nested assignment through a parenthesized table selection is not implemented",
                        location,
                    ));
                }
                let assignment_target = if mode == AssignmentMode::Store {
                    self.grow_nested_paren_target(target, arguments, location)?
                } else {
                    target.clone()
                };
                let selected = if is_object_value(&assignment_target) {
                    self.index_object_value(&assignment_target, arguments, location)?
                } else {
                    self.index_aggregate_value(&assignment_target, arguments, false, location)?
                };
                let updated = stack
                    .run(|stack| {
                        self.assign_path(stack, &selected, tail, source, mode, context, location)
                    })
                    .await?;
                if is_object_value(&assignment_target) {
                    self.index_assign_object(&assignment_target, arguments, &updated, location)
                } else {
                    self.write_aggregate_selection_raw(
                        &assignment_target,
                        arguments,
                        &updated,
                        location,
                    )
                }
            }
            ResolvedPlaceStep::Brace(arguments) => {
                let Value::Cell(cell) = target else {
                    return Err(self.aggregate_type_error(
                        "nested cell brace assignment",
                        "cell",
                        target,
                        location,
                    ));
                };
                let selection = array_ops::resolve_index_selection(
                    cell.shape().dimensions(),
                    arguments,
                    &self.cancellation,
                )
                .map_err(|kind| self.error(kind, location))?;
                if selection.offsets.len() != 1 {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                            selected: u64::try_from(selection.offsets.len()).unwrap_or(u64::MAX),
                            supplied: 1,
                        }),
                        location,
                    ));
                }
                let offset = selection.offsets[0];
                let child = cell
                    .value_at_offset(offset)
                    .cloned()
                    .ok_or_else(|| self.invalid_state("nested cell offset escaped validation"))?;
                let updated = stack
                    .run(|stack| {
                        self.assign_path(stack, &child, tail, source, mode, context, location)
                    })
                    .await?;
                let mut assigned = cell.clone();
                assigned
                    .replace_at_offset(offset, updated)
                    .map_err(|error| {
                        self.aggregate_value_error("nested cell write-back", error, location)
                    })?;
                Ok(Value::Cell(assigned))
            }
            ResolvedPlaceStep::Field(field) => {
                stack
                    .run(|stack| {
                        self.assign_nested_field(
                            stack, target, field, tail, source, mode, context, location,
                        )
                    })
                    .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn assign_nested_field(
        &mut self,
        stack: &mut FrameStack,
        target: &Value,
        field: &str,
        tail: &[ResolvedPlaceStep],
        source: &AssignmentSource,
        mode: AssignmentMode,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        match target {
            Value::Table(table) if field == "Properties" => {
                self.assign_table_properties(table, tail, source, mode, location)
            }
            Value::Table(table) => {
                let index = table.variable_index(field).ok_or_else(|| {
                    self.table_error(
                        "table variable assignment",
                        format!("variable `{field}` does not exist"),
                        location,
                    )
                })?;
                let child = table.variable(index).cloned().ok_or_else(|| {
                    self.invalid_state("table variable assignment escaped storage validation")
                })?;
                let updated = stack
                    .run(|stack| {
                        self.assign_path(stack, &child, tail, source, mode, context, location)
                    })
                    .await?;
                let mut assigned = table.clone();
                assigned.replace_variable(index, updated).map_err(|error| {
                    self.table_error("table variable assignment", error.to_string(), location)
                })?;
                Ok(Value::Table(assigned))
            }
            Value::Struct(structure) => {
                if structure.numel() != 1 {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                            selected: structure.numel(),
                            supplied: 1,
                        }),
                        location,
                    ));
                }
                let field_name = self.validate_struct_field_name(field, location)?;
                let field_index = structure.field_index(field_name.as_str()).ok_or_else(|| {
                    self.error(
                        array_error(ArrayRuntimeError::MissingStructField {
                            name: field_name.to_string(),
                        }),
                        location,
                    )
                })?;
                let child = structure
                    .value_at(field_index, 0)
                    .cloned()
                    .ok_or_else(|| self.invalid_state("nested struct field escaped validation"))?;
                let updated = stack
                    .run(|stack| {
                        self.assign_path(stack, &child, tail, source, mode, context, location)
                    })
                    .await?;
                let mut assigned = structure.clone();
                assigned
                    .replace_at(field_index, 0, updated)
                    .map_err(|error| {
                        self.aggregate_value_error("nested struct write-back", error, location)
                    })?;
                Ok(Value::Struct(assigned))
            }
            Value::Object(_) | Value::ObjectArray(_) => {
                let child = stack
                    .run(|stack| self.get_field_on_stack(stack, target, field, context, location))
                    .await?;
                let updated = stack
                    .run(|stack| {
                        self.assign_path(stack, &child, tail, source, mode, context, location)
                    })
                    .await?;
                // Mutating a handle's child property does not replace the handle
                // stored in its owner (which may have private SetAccess).
                if let (Value::Object(before), Value::Object(after)) = (&child, &updated)
                    && before == after
                    && self
                        .object_reference(*before)
                        .is_ok_and(|reference| reference.semantics() == ClassSemantics::Handle)
                {
                    return Ok(target.clone());
                }
                stack
                    .run(|stack| {
                        self.set_field_on_stack(stack, target, field, &updated, context, location)
                    })
                    .await
            }
            _ => Err(self.aggregate_type_error(
                "nested aggregate field assignment",
                "struct or object",
                target,
                location,
            )),
        }
    }

    fn assign_final_paren(
        &mut self,
        target: &Value,
        arguments: &[IndexInput],
        source: &AssignmentSource,
        mode: AssignmentMode,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if mode == AssignmentMode::Delete {
            return match target {
                Value::Cell(cell) => self.delete_cell(cell, arguments, location),
                Value::Struct(structure) => self.delete_struct(structure, arguments, location),
                Value::Table(table) => {
                    self.assign_table_parentheses(table, arguments, None, mode, location)
                }
                _ => Err(self.aggregate_type_error(
                    "aggregate deletion",
                    "cell or struct",
                    target,
                    location,
                )),
            };
        }
        if source.expanded || source.values.len() != 1 {
            return Err(self.error(
                array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                    selected: 1,
                    supplied: u64::try_from(source.values.len()).unwrap_or(u64::MAX),
                }),
                location,
            ));
        }
        let supplied = source
            .values
            .first()
            .ok_or_else(|| self.invalid_state("single assignment source disappeared"))?;
        match target {
            Value::Table(table) => {
                self.assign_table_parentheses(table, arguments, Some(supplied), mode, location)
            }
            Value::Cell(cell) => self.assign_cell_paren(cell, arguments, supplied, location),
            Value::Struct(structure) => {
                self.assign_struct_paren(structure, arguments, supplied, location)
            }
            Value::Nothing => match supplied {
                Value::Cell(_) => {
                    let empty = CellArray::from_values(
                        Shape::new([0, 0]).map_err(|_| {
                            self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                        })?,
                        Vec::new(),
                    )
                    .map_err(|error| self.aggregate_value_error("cell growth", error, location))?;
                    self.assign_cell_paren(&empty, arguments, supplied, location)
                }
                Value::Struct(structure) => {
                    let empty = StructArray::empty(
                        Shape::new([0, 0]).map_err(|_| {
                            self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                        })?,
                        structure.field_names().to_vec(),
                    )
                    .map_err(|error| {
                        self.aggregate_value_error("struct growth", error, location)
                    })?;
                    self.assign_struct_paren(&empty, arguments, supplied, location)
                }
                _ => self.index_assign_value(target, arguments, supplied, location),
            },
            _ => self.index_assign_value(target, arguments, supplied, location),
        }
    }

    fn assign_final_brace(
        &mut self,
        target: &Value,
        arguments: &[IndexInput],
        source: &AssignmentSource,
        mode: AssignmentMode,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if mode == AssignmentMode::Delete {
            if matches!(target, Value::Table(_)) {
                return Err(self.table_error(
                    "table deletion",
                    "table content deletion is not implemented",
                    location,
                ));
            }
            return Err(self.aggregate_type_error(
                "cell brace deletion",
                "parenthesized cell selection",
                target,
                location,
            ));
        }
        if matches!(target, Value::Table(_)) {
            return Err(self.table_error(
                "table content assignment",
                "brace-indexed table content assignment is not implemented",
                location,
            ));
        }
        let owned_empty;
        let cell = match target {
            Value::Cell(cell) => cell,
            Value::Nothing => {
                owned_empty = CellArray::from_values(
                    Shape::new([0, 0]).map_err(|_| {
                        self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                    })?,
                    Vec::new(),
                )
                .map_err(|error| self.aggregate_value_error("cell growth", error, location))?;
                &owned_empty
            }
            _ => {
                return Err(self.aggregate_type_error(
                    "cell brace assignment",
                    "cell",
                    target,
                    location,
                ));
            }
        };
        let (selection, grown_shape) = self.selection_with_scalar_growth(
            cell.shape(),
            arguments,
            "cell brace assignment",
            location,
        )?;
        let selected = selection.offsets.len();
        if source.values.len() != selected || (!source.expanded && selected != 1) {
            return Err(self.error(
                array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                    selected: u64::try_from(selected).unwrap_or(u64::MAX),
                    supplied: u64::try_from(source.values.len()).unwrap_or(u64::MAX),
                }),
                location,
            ));
        }
        if selected == 0 {
            return Ok(Value::Cell(cell.clone()));
        }
        let mut values = self.grow_cell_values(cell, grown_shape.as_ref(), location)?;
        let copied = self.language_copy_values(&source.values)?;
        for (&offset, value) in selection.offsets.iter().zip(copied) {
            let slot = values
                .get_mut(offset)
                .ok_or_else(|| self.invalid_state("cell assignment offset escaped validation"))?;
            *slot = value;
        }
        let shape = grown_shape.unwrap_or_else(|| cell.shape().clone());
        CellArray::from_values(shape, values)
            .map(Value::Cell)
            .map_err(|error| self.aggregate_value_error("cell brace assignment", error, location))
    }

    #[allow(clippy::too_many_arguments)]
    async fn assign_final_field(
        &mut self,
        stack: &mut FrameStack,
        target: &Value,
        field: &str,
        source: &AssignmentSource,
        mode: AssignmentMode,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if mode == AssignmentMode::Delete {
            if let Value::Table(table) = target {
                return self.delete_table_field(table, field, location);
            }
            return Err(self.error(
                array_error(ArrayRuntimeError::InvalidDeletionShape {
                    dimensions: target.dimensions().map_or_else(Vec::new, <[u64]>::to_vec),
                    arguments: 0,
                }),
                location,
            ));
        }
        match target {
            Value::Table(table) => {
                if source.expanded || source.values.len() != 1 {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                            selected: 1,
                            supplied: u64::try_from(source.values.len()).unwrap_or(u64::MAX),
                        }),
                        location,
                    ));
                }
                let value = source
                    .values
                    .first()
                    .ok_or_else(|| self.invalid_state("table field source disappeared"))?;
                if is_empty_double(value) {
                    return self.delete_table_field(table, field, location);
                }
                self.assign_table_field(table, field, value, location)
            }
            Value::Struct(structure) => {
                self.assign_struct_field(structure, field, source, location)
            }
            Value::Nothing => {
                let structure = StructArray::empty(
                    Shape::new([1, 1]).map_err(|_| {
                        self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                    })?,
                    Vec::new(),
                )
                .map_err(|error| {
                    self.aggregate_value_error("struct field assignment", error, location)
                })?;
                self.assign_struct_field(&structure, field, source, location)
            }
            Value::Object(_) | Value::ObjectArray(_) => {
                if source.values.len() != 1 {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                            selected: 1,
                            supplied: u64::try_from(source.values.len()).unwrap_or(u64::MAX),
                        }),
                        location,
                    ));
                }
                let value = source
                    .values
                    .first()
                    .ok_or_else(|| self.invalid_state("object field source disappeared"))?;
                stack
                    .run(|stack| {
                        self.set_field_on_stack(stack, target, field, value, context, location)
                    })
                    .await
            }
            _ => Err(self.aggregate_type_error(
                "aggregate field assignment",
                "struct or object",
                target,
                location,
            )),
        }
    }

    fn assign_cell_paren(
        &mut self,
        cell: &CellArray,
        arguments: &[IndexInput],
        supplied: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let Value::Cell(supplied) = supplied else {
            return Err(self.aggregate_type_error(
                "cell parenthesized assignment",
                "cell",
                supplied,
                location,
            ));
        };
        let (selection, grown_shape) = self.selection_with_scalar_growth(
            cell.shape(),
            arguments,
            "cell parenthesized assignment",
            location,
        )?;
        self.check_scalar_expansion(supplied.numel(), selection.offsets.len(), location)?;
        if selection.offsets.is_empty() {
            return Ok(Value::Cell(cell.clone()));
        }

        let mut raw = Vec::new();
        raw.try_reserve_exact(selection.offsets.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for position in 0..selection.offsets.len() {
            let source = if supplied.numel() == 1 { 0 } else { position };
            raw.push(
                supplied.value_at_offset(source).cloned().ok_or_else(|| {
                    self.invalid_state("cell assignment source escaped validation")
                })?,
            );
        }
        let mut values = self.grow_cell_values(cell, grown_shape.as_ref(), location)?;
        let copied = self.language_copy_values(&raw)?;
        for (&offset, value) in selection.offsets.iter().zip(copied) {
            let slot = values
                .get_mut(offset)
                .ok_or_else(|| self.invalid_state("cell assignment offset escaped validation"))?;
            *slot = value;
        }
        let shape = grown_shape.unwrap_or_else(|| cell.shape().clone());
        CellArray::from_values(shape, values)
            .map(Value::Cell)
            .map_err(|error| {
                self.aggregate_value_error("cell parenthesized assignment", error, location)
            })
    }

    fn assign_struct_paren(
        &mut self,
        structure: &StructArray,
        arguments: &[IndexInput],
        supplied: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let Value::Struct(supplied) = supplied else {
            return Err(self.aggregate_type_error(
                "struct parenthesized assignment",
                "struct",
                supplied,
                location,
            ));
        };
        self.check_struct_schema(structure, supplied, location)?;
        let (selection, grown_shape) = self.selection_with_scalar_growth(
            structure.shape(),
            arguments,
            "struct parenthesized assignment",
            location,
        )?;
        self.check_scalar_expansion(supplied.numel(), selection.offsets.len(), location)?;
        if selection.offsets.is_empty() {
            return Ok(Value::Struct(structure.clone()));
        }

        let output_shape = grown_shape
            .clone()
            .unwrap_or_else(|| structure.shape().clone());
        let output_len = usize::try_from(output_shape.numel())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let mut columns = Vec::new();
        columns
            .try_reserve_exact(structure.field_count())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let mut source_columns = Vec::new();
        source_columns
            .try_reserve_exact(structure.field_count())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for (destination_field, name) in structure.field_names().iter().enumerate() {
            let source_field = supplied
                .field_index(name.as_str())
                .ok_or_else(|| self.invalid_state("equal struct schema lost a source field"))?;
            let source_column = supplied.field_values(source_field).ok_or_else(|| {
                self.invalid_state("struct assignment source column escaped validation")
            })?;
            let mut raw = Vec::new();
            raw.try_reserve_exact(selection.offsets.len())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            for position in 0..selection.offsets.len() {
                let source = if supplied.numel() == 1 { 0 } else { position };
                raw.push(source_column.get(source).cloned().ok_or_else(|| {
                    self.invalid_state("struct assignment source offset escaped validation")
                })?);
            }
            let original = structure.field_values(destination_field).ok_or_else(|| {
                self.invalid_state("struct assignment destination column escaped validation")
            })?;
            let column = self.grow_aggregate_column(
                structure.shape(),
                &output_shape,
                original,
                output_len,
                location,
            )?;
            source_columns.push(raw);
            columns.push(column);
        }
        self.language_copy_columns(&mut source_columns)?;
        for (column, copied) in columns.iter_mut().zip(source_columns) {
            for (&offset, value) in selection.offsets.iter().zip(copied) {
                let slot = column.get_mut(offset).ok_or_else(|| {
                    self.invalid_state("struct assignment offset escaped validation")
                })?;
                *slot = value;
            }
        }
        StructArray::from_columns(output_shape, structure.field_names().to_vec(), columns)
            .map(Value::Struct)
            .map_err(|error| {
                self.aggregate_value_error("struct parenthesized assignment", error, location)
            })
    }

    fn assign_table_properties(
        &self,
        table: &TableArray,
        path: &[ResolvedPlaceStep],
        source: &AssignmentSource,
        mode: AssignmentMode,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let [ResolvedPlaceStep::Field(property)] = path else {
            return Err(self.table_error(
                "table property assignment",
                "only complete Properties.VariableNames assignment is implemented",
                location,
            ));
        };
        if mode != AssignmentMode::Store {
            return Err(self.table_error(
                "table property assignment",
                format!("Properties.{property} cannot be deleted"),
                location,
            ));
        }
        if source.expanded || source.values.len() != 1 {
            return Err(self.error(
                array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                    selected: 1,
                    supplied: u64::try_from(source.values.len()).unwrap_or(u64::MAX),
                }),
                location,
            ));
        }
        let value = source
            .values
            .first()
            .ok_or_else(|| self.invalid_state("table schema assignment source disappeared"))?;
        let mut assigned = table.clone();
        if property == "VariableNames" {
            let names = self.table_schema_names(value, location)?;
            assigned.rename_variables(names).map_err(|error| {
                self.table_error("table property assignment", error.to_string(), location)
            })?;
        } else if property == "RowNames" {
            let names = self.table_row_names(value, location)?;
            assigned.set_row_names(names).map_err(|error| {
                self.table_error("table property assignment", error.to_string(), location)
            })?;
        } else {
            return Err(self.table_error(
                "table property assignment",
                format!("Properties.{property} is read-only or unknown"),
                location,
            ));
        }
        Ok(Value::Table(assigned))
    }

    fn table_row_names(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<TableRowName>> {
        let raw = self.table_text_names(value, "RowNames", location)?;
        raw.into_iter()
            .map(|name| {
                TableRowName::new(name).map_err(|error| {
                    self.table_error("table property assignment", error.to_string(), location)
                })
            })
            .collect()
    }

    fn table_schema_names(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<TableVariableName>> {
        let raw = self.table_text_names(value, "VariableNames", location)?;
        let mut names = Vec::new();
        names
            .try_reserve_exact(raw.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for name in raw {
            names.push(TableVariableName::new(name).map_err(|error| {
                self.table_error("table schema assignment", error.to_string(), location)
            })?);
        }
        Ok(names)
    }

    fn table_text_names(
        &self,
        value: &Value,
        property: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<String>> {
        let mut raw = Vec::new();
        match value {
            Value::Array(ArrayData::Char(array)) => {
                raw.push(self.table_name_from_char(array, "table schema assignment", location)?);
            }
            Value::String(strings) => {
                let count = usize::try_from(strings.numel())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                raw.try_reserve_exact(count)
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                for offset in 0..count {
                    let element = strings.element(offset).ok_or_else(|| {
                        self.invalid_state("string table schema escaped storage validation")
                    })?;
                    if element.is_missing() {
                        return Err(self.table_error(
                            "table schema assignment",
                            "missing strings cannot be table variable names",
                            location,
                        ));
                    }
                    raw.push(String::from_utf16(element.code_units()).map_err(|_| {
                        self.table_error(
                            "table schema assignment",
                            "a string variable name contains invalid UTF-16",
                            location,
                        )
                    })?);
                }
            }
            Value::Cell(cell) => {
                raw.try_reserve_exact(cell.values().len())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                for value in cell.values() {
                    let Value::Array(ArrayData::Char(array)) = value else {
                        return Err(self.table_error(
                            "table schema assignment",
                            format!(
                                "{property} must be a cell array of char row vectors or a string array"
                            ),
                            location,
                        ));
                    };
                    raw.push(self.table_name_from_char(
                        array,
                        "table schema assignment",
                        location,
                    )?);
                }
            }
            _ => {
                return Err(self.table_error(
                    "table schema assignment",
                    format!("{property} must be char, cellstr, or a string array"),
                    location,
                ));
            }
        }
        Ok(raw)
    }

    fn delete_table_field(
        &self,
        table: &TableArray,
        field: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if field == "Properties" {
            return Err(self.table_error(
                "table variable deletion",
                "Properties cannot be deleted",
                location,
            ));
        }
        let index = table.variable_index(field).ok_or_else(|| {
            self.table_error(
                "table variable deletion",
                format!("variable `{field}` does not exist"),
                location,
            )
        })?;
        let mut assigned = table.clone();
        assigned.remove_variable(index).map_err(|error| {
            self.table_error("table variable deletion", error.to_string(), location)
        })?;
        Ok(Value::Table(assigned))
    }

    fn assign_table_field(
        &mut self,
        table: &TableArray,
        field: &str,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if field == "Properties" {
            return Err(self.table_error(
                "table property assignment",
                "Properties itself is read-only; only Properties.VariableNames may be assigned",
                location,
            ));
        }
        let existing = table.variable_index(field);
        let name = if existing.is_none() {
            Some(TableVariableName::new(field).map_err(|error| {
                self.table_error("table variable assignment", error.to_string(), location)
            })?)
        } else {
            None
        };

        // Validate the schema and row count before crossing the language-copy
        // boundary. This keeps a failed replacement or insertion fully
        // transactional, including for value-class object variables.
        let mut validation = table.clone();
        if let Some(index) = existing {
            validation
                .replace_variable(index, value.clone())
                .map_err(|error| {
                    self.table_error("table variable assignment", error.to_string(), location)
                })?;
        } else {
            validation
                .append_variable(
                    name.clone().ok_or_else(|| {
                        self.invalid_state("new table variable lost its validated name")
                    })?,
                    value.clone(),
                )
                .map_err(|error| {
                    self.table_error("table variable assignment", error.to_string(), location)
                })?;
        }

        let (copied, created) = self.language_copy_tracked(value)?;
        let mut assigned = table.clone();
        let result = if let Some(index) = existing {
            assigned.replace_variable(index, copied).map(|_| ())
        } else {
            assigned.append_variable(
                name.ok_or_else(|| {
                    self.invalid_state("new table variable lost its validated name")
                })?,
                copied,
            )
        };
        if let Err(error) = result {
            self.rollback_assignment_copies(&created);
            return Err(self.table_error("table variable assignment", error.to_string(), location));
        }
        Ok(Value::Table(assigned))
    }

    fn assign_struct_field(
        &mut self,
        structure: &StructArray,
        field: &str,
        source: &AssignmentSource,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let field = self.validate_struct_field_name(field, location)?;
        let selected = usize::try_from(structure.numel())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        if source.values.len() != selected || (!source.expanded && selected != 1) {
            return Err(self.error(
                array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                    selected: structure.numel(),
                    supplied: u64::try_from(source.values.len()).unwrap_or(u64::MAX),
                }),
                location,
            ));
        }
        let mut assigned = structure.clone();
        let field_index = if let Some(field_index) = assigned.field_index(field.as_str()) {
            field_index
        } else {
            assigned.append_empty_field(field).map_err(|error| {
                self.aggregate_value_error("struct field insertion", error, location)
            })?;
            assigned
                .field_count()
                .checked_sub(1)
                .ok_or_else(|| self.invalid_state("appended struct field has no schema position"))?
        };
        let copied = self.language_copy_values(&source.values)?;
        for (offset, value) in copied.into_iter().enumerate() {
            assigned
                .replace_at(field_index, offset, value)
                .map_err(|error| {
                    self.aggregate_value_error("struct field assignment", error, location)
                })?;
        }
        Ok(Value::Struct(assigned))
    }

    fn write_aggregate_selection_raw(
        &mut self,
        target: &Value,
        arguments: &[IndexInput],
        supplied: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        match (target, supplied) {
            (Value::Cell(target), Value::Cell(supplied)) => {
                let selection = array_ops::resolve_index_selection(
                    target.shape().dimensions(),
                    arguments,
                    &self.cancellation,
                )
                .map_err(|kind| self.error(kind, location))?;
                if supplied.numel() != u64::try_from(selection.offsets.len()).unwrap_or(u64::MAX) {
                    return Err(self.invalid_state(
                        "nested cell selection changed cardinality during write-back",
                    ));
                }
                let mut assigned = target.clone();
                for (source, offset) in selection.offsets.into_iter().enumerate() {
                    let value = supplied.value_at_offset(source).cloned().ok_or_else(|| {
                        self.invalid_state("nested cell source escaped validation")
                    })?;
                    assigned.replace_at_offset(offset, value).map_err(|error| {
                        self.aggregate_value_error("nested cell write-back", error, location)
                    })?;
                }
                Ok(Value::Cell(assigned))
            }
            (Value::Struct(target), Value::Struct(supplied)) => {
                self.check_struct_schema(target, supplied, location)?;
                let selection = array_ops::resolve_index_selection(
                    target.shape().dimensions(),
                    arguments,
                    &self.cancellation,
                )
                .map_err(|kind| self.error(kind, location))?;
                if supplied.numel() != u64::try_from(selection.offsets.len()).unwrap_or(u64::MAX) {
                    return Err(self.invalid_state(
                        "nested struct selection changed cardinality during write-back",
                    ));
                }
                let mut assigned = target.clone();
                for (field, name) in target.field_names().iter().enumerate() {
                    let source_field = supplied
                        .field_index(name.as_str())
                        .ok_or_else(|| self.invalid_state("nested struct source lost a field"))?;
                    for (source, &offset) in selection.offsets.iter().enumerate() {
                        let value = supplied
                            .value_at(source_field, source)
                            .cloned()
                            .ok_or_else(|| {
                                self.invalid_state("nested struct source escaped validation")
                            })?;
                        assigned.replace_at(field, offset, value).map_err(|error| {
                            self.aggregate_value_error("nested struct write-back", error, location)
                        })?;
                    }
                }
                Ok(Value::Struct(assigned))
            }
            _ => Err(self.aggregate_type_error(
                "nested parenthesized aggregate write-back",
                "matching cell or struct",
                supplied,
                location,
            )),
        }
    }

    fn selection_with_scalar_growth(
        &self,
        shape: &Shape,
        arguments: &[IndexInput],
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(array_ops::ResolvedSelection, Option<Shape>)> {
        match array_ops::resolve_index_selection(shape.dimensions(), arguments, &self.cancellation)
        {
            Ok(selection) => Ok((selection, None)),
            Err(original) => {
                if let Some(grown) =
                    self.grown_shape_for_scalar_and_colon(shape, arguments, location)?
                {
                    let selection = array_ops::resolve_index_selection(
                        grown.dimensions(),
                        arguments,
                        &self.cancellation,
                    )
                    .map_err(|kind| self.error(kind, location))?;
                    return Ok((selection, Some(grown)));
                }
                let Ok(indices) = array_ops::scalar_subscripts(arguments, &self.cancellation)
                else {
                    return Err(self.error(original, location));
                };
                let grown = self.grown_shape(shape, &indices, operation, location)?;
                let offset = grown
                    .offset_for_subscripts(&indices)
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                let offset = usize::try_from(offset)
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                let scalar = Shape::new([1, 1])
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                Ok((
                    array_ops::ResolvedSelection {
                        offsets: vec![offset],
                        shape: scalar,
                    },
                    Some(grown),
                ))
            }
        }
    }

    fn grown_shape_for_scalar_and_colon(
        &self,
        shape: &Shape,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Option<Shape>> {
        if let [IndexInput::Value(value)] = arguments {
            let maximum = array_ops::numeric_assignment_extent(value, &self.cancellation)
                .map_err(|kind| self.error(kind, location))?;
            return maximum
                .filter(|maximum| *maximum > shape.numel())
                .map(|maximum| {
                    self.grown_shape(shape, &[maximum], "indexed assignment growth", location)
                })
                .transpose();
        }
        if arguments.len() < shape.ndims() || arguments.len() <= 1 {
            return Ok(None);
        }
        let mut dimensions = shape.dimensions().to_vec();
        dimensions.resize(arguments.len(), 1);
        let mut changed = false;
        for (argument, extent) in arguments.iter().zip(&mut dimensions) {
            let IndexInput::Value(value) = argument else {
                continue;
            };
            let Some(index) = array_ops::numeric_assignment_extent(value, &self.cancellation)
                .map_err(|kind| self.error(kind, location))?
            else {
                return Ok(None);
            };
            if index > *extent {
                *extent = index;
                changed = true;
            }
        }
        if !changed {
            return Ok(None);
        }
        Shape::new(dimensions)
            .map(Some)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))
    }

    fn grown_shape(
        &self,
        shape: &Shape,
        indices: &[u64],
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Shape> {
        let dimensions = if indices.len() == 1 {
            let index = indices[0];
            let dimensions = shape.dimensions();
            if dimensions.len() == 2 {
                let rows = dimensions[0];
                let columns = dimensions[1];
                if rows == 0 && columns == 0 {
                    vec![1, index]
                } else if rows == 0 {
                    vec![1, columns.max(index)]
                } else if columns == 0 {
                    vec![rows.max(index), 1]
                } else if rows == 1 {
                    vec![1, columns.max(index)]
                } else if columns == 1 {
                    vec![rows.max(index), 1]
                } else {
                    let columns_needed = index.div_ceil(rows);
                    vec![rows, columns.max(columns_needed)]
                }
            } else {
                let last = dimensions.len().saturating_sub(1);
                let base = dimensions[..last]
                    .iter()
                    .try_fold(1_u64, |total, extent| total.checked_mul(*extent));
                let Some(base) = base.filter(|base| *base != 0) else {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::InvalidOperand {
                            operation,
                            actual: openmat_value::ValueKind::Array,
                        }),
                        location,
                    ));
                };
                let mut grown = dimensions.to_vec();
                grown[last] = grown[last].max(index.div_ceil(base));
                grown
            }
        } else {
            let mut dimensions = shape.dimensions().to_vec();
            if indices.len() < shape.ndims() {
                let collapse_start = indices.len() - 1;
                for (extent, &index) in dimensions[..collapse_start]
                    .iter_mut()
                    .zip(&indices[..collapse_start])
                {
                    *extent = (*extent).max(index);
                }
                let last = dimensions.len() - 1;
                let collapsed_base = dimensions[collapse_start..last]
                    .iter()
                    .try_fold(1_u64, |total, extent| total.checked_mul(*extent));
                let Some(collapsed_base) = collapsed_base.filter(|base| *base != 0) else {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::InvalidOperand {
                            operation,
                            actual: openmat_value::ValueKind::Array,
                        }),
                        location,
                    ));
                };
                dimensions[last] =
                    dimensions[last].max(indices[collapse_start].div_ceil(collapsed_base));
            } else {
                dimensions.resize(indices.len(), 1);
                for (extent, &index) in dimensions.iter_mut().zip(indices) {
                    *extent = (*extent).max(index);
                }
            }
            dimensions
        };
        let grown = Shape::new(dimensions)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        if grown.numel() <= shape.numel() {
            return Err(self.invalid_state("aggregate growth did not increase the target shape"));
        }
        Ok(grown)
    }

    fn grow_cell_values(
        &self,
        cell: &CellArray,
        grown_shape: Option<&Shape>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let Some(grown_shape) = grown_shape else {
            return Ok(cell.values().to_vec());
        };
        let output_len = usize::try_from(grown_shape.numel())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        self.grow_aggregate_column(
            cell.shape(),
            grown_shape,
            cell.values(),
            output_len,
            location,
        )
    }

    fn grow_nested_paren_target(
        &self,
        target: &Value,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let (shape, grown_shape) = match target {
            Value::Cell(cell) => (
                cell.shape(),
                self.selection_with_scalar_growth(
                    cell.shape(),
                    arguments,
                    "nested cell parenthesized assignment",
                    location,
                )?
                .1,
            ),
            Value::Struct(structure) => (
                structure.shape(),
                self.selection_with_scalar_growth(
                    structure.shape(),
                    arguments,
                    "nested struct parenthesized assignment",
                    location,
                )?
                .1,
            ),
            _ => return Ok(target.clone()),
        };
        let Some(grown_shape) = grown_shape else {
            return Ok(target.clone());
        };
        let output_len = usize::try_from(grown_shape.numel())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;

        match target {
            Value::Cell(cell) => {
                let values = self.grow_aggregate_column(
                    shape,
                    &grown_shape,
                    cell.values(),
                    output_len,
                    location,
                )?;
                CellArray::from_values(grown_shape, values)
                    .map(Value::Cell)
                    .map_err(|error| {
                        self.aggregate_value_error("nested cell growth", error, location)
                    })
            }
            Value::Struct(structure) => {
                let mut columns = Vec::new();
                columns
                    .try_reserve_exact(structure.field_count())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                for field in 0..structure.field_count() {
                    let values = structure.field_values(field).ok_or_else(|| {
                        self.invalid_state("nested struct growth field escaped validation")
                    })?;
                    columns.push(self.grow_aggregate_column(
                        shape,
                        &grown_shape,
                        values,
                        output_len,
                        location,
                    )?);
                }
                StructArray::from_columns(grown_shape, structure.field_names().to_vec(), columns)
                    .map(Value::Struct)
                    .map_err(|error| {
                        self.aggregate_value_error("nested struct growth", error, location)
                    })
            }
            _ => Err(self.invalid_state("nested aggregate growth changed target kind")),
        }
    }

    fn grow_aggregate_column(
        &self,
        old_shape: &Shape,
        new_shape: &Shape,
        values: &[Value],
        output_len: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if old_shape == new_shape {
            return Ok(values.to_vec());
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(output_len)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        output.resize_with(output_len, Value::empty_double);
        for (offset, value) in values.iter().enumerate() {
            let linear = u64::try_from(offset)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let subscripts = old_shape
                .subscripts_for_linear_index(linear, new_shape.ndims())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let destination = new_shape
                .offset_for_subscripts(&subscripts)
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let destination = usize::try_from(destination)
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let slot = output.get_mut(destination).ok_or_else(|| {
                self.invalid_state("aggregate growth destination escaped validation")
            })?;
            *slot = value.clone();
        }
        Ok(output)
    }

    fn delete_cell(
        &self,
        cell: &CellArray,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let selection = array_ops::resolve_index_selection(
            cell.shape().dimensions(),
            arguments,
            &self.cancellation,
        )
        .map_err(|kind| self.error(kind, location))?;
        if selection.offsets.is_empty() {
            return Ok(Value::Cell(cell.clone()));
        }
        let (shape, keep) =
            self.deletion_plan(cell.shape(), arguments, &selection.offsets, location)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(keep.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for offset in keep {
            values.push(
                cell.value_at_offset(offset)
                    .cloned()
                    .ok_or_else(|| self.invalid_state("cell deletion offset escaped validation"))?,
            );
        }
        CellArray::from_values(shape, values)
            .map(Value::Cell)
            .map_err(|error| self.aggregate_value_error("cell deletion", error, location))
    }

    fn delete_struct(
        &self,
        structure: &StructArray,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let selection = array_ops::resolve_index_selection(
            structure.shape().dimensions(),
            arguments,
            &self.cancellation,
        )
        .map_err(|kind| self.error(kind, location))?;
        if selection.offsets.is_empty() {
            return Ok(Value::Struct(structure.clone()));
        }
        let (shape, keep) =
            self.deletion_plan(structure.shape(), arguments, &selection.offsets, location)?;
        let mut columns = Vec::new();
        columns
            .try_reserve_exact(structure.field_count())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for field in 0..structure.field_count() {
            let source = structure
                .field_values(field)
                .ok_or_else(|| self.invalid_state("struct deletion field escaped validation"))?;
            let mut column = Vec::new();
            column
                .try_reserve_exact(keep.len())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            for &offset in &keep {
                column.push(source.get(offset).cloned().ok_or_else(|| {
                    self.invalid_state("struct deletion offset escaped validation")
                })?);
            }
            columns.push(column);
        }
        StructArray::from_columns(shape, structure.field_names().to_vec(), columns)
            .map(Value::Struct)
            .map_err(|error| self.aggregate_value_error("struct deletion", error, location))
    }

    fn deletion_plan(
        &self,
        shape: &Shape,
        arguments: &[IndexInput],
        offsets: &[usize],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(Shape, Vec<usize>)> {
        if offsets.is_empty() {
            let keep = (0..usize::try_from(shape.numel())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?)
                .collect();
            return Ok((shape.clone(), keep));
        }
        let removed = offsets
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let length = usize::try_from(shape.numel())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        if arguments.len() == 1 {
            let dimensions = shape.dimensions();
            let is_row = dimensions.len() == 2 && dimensions[0] == 1;
            let is_column = dimensions.len() == 2 && dimensions[1] == 1;
            let remaining = length.checked_sub(removed.len()).ok_or_else(|| {
                self.invalid_state("aggregate deletion removed more elements than exist")
            })?;
            let new_shape = if is_row && is_column {
                Shape::new([0, 0])
            } else if is_row {
                Shape::new([1, u64::try_from(remaining).unwrap_or(u64::MAX)])
            } else if is_column {
                Shape::new([u64::try_from(remaining).unwrap_or(u64::MAX), 1])
            } else if remaining == 0 {
                Shape::new([0, 0])
            } else {
                return Err(self.invalid_deletion(shape, arguments.len(), location));
            }
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let keep = (0..length)
                .filter(|offset| !removed.contains(offset))
                .collect();
            return Ok((new_shape, keep));
        }

        if arguments.len() != shape.ndims() {
            return Err(self.invalid_deletion(shape, arguments.len(), location));
        }
        let varying = arguments
            .iter()
            .enumerate()
            .filter_map(|(dimension, argument)| {
                (!matches!(argument, IndexInput::Colon)).then_some(dimension)
            })
            .collect::<Vec<_>>();
        if varying.is_empty() {
            return Ok((
                Shape::new([0, 0])
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
                Vec::new(),
            ));
        }
        if varying.len() != 1 {
            return Err(self.invalid_deletion(shape, arguments.len(), location));
        }
        let dimension = varying[0];
        let extent = shape.extent(dimension);
        let stride = shape.stride(dimension);
        let mut coordinates = std::collections::BTreeSet::new();
        for &offset in offsets {
            let offset = u64::try_from(offset)
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            coordinates.insert((offset / stride) % extent);
        }
        let mut dimensions = shape.dimensions().to_vec();
        dimensions[dimension] = dimensions[dimension]
            .checked_sub(u64::try_from(coordinates.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| self.invalid_state("aggregate deletion extent underflowed"))?;
        let new_shape = Shape::new(dimensions)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let keep = (0..length)
            .filter(|offset| {
                let coordinate = (u64::try_from(*offset).unwrap_or(u64::MAX) / stride) % extent;
                !coordinates.contains(&coordinate)
            })
            .collect();
        Ok((new_shape, keep))
    }

    fn invalid_deletion(
        &self,
        shape: &Shape,
        arguments: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        self.error(
            array_error(ArrayRuntimeError::InvalidDeletionShape {
                dimensions: shape.dimensions().to_vec(),
                arguments,
            }),
            location,
        )
    }

    fn check_scalar_expansion(
        &self,
        supplied: u64,
        selected: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let selected = u64::try_from(selected)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        if supplied == 1 || supplied == selected {
            Ok(())
        } else {
            Err(self.error(
                array_error(ArrayRuntimeError::AssignmentSizeMismatch { selected, supplied }),
                location,
            ))
        }
    }

    fn check_struct_schema(
        &self,
        destination: &StructArray,
        source: &StructArray,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let equal = destination.field_count() == source.field_count()
            && destination
                .field_names()
                .iter()
                .all(|name| source.field_index(name.as_str()).is_some());
        if equal {
            return Ok(());
        }
        Err(self.error(
            array_error(ArrayRuntimeError::StructSchemaMismatch {
                destination: destination
                    .field_names()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                source: source
                    .field_names()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            }),
            location,
        ))
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn call_value(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        callee: &Value,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        match callee {
            Value::Function(FunctionHandle::Bytecode(handle)) => {
                let code = if let Some(code) = self.function_handles.get(handle).cloned() {
                    code
                } else if let Some(name) = self.named_function_handles.get(handle).cloned() {
                    let fallback = self.named_function_fallbacks.get(handle).cloned();
                    return stack
                        .run(|stack| {
                            self.call_named_callable(
                                stack,
                                module,
                                &name,
                                arguments,
                                requested_outputs,
                                caller_context,
                                location,
                                fallback,
                            )
                        })
                        .await;
                } else if handle.index() < REGISTERED_FUNCTION_HANDLE_BASE {
                    RuntimeFunctionCode {
                        module: Arc::clone(module),
                        function: FunctionId::new(handle.index()),
                        captures: Arc::new(BTreeMap::new()),
                        access_context: AccessContext::external(),
                    }
                } else {
                    return Err(self.error(
                        RuntimeErrorKind::InvalidFunctionHandle {
                            handle: FunctionHandle::Bytecode(*handle),
                        },
                        location,
                    ));
                };
                stack
                    .run(|stack| {
                        self.execute_function(
                            stack,
                            &code.module,
                            code.function,
                            arguments,
                            Invocation {
                                requested_outputs,
                                seeded_local: None,
                                copy_arguments: true,
                                access_context: code.access_context,
                                captures: code.captures,
                                scope: InvocationScope::New,
                            },
                        )
                    })
                    .await
            }
            Value::Function(FunctionHandle::Builtin(handle)) => {
                stack
                    .run(|stack| {
                        self.call_builtin(
                            stack,
                            module,
                            *handle,
                            arguments.to_vec(),
                            requested_outputs,
                            location,
                        )
                    })
                    .await
            }
            Value::Function(FunctionHandle::Class(handle)) => stack
                .run(|stack| {
                    self.construct(
                        stack,
                        *handle,
                        arguments.to_vec(),
                        requested_outputs,
                        caller_context,
                        location,
                    )
                })
                .await
                .map(|value| vec![value]),
            Value::Function(FunctionHandle::Intrinsic(intrinsic)) => {
                stack
                    .run(|stack| {
                        self.call_intrinsic(
                            stack,
                            module,
                            *intrinsic,
                            arguments,
                            requested_outputs,
                            caller_context,
                            location,
                        )
                    })
                    .await
            }
            other => Err(self.error(
                RuntimeErrorKind::NotCallable {
                    actual: other.kind(),
                },
                location,
            )),
        }
    }

    fn call_value_owned(
        &mut self,
        module: &Arc<BytecodeModule>,
        callee: &Value,
        arguments: Vec<Value>,
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        frames::run(async |stack| {
            self.call_value_owned_on_stack(
                stack,
                module,
                callee,
                arguments,
                requested_outputs,
                caller_context,
                location,
            )
            .await
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn call_value_owned_on_stack(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        callee: &Value,
        arguments: Vec<Value>,
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        match callee {
            Value::Function(FunctionHandle::Builtin(handle)) => {
                stack
                    .run(|stack| {
                        self.call_builtin(
                            stack,
                            module,
                            *handle,
                            arguments,
                            requested_outputs,
                            location,
                        )
                    })
                    .await
            }
            Value::Function(FunctionHandle::Class(handle)) => stack
                .run(|stack| {
                    self.construct(
                        stack,
                        *handle,
                        arguments,
                        requested_outputs,
                        caller_context,
                        location,
                    )
                })
                .await
                .map(|value| vec![value]),
            _ => {
                stack
                    .run(|stack| {
                        self.call_value(
                            stack,
                            module,
                            callee,
                            &arguments,
                            requested_outputs,
                            caller_context,
                            location,
                        )
                    })
                    .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn call_event_runtime(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        name: &str,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        match name {
            "addlistener" => stack
                .run(|stack| {
                    self.add_event_listener(
                        stack,
                        module,
                        arguments,
                        requested_outputs,
                        caller_context,
                        location,
                    )
                })
                .await
                .map(|listener| vec![listener]),
            "notify" => {
                if requested_outputs != 0 {
                    return Err(self.event_type_error(
                        "notify",
                        "notify does not return a value",
                        location,
                    ));
                }
                stack
                    .run(|stack| self.notify_event(stack, arguments, caller_context, location))
                    .await?;
                Ok(Vec::new())
            }
            _ => Err(self.invalid_state("unknown event runtime callable escaped dispatch")),
        }
    }

    async fn add_event_listener(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 3 {
            return Err(self.event_type_error(
                "addlistener",
                format!(
                    "addlistener requires source, event name, and callback; got {} input(s)",
                    arguments.len()
                ),
                location,
            ));
        }
        if requested_outputs > 1 {
            return Err(self.event_type_error(
                "addlistener",
                "addlistener returns at most one listener handle",
                location,
            ));
        }
        let (source, class_id) =
            self.event_source(&arguments[0], "addlistener", "event source", location)?;
        let event = self.function_name_text("addlistener", &arguments[1], location)?;
        if event != "ObjectBeingDestroyed" {
            self.classes
                .resolve_event(class_id, &event, EventAccess::Listen, caller_context)
                .map_err(|error| {
                    self.event_type_error("addlistener", error.to_string(), location)
                })?;
        }
        if !matches!(arguments[2], Value::Function(_)) {
            return Err(self.event_type_error(
                "addlistener",
                "listener callback must be a function handle",
                location,
            ));
        }
        let callback = self.language_copy(&arguments[2])?;
        let listener = stack
            .run(|stack| self.allocate_event_listener_object(stack, source, &callback, location))
            .await?;

        let order_key = (source, event.clone());
        if self
            .event_listener_order
            .entry(order_key.clone())
            .or_default()
            .try_reserve(1)
            .is_err()
        {
            if self
                .event_listener_order
                .get(&order_key)
                .is_some_and(Vec::is_empty)
            {
                self.event_listener_order.remove(&order_key);
            }
            stack
                .run(|stack| self.rollback_object_on_stack(stack, listener))
                .await;
            return Err(self.invalid_state("event listener order exceeds host capacity"));
        }
        if let Err(error) = self.append_event_listener_root(source, listener, location) {
            if self
                .event_listener_order
                .get(&order_key)
                .is_some_and(Vec::is_empty)
            {
                self.event_listener_order.remove(&order_key);
            }
            stack
                .run(|stack| self.rollback_object_on_stack(stack, listener))
                .await;
            return Err(error);
        }
        self.event_listeners.insert(
            listener,
            RuntimeListener {
                module: Arc::clone(module),
                source,
                event,
                callback,
            },
        );
        self.event_listener_order
            .get_mut(&order_key)
            .expect("event order was reserved before listener attachment")
            .push(listener);
        Ok(Value::Object(listener))
    }

    async fn allocate_event_listener_object(
        &mut self,
        stack: &mut FrameStack,
        source: ObjectHandle,
        callback: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ObjectHandle> {
        let listener_class = self.ensure_event_listener_class(location)?;
        let reference = self
            .objects
            .allocate_with(&self.classes, listener_class.class_id, |property| {
                Ok(match property.name() {
                    EVENT_LISTENER_SOURCE_PROPERTY => Value::Object(source),
                    EVENT_LISTENER_CALLBACK_PROPERTY => callback.clone(),
                    _ => Value::Nothing,
                })
            })
            .map_err(|error| self.object_error("event listener allocation", &error, location))?;
        let listener = ObjectHandle::new(reference.id().get());
        self.object_references.insert(listener, reference);
        let construction = (|| {
            let reference = self.object_references.get(&listener).ok_or_else(|| {
                unknown_object_error(
                    &self.call_stack,
                    listener,
                    "event listener allocation",
                    location,
                )
            })?;
            self.objects
                .begin_construction(reference)
                .map_err(|error| {
                    self.object_error("event listener allocation", &error, location)
                })?;
            self.objects
                .finish_construction(reference)
                .map_err(|error| self.object_error("event listener allocation", &error, location))
        })();
        if let Err(error) = construction {
            stack
                .run(|stack| self.rollback_object_on_stack(stack, listener))
                .await;
            return Err(error);
        }
        Ok(listener)
    }

    async fn notify_event(
        &mut self,
        stack: &mut FrameStack,
        arguments: &[Value],
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        if !(2..=3).contains(&arguments.len()) {
            return Err(self.event_type_error(
                "notify",
                format!(
                    "notify requires source, event name, and optional event data; got {} input(s)",
                    arguments.len()
                ),
                location,
            ));
        }
        let (source, class_id) =
            self.event_source(&arguments[0], "notify", "event source", location)?;
        let event = self.function_name_text("notify", &arguments[1], location)?;
        if event == "ObjectBeingDestroyed" {
            return Err(self.event_type_error(
                "notify",
                "ObjectBeingDestroyed is emitted only by handle finalization",
                location,
            ));
        }
        self.classes
            .resolve_event(class_id, &event, EventAccess::Notify, caller_context)
            .map_err(|error| self.event_type_error("notify", error.to_string(), location))?;
        let event_data = if let Some(value) = arguments.get(2) {
            self.language_copy(value)?
        } else {
            self.default_event_data(source, &event, location)?
        };
        stack
            .run(|stack| self.dispatch_event(stack, source, &event, &event_data, location))
            .await
    }

    async fn dispatch_event(
        &mut self,
        stack: &mut FrameStack,
        source: ObjectHandle,
        event: &str,
        event_data: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let snapshot = self
            .event_listener_order
            .get(&(source, event.to_owned()))
            .cloned()
            .unwrap_or_default();
        self.event_dispatch_depth = self.event_dispatch_depth.saturating_add(1);
        for listener in snapshot.into_iter().rev() {
            if let Err(error) = self.check_cancelled(location) {
                self.event_dispatch_depth = self.event_dispatch_depth.saturating_sub(1);
                if self.event_dispatch_depth == 0 {
                    self.flush_event_warnings();
                }
                return Err(error);
            }
            let Some(registration) = self.event_listeners.get(&listener).cloned() else {
                continue;
            };
            if registration.source != source
                || registration.event != event
                || self.active_event_listeners.contains(&listener)
            {
                continue;
            }
            let listener_alive = self
                .object_references
                .get(&listener)
                .is_some_and(|reference| {
                    self.objects
                        .handle_state(reference)
                        .is_ok_and(|state| state == HandleState::Alive)
                });
            if !listener_alive {
                continue;
            }
            self.active_event_listeners.insert(listener);
            let event_arguments = [Value::Object(source), event_data.clone()];
            let result = stack
                .run(|stack| {
                    self.call_value(
                        stack,
                        &registration.module,
                        &registration.callback,
                        &event_arguments,
                        0,
                        AccessContext::external(),
                        location,
                    )
                })
                .await;
            self.active_event_listeners.remove(&listener);
            if let Err(error) = result {
                self.pending_event_errors.push(error);
            }
        }
        self.event_dispatch_depth = self.event_dispatch_depth.saturating_sub(1);
        if self.event_dispatch_depth == 0 {
            self.flush_event_warnings();
        }
        Ok(())
    }

    fn event_source(
        &self,
        value: &Value,
        function: &str,
        label: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(ObjectHandle, ClassId)> {
        let handle = object_handle(value).ok_or_else(|| {
            self.event_type_error(
                function,
                format!("{label} must be a scalar handle object"),
                location,
            )
        })?;
        let reference = self.object_references.get(&handle).ok_or_else(|| {
            self.event_type_error(function, format!("{label} is unknown"), location)
        })?;
        if reference.semantics() != ClassSemantics::Handle
            || !matches!(self.objects.handle_state(reference), Ok(HandleState::Alive))
        {
            return Err(self.event_type_error(
                function,
                format!("{label} must be a valid handle object"),
                location,
            ));
        }
        Ok((handle, reference.class_id()))
    }

    fn ensure_event_listener_class(
        &mut self,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<RuntimeListenerClass> {
        if let Some(listener_class) = &self.event_listener_class {
            return Ok(listener_class.clone());
        }
        let listener_root_shape = Shape::new([0, 0])
            .map_err(|_| self.invalid_state("event listener root shape is invalid"))?;
        let listener_root = CellArray::from_values(listener_root_shape, Vec::new())
            .map(Value::Cell)
            .map_err(|error| self.aggregate_value_error("event listener root", error, location))?;
        let definition = ClassDefinition::handle(EVENT_LISTENER_CLASS_NAME)
            .sealed()
            .with_property(
                PropertyDescriptor::stored(EVENT_LISTENER_SOURCE_PROPERTY, None)
                    .with_get_access(ObjectAccess::Private)
                    .with_set_access(ObjectAccess::Private),
            )
            .with_property(
                PropertyDescriptor::stored(EVENT_LISTENER_CALLBACK_PROPERTY, None)
                    .with_get_access(ObjectAccess::Private)
                    .with_set_access(ObjectAccess::Private),
            )
            .with_property(
                PropertyDescriptor::stored(EVENT_SOURCE_LISTENERS_PROPERTY, Some(listener_root))
                    .with_get_access(ObjectAccess::Private)
                    .with_set_access(ObjectAccess::Private),
            );
        let class_id = self
            .classes
            .register_class(definition)
            .map_err(|error| self.object_error("event listener registration", &error, location))?;
        let mut source_key = None;
        let mut callback_key = None;
        let mut listeners_key = None;
        for (key, descriptor) in self
            .classes
            .stored_properties(class_id)
            .map_err(|error| self.object_error("event listener registration", &error, location))?
        {
            match descriptor.name() {
                EVENT_LISTENER_SOURCE_PROPERTY => source_key = Some(key),
                EVENT_LISTENER_CALLBACK_PROPERTY => callback_key = Some(key),
                EVENT_SOURCE_LISTENERS_PROPERTY => listeners_key = Some(key),
                _ => {}
            }
        }
        let listener_class = RuntimeListenerClass {
            class_id,
            source_key: source_key.ok_or_else(|| {
                self.invalid_state("event listener class is missing its source slot")
            })?,
        };
        let _callback_key = callback_key.ok_or_else(|| {
            self.invalid_state("event listener class is missing its callback slot")
        })?;
        let listeners_key = listeners_key.ok_or_else(|| {
            self.invalid_state("event listener class is missing its listener root slot")
        })?;
        self.class_code.entry(class_id).or_default();
        self.event_source_slots.insert(class_id, listeners_key);
        self.event_listener_class = Some(listener_class.clone());
        Ok(listener_class)
    }

    fn append_event_listener_root(
        &mut self,
        source: ObjectHandle,
        listener: ObjectHandle,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let reference = self.reference(source, "event listener attachment", location)?;
        let key = self
            .event_source_slots
            .get(&reference.class_id())
            .cloned()
            .ok_or_else(|| self.invalid_state("event source is missing its listener root slot"))?;
        let mut values = match self
            .objects
            .slot(reference, &key)
            .map_err(|error| self.object_error("event listener attachment", &error, location))?
        {
            Value::Cell(cell) => cell.values().to_vec(),
            _ => return Err(self.invalid_state("event listener root slot is not a cell array")),
        };
        values
            .try_reserve(1)
            .map_err(|_| self.invalid_state("event listener roots exceed host capacity"))?;
        values.push(Value::Object(listener));
        let columns = u64::try_from(values.len())
            .map_err(|_| self.invalid_state("event listener roots exceed language limits"))?;
        let shape = Shape::new([1, columns])
            .map_err(|_| self.invalid_state("event listener root shape is invalid"))?;
        let assigned = CellArray::from_values(shape, values)
            .map(Value::Cell)
            .map_err(|error| {
                self.aggregate_value_error("event listener attachment", error, location)
            })?;
        let reference = self.object_references.get(&source).ok_or_else(|| {
            unknown_object_error(
                &self.call_stack,
                source,
                "event listener attachment",
                location,
            )
        })?;
        match self.objects.slot_mut(reference, &key) {
            Ok(slot) => *slot = assigned,
            Err(error) => {
                return Err(self.object_error("event listener attachment", &error, location));
            }
        }
        Ok(())
    }

    fn detach_event_listener(&mut self, listener: ObjectHandle) {
        let Some(registration) = self.event_listeners.remove(&listener) else {
            return;
        };
        let order_key = (registration.source, registration.event);
        if let Some(order) = self.event_listener_order.get_mut(&order_key) {
            order.retain(|candidate| *candidate != listener);
            if order.is_empty() {
                self.event_listener_order.remove(&order_key);
            }
        }
        self.active_event_listeners.remove(&listener);
        self.remove_event_listener_root(registration.source, listener);
        if let Some(listener_class) = &self.event_listener_class
            && let Some(reference) = self.object_references.get(&listener)
            && self
                .objects
                .handle_state(reference)
                .is_ok_and(|state| state == HandleState::Alive)
            && let Ok(slot) = self.objects.slot_mut(reference, &listener_class.source_key)
        {
            *slot = Value::Nothing;
        }
    }

    fn detach_event_source(&mut self, source: ObjectHandle) {
        let listeners = self
            .event_listeners
            .iter()
            .filter_map(|(listener, registration)| {
                (registration.source == source).then_some(*listener)
            })
            .collect::<Vec<_>>();
        for listener in listeners {
            self.detach_event_listener(listener);
        }
    }

    fn remove_event_listener_root(&mut self, source: ObjectHandle, listener: ObjectHandle) {
        let Some(reference) = self.object_references.get(&source) else {
            return;
        };
        if !matches!(self.objects.handle_state(reference), Ok(HandleState::Alive)) {
            return;
        }
        let Some(key) = self.event_source_slots.get(&reference.class_id()).cloned() else {
            return;
        };
        let Ok(Value::Cell(cell)) = self.objects.slot(reference, &key) else {
            return;
        };
        let values = cell
            .values()
            .iter()
            .filter(|value| object_handle(value) != Some(listener))
            .cloned()
            .collect::<Vec<_>>();
        let Ok(columns) = u64::try_from(values.len()) else {
            return;
        };
        let Ok(shape) = Shape::new([u64::from(columns != 0), columns]) else {
            return;
        };
        let Ok(cell) = CellArray::from_values(shape, values) else {
            return;
        };
        if let Ok(slot) = self.objects.slot_mut(reference, &key) {
            *slot = Value::Cell(cell);
        }
    }

    fn default_event_data(
        &self,
        source: ObjectHandle,
        event: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let shape =
            Shape::new([1, 1]).map_err(|_| self.invalid_state("event data shape is invalid"))?;
        let fields = ["Source", "EventName"]
            .into_iter()
            .map(FieldName::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| self.invalid_state("event data schema is invalid"))?;
        let event_name = self.char_row_value(event, "event name", location)?;
        StructArray::from_columns(
            shape,
            fields,
            vec![vec![Value::Object(source)], vec![event_name]],
        )
        .map(Value::Struct)
        .map_err(|error| self.aggregate_value_error("event data", error, location))
    }

    fn event_type_error(
        &self,
        name: &str,
        message: impl Into<String>,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        self.intrinsic_builtin_error(name, BuiltinErrorCategory::Type, None, message, location)
    }

    fn flush_event_warnings(&mut self) {
        const IDENTIFIER: &str = "OpenMat:EventCallbackError";
        for error in std::mem::take(&mut self.pending_event_errors) {
            let message = format!("error in event listener callback: {error}");
            if self.warning_state.record(&message, IDENTIFIER) {
                let _ = self
                    .output
                    .emit(OutputEvent::CommandText(format!("Warning: {message}\n")));
            }
        }
    }

    fn prepare_builtin_scope(
        &mut self,
        module: &BytecodeModule,
        function: &Function,
        frame: &ExecutionFrame,
    ) -> RuntimeResult<()> {
        let mut workspace = if frame.function == module.entry {
            self.workspace.clone()
        } else {
            Workspace::new()
        };
        if frame.function != module.entry {
            for (name, kind) in &frame.named_bindings {
                let value = match kind {
                    NamedBindingKind::Local(slot) => {
                        if let Some(shared) = frame.shared_locals.get(slot) {
                            shared.read().ok_or_else(|| {
                                self.invalid_state("shared local lock is poisoned")
                            })?
                        } else {
                            get(&frame.locals, slot.get()).cloned().ok_or_else(|| {
                                self.invalid_state("named local escaped validation")
                            })?
                        }
                    }
                    NamedBindingKind::Persistent(slot) => {
                        let key = persistent_binding_key(function, frame, *slot);
                        let Some(value) = self.persistent_bindings.get(&key).cloned() else {
                            continue;
                        };
                        value
                    }
                    NamedBindingKind::Capture => {
                        let Some(binding) = frame.captures.get(name) else {
                            continue;
                        };
                        match binding {
                            CapturedBinding::Value(value) => value.clone(),
                            CapturedBinding::Shared(shared) => shared.read().ok_or_else(|| {
                                self.invalid_state("shared capture lock is poisoned")
                            })?,
                            CapturedBinding::MissingValue | CapturedBinding::ResolverOnly => {
                                continue;
                            }
                        }
                    }
                    NamedBindingKind::Workspace => {
                        let Some(value) = self.workspace.get(name).cloned() else {
                            continue;
                        };
                        value
                    }
                };
                if !matches!(value, Value::Nothing) {
                    workspace.insert(name.clone(), value);
                }
            }
        }
        for (name, value) in &frame.dynamic_locals {
            workspace.insert(name.clone(), value.clone());
        }
        self.builtin_workspace = Some(workspace.clone());
        self.builtin_scope_context = Some(Arc::new(ScopeContext {
            address: frame.scope_address,
            workspace,
            imports: frame.imports.clone(),
            caller: frame.caller_scope.clone(),
        }));
        Ok(())
    }

    fn apply_pending_scope_bindings(
        &mut self,
        module: &BytecodeModule,
        function: &Function,
        frame: &mut ExecutionFrame,
    ) -> RuntimeResult<()> {
        self.builtin_workspace = None;
        self.builtin_scope_context = None;
        let changes = std::mem::take(&mut self.pending_scope_bindings);
        let mut deferred = Vec::new();
        deferred
            .try_reserve(changes.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), None))?;
        for change in changes {
            if change.target == frame.scope_address {
                self.apply_frame_scope_binding(module, function, frame, change.name, change.value)?;
            } else if change.target == ScopeAddress::Base {
                self.apply_base_scope_binding(change.name, change.value);
            } else {
                deferred.push(change);
            }
        }
        self.pending_scope_bindings = deferred;
        Ok(())
    }

    fn apply_frame_scope_binding(
        &mut self,
        module: &BytecodeModule,
        function: &Function,
        frame: &mut ExecutionFrame,
        name: String,
        value: Value,
    ) -> RuntimeResult<()> {
        if frame.function == module.entry {
            if matches!(value, Value::Nothing) {
                self.remove_workspace_binding(&name);
            } else {
                self.replace_workspace_binding(name, value);
            }
            return Ok(());
        }
        let Some(kind) = frame.named_bindings.get(&name).copied() else {
            if matches!(value, Value::Nothing) {
                if let Some(previous) = frame.dynamic_locals.remove(&name) {
                    self.note_root_removal(&previous);
                }
            } else if let Some(previous) = frame.dynamic_locals.insert(name, value) {
                self.note_root_removal(&previous);
            }
            return Ok(());
        };
        match kind {
            NamedBindingKind::Local(slot) => {
                self.commit_binding(frame, &RuntimeBindingTarget::Local(slot), value)?;
            }
            NamedBindingKind::Persistent(slot) => {
                let key = persistent_binding_key(function, frame, slot);
                if matches!(value, Value::Nothing) {
                    if let Some(previous) = self.persistent_bindings.remove(&key) {
                        self.note_root_removal(&previous);
                    }
                } else if let Some(previous) = self.persistent_bindings.insert(key, value) {
                    self.note_root_removal(&previous);
                }
            }
            NamedBindingKind::Capture => {
                let Some(binding) = frame.captures.get(&name) else {
                    if !matches!(value, Value::Nothing) {
                        frame.dynamic_locals.insert(name, value);
                    }
                    return Ok(());
                };
                match binding {
                    CapturedBinding::Shared(shared) => {
                        let previous = shared
                            .replace(value)
                            .ok_or_else(|| self.invalid_state("shared capture lock is poisoned"))?;
                        self.note_root_removal(&previous);
                    }
                    CapturedBinding::Value(_)
                    | CapturedBinding::MissingValue
                    | CapturedBinding::ResolverOnly => {
                        if matches!(value, Value::Nothing) {
                            if let Some(previous) = frame.dynamic_locals.remove(&name) {
                                self.note_root_removal(&previous);
                            }
                        } else if let Some(previous) = frame.dynamic_locals.insert(name, value) {
                            self.note_root_removal(&previous);
                        }
                    }
                }
            }
            NamedBindingKind::Workspace => {
                if matches!(value, Value::Nothing) {
                    self.remove_workspace_binding(&name);
                } else {
                    self.replace_workspace_binding(name, value);
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_value(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        target: &Value,
        arguments: &[IndexInput],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if let Some(result) = self.ffi_apply_value(target, arguments, requested_outputs, location) {
            return result;
        }
        if matches!(target, Value::Function(_)) {
            let mut values = Vec::with_capacity(arguments.len());
            for (argument, value) in arguments.iter().enumerate() {
                match value {
                    IndexInput::Value(value) => values.push(value.clone()),
                    IndexInput::Colon => {
                        return Err(self.error(
                            array_error(ArrayRuntimeError::ColonCallArgument { argument }),
                            location,
                        ));
                    }
                }
            }
            return stack
                .run(|stack| {
                    self.call_value(
                        stack,
                        module,
                        target,
                        &values,
                        requested_outputs,
                        caller_context,
                        location,
                    )
                })
                .await;
        }
        if requested_outputs > 1 {
            return Err(self.error(
                array_error(ArrayRuntimeError::IndexOutputArity {
                    requested: requested_outputs,
                }),
                location,
            ));
        }
        if arguments.is_empty() {
            return Ok(vec![target.clone()]);
        }
        let value = if let Value::Table(table) = target {
            self.table_parenthesis_index(table, arguments, location)?
        } else if is_object_value(target) {
            self.index_object_value(target, arguments, location)?
        } else if matches!(target, Value::Cell(_) | Value::Struct(_)) {
            self.index_aggregate_value(target, arguments, true, location)?
        } else if let Some(value) = sparse::try_index(target, arguments, &self.cancellation) {
            value.map_err(|kind| self.error(kind, location))?
        } else {
            array_ops::index(target, arguments, &self.cancellation)
                .map_err(|kind| self.error(kind, location))?
        };
        Ok(vec![value])
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_value_owned(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        target: &Value,
        arguments: Vec<IndexInput>,
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if !matches!(target, Value::Function(_)) {
            return stack
                .run(|stack| {
                    self.apply_value(
                        stack,
                        module,
                        target,
                        &arguments,
                        requested_outputs,
                        caller_context,
                        location,
                    )
                })
                .await;
        }
        let mut values = Vec::with_capacity(arguments.len());
        for (argument, value) in arguments.into_iter().enumerate() {
            match value {
                IndexInput::Value(value) => values.push(value),
                IndexInput::Colon => {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::ColonCallArgument { argument }),
                        location,
                    ));
                }
            }
        }
        stack
            .run(|stack| {
                self.call_value_owned_on_stack(
                    stack,
                    module,
                    target,
                    values,
                    requested_outputs,
                    caller_context,
                    location,
                )
            })
            .await
    }

    fn statement_field_output_count(
        &self,
        object: &Value,
        name: &str,
        context: AccessContext,
    ) -> usize {
        if matches!(object, Value::Struct(_)) {
            return 1;
        }
        if let Some(handle) = class_handle(object)
            && let Ok(class_id) = self.class_id(handle, "static method dispatch", None)
        {
            return usize::from(
                self.classes
                    .resolve_method(class_id, name, MethodKind::Static, context)
                    .is_err(),
            );
        }
        if let Some(handle) = object_handle(object)
            && let Ok(reference) = self.reference(handle, "method dispatch", None)
        {
            return usize::from(
                self.classes
                    .resolve_method(reference.class_id(), name, MethodKind::Instance, context)
                    .is_err(),
            );
        }
        1
    }

    fn is_native_object(&self, value: &Value) -> bool {
        object_handle(value)
            .and_then(|handle| self.object_references.get(&handle))
            .is_some_and(|reference| self.native_classes.contains_key(&reference.class_id()))
    }

    fn finish_statement_result(
        &mut self,
        function: &Function,
        frame: &mut ExecutionFrame,
        target: StatementResultTarget,
        display: bool,
        value: Option<Value>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let Some(value) = value else {
            return Ok(());
        };
        let value = self.language_copy(&value)?;
        match target {
            StatementResultTarget::Global(name) => {
                let name = global_name(function, name)
                    .ok_or_else(|| self.invalid_state("statement result name escaped validation"))?
                    .to_owned();
                self.replace_workspace_binding(name, value.clone());
            }
            StatementResultTarget::Local(slot) => {
                let target = get_mut(&mut frame.locals, slot.get()).ok_or_else(|| {
                    self.invalid_state("statement result local escaped validation")
                })?;
                let previous = std::mem::replace(target, value.clone());
                self.note_root_removal(&previous);
            }
        }
        if display {
            self.emit_named_display("ans", value, location)?;
        }
        Ok(())
    }

    fn emit_named_display(
        &mut self,
        name: &str,
        value: Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        if let Err(error) = self.output.emit(OutputEvent::NamedDisplay {
            name: name.to_owned(),
            value,
        }) {
            return Err(self.error(
                RuntimeErrorKind::Builtin {
                    name: "automatic display".to_owned(),
                    category: BuiltinErrorCategory::Output,
                    identifier: None,
                    message: error.message,
                },
                location,
            ));
        }
        Ok(())
    }

    fn index_aggregate_value(
        &mut self,
        target: &Value,
        arguments: &[IndexInput],
        copy_values: bool,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let dimensions = target.dimensions().ok_or_else(|| {
            self.aggregate_type_error("aggregate indexing", "cell or struct", target, location)
        })?;
        let selection =
            array_ops::resolve_index_selection(dimensions, arguments, &self.cancellation)
                .map_err(|kind| self.error(kind, location))?;
        match target {
            Value::Cell(cell) => {
                let mut values = Vec::new();
                values
                    .try_reserve_exact(selection.offsets.len())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                for offset in selection.offsets {
                    values.push(cell.value_at_offset(offset).cloned().ok_or_else(|| {
                        self.invalid_state("cell index offset escaped validation")
                    })?);
                }
                let values = if copy_values {
                    self.language_copy_values(&values)?
                } else {
                    values
                };
                CellArray::from_values(selection.shape, values)
                    .map(Value::Cell)
                    .map_err(|error| self.aggregate_value_error("cell indexing", error, location))
            }
            Value::Struct(structure) => {
                let mut columns = Vec::new();
                columns
                    .try_reserve_exact(structure.field_count())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                for field in 0..structure.field_count() {
                    let source = structure.field_values(field).ok_or_else(|| {
                        self.invalid_state("struct field escaped validated schema")
                    })?;
                    let mut column = Vec::new();
                    column
                        .try_reserve_exact(selection.offsets.len())
                        .map_err(|_| {
                            self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                        })?;
                    for &offset in &selection.offsets {
                        column.push(source.get(offset).cloned().ok_or_else(|| {
                            self.invalid_state("struct index offset escaped validation")
                        })?);
                    }
                    columns.push(column);
                }
                if copy_values {
                    self.language_copy_columns(&mut columns)?;
                }
                StructArray::from_columns(
                    selection.shape,
                    structure.field_names().to_vec(),
                    columns,
                )
                .map(Value::Struct)
                .map_err(|error| self.aggregate_value_error("struct indexing", error, location))
            }
            _ => Err(self.aggregate_type_error(
                "aggregate indexing",
                "cell or struct",
                target,
                location,
            )),
        }
    }

    fn index_object_value(
        &self,
        target: &Value,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let parts = self.object_value_parts(target, "indexing", location)?;
        let selection = array_ops::resolve_index_selection(
            parts.shape.dimensions(),
            arguments,
            &self.cancellation,
        )
        .map_err(|kind| self.error(kind, location))?;
        if selection.offsets.len() == 1 {
            return parts
                .handles
                .get(selection.offsets[0])
                .copied()
                .map(Value::Object)
                .ok_or_else(|| self.invalid_state("object index offset escaped validation"));
        }
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(selection.offsets.len())
            .map_err(|_| self.invalid_state("object index result exceeds host capacity"))?;
        for (progress, offset) in selection.offsets.into_iter().enumerate() {
            self.check_loop_cancelled(progress, location)?;
            handles.push(
                *parts
                    .handles
                    .get(offset)
                    .ok_or_else(|| self.invalid_state("object index offset escaped validation"))?,
            );
        }
        let array = ObjectArray::from_vec(
            ClassHandle::new(parts.class_id.get()),
            selection.shape,
            handles,
        )
        .map_err(|_| self.invalid_state("object index result shape escaped validation"))?;
        Ok(Value::ObjectArray(array))
    }

    fn index_assign_value(
        &mut self,
        target: &Value,
        arguments: &[IndexInput],
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if is_object_value(target) || (matches!(target, Value::Nothing) && is_object_value(value)) {
            self.index_assign_object(target, arguments, value, location)
        } else if let Some(assigned) =
            sparse::try_index_assign(target, arguments, value, &self.cancellation)
        {
            assigned.map_err(|kind| self.error(kind, location))
        } else {
            match array_ops::index_assign(target, arguments, value, &self.cancellation) {
                Ok(assigned) => Ok(assigned),
                Err(original) => {
                    let Value::Array(ArrayData::F64(target)) = target else {
                        return Err(self.error(original, location));
                    };
                    let Some(grown_shape) =
                        self.grown_shape_for_scalar_and_colon(target.shape(), arguments, location)?
                    else {
                        return Err(self.error(original, location));
                    };
                    let grown = self.grow_double_array(target, grown_shape, location)?;
                    array_ops::index_assign(
                        &Value::Array(ArrayData::F64(grown)),
                        arguments,
                        value,
                        &self.cancellation,
                    )
                    .map_err(|kind| self.error(kind, location))
                }
            }
        }
    }

    fn grow_double_array(
        &self,
        array: &DenseArray<f64>,
        shape: Shape,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<DenseArray<f64>> {
        let output_len = usize::try_from(shape.numel())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(output_len)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        values.resize(output_len, 0.0);
        for (offset, value) in array.as_slice().iter().copied().enumerate() {
            let linear = u64::try_from(offset)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let subscripts = array
                .shape()
                .subscripts_for_linear_index(linear, shape.ndims())
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let destination = shape
                .offset_for_subscripts(&subscripts)
                .ok()
                .and_then(|offset| usize::try_from(offset).ok())
                .ok_or_else(|| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            values[destination] = value;
        }
        DenseArray::from_vec(shape, values)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))
    }

    #[allow(clippy::too_many_lines)]
    fn index_assign_object(
        &mut self,
        target: &Value,
        arguments: &[IndexInput],
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let supplied = self.object_value_parts(value, "indexed assignment", location)?;
        let (class_id, shape, mut handles, offsets) = if matches!(target, Value::Nothing) {
            let requested = array_ops::scalar_linear_index(arguments, &self.cancellation)
                .map_err(|kind| self.error(kind, location))?;
            if requested != 1 {
                return Err(self.error(
                    array_error(ArrayRuntimeError::ObjectArrayGrowthGap {
                        current: 0,
                        requested,
                    }),
                    location,
                ));
            }
            (
                supplied.class_id,
                Shape::new([1, 1]).map_err(|_| {
                    self.invalid_state("scalar object-array shape cannot be represented")
                })?,
                Vec::new(),
                vec![0],
            )
        } else {
            let target = self.object_value_parts(target, "indexed assignment", location)?;
            if target.class_id != supplied.class_id {
                return Err(self.object_error(
                    "indexed assignment",
                    &ObjectError::HeterogeneousArrayElement {
                        index: 0,
                        expected: target.class_id,
                        actual: supplied.class_id,
                    },
                    location,
                ));
            }
            match array_ops::resolve_index_selection(
                target.shape.dimensions(),
                arguments,
                &self.cancellation,
            ) {
                Ok(selection) => (
                    target.class_id,
                    target.shape,
                    target.handles,
                    selection.offsets,
                ),
                Err(original) => {
                    let Ok(requested) =
                        array_ops::scalar_linear_index(arguments, &self.cancellation)
                    else {
                        return Err(self.error(original, location));
                    };
                    let current = target.shape.numel();
                    if requested <= current {
                        return Err(self.error(original, location));
                    }
                    let next = current.checked_add(1).ok_or_else(|| {
                        self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                    })?;
                    if requested != next {
                        return Err(self.error(
                            array_error(ArrayRuntimeError::ObjectArrayGrowthGap {
                                current,
                                requested,
                            }),
                            location,
                        ));
                    }
                    let dimensions = target.shape.dimensions();
                    let grown = if dimensions.len() == 2 && dimensions[0] == 1 {
                        Shape::new([1, requested])
                    } else if dimensions.len() == 2 && dimensions[1] == 1 {
                        Shape::new([requested, 1])
                    } else {
                        return Err(self.error(original, location));
                    }
                    .map_err(|_| self.invalid_state("grown object-array shape overflowed"))?;
                    (
                        target.class_id,
                        grown,
                        target.handles,
                        vec![usize::try_from(current).map_err(|_| {
                            self.invalid_state("object-array length exceeds host capacity")
                        })?],
                    )
                }
            }
        };

        let selected = u64::try_from(offsets.len())
            .map_err(|_| self.invalid_state("object selection exceeds u64"))?;
        if supplied.shape.numel() != 1 && supplied.shape.numel() != selected {
            return Err(self.error(
                array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                    selected,
                    supplied: supplied.shape.numel(),
                }),
                location,
            ));
        }
        if offsets.is_empty() {
            return Ok(target.clone());
        }

        let result_len = usize::try_from(shape.numel())
            .map_err(|_| self.invalid_state("grown object-array length exceeds host capacity"))?;
        let growing = handles.len().checked_add(1) == Some(result_len)
            && offsets.as_slice() == [handles.len()];
        let replacing =
            result_len == handles.len() && offsets.iter().all(|offset| *offset < handles.len());
        if !growing && !replacing {
            return Err(self.invalid_state("grown object-array element count is inconsistent"));
        }

        let mut sources = Vec::new();
        sources
            .try_reserve_exact(offsets.len())
            .map_err(|_| self.invalid_state("object assignment exceeds host capacity"))?;
        for position in 0..offsets.len() {
            self.check_loop_cancelled(position, location)?;
            let source = if supplied.shape.numel() == 1 {
                0
            } else {
                position
            };
            sources.push(*supplied.handles.get(source).ok_or_else(|| {
                self.invalid_state("object assignment source offset escaped validation")
            })?);
        }
        let (assigned, created) =
            self.copy_object_handles(&sources, "indexed assignment", location)?;
        if growing && assigned.len() == 1 {
            handles.try_reserve_exact(1).map_err(|_| {
                self.rollback_assignment_copies(&created);
                self.invalid_state("grown object-array exceeds host capacity")
            })?;
            handles.push(assigned[0]);
        } else if replacing {
            for (position, (offset, assigned)) in offsets.iter().zip(&assigned).enumerate() {
                self.check_loop_cancelled(position, location)
                    .inspect_err(|_| {
                        self.rollback_assignment_copies(&created);
                    })?;
                handles[*offset] = *assigned;
            }
        } else {
            self.rollback_assignment_copies(&created);
            return Err(self.invalid_state("grown object-array element count is inconsistent"));
        }
        let result = self.make_validated_object_array(class_id, shape, handles, location);
        if result.is_err() {
            self.rollback_assignment_copies(&created);
        }
        result
    }

    fn object_value_parts(
        &self,
        value: &Value,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ObjectValueParts> {
        match value {
            Value::Object(handle) => {
                let reference = self.reference(*handle, operation, location)?;
                Ok(ObjectValueParts {
                    class_id: reference.class_id(),
                    shape: Shape::new([1, 1]).map_err(|_| {
                        self.invalid_state("scalar object shape cannot be represented")
                    })?,
                    handles: vec![*handle],
                })
            }
            Value::ObjectArray(array) => {
                let class_id = self
                    .class_handles
                    .get(&array.class_handle())
                    .copied()
                    .ok_or_else(|| {
                        self.error(
                            RuntimeErrorKind::Object {
                                operation,
                                message: format!(
                                    "unknown object-array class handle {}",
                                    array.class_handle().identifier()
                                ),
                            },
                            location,
                        )
                    })?;
                for (index, handle) in array.as_slice().iter().copied().enumerate() {
                    self.check_loop_cancelled(index, location)?;
                    let reference = self.reference(handle, operation, location)?;
                    if reference.class_id() != class_id {
                        return Err(self.object_error(
                            operation,
                            &ObjectError::HeterogeneousArrayElement {
                                index,
                                expected: class_id,
                                actual: reference.class_id(),
                            },
                            location,
                        ));
                    }
                }
                let mut handles = Vec::new();
                handles
                    .try_reserve_exact(array.as_slice().len())
                    .map_err(|_| {
                        self.invalid_state("object-array handle list exceeds host capacity")
                    })?;
                handles.extend_from_slice(array.as_slice());
                Ok(ObjectValueParts {
                    class_id,
                    shape: array.shape().clone(),
                    handles,
                })
            }
            other => Err(self.error(
                RuntimeErrorKind::Object {
                    operation,
                    message: format!(
                        "value has class `{}` instead of an object class",
                        other.kind()
                    ),
                },
                location,
            )),
        }
    }

    fn object_value_class_id(&self, value: &Value) -> Result<ClassId, ()> {
        match value {
            Value::Object(handle) => self.object_reference(*handle).map(ObjectRef::class_id),
            Value::ObjectArray(array) => self
                .class_handles
                .get(&array.class_handle())
                .copied()
                .ok_or(()),
            _ => Err(()),
        }
    }

    fn copy_object_handles(
        &mut self,
        sources: &[ObjectHandle],
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(Vec<ObjectHandle>, Vec<ObjectHandle>)> {
        let mut copies = Vec::new();
        copies
            .try_reserve_exact(sources.len())
            .map_err(|_| self.invalid_state("object copy list exceeds host capacity"))?;
        let mut created = Vec::new();
        created
            .try_reserve_exact(sources.len())
            .map_err(|_| self.invalid_state("object rollback list exceeds host capacity"))?;
        for (progress, source) in sources.iter().copied().enumerate() {
            if let Err(error) = self.check_loop_cancelled(progress, location) {
                self.rollback_assignment_copies(&created);
                return Err(error);
            }
            if self.active_finalizer_for_handle(source).is_some() {
                copies.push(source);
                continue;
            }
            let Some(reference) = self.object_references.get(&source) else {
                self.rollback_assignment_copies(&created);
                return Err(unknown_object_error(
                    &self.call_stack,
                    source,
                    operation,
                    location,
                ));
            };
            if reference.semantics() == ClassSemantics::Handle
                && self
                    .objects
                    .handle_state(reference)
                    .is_ok_and(|state| state != HandleState::Alive)
            {
                copies.push(source);
                continue;
            }
            let exception = self.exception_errors.get(&source).cloned();
            let state = match self.objects.state(reference) {
                Ok(state) => state,
                Err(error) => {
                    let error = self.object_error(operation, &error, location);
                    self.rollback_assignment_copies(&created);
                    return Err(error);
                }
            };
            if state != ConstructionState::Constructed {
                copies.push(source);
                continue;
            }
            let copy = match self.objects.assignment_copy(reference) {
                Ok(copy) => copy,
                Err(error) => {
                    let error = self.object_error(operation, &error, location);
                    self.rollback_assignment_copies(&created);
                    return Err(error);
                }
            };
            if copy.semantics() == ClassSemantics::Handle {
                copies.push(source);
                continue;
            }
            let handle = ObjectHandle::new(copy.id().get());
            self.object_references.insert(handle, copy);
            if let Some(exception) = exception {
                self.exception_errors.insert(handle, exception);
            }
            copies.push(handle);
            created.push(handle);
        }
        Ok((copies, created))
    }

    fn rollback_assignment_copies(&mut self, handles: &[ObjectHandle]) {
        for handle in handles.iter().rev() {
            if let Some(reference) = self.object_references.get(handle) {
                let _ = self.objects.discard_value_assignment_copy(reference);
            }
            self.object_references.remove(handle);
            self.exception_errors.remove(handle);
        }
    }

    fn make_validated_object_array(
        &self,
        class_id: ClassId,
        shape: Shape,
        handles: Vec<ObjectHandle>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let mut references = Vec::new();
        references
            .try_reserve_exact(handles.len())
            .map_err(|_| self.invalid_state("object-array validation exceeds host capacity"))?;
        for (progress, handle) in handles.iter().enumerate() {
            self.check_loop_cancelled(progress, location)?;
            references.push(self.object_references.get(handle).ok_or_else(|| {
                unknown_object_error(
                    &self.call_stack,
                    *handle,
                    "object-array validation",
                    location,
                )
            })?);
        }
        let mut dimensions = Vec::new();
        dimensions
            .try_reserve_exact(shape.dimensions().len())
            .map_err(|_| self.invalid_state("object-array rank exceeds host capacity"))?;
        dimensions.extend_from_slice(shape.dimensions());
        ObjectArrayDescriptor::new(&self.classes, class_id, dimensions, &references)
            .map_err(|error| self.object_error("object-array validation", &error, location))?;
        ObjectArray::from_vec(ClassHandle::new(class_id.get()), shape, handles)
            .map(Value::ObjectArray)
            .map_err(|_| self.invalid_state("object-array shape and element count disagree"))
    }

    fn check_loop_cancelled(
        &self,
        progress: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        if progress.is_multiple_of(1_024) {
            self.check_cancelled(location)
        } else {
            Ok(())
        }
    }

    fn build_table_matrix(
        &mut self,
        rows: &[Vec<Value>],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let mut horizontal = Vec::with_capacity(rows.len());
        for row in rows {
            let tables = row
                .iter()
                .map(|value| match value {
                    Value::Table(table) => Ok(table),
                    other => Err(self.table_error(
                        "table concatenation",
                        format!(
                            "cannot concatenate a table with a value of class `{}`",
                            other.class_name()
                        ),
                        location,
                    )),
                })
                .collect::<RuntimeResult<Vec<_>>>()?;
            horizontal.push(self.concatenate_table_columns(&tables, location)?);
        }
        self.concatenate_table_rows(&horizontal.iter().collect::<Vec<_>>(), location)
            .map(Value::Table)
    }

    fn concatenate_table_columns(
        &self,
        tables: &[&TableArray],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<TableArray> {
        let row_count = tables.first().map_or(0, |table| table.row_count());
        let row_names = tables
            .first()
            .and_then(|table| table.row_names())
            .map(<[TableRowName]>::to_vec);
        let mut names = Vec::new();
        let mut variables = Vec::new();
        for table in tables {
            if table.row_count() != row_count {
                return Err(self.table_error(
                    "horizontal table concatenation",
                    format!(
                        "all tables must have {row_count} rows, found {}",
                        table.row_count()
                    ),
                    location,
                ));
            }
            if table.row_names() != row_names.as_deref() {
                return Err(self.table_error(
                    "horizontal table concatenation",
                    "tables with row names must have identical row names",
                    location,
                ));
            }
            names.extend_from_slice(table.variable_names());
            for index in 0..table.variable_count() {
                variables.push(table.variable(index).cloned().ok_or_else(|| {
                    self.invalid_state("horizontal table concatenation storage is incomplete")
                })?);
            }
        }
        TableArray::from_parts_with_row_names(row_count, names, variables, row_names).map_err(
            |error| {
                self.table_error(
                    "horizontal table concatenation",
                    error.to_string(),
                    location,
                )
            },
        )
    }

    fn concatenate_table_rows(
        &mut self,
        tables: &[&TableArray],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<TableArray> {
        let Some(first) = tables.first().copied() else {
            return TableArray::empty(0).map_err(|error| {
                self.table_error("vertical table concatenation", error.to_string(), location)
            });
        };
        let names = first.variable_names().to_vec();
        let mut row_count = 0_u64;
        let has_row_names = first.row_names().is_some();
        let mut row_names = has_row_names.then(Vec::new);
        for table in tables {
            if table.variable_names() != names {
                return Err(self.table_error(
                    "vertical table concatenation",
                    "all tables must have identical variable names in identical order",
                    location,
                ));
            }
            if table.row_names().is_some() != has_row_names {
                return Err(self.table_error(
                    "vertical table concatenation",
                    "either every table or no table must define row names",
                    location,
                ));
            }
            if let (Some(output), Some(names)) = (&mut row_names, table.row_names()) {
                output.extend_from_slice(names);
            }
            row_count = row_count
                .checked_add(table.row_count())
                .ok_or_else(|| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        }
        let mut variables = Vec::with_capacity(names.len());
        for variable in 0..names.len() {
            let values = tables
                .iter()
                .map(|table| {
                    table.variable(variable).ok_or_else(|| {
                        self.invalid_state("vertical table concatenation storage is incomplete")
                    })
                })
                .collect::<RuntimeResult<Vec<_>>>()?;
            variables.push(self.concatenate_table_variable_rows(&values, location)?);
        }
        TableArray::from_parts_with_row_names(row_count, names, variables, row_names).map_err(
            |error| self.table_error("vertical table concatenation", error.to_string(), location),
        )
    }

    fn concatenate_table_variable_rows(
        &mut self,
        values: &[&Value],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if values.iter().all(|value| matches!(value, Value::Cell(_))) {
            let columns = values
                .first()
                .and_then(|value| value.dimensions())
                .and_then(|dimensions| dimensions.get(1))
                .copied()
                .unwrap_or(0);
            let total_rows = values.iter().try_fold(0_u64, |total, value| {
                let dimensions = value
                    .dimensions()
                    .ok_or_else(|| self.invalid_state("cell table variable has no dimensions"))?;
                if dimensions.len() != 2 || dimensions[1] != columns {
                    return Err(self.table_error(
                        "vertical table concatenation",
                        "cell variables must have matching column counts",
                        location,
                    ));
                }
                total
                    .checked_add(dimensions[0])
                    .ok_or_else(|| self.error(array_error(ArrayRuntimeError::SizeLimit), location))
            })?;
            let length =
                usize::try_from(total_rows.checked_mul(columns).ok_or_else(|| {
                    self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                })?)
                .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
            let mut concatenated = Vec::with_capacity(length);
            for column in 0..columns {
                for value in values {
                    let Value::Cell(cell) = value else {
                        unreachable!("cell table variables were checked above")
                    };
                    let rows = cell.shape().extent(0);
                    let start = usize::try_from(column.checked_mul(rows).ok_or_else(|| {
                        self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                    })?)
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                    let end = start
                        .checked_add(usize::try_from(rows).map_err(|_| {
                            self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                        })?)
                        .ok_or_else(|| {
                            self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                        })?;
                    concatenated.extend_from_slice(cell.values().get(start..end).ok_or_else(
                        || {
                            self.invalid_state(
                                "cell table variable concatenation escaped storage validation",
                            )
                        },
                    )?);
                }
            }
            return CellArray::from_values(
                Shape::new([total_rows, columns])
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?,
                concatenated,
            )
            .map(Value::Cell)
            .map_err(|error| {
                self.aggregate_value_error("vertical cell concatenation", error, location)
            });
        }
        let rows = values
            .iter()
            .map(|value| vec![(*value).clone()])
            .collect::<Vec<_>>();
        array_ops::build_matrix(&rows, &self.cancellation)
            .map_err(|kind| self.error(kind, location))
    }

    #[allow(clippy::too_many_lines)]
    fn build_object_matrix(
        &mut self,
        rows: &[Vec<Value>],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let mut blocks = Vec::new();
        blocks
            .try_reserve_exact(rows.len())
            .map_err(|_| self.invalid_state("object matrix row table exceeds host capacity"))?;
        let mut exact_class = None;
        let mut total_rows = 0_u64;
        let mut total_columns = None;
        for row in rows {
            let mut block_row = Vec::new();
            block_row
                .try_reserve_exact(row.len())
                .map_err(|_| self.invalid_state("object matrix block row exceeds host capacity"))?;
            let mut row_height = None;
            let mut row_columns = 0_u64;
            for (index, value) in row.iter().enumerate() {
                self.check_loop_cancelled(index, location)?;
                let block = self.object_value_parts(value, "concatenation", location)?;
                if block.shape.dimensions().len() != 2 {
                    return Err(self.error(
                        RuntimeErrorKind::UnsupportedClassFeature {
                            feature: "object-array concatenation above two dimensions".to_owned(),
                        },
                        location,
                    ));
                }
                match exact_class {
                    None => exact_class = Some(block.class_id),
                    Some(expected) if expected != block.class_id => {
                        return Err(self.object_error(
                            "concatenation",
                            &ObjectError::HeterogeneousArrayElement {
                                index,
                                expected,
                                actual: block.class_id,
                            },
                            location,
                        ));
                    }
                    Some(_) => {}
                }
                let height = block.shape.dimensions()[0];
                if let Some(expected) = row_height
                    && expected != height
                {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::ConcatenationMismatch {
                            axis: 0,
                            expected,
                            actual: height,
                        }),
                        location,
                    ));
                }
                row_height = Some(height);
                row_columns = row_columns
                    .checked_add(block.shape.dimensions()[1])
                    .ok_or_else(|| self.invalid_state("object matrix width overflowed"))?;
                block_row.push(block);
            }
            let height = row_height.unwrap_or(0);
            if let Some(expected) = total_columns
                && expected != row_columns
            {
                return Err(self.error(
                    array_error(ArrayRuntimeError::ConcatenationMismatch {
                        axis: 1,
                        expected,
                        actual: row_columns,
                    }),
                    location,
                ));
            }
            total_columns = Some(row_columns);
            total_rows = total_rows
                .checked_add(height)
                .ok_or_else(|| self.invalid_state("object matrix height overflowed"))?;
            blocks.push((height, block_row));
        }
        let class_id = exact_class
            .ok_or_else(|| self.invalid_state("object matrix detection found no object element"))?;
        let total_columns = total_columns.unwrap_or(0);
        let total = total_rows
            .checked_mul(total_columns)
            .ok_or_else(|| self.invalid_state("object matrix element count overflowed"))?;
        let length = usize::try_from(total)
            .map_err(|_| self.invalid_state("object matrix exceeds host capacity"))?;
        let mut ordered = Vec::new();
        ordered
            .try_reserve_exact(length)
            .map_err(|_| self.invalid_state("object matrix exceeds host capacity"))?;
        ordered.resize(length, None);
        let mut row_base = 0_u64;
        for (height, row) in blocks {
            let mut column_base = 0_u64;
            for block in row {
                let block_columns = block.shape.dimensions()[1];
                for column in 0..block_columns {
                    for row in 0..height {
                        let source = row
                            .checked_add(column.checked_mul(height).ok_or_else(|| {
                                self.invalid_state("object matrix source offset overflowed")
                            })?)
                            .ok_or_else(|| {
                                self.invalid_state("object matrix source offset overflowed")
                            })?;
                        let destination = row_base
                            .checked_add(row)
                            .and_then(|value| {
                                column_base
                                    .checked_add(column)
                                    .and_then(|column| column.checked_mul(total_rows))
                                    .and_then(|column| value.checked_add(column))
                            })
                            .ok_or_else(|| {
                                self.invalid_state("object matrix destination offset overflowed")
                            })?;
                        let destination = usize::try_from(destination).map_err(|_| {
                            self.invalid_state("object matrix offset exceeds host capacity")
                        })?;
                        let source = usize::try_from(source).map_err(|_| {
                            self.invalid_state("object matrix offset exceeds host capacity")
                        })?;
                        *ordered.get_mut(destination).ok_or_else(|| {
                            self.invalid_state("object matrix destination escaped validation")
                        })? = Some(*block.handles.get(source).ok_or_else(|| {
                            self.invalid_state("object matrix source escaped validation")
                        })?);
                    }
                }
                column_base = column_base
                    .checked_add(block_columns)
                    .ok_or_else(|| self.invalid_state("object matrix column offset overflowed"))?;
            }
            row_base = row_base
                .checked_add(height)
                .ok_or_else(|| self.invalid_state("object matrix row offset overflowed"))?;
        }
        let shape = Shape::new([total_rows, total_columns])
            .map_err(|_| self.invalid_state("object matrix shape overflowed"))?;
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(ordered.len())
            .map_err(|_| self.invalid_state("object matrix exceeds host capacity"))?;
        for source in ordered {
            sources.push(
                source.ok_or_else(|| {
                    self.invalid_state("object matrix was not completely populated")
                })?,
            );
        }
        let (handles, created) = self.copy_object_handles(&sources, "concatenation", location)?;
        let result = self.make_validated_object_array(class_id, shape, handles, location);
        if result.is_err() {
            self.rollback_assignment_copies(&created);
        }
        result
    }

    async fn register_class(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        class_id: ClassDefinitionId,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let Some(encoded) = get(&module.classes, class_id.get()).cloned() else {
            return Err(self.invalid_state("class definition index escaped validation"));
        };
        let feature = module
            .class_features
            .iter()
            .find(|feature| feature.class == class_id)
            .cloned();
        let mut definition = self.class_definition_shell(&encoded, feature.as_ref(), location)?;
        for property in &encoded.properties {
            self.check_cancelled(Some(property.location))?;
            let default = stack
                .run(|stack| self.evaluate_property_default(stack, module, &encoded.name, property))
                .await?;
            let descriptor = self.property_descriptor(property, default)?;
            definition = definition.with_property(descriptor);
        }

        let mut runtime_code = RuntimeClassCode::default();
        if let Some(feature) = &feature {
            definition =
                self.install_class_features(module, feature, definition, &mut runtime_code)?;
        }
        for method in &encoded.methods {
            self.check_cancelled(Some(method.location))?;
            if method.is_external {
                return Err(self.invalid_state(&format!(
                    "external method `{}.{}` reached runtime before class-folder linking",
                    encoded.name, method.name
                )));
            }
            let descriptor = match method.kind {
                BytecodeMethodKind::Instance => MethodDescriptor::instance(method.name.clone()),
                BytecodeMethodKind::Static => MethodDescriptor::static_method(method.name.clone()),
            }
            .with_access(object_access(method.access));
            let descriptor = if method.is_abstract {
                descriptor.abstract_method()
            } else {
                descriptor
            };
            let descriptor = if feature
                .as_ref()
                .is_some_and(|feature| feature.sealed_methods.contains(&method.name))
            {
                descriptor.sealed()
            } else {
                descriptor
            };
            definition = definition.with_method(descriptor);
            if !method.is_abstract {
                runtime_code.methods.insert(
                    method.name.clone(),
                    RuntimeMethodCode {
                        module: Arc::clone(module),
                        function: method.function,
                        constructor_output: method.constructor_output,
                    },
                );
            }
        }

        self.check_cancelled(location)?;
        let class = self
            .classes
            .register_class(definition)
            .map_err(|error| self.object_error("class registration", &error, location))?;
        let handle = ClassHandle::new(class.get());
        self.class_handles.insert(handle, class);
        self.class_code.insert(class, runtime_code);
        // A separately loaded subclass can be encoded before its base's handle
        // semantics are known. Listener roots follow the resolved class model.
        if self
            .classes
            .semantics(class)
            .map_err(|error| self.object_error("class registration", &error, location))?
            == ClassSemantics::Handle
        {
            let source_slot = self
                .classes
                .stored_properties(class)
                .map_err(|error| self.object_error("class registration", &error, location))?
                .into_iter()
                .find_map(|(key, property)| {
                    (property.name() == EVENT_SOURCE_LISTENERS_PROPERTY).then_some(key)
                })
                .ok_or_else(|| {
                    self.invalid_state("handle class is missing its event-listener root slot")
                })?;
            self.event_source_slots.insert(class, source_slot);
        }
        Ok(())
    }

    fn install_native_classes(&mut self) {
        let classes = self.builtins.native_classes().cloned().collect::<Vec<_>>();
        for native in classes {
            let name = native.name().to_owned();
            let mut definition = ClassDefinition::handle(name.clone())
                .with_method(MethodDescriptor::instance(name.clone()));
            for method in native.methods() {
                definition = definition.with_method(MethodDescriptor::instance(method.name()));
            }
            for property in native.properties() {
                let mut descriptor = PropertyDescriptor::dependent(property.name());
                if !property.readable() {
                    descriptor = descriptor.write_only();
                }
                if !property.writable() {
                    descriptor = descriptor.read_only();
                }
                definition = definition.with_property(descriptor);
                if property.readable() {
                    definition = definition.with_method(MethodDescriptor::instance(format!(
                        "get.{}",
                        property.name()
                    )));
                }
                if property.writable() {
                    definition = definition.with_method(MethodDescriptor::instance(format!(
                        "set.{}",
                        property.name()
                    )));
                }
            }
            let class = self
                .classes
                .register_class(definition)
                .expect("validated native class registration must succeed");
            self.class_handles
                .insert(ClassHandle::new(class.get()), class);
            self.native_classes.insert(class, native);
        }
    }

    fn install_exception_class(&mut self) {
        let mut definition = ClassDefinition::value(EXCEPTION_CLASS_NAME)
            .with_method(MethodDescriptor::instance(EXCEPTION_CLASS_NAME));
        for name in EXCEPTION_PROPERTIES {
            definition = definition.with_property(
                PropertyDescriptor::stored(name, Some(Value::Nothing))
                    .with_set_access(ObjectAccess::Private),
            );
        }
        for name in ["throw", "throwAsCaller", "addCause", "getReport"] {
            definition = definition.with_method(MethodDescriptor::instance(name));
        }
        let class_id = self
            .classes
            .register_class(definition)
            .expect("validated MException class registration must succeed");
        self.class_handles
            .insert(ClassHandle::new(class_id.get()), class_id);
        self.class_code.entry(class_id).or_default();
        self.exception_class = Some(class_id);
    }

    fn class_definition_shell(
        &self,
        encoded: &openmat_bytecode::ClassDefinition,
        feature: Option<&openmat_bytecode::ClassFeatureDefinition>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ClassDefinition<Value>> {
        let mut definition = match encoded.semantics {
            BytecodeClassSemantics::Value => ClassDefinition::value(encoded.name.clone()),
            BytecodeClassSemantics::Handle => ClassDefinition::handle(encoded.name.clone()),
        };
        if let Some(superclass) = &encoded.superclass {
            definition = definition.with_superclass(superclass.clone());
        }
        if let Some(abstract_) = encoded.declared_abstract {
            definition = definition.with_abstract(abstract_);
        }
        if encoded.sealed {
            definition = definition.sealed();
        }
        let declares_event_source_slot =
            encoded.semantics == BytecodeClassSemantics::Handle && encoded.superclass.is_none();
        definition
            .try_reserve_members(
                encoded
                    .properties
                    .len()
                    .saturating_add(usize::from(declares_event_source_slot)),
                encoded.methods.len(),
            )
            .map_err(|error| self.object_error("class registration", &error, location))?;
        if let Some(feature) = feature {
            definition
                .try_reserve_features(feature.enumeration_members.len(), feature.events.len())
                .map_err(|error| self.object_error("class registration", &error, location))?;
        }
        if declares_event_source_slot {
            let shape = Shape::new([0, 0])
                .map_err(|_| self.invalid_state("event listener root shape is invalid"))?;
            let listeners = CellArray::from_values(shape, Vec::new())
                .map(Value::Cell)
                .map_err(|error| {
                    self.aggregate_value_error("event listener root", error, location)
                })?;
            definition = definition.with_property(
                PropertyDescriptor::stored(EVENT_SOURCE_LISTENERS_PROPERTY, Some(listeners))
                    .with_get_access(ObjectAccess::Private)
                    .with_set_access(ObjectAccess::Private),
            );
        }
        Ok(definition)
    }

    fn install_class_features(
        &self,
        module: &Arc<BytecodeModule>,
        feature: &openmat_bytecode::ClassFeatureDefinition,
        mut definition: ClassDefinition<Value>,
        runtime_code: &mut RuntimeClassCode,
    ) -> RuntimeResult<ClassDefinition<Value>> {
        if feature.kind == ClassKind::Enumeration {
            if let Some(base) = &feature.enumeration_base {
                definition = definition.with_enumeration_base(base.clone());
            }
            for member in &feature.enumeration_members {
                let argument_count = usize::try_from(member.argument_count).map_err(|_| {
                    self.invalid_state("enumeration argument count escaped validation")
                })?;
                definition = definition.with_enumeration_member(EnumerationMemberDescriptor::new(
                    member.name.clone(),
                    vec![Value::Nothing; argument_count],
                ));
                runtime_code.enumeration_members.insert(
                    member.name.clone(),
                    RuntimeEnumMemberCode {
                        module: Arc::clone(module),
                        argument_initializer: member.argument_initializer,
                        argument_count,
                    },
                );
            }
        }
        for event in &feature.events {
            let mut descriptor = EventDescriptor::new(event.name.clone())
                .with_listen_access(object_access(event.listen_access))
                .with_notify_access(object_access(event.notify_access));
            if event.hidden {
                descriptor = descriptor.hidden();
            }
            definition = definition.with_event(descriptor);
        }
        for property in &feature.abstract_properties {
            let mut descriptor = match property.kind {
                BytecodePropertyKind::Stored => {
                    PropertyDescriptor::abstract_property(property.name.clone())
                }
                BytecodePropertyKind::Constant => {
                    PropertyDescriptor::abstract_constant(property.name.clone())
                }
                BytecodePropertyKind::Dependent => {
                    PropertyDescriptor::abstract_dependent(property.name.clone())
                }
            };
            descriptor = if let Some(access) = property.get_access {
                descriptor.with_get_access(object_access(access))
            } else {
                descriptor.write_only()
            };
            descriptor = if let Some(access) = property.set_access {
                descriptor.with_set_access(object_access(access))
            } else {
                descriptor.read_only()
            };
            definition = definition.with_property(descriptor);
        }
        Ok(definition)
    }

    async fn evaluate_property_default(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        class_name: &str,
        property: &openmat_bytecode::PropertyDefinition,
    ) -> RuntimeResult<Option<Value>> {
        let Some(function) = property.default else {
            return Ok(None);
        };
        let returned = stack
            .run(|stack| {
                self.execute_function(
                    stack,
                    module,
                    function,
                    &[],
                    Invocation {
                        requested_outputs: 1,
                        seeded_local: None,
                        copy_arguments: false,
                        access_context: AccessContext::external(),
                        captures: Arc::new(BTreeMap::new()),
                        scope: InvocationScope::New,
                    },
                )
            })
            .await?;
        if returned.len() != 1 {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "property default",
                    message: format!(
                        "`{class_name}.{}` returned {} values instead of one",
                        property.name,
                        returned.len()
                    ),
                },
                Some(property.location),
            ));
        }
        Ok(returned.into_iter().next())
    }

    fn property_descriptor(
        &self,
        property: &openmat_bytecode::PropertyDefinition,
        default: Option<Value>,
    ) -> RuntimeResult<PropertyDescriptor<Value>> {
        match property.kind {
            BytecodePropertyKind::Stored => {
                let Some(get_access) = property.get_access else {
                    return Err(
                        self.invalid_state("stored property read access escaped validation")
                    );
                };
                let Some(set_access) = property.set_access else {
                    return Err(
                        self.invalid_state("stored property write access escaped validation")
                    );
                };
                Ok(PropertyDescriptor::stored(property.name.clone(), default)
                    .with_get_access(object_access(get_access))
                    .with_set_access(object_access(set_access)))
            }
            BytecodePropertyKind::Constant => {
                let Some(get_access) = property.get_access else {
                    return Err(
                        self.invalid_state("constant property read access escaped validation")
                    );
                };
                let Some(value) = default else {
                    return Err(
                        self.invalid_state("constant property initializer escaped validation")
                    );
                };
                Ok(PropertyDescriptor::constant(property.name.clone(), value)
                    .with_get_access(object_access(get_access)))
            }
            BytecodePropertyKind::Dependent => {
                let mut descriptor = PropertyDescriptor::dependent(property.name.clone());
                descriptor = if let Some(get_access) = property.get_access {
                    descriptor.with_get_access(object_access(get_access))
                } else {
                    descriptor.write_only()
                };
                descriptor = if let Some(set_access) = property.set_access {
                    descriptor.with_set_access(object_access(set_access))
                } else {
                    descriptor.read_only()
                };
                Ok(descriptor)
            }
        }
    }

    fn get_field(
        &mut self,
        object: &Value,
        name: &str,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        frames::run(async |stack| {
            self.get_field_on_stack(stack, object, name, context, location)
                .await
        })
    }

    async fn get_field_on_stack(
        &mut self,
        stack: &mut FrameStack,
        object: &Value,
        name: &str,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if matches!(
            object,
            Value::Function(FunctionHandle::Intrinsic(IntrinsicFunction::Ffi))
        ) {
            let namespace = self.ffi_namespace(location)?;
            return self
                .ffi_get_field(&namespace, name, location)
                .ok_or_else(|| {
                    self.invalid_state("FFI namespace field dispatch was unavailable")
                })?;
        }
        if let Value::Table(table) = object {
            return self.get_table_field(table, name, location);
        }
        if matches!(object, Value::Graphics(_) | Value::GraphicsArray(_)) {
            return stack
                .run(|stack| self.get_graphics_field(stack, object, name, location))
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| self.invalid_state("graphics property read returned no value"));
        }
        if let Some(result) = self.ffi_get_field(object, name, location) {
            return result;
        }
        if let Some(class_handle) = class_handle(object) {
            return stack
                .run(|stack| self.get_class_field(stack, class_handle, name, context, location))
                .await;
        }
        let handle = object_handle(object).ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation: "property read",
                    message: format!("receiver has class `{}`", object.kind()),
                },
                location,
            )
        })?;
        let reference = self.reference(handle, "property read", location)?;
        let resolution = self
            .classes
            .resolve_property(reference.class_id(), name, PropertyAccess::Get, context)
            .map_err(|error| self.object_error("property read", &error, location))?;
        let key = match resolution {
            PropertyResolution::Stored { key, .. } => key,
            PropertyResolution::Constant { descriptor } => {
                let value = descriptor.default_value().cloned().ok_or_else(|| {
                    self.invalid_state("constant property value is missing after registration")
                })?;
                return self.language_copy(&value);
            }
            PropertyResolution::Accessor { descriptor, method } => {
                let declaring_class = method.declaring_class();
                if self.native_classes.contains_key(&declaring_class) {
                    let property = descriptor.name().to_owned();
                    return stack
                        .run(|stack| {
                            self.execute_native_property_get(
                                stack,
                                declaring_class,
                                handle,
                                &property,
                                location,
                            )
                        })
                        .await;
                }
                let accessor = method.descriptor().name().to_owned();
                let arguments = [object.clone()];
                let returned = stack
                    .run(|stack| {
                        self.execute_class_method(
                            stack,
                            declaring_class,
                            &accessor,
                            &arguments,
                            1,
                            "dependent property getter",
                            location,
                        )
                    })
                    .await?;
                return returned.into_iter().next().ok_or_else(|| {
                    self.invalid_state("verified dependent getter returned no value")
                });
            }
        };
        let value = if let Some(candidate) = self.active_finalizer_for_handle(handle) {
            self.objects.finalizer_slot(&candidate, &key)
        } else {
            self.objects.slot(reference, &key)
        }
        .cloned()
        .map_err(|error| self.object_error("property read", &error, location))?;
        self.language_copy(&value)
    }

    async fn get_graphics_field(
        &mut self,
        stack: &mut FrameStack,
        object: &Value,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let handle = self
            .builtins
            .handle_by_name("get")
            .ok_or_else(|| self.invalid_state("graphics property reads require builtin `get`"))?;
        let module = Arc::clone(&self.module);
        stack
            .run(|stack| {
                self.call_builtin(
                    stack,
                    &module,
                    handle,
                    vec![object.clone(), Value::from(name)],
                    1,
                    location,
                )
            })
            .await
    }

    #[allow(clippy::too_many_lines)]
    #[cfg(test)]
    fn set_field(
        &mut self,
        object: &Value,
        name: &str,
        value: &Value,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        frames::run(async |stack| {
            self.set_field_on_stack(stack, object, name, value, context, location)
                .await
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn set_field_on_stack(
        &mut self,
        stack: &mut FrameStack,
        object: &Value,
        name: &str,
        value: &Value,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if let Value::Table(table) = object {
            if is_empty_double(value) {
                return self.delete_table_field(table, name, location);
            }
            return self.assign_table_field(table, name, value, location);
        }
        if let Some(result) = self.ffi_set_field(object, name, value, location) {
            return result;
        }
        if let Some(class_handle) = class_handle(object) {
            return self.set_class_field(class_handle, name, context, location);
        }
        let handle = object_handle(object).ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation: "property write",
                    message: format!("receiver has class `{}`", object.kind()),
                },
                location,
            )
        })?;
        let class_id = self
            .reference(handle, "property write", location)?
            .class_id();
        let resolution = self
            .classes
            .resolve_property(class_id, name, PropertyAccess::Set, context)
            .map_err(|error| self.object_error("property write", &error, location))?;
        let key = match resolution {
            PropertyResolution::Stored { key, .. } => key,
            PropertyResolution::Constant { .. } => {
                return Err(self.invalid_state("writable constant escaped class validation"));
            }
            PropertyResolution::Accessor { descriptor, method } => {
                let declaring_class = method.declaring_class();
                if self.native_classes.contains_key(&declaring_class) {
                    let property = descriptor.name().to_owned();
                    stack
                        .run(|stack| {
                            self.execute_native_property_set(
                                stack,
                                declaring_class,
                                handle,
                                &property,
                                value.clone(),
                                location,
                            )
                        })
                        .await?;
                    return Ok(object.clone());
                }
                let accessor = method.descriptor().name().to_owned();
                let arguments = [object.clone(), value.clone()];
                let returned = stack
                    .run(|stack| {
                        self.execute_class_method(
                            stack,
                            declaring_class,
                            &accessor,
                            &arguments,
                            1,
                            "dependent property setter",
                            location,
                        )
                    })
                    .await?;
                let Some(updated) = returned.into_iter().next() else {
                    return Err(
                        self.invalid_state("verified dependent setter returned no updated object")
                    );
                };
                let Some(updated_handle) = object_handle(&updated) else {
                    return Err(self.error(
                        RuntimeErrorKind::Object {
                            operation: "dependent property setter",
                            message: format!("`{accessor}` did not return an object"),
                        },
                        location,
                    ));
                };
                let updated_class = self
                    .reference(updated_handle, "dependent property setter", location)?
                    .class_id();
                if updated_class != class_id {
                    return Err(self.error(
                        RuntimeErrorKind::Object {
                            operation: "dependent property setter",
                            message: format!("`{accessor}` returned an object of another class"),
                        },
                        location,
                    ));
                }
                return Ok(updated);
            }
        };
        let target_handle = self.copy_receiver_for_write(handle, location)?;
        let receiver_copy = (target_handle != handle).then_some(target_handle);
        let (assigned, assigned_copies) = match self.language_copy_tracked(value) {
            Ok(copied) => copied,
            Err(error) => {
                if let Some(receiver_copy) = receiver_copy {
                    self.rollback_assignment_copies(&[receiver_copy]);
                }
                return Err(error);
            }
        };
        let Some(reference) = self.object_references.get(&target_handle) else {
            self.rollback_assignment_copies(&assigned_copies);
            if let Some(receiver_copy) = receiver_copy {
                self.rollback_assignment_copies(&[receiver_copy]);
            }
            return Err(unknown_object_error(
                &self.call_stack,
                target_handle,
                "property write",
                location,
            ));
        };
        if let Err(error) = self
            .objects
            .slot_mut(reference, &key)
            .map(|slot| *slot = assigned)
        {
            let error = self.object_error("property write", &error, location);
            self.rollback_assignment_copies(&assigned_copies);
            if let Some(receiver_copy) = receiver_copy {
                self.rollback_assignment_copies(&[receiver_copy]);
            }
            return Err(error);
        }
        Ok(Value::Object(target_handle))
    }

    async fn get_class_field(
        &mut self,
        stack: &mut FrameStack,
        handle: ClassHandle,
        name: &str,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let class_id = self.class_id(handle, "class property read", location)?;
        if self
            .class_code
            .get(&class_id)
            .is_some_and(|class| class.enumeration_members.contains_key(name))
        {
            return stack
                .run(|stack| self.get_enumeration_member(stack, class_id, name, location))
                .await;
        }
        let resolution = self
            .classes
            .resolve_property(class_id, name, PropertyAccess::Get, context)
            .map_err(|error| self.object_error("class property read", &error, location))?;
        let value = match resolution {
            PropertyResolution::Constant { descriptor } => {
                descriptor.default_value().cloned().ok_or_else(|| {
                    self.invalid_state("constant property value is missing after registration")
                })?
            }
            PropertyResolution::Stored { .. } => {
                return Err(self.error(
                    RuntimeErrorKind::Object {
                        operation: "class property read",
                        message: format!(
                            "stored instance property `{name}` cannot be read through its class"
                        ),
                    },
                    location,
                ));
            }
            PropertyResolution::Accessor { .. } => {
                return Err(self.error(
                    RuntimeErrorKind::UnsupportedClassFeature {
                        feature: format!("dependent property `{name}`"),
                    },
                    location,
                ));
            }
        };
        self.language_copy(&value)
    }

    async fn get_enumeration_member(
        &mut self,
        stack: &mut FrameStack,
        class_id: ClassId,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let key = (class_id, name.to_owned());
        if let Some(value) = self.enumeration_members.get(&key).cloned() {
            let semantics = self
                .classes
                .semantics(class_id)
                .map_err(|error| self.object_error("enumeration lookup", &error, location))?;
            return if semantics == ClassSemantics::Handle {
                Ok(value)
            } else {
                self.language_copy(&value)
            };
        }
        if !self.enumeration_members_loading.insert(key.clone()) {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "enumeration construction",
                    message: format!("recursive construction of enumeration member `{name}`"),
                },
                location,
            ));
        }

        let result = (async {
            let code = self
                .class_code
                .get(&class_id)
                .and_then(|class| class.enumeration_members.get(name))
                .cloned()
                .ok_or_else(|| {
                    self.invalid_state("enumeration member code is missing after registration")
                })?;
            let arguments = stack
                .run(|stack| {
                    self.execute_function(
                        stack,
                        &code.module,
                        code.argument_initializer,
                        &[],
                        Invocation {
                            requested_outputs: code.argument_count,
                            seeded_local: None,
                            copy_arguments: false,
                            access_context: AccessContext::class(class_id),
                            captures: Arc::new(BTreeMap::new()),
                            scope: InvocationScope::New,
                        },
                    )
                })
                .await?;
            if arguments.len() != code.argument_count {
                return Err(self.error(
                    RuntimeErrorKind::Object {
                        operation: "enumeration construction",
                        message: format!(
                            "initializer for `{name}` returned {} arguments instead of {}",
                            arguments.len(),
                            code.argument_count
                        ),
                    },
                    location,
                ));
            }
            stack
                .run(|stack| {
                    self.construct_enumeration_member(stack, class_id, name, &arguments, location)
                })
                .await
        })
        .await;
        self.enumeration_members_loading.remove(&key);

        let value = result?;
        self.enumeration_members.insert(key, value.clone());
        let semantics = self
            .classes
            .semantics(class_id)
            .map_err(|error| self.object_error("enumeration lookup", &error, location))?;
        if semantics == ClassSemantics::Handle {
            Ok(value)
        } else {
            self.language_copy(&value)
        }
    }

    fn set_class_field(
        &self,
        handle: ClassHandle,
        name: &str,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let class_id = self.class_id(handle, "class property write", location)?;
        let resolution = self
            .classes
            .resolve_property(class_id, name, PropertyAccess::Set, context)
            .map_err(|error| self.object_error("class property write", &error, location))?;
        let feature = match resolution {
            PropertyResolution::Stored { .. } => {
                format!("stored instance property `{name}` cannot be written through its class")
            }
            PropertyResolution::Constant { .. } => {
                return Err(self.invalid_state("writable constant escaped class validation"));
            }
            PropertyResolution::Accessor { .. } => format!("dependent property `{name}`"),
        };
        Err(self.error(
            RuntimeErrorKind::UnsupportedClassFeature { feature },
            location,
        ))
    }

    async fn evaluate_binary(
        &mut self,
        stack: &mut FrameStack,
        operator: BinaryOperator,
        lhs: &Value,
        rhs: &Value,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let lhs_class = self.binary_object_class(lhs, "operator dispatch", location)?;
        let rhs_class = self.binary_object_class(rhs, "operator dispatch", location)?;
        let receiver_class = match (lhs_class, rhs_class) {
            (None, None) => {
                if let Some(outcome) = sparse::try_binary(operator, lhs, rhs, &self.cancellation) {
                    let outcome = outcome.map_err(|kind| self.error(kind, location))?;
                    if let Some(warning) = outcome.warning {
                        let identifier = warning.identifier();
                        let message = warning.message();
                        if self.warning_state.record(&message, identifier)
                            && let Err(error) = self
                                .output
                                .emit(OutputEvent::CommandText(format!("Warning: {message}\n")))
                        {
                            return Err(self.error(
                                RuntimeErrorKind::Builtin {
                                    name: "sparse matrix left division".to_owned(),
                                    category: BuiltinErrorCategory::Output,
                                    identifier: None,
                                    message: error.message,
                                },
                                location,
                            ));
                        }
                    }
                    return Ok(outcome.value);
                }
                return array_ops::evaluate_binary(
                    operator,
                    lhs,
                    rhs,
                    &self.cancellation,
                    self.linalg_provider.as_ref(),
                )
                .map_err(|kind| self.error(kind, location));
            }
            (Some(left), Some(right)) if left != right => {
                return Err(self.invalid_operands(operator, lhs, rhs, location));
            }
            (Some(class), _) | (_, Some(class)) => class,
        };
        let object_operator = object_operator(operator);
        let selection = self
            .classes
            .resolve_operator(receiver_class, object_operator, context);
        let declaring_class = match selection {
            Ok(selection) => selection.declaring_class(),
            Err(ObjectError::MethodNotFound { .. }) => {
                return Err(self.invalid_operands(operator, lhs, rhs, location));
            }
            Err(error) => return Err(self.object_error("operator dispatch", &error, location)),
        };
        let arguments = [lhs.clone(), rhs.clone()];
        let returned = stack
            .run(|stack| {
                self.execute_class_method(
                    stack,
                    declaring_class,
                    object_operator.method_name(),
                    &arguments,
                    1,
                    "operator dispatch",
                    location,
                )
            })
            .await?;
        returned
            .into_iter()
            .next()
            .ok_or_else(|| self.invalid_state("verified operator returned no value"))
    }

    fn binary_object_class(
        &self,
        value: &Value,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Option<ClassId>> {
        match value {
            Value::Object(handle) => Ok(Some(
                self.reference(*handle, operation, location)?.class_id(),
            )),
            Value::ObjectArray(array) => Ok(Some(self.class_id(
                array.class_handle(),
                operation,
                location,
            )?)),
            _ => Ok(None),
        }
    }

    fn invalid_operands(
        &self,
        operator: BinaryOperator,
        lhs: &Value,
        rhs: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        self.error(
            RuntimeErrorKind::InvalidOperands {
                operator,
                lhs: lhs.kind(),
                rhs: rhs.kind(),
            },
            location,
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_class_method(
        &mut self,
        stack: &mut FrameStack,
        declaring_class: ClassId,
        name: &str,
        arguments: &[Value],
        requested_outputs: usize,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let code = self
            .class_code
            .get(&declaring_class)
            .and_then(|class| class.methods.get(name))
            .cloned()
            .ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::Object {
                        operation,
                        message: format!("executable body for `{name}` is missing"),
                    },
                    location,
                )
            })?;
        let copy_arguments = !arguments.iter().any(|argument| {
            object_handle(argument)
                .is_some_and(|handle| self.active_finalizer_for_handle(handle).is_some())
        });
        stack
            .run(|stack| {
                self.execute_function(
                    stack,
                    &code.module,
                    code.function,
                    arguments,
                    Invocation {
                        requested_outputs,
                        seeded_local: None,
                        copy_arguments,
                        access_context: AccessContext::class(declaring_class),
                        captures: Arc::new(BTreeMap::new()),
                        scope: InvocationScope::New,
                    },
                )
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_native_method(
        &mut self,
        stack: &mut FrameStack,
        declaring_class: ClassId,
        handle: ObjectHandle,
        name: &str,
        arguments: Vec<Value>,
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let native = self.native_instances.get(&handle).ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation: "native method dispatch",
                    message: format!("native instance {} is unavailable", handle.identifier()),
                },
                location,
            )
        })?;
        let registered_class = self.native_classes.get(&declaring_class).ok_or_else(|| {
            self.invalid_state("native method declaring class has no implementation")
        })?;
        if !Arc::ptr_eq(&native.class, registered_class) {
            return Err(self.invalid_state("native instance class does not match method dispatch"));
        }
        let function: Arc<dyn BuiltinFunction> = Arc::new(NativeMethodCall {
            class: Arc::clone(&native.class),
            method: Arc::from(name),
            instance: native.instance,
        });
        let callback_name = Arc::<str>::from(format!("{}.{}", registered_class.name(), name));
        let module = Arc::clone(&self.module);
        stack
            .run(|stack| {
                self.call_builtin_function(
                    stack,
                    &module,
                    &callback_name,
                    function.as_ref(),
                    arguments,
                    requested_outputs,
                    location,
                )
            })
            .await
    }

    fn native_member(
        &self,
        declaring_class: ClassId,
        handle: ObjectHandle,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(Arc<dyn NativeClass>, NativeInstance)> {
        let native = self.native_instances.get(&handle).ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation,
                    message: format!("native instance {} is unavailable", handle.identifier()),
                },
                location,
            )
        })?;
        let registered_class = self.native_classes.get(&declaring_class).ok_or_else(|| {
            self.invalid_state("native member declaring class has no implementation")
        })?;
        if !Arc::ptr_eq(&native.class, registered_class) {
            return Err(self.invalid_state("native instance class does not match member dispatch"));
        }
        Ok((Arc::clone(&native.class), native.instance))
    }

    async fn execute_native_property_get(
        &mut self,
        stack: &mut FrameStack,
        declaring_class: ClassId,
        handle: ObjectHandle,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let (class, instance) =
            self.native_member(declaring_class, handle, "native property read", location)?;
        let callback_name = Arc::<str>::from(format!("{}.get.{name}", class.name()));
        let function: Arc<dyn BuiltinFunction> = Arc::new(NativePropertyGetCall {
            class,
            property: Arc::from(name),
            instance,
        });
        let module = Arc::clone(&self.module);
        stack
            .run(|stack| {
                self.call_builtin_function(
                    stack,
                    &module,
                    &callback_name,
                    function.as_ref(),
                    Vec::new(),
                    1,
                    location,
                )
            })
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| self.invalid_state("native property getter returned no value"))
    }

    async fn execute_native_property_set(
        &mut self,
        stack: &mut FrameStack,
        declaring_class: ClassId,
        handle: ObjectHandle,
        name: &str,
        value: Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let (class, instance) =
            self.native_member(declaring_class, handle, "native property write", location)?;
        let callback_name = Arc::<str>::from(format!("{}.set.{name}", class.name()));
        let function: Arc<dyn BuiltinFunction> = Arc::new(NativePropertySetCall {
            class,
            property: Arc::from(name),
            instance,
        });
        let module = Arc::clone(&self.module);
        stack
            .run(|stack| {
                self.call_builtin_function(
                    stack,
                    &module,
                    &callback_name,
                    function.as_ref(),
                    vec![value],
                    0,
                    location,
                )
            })
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_class_field(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        handle: ClassHandle,
        name: &str,
        arguments: &[IndexInput],
        requested_outputs: usize,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let class_id = self.class_id(handle, "static method dispatch", location)?;
        let selection = self
            .classes
            .resolve_method(class_id, name, MethodKind::Static, context);
        let declaring_class = match selection {
            Ok(selection) => Some(selection.declaring_class()),
            Err(ObjectError::MethodNotFound { .. }) => None,
            Err(error) => {
                return Err(self.object_error("static method dispatch", &error, location));
            }
        };
        let Some(declaring_class) = declaring_class else {
            let property = stack
                .run(|stack| self.get_class_field(stack, handle, name, context, location))
                .await?;
            return stack
                .run(|stack| {
                    self.apply_value(
                        stack,
                        module,
                        &property,
                        arguments,
                        requested_outputs,
                        context,
                        location,
                    )
                })
                .await;
        };
        let code = self
            .class_code
            .get(&declaring_class)
            .and_then(|class| class.methods.get(name))
            .cloned()
            .ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::Object {
                        operation: "static method dispatch",
                        message: format!("executable body for `{name}` is missing"),
                    },
                    location,
                )
            })?;
        let mut values = Vec::new();
        values
            .try_reserve(arguments.len())
            .map_err(|_| self.invalid_state("static method argument list exceeds host capacity"))?;
        for (argument, value) in arguments.iter().enumerate() {
            match value {
                IndexInput::Value(value) => values.push(value.clone()),
                IndexInput::Colon => {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::ColonCallArgument { argument }),
                        location,
                    ));
                }
            }
        }
        let returned = stack
            .run(|stack| {
                self.execute_function(
                    stack,
                    &code.module,
                    code.function,
                    &values,
                    Invocation {
                        requested_outputs,
                        seeded_local: None,
                        copy_arguments: true,
                        access_context: AccessContext::class(declaring_class),
                        captures: Arc::new(BTreeMap::new()),
                        scope: InvocationScope::New,
                    },
                )
            })
            .await?;
        Ok(returned)
    }

    fn class_id(
        &self,
        handle: ClassHandle,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ClassId> {
        self.class_handles.get(&handle).copied().ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation,
                    message: format!("unknown class handle {}", handle.identifier()),
                },
                location,
            )
        })
    }

    fn qualified_suffix_members<'a>(
        &self,
        frame: &ExecutionFrame,
        unresolved_suffix: Register,
        members: &'a [String],
    ) -> RuntimeResult<&'a [String]> {
        let Value::Double(suffix) = self.read_register(frame, unresolved_suffix)? else {
            return Err(self.invalid_state("qualified unresolved suffix is not numeric"));
        };
        if !suffix.is_finite() || *suffix < 0.0 || suffix.fract() != 0.0 {
            return Err(self.invalid_state("qualified unresolved suffix is invalid"));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let suffix = *suffix as usize;
        if suffix > members.len() {
            return Err(self.invalid_state("qualified unresolved suffix exceeds member count"));
        }
        Ok(&members[members.len() - suffix..])
    }

    async fn resolve_qualified_member_pack(
        &mut self,
        stack: &mut FrameStack,
        mut target: Value,
        members: &[String],
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let Some((last, prefix)) = members.split_last() else {
            return Ok(vec![target]);
        };
        for member in prefix {
            let values = stack
                .run(|stack| self.aggregate_field_pack(stack, &target, member, context, location))
                .await?;
            target = values.into_iter().next().ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::MissingOutputs {
                        requested: 1,
                        returned: 0,
                    },
                    location,
                )
            })?;
        }
        stack
            .run(|stack| self.aggregate_field_pack(stack, &target, last, context, location))
            .await
    }

    async fn resolve_qualified_member_receiver<'a>(
        &mut self,
        stack: &mut FrameStack,
        mut target: Value,
        members: &'a [String],
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(Value, &'a str)> {
        let (member, parents) = members
            .split_last()
            .ok_or_else(|| self.invalid_state("qualified call has no member components"))?;
        for parent in parents {
            let values = stack
                .run(|stack| self.aggregate_field_pack(stack, &target, parent, context, location))
                .await?;
            target = values.into_iter().next().ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::MissingOutputs {
                        requested: 1,
                        returned: 0,
                    },
                    location,
                )
            })?;
        }
        Ok((target, member))
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn apply_field(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        object: &Value,
        name: &str,
        arguments: Vec<IndexInput>,
        requested_outputs: usize,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if matches!(
            object,
            Value::Function(FunctionHandle::Intrinsic(IntrinsicFunction::Ffi))
        ) {
            let namespace = self.ffi_namespace(location)?;
            return self
                .ffi_apply_field(&namespace, name, &arguments, requested_outputs, location)
                .ok_or_else(|| {
                    self.invalid_state("FFI namespace call dispatch was unavailable")
                })?;
        }
        if let Some(result) =
            self.ffi_apply_field(object, name, &arguments, requested_outputs, location)
        {
            return result;
        }
        if let Value::Table(table) = object {
            let variable = self.get_table_field(table, name, location)?;
            return stack
                .run(|stack| {
                    self.apply_value(
                        stack,
                        module,
                        &variable,
                        &arguments,
                        requested_outputs,
                        context,
                        location,
                    )
                })
                .await;
        }
        if matches!(object, Value::Struct(_)) {
            return stack
                .run(|stack| {
                    self.apply_struct_field(
                        stack,
                        module,
                        object,
                        name,
                        &arguments,
                        requested_outputs,
                        context,
                        location,
                    )
                })
                .await;
        }
        if let Some(class_handle) = class_handle(object) {
            return stack
                .run(|stack| {
                    self.apply_class_field(
                        stack,
                        module,
                        class_handle,
                        name,
                        &arguments,
                        requested_outputs,
                        context,
                        location,
                    )
                })
                .await;
        }
        let Some(handle) = object_handle(object) else {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "method dispatch",
                    message: format!("receiver has class `{}`", object.kind()),
                },
                location,
            ));
        };
        let reference = self.reference(handle, "method dispatch", location)?;
        let selection = self
            .classes
            .resolve_method_any(reference.class_id(), name, context);
        let selection = match selection {
            Ok(selection) => Some((selection.declaring_class(), selection.descriptor().kind())),
            Err(ObjectError::MethodNotFound { .. }) => None,
            Err(error) => return Err(self.object_error("method dispatch", &error, location)),
        };
        let Some((declaring_class, method_kind)) = selection else {
            let property = stack
                .run(|stack| self.get_field_on_stack(stack, object, name, context, location))
                .await?;
            return stack
                .run(|stack| {
                    self.apply_value(
                        stack,
                        module,
                        &property,
                        &arguments,
                        requested_outputs,
                        context,
                        location,
                    )
                })
                .await;
        };
        if Some(declaring_class) == self.exception_class {
            let mut values = Vec::new();
            values
                .try_reserve_exact(arguments.len().saturating_add(1))
                .map_err(|_| {
                    self.invalid_state("MException method arguments exceed host capacity")
                })?;
            values.push(object.clone());
            for (argument, value) in arguments.into_iter().enumerate() {
                match value {
                    IndexInput::Value(value) => values.push(value),
                    IndexInput::Colon => {
                        return Err(self.error(
                            array_error(ArrayRuntimeError::ColonCallArgument { argument }),
                            location,
                        ));
                    }
                }
            }
            return self.call_exception_method(name, &values, requested_outputs, location);
        }
        if method_kind == MethodKind::Instance && self.native_classes.contains_key(&declaring_class)
        {
            let mut values = Vec::new();
            values.try_reserve(arguments.len()).map_err(|_| {
                self.invalid_state("native method argument list exceeds host capacity")
            })?;
            for (argument, value) in arguments.into_iter().enumerate() {
                match value {
                    IndexInput::Value(value) => values.push(value),
                    IndexInput::Colon => {
                        return Err(self.error(
                            array_error(ArrayRuntimeError::ColonCallArgument { argument }),
                            location,
                        ));
                    }
                }
            }
            return stack
                .run(|stack| {
                    self.execute_native_method(
                        stack,
                        declaring_class,
                        handle,
                        name,
                        values,
                        requested_outputs,
                        location,
                    )
                })
                .await;
        }
        let code = self
            .class_code
            .get(&declaring_class)
            .and_then(|class| class.methods.get(name))
            .cloned()
            .ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::Object {
                        operation: "method dispatch",
                        message: format!("executable body for `{name}` is missing"),
                    },
                    location,
                )
            })?;
        let mut values = Vec::new();
        let receiver_count = usize::from(method_kind == MethodKind::Instance);
        if values
            .try_reserve(arguments.len().saturating_add(receiver_count))
            .is_err()
        {
            return Err(self.invalid_state("method argument list exceeds host capacity"));
        }
        if method_kind == MethodKind::Instance {
            values.push(object.clone());
        }
        for (argument, value) in arguments.iter().enumerate() {
            match value {
                IndexInput::Value(value) => values.push(value.clone()),
                IndexInput::Colon => {
                    return Err(self.error(
                        array_error(ArrayRuntimeError::ColonCallArgument { argument }),
                        location,
                    ));
                }
            }
        }
        let returned = stack
            .run(|stack| {
                self.execute_function(
                    stack,
                    &code.module,
                    code.function,
                    &values,
                    Invocation {
                        requested_outputs,
                        seeded_local: None,
                        copy_arguments: true,
                        access_context: AccessContext::class(declaring_class),
                        captures: Arc::new(BTreeMap::new()),
                        scope: InvocationScope::New,
                    },
                )
            })
            .await?;
        Ok(returned)
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_struct_field(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        structure: &Value,
        name: &str,
        arguments: &[IndexInput],
        requested_outputs: usize,
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let Value::Struct(structure_value) = structure else {
            return Err(self.invalid_state("struct member dispatch received a non-struct value"));
        };
        if structure_value.numel() != 1 {
            return Err(self.error(
                array_error(ArrayRuntimeError::StructMemberApplyRequiresScalar {
                    numel: structure_value.numel(),
                }),
                location,
            ));
        }
        let mut field = stack
            .run(|stack| self.aggregate_field_pack(stack, structure, name, context, location))
            .await?;
        let value = field.pop().ok_or_else(|| {
            self.error(
                RuntimeErrorKind::MissingOutputs {
                    requested: 1,
                    returned: 0,
                },
                location,
            )
        })?;
        stack
            .run(|stack| {
                self.apply_value(
                    stack,
                    module,
                    &value,
                    arguments,
                    requested_outputs,
                    context,
                    location,
                )
            })
            .await
    }

    #[allow(clippy::too_many_lines)]
    async fn construct(
        &mut self,
        stack: &mut FrameStack,
        class_handle: ClassHandle,
        arguments: Vec<Value>,
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        self.check_cancelled(location)?;
        let class_id = self
            .class_handles
            .get(&class_handle)
            .copied()
            .ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::Object {
                        operation: "construction",
                        message: format!("unknown class handle {}", class_handle.identifier()),
                    },
                    location,
                )
            })?;
        if Some(class_id) == self.exception_class {
            return stack
                .run(|stack| {
                    self.construct_exception(stack, &arguments, requested_outputs, location)
                })
                .await;
        }
        self.classes
            .ensure_instantiable(class_id)
            .map_err(|error| self.object_error("construction", &error, location))?;
        if let Some(native) = self.native_classes.get(&class_id).cloned() {
            return stack
                .run(|stack| self.construct_native(stack, native, class_id, arguments, location))
                .await;
        }
        let reference = self
            .objects
            .allocate_with(&self.classes, class_id, |_| empty_double())
            .map_err(|error| self.object_error("construction", &error, location))?;
        let handle = ObjectHandle::new(reference.id().get());
        self.object_references.insert(handle, reference);
        let reference = self.object_references.get(&handle).ok_or_else(|| {
            unknown_object_error(&self.call_stack, handle, "construction", location)
        })?;
        if let Err(error) = self.objects.begin_construction(reference) {
            let error = self.object_error("construction", &error, location);
            stack
                .run(|stack| self.rollback_object_on_stack(stack, handle))
                .await;
            return Err(error);
        }
        if let Err(error) = self.check_cancelled(location) {
            stack
                .run(|stack| self.rollback_object_on_stack(stack, handle))
                .await;
            return Err(error);
        }

        let execution = stack
            .run(|stack| {
                self.execute_constructor_body(
                    stack,
                    class_id,
                    handle,
                    &arguments,
                    requested_outputs,
                    caller_context,
                    location,
                )
            })
            .await;

        let result = execution.and_then(|_| {
            let reference = self.object_references.get(&handle).ok_or_else(|| {
                unknown_object_error(&self.call_stack, handle, "construction", location)
            })?;
            if let Err(error) = self.objects.finish_construction(reference) {
                return Err(self.object_error("construction", &error, location));
            }
            Ok(Value::Object(handle))
        });
        if result.is_err() {
            stack
                .run(|stack| self.rollback_object_on_stack(stack, handle))
                .await;
        }
        result
    }

    async fn construct_native(
        &mut self,
        stack: &mut FrameStack,
        class: Arc<dyn NativeClass>,
        class_id: ClassId,
        arguments: Vec<Value>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let result = Arc::new(Mutex::new(None));
        let callback: Arc<dyn BuiltinFunction> = Arc::new(NativeConstructionCall {
            class: Arc::clone(&class),
            result: Arc::clone(&result),
        });
        let name = Arc::<str>::from(class.name());
        let module = Arc::clone(&self.module);
        stack
            .run(|stack| {
                self.call_builtin_function(
                    stack,
                    &module,
                    &name,
                    callback.as_ref(),
                    arguments,
                    0,
                    location,
                )
            })
            .await?;
        let instance = result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| self.invalid_state("native constructor returned no instance"))?;
        let native = Arc::new(NativeObjectInstance { class, instance });
        stack
            .run(|stack| self.install_native_object(stack, native, class_id, None, location))
            .await
            .map(Value::Object)
    }

    async fn construct_exception(
        &mut self,
        stack: &mut FrameStack,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if arguments.len() < 2 {
            return Err(self.intrinsic_builtin_error(
                EXCEPTION_CLASS_NAME,
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:MException:wrongNumberInputs"),
                "MException requires an identifier and message text",
                location,
            ));
        }
        if requested_outputs > 1 {
            return Err(self.intrinsic_builtin_error(
                EXCEPTION_CLASS_NAME,
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:maxlhs"),
                "MException returns one object",
                location,
            ));
        }
        let identifier =
            self.function_name_text("MException identifier", &arguments[0], location)?;
        if !identifier.is_empty() && !is_exception_identifier(&identifier) {
            return Err(self.intrinsic_builtin_error(
                EXCEPTION_CLASS_NAME,
                BuiltinErrorCategory::Domain,
                Some("MATLAB:MException:badIdentifier"),
                "MException identifier must use colon-separated name components",
                location,
            ));
        }
        let message = if arguments.len() == 2 {
            self.function_name_text("MException message", &arguments[1], location)?
        } else {
            let handle = self.builtins.handle_by_name("sprintf").ok_or_else(|| {
                self.intrinsic_builtin_error(
                    EXCEPTION_CLASS_NAME,
                    BuiltinErrorCategory::HostService,
                    Some("OpenMat:MException:FormattingUnavailable"),
                    "formatted MException messages require the standard sprintf built-in",
                    location,
                )
            })?;
            let module = Arc::clone(&self.module);
            let mut output = stack
                .run(|stack| {
                    self.call_builtin(stack, &module, handle, arguments[1..].to_vec(), 1, location)
                })
                .await?;
            let value = output.pop().ok_or_else(|| {
                self.invalid_state("sprintf returned no MException message value")
            })?;
            self.function_name_text("MException message", &value, location)?
        };
        let values = [
            Self::exception_text_value(&identifier)
                .map_err(|detail| self.invalid_state(&detail))?,
            Self::exception_text_value(&message).map_err(|detail| self.invalid_state(&detail))?,
            Self::stack_value(&[]).map_err(|detail| self.invalid_state(&detail))?,
            Self::empty_exception_cause().map_err(|detail| self.invalid_state(&detail))?,
        ];
        self.allocate_exception(values)
            .map(Value::Object)
            .map_err(|message| {
                self.error(
                    RuntimeErrorKind::Object {
                        operation: "MException construction",
                        message,
                    },
                    location,
                )
            })
    }

    async fn install_native_object(
        &mut self,
        stack: &mut FrameStack,
        native: Arc<NativeObjectInstance>,
        class_id: ClassId,
        expected_handle: Option<ObjectHandle>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ObjectHandle> {
        let reference = self
            .objects
            .allocate_with(&self.classes, class_id, |_| empty_double())
            .map_err(|error| self.object_error("native construction", &error, location))?;
        let handle = ObjectHandle::new(reference.id().get());
        if expected_handle.is_some_and(|expected| expected != handle) {
            let _ = self.objects.discard_incomplete(&reference);
            return Err(self.invalid_state(
                "native-object transaction allocation no longer matches its predicted identity",
            ));
        }
        self.object_references.insert(handle, reference);
        let reference = self.object_references.get(&handle).ok_or_else(|| {
            unknown_object_error(&self.call_stack, handle, "native construction", location)
        })?;
        if let Err(error) = self.objects.begin_construction(reference) {
            let error = self.object_error("native construction", &error, location);
            stack
                .run(|stack| self.rollback_object_on_stack(stack, handle))
                .await;
            return Err(error);
        }
        self.native_instances.insert(handle, native);
        let reference = self.object_references.get(&handle).ok_or_else(|| {
            unknown_object_error(&self.call_stack, handle, "native construction", location)
        })?;
        if let Err(error) = self.objects.finish_construction(reference) {
            let error = self.object_error("native construction", &error, location);
            stack
                .run(|stack| self.rollback_object_on_stack(stack, handle))
                .await;
            return Err(error);
        }
        Ok(handle)
    }

    async fn commit_native_creations(
        &mut self,
        stack: &mut FrameStack,
        native_objects: &mut BuiltinNativeObjects,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<ObjectHandle>> {
        let pending = std::mem::take(&mut native_objects.pending);
        let mut created = Vec::new();
        created
            .try_reserve_exact(pending.len())
            .map_err(|_| self.invalid_state("native-object commit exceeded runtime capacity"))?;
        for pending in pending {
            match stack
                .run(|stack| {
                    self.install_native_object(
                        stack,
                        pending.native,
                        pending.class_id,
                        Some(pending.temporary),
                        location,
                    )
                })
                .await
            {
                Ok(handle) => created.push(handle),
                Err(error) => {
                    for handle in created.iter().rev().copied() {
                        stack
                            .run(|stack| self.rollback_object_on_stack(stack, handle))
                            .await;
                    }
                    return Err(error);
                }
            }
        }
        Ok(created)
    }

    #[allow(clippy::too_many_lines)]
    async fn construct_enumeration_member(
        &mut self,
        stack: &mut FrameStack,
        class_id: ClassId,
        member: &str,
        arguments: &[Value],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        self.check_cancelled(location)?;
        let class = self
            .classes
            .class(class_id)
            .map_err(|error| self.object_error("enumeration construction", &error, location))?;
        if !class.is_enumeration()
            || !class
                .enumeration_members()
                .iter()
                .any(|descriptor| descriptor.name() == member)
        {
            return Err(self.invalid_state(
                "privileged enumeration construction targeted an undeclared member",
            ));
        }

        let reference = self
            .objects
            .allocate_with(&self.classes, class_id, |_| empty_double())
            .map_err(|error| self.object_error("enumeration construction", &error, location))?;
        let handle = ObjectHandle::new(reference.id().get());
        self.object_references.insert(handle, reference);
        let reference = self.object_references.get(&handle).ok_or_else(|| {
            unknown_object_error(
                &self.call_stack,
                handle,
                "enumeration construction",
                location,
            )
        })?;
        if let Err(error) = self.objects.begin_construction(reference) {
            let error = self.object_error("enumeration construction", &error, location);
            stack
                .run(|stack| self.rollback_object_on_stack(stack, handle))
                .await;
            return Err(error);
        }

        let execution = stack
            .run(|stack| {
                self.execute_constructor_body(
                    stack,
                    class_id,
                    handle,
                    arguments,
                    1,
                    AccessContext::class(class_id),
                    location,
                )
            })
            .await;
        let result = execution.and_then(|_| {
            let reference = self.object_references.get(&handle).ok_or_else(|| {
                unknown_object_error(
                    &self.call_stack,
                    handle,
                    "enumeration construction",
                    location,
                )
            })?;
            self.objects
                .finish_construction(reference)
                .map_err(|error| self.object_error("enumeration construction", &error, location))?;
            Ok(Value::Object(handle))
        });
        if result.is_err() {
            stack
                .run(|stack| self.rollback_object_on_stack(stack, handle))
                .await;
        }
        result
    }

    async fn invoke_superclass_constructor(
        &mut self,
        stack: &mut FrameStack,
        superclass_name: &str,
        object: &Value,
        arguments: &[Value],
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        self.check_cancelled(location)?;
        let current_class = context.caller_class().ok_or_else(|| {
            self.invalid_state("superclass constructor has no declaring-class context")
        })?;
        let superclass = self
            .classes
            .class(current_class)
            .map_err(|error| self.object_error("superclass construction", &error, location))?
            .superclass()
            .ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::Object {
                        operation: "superclass construction",
                        message: "current class has no direct user superclass".to_owned(),
                    },
                    location,
                )
            })?;
        let registered_name = self
            .classes
            .class(superclass)
            .map_err(|error| self.object_error("superclass construction", &error, location))?
            .name()
            .to_owned();
        if registered_name != superclass_name {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "superclass construction",
                    message: format!(
                        "`{superclass_name}` is not the current class's direct superclass `{registered_name}`"
                    ),
                },
                location,
            ));
        }
        let handle = object_handle(object).ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation: "superclass construction",
                    message: "constructor output local is not an object".to_owned(),
                },
                location,
            )
        })?;
        let reference = self.reference(handle, "superclass construction", location)?;
        let belongs_to_current_hierarchy = self
            .classes
            .is_subclass_of(reference.class_id(), current_class)
            .map_err(|error| self.object_error("superclass construction", &error, location))?;
        if !belongs_to_current_hierarchy {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "superclass construction",
                    message: "constructor output object is outside the current class hierarchy"
                        .to_owned(),
                },
                location,
            ));
        }
        let state = self
            .objects
            .state(reference)
            .map_err(|error| self.object_error("superclass construction", &error, location))?;
        if state != ConstructionState::Constructing {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "superclass construction",
                    message: "superclass constructor requires the current incomplete object"
                        .to_owned(),
                },
                location,
            ));
        }

        stack
            .run(|stack| {
                self.execute_constructor_body(
                    stack,
                    superclass,
                    handle,
                    arguments,
                    1,
                    AccessContext::class(current_class),
                    location,
                )
            })
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_constructor_body(
        &mut self,
        stack: &mut FrameStack,
        class_id: ClassId,
        handle: ObjectHandle,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let constructor = self
            .classes
            .resolve_constructor(class_id, caller_context)
            .map_err(|error| self.object_error("construction", &error, location))?;
        let constructor_name = constructor.map(|method| method.descriptor().name().to_owned());
        let returned = if let Some(constructor_name) = constructor_name {
            let code = self
                .class_code
                .get(&class_id)
                .and_then(|class| class.methods.get(&constructor_name))
                .cloned()
                .ok_or_else(|| {
                    self.error(
                        RuntimeErrorKind::Object {
                            operation: "construction",
                            message: "constructor body is missing".to_owned(),
                        },
                        location,
                    )
                })?;
            let output = code.constructor_output.ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::Object {
                        operation: "construction",
                        message: "constructor has no seeded object output".to_owned(),
                    },
                    location,
                )
            })?;
            let explicitly_constructs_parent = code.module.functions[code.function.get() as usize]
                .instructions
                .iter()
                .any(|instruction| {
                    matches!(
                        instruction.kind,
                        openmat_bytecode::InstructionKind::InvokeSuperclassConstructor { .. }
                    )
                });
            if !explicitly_constructs_parent {
                stack
                    .run(|stack| self.construct_implicit_parent(stack, class_id, handle, location))
                    .await?;
            }
            stack
                .run(|stack| {
                    self.execute_function(
                        stack,
                        &code.module,
                        code.function,
                        arguments,
                        Invocation {
                            requested_outputs,
                            seeded_local: Some((output, Value::Object(handle))),
                            copy_arguments: true,
                            access_context: AccessContext::class(class_id),
                            captures: Arc::new(BTreeMap::new()),
                            scope: InvocationScope::New,
                        },
                    )
                })
                .await?
        } else if arguments.is_empty() {
            stack
                .run(|stack| self.construct_implicit_parent(stack, class_id, handle, location))
                .await?;
            vec![Value::Object(handle)]
        } else {
            let class_name = self
                .classes
                .class(class_id)
                .map_or("<class>", |class| class.name());
            return Err(self.error(
                RuntimeErrorKind::InputArity {
                    function: class_name.to_owned(),
                    expected: 0,
                    actual: arguments.len(),
                },
                location,
            ));
        };
        if !matches!(returned.first(), Some(Value::Object(returned)) if *returned == handle) {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "construction",
                    message: "constructor did not return its allocated object".to_owned(),
                },
                location,
            ));
        }
        Ok(returned)
    }

    async fn construct_implicit_parent(
        &mut self,
        stack: &mut FrameStack,
        class_id: ClassId,
        handle: ObjectHandle,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let parent = self
            .classes
            .class(class_id)
            .map_err(|error| self.object_error("construction", &error, location))?
            .superclass();
        if let Some(parent) = parent {
            stack
                .run(|stack| {
                    self.execute_constructor_body(
                        stack,
                        parent,
                        handle,
                        &[],
                        1,
                        AccessContext::class(class_id),
                        location,
                    )
                })
                .await?;
        }
        Ok(())
    }

    fn class_metadata_value(
        &self,
        source: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let class_id = if let Ok(class_id) = self.object_value_class_id(source) {
            class_id
        } else {
            let name = self.function_name_text("class metadata", source, location)?;
            self.classes
                .class_id(&name)
                .map_err(|error| self.object_error("class metadata", &error, location))?
        };
        let class = self
            .classes
            .class(class_id)
            .map_err(|error| self.object_error("class metadata", &error, location))?;
        let name = self.char_row_value(class.name(), "class metadata name", location)?;
        let abstract_ = Value::Logical(class.is_abstract());
        let sealed = Value::Logical(class.is_sealed());

        let methods = self
            .classes
            .effective_methods(class_id)
            .map_err(|error| self.object_error("class metadata", &error, location))?;
        let method_count = u64::try_from(methods.len())
            .map_err(|_| self.invalid_state("class method metadata exceeds language limits"))?;
        let mut method_names = Vec::with_capacity(methods.len());
        let mut method_abstract = Vec::with_capacity(methods.len());
        let mut method_static = Vec::with_capacity(methods.len());
        let mut method_access = Vec::with_capacity(methods.len());
        for (_, method) in methods {
            method_names.push(self.char_row_value(
                method.name(),
                "method metadata name",
                location,
            )?);
            method_abstract.push(Value::Logical(method.is_abstract()));
            method_static.push(Value::Logical(method.kind() == MethodKind::Static));
            let access = match method.access() {
                ObjectAccess::Public => "public",
                ObjectAccess::Protected => "protected",
                ObjectAccess::Private => "private",
            };
            method_access.push(self.char_row_value(access, "method metadata access", location)?);
        }
        let method_list = StructArray::from_columns(
            Shape::new([method_count, 1])
                .map_err(|_| self.invalid_state("class method metadata shape is invalid"))?,
            ["Name", "Abstract", "Static", "Access"]
                .into_iter()
                .map(FieldName::new)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| self.invalid_state("class method metadata schema is invalid"))?,
            vec![method_names, method_abstract, method_static, method_access],
        )
        .map(Value::Struct)
        .map_err(|_| self.invalid_state("class method metadata value is invalid"))?;

        StructArray::from_columns(
            Shape::new([1, 1])
                .map_err(|_| self.invalid_state("class metadata shape is invalid"))?,
            ["Name", "Abstract", "Sealed", "MethodList"]
                .into_iter()
                .map(FieldName::new)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| self.invalid_state("class metadata schema is invalid"))?,
            vec![vec![name], vec![abstract_], vec![sealed], vec![method_list]],
        )
        .map(Value::Struct)
        .map_err(|_| self.invalid_state("class metadata value is invalid"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn call_intrinsic(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        intrinsic: IntrinsicFunction,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if intrinsic == IntrinsicFunction::OpenMatUi {
            return self.ui_intrinsic(arguments, requested_outputs, location);
        }
        if intrinsic == IntrinsicFunction::Ffi {
            self.expect_intrinsic_arity("ffi", arguments, 0, location)?;
            if requested_outputs > 1 {
                return Err(self.ffi_error("ffi returns at most one value", location));
            }
            let namespace = self.ffi_namespace(location)?;
            return if requested_outputs == 0 {
                Ok(Vec::new())
            } else {
                Ok(vec![namespace])
            };
        }
        if matches!(
            intrinsic,
            IntrinsicFunction::Feval
                | IntrinsicFunction::Eval
                | IntrinsicFunction::EvalC
                | IntrinsicFunction::EvalIn
                | IntrinsicFunction::AssignIn
                | IntrinsicFunction::Str2Func
                | IntrinsicFunction::Func2Str
                | IntrinsicFunction::Tic
                | IntrinsicFunction::Toc
                | IntrinsicFunction::LastErr
                | IntrinsicFunction::LastError
                | IntrinsicFunction::Throw
                | IntrinsicFunction::ThrowAsCaller
                | IntrinsicFunction::Rethrow
                | IntrinsicFunction::AddCause
                | IntrinsicFunction::GetReport
        ) {
            return stack
                .run(|stack| {
                    self.call_dynamic_intrinsic(
                        stack,
                        module,
                        intrinsic,
                        arguments,
                        requested_outputs,
                        caller_context,
                        location,
                    )
                })
                .await;
        }

        stack
            .run(|stack| self.call_object_intrinsic(stack, intrinsic, arguments, location))
            .await
    }

    #[allow(clippy::too_many_lines)]
    async fn call_object_intrinsic(
        &mut self,
        stack: &mut FrameStack,
        intrinsic: IntrinsicFunction,
        arguments: &[Value],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if intrinsic == IntrinsicFunction::Class && arguments.len() > 1 {
            return stack
                .run(|stack| self.construct_legacy_class(stack, arguments, location))
                .await
                .map(|value| vec![value]);
        }
        let expected = match intrinsic {
            IntrinsicFunction::Ffi | IntrinsicFunction::OpenMatUi => {
                unreachable!("host intrinsic returned above")
            }
            IntrinsicFunction::Class | IntrinsicFunction::IsObject => 1,
            IntrinsicFunction::Isa => 2,
            IntrinsicFunction::Feval
            | IntrinsicFunction::Eval
            | IntrinsicFunction::EvalC
            | IntrinsicFunction::EvalIn
            | IntrinsicFunction::AssignIn
            | IntrinsicFunction::Str2Func
            | IntrinsicFunction::Func2Str
            | IntrinsicFunction::Tic
            | IntrinsicFunction::Toc
            | IntrinsicFunction::LastErr
            | IntrinsicFunction::LastError
            | IntrinsicFunction::Throw
            | IntrinsicFunction::ThrowAsCaller
            | IntrinsicFunction::Rethrow
            | IntrinsicFunction::AddCause
            | IntrinsicFunction::GetReport => unreachable!("session intrinsics returned above"),
        };
        if arguments.len() != expected {
            return Err(self.error(
                RuntimeErrorKind::InputArity {
                    function: format!("{intrinsic:?}").to_ascii_lowercase(),
                    expected,
                    actual: arguments.len(),
                },
                location,
            ));
        }
        let value = match intrinsic {
            IntrinsicFunction::Ffi | IntrinsicFunction::OpenMatUi => {
                unreachable!("host intrinsic returned above")
            }
            IntrinsicFunction::Class => self.char_row_value(
                &self.value_class_name(&arguments[0]),
                "class name",
                location,
            )?,
            IntrinsicFunction::IsObject => Value::Logical(is_object_value(&arguments[0])),
            IntrinsicFunction::Isa => {
                let code_units = match &arguments[1] {
                    Value::String(value) => value
                        .as_scalar()
                        .filter(|value| !value.is_missing())
                        .map(|value| value.code_units().to_vec()),
                    Value::Array(ArrayData::Char(array))
                        if array.shape().dimensions() == [0, 0]
                            || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
                    {
                        Some(array.as_slice().iter().map(|value| value.get()).collect())
                    }
                    _ => None,
                }
                .ok_or_else(|| {
                    self.error(
                        RuntimeErrorKind::Object {
                            operation: "isa",
                            message: "second argument must be a char row vector or string scalar"
                                .to_owned(),
                        },
                        location,
                    )
                })?;
                let class_name = String::from_utf16(&code_units).map_err(|_| {
                    self.error(
                        RuntimeErrorKind::Object {
                            operation: "isa",
                            message: "second argument must contain valid UTF-16 class-name text"
                                .to_owned(),
                        },
                        location,
                    )
                })?;
                let matches = if let Ok(class_id) = self.object_value_class_id(&arguments[0]) {
                    if class_name == "handle" {
                        self.classes
                            .semantics(class_id)
                            .map_err(|error| self.object_error("isa", &error, location))?
                            == ClassSemantics::Handle
                    } else {
                        match self.classes.class_id(&class_name) {
                            Ok(ancestor) => self
                                .classes
                                .is_subclass_of(class_id, ancestor)
                                .map_err(|error| self.object_error("isa", &error, location))?,
                            Err(ObjectError::UnknownClassName { .. }) => false,
                            Err(error) => return Err(self.object_error("isa", &error, location)),
                        }
                    }
                } else {
                    self.value_class_name(&arguments[0]) == class_name
                };
                Value::Logical(matches)
            }
            IntrinsicFunction::Feval
            | IntrinsicFunction::Eval
            | IntrinsicFunction::EvalC
            | IntrinsicFunction::EvalIn
            | IntrinsicFunction::AssignIn
            | IntrinsicFunction::Str2Func
            | IntrinsicFunction::Func2Str
            | IntrinsicFunction::Tic
            | IntrinsicFunction::Toc
            | IntrinsicFunction::LastErr
            | IntrinsicFunction::LastError
            | IntrinsicFunction::Throw
            | IntrinsicFunction::ThrowAsCaller
            | IntrinsicFunction::Rethrow
            | IntrinsicFunction::AddCause
            | IntrinsicFunction::GetReport => unreachable!("session intrinsics returned above"),
        };
        Ok(vec![value])
    }

    async fn construct_legacy_class(
        &mut self,
        stack: &mut FrameStack,
        arguments: &[Value],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if arguments.len() < 2 {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "class",
                    message: "old-style class construction expects a struct, class name, and optional parent objects"
                        .to_owned(),
                },
                location,
            ));
        }
        let Value::Struct(structure) = &arguments[0] else {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "class",
                    message: "old-style class construction requires a struct as its first input"
                        .to_owned(),
                },
                location,
            ));
        };
        let class_name = self.function_name_text("class", &arguments[1], location)?;
        if !is_matlab_identifier(&class_name) {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "class",
                    message: format!("`{class_name}` is not a valid old-style class name"),
                },
                location,
            ));
        }
        let constructor_source = self.legacy_constructor_source(&class_name, location)?;
        let mut superclasses = Vec::new();
        let mut parent_values = BTreeMap::new();
        let mut seen_superclasses = BTreeSet::new();
        for parent in &arguments[2..] {
            let (superclass, values) = self.legacy_parent(parent, structure, location)?;
            if !seen_superclasses.insert(superclass) {
                return Err(self.error(
                    RuntimeErrorKind::Object {
                        operation: "class",
                        message:
                            "an old-style class cannot list the same direct parent more than once"
                                .to_owned(),
                    },
                    location,
                ));
            }
            superclasses.push(superclass);
            for (key, value) in values {
                if parent_values.insert(key, value).is_some() {
                    return Err(self.error(
                        RuntimeErrorKind::Object {
                            operation: "class",
                            message: "old-style parent objects contain the same inherited storage slot through more than one inheritance path"
                                .to_owned(),
                        },
                        location,
                    ));
                }
            }
        }
        let own_fields = structure
            .field_names()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let class_id = self.register_legacy_class(
            &class_name,
            constructor_source,
            &own_fields,
            &superclasses,
            location,
        )?;
        stack
            .run(|stack| {
                self.allocate_legacy_values(stack, class_id, structure, &parent_values, location)
            })
            .await
    }

    fn legacy_constructor_source(
        &self,
        class_name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<PathBuf> {
        let source = self.caller_file_source(location).ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation: "class",
                    message: "old-style class construction is allowed only inside @Class/Class.m"
                        .to_owned(),
                },
                location,
            )
        })?;
        let source = PathBuf::from(source);
        let stem = source.file_stem().and_then(std::ffi::OsStr::to_str);
        let directory = source
            .parent()
            .and_then(Path::file_name)
            .and_then(std::ffi::OsStr::to_str);
        let expected_directory = format!("@{class_name}");
        let matches = stem.is_some_and(|stem| platform_name_eq(stem, class_name))
            && directory.is_some_and(|directory| platform_name_eq(directory, &expected_directory));
        if !matches {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "class",
                    message: format!(
                        "old-style class `{class_name}` must be created by @{class_name}/{class_name}.m"
                    ),
                },
                location,
            ));
        }
        Ok(source)
    }

    fn legacy_parent(
        &mut self,
        parent: &Value,
        structure: &StructArray,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(ClassId, BTreeMap<PropertyKey, Value>)> {
        if structure.numel() != 1 {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "class",
                    message: "old-style inheritance currently requires a scalar struct".to_owned(),
                },
                location,
            ));
        }
        let handle = object_handle(parent).ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation: "class",
                    message: "old-style class parent must be a scalar old-style object".to_owned(),
                },
                location,
            )
        })?;
        let reference = self.reference(handle, "old-style parent", location)?;
        let class_id = reference.class_id();
        if !self.legacy_classes.contains_key(&class_id) {
            return Err(self.error(
                RuntimeErrorKind::Object {
                    operation: "class",
                    message: "old-style class parent must itself use tagged-struct semantics"
                        .to_owned(),
                },
                location,
            ));
        }
        let slots = self
            .classes
            .stored_properties(class_id)
            .map_err(|error| self.object_error("class", &error, location))?
            .into_iter()
            .map(|(key, _)| {
                self.objects
                    .slot(reference, &key)
                    .cloned()
                    .map(|value| (key, value))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| self.object_error("class", &error, location))?;
        let mut values = BTreeMap::new();
        for (key, value) in slots {
            values.insert(key, self.language_copy(&value)?);
        }
        Ok((class_id, values))
    }

    fn register_legacy_class(
        &mut self,
        class_name: &str,
        constructor_source: PathBuf,
        own_fields: &[String],
        superclasses: &[ClassId],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ClassId> {
        match self.classes.class_id(class_name) {
            Ok(class_id) => {
                let Some(metadata) = self.legacy_classes.get(&class_id) else {
                    return Err(self.error(
                        RuntimeErrorKind::Object {
                            operation: "class",
                            message: format!(
                                "class `{class_name}` is already registered by another object model"
                            ),
                        },
                        location,
                    ));
                };
                if metadata.constructor_source != constructor_source
                    || metadata.own_fields != own_fields
                    || metadata.superclasses != superclasses
                {
                    return Err(self.error(
                        RuntimeErrorKind::Object {
                            operation: "class",
                            message: format!(
                                "old-style class `{class_name}` changed its constructor, field schema, or superclasses during this session"
                            ),
                        },
                        location,
                    ));
                }
                return Ok(class_id);
            }
            Err(ObjectError::UnknownClassName { .. }) => {}
            Err(error) => return Err(self.object_error("class", &error, location)),
        }

        let mut definition = ClassDefinition::value(class_name.to_owned());
        if let Some(superclass) = superclasses.first().copied() {
            definition = definition.with_superclass(superclass);
        }
        for superclass in superclasses.iter().copied().skip(1) {
            definition = definition.with_additional_superclass(superclass);
        }
        for field in own_fields {
            definition = definition.with_property(
                PropertyDescriptor::stored(field.clone(), None)
                    .with_get_access(ObjectAccess::Private)
                    .with_set_access(ObjectAccess::Private),
            );
        }
        let class_id = self
            .classes
            .register_class(definition)
            .map_err(|error| self.object_error("class", &error, location))?;
        self.class_handles
            .insert(ClassHandle::new(class_id.get()), class_id);
        self.class_code.entry(class_id).or_default();
        self.legacy_classes.insert(
            class_id,
            LegacyClassMetadata {
                constructor_source,
                own_fields: own_fields.to_vec(),
                superclasses: superclasses.to_vec(),
            },
        );
        Ok(class_id)
    }

    async fn allocate_legacy_values(
        &mut self,
        stack: &mut FrameStack,
        class_id: ClassId,
        structure: &StructArray,
        parent_values: &BTreeMap<PropertyKey, Value>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let slots = self
            .classes
            .stored_properties(class_id)
            .map_err(|error| self.object_error("class", &error, location))?
            .into_iter()
            .map(|(key, property)| (key, property.name().to_owned()))
            .collect::<Vec<_>>();
        let own_slots = slots
            .iter()
            .filter(|(key, _)| key.declaring_class() == class_id)
            .map(|(key, name)| (name.clone(), key.clone()))
            .collect::<BTreeMap<_, _>>();
        let length = usize::try_from(structure.numel())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(length)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for offset in 0..length {
            let mut values = parent_values.clone();
            for (field, name) in structure.field_names().iter().enumerate() {
                let value = structure.value_at(field, offset).ok_or_else(|| {
                    self.invalid_state("old-style struct field storage is incomplete")
                })?;
                let key = own_slots.get(name.as_ref()).ok_or_else(|| {
                    self.invalid_state("old-style class field has no registered slot")
                })?;
                values.insert(key.clone(), self.language_copy(value)?);
            }
            let reference = self
                .objects
                .allocate_with(&self.classes, class_id, |_| Ok(Value::Nothing))
                .map_err(|error| self.object_error("class", &error, location))?;
            let initialized = (|| -> RuntimeResult<()> {
                self.objects
                    .begin_construction(&reference)
                    .map_err(|error| self.object_error("class", &error, location))?;
                for (key, _) in &slots {
                    let value = values.get(key).cloned().ok_or_else(|| {
                        self.invalid_state("old-style class slot has no source field")
                    })?;
                    match self.objects.slot_mut(&reference, key) {
                        Ok(slot) => *slot = value,
                        Err(error) => {
                            return Err(self.object_error("class", &error, location));
                        }
                    }
                }
                self.objects
                    .finish_construction(&reference)
                    .map_err(|error| self.object_error("class", &error, location))
            })();
            if let Err(error) = initialized {
                let _ = self.objects.fail_construction(&reference);
                let _ = self.objects.discard_incomplete(&reference);
                for handle in handles {
                    stack
                        .run(|stack| self.rollback_object_on_stack(stack, handle))
                        .await;
                }
                return Err(error);
            }
            let handle = ObjectHandle::new(reference.id().get());
            self.object_references.insert(handle, reference);
            handles.push(handle);
        }
        if let [handle] = handles.as_slice() {
            return Ok(Value::Object(*handle));
        }
        let class_handle = ClassHandle::new(class_id.get());
        match ObjectArray::from_vec(class_handle, structure.shape().clone(), handles) {
            Ok(array) => Ok(Value::ObjectArray(array)),
            Err(_) => Err(self.invalid_state("old-style object-array shape is invalid")),
        }
    }

    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_arguments)]
    async fn call_dynamic_intrinsic(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        intrinsic: IntrinsicFunction,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        match intrinsic {
            IntrinsicFunction::Feval => {
                if arguments.is_empty() {
                    return Err(self.error(
                        RuntimeErrorKind::InputArity {
                            function: "feval".to_owned(),
                            expected: 1,
                            actual: 0,
                        },
                        location,
                    ));
                }
                match &arguments[0] {
                    callable @ Value::Function(_) => {
                        stack
                            .run(|stack| {
                                self.call_value(
                                    stack,
                                    module,
                                    callable,
                                    &arguments[1..],
                                    requested_outputs,
                                    caller_context,
                                    location,
                                )
                            })
                            .await
                    }
                    value => {
                        let name = self.function_name_text("feval", value, location)?;
                        stack
                            .run(|stack| {
                                self.call_named_callable(
                                    stack,
                                    module,
                                    &name,
                                    &arguments[1..],
                                    requested_outputs,
                                    caller_context,
                                    location,
                                    None,
                                )
                            })
                            .await
                    }
                }
            }
            IntrinsicFunction::Eval => {
                stack
                    .run(|stack| {
                        self.eval_intrinsic(
                            stack,
                            module,
                            arguments,
                            requested_outputs,
                            caller_context,
                            location,
                        )
                    })
                    .await
            }
            IntrinsicFunction::EvalC => {
                stack
                    .run(|stack| {
                        self.evalc_intrinsic(
                            stack,
                            module,
                            arguments,
                            requested_outputs,
                            caller_context,
                            location,
                        )
                    })
                    .await
            }
            IntrinsicFunction::EvalIn => {
                stack
                    .run(|stack| {
                        self.evalin_intrinsic(
                            stack,
                            module,
                            arguments,
                            requested_outputs,
                            caller_context,
                            location,
                        )
                    })
                    .await
            }
            IntrinsicFunction::AssignIn => {
                self.assignin_intrinsic(arguments, requested_outputs, location)
            }
            IntrinsicFunction::Str2Func => {
                self.expect_intrinsic_arity("str2func", arguments, 1, location)?;
                let name = self.function_name_text("str2func", &arguments[0], location)?;
                if name.is_empty() {
                    return Err(self.error(
                        RuntimeErrorKind::UnknownFunctionHandleTarget { name },
                        location,
                    ));
                }
                let handle = self.register_named_function_handle(&name, location)?;
                Ok(vec![Value::Function(FunctionHandle::Bytecode(handle))])
            }
            IntrinsicFunction::Func2Str => {
                self.expect_intrinsic_arity("func2str", arguments, 1, location)?;
                let Value::Function(handle) = &arguments[0] else {
                    return Err(self.error(
                        RuntimeErrorKind::NotCallable {
                            actual: arguments[0].kind(),
                        },
                        location,
                    ));
                };
                let text = self.function_handle_text(module, *handle, location)?;
                self.char_row_value(&text, "function name", location)
                    .map(|value| vec![value])
            }
            IntrinsicFunction::Tic => self.tic_intrinsic(arguments, requested_outputs, location),
            IntrinsicFunction::Toc => self.toc_intrinsic(arguments, requested_outputs, location),
            IntrinsicFunction::LastErr => {
                self.lasterr_intrinsic(arguments, requested_outputs, location)
            }
            IntrinsicFunction::LastError => {
                self.lasterror_intrinsic(arguments, requested_outputs, location)
            }
            IntrinsicFunction::Throw => {
                self.throw_intrinsic(arguments, requested_outputs, false, location)
            }
            IntrinsicFunction::ThrowAsCaller => {
                self.throw_intrinsic(arguments, requested_outputs, true, location)
            }
            IntrinsicFunction::Rethrow => {
                self.rethrow_intrinsic(arguments, requested_outputs, location)
            }
            IntrinsicFunction::AddCause => {
                self.add_cause_intrinsic(arguments, requested_outputs, location)
            }
            IntrinsicFunction::GetReport => {
                self.get_report_intrinsic(arguments, requested_outputs, location)
            }
            IntrinsicFunction::Class | IntrinsicFunction::Isa | IntrinsicFunction::IsObject => {
                unreachable!("non-session intrinsic reached session dispatch")
            }
            IntrinsicFunction::Ffi | IntrinsicFunction::OpenMatUi => {
                unreachable!("host intrinsic returned above")
            }
        }
    }

    async fn eval_intrinsic(
        &mut self,
        stack: &mut FrameStack,
        caller_module: &Arc<BytecodeModule>,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let (source, catch_source) = match arguments {
            [source] => (source, None),
            [source, catch_source] => (source, Some(catch_source)),
            _ => {
                return Err(self.error(
                    RuntimeErrorKind::InputArity {
                        function: "eval".to_owned(),
                        expected: if arguments.is_empty() { 1 } else { 2 },
                        actual: arguments.len(),
                    },
                    location,
                ));
            }
        };
        stack
            .run(|stack| {
                self.eval_in_scope(
                    stack,
                    caller_module,
                    source,
                    catch_source,
                    requested_outputs,
                    caller_context,
                    ScopeTarget::Current,
                    ScopeTarget::Current,
                    "eval",
                    location,
                )
            })
            .await
    }

    async fn evalc_intrinsic(
        &mut self,
        stack: &mut FrameStack,
        caller_module: &Arc<BytecodeModule>,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let (source, catch_source) = match arguments {
            [source] => (source, None),
            [source, catch_source] => (source, Some(catch_source)),
            _ => {
                return Err(self.error(
                    RuntimeErrorKind::InputArity {
                        function: "evalc".to_owned(),
                        expected: if arguments.is_empty() { 1 } else { 2 },
                        actual: arguments.len(),
                    },
                    location,
                ));
            }
        };
        let initial_format = self.display_format;
        let captured_events = Arc::new(Mutex::new(Vec::new()));
        let output = std::mem::replace(
            &mut self.output,
            Box::new(EvalCaptureOutput {
                events: Arc::clone(&captured_events),
            }),
        );
        let evaluated = stack
            .run(|stack| {
                self.eval_in_scope(
                    stack,
                    caller_module,
                    source,
                    catch_source,
                    requested_outputs.saturating_sub(1),
                    caller_context,
                    ScopeTarget::Current,
                    ScopeTarget::Current,
                    "evalc",
                    location,
                )
            })
            .await;
        self.output = output;
        let events = std::mem::take(
            &mut *captured_events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let (captured, passthrough) = self.render_evalc_events(events, initial_format);
        for event in passthrough {
            self.output.emit(event).map_err(|error| {
                self.intrinsic_builtin_error(
                    "evalc",
                    BuiltinErrorCategory::Output,
                    None,
                    error.message,
                    location,
                )
            })?;
        }
        let mut evaluated = evaluated?;
        if requested_outputs == 0 {
            return Ok(Vec::new());
        }
        let mut returned = Vec::new();
        returned
            .try_reserve_exact(requested_outputs)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        returned.push(self.char_row_value(&captured, "evalc output", location)?);
        returned.append(&mut evaluated);
        Ok(returned)
    }

    fn render_evalc_events(
        &self,
        events: Vec<OutputEvent>,
        initial_format: DisplayFormat,
    ) -> (String, Vec<OutputEvent>) {
        let mut captured = String::new();
        let mut passthrough = Vec::new();
        let mut format = initial_format;
        for event in events {
            match event {
                OutputEvent::Display(value) => {
                    captured.push_str(&crate::display::format_display_value_with(
                        &value,
                        self,
                        format.numeric,
                    ));
                    captured.push('\n');
                }
                OutputEvent::NamedDisplay { name, value } => {
                    let separator = match format.line_spacing {
                        crate::LineSpacing::Compact => "\n",
                        crate::LineSpacing::Loose => "\n\n",
                    };
                    if format.line_spacing == crate::LineSpacing::Loose {
                        captured.push('\n');
                    }
                    captured.push_str(&name);
                    captured.push_str(" =");
                    captured.push_str(separator);
                    captured.push_str(&crate::display::format_display_value_with(
                        &value,
                        self,
                        format.numeric,
                    ));
                    captured.push_str(separator);
                }
                OutputEvent::CommandText(text) => captured.push_str(&text),
                OutputEvent::DisplayFormatChanged(next) => {
                    format = next;
                    passthrough.push(OutputEvent::DisplayFormatChanged(next));
                }
                event @ (OutputEvent::GraphicsNotice(_)
                | OutputEvent::CommandWindowClear
                | OutputEvent::UiDisplay(_)) => {
                    passthrough.push(event);
                }
            }
        }
        (captured, passthrough)
    }

    async fn evalin_intrinsic(
        &mut self,
        stack: &mut FrameStack,
        caller_module: &Arc<BytecodeModule>,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let (workspace, source, catch_source) = match arguments {
            [workspace, source] => (workspace, source, None),
            [workspace, source, catch_source] => (workspace, source, Some(catch_source)),
            _ => {
                return Err(self.error(
                    RuntimeErrorKind::InputArity {
                        function: "evalin".to_owned(),
                        expected: if arguments.len() < 2 { 2 } else { 3 },
                        actual: arguments.len(),
                    },
                    location,
                ));
            }
        };
        let target = self.named_scope_target("evalin", workspace, location)?;
        stack
            .run(|stack| {
                self.eval_in_scope(
                    stack,
                    caller_module,
                    source,
                    catch_source,
                    requested_outputs,
                    caller_context,
                    target,
                    ScopeTarget::Current,
                    "evalin",
                    location,
                )
            })
            .await
    }

    fn assignin_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        self.expect_intrinsic_arity("assignin", arguments, 3, location)?;
        if requested_outputs != 0 {
            return Err(self.intrinsic_builtin_error(
                "assignin",
                BuiltinErrorCategory::ArgumentCount,
                Some("OpenMat:assignin:TooManyOutputs"),
                "assignin does not return a value",
                location,
            ));
        }
        let target = self.named_scope_target("assignin", &arguments[0], location)?;
        let name =
            self.metaprogramming_text("assignin", "variable name", &arguments[1], location)?;
        if !crate::eval::is_variable_name(&name) {
            return Err(self.intrinsic_builtin_error(
                "assignin",
                BuiltinErrorCategory::Domain,
                Some("OpenMat:assignin:InvalidVariable"),
                "assignin variable name must be a valid non-keyword identifier",
                location,
            ));
        }
        let (address, _) = self.scope_context(target, location)?;
        let value = self.language_copy(&arguments[2])?;
        self.pending_scope_bindings
            .try_reserve(1)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        self.pending_scope_bindings.push(ScopeBindingChange {
            target: address,
            name,
            value,
        });
        Ok(Vec::new())
    }

    fn named_scope_target(
        &self,
        operation: &'static str,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ScopeTarget> {
        let workspace = self.metaprogramming_text(operation, "workspace", value, location)?;
        match workspace.as_str() {
            "base" => Ok(ScopeTarget::Base),
            "caller" => Ok(ScopeTarget::Caller),
            _ => Err(self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Domain,
                Some(match operation {
                    "assignin" => "OpenMat:assignin:InvalidWorkspace",
                    _ => "OpenMat:evalin:InvalidWorkspace",
                }),
                "workspace must be 'base' or 'caller'",
                location,
            )),
        }
    }

    fn metaprogramming_text(
        &self,
        operation: &'static str,
        role: &'static str,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<String> {
        let code_units = match value {
            Value::String(value) => value
                .as_scalar()
                .filter(|value| !value.is_missing())
                .map(|value| value.code_units().to_vec()),
            Value::Array(ArrayData::Char(array))
                if array.shape().dimensions() == [0, 0]
                    || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
            {
                Some(array.as_slice().iter().map(|value| value.get()).collect())
            }
            _ => None,
        }
        .ok_or_else(|| {
            self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Type,
                Some(match operation {
                    "assignin" => "OpenMat:assignin:InvalidText",
                    _ => "OpenMat:evalin:InvalidText",
                }),
                format!("{role} must be a char row vector or string scalar"),
                location,
            )
        })?;
        String::from_utf16(&code_units).map_err(|_| {
            self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Type,
                Some(match operation {
                    "assignin" => "OpenMat:assignin:InvalidText",
                    _ => "OpenMat:evalin:InvalidText",
                }),
                format!("{role} must contain valid UTF-16 text"),
                location,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn eval_in_scope(
        &mut self,
        stack: &mut FrameStack,
        caller_module: &Arc<BytecodeModule>,
        source_value: &Value,
        catch_source: Option<&Value>,
        requested_outputs: usize,
        caller_context: AccessContext,
        target: ScopeTarget,
        catch_target: ScopeTarget,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let (scope_address, scope_context) = self.scope_context(target, location)?;
        let catch_scope = catch_source
            .map(|_| self.scope_context(catch_target, location))
            .transpose()?;
        let primary = stack
            .run(|stack| {
                self.eval_in_resolved_scope(
                    stack,
                    caller_module,
                    source_value,
                    requested_outputs,
                    caller_context,
                    scope_address,
                    Arc::clone(&scope_context),
                    operation,
                    location,
                )
            })
            .await;
        if let Err(error) = &primary {
            self.record_last_error(error);
        }
        match (primary, catch_source) {
            (result @ Ok(_), _) | (result @ Err(_), None) => result,
            (Err(_), Some(catch_source)) => {
                self.pending_exception = None;
                let (catch_address, catch_context) = catch_scope.ok_or_else(|| {
                    self.invalid_state("dynamic catch source has no prepared workspace")
                })?;
                let (_, catch_context) = self.context_with_pending(catch_address, &catch_context);
                stack
                    .run(|stack| {
                        self.eval_in_resolved_scope(
                            stack,
                            caller_module,
                            catch_source,
                            requested_outputs,
                            caller_context,
                            catch_address,
                            catch_context,
                            operation,
                            location,
                        )
                    })
                    .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn eval_in_resolved_scope(
        &mut self,
        stack: &mut FrameStack,
        caller_module: &Arc<BytecodeModule>,
        source_value: &Value,
        requested_outputs: usize,
        caller_context: AccessContext,
        scope_address: ScopeAddress,
        scope_context: Arc<ScopeContext>,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let source = self.dynamic_source_text(operation, source_value, location)?;
        let source_id = match self.module_loader.allocate_source_id() {
            Ok(source_id) => source_id,
            Err(error) => return Err(self.invalid_state(&error.message)),
        };
        self.register_source_metadata(source_id, "<eval>", &source);
        let mut eval_workspace = scope_context.workspace.clone();
        let original_workspace = eval_workspace.clone();
        let program = {
            let occupied_names = eval_workspace.iter().map(|(name, _)| name).collect();
            crate::eval::compile(
                source_id,
                &source,
                requested_outputs,
                &occupied_names,
                &scope_context.imports,
            )
            .map_err(|error| {
                self.intrinsic_builtin_error(
                    operation,
                    BuiltinErrorCategory::Other,
                    Some(error.identifier),
                    error.message,
                    location,
                )
            })?
        };
        let mut returned = Vec::new();
        returned
            .try_reserve_exact(program.output_names.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        let seeded_functions =
            self.seed_eval_functions(caller_module, &mut eval_workspace, location)?;
        let eval_module = Arc::new(program.module);
        self.enter_dynamic_workspace(eval_workspace, location)?;
        let suspended_changes = self.suspend_scope_changes(scope_address);
        let execution = stack
            .run(|stack| {
                self.execute_function(
                    stack,
                    &eval_module,
                    eval_module.entry,
                    &[],
                    Invocation {
                        requested_outputs: 0,
                        seeded_local: None,
                        copy_arguments: false,
                        access_context: caller_context,
                        captures: Arc::new(BTreeMap::new()),
                        scope: InvocationScope::Reuse(scope_context),
                    },
                )
            })
            .await;
        let mut evaluated_workspace = self.leave_dynamic_workspace()?;
        self.builtin_workspace = None;
        self.builtin_scope_context = None;
        self.restore_scope_changes(suspended_changes, location)?;

        if execution.is_ok() {
            for name in &program.output_names {
                if let Some(value) = evaluated_workspace.get(name).cloned() {
                    returned.push(value);
                }
            }
        }
        for name in &program.output_names {
            evaluated_workspace.remove(name);
        }
        for (name, injected) in seeded_functions {
            if evaluated_workspace.get(&name) == Some(&injected) {
                evaluated_workspace.remove(&name);
            }
        }
        self.stage_eval_scope_delta(
            scope_address,
            &original_workspace,
            &evaluated_workspace,
            location,
        )?;
        execution?;
        if returned.len() < requested_outputs {
            return Err(self.error(
                RuntimeErrorKind::MissingOutputs {
                    requested: requested_outputs,
                    returned: returned.len(),
                },
                location,
            ));
        }
        Ok(returned)
    }

    fn dynamic_source_text(
        &self,
        operation: &'static str,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<String> {
        let code_units = match value {
            Value::String(value) => value
                .as_scalar()
                .filter(|value| !value.is_missing())
                .map(|value| value.code_units().to_vec()),
            Value::Array(ArrayData::Char(array))
                if array.shape().dimensions() == [0, 0]
                    || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
            {
                Some(array.as_slice().iter().map(|value| value.get()).collect())
            }
            _ => None,
        }
        .ok_or_else(|| {
            self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Type,
                Some(match operation {
                    "evalin" => "OpenMat:evalin:InvalidText",
                    _ => "OpenMat:eval:InvalidText",
                }),
                "dynamic source must be a char row vector or string scalar",
                location,
            )
        })?;
        String::from_utf16(&code_units).map_err(|_| {
            self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Type,
                Some(match operation {
                    "evalin" => "OpenMat:evalin:InvalidText",
                    _ => "OpenMat:eval:InvalidText",
                }),
                "dynamic source must contain valid UTF-16 text",
                location,
            )
        })
    }

    fn scope_context(
        &self,
        target: ScopeTarget,
        _location: Option<SourceLocation>,
    ) -> RuntimeResult<(ScopeAddress, Arc<ScopeContext>)> {
        let selected = match target {
            ScopeTarget::Base => None,
            ScopeTarget::Current | ScopeTarget::Caller => {
                let current = self.builtin_scope_context.clone().ok_or_else(|| {
                    self.invalid_state("dynamic scope operation has no calling workspace")
                })?;
                if target == ScopeTarget::Current {
                    return Ok(self.context_with_pending(current.address, &current));
                }
                current.caller.clone()
            }
        };
        if let Some(scope) = selected {
            return Ok(self.context_with_pending(scope.address, &scope));
        }
        let scope = Arc::new(ScopeContext {
            address: ScopeAddress::Base,
            workspace: self.base_workspace().clone(),
            imports: self.base_imports.clone(),
            caller: None,
        });
        Ok((ScopeAddress::Base, scope))
    }

    fn context_with_pending(
        &self,
        address: ScopeAddress,
        scope: &Arc<ScopeContext>,
    ) -> (ScopeAddress, Arc<ScopeContext>) {
        let mut workspace = scope.workspace.clone();
        for change in self
            .pending_scope_bindings
            .iter()
            .filter(|change| change.target == address)
        {
            if matches!(change.value, Value::Nothing) {
                workspace.remove(&change.name);
            } else {
                workspace.insert(change.name.clone(), change.value.clone());
            }
        }
        (
            address,
            Arc::new(ScopeContext {
                address: scope.address,
                workspace,
                imports: scope.imports.clone(),
                caller: scope.caller.clone(),
            }),
        )
    }

    fn suspend_scope_changes(&mut self, address: ScopeAddress) -> Vec<ScopeBindingChange> {
        let mut suspended = Vec::new();
        let mut retained = Vec::new();
        for change in std::mem::take(&mut self.pending_scope_bindings) {
            if change.target == address {
                suspended.push(change);
            } else {
                retained.push(change);
            }
        }
        self.pending_scope_bindings = retained;
        suspended
    }

    fn restore_scope_changes(
        &mut self,
        mut suspended: Vec<ScopeBindingChange>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        suspended
            .try_reserve(self.pending_scope_bindings.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        suspended.append(&mut self.pending_scope_bindings);
        self.pending_scope_bindings = suspended;
        Ok(())
    }

    fn seed_eval_functions(
        &mut self,
        module: &Arc<BytecodeModule>,
        workspace: &mut Workspace,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<(String, Value)>> {
        let nested = module
            .functions
            .iter()
            .flat_map(|function| &function.instructions)
            .filter_map(|instruction| match instruction.kind {
                InstructionKind::MakeClosure { function, .. }
                | InstructionKind::MakeSharedClosure { function, .. } => Some(function.get()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let mut seeded = Vec::new();
        for (raw, function) in (0_u32..).zip(&module.functions) {
            let function_id = FunctionId::new(raw);
            if function_id == module.entry
                || nested.contains(&raw)
                || function.name.is_empty()
                || workspace.contains(&function.name)
            {
                continue;
            }
            let value = self.constant_value(module, &Constant::Function(function_id), location)?;
            workspace.insert(function.name.clone(), value.clone());
            seeded.push((function.name.clone(), value));
        }
        Ok(seeded)
    }

    fn stage_eval_scope_delta(
        &mut self,
        target: ScopeAddress,
        before: &Workspace,
        after: &Workspace,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let mut changes = Vec::new();
        changes
            .try_reserve(before.len().saturating_add(after.len()))
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        for (name, value) in after.iter() {
            if before.get(name) != Some(value) {
                changes.push(ScopeBindingChange {
                    target,
                    name: name.to_owned(),
                    value: value.clone(),
                });
            }
        }
        for (name, _) in before.iter() {
            if !after.contains(name) {
                changes.push(ScopeBindingChange {
                    target,
                    name: name.to_owned(),
                    value: Value::Nothing,
                });
            }
        }
        self.pending_scope_bindings
            .try_reserve(changes.len())
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        self.pending_scope_bindings.append(&mut changes);
        Ok(())
    }

    fn lasterr_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if arguments.len() > 2 {
            return Err(self.intrinsic_builtin_error(
                "lasterr",
                BuiltinErrorCategory::ArgumentCount,
                Some("OpenMat:lasterr:InputArity"),
                "lasterr accepts at most a message and identifier",
                location,
            ));
        }
        if requested_outputs > 2 {
            return Err(self.intrinsic_builtin_error(
                "lasterr",
                BuiltinErrorCategory::ArgumentCount,
                Some("OpenMat:lasterr:OutputArity"),
                "lasterr returns at most a message and identifier",
                location,
            ));
        }
        let previous = self.last_error.clone().unwrap_or_default();
        let returned = if arguments.is_empty() {
            previous.clone()
        } else {
            let message =
                self.last_error_input_text("lasterr", "message", &arguments[0], location)?;
            let identifier = arguments
                .get(1)
                .map(|value| self.last_error_input_text("lasterr", "identifier", value, location))
                .transpose()?
                .unwrap_or_default();
            self.last_error = Some(LastErrorState {
                message,
                identifier,
                stack: Vec::new(),
            });
            previous
        };
        match requested_outputs {
            0 => Ok(Vec::new()),
            1 => Ok(vec![self.last_error_text_value(
                &returned.message,
                "last error message",
                location,
            )?]),
            2 => Ok(vec![
                self.last_error_text_value(&returned.message, "last error message", location)?,
                self.last_error_text_value(
                    &returned.identifier,
                    "last error identifier",
                    location,
                )?,
            ]),
            _ => unreachable!("lasterr output arity was validated"),
        }
    }

    fn lasterror_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if arguments.len() > 1 {
            return Err(self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::ArgumentCount,
                Some("OpenMat:lasterror:InputArity"),
                "lasterror accepts at most one state struct",
                location,
            ));
        }
        if requested_outputs > 1 {
            return Err(self.intrinsic_builtin_error(
                "lasterror",
                BuiltinErrorCategory::ArgumentCount,
                Some("OpenMat:lasterror:OutputArity"),
                "lasterror returns at most one state struct",
                location,
            ));
        }
        let previous = self.last_error.clone().unwrap_or_default();
        let returned = if let Some(state) = arguments.first() {
            let replacement = self.last_error_state_from_value(state, location)?;
            self.last_error = Some(replacement);
            previous
        } else {
            previous
        };
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![self.last_error_struct_value(&returned, location)?])
        }
    }

    fn call_exception_method(
        &mut self,
        name: &str,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        match name {
            "throw" => self.throw_intrinsic(arguments, requested_outputs, false, location),
            "throwAsCaller" => self.throw_intrinsic(arguments, requested_outputs, true, location),
            "addCause" => self.add_cause_intrinsic(arguments, requested_outputs, location),
            "getReport" => self.get_report_intrinsic(arguments, requested_outputs, location),
            _ => Err(self.invalid_state("unknown runtime MException method escaped dispatch")),
        }
    }

    fn exception_handle(
        &self,
        value: &Value,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ObjectHandle> {
        let Value::Object(handle) = value else {
            return Err(self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Type,
                Some("MATLAB:MException:invalidInput"),
                format!("{operation} requires a scalar MException"),
                location,
            ));
        };
        let reference = self.reference(*handle, operation, location)?;
        if Some(reference.class_id()) != self.exception_class {
            return Err(self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::Type,
                Some("MATLAB:MException:invalidInput"),
                format!("{operation} requires a scalar MException"),
                location,
            ));
        }
        Ok(*handle)
    }

    fn exception_property(
        &self,
        handle: ObjectHandle,
        name: &'static str,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let reference = self.reference(handle, operation, location)?;
        let class_id = self
            .exception_class
            .ok_or_else(|| self.invalid_state("MException class is not installed"))?;
        let resolution = self
            .classes
            .resolve_property(
                class_id,
                name,
                PropertyAccess::Get,
                AccessContext::external(),
            )
            .map_err(|error| self.object_error(operation, &error, location))?;
        let PropertyResolution::Stored { key, .. } = resolution else {
            return Err(self.invalid_state("MException runtime property is not stored"));
        };
        self.objects
            .slot(reference, &key)
            .cloned()
            .map_err(|error| self.object_error(operation, &error, location))
    }

    fn set_exception_property(
        &mut self,
        handle: ObjectHandle,
        name: &'static str,
        value: Value,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let Some(reference) = self.object_references.get(&handle) else {
            return Err(unknown_object_error(
                &self.call_stack,
                handle,
                operation,
                location,
            ));
        };
        let class_id = self
            .exception_class
            .ok_or_else(|| self.invalid_state("MException class is not installed"))?;
        let resolution = self
            .classes
            .resolve_property(
                class_id,
                name,
                PropertyAccess::Get,
                AccessContext::external(),
            )
            .map_err(|error| self.object_error(operation, &error, location))?;
        let PropertyResolution::Stored { key, .. } = resolution else {
            return Err(self.invalid_state("MException runtime property is not stored"));
        };
        match self.objects.slot_mut(reference, &key) {
            Ok(slot) => {
                *slot = value;
                Ok(())
            }
            Err(error) => Err(self.object_error(operation, &error, location)),
        }
    }

    fn exception_text_property(
        &self,
        handle: ObjectHandle,
        name: &'static str,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<String> {
        let value = self.exception_property(handle, name, operation, location)?;
        self.function_name_text(operation, &value, location)
    }

    fn throw_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        as_caller: bool,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let operation = if as_caller { "throwAsCaller" } else { "throw" };
        if arguments.len() != 1 {
            return Err(self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:MException:wrongNumberInputs"),
                format!("{operation} requires exactly one MException"),
                location,
            ));
        }
        if requested_outputs != 0 {
            return Err(self.intrinsic_builtin_error(
                operation,
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:maxlhs"),
                format!("{operation} does not return an output"),
                location,
            ));
        }
        let source = self.exception_handle(&arguments[0], operation, location)?;
        let identifier = self.exception_text_property(source, "identifier", operation, location)?;
        let message = self.exception_text_property(source, "message", operation, location)?;
        let (copied, created) = self.language_copy_tracked(&arguments[0])?;
        let Value::Object(handle) = copied else {
            self.rollback_assignment_copies(&created);
            return Err(self.invalid_state("MException language copy returned a non-object"));
        };
        if self.pending_exception.is_some() {
            self.rollback_assignment_copies(&created);
            return Err(self.invalid_state("a second exception was thrown during active unwind"));
        }
        let mut error = self.error(
            RuntimeErrorKind::Builtin {
                name: operation.to_owned(),
                category: BuiltinErrorCategory::Other,
                identifier: Some(identifier),
                message,
            },
            location,
        );
        if as_caller {
            error.stack.pop();
            error.location = error.stack.last().and_then(|frame| frame.location);
        }
        let stack = match self.exception_stack_value(&error) {
            Ok(stack) => stack,
            Err(detail) => {
                self.rollback_assignment_copies(&created);
                return Err(self.invalid_state(&detail));
            }
        };
        if let Err(failure) =
            self.set_exception_property(handle, "stack", stack, operation, location)
        {
            self.rollback_assignment_copies(&created);
            return Err(failure);
        }
        self.exception_errors.insert(handle, error.clone());
        self.pending_exception = Some(handle);
        Err(error)
    }

    fn add_cause_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        const OPERATION: &str = "addCause";
        if arguments.len() != 2 {
            return Err(self.intrinsic_builtin_error(
                OPERATION,
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:MException:wrongNumberInputs"),
                "addCause requires a primary MException and one cause MException",
                location,
            ));
        }
        if requested_outputs > 1 {
            return Err(self.intrinsic_builtin_error(
                OPERATION,
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:maxlhs"),
                "addCause returns at most one MException",
                location,
            ));
        }
        self.exception_handle(&arguments[0], OPERATION, location)?;
        self.exception_handle(&arguments[1], OPERATION, location)?;
        let (copied, created) = self.language_copy_tracked(&arguments[0])?;
        let Value::Object(handle) = copied else {
            self.rollback_assignment_copies(&created);
            return Err(self.invalid_state("MException language copy returned a non-object"));
        };
        let current = self.exception_property(handle, "cause", OPERATION, location);
        let current = match current {
            Ok(Value::Cell(cause)) => cause,
            Ok(_) => {
                self.rollback_assignment_copies(&created);
                return Err(self.invalid_state("MException cause property is not a cell array"));
            }
            Err(error) => {
                self.rollback_assignment_copies(&created);
                return Err(error);
            }
        };
        let mut causes = current.values().to_vec();
        causes.try_reserve_exact(1).map_err(|_| {
            self.rollback_assignment_copies(&created);
            self.invalid_state("MException cause list exceeds host capacity")
        })?;
        causes.push(arguments[1].clone());
        let count = u64::try_from(causes.len()).map_err(|_| {
            self.rollback_assignment_copies(&created);
            self.invalid_state("MException cause list exceeds language limits")
        })?;
        let cause = CellArray::from_values(
            Shape::new([count, 1]).map_err(|_| {
                self.rollback_assignment_copies(&created);
                self.invalid_state("MException cause shape is invalid")
            })?,
            causes,
        )
        .map(Value::Cell)
        .map_err(|_| {
            self.rollback_assignment_copies(&created);
            self.invalid_state("MException cause value is invalid")
        })?;
        if let Err(error) = self.set_exception_property(handle, "cause", cause, OPERATION, location)
        {
            self.rollback_assignment_copies(&created);
            return Err(error);
        }
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![Value::Object(handle)])
        }
    }

    fn get_report_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        const OPERATION: &str = "getReport";
        if arguments.is_empty() {
            return Err(self.intrinsic_builtin_error(
                OPERATION,
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:MException:wrongNumberInputs"),
                "getReport requires an MException",
                location,
            ));
        }
        if requested_outputs > 1 {
            return Err(self.intrinsic_builtin_error(
                OPERATION,
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:maxlhs"),
                "getReport returns at most one text value",
                location,
            ));
        }
        let handle = self.exception_handle(&arguments[0], OPERATION, location)?;
        let mut extended = true;
        let mut index = 1;
        if let Some(value) = arguments.get(index) {
            let option = self.function_name_text(OPERATION, value, location)?;
            match option.to_ascii_lowercase().as_str() {
                "basic" => {
                    extended = false;
                    index += 1;
                }
                "extended" => index += 1,
                _ => {}
            }
        }
        while index < arguments.len() {
            let Some(value) = arguments.get(index + 1) else {
                return Err(self.intrinsic_builtin_error(
                    OPERATION,
                    BuiltinErrorCategory::ArgumentCount,
                    Some("MATLAB:MException:InvalidGetReportOption"),
                    "getReport options must be name/value pairs",
                    location,
                ));
            };
            let name = self.function_name_text(OPERATION, &arguments[index], location)?;
            let value = self.function_name_text(OPERATION, value, location)?;
            if !name.eq_ignore_ascii_case("hyperlinks")
                || !matches!(
                    value.to_ascii_lowercase().as_str(),
                    "on" | "off" | "default"
                )
            {
                return Err(self.intrinsic_builtin_error(
                    OPERATION,
                    BuiltinErrorCategory::Domain,
                    Some("MATLAB:MException:InvalidGetReportOption"),
                    "getReport supports only the hyperlinks on/off/default option",
                    location,
                ));
            }
            index += 2;
        }
        let mut report = String::new();
        self.append_exception_report(handle, extended, 0, &mut report, location)?;
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            self.char_row_value(&report, "MException report", location)
                .map(|value| vec![value])
        }
    }

    fn append_exception_report(
        &self,
        handle: ObjectHandle,
        extended: bool,
        depth: usize,
        report: &mut String,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        if depth >= 64 {
            return Err(self.intrinsic_builtin_error(
                "getReport",
                BuiltinErrorCategory::Domain,
                Some("OpenMat:MException:CauseDepth"),
                "MException cause nesting exceeds the report limit",
                location,
            ));
        }
        let indent = "  ".repeat(depth);
        let identifier =
            self.exception_text_property(handle, "identifier", "getReport", location)?;
        let message = self.exception_text_property(handle, "message", "getReport", location)?;
        report.push_str(&indent);
        if identifier.is_empty() {
            report.push_str(&message);
        } else if message.is_empty() {
            report.push_str(&identifier);
        } else {
            report.push_str(&identifier);
            report.push_str(": ");
            report.push_str(&message);
        }
        if !extended {
            return Ok(());
        }
        let stack = self.exception_property(handle, "stack", "getReport", location)?;
        let frames = self.last_error_stack_from_value(&stack, location)?;
        for frame in frames {
            report.push('\n');
            report.push_str(&indent);
            report.push_str("  in ");
            report.push_str(&frame.name);
            if !frame.file.is_empty() {
                report.push_str(" (");
                report.push_str(&frame.file);
                if frame.line > 0.0 {
                    report.push(':');
                    write!(report, "{:.0}", frame.line)
                        .expect("writing MException line number to String cannot fail");
                }
                report.push(')');
            }
        }
        let causes = self.exception_property(handle, "cause", "getReport", location)?;
        let Value::Cell(causes) = causes else {
            return Err(self.invalid_state("MException cause property is not a cell array"));
        };
        for cause in causes.values() {
            let cause = self.exception_handle(cause, "getReport", location)?;
            report.push('\n');
            report.push_str(&indent);
            report.push_str("Caused by:\n");
            self.append_exception_report(cause, true, depth + 1, report, location)?;
        }
        Ok(())
    }

    fn rethrow_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if arguments.len() != 1 {
            return Err(self.intrinsic_builtin_error(
                "rethrow",
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:UndefinedFunction"),
                "`rethrow` requires exactly one caught MException",
                location,
            ));
        }
        if requested_outputs != 0 {
            return Err(self.intrinsic_builtin_error(
                "rethrow",
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:UndefinedFunction"),
                "`rethrow` does not return an output",
                location,
            ));
        }
        let Value::Object(handle) = arguments[0] else {
            return Err(self.intrinsic_builtin_error(
                "rethrow",
                BuiltinErrorCategory::Type,
                Some("MATLAB:rethrow:invalidInputType"),
                "the input to `rethrow` must be a caught MException",
                location,
            ));
        };
        let error = self.exception_errors.get(&handle).cloned().ok_or_else(|| {
            self.intrinsic_builtin_error(
                "rethrow",
                BuiltinErrorCategory::Type,
                Some("MATLAB:rethrow:invalidInputType"),
                "the input to `rethrow` must be a caught MException",
                location,
            )
        })?;
        let (copied, created) = self.language_copy_tracked(&arguments[0])?;
        let Value::Object(copied_handle) = copied else {
            self.rollback_assignment_copies(&created);
            return Err(self.invalid_state("caught MException copy returned a non-object"));
        };
        if self.pending_exception.is_some() {
            self.rollback_assignment_copies(&created);
            return Err(self.invalid_state("a second exception was rethrown during active unwind"));
        }
        self.exception_errors.insert(copied_handle, error.clone());
        self.pending_exception = Some(copied_handle);
        Err(error)
    }

    fn tic_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if !arguments.is_empty() {
            return Err(self.intrinsic_builtin_error(
                "tic",
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:maxrhs"),
                "too many input arguments",
                location,
            ));
        }
        if requested_outputs > 1 {
            return Err(self.intrinsic_builtin_error(
                "tic",
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:maxlhs"),
                "too many output arguments",
                location,
            ));
        }

        let started = Instant::now();
        if requested_outputs == 0 {
            self.default_timer = Some(started);
            return Ok(Vec::new());
        }

        let identifier = self.next_timer_handle.ok_or_else(|| {
            self.intrinsic_builtin_error(
                "tic",
                BuiltinErrorCategory::Domain,
                None,
                "the session timer-handle table is exhausted",
                location,
            )
        })?;
        self.next_timer_handle = identifier.checked_add(1);
        self.timer_handles.insert(identifier, started);
        self.uint64_scalar(identifier, "timer handle", location)
            .map(|value| vec![value])
    }

    fn toc_intrinsic(
        &mut self,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if arguments.len() > 1 {
            return Err(self.intrinsic_builtin_error(
                "toc",
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:maxrhs"),
                "too many input arguments",
                location,
            ));
        }
        if requested_outputs > 1 {
            return Err(self.intrinsic_builtin_error(
                "toc",
                BuiltinErrorCategory::ArgumentCount,
                Some("MATLAB:maxlhs"),
                "too many output arguments",
                location,
            ));
        }

        let started = if let Some(value) = arguments.first() {
            let identifier = Self::timer_identifier(value).ok_or_else(|| {
                self.intrinsic_builtin_error(
                    "toc",
                    BuiltinErrorCategory::Type,
                    Some("MATLAB:toc:wrongTocArgument"),
                    "the input to `toc` must be a scalar uint64 timer handle",
                    location,
                )
            })?;
            self.timer_handles
                .get(&identifier)
                .copied()
                .ok_or_else(|| {
                    self.intrinsic_builtin_error(
                        "toc",
                        BuiltinErrorCategory::Domain,
                        Some("MATLAB:toc:wrongTocArgument"),
                        "the uint64 timer handle does not belong to this session",
                        location,
                    )
                })?
        } else {
            self.default_timer.ok_or_else(|| {
                self.intrinsic_builtin_error(
                    "toc",
                    BuiltinErrorCategory::Domain,
                    Some("MATLAB:toc:callTicFirstNoInputs"),
                    "call `tic` without an output before calling `toc` without an input",
                    location,
                )
            })?
        };
        let elapsed = started.elapsed().as_secs_f64();
        if requested_outputs == 0 {
            self.output
                .emit(OutputEvent::CommandText(format!(
                    "Elapsed time is {elapsed:.6} seconds.\n"
                )))
                .map_err(|error| {
                    self.intrinsic_builtin_error(
                        "toc",
                        BuiltinErrorCategory::Output,
                        None,
                        error.message,
                        location,
                    )
                })?;
            Ok(Vec::new())
        } else {
            Ok(vec![Value::Double(elapsed)])
        }
    }

    fn timer_identifier(value: &Value) -> Option<u64> {
        match value {
            Value::Array(ArrayData::Integer(IntegerArrayData::U64(array)))
                if array.numel() == 1 =>
            {
                array.as_slice().first().copied()
            }
            _ => None,
        }
    }

    fn uint64_scalar(
        &self,
        value: u64,
        label: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let shape = Shape::new([1, 1])
            .map_err(|_| self.invalid_state(&format!("{label} shape is invalid")))?;
        DenseArray::from_vec(shape, vec![value])
            .map(IntegerArrayData::U64)
            .map(ArrayData::Integer)
            .map(Value::Array)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))
    }

    fn intrinsic_builtin_error(
        &self,
        name: &str,
        category: BuiltinErrorCategory,
        identifier: Option<&str>,
        message: impl Into<String>,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        self.error(
            RuntimeErrorKind::Builtin {
                name: name.to_owned(),
                category,
                identifier: identifier.map(str::to_owned),
                message: message.into(),
            },
            location,
        )
    }

    fn expect_intrinsic_arity(
        &self,
        name: &str,
        arguments: &[Value],
        expected: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        if arguments.len() == expected {
            Ok(())
        } else {
            Err(self.error(
                RuntimeErrorKind::InputArity {
                    function: name.to_owned(),
                    expected,
                    actual: arguments.len(),
                },
                location,
            ))
        }
    }

    fn function_name_text(
        &self,
        operation: &'static str,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<String> {
        let code_units = match value {
            Value::String(value) => value
                .as_scalar()
                .filter(|value| !value.is_missing())
                .map(|value| value.code_units().to_vec()),
            Value::Array(ArrayData::Char(array))
                if array.shape().dimensions() == [0, 0]
                    || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
            {
                Some(array.as_slice().iter().map(|value| value.get()).collect())
            }
            _ => None,
        }
        .ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation,
                    message: "function name must be a char row vector or string scalar".to_owned(),
                },
                location,
            )
        })?;
        String::from_utf16(&code_units).map_err(|_| {
            self.error(
                RuntimeErrorKind::Object {
                    operation,
                    message: "function name must contain valid UTF-16 text".to_owned(),
                },
                location,
            )
        })
    }

    fn resolve_named_callable(
        &mut self,
        module: &Arc<BytecodeModule>,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if !name.contains('.') {
            let imports = self
                .builtin_scope_context
                .as_ref()
                .map_or_else(|| self.base_imports.clone(), |scope| scope.imports.clone());
            for candidate in imported_name_candidates(&imports, name) {
                if let Some(value) =
                    self.resolve_named_callable_if_present(module, &candidate, location)?
                {
                    return Ok(value);
                }
            }
        }
        if let Some(value) = self.resolve_named_callable_if_present(module, name, location)? {
            return Ok(value);
        }
        Err(self.error(
            RuntimeErrorKind::UnknownFunctionHandleTarget {
                name: name.to_owned(),
            },
            location,
        ))
    }

    fn resolve_named_callable_if_present(
        &mut self,
        module: &Arc<BytecodeModule>,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Option<Value>> {
        if let Some(function) = module_function_named(module, name) {
            return self
                .constant_value(module, &Constant::Function(function), location)
                .map(Some);
        }
        if let Ok(class) = self.classes.class_named(name) {
            return Ok(Some(Value::Function(FunctionHandle::Class(
                ClassHandle::new(class.id().get()),
            ))));
        }
        if event_runtime_named(name) {
            return Ok(Some(Value::Function(FunctionHandle::Bytecode(
                self.register_named_function_handle(name, location)?,
            ))));
        }
        if let Some(intrinsic) = intrinsic_named(name) {
            return Ok(Some(Value::Function(FunctionHandle::Intrinsic(intrinsic))));
        }
        if let Some(handle) = self.builtins.handle_by_name(name) {
            return Ok(Some(Value::Function(FunctionHandle::Builtin(handle))));
        }
        self.resolve_dynamic_file_callable(name, location)
    }

    fn resolve_qualified_callable_candidates(
        &mut self,
        module: &Arc<BytecodeModule>,
        candidates: &[String],
        member_count: u32,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(Value, u32)> {
        for candidate in candidates {
            let components = candidate.split('.').collect::<Vec<_>>();
            let member_count = usize::try_from(member_count).unwrap_or(usize::MAX);
            let minimum = components.len().saturating_sub(member_count).max(1);
            for prefix_count in (minimum..=components.len()).rev() {
                let prefix = components[..prefix_count].join(".");
                if let Some(value) =
                    self.resolve_named_callable_if_present(module, &prefix, location)?
                {
                    let suffix = u32::try_from(components.len() - prefix_count)
                        .map_err(|_| self.invalid_state("qualified suffix count overflowed"))?;
                    return Ok((value, suffix));
                }
            }
        }
        Err(self.error(
            RuntimeErrorKind::UnknownFunctionHandleTarget {
                name: candidates.first().cloned().unwrap_or_default(),
            },
            location,
        ))
    }

    fn load_call_target(
        &mut self,
        frame: &ExecutionFrame,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if let Some(value) = frame.dynamic_locals.get(name) {
            return Ok(value.clone());
        }
        match frame.captures.get(name) {
            Some(CapturedBinding::Value(value)) => return Ok(value.clone()),
            Some(CapturedBinding::Shared(value)) => {
                return value
                    .read()
                    .ok_or_else(|| self.invalid_state("shared capture lock is poisoned"));
            }
            Some(CapturedBinding::MissingValue) => {
                return Err(self.error(
                    RuntimeErrorKind::UndefinedGlobal {
                        name: name.to_owned(),
                    },
                    location,
                ));
            }
            Some(CapturedBinding::ResolverOnly) => {}
            None => {
                if let Some(value) = self.workspace.get(name) {
                    return Ok(value.clone());
                }
            }
        }
        self.register_named_function_handle(name, location)
            .map(FunctionHandle::Bytecode)
            .map(Value::Function)
    }

    fn resolve_global_name_if_present(
        &mut self,
        module: &Arc<BytecodeModule>,
        frame: &ExecutionFrame,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Option<Value>> {
        if let Some(value) = frame.dynamic_locals.get(name) {
            return Ok(Some(value.clone()));
        }
        match frame.captures.get(name) {
            Some(CapturedBinding::Value(value)) => return Ok(Some(value.clone())),
            Some(CapturedBinding::Shared(value)) => {
                return value
                    .read()
                    .ok_or_else(|| self.invalid_state("shared capture lock is poisoned"))
                    .map(Some);
            }
            Some(CapturedBinding::MissingValue) => {
                return Err(self.error(
                    RuntimeErrorKind::UndefinedGlobal {
                        name: name.to_owned(),
                    },
                    location,
                ));
            }
            Some(CapturedBinding::ResolverOnly) => {}
            None => {
                if let Some(value) = self.workspace.get(name) {
                    return Ok(Some(value.clone()));
                }
            }
        }
        if let Some(function) = module_function_named(module, name) {
            return self
                .constant_value(module, &Constant::Function(function), location)
                .map(Some);
        }
        if let Ok(class) = self.classes.class_named(name) {
            return Ok(Some(Value::Function(FunctionHandle::Class(
                ClassHandle::new(class.id().get()),
            ))));
        }
        if event_runtime_named(name) {
            return self
                .register_named_function_handle(name, location)
                .map(FunctionHandle::Bytecode)
                .map(Value::Function)
                .map(Some);
        }
        if let Some(intrinsic) = intrinsic_named(name) {
            return Ok(Some(Value::Function(FunctionHandle::Intrinsic(intrinsic))));
        }
        if let Some(handle) = self.builtins.handle_by_name(name) {
            return Ok(Some(Value::Function(FunctionHandle::Builtin(handle))));
        }
        self.resolve_dynamic_file_callable(name, location)
    }

    fn resolve_dynamic_file_callable(
        &mut self,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Option<Value>> {
        let caller = self.caller_file_source(location);
        let file_system = self.file_system.clone();
        let Some(file_system) = file_system else {
            return Ok(None);
        };
        let loaded = {
            let mut file_system = file_system
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.module_loader
                .load_function(file_system.as_mut(), caller.as_deref(), name)
        };
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(error) if error.kind == ModuleLoadErrorKind::SourceIdExhausted => {
                return Err(self.invalid_state(&error.message));
            }
            Err(error) => {
                let category = if error.kind == ModuleLoadErrorKind::FileSystem {
                    BuiltinErrorCategory::FileSystem
                } else {
                    BuiltinErrorCategory::Other
                };
                return Err(self.error(
                    RuntimeErrorKind::Builtin {
                        name: name.to_owned(),
                        category,
                        identifier: Some(error.identifier().to_owned()),
                        message: error.message,
                    },
                    location,
                ));
            }
        };
        let Some(loaded) = loaded else {
            return Ok(None);
        };
        self.register_source_metadata(loaded.source_id, loaded.source_name.clone(), &loaded.source);
        let code = RuntimeFunctionCode {
            module: loaded.module,
            function: loaded.function,
            captures: Arc::new(BTreeMap::new()),
            access_context: AccessContext::external(),
        };
        let handle = self.register_function_code(code, location)?;
        Ok(Some(Value::Function(FunctionHandle::Bytecode(handle))))
    }

    fn caller_file_source(&self, location: Option<SourceLocation>) -> Option<String> {
        location
            .into_iter()
            .chain(
                self.call_stack
                    .iter()
                    .rev()
                    .filter_map(|frame| frame.location),
            )
            .filter_map(|location| self.source_metadata.get(&location.source_id))
            .map(|metadata| metadata.name.as_str())
            .find(|name| Path::new(name).is_absolute())
            .map(str::to_owned)
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    async fn call_named_callable(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        name: &str,
        arguments: &[Value],
        requested_outputs: usize,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
        fallback: Option<RuntimeFunctionCode>,
    ) -> RuntimeResult<Vec<Value>> {
        // Handle destructors are invoked by finalization, after invalidating
        // aliases and emitting ObjectBeingDestroyed. Ordinary method dispatch
        // would call user delete code without performing those lifecycle steps.
        if name == "delete"
            && arguments
                .first()
                .and_then(|value| self.object_value_class_id(value).ok())
                .is_some_and(|class| self.classes.semantics(class) == Ok(ClassSemantics::Handle))
            && let Some(handle) = self.builtins.handle_by_name(name)
        {
            return stack
                .run(|stack| {
                    self.call_builtin(
                        stack,
                        module,
                        handle,
                        arguments.to_vec(),
                        requested_outputs,
                        location,
                    )
                })
                .await;
        }
        if let Some(returned) = stack
            .run(|stack| {
                self.call_legacy_method_if_present(
                    stack,
                    name,
                    arguments,
                    requested_outputs,
                    location,
                )
            })
            .await?
        {
            return Ok(returned);
        }
        if event_runtime_named(name) {
            return stack
                .run(|stack| {
                    self.call_event_runtime(
                        stack,
                        module,
                        name,
                        arguments,
                        requested_outputs,
                        caller_context,
                        location,
                    )
                })
                .await;
        }
        if let Some((receiver_index, handle, declaring_class, method_kind)) =
            self.dynamic_method_target(arguments, name, caller_context, location)?
        {
            if Some(declaring_class) == self.exception_class {
                return self.call_exception_method(name, arguments, requested_outputs, location);
            }
            if method_kind == MethodKind::Instance
                && self.native_classes.contains_key(&declaring_class)
            {
                let mut method_arguments = Vec::new();
                method_arguments
                    .try_reserve_exact(arguments.len().saturating_sub(1))
                    .map_err(|_| {
                        self.invalid_state("dynamic native-method arguments exceed host capacity")
                    })?;
                method_arguments.extend(
                    arguments
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| *index != receiver_index)
                        .map(|(_, value)| value.clone()),
                );
                return stack
                    .run(|stack| {
                        self.execute_native_method(
                            stack,
                            declaring_class,
                            handle,
                            name,
                            method_arguments,
                            requested_outputs,
                            location,
                        )
                    })
                    .await;
            }
            return stack
                .run(|stack| {
                    self.execute_class_method(
                        stack,
                        declaring_class,
                        name,
                        arguments,
                        requested_outputs,
                        "dynamic method dispatch",
                        location,
                    )
                })
                .await;
        }
        if let Some(code) = fallback {
            return stack
                .run(|stack| {
                    self.execute_function(
                        stack,
                        &code.module,
                        code.function,
                        arguments,
                        Invocation {
                            requested_outputs,
                            seeded_local: None,
                            copy_arguments: true,
                            access_context: code.access_context,
                            captures: code.captures,
                            scope: InvocationScope::New,
                        },
                    )
                })
                .await;
        }
        let callable = self.resolve_named_callable(module, name, location)?;
        stack
            .run(|stack| {
                self.call_value(
                    stack,
                    module,
                    &callable,
                    arguments,
                    requested_outputs,
                    caller_context,
                    location,
                )
            })
            .await
    }

    async fn call_legacy_method_if_present(
        &mut self,
        stack: &mut FrameStack,
        name: &str,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Option<Vec<Value>>> {
        for argument in arguments {
            let Ok(class_id) = self.object_value_class_id(argument) else {
                continue;
            };
            let mut pending = vec![class_id];
            let mut visited = BTreeSet::new();
            while let Some(class_id) = pending.pop() {
                if !visited.insert(class_id) {
                    continue;
                }
                let Some(metadata) = self.legacy_classes.get(&class_id).cloned() else {
                    continue;
                };
                if let Some(code) = self.load_legacy_method(&metadata, name, location)? {
                    let returned = stack
                        .run(|stack| {
                            self.execute_function(
                                stack,
                                &code.module,
                                code.function,
                                arguments,
                                Invocation {
                                    requested_outputs,
                                    seeded_local: None,
                                    copy_arguments: true,
                                    access_context: AccessContext::class(class_id),
                                    captures: Arc::new(BTreeMap::new()),
                                    scope: InvocationScope::New,
                                },
                            )
                        })
                        .await?;
                    return Ok(Some(returned));
                }
                for superclass in metadata.superclasses.iter().copied().rev() {
                    pending.push(superclass);
                }
            }
        }
        Ok(None)
    }

    fn load_legacy_method(
        &mut self,
        metadata: &LegacyClassMetadata,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Option<RuntimeFunctionCode>> {
        let Some(file_system) = self.file_system.clone() else {
            return Ok(None);
        };
        let constructor = metadata.constructor_source.to_string_lossy();
        let loaded = {
            let mut file_system = file_system
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.module_loader
                .load_function(file_system.as_mut(), Some(&constructor), name)
        };
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(error) if error.kind == ModuleLoadErrorKind::SourceIdExhausted => {
                return Err(self.invalid_state(&error.message));
            }
            Err(error) => {
                let category = if error.kind == ModuleLoadErrorKind::FileSystem {
                    BuiltinErrorCategory::FileSystem
                } else {
                    BuiltinErrorCategory::Other
                };
                return Err(self.error(
                    RuntimeErrorKind::Builtin {
                        name: name.to_owned(),
                        category,
                        identifier: Some(error.identifier().to_owned()),
                        message: error.message,
                    },
                    location,
                ));
            }
        };
        let Some(loaded) = loaded else {
            return Ok(None);
        };
        let method_path = Path::new(&loaded.source_name);
        let same_directory = method_path
            .parent()
            .zip(metadata.constructor_source.parent())
            .is_some_and(|(method, constructor)| platform_path_eq(method, constructor));
        let same_name = method_path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|stem| platform_name_eq(stem, name));
        if !same_directory || !same_name {
            return Ok(None);
        }
        self.register_source_metadata(loaded.source_id, loaded.source_name, &loaded.source);
        Ok(Some(RuntimeFunctionCode {
            module: loaded.module,
            function: loaded.function,
            captures: Arc::new(BTreeMap::new()),
            access_context: AccessContext::external(),
        }))
    }

    fn dynamic_method_target(
        &self,
        arguments: &[Value],
        name: &str,
        caller_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Option<(usize, ObjectHandle, ClassId, MethodKind)>> {
        for (index, argument) in arguments.iter().enumerate() {
            let Some(handle) = object_handle(argument) else {
                continue;
            };
            let class_id = self
                .reference(handle, "dynamic method dispatch", location)?
                .class_id();
            match self
                .classes
                .resolve_method_any(class_id, name, caller_context)
            {
                Ok(selection) => {
                    return Ok(Some((
                        index,
                        handle,
                        selection.declaring_class(),
                        selection.descriptor().kind(),
                    )));
                }
                Err(ObjectError::MethodNotFound { .. }) => {}
                Err(error) => {
                    return Err(self.object_error("dynamic method dispatch", &error, location));
                }
            }
        }
        Ok(None)
    }

    fn function_handle_text(
        &self,
        module: &BytecodeModule,
        handle: FunctionHandle,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<String> {
        let name = match handle {
            FunctionHandle::Builtin(handle) => self.builtins.name(handle).map(str::to_owned),
            FunctionHandle::Intrinsic(intrinsic) => Some(intrinsic_name(intrinsic).to_owned()),
            FunctionHandle::Class(handle) => self
                .class_handles
                .get(&handle)
                .and_then(|class| self.classes.class(*class).ok())
                .map(|class| class.name().to_owned()),
            FunctionHandle::Bytecode(handle) => {
                if let Some(name) = self.named_function_handles.get(&handle) {
                    Some(name.clone())
                } else if let Some(code) = self.function_handles.get(&handle) {
                    code.module
                        .functions
                        .get(code.function.get() as usize)
                        .map(|function| function.name.clone())
                } else if handle.index() < REGISTERED_FUNCTION_HANDLE_BASE {
                    module
                        .functions
                        .get(handle.index() as usize)
                        .map(|function| function.name.clone())
                } else {
                    None
                }
            }
        };
        name.ok_or_else(|| self.error(RuntimeErrorKind::InvalidFunctionHandle { handle }, location))
    }

    fn char_row_value(
        &self,
        text: &str,
        label: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let code_units = text
            .encode_utf16()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        let length = u64::try_from(code_units.len())
            .map_err(|_| self.invalid_state(&format!("{label} exceeds language shape limits")))?;
        let shape = Shape::new([1, length])
            .map_err(|_| self.invalid_state(&format!("{label} shape is invalid")))?;
        let array = DenseArray::from_vec(shape, code_units)
            .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
        Ok(Value::Array(ArrayData::Char(array)))
    }

    fn language_copy(&mut self, value: &Value) -> RuntimeResult<Value> {
        self.language_copy_tracked(value).map(|(value, _)| value)
    }

    fn language_copy_values(&mut self, values: &[Value]) -> RuntimeResult<Vec<Value>> {
        let mut copied = Vec::new();
        copied
            .try_reserve_exact(values.len())
            .map_err(|_| self.invalid_state("language-copy result exceeds host capacity"))?;
        let mut created = Vec::new();
        for value in values {
            match self.language_copy_tracked(value) {
                Ok((value, mut handles)) => {
                    if created.try_reserve(handles.len()).is_err() {
                        self.rollback_assignment_copies(&handles);
                        self.rollback_assignment_copies(&created);
                        return Err(
                            self.invalid_state("language-copy rollback list exceeds host capacity")
                        );
                    }
                    created.append(&mut handles);
                    copied.push(value);
                }
                Err(error) => {
                    self.rollback_assignment_copies(&created);
                    return Err(error);
                }
            }
        }
        Ok(copied)
    }

    fn language_copy_columns(&mut self, columns: &mut [Vec<Value>]) -> RuntimeResult<()> {
        let total = columns
            .iter()
            .try_fold(0_usize, |total, column| total.checked_add(column.len()));
        let total = total
            .ok_or_else(|| self.invalid_state("aggregate language-copy column size overflowed"))?;
        let mut flattened = Vec::new();
        flattened
            .try_reserve_exact(total)
            .map_err(|_| self.invalid_state("aggregate language-copy exceeds host capacity"))?;
        for column in columns.iter() {
            flattened.extend_from_slice(column);
        }
        let copied = self.language_copy_values(&flattened)?;
        for (slot, value) in columns
            .iter_mut()
            .flat_map(|column| column.iter_mut())
            .zip(copied)
        {
            *slot = value;
        }
        Ok(())
    }

    fn language_copy_tracked(
        &mut self,
        value: &Value,
    ) -> RuntimeResult<(Value, Vec<ObjectHandle>)> {
        let location = self.call_stack.last().and_then(|frame| frame.location);
        match value {
            Value::Object(handle) => {
                let (mut copied, created) =
                    self.copy_object_handles(&[*handle], "assignment", location)?;
                let value = copied
                    .pop()
                    .map(Value::Object)
                    .ok_or_else(|| self.invalid_state("scalar object copy produced no handle"))?;
                Ok((value, created))
            }
            Value::ObjectArray(array) => {
                let parts = self.object_value_parts(value, "assignment", location)?;
                let (handles, created) =
                    self.copy_object_handles(&parts.handles, "assignment", location)?;
                let result = self.make_validated_object_array(
                    parts.class_id,
                    array.shape().clone(),
                    handles,
                    location,
                );
                if result.is_err() {
                    self.rollback_assignment_copies(&created);
                }
                result.map(|value| (value, created))
            }
            _ => Ok((value.clone(), Vec::new())),
        }
    }

    fn constant_value(
        &mut self,
        module: &Arc<BytecodeModule>,
        constant: &Constant,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        match constant {
            Constant::Nothing => Ok(Value::Nothing),
            Constant::Logical(value) => Ok(Value::Logical(*value)),
            Constant::Double(value) => Ok(Value::Double(*value)),
            Constant::Complex { real, imaginary } => {
                Ok(Value::Complex(Complex64::new(*real, *imaginary)))
            }
            Constant::String(value) => Ok(Value::from(value.clone())),
            Constant::Char(code_units) => {
                let dimensions = if code_units.is_empty() {
                    [0, 0]
                } else {
                    [
                        1,
                        u64::try_from(code_units.len()).map_err(|_| {
                            self.error(array_error(ArrayRuntimeError::SizeLimit), location)
                        })?,
                    ]
                };
                let shape = Shape::new(dimensions)
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                let values = code_units.iter().copied().map(CharCodeUnit::new).collect();
                let array = DenseArray::from_vec(shape, values)
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                Ok(Value::Array(ArrayData::Char(array)))
            }
            Constant::Function(function) => {
                if let Some((handle, _)) = self.function_handles.iter().find(|(_, code)| {
                    code.function == *function && Arc::ptr_eq(&code.module, module)
                }) {
                    return Ok(Value::Function(FunctionHandle::Bytecode(*handle)));
                }
                let handle = self.register_function_code(
                    RuntimeFunctionCode {
                        module: Arc::clone(module),
                        function: *function,
                        captures: Arc::new(BTreeMap::new()),
                        access_context: AccessContext::external(),
                    },
                    location,
                )?;
                Ok(Value::Function(FunctionHandle::Bytecode(handle)))
            }
        }
    }

    fn register_function_code(
        &mut self,
        code: RuntimeFunctionCode,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<BytecodeFunctionHandle> {
        let handle = self.allocate_function_handle(location)?;
        self.function_handles.insert(handle, code);
        Ok(handle)
    }

    fn register_named_function_handle(
        &mut self,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<BytecodeFunctionHandle> {
        let handle = self.allocate_function_handle(location)?;
        self.named_function_handles.insert(handle, name.to_owned());
        Ok(handle)
    }

    fn register_bound_named_function_handle(
        &mut self,
        name: &str,
        fallback: RuntimeFunctionCode,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<BytecodeFunctionHandle> {
        let handle = self.register_named_function_handle(name, location)?;
        self.named_function_fallbacks.insert(handle, fallback);
        Ok(handle)
    }

    fn allocate_function_handle(
        &mut self,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<BytecodeFunctionHandle> {
        let identifier = self.next_function_handle.ok_or_else(|| {
            self.error(
                RuntimeErrorKind::InvalidExecutionState {
                    message: "function-handle registry is exhausted".to_owned(),
                    array: None,
                },
                location,
            )
        })?;
        self.next_function_handle = identifier.checked_add(1);
        Ok(BytecodeFunctionHandle::new(identifier))
    }

    fn copy_receiver_for_write(
        &mut self,
        handle: ObjectHandle,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<ObjectHandle> {
        let reference = self.object_references.get(&handle).ok_or_else(|| {
            unknown_object_error(&self.call_stack, handle, "property write", location)
        })?;
        let state = self
            .objects
            .state(reference)
            .map_err(|error| self.object_error("property write", &error, location))?;
        if reference.semantics() == ClassSemantics::Handle
            || state != ConstructionState::Constructed
        {
            return Ok(handle);
        }
        let copy = match self.objects.assignment_copy(reference) {
            Ok(copy) => copy,
            Err(error) => return Err(self.object_error("property write", &error, location)),
        };
        let exception = self.exception_errors.get(&handle).cloned();
        let copy_handle = ObjectHandle::new(copy.id().get());
        self.object_references.insert(copy_handle, copy);
        if let Some(exception) = exception {
            self.exception_errors.insert(copy_handle, exception);
        }
        Ok(copy_handle)
    }

    fn reference(
        &self,
        handle: ObjectHandle,
        operation: &'static str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<&ObjectRef> {
        self.object_references.get(&handle).ok_or_else(|| {
            self.error(
                RuntimeErrorKind::Object {
                    operation,
                    message: format!("unknown object handle {}", handle.identifier()),
                },
                location,
            )
        })
    }

    fn object_reference(&self, handle: ObjectHandle) -> Result<&ObjectRef, ()> {
        self.object_references.get(&handle).ok_or(())
    }

    fn active_finalizer_for_handle(&self, handle: ObjectHandle) -> Option<FinalizationCandidate> {
        self.active_finalizer
            .filter(|candidate| candidate.id().get() == handle.identifier())
    }

    #[cfg(test)]
    fn rollback_object(&mut self, handle: ObjectHandle) {
        frames::run(async |stack| self.rollback_object_on_stack(stack, handle).await);
    }

    async fn rollback_object_on_stack(&mut self, stack: &mut FrameStack, handle: ObjectHandle) {
        let semantics = self
            .object_references
            .get(&handle)
            .map(ObjectRef::semantics);
        if let Some(reference) = self.object_references.get(&handle) {
            let _ = self.objects.fail_construction(reference);
        }
        if semantics == Some(ClassSemantics::Handle) {
            if stack
                .run(|stack| self.begin_explicit_handle_finalization(stack, handle, None))
                .await
                .is_ok()
            {
                stack
                    .run(|stack| {
                        self.drain_requested_finalizers(stack, SafepointReason::ConstructionFailure)
                    })
                    .await;
            }
            return;
        }
        if let Some(reference) = self.object_references.get(&handle) {
            let _ = self.objects.discard_incomplete(reference);
        }
        self.object_references.remove(&handle);
        self.exception_errors.remove(&handle);
    }

    fn object_error(
        &self,
        operation: &'static str,
        error: &ObjectError,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        let kind = match error {
            ObjectError::AccessDenied {
                member, required, ..
            } => RuntimeErrorKind::AccessViolation {
                operation,
                member: member.clone(),
                required: *required,
            },
            ObjectError::AbstractClassInstantiation { .. } => RuntimeErrorKind::ClassConstraint {
                operation,
                constraint: ClassConstraintKind::AbstractClassInstantiation,
                message: error.to_string(),
            },
            ObjectError::SealedSuperclass { .. } => RuntimeErrorKind::ClassConstraint {
                operation,
                constraint: ClassConstraintKind::SealedSuperclass,
                message: error.to_string(),
            },
            ObjectError::ExplicitConcreteWithAbstractMethods { .. } => {
                RuntimeErrorKind::ClassConstraint {
                    operation,
                    constraint: ClassConstraintKind::ExplicitConcreteWithAbstractMethods,
                    message: error.to_string(),
                }
            }
            ObjectError::AbstractOverrideAccessMismatch { .. } => {
                RuntimeErrorKind::ClassConstraint {
                    operation,
                    constraint: ClassConstraintKind::AbstractOverrideAccessMismatch,
                    message: error.to_string(),
                }
            }
            ObjectError::AbstractMethodCall { .. } => RuntimeErrorKind::ClassConstraint {
                operation,
                constraint: ClassConstraintKind::AbstractMethodUnavailable,
                message: error.to_string(),
            },
            _ => RuntimeErrorKind::Object {
                operation,
                message: error.to_string(),
            },
        };
        self.error(kind, location)
    }

    fn read_apply_arguments(
        &self,
        frame: &ExecutionFrame,
        arguments: &[ApplyArgument],
    ) -> RuntimeResult<Vec<IndexInput>> {
        let mut values = Vec::new();
        values
            .try_reserve(arguments.len())
            .map_err(|_| self.invalid_state("apply argument list exceeds host capacity"))?;
        for argument in arguments {
            match argument {
                ApplyArgument::Value(register) => {
                    values.push(IndexInput::Value(
                        self.read_register(frame, *register)?.clone(),
                    ));
                }
                ApplyArgument::Expand(register) => {
                    let pack = self.read_pack_register(frame, *register)?;
                    values.try_reserve(pack.len()).map_err(|_| {
                        self.invalid_state("expanded argument list exceeds host capacity")
                    })?;
                    values.extend(pack.iter().cloned().map(IndexInput::Value));
                }
                ApplyArgument::Colon => values.push(IndexInput::Colon),
            }
        }
        Ok(values)
    }

    fn take_call_arguments(
        &self,
        frame: &mut ExecutionFrame,
        function: &Function,
        instruction: InstructionIndex,
        arguments: &[Register],
    ) -> RuntimeResult<Vec<Value>> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(arguments.len())
            .map_err(|_| self.invalid_state("call argument list exceeds host capacity"))?;
        for (index, register) in arguments.iter().copied().enumerate() {
            if arguments[index + 1..].contains(&register)
                || register_is_read_elsewhere(function, instruction, register)
            {
                values.push(self.read_register(frame, register)?.clone());
            } else {
                values.push(self.take_register(frame, register)?);
            }
        }
        Ok(values)
    }

    fn take_apply_arguments(
        &self,
        frame: &mut ExecutionFrame,
        function: &Function,
        instruction: InstructionIndex,
        arguments: &[ApplyArgument],
    ) -> RuntimeResult<Vec<IndexInput>> {
        let mut values = Vec::new();
        values
            .try_reserve(arguments.len())
            .map_err(|_| self.invalid_state("apply argument list exceeds host capacity"))?;
        for (index, argument) in arguments.iter().copied().enumerate() {
            match argument {
                ApplyArgument::Value(register) => {
                    let appears_again =
                        arguments[index + 1..].contains(&ApplyArgument::Value(register));
                    let value = if appears_again
                        || register_is_read_elsewhere(function, instruction, register)
                    {
                        self.read_register(frame, register)?.clone()
                    } else {
                        self.take_register(frame, register)?
                    };
                    values.push(IndexInput::Value(value));
                }
                ApplyArgument::Expand(register) => {
                    let appears_again =
                        arguments[index + 1..].contains(&ApplyArgument::Expand(register));
                    let pack = if appears_again
                        || pack_register_is_read_elsewhere(function, instruction, register)
                    {
                        self.read_pack_register(frame, register)?.to_vec()
                    } else {
                        self.take_pack_register(frame, register)?
                    };
                    values.try_reserve(pack.len()).map_err(|_| {
                        self.invalid_state("expanded argument list exceeds host capacity")
                    })?;
                    values.extend(pack.into_iter().map(IndexInput::Value));
                }
                ApplyArgument::Colon => values.push(IndexInput::Colon),
            }
        }
        Ok(values)
    }

    fn dynamic_field_name(&self, value: &Value) -> RuntimeResult<String> {
        let location = self.call_stack.last().and_then(|frame| frame.location);
        let code_units = match value {
            Value::String(value) => value
                .as_scalar()
                .filter(|value| !value.is_missing())
                .map(|value| value.code_units().to_vec()),
            Value::Array(ArrayData::Char(array))
                if array.shape().dimensions() == [0, 0]
                    || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
            {
                Some(array.as_slice().iter().map(|value| value.get()).collect())
            }
            _ => None,
        }
        .ok_or_else(|| {
            self.error(
                array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "dynamic aggregate field name",
                    actual: value.kind(),
                }),
                location,
            )
        })?;
        String::from_utf16(&code_units).map_err(|_| {
            self.error(
                array_error(ArrayRuntimeError::InvalidStructFieldName {
                    name: String::from("<invalid UTF-16>"),
                }),
                location,
            )
        })
    }

    fn validate_struct_field_name(
        &self,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<FieldName> {
        FieldName::new(name).map_err(|_| {
            self.error(
                array_error(ArrayRuntimeError::InvalidStructFieldName {
                    name: name.to_owned(),
                }),
                location,
            )
        })
    }

    fn aggregate_type_error(
        &self,
        operation: &'static str,
        _expected: &'static str,
        actual: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        self.error(
            array_error(ArrayRuntimeError::InvalidOperand {
                operation,
                actual: actual.kind(),
            }),
            location,
        )
    }

    fn aggregate_value_error(
        &self,
        operation: &'static str,
        error: openmat_value::AggregateError,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        let detail = match error {
            openmat_value::AggregateError::InvalidFieldName { name }
            | openmat_value::AggregateError::DuplicateFieldName { name } => {
                ArrayRuntimeError::InvalidStructFieldName { name }
            }
            openmat_value::AggregateError::Array(_)
            | openmat_value::AggregateError::FieldColumnCountMismatch { .. }
            | openmat_value::AggregateError::FieldColumnLengthMismatch { .. }
            | openmat_value::AggregateError::FieldOutOfBounds { .. }
            | openmat_value::AggregateError::OffsetOutOfBounds { .. } => {
                ArrayRuntimeError::SizeLimit
            }
        };
        let mut error = self.error(array_error(detail), location);
        if let RuntimeErrorKind::InvalidExecutionState { message, .. } = &mut error.kind {
            *message = format!("{operation} failed");
        }
        error
    }

    #[allow(clippy::too_many_lines)]
    async fn call_builtin(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        handle: BuiltinHandle,
        arguments: Vec<Value>,
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let Some((name, function)) = self.builtins.resolve(handle) else {
            return Err(self.error(
                RuntimeErrorKind::InvalidFunctionHandle {
                    handle: FunctionHandle::Builtin(handle),
                },
                location,
            ));
        };
        stack
            .run(|stack| {
                self.call_builtin_function(
                    stack,
                    module,
                    &name,
                    function.as_ref(),
                    arguments,
                    requested_outputs,
                    location,
                )
            })
            .await
    }

    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_arguments)]
    async fn call_builtin_function(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        name: &str,
        function: &dyn BuiltinFunction,
        arguments: Vec<Value>,
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        self.check_cancelled(location)?;
        let caller_source = self.caller_file_source(location);
        let scope_target = self
            .builtin_scope_context
            .as_ref()
            .map_or(ScopeAddress::Base, |scope| scope.address);
        let cancellation = self.cancellation.clone();
        let linalg_provider = Arc::clone(&self.linalg_provider);
        let fft_provider = Arc::clone(&self.fft_provider);
        let regex_provider = Arc::clone(&self.regex_provider);
        let output = std::mem::replace(&mut self.output, Box::new(NullOutput));
        let output = Arc::new(Mutex::new(Some(output)));
        self.output = Box::new(SharedOutputSink::new(Arc::clone(&output)));
        let mut builtin_output = SharedOutputSink::new(Arc::clone(&output));
        let mut graphics = SharedGraphicsService {
            graphics: Arc::clone(&self.graphics),
        };
        let mut random = SharedRandomService {
            random: Arc::clone(&self.random),
        };
        let mut created = Vec::new();
        let mut copy_failure = None;
        let mut callback_failure = None;
        let workspace = self
            .builtin_workspace
            .clone()
            .unwrap_or_else(|| self.workspace.clone());
        let mut staged_scope_bindings = Vec::new();
        let mut object_lifecycle = self.builtin_object_lifecycle();
        let mut native_objects = self.builtin_native_objects();
        let mut file_system =
            self.file_system
                .as_ref()
                .map(|file_system| SharedFileSystemService {
                    file_system: Arc::clone(file_system),
                });
        let result = {
            let mut language = InterpreterLanguageService {
                interpreter: self,
                module: Arc::clone(module),
                location,
                created: &mut created,
                copy_failure: &mut copy_failure,
                callback_failure: &mut callback_failure,
            };
            let mut context = BuiltinContext::with_services(
                requested_outputs,
                &cancellation,
                &mut builtin_output,
                linalg_provider.as_ref(),
                fft_provider.as_ref(),
                regex_provider.as_ref(),
                &mut language,
                &mut graphics,
                &mut random,
                file_system
                    .as_mut()
                    .map(|service| service as &mut dyn crate::FileSystemService),
                &workspace,
                &mut staged_scope_bindings,
                caller_source.as_deref(),
            );
            context.set_object_lifecycle_service(&mut object_lifecycle);
            context.set_native_object_service(&mut native_objects);
            function.call_owned(arguments, &mut context)
        };
        let displaced_output = std::mem::replace(&mut self.output, Box::new(NullOutput));
        drop(displaced_output);
        self.output = output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .expect("the reentrant runtime output owner remains installed");
        if let Some(error) = callback_failure {
            self.rollback_assignment_copies(&created);
            return Err(error);
        }
        if let Some(category) = copy_failure {
            self.rollback_assignment_copies(&created);
            if category == BuiltinErrorCategory::Cancelled {
                return Err(self.error(RuntimeErrorKind::Cancelled, location));
            }
            return Err(self.error(
                RuntimeErrorKind::Builtin {
                    name: name.to_string(),
                    category,
                    identifier: None,
                    message: "the built-in language-copy transaction failed".to_owned(),
                },
                location,
            ));
        }
        if cancellation.is_cancelled() {
            self.rollback_assignment_copies(&created);
            return Err(self.error(RuntimeErrorKind::Cancelled, location));
        }
        if result.is_ok() {
            for value in std::mem::take(&mut object_lifecycle.accepted_deletions) {
                stack
                    .run(|stack| self.process_lifecycle_deletion_on_stack(stack, &value, location))
                    .await?;
            }
        }
        let returned = result.map_err(|error| {
            self.rollback_assignment_copies(&created);
            let kind = builtin_failure_kind(name, error);
            self.error(kind, location)
        })?;
        let created_native = match stack
            .run(|stack| self.commit_native_creations(stack, &mut native_objects, location))
            .await
        {
            Ok(created_native) => created_native,
            Err(error) => {
                self.rollback_assignment_copies(&created);
                return Err(error);
            }
        };
        if self
            .pending_scope_bindings
            .try_reserve(staged_scope_bindings.len())
            .is_err()
        {
            self.rollback_assignment_copies(&created);
            for handle in created_native.iter().rev().copied() {
                stack
                    .run(|stack| self.rollback_object_on_stack(stack, handle))
                    .await;
            }
            return Err(self.error(
                RuntimeErrorKind::Builtin {
                    name: name.to_string(),
                    category: BuiltinErrorCategory::LanguageCopy,
                    identifier: None,
                    message: "the staged scope-binding transaction exceeded runtime limits"
                        .to_owned(),
                },
                location,
            ));
        }
        self.pending_scope_bindings
            .extend(
                staged_scope_bindings
                    .drain(..)
                    .map(|(name, value)| ScopeBindingChange {
                        target: scope_target,
                        name,
                        value,
                    }),
            );
        Ok(returned)
    }

    fn builtin_object_lifecycle(&self) -> BuiltinObjectLifecycle {
        let objects = self
            .object_references
            .iter()
            .map(|(handle, reference)| {
                let valid = reference.semantics() == ClassSemantics::Handle
                    && self
                        .objects
                        .handle_state(reference)
                        .is_ok_and(|state| matches!(state, HandleState::Alive));
                (
                    *handle,
                    LifecycleObjectSnapshot {
                        semantics: reference.semantics(),
                        valid,
                    },
                )
            })
            .collect();
        BuiltinObjectLifecycle {
            objects,
            accepted_deletions: Vec::new(),
        }
    }

    fn builtin_native_objects(&self) -> BuiltinNativeObjects {
        let classes = self
            .native_classes
            .iter()
            .map(|(class_id, class)| (class.token(), (*class_id, Arc::clone(class))))
            .collect();
        let objects = self
            .native_instances
            .iter()
            .map(|(handle, native)| {
                let valid = self.object_references.get(handle).is_some_and(|reference| {
                    self.objects.state(reference) == Ok(ConstructionState::Constructed)
                        && self.objects.handle_state(reference) == Ok(HandleState::Alive)
                });
                (
                    *handle,
                    NativeObjectSnapshot {
                        native: Arc::clone(native),
                        valid,
                    },
                )
            })
            .collect();
        BuiltinNativeObjects {
            objects,
            classes,
            pending: Vec::new(),
            next_identifier: Some(self.objects.next_allocation_id().get()),
        }
    }

    fn process_lifecycle_deletion(
        &mut self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        frames::run(async |stack| {
            self.process_lifecycle_deletion_on_stack(stack, value, location)
                .await
        })
    }

    async fn process_lifecycle_deletion_on_stack(
        &mut self,
        stack: &mut FrameStack,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        let handles = match value {
            Value::Object(handle) => std::slice::from_ref(handle),
            Value::ObjectArray(array) => array.as_slice(),
            _ => {
                return Err(self.error(
                    RuntimeErrorKind::Object {
                        operation: "explicit delete",
                        message: "lifecycle service accepted a non-object receiver".to_owned(),
                    },
                    location,
                ));
            }
        };
        let Some(first) = handles.first().copied() else {
            return Ok(());
        };
        let semantics = self
            .object_reference(first)
            .map(ObjectRef::semantics)
            .map_err(|()| {
                unknown_object_error(&self.call_stack, first, "explicit delete", location)
            })?;
        if semantics == ClassSemantics::Value {
            let class_id = self.object_value_class_id(value).map_err(|()| {
                unknown_object_error(&self.call_stack, first, "value delete dispatch", location)
            })?;
            let method = self
                .classes
                .resolve_method(
                    class_id,
                    "delete",
                    MethodKind::Instance,
                    AccessContext::external(),
                )
                .map_err(|error| self.object_error("value delete dispatch", &error, location))?;
            let declaring_class = method.declaring_class();
            stack
                .run(|stack| {
                    self.execute_class_method(
                        stack,
                        declaring_class,
                        "delete",
                        std::slice::from_ref(value),
                        0,
                        "value delete dispatch",
                        location,
                    )
                })
                .await?;
            return Ok(());
        }

        for handle in handles.iter().copied() {
            stack
                .run(|stack| self.begin_explicit_handle_finalization(stack, handle, location))
                .await?;
        }
        stack
            .run(|stack| self.drain_requested_finalizers(stack, SafepointReason::ExplicitDelete))
            .await;
        Ok(())
    }

    async fn drain_requested_finalizers(
        &mut self,
        stack: &mut FrameStack,
        reason: SafepointReason,
    ) {
        debug_assert!(matches!(
            reason,
            SafepointReason::ExplicitDelete | SafepointReason::ConstructionFailure
        ));
        if self.finalization_safepoints.begin_drain() {
            stack.run(|stack| self.drain_finalizer_queue(stack)).await;
            self.finalization_safepoints.end_drain();
        }
    }

    async fn begin_explicit_handle_finalization(
        &mut self,
        stack: &mut FrameStack,
        handle: ObjectHandle,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        if self.event_listeners.contains_key(&handle) {
            self.detach_event_listener(handle);
        }
        if !self.destroying_event_sources.insert(handle) {
            return Ok(());
        }
        let destruction = (async {
            if self
                .event_listener_order
                .contains_key(&(handle, "ObjectBeingDestroyed".to_owned()))
            {
                let data = self.default_event_data(handle, "ObjectBeingDestroyed", location)?;
                stack
                    .run(|stack| {
                        self.dispatch_event(stack, handle, "ObjectBeingDestroyed", &data, location)
                    })
                    .await?;
            }
            self.detach_event_source(handle);
            Ok(())
        })
        .await;
        self.destroying_event_sources.remove(&handle);
        destruction?;
        let class_id = self
            .object_references
            .get(&handle)
            .map(ObjectRef::class_id)
            .ok_or_else(|| {
                unknown_object_error(&self.call_stack, handle, "explicit delete", location)
            })?;
        let requires_destructor = self
            .classes
            .resolve_method(
                class_id,
                "delete",
                MethodKind::Instance,
                AccessContext::class(class_id),
            )
            .is_ok()
            || self.native_classes.contains_key(&class_id);
        let candidate = {
            let reference = self.object_references.get(&handle).ok_or_else(|| {
                unknown_object_error(&self.call_stack, handle, "explicit delete", location)
            })?;
            self.objects
                .begin_finalization(reference, requires_destructor)
        };
        let candidate =
            candidate.map_err(|error| self.object_error("explicit delete", &error, location))?;
        if let Some(candidate) = candidate {
            self.finalizer_queue
                .enqueue(candidate.id().get(), candidate);
        }
        Ok(())
    }

    fn refresh_current_frame_roots(&mut self, frame: &ExecutionFrame) {
        if let Some(roots) = self.active_frame_roots.last_mut() {
            *roots = frame.gc_roots();
        }
    }

    #[allow(clippy::too_many_lines)]
    #[cfg(test)]
    fn collect_garbage_at_safepoint<'a>(
        &mut self,
        temporary_roots: impl IntoIterator<Item = &'a Value>,
        reason: SafepointReason,
        current_language_frame: Option<&ExecutionFrame>,
    ) -> Option<GcCollection> {
        frames::run(async |stack| {
            self.collect_garbage_at_safepoint_on_stack(
                stack,
                temporary_roots,
                reason,
                current_language_frame,
            )
            .await
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn collect_garbage_at_safepoint_on_stack<'a>(
        &mut self,
        stack: &mut FrameStack,
        temporary_roots: impl IntoIterator<Item = &'a Value>,
        reason: SafepointReason,
        current_language_frame: Option<&ExecutionFrame>,
    ) -> Option<GcCollection> {
        let force = !matches!(reason, SafepointReason::AllocationDebt);
        if !force && !self.objects.collection_due(GC_ALLOCATION_THRESHOLD) {
            return None;
        }
        if !self.finalization_safepoints.begin_drain() {
            return None;
        }
        if force {
            self.finalization_safepoints.take_root_removal();
        }

        let mut object_roots = Vec::new();
        let mut visited_functions = BTreeSet::new();
        for (_, value) in self.workspace.iter() {
            trace_gc_value(
                value,
                &self.function_handles,
                &mut visited_functions,
                &mut |handle| object_roots.push(handle),
            );
        }
        for workspace in &self.displaced_workspaces {
            for (_, value) in workspace.iter() {
                trace_gc_value(
                    value,
                    &self.function_handles,
                    &mut visited_functions,
                    &mut |handle| object_roots.push(handle),
                );
            }
        }
        for value in self.persistent_bindings.values() {
            trace_gc_value(
                value,
                &self.function_handles,
                &mut visited_functions,
                &mut |handle| object_roots.push(handle),
            );
        }
        for value in self.classes.default_values() {
            trace_gc_value(
                value,
                &self.function_handles,
                &mut visited_functions,
                &mut |handle| object_roots.push(handle),
            );
        }
        for value in self.enumeration_members.values() {
            trace_gc_value(
                value,
                &self.function_handles,
                &mut visited_functions,
                &mut |handle| object_roots.push(handle),
            );
        }
        if let Some(handle) = self.pending_exception {
            object_roots.push(handle);
        }
        let active_frame_count = self
            .active_frame_roots
            .len()
            .saturating_sub(usize::from(current_language_frame.is_some()));
        for value in self
            .active_frame_roots
            .iter()
            .take(active_frame_count)
            .flatten()
        {
            trace_gc_value(
                value,
                &self.function_handles,
                &mut visited_functions,
                &mut |handle| object_roots.push(handle),
            );
        }
        if let Some(frame) = current_language_frame {
            for value in frame.language_gc_roots() {
                trace_gc_value(
                    &value,
                    &self.function_handles,
                    &mut visited_functions,
                    &mut |handle| object_roots.push(handle),
                );
            }
            for value in frame
                .registers
                .iter()
                .chain(frame.pack_registers.iter().flatten())
                .chain(&frame.call_inputs)
            {
                trace_gc_value(
                    value,
                    &self.function_handles,
                    &mut visited_functions,
                    &mut |handle| {
                        let handle_semantics =
                            self.object_references
                                .get(&handle)
                                .is_some_and(|reference| {
                                    reference.semantics() == ClassSemantics::Handle
                                });
                        if !handle_semantics {
                            object_roots.push(handle);
                        }
                    },
                );
            }
        }
        if let Some(workspace) = &self.builtin_workspace {
            for (_, value) in workspace.iter() {
                trace_gc_value(
                    value,
                    &self.function_handles,
                    &mut visited_functions,
                    &mut |handle| object_roots.push(handle),
                );
            }
        }
        for change in &self.pending_scope_bindings {
            trace_gc_value(
                &change.value,
                &self.function_handles,
                &mut visited_functions,
                &mut |handle| object_roots.push(handle),
            );
        }
        for value in temporary_roots {
            trace_gc_value(
                value,
                &self.function_handles,
                &mut visited_functions,
                &mut |handle| object_roots.push(handle),
            );
        }

        let function_handles = &self.function_handles;
        let plan = self.objects.begin_collection_with(
            object_roots,
            |slot, visit| {
                trace_gc_value(slot, function_handles, &mut visited_functions, visit);
            },
            |_class_id| true,
        );
        self.finalizer_queue
            .enqueue_all(plan.candidates().iter().copied(), |candidate| {
                candidate.id().get()
            });
        stack.run(|stack| self.drain_finalizer_queue(stack)).await;
        let collection = self.objects.finish_collection(plan);
        self.retain_live_ffi_objects();

        let objects = &self.objects;
        self.object_references.retain(|_, reference| {
            reference.semantics() == ClassSemantics::Handle || objects.state(reference).is_ok()
        });
        let object_references = &self.object_references;
        self.exception_errors.retain(|handle, _| {
            let Some(reference) = object_references.get(handle) else {
                return false;
            };
            if reference.semantics() == ClassSemantics::Handle {
                objects
                    .handle_state(reference)
                    .is_ok_and(|state| state == HandleState::Alive)
            } else {
                objects.state(reference).is_ok()
            }
        });
        self.finalization_safepoints.end_drain();
        Some(collection)
    }

    async fn drain_finalizer_queue(&mut self, stack: &mut FrameStack) {
        while let Some(candidate) = self.finalizer_queue.pop_front() {
            if let Err(error) = stack
                .run(|stack| self.execute_finalizer(stack, candidate))
                .await
            {
                self.pending_finalizer_errors.push(error);
            }
            let _ = self.objects.finish_finalization(&candidate);
        }
        self.flush_finalizer_warnings();
    }

    async fn execute_finalizer(
        &mut self,
        stack: &mut FrameStack,
        candidate: FinalizationCandidate,
    ) -> RuntimeResult<()> {
        let handle = ObjectHandle::new(candidate.id().get());
        if self.event_listeners.contains_key(&handle) {
            self.detach_event_listener(handle);
        }
        self.active_finalizer = Some(candidate);
        let destruction = (async {
            if self
                .event_listener_order
                .contains_key(&(handle, "ObjectBeingDestroyed".to_owned()))
            {
                let data = self.default_event_data(handle, "ObjectBeingDestroyed", None)?;
                stack
                    .run(|stack| {
                        self.dispatch_event(stack, handle, "ObjectBeingDestroyed", &data, None)
                    })
                    .await?;
            }
            self.detach_event_source(handle);
            Ok(())
        })
        .await;
        if let Err(error) = destruction {
            self.active_finalizer = None;
            return Err(error);
        }
        if self.native_instances.remove(&handle).is_some() {
            self.active_finalizer = None;
            return Ok(());
        }
        let declaring_class = match self.classes.resolve_method(
            candidate.class_id(),
            "delete",
            MethodKind::Instance,
            AccessContext::class(candidate.class_id()),
        ) {
            Ok(selection) => selection.declaring_class(),
            Err(ObjectError::MethodNotFound { .. }) => {
                self.active_finalizer = None;
                return Ok(());
            }
            Err(error) => {
                self.active_finalizer = None;
                return Err(self.object_error("handle finalization", &error, None));
            }
        };
        let code = self
            .class_code
            .get(&declaring_class)
            .and_then(|class| class.methods.get("delete"))
            .cloned()
            .ok_or_else(|| {
                self.error(
                    RuntimeErrorKind::Object {
                        operation: "handle finalization",
                        message: "executable body for `delete` is missing".to_owned(),
                    },
                    None,
                )
            })?;
        let receiver = Value::Object(handle);
        let result = stack
            .run(|stack| {
                self.execute_function(
                    stack,
                    &code.module,
                    code.function,
                    std::slice::from_ref(&receiver),
                    Invocation {
                        requested_outputs: 0,
                        seeded_local: None,
                        copy_arguments: false,
                        access_context: AccessContext::class(declaring_class),
                        captures: Arc::new(BTreeMap::new()),
                        scope: InvocationScope::New,
                    },
                )
            })
            .await;
        self.active_finalizer = None;
        result.map(|_| ())
    }

    fn flush_finalizer_warnings(&mut self) {
        const IDENTIFIER: &str = "OpenMat:HandleDeleteError";
        for error in std::mem::take(&mut self.pending_finalizer_errors) {
            let message = format!("error while deleting handle object: {error}");
            if self.warning_state.record(&message, IDENTIFIER) {
                let _ = self
                    .output
                    .emit(OutputEvent::CommandText(format!("Warning: {message}\n")));
            }
        }
    }

    async fn load_named_callable_value(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        callable: Value,
        invoke_if_callable: bool,
        access_context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if !invoke_if_callable {
            return Ok(callable);
        }
        let mut outputs = stack
            .run(|stack| {
                self.call_value(stack, module, &callable, &[], 1, access_context, location)
            })
            .await?
            .into_iter();
        let value = outputs.next().ok_or_else(|| {
            self.error(
                RuntimeErrorKind::MissingOutputs {
                    requested: 1,
                    returned: 0,
                },
                location,
            )
        })?;
        if outputs.next().is_some() {
            return Err(self.invalid_state(
                "a bare named callable returned more than its one requested output",
            ));
        }
        Ok(value)
    }

    fn read_pack_count(&self, frame: &ExecutionFrame, register: Register) -> RuntimeResult<usize> {
        match self.read_register(frame, register)? {
            Value::Double(value)
                if value.is_finite()
                    && *value >= 0.0
                    && value.fract() == 0.0
                    && *value <= f64::from(u32::MAX) =>
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(*value as usize)
            }
            _ => Err(self.invalid_state("output pack count must be a nonnegative u32 scalar")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_output_pack(
        &mut self,
        stack: &mut FrameStack,
        module: &Arc<BytecodeModule>,
        function: &Function,
        frame: &ExecutionFrame,
        target: PackApplyTarget,
        arguments: Vec<IndexInput>,
        count: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        let context = frame.access_context;
        let (receiver, member) = match target {
            PackApplyTarget::Value(target) => (self.read_register(frame, target)?.clone(), None),
            PackApplyTarget::Field { object, name } => (
                self.read_register(frame, object)?.clone(),
                Some(
                    global_name(function, name)
                        .ok_or_else(|| self.invalid_state("pack member escaped validation"))?
                        .to_owned(),
                ),
            ),
            PackApplyTarget::Qualified {
                target,
                unresolved_suffix,
                members,
            } => {
                let target = self.read_register(frame, target)?.clone();
                let members = members
                    .iter()
                    .map(|name| {
                        global_name(function, *name)
                            .map(str::to_owned)
                            .ok_or_else(|| self.invalid_state("pack member escaped validation"))
                    })
                    .collect::<RuntimeResult<Vec<_>>>()?;
                let members = self.qualified_suffix_members(frame, unresolved_suffix, &members)?;
                if members.is_empty() {
                    (target, None)
                } else {
                    let (receiver, member) = stack
                        .run(|stack| {
                            self.resolve_qualified_member_receiver(
                                stack, target, members, context, location,
                            )
                        })
                        .await?;
                    (receiver, Some(member.to_owned()))
                }
            }
        };
        // A qualified receiver may itself invoke getters and consume the
        // prepared scope. Refresh it for the actual RHS call.
        self.prepare_builtin_scope(module, function, frame)?;
        if let Some(member) = member {
            stack
                .run(|stack| {
                    self.apply_field(
                        stack, module, &receiver, &member, arguments, count, context, location,
                    )
                })
                .await
        } else {
            stack
                .run(|stack| {
                    self.apply_value_owned(
                        stack, module, &receiver, arguments, count, context, location,
                    )
                })
                .await
        }
    }

    async fn count_place_outputs(
        &mut self,
        stack: &mut FrameStack,
        root: &Value,
        path: &[ResolvedPlaceStep],
        context: AccessContext,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<usize> {
        let (last, prefix) = path
            .split_last()
            .ok_or_else(|| self.invalid_state("empty output place"))?;
        let mut target = root.clone();
        for step in prefix {
            target = match step {
                ResolvedPlaceStep::Paren(arguments) => {
                    let grown = self.grow_nested_paren_target(&target, arguments, location)?;
                    if is_object_value(&grown) {
                        self.index_object_value(&grown, arguments, location)?
                    } else {
                        self.index_aggregate_value(&grown, arguments, false, location)?
                    }
                }
                ResolvedPlaceStep::Brace(arguments) => {
                    let values = self.brace_apply(&target, arguments, location)?;
                    self.single_place_prefix_value(values, location)?
                }
                ResolvedPlaceStep::Field(field) => {
                    let values = stack
                        .run(|stack| {
                            self.aggregate_field_pack(stack, &target, field, context, location)
                        })
                        .await?;
                    self.single_place_prefix_value(values, location)?
                }
            };
        }
        match last {
            ResolvedPlaceStep::Paren(_) => Ok(1),
            ResolvedPlaceStep::Brace(arguments) => {
                let empty = Shape::new([0, 0])
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location))?;
                let shape = match &target {
                    Value::Cell(cell) => cell.shape(),
                    Value::Nothing => &empty,
                    _ => {
                        return Err(self.aggregate_type_error(
                            "output destination count",
                            "cell",
                            &target,
                            location,
                        ));
                    }
                };
                Ok(self
                    .selection_with_scalar_growth(
                        shape,
                        arguments,
                        "output destination count",
                        location,
                    )?
                    .0
                    .offsets
                    .len())
            }
            ResolvedPlaceStep::Field(_) => match &target {
                Value::Struct(structure) => usize::try_from(structure.numel())
                    .map_err(|_| self.error(array_error(ArrayRuntimeError::SizeLimit), location)),
                Value::Nothing | Value::Object(_) | Value::Table(_) => Ok(1),
                _ => Err(self.aggregate_type_error(
                    "output destination count",
                    "struct or scalar object",
                    &target,
                    location,
                )),
            },
        }
    }

    fn single_place_prefix_value(
        &self,
        mut values: Vec<Value>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if values.len() != 1 {
            return Err(self.error(
                array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                    selected: u64::try_from(values.len()).unwrap_or(u64::MAX),
                    supplied: 1,
                }),
                location,
            ));
        }
        Ok(values.remove(0))
    }

    fn read_register<'a>(
        &self,
        frame: &'a ExecutionFrame,
        register: Register,
    ) -> RuntimeResult<&'a Value> {
        frame
            .read_register(register)
            .ok_or_else(|| self.invalid_state("register index escaped validation"))
    }

    fn write_register(
        &self,
        frame: &mut ExecutionFrame,
        register: Register,
        value: Value,
    ) -> RuntimeResult<()> {
        if frame.write_register(register, value) {
            Ok(())
        } else {
            Err(self.invalid_state("register index escaped validation"))
        }
    }

    fn take_register(
        &self,
        frame: &mut ExecutionFrame,
        register: Register,
    ) -> RuntimeResult<Value> {
        frame
            .take_register(register)
            .ok_or_else(|| self.invalid_state("register index escaped validation"))
    }

    fn read_pack_register<'a>(
        &self,
        frame: &'a ExecutionFrame,
        register: PackRegister,
    ) -> RuntimeResult<&'a [Value]> {
        frame
            .read_pack_register(register)
            .ok_or_else(|| self.invalid_state("pack register index escaped validation"))
    }

    fn write_pack_register(
        &self,
        frame: &mut ExecutionFrame,
        register: PackRegister,
        values: Vec<Value>,
    ) -> RuntimeResult<()> {
        if frame.write_pack_register(register, values) {
            Ok(())
        } else {
            Err(self.invalid_state("pack register index escaped validation"))
        }
    }

    fn take_pack_register(
        &self,
        frame: &mut ExecutionFrame,
        register: PackRegister,
    ) -> RuntimeResult<Vec<Value>> {
        frame
            .take_pack_register(register)
            .ok_or_else(|| self.invalid_state("pack register index escaped validation"))
    }

    fn check_cancelled(&self, location: Option<SourceLocation>) -> RuntimeResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(RuntimeErrorKind::Cancelled, location))
        } else {
            Ok(())
        }
    }

    fn update_stack(&mut self, instruction: InstructionIndex, location: Option<SourceLocation>) {
        if let Some(frame) = self.call_stack.last_mut() {
            frame.instruction = instruction;
            frame.location = location;
        }
    }

    fn invalid_state(&self, message: &str) -> RuntimeError {
        self.error(
            RuntimeErrorKind::InvalidExecutionState {
                message: message.to_owned(),
                array: None,
            },
            self.call_stack.last().and_then(|frame| frame.location),
        )
    }

    fn error(&self, kind: RuntimeErrorKind, location: Option<SourceLocation>) -> RuntimeError {
        RuntimeError {
            kind,
            location,
            stack: self.call_stack.clone(),
        }
    }
}

fn is_matlab_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn platform_name_eq(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn platform_path_eq(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    #[cfg(windows)]
    {
        fn comparable(path: &Path) -> String {
            let text = path.to_string_lossy();
            if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
                format!(r"\\{rest}")
            } else {
                text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
            }
        }
        comparable(&left).eq_ignore_ascii_case(&comparable(&right))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn function_shares_source(
    module: &BytecodeModule,
    function: FunctionId,
    location: Option<SourceLocation>,
) -> bool {
    let Some(source_id) = location.map(|location| location.source_id) else {
        return true;
    };
    get(&module.functions, function.get())
        .and_then(|function| {
            function
                .instructions
                .iter()
                .find_map(|instruction| instruction.location)
        })
        .is_none_or(|location| location.source_id == source_id)
}

fn trace_gc_value(
    value: &Value,
    function_handles: &BTreeMap<BytecodeFunctionHandle, RuntimeFunctionCode>,
    visited_functions: &mut BTreeSet<BytecodeFunctionHandle>,
    visit_object: &mut dyn FnMut(ObjectHandle),
) {
    let mut pending_functions = Vec::new();
    value.trace_handles(&mut |handle| visit_object(handle), &mut |handle| {
        pending_functions.push(handle);
    });

    while let Some(handle) = pending_functions.pop() {
        if !visited_functions.insert(handle) {
            continue;
        }
        let Some(code) = function_handles.get(&handle) else {
            continue;
        };
        for binding in code.captures.values() {
            let Some(value) = binding.gc_value() else {
                continue;
            };
            value.trace_handles(&mut |handle| visit_object(handle), &mut |handle| {
                pending_functions.push(handle);
            });
        }
    }
}

fn register_is_read_elsewhere(
    function: &Function,
    current: InstructionIndex,
    register: Register,
) -> bool {
    let current = usize::try_from(current.get()).unwrap_or(usize::MAX);
    function
        .instructions
        .iter()
        .enumerate()
        .any(|(index, instruction)| {
            index != current && instruction_reads_register(&instruction.kind, register)
        })
}

fn pack_register_is_read_elsewhere(
    function: &Function,
    current: InstructionIndex,
    register: PackRegister,
) -> bool {
    let current = usize::try_from(current.get()).unwrap_or(usize::MAX);
    function
        .instructions
        .iter()
        .enumerate()
        .any(|(index, instruction)| {
            index != current && instruction_reads_pack_register(&instruction.kind, register)
        })
}

#[allow(clippy::too_many_lines)]
fn instruction_reads_register(instruction: &InstructionKind, register: Register) -> bool {
    let apply = |arguments: &[ApplyArgument]| {
        arguments
            .iter()
            .any(|argument| matches!(argument, ApplyArgument::Value(value) if *value == register))
    };
    let source =
        |source: &ValueSource| matches!(source, ValueSource::One(value) if *value == register);
    let field =
        |field: &FieldOperand| matches!(field, FieldOperand::Dynamic(value) if *value == register);
    let path = |path: &[PlaceStep]| {
        path.iter().any(|step| match step {
            PlaceStep::Paren(arguments) | PlaceStep::Brace(arguments) => apply(arguments),
            PlaceStep::Field(operand) => field(operand),
        })
    };
    match instruction {
        InstructionKind::Move { src, .. }
        | InstructionKind::StoreLocal { src, .. }
        | InstructionKind::StorePersistent { src, .. }
        | InstructionKind::StoreCapture { src, .. }
        | InstructionKind::StoreGlobal { src, .. }
        | InstructionKind::StatementValue { src, .. }
        | InstructionKind::Display { src, .. } => *src == register,
        InstructionKind::Binary { lhs, rhs, .. } => *lhs == register || *rhs == register,
        InstructionKind::SwitchMatch {
            selector,
            case_value,
            ..
        } => *selector == register || *case_value == register,
        InstructionKind::BuildMatrix { rows, .. } => {
            rows.iter().flatten().any(|value| *value == register)
        }
        InstructionKind::BuildCell { rows, .. } => rows.iter().flatten().any(source),
        InstructionKind::Range {
            start, step, end, ..
        } => *start == register || *step == register || *end == register,
        InstructionKind::Transpose { operand, .. } => *operand == register,
        InstructionKind::ApplyBinding {
            target, arguments, ..
        }
        | InstructionKind::Apply {
            target, arguments, ..
        }
        | InstructionKind::StatementApply {
            target, arguments, ..
        }
        | InstructionKind::ApplyQualified {
            target, arguments, ..
        }
        | InstructionKind::StatementApplyQualified {
            target, arguments, ..
        }
        | InstructionKind::ReturnApply { target, arguments } => {
            let suffix = match instruction {
                InstructionKind::ApplyQualified {
                    unresolved_suffix, ..
                }
                | InstructionKind::StatementApplyQualified {
                    unresolved_suffix, ..
                } => *unresolved_suffix == register,
                _ => false,
            };
            *target == register || suffix || apply(arguments)
        }
        InstructionKind::MakeClosure { captures, .. } => captures
            .iter()
            .any(|(_, value, _)| value.is_some_and(|value| value == register)),
        InstructionKind::InvokeSuperclassConstructor { arguments, .. } => {
            arguments.contains(&register)
        }
        InstructionKind::GetField { object, .. } => *object == register,
        InstructionKind::GetQualifiedPack {
            target,
            root_found,
            unresolved_suffix,
            ..
        } => *target == register || *root_found == register || *unresolved_suffix == register,
        InstructionKind::SetField { object, value, .. } => {
            *object == register || *value == register
        }
        InstructionKind::ApplyField {
            object, arguments, ..
        }
        | InstructionKind::StatementApplyField {
            object, arguments, ..
        }
        | InstructionKind::ReturnApplyField {
            object, arguments, ..
        } => *object == register || apply(arguments),
        InstructionKind::BraceApply {
            target, arguments, ..
        } => *target == register || apply(arguments),
        InstructionKind::GetAggregateField {
            target,
            field: name,
            ..
        } => *target == register || field(name),
        InstructionKind::AssignPlace {
            root,
            path: steps,
            source: value,
            ..
        }
        | InstructionKind::AssignBindingPlace {
            root,
            path: steps,
            source: value,
            ..
        } => *root == register || path(steps) || source(value),
        InstructionKind::RequireDefined { value, .. } => *value == register,
        InstructionKind::CountPlaceOutputs {
            root, path: steps, ..
        } => *root == register || path(steps),
        InstructionKind::SlicePack { start, count, .. } => *start == register || *count == register,
        InstructionKind::ApplyOutputPack {
            count,
            target,
            arguments,
            ..
        } => {
            *count == register
                || apply(arguments)
                || match target {
                    PackApplyTarget::Value(value) => *value == register,
                    PackApplyTarget::Field { object, .. } => *object == register,
                    PackApplyTarget::Qualified {
                        target,
                        unresolved_suffix,
                        ..
                    } => *target == register || *unresolved_suffix == register,
                }
        }
        InstructionKind::ResolveEnd { target, .. } => *target == register,
        InstructionKind::JumpIfFalse { condition, .. } => *condition == register,
        InstructionKind::Call {
            callee, arguments, ..
        } => *callee == register || arguments.contains(&register),
        InstructionKind::IndexAssign {
            target,
            arguments,
            value,
            ..
        } => *target == register || *value == register || apply(arguments),
        InstructionKind::ForEach {
            iterable, index, ..
        } => *iterable == register || *index == register,
        InstructionKind::Return { values } => values.contains(&register),
        InstructionKind::ReturnVariadic { fixed, variadic } => {
            fixed.contains(&register) || *variadic == register
        }
        InstructionKind::DeclareImports { .. }
        | InstructionKind::ClearImports
        | InstructionKind::DeclareNamedBindings { .. }
        | InstructionKind::LoadConstant { .. }
        | InstructionKind::LoadLocal { .. }
        | InstructionKind::DeclareGlobal { .. }
        | InstructionKind::DeclarePersistent { .. }
        | InstructionKind::LoadPersistent { .. }
        | InstructionKind::LoadCallInputCount { .. }
        | InstructionKind::LoadCallOutputCount { .. }
        | InstructionKind::LoadVariadicInputs { .. }
        | InstructionKind::LoadGlobal { .. }
        | InstructionKind::LoadCallTarget { .. }
        | InstructionKind::LoadQualifiedTarget { .. }
        | InstructionKind::LoadFunctionHandle { .. }
        | InstructionKind::LoadFunctionHandleCandidates { .. }
        | InstructionKind::MakeSharedClosure { .. }
        | InstructionKind::LoadCapture { .. }
        | InstructionKind::LoadGlobalOrNothing { .. }
        | InstructionKind::ClearGlobal { .. }
        | InstructionKind::ClearGlobalAll
        | InstructionKind::ClearLocal { .. }
        | InstructionKind::ClearCapture { .. }
        | InstructionKind::ClearDynamicBindings { .. }
        | InstructionKind::RegisterClass { .. }
        | InstructionKind::RequireSinglePack { .. }
        | InstructionKind::Unpack { .. }
        | InstructionKind::Jump { .. }
        | InstructionKind::StatementPack { .. } => false,
    }
}

fn instruction_reads_pack_register(instruction: &InstructionKind, register: PackRegister) -> bool {
    let apply = |arguments: &[ApplyArgument]| {
        arguments
            .iter()
            .any(|argument| matches!(argument, ApplyArgument::Expand(value) if *value == register))
    };
    let source =
        |source: &ValueSource| matches!(source, ValueSource::Expand(value) if *value == register);
    let path = |path: &[PlaceStep]| {
        path.iter().any(|step| match step {
            PlaceStep::Paren(arguments) | PlaceStep::Brace(arguments) => apply(arguments),
            PlaceStep::Field(_) => false,
        })
    };
    match instruction {
        InstructionKind::BuildCell { rows, .. } => rows.iter().flatten().any(source),
        InstructionKind::CountPlaceOutputs { path: steps, .. } => path(steps),
        InstructionKind::SlicePack { source, .. } => *source == register,
        InstructionKind::ApplyOutputPack { arguments, .. }
        | InstructionKind::ApplyBinding { arguments, .. }
        | InstructionKind::ApplyField { arguments, .. }
        | InstructionKind::ApplyQualified { arguments, .. }
        | InstructionKind::BraceApply { arguments, .. }
        | InstructionKind::Apply { arguments, .. }
        | InstructionKind::StatementApply { arguments, .. }
        | InstructionKind::StatementApplyField { arguments, .. }
        | InstructionKind::StatementApplyQualified { arguments, .. }
        | InstructionKind::ReturnApply { arguments, .. }
        | InstructionKind::ReturnApplyField { arguments, .. }
        | InstructionKind::IndexAssign { arguments, .. } => apply(arguments),
        InstructionKind::AssignPlace {
            path: steps,
            source: value,
            ..
        }
        | InstructionKind::AssignBindingPlace {
            path: steps,
            source: value,
            ..
        } => path(steps) || source(value),
        InstructionKind::Unpack { pack, .. }
        | InstructionKind::StatementPack { pack, .. }
        | InstructionKind::RequireSinglePack { pack } => *pack == register,
        InstructionKind::RequireDefined { .. }
        | InstructionKind::DeclareImports { .. }
        | InstructionKind::ClearImports
        | InstructionKind::DeclareNamedBindings { .. }
        | InstructionKind::LoadConstant { .. }
        | InstructionKind::Move { .. }
        | InstructionKind::Binary { .. }
        | InstructionKind::SwitchMatch { .. }
        | InstructionKind::BuildMatrix { .. }
        | InstructionKind::Range { .. }
        | InstructionKind::Transpose { .. }
        | InstructionKind::LoadLocal { .. }
        | InstructionKind::StoreLocal { .. }
        | InstructionKind::DeclareGlobal { .. }
        | InstructionKind::DeclarePersistent { .. }
        | InstructionKind::LoadPersistent { .. }
        | InstructionKind::StorePersistent { .. }
        | InstructionKind::LoadCallInputCount { .. }
        | InstructionKind::LoadCallOutputCount { .. }
        | InstructionKind::LoadVariadicInputs { .. }
        | InstructionKind::LoadGlobal { .. }
        | InstructionKind::LoadCallTarget { .. }
        | InstructionKind::LoadQualifiedTarget { .. }
        | InstructionKind::LoadFunctionHandle { .. }
        | InstructionKind::LoadFunctionHandleCandidates { .. }
        | InstructionKind::MakeClosure { .. }
        | InstructionKind::MakeSharedClosure { .. }
        | InstructionKind::LoadCapture { .. }
        | InstructionKind::StoreCapture { .. }
        | InstructionKind::LoadGlobalOrNothing { .. }
        | InstructionKind::StoreGlobal { .. }
        | InstructionKind::ClearGlobal { .. }
        | InstructionKind::ClearGlobalAll
        | InstructionKind::ClearLocal { .. }
        | InstructionKind::ClearCapture { .. }
        | InstructionKind::ClearDynamicBindings { .. }
        | InstructionKind::RegisterClass { .. }
        | InstructionKind::InvokeSuperclassConstructor { .. }
        | InstructionKind::GetField { .. }
        | InstructionKind::GetQualifiedPack { .. }
        | InstructionKind::SetField { .. }
        | InstructionKind::GetAggregateField { .. }
        | InstructionKind::ResolveEnd { .. }
        | InstructionKind::Jump { .. }
        | InstructionKind::JumpIfFalse { .. }
        | InstructionKind::Call { .. }
        | InstructionKind::StatementValue { .. }
        | InstructionKind::Display { .. }
        | InstructionKind::ForEach { .. }
        | InstructionKind::Return { .. }
        | InstructionKind::ReturnVariadic { .. } => false,
    }
}

fn imported_name_candidates(imports: &[String], name: &str) -> Vec<String> {
    imports
        .iter()
        .filter_map(|import| {
            if let Some(package) = import.strip_suffix(".*") {
                Some(format!("{package}.{name}"))
            } else {
                import
                    .rsplit_once('.')
                    .is_some_and(|(_, leaf)| leaf == name)
                    .then(|| import.clone())
            }
        })
        .collect()
}

fn qualified_candidate_visible(
    root: &str,
    candidate: &str,
    member_count: u32,
    imports: &[String],
) -> bool {
    let mut components = candidate.split('.');
    let first = components.next().unwrap_or_default();
    let component_count = 1_usize.saturating_add(components.count());
    let direct_count = usize::try_from(member_count)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    if first == root && component_count == direct_count {
        return true;
    }
    imports
        .iter()
        .any(|import| import_candidate_matches(import, candidate, root))
}

fn import_candidate_matches(import: &str, candidate: &str, root: &str) -> bool {
    if let Some(package) = import.strip_suffix(".*") {
        let imported_root = format!("{package}.{root}");
        candidate == imported_root
            || candidate
                .strip_prefix(&imported_root)
                .is_some_and(|suffix| suffix.starts_with('.'))
    } else {
        import
            .rsplit_once('.')
            .is_some_and(|(_, leaf)| leaf == root)
            && (candidate == import
                || candidate
                    .strip_prefix(import)
                    .is_some_and(|suffix| suffix.starts_with('.')))
    }
}

fn builtin_failure_kind(name: &str, error: crate::BuiltinError) -> RuntimeErrorKind {
    match error.category {
        BuiltinErrorCategory::Cancelled => RuntimeErrorKind::Cancelled,
        _ => RuntimeErrorKind::Builtin {
            name: name.to_string(),
            category: error.category,
            identifier: error.identifier,
            message: error.message,
        },
    }
}

fn intrinsic_named(name: &str) -> Option<IntrinsicFunction> {
    match name {
        "openmat_ui" => Some(IntrinsicFunction::OpenMatUi),
        "ffi" => Some(IntrinsicFunction::Ffi),
        "class" => Some(IntrinsicFunction::Class),
        "isa" | "__openmat_argument_isa" => Some(IntrinsicFunction::Isa),
        "isobject" => Some(IntrinsicFunction::IsObject),
        "feval" => Some(IntrinsicFunction::Feval),
        "eval" => Some(IntrinsicFunction::Eval),
        "evalc" => Some(IntrinsicFunction::EvalC),
        "evalin" => Some(IntrinsicFunction::EvalIn),
        "assignin" => Some(IntrinsicFunction::AssignIn),
        "str2func" => Some(IntrinsicFunction::Str2Func),
        "func2str" => Some(IntrinsicFunction::Func2Str),
        "tic" => Some(IntrinsicFunction::Tic),
        "toc" => Some(IntrinsicFunction::Toc),
        "lasterr" => Some(IntrinsicFunction::LastErr),
        "lasterror" => Some(IntrinsicFunction::LastError),
        "throw" => Some(IntrinsicFunction::Throw),
        "throwAsCaller" => Some(IntrinsicFunction::ThrowAsCaller),
        "rethrow" => Some(IntrinsicFunction::Rethrow),
        "addCause" => Some(IntrinsicFunction::AddCause),
        "getReport" => Some(IntrinsicFunction::GetReport),
        _ => None,
    }
}

fn event_runtime_named(name: &str) -> bool {
    matches!(name, "addlistener" | "notify")
}

const fn intrinsic_name(intrinsic: IntrinsicFunction) -> &'static str {
    match intrinsic {
        IntrinsicFunction::OpenMatUi => "openmat_ui",
        IntrinsicFunction::Ffi => "ffi",
        IntrinsicFunction::Class => "class",
        IntrinsicFunction::Isa => "isa",
        IntrinsicFunction::IsObject => "isobject",
        IntrinsicFunction::Feval => "feval",
        IntrinsicFunction::Eval => "eval",
        IntrinsicFunction::EvalC => "evalc",
        IntrinsicFunction::EvalIn => "evalin",
        IntrinsicFunction::AssignIn => "assignin",
        IntrinsicFunction::Str2Func => "str2func",
        IntrinsicFunction::Func2Str => "func2str",
        IntrinsicFunction::Tic => "tic",
        IntrinsicFunction::Toc => "toc",
        IntrinsicFunction::LastErr => "lasterr",
        IntrinsicFunction::LastError => "lasterror",
        IntrinsicFunction::Throw => "throw",
        IntrinsicFunction::ThrowAsCaller => "throwAsCaller",
        IntrinsicFunction::Rethrow => "rethrow",
        IntrinsicFunction::AddCause => "addCause",
        IntrinsicFunction::GetReport => "getReport",
    }
}

fn is_exception_identifier(identifier: &str) -> bool {
    identifier.contains(':')
        && identifier.split(':').all(|part| {
            let mut characters = part.chars();
            characters
                .next()
                .is_some_and(|first| first == '_' || first.is_alphabetic())
                && characters.all(|character| character == '_' || character.is_alphanumeric())
        })
}

fn innermost_exception_handler(
    function: &Function,
    instruction: InstructionIndex,
) -> Option<ExceptionHandler> {
    function
        .exception_handlers
        .iter()
        .copied()
        .filter(|handler| {
            handler.protected_start.get() <= instruction.get()
                && instruction.get() < handler.protected_end.get()
        })
        .min_by_key(|handler| {
            handler
                .protected_end
                .get()
                .saturating_sub(handler.protected_start.get())
        })
}

const fn object_access(access: BytecodeAccess) -> ObjectAccess {
    match access {
        BytecodeAccess::Public => ObjectAccess::Public,
        BytecodeAccess::Protected => ObjectAccess::Protected,
        BytecodeAccess::Private => ObjectAccess::Private,
    }
}

fn object_handle(value: &Value) -> Option<ObjectHandle> {
    match value {
        Value::Object(handle) => Some(*handle),
        Value::ObjectArray(array) if array.numel() == 1 => array.as_slice().first().copied(),
        _ => None,
    }
}

/// Keeps the complete bytecode-to-class-operator vocabulary in one place.
const fn object_operator(operator: BinaryOperator) -> Operator {
    match operator {
        BinaryOperator::Add => Operator::Add,
        BinaryOperator::Subtract => Operator::Subtract,
        BinaryOperator::Multiply => Operator::MatrixMultiply,
        BinaryOperator::ElementMultiply => Operator::ElementWiseMultiply,
        BinaryOperator::Divide => Operator::MatrixRightDivide,
        BinaryOperator::LeftDivide => Operator::MatrixLeftDivide,
        BinaryOperator::ElementDivide => Operator::ElementWiseRightDivide,
        BinaryOperator::ElementLeftDivide => Operator::ElementWiseLeftDivide,
        BinaryOperator::Power => Operator::MatrixPower,
        BinaryOperator::ElementPower => Operator::ElementWisePower,
        BinaryOperator::Equal => Operator::Equal,
        BinaryOperator::NotEqual => Operator::NotEqual,
        BinaryOperator::LessThan => Operator::LessThan,
        BinaryOperator::LessThanOrEqual => Operator::LessThanOrEqual,
        BinaryOperator::GreaterThan => Operator::GreaterThan,
        BinaryOperator::GreaterThanOrEqual => Operator::GreaterThanOrEqual,
    }
}

const fn class_handle(value: &Value) -> Option<ClassHandle> {
    match value {
        Value::Function(FunctionHandle::Class(handle)) => Some(*handle),
        _ => None,
    }
}

fn is_object_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Table(_)
            | Value::Object(_)
            | Value::ObjectArray(_)
            | Value::Graphics(_)
            | Value::GraphicsArray(_)
    )
}

fn is_table_numeric_index(value: &Value) -> bool {
    matches!(
        value,
        Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Logical(_)
                    | ArrayData::Integer(_)
            )
    )
}

fn is_empty_double(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(ArrayData::F64(array))
            if array.shape().dimensions() == [0, 0] && array.numel() == 0
    )
}

fn empty_double() -> Result<Value, ObjectError> {
    let shape = Shape::new([0, 0]).map_err(|_| ObjectError::ShapeOverflow)?;
    let array =
        DenseArray::from_vec(shape, Vec::<f64>::new()).map_err(|_| ObjectError::ShapeOverflow)?;
    Ok(Value::Array(ArrayData::F64(array)))
}

fn unknown_object_error(
    stack: &[StackTraceFrame],
    handle: ObjectHandle,
    operation: &'static str,
    location: Option<SourceLocation>,
) -> RuntimeError {
    RuntimeError {
        kind: RuntimeErrorKind::Object {
            operation,
            message: format!("unknown object handle {}", handle.identifier()),
        },
        location,
        stack: stack.to_vec(),
    }
}

fn global_name(function: &Function, constant: openmat_bytecode::ConstantId) -> Option<&str> {
    match get(&function.constants, constant.get()) {
        Some(Constant::String(name)) => Some(name),
        _ => None,
    }
}

fn module_function_named(module: &BytecodeModule, name: &str) -> Option<FunctionId> {
    module
        .functions
        .iter()
        .position(|function| function.name == name)
        .and_then(|index| u32::try_from(index).ok())
        .map(FunctionId::new)
}

fn persistent_binding_key(
    function: &Function,
    frame: &ExecutionFrame,
    slot: PersistentSlot,
) -> PersistentBindingKey {
    PersistentBindingKey {
        function_name: function.name.clone(),
        caller_class: frame.access_context.caller_class(),
        slot,
    }
}

fn get<T>(items: &[T], index: u32) -> Option<&T> {
    usize::try_from(index)
        .ok()
        .and_then(|converted| items.get(converted))
}

fn get_mut<T>(items: &mut [T], index: u32) -> Option<&mut T> {
    usize::try_from(index)
        .ok()
        .and_then(|converted| items.get_mut(converted))
}

#[cfg(test)]
mod gc_tests;

#[cfg(test)]
mod tests {
    use openmat_array::{ArrayData, Logical};
    use openmat_bytecode::{BinaryOperator, Instruction, InstructionKind};
    use openmat_object::Operator;

    use super::*;

    fn interpreter() -> Interpreter {
        let mut entry = Function::new("<table-runtime-test>", 0, 0, 0);
        entry
            .instructions
            .push(Instruction::new(InstructionKind::Return {
                values: Vec::new(),
            }));
        Interpreter::new(BytecodeModule::new(vec![entry], FunctionId::new(0))).unwrap()
    }

    fn table_name(name: &str) -> TableVariableName {
        TableVariableName::new(name).unwrap()
    }

    fn double_array(dimensions: [u64; 2], values: Vec<f64>) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    fn single_array(dimensions: [u64; 2], values: Vec<f32>) -> Value {
        Value::Array(ArrayData::F32(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    fn logical_array(dimensions: [u64; 2], values: &[bool]) -> Value {
        Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                Shape::new(dimensions).unwrap(),
                values.iter().copied().map(Logical::from).collect(),
            )
            .unwrap(),
        ))
    }

    fn char_row(value: &str) -> Value {
        let values = value
            .encode_utf16()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        Value::Array(ArrayData::Char(
            DenseArray::from_vec(Shape::new([1, values.len() as u64]).unwrap(), values).unwrap(),
        ))
    }

    fn cellstr(values: &[&str]) -> Value {
        Value::Cell(
            CellArray::from_values(
                Shape::new([1, values.len() as u64]).unwrap(),
                values.iter().map(|value| char_row(value)).collect(),
            )
            .unwrap(),
        )
    }

    fn sample_table() -> TableArray {
        TableArray::from_variables(
            vec![table_name("A"), table_name("B")],
            vec![
                double_array([3, 1], vec![10.0, 20.0, 30.0]),
                single_array([3, 2], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
            ],
        )
        .unwrap()
    }

    fn char_text(value: &Value) -> String {
        let Value::Array(ArrayData::Char(array)) = value else {
            panic!("expected a char array")
        };
        String::from_utf16(
            &array
                .as_slice()
                .iter()
                .map(|value| value.get())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn division_object_vocabulary_preserves_direction_and_element_kind() {
        assert_eq!(
            object_operator(BinaryOperator::Divide),
            Operator::MatrixRightDivide
        );
        assert_eq!(
            object_operator(BinaryOperator::ElementDivide),
            Operator::ElementWiseRightDivide
        );
        assert_eq!(
            object_operator(BinaryOperator::LeftDivide),
            Operator::MatrixLeftDivide
        );
        assert_eq!(
            object_operator(BinaryOperator::ElementLeftDivide),
            Operator::ElementWiseLeftDivide
        );
    }

    #[test]
    fn table_fields_return_complete_variables_and_read_only_properties() {
        let mut runtime = interpreter();
        let table = sample_table();
        let value = Value::Table(table.clone());

        let variable = runtime
            .get_field(&value, "B", AccessContext::external(), None)
            .unwrap();
        assert_eq!(variable.class_name(), "single");
        assert_eq!(variable.dimensions(), Some([3, 2].as_slice()));
        let packed = super::frames::run(async |stack| {
            runtime
                .aggregate_field_pack(stack, &value, "A", AccessContext::external(), None)
                .await
        })
        .unwrap();
        assert!(matches!(
            packed.as_slice(),
            [Value::Array(ArrayData::F64(_))]
        ));

        let properties = runtime
            .get_field(&value, "Properties", AccessContext::external(), None)
            .unwrap();
        let Value::Struct(properties) = properties else {
            panic!("Properties must be a scalar struct")
        };
        assert_eq!(properties.shape().dimensions(), &[1, 1]);
        let names = properties
            .value_at(properties.field_index("VariableNames").unwrap(), 0)
            .unwrap();
        let Value::Cell(names) = names else {
            panic!("VariableNames must be cellstr")
        };
        assert_eq!(names.shape().dimensions(), &[1, 2]);
        assert_eq!(char_text(&names.values()[0]), "A");
        assert_eq!(char_text(&names.values()[1]), "B");

        let error = runtime
            .get_field(&value, "Missing", AccessContext::external(), None)
            .unwrap_err();
        assert!(matches!(
            error.kind,
            RuntimeErrorKind::Object {
                operation: "table variable read",
                ref message,
            } if message.contains("Missing")
        ));
        assert!(
            runtime
                .set_field(
                    &value,
                    "Properties",
                    &Value::Double(1.0),
                    AccessContext::external(),
                    None,
                )
                .is_err()
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn table_parentheses_support_colon_numeric_logical_and_name_selectors() {
        let mut runtime = interpreter();
        let table = sample_table();
        let module = Arc::clone(&runtime.module);

        let all_by_name = super::frames::run(async |stack| {
            runtime
                .apply_value(
                    stack,
                    &module,
                    &Value::Table(table.clone()),
                    &[IndexInput::Colon, IndexInput::Value(cellstr(&["B", "A"]))],
                    1,
                    AccessContext::external(),
                    None,
                )
                .await
        })
        .unwrap()
        .pop()
        .unwrap();
        let Value::Table(all_by_name) = all_by_name else {
            panic!("parentheses must preserve table class")
        };
        assert_eq!(all_by_name.shape().dimensions(), &[3, 2]);
        assert_eq!(
            all_by_name.variable_names(),
            &[table_name("B"), table_name("A")]
        );
        let logical_rows = logical_array([3, 1], &[true, false, true]);
        let selected = super::frames::run(async |stack| {
            runtime
                .apply_value(
                    stack,
                    &module,
                    &Value::Table(table.clone()),
                    &[
                        IndexInput::Value(logical_rows),
                        IndexInput::Value(Value::from("B")),
                    ],
                    1,
                    AccessContext::external(),
                    None,
                )
                .await
        })
        .unwrap()
        .pop()
        .unwrap();
        let Value::Table(selected) = selected else {
            panic!("parentheses must return a table")
        };
        assert_eq!(selected.shape().dimensions(), &[2, 1]);
        let Value::Array(ArrayData::F32(variable)) = selected.variable(0).unwrap() else {
            panic!("row indexing must preserve single")
        };
        assert_eq!(variable.shape().dimensions(), &[2, 2]);
        assert_eq!(variable.as_slice(), &[1.0, 3.0, 4.0, 6.0]);

        let numeric_variable = super::frames::run(async |stack| {
            runtime
                .apply_value(
                    stack,
                    &module,
                    &Value::Table(table.clone()),
                    &[
                        IndexInput::Value(double_array([1, 2], vec![3.0, 1.0])),
                        IndexInput::Value(Value::Double(1.0)),
                    ],
                    1,
                    AccessContext::external(),
                    None,
                )
                .await
        })
        .unwrap()
        .pop()
        .unwrap();
        let Value::Table(numeric_variable) = numeric_variable else {
            panic!("parentheses must return a table")
        };
        assert_eq!(numeric_variable.shape().dimensions(), &[2, 1]);
        let Value::Array(ArrayData::F64(variable)) = numeric_variable.variable(0).unwrap() else {
            panic!("numeric table variable changed class")
        };
        assert_eq!(variable.as_slice(), &[30.0, 10.0]);

        let logical_variable = super::frames::run(async |stack| {
            runtime
                .apply_value(
                    stack,
                    &module,
                    &Value::Table(table.clone()),
                    &[
                        IndexInput::Colon,
                        IndexInput::Value(logical_array([1, 2], &[false, true])),
                    ],
                    1,
                    AccessContext::external(),
                    None,
                )
                .await
        })
        .unwrap()
        .pop()
        .unwrap();
        let Value::Table(logical_variable) = logical_variable else {
            panic!("parentheses must return a table")
        };
        assert_eq!(logical_variable.variable_names(), &[table_name("B")]);
    }

    #[test]
    fn table_braces_preserve_one_variable_and_losslessly_join_compatible_variables() {
        let mut runtime = interpreter();
        let table = TableArray::from_variables(
            vec![table_name("A"), table_name("C"), table_name("S")],
            vec![
                double_array([3, 1], vec![1.0, 2.0, 3.0]),
                double_array([3, 2], vec![4.0, 5.0, 6.0, 7.0, 8.0, 9.0]),
                single_array([3, 1], vec![10.0, 11.0, 12.0]),
            ],
        )
        .unwrap();

        let one = runtime
            .brace_apply(
                &Value::Table(table.clone()),
                &[
                    IndexInput::Value(double_array([1, 2], vec![3.0, 1.0])),
                    IndexInput::Value(Value::from("S")),
                ],
                None,
            )
            .unwrap();
        let [Value::Array(ArrayData::F32(one))] = one.as_slice() else {
            panic!("one selected variable must retain single storage")
        };
        assert_eq!(one.shape().dimensions(), &[2, 1]);
        assert_eq!(one.as_slice(), &[12.0, 10.0]);

        let joined = runtime
            .brace_apply(
                &Value::Table(table.clone()),
                &[IndexInput::Colon, IndexInput::Value(cellstr(&["A", "C"]))],
                None,
            )
            .unwrap();
        let [Value::Array(ArrayData::F64(joined))] = joined.as_slice() else {
            panic!("compatible double variables must concatenate")
        };
        assert_eq!(joined.shape().dimensions(), &[3, 3]);
        assert_eq!(
            joined.as_slice(),
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]
        );

        let error = runtime
            .brace_apply(
                &Value::Table(table),
                &[IndexInput::Colon, IndexInput::Value(cellstr(&["A", "S"]))],
                None,
            )
            .unwrap_err();
        assert!(matches!(
            error.kind,
            RuntimeErrorKind::Object {
                operation: "table brace indexing",
                ref message,
            } if message.contains("lossy")
        ));
    }

    #[test]
    fn table_variable_assignment_is_cow_and_failure_is_transactional() {
        let mut runtime = interpreter();
        let original = sample_table();
        let original_alias = original.clone();
        assert!(original.shares_storage_with(&original_alias));

        let replacement = AssignmentSource {
            values: vec![double_array([3, 1], vec![7.0, 8.0, 9.0])],
            expanded: false,
        };
        let replaced = super::frames::run(async |stack| {
            runtime
                .assign_final_field(
                    stack,
                    &Value::Table(original.clone()),
                    "A",
                    &replacement,
                    AssignmentMode::Store,
                    AccessContext::external(),
                    None,
                )
                .await
        })
        .unwrap();
        let Value::Table(replaced) = replaced else {
            panic!("table assignment must preserve table class")
        };
        assert!(original.shares_schema_with(&replaced));
        assert!(!original.shares_storage_with(&replaced));
        let Value::Array(ArrayData::F64(old)) = original.variable(0).unwrap() else {
            panic!()
        };
        let Value::Array(ArrayData::F64(new)) = replaced.variable(0).unwrap() else {
            panic!()
        };
        assert_eq!(old.as_slice(), &[10.0, 20.0, 30.0]);
        assert_eq!(new.as_slice(), &[7.0, 8.0, 9.0]);

        let appended = runtime
            .set_field(
                &Value::Table(original.clone()),
                "C",
                &logical_array([3, 1], &[true, false, true]),
                AccessContext::external(),
                None,
            )
            .unwrap();
        let Value::Table(appended) = appended else {
            panic!()
        };
        assert_eq!(appended.shape().dimensions(), &[3, 3]);
        assert_eq!(appended.variable_names()[2], table_name("C"));
        assert!(!original.shares_schema_with(&appended));

        let error = runtime
            .set_field(
                &Value::Table(original.clone()),
                "A",
                &double_array([2, 1], vec![1.0, 2.0]),
                AccessContext::external(),
                None,
            )
            .unwrap_err();
        assert!(matches!(
            error.kind,
            RuntimeErrorKind::Object {
                operation: "table variable assignment",
                ..
            }
        ));
        assert!(original.shares_storage_with(&original_alias));
        assert_eq!(original.variable(0), original_alias.variable(0));
    }

    #[test]
    fn table_schema_rename_is_atomic_and_preserves_variable_storage() {
        let mut runtime = interpreter();
        let original = sample_table();
        let rename = AssignmentSource {
            values: vec![cellstr(&["X", "Y"])],
            expanded: false,
        };
        let renamed = super::frames::run(async |stack| {
            runtime
                .assign_nested_field(
                    stack,
                    &Value::Table(original.clone()),
                    "Properties",
                    &[ResolvedPlaceStep::Field("VariableNames".to_owned())],
                    &rename,
                    AssignmentMode::Store,
                    AccessContext::external(),
                    None,
                )
                .await
        })
        .unwrap();
        let Value::Table(renamed) = renamed else {
            panic!("schema rename must preserve table class")
        };
        assert_eq!(
            renamed.variable_names(),
            &[table_name("X"), table_name("Y")]
        );
        assert!(original.shares_storage_with(&renamed));
        assert!(!original.shares_schema_with(&renamed));
        assert_eq!(
            original.variable_names(),
            &[table_name("A"), table_name("B")]
        );

        let duplicate = AssignmentSource {
            values: vec![cellstr(&["X", "X"])],
            expanded: false,
        };
        assert!(
            super::frames::run(async |stack| runtime
                .assign_nested_field(
                    stack,
                    &Value::Table(original.clone()),
                    "Properties",
                    &[ResolvedPlaceStep::Field("VariableNames".to_owned())],
                    &duplicate,
                    AssignmentMode::Store,
                    AccessContext::external(),
                    None,
                )
                .await)
            .is_err()
        );
        assert_eq!(
            original.variable_names(),
            &[table_name("A"), table_name("B")]
        );
    }

    #[test]
    fn table_variable_deletion_and_first_append_preserve_row_identity() {
        let mut runtime = interpreter();
        let one = TableArray::from_variables(
            vec![table_name("Only")],
            vec![double_array([3, 1], vec![1.0, 2.0, 3.0])],
        )
        .unwrap();
        let deletion = AssignmentSource {
            values: vec![Value::empty_double()],
            expanded: false,
        };
        let deleted = super::frames::run(async |stack| {
            runtime
                .assign_final_field(
                    stack,
                    &Value::Table(one),
                    "Only",
                    &deletion,
                    AssignmentMode::Delete,
                    AccessContext::external(),
                    None,
                )
                .await
        })
        .unwrap();
        let Value::Table(deleted) = deleted else {
            panic!("variable deletion must preserve table class")
        };
        assert_eq!(deleted.shape().dimensions(), &[3, 0]);

        let restored = runtime
            .set_field(
                &Value::Table(deleted.clone()),
                "Restored",
                &double_array([3, 1], vec![4.0, 5.0, 6.0]),
                AccessContext::external(),
                None,
            )
            .unwrap();
        let Value::Table(restored) = restored else {
            panic!()
        };
        assert_eq!(restored.shape().dimensions(), &[3, 1]);
        assert!(
            runtime
                .set_field(
                    &Value::Table(deleted.clone()),
                    "WrongHeight",
                    &double_array([2, 1], vec![1.0, 2.0]),
                    AccessContext::external(),
                    None,
                )
                .is_err()
        );
        assert_eq!(deleted.shape().dimensions(), &[3, 0]);

        let empty = TableArray::empty(0).unwrap();
        let established = runtime
            .set_field(
                &Value::Table(empty),
                "First",
                &logical_array([2, 1], &[true, false]),
                AccessContext::external(),
                None,
            )
            .unwrap();
        let Value::Table(established) = established else {
            panic!()
        };
        assert_eq!(established.shape().dimensions(), &[2, 1]);
    }

    #[test]
    fn table_dot_empty_assignment_deletes_a_variable_in_store_mode() {
        let mut runtime = interpreter();
        let original = sample_table();
        let source = AssignmentSource {
            values: vec![Value::empty_double()],
            expanded: false,
        };

        let assigned = super::frames::run(async |stack| {
            runtime
                .assign_final_field(
                    stack,
                    &Value::Table(original.clone()),
                    "B",
                    &source,
                    AssignmentMode::Store,
                    AccessContext::external(),
                    None,
                )
                .await
        })
        .unwrap();
        let Value::Table(assigned) = assigned else {
            panic!("table dot deletion must preserve table class")
        };

        assert_eq!(assigned.shape().dimensions(), &[3, 1]);
        assert_eq!(assigned.variable_names(), &[table_name("A")]);
        assert_eq!(original.shape().dimensions(), &[3, 2]);
        assert_eq!(
            original.variable_names(),
            &[table_name("A"), table_name("B")]
        );
    }
}

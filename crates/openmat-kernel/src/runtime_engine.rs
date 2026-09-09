use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use openmat_array::{
    ArrayData, Complex32, Complex64 as ArrayComplex64, ComplexInteger, DenseArray,
    IntegerArrayData, IntegerComponent, Shape,
};
use openmat_bytecode::{
    BytecodeModule, Constant, ConstantId, Function, FunctionId, InstructionKind, LocalSlot,
    NamedBindingKind, Register, SourceLocation,
};
use openmat_hir::{ExprKind, HirFile, Stmt, StmtKind};
use openmat_oex::OexPlugin;
use openmat_protocol::kernel_v1;
use openmat_protocol::kernel_v2::{self, ActivePathGuard};
use openmat_protocol::kernel_v3;
use openmat_protocol::{
    Capabilities, Diagnostic, DiagnosticSeverity, DisplayEvent, Event, ExecuteRequest,
    ExecuteResult, ExecutionMode, ImplementationInfo, InspectRequest, MAX_PREVIEW_ELEMENTS,
    MatrixPreview, PreviewTruncation, PreviewValue, RelatedDiagnostic, SourceRange, StreamEvent,
    StreamKind, VariableSummary, WorkspaceDeltaEvent,
};
use openmat_runtime::{
    ArrayRuntimeError, BuiltinErrorCategory, CancellationToken as RuntimeCancellationToken,
    ClassConstraintKind, GraphicsDelta, GraphicsNotice, GraphicsSession, Interpreter, LinalgError,
    LinalgProvider, LocalFileSystem, OutputError, OutputEvent, OutputSink, RuntimeBuildError,
    RuntimeError, RuntimeErrorKind, RuntimeLinalgError, SearchPath, WorkingDirectory, Workspace,
    format_display_value_with,
};
use openmat_source::{Diagnostic as SourceDiagnostic, Severity, SourceId, TextRange};
use openmat_value::{
    CellArray, SparseArrayData, SparseScalarValue, StringArray, StringElement, StringValue,
    StructArray, TableArray, Value, ValueKind,
};

#[cfg(test)]
use openmat_runtime::NumericFormat;

use crate::module_linker::{
    MergedModule, inline_script_entry, merge_support_module, prepend_entry_instructions,
    replace_load_with_function,
};
use crate::{CancellationToken, EngineError, ExecutionEngine, ExecutionOutput, FileSourceResolver};

const PREVIEW_MAX_TEXT_BYTES: usize = 4_096;
const COMMAND_WINDOW_CLEAR_MIME: &str = "application/vnd.openmat.command-window-clear+json";
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(2);

fn performance_logging_enabled() -> bool {
    std::env::var_os("OPENMAT_PERF_LOG").is_some_and(|value| value != "0")
}

fn duration_milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn canonical_oex_plugin_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, EngineError> {
    let mut canonical = Vec::new();
    canonical.try_reserve_exact(paths.len()).map_err(|_| {
        EngineError::new(
            "oex.load",
            "the OEX plugin path list exceeded runtime capacity",
        )
    })?;
    let mut unique = BTreeSet::new();
    for path in paths {
        let resolved = path.canonicalize().map_err(|error| {
            EngineError::new(
                "oex.load",
                format!("failed to resolve OEX plugin `{}`: {error}", path.display()),
            )
        })?;
        if !unique.insert(resolved.clone()) {
            return Err(EngineError::new(
                "oex.load",
                format!(
                    "OEX plugin `{}` was supplied more than once",
                    resolved.display()
                ),
            ));
        }
        canonical.push(resolved);
    }
    Ok(canonical)
}

/// Concrete parser/compiler/interpreter adapter used by application frontends.
///
/// The public boundary remains protocol-only. Compiler and runtime types are
/// contained inside this crate and do not leak into server or CLI APIs.
pub struct RuntimeEngine {
    interpreter: Interpreter,
    output_events: Arc<Mutex<Vec<OutputEvent>>>,
    pending_graphics_notices: Vec<GraphicsNotice>,
    file_resolver: FileSourceResolver,
    working_directory: WorkingDirectory,
    known_classes: BTreeSet<String>,
    sources: BTreeMap<u32, SourceRecord>,
    next_source_id: Option<u32>,
    shut_down: bool,
}

#[derive(Clone, Debug)]
struct SourceRecord {
    name: String,
    length: usize,
}

struct PreparedModule {
    module: BytecodeModule,
    class_names: BTreeSet<String>,
}

#[derive(Clone)]
struct CompiledSourceUnit {
    module: BytecodeModule,
    kind: SourceUnitKind,
    bare_script_sites: BTreeMap<SourceSite, BareScriptSite>,
}

#[derive(Clone, Debug)]
enum SourceUnitKind {
    Script,
    Function { name: String, function: FunctionId },
    Class { name: String },
}

#[derive(Clone, Debug)]
struct BareScriptSite {
    name: String,
    workspace_scope: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SourceSite {
    source_id: u32,
    start: u32,
    end: u32,
}

impl From<SourceLocation> for SourceSite {
    fn from(location: SourceLocation) -> Self {
        Self {
            source_id: location.source_id,
            start: location.start,
            end: location.end,
        }
    }
}

#[derive(Clone)]
enum LinkedSource {
    Function { name: String, function: FunctionId },
    Class { name: String },
    Script(Function),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NameLoadKind {
    Global,
    CallTarget,
    FunctionHandle,
    Qualified,
}

struct NameLoad {
    kind: NameLoadKind,
    destination: Register,
    names: Vec<String>,
    root: Option<String>,
    member_count: u32,
    location: Option<SourceLocation>,
}

struct LinkState {
    loaded: BTreeMap<PathBuf, LinkedSource>,
    function_origins: Vec<Option<PathBuf>>,
    class_origins: BTreeMap<String, Option<PathBuf>>,
    bare_script_sites: BTreeMap<SourceSite, BareScriptSite>,
    class_entries: Vec<Function>,
    class_entry_names: BTreeSet<String>,
}

impl RuntimeEngine {
    /// Constructs a concrete session engine and validates built-in
    /// registration.
    ///
    /// # Errors
    ///
    /// Returns an engine-initialization error when the minimal built-in set
    /// cannot be registered.
    pub fn new() -> Result<RuntimeEngine, EngineError> {
        Self::with_file_resolver(FileSourceResolver::empty())
    }

    /// Constructs an engine with deterministic MATLAB source search paths.
    ///
    /// # Errors
    ///
    /// Returns a stable `source.*` error for an invalid search directory, or
    /// an engine-initialization error when built-in registration fails.
    pub fn with_search_paths<I, P>(paths: I) -> Result<RuntimeEngine, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        Self::with_file_resolver(FileSourceResolver::new(paths)?)
    }

    /// Constructs an engine with deterministic source search paths and trusted OEX libraries.
    ///
    /// Plugin paths are canonicalized eagerly, loaded in the supplied order, and rejected when
    /// the same canonical library is supplied more than once.
    ///
    /// # Safety
    ///
    /// Native OEX libraries execute arbitrary in-process code. The caller must trust every
    /// library and ensure that it implements the OEX v1 C ABI and ownership contract.
    ///
    /// # Errors
    ///
    /// Returns a stable source-path, filesystem, OEX loading, or runtime-initialization error.
    pub unsafe fn with_search_paths_and_oex_plugins<I, P, J, Q>(
        paths: I,
        plugins: J,
    ) -> Result<RuntimeEngine, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
        J: IntoIterator<Item = Q>,
        Q: AsRef<Path>,
    {
        let file_resolver = FileSourceResolver::new(paths)?;
        let search_paths = file_resolver.search_paths()?;
        let working_directory = if let Some(path) = search_paths.first() {
            path.clone()
        } else {
            std::env::current_dir().map_err(|error| {
                EngineError::new(
                    "filesystem.workingDirectory",
                    format!("failed to resolve the session working directory: {error}"),
                )
            })?
        };
        let working_directory = WorkingDirectory::new(working_directory).map_err(|error| {
            EngineError::new(
                "filesystem.workingDirectory",
                format!("failed to initialize the session filesystem: {error}"),
            )
        })?;
        let plugins = plugins
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect::<Vec<_>>();
        // SAFETY: The method contract transfers trust in every supplied native library.
        unsafe {
            Self::build_with_working_directory_and_oex_plugins(
                file_resolver,
                working_directory,
                None,
                &plugins,
            )
        }
    }

    /// Constructs an engine with trusted native OEX libraries registered as language functions.
    ///
    /// Plugin paths are resolved eagerly and each DLL remains loaded for as long as one of its
    /// registered built-ins is reachable from the interpreter registry.
    ///
    /// # Safety
    ///
    /// Native OEX libraries execute arbitrary in-process code. The caller must trust every
    /// library and ensure that it implements the OEX v1 C ABI and ownership contract.
    ///
    /// # Errors
    ///
    /// Returns a stable `oex.load` error when a library cannot be loaded, initialized, or
    /// registered, or an engine-initialization error for the ordinary runtime components.
    pub unsafe fn with_oex_plugins<I, P>(plugins: I) -> Result<RuntimeEngine, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let working_directory = std::env::current_dir().map_err(|error| {
            EngineError::new(
                "filesystem.workingDirectory",
                format!("failed to resolve the session working directory: {error}"),
            )
        })?;
        // SAFETY: The method contract transfers trust in every supplied native library.
        unsafe { Self::with_working_directory_and_oex_plugins(working_directory, plugins) }
    }

    /// Constructs an engine with an explicit working directory and trusted OEX libraries.
    ///
    /// # Safety
    ///
    /// Native OEX libraries execute arbitrary in-process code. The caller must trust every
    /// library and ensure that it implements the OEX v1 C ABI and ownership contract.
    ///
    /// # Errors
    ///
    /// Returns a stable filesystem, OEX loading, or runtime-initialization error.
    pub unsafe fn with_working_directory_and_oex_plugins<I, P>(
        working_directory: impl AsRef<Path>,
        plugins: I,
    ) -> Result<RuntimeEngine, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let plugins = plugins
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect::<Vec<_>>();
        let working_directory = WorkingDirectory::new(working_directory).map_err(|error| {
            EngineError::new(
                "filesystem.workingDirectory",
                format!("failed to initialize the session filesystem: {error}"),
            )
        })?;
        // SAFETY: The public method contract requires every path to identify a trusted OEX DLL.
        unsafe {
            Self::build_with_working_directory_and_oex_plugins(
                FileSourceResolver::empty(),
                working_directory,
                None,
                &plugins,
            )
        }
    }

    /// Constructs an engine with an already validated file resolver.
    ///
    /// # Errors
    ///
    /// Returns an engine-initialization error when the minimal built-in set
    /// cannot be registered.
    pub fn with_file_resolver(
        file_resolver: FileSourceResolver,
    ) -> Result<RuntimeEngine, EngineError> {
        let search_paths = file_resolver.search_paths()?;
        let working_directory = if let Some(path) = search_paths.first() {
            path.clone()
        } else {
            std::env::current_dir().map_err(|error| {
                EngineError::new(
                    "filesystem.workingDirectory",
                    format!("failed to resolve the session working directory: {error}"),
                )
            })?
        };
        Self::with_file_resolver_and_working_directory(file_resolver, working_directory)
    }

    /// Constructs an engine whose language-level relative file paths resolve
    /// against an explicitly selected working directory.
    ///
    /// Source lookup remains empty; hosts that also require MATLAB source
    /// search paths should use their existing resolver configuration.
    ///
    /// # Errors
    ///
    /// Returns a stable initialization error for an invalid directory.
    pub fn with_working_directory(
        working_directory: impl AsRef<Path>,
    ) -> Result<RuntimeEngine, EngineError> {
        Self::with_file_resolver_and_working_directory(
            FileSourceResolver::empty(),
            working_directory.as_ref().to_path_buf(),
        )
    }

    /// Constructs an engine with an explicit working directory and numerical
    /// provider selected by the native host.
    ///
    /// The provider remains process-private. It is not exposed through the
    /// kernel protocol and therefore does not become a serialized or plugin
    /// ABI boundary.
    ///
    /// # Errors
    ///
    /// Returns a stable initialization error for an invalid directory or an
    /// interpreter construction failure.
    pub fn with_working_directory_and_linalg_provider(
        working_directory: impl AsRef<Path>,
        linalg_provider: Arc<dyn LinalgProvider>,
    ) -> Result<RuntimeEngine, EngineError> {
        Self::build(
            FileSourceResolver::empty(),
            working_directory.as_ref().to_path_buf(),
            Some(linalg_provider),
        )
    }

    /// Constructs an engine over host-owned, dynamically changeable directory state.
    ///
    /// # Errors
    ///
    /// Returns an engine initialization error when runtime services cannot be built.
    pub fn with_shared_working_directory(
        working_directory: WorkingDirectory,
    ) -> Result<RuntimeEngine, EngineError> {
        Self::build_with_working_directory(FileSourceResolver::empty(), working_directory, None)
    }

    /// Constructs an engine over shared current-directory and MATLAB search-path state.
    ///
    /// # Errors
    ///
    /// Returns an engine initialization error when runtime services cannot be built.
    pub fn with_shared_session_paths(
        working_directory: WorkingDirectory,
        search_path: SearchPath,
    ) -> Result<RuntimeEngine, EngineError> {
        Self::build_with_working_directory(
            FileSourceResolver::from_search_path(search_path),
            working_directory,
            None,
        )
    }

    /// Constructs an engine over shared directory state and a native numerical provider.
    ///
    /// # Errors
    ///
    /// Returns an engine initialization error when runtime services cannot be built.
    pub fn with_shared_working_directory_and_linalg_provider(
        working_directory: WorkingDirectory,
        linalg_provider: Arc<dyn LinalgProvider>,
    ) -> Result<RuntimeEngine, EngineError> {
        Self::build_with_working_directory(
            FileSourceResolver::empty(),
            working_directory,
            Some(linalg_provider),
        )
    }

    /// Constructs an engine over shared directory state with trusted OEX libraries.
    ///
    /// # Safety
    ///
    /// Native OEX libraries execute arbitrary in-process code. The caller must trust every
    /// supplied library and its implementation of the OEX v1 contract.
    ///
    /// # Errors
    ///
    /// Returns a stable OEX loading or runtime-initialization error.
    pub unsafe fn with_shared_working_directory_and_oex_plugins<I, P>(
        working_directory: WorkingDirectory,
        plugins: I,
    ) -> Result<RuntimeEngine, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let plugins = plugins
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect::<Vec<_>>();
        // SAFETY: The method contract transfers trust in every supplied native library.
        unsafe {
            Self::build_with_working_directory_and_oex_plugins(
                FileSourceResolver::empty(),
                working_directory,
                None,
                &plugins,
            )
        }
    }

    /// Constructs an engine over shared directory and search-path state with trusted plugins.
    ///
    /// # Safety
    ///
    /// Every supplied native library must be trusted and implement the OEX v1 contract.
    ///
    /// # Errors
    ///
    /// Returns a stable OEX loading or runtime-initialization error.
    pub unsafe fn with_shared_session_paths_and_oex_plugins<I, P>(
        working_directory: WorkingDirectory,
        search_path: SearchPath,
        plugins: I,
    ) -> Result<RuntimeEngine, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let plugins = plugins
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect::<Vec<_>>();
        // SAFETY: The method contract transfers trust in every supplied native library.
        unsafe {
            Self::build_with_working_directory_and_oex_plugins(
                FileSourceResolver::from_search_path(search_path),
                working_directory,
                None,
                &plugins,
            )
        }
    }

    /// Constructs an engine over shared directory state, a native numerical provider, and
    /// trusted OEX libraries.
    ///
    /// # Safety
    ///
    /// Native OEX libraries execute arbitrary in-process code. The caller must trust every
    /// supplied library and its implementation of the OEX v1 contract.
    ///
    /// # Errors
    ///
    /// Returns a stable OEX loading or runtime-initialization error.
    pub unsafe fn with_shared_working_directory_linalg_provider_and_oex_plugins<I, P>(
        working_directory: WorkingDirectory,
        linalg_provider: Arc<dyn LinalgProvider>,
        plugins: I,
    ) -> Result<RuntimeEngine, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let plugins = plugins
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect::<Vec<_>>();
        // SAFETY: The method contract transfers trust in every supplied native library.
        unsafe {
            Self::build_with_working_directory_and_oex_plugins(
                FileSourceResolver::empty(),
                working_directory,
                Some(linalg_provider),
                &plugins,
            )
        }
    }

    /// Constructs an engine over shared session paths, a provider, and trusted plugins.
    ///
    /// # Safety
    ///
    /// Every supplied native library must be trusted and implement the OEX v1 contract.
    ///
    /// # Errors
    ///
    /// Returns a stable OEX loading or runtime-initialization error.
    pub unsafe fn with_shared_session_paths_linalg_provider_and_oex_plugins<I, P>(
        working_directory: WorkingDirectory,
        search_path: SearchPath,
        linalg_provider: Arc<dyn LinalgProvider>,
        plugins: I,
    ) -> Result<RuntimeEngine, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let plugins = plugins
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect::<Vec<_>>();
        // SAFETY: The method contract transfers trust in every supplied native library.
        unsafe {
            Self::build_with_working_directory_and_oex_plugins(
                FileSourceResolver::from_search_path(search_path),
                working_directory,
                Some(linalg_provider),
                &plugins,
            )
        }
    }

    fn with_file_resolver_and_working_directory(
        file_resolver: FileSourceResolver,
        working_directory: PathBuf,
    ) -> Result<RuntimeEngine, EngineError> {
        Self::build(file_resolver, working_directory, None)
    }

    fn build(
        file_resolver: FileSourceResolver,
        working_directory: PathBuf,
        linalg_provider: Option<Arc<dyn LinalgProvider>>,
    ) -> Result<RuntimeEngine, EngineError> {
        let working_directory = WorkingDirectory::new(working_directory).map_err(|error| {
            EngineError::new(
                "filesystem.workingDirectory",
                format!("failed to initialize the session filesystem: {error}"),
            )
        })?;
        Self::build_with_working_directory(file_resolver, working_directory, linalg_provider)
    }

    fn build_with_working_directory(
        file_resolver: FileSourceResolver,
        working_directory: WorkingDirectory,
        linalg_provider: Option<Arc<dyn LinalgProvider>>,
    ) -> Result<RuntimeEngine, EngineError> {
        // SAFETY: An empty plugin list performs no native loading.
        unsafe {
            Self::build_with_working_directory_and_oex_plugins(
                file_resolver,
                working_directory,
                linalg_provider,
                &[],
            )
        }
    }

    unsafe fn build_with_working_directory_and_oex_plugins(
        file_resolver: FileSourceResolver,
        working_directory: WorkingDirectory,
        linalg_provider: Option<Arc<dyn LinalgProvider>>,
        plugin_paths: &[PathBuf],
    ) -> Result<RuntimeEngine, EngineError> {
        let plugin_paths = canonical_oex_plugin_paths(plugin_paths)?;
        let mut registry = openmat_builtins::minimal_registry().map_err(|error| {
            EngineError::new(
                "runtime.initialization",
                format!("failed to register minimal built-ins: {error}"),
            )
        })?;
        for path in &plugin_paths {
            // SAFETY: The caller of this private boundary has established trust in each DLL.
            let plugin = unsafe { OexPlugin::load(path) }.map_err(|error| {
                EngineError::new(
                    "oex.load",
                    format!("failed to load `{}`: {error}", path.display()),
                )
            })?;
            plugin.register_into(&mut registry).map_err(|error| {
                EngineError::new(
                    "oex.load",
                    format!("failed to register `{}`: {error}", path.display()),
                )
            })?;
        }
        let output_events = Arc::new(Mutex::new(Vec::new()));
        let output = SharedOutput {
            events: Arc::clone(&output_events),
        };
        let mut entry = openmat_bytecode::Function::new("<empty>", 0, 0, 0);
        entry.instructions.push(openmat_bytecode::Instruction::new(
            openmat_bytecode::InstructionKind::Return { values: Vec::new() },
        ));
        let module = openmat_bytecode::BytecodeModule::new(
            vec![entry],
            openmat_bytecode::FunctionId::new(0),
        );
        let mut interpreter = match linalg_provider {
            Some(provider) => Interpreter::with_components_and_linalg_provider(
                module,
                registry,
                Box::new(output),
                provider,
            ),
            None => Interpreter::with_components(module, registry, Box::new(output)),
        }
        .map_err(|error| {
            EngineError::new(
                "runtime.initialization",
                format!("failed to initialize runtime interpreter: {error}"),
            )
        })?;
        let file_system = LocalFileSystem::from_session_paths(
            working_directory.clone(),
            file_resolver.search_path_handle(),
        );
        interpreter.set_file_system_service(Box::new(file_system));
        Ok(Self {
            interpreter,
            output_events,
            pending_graphics_notices: Vec::new(),
            file_resolver,
            working_directory,
            known_classes: BTreeSet::new(),
            sources: BTreeMap::new(),
            next_source_id: Some(1),
            shut_down: false,
        })
    }

    /// Returns the configured protocol-independent source resolver.
    #[must_use]
    pub fn file_resolver(&self) -> &FileSourceResolver {
        &self.file_resolver
    }

    fn current_directory(&self) -> Result<PathBuf, EngineError> {
        self.working_directory
            .snapshot()
            .map(|snapshot| snapshot.path)
            .map_err(|error| EngineError::new("filesystem.workingDirectory", error.to_string()))
    }

    /// Returns the authoritative session-owned graphics model for a future
    /// kernel/server host adapter.
    pub fn graphics_session(&self) -> std::sync::MutexGuard<'_, GraphicsSession> {
        self.interpreter.graphics_session()
    }

    /// Clones the synchronized graphics hierarchy for the in-process Server
    /// adapter. This is not a process or plugin ABI.
    #[must_use]
    pub fn graphics_session_handle(&self) -> Arc<Mutex<GraphicsSession>> {
        self.interpreter.graphics_session_handle()
    }

    /// Drains committed internal graphics notices.
    ///
    /// Notices contain no numerical buffers, endpoint, or attachment token.
    /// The Server is responsible for session registration, secure token
    /// generation, first-discovery display, and ordinary delta transport.
    pub fn take_graphics_notices(&mut self) -> Vec<GraphicsNotice> {
        std::mem::take(&mut self.pending_graphics_notices)
    }

    /// Drains all committed graphics deltas in their built-in transaction order.
    pub fn take_graphics_deltas(&mut self) -> Vec<GraphicsDelta> {
        self.interpreter
            .graphics_session_mut()
            .take_pending_deltas()
    }

    /// Reads and executes one real UTF-8 `.m` file.
    ///
    /// The canonical file path becomes the diagnostic `source_name`, and its
    /// directory has lookup priority over explicitly configured search paths
    /// while compiling dependencies.
    ///
    /// # Errors
    ///
    /// Returns a stable file, compilation, or runtime error.
    pub fn execute_file(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &CancellationToken,
    ) -> Result<ExecutionOutput, EngineError> {
        self.ensure_running()?;
        let source = self
            .file_resolver
            .read_entry_from(&self.current_directory()?, path)?;
        let request = ExecuteRequest {
            code: source.text().to_owned(),
            source_name: source.source_name().to_owned(),
            mode: ExecutionMode::File,
        };
        self.execute_request(&request, Some(source.path()), cancellation)
    }

    fn ensure_running(&self) -> Result<(), EngineError> {
        if self.shut_down {
            Err(EngineError::new(
                "kernel.dead",
                "runtime engine has already shut down",
            ))
        } else {
            Ok(())
        }
    }

    fn allocate_source_id(&mut self) -> Result<SourceId, EngineError> {
        let raw = self.next_source_id.ok_or_else(|| {
            EngineError::new(
                "compile.sourceIdExhausted",
                "the checked source identifier space is exhausted",
            )
        })?;
        self.next_source_id = raw.checked_add(1);
        Ok(SourceId::new(raw))
    }

    fn compile_request(
        &mut self,
        request: &ExecuteRequest,
        source_path: Option<&Path>,
    ) -> Result<PreparedModule, EngineError> {
        let inherited_imports = self.interpreter.workspace_imports().to_vec();
        let unit = self.compile_source_with_imports(
            &request.source_name,
            &request.code,
            &inherited_imports,
        )?;
        if request.mode == ExecutionMode::File {
            reject_file_clear_import(&unit, &request.source_name)?;
        }
        if let Some(path) = source_path {
            validate_primary_file_name(path, &unit.kind)?;
        }
        let mut module = unit.module;
        let class_origins = module
            .classes
            .iter()
            .map(|class| (class.name.clone(), source_path.map(Path::to_path_buf)))
            .collect();
        let mut state = LinkState {
            loaded: BTreeMap::new(),
            function_origins: vec![source_path.map(Path::to_path_buf); module.functions.len()],
            class_origins,
            bare_script_sites: unit.bare_script_sites,
            class_entries: Vec::new(),
            class_entry_names: BTreeSet::new(),
        };
        let mut primary_classes = Vec::new();
        primary_classes
            .try_reserve(module.classes.len())
            .map_err(|_| EngineError::new("source.link", "class dependency list is too large"))?;
        primary_classes.extend(module.classes.iter().map(|class| class.name.clone()));
        for class in primary_classes {
            self.link_external_class_methods(&mut module, &mut state, &class)?;
            self.link_superclass_chain(&mut module, &mut state, &class, &mut BTreeSet::new())?;
        }
        let mut function_index = 0;
        while function_index < module.functions.len() {
            let instruction_count = module.functions[function_index].instructions.len();
            self.link_function_range(
                &mut module,
                &mut state,
                function_index,
                0,
                instruction_count,
                &mut Vec::new(),
            )?;
            function_index += 1;
        }
        let entry_index = module.entry.get() as usize;
        let entry = module
            .functions
            .get_mut(entry_index)
            .ok_or_else(|| EngineError::new("source.link", "compiled entry is missing"))?;
        for class_entry in state.class_entries.into_iter().rev() {
            prepend_entry_instructions(entry, class_entry)?;
        }
        let class_names = module
            .classes
            .iter()
            .map(|class| class.name.clone())
            .collect();
        Ok(PreparedModule {
            module,
            class_names,
        })
    }

    fn compile_source(
        &mut self,
        source_name: &str,
        code: &str,
    ) -> Result<CompiledSourceUnit, EngineError> {
        let unit = self.compile_source_with_imports(source_name, code, &[])?;
        reject_file_clear_import(&unit, source_name)?;
        Ok(unit)
    }

    fn compile_source_with_imports(
        &mut self,
        source_name: &str,
        code: &str,
        inherited_imports: &[String],
    ) -> Result<CompiledSourceUnit, EngineError> {
        let source_id = self.allocate_source_id()?;
        self.sources.insert(
            source_id.raw(),
            SourceRecord {
                name: source_name.to_owned(),
                length: code.len(),
            },
        );
        self.interpreter
            .register_source_metadata(source_id, source_name, code);
        let parsed = openmat_parser::parse(source_id, code);
        let lowered = openmat_hir::lower(&parsed.syntax);

        if !parsed.diagnostics.is_empty() || !lowered.diagnostics.is_empty() {
            let mut diagnostics = parsed
                .diagnostics
                .iter()
                .map(|diagnostic| source_diagnostic(diagnostic, source_name, code.len(), "OMP0000"))
                .collect::<Vec<_>>();
            diagnostics.extend(lowered.diagnostics.iter().map(|diagnostic| {
                source_diagnostic(diagnostic, source_name, code.len(), "OMH0000")
            }));
            diagnostics.sort_by_key(diagnostic_sort_key);
            let category = if parsed.diagnostics.is_empty() {
                "compile.hir"
            } else {
                "compile.parse"
            };
            return Err(error_with_diagnostics(
                category,
                "source could not be lowered to executable HIR",
                diagnostics,
            ));
        }

        let kind = classify_source_unit(&lowered.file);
        let bare_script_sites = collect_bare_script_sites(&lowered.file);
        let module = openmat_compiler::compile_with_imports(&lowered.file, inherited_imports)
            .map_err(|error| {
                let mut diagnostics = error
                    .diagnostics()
                    .iter()
                    .map(|diagnostic| Diagnostic {
                        code: Some(diagnostic.code().to_owned()),
                        severity: DiagnosticSeverity::Error,
                        message: diagnostic.to_string(),
                        range: Some(protocol_range(source_name, diagnostic.range, code.len())),
                        related: Vec::new(),
                    })
                    .collect::<Vec<_>>();
                diagnostics.sort_by_key(diagnostic_sort_key);
                error_with_diagnostics(
                    "compile.error",
                    "source could not be compiled to verified bytecode",
                    diagnostics,
                )
            })?;
        let kind = attach_primary_function_id(kind, &module)?;
        Ok(CompiledSourceUnit {
            module,
            kind,
            bare_script_sites,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn link_superclass_chain(
        &mut self,
        module: &mut BytecodeModule,
        state: &mut LinkState,
        class_name: &str,
        visiting: &mut BTreeSet<String>,
    ) -> Result<(), EngineError> {
        if self.known_classes.contains(class_name) {
            return Ok(());
        }
        if !visiting.insert(class_name.to_owned()) {
            return Err(EngineError::new(
                "source.cyclicSuperclass",
                format!("cyclic superclass dependency includes `{class_name}`"),
            ));
        }
        let superclass = module
            .classes
            .iter()
            .find(|class| class.name == class_name)
            .ok_or_else(|| {
                EngineError::new(
                    "source.link",
                    format!("linked class `{class_name}` is missing its bytecode definition"),
                )
            })?
            .superclass
            .clone();
        let Some(superclass) = superclass else {
            visiting.remove(class_name);
            return Ok(());
        };
        if self.known_classes.contains(&superclass) {
            visiting.remove(class_name);
            return Ok(());
        }
        if module.classes.iter().any(|class| class.name == superclass) {
            self.link_superclass_chain(module, state, &superclass, visiting)?;
            visiting.remove(class_name);
            return Ok(());
        }

        let origin = state
            .class_origins
            .get(class_name)
            .and_then(|path| path.as_deref());
        let source = self
            .file_resolver
            .resolve_from(Some(&self.current_directory()?), origin, &superclass)?
            .ok_or_else(|| {
                EngineError::new(
                    "source.unresolvedSuperclass",
                    format!("superclass `{superclass}` could not be resolved to a MATLAB file"),
                )
            })?;
        let source_path = source.path().to_path_buf();
        if let Some(linked) = state.loaded.get(&source_path) {
            validate_loaded_dependency_name(&superclass, linked, source.path())?;
            if !module.classes.iter().any(|class| class.name == superclass) {
                return Err(EngineError::new(
                    "source.link",
                    "loaded superclass is missing from the linked class table",
                ));
            }
            self.link_superclass_chain(module, state, &superclass, visiting)?;
            visiting.remove(class_name);
            return Ok(());
        }

        let mut unit = self.compile_source(source.source_name(), source.text())?;
        validate_dependency_name(&superclass, &unit.kind, source.path())?;
        qualify_dependency(&mut unit, &superclass)?;
        let SourceUnitKind::Class {
            name: dependency_name,
        } = &unit.kind
        else {
            return Err(EngineError::new(
                "source.link",
                "validated superclass source is not a class file",
            ));
        };
        let dependency_name = dependency_name.clone();
        let appended_functions = unit.module.functions.len().saturating_sub(1);
        let merged = merge_support_module(module, &unit.module)?;
        merge_support_class_features(module, &unit.module, &merged)?;
        if !merged.classes.contains_key(&dependency_name) {
            return Err(EngineError::new(
                "source.link",
                "primary superclass was not linked",
            ));
        }
        state.function_origins.extend(std::iter::repeat_n(
            Some(source_path.clone()),
            appended_functions,
        ));
        state.bare_script_sites.extend(unit.bare_script_sites);
        for class in &unit.module.classes {
            state
                .class_origins
                .insert(class.name.clone(), Some(source_path.clone()));
        }
        self.link_external_class_methods(module, state, &dependency_name)?;
        state.loaded.insert(
            source_path,
            LinkedSource::Class {
                name: dependency_name.clone(),
            },
        );
        self.link_superclass_chain(module, state, &dependency_name, visiting)?;
        if state.class_entry_names.insert(dependency_name) {
            state.class_entries.push(merged.entry);
        }
        visiting.remove(class_name);
        Ok(())
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn link_function_range(
        &mut self,
        module: &mut BytecodeModule,
        state: &mut LinkState,
        function_index: usize,
        mut instruction_index: usize,
        mut end: usize,
        script_stack: &mut Vec<PathBuf>,
    ) -> Result<usize, EngineError> {
        while instruction_index < end {
            let Some(load) = name_load(
                module
                    .functions
                    .get(function_index)
                    .ok_or_else(|| EngineError::new("source.link", "linked function is missing"))?,
                instruction_index,
            ) else {
                instruction_index += 1;
                continue;
            };
            if load.root.as_deref().is_some_and(|root| {
                self.name_resolves_without_file(module, function_index, root, NameLoadKind::Global)
            }) {
                instruction_index += 1;
                continue;
            }
            let origin = load
                .location
                .and_then(|location| self.sources.get(&location.source_id))
                .map(|source| Path::new(&source.name))
                .filter(|path| path.is_absolute())
                .map(Path::to_path_buf)
                .or_else(|| {
                    state
                        .function_origins
                        .get(function_index)
                        .cloned()
                        .flatten()
                });
            let current_directory = self.current_directory()?;
            let mut resolved = None;
            let mut resolved_without_file = false;
            'candidates: for candidate in &load.names {
                for name in candidate_resolution_names(candidate, load.member_count) {
                    let callable_kind = if load.kind == NameLoadKind::Qualified {
                        NameLoadKind::Global
                    } else {
                        load.kind
                    };
                    if self.name_resolves_without_file(module, function_index, &name, callable_kind)
                    {
                        resolved_without_file = true;
                        break 'candidates;
                    }
                    if let Some(source) = self.file_resolver.resolve_from(
                        Some(&current_directory),
                        origin.as_deref(),
                        &name,
                    )? {
                        resolved = Some((name, source));
                        break 'candidates;
                    }
                }
            }
            if resolved_without_file {
                instruction_index += 1;
                continue;
            }
            let Some((name, source)) = resolved else {
                instruction_index += 1;
                continue;
            };
            let source_path = source.path().to_path_buf();
            let linked = if let Some(linked) = state.loaded.get(&source_path) {
                validate_loaded_dependency_name(&name, linked, source.path())?;
                linked.clone()
            } else {
                let mut unit = self.compile_source(source.source_name(), source.text())?;
                validate_dependency_name(&name, &unit.kind, source.path())?;
                qualify_dependency(&mut unit, &name)?;
                if load.kind == NameLoadKind::FunctionHandle
                    && matches!(unit.kind, SourceUnitKind::Script)
                {
                    instruction_index += 1;
                    continue;
                }
                let appended_functions = unit.module.functions.len().saturating_sub(1);
                let merged = merge_support_module(module, &unit.module)?;
                merge_support_class_features(module, &unit.module, &merged)?;
                state.function_origins.extend(std::iter::repeat_n(
                    Some(source_path.clone()),
                    appended_functions,
                ));
                state.bare_script_sites.extend(unit.bare_script_sites);
                let linked = match unit.kind {
                    SourceUnitKind::Function {
                        name: primary_name,
                        function,
                    } => LinkedSource::Function {
                        name: primary_name,
                        function: merged.function(function).ok_or_else(|| {
                            EngineError::new(
                                "source.link",
                                "primary support function was not linked",
                            )
                        })?,
                    },
                    SourceUnitKind::Class { ref name } => {
                        if !merged.classes.contains_key(name) {
                            return Err(EngineError::new(
                                "source.link",
                                "primary support class was not linked",
                            ));
                        }
                        state
                            .class_origins
                            .insert(name.clone(), Some(source_path.clone()));
                        self.link_external_class_methods(module, state, name)?;
                        self.link_superclass_chain(module, state, name, &mut BTreeSet::new())?;
                        if state.class_entry_names.insert(name.clone()) {
                            state.class_entries.push(merged.entry);
                        }
                        LinkedSource::Class { name: name.clone() }
                    }
                    SourceUnitKind::Script => LinkedSource::Script(merged.entry),
                };
                state.loaded.insert(source_path.clone(), linked.clone());
                linked
            };

            match linked {
                LinkedSource::Function { function, .. } => {
                    if load.kind == NameLoadKind::Global && name.contains('.') {
                        let caller = module.functions.get_mut(function_index).ok_or_else(|| {
                            EngineError::new("source.link", "caller function is missing")
                        })?;
                        replace_load_with_function(
                            caller,
                            instruction_index,
                            load.destination,
                            function,
                        )?;
                    }
                    instruction_index += 1;
                }
                LinkedSource::Class { .. } => {
                    instruction_index += 1;
                }
                LinkedSource::Script(entry) => {
                    if load.kind == NameLoadKind::FunctionHandle {
                        instruction_index += 1;
                        continue;
                    }
                    let site = load
                        .location
                        .map(SourceSite::from)
                        .and_then(|site| state.bare_script_sites.get(&site));
                    let Some(site) = site else {
                        return Err(source_resolution_error(
                            "source.scriptNotCallable",
                            "OMS0002",
                            &format!("script `{name}` cannot be called with arguments"),
                            load.location,
                            &self.sources,
                            source.source_name(),
                            source.text().len(),
                        ));
                    };
                    if site.name != name {
                        return Err(source_resolution_error(
                            "source.scriptFunctionScope",
                            "OMS0003",
                            &format!(
                                "script `{name}` requires caller-local scope support outside the top-level workspace"
                            ),
                            load.location,
                            &self.sources,
                            source.source_name(),
                            source.text().len(),
                        ));
                    }
                    if script_stack.contains(&source_path) {
                        return Err(source_resolution_error(
                            "source.cyclicDependency",
                            "OMS0004",
                            &format!(
                                "cyclic script dependency includes `{}`",
                                source.source_name()
                            ),
                            load.location,
                            &self.sources,
                            source.source_name(),
                            source.text().len(),
                        ));
                    }
                    let caller = module.functions.get_mut(function_index).ok_or_else(|| {
                        EngineError::new("source.link", "script caller function is missing")
                    })?;
                    let inserted = if site.workspace_scope
                        && function_index == module.entry.get() as usize
                    {
                        inline_script_entry(caller, instruction_index, entry)?
                    } else {
                        inline_script_entry_with_caller_locals(caller, instruction_index, entry)?
                    };
                    end = end
                        .checked_sub(1)
                        .and_then(|value| value.checked_add(inserted))
                        .ok_or_else(|| {
                            EngineError::new("source.link", "script instruction range overflowed")
                        })?;
                    let nested_end = instruction_index.checked_add(inserted).ok_or_else(|| {
                        EngineError::new("source.link", "script instruction range overflowed")
                    })?;
                    script_stack.push(source_path);
                    let processed_end = self.link_function_range(
                        module,
                        state,
                        function_index,
                        instruction_index,
                        nested_end,
                        script_stack,
                    )?;
                    script_stack.pop();
                    end = end
                        .checked_add(processed_end.saturating_sub(nested_end))
                        .ok_or_else(|| {
                            EngineError::new("source.link", "script instruction range overflowed")
                        })?;
                    instruction_index = processed_end;
                }
            }
        }
        Ok(end)
    }

    fn link_external_class_methods(
        &mut self,
        module: &mut BytecodeModule,
        state: &mut LinkState,
        class_name: &str,
    ) -> Result<(), EngineError> {
        let declarations = external_method_declarations(module, class_name)?;
        if declarations.is_empty() {
            return Ok(());
        }

        let class_origin = state
            .class_origins
            .get(class_name)
            .cloned()
            .flatten()
            .ok_or_else(|| {
                EngineError::new(
                    "source.classFolderRequired",
                    format!(
                        "class `{class_name}` declares external methods but has no `@Class/Class.m` source origin"
                    ),
                )
            })?;

        for (method_name, expected_signature) in declarations {
            let function = self.link_external_class_method_source(
                module,
                state,
                &class_origin,
                class_name,
                &method_name,
                expected_signature,
            )?;
            set_linked_external_method(module, class_name, &method_name, function)?;
        }
        Ok(())
    }

    fn link_external_class_method_source(
        &mut self,
        module: &mut BytecodeModule,
        state: &mut LinkState,
        class_origin: &Path,
        class_name: &str,
        method_name: &str,
        expected_signature: ExternalMethodSignature,
    ) -> Result<FunctionId, EngineError> {
        let source = self
            .file_resolver
            .resolve_class_method(class_origin, class_name, method_name)?
            .ok_or_else(|| {
                EngineError::new(
                    "source.unresolvedExternalMethod",
                    format!(
                        "external method `{class_name}.{method_name}` has no sibling `{method_name}.m` implementation"
                    ),
                )
            })?;
        let source_path = source.path().to_path_buf();
        if let Some(linked) = state.loaded.get(&source_path) {
            validate_loaded_dependency_name(method_name, linked, source.path())?;
            let LinkedSource::Function { function, .. } = linked else {
                return Err(EngineError::new(
                    "source.externalMethodKind",
                    format!(
                        "external method source `{}` is not a function file",
                        source.path().to_string_lossy()
                    ),
                ));
            };
            return Ok(*function);
        }

        let unit = self.compile_source(source.source_name(), source.text())?;
        validate_dependency_name(method_name, &unit.kind, source.path())?;
        let SourceUnitKind::Function {
            name: implementation_name,
            function: implementation,
        } = &unit.kind
        else {
            return Err(EngineError::new(
                "source.externalMethodKind",
                format!(
                    "external method source `{}` must define function `{method_name}`",
                    source.path().to_string_lossy()
                ),
            ));
        };
        let implementation_signature = unit
            .module
            .functions
            .get(implementation.get() as usize)
            .map(external_method_signature)
            .ok_or_else(|| {
                EngineError::new(
                    "source.link",
                    format!(
                        "external method source `{}` has no primary function",
                        source.path().to_string_lossy()
                    ),
                )
            })?;
        if implementation_signature != expected_signature {
            return Err(EngineError::new(
                "source.externalMethodSignature",
                format!(
                    "external method `{class_name}.{method_name}` implements signature {} but its class declaration requires {}",
                    implementation_signature.describe(),
                    expected_signature.describe()
                ),
            ));
        }

        let appended_functions = unit.module.functions.len().saturating_sub(1);
        let merged = merge_support_module(module, &unit.module)?;
        merge_support_class_features(module, &unit.module, &merged)?;
        state.function_origins.extend(std::iter::repeat_n(
            Some(source_path.clone()),
            appended_functions,
        ));
        state.bare_script_sites.extend(unit.bare_script_sites);
        let function = merged.function(*implementation).ok_or_else(|| {
            EngineError::new(
                "source.link",
                format!("external method function `{implementation_name}` was not linked"),
            )
        })?;
        state.loaded.insert(
            source_path,
            LinkedSource::Function {
                name: implementation_name.clone(),
                function,
            },
        );
        Ok(function)
    }

    fn name_resolves_without_file(
        &self,
        module: &BytecodeModule,
        function_index: usize,
        name: &str,
        load_kind: NameLoadKind,
    ) -> bool {
        match load_kind {
            NameLoadKind::Global | NameLoadKind::CallTarget | NameLoadKind::Qualified => {
                module
                    .functions
                    .iter()
                    .any(|function| function.name == name)
                    || (self.interpreter.workspace().contains(name)
                        && module
                            .functions
                            .get(function_index)
                            .is_none_or(|function| !function_clears_global(function, name)))
                    || self.known_classes.contains(name)
                    || module.classes.iter().any(|class| class.name == name)
                    || matches!(
                        name,
                        "class" | "isa" | "isobject" | "addlistener" | "notify"
                    )
                    || self.interpreter.builtins().handle_by_name(name).is_some()
            }
            NameLoadKind::FunctionHandle => {
                matches!(name, "addlistener" | "notify")
                    || self.interpreter.builtins().handle_by_name(name).is_some()
            }
        }
    }

    fn run_module(
        &mut self,
        prepared: PreparedModule,
        request: &ExecuteRequest,
        cancellation: &CancellationToken,
    ) -> Result<ExecutionOutput, EngineError> {
        let before = self.interpreter.workspace().clone();
        let initial_display_format = self.interpreter.display_format();
        self.output_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.interpreter
            .replace_module(prepared.module)
            .map_err(|error| runtime_build_error(&error, &self.sources, request))?;

        let runtime_cancellation = self.interpreter.cancellation_token();
        runtime_cancellation.reset();
        let bridge = CancellationBridge::start(cancellation.clone(), runtime_cancellation)?;
        let execution = self.interpreter.execute_entry(&[]);
        let bridge_result = bridge.stop();

        let emitted = self
            .output_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let (mut events, notices) =
            display_events(emitted, &self.interpreter, initial_display_format);
        self.pending_graphics_notices.extend(notices);
        events.push(Event::WorkspaceDelta(workspace_delta(
            &before,
            self.interpreter.workspace(),
            &self.interpreter,
        )));

        if let Err(error) = bridge_result {
            return Err(error.with_events(events));
        }

        match execution {
            Ok(_) => {
                self.known_classes.extend(prepared.class_names);
                Ok(ExecutionOutput {
                    result: ExecuteResult {
                        interrupted: cancellation.is_cancelled(),
                    },
                    events,
                })
            }
            Err(error) if matches!(error.kind, RuntimeErrorKind::Cancelled) => {
                Ok(ExecutionOutput {
                    result: ExecuteResult { interrupted: true },
                    events,
                })
            }
            Err(error) => {
                let diagnostic = runtime_diagnostic(&error, &self.sources, request);
                events.push(Event::Diagnostic(diagnostic.clone()));
                Err(
                    EngineError::new(runtime_category(&error.kind), error.to_string())
                        .with_diagnostics(vec![diagnostic])
                        .with_events(events),
                )
            }
        }
    }

    fn execute_request(
        &mut self,
        request: &ExecuteRequest,
        source_path: Option<&Path>,
        cancellation: &CancellationToken,
    ) -> Result<ExecutionOutput, EngineError> {
        let started = Instant::now();
        if performance_logging_enabled() {
            eprintln!(
                "OPENMAT_PERF kernel stage=begin source_bytes={}",
                request.code.len()
            );
        }
        let module = match self.compile_request(request, source_path) {
            Ok(module) => module,
            Err(error) => {
                if performance_logging_enabled() {
                    eprintln!(
                        "OPENMAT_PERF kernel source_bytes={} compile_ms={:.3} execute_ms=0.000 total_ms={:.3} outcome=compile_error",
                        request.code.len(),
                        duration_milliseconds(started.elapsed()),
                        duration_milliseconds(started.elapsed()),
                    );
                }
                return Err(error);
            }
        };
        let compiled = Instant::now();
        if performance_logging_enabled() {
            eprintln!(
                "OPENMAT_PERF kernel stage=compiled compile_ms={:.3}",
                duration_milliseconds(compiled.duration_since(started))
            );
        }
        let result = self.run_module(module, request, cancellation);
        if performance_logging_enabled() {
            let finished = Instant::now();
            let outcome = if result.is_ok() {
                "ok"
            } else {
                "runtime_error"
            };
            eprintln!(
                "OPENMAT_PERF kernel source_bytes={} compile_ms={:.3} execute_ms={:.3} total_ms={:.3} outcome={outcome}",
                request.code.len(),
                duration_milliseconds(compiled.duration_since(started)),
                duration_milliseconds(finished.duration_since(compiled)),
                duration_milliseconds(finished.duration_since(started)),
            );
        }
        result
    }
}

impl ExecutionEngine for RuntimeEngine {
    fn implementation(&self) -> ImplementationInfo {
        ImplementationInfo {
            name: "openmat-runtime".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            execution_modes: vec![
                ExecutionMode::File,
                ExecutionMode::Cell,
                ExecutionMode::Repl,
            ],
            display_mime_types: vec!["text/plain".to_owned()],
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            interrupt: true,
            workspace_delta: true,
        }
    }

    fn graphics_session(&self) -> Option<Arc<Mutex<GraphicsSession>>> {
        Some(self.graphics_session_handle())
    }

    fn take_graphics_notices(&mut self) -> Vec<GraphicsNotice> {
        RuntimeEngine::take_graphics_notices(self)
    }

    fn take_graphics_deltas(&mut self) -> Vec<GraphicsDelta> {
        RuntimeEngine::take_graphics_deltas(self)
    }

    fn execute(
        &mut self,
        request: &ExecuteRequest,
        cancellation: &CancellationToken,
    ) -> Result<ExecutionOutput, EngineError> {
        self.ensure_running()?;
        if request.mode == ExecutionMode::File && is_explicit_file_source_name(&request.source_name)
        {
            let source = self
                .file_resolver
                .read_entry_from(&self.current_directory()?, &request.source_name)?;
            if source.text() != request.code {
                return Err(EngineError::new(
                    "source.contentMismatch",
                    format!(
                        "request source does not match UTF-8 file `{}`",
                        source.source_name()
                    ),
                ));
            }
            let canonical_request = ExecuteRequest {
                code: request.code.clone(),
                source_name: source.source_name().to_owned(),
                mode: request.mode,
            };
            return self.execute_request(&canonical_request, Some(source.path()), cancellation);
        }
        self.execute_request(request, None, cancellation)
    }

    fn inspect(&mut self, request: &InspectRequest) -> Result<MatrixPreview, EngineError> {
        self.ensure_running()?;
        validate_inspect_request(request)?;
        let value = self
            .interpreter
            .workspace()
            .get(&request.name)
            .ok_or_else(|| {
                EngineError::new(
                    "workspace.notFound",
                    format!("workspace variable `{}` does not exist", request.name),
                )
            })?;
        inspect_value(value, &self.interpreter.value_class_name(value), request)
    }

    fn inspect_v1(
        &mut self,
        request: &kernel_v1::InspectRequest,
        limits: &kernel_v1::PreviewLimits,
    ) -> Result<kernel_v1::MatrixPreview, EngineError> {
        self.ensure_running()?;
        request.validate(limits).map_err(|error| {
            EngineError::new(
                error.category(),
                format!("invalid kernel-v1 inspect request: {error}"),
            )
        })?;
        let value = self
            .interpreter
            .workspace()
            .get(&request.name)
            .ok_or_else(|| {
                EngineError::new(
                    "workspace.notFound",
                    format!("workspace variable `{}` does not exist", request.name),
                )
            })?;
        inspect_value_v1(
            value,
            &self.interpreter.value_class_name(value),
            request,
            limits,
        )
    }

    fn inspect_v2(
        &mut self,
        request: &kernel_v2::InspectRequest,
        limits: &kernel_v2::AggregateLimits,
    ) -> Result<kernel_v2::InspectPreview, EngineError> {
        self.ensure_running()?;
        request
            .validate(&limits.preview_limits())
            .map_err(|error| {
                EngineError::new(
                    error.category(),
                    format!("invalid kernel-v2 inspect request: {error}"),
                )
            })?;
        let value = self
            .interpreter
            .workspace()
            .get(&request.name)
            .ok_or_else(|| {
                EngineError::new(
                    "workspace.notFound",
                    format!("workspace variable `{}` does not exist", request.name),
                )
            })?;
        inspect_value_v2(
            value,
            &self.interpreter.value_class_name(value),
            request,
            limits,
        )
    }

    fn set_variable_element(
        &mut self,
        request: &kernel_v3::SetVariableElementRequest,
    ) -> Result<VariableSummary, EngineError> {
        self.ensure_running()?;
        request.validate().map_err(|error| {
            EngineError::new(
                "protocol.validation",
                format!("invalid kernel-v3 workspace mutation: {error}"),
            )
        })?;
        let (real, imaginary) = request.value.components().map_err(|error| {
            EngineError::new(
                "protocol.validation",
                format!("invalid kernel-v3 numeric value: {error}"),
            )
        })?;

        let dimensions = self
            .interpreter
            .workspace()
            .get(&request.name)
            .ok_or_else(|| {
                EngineError::new(
                    "workspace.notFound",
                    format!("workspace variable `{}` does not exist", request.name),
                )
            })?
            .dimensions()
            .ok_or_else(|| {
                EngineError::new(
                    "workspace.editUnsupported",
                    format!(
                        "workspace variable `{}` has no editable shape",
                        request.name
                    ),
                )
            })?
            .to_vec();
        if request.indices.len() != dimensions.len() {
            return Err(EngineError::new(
                "workspace.indexOutOfBounds",
                format!(
                    "workspace variable `{}` requires {} subscripts, got {}",
                    request.name,
                    dimensions.len(),
                    request.indices.len(),
                ),
            ));
        }
        let shape = Shape::new(dimensions).map_err(|error| {
            EngineError::new(
                "engine.invalidValue",
                format!(
                    "workspace variable `{}` has an invalid shape: {error}",
                    request.name
                ),
            )
        })?;
        let one_based = shape
            .linear_index_for_subscripts(&request.indices)
            .map_err(|error| {
                EngineError::new(
                    "workspace.indexOutOfBounds",
                    format!("workspace element indices are invalid: {error}"),
                )
            })?;

        let value = self
            .interpreter
            .workspace_mut()
            .get_mut(&request.name)
            .expect("workspace binding was checked before mutable access");
        match value {
            Value::Array(ArrayData::Integer(storage)) => set_integer_element(
                storage,
                one_based,
                &request.value.real,
                &request.value.imaginary,
            )?,
            _ => set_numeric_element(value, one_based, real, imaginary)?,
        }
        let class_name = value.class_name().to_owned();
        Ok(variable_summary(&request.name, value, &class_name))
    }

    fn list_workspace(&mut self) -> Result<Vec<VariableSummary>, EngineError> {
        self.ensure_running()?;
        Ok(self
            .interpreter
            .workspace()
            .iter()
            .map(|(name, value)| {
                variable_summary(name, value, &self.interpreter.value_class_name(value))
            })
            .collect())
    }

    fn shutdown(&mut self) -> Result<(), EngineError> {
        self.interpreter.clear_session();
        self.known_classes.clear();
        self.sources.clear();
        self.output_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.pending_graphics_notices.clear();
        self.shut_down = true;
        Ok(())
    }
}

fn merge_support_class_features(
    target: &mut BytecodeModule,
    support: &BytecodeModule,
    merged: &MergedModule,
) -> Result<(), EngineError> {
    target
        .class_features
        .try_reserve(support.class_features.len())
        .map_err(|_| EngineError::new("source.link", "linked class feature table is too large"))?;
    for feature in &support.class_features {
        let class = support
            .classes
            .get(feature.class.get() as usize)
            .ok_or_else(|| {
                EngineError::new(
                    "source.link",
                    "support class feature refers to a missing class",
                )
            })?;
        let mut linked = feature.clone();
        linked.class = merged.classes.get(&class.name).copied().ok_or_else(|| {
            EngineError::new(
                "source.link",
                "support class feature was not assigned a linked class",
            )
        })?;
        for member in &mut linked.enumeration_members {
            member.argument_initializer =
                merged
                    .function(member.argument_initializer)
                    .ok_or_else(|| {
                        EngineError::new(
                            "source.link",
                            "enumeration initializer was not linked into the request module",
                        )
                    })?;
        }
        target.class_features.push(linked);
    }
    Ok(())
}

fn classify_source_unit(file: &HirFile) -> SourceUnitKind {
    if file.statements.len() == 1
        && let StmtKind::Class(class) = &file.statements[0].kind
        && let Some(name) = &class.name
    {
        return SourceUnitKind::Class {
            name: name.text.clone(),
        };
    }
    if !file.statements.is_empty()
        && file
            .statements
            .iter()
            .all(|statement| matches!(statement.kind, StmtKind::Function(_)))
        && let StmtKind::Function(function) = &file.statements[0].kind
        && let Some(name) = &function.name
    {
        return SourceUnitKind::Function {
            name: name.text.clone(),
            function: FunctionId::new(0),
        };
    }
    SourceUnitKind::Script
}

fn attach_primary_function_id(
    kind: SourceUnitKind,
    module: &BytecodeModule,
) -> Result<SourceUnitKind, EngineError> {
    let SourceUnitKind::Function { name, .. } = kind else {
        return Ok(kind);
    };
    let index = module
        .functions
        .iter()
        .position(|function| function.name == name)
        .and_then(|index| u32::try_from(index).ok())
        .ok_or_else(|| {
            EngineError::new(
                "source.link",
                "compiled primary function is missing from its source module",
            )
        })?;
    Ok(SourceUnitKind::Function {
        name,
        function: FunctionId::new(index),
    })
}

fn collect_bare_script_sites(file: &HirFile) -> BTreeMap<SourceSite, BareScriptSite> {
    let mut sites = BTreeMap::new();
    collect_bare_script_statements(file.source_id.raw(), &file.statements, true, &mut sites);
    sites
}

fn collect_bare_script_statements(
    source_id: u32,
    statements: &[Stmt],
    workspace_scope: bool,
    sites: &mut BTreeMap<SourceSite, BareScriptSite>,
) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Expr(expression) => {
                if let ExprKind::Name(name) = &expression.kind {
                    sites.insert(
                        SourceSite {
                            source_id,
                            start: expression.span.start(),
                            end: expression.span.end(),
                        },
                        BareScriptSite {
                            name: name.clone(),
                            workspace_scope,
                        },
                    );
                }
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    collect_bare_script_statements(source_id, &branch.body, workspace_scope, sites);
                }
                collect_bare_script_statements(source_id, else_body, workspace_scope, sites);
            }
            StmtKind::For { body, .. } | StmtKind::While { body, .. } => {
                collect_bare_script_statements(source_id, body, workspace_scope, sites);
            }
            StmtKind::Try(statement) => {
                collect_bare_script_statements(source_id, &statement.body, workspace_scope, sites);
                if let Some(catch) = &statement.catch {
                    collect_bare_script_statements(source_id, &catch.body, workspace_scope, sites);
                }
            }
            StmtKind::Function(function) => {
                collect_bare_script_statements(source_id, &function.body, false, sites);
            }
            StmtKind::Class(class) => {
                for block in &class.method_blocks {
                    for method in &block.methods {
                        collect_bare_script_statements(source_id, &method.body, false, sites);
                    }
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_lines)]
fn inline_script_entry_with_caller_locals(
    caller: &mut Function,
    instruction_index: usize,
    entry: Function,
) -> Result<usize, EngineError> {
    let caller_bindings = caller
        .instructions
        .iter()
        .filter_map(|instruction| {
            let InstructionKind::DeclareNamedBindings { bindings } = &instruction.kind else {
                return None;
            };
            Some(bindings.iter().filter_map(|(name, kind)| {
                function_constant_name(caller, *name).map(|name| (name.to_owned(), *kind))
            }))
        })
        .flatten()
        .collect::<BTreeMap<_, _>>();
    let inserted = inline_script_entry(caller, instruction_index, entry)?;
    let inserted_end = instruction_index
        .checked_add(inserted)
        .ok_or_else(|| EngineError::new("source.link", "script instruction range overflowed"))?;

    let explicit_globals = caller.instructions[instruction_index..inserted_end]
        .iter()
        .filter_map(|instruction| {
            let InstructionKind::DeclareGlobal { name } = instruction.kind else {
                return None;
            };
            function_constant_name(caller, name).map(str::to_owned)
        })
        .collect::<BTreeSet<_>>();
    let mut remapped = BTreeMap::new();
    let mut bindings_by_name = BTreeMap::new();
    for instruction in &caller.instructions[instruction_index..inserted_end] {
        let InstructionKind::DeclareNamedBindings { bindings } = &instruction.kind else {
            continue;
        };
        for (name, kind) in bindings {
            if *kind != NamedBindingKind::Workspace {
                continue;
            }
            let Some(text) = function_constant_name(caller, *name) else {
                continue;
            };
            let text = text.to_owned();
            if explicit_globals.contains(&text) {
                continue;
            }
            let kind = caller_bindings
                .get(&text)
                .or_else(|| bindings_by_name.get(&text))
                .copied()
                .unwrap_or_else(|| {
                    let local = LocalSlot::new(caller.local_count);
                    caller.local_count = caller.local_count.saturating_add(1);
                    NamedBindingKind::Local(local)
                });
            bindings_by_name.insert(text, kind);
            remapped.insert(name.get(), kind);
        }
    }
    for instruction in &caller.instructions[instruction_index..inserted_end] {
        let InstructionKind::StoreGlobal { name, .. } = instruction.kind else {
            continue;
        };
        let Some(text) = function_constant_name(caller, name) else {
            continue;
        };
        let text = text.to_owned();
        if explicit_globals.contains(&text) {
            continue;
        }
        let kind = caller_bindings
            .get(&text)
            .or_else(|| bindings_by_name.get(&text))
            .copied()
            .unwrap_or_else(|| {
                let local = LocalSlot::new(caller.local_count);
                caller.local_count = caller.local_count.saturating_add(1);
                NamedBindingKind::Local(local)
            });
        bindings_by_name.insert(text, kind);
        remapped.insert(name.get(), kind);
    }
    for instruction in &caller.instructions[instruction_index..inserted_end] {
        let (InstructionKind::LoadGlobal { name, .. }
        | InstructionKind::LoadCallTarget { name, .. }
        | InstructionKind::LoadGlobalOrNothing { name, .. }) = instruction.kind
        else {
            continue;
        };
        let Some(text) = function_constant_name(caller, name) else {
            continue;
        };
        if let Some(kind) = caller_bindings
            .get(text)
            .or_else(|| bindings_by_name.get(text))
        {
            remapped.insert(name.get(), *kind);
        }
    }
    let following_remapped = caller.instructions[inserted_end..]
        .iter()
        .filter_map(|instruction| {
            let (InstructionKind::LoadGlobal { name, .. }
            | InstructionKind::LoadCallTarget { name, .. }
            | InstructionKind::LoadGlobalOrNothing { name, .. }
            | InstructionKind::StoreGlobal { name, .. }) = instruction.kind
            else {
                return None;
            };
            let text = function_constant_name(caller, name)?;
            bindings_by_name
                .get(text)
                .copied()
                .map(|kind| (name.get(), kind))
        })
        .collect::<BTreeMap<_, _>>();

    for instruction in &mut caller.instructions[instruction_index..inserted_end] {
        match &mut instruction.kind {
            InstructionKind::DeclareNamedBindings { bindings } => {
                for (name, kind) in bindings {
                    if let Some(remapped) = remapped.get(&name.get()) {
                        *kind = *remapped;
                    }
                }
            }
            InstructionKind::LoadGlobal { dst, name, .. }
            | InstructionKind::LoadCallTarget { dst, name }
            | InstructionKind::LoadGlobalOrNothing { dst, name } => {
                let Some(binding) = remapped.get(&name.get()).copied() else {
                    continue;
                };
                instruction.kind = match binding {
                    NamedBindingKind::Local(local) => {
                        InstructionKind::LoadLocal { dst: *dst, local }
                    }
                    NamedBindingKind::Persistent(slot) => {
                        InstructionKind::LoadPersistent { dst: *dst, slot }
                    }
                    NamedBindingKind::Capture => InstructionKind::LoadCapture {
                        dst: *dst,
                        name: *name,
                    },
                    NamedBindingKind::Workspace => continue,
                };
            }
            InstructionKind::StoreGlobal { name, src } => {
                let Some(binding) = remapped.get(&name.get()).copied() else {
                    continue;
                };
                instruction.kind = match binding {
                    NamedBindingKind::Local(local) => {
                        InstructionKind::StoreLocal { local, src: *src }
                    }
                    NamedBindingKind::Persistent(slot) => {
                        InstructionKind::StorePersistent { slot, src: *src }
                    }
                    NamedBindingKind::Capture => InstructionKind::StoreCapture {
                        name: *name,
                        src: *src,
                    },
                    NamedBindingKind::Workspace => continue,
                };
            }
            _ => {}
        }
    }
    for instruction in &mut caller.instructions[inserted_end..] {
        match &mut instruction.kind {
            InstructionKind::LoadGlobal { dst, name, .. }
            | InstructionKind::LoadCallTarget { dst, name }
            | InstructionKind::LoadGlobalOrNothing { dst, name } => {
                let Some(binding) = following_remapped.get(&name.get()).copied() else {
                    continue;
                };
                instruction.kind = match binding {
                    NamedBindingKind::Local(local) => {
                        InstructionKind::LoadLocal { dst: *dst, local }
                    }
                    NamedBindingKind::Persistent(slot) => {
                        InstructionKind::LoadPersistent { dst: *dst, slot }
                    }
                    NamedBindingKind::Capture => InstructionKind::LoadCapture {
                        dst: *dst,
                        name: *name,
                    },
                    NamedBindingKind::Workspace => continue,
                };
            }
            InstructionKind::StoreGlobal { name, src } => {
                let Some(binding) = following_remapped.get(&name.get()).copied() else {
                    continue;
                };
                instruction.kind = match binding {
                    NamedBindingKind::Local(local) => {
                        InstructionKind::StoreLocal { local, src: *src }
                    }
                    NamedBindingKind::Persistent(slot) => {
                        InstructionKind::StorePersistent { slot, src: *src }
                    }
                    NamedBindingKind::Capture => InstructionKind::StoreCapture {
                        name: *name,
                        src: *src,
                    },
                    NamedBindingKind::Workspace => continue,
                };
            }
            _ => {}
        }
    }
    Ok(inserted)
}

fn function_constant_name(
    function: &Function,
    constant: openmat_bytecode::ConstantId,
) -> Option<&str> {
    let Constant::String(name) = function.constants.get(constant.get() as usize)? else {
        return None;
    };
    Some(name)
}

fn name_load(function: &Function, instruction_index: usize) -> Option<NameLoad> {
    let instruction = function.instructions.get(instruction_index)?;
    let (kind, dst, names, root, member_count) = match &instruction.kind {
        InstructionKind::LoadGlobal {
            dst,
            name,
            construct_if_class: _,
        } => (
            NameLoadKind::Global,
            *dst,
            vec![function_constant_name(function, *name)?.to_owned()],
            None,
            0,
        ),
        InstructionKind::LoadCallTarget { dst, name } => (
            NameLoadKind::CallTarget,
            *dst,
            vec![function_constant_name(function, *name)?.to_owned()],
            None,
            0,
        ),
        InstructionKind::LoadFunctionHandle { dst, name } => (
            NameLoadKind::FunctionHandle,
            *dst,
            vec![function_constant_name(function, *name)?.to_owned()],
            None,
            0,
        ),
        InstructionKind::LoadFunctionHandleCandidates { dst, names } => (
            NameLoadKind::FunctionHandle,
            *dst,
            names
                .iter()
                .map(|name| function_constant_name(function, *name).map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
            None,
            0,
        ),
        InstructionKind::LoadQualifiedTarget {
            dst,
            root,
            qualified,
            member_count,
            ..
        } => (
            NameLoadKind::Qualified,
            *dst,
            qualified
                .iter()
                .map(|name| function_constant_name(function, *name).map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
            Some(function_constant_name(function, *root)?.to_owned()),
            *member_count,
        ),
        _ => return None,
    };
    Some(NameLoad {
        kind,
        destination: dst,
        names,
        root,
        member_count,
        location: instruction.location,
    })
}

fn candidate_resolution_names(candidate: &str, member_count: u32) -> Vec<String> {
    let components = candidate.split('.').collect::<Vec<_>>();
    let member_count = usize::try_from(member_count).unwrap_or(usize::MAX);
    let minimum = components.len().saturating_sub(member_count).max(1);
    (minimum..=components.len())
        .rev()
        .map(|component_count| components[..component_count].join("."))
        .collect()
}

fn reject_file_clear_import(
    unit: &CompiledSourceUnit,
    source_name: &str,
) -> Result<(), EngineError> {
    let entry = unit
        .module
        .functions
        .get(unit.module.entry.get() as usize)
        .ok_or_else(|| EngineError::new("source.link", "compiled entry is missing"))?;
    if entry
        .instructions
        .iter()
        .any(|instruction| matches!(instruction.kind, InstructionKind::ClearImports))
    {
        Err(EngineError::new(
            "compile.clearImportScope",
            format!(
                "`clear import` is available only in the command workspace, not source file `{source_name}`"
            ),
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalOutputSignature {
    Fixed(usize),
    Variadic { fixed: usize },
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalMethodSignature {
    parameters: u32,
    variadic_inputs: bool,
    outputs: ExternalOutputSignature,
}

fn external_method_declarations(
    module: &BytecodeModule,
    class_name: &str,
) -> Result<Vec<(String, ExternalMethodSignature)>, EngineError> {
    module
        .classes
        .iter()
        .find(|class| class.name == class_name)
        .ok_or_else(|| {
            EngineError::new(
                "source.link",
                format!("linked class `{class_name}` is missing its bytecode definition"),
            )
        })?
        .methods
        .iter()
        .filter(|method| method.is_external)
        .map(|method| {
            let signature = module
                .functions
                .get(method.function.get() as usize)
                .map(external_method_signature)
                .ok_or_else(|| {
                    EngineError::new(
                        "source.link",
                        format!(
                            "external method declaration `{class_name}.{}` has no signature function",
                            method.name
                        ),
                    )
                })?;
            Ok((method.name.clone(), signature))
        })
        .collect()
}

fn set_linked_external_method(
    module: &mut BytecodeModule,
    class_name: &str,
    method_name: &str,
    function: FunctionId,
) -> Result<(), EngineError> {
    let method = module
        .classes
        .iter_mut()
        .find(|class| class.name == class_name)
        .and_then(|class| {
            class
                .methods
                .iter_mut()
                .find(|method| method.name == method_name && method.is_external)
        })
        .ok_or_else(|| {
            EngineError::new(
                "source.link",
                format!("external method `{class_name}.{method_name}` disappeared while linking"),
            )
        })?;
    method.function = function;
    method.is_external = false;
    Ok(())
}

impl ExternalMethodSignature {
    fn describe(self) -> String {
        let inputs = if self.variadic_inputs {
            format!("{}+ inputs", self.parameters)
        } else {
            format!("{} inputs", self.parameters)
        };
        let outputs = match self.outputs {
            ExternalOutputSignature::Fixed(outputs) => format!("{outputs} outputs"),
            ExternalOutputSignature::Variadic { fixed } => format!("{fixed}+ outputs"),
            ExternalOutputSignature::Unknown => "unknown outputs".to_owned(),
        };
        format!("{inputs}, {outputs}")
    }
}

fn external_method_signature(function: &Function) -> ExternalMethodSignature {
    let variadic_inputs = function
        .instructions
        .iter()
        .any(|instruction| matches!(instruction.kind, InstructionKind::LoadVariadicInputs { .. }));
    let outputs = function
        .instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::Return { values } => {
                Some(ExternalOutputSignature::Fixed(values.len()))
            }
            InstructionKind::ReturnVariadic { fixed, .. } => {
                Some(ExternalOutputSignature::Variadic { fixed: fixed.len() })
            }
            _ => None,
        })
        .unwrap_or(ExternalOutputSignature::Unknown);
    ExternalMethodSignature {
        parameters: function.parameter_count,
        variadic_inputs,
        outputs,
    }
}

fn qualify_dependency(unit: &mut CompiledSourceUnit, requested: &str) -> Result<(), EngineError> {
    let leaf = requested.rsplit('.').next().unwrap_or(requested);
    if leaf == requested || matches!(unit.kind, SourceUnitKind::Script) {
        return Ok(());
    }

    let is_class = matches!(unit.kind, SourceUnitKind::Class { .. });
    match &mut unit.kind {
        SourceUnitKind::Function { name, .. } | SourceUnitKind::Class { name } => {
            if name != leaf {
                return Err(EngineError::new(
                    "source.link",
                    format!("cannot qualify source `{name}` as `{requested}`"),
                ));
            }
            requested.clone_into(name);
        }
        SourceUnitKind::Script => return Ok(()),
    }

    let entry = unit.module.entry.get() as usize;
    for (index, function) in unit.module.functions.iter_mut().enumerate() {
        if function.name == leaf && !(is_class && index == entry) {
            requested.clone_into(&mut function.name);
        } else if let Some(suffix) = function.name.strip_prefix(leaf)
            && suffix.starts_with('.')
        {
            function.name = format!("{requested}{suffix}");
        }
        qualify_function_name_operands(function, leaf, requested)?;
    }
    for class in &mut unit.module.classes {
        if class.name == leaf {
            requested.clone_into(&mut class.name);
        }
    }
    Ok(())
}

fn qualify_function_name_operands(
    function: &mut Function,
    leaf: &str,
    qualified: &str,
) -> Result<(), EngineError> {
    let mut replacements = BTreeMap::new();
    let original_constant_count = function.constants.len();
    for index in 0..original_constant_count {
        let Some(Constant::String(name)) = function.constants.get(index) else {
            continue;
        };
        let replacement = if name == leaf {
            Some(qualified.to_owned())
        } else {
            name.strip_prefix(leaf)
                .filter(|suffix| suffix.starts_with('.'))
                .map(|suffix| format!("{qualified}{suffix}"))
        };
        let Some(replacement) = replacement else {
            continue;
        };
        let replacement_index = u32::try_from(function.constants.len()).map_err(|_| {
            EngineError::new(
                "source.link",
                "qualified constant table exceeds the bytecode limit",
            )
        })?;
        let original_index = u32::try_from(index).map_err(|_| {
            EngineError::new(
                "source.link",
                "source constant table exceeds the bytecode limit",
            )
        })?;
        function.constants.push(Constant::String(replacement));
        replacements.insert(original_index, ConstantId::new(replacement_index));
    }

    let replace = |constant: &mut ConstantId| {
        if let Some(replacement) = replacements.get(&constant.get()) {
            *constant = *replacement;
        }
    };
    for instruction in &mut function.instructions {
        match &mut instruction.kind {
            InstructionKind::LoadGlobal { name, .. }
            | InstructionKind::LoadCallTarget { name, .. }
            | InstructionKind::LoadGlobalOrNothing { name, .. }
            | InstructionKind::LoadFunctionHandle { name, .. } => replace(name),
            InstructionKind::LoadQualifiedTarget {
                root, qualified, ..
            } => {
                replace(root);
                for candidate in qualified {
                    replace(candidate);
                }
            }
            InstructionKind::LoadFunctionHandleCandidates { names, .. } => {
                for name in names {
                    replace(name);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn function_clears_global(function: &Function, name: &str) -> bool {
    function.instructions.iter().any(|instruction| {
        let InstructionKind::ClearGlobal { names } = &instruction.kind else {
            return false;
        };
        names.is_empty()
            || names.iter().any(|constant| {
                matches!(
                    function.constants.get(constant.get() as usize),
                    Some(Constant::String(candidate)) if candidate == name
                )
            })
    })
}

fn validate_primary_file_name(path: &Path, kind: &SourceUnitKind) -> Result<(), EngineError> {
    let expected = match kind {
        SourceUnitKind::Function { name, .. } | SourceUnitKind::Class { name } => name,
        SourceUnitKind::Script => return Ok(()),
    };
    validate_file_stem(path, expected)
}

fn validate_dependency_name(
    requested: &str,
    kind: &SourceUnitKind,
    path: &Path,
) -> Result<(), EngineError> {
    let expected = requested.rsplit('.').next().unwrap_or(requested);
    match kind {
        SourceUnitKind::Function { name, .. } | SourceUnitKind::Class { name } => {
            if name != expected {
                return Err(EngineError::new(
                    "source.nameMismatch",
                    format!(
                        "source `{}` defines `{name}` but was resolved as `{requested}`",
                        path.to_string_lossy()
                    ),
                ));
            }
            validate_file_stem(path, expected)
        }
        SourceUnitKind::Script => Ok(()),
    }
}

fn validate_loaded_dependency_name(
    requested: &str,
    linked: &LinkedSource,
    path: &Path,
) -> Result<(), EngineError> {
    let defined = match linked {
        LinkedSource::Function { name, .. } | LinkedSource::Class { name } => name,
        LinkedSource::Script(_) => return Ok(()),
    };
    if defined == requested {
        Ok(())
    } else {
        Err(EngineError::new(
            "source.nameMismatch",
            format!(
                "source `{}` defines `{defined}` but was resolved as `{requested}`",
                path.to_string_lossy()
            ),
        ))
    }
}

fn validate_file_stem(path: &Path, expected: &str) -> Result<(), EngineError> {
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| {
            EngineError::new(
                "source.nonUtf8Path",
                "MATLAB function and class file names must be valid UTF-8",
            )
        })?;
    if stem == expected {
        Ok(())
    } else {
        Err(EngineError::new(
            "source.nameMismatch",
            format!(
                "source file `{}` must be named `{expected}.m`",
                path.to_string_lossy()
            ),
        ))
    }
}

fn is_explicit_file_source_name(source_name: &str) -> bool {
    let path = Path::new(source_name);
    path.is_absolute() || path.components().count() > 1
}

fn source_resolution_error(
    category: &'static str,
    code: &'static str,
    message: &str,
    location: Option<SourceLocation>,
    sources: &BTreeMap<u32, SourceRecord>,
    fallback_name: &str,
    fallback_len: usize,
) -> EngineError {
    let range = location.map(|location| {
        let fallback = SourceRecord {
            name: fallback_name.to_owned(),
            length: fallback_len,
        };
        let source = sources.get(&location.source_id).unwrap_or(&fallback);
        let source_len = u64::try_from(source.length).unwrap_or(u64::MAX);
        SourceRange {
            source_name: source.name.clone(),
            start: u64::from(location.start).min(source_len),
            end: u64::from(location.end).min(source_len),
        }
    });
    let diagnostic = Diagnostic {
        code: Some(code.to_owned()),
        severity: DiagnosticSeverity::Error,
        message: message.to_owned(),
        range,
        related: Vec::new(),
    };
    error_with_diagnostics(category, message, vec![diagnostic])
}

#[derive(Clone)]
struct SharedOutput {
    events: Arc<Mutex<Vec<OutputEvent>>>,
}

impl OutputSink for SharedOutput {
    fn emit(&mut self, event: OutputEvent) -> Result<(), OutputError> {
        let mut events = self
            .events
            .lock()
            .map_err(|_| OutputError::new("kernel display collector was poisoned"))?;
        events.push(event);
        Ok(())
    }
}

struct CancellationBridge {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl CancellationBridge {
    fn start(
        kernel: CancellationToken,
        runtime: RuntimeCancellationToken,
    ) -> Result<Self, EngineError> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("openmat-cancellation-bridge".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    if kernel.is_cancelled() {
                        runtime.cancel();
                        return;
                    }
                    thread::park_timeout(CANCELLATION_POLL_INTERVAL);
                }
            })
            .map_err(|error| {
                EngineError::new(
                    "runtime.cancellationBridge",
                    format!("failed to start cancellation bridge: {error}"),
                )
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    fn stop(mut self) -> Result<(), EngineError> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            handle.join().map_err(|_| {
                EngineError::new(
                    "runtime.cancellationBridge",
                    "cancellation bridge terminated unexpectedly",
                )
            })?;
        }
        Ok(())
    }
}

impl Drop for CancellationBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            drop(handle.join());
        }
    }
}

fn source_diagnostic(
    diagnostic: &SourceDiagnostic,
    source_name: &str,
    source_len: usize,
    fallback_code: &str,
) -> Diagnostic {
    Diagnostic {
        code: Some(
            diagnostic
                .code
                .clone()
                .unwrap_or_else(|| fallback_code.to_owned()),
        ),
        severity: match diagnostic.severity {
            Severity::Error => DiagnosticSeverity::Error,
            Severity::Warning => DiagnosticSeverity::Warning,
            Severity::Note => DiagnosticSeverity::Information,
        },
        message: diagnostic.message.clone(),
        range: Some(protocol_range(source_name, diagnostic.range, source_len)),
        related: Vec::new(),
    }
}

fn protocol_range(source_name: &str, range: TextRange, source_len: usize) -> SourceRange {
    let source_len = u64::try_from(source_len).unwrap_or(u64::MAX);
    let start = u64::from(range.start()).min(source_len);
    let end = u64::from(range.end()).min(source_len).max(start);
    SourceRange {
        source_name: source_name.to_owned(),
        start,
        end,
    }
}

fn diagnostic_sort_key(diagnostic: &Diagnostic) -> (u64, u64, String) {
    let (start, end) = diagnostic
        .range
        .as_ref()
        .map_or((u64::MAX, u64::MAX), |range| (range.start, range.end));
    (start, end, diagnostic.code.clone().unwrap_or_default())
}

fn error_with_diagnostics(
    category: &str,
    message: &str,
    diagnostics: Vec<Diagnostic>,
) -> EngineError {
    let events = diagnostics.iter().cloned().map(Event::Diagnostic).collect();
    EngineError::new(category, message)
        .with_diagnostics(diagnostics)
        .with_events(events)
}

fn runtime_build_error(
    error: &RuntimeBuildError,
    sources: &BTreeMap<u32, SourceRecord>,
    request: &ExecuteRequest,
) -> EngineError {
    let range = error
        .verification
        .location
        .map(|location| source_range_for_location(location, sources, request));
    let diagnostic = Diagnostic {
        code: Some("OMK0001".to_owned()),
        severity: DiagnosticSeverity::Error,
        message: error.to_string(),
        range,
        related: Vec::new(),
    };
    error_with_diagnostics(
        "compile.invalidBytecode",
        "compiled bytecode failed runtime verification",
        vec![diagnostic],
    )
}

fn runtime_diagnostic(
    error: &RuntimeError,
    sources: &BTreeMap<u32, SourceRecord>,
    request: &ExecuteRequest,
) -> Diagnostic {
    let range = error
        .location
        .map(|location| source_range_for_location(location, sources, request));
    let related = error
        .stack
        .iter()
        .filter_map(|frame| {
            frame.location.map(|location| RelatedDiagnostic {
                message: format!("in `{}`", frame.name),
                range: source_range_for_location(location, sources, request),
            })
        })
        .collect();
    Diagnostic {
        code: Some(runtime_code(&error.kind).to_owned()),
        severity: DiagnosticSeverity::Error,
        message: error.to_string(),
        range,
        related,
    }
}

fn source_range_for_location(
    location: openmat_bytecode::SourceLocation,
    sources: &BTreeMap<u32, SourceRecord>,
    request: &ExecuteRequest,
) -> SourceRange {
    let fallback = SourceRecord {
        name: request.source_name.clone(),
        length: request.code.len(),
    };
    let source = sources.get(&location.source_id).unwrap_or(&fallback);
    let source_len = u64::try_from(source.length).unwrap_or(u64::MAX);
    let start = u64::from(location.start).min(source_len);
    let end = u64::from(location.end).min(source_len).max(start);
    SourceRange {
        source_name: source.name.clone(),
        start,
        end,
    }
}

fn runtime_code(kind: &RuntimeErrorKind) -> &'static str {
    match kind {
        RuntimeErrorKind::Cancelled => "OMR0001",
        RuntimeErrorKind::UndefinedGlobal { .. } => "OMR0002",
        RuntimeErrorKind::NotCallable { .. } => "OMR0003",
        RuntimeErrorKind::InvalidFunctionHandle { .. } => "OMR0004",
        RuntimeErrorKind::UnknownFunctionHandleTarget { .. } => "OMR0018",
        RuntimeErrorKind::InputArity { .. } => "OMR0005",
        RuntimeErrorKind::MissingOutputs { .. } => "OMR0006",
        RuntimeErrorKind::InvalidOperands { .. } => "OMR0007",
        RuntimeErrorKind::UnsupportedArrayOperator { .. } => "OMR0008",
        RuntimeErrorKind::InvalidCondition { .. } => "OMR0009",
        RuntimeErrorKind::Builtin { .. } => "OMR0010",
        RuntimeErrorKind::Object { .. } => "OMR0014",
        RuntimeErrorKind::ClassConstraint { .. } => "OMR0020",
        RuntimeErrorKind::UnsupportedClassFeature { .. } => "OMR0015",
        RuntimeErrorKind::AccessViolation { .. } => "OMR0017",
        RuntimeErrorKind::CallDepthExceeded { .. } => "OMR0011",
        RuntimeErrorKind::InstructionPointerOutOfBounds { .. } => "OMR0012",
        RuntimeErrorKind::InvalidExecutionState { .. } if is_indexing_category(kind) => "OMR0016",
        RuntimeErrorKind::InvalidExecutionState { .. } if is_linalg_dimension_mismatch(kind) => {
            "OMR0019"
        }
        RuntimeErrorKind::InvalidExecutionState { .. } => "OMR0013",
    }
}

fn runtime_category(kind: &RuntimeErrorKind) -> &'static str {
    match kind {
        RuntimeErrorKind::Cancelled => "runtime.cancelled",
        RuntimeErrorKind::UndefinedGlobal { .. }
        | RuntimeErrorKind::ClassConstraint {
            constraint: ClassConstraintKind::AbstractMethodUnavailable,
            ..
        } => "runtime.undefinedName",
        RuntimeErrorKind::NotCallable { .. } => "runtime.notCallable",
        RuntimeErrorKind::InvalidFunctionHandle { .. } => "runtime.invalidFunction",
        RuntimeErrorKind::UnknownFunctionHandleTarget { .. } => {
            "runtime.unknownFunctionHandleTarget"
        }
        RuntimeErrorKind::InputArity { .. } => "runtime.inputArity",
        RuntimeErrorKind::MissingOutputs { .. } => "runtime.missingOutputs",
        RuntimeErrorKind::InvalidOperands { .. }
        | RuntimeErrorKind::Builtin {
            category: BuiltinErrorCategory::Type,
            ..
        }
        | RuntimeErrorKind::ClassConstraint { .. } => "runtime.invalidOperands",
        RuntimeErrorKind::UnsupportedArrayOperator { .. } => "runtime.unsupportedArrayOperator",
        RuntimeErrorKind::InvalidCondition { .. } => "runtime.invalidCondition",
        RuntimeErrorKind::Builtin { .. } => "runtime.builtin",
        RuntimeErrorKind::Object { .. } => "runtime.object",
        RuntimeErrorKind::UnsupportedClassFeature { .. } => "runtime.unsupportedClassFeature",
        RuntimeErrorKind::AccessViolation { .. } => "access-violation",
        RuntimeErrorKind::CallDepthExceeded { .. } => "runtime.callDepth",
        RuntimeErrorKind::InstructionPointerOutOfBounds { .. } => "runtime.instructionPointer",
        RuntimeErrorKind::InvalidExecutionState { .. } if is_indexing_category(kind) => {
            "runtime.indexOutOfBounds"
        }
        RuntimeErrorKind::InvalidExecutionState { .. } if is_linalg_dimension_mismatch(kind) => {
            "runtime.dimensionMismatch"
        }
        RuntimeErrorKind::InvalidExecutionState { .. } => "runtime.invalidState",
    }
}

fn is_linalg_dimension_mismatch(kind: &RuntimeErrorKind) -> bool {
    matches!(
        kind.linalg_error(),
        Some(RuntimeLinalgError::Linalg(
            LinalgError::DimensionMismatch { .. }
        ))
    )
}

fn is_indexing_category(kind: &RuntimeErrorKind) -> bool {
    matches!(
        kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(array),
            ..
        } if matches!(
            array.as_ref(),
            ArrayRuntimeError::InvalidIndex { .. }
                | ArrayRuntimeError::IndexOutOfBounds { .. }
                | ArrayRuntimeError::AssignmentSizeMismatch { .. }
        )
    )
}

fn display_events(
    events: Vec<OutputEvent>,
    interpreter: &Interpreter,
    initial_display_format: openmat_runtime::DisplayFormat,
) -> (Vec<Event>, Vec<GraphicsNotice>) {
    let mut display = Vec::new();
    let mut graphics = Vec::new();
    let mut display_format = initial_display_format;
    for event in events {
        match event {
            OutputEvent::UiDisplay(payload) => {
                let mut representations = BTreeMap::new();
                representations.insert("application/vnd.openmat.ui+json".to_owned(), payload);
                display.push(Event::Display(DisplayEvent { representations }));
            }
            OutputEvent::Display(value) => {
                let mut representations = BTreeMap::new();
                representations.insert(
                    "text/plain".to_owned(),
                    format_display_value_with(&value, interpreter, display_format.numeric),
                );
                display.push(Event::Display(DisplayEvent { representations }));
            }
            OutputEvent::NamedDisplay { name, value } => {
                let mut representations = BTreeMap::new();
                let separator = match display_format.line_spacing {
                    openmat_runtime::LineSpacing::Compact => "\n",
                    openmat_runtime::LineSpacing::Loose => "\n\n",
                };
                representations.insert(
                    "text/plain".to_owned(),
                    format!(
                        "{name} ={separator}{}",
                        format_display_value_with(&value, interpreter, display_format.numeric)
                    ),
                );
                display.push(Event::Display(DisplayEvent { representations }));
            }
            OutputEvent::GraphicsNotice(notice) => graphics.push(notice),
            OutputEvent::CommandWindowClear => {
                let mut representations = BTreeMap::new();
                representations.insert(COMMAND_WINDOW_CLEAR_MIME.to_owned(), "{}".to_owned());
                display.push(Event::Display(DisplayEvent { representations }));
            }
            OutputEvent::CommandText(text) => display.push(Event::Stream(StreamEvent {
                stream: StreamKind::Stdout,
                text,
            })),
            OutputEvent::DisplayFormatChanged(format) => display_format = format,
        }
    }
    (display, graphics)
}

#[cfg(test)]
fn format_display_value(value: &Value, interpreter: &Interpreter) -> String {
    format_display_value_with(value, interpreter, interpreter.display_format().numeric)
}

fn variable_summary(name: &str, value: &Value, class_name: &str) -> VariableSummary {
    let dimensions = value
        .dimensions()
        .map_or_else(|| vec![1, 1], <[u64]>::to_vec);
    let complex = value.is_complex_numeric();
    VariableSummary {
        name: name.to_owned(),
        class: class_name.to_owned(),
        dimensions,
        complex,
        bytes: value_bytes(value),
    }
}

// Converting an editor value into an existing `single` array intentionally
// applies IEEE-754 binary32 rounding, matching assignment into single storage.
#[allow(clippy::cast_possible_truncation)]
fn set_numeric_element(
    target: &mut Value,
    one_based: u64,
    real: f64,
    imaginary: f64,
) -> Result<(), EngineError> {
    let offset_error = || {
        EngineError::new(
            "workspace.indexOutOfBounds",
            "workspace element index escaped validated dimensions",
        )
    };
    match target {
        Value::Double(value) if one_based == 1 => {
            if imaginary == 0.0 {
                *value = real;
            } else {
                *target = Value::Complex(openmat_value::Complex64::new(real, imaginary));
            }
        }
        Value::Complex(value) if one_based == 1 => {
            *value = openmat_value::Complex64::new(real, imaginary);
        }
        Value::Logical(value) if one_based == 1 => {
            *value = real != 0.0 || imaginary != 0.0;
        }
        Value::Array(ArrayData::F64(array)) if imaginary == 0.0 => {
            *array
                .get_mut_linear(one_based)
                .map_err(|_| offset_error())? = real;
        }
        Value::Array(ArrayData::F64(array)) => {
            let shape = array.shape().clone();
            let mut values = array
                .as_slice()
                .iter()
                .map(|value| ArrayComplex64::new(*value, 0.0))
                .collect::<Vec<_>>();
            let offset = usize::try_from(one_based - 1).map_err(|_| offset_error())?;
            values[offset] = ArrayComplex64::new(real, imaginary);
            *target = Value::Array(ArrayData::ComplexF64(
                DenseArray::from_vec(shape, values).map_err(|error| {
                    EngineError::new(
                        "workspace.editFailed",
                        format!("could not promote the edited array to complex storage: {error}"),
                    )
                })?,
            ));
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            *array
                .get_mut_linear(one_based)
                .map_err(|_| offset_error())? = ArrayComplex64::new(real, imaginary);
        }
        Value::Array(ArrayData::Logical(array)) => {
            *array
                .get_mut_linear(one_based)
                .map_err(|_| offset_error())? =
                openmat_array::Logical::from(real != 0.0 || imaginary != 0.0);
        }
        Value::Array(ArrayData::F32(array)) if imaginary == 0.0 => {
            *array
                .get_mut_linear(one_based)
                .map_err(|_| offset_error())? = real as f32;
        }
        Value::Array(ArrayData::F32(array)) => {
            let shape = array.shape().clone();
            let mut values = array
                .as_slice()
                .iter()
                .map(|value| Complex32::new(*value, 0.0))
                .collect::<Vec<_>>();
            let offset = usize::try_from(one_based - 1).map_err(|_| offset_error())?;
            values[offset] = Complex32::new(real as f32, imaginary as f32);
            *target = Value::Array(ArrayData::ComplexF32(
                DenseArray::from_vec(shape, values).map_err(|error| {
                    EngineError::new(
                        "workspace.editFailed",
                        format!("could not promote the edited single array: {error}"),
                    )
                })?,
            ));
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            *array
                .get_mut_linear(one_based)
                .map_err(|_| offset_error())? = Complex32::new(real as f32, imaginary as f32);
        }
        Value::Double(_) | Value::Complex(_) | Value::Logical(_) => {
            return Err(offset_error());
        }
        _ => {
            return Err(EngineError::new(
                "workspace.editUnsupported",
                format!(
                    "workspace variable class `{}` is not editable in this Variable Editor tranche",
                    target.class_name(),
                ),
            ));
        }
    }
    Ok(())
}

fn set_integer_element(
    storage: &mut IntegerArrayData,
    one_based: u64,
    real: &str,
    imaginary: &str,
) -> Result<(), EngineError> {
    fn normalized_zero(value: &str) -> &str {
        if value == "-0" { "0" } else { value }
    }

    let class_name = storage.class_name();
    let invalid_component = |component: &str| {
        EngineError::new(
            "workspace.invalidValue",
            format!(
                "`{component}` is not an exact in-range integer component for class `{class_name}`"
            ),
        )
    };
    let index_error = || {
        EngineError::new(
            "workspace.indexOutOfBounds",
            "workspace element index escaped validated dimensions",
        )
    };
    let real_storage_complex_error = || {
        EngineError::new(
            "workspace.editUnsupported",
            format!(
                "a real `{class_name}` array cannot receive an imaginary component in this edit"
            ),
        )
    };

    macro_rules! set_real {
        ($array:expr, $component:ty) => {{
            if !matches!(imaginary, "0" | "-0") {
                return Err(real_storage_complex_error());
            }
            let parsed = normalized_zero(real)
                .parse::<$component>()
                .map_err(|_| invalid_component(real))?;
            *$array
                .get_mut_linear(one_based)
                .map_err(|_| index_error())? = parsed;
        }};
    }
    macro_rules! set_complex {
        ($array:expr, $component:ty) => {{
            let parsed_real = normalized_zero(real)
                .parse::<$component>()
                .map_err(|_| invalid_component(real))?;
            let parsed_imaginary = normalized_zero(imaginary)
                .parse::<$component>()
                .map_err(|_| invalid_component(imaginary))?;
            *$array
                .get_mut_linear(one_based)
                .map_err(|_| index_error())? = ComplexInteger::new(parsed_real, parsed_imaginary);
        }};
    }

    match storage {
        IntegerArrayData::I8(array) => set_real!(array, i8),
        IntegerArrayData::ComplexI8(array) => set_complex!(array, i8),
        IntegerArrayData::U8(array) => set_real!(array, u8),
        IntegerArrayData::ComplexU8(array) => set_complex!(array, u8),
        IntegerArrayData::I16(array) => set_real!(array, i16),
        IntegerArrayData::ComplexI16(array) => set_complex!(array, i16),
        IntegerArrayData::U16(array) => set_real!(array, u16),
        IntegerArrayData::ComplexU16(array) => set_complex!(array, u16),
        IntegerArrayData::I32(array) => set_real!(array, i32),
        IntegerArrayData::ComplexI32(array) => set_complex!(array, i32),
        IntegerArrayData::U32(array) => set_real!(array, u32),
        IntegerArrayData::ComplexU32(array) => set_complex!(array, u32),
        IntegerArrayData::I64(array) => set_real!(array, i64),
        IntegerArrayData::ComplexI64(array) => set_complex!(array, i64),
        IntegerArrayData::U64(array) => set_real!(array, u64),
        IntegerArrayData::ComplexU64(array) => set_complex!(array, u64),
    }
    Ok(())
}

fn value_bytes(value: &Value) -> Option<u64> {
    match value {
        Value::Nothing => Some(0),
        Value::Logical(_) => Some(1),
        Value::Double(_) | Value::Object(_) | Value::Function(_) => Some(8),
        Value::ObjectArray(array) => array.numel().checked_mul(8),
        Value::Graphics(_) => Some(12),
        Value::GraphicsArray(array) => array.numel().checked_mul(12),
        Value::Complex(_) => Some(16),
        Value::String(value) => string_value_bytes(value),
        Value::Sparse(array) => array.payload_bytes(),
        Value::Cell(_) | Value::Struct(_) | Value::Table(_) => None,
        Value::Array(array) => {
            let element_width = u64::try_from(array.dtype().element_width_bytes()).ok()?;
            array.numel().checked_mul(element_width)
        }
    }
}

fn string_value_bytes(value: &openmat_value::StringValue) -> Option<u64> {
    let code_units = u64::try_from(value.utf16_code_unit_len()).ok()?;
    let code_unit_bytes = code_units.checked_mul(2)?;
    let missing_bytes = value.numel();
    let offset_bytes = value.numel().checked_add(1)?.checked_mul(8)?;
    code_unit_bytes
        .checked_add(missing_bytes)?
        .checked_add(offset_bytes)
}

fn workspace_delta(
    before: &Workspace,
    after: &Workspace,
    interpreter: &Interpreter,
) -> WorkspaceDeltaEvent {
    let before = before.iter().collect::<BTreeMap<_, _>>();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut removed = before
        .keys()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    for (name, value) in after.iter() {
        match before.get(name) {
            None => added.push(variable_summary(
                name,
                value,
                &interpreter.value_class_name(value),
            )),
            Some(previous) if values_changed(previous, value) => {
                changed.push(variable_summary(
                    name,
                    value,
                    &interpreter.value_class_name(value),
                ));
            }
            Some(_) => {}
        }
        if let Ok(index) = removed.binary_search_by(|candidate| candidate.as_str().cmp(name)) {
            removed.remove(index);
        }
    }

    WorkspaceDeltaEvent {
        added,
        changed,
        removed,
    }
}

fn values_changed(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Double(left), Value::Double(right)) => left.to_bits() != right.to_bits(),
        (Value::Complex(left), Value::Complex(right)) => {
            left.real.to_bits() != right.real.to_bits()
                || left.imaginary.to_bits() != right.imaginary.to_bits()
        }

        (Value::Array(left), Value::Array(right)) => arrays_changed(left, right),
        (Value::Object(_), Value::Object(_)) | (Value::ObjectArray(_), Value::ObjectArray(_)) => {
            true
        }
        _ => left != right,
    }
}

fn arrays_changed(left: &ArrayData, right: &ArrayData) -> bool {
    match (left, right) {
        (ArrayData::F32(left), ArrayData::F32(right)) => {
            left.shape() != right.shape()
                || left
                    .as_slice()
                    .iter()
                    .zip(right.as_slice())
                    .any(|(left, right)| left.to_bits() != right.to_bits())
        }
        (ArrayData::ComplexF32(left), ArrayData::ComplexF32(right)) => {
            left.shape() != right.shape()
                || left
                    .as_slice()
                    .iter()
                    .zip(right.as_slice())
                    .any(|(left, right)| {
                        left.re.to_bits() != right.re.to_bits()
                            || left.im.to_bits() != right.im.to_bits()
                    })
        }
        (ArrayData::F64(left), ArrayData::F64(right)) => {
            left.shape() != right.shape()
                || left
                    .as_slice()
                    .iter()
                    .zip(right.as_slice())
                    .any(|(left, right)| left.to_bits() != right.to_bits())
        }
        (ArrayData::ComplexF64(left), ArrayData::ComplexF64(right)) => {
            left.shape() != right.shape()
                || left
                    .as_slice()
                    .iter()
                    .zip(right.as_slice())
                    .any(|(left, right)| {
                        left.re.to_bits() != right.re.to_bits()
                            || left.im.to_bits() != right.im.to_bits()
                    })
        }
        (ArrayData::Logical(left), ArrayData::Logical(right)) => left != right,
        (ArrayData::Char(left), ArrayData::Char(right)) => left != right,
        (ArrayData::Integer(left), ArrayData::Integer(right)) => left != right,
        _ => true,
    }
}

fn validate_inspect_request(request: &InspectRequest) -> Result<(), EngineError> {
    request.range.validate().map_err(|error| {
        EngineError::new(
            "workspace.invalidRange",
            format!("invalid matrix range: {error}"),
        )
    })?;
    if request.max_elements == 0 || request.max_elements > MAX_PREVIEW_ELEMENTS {
        return Err(EngineError::new(
            "workspace.previewLimit",
            format!("maxElements must be between 1 and {MAX_PREVIEW_ELEMENTS}"),
        ));
    }
    Ok(())
}

fn inspect_value(
    value: &Value,
    class_name: &str,
    request: &InspectRequest,
) -> Result<MatrixPreview, EngineError> {
    if matches!(value.kind(), ValueKind::Char | ValueKind::Integer) {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            format!("values of class `{class_name}` cannot be represented losslessly in kernel-v0"),
        ));
    }
    if matches!(value, Value::Nothing | Value::Function(_)) {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            format!("values of class `{class_name}` cannot be inspected"),
        ));
    }
    let dimensions = value
        .dimensions()
        .ok_or_else(|| EngineError::new("workspace.unsupportedValue", "value has no dimensions"))?
        .to_vec();
    validate_selection(&request.range.start, &request.range.size, &dimensions)?;
    let selected_count = checked_product(&request.range.size).ok_or_else(|| {
        EngineError::new(
            "workspace.invalidRange",
            "selected matrix range element count overflows u64",
        )
    })?;
    if let Value::String(strings) = value {
        for position in 0..selected_count {
            let offset = selection_offset(
                position,
                &request.range.start,
                &request.range.size,
                &dimensions,
            )
            .ok_or_else(selection_offset_error)?;
            let offset = usize::try_from(offset).map_err(|_| {
                EngineError::new(
                    "workspace.previewLimit",
                    "matrix offset does not fit this host",
                )
            })?;
            let element = strings.element(offset).ok_or_else(preview_offset_error)?;
            string_text_v0(element)?;
        }
    }
    let returned_count = selected_count
        .min(request.max_elements)
        .min(MAX_PREVIEW_ELEMENTS);
    let capacity = usize::try_from(returned_count).map_err(|_| {
        EngineError::new(
            "workspace.previewLimit",
            "preview element count does not fit this host",
        )
    })?;
    let mut values = Vec::with_capacity(capacity);
    for position in 0..returned_count {
        let offset = selection_offset(
            position,
            &request.range.start,
            &request.range.size,
            &dimensions,
        )
        .ok_or_else(selection_offset_error)?;
        values.push(preview_value_v0(value, offset)?);
    }
    let omitted_elements = selected_count - returned_count;
    Ok(MatrixPreview {
        class: class_name.to_owned(),
        dimensions,
        selected_range: request.range.clone(),
        values,
        truncation: PreviewTruncation {
            truncated: omitted_elements > 0,
            omitted_elements,
        },
    })
}

fn inspect_value_v1(
    value: &Value,
    class_name: &str,
    request: &kernel_v1::InspectRequest,
    limits: &kernel_v1::PreviewLimits,
) -> Result<kernel_v1::MatrixPreview, EngineError> {
    if matches!(
        value,
        Value::Nothing | Value::Function(_) | Value::Object(_) | Value::ObjectArray(_)
    ) {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            format!("values of class `{class_name}` cannot be inspected in kernel-v1"),
        ));
    }
    let dimensions = value
        .dimensions()
        .ok_or_else(|| EngineError::new("workspace.unsupportedValue", "value has no dimensions"))?
        .to_vec();
    validate_selection(&request.range.start, &request.range.size, &dimensions)?;
    let selected_count = checked_product(&request.range.size).ok_or_else(|| {
        EngineError::new(
            "workspace.invalidRange",
            "selected matrix range element count overflows u64",
        )
    })?;

    validate_selected_complex_floats(value, request, &dimensions, selected_count)?;

    let effective_limit = request.max_elements.min(limits.max_preview_elements);
    let capacity = usize::try_from(selected_count.min(effective_limit)).map_err(|_| {
        EngineError::new(
            "workspace.previewLimit",
            "preview element count does not fit this host",
        )
    })?;
    let mut values = Vec::with_capacity(capacity);
    let mut aggregate_code_units = 0_u64;
    for position in 0..selected_count {
        if u64::try_from(values.len()).unwrap_or(u64::MAX) == effective_limit {
            break;
        }
        let offset = selection_offset(
            position,
            &request.range.start,
            &request.range.size,
            &dimensions,
        )
        .ok_or_else(selection_offset_error)?;
        let Some(preview) = preview_value_v1(value, offset, limits, &mut aggregate_code_units)?
        else {
            break;
        };
        values.push(preview);
    }

    let complex = value.is_complex_numeric();
    if complex
        && matches!(value.kind(), ValueKind::Integer)
        && !values.iter().any(|value| {
            matches!(
                value,
                kernel_v1::PreviewValue::Integer { imaginary, .. } if imaginary != "0"
            )
        })
    {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            "the bounded complex-integer prefix contains no nonzero imaginary component",
        ));
    }

    let returned_count = u64::try_from(values.len()).map_err(|_| {
        EngineError::new(
            "workspace.previewLimit",
            "preview element count does not fit u64",
        )
    })?;
    let omitted_elements = selected_count - returned_count;
    Ok(kernel_v1::MatrixPreview {
        class: class_name.to_owned(),
        dimensions,
        complex,
        selected_range: request.range.clone(),
        values,
        truncation: PreviewTruncation {
            truncated: omitted_elements > 0,
            omitted_elements,
        },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ValueIdentity(usize);

impl ValueIdentity {
    fn of(value: &Value) -> Self {
        Self(std::ptr::from_ref(value).addr())
    }
}

#[allow(clippy::too_many_lines)]
fn inspect_value_v2(
    value: &Value,
    class_name: &str,
    request: &kernel_v2::InspectRequest,
    limits: &kernel_v2::AggregateLimits,
) -> Result<kernel_v2::InspectPreview, EngineError> {
    if !matches!(value, Value::Cell(_) | Value::Struct(_) | Value::Table(_)) {
        return inspect_value_v1(value, class_name, request, &limits.preview_limits())
            .map(kernel_v2::InspectPreview::Matrix);
    }

    let dimensions = canonical_dimensions(value)?;
    canonical_numel(value, &dimensions)?;
    validate_selection(&request.range.start, &request.range.size, &dimensions)?;
    let selected_count = if matches!(value, Value::Table(_)) {
        request.range.size[1]
    } else {
        checked_product(&request.range.size)
            .ok_or_else(|| preview_limit("selected aggregate range element count overflows u64"))?
    };
    ensure_safe_wire_integer(selected_count, "selected aggregate item count")?;
    let effective_limit = request.max_elements.min(limits.max_preview_elements);
    let capacity = usize::try_from(selected_count.min(effective_limit))
        .map_err(|_| preview_limit("aggregate preview element count does not fit this host"))?;
    let mut guard = ActivePathGuard::default();
    enter_active(&mut guard, ValueIdentity::of(value))?;

    let result = match value {
        Value::Cell(cell) => {
            let mut items = Vec::with_capacity(capacity);
            let mut usage = kernel_v2::PreviewUsage {
                nodes: 1,
                elements: 0,
                code_units: 0,
                depth: 0,
            };
            for position in 0..selected_count {
                if u64::try_from(items.len()).unwrap_or(u64::MAX) == effective_limit {
                    break;
                }
                let offset = aggregate_selection_offset(position, request, &dimensions)?;
                let child = cell
                    .value_at_offset(offset)
                    .ok_or_else(preview_offset_error)?;
                let mut candidate_usage = kernel_v2::PreviewUsage {
                    nodes: 0,
                    elements: 1,
                    code_units: 0,
                    depth: 0,
                };
                let candidate = exact_value(child, 1, limits, &mut candidate_usage, &mut guard)?;
                if !commit_candidate_usage(&mut usage, candidate_usage, limits)? {
                    break;
                }
                items.push(candidate);
            }
            let returned = u64::try_from(items.len())
                .map_err(|_| preview_limit("returned cell prefix does not fit u64"))?;
            Ok(kernel_v2::AggregatePreview::Cell {
                dimensions,
                selected_range: request.range.clone(),
                items,
                truncation: preview_truncation(selected_count, returned)?,
                usage,
            })
        }
        Value::Struct(structure) => {
            let fields = aggregate_fields(structure)?;
            let root_code_units = field_code_units(&fields)?;
            if root_code_units > limits.max_preview_code_units {
                return finish_active(
                    &mut guard,
                    ValueIdentity::of(value),
                    Err(preview_limit(
                        "root struct schema exceeds the negotiated code-unit limit",
                    )),
                )
                .map(kernel_v2::InspectPreview::Aggregate);
            }
            let mut records = Vec::with_capacity(capacity);
            let mut usage = kernel_v2::PreviewUsage {
                nodes: 1,
                elements: 0,
                code_units: root_code_units,
                depth: 0,
            };
            for position in 0..selected_count {
                if u64::try_from(records.len()).unwrap_or(u64::MAX) == effective_limit {
                    break;
                }
                let offset = aggregate_selection_offset(position, request, &dimensions)?;
                let mut candidate_usage = kernel_v2::PreviewUsage {
                    nodes: 0,
                    elements: 1,
                    code_units: 0,
                    depth: 0,
                };
                let mut entries = Vec::with_capacity(fields.len());
                for (field_index, field) in fields.iter().enumerate() {
                    let child = structure
                        .value_at(field_index, offset)
                        .ok_or_else(|| invalid_preview("struct field storage is incomplete"))?;
                    entries.push((
                        field.clone(),
                        exact_value(child, 1, limits, &mut candidate_usage, &mut guard)?,
                    ));
                }
                if !commit_candidate_usage(&mut usage, candidate_usage, limits)? {
                    break;
                }
                records.push(kernel_v2::StructRecord::new(entries));
            }
            let returned = u64::try_from(records.len())
                .map_err(|_| preview_limit("returned struct prefix does not fit u64"))?;
            Ok(kernel_v2::AggregatePreview::Struct {
                dimensions,
                selected_range: request.range.clone(),
                fields,
                records,
                truncation: preview_truncation(selected_count, returned)?,
                usage,
            })
        }
        Value::Table(table) => {
            let row_start = request.range.start[0];
            let selected_rows = request.range.size[0];
            let first_variable = request.range.start[1]
                .checked_sub(1)
                .ok_or_else(|| invalid_preview("table variable range starts at zero"))?;
            let first_variable = usize::try_from(first_variable)
                .map_err(|_| preview_limit("table variable offset does not fit this host"))?;
            let mut variable_names = Vec::with_capacity(capacity);
            let mut variables = Vec::with_capacity(capacity);
            let mut usage = kernel_v2::PreviewUsage {
                nodes: 1,
                elements: 0,
                code_units: 0,
                depth: 0,
            };
            for position in 0..selected_count {
                if u64::try_from(variables.len()).unwrap_or(u64::MAX) == effective_limit {
                    break;
                }
                let position = usize::try_from(position)
                    .map_err(|_| preview_limit("table variable position does not fit this host"))?;
                let index = first_variable
                    .checked_add(position)
                    .ok_or_else(|| preview_limit("table variable offset overflows this host"))?;
                let name = table
                    .variable_names()
                    .get(index)
                    .ok_or_else(|| invalid_preview("table variable selection escaped schema"))?
                    .as_str()
                    .to_owned();
                let variable = table
                    .variable(index)
                    .ok_or_else(|| invalid_preview("table variable storage is incomplete"))?;
                let sliced = slice_table_variable_rows(variable, row_start, selected_rows)?;
                let name_code_units = u64::try_from(name.encode_utf16().count())
                    .map_err(|_| preview_limit("table variable name length does not fit u64"))?;
                let mut candidate_usage = kernel_v2::PreviewUsage {
                    nodes: 0,
                    elements: 1,
                    code_units: name_code_units,
                    depth: 0,
                };
                let candidate = exact_value(&sliced, 1, limits, &mut candidate_usage, &mut guard)?;
                if !commit_candidate_usage(&mut usage, candidate_usage, limits)? {
                    break;
                }
                variable_names.push(name);
                variables.push(candidate);
            }
            let returned = u64::try_from(variables.len())
                .map_err(|_| preview_limit("returned table prefix does not fit u64"))?;
            Ok(kernel_v2::AggregatePreview::Table {
                dimensions,
                selected_range: request.range.clone(),
                variable_names,
                variables,
                truncation: preview_truncation(selected_count, returned)?,
                usage,
            })
        }
        _ => unreachable!("aggregate kind checked above"),
    };
    let preview = finish_active(&mut guard, ValueIdentity::of(value), result)?;
    let preview = kernel_v2::InspectPreview::Aggregate(preview);
    preview
        .validate_for_request(limits, request.max_elements)
        .map_err(|error| {
            invalid_preview(format!(
                "constructed aggregate preview failed protocol validation: {error}"
            ))
        })?;
    Ok(preview)
}

#[allow(clippy::too_many_lines)]
fn slice_table_variable_rows(
    value: &Value,
    row_start: u64,
    row_count: u64,
) -> Result<Value, EngineError> {
    if matches!(
        value,
        Value::Nothing
            | Value::Sparse(_)
            | Value::Object(_)
            | Value::ObjectArray(_)
            | Value::Graphics(_)
            | Value::GraphicsArray(_)
            | Value::Function(_)
    ) {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            format!(
                "table variables of kind `{}` have no canonical kernel-v2 row slice",
                value.kind()
            ),
        ));
    }
    let dimensions = canonical_dimensions(value)?;
    validate_row_slice(row_start, row_count, &dimensions)?;
    let full_rows = dimensions[0];
    if row_count == full_rows && (row_count == 0 || row_start == 1) {
        return Ok(value.clone());
    }
    let shape = row_slice_shape(&dimensions, row_count)?;
    match value {
        Value::Logical(_) => empty_scalar_row_slice(
            shape,
            ArrayData::Logical,
            "logical table variable row slice",
        ),
        Value::Double(_) => {
            empty_scalar_row_slice(shape, ArrayData::F64, "double table variable row slice")
        }
        Value::Complex(_) => empty_scalar_row_slice(
            shape,
            ArrayData::ComplexF64,
            "complex table variable row slice",
        ),
        Value::Array(ArrayData::F32(array)) => slice_dense_rows(
            array,
            row_start,
            row_count,
            "single table variable row slice",
        )
        .map(ArrayData::F32)
        .map(Value::Array),
        Value::Array(ArrayData::ComplexF32(array)) => slice_dense_rows(
            array,
            row_start,
            row_count,
            "complex single table variable row slice",
        )
        .map(ArrayData::ComplexF32)
        .map(Value::Array),
        Value::String(StringValue::Scalar(_)) => StringArray::from_elements(shape, Vec::new())
            .map(StringValue::array)
            .map(Value::String)
            .map_err(|error| row_slice_error("string table variable row slice", error)),
        Value::String(StringValue::Array(array)) => {
            let values =
                slice_column_major_rows(array.as_slice(), &dimensions, row_start, row_count)?;
            StringArray::from_elements(shape, values)
                .map(StringValue::array)
                .map(Value::String)
                .map_err(|error| row_slice_error("string table variable row slice", error))
        }
        Value::Array(ArrayData::F64(array)) => slice_dense_rows(
            array,
            row_start,
            row_count,
            "double table variable row slice",
        )
        .map(ArrayData::F64)
        .map(Value::Array),
        Value::Array(ArrayData::ComplexF64(array)) => slice_dense_rows(
            array,
            row_start,
            row_count,
            "complex double table variable row slice",
        )
        .map(ArrayData::ComplexF64)
        .map(Value::Array),
        Value::Array(ArrayData::Logical(array)) => slice_dense_rows(
            array,
            row_start,
            row_count,
            "logical table variable row slice",
        )
        .map(ArrayData::Logical)
        .map(Value::Array),
        Value::Array(ArrayData::Char(array)) => {
            slice_dense_rows(array, row_start, row_count, "char table variable row slice")
                .map(ArrayData::Char)
                .map(Value::Array)
        }
        Value::Array(ArrayData::Integer(array)) => slice_integer_rows(array, row_start, row_count)
            .map(ArrayData::Integer)
            .map(Value::Array),
        Value::Cell(cell) => {
            let values = slice_column_major_rows(cell.values(), &dimensions, row_start, row_count)?;
            CellArray::from_values(shape, values)
                .map(Value::Cell)
                .map_err(|error| row_slice_error("cell table variable row slice", error))
        }
        Value::Struct(structure) => {
            let fields = structure.field_names().to_vec();
            let mut columns = Vec::with_capacity(structure.field_count());
            for field in 0..structure.field_count() {
                let source = structure
                    .field_values(field)
                    .ok_or_else(|| invalid_preview("struct field storage is incomplete"))?;
                columns.push(slice_column_major_rows(
                    source,
                    &dimensions,
                    row_start,
                    row_count,
                )?);
            }
            StructArray::from_columns(shape, fields, columns)
                .map(Value::Struct)
                .map_err(|error| row_slice_error("struct table variable row slice", error))
        }
        Value::Table(table) => {
            let mut names = Vec::with_capacity(table.variable_count());
            let mut variables = Vec::with_capacity(table.variable_count());
            for index in 0..table.variable_count() {
                names.push(
                    table
                        .variable_names()
                        .get(index)
                        .ok_or_else(|| invalid_preview("nested table schema is incomplete"))?
                        .clone(),
                );
                variables.push(slice_table_variable_rows(
                    table
                        .variable(index)
                        .ok_or_else(|| invalid_preview("nested table storage is incomplete"))?,
                    row_start,
                    row_count,
                )?);
            }
            TableArray::from_parts(row_count, names, variables)
                .map(Value::Table)
                .map_err(|error| row_slice_error("nested table variable row slice", error))
        }
        Value::Nothing
        | Value::Sparse(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => unreachable!("unsupported table variable kinds returned above"),
    }
}

fn validate_row_slice(
    row_start: u64,
    row_count: u64,
    dimensions: &[u64],
) -> Result<(), EngineError> {
    let Some(&rows) = dimensions.first() else {
        return Err(invalid_preview("table variable has no row dimension"));
    };
    if row_start == 0 {
        return Err(EngineError::new(
            "workspace.invalidRange",
            "table row range starts at zero; indices are one-based",
        ));
    }
    if row_count == 0 {
        return Ok(());
    }
    let end = row_start
        .checked_add(row_count - 1)
        .ok_or_else(|| EngineError::new("workspace.invalidRange", "table row range overflows"))?;
    if end > rows {
        return Err(EngineError::new(
            "workspace.invalidRange",
            format!("table row selection exceeds extent {rows}"),
        ));
    }
    Ok(())
}

fn row_slice_shape(dimensions: &[u64], row_count: u64) -> Result<Shape, EngineError> {
    let mut sliced = dimensions.to_vec();
    sliced[0] = row_count;
    Shape::new(sliced).map_err(|error| row_slice_error("table variable shape", error))
}

fn empty_scalar_row_slice<T>(
    shape: Shape,
    wrap: impl FnOnce(DenseArray<T>) -> ArrayData,
    context: &str,
) -> Result<Value, EngineError> {
    DenseArray::from_vec(shape, Vec::new())
        .map(wrap)
        .map(Value::Array)
        .map_err(|error| row_slice_error(context, error))
}

fn slice_dense_rows<T: Clone>(
    array: &DenseArray<T>,
    row_start: u64,
    row_count: u64,
    context: &str,
) -> Result<DenseArray<T>, EngineError> {
    let dimensions = array.shape().dimensions();
    if row_count == dimensions[0] && (row_count == 0 || row_start == 1) {
        return Ok(array.clone());
    }
    let shape = row_slice_shape(dimensions, row_count)?;
    let values = slice_column_major_rows(array.as_slice(), dimensions, row_start, row_count)?;
    DenseArray::from_vec(shape, values).map_err(|error| row_slice_error(context, error))
}

fn slice_column_major_rows<T: Clone>(
    source: &[T],
    dimensions: &[u64],
    row_start: u64,
    row_count: u64,
) -> Result<Vec<T>, EngineError> {
    validate_row_slice(row_start, row_count, dimensions)?;
    let source_count = checked_product(dimensions)
        .ok_or_else(|| preview_limit("table variable shape product overflows u64"))?;
    let source_length = u64::try_from(source.len())
        .map_err(|_| preview_limit("table variable storage length does not fit u64"))?;
    if source_count != source_length {
        return Err(invalid_preview(
            "table variable shape disagrees with its column-major storage",
        ));
    }
    let trailing = checked_product(&dimensions[1..])
        .ok_or_else(|| preview_limit("table variable trailing shape product overflows u64"))?;
    let output_count = row_count
        .checked_mul(trailing)
        .ok_or_else(|| preview_limit("table variable row slice size overflows u64"))?;
    let capacity = usize::try_from(output_count)
        .map_err(|_| preview_limit("table variable row slice does not fit this host"))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| preview_limit("table variable row slice allocation failed"))?;
    if row_count == 0 {
        return Ok(values);
    }
    let source_rows = usize::try_from(dimensions[0])
        .map_err(|_| preview_limit("table variable row count does not fit this host"))?;
    let start = usize::try_from(row_start - 1)
        .map_err(|_| preview_limit("table variable row offset does not fit this host"))?;
    let count = usize::try_from(row_count)
        .map_err(|_| preview_limit("table variable row count does not fit this host"))?;
    let blocks = usize::try_from(trailing)
        .map_err(|_| preview_limit("table variable trailing extent does not fit this host"))?;
    for block in 0..blocks {
        let block_start = block
            .checked_mul(source_rows)
            .and_then(|offset| offset.checked_add(start))
            .ok_or_else(|| preview_limit("table variable row offset overflows this host"))?;
        let block_end = block_start
            .checked_add(count)
            .ok_or_else(|| preview_limit("table variable row end overflows this host"))?;
        let slice = source
            .get(block_start..block_end)
            .ok_or_else(|| invalid_preview("table variable row slice escaped storage"))?;
        values.extend_from_slice(slice);
    }
    Ok(values)
}

fn slice_integer_rows(
    array: &IntegerArrayData,
    row_start: u64,
    row_count: u64,
) -> Result<IntegerArrayData, EngineError> {
    macro_rules! sliced {
        ($variant:ident, $array:expr) => {
            slice_dense_rows(
                $array,
                row_start,
                row_count,
                "integer table variable row slice",
            )
            .map(IntegerArrayData::$variant)
        };
    }
    match array {
        IntegerArrayData::I8(array) => sliced!(I8, array),
        IntegerArrayData::ComplexI8(array) => sliced!(ComplexI8, array),
        IntegerArrayData::U8(array) => sliced!(U8, array),
        IntegerArrayData::ComplexU8(array) => sliced!(ComplexU8, array),
        IntegerArrayData::I16(array) => sliced!(I16, array),
        IntegerArrayData::ComplexI16(array) => sliced!(ComplexI16, array),
        IntegerArrayData::U16(array) => sliced!(U16, array),
        IntegerArrayData::ComplexU16(array) => sliced!(ComplexU16, array),
        IntegerArrayData::I32(array) => sliced!(I32, array),
        IntegerArrayData::ComplexI32(array) => sliced!(ComplexI32, array),
        IntegerArrayData::U32(array) => sliced!(U32, array),
        IntegerArrayData::ComplexU32(array) => sliced!(ComplexU32, array),
        IntegerArrayData::I64(array) => sliced!(I64, array),
        IntegerArrayData::ComplexI64(array) => sliced!(ComplexI64, array),
        IntegerArrayData::U64(array) => sliced!(U64, array),
        IntegerArrayData::ComplexU64(array) => sliced!(ComplexU64, array),
    }
}

fn row_slice_error(context: &str, error: impl std::fmt::Display) -> EngineError {
    invalid_preview(format!("{context} failed: {error}"))
}

fn exact_value(
    value: &Value,
    depth: u64,
    limits: &kernel_v2::AggregateLimits,
    usage: &mut kernel_v2::PreviewUsage,
    guard: &mut ActivePathGuard<ValueIdentity>,
) -> Result<kernel_v2::ExactValue, EngineError> {
    let identity = ValueIdentity::of(value);
    enter_active(guard, identity)?;
    let result = exact_value_entered(value, depth, limits, usage, guard);
    finish_active(guard, identity, result)
}

#[allow(clippy::too_many_lines)]
fn exact_value_entered(
    value: &Value,
    depth: u64,
    limits: &kernel_v2::AggregateLimits,
    usage: &mut kernel_v2::PreviewUsage,
    guard: &mut ActivePathGuard<ValueIdentity>,
) -> Result<kernel_v2::ExactValue, EngineError> {
    if matches!(
        value,
        Value::Nothing
            | Value::Sparse(_)
            | Value::Object(_)
            | Value::ObjectArray(_)
            | Value::Graphics(_)
            | Value::GraphicsArray(_)
            | Value::Function(_)
    ) {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            format!(
                "values of kind `{}` have no canonical kernel-v2 exact representation",
                value.kind()
            ),
        ));
    }
    let struct_fields = match value {
        Value::Struct(structure) => Some(aggregate_fields(structure)?),
        _ => None,
    };
    if depth > limits.max_aggregate_depth {
        return Err(EngineError::new(
            "workspace.previewDepth",
            "exact value exceeds the negotiated aggregate depth",
        ));
    }
    let size = canonical_dimensions(value)?;
    let numel = canonical_numel(value, &size)?;
    if numel > limits.max_preview_elements {
        return Err(preview_limit(format!(
            "exact value has {numel} elements; negotiated per-node limit is {}",
            limits.max_preview_elements
        )));
    }
    usage.nodes = checked_usage_add(usage.nodes, 1, "node count")?;
    usage.elements = checked_usage_add(usage.elements, numel, "element count")?;
    usage.depth = usage.depth.max(depth);

    let (class, complex, payload) = match value {
        Value::Logical(value) => (
            "logical".to_owned(),
            false,
            kernel_v2::ExactPayload::Logical {
                logical: vec![*value],
            },
        ),
        Value::Double(value) => (
            "double".to_owned(),
            false,
            kernel_v2::ExactPayload::Numeric {
                real: vec![exact_number(*value)],
                imag: vec!["0".to_owned()],
            },
        ),
        Value::Complex(value) => {
            let complex = !numeric_zero(value.imaginary);
            (
                "double".to_owned(),
                complex,
                kernel_v2::ExactPayload::Numeric {
                    real: vec![exact_number(value.real)],
                    imag: vec![exact_imaginary(value.imaginary)],
                },
            )
        }
        Value::Array(ArrayData::F32(array)) => (
            "single".to_owned(),
            false,
            kernel_v2::ExactPayload::Numeric {
                real: array
                    .as_slice()
                    .iter()
                    .copied()
                    .map(exact_number_f32)
                    .collect(),
                imag: vec!["0".to_owned(); array.as_slice().len()],
            },
        ),
        Value::Array(ArrayData::ComplexF32(array)) => {
            if !array
                .as_slice()
                .iter()
                .any(|value| !numeric_zero_f32(value.im))
            {
                return Err(EngineError::new(
                    "workspace.unsupportedValue",
                    "complex single storage with only zero imaginary components has no exact kernel-v2 representation",
                ));
            }
            (
                "single".to_owned(),
                true,
                kernel_v2::ExactPayload::Numeric {
                    real: array
                        .as_slice()
                        .iter()
                        .map(|value| exact_number_f32(value.re))
                        .collect(),
                    imag: array
                        .as_slice()
                        .iter()
                        .map(|value| exact_imaginary_f32(value.im))
                        .collect(),
                },
            )
        }
        Value::String(strings) => {
            let mut string_code_units = Vec::with_capacity(
                usize::try_from(numel)
                    .map_err(|_| preview_limit("string node does not fit this host"))?,
            );
            let mut missing = Vec::with_capacity(string_code_units.capacity());
            for offset in 0..numel {
                let offset = usize::try_from(offset)
                    .map_err(|_| preview_limit("string offset does not fit this host"))?;
                let element = strings.element(offset).ok_or_else(preview_offset_error)?;
                let code_units = u64::try_from(element.code_unit_len())
                    .map_err(|_| preview_limit("string element length does not fit u64"))?;
                if code_units > limits.max_string_element_code_units {
                    return Err(preview_limit(format!(
                        "string element exceeds negotiated limit {}",
                        limits.max_string_element_code_units
                    )));
                }
                usage.code_units =
                    checked_usage_add(usage.code_units, code_units, "code-unit count")?;
                string_code_units.push(element.code_units().to_vec());
                missing.push(element.is_missing());
            }
            (
                "string".to_owned(),
                false,
                kernel_v2::ExactPayload::String {
                    string_code_units,
                    missing,
                },
            )
        }
        Value::Array(ArrayData::F64(array)) => (
            "double".to_owned(),
            false,
            kernel_v2::ExactPayload::Numeric {
                real: array.as_slice().iter().copied().map(exact_number).collect(),
                imag: vec!["0".to_owned(); array.as_slice().len()],
            },
        ),
        Value::Array(ArrayData::ComplexF64(array)) => {
            let complex = array.as_slice().iter().any(|value| !numeric_zero(value.im));
            (
                "double".to_owned(),
                complex,
                kernel_v2::ExactPayload::Numeric {
                    real: array
                        .as_slice()
                        .iter()
                        .map(|value| exact_number(value.re))
                        .collect(),
                    imag: array
                        .as_slice()
                        .iter()
                        .map(|value| exact_imaginary(value.im))
                        .collect(),
                },
            )
        }
        Value::Array(ArrayData::Logical(array)) => (
            "logical".to_owned(),
            false,
            kernel_v2::ExactPayload::Logical {
                logical: array.as_slice().iter().map(|value| value.get()).collect(),
            },
        ),
        Value::Array(ArrayData::Char(array)) => {
            usage.code_units = checked_usage_add(usage.code_units, numel, "code-unit count")?;
            (
                "char".to_owned(),
                false,
                kernel_v2::ExactPayload::Char {
                    code_units: array.as_slice().iter().map(|value| value.get()).collect(),
                },
            )
        }
        Value::Array(ArrayData::Integer(integer)) => {
            let mut values = Vec::with_capacity(
                usize::try_from(numel)
                    .map_err(|_| preview_limit("integer node does not fit this host"))?,
            );
            let mut complex = false;
            for element in integer.elements() {
                let real = integer_component_decimal(element.real_component());
                let imaginary = element
                    .imaginary_component()
                    .map_or_else(|| "0".to_owned(), integer_component_decimal);
                complex |= imaginary != "0";
                values.push(kernel_v2::IntegerValue { real, imaginary });
            }
            (
                integer.class_name().to_owned(),
                complex,
                kernel_v2::ExactPayload::Integer { integer: values },
            )
        }
        Value::Cell(cell) => {
            let mut items = Vec::with_capacity(
                usize::try_from(numel)
                    .map_err(|_| preview_limit("cell node does not fit this host"))?,
            );
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| preview_limit("exact value depth overflows u64"))?;
            for child in cell.values() {
                items.push(exact_value(child, child_depth, limits, usage, guard)?);
            }
            (
                "cell".to_owned(),
                false,
                kernel_v2::ExactPayload::Cell { items },
            )
        }
        Value::Struct(structure) => {
            let fields = struct_fields
                .expect("struct fields are validated before the depth and shape checks");
            let code_units = field_code_units(&fields)?;
            usage.code_units = checked_usage_add(usage.code_units, code_units, "code-unit count")?;
            let mut records = Vec::with_capacity(
                usize::try_from(numel)
                    .map_err(|_| preview_limit("struct node does not fit this host"))?,
            );
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| preview_limit("exact value depth overflows u64"))?;
            for offset in 0..numel {
                let offset = usize::try_from(offset)
                    .map_err(|_| preview_limit("struct offset does not fit this host"))?;
                let mut entries = Vec::with_capacity(fields.len());
                for (field_index, field) in fields.iter().enumerate() {
                    let child = structure
                        .value_at(field_index, offset)
                        .ok_or_else(|| invalid_preview("struct field storage is incomplete"))?;
                    entries.push((
                        field.clone(),
                        exact_value(child, child_depth, limits, usage, guard)?,
                    ));
                }
                records.push(kernel_v2::StructRecord::new(entries));
            }
            (
                "struct".to_owned(),
                false,
                kernel_v2::ExactPayload::Struct { fields, records },
            )
        }
        Value::Table(table) => {
            let variable_names = table
                .variable_names()
                .iter()
                .map(|name| name.as_str().to_owned())
                .collect::<Vec<_>>();
            let code_units = field_code_units(&variable_names)?;
            usage.code_units = checked_usage_add(
                usage.code_units,
                code_units,
                "table variable-name code-unit count",
            )?;
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| preview_limit("exact value depth overflows u64"))?;
            let mut variables = Vec::with_capacity(table.variable_count());
            for index in 0..table.variable_count() {
                let variable = table
                    .variable(index)
                    .ok_or_else(|| invalid_preview("table variable storage is incomplete"))?;
                variables.push(exact_value(variable, child_depth, limits, usage, guard)?);
            }
            (
                "table".to_owned(),
                false,
                kernel_v2::ExactPayload::Table {
                    variable_names,
                    variables,
                },
            )
        }
        Value::Nothing
        | Value::Sparse(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => {
            unreachable!("unsupported kinds returned above")
        }
    };
    Ok(kernel_v2::ExactValue {
        class,
        size,
        ndims: u64::try_from(value.dimensions().map_or(0, <[u64]>::len))
            .map_err(|_| preview_limit("exact value rank does not fit u64"))?,
        numel,
        complex,
        payload,
    })
}

fn aggregate_selection_offset(
    position: u64,
    request: &kernel_v2::InspectRequest,
    dimensions: &[u64],
) -> Result<usize, EngineError> {
    let offset = selection_offset(
        position,
        &request.range.start,
        &request.range.size,
        dimensions,
    )
    .ok_or_else(selection_offset_error)?;
    usize::try_from(offset).map_err(|_| preview_limit("aggregate offset does not fit this host"))
}

fn canonical_dimensions(value: &Value) -> Result<Vec<u64>, EngineError> {
    let dimensions = value
        .dimensions()
        .map(<[u64]>::to_vec)
        .ok_or_else(|| invalid_preview("exact value has no canonical dimensions"))?;
    for dimension in &dimensions {
        ensure_safe_wire_integer(*dimension, "exact value dimension")?;
    }
    Ok(dimensions)
}

fn canonical_numel(value: &Value, dimensions: &[u64]) -> Result<u64, EngineError> {
    let product = checked_product(dimensions)
        .ok_or_else(|| preview_limit("exact value shape product overflows u64"))?;
    ensure_safe_wire_integer(product, "exact value element count")?;
    if value.numel() != Some(product) {
        return Err(invalid_preview(
            "exact value shape product disagrees with stored element count",
        ));
    }
    Ok(product)
}

fn aggregate_fields(structure: &openmat_value::StructArray) -> Result<Vec<String>, EngineError> {
    let mut fields = Vec::with_capacity(structure.field_count());
    let mut unique = BTreeSet::new();
    for field in structure.field_names() {
        let field = field.as_str();
        if !valid_exact_field_name(field) {
            return Err(EngineError::new(
                "workspace.unsupportedValue",
                "struct field name is outside the kernel-v2 ASCII identifier subset",
            ));
        }
        if !unique.insert(field) {
            return Err(invalid_preview("struct field schema contains a duplicate"));
        }
        fields.push(field.to_owned());
    }
    Ok(fields)
}

fn valid_exact_field_name(field: &str) -> bool {
    let mut bytes = field.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn field_code_units(fields: &[String]) -> Result<u64, EngineError> {
    fields.iter().try_fold(0_u64, |total, field| {
        let units = u64::try_from(field.encode_utf16().count())
            .map_err(|_| preview_limit("field name length does not fit u64"))?;
        checked_usage_add(total, units, "field-name code-unit count")
    })
}

fn commit_candidate_usage(
    committed: &mut kernel_v2::PreviewUsage,
    candidate: kernel_v2::PreviewUsage,
    limits: &kernel_v2::AggregateLimits,
) -> Result<bool, EngineError> {
    let nodes = checked_usage_add(committed.nodes, candidate.nodes, "node count")?;
    let elements = checked_usage_add(committed.elements, candidate.elements, "element count")?;
    let code_units = checked_usage_add(
        committed.code_units,
        candidate.code_units,
        "code-unit count",
    )?;
    if nodes > limits.max_aggregate_nodes
        || elements > limits.max_aggregate_elements
        || code_units > limits.max_preview_code_units
    {
        return Ok(false);
    }
    committed.nodes = nodes;
    committed.elements = elements;
    committed.code_units = code_units;
    committed.depth = committed.depth.max(candidate.depth);
    Ok(true)
}

fn checked_usage_add(left: u64, right: u64, label: &str) -> Result<u64, EngineError> {
    let value = left
        .checked_add(right)
        .ok_or_else(|| preview_limit(format!("aggregate {label} overflows u64")))?;
    ensure_safe_wire_integer(value, label)?;
    Ok(value)
}

fn ensure_safe_wire_integer(value: u64, label: &str) -> Result<(), EngineError> {
    if value > kernel_v1::MAX_SAFE_JSON_INTEGER {
        Err(preview_limit(format!(
            "{label} exceeds the maximum safe JSON integer"
        )))
    } else {
        Ok(())
    }
}

fn preview_truncation(selected: u64, returned: u64) -> Result<PreviewTruncation, EngineError> {
    let omitted_elements = selected
        .checked_sub(returned)
        .ok_or_else(|| invalid_preview("returned aggregate prefix exceeds selected count"))?;
    Ok(PreviewTruncation {
        truncated: omitted_elements != 0,
        omitted_elements,
    })
}

fn enter_active(
    guard: &mut ActivePathGuard<ValueIdentity>,
    identity: ValueIdentity,
) -> Result<(), EngineError> {
    guard
        .enter(identity, 0, u64::MAX)
        .map_err(|error| EngineError::new(error.category(), error.to_string()))
}

fn finish_active<T>(
    guard: &mut ActivePathGuard<ValueIdentity>,
    identity: ValueIdentity,
    result: Result<T, EngineError>,
) -> Result<T, EngineError> {
    let leave = guard
        .leave(&identity)
        .map_err(|error| EngineError::new(error.category(), error.to_string()));
    match result {
        Err(error) => Err(error),
        Ok(value) => {
            leave?;
            Ok(value)
        }
    }
}

fn exact_number(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value == f64::INFINITY {
        "+Inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-Inf".to_owned()
    } else {
        value.to_string()
    }
}

fn exact_number_f32(value: f32) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value == f32::INFINITY {
        "+Inf".to_owned()
    } else if value == f32::NEG_INFINITY {
        "-Inf".to_owned()
    } else {
        value.to_string()
    }
}

fn numeric_zero(value: f64) -> bool {
    value == 0.0
}

fn numeric_zero_f32(value: f32) -> bool {
    value == 0.0
}

fn exact_imaginary(value: f64) -> String {
    if numeric_zero(value) {
        "0".to_owned()
    } else {
        exact_number(value)
    }
}

fn exact_imaginary_f32(value: f32) -> String {
    if numeric_zero_f32(value) {
        "0".to_owned()
    } else {
        exact_number_f32(value)
    }
}

fn preview_limit(message: impl Into<String>) -> EngineError {
    EngineError::new("workspace.previewLimit", message)
}

fn invalid_preview(message: impl Into<String>) -> EngineError {
    EngineError::new("engine.invalidPreview", message)
}

fn validate_selection(start: &[u64], size: &[u64], dimensions: &[u64]) -> Result<(), EngineError> {
    if start.len() != dimensions.len() || size.len() != dimensions.len() {
        return Err(EngineError::new(
            "workspace.invalidRange",
            "matrix range rank must match the value rank",
        ));
    }
    for (dimension, ((start, size), extent)) in start.iter().zip(size).zip(dimensions).enumerate() {
        if *start == 0 {
            return Err(EngineError::new(
                "workspace.invalidRange",
                format!("dimension {dimension} starts at zero; indices are one-based"),
            ));
        }
        if *size == 0 {
            continue;
        }
        let end = start
            .checked_add(size - 1)
            .ok_or_else(|| EngineError::new("workspace.invalidRange", "matrix range overflows"))?;
        if end > *extent {
            return Err(EngineError::new(
                "workspace.invalidRange",
                format!("dimension {dimension} selection exceeds extent {extent}"),
            ));
        }
    }
    Ok(())
}

fn checked_product(values: &[u64]) -> Option<u64> {
    values
        .iter()
        .try_fold(1_u64, |product, value| product.checked_mul(*value))
}

fn selection_offset(position: u64, start: &[u64], size: &[u64], dimensions: &[u64]) -> Option<u64> {
    let mut remaining = position;
    let mut offset = 0_u64;
    let mut stride = 1_u64;
    for ((start, size), extent) in start.iter().zip(size).zip(dimensions) {
        if *size == 0 {
            return None;
        }
        let coordinate = remaining % size;
        remaining /= size;
        let index = start.checked_sub(1)?.checked_add(coordinate)?;
        if index >= *extent {
            return None;
        }
        offset = offset.checked_add(index.checked_mul(stride)?)?;
        stride = stride.checked_mul(*extent)?;
    }
    (remaining == 0).then_some(offset)
}

fn selection_offset_error() -> EngineError {
    EngineError::new(
        "workspace.invalidRange",
        "selected matrix offset overflowed",
    )
}

fn preview_value_v0(value: &Value, offset: u64) -> Result<PreviewValue, EngineError> {
    let offset = usize::try_from(offset).map_err(|_| {
        EngineError::new(
            "workspace.previewLimit",
            "matrix offset does not fit this host",
        )
    })?;
    match value {
        Value::Logical(value) if offset == 0 => Ok(PreviewValue::Logical { value: *value }),
        Value::Double(value) if offset == 0 => Ok(real_preview(*value)),
        Value::Complex(value) if offset == 0 => Ok(complex_preview(value.real, value.imaginary)),
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .get(offset)
            .copied()
            .map(|value| real_preview(f64::from(value)))
            .ok_or_else(preview_offset_error),
        Value::Array(ArrayData::ComplexF32(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| complex_preview(f64::from(value.re), f64::from(value.im)))
            .ok_or_else(preview_offset_error),
        Value::String(value) => value
            .element(offset)
            .ok_or_else(preview_offset_error)
            .and_then(string_text_v0)
            .map(|value| PreviewValue::Text { value }),
        Value::Sparse(array) => sparse_preview_v0(array, offset),
        Value::Object(_) | Value::Graphics(_) if offset == 0 => Ok(PreviewValue::Missing),
        Value::ObjectArray(array) if offset < array.as_slice().len() => Ok(PreviewValue::Missing),
        Value::GraphicsArray(array) if offset < array.as_slice().len() => Ok(PreviewValue::Missing),
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .get(offset)
            .copied()
            .map(real_preview)
            .ok_or_else(preview_offset_error),
        Value::Array(ArrayData::ComplexF64(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| complex_preview(value.re, value.im))
            .ok_or_else(preview_offset_error),
        Value::Array(ArrayData::Logical(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| PreviewValue::Logical { value: value.get() })
            .ok_or_else(preview_offset_error),
        Value::Array(ArrayData::Char(_) | ArrayData::Integer(_)) => Err(EngineError::new(
            "workspace.unsupportedValue",
            "char and integer values cannot be represented losslessly in kernel-v0",
        )),
        Value::Cell(_) | Value::Struct(_) | Value::Table(_) => Err(EngineError::new(
            "workspace.unsupportedValue",
            "cell, struct, and table values cannot be represented losslessly in kernel-v0",
        )),
        Value::Nothing
        | Value::Logical(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => Err(preview_offset_error()),
    }
}

fn string_text_v0(value: &StringElement) -> Result<String, EngineError> {
    if value.is_missing() {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            "missing string elements cannot be represented in kernel-v0",
        ));
    }
    let text = String::from_utf16(value.code_units()).map_err(|_| {
        EngineError::new(
            "workspace.unsupportedValue",
            "string element contains UTF-16 that is not Unicode scalar text",
        )
    })?;
    if text.len() > PREVIEW_MAX_TEXT_BYTES {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            format!(
                "string element exceeds the kernel-v0 exact text limit of {PREVIEW_MAX_TEXT_BYTES} bytes"
            ),
        ));
    }
    Ok(text)
}

fn validate_selected_complex_floats(
    value: &Value,
    request: &kernel_v1::InspectRequest,
    dimensions: &[u64],
    selected_count: u64,
) -> Result<(), EngineError> {
    if !matches!(
        value,
        Value::Complex(_) | Value::Array(ArrayData::ComplexF32(_) | ArrayData::ComplexF64(_))
    ) {
        return Ok(());
    }
    for position in 0..selected_count {
        let offset = selection_offset(
            position,
            &request.range.start,
            &request.range.size,
            dimensions,
        )
        .ok_or_else(selection_offset_error)?;
        let offset = usize::try_from(offset).map_err(|_| {
            EngineError::new(
                "workspace.previewLimit",
                "matrix offset does not fit this host",
            )
        })?;
        let (real, imaginary) = match value {
            Value::Complex(value) if offset == 0 => (value.real, value.imaginary),
            Value::Array(ArrayData::ComplexF32(array)) => array
                .as_slice()
                .get(offset)
                .map(|value| (f64::from(value.re), f64::from(value.im)))
                .ok_or_else(preview_offset_error)?,
            Value::Array(ArrayData::ComplexF64(array)) => array
                .as_slice()
                .get(offset)
                .map(|value| (value.re, value.im))
                .ok_or_else(preview_offset_error)?,
            _ => return Err(preview_offset_error()),
        };
        if !real.is_finite() || !imaginary.is_finite() {
            return Err(EngineError::new(
                "workspace.unsupportedValue",
                "non-finite complex floating-point components have no lossless kernel-v1 preview",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn preview_value_v1(
    value: &Value,
    offset: u64,
    limits: &kernel_v1::PreviewLimits,
    aggregate_code_units: &mut u64,
) -> Result<Option<kernel_v1::PreviewValue>, EngineError> {
    let offset = usize::try_from(offset).map_err(|_| {
        EngineError::new(
            "workspace.previewLimit",
            "matrix offset does not fit this host",
        )
    })?;
    let preview = match value {
        Value::Logical(value) if offset == 0 => kernel_v1::PreviewValue::Logical { value: *value },
        Value::Double(value) if offset == 0 => real_preview_v1(*value),
        Value::Complex(value) if offset == 0 => complex_preview_v1(value.real, value.imaginary)?,
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .get(offset)
            .copied()
            .map(|value| real_preview_v1(f64::from(value)))
            .ok_or_else(preview_offset_error)?,
        Value::Array(ArrayData::ComplexF32(array)) => {
            let value = array
                .as_slice()
                .get(offset)
                .ok_or_else(preview_offset_error)?;
            complex_preview_v1(f64::from(value.re), f64::from(value.im))?
        }
        Value::String(value) => {
            let element = value.element(offset).ok_or_else(preview_offset_error)?;
            let element_code_units = u64::try_from(element.code_unit_len()).map_err(|_| {
                EngineError::new(
                    "workspace.unsupportedValue",
                    "string element code-unit count does not fit u64",
                )
            })?;
            if element_code_units > limits.max_string_element_code_units {
                return Err(EngineError::new(
                    "workspace.unsupportedValue",
                    format!(
                        "string element exceeds negotiated limit {}",
                        limits.max_string_element_code_units
                    ),
                ));
            }
            let next_aggregate = aggregate_code_units
                .checked_add(element_code_units)
                .ok_or_else(|| {
                    EngineError::new(
                        "workspace.unsupportedValue",
                        "aggregate string code-unit count overflows u64",
                    )
                })?;
            if next_aggregate > limits.max_preview_code_units {
                return Ok(None);
            }
            *aggregate_code_units = next_aggregate;
            kernel_v1::PreviewValue::String {
                code_units: element.code_units().to_vec(),
                missing: element.is_missing(),
            }
        }
        Value::Sparse(array) => sparse_preview_v1(array, offset)?,
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .get(offset)
            .copied()
            .map(real_preview_v1)
            .ok_or_else(preview_offset_error)?,
        Value::Array(ArrayData::ComplexF64(array)) => {
            let value = array
                .as_slice()
                .get(offset)
                .ok_or_else(preview_offset_error)?;
            complex_preview_v1(value.re, value.im)?
        }
        Value::Array(ArrayData::Logical(array)) => {
            let value = array
                .as_slice()
                .get(offset)
                .ok_or_else(preview_offset_error)?;
            kernel_v1::PreviewValue::Logical { value: value.get() }
        }
        Value::Array(ArrayData::Char(array)) => {
            let value = array
                .as_slice()
                .get(offset)
                .ok_or_else(preview_offset_error)?;
            kernel_v1::PreviewValue::CharCodeUnit { value: value.get() }
        }
        Value::Array(ArrayData::Integer(integer)) => {
            let value = integer.element(offset).ok_or_else(preview_offset_error)?;
            kernel_v1::PreviewValue::Integer {
                real: integer_component_decimal(value.real_component()),
                imaginary: value
                    .imaginary_component()
                    .map_or_else(|| "0".to_owned(), integer_component_decimal),
            }
        }
        Value::Nothing
        | Value::Cell(_)
        | Value::Struct(_)
        | Value::Table(_)
        | Value::Logical(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => {
            return Err(EngineError::new(
                "workspace.unsupportedValue",
                "value has no canonical kernel-v1 preview at the selected offset",
            ));
        }
    };
    Ok(Some(preview))
}

fn sparse_preview_v0(array: &SparseArrayData, offset: usize) -> Result<PreviewValue, EngineError> {
    let offset = u64::try_from(offset).map_err(|_| preview_offset_error())?;
    match array
        .element_at_offset(offset)
        .map_err(|_| preview_offset_error())?
    {
        SparseScalarValue::Logical(value) => Ok(PreviewValue::Logical { value }),
        SparseScalarValue::F64(value) => Ok(real_preview(value)),
        SparseScalarValue::ComplexF64(value) => Ok(complex_preview(value.re, value.im)),
    }
}

fn sparse_preview_v1(
    array: &SparseArrayData,
    offset: usize,
) -> Result<kernel_v1::PreviewValue, EngineError> {
    let offset = u64::try_from(offset).map_err(|_| preview_offset_error())?;
    match array
        .element_at_offset(offset)
        .map_err(|_| preview_offset_error())?
    {
        SparseScalarValue::Logical(value) => Ok(kernel_v1::PreviewValue::Logical { value }),
        SparseScalarValue::F64(value) => Ok(real_preview_v1(value)),
        SparseScalarValue::ComplexF64(value) => complex_preview_v1(value.re, value.im),
    }
}

fn integer_component_decimal(component: IntegerComponent) -> String {
    component.canonical_decimal()
}

fn real_preview_v1(value: f64) -> kernel_v1::PreviewValue {
    match real_preview(value) {
        PreviewValue::Number { value } => kernel_v1::PreviewValue::Number { value },
        PreviewValue::Special { value } => kernel_v1::PreviewValue::Special { value },
        _ => unreachable!("real_preview returns only number or special"),
    }
}

fn complex_preview_v1(real: f64, imaginary: f64) -> Result<kernel_v1::PreviewValue, EngineError> {
    if !real.is_finite() || !imaginary.is_finite() {
        return Err(EngineError::new(
            "workspace.unsupportedValue",
            "non-finite complex floating-point components have no lossless kernel-v1 preview",
        ));
    }
    Ok(kernel_v1::PreviewValue::Complex { real, imaginary })
}

fn preview_offset_error() -> EngineError {
    EngineError::new(
        "workspace.invalidRange",
        "selected matrix offset is outside the stored value",
    )
}

fn real_preview(value: f64) -> PreviewValue {
    if value.is_nan() {
        PreviewValue::Special {
            value: "nan".to_owned(),
        }
    } else if value == f64::INFINITY {
        PreviewValue::Special {
            value: "infinity".to_owned(),
        }
    } else if value == f64::NEG_INFINITY {
        PreviewValue::Special {
            value: "negativeInfinity".to_owned(),
        }
    } else {
        PreviewValue::Number { value }
    }
}

fn complex_preview(real: f64, imaginary: f64) -> PreviewValue {
    if real.is_finite() && imaginary.is_finite() {
        PreviewValue::Complex { real, imaginary }
    } else {
        // Kernel v0 has no lossless representation for a complex value with a
        // non-finite component. Missing is stable and passes protocol validation.
        PreviewValue::Missing
    }
}

#[cfg(test)]
mod tests {
    include!("argument_blocks_tests.rs");
    use std::fs;
    use std::path::{Path, PathBuf};
    #[cfg(windows)]
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use openmat_array::{
        CharCodeUnit, Complex32, Complex64, ComplexInteger, DenseArray, IntegerArrayData,
        IntegerElement, Logical, Shape,
    };
    use openmat_protocol::{
        EventEnvelope, InitializeRequest, InterruptRequest, ListWorkspaceRequest, MatrixRange,
        PROTOCOL_V0, Request, RequestEnvelope, ResponseEnvelope, ResponseResult, ServerMessage,
        ShutdownRequest,
    };
    use openmat_value::{
        BuiltinHandle, CellArray, FieldName, FunctionHandle, GraphicsClass, GraphicsHandle,
        ObjectHandle, StringArray, StringValue, StructArray, TableArray, TableVariableName,
    };

    use super::*;
    use crate::{Kernel, KernelStatus};

    static NEXT_FILE_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_FILE_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "openmat-kernel-runtime-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("temporary source directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn execute(engine: &mut RuntimeEngine, code: &str) -> Result<ExecutionOutput, EngineError> {
        engine.execute(
            &ExecuteRequest {
                code: code.to_owned(),
                source_name: "test.m".to_owned(),
                mode: ExecutionMode::Repl,
            },
            &CancellationToken::new(),
        )
    }

    #[cfg(windows)]
    #[test]
    fn ctypes_style_ffi_calls_windows_dll_and_dereferences_struct_pointer() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "lib = ffi.Library(\"kernel32.dll\");\n\
             lib.GetCurrentProcessId.argtypes = {};\n\
             lib.GetCurrentProcessId.restype = ffi.u32;\n\
             lib.GetCurrentProcessId.abi = ffi.win64;\n\
             pid = lib.GetCurrentProcessId();\n\
             SYSTEMTIME = ffi.Structure(\"SYSTEMTIME\", {\"year\", ffi.u16; \"month\", ffi.u16; \"dayOfWeek\", ffi.u16; \"day\", ffi.u16; \"hour\", ffi.u16; \"minute\", ffi.u16; \"second\", ffi.u16; \"milliseconds\", ffi.u16});\n\
             now = SYSTEMTIME();\n\
             lib.GetSystemTime.argtypes = {ffi.Ptr(SYSTEMTIME)};\n\
             lib.GetSystemTime.restype = ffi.void;\n\
             lib.GetSystemTime.abi = ffi.win64;\n\
             lib.GetSystemTime(now);\n\
             pointer = ffi.cast(ffi.addressof(now), ffi.Ptr(SYSTEMTIME));\n\
             year = pointer.contents.year;\n\
             pointer.contents.month = pointer.contents.month;\n\
             GETPID = ffi.FunctionType(ffi.u32, {}, ffi.win64);\n\
             getPid = ffi.cast(lib.GetCurrentProcessId.address, GETPID);\n\
             pid2 = getPid();\n",
        )
        .expect("M source should call a real Windows DLL through FFI");
        execute(&mut engine, "pid3 = lib.GetCurrentProcessId();\n")
            .expect("library symbol signatures should survive a REPL command boundary");

        let Value::Array(ArrayData::Integer(IntegerArrayData::U32(pid))) =
            engine.interpreter.workspace().get("pid").unwrap()
        else {
            panic!("GetCurrentProcessId must return an exact uint32 scalar")
        };
        assert_eq!(pid.as_slice(), &[std::process::id()]);

        let Value::Array(ArrayData::Integer(IntegerArrayData::U32(pid2))) =
            engine.interpreter.workspace().get("pid2").unwrap()
        else {
            panic!("function-pointer call must return an exact uint32 scalar")
        };
        assert_eq!(pid2.as_slice(), &[std::process::id()]);

        let Value::Array(ArrayData::Integer(IntegerArrayData::U32(pid3))) =
            engine.interpreter.workspace().get("pid3").unwrap()
        else {
            panic!("cached library function signature must survive collection")
        };
        assert_eq!(pid3.as_slice(), &[std::process::id()]);

        let Value::Array(ArrayData::Integer(IntegerArrayData::U16(year))) =
            engine.interpreter.workspace().get("year").unwrap()
        else {
            panic!("SYSTEMTIME.year must be read as an exact uint16 scalar")
        };
        assert!((2020..=2200).contains(&year.as_slice()[0]));
    }

    #[cfg(windows)]
    #[test]
    fn ffi_calls_a_c_fixture_with_struct_values_and_function_pointers() {
        if Command::new("gcc").arg("--version").output().is_err() {
            eprintln!("skipping real C FFI fixture because gcc is unavailable");
            return;
        }
        let directory = TestDirectory::new("ffi-dll");
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("kernel crate lives beneath the repository root");
        let library = directory.path().join("basic_ffi.dll");
        let status = Command::new("gcc")
            .arg("-shared")
            .arg("-std=c11")
            .arg("-O2")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg(repository.join("crates/openmat-ffi/tests/fixtures/basic_ffi.c"))
            .arg("-o")
            .arg(&library)
            .status()
            .expect("run gcc for FFI fixture");
        assert!(status.success(), "C FFI fixture must compile cleanly");

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        let path = library.to_string_lossy().replace('\\', "/");
        execute(
            &mut engine,
            &format!(
                "lib = ffi.Library(\"{path}\");\n\
                 POINT = ffi.Structure(\"POINT\", {{\"x\", ffi.i32; \"y\", ffi.i32}});\n\
                 p = POINT(); p.x = 10; p.y = 20;\n\
                 lib.ffi_translate_point.argtypes = {{POINT, ffi.i32, ffi.i32}};\n\
                 lib.ffi_translate_point.restype = POINT;\n\
                 lib.ffi_translate_point.abi = ffi.win64;\n\
                 q = lib.ffi_translate_point(p, 5, -3);\n\
                 qx = q.x; qy = q.y;\n\
                 BINARY = ffi.FunctionType(ffi.i32, {{ffi.i32, ffi.i32}}, ffi.win64);\n\
                 lib.ffi_get_add_i32.argtypes = {{}};\n\
                 lib.ffi_get_add_i32.restype = BINARY;\n\
                 lib.ffi_get_add_i32.abi = ffi.win64;\n\
                 add = lib.ffi_get_add_i32();\n\
                 sum = add(7, 8);\n\
                 lib.ffi_call_binary.argtypes = {{BINARY, ffi.i32, ffi.i32}};\n\
                 lib.ffi_call_binary.restype = ffi.i32;\n\
                 lib.ffi_call_binary.abi = ffi.win64;\n\
                 sum2 = lib.ffi_call_binary(add, 9, 4);\n"
            ),
        )
        .expect("M source should call the compiled C FFI fixture");

        let scalar_i32 = |name: &str| {
            let Value::Array(ArrayData::Integer(IntegerArrayData::I32(value))) =
                engine.interpreter.workspace().get(name).unwrap()
            else {
                panic!("{name} must be an exact int32 scalar")
            };
            value.as_slice()[0]
        };
        assert_eq!(scalar_i32("qx"), 15);
        assert_eq!(scalar_i32("qy"), 17);
        assert_eq!(scalar_i32("sum"), 15);
        assert_eq!(scalar_i32("sum2"), 13);
    }

    #[cfg(windows)]
    #[test]
    fn trusted_oex_plugin_loads_into_source_execution() {
        if Command::new("gcc").arg("--version").output().is_err() {
            eprintln!("skipping real C OEX fixture because gcc is unavailable");
            return;
        }
        let directory = TestDirectory::new("oex-plugin");
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("kernel crate lives beneath the repository root");
        let test_executable = std::env::current_exe().expect("locate kernel test executable");
        let dependency_directory = test_executable
            .parent()
            .expect("kernel test dependency directory");
        let profile_directory = dependency_directory
            .parent()
            .expect("kernel target profile directory");
        let bridge = [
            dependency_directory.join("openmat_oex.dll"),
            profile_directory.join("openmat_oex.dll"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .expect("cargo must build the OEX bridge DLL");
        std::fs::copy(&bridge, directory.path().join("openmat_oex.dll"))
            .expect("copy OEX bridge beside kernel fixture");
        let library = directory.path().join("arithmetic.oex.dll");
        let status = Command::new("gcc")
            .arg("-shared")
            .arg("-std=c11")
            .arg("-O2")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg("-I")
            .arg(repository.join("include"))
            .arg(repository.join("crates/openmat-oex/tests/fixtures/arithmetic_plugin.c"))
            .arg(&bridge)
            .arg("-o")
            .arg(&library)
            .status()
            .expect("run gcc for kernel OEX fixture");
        assert!(
            status.success(),
            "kernel C OEX fixture must compile cleanly"
        );

        // SAFETY: The DLL was compiled immediately above from the checked-in fixture against
        // this checkout's OEX header.
        let mut engine = unsafe {
            RuntimeEngine::with_working_directory_and_oex_plugins(directory.path(), [&library])
        }
        .expect("construct engine with OEX fixture");
        execute(
            &mut engine,
            "result = add2(7, 8); counter = NativeCounter(5); by_name = feval('value', counter); by_handle = feval(@value, counter); object_second = feval('add', 2, counter);",
        )
        .expect("execute OEX function and dynamic native-method calls");
        assert_eq!(
            engine.interpreter.workspace().get("result"),
            Some(&Value::Double(15.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("by_name"),
            Some(&Value::Double(5.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("by_handle"),
            Some(&Value::Double(5.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("object_second"),
            Some(&Value::Double(7.0))
        );
    }

    #[test]
    fn duplicate_canonical_oex_plugin_paths_are_rejected_before_loading() {
        let directory = TestDirectory::new("duplicate-oex-plugin");
        let library = directory.path().join("duplicate.oex.dll");
        std::fs::write(&library, []).expect("placeholder plugin path");

        // SAFETY: Duplicate validation occurs before the placeholder file can be loaded.
        let result = unsafe {
            RuntimeEngine::with_working_directory_and_oex_plugins(
                directory.path(),
                [&library, &library],
            )
        };
        let Err(error) = result else {
            panic!("duplicate canonical plugin paths must fail");
        };
        assert_eq!(error.category(), "oex.load");
        assert!(error.message().contains("supplied more than once"));
    }

    fn scalar_inspect(name: &str) -> InspectRequest {
        InspectRequest {
            name: name.to_owned(),
            range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            max_elements: 1,
        }
    }

    fn row_inspect(name: &str, columns: u64) -> InspectRequest {
        InspectRequest {
            name: name.to_owned(),
            range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, columns],
            },
            max_elements: columns,
        }
    }

    fn v1_row_inspect(columns: u64, max_elements: u64) -> kernel_v1::InspectRequest {
        kernel_v1::InspectRequest {
            name: "value".to_owned(),
            range: kernel_v1::MatrixRange {
                start: vec![1, 1],
                size: vec![1, columns],
            },
            max_elements,
        }
    }

    fn v2_inspect(size: [u64; 2], max_elements: u64) -> kernel_v2::InspectRequest {
        kernel_v2::InspectRequest {
            name: "value".to_owned(),
            range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: size.to_vec(),
            },
            max_elements,
        }
    }

    fn integer_value<T: IntegerElement>(shape: [u64; 2], values: Vec<T>) -> Value {
        Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
            DenseArray::from_vec(Shape::new(shape).unwrap(), values).unwrap(),
        )))
    }

    fn real_single_value(shape: [u64; 2], values: Vec<f32>) -> Value {
        Value::Array(ArrayData::F32(
            DenseArray::from_vec(Shape::new(shape).unwrap(), values).unwrap(),
        ))
    }

    fn complex_single_value(shape: [u64; 2], values: Vec<Complex32>) -> Value {
        Value::Array(ArrayData::ComplexF32(
            DenseArray::from_vec(Shape::new(shape).unwrap(), values).unwrap(),
        ))
    }

    fn char_row(value: &str) -> Value {
        let code_units = value
            .encode_utf16()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        let columns = u64::try_from(code_units.len()).unwrap();
        Value::Array(ArrayData::Char(
            DenseArray::from_vec(Shape::new([1, columns]).unwrap(), code_units).unwrap(),
        ))
    }

    #[test]
    fn language_file_io_roundtrips_through_the_session_working_directory() {
        let directory = TestDirectory::new("language-file-io");
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "source = [1 2; 3 4];\nwritematrix(source, 'roundtrip.csv');\nloaded = readmatrix('roundtrip.csv');\ntext = fileread('roundtrip.csv');\n",
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(directory.path().join("roundtrip.csv")).unwrap(),
            if cfg!(windows) {
                "1,2\r\n3,4\r\n"
            } else {
                "1,2\n3,4\n"
            }
        );
        assert_eq!(
            engine.interpreter.workspace().get("loaded"),
            engine.interpreter.workspace().get("source")
        );
        assert_eq!(
            engine
                .interpreter
                .workspace()
                .get("text")
                .and_then(Value::dimensions),
            Some([1, if cfg!(windows) { 10 } else { 8 }].as_slice())
        );
    }

    #[test]
    fn current_directory_changes_preserve_session_state_and_drive_relative_resolution() {
        let directory = TestDirectory::new("dynamic-current-directory");
        let child = directory.path().join("child");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("payload.txt"), "from child\n").unwrap();
        fs::write(
            child.join("current_folder_helper.m"),
            "function value = current_folder_helper()\nvalue = 42;\nend\n",
        )
        .unwrap();

        let working_directory = WorkingDirectory::new(directory.path()).unwrap();
        let initial = working_directory.snapshot().unwrap();
        let mut engine =
            RuntimeEngine::with_shared_working_directory(working_directory.clone()).unwrap();
        execute(
            &mut engine,
            "preserved = 17;\nprevious = cd('child');\nhere = pwd;\n",
        )
        .unwrap();
        execute(
            &mut engine,
            "text = fileread('payload.txt');\nanswer = current_folder_helper();\n",
        )
        .unwrap();

        let changed = working_directory.snapshot().unwrap();
        assert_eq!(
            changed.path,
            WorkingDirectory::new(&child)
                .unwrap()
                .snapshot()
                .unwrap()
                .path
        );
        assert_eq!(changed.generation, initial.generation + 1);
        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("preserved"), Some(&Value::Double(17.0)));
        assert_eq!(
            workspace.get("previous"),
            Some(&char_row(&initial.path.to_string_lossy()))
        );
        assert_eq!(
            workspace.get("here"),
            Some(&char_row(&changed.path.to_string_lossy()))
        );
        assert_eq!(workspace.get("answer"), Some(&Value::Double(42.0)));

        working_directory.change("..").unwrap();
        execute(
            &mut engine,
            "after_ui_change = pwd;\nstill_preserved = preserved;\n",
        )
        .unwrap();
        let restored = working_directory.snapshot().unwrap();
        assert_eq!(
            engine.interpreter.workspace().get("after_ui_change"),
            Some(&char_row(&restored.path.to_string_lossy()))
        );
        assert_eq!(
            engine.interpreter.workspace().get("still_preserved"),
            Some(&Value::Double(17.0))
        );
    }

    #[test]
    fn dynamic_search_path_drives_direct_feval_handle_eval_and_introspection() {
        let directory = TestDirectory::new("dynamic-search-path");
        let library = directory.path().join("library");
        fs::create_dir(&library).unwrap();
        fs::write(
            library.join("path_target.m"),
            "function value = path_target(input)\nvalue = input + 10;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "before = path;\naddpath('library');\nafter = path;\ndirect = path_target(1);\nby_name = feval('path_target', 2);\nhandle = @path_target;\nby_handle = handle(3);\nby_eval = eval('path_target(4)');\nlocated = which('path_target');\nexists_code = exist('path_target', 'file');\nold_path = rmpath('library');\n",
        )
        .unwrap();

        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("direct"), Some(&Value::Double(11.0)));
        assert_eq!(workspace.get("by_name"), Some(&Value::Double(12.0)));
        assert_eq!(workspace.get("by_handle"), Some(&Value::Double(13.0)));
        assert_eq!(workspace.get("by_eval"), Some(&Value::Double(14.0)));
        assert_eq!(workspace.get("exists_code"), Some(&Value::Double(2.0)));
        assert_eq!(
            workspace.get("located"),
            Some(&char_row(&library.join("path_target.m").to_string_lossy()))
        );
        assert_eq!(
            engine.file_resolver.search_paths().unwrap(),
            Vec::<PathBuf>::new()
        );

        let error = execute(&mut engine, "missing_after_remove = path_target(5);\n")
            .expect_err("a removed path must stop later name resolution");
        assert!(error.message().contains("path_target"));
    }

    #[test]
    fn package_functions_follow_r2022b_qualified_lookup_across_call_forms() {
        let directory = TestDirectory::new("package-functions");
        let package = directory.path().join("+alpha");
        let nested = package.join("+nested");
        fs::create_dir(&package).unwrap();
        fs::create_dir(&nested).unwrap();
        fs::write(
            package.join("twice.m"),
            "function value = twice(input)\nvalue = 2 * input;\nend\n",
        )
        .unwrap();
        fs::write(
            nested.join("inc.m"),
            "function value = inc(input)\nvalue = input + 1;\nend\n",
        )
        .unwrap();
        fs::write(
            package.join("caller.m"),
            "function value = caller()\nvalue = alpha.twice(7);\nend\n",
        )
        .unwrap();
        fs::write(
            package.join("badcaller.m"),
            "function value = badcaller()\nvalue = twice(7);\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "direct = alpha.twice(3);\nnested = alpha.nested.inc(9);\nby_name = feval('alpha.twice', 4);\nhandle = @alpha.twice;\nby_handle = handle(5);\nby_eval = eval('alpha.twice(6)');\nby_caller = alpha.caller();\nlocated = which('alpha.twice');\nexists_code = exist('alpha.twice', 'file');\naddpath('+alpha');\n",
        )
        .unwrap();
        execute(
            &mut engine,
            "alpha = struct('twice', @(x) x + 100); shadowed = alpha.twice(8);\n",
        )
        .unwrap();

        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("direct"), Some(&Value::Double(6.0)));
        assert_eq!(workspace.get("nested"), Some(&Value::Double(10.0)));
        assert_eq!(workspace.get("by_name"), Some(&Value::Double(8.0)));
        assert_eq!(workspace.get("by_handle"), Some(&Value::Double(10.0)));
        assert_eq!(workspace.get("by_eval"), Some(&Value::Double(12.0)));
        assert_eq!(workspace.get("by_caller"), Some(&Value::Double(14.0)));
        assert_eq!(workspace.get("shadowed"), Some(&Value::Double(108.0)));
        assert_eq!(workspace.get("exists_code"), Some(&Value::Double(0.0)));
        assert_eq!(
            workspace.get("located"),
            Some(&char_row(&package.join("twice.m").to_string_lossy()))
        );
        assert!(engine.file_resolver.search_paths().unwrap().is_empty());

        let error = execute(
            &mut engine,
            "clear alpha; unqualified = alpha.badcaller();\n",
        )
        .expect_err("package siblings require a qualified name or import");
        assert!(error.message().contains("twice"));
    }

    #[test]
    fn imports_package_classes_and_bare_zero_input_functions_work_together() {
        let directory = TestDirectory::new("package-imports-and-classes");
        let package = directory.path().join("+alpha");
        let beta = directory.path().join("+beta");
        fs::create_dir(&package).unwrap();
        fs::create_dir(&beta).unwrap();
        fs::write(
            package.join("twice.m"),
            "function value = twice(input)\nvalue = 2 * input;\nend\n",
        )
        .unwrap();
        fs::write(
            beta.join("twice.m"),
            "function value = twice(input)\nvalue = 100 + input;\nend\n",
        )
        .unwrap();
        fs::write(
            package.join("zero.m"),
            "function value = zero()\nvalue = 17;\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("first_alpha.m"),
            "function value = first_alpha()\nimport alpha.*\nimport beta.*\nvalue = twice(1);\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("first_beta.m"),
            "function value = first_beta()\nimport beta.*\nimport alpha.*\nvalue = twice(1);\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("explicit_box.m"),
            "function value = explicit_box(input)\nimport alpha.Box\nobject = Box(input);\nvalue = object.Value;\nend\n",
        )
        .unwrap();
        fs::write(
            package.join("Box.m"),
            "classdef Box\nproperties\nValue\nend\nproperties (Constant)\nFactor = 3\nend\nmethods\nfunction obj = Box(value)\nobj.Value = value;\nend\nend\nmethods (Static)\nfunction result = scale(value)\nresult = Box.Factor * value;\nend\nend\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "before_import = twice(2); import alpha.*; after_import = twice(3); direct_zero = alpha.zero; direct_box = alpha.Box(7); direct_class = class(direct_box); direct_value = direct_box.Value; direct_factor = alpha.Box.Factor; static_value = alpha.Box.scale(4); imported_box = Box(8); imported_value = imported_box.Value; by_name = feval('twice', 5); alpha_first = first_alpha(); beta_first = first_beta(); explicit_value = explicit_box(10);\n",
        )
        .unwrap();
        execute(
            &mut engine,
            "persisted = twice(6); persisted_box = Box(9); persisted_value = persisted_box.Value;\n",
        )
        .unwrap();
        execute(
            &mut engine,
            "alpha = struct('zero', 29); shadowed_zero = alpha.zero;\n",
        )
        .unwrap();

        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("before_import"), Some(&Value::Double(4.0)));
        assert_eq!(workspace.get("after_import"), Some(&Value::Double(6.0)));
        assert_eq!(workspace.get("direct_zero"), Some(&Value::Double(17.0)));
        assert_eq!(workspace.get("direct_value"), Some(&Value::Double(7.0)));
        assert_eq!(workspace.get("direct_factor"), Some(&Value::Double(3.0)));
        assert_eq!(workspace.get("static_value"), Some(&Value::Double(12.0)));
        assert_eq!(workspace.get("imported_value"), Some(&Value::Double(8.0)));
        assert_eq!(workspace.get("by_name"), Some(&Value::Double(10.0)));
        assert_eq!(workspace.get("alpha_first"), Some(&Value::Double(2.0)));
        assert_eq!(workspace.get("beta_first"), Some(&Value::Double(101.0)));
        assert_eq!(workspace.get("explicit_value"), Some(&Value::Double(10.0)));
        assert_eq!(workspace.get("persisted"), Some(&Value::Double(12.0)));
        assert_eq!(workspace.get("persisted_value"), Some(&Value::Double(9.0)));
        assert_eq!(workspace.get("shadowed_zero"), Some(&Value::Double(29.0)));
        assert_eq!(workspace.get("direct_class"), Some(&char_row("alpha.Box")));
    }

    #[test]
    fn clear_import_mutates_only_the_base_import_list() {
        let directory = TestDirectory::new("clear-import");
        let package = directory.path().join("+alpha");
        fs::create_dir(&package).unwrap();
        fs::write(
            package.join("twice.m"),
            "function value = twice(input)\nvalue = 2 * input;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(&mut engine, "import alpha.*; seeded = twice(2);\n").unwrap();
        assert_eq!(engine.interpreter.workspace_imports(), &["alpha.*"]);
        execute(
            &mut engine,
            "if false\nclear import;\nend\nconditional_kept = twice(7);\nconditional_handle = @twice;\nconditional_handle_value = conditional_handle(9);\n",
        )
        .unwrap();
        assert_eq!(
            engine.interpreter.workspace().get("conditional_kept"),
            Some(&Value::Double(14.0))
        );
        assert_eq!(
            engine
                .interpreter
                .workspace()
                .get("conditional_handle_value"),
            Some(&Value::Double(18.0))
        );
        execute(
            &mut engine,
            "if true\nclear import;\nend\nconditional_failed = false;\ntry\ntwice(8);\ncatch\nconditional_failed = true;\nend\ncleared_handle_failed = false;\ncleared_handle = @twice;\ntry\ncleared_handle(9);\ncatch\ncleared_handle_failed = true;\nend\n",
        )
        .unwrap();
        assert_eq!(
            engine.interpreter.workspace().get("conditional_failed"),
            Some(&Value::Logical(true))
        );
        assert_eq!(
            engine.interpreter.workspace().get("cleared_handle_failed"),
            Some(&Value::Logical(true))
        );
        execute(&mut engine, "import alpha.*;\n").unwrap();
        execute(&mut engine, "clear; after_clear = twice(3);\n").unwrap();
        assert_eq!(engine.interpreter.workspace_imports(), &["alpha.*"]);
        assert_eq!(
            engine.interpreter.workspace().get("after_clear"),
            Some(&Value::Double(6.0))
        );

        execute(&mut engine, "clear import;\n").unwrap();
        assert!(engine.interpreter.workspace_imports().is_empty());
        assert_eq!(
            engine.interpreter.workspace().get("after_clear"),
            Some(&Value::Double(6.0)),
            "clear import must not remove ordinary values"
        );
        let error = execute(&mut engine, "missing = twice(4);\n")
            .expect_err("cleared wildcard import must stop unqualified resolution");
        assert!(error.message().contains("twice"));
        let file_error = engine
            .execute(
                &ExecuteRequest {
                    code: "clear import;\n".to_owned(),
                    source_name: "script_with_clear_import.m".to_owned(),
                    mode: ExecutionMode::File,
                },
                &CancellationToken::new(),
            )
            .expect_err("source files cannot clear the command workspace import list");
        assert_eq!(file_error.category(), "compile.clearImportScope");
        execute(
            &mut engine,
            "direct = alpha.twice(5); import alpha.*; restored = twice(6);\n",
        )
        .unwrap();
        assert_eq!(
            engine.interpreter.workspace().get("direct"),
            Some(&Value::Double(10.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("restored"),
            Some(&Value::Double(12.0))
        );
    }

    #[test]
    fn class_folders_link_inline_and_external_classdef_methods() {
        let directory = TestDirectory::new("class-folder");
        let class_directory = directory.path().join("@FolderBox");
        let packaged_class_directory = directory.path().join("+alpha/@PackagedBox");
        fs::create_dir(&class_directory).unwrap();
        fs::create_dir_all(&packaged_class_directory).unwrap();
        fs::write(
            class_directory.join("FolderBox.m"),
            "classdef FolderBox\nproperties (Access = private)\nValue\nend\nmethods\nfunction object = FolderBox(value)\nobject.Value = value;\nend\nresult = bump(object, amount)\nend\nmethods (Static)\nresult = scale(value)\nend\nend\n",
        )
        .unwrap();
        fs::write(
            class_directory.join("bump.m"),
            "function result = bump(object, amount)\nresult = object.Value + amount;\nend\n",
        )
        .unwrap();
        fs::write(
            class_directory.join("scale.m"),
            "function result = scale(value)\nresult = value * 3;\nend\n",
        )
        .unwrap();
        fs::write(
            packaged_class_directory.join("PackagedBox.m"),
            "classdef PackagedBox\nproperties\nValue\nend\nmethods\nfunction object = PackagedBox(value)\nobject.Value = value;\nend\nresult = bump(object, amount)\nend\nend\n",
        )
        .unwrap();
        fs::write(
            packaged_class_directory.join("bump.m"),
            "function result = bump(object, amount)\nresult = object.Value + amount;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "object = FolderBox(3); bumped = object.bump(4); scaled = FolderBox.scale(5); kind = class(object); packaged = alpha.PackagedBox(8); package_bumped = packaged.bump(2); package_kind = class(packaged);\n",
        )
        .expect("@Class classdef and external methods should link and execute");
        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("bumped"), Some(&Value::Double(7.0)));
        assert_eq!(workspace.get("scaled"), Some(&Value::Double(15.0)));
        assert_eq!(workspace.get("kind"), Some(&char_row("FolderBox")));
        assert_eq!(workspace.get("package_bumped"), Some(&Value::Double(10.0)));
        assert_eq!(
            workspace.get("package_kind"),
            Some(&char_row("alpha.PackagedBox"))
        );
    }

    #[test]
    fn class_folder_external_method_signature_mismatch_is_diagnostic() {
        let directory = TestDirectory::new("class-folder-signature");
        let class_directory = directory.path().join("@BrokenBox");
        fs::create_dir(&class_directory).unwrap();
        fs::write(
            class_directory.join("BrokenBox.m"),
            "classdef BrokenBox\nmethods\nfunction object = BrokenBox()\nend\nresult = bump(object, amount)\nend\nend\n",
        )
        .unwrap();
        fs::write(
            class_directory.join("bump.m"),
            "function result = bump(object)\nresult = object;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        let error = execute(&mut engine, "object = BrokenBox();\n")
            .expect_err("external method declaration and body signatures must match");
        assert_eq!(error.category(), "source.externalMethodSignature");
    }

    #[test]
    fn legacy_tagged_struct_classes_construct_and_dispatch_function_style_methods() {
        let directory = TestDirectory::new("legacy-tagged-struct-class");
        let class_directory = directory.path().join("@LegacyPoint");
        fs::create_dir(&class_directory).unwrap();
        fs::write(
            class_directory.join("LegacyPoint.m"),
            "function object = LegacyPoint(value)\ndata.value = value;\nobject = class(data, 'LegacyPoint');\nend\n",
        )
        .unwrap();
        fs::write(
            class_directory.join("bump.m"),
            "function object = bump(object, amount)\nobject.value = object.value + amount;\nend\n",
        )
        .unwrap();
        fs::write(
            class_directory.join("readvalue.m"),
            "function value = readvalue(object)\nvalue = object.value;\nend\n",
        )
        .unwrap();
        fs::write(
            class_directory.join("validate.m"),
            "function validate(object)\nvalue = object.value;\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("readvalue.m"),
            "function value = readvalue(input)\nvalue = -1;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "original = LegacyPoint(2); updated = bump(original, 3); validate(updated); original_value = readvalue(original); updated_value = readvalue(updated); by_name = feval('readvalue', updated); method_handle = @readvalue; by_handle = method_handle(updated); ordinary = readvalue(1); kind = class(updated); object_flag = isobject(updated); struct_flag = isstruct(updated); isa_flag = isa(updated, 'LegacyPoint'); outside_field_failed = false; try\nleaked = updated.value;\ncatch\noutside_field_failed = true;\nend\ndot_call_failed = false; try\nignored = updated.bump(1);\ncatch\ndot_call_failed = true;\nend\noutside_tag_failed = false; try\ninvalid = class(struct('value', 1), 'Other');\ncatch\noutside_tag_failed = true;\nend\n",
        )
        .expect("old-style construction and method dispatch should execute");

        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("original_value"), Some(&Value::Double(2.0)));
        assert_eq!(workspace.get("updated_value"), Some(&Value::Double(5.0)));
        assert_eq!(workspace.get("by_name"), Some(&Value::Double(5.0)));
        assert_eq!(workspace.get("by_handle"), Some(&Value::Double(5.0)));
        assert_eq!(workspace.get("ordinary"), Some(&Value::Double(-1.0)));
        assert_eq!(workspace.get("kind"), Some(&char_row("LegacyPoint")));
        assert_eq!(workspace.get("object_flag"), Some(&Value::Logical(true)));
        assert_eq!(workspace.get("struct_flag"), Some(&Value::Logical(false)));
        assert_eq!(workspace.get("isa_flag"), Some(&Value::Logical(true)));
        assert_eq!(
            workspace.get("outside_field_failed"),
            Some(&Value::Logical(true))
        );
        assert_eq!(
            workspace.get("dot_call_failed"),
            Some(&Value::Logical(true))
        );
        assert_eq!(
            workspace.get("outside_tag_failed"),
            Some(&Value::Logical(true))
        );
    }

    #[test]
    fn legacy_tagged_struct_classes_support_one_parent_object() {
        let directory = TestDirectory::new("legacy-tagged-struct-inheritance");
        let base_directory = directory.path().join("@LegacyBase");
        let child_directory = directory.path().join("@LegacyChild");
        fs::create_dir(&base_directory).unwrap();
        fs::create_dir(&child_directory).unwrap();
        fs::write(
            base_directory.join("LegacyBase.m"),
            "function object = LegacyBase(value)\ndata.basevalue = value;\nobject = class(data, 'LegacyBase');\nend\n",
        )
        .unwrap();
        fs::write(
            base_directory.join("readbase.m"),
            "function value = readbase(object)\nvalue = object.basevalue;\nend\n",
        )
        .unwrap();
        fs::write(
            child_directory.join("LegacyChild.m"),
            "function object = LegacyChild(basevalue, extra)\nparent = LegacyBase(basevalue);\ndata.extra = extra;\nobject = class(data, 'LegacyChild', parent);\nend\n",
        )
        .unwrap();
        fs::write(
            child_directory.join("readextra.m"),
            "function value = readextra(object)\nvalue = object.extra;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "object = LegacyChild(5, 7); base_value = readbase(object); extra_value = readextra(object); child_kind = class(object); is_base = isa(object, 'LegacyBase'); is_child = isa(object, 'LegacyChild');\n",
        )
        .expect("one-parent old-style inheritance should execute");

        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("base_value"), Some(&Value::Double(5.0)));
        assert_eq!(workspace.get("extra_value"), Some(&Value::Double(7.0)));
        assert_eq!(workspace.get("child_kind"), Some(&char_row("LegacyChild")));
        assert_eq!(workspace.get("is_base"), Some(&Value::Logical(true)));
        assert_eq!(workspace.get("is_child"), Some(&Value::Logical(true)));
    }

    #[test]
    fn legacy_tagged_struct_classes_support_multiple_parent_objects() {
        let directory = TestDirectory::new("legacy-tagged-struct-multiple-inheritance");
        let left_directory = directory.path().join("@LegacyLeft");
        let right_directory = directory.path().join("@LegacyRight");
        let child_directory = directory.path().join("@LegacyPair");
        fs::create_dir(&left_directory).unwrap();
        fs::create_dir(&right_directory).unwrap();
        fs::create_dir(&child_directory).unwrap();
        fs::write(
            left_directory.join("LegacyLeft.m"),
            "function object = LegacyLeft(value)\ndata.value = value;\nobject = class(data, 'LegacyLeft');\nend\n",
        )
        .unwrap();
        fs::write(
            left_directory.join("readleft.m"),
            "function value = readleft(object)\nvalue = object.value;\nend\n",
        )
        .unwrap();
        fs::write(
            left_directory.join("pick.m"),
            "function value = pick(~)\nvalue = 'left';\nend\n",
        )
        .unwrap();
        fs::write(
            right_directory.join("LegacyRight.m"),
            "function object = LegacyRight(value)\ndata.value = value;\nobject = class(data, 'LegacyRight');\nend\n",
        )
        .unwrap();
        fs::write(
            right_directory.join("readright.m"),
            "function value = readright(object)\nvalue = object.value;\nend\n",
        )
        .unwrap();
        fs::write(
            right_directory.join("bumpright.m"),
            "function object = bumpright(object, amount)\nobject.value = object.value + amount;\nend\n",
        )
        .unwrap();
        fs::write(
            right_directory.join("pick.m"),
            "function value = pick(~)\nvalue = 'right';\nend\n",
        )
        .unwrap();
        fs::write(
            child_directory.join("LegacyPair.m"),
            "function object = LegacyPair(leftvalue, rightvalue, extra)\nleft = LegacyLeft(leftvalue);\nright = LegacyRight(rightvalue);\ndata.extra = extra;\nobject = class(data, 'LegacyPair', left, right);\nend\n",
        )
        .unwrap();
        fs::write(
            child_directory.join("readextra.m"),
            "function value = readextra(object)\nvalue = object.extra;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "object = LegacyPair(3, 5, 7); updated = bumpright(object, 4); left_value = readleft(object); right_value = readright(object); updated_right = readright(updated); extra_value = readextra(object); chosen_parent = pick(object); pair_kind = class(object); is_left = isa(object, 'LegacyLeft'); is_right = isa(object, 'LegacyRight'); is_pair = isa(object, 'LegacyPair');\n",
        )
        .expect("multiple-parent old-style inheritance should execute");

        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("left_value"), Some(&Value::Double(3.0)));
        assert_eq!(workspace.get("right_value"), Some(&Value::Double(5.0)));
        assert_eq!(workspace.get("updated_right"), Some(&Value::Double(9.0)));
        assert_eq!(workspace.get("extra_value"), Some(&Value::Double(7.0)));
        assert_eq!(workspace.get("chosen_parent"), Some(&char_row("left")));
        assert_eq!(workspace.get("pair_kind"), Some(&char_row("LegacyPair")));
        assert_eq!(workspace.get("is_left"), Some(&Value::Logical(true)));
        assert_eq!(workspace.get("is_right"), Some(&Value::Logical(true)));
        assert_eq!(workspace.get("is_pair"), Some(&Value::Logical(true)));
    }

    #[test]
    fn private_functions_follow_r2022b_caller_visibility_across_call_forms() {
        let directory = TestDirectory::new("private-functions");
        let private = directory.path().join("private");
        let sub = directory.path().join("sub");
        let dynamic = directory.path().join("dynamic");
        fs::create_dir(&private).unwrap();
        fs::create_dir(&sub).unwrap();
        fs::create_dir_all(dynamic.join("private")).unwrap();
        fs::write(
            directory.path().join("choice.m"),
            "function value = choice()\nvalue = 22;\nend\n",
        )
        .unwrap();
        fs::write(
            private.join("choice.m"),
            "function value = choice()\nvalue = 11;\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("caller_dynamic.m"),
            "function [direct, by_name, by_handle, by_eval, located, exists_code] = caller_dynamic()\ndirect = choice();\nby_name = feval('choice');\nhandle = @choice;\nby_handle = handle();\nby_eval = eval('choice()');\nlocated = which('choice');\nexists_code = exist('choice', 'file');\nend\n",
        )
        .unwrap();
        fs::write(
            private.join("sibling.m"),
            "function value = sibling()\nvalue = 44;\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("sibling.m"),
            "function value = sibling()\nvalue = 55;\nend\n",
        )
        .unwrap();
        fs::write(
            private.join("private_caller.m"),
            "function value = private_caller()\nvalue = sibling();\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("caller_private_sibling.m"),
            "function value = caller_private_sibling()\nvalue = private_caller();\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("script_probe.m"),
            "script_value = choice();\n",
        )
        .unwrap();
        fs::write(
            sub.join("subcaller.m"),
            "function value = subcaller()\nvalue = choice();\nend\n",
        )
        .unwrap();
        fs::write(
            dynamic.join("dynamic_caller.m"),
            "function value = dynamic_caller()\nvalue = dynamic_choice();\nend\n",
        )
        .unwrap();
        fs::write(
            dynamic.join("private/dynamic_choice.m"),
            "function value = dynamic_choice()\nvalue = 66;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(&mut engine, "command_value = choice();\n").unwrap();
        execute(
            &mut engine,
            "[direct, by_name, by_handle, by_eval, located, exists_code] = caller_dynamic();\n",
        )
        .unwrap();
        execute(&mut engine, "private_sibling = caller_private_sibling();\n").unwrap();
        execute(
            &mut engine,
            "script_probe; command_script = script_value;\n",
        )
        .unwrap();
        execute(&mut engine, "addpath('sub'); sub_value = subcaller();\n").unwrap();
        execute(
            &mut engine,
            "addpath('dynamic'); dynamic_private = dynamic_caller();\n",
        )
        .unwrap();

        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("command_value"), Some(&Value::Double(22.0)));
        for name in ["direct", "by_name", "by_handle", "by_eval"] {
            assert_eq!(workspace.get(name), Some(&Value::Double(11.0)), "{name}");
        }
        assert_eq!(workspace.get("exists_code"), Some(&Value::Double(2.0)));
        assert_eq!(
            workspace.get("located"),
            Some(&char_row(&private.join("choice.m").to_string_lossy()))
        );
        assert_eq!(workspace.get("private_sibling"), Some(&Value::Double(44.0)));
        assert_eq!(workspace.get("command_script"), Some(&Value::Double(11.0)));
        assert_eq!(workspace.get("sub_value"), Some(&Value::Double(22.0)));
        assert_eq!(workspace.get("dynamic_private"), Some(&Value::Double(66.0)));
    }

    #[test]
    fn genpath_recurses_but_excludes_matlab_special_directories() {
        let directory = TestDirectory::new("recursive-search-path");
        let root = directory.path().join("toolbox");
        let child = root.join("ordinary");
        let private = root.join("private");
        let package = root.join("+hidden");
        let dot_directory = root.join(".visible-to-genpath");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&private).unwrap();
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(&dot_directory).unwrap();
        fs::write(
            child.join("nested_target.m"),
            "function value = nested_target()\nvalue = 23;\nend\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "recursive_path = genpath('toolbox');\naddpath(recursive_path, '-end');\nanswer = nested_target();\n",
        )
        .unwrap();

        assert_eq!(
            engine.interpreter.workspace().get("answer"),
            Some(&Value::Double(23.0))
        );
        assert_eq!(
            engine.file_resolver.search_paths().unwrap(),
            vec![
                WorkingDirectory::new(&root)
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .path,
                WorkingDirectory::new(&dot_directory)
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .path,
                WorkingDirectory::new(&child)
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .path
            ]
        );
    }

    #[test]
    fn mat_load_save_roundtrip_scope_injection_and_append() {
        let directory = TestDirectory::new("mat-load-save");
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "a = [1 2; 3 4];\nb = single([5 6]);\nsave('roundtrip', 'a', 'b');\nclearvars a b;\nS = load('roundtrip');\nload('roundtrip', 'a');\nobserved = a;\nb = 9;\nsave('roundtrip', 'b', '-append');\nT = load('roundtrip');\ncommand_a = 12;\nsave command_form command_a\nclearvars command_a;\nload command_form command_a\ncommand_observed = command_a;\n",
        )
        .unwrap();

        assert!(directory.path().join("roundtrip.mat").is_file());
        let workspace = engine.interpreter.workspace();
        assert_eq!(workspace.get("observed"), workspace.get("a"));
        let Value::Struct(loaded) = workspace.get("S").unwrap() else {
            panic!("load with output must return a scalar struct");
        };
        assert_eq!(
            loaded
                .field_names()
                .iter()
                .map(FieldName::as_str)
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(loaded.value_at_name("a", 0), workspace.get("a"));
        let Value::Struct(appended) = workspace.get("T").unwrap() else {
            panic!("appended load must return a scalar struct");
        };
        assert_eq!(
            appended
                .field_names()
                .iter()
                .map(FieldName::as_str)
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(appended.value_at_name("b", 0), Some(&Value::Double(9.0)));
        assert_eq!(
            workspace.get("command_observed"),
            Some(&Value::Double(12.0))
        );
    }

    #[test]
    fn save_all_ignores_discarded_graphics_results_and_names_explicit_failures() {
        let directory = TestDirectory::new("mat-save-graphics-diagnostic");
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();

        execute(
            &mut engine,
            "title('discarded');\nsave('without_handle');\n",
        )
        .unwrap();
        assert!(!engine.interpreter.workspace().contains("ans"));
        assert!(directory.path().join("without_handle.mat").is_file());

        let error = execute(
            &mut engine,
            "label_handle = title('named');\nsave('with_handle');\n",
        )
        .unwrap_err();
        assert_eq!(error.category(), "runtime.builtin");
        assert!(error.message().contains("variable 'label_handle'"));
        assert!(error.message().contains("matlab.graphics.primitive.Text"));
        assert!(!directory.path().join("with_handle.mat").exists());
    }

    #[test]
    fn mat_load_and_save_use_the_current_function_workspace() {
        let directory = TestDirectory::new("mat-function-scope");
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "seed = 41;\nsave('seed', 'seed');\ntemporary = 99;\nsave('dynamic', 'temporary');\nclearvars temporary;\nresult = load_local();\ncleared = clear_loaded();\nwrite_local(17);\nwritten = load('written');\nfunction y = load_local()\nload('seed', 'seed');\ny = seed + 1;\nend\nfunction y = clear_loaded()\nload('dynamic', 'temporary');\nclear temporary;\ntry\ny = temporary;\ncatch\ny = -1;\nend\nend\nfunction write_local(x)\nsave('written', 'x');\nend\n",
        )
        .unwrap();

        assert_eq!(
            engine.interpreter.workspace().get("result"),
            Some(&Value::Double(42.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("cleared"),
            Some(&Value::Double(-1.0))
        );
        let Value::Struct(written) = engine.interpreter.workspace().get("written").unwrap() else {
            panic!("function-scope save must produce a loadable struct");
        };
        assert_eq!(
            written
                .field_names()
                .iter()
                .map(FieldName::as_str)
                .collect::<Vec<_>>(),
            ["x"]
        );
        assert_eq!(written.value_at_name("x", 0), Some(&Value::Double(17.0)));
        assert!(!engine.interpreter.workspace().contains("x"));
    }

    #[test]
    fn failed_mat_load_does_not_publish_decoded_prefix_variables() {
        let directory = TestDirectory::new("mat-load-atomic");
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "payload = 88;\nsave('bad', 'payload');\nclearvars payload;\nstable = 7;\n",
        )
        .unwrap();
        let path = directory.path().join("bad.mat");
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&[14, 0]);
        fs::write(&path, bytes).unwrap();

        let error = execute(&mut engine, "load('bad');\n").unwrap_err();
        assert_eq!(error.category(), "runtime.builtin");
        assert!(error.message().contains("cannot load MAT-file"));
        assert!(!engine.interpreter.workspace().contains("payload"));
        assert_eq!(
            engine.interpreter.workspace().get("stable"),
            Some(&Value::Double(7.0))
        );
    }

    fn assert_integer_preview<T: IntegerElement>(
        value: T,
        class: &str,
        real: &str,
        imaginary: &str,
        complex: bool,
    ) {
        let value = integer_value([1, 1], vec![value]);
        let preview = inspect_value_v1(
            &value,
            class,
            &v1_row_inspect(1, 1),
            &kernel_v1::PreviewLimits::default(),
        )
        .expect("exact integer preview");
        assert_eq!(preview.class, class);
        assert_eq!(preview.complex, complex);
        assert_eq!(
            preview.values,
            vec![kernel_v1::PreviewValue::Integer {
                real: real.to_owned(),
                imaginary: imaginary.to_owned(),
            }]
        );
        preview
            .validate(&kernel_v1::PreviewLimits::default())
            .expect("protocol-valid exact integer preview");
    }

    #[test]
    fn bare_script_collection_traverses_both_try_bodies_with_existing_scope_rules() {
        let source = "try\nworkspace_try_script\ncatch workspace_error\nworkspace_catch_script\nend\nfunction probe\ntry\nfunction_try_script\ncatch function_error\nfunction_catch_script\nend\nend\n";
        let parsed = openmat_parser::parse(SourceId::new(77), source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let lowered = openmat_hir::lower(&parsed.syntax);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

        let sites = collect_bare_script_sites(&lowered.file);
        let observed = sites
            .values()
            .map(|site| (site.name.as_str(), site.workspace_scope))
            .collect::<Vec<_>>();

        assert_eq!(
            observed,
            vec![
                ("workspace_try_script", true),
                ("workspace_catch_script", true),
                ("function_try_script", false),
                ("function_catch_script", false),
            ]
        );
        assert!(
            !sites
                .values()
                .any(|site| { matches!(site.name.as_str(), "workspace_error" | "function_error") })
        );
    }

    #[test]
    fn packaged_constructors_invoke_implicit_parents_once_and_preserve_explicit_arguments() {
        let directory = TestDirectory::new("designer-parent-constructors");
        let package = directory.path().join("+designerprobe");
        fs::create_dir(&package).unwrap();
        for (name, source) in [
            (
                "Base",
                r"classdef Base < handle
properties
    Trace = ''
end
methods
    function obj = Base(value)
        if nargin == 0
            value = 'B';
        end
        obj.Trace = [obj.Trace value];
    end
end
end",
            ),
            ("Implicit", "classdef Implicit < designerprobe.Base\nend"),
            (
                "Local",
                r"classdef Local < designerprobe.Base
methods
    function obj = Local()
        obj.Trace = [obj.Trace 'L'];
    end
end
end",
            ),
            (
                "Explicit",
                r"classdef Explicit < designerprobe.Base
methods
    function obj = Explicit()
        obj@designerprobe.Base('E');
        obj.Trace = [obj.Trace 'X'];
    end
end
end",
            ),
        ] {
            fs::write(package.join(format!("{name}.m")), source).unwrap();
        }
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(&mut engine, "a = designerprobe.Implicit(); b = designerprobe.Local(); c = designerprobe.Explicit(); assert(strcmp(a.Trace, 'B')); assert(strcmp(b.Trace, 'BL')); assert(strcmp(c.Trace, 'EX'));").unwrap();
    }

    #[test]
    fn app_designer_class_instances_inherit_controls_and_dispatch_private_methods() {
        let directory = TestDirectory::new("designer-class-app");
        fs::write(
            directory.path().join("CounterButton.m"),
            r"
classdef CounterButton < openmat.ui.Button
    properties
        Step = 2
        Count = 0
    end
    events
        Incremented
    end
    methods
        function increment(obj, source, event)
            obj.Count = obj.Count + obj.Step;
            notify(obj, 'Incremented');
        end
    end
end
",
        )
        .unwrap();
        fs::write(directory.path().join("CounterApp.m"), r"
classdef CounterApp < openmat.ui.AppBase
    properties
        Counter
        Total = 0
    end
    methods
        function wire(app)
            app.Counter = CounterButton();
            app.add(app.Counter);
            addlistener(app.Counter, 'Clicked', @(source,event) app.Counter.increment(source,event));
            addlistener(app.Counter, 'Incremented', @(source,event) app.onIncrement(source,event));
        end
    end
    methods (Access = private)
        function onIncrement(app, source, event)
            app.Total = source.Count;
        end
    end
end
").unwrap();
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(&mut engine, "app = CounterApp(); app.wire(); app.Counter.Step = 3; notify(app.Counter, 'Clicked'); notify(app.Counter, 'Clicked'); assert(app.Total == 6); assert(isa(app, 'openmat.ui.AppBase')); assert(isa(app.Counter, 'openmat.ui.Button')); assert(strcmp(app.Counter.Type, 'Button')); ").unwrap();
        let output = execute(&mut engine, "openmat_ui('describe', app.Counter);").unwrap();
        let payload = output
            .events
            .iter()
            .find_map(|event| match event {
                Event::Display(d) => d.representations.get("application/vnd.openmat.ui+json"),
                _ => None,
            })
            .unwrap();
        let metadata: serde_json::Value = serde_json::from_str(payload).unwrap();
        let events = metadata["component"]["events"].as_array().unwrap();
        assert!(events.iter().any(|e| e["name"] == "Clicked"));
        assert!(events.iter().any(|e| e["name"] == "Incremented"));
        let methods = metadata["component"]["methods"].as_array().unwrap();
        assert!(methods.iter().any(|m| m["name"] == "increment"
            && m["declaringClass"] == "CounterButton"
            && m["access"] == "public"));
        assert!(execute(&mut engine, "app.onIncrement(app.Counter, struct());").is_err());
    }

    #[test]
    fn explicit_handle_delete_uses_finalization_before_user_methods() {
        let directory = TestDirectory::new("explicit-handle-delete");
        fs::write(
            directory.path().join("DeleteProbe.m"),
            r"
classdef DeleteProbe < handle
    properties
        Fail = false
    end
    methods
        function delete(obj)
            global deletion_log
            deletion_log = [deletion_log, 2, double(isvalid(obj))];
            if obj.Fail
                error('OpenMatTest:DeleteFailure', 'Deletion test failure');
            end
        end
    end
end
",
        )
        .unwrap();
        fs::write(
            directory.path().join("DeleteChild.m"),
            "classdef DeleteChild < DeleteProbe\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("RecordDestroy.m"),
            r"
function RecordDestroy(source, event)
    global deletion_log
    deletion_log = [deletion_log, 1];
end
",
        )
        .unwrap();
        fs::write(
            directory.path().join("ValueDeleteProbe.m"),
            r"
classdef ValueDeleteProbe
    properties
        Value = 41
    end
    methods
        function result = delete(obj)
            result = obj.Value + 1;
        end
    end
end
",
        )
        .unwrap();
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        for class in ["DeleteProbe", "DeleteChild"] {
            for call in ["delete(obj)", "f = @delete; f(obj)", "feval('delete', obj)"] {
                execute(
                    &mut engine,
                    &format!(
                        r"
global deletion_log
deletion_log = [];
obj = {class}(); alias = obj;
listener = addlistener(obj, 'ObjectBeingDestroyed', @RecordDestroy);
{call};
assert(isequal(deletion_log, [1, 2, 0]));
assert(~isvalid(obj)); assert(~isvalid(alias)); assert(isvalid(listener));
delete(alias);
assert(isequal(deletion_log, [1, 2, 0]));
"
                    ),
                )
                .unwrap();
            }
        }
        execute(
            &mut engine,
            r"
lastwarn(''); deletion_log = [];
obj = DeleteProbe(); obj.Fail = true;
caught = false;
try
    delete(obj);
catch
    caught = true;
end
[warning_text, ~] = lastwarn;
assert(~caught); assert(~isvalid(obj)); assert(~isempty(warning_text));
assert(isequal(deletion_log, [2, 0]));
value = ValueDeleteProbe(); assert(delete(value) == 42);
",
        )
        .unwrap();
    }

    #[test]
    fn ui_tree_initializes_once_reparents_and_deletes_owned_children_and_listeners() {
        let directory = TestDirectory::new("ui-tree-lifecycle");
        fs::write(
            directory.path().join("TreeProbe.m"),
            r"classdef TreeProbe < openmat.ui.Panel
properties
    SetupCount = 0
end
methods (Access = protected)
    function setup(obj)
        obj.SetupCount = obj.SetupCount + 1;
        button = openmat.ui.Button();
        button.Position = [10 20 100 30];
        obj.add(button);
    end
    function teardown(obj)
        global tree_teardown_log
        tree_teardown_log = [tree_teardown_log, obj.SetupCount];
        obj.refresh();
    end
end
end",
        )
        .unwrap();
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        execute(
            &mut engine,
            "global tree_teardown_log; tree_teardown_log = []; lastwarn('');",
        )
        .unwrap();
        execute(&mut engine, "root = openmat.ui.Window(); a = TreeProbe(); b = openmat.ui.Panel(); root.add(a); root.add(b); root.initialize(); a.initialize(); assert(a.SetupCount == 1); button = a.Children{1}; key = button.RuntimeId; b.add(a); assert(isempty(root.Children{1}.Children) == false); assert(numel(root.Children) == 1); assert(a.SetupCount == 1); assert(strcmp(a.Parent.RuntimeId, b.RuntimeId)); assert(strcmp(root.findRuntime(key).RuntimeId, key)); b.Layout.Mode = 'grid'; b.Layout.Columns = 3; a.Layout.Column = 2;").unwrap();
        assert!(execute(&mut engine, "a.add(root);").is_err());
        execute(&mut engine, "source = openmat.ui.Button(); listener = a.listen(source, 'Clicked', @(s,e) disp(1)); delete(a); assert(~isvalid(a)); assert(~isvalid(button)); assert(~isvalid(listener)); assert(isempty(b.Children)); assert(isempty(root.findRuntime(key))); added = TreeProbe(); b.add(added); assert(added.SetupCount == 1); delete(root); assert(~isvalid(added));").unwrap();
        execute(&mut engine, "assert(isequal(tree_teardown_log, [1 1])); direct = TreeProbe(); direct.initialize(); direct.delete(); assert(~isvalid(direct)); assert(isequal(tree_teardown_log, [1 1 1])); [warning_text, ~] = lastwarn; assert(isempty(warning_text));").unwrap();
    }

    #[test]
    fn designer_generated_components_have_independent_children_state_and_layout() {
        let directory = TestDirectory::new("ui-composite-instances");
        fs::write(
            directory.path().join("ParameterEditor.m"),
            include_str!("../../../examples/app-designer/ParameterEditor.m"),
        )
        .unwrap();
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        fs::write(
            directory.path().join("ParameterApp.m"),
            include_str!("../../../examples/app-designer/ParameterApp.m"),
        )
        .unwrap();
        execute(
            &mut engine,
            "prototype = ParameterApp(); openmat_ui('describe', prototype);",
        )
        .unwrap();
        execute(&mut engine, "first = ParameterEditor(); second = ParameterEditor(); first.Value = 10; second.Value = 25; first.initialize(); second.initialize(); first.initialize(); assert(numel(first.Children) == 3); assert(~strcmp(first.Editor.RuntimeId, second.Editor.RuntimeId)); first.Editor.Value = 42; notify(first.Editor, 'ValueChanged', struct()); assert(first.Value == 42); assert(second.Value == 25); assert(first.Dial.Value == 42); first.Layout.Column = 2;").unwrap();
        let output = execute(&mut engine, "openmat_ui('snapshot', first);").unwrap();
        let payload = output
            .events
            .iter()
            .find_map(|event| match event {
                Event::Display(display) => display
                    .representations
                    .get("application/vnd.openmat.ui+json"),
                _ => None,
            })
            .unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(snapshot["component"]["layout"]["mode"], "grid");
        assert_eq!(
            snapshot["component"]["layout"]["column"].as_f64(),
            Some(2.0)
        );
        assert_eq!(
            snapshot["component"]["children"][2]["layout"]["column"].as_f64(),
            Some(3.0)
        );
        assert!(execute(&mut engine, "first.Editor = openmat.ui.NumericField();").is_err());
        execute(&mut engine, "child = first.Editor; delete(first); assert(~isvalid(first)); assert(~isvalid(child)); assert(isvalid(second.Editor));").unwrap();
    }

    #[test]
    fn ui_extended_layout_and_scroll_panel_publish_validated_native_snapshots() {
        let mut engine = RuntimeEngine::new().unwrap();
        execute(&mut engine, "panel = openmat.ui.ScrollPanel(); panel.Layout.WidthMode = 'fill'; panel.Layout.HeightMode = 'content'; panel.Layout.MinWidth = 80; panel.Layout.MaxWidth = 500; panel.Layout.GrowX = 2; panel.Layout.ColumnTracks = '240 auto 1.5fr'; panel.Layout.RowTracks = 'auto 1fr'; panel.ScrollDirection = 'both'; panel.initialize();").unwrap();
        let output = execute(&mut engine, "openmat_ui('snapshot', panel);").unwrap();
        let payload = output
            .events
            .iter()
            .find_map(|event| match event {
                Event::Display(display) => display
                    .representations
                    .get("application/vnd.openmat.ui+json"),
                _ => None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(value["component"]["properties"]["Type"], "ScrollPanel");
        assert_eq!(value["component"]["properties"]["ScrollDirection"], "both");
        assert_eq!(value["component"]["layout"]["widthMode"], "fill");
        assert_eq!(
            value["component"]["layout"]["columnTracks"],
            "240 auto 1.5fr"
        );
        assert_eq!(
            value["component"]["layout"]["minWidth"].as_f64(),
            Some(80.0)
        );
        for track in [
            "0fr",
            "-1",
            "1e2",
            "1FR",
            "calc(100% - 2px)",
            "1fr;display:none",
        ] {
            assert!(
                execute(
                    &mut engine,
                    &format!(
                        "panel.Layout.ColumnTracks = '{track}'; openmat_ui('snapshot', panel);"
                    )
                )
                .is_err(),
                "invalid track: {track}"
            );
        }
        assert!(execute(&mut engine, "panel.Layout.ColumnTracks = ''; panel.Layout.MinWidth = 600; openmat_ui('snapshot', panel);").is_err());
        assert!(execute(&mut engine, "panel.Layout.MinWidth = 0; panel.Layout.WidthMode = 'stretch'; openmat_ui('snapshot', panel);").is_err());
        execute(
            &mut engine,
            "panel.Layout.WidthMode = 'fixed'; openmat_ui('snapshot', panel);",
        )
        .unwrap();
    }

    #[test]
    fn app_designer_embedded_control_publishes_typed_snapshot() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        for code in [
            "root = openmat.ui.Control();",
            "root.Type = 'Window'; root.Id = 'root';",
            "child = openmat.ui.Control();",
            "child.Type = 'NumericField'; child.Value = 3;",
            "root.add(child);",
            "root.initialize();",
        ] {
            execute(&mut engine, code).expect("UI classes execute without filesystem installation");
        }
        let output = execute(&mut engine, "openmat_ui('snapshot', root);").unwrap();
        let payload = output
            .events
            .iter()
            .find_map(|event| match event {
                Event::Display(display) => display
                    .representations
                    .get("application/vnd.openmat.ui+json"),
                _ => None,
            })
            .expect("UI display");
        let value: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["component"]["properties"]["Id"], "root");
        assert_eq!(
            value["component"]["children"][0]["properties"]["Value"].as_f64(),
            Some(3.0)
        );
    }

    #[test]
    fn app_designer_custom_properties_and_updates_use_native_class_semantics() {
        let directory = TestDirectory::new("designer-custom-class");
        fs::write(
            directory.path().join("GainControl.m"),
            include_str!("../../../examples/app-designer/GainControl.m"),
        )
        .unwrap();
        let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        let output = execute(&mut engine, "widget = GainControl(); widget.Value = 7; widget.initialize(); openmat_ui('describe', widget);").expect("custom UI class initializes");
        let payload = output
            .events
            .iter()
            .find_map(|event| match event {
                Event::Display(d) => d.representations.get("application/vnd.openmat.ui+json"),
                _ => None,
            })
            .unwrap();
        let metadata: serde_json::Value = serde_json::from_str(payload).unwrap();
        let fields = metadata["component"]["schema"].as_array().unwrap();
        assert!(
            fields
                .iter()
                .any(|p| p["name"] == "Value" && p["writable"] == true)
        );
        assert!(!fields.iter().any(|p| p["name"] == "Readout"));
        assert_eq!(
            metadata["component"]["children"][0]["properties"]["Text"],
            "Gain: 7"
        );
        execute(
            &mut engine,
            "app = struct('Gain', widget); app.Gain.Value = 9; app.Gain.refresh();",
        )
        .expect("callback changes live handle property");
        let updated = execute(&mut engine, "openmat_ui('snapshot', app.Gain);").unwrap();
        assert!(updated.events.iter().any(|event| matches!(event, Event::Display(d) if d.representations.get("application/vnd.openmat.ui+json").is_some_and(|p| p.contains("Gain: 9")))));
        execute(&mut engine, "children = app.Gain.Children; callback = children{2}.ButtonPushedFcn; feval(callback, children{2}, struct()); app.Gain.refresh();").expect("internal UI callback preserves captured object identity");
        let updated = execute(&mut engine, "openmat_ui('snapshot', app.Gain);").unwrap();
        assert!(updated.events.iter().any(|event| matches!(event, Event::Display(d) if d.representations.get("application/vnd.openmat.ui+json").is_some_and(|p| p.contains("Gain: 10")))));
        let mut second = RuntimeEngine::with_working_directory(directory.path()).unwrap();
        assert!(
            execute(&mut second, "disp(widget);").is_err(),
            "preview workspace must remain session-local"
        );
    }

    #[test]
    fn source_pipeline_executes_control_flow_functions_and_persists_workspace() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        let output = execute(
            &mut engine,
            "x = 0;\nwhile x < 3\nx = x + 1;\nend\nif x == 3\ny = x * 10;\nelse\ny = 0;\nend\ndisp(y);\n",
        )
        .expect("control flow executes");
        assert!(output.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str) == Some("    30")
        )));
        assert!(output.events.iter().any(|event| matches!(
            event,
            Event::WorkspaceDelta(delta)
                if delta.added.iter().map(|item| item.name.as_str()).collect::<Vec<_>>()
                    == ["x", "y"]
        )));

        execute(
            &mut engine,
            "function y = twice(value)\ny = value * 2;\nend\nanswer = twice(y);\n",
        )
        .expect("user function executes with prior workspace");
        assert_eq!(
            engine.inspect(&scalar_inspect("answer")).unwrap().values,
            vec![PreviewValue::Number { value: 60.0 }]
        );
    }

    #[test]
    // One end-to-end scenario intentionally keeps COW, promotion, and exact integer
    // coverage together so every supported writeback storage path shares the same setup.
    #[allow(clippy::too_many_lines)]
    fn variable_editor_writeback_uses_one_based_column_major_cow_and_complex_promotion() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(&mut engine, "A = [1 2; 3 4];\nB = A;\n").expect("workspace arrays execute");

        let summary = engine
            .set_variable_element(&kernel_v3::SetVariableElementRequest {
                name: "A".to_owned(),
                indices: vec![2, 1],
                value: kernel_v3::NumericScalar {
                    real: "9".to_owned(),
                    imaginary: "0".to_owned(),
                },
                expected_revision: 0,
            })
            .expect("real element writeback");
        assert_eq!(summary.class, "double");
        assert!(!summary.complex);

        let matrix_inspect = InspectRequest {
            name: "A".to_owned(),
            range: MatrixRange {
                start: vec![1, 1],
                size: vec![2, 2],
            },
            max_elements: 4,
        };
        assert_eq!(
            engine.inspect(&matrix_inspect).unwrap().values,
            vec![
                PreviewValue::Number { value: 1.0 },
                PreviewValue::Number { value: 9.0 },
                PreviewValue::Number { value: 2.0 },
                PreviewValue::Number { value: 4.0 },
            ]
        );
        assert_eq!(
            engine
                .inspect(&InspectRequest {
                    name: "B".to_owned(),
                    ..matrix_inspect.clone()
                })
                .unwrap()
                .values,
            vec![
                PreviewValue::Number { value: 1.0 },
                PreviewValue::Number { value: 3.0 },
                PreviewValue::Number { value: 2.0 },
                PreviewValue::Number { value: 4.0 },
            ],
            "editing A must detach its shared storage from B"
        );

        let summary = engine
            .set_variable_element(&kernel_v3::SetVariableElementRequest {
                name: "A".to_owned(),
                indices: vec![1, 2],
                value: kernel_v3::NumericScalar {
                    real: "7".to_owned(),
                    imaginary: "-2".to_owned(),
                },
                expected_revision: 0,
            })
            .expect("complex element writeback");
        assert!(summary.complex);
        assert_eq!(
            engine.inspect(&matrix_inspect).unwrap().values,
            vec![
                PreviewValue::Complex {
                    real: 1.0,
                    imaginary: 0.0,
                },
                PreviewValue::Complex {
                    real: 9.0,
                    imaginary: 0.0,
                },
                PreviewValue::Complex {
                    real: 7.0,
                    imaginary: -2.0,
                },
                PreviewValue::Complex {
                    real: 4.0,
                    imaginary: 0.0,
                },
            ]
        );

        let unsigned = integer_value([1, 2], vec![1_u64, 2_u64]);
        engine
            .interpreter
            .workspace_mut()
            .insert("U_copy", unsigned.clone());
        engine.interpreter.workspace_mut().insert("U", unsigned);
        engine
            .set_variable_element(&kernel_v3::SetVariableElementRequest {
                name: "U".to_owned(),
                indices: vec![1, 2],
                value: kernel_v3::NumericScalar {
                    real: u64::MAX.to_string(),
                    imaginary: "0".to_owned(),
                },
                expected_revision: 0,
            })
            .expect("exact uint64 writeback");
        let Value::Array(ArrayData::Integer(updated)) =
            engine.interpreter.workspace().get("U").unwrap()
        else {
            panic!("updated uint64 array")
        };
        assert_eq!(
            updated
                .element_decimal(1)
                .expect("updated element")
                .real_component(),
            u64::MAX.to_string()
        );
        let Value::Array(ArrayData::Integer(copy)) =
            engine.interpreter.workspace().get("U_copy").unwrap()
        else {
            panic!("copied uint64 array")
        };
        assert_eq!(
            copy.element_decimal(1)
                .expect("copied element")
                .real_component(),
            "2",
            "integer writeback must preserve copy-on-write isolation"
        );

        engine.interpreter.workspace_mut().insert(
            "CI",
            integer_value([1, 1], vec![ComplexInteger::new(1_i64, 2_i64)]),
        );
        engine
            .set_variable_element(&kernel_v3::SetVariableElementRequest {
                name: "CI".to_owned(),
                indices: vec![1, 1],
                value: kernel_v3::NumericScalar {
                    real: i64::MIN.to_string(),
                    imaginary: i64::MAX.to_string(),
                },
                expected_revision: 0,
            })
            .expect("exact complex int64 writeback");
        let Value::Array(ArrayData::Integer(complex_integer)) =
            engine.interpreter.workspace().get("CI").unwrap()
        else {
            panic!("updated complex int64 array")
        };
        let components = complex_integer
            .element_decimal(0)
            .expect("complex integer element");
        assert_eq!(components.real_component(), i64::MIN.to_string());
        assert_eq!(components.imaginary_component(), i64::MAX.to_string());
    }

    #[test]
    fn workspace_index_assignment_captures_root_between_lhs_and_rhs() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "global A;\nA = [1 2];\nA(touch_index()) = touch_rhs();\n\
             function i = touch_index()\n\
             global A;\nA = [10 20];\ni = 1;\nend\n\
             function value = touch_rhs()\n\
             global A;\nA = [100 200];\nvalue = 7;\nend\n",
        )
        .expect("side-effecting indexed assignment executes");

        assert_eq!(
            engine.inspect(&row_inspect("A", 2)).unwrap().values,
            vec![
                PreviewValue::Number { value: 7.0 },
                PreviewValue::Number { value: 20.0 },
            ],
            "MATLAB R2022b evaluates LHS selectors, captures A, then evaluates the RHS"
        );
    }

    #[test]
    fn workspace_index_read_captures_target_before_selector_evaluation() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "global A;\nA = [1 2];\nvalue = A(touch_index());\n\
             function i = touch_index()\n\
             global A;\nA = [10 20];\ni = 1;\nend\n",
        )
        .expect("side-effecting indexed read executes");

        assert_eq!(
            engine.inspect(&scalar_inspect("value")).unwrap().values,
            vec![PreviewValue::Number { value: 1.0 }],
            "MATLAB R2022b captures A before evaluating the selector"
        );
        assert_eq!(
            engine.inspect(&row_inspect("A", 2)).unwrap().values,
            vec![
                PreviewValue::Number { value: 10.0 },
                PreviewValue::Number { value: 20.0 },
            ]
        );
    }

    #[test]
    fn owned_indexing_updates_local_persistent_and_shared_capture_bindings() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "[local_value, local_alias, persistent_value, captured_value, captured_alias] = exercise_bindings();\n\
             function [local_value, local_alias, persistent_value, captured_value, captured_alias] = exercise_bindings()\n\
             local_value = [1 2 3 4];\nlocal_alias = local_value;\nlocal_value(2) = local_value(1);\ntry\nlocal_value(0) = 77;\ncatch\nend\n\
             persistent retained\nretained = [10 20 30];\nretained(3) = 99;\ntry\nretained(0) = 77;\ncatch\nend\npersistent_value = retained;\n\
             captured_value = [5 6 7];\ncaptured_alias = captured_value;\nupdate_capture();\n\
             function update_capture()\ntry\ncaptured_value(0) = 77;\ncatch\nend\ncaptured_value(1) = 55;\nend\nend\n",
        )
        .expect("all statically resolved binding classes should execute indexed updates");

        assert_eq!(
            engine
                .inspect(&row_inspect("local_value", 4))
                .unwrap()
                .values,
            vec![1.0, 1.0, 3.0, 4.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("local_alias", 4))
                .unwrap()
                .values,
            vec![1.0, 2.0, 3.0, 4.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("persistent_value", 3))
                .unwrap()
                .values,
            vec![10.0, 20.0, 99.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("captured_value", 3))
                .unwrap()
                .values,
            vec![55.0, 6.0, 7.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("captured_alias", 3))
                .unwrap()
                .values,
            vec![5.0, 6.0, 7.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn shared_local_assignment_captures_root_between_selector_and_rhs() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "[ordered, original] = local_order_probe();\n\
             function [value, original] = local_order_probe()\n\
             value = [1 2];\noriginal = value;\nvalue(touch_index()) = touch_rhs();\n\
             function index = touch_index()\nvalue = [10 20];\nindex = 1;\nend\n\
             function result = touch_rhs()\nvalue = [100 200];\nresult = 7;\nend\nend\n",
        )
        .expect("shared local selector and RHS side effects should preserve evaluation order");

        assert_eq!(
            engine.inspect(&row_inspect("ordered", 2)).unwrap().values,
            vec![
                PreviewValue::Number { value: 7.0 },
                PreviewValue::Number { value: 20.0 },
            ],
            "MATLAB R2022b evaluates selectors, captures the local root, then evaluates the RHS"
        );
        assert_eq!(
            engine.inspect(&row_inspect("original", 2)).unwrap().values,
            vec![
                PreviewValue::Number { value: 1.0 },
                PreviewValue::Number { value: 2.0 },
            ]
        );
    }

    #[test]
    fn overlapping_self_assignment_reads_the_original_rhs_snapshot() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "values = [1 2 3 4];\nvalues([2 3 4]) = values([1 2 3]);\n",
        )
        .expect("overlapping self-assignment should execute from an RHS snapshot");

        assert_eq!(
            engine.inspect(&row_inspect("values", 4)).unwrap().values,
            vec![1.0, 1.0, 2.0, 3.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn bare_source_function_values_invoke_zero_arguments_and_workspace_values_shadow_them() {
        let directory = TestDirectory::new("bare-source-function-value");
        fs::write(
            directory.path().join("value_source.m"),
            "function value = value_source\nvalue = 42;\nend\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::with_search_paths([directory.path()]).unwrap();

        execute(
            &mut engine,
            "bare = value_source;\nhandle = @value_source;\nfrom_handle = handle();\n",
        )
        .expect("bare source function and explicit handle execute");
        assert_eq!(
            engine.inspect(&scalar_inspect("bare")).unwrap().values,
            vec![PreviewValue::Number { value: 42.0 }]
        );
        assert_eq!(
            engine
                .inspect(&scalar_inspect("from_handle"))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 42.0 }]
        );

        execute(&mut engine, "value_source = 9;\nshadowed = value_source;\n")
            .expect("workspace value shadows source function");
        assert_eq!(
            engine.inspect(&scalar_inspect("shadowed")).unwrap().values,
            vec![PreviewValue::Number { value: 9.0 }]
        );

        execute(
            &mut engine,
            "clear value_source;\nrestored = value_source;\n",
        )
        .expect("clearing workspace shadow restores source function");
        assert_eq!(
            engine.inspect(&scalar_inspect("restored")).unwrap().values,
            vec![PreviewValue::Number { value: 42.0 }]
        );
    }

    #[test]
    fn source_pipeline_commits_plot_handles_and_keeps_notices_internal() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        let output = execute(
            &mut engine,
            "h = plot([1 2; 3 4; 5 6]);\nkind = class(h);\n",
        )
        .expect("matrix plot executes through the real source pipeline");
        assert!(!output.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.contains_key("application/vnd.openmat.figure+json")
        )));
        let notices = engine.take_graphics_notices();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].discovery);
        assert_eq!(notices[0].revision, 1);

        let Value::GraphicsArray(handles) = engine.interpreter.workspace().get("h").unwrap() else {
            panic!("plot output must be a homogeneous Line handle column")
        };
        assert_eq!(handles.shape().dimensions(), &[2, 1]);
        assert_eq!(handles.class(), openmat_runtime::GraphicsClass::LineSeries);
        assert_eq!(
            engine.interpreter.workspace().get("kind"),
            Some(&char_row("matlab.graphics.chart.primitive.Line"))
        );
        let axes = engine.graphics_session().current_axes().unwrap();
        assert_eq!(engine.graphics_session().children(axes).unwrap().len(), 2);

        execute(&mut engine, "clear h;\n").expect("workspace clear executes");
        assert!(!engine.interpreter.workspace().contains("h"));
        assert_eq!(engine.graphics_session().children(axes).unwrap().len(), 2);

        execute(
            &mut engine,
            "hold(\"on\");\nplot([7 8 9]);\nheld = ishold();\n",
        )
        .expect("functional hold form executes");
        assert_eq!(engine.graphics_session().children(axes).unwrap().len(), 3);
        assert_eq!(
            engine.interpreter.workspace().get("held"),
            Some(&Value::Double(1.0))
        );
        let notices = engine.take_graphics_notices();
        assert_eq!(notices.len(), 2);
        assert!(notices.iter().all(|notice| !notice.discovery));
    }

    #[test]
    fn source_pipeline_xy_plot_completes_without_spinning() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        let output = execute(&mut engine, "plot([1 2 3], [1 4 9]);\n")
            .expect("two-vector plot executes through the real source pipeline");
        assert!(!output.result.interrupted);
        let notices = engine.take_graphics_notices();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].discovery);
    }

    #[test]
    fn session_commands_clear_presentational_history_and_persist_format_state() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(&mut engine, "marker = 41;\n").expect("workspace seed");

        let cleared = execute(&mut engine, "clc;\n").expect("clc executes");
        assert!(cleared.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.contains_key(COMMAND_WINDOW_CLEAR_MIME)
        )));
        assert_eq!(
            engine.inspect(&scalar_inspect("marker")).unwrap().values,
            vec![PreviewValue::Number { value: 41.0 }]
        );

        execute(&mut engine, "format long g;\n").expect("format command executes");
        assert_eq!(
            engine.interpreter.display_format().numeric,
            NumericFormat::LongG
        );
        let rendered = execute(&mut engine, "1 / 3\n").expect("formatted expression executes");
        assert!(rendered.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("ans =\n\n0.333333333333333")
        )));

        let ordered = execute(
            &mut engine,
            "format short;\n1 / 3\nformat long g;\n1 / 3\nformat compact;\n2\n",
        )
        .expect("ordered format changes execute");
        let rendered = ordered
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Display(DisplayEvent { representations }) => {
                    representations.get("text/plain").map(String::as_str)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rendered,
            [
                "ans =\n\n0.3333",
                "ans =\n\n0.333333333333333",
                "ans =\n     2",
            ]
        );

        let names = execute(&mut engine, "who marker;\n").expect("who command executes");
        assert!(names.events.iter().any(|event| matches!(
            event,
            Event::Stream(StreamEvent { stream: StreamKind::Stdout, text })
                if text.contains("marker")
        )));
        let details = execute(&mut engine, "whos marker;\n").expect("whos command executes");
        assert!(details.events.iter().any(|event| matches!(
            event,
            Event::Stream(StreamEvent { stream: StreamKind::Stdout, text })
                if text.contains("marker") && text.contains("double")
        )));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn command_window_expression_results_match_r2022b_ans_and_display_boundaries() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");

        let scalar = execute(&mut engine, "1\n").expect("scalar expression executes");
        assert!(scalar.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("ans =\n\n     1")
        )));
        assert_eq!(
            engine.inspect(&scalar_inspect("ans")).unwrap().values,
            vec![PreviewValue::Number { value: 1.0 }]
        );

        let comma = execute(&mut engine, "8,\n").expect("comma-terminated expression executes");
        assert!(comma.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("ans =\n\n     8")
        )));
        assert_eq!(
            engine.inspect(&scalar_inspect("ans")).unwrap().values,
            vec![PreviewValue::Number { value: 8.0 }]
        );

        let vector = execute(&mut engine, "[1 2]\n").expect("row vector expression executes");
        assert!(vector.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("ans =\n\n     1     2")
        )));
        assert_eq!(
            engine.inspect(&row_inspect("ans", 2)).unwrap().values,
            vec![
                PreviewValue::Number { value: 1.0 },
                PreviewValue::Number { value: 2.0 },
            ]
        );

        let answer = execute(&mut engine, "ans\n").expect("ans expression executes");
        assert!(answer.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("ans =\n\n     1     2")
        )));

        let suppressed = execute(&mut engine, "7;\n").expect("suppressed expression executes");
        assert!(
            !suppressed
                .events
                .iter()
                .any(|event| matches!(event, Event::Display(_)))
        );
        assert_eq!(
            engine.inspect(&scalar_inspect("ans")).unwrap().values,
            vec![PreviewValue::Number { value: 7.0 }]
        );

        let assigned = execute(&mut engine, "x = 5\n").expect("visible assignment executes");
        assert!(assigned.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("x =\n\n     5")
        )));

        execute(&mut engine, "ans = 99;\n").expect("answer seed executes");
        let disp = execute(&mut engine, "disp(1)\n").expect("disp executes without an output");
        assert_eq!(
            disp.events
                .iter()
                .filter_map(|event| match event {
                    Event::Display(DisplayEvent { representations }) => {
                        representations.get("text/plain").map(String::as_str)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            ["     1"]
        );
        assert_eq!(
            engine.inspect(&scalar_inspect("ans")).unwrap().values,
            vec![PreviewValue::Number { value: 99.0 }]
        );

        let call = execute(&mut engine, "sqrt(4)\n").expect("value-returning call executes");
        assert!(call.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("ans =\n\n     2")
        )));
        assert_eq!(
            engine.inspect(&scalar_inspect("ans")).unwrap().values,
            vec![PreviewValue::Number { value: 2.0 }]
        );

        let intrinsic = execute(&mut engine, "isobject(1)\n")
            .expect("value-returning intrinsic executes as a statement");
        assert!(intrinsic.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("ans =\n\n     0")
        )));

        let call_metadata = execute(
            &mut engine,
            "function y = report_nargout()\ny = nargout;\nend\nreport_nargout()\n",
        )
        .expect("statement call preserves zero requested outputs");
        assert!(call_metadata.events.iter().any(|event| matches!(
            event,
            Event::Display(DisplayEvent { representations })
                if representations.get("text/plain").map(String::as_str)
                    == Some("ans =\n\n     0")
        )));

        execute(&mut engine, "ans = 123;\n").expect("answer seed executes");
        let no_output_call = execute(
            &mut engine,
            "function touch_output()\ndisp(5);\nend\ntouch_output()\n",
        )
        .expect("zero-output function executes");
        assert_eq!(
            no_output_call
                .events
                .iter()
                .filter_map(|event| match event {
                    Event::Display(DisplayEvent { representations }) => {
                        representations.get("text/plain").map(String::as_str)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            ["     5"]
        );
        assert_eq!(
            engine.inspect(&scalar_inspect("ans")).unwrap().values,
            vec![PreviewValue::Number { value: 123.0 }]
        );
    }

    #[test]
    fn command_window_renders_seeded_six_by_six_rand_as_an_aligned_matrix() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        let output =
            execute(&mut engine, "rng(0); rand(6,6)\n").expect("seeded matrix expression executes");
        let rendered = output.events.iter().find_map(|event| match event {
            Event::Display(DisplayEvent { representations }) => {
                representations.get("text/plain").map(String::as_str)
            }
            _ => None,
        });

        assert_eq!(
            rendered,
            Some(concat!(
                "ans =\n\n",
                "    0.8147    0.2785    0.9572    0.7922    0.6787    0.7060\n",
                "    0.9058    0.5469    0.4854    0.9595    0.7577    0.0318\n",
                "    0.1270    0.9575    0.8003    0.6557    0.7431    0.2769\n",
                "    0.9134    0.9649    0.1419    0.0357    0.3922    0.0462\n",
                "    0.6324    0.1576    0.4218    0.8491    0.6555    0.0971\n",
                "    0.0975    0.9706    0.9157    0.9340    0.1712    0.8235",
            ))
        );
    }

    #[test]
    fn compiler_failure_has_named_utf8_byte_range_and_does_not_mutate_workspace() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(&mut engine, "kept = 7;\n").expect("initial assignment");
        let error = execute(&mut engine, "变量 = ;\n").expect_err("bad source must fail");
        assert_eq!(error.category(), "compile.parse");
        let range = error.diagnostics()[0].range.as_ref().expect("source range");
        assert_eq!(range.source_name, "test.m");
        assert!(range.end <= "变量 = ;\n".len() as u64);
        assert_eq!(
            engine.inspect(&scalar_inspect("kept")).unwrap().values,
            vec![PreviewValue::Number { value: 7.0 }]
        );
    }

    #[test]
    fn bare_clear_removes_prior_workspace_values_without_clearing_the_host_session() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(&mut engine, "stale = 7;\n").expect("initial assignment");

        let output = execute(&mut engine, "clear;\nfresh = 9;\n")
            .expect("bare clear and following assignment should execute");
        assert!(output.events.iter().any(|event| matches!(
            event,
            Event::WorkspaceDelta(delta)
                if delta.removed.iter().any(|name| name == "stale")
                    && delta.added.iter().any(|summary| summary.name == "fresh")
        )));
        assert!(engine.inspect(&scalar_inspect("stale")).is_err());
        assert_eq!(
            engine.inspect(&scalar_inspect("fresh")).unwrap().values,
            vec![PreviewValue::Number { value: 9.0 }]
        );
    }

    #[test]
    fn runtime_error_preserves_prior_assignment_and_reports_delta() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        let error = execute(&mut engine, "saved = 9;\nmissing_name;\n")
            .expect_err("undefined global must fail");
        assert_eq!(error.category(), "runtime.undefinedName");
        assert_eq!(error.diagnostics()[0].code.as_deref(), Some("OMR0002"));
        assert!(error.events.iter().any(|event| matches!(
            event,
            Event::WorkspaceDelta(delta)
                if delta.added.iter().any(|summary| summary.name == "saved")
        )));
        assert_eq!(
            engine.inspect(&scalar_inspect("saved")).unwrap().values,
            vec![PreviewValue::Number { value: 9.0 }]
        );
    }

    #[test]
    fn array_bounds_error_has_specific_category_and_diagnostic_code() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        let error = execute(&mut engine, "values = [10 20]; selected = values(3);\n")
            .expect_err("out-of-bounds index must fail");

        assert_eq!(error.category(), "runtime.indexOutOfBounds");
        assert_eq!(error.diagnostics()[0].code.as_deref(), Some("OMR0016"));
        assert!(
            error.diagnostics()[0]
                .range
                .as_ref()
                .is_some_and(|range| range.source_name == "test.m")
        );

        let generic = execute(&mut engine, "bad = [1 2; 3];\n")
            .expect_err("concatenation mismatch must fail");
        assert_eq!(generic.category(), "runtime.invalidState");
        assert_eq!(generic.diagnostics()[0].code.as_deref(), Some("OMR0013"));
    }

    #[test]
    fn every_structured_invalid_index_reason_uses_the_indexing_category() {
        use openmat_runtime::IndexErrorKind;

        let reasons = [
            IndexErrorKind::NonFinite,
            IndexErrorKind::NonInteger,
            IndexErrorKind::NonPositive,
            IndexErrorKind::LogicalLength {
                expected: 2,
                actual: 3,
            },
            IndexErrorKind::NonNumeric,
            IndexErrorKind::NoSubscripts,
        ];
        for reason in reasons {
            let kind = RuntimeErrorKind::InvalidExecutionState {
                message: "ordinary internal invalid state".to_owned(),
                array: Some(Box::new(ArrayRuntimeError::InvalidIndex {
                    argument: 0,
                    reason,
                })),
            };
            assert_eq!(runtime_category(&kind), "runtime.indexOutOfBounds");
            assert_eq!(runtime_code(&kind), "OMR0016");
        }

        let ordinary = RuntimeErrorKind::InvalidExecutionState {
            message: "invalid index is outside array bounds".to_owned(),
            array: None,
        };
        assert_eq!(runtime_category(&ordinary), "runtime.invalidState");
        assert_eq!(runtime_code(&ordinary), "OMR0013");
    }

    #[test]
    fn only_structured_linalg_dimension_mismatch_uses_dimension_category() {
        let dimension_mismatch: RuntimeErrorKind =
            RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
                operation: "linear solve coefficient and right-hand-side rows",
                left: 2,
                right: 3,
            })
            .into();
        assert_eq!(
            runtime_category(&dimension_mismatch),
            "runtime.dimensionMismatch"
        );
        assert_eq!(runtime_code(&dimension_mismatch), "OMR0019");

        let other_linalg_errors = [
            LinalgError::SingularMatrix {
                provider: "test",
                operation: "linear solve",
                pivot: 1,
            },
            LinalgError::RankDeficient {
                provider: "test",
                operation: "least-squares solve",
                deficient_diagonal: 1,
                required_rank: 2,
            },
            LinalgError::SquareMatrixRequired {
                operand: "coefficient matrix",
                rows: 2,
                columns: 3,
            },
            LinalgError::ProviderFailure {
                provider: "test",
                operation: "linear solve",
                detail: "dimension mismatch text is non-structural".to_owned(),
            },
        ];
        for error in other_linalg_errors {
            let kind: RuntimeErrorKind = RuntimeLinalgError::Linalg(error).into();
            assert_eq!(runtime_category(&kind), "runtime.invalidState");
            assert_eq!(runtime_code(&kind), "OMR0013");
        }

        let array_shape_mismatch = RuntimeErrorKind::InvalidExecutionState {
            message: "array operation failed".to_owned(),
            array: Some(Box::new(ArrayRuntimeError::ShapeMismatch {
                operation: "addition",
                lhs: vec![1, 2],
                rhs: vec![2, 1],
            })),
        };
        assert_eq!(
            runtime_category(&array_shape_mismatch),
            "runtime.invalidState"
        );
        assert_eq!(runtime_code(&array_shape_mismatch), "OMR0013");

        let cancelled: RuntimeErrorKind = RuntimeLinalgError::Linalg(LinalgError::Cancelled {
            operation: "linear solve",
        })
        .into();
        assert_eq!(runtime_category(&cancelled), "runtime.cancelled");
        assert_eq!(runtime_code(&cancelled), "OMR0001");
    }

    #[test]
    fn builtin_type_category_preserves_type_boundary_without_reclassifying_domains() {
        let builtin = |category| RuntimeErrorKind::Builtin {
            name: "set".to_owned(),
            category,
            identifier: None,
            message: "test failure".to_owned(),
        };

        assert_eq!(
            runtime_category(&builtin(BuiltinErrorCategory::Type)),
            "runtime.invalidOperands"
        );
        for category in [
            BuiltinErrorCategory::Domain,
            BuiltinErrorCategory::Graphics,
            BuiltinErrorCategory::Other,
        ] {
            assert_eq!(runtime_category(&builtin(category)), "runtime.builtin");
        }
    }

    #[test]
    fn class_constraints_keep_r2022b_normalization_without_message_matching() {
        let constraint = |constraint| RuntimeErrorKind::ClassConstraint {
            operation: "test",
            constraint,
            message: "non-semantic probe text".to_owned(),
        };

        for kind in [
            ClassConstraintKind::AbstractClassInstantiation,
            ClassConstraintKind::SealedSuperclass,
            ClassConstraintKind::ExplicitConcreteWithAbstractMethods,
            ClassConstraintKind::AbstractOverrideAccessMismatch,
        ] {
            assert_eq!(
                runtime_category(&constraint(kind)),
                "runtime.invalidOperands"
            );
            assert_eq!(runtime_code(&constraint(kind)), "OMR0020");
        }
        let unresolved = constraint(ClassConstraintKind::AbstractMethodUnavailable);
        assert_eq!(runtime_category(&unresolved), "runtime.undefinedName");
        assert_eq!(runtime_code(&unresolved), "OMR0020");
    }

    #[test]
    fn aggregate_assignment_cardinality_error_uses_indexing_category() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        let error = execute(&mut engine, "cells = {1, 2}; cells(1:2) = {3, 4, 5};\n")
            .expect_err("aggregate assignment cardinality mismatch must fail");

        assert_eq!(error.category(), "runtime.indexOutOfBounds");
        assert_eq!(error.diagnostics()[0].code.as_deref(), Some("OMR0016"));
    }

    #[test]
    fn list_and_inspect_cover_column_major_arrays_specials_and_bounds() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        engine.interpreter.workspace_mut().insert(
            "real",
            Value::Array(ArrayData::F64(
                DenseArray::from_vec(
                    Shape::new([2, 3]).unwrap(),
                    vec![1.0, 2.0, f64::NAN, 4.0, 5.0, 6.0],
                )
                .unwrap(),
            )),
        );
        engine.interpreter.workspace_mut().insert(
            "complex",
            Value::Array(ArrayData::ComplexF64(
                DenseArray::from_vec(
                    Shape::new([1, 2]).unwrap(),
                    vec![Complex64::new(1.0, 2.0), Complex64::new(f64::INFINITY, 0.0)],
                )
                .unwrap(),
            )),
        );
        engine.interpreter.workspace_mut().insert(
            "logical",
            Value::Array(ArrayData::Logical(
                DenseArray::from_vec(
                    Shape::new([1, 2]).unwrap(),
                    vec![Logical::TRUE, Logical::FALSE],
                )
                .unwrap(),
            )),
        );
        engine
            .interpreter
            .workspace_mut()
            .insert("text", Value::from("hello"));

        let summaries = engine.list_workspace().unwrap();
        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary.name.as_str())
                .collect::<Vec<_>>(),
            ["complex", "logical", "real", "text"]
        );
        assert_eq!(summaries[2].bytes, Some(48));

        let preview = engine
            .inspect(&InspectRequest {
                name: "real".to_owned(),
                range: MatrixRange {
                    start: vec![2, 1],
                    size: vec![1, 3],
                },
                max_elements: 2,
            })
            .unwrap();
        assert_eq!(
            preview.values,
            vec![
                PreviewValue::Number { value: 2.0 },
                PreviewValue::Number { value: 4.0 }
            ]
        );
        assert_eq!(preview.truncation.omitted_elements, 1);

        let complex = engine
            .inspect(&InspectRequest {
                name: "complex".to_owned(),
                range: MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 2],
                },
                max_elements: 2,
            })
            .unwrap();
        assert_eq!(complex.values[1], PreviewValue::Missing);
        complex.validate().expect("protocol-valid complex preview");
        assert_eq!(
            engine.inspect(&scalar_inspect("text")).unwrap().values,
            vec![PreviewValue::Text {
                value: "hello".to_owned()
            }]
        );
    }

    #[test]
    fn single_summary_display_and_workspace_delta_preserve_binary32_truth() {
        let engine = RuntimeEngine::new().expect("runtime engine");
        let real = real_single_value([2, 2], vec![1.0, 2.0, 3.0, 4.0]);
        let summary = variable_summary("real", &real, real.class_name());
        assert_eq!(summary.class, "single");
        assert_eq!(summary.dimensions, vec![2, 2]);
        assert!(!summary.complex);
        assert_eq!(summary.bytes, Some(16));
        assert_eq!(
            format_display_value(&real, &engine.interpreter),
            "     1     3\n     2     4"
        );

        let complex = complex_single_value(
            [1, 2],
            vec![Complex32::new(1.0, -2.0), Complex32::new(3.0, 4.0)],
        );
        let summary = variable_summary("complex", &complex, complex.class_name());
        assert_eq!(summary.class, "single");
        assert_eq!(summary.dimensions, vec![1, 2]);
        assert!(summary.complex);
        assert_eq!(summary.bytes, Some(16));
        assert_eq!(
            format_display_value(&complex, &engine.interpreter),
            "    1 - 2i    3 + 4i"
        );

        let large = real_single_value([1, 500], vec![0.1; 500]);
        assert!(format_display_value(&large, &engine.interpreter).starts_with("single [1x500]"));
        let empty = real_single_value([0, 3], Vec::new());
        let empty_summary = variable_summary("empty", &empty, empty.class_name());
        assert_eq!(empty_summary.dimensions, vec![0, 3]);
        assert_eq!(empty_summary.bytes, Some(0));
        assert_eq!(format_display_value(&empty, &engine.interpreter), "[]");

        let stable_nan = real_single_value([1, 2], vec![f32::from_bits(0x7fc0_0001), -0.0]);
        let mut before = Workspace::new();
        before.insert("stable", stable_nan.clone());
        let mut after = Workspace::new();
        after.insert("stable", stable_nan);
        let delta = workspace_delta(&before, &after, &engine.interpreter);
        assert!(delta.added.is_empty());
        assert!(delta.changed.is_empty());
        assert!(delta.removed.is_empty());

        after.insert(
            "stable",
            real_single_value([1, 2], vec![f32::from_bits(0x7fc0_0001), 0.0]),
        );
        let delta = workspace_delta(&before, &after, &engine.interpreter);
        assert_eq!(delta.changed.len(), 1);
        assert_eq!(delta.changed[0].class, "single");
        assert_eq!(delta.changed[0].dimensions, vec![1, 2]);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn single_previews_and_protocol_roundtrips_preserve_exact_storage() {
        let real = real_single_value(
            [1, 4],
            vec![0.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
        );
        let v0 = inspect_value(&real, "single", &row_inspect("value", 4))
            .expect("kernel-v0 single preview");
        assert_eq!(v0.class, "single");
        assert_eq!(v0.dimensions, vec![1, 4]);
        assert_eq!(
            v0.values,
            vec![
                PreviewValue::Number {
                    value: f64::from(0.1_f32),
                },
                PreviewValue::Special {
                    value: "nan".to_owned(),
                },
                PreviewValue::Special {
                    value: "infinity".to_owned(),
                },
                PreviewValue::Special {
                    value: "negativeInfinity".to_owned(),
                },
            ]
        );
        v0.validate().expect("protocol-valid kernel-v0 preview");

        let limits = kernel_v1::PreviewLimits::default();
        let v1_request = v1_row_inspect(4, 4);
        let v1 = inspect_value_v1(&real, "single", &v1_request, &limits)
            .expect("kernel-v1 single preview");
        assert_eq!(v1.class, "single");
        assert!(!v1.complex);
        assert_eq!(
            v1.values,
            vec![
                kernel_v1::PreviewValue::Number {
                    value: f64::from(0.1_f32),
                },
                kernel_v1::PreviewValue::Special {
                    value: "nan".to_owned(),
                },
                kernel_v1::PreviewValue::Special {
                    value: "infinity".to_owned(),
                },
                kernel_v1::PreviewValue::Special {
                    value: "negativeInfinity".to_owned(),
                },
            ]
        );

        let request = kernel_v1::RequestEnvelope::new(
            "single-session",
            "single-v1-request",
            kernel_v1::Request::Inspect(v1_request),
        );
        let response = kernel_v1::ResponseEnvelope::success(
            &request,
            "single-v1-response",
            kernel_v1::ResponseResult::Inspect(v1.clone()),
        );
        let encoded = kernel_v1::encode_response_for_request(&response, &request, &limits)
            .expect("encode single v1 response");
        let decoded = kernel_v1::decode_response_for_request(&encoded, &request, &limits)
            .expect("decode single v1 response");
        assert_eq!(decoded, response);

        let empty = real_single_value([0, 3], Vec::new());
        let empty_request = kernel_v1::InspectRequest {
            name: "empty".to_owned(),
            range: kernel_v1::MatrixRange {
                start: vec![1, 1],
                size: vec![0, 3],
            },
            max_elements: 1,
        };
        let empty_preview = inspect_value_v1(&empty, "single", &empty_request, &limits)
            .expect("empty single preview");
        assert_eq!(empty_preview.dimensions, vec![0, 3]);
        assert!(empty_preview.values.is_empty());

        let empty_complex = complex_single_value([0, 3], Vec::new());
        let empty_complex_preview =
            inspect_value_v1(&empty_complex, "single", &empty_request, &limits)
                .expect("empty complex single remains complex in v1");
        assert!(empty_complex_preview.complex);
        assert!(empty_complex_preview.values.is_empty());

        let finite_complex = complex_single_value(
            [1, 2],
            vec![Complex32::new(1.0, -2.0), Complex32::new(0.1, 0.0)],
        );
        let complex_v1 =
            inspect_value_v1(&finite_complex, "single", &v1_row_inspect(2, 2), &limits)
                .expect("finite complex single preview");
        assert!(complex_v1.complex);
        assert_eq!(
            complex_v1.values,
            vec![
                kernel_v1::PreviewValue::Complex {
                    real: 1.0,
                    imaginary: -2.0,
                },
                kernel_v1::PreviewValue::Complex {
                    real: f64::from(0.1_f32),
                    imaginary: 0.0,
                },
            ]
        );

        let nonfinite_complex =
            complex_single_value([1, 1], vec![Complex32::new(f32::NAN, f32::INFINITY)]);
        assert_eq!(
            inspect_value_v1(&nonfinite_complex, "single", &v1_row_inspect(1, 1), &limits,)
                .expect_err("v1 cannot encode non-finite complex components")
                .category(),
            "workspace.unsupportedValue"
        );
        assert_eq!(
            inspect_value(&nonfinite_complex, "single", &scalar_inspect("nonfinite"),)
                .expect("v0 degrades non-finite complex to missing")
                .values,
            vec![PreviewValue::Missing]
        );

        let aggregate = Value::Cell(
            CellArray::from_values(Shape::new([1, 2]).unwrap(), vec![real, finite_complex])
                .unwrap(),
        );
        let aggregate_limits = kernel_v2::AggregateLimits::default();
        let v2_request = v2_inspect([1, 2], 2);
        let v2 = inspect_value_v2(&aggregate, "cell", &v2_request, &aggregate_limits)
            .expect("v2 exact single leaves");
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            items, ..
        }) = &v2
        else {
            panic!("cell aggregate preview");
        };
        assert_eq!(items[0].class, "single");
        assert_eq!(items[0].size, vec![1, 4]);
        assert!(!items[0].complex);
        assert_eq!(
            items[0].payload,
            kernel_v2::ExactPayload::Numeric {
                real: vec![
                    "0.1".to_owned(),
                    "NaN".to_owned(),
                    "+Inf".to_owned(),
                    "-Inf".to_owned(),
                ],
                imag: vec!["0".to_owned(); 4],
            }
        );
        assert_eq!(items[1].class, "single");
        assert!(items[1].complex);
        assert_eq!(
            items[1].payload,
            kernel_v2::ExactPayload::Numeric {
                real: vec!["1".to_owned(), "0.1".to_owned()],
                imag: vec!["-2".to_owned(), "0".to_owned()],
            }
        );

        let request = kernel_v2::RequestEnvelope::new(
            "single-session",
            "single-v2-request",
            kernel_v2::Request::Inspect(v2_request),
        );
        let response = kernel_v2::ResponseEnvelope::success(
            &request,
            "single-v2-response",
            kernel_v2::ResponseResult::Inspect(v2),
        );
        let encoded =
            kernel_v2::encode_response_for_request(&response, &request, &aggregate_limits)
                .expect("encode exact single v2 response");
        let decoded = kernel_v2::decode_response_for_request(&encoded, &request, &aggregate_limits)
            .expect("decode exact single v2 response");
        assert_eq!(decoded, response);

        for unrepresentable in [
            empty_complex,
            complex_single_value([1, 1], vec![Complex32::new(1.0, -0.0)]),
        ] {
            let aggregate = Value::Cell(
                CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![unrepresentable]).unwrap(),
            );
            assert_eq!(
                inspect_value_v2(
                    &aggregate,
                    "cell",
                    &v2_inspect([1, 1], 1),
                    &aggregate_limits,
                )
                .expect_err("v2 cannot preserve complex storage with only zero imaginary parts")
                .category(),
                "workspace.unsupportedValue"
            );
        }
    }

    #[test]
    fn v1_char_and_string_previews_preserve_utf16_empty_and_missing() {
        let char_value = Value::Array(ArrayData::Char(
            DenseArray::from_vec(
                Shape::new([1, 3]).unwrap(),
                vec![
                    CharCodeUnit::new(65),
                    CharCodeUnit::new(0xD83D),
                    CharCodeUnit::new(0xDE42),
                ],
            )
            .unwrap(),
        ));
        let char_preview = inspect_value_v1(
            &char_value,
            "char",
            &v1_row_inspect(3, 3),
            &kernel_v1::PreviewLimits::default(),
        )
        .expect("exact char preview");
        assert_eq!(char_preview.dimensions, vec![1, 3]);
        assert!(!char_preview.complex);
        assert_eq!(
            char_preview.values,
            vec![
                kernel_v1::PreviewValue::CharCodeUnit { value: 65 },
                kernel_v1::PreviewValue::CharCodeUnit { value: 0xD83D },
                kernel_v1::PreviewValue::CharCodeUnit { value: 0xDE42 },
            ]
        );

        let empty_char = Value::Array(ArrayData::Char(
            DenseArray::from_vec(Shape::new([0, 0]).unwrap(), Vec::new()).unwrap(),
        ));
        let empty_request = kernel_v1::InspectRequest {
            name: "empty".to_owned(),
            range: kernel_v1::MatrixRange {
                start: vec![1, 1],
                size: vec![0, 0],
            },
            max_elements: 1,
        };
        let empty_preview = inspect_value_v1(
            &empty_char,
            "char",
            &empty_request,
            &kernel_v1::PreviewLimits::default(),
        )
        .expect("empty char preview");
        assert_eq!(empty_preview.dimensions, vec![0, 0]);
        assert!(empty_preview.values.is_empty());
        assert!(!empty_preview.truncation.truncated);

        let strings = Value::from(
            StringArray::from_elements(
                Shape::new([1, 3]).unwrap(),
                vec![
                    StringElement::from_code_units(vec![0xD83D]),
                    StringElement::from_code_units(Vec::<u16>::new()),
                    StringElement::missing(),
                ],
            )
            .unwrap(),
        );
        let string_preview = inspect_value_v1(
            &strings,
            "string",
            &v1_row_inspect(3, 3),
            &kernel_v1::PreviewLimits::default(),
        )
        .expect("exact string preview");
        assert_eq!(
            string_preview.values,
            vec![
                kernel_v1::PreviewValue::String {
                    code_units: vec![0xD83D],
                    missing: false,
                },
                kernel_v1::PreviewValue::String {
                    code_units: Vec::new(),
                    missing: false,
                },
                kernel_v1::PreviewValue::String {
                    code_units: Vec::new(),
                    missing: true,
                },
            ]
        );
        string_preview
            .validate(&kernel_v1::PreviewLimits::default())
            .expect("protocol-valid exact string preview");
    }

    #[test]
    fn every_integer_storage_variant_has_an_exact_v1_preview() {
        assert_integer_preview(i8::MIN, "int8", "-128", "0", false);
        assert_integer_preview(ComplexInteger::new(-7_i8, 2_i8), "int8", "-7", "2", true);
        assert_integer_preview(u8::MAX, "uint8", "255", "0", false);
        assert_integer_preview(ComplexInteger::new(7_u8, 2_u8), "uint8", "7", "2", true);
        assert_integer_preview(i16::MIN, "int16", "-32768", "0", false);
        assert_integer_preview(ComplexInteger::new(-7_i16, 2_i16), "int16", "-7", "2", true);
        assert_integer_preview(u16::MAX, "uint16", "65535", "0", false);
        assert_integer_preview(ComplexInteger::new(7_u16, 2_u16), "uint16", "7", "2", true);
        assert_integer_preview(i32::MIN, "int32", "-2147483648", "0", false);
        assert_integer_preview(ComplexInteger::new(-7_i32, 2_i32), "int32", "-7", "2", true);
        assert_integer_preview(u32::MAX, "uint32", "4294967295", "0", false);
        assert_integer_preview(ComplexInteger::new(7_u32, 2_u32), "uint32", "7", "2", true);
        assert_integer_preview(i64::MIN, "int64", "-9223372036854775808", "0", false);
        assert_integer_preview(ComplexInteger::new(-7_i64, 2_i64), "int64", "-7", "2", true);
        assert_integer_preview(u64::MAX, "uint64", "18446744073709551615", "0", false);
        assert_integer_preview(ComplexInteger::new(7_u64, 2_u64), "uint64", "7", "2", true);

        let complex = integer_value(
            [1, 2],
            vec![ComplexInteger::new(i64::MIN, 0), ComplexInteger::new(7, -9)],
        );
        let preview = inspect_value_v1(
            &complex,
            "int64",
            &v1_row_inspect(2, 2),
            &kernel_v1::PreviewLimits::default(),
        )
        .expect("exact complex integer boundaries");
        assert_eq!(
            preview.values,
            vec![
                kernel_v1::PreviewValue::Integer {
                    real: "-9223372036854775808".to_owned(),
                    imaginary: "0".to_owned(),
                },
                kernel_v1::PreviewValue::Integer {
                    real: "7".to_owned(),
                    imaginary: "-9".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn integer_decimal_bridge_preserves_internal_128_bit_boundaries() {
        assert_eq!(
            integer_component_decimal(IntegerComponent::Signed(i128::MIN)),
            "-170141183460469231731687303715884105728"
        );
        assert_eq!(
            integer_component_decimal(IntegerComponent::Unsigned(u128::MAX)),
            "340282366920938463463374607431768211455"
        );
    }

    #[test]
    fn v1_preview_truncates_whole_elements_and_rejects_oversized_strings() {
        let strings = Value::from(
            StringArray::from_elements(
                Shape::new([1, 3]).unwrap(),
                vec![
                    StringElement::from_code_units(vec![1, 2]),
                    StringElement::from_code_units(vec![3, 4]),
                    StringElement::missing(),
                ],
            )
            .unwrap(),
        );
        let limits = kernel_v1::PreviewLimits::new(3, 2, 3).unwrap();
        let preview = inspect_value_v1(&strings, "string", &v1_row_inspect(3, 3), &limits).unwrap();
        assert_eq!(preview.values.len(), 1);
        assert_eq!(preview.truncation.omitted_elements, 2);
        assert!(preview.truncation.truncated);
        preview.validate(&limits).expect("whole-element prefix");

        let oversized = Value::from(StringElement::from_code_units(vec![1, 2, 3]));
        let error = inspect_value_v1(&oversized, "string", &v1_row_inspect(1, 1), &limits)
            .expect_err("per-element overflow is unsupported");
        assert_eq!(error.category(), "workspace.unsupportedValue");

        let chars = Value::Array(ArrayData::Char(
            DenseArray::from_vec(
                Shape::new([1, 3]).unwrap(),
                vec![
                    CharCodeUnit::new(1),
                    CharCodeUnit::new(2),
                    CharCodeUnit::new(3),
                ],
            )
            .unwrap(),
        ));
        let preview = inspect_value_v1(
            &chars,
            "char",
            &v1_row_inspect(3, 2),
            &kernel_v1::PreviewLimits::default(),
        )
        .unwrap();
        assert_eq!(preview.values.len(), 2);
        assert_eq!(preview.truncation.omitted_elements, 1);
    }

    #[test]
    fn v1_nonfinite_complex_selection_fails_even_beyond_element_prefix() {
        let value = Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                vec![Complex64::new(1.0, 0.0), Complex64::new(f64::NAN, 2.0)],
            )
            .unwrap(),
        ));
        let error = inspect_value_v1(
            &value,
            "double",
            &v1_row_inspect(2, 1),
            &kernel_v1::PreviewLimits::default(),
        )
        .expect_err("non-finite selected component must not be hidden by truncation");
        assert_eq!(error.category(), "workspace.unsupportedValue");
    }

    #[test]
    fn v0_downgrade_is_exact_or_structured_unsupported() {
        let char_value = Value::Array(ArrayData::Char(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![CharCodeUnit::new(65)]).unwrap(),
        ));
        assert_eq!(
            inspect_value(&char_value, "char", &scalar_inspect("value"))
                .expect_err("v0 char is unsupported")
                .category(),
            "workspace.unsupportedValue"
        );

        let integer = integer_value([1, 1], vec![u64::MAX]);
        assert_eq!(
            inspect_value(&integer, "uint64", &scalar_inspect("value"))
                .expect_err("v0 integer is unsupported")
                .category(),
            "workspace.unsupportedValue"
        );

        let valid = Value::from(StringElement::from_code_units(vec![0xD83D, 0xDE42]));
        assert_eq!(
            inspect_value(&valid, "string", &scalar_inspect("value"))
                .unwrap()
                .values,
            vec![PreviewValue::Text {
                value: "🙂".to_owned(),
            }]
        );

        let isolated = Value::from(StringElement::from_code_units(vec![0xD83D]));
        assert_eq!(
            inspect_value(&isolated, "string", &scalar_inspect("value"))
                .expect_err("isolated surrogate is not scalar text")
                .category(),
            "workspace.unsupportedValue"
        );

        let mixed = Value::from(
            StringArray::from_elements(
                Shape::new([1, 2]).unwrap(),
                vec![StringElement::from_utf8("ok"), StringElement::missing()],
            )
            .unwrap(),
        );
        let request = InspectRequest {
            name: "value".to_owned(),
            range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 2],
            },
            max_elements: 1,
        };
        assert_eq!(
            inspect_value(&mixed, "string", &request)
                .expect_err("missing after the returned prefix still forbids v0 downgrade")
                .category(),
            "workspace.unsupportedValue"
        );
    }

    #[test]
    fn aggregate_summaries_are_exact_while_v0_and_v1_previews_are_unsupported() {
        let cell =
            Value::Cell(CellArray::filled_empty_double(Shape::new([1, 2]).unwrap()).unwrap());
        let structure =
            Value::Struct(StructArray::empty(Shape::new([0, 3]).unwrap(), Vec::new()).unwrap());

        let cell_summary = variable_summary("c", &cell, "cell");
        assert_eq!(cell_summary.dimensions, vec![1, 2]);
        assert_eq!(cell_summary.bytes, None);
        let struct_summary = variable_summary("s", &structure, "struct");
        assert_eq!(struct_summary.dimensions, vec![0, 3]);
        assert_eq!(struct_summary.bytes, None);

        assert_eq!(
            inspect_value(
                &cell,
                "cell",
                &InspectRequest {
                    name: "c".to_owned(),
                    range: MatrixRange {
                        start: vec![1, 1],
                        size: vec![1, 2],
                    },
                    max_elements: 2,
                },
            )
            .expect_err("kernel-v0 cannot encode cell values")
            .category(),
            "workspace.unsupportedValue"
        );
        assert_eq!(
            inspect_value_v1(
                &cell,
                "cell",
                &v1_row_inspect(2, 2),
                &kernel_v1::PreviewLimits::default(),
            )
            .expect_err("kernel-v1 cannot encode cell values")
            .category(),
            "workspace.unsupportedValue"
        );
    }

    #[test]
    fn v2_real_source_cell_observation_preserves_column_major_order() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(&mut engine, "value = {11, 22; 33, 44};\n")
            .expect("cell construction executes through the source pipeline");

        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            dimensions,
            items,
            truncation,
            ..
        }) = engine
            .inspect_v2(
                &v2_inspect([2, 2], 4),
                &kernel_v2::AggregateLimits::default(),
            )
            .expect("v2 cell observation")
        else {
            panic!("cell aggregate preview")
        };

        assert_eq!(dimensions, vec![2, 2]);
        assert!(!truncation.truncated);
        assert_eq!(
            items
                .iter()
                .map(|item| match &item.payload {
                    kernel_v2::ExactPayload::Numeric { real, .. } => real[0].as_str(),
                    _ => panic!("numeric cell item"),
                })
                .collect::<Vec<_>>(),
            ["11", "33", "22", "44"]
        );
    }

    #[test]
    fn v2_nested_cell_and_ordered_shaped_empty_struct_match_canonical_usage() {
        let shaped_empty = Value::Struct(
            StructArray::empty(
                Shape::new([0, 3]).unwrap(),
                vec![
                    FieldName::new("beta").unwrap(),
                    FieldName::new("alpha").unwrap(),
                ],
            )
            .unwrap(),
        );
        let value = Value::Cell(
            CellArray::from_values(
                Shape::new([1, 2]).unwrap(),
                vec![Value::Double(7.0), shaped_empty],
            )
            .unwrap(),
        );
        let limits = kernel_v2::AggregateLimits::default();
        let preview = inspect_value_v2(&value, "cell", &v2_inspect([1, 2], 2), &limits)
            .expect("nested aggregate preview");
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            items,
            usage,
            truncation,
            ..
        }) = &preview
        else {
            panic!("cell aggregate preview")
        };
        assert_eq!(
            *usage,
            kernel_v2::PreviewUsage {
                nodes: 3,
                elements: 3,
                code_units: 9,
                depth: 1,
            }
        );
        assert!(!truncation.truncated);
        assert!(matches!(
            &items[0].payload,
            kernel_v2::ExactPayload::Numeric { real, imag }
                if real == &["7"] && imag == &["0"]
        ));
        assert!(matches!(
            &items[1].payload,
            kernel_v2::ExactPayload::Struct { fields, records }
                if fields == &["beta", "alpha"] && records.is_empty()
        ));
        preview
            .validate_for_request(&limits, 2)
            .expect("protocol-valid nested preview");

        let matrix = Value::Cell(
            CellArray::from_values(
                Shape::new([2, 2]).unwrap(),
                [11.0, 33.0, 22.0, 44.0]
                    .into_iter()
                    .map(Value::Double)
                    .collect(),
            )
            .unwrap(),
        );
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            items, ..
        }) = inspect_value_v2(&matrix, "cell", &v2_inspect([2, 2], 4), &limits).unwrap()
        else {
            panic!("column-major cell preview")
        };
        assert_eq!(
            items
                .iter()
                .map(|item| match &item.payload {
                    kernel_v2::ExactPayload::Numeric { real, .. } => real[0].as_str(),
                    _ => panic!("numeric cell item"),
                })
                .collect::<Vec<_>>(),
            ["11", "33", "22", "44"]
        );
    }

    #[test]
    fn v2_exact_values_preserve_table_schema_and_complete_variables() {
        let column = |values: Vec<f64>| {
            Value::Array(ArrayData::F64(
                DenseArray::from_vec(Shape::new([2, 1]).unwrap(), values).unwrap(),
            ))
        };
        let table = Value::Table(
            TableArray::from_variables(
                vec![
                    TableVariableName::new("A").unwrap(),
                    TableVariableName::new("B").unwrap(),
                ],
                vec![column(vec![1.0, 2.0]), column(vec![3.0, 4.0])],
            )
            .unwrap(),
        );
        let wrapped =
            Value::Cell(CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![table]).unwrap());
        let limits = kernel_v2::AggregateLimits::default();
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            items, ..
        }) = inspect_value_v2(&wrapped, "cell", &v2_inspect([1, 1], 1), &limits).unwrap()
        else {
            panic!("cell aggregate preview")
        };
        let kernel_v2::ExactPayload::Table {
            variable_names,
            variables,
        } = &items[0].payload
        else {
            panic!("exact table payload")
        };
        assert_eq!(items[0].class, "table");
        assert_eq!(items[0].size, [2, 2]);
        assert_eq!(variable_names, &["A", "B"]);
        assert!(matches!(
            &variables[0].payload,
            kernel_v2::ExactPayload::Numeric { real, imag }
                if real == &["1", "2"] && imag == &["0", "0"]
        ));
        assert!(matches!(
            &variables[1].payload,
            kernel_v2::ExactPayload::Numeric { real, imag }
                if real == &["3", "4"] && imag == &["0", "0"]
        ));
    }

    #[test]
    fn v2_table_preview_pages_rows_and_truncates_only_complete_variables() {
        let matrix = Value::Array(ArrayData::F64(
            DenseArray::from_vec(
                Shape::new([4, 2]).unwrap(),
                vec![10.0, 20.0, 30.0, 40.0, 100.0, 200.0, 300.0, 400.0],
            )
            .unwrap(),
        ));
        let tail = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([4, 1]).unwrap(), vec![7.0, 8.0, 9.0, 10.0]).unwrap(),
        ));
        let table = Value::Table(
            TableArray::from_variables(
                vec![
                    TableVariableName::new("Skip").unwrap(),
                    TableVariableName::new("Matrix").unwrap(),
                    TableVariableName::new("Tail").unwrap(),
                ],
                vec![tail.clone(), matrix.clone(), tail],
            )
            .unwrap(),
        );
        let original = table.clone();
        let request = kernel_v2::InspectRequest {
            name: "value".to_owned(),
            range: kernel_v2::MatrixRange {
                start: vec![2, 2],
                size: vec![2, 2],
            },
            max_elements: 1,
        };
        let limits = kernel_v2::AggregateLimits::default();
        let preview = inspect_value_v2(&table, "table", &request, &limits).unwrap();
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Table {
            dimensions,
            selected_range,
            variable_names,
            variables,
            truncation,
            usage,
        }) = &preview
        else {
            panic!("table aggregate preview")
        };
        assert_eq!(dimensions, &[4, 3]);
        assert_eq!(selected_range, &request.range);
        assert_eq!(variable_names, &["Matrix"]);
        assert_eq!(variables.len(), 1);
        assert_eq!(variables[0].size, [2, 2]);
        assert!(matches!(
            &variables[0].payload,
            kernel_v2::ExactPayload::Numeric { real, imag }
                if real == &["20", "30", "200", "300"]
                    && imag == &["0", "0", "0", "0"]
        ));
        assert_eq!(truncation.omitted_elements, 1);
        assert!(truncation.truncated);
        assert_eq!(
            *usage,
            kernel_v2::PreviewUsage {
                nodes: 2,
                elements: 5,
                code_units: 6,
                depth: 1,
            }
        );
        assert_eq!(
            table, original,
            "inspect must not detach or mutate the source"
        );
        preview
            .validate_for_request(&limits, request.max_elements)
            .expect("protocol-valid table page");

        let Value::Table(table) = &table else {
            unreachable!()
        };
        let Value::Array(ArrayData::F64(source)) = table.variable(1).unwrap() else {
            unreachable!()
        };
        let Value::Array(ArrayData::F64(full_slice)) =
            slice_table_variable_rows(table.variable(1).unwrap(), 1, 4).unwrap()
        else {
            unreachable!()
        };
        assert!(source.shares_storage_with(&full_slice));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v2_table_row_slices_preserve_every_exact_supported_variable_kind() {
        let rows = Shape::new([3, 1]).unwrap();
        let double = Value::Array(ArrayData::F64(
            DenseArray::from_vec(rows.clone(), vec![1.0, 2.0, 3.0]).unwrap(),
        ));
        let single = Value::Array(ArrayData::F32(
            DenseArray::from_vec(rows.clone(), vec![1.0, 2.0, 3.0]).unwrap(),
        ));
        let complex = Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                rows.clone(),
                vec![
                    Complex64::new(1.0, 1.0),
                    Complex64::new(2.0, 2.0),
                    Complex64::new(3.0, 3.0),
                ],
            )
            .unwrap(),
        ));
        let logical = Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                rows.clone(),
                vec![
                    Logical::from(true),
                    Logical::from(false),
                    Logical::from(true),
                ],
            )
            .unwrap(),
        ));
        let integer = integer_value([3, 1], vec![10_i16, 20_i16, 30_i16]);
        let char_matrix = Value::Array(ArrayData::Char(
            DenseArray::from_vec(
                Shape::new([3, 2]).unwrap(),
                [65_u16, 66, 67, 68, 69, 70]
                    .into_iter()
                    .map(CharCodeUnit::new)
                    .collect(),
            )
            .unwrap(),
        ));
        let strings = Value::from(
            StringArray::from_elements(
                rows.clone(),
                vec![
                    StringElement::from_utf8("one"),
                    StringElement::missing(),
                    StringElement::from_utf8("three"),
                ],
            )
            .unwrap(),
        );
        let cell = Value::Cell(
            CellArray::from_values(
                rows.clone(),
                vec![Value::Double(1.0), Value::Double(2.0), Value::Double(3.0)],
            )
            .unwrap(),
        );
        let structure = Value::Struct(
            StructArray::from_columns(
                rows.clone(),
                vec![FieldName::new("A").unwrap()],
                vec![vec![
                    Value::Double(4.0),
                    Value::Double(5.0),
                    Value::Double(6.0),
                ]],
            )
            .unwrap(),
        );
        let nested = Value::Table(
            TableArray::from_variables(
                vec![TableVariableName::new("Inner").unwrap()],
                vec![Value::Array(ArrayData::F64(
                    DenseArray::from_vec(rows, vec![7.0, 8.0, 9.0]).unwrap(),
                ))],
            )
            .unwrap(),
        );
        let names = [
            "Double", "Single", "Complex", "Logical", "Integer", "Char", "String", "Cell",
            "Struct", "Table",
        ]
        .into_iter()
        .map(|name| TableVariableName::new(name).unwrap())
        .collect();
        let value = Value::Table(
            TableArray::from_variables(
                names,
                vec![
                    double,
                    single,
                    complex,
                    logical,
                    integer,
                    char_matrix,
                    strings,
                    cell,
                    structure,
                    nested,
                ],
            )
            .unwrap(),
        );
        let request = kernel_v2::InspectRequest {
            name: "value".to_owned(),
            range: kernel_v2::MatrixRange {
                start: vec![2, 1],
                size: vec![2, 10],
            },
            max_elements: 10,
        };
        let limits = kernel_v2::AggregateLimits::default();
        let preview = inspect_value_v2(&value, "table", &request, &limits).unwrap();
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Table {
            variable_names,
            variables,
            truncation,
            ..
        }) = &preview
        else {
            panic!("table aggregate preview")
        };
        assert_eq!(variable_names.len(), 10);
        assert!(!truncation.truncated);
        assert_eq!(
            variables
                .iter()
                .map(|value| (value.class.as_str(), value.size.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("double", vec![2, 1]),
                ("single", vec![2, 1]),
                ("double", vec![2, 1]),
                ("logical", vec![2, 1]),
                ("int16", vec![2, 1]),
                ("char", vec![2, 2]),
                ("string", vec![2, 1]),
                ("cell", vec![2, 1]),
                ("struct", vec![2, 1]),
                ("table", vec![2, 1]),
            ]
        );
        assert!(variables[2].complex);
        assert!(matches!(
            &variables[5].payload,
            kernel_v2::ExactPayload::Char { code_units }
                if code_units == &[66, 67, 69, 70]
        ));
        assert!(matches!(
            &variables[9].payload,
            kernel_v2::ExactPayload::Table { variable_names, variables }
                if variable_names == &["Inner"] && variables[0].size == [2, 1]
        ));
        preview
            .validate_for_request(&limits, request.max_elements)
            .expect("all exact variable kinds remain protocol valid");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v2_table_empty_width_scalar_rows_and_budget_boundaries_are_exact() {
        let empty = Value::Table(TableArray::empty(3).unwrap());
        let empty_request = kernel_v2::InspectRequest {
            name: "empty".to_owned(),
            range: kernel_v2::MatrixRange {
                start: vec![2, 1],
                size: vec![2, 0],
            },
            max_elements: 1,
        };
        let limits = kernel_v2::AggregateLimits::default();
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Table {
            dimensions,
            variable_names,
            variables,
            truncation,
            usage,
            ..
        }) = inspect_value_v2(&empty, "table", &empty_request, &limits).unwrap()
        else {
            panic!("empty-width table preview")
        };
        assert_eq!(dimensions, [3, 0]);
        assert!(variable_names.is_empty());
        assert!(variables.is_empty());
        assert!(!truncation.truncated);
        assert_eq!(
            usage,
            kernel_v2::PreviewUsage {
                nodes: 1,
                elements: 0,
                code_units: 0,
                depth: 0,
            }
        );

        let scalar = Value::Table(
            TableArray::from_variables(
                vec![TableVariableName::new("A").unwrap()],
                vec![Value::Double(42.0)],
            )
            .unwrap(),
        );
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Table {
            variables,
            ..
        }) = inspect_value_v2(&scalar, "table", &v2_inspect([1, 1], 1), &limits).unwrap()
        else {
            panic!("scalar table variable preview")
        };
        assert_eq!(variables[0].size, [1, 1]);
        assert!(matches!(
            &variables[0].payload,
            kernel_v2::ExactPayload::Numeric { real, .. } if real == &["42"]
        ));

        let matrix = || {
            Value::Array(ArrayData::F64(
                DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![1.0, 2.0, 3.0, 4.0])
                    .unwrap(),
            ))
        };
        let bounded = Value::Table(
            TableArray::from_variables(
                vec![
                    TableVariableName::new("A").unwrap(),
                    TableVariableName::new("B").unwrap(),
                ],
                vec![matrix(), matrix()],
            )
            .unwrap(),
        );
        let bounded_limits = kernel_v2::AggregateLimits::new(4, 16, 16, 8, 5, 8).unwrap();
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Table {
            variable_names,
            variables,
            truncation,
            usage,
            ..
        }) = inspect_value_v2(&bounded, "table", &v2_inspect([2, 2], 2), &bounded_limits).unwrap()
        else {
            panic!("budgeted table preview")
        };
        assert_eq!(variable_names, ["A"]);
        assert_eq!(variables.len(), 1);
        assert_eq!(truncation.omitted_elements, 1);
        assert_eq!(usage.elements, 5);

        let per_node_limits = kernel_v2::AggregateLimits::new(1, 16, 16, 8, 8, 8).unwrap();
        assert_eq!(
            inspect_value_v2(
                &Value::Table(
                    TableArray::from_variables(
                        vec![TableVariableName::new("Wide").unwrap()],
                        vec![Value::Array(ArrayData::F64(
                            DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![1.0, 2.0])
                                .unwrap(),
                        ))],
                    )
                    .unwrap(),
                ),
                "table",
                &v2_inspect([1, 1], 1),
                &per_node_limits,
            )
            .expect_err("an indivisible oversized variable is not silently omitted")
            .category(),
            "workspace.previewLimit"
        );
    }

    #[test]
    fn v2_table_rejects_identity_and_callable_variable_rows_explicitly() {
        let unsupported = [
            Value::Object(ObjectHandle::new(7)),
            Value::Graphics(
                GraphicsHandle::new(1, 1, GraphicsClass::Figure).expect("graphics handle"),
            ),
            Value::Function(FunctionHandle::Builtin(BuiltinHandle::new(9))),
        ];
        for value in unsupported {
            assert_eq!(
                slice_table_variable_rows(&value, 1, 1)
                    .expect_err("identity and callable rows have no exact table slice")
                    .category(),
                "workspace.unsupportedValue"
            );
        }

        let table = Value::Table(
            TableArray::from_variables(
                vec![
                    TableVariableName::new("Good").unwrap(),
                    TableVariableName::new("Object").unwrap(),
                ],
                vec![Value::Logical(true), Value::Object(ObjectHandle::new(11))],
            )
            .unwrap(),
        );
        assert_eq!(
            inspect_value_v2(
                &table,
                "table",
                &v2_inspect([1, 2], 2),
                &kernel_v2::AggregateLimits::default(),
            )
            .expect_err("unsupported later table node must discard the earlier candidate")
            .category(),
            "workspace.unsupportedValue"
        );
    }

    #[test]
    fn v2_real_source_table_inspect_slices_rows_through_registered_builtin() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "value = table([1;2;3], [10 20;30 40;50 60], 'VariableNames', {'A','B'});\n",
        )
        .expect("table source executes through the registered builtin");
        let request = kernel_v2::InspectRequest {
            name: "value".to_owned(),
            range: kernel_v2::MatrixRange {
                start: vec![2, 1],
                size: vec![2, 2],
            },
            max_elements: 2,
        };
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Table {
            variable_names,
            variables,
            truncation,
            ..
        }) = engine
            .inspect_v2(&request, &kernel_v2::AggregateLimits::default())
            .expect("real RuntimeEngine table inspect")
        else {
            panic!("table aggregate preview")
        };
        assert_eq!(variable_names, ["A", "B"]);
        assert!(!truncation.truncated);
        assert!(matches!(
            &variables[0].payload,
            kernel_v2::ExactPayload::Numeric { real, .. } if real == &["2", "3"]
        ));
        assert!(matches!(
            &variables[1].payload,
            kernel_v2::ExactPayload::Numeric { real, .. }
                if real == &["30", "50", "40", "60"]
        ));
    }

    #[test]
    fn real_source_table_workflow_infers_names_filters_and_mutates_rows() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "Name={'Alice';'Bob';'Carol';'David';'Eve'};\
             Age=[23;31;27;35;29]; Score=[88;91;79;95;93];\
             T=table(Name,Age,Score);\
             Filtered=T(T.Age>=25 & T.Score>=90,:);\
             T.Score(3)=85;\
             T(end+1,:)={'Frank',40,96};\
             T(2,:)=[];\
             R=table([1;2;3],[10;20;30],'VariableNames',{'ID','Value'},\
                     'RowNames',{'one','two','three'});\
             Picked=R({'two','three'},:);\
             L=table([1;2],'VariableNames',{'ID'});\
             V=[L;table([3;4],'VariableNames',{'ID'})];\
             H=[L,table([10;20],'VariableNames',{'Value'})];\n",
        )
        .expect("table workflow executes through source");
        let workspace = engine.interpreter.workspace();

        let Value::Table(table) = workspace.get("T").unwrap() else {
            panic!("T must remain a table")
        };
        assert_eq!(
            table
                .variable_names()
                .iter()
                .map(TableVariableName::as_str)
                .collect::<Vec<_>>(),
            ["Name", "Age", "Score"]
        );
        assert_eq!(table.row_count(), 5);
        let Value::Array(ArrayData::F64(scores)) = table.variable_by_name("Score").unwrap() else {
            panic!("Score must remain double")
        };
        assert_eq!(scores.as_slice(), &[88.0, 85.0, 95.0, 93.0, 96.0]);

        let Value::Table(filtered) = workspace.get("Filtered").unwrap() else {
            panic!("logical filtering must return a table")
        };
        assert_eq!(filtered.row_count(), 3);
        let Value::Array(ArrayData::F64(scores)) = filtered.variable_by_name("Score").unwrap()
        else {
            panic!("filtered Score must remain double")
        };
        assert_eq!(scores.as_slice(), &[91.0, 95.0, 93.0]);

        let Value::Table(picked) = workspace.get("Picked").unwrap() else {
            panic!("row-name selection must return a table")
        };
        assert_eq!(
            picked
                .row_names()
                .unwrap()
                .iter()
                .map(openmat_value::TableRowName::as_str)
                .collect::<Vec<_>>(),
            ["two", "three"]
        );
        assert!(
            matches!(workspace.get("V"), Some(Value::Table(table)) if table.shape().dimensions() == [4, 1])
        );
        assert!(
            matches!(workspace.get("H"), Some(Value::Table(table)) if table.shape().dimensions() == [2, 2])
        );
    }

    #[test]
    fn elementwise_logical_operators_preserve_array_shape_and_nonfinite_truth() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "left=[1;Inf;0]; right=[1;0;NaN]; both=left&right; either=left|right; inverse=~left;\n",
        )
        .expect("elementwise logical source executes");
        let workspace = engine.interpreter.workspace();
        for (name, expected) in [
            ("both", [true, false, false]),
            ("either", [true, true, true]),
            ("inverse", [false, false, true]),
        ] {
            let Value::Array(ArrayData::Logical(values)) = workspace.get(name).unwrap() else {
                panic!("{name} must be a logical array")
            };
            assert_eq!(values.shape().dimensions(), &[3, 1]);
            assert_eq!(
                values
                    .as_slice()
                    .iter()
                    .map(|value| value.get())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn command_window_table_display_contains_schema_rows_and_missing_values() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        let output = execute(
            &mut engine,
            "Name=[\"Alice\";missing]; Score=[88;NaN]; T=table(Name,Score); disp(T);\n",
        )
        .expect("table display executes");
        let text = output
            .events
            .iter()
            .find_map(|event| match event {
                Event::Display(DisplayEvent { representations }) => {
                    representations.get("text/plain")
                }
                _ => None,
            })
            .expect("disp(table) emits plain text");
        assert!(text.contains("Name"));
        assert!(text.contains("Score"));
        assert!(text.contains("\"Alice\""));
        assert!(text.contains("<missing>"));
        assert!(text.contains("NaN"));
        assert!(!text.contains("table [2x2]"));
    }

    #[test]
    fn v2_empty_aggregate_states_and_exact_leaf_kinds_are_distinct() {
        let limits = kernel_v2::AggregateLimits::default();
        let empty_cell =
            Value::Cell(CellArray::from_values(Shape::new([0, 0]).unwrap(), Vec::new()).unwrap());
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            dimensions,
            items,
            usage,
            ..
        }) = inspect_value_v2(&empty_cell, "cell", &v2_inspect([0, 0], 1), &limits).unwrap()
        else {
            panic!("empty cell preview")
        };
        assert_eq!(dimensions, vec![0, 0]);
        assert!(items.is_empty());
        assert_eq!(usage.nodes, 1);
        assert_eq!(usage.elements, 0);

        let scalar_struct =
            Value::Struct(StructArray::empty(Shape::new([1, 1]).unwrap(), Vec::new()).unwrap());
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Struct {
            fields,
            records,
            usage,
            ..
        }) = inspect_value_v2(&scalar_struct, "struct", &v2_inspect([1, 1], 1), &limits).unwrap()
        else {
            panic!("unfielded scalar struct preview")
        };
        assert!(fields.is_empty());
        assert_eq!(records, vec![kernel_v2::StructRecord::new(Vec::new())]);
        assert_eq!(usage.nodes, 1);
        assert_eq!(usage.elements, 1);

        let exact_leaves = Value::Cell(
            CellArray::from_values(
                Shape::new([1, 5]).unwrap(),
                vec![
                    Value::Logical(true),
                    Value::Array(ArrayData::Char(
                        DenseArray::from_vec(
                            Shape::new([1, 2]).unwrap(),
                            vec![CharCodeUnit::new(65), CharCodeUnit::new(0xD83D)],
                        )
                        .unwrap(),
                    )),
                    Value::from(StringElement::missing()),
                    integer_value([1, 1], vec![u64::MAX]),
                    Value::Complex(openmat_value::Complex64::new(f64::INFINITY, -2.0)),
                ],
            )
            .unwrap(),
        );
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            items, ..
        }) = inspect_value_v2(&exact_leaves, "cell", &v2_inspect([1, 5], 5), &limits).unwrap()
        else {
            panic!("exact leaf preview")
        };
        assert!(matches!(
            items[0].payload,
            kernel_v2::ExactPayload::Logical { .. }
        ));
        assert!(matches!(
            items[1].payload,
            kernel_v2::ExactPayload::Char { .. }
        ));
        assert!(matches!(
            &items[2].payload,
            kernel_v2::ExactPayload::String { string_code_units, missing }
                if string_code_units == &[Vec::<u16>::new()] && missing == &[true]
        ));
        assert!(matches!(
            &items[3].payload,
            kernel_v2::ExactPayload::Integer { integer }
                if integer[0].real == u64::MAX.to_string()
        ));
        assert!(matches!(
            &items[4].payload,
            kernel_v2::ExactPayload::Numeric { real, imag }
                if real == &["+Inf"] && imag == &["-2"]
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v2_aggregate_limits_truncate_whole_candidates_and_hard_fail_invalid_leaves() {
        let logicals = Value::Cell(
            CellArray::from_values(
                Shape::new([1, 2]).unwrap(),
                vec![Value::Logical(true), Value::Logical(false)],
            )
            .unwrap(),
        );
        let element_limits = kernel_v2::AggregateLimits::new(
            2,
            16,
            16,
            kernel_v2::MAX_AGGREGATE_NODES,
            2,
            kernel_v2::MAX_AGGREGATE_DEPTH,
        )
        .unwrap();
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            items,
            truncation,
            usage,
            ..
        }) = inspect_value_v2(&logicals, "cell", &v2_inspect([1, 2], 2), &element_limits).unwrap()
        else {
            panic!("whole-element truncation")
        };
        assert_eq!(items.len(), 1);
        assert_eq!(truncation.omitted_elements, 1);
        assert_eq!(usage.elements, 2);

        let node_limits = kernel_v2::AggregateLimits::new(2, 1, 1, 2, 8, 32).unwrap();
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            items,
            truncation,
            ..
        }) = inspect_value_v2(&logicals, "cell", &v2_inspect([1, 2], 2), &node_limits).unwrap()
        else {
            panic!("node-bound truncation")
        };
        assert_eq!(items.len(), 1);
        assert_eq!(truncation.omitted_elements, 1);

        let strings = Value::Cell(
            CellArray::from_values(
                Shape::new([1, 2]).unwrap(),
                vec![Value::from("ab"), Value::from("cd")],
            )
            .unwrap(),
        );
        let code_limits = kernel_v2::AggregateLimits::new(2, 2, 3, 8, 8, 32).unwrap();
        let kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            items,
            truncation,
            usage,
            ..
        }) = inspect_value_v2(&strings, "cell", &v2_inspect([1, 2], 2), &code_limits).unwrap()
        else {
            panic!("code-unit-bound truncation")
        };
        assert_eq!(items.len(), 1);
        assert_eq!(truncation.omitted_elements, 1);
        assert_eq!(usage.code_units, 2);

        let string_limits = kernel_v2::AggregateLimits::new(1, 1, 1, 4, 4, 32).unwrap();
        assert_eq!(
            inspect_value_v2(
                &Value::Cell(
                    CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![Value::from("ab")],)
                        .unwrap(),
                ),
                "cell",
                &v2_inspect([1, 1], 1),
                &string_limits,
            )
            .expect_err("per-string overflow is a hard failure")
            .category(),
            "workspace.previewLimit"
        );

        let depth_limits = kernel_v2::AggregateLimits::new(1, 1, 1, 2, 2, 0).unwrap();
        assert_eq!(
            inspect_value_v2(
                &Value::Cell(
                    CellArray::from_values(
                        Shape::new([1, 1]).unwrap(),
                        vec![Value::Logical(true)],
                    )
                    .unwrap(),
                ),
                "cell",
                &v2_inspect([1, 1], 1),
                &depth_limits,
            )
            .expect_err("depth zero rejects a child exact node")
            .category(),
            "workspace.previewDepth"
        );

        let oversized_node = Value::Cell(
            CellArray::from_values(
                Shape::new([1, 1]).unwrap(),
                vec![Value::Array(ArrayData::F64(
                    DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![1.0, 2.0]).unwrap(),
                ))],
            )
            .unwrap(),
        );
        let per_node_limits = kernel_v2::AggregateLimits::new(1, 1, 1, 2, 2, 32).unwrap();
        assert_eq!(
            inspect_value_v2(
                &oversized_node,
                "cell",
                &v2_inspect([1, 1], 1),
                &per_node_limits,
            )
            .expect_err("per-node overflow is a hard failure")
            .category(),
            "workspace.previewLimit"
        );

        for unsupported in [
            Value::Object(ObjectHandle::new(7)),
            Value::Function(FunctionHandle::Builtin(BuiltinHandle::new(9))),
        ] {
            let aggregate = Value::Cell(
                CellArray::from_values(
                    Shape::new([1, 2]).unwrap(),
                    vec![Value::Logical(true), unsupported],
                )
                .unwrap(),
            );
            assert_eq!(
                inspect_value_v2(
                    &aggregate,
                    "cell",
                    &v2_inspect([1, 2], 2),
                    &kernel_v2::AggregateLimits::default(),
                )
                .expect_err("unsupported later leaf discards the earlier candidate")
                .category(),
                "workspace.unsupportedValue"
            );
        }
    }

    #[test]
    fn summaries_and_workspace_delta_use_exact_new_value_truth() {
        let integer = integer_value([1, 1], vec![ComplexInteger::new(i64::MIN, i64::MAX)]);
        let summary = variable_summary("wide", &integer, "int64");
        assert_eq!(summary.class, "int64");
        assert!(summary.complex);
        assert_eq!(summary.dimensions, vec![1, 1]);
        assert_eq!(summary.bytes, Some(16));

        let char_value = Value::Array(ArrayData::Char(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![CharCodeUnit::new(0xD83D)])
                .unwrap(),
        ));
        assert_eq!(variable_summary("c", &char_value, "char").bytes, Some(2));

        let before_string =
            Value::String(StringValue::scalar(StringElement::from_code_units(Vec::<
                u16,
            >::new(
            ))));
        let after_string = Value::String(StringValue::missing());
        assert!(values_changed(&before_string, &after_string));

        let mut before = Workspace::new();
        before.insert("text", before_string);
        before.insert("wide", integer_value([1, 1], vec![u64::MAX]));
        before.insert("char", char_value.clone());
        let mut after = Workspace::new();
        after.insert("text", after_string);
        after.insert("wide", integer_value([1, 1], vec![u64::MAX - 1]));
        after.insert(
            "char",
            Value::Array(ArrayData::Char(
                DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![CharCodeUnit::new(0xDE42)])
                    .unwrap(),
            )),
        );
        let engine = RuntimeEngine::new().expect("runtime engine");
        let delta = workspace_delta(&before, &after, &engine.interpreter);
        assert_eq!(
            delta
                .changed
                .iter()
                .map(|summary| summary.name.as_str())
                .collect::<Vec<_>>(),
            vec!["char", "text", "wide"]
        );
    }

    fn request(message_id: &str, request: Request) -> RequestEnvelope {
        RequestEnvelope::new("runtime-session", message_id, request)
    }

    fn initialize_request() -> RequestEnvelope {
        request(
            "initialize",
            Request::Initialize(InitializeRequest {
                client: ImplementationInfo {
                    name: "kernel-test".to_owned(),
                    version: "1".to_owned(),
                },
                supported_protocols: vec![PROTOCOL_V0.to_owned()],
                capabilities: Capabilities {
                    execution_modes: vec![ExecutionMode::Repl],
                    display_mime_types: vec!["text/plain".to_owned()],
                    max_preview_elements: 32,
                    interrupt: true,
                    workspace_delta: true,
                },
            }),
        )
    }

    fn response(messages: &[ServerMessage]) -> &ResponseEnvelope {
        messages
            .iter()
            .find_map(|message| match message {
                ServerMessage::Response(response) => Some(response),
                ServerMessage::Event(_) => None,
            })
            .expect("response")
    }

    #[test]
    fn request_envelopes_cover_initialize_execute_list_inspect_shutdown() {
        let mut kernel = Kernel::new(
            "runtime-session",
            RuntimeEngine::new().expect("runtime engine"),
        );
        let initialize = kernel.handle_request(&initialize_request());
        let Some(ResponseResult::Initialize(result)) = &response(&initialize).result else {
            panic!("initialize result");
        };
        assert_eq!(result.implementation.name, "openmat-runtime");

        let executed = kernel.handle_request(&request(
            "execute",
            Request::Execute(ExecuteRequest {
                code: "answer = 6 * 7;\n".to_owned(),
                source_name: "lifecycle.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        ));
        assert!(response(&executed).ok);
        assert!(executed.iter().any(|message| matches!(
            message,
            ServerMessage::Event(EventEnvelope {
                event: Event::WorkspaceDelta(_),
                ..
            })
        )));

        let listed = kernel.handle_request(&request(
            "list",
            Request::ListWorkspace(ListWorkspaceRequest {}),
        ));
        let Some(ResponseResult::ListWorkspace(workspace)) = &response(&listed).result else {
            panic!("workspace result");
        };
        assert_eq!(workspace.variables[0].name, "answer");

        let inspected = kernel.handle_request(&request(
            "inspect",
            Request::Inspect(scalar_inspect("answer")),
        ));
        let Some(ResponseResult::Inspect(preview)) = &response(&inspected).result else {
            panic!("inspect result");
        };
        assert_eq!(preview.values, vec![PreviewValue::Number { value: 42.0 }]);

        let shutdown =
            kernel.handle_request(&request("shutdown", Request::Shutdown(ShutdownRequest {})));
        assert!(response(&shutdown).ok);
        assert_eq!(kernel.status(), KernelStatus::Dead);
    }

    #[test]
    fn request_envelope_file_loads_script_function_and_class_from_real_sources() {
        let entry_directory = TestDirectory::new("envelope-entry");
        let search_directory = TestDirectory::new("envelope-search");
        let helper_path = search_directory.path().join("triple.m");
        let class_path = entry_directory.path().join("BoxValue.m");
        let script_path = entry_directory.path().join("setup.m");
        let main_path = entry_directory.path().join("main.m");
        fs::write(
            &helper_path,
            "function value = triple(input)\nvalue = input * 3;\nend\n",
        )
        .unwrap();
        fs::write(
            &class_path,
            "classdef BoxValue\nproperties\nvalue = 4\nend\nend\n",
        )
        .unwrap();
        fs::write(&script_path, "from_script = triple(5);\n").unwrap();
        let main_code = "setup\nitem = BoxValue();\nanswer = from_script + item.value;\n";
        fs::write(&main_path, main_code).unwrap();

        let engine = RuntimeEngine::with_search_paths([search_directory.path()]).unwrap();
        let mut kernel = Kernel::new("runtime-session", engine);
        assert!(response(&kernel.handle_request(&initialize_request())).ok);
        let executed = kernel.handle_request(&request(
            "execute-real-file",
            Request::Execute(ExecuteRequest {
                code: main_code.to_owned(),
                source_name: main_path.to_string_lossy().into_owned(),
                mode: ExecutionMode::File,
            }),
        ));
        assert!(response(&executed).ok, "{:?}", response(&executed));

        let listed = kernel.handle_request(&request(
            "list-real-file",
            Request::ListWorkspace(ListWorkspaceRequest {}),
        ));
        let Some(ResponseResult::ListWorkspace(workspace)) = &response(&listed).result else {
            panic!("workspace result");
        };
        assert!(
            workspace
                .variables
                .iter()
                .any(|variable| variable.name == "from_script")
        );
        assert!(
            workspace
                .variables
                .iter()
                .any(|variable| variable.name == "item" && variable.class == "BoxValue")
        );

        let inspected = kernel.handle_request(&request(
            "inspect-real-file",
            Request::Inspect(scalar_inspect("answer")),
        ));
        let Some(ResponseResult::Inspect(preview)) = &response(&inspected).result else {
            panic!("inspect result");
        };
        assert_eq!(preview.values, vec![PreviewValue::Number { value: 19.0 }]);
    }

    #[test]
    fn real_class_file_executes_static_method_and_session_constant_without_an_instance() {
        let directory = TestDirectory::new("class-static-constant");
        fs::write(
            directory.path().join("OpenMatStaticConstant.m"),
            "classdef OpenMatStaticConstant\nproperties (Constant)\nFactor = 3\nend\nmethods (Static)\nfunction result = scale(value)\nresult = OpenMatStaticConstant.Factor * value;\nend\nend\nend\n",
        )
        .unwrap();
        let main_path = directory.path().join("main.m");
        fs::write(
            &main_path,
            "openmat_result = [OpenMatStaticConstant.Factor, OpenMatStaticConstant.scale(5)];\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect("real static/constant class should execute");

        assert_eq!(
            engine
                .inspect(&row_inspect("openmat_result", 2))
                .unwrap()
                .values,
            vec![
                PreviewValue::Number { value: 3.0 },
                PreviewValue::Number { value: 15.0 },
            ]
        );
        assert_eq!(engine.interpreter.object_count(), 0);
        assert_eq!(engine.interpreter.class_count(), 2);
    }

    #[test]
    fn real_class_files_execute_dependent_accessors_and_plus_overload() {
        let directory = TestDirectory::new("class-dependent-plus");
        fs::write(
            directory.path().join("OpenMatDependentBox.m"),
            "classdef OpenMatDependentBox\nproperties (Access = private)\nBase\nend\nproperties (Dependent)\nTwice\nend\nmethods\nfunction object = OpenMatDependentBox(value)\nobject.Base = value;\nend\nfunction value = get.Twice(object)\nvalue = object.Base * 2;\nend\nfunction object = set.Twice(object, value)\nobject.Base = value / 2;\nend\nend\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("OpenMatAddend.m"),
            "classdef OpenMatAddend\nproperties\nValue\nend\nmethods\nfunction object = OpenMatAddend(value)\nobject.Value = value;\nend\nfunction result = plus(left, right)\nresult = left.Value + right.Value;\nend\nend\nend\n",
        )
        .unwrap();
        let main_path = directory.path().join("main.m");
        fs::write(
            &main_path,
            "object = OpenMatDependentBox(4);\nfirst = object.Twice;\nobject.Twice = 18;\nleft = OpenMatAddend(7);\nright = OpenMatAddend(5);\nopenmat_result = [first, object.Twice, left + right];\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect("dependent accessors and plus should execute from real files");

        assert_eq!(
            engine
                .inspect(&row_inspect("openmat_result", 3))
                .unwrap()
                .values,
            vec![
                PreviewValue::Number { value: 8.0 },
                PreviewValue::Number { value: 18.0 },
                PreviewValue::Number { value: 12.0 },
            ]
        );
    }

    #[test]
    fn real_class_operator_failures_are_structured() {
        let directory = TestDirectory::new("class-plus-errors");
        fs::write(
            directory.path().join("PlainValue.m"),
            "classdef PlainValue\nproperties\nValue\nend\nmethods\nfunction object = PlainValue(value)\nobject.Value = value;\nend\nend\nend\n",
        )
        .unwrap();
        let missing_path = directory.path().join("missing_plus.m");
        fs::write(
            &missing_path,
            "object = PlainValue(1);\nopenmat_result = object + 2;\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();
        let missing = engine
            .execute_file(&missing_path, &CancellationToken::new())
            .expect_err("missing plus overload must be invalid operands");
        assert_eq!(missing.category(), "runtime.invalidOperands");
        assert_eq!(missing.diagnostics()[0].code.as_deref(), Some("OMR0007"));

        fs::write(
            directory.path().join("FirstAddend.m"),
            "classdef FirstAddend\nproperties\nValue\nend\nmethods\nfunction object = FirstAddend(value)\nobject.Value = value;\nend\nfunction result = plus(left, right)\nresult = left.Value + right.Value;\nend\nend\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("SecondAddend.m"),
            "classdef SecondAddend\nproperties\nValue\nend\nmethods\nfunction object = SecondAddend(value)\nobject.Value = value;\nend\nfunction result = plus(left, right)\nresult = left.Value + right.Value;\nend\nend\nend\n",
        )
        .unwrap();
        let heterogeneous_path = directory.path().join("heterogeneous_plus.m");
        fs::write(
            &heterogeneous_path,
            "left = FirstAddend(1);\nright = SecondAddend(2);\nopenmat_result = left + right;\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();
        let heterogeneous = engine
            .execute_file(&heterogeneous_path, &CancellationToken::new())
            .expect_err("heterogeneous objects must reject operator dispatch");
        assert_eq!(heterogeneous.category(), "runtime.invalidOperands");
        assert_eq!(
            heterogeneous.diagnostics()[0].code.as_deref(),
            Some("OMR0007")
        );
    }

    #[test]
    fn real_dependent_property_failures_are_structured() {
        let directory = TestDirectory::new("class-dependent-errors");

        fs::write(
            directory.path().join("ReadOnlyDependent.m"),
            "classdef ReadOnlyDependent\nproperties (Dependent)\nValue\nend\nmethods\nfunction value = get.Value(object)\nvalue = 1;\nend\nend\nend\n",
        )
        .unwrap();
        let read_only_path = directory.path().join("read_only_dependent.m");
        fs::write(
            &read_only_path,
            "object = ReadOnlyDependent;\nobject.Value = 2;\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();
        let read_only = engine
            .execute_file(&read_only_path, &CancellationToken::new())
            .expect_err("getter-only dependent property must reject assignment");
        assert_eq!(read_only.category(), "runtime.object");
        assert_eq!(read_only.diagnostics()[0].code.as_deref(), Some("OMR0014"));
        assert!(read_only.diagnostics()[0].message.contains("read-only"));

        fs::write(
            directory.path().join("WriteOnlyDependent.m"),
            "classdef WriteOnlyDependent\nproperties (Access = private)\nBase\nend\nproperties (Dependent)\nValue\nend\nmethods\nfunction object = set.Value(object, value)\nobject.Base = value;\nend\nend\nend\n",
        )
        .unwrap();
        let write_only_path = directory.path().join("write_only_dependent.m");
        fs::write(
            &write_only_path,
            "object = WriteOnlyDependent;\nobject.Value = 2;\nopenmat_result = object.Value;\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();
        let write_only = engine
            .execute_file(&write_only_path, &CancellationToken::new())
            .expect_err("setter-only dependent property must reject reads");
        assert_eq!(write_only.category(), "runtime.object");
        assert_eq!(write_only.diagnostics()[0].code.as_deref(), Some("OMR0014"));
        assert!(write_only.diagnostics()[0].message.contains("write-only"));

        let bad_class = directory.path().join("BadDependent.m");
        fs::write(
            &bad_class,
            "classdef BadDependent\nproperties (Dependent)\nValue\nend\nmethods\nfunction set.Value(object)\nend\nend\nend\n",
        )
        .unwrap();
        let bad_main = directory.path().join("bad_accessor.m");
        fs::write(&bad_main, "object = BadDependent;\n").unwrap();
        let mut engine = RuntimeEngine::new().unwrap();
        let signature = engine
            .execute_file(&bad_main, &CancellationToken::new())
            .expect_err("invalid setter signature must fail compilation");
        assert_eq!(signature.category(), "compile.error");
        assert_eq!(signature.diagnostics()[0].code.as_deref(), Some("OMC0010"));
        assert_eq!(
            signature.diagnostics()[0]
                .range
                .as_ref()
                .map(|range| range.source_name.as_str()),
            Some(fs::canonicalize(&bad_class).unwrap().to_str().unwrap())
        );

        let missing_class = directory.path().join("MissingDependent.m");
        fs::write(
            &missing_class,
            "classdef MissingDependent\nproperties (Dependent)\nValue\nend\nend\n",
        )
        .unwrap();
        let missing_main = directory.path().join("missing_accessor.m");
        fs::write(&missing_main, "object = MissingDependent;\n").unwrap();
        let mut engine = RuntimeEngine::new().unwrap();
        let missing_accessor = engine
            .execute_file(&missing_main, &CancellationToken::new())
            .expect_err("dependent property without accessors must fail compilation");
        assert_eq!(missing_accessor.category(), "compile.error");
        assert_eq!(
            missing_accessor.diagnostics()[0].code.as_deref(),
            Some("OMC0010")
        );
        assert_eq!(
            missing_accessor.diagnostics()[0]
                .range
                .as_ref()
                .map(|range| range.source_name.as_str()),
            Some(fs::canonicalize(&missing_class).unwrap().to_str().unwrap())
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn real_class_files_enforce_public_private_and_protected_access() {
        let directory = TestDirectory::new("class-access");
        fs::write(
            directory.path().join("OpenMatAccessProbe.m"),
            "classdef OpenMatAccessProbe\nproperties\nPublicValue\nend\nproperties (Access = private)\nSecretValue\nend\nmethods\nfunction object = OpenMatAccessProbe(publicValue, secretValue)\nobject.PublicValue = publicValue;\nobject.SecretValue = secretValue;\nend\nfunction value = revealSecret(object)\nvalue = object.SecretValue;\nend\nend\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("OpenMatAccessBase.m"),
            "classdef OpenMatAccessBase\nproperties\nPublicValue = 1\nend\nproperties (Access = protected)\nProtectedValue = 2\nend\nproperties (Access = private)\nPrivateValue = 3\nend\nmethods\nfunction values = readPrivate(object)\nvalues = [object.PrivateValue, object.privateMethod()];\nend\nend\nmethods (Access = protected)\nfunction value = protectedMethod(object)\nvalue = object.ProtectedValue * 2;\nend\nend\nmethods (Access = private)\nfunction value = privateMethod(object)\nvalue = object.PrivateValue * 2;\nend\nend\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("OpenMatAccessDerived.m"),
            "classdef OpenMatAccessDerived < OpenMatAccessBase\nmethods\nfunction object = setProtected(object, value)\nobject.ProtectedValue = value;\nend\nfunction values = readProtected(object)\nvalues = [object.ProtectedValue, object.protectedMethod()];\nend\nend\nend\n",
        )
        .unwrap();

        let public_path = directory.path().join("public_case.m");
        fs::write(
            &public_path,
            "value = OpenMatAccessProbe(7, 11);\nopenmat_result = [value.PublicValue, value.revealSecret()];\n",
        )
        .unwrap();
        let private_path = directory.path().join("private_error.m");
        fs::write(
            &private_path,
            "value = OpenMatAccessProbe(7, 11);\nopenmat_result = value.SecretValue;\n",
        )
        .unwrap();
        let controls_path = directory.path().join("controls_case.m");
        fs::write(
            &controls_path,
            "object = OpenMatAccessDerived;\nobject.PublicValue = 10;\nobject = object.setProtected(20);\nopenmat_result = [object.PublicValue, object.readProtected(), object.readPrivate()];\n",
        )
        .unwrap();
        let protected_path = directory.path().join("protected_error.m");
        fs::write(
            &protected_path,
            "object = OpenMatAccessDerived;\nopenmat_result = object.ProtectedValue;\n",
        )
        .unwrap();
        let protected_method_path = directory.path().join("protected_method_error.m");
        fs::write(
            &protected_method_path,
            "object = OpenMatAccessDerived;\nopenmat_result = object.protectedMethod();\n",
        )
        .unwrap();
        let private_method_path = directory.path().join("private_method_error.m");
        fs::write(
            &private_method_path,
            "object = OpenMatAccessDerived;\nopenmat_result = object.privateMethod();\n",
        )
        .unwrap();

        let inspect_row = |engine: &mut RuntimeEngine, columns: u64| {
            engine
                .inspect(&InspectRequest {
                    name: "openmat_result".to_owned(),
                    range: MatrixRange {
                        start: vec![1, 1],
                        size: vec![1, columns],
                    },
                    max_elements: columns,
                })
                .unwrap()
                .values
        };

        let mut public_engine = RuntimeEngine::new().unwrap();
        public_engine
            .execute_file(&public_path, &CancellationToken::new())
            .expect("public access and class-private access should execute");
        assert_eq!(
            inspect_row(&mut public_engine, 2),
            vec![
                PreviewValue::Number { value: 7.0 },
                PreviewValue::Number { value: 11.0 },
            ]
        );
        let private = public_engine
            .execute_file(&private_path, &CancellationToken::new())
            .expect_err("external private access must fail");
        assert_eq!(private.category(), "access-violation");
        assert_eq!(private.diagnostics()[0].code.as_deref(), Some("OMR0017"));
        assert_eq!(
            private.diagnostics()[0]
                .range
                .as_ref()
                .map(|range| range.source_name.as_str()),
            Some(fs::canonicalize(&private_path).unwrap().to_str().unwrap())
        );

        let mut hierarchy_engine = RuntimeEngine::new().unwrap();
        hierarchy_engine
            .execute_file(&controls_path, &CancellationToken::new())
            .expect("derived protected access and base private access should execute");
        assert_eq!(
            inspect_row(&mut hierarchy_engine, 5),
            vec![
                PreviewValue::Number { value: 10.0 },
                PreviewValue::Number { value: 20.0 },
                PreviewValue::Number { value: 40.0 },
                PreviewValue::Number { value: 3.0 },
                PreviewValue::Number { value: 6.0 },
            ]
        );
        let protected = hierarchy_engine
            .execute_file(&protected_path, &CancellationToken::new())
            .expect_err("external protected access must fail");
        assert_eq!(protected.category(), "access-violation");
        assert_eq!(protected.diagnostics()[0].code.as_deref(), Some("OMR0017"));
        for path in [&protected_method_path, &private_method_path] {
            let method = hierarchy_engine
                .execute_file(path, &CancellationToken::new())
                .expect_err("external non-public method access must fail");
            assert_eq!(method.category(), "access-violation");
            assert_eq!(method.diagnostics()[0].code.as_deref(), Some("OMR0017"));
        }
    }

    #[test]
    fn real_class_files_execute_superclass_construction_override_and_reflection() {
        let directory = TestDirectory::new("class-inheritance-runtime");
        fs::write(
            directory.path().join("OpenMatBaseNumber.m"),
            "classdef OpenMatBaseNumber\nproperties\nValue\nend\nmethods\nfunction object = OpenMatBaseNumber(value)\nobject.Value = value;\nend\nfunction result = score(object)\nresult = object.Value;\nend\nfunction result = describe(object)\nresult = [object.Value, object.score()];\nend\nend\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("OpenMatDerivedNumber.m"),
            "classdef OpenMatDerivedNumber < OpenMatBaseNumber\nmethods\nfunction object = OpenMatDerivedNumber(value)\nobject@OpenMatBaseNumber(value);\nend\nfunction result = score(object)\nresult = object.Value * 3;\nend\nend\nend\n",
        )
        .unwrap();
        let main = directory.path().join("inheritance_case.m");
        fs::write(
            &main,
            "object = OpenMatDerivedNumber(4);\noverride_result = [object.score(), object.describe()];\nreflection_result = [strcmp(class(object), 'OpenMatDerivedNumber'), isa(object, 'OpenMatDerivedNumber'), isa(object, 'OpenMatBaseNumber'), isa(object, 'handle'), isobject(object), isobject(4)];\n",
        )
        .unwrap();

        let mut engine = RuntimeEngine::new().unwrap();
        engine
            .execute_file(&main, &CancellationToken::new())
            .expect("explicit base construction and dynamic override should execute");

        assert_eq!(
            engine
                .inspect(&row_inspect("override_result", 3))
                .unwrap()
                .values,
            vec![
                PreviewValue::Number { value: 12.0 },
                PreviewValue::Number { value: 4.0 },
                PreviewValue::Number { value: 12.0 },
            ]
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("reflection_result", 6))
                .unwrap()
                .values,
            [true, true, true, false, true, false]
                .into_iter()
                .map(|value| PreviewValue::Logical { value })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn support_file_runtime_diagnostic_uses_its_canonical_source_path() {
        let directory = TestDirectory::new("support-diagnostic");
        let helper_path = directory.path().join("broken_helper.m");
        let main_path = directory.path().join("main.m");
        fs::write(
            &helper_path,
            "function value = broken_helper()\nvalue = missing_in_helper;\nend\n",
        )
        .unwrap();
        fs::write(&main_path, "answer = broken_helper();\n").unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        let error = engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect_err("support function must fail at runtime");
        assert_eq!(error.category(), "runtime.undefinedName");
        assert_eq!(
            error.diagnostics()[0]
                .range
                .as_ref()
                .map(|range| range.source_name.as_str()),
            Some(fs::canonicalize(&helper_path).unwrap().to_str().unwrap())
        );
    }

    #[test]
    fn support_file_compile_diagnostic_is_real_and_prevents_entry_execution() {
        let directory = TestDirectory::new("support-compile-diagnostic");
        let helper_path = directory.path().join("bad_helper.m");
        let main_path = directory.path().join("main.m");
        fs::write(
            &helper_path,
            "function value = bad_helper()\nvalue = ;\nend\n",
        )
        .unwrap();
        fs::write(
            &main_path,
            "should_not_exist = 7;\nanswer = bad_helper();\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        let error = engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect_err("support parse failure must stop the complete module");
        assert_eq!(error.category(), "compile.parse");
        assert_eq!(
            error.diagnostics()[0]
                .range
                .as_ref()
                .map(|range| range.source_name.as_str()),
            Some(fs::canonicalize(&helper_path).unwrap().to_str().unwrap())
        );
        assert!(engine.list_workspace().unwrap().is_empty());
    }

    #[test]
    fn cyclic_scripts_and_request_content_mismatch_are_stable_errors() {
        let directory = TestDirectory::new("cycle-mismatch");
        let main_path = directory.path().join("main.m");
        fs::write(directory.path().join("first.m"), "second\n").unwrap();
        fs::write(directory.path().join("second.m"), "first\n").unwrap();
        fs::write(&main_path, "first\n").unwrap();
        let mut engine = RuntimeEngine::new().unwrap();
        let cycle = engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect_err("script cycle must fail before execution");
        assert_eq!(cycle.category(), "source.cyclicDependency");
        assert_eq!(cycle.diagnostics()[0].code.as_deref(), Some("OMS0004"));
        assert!(engine.list_workspace().unwrap().is_empty());

        let mismatch = engine
            .execute(
                &ExecuteRequest {
                    code: "different = 1;\n".to_owned(),
                    source_name: main_path.to_string_lossy().into_owned(),
                    mode: ExecutionMode::File,
                },
                &CancellationToken::new(),
            )
            .expect_err("request code must match the named real file");
        assert_eq!(mismatch.category(), "source.contentMismatch");
    }

    #[test]
    fn script_in_loop_runs_at_call_site_and_mutual_functions_link_once() {
        let directory = TestDirectory::new("loop-and-function-cycle");
        fs::write(directory.path().join("step_once.m"), "count = count + 1;\n").unwrap();
        fs::write(
            directory.path().join("left_call.m"),
            "function value = left_call(depth)\nif depth == 0\nvalue = 0;\nelse\nvalue = right_call(depth - 1) + 1;\nend\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("right_call.m"),
            "function value = right_call(depth)\nif depth == 0\nvalue = 0;\nelse\nvalue = left_call(depth - 1) + 1;\nend\nend\n",
        )
        .unwrap();
        let main_path = directory.path().join("main.m");
        fs::write(
            &main_path,
            "count = 0;\nwhile count < 3\nstep_once\nend\nanswer = left_call(4);\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect("loop script and mutually recursive functions execute");
        assert_eq!(
            engine.inspect(&scalar_inspect("count")).unwrap().values,
            vec![PreviewValue::Number { value: 3.0 }]
        );
        assert_eq!(
            engine.inspect(&scalar_inspect("answer")).unwrap().values,
            vec![PreviewValue::Number { value: 4.0 }]
        );
    }

    #[test]
    fn function_scoped_script_shares_existing_and_new_caller_locals() {
        let directory = TestDirectory::new("function-script-scope");
        fs::write(
            directory.path().join("scope_step.m"),
            "value = value * 3;\ncreated = 4;\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("scope_caller.m"),
            "function value = scope_caller()\nvalue = 2;\nscope_step\nvalue = value + created;\nend\n",
        )
        .unwrap();
        let main_path = directory.path().join("main.m");
        fs::write(&main_path, "openmat_result = scope_caller();\n").unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect("script executes in its calling function workspace");

        assert_eq!(
            engine
                .inspect(&scalar_inspect("openmat_result"))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 10.0 }]
        );
        assert!(engine.interpreter.workspace().get("created").is_none());
    }

    #[test]
    fn real_files_execute_call_metadata_and_ordered_script_workspace_clear() {
        let directory = TestDirectory::new("metadata-clear");
        fs::write(
            directory.path().join("openmat_nargin_nargout.m"),
            "function [head, tail] = openmat_nargin_nargout(left, right)\nhead = [nargin, nargout, left];\ntail = right;\nend\n",
        )
        .unwrap();
        let metadata_path = directory.path().join("metadata_case.m");
        fs::write(
            &metadata_path,
            "single_output = openmat_nargin_nargout(4, 5);\n[multiple_output, tail] = openmat_nargin_nargout(4, 5);\nopenmat_result = [single_output, multiple_output, tail];\n",
        )
        .unwrap();
        let mut metadata_engine = RuntimeEngine::new().unwrap();
        metadata_engine
            .execute_file(&metadata_path, &CancellationToken::new())
            .expect("real function metadata case");
        assert_eq!(
            metadata_engine
                .inspect(&row_inspect("openmat_result", 7))
                .unwrap()
                .values,
            [2.0, 1.0, 4.0, 2.0, 2.0, 4.0, 5.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );

        fs::write(
            directory.path().join("openmat_workspace_step.m"),
            "openmat_stage = openmat_accumulator * 3;\nopenmat_accumulator = openmat_accumulator + 1;\n",
        )
        .unwrap();
        let clear_path = directory.path().join("clear_case.m");
        fs::write(
            &clear_path,
            "kept = 42;\nopenmat_accumulator = 2;\nopenmat_workspace_step;\nopenmat_accumulator = openmat_accumulator + openmat_stage;\nopenmat_result = [openmat_accumulator, openmat_stage];\nclear openmat_accumulator openmat_stage;\n",
        )
        .unwrap();
        let mut clear_engine = RuntimeEngine::new().unwrap();
        clear_engine
            .execute_file(&clear_path, &CancellationToken::new())
            .expect("real linked-script clear case");
        assert_eq!(
            clear_engine
                .inspect(&row_inspect("openmat_result", 2))
                .unwrap()
                .values,
            vec![
                PreviewValue::Number { value: 9.0 },
                PreviewValue::Number { value: 6.0 },
            ]
        );
        assert_eq!(
            clear_engine
                .inspect(&scalar_inspect("kept"))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 42.0 }]
        );
        assert!(
            clear_engine
                .interpreter
                .workspace()
                .get("openmat_accumulator")
                .is_none()
        );
        assert!(
            clear_engine
                .interpreter
                .workspace()
                .get("openmat_stage")
                .is_none()
        );
    }

    #[test]
    fn real_named_function_handles_link_flow_and_persist_across_requests() {
        let main_directory = TestDirectory::new("named-handles-main");
        let support_directory = TestDirectory::new("named-handles-support");
        fs::write(
            main_directory.path().join("same_directory_target.m"),
            "function result = same_directory_target(value)\nresult = value + 10;\nend\n",
        )
        .unwrap();
        fs::write(
            main_directory.path().join("deferred_target.m"),
            "function result = deferred_target()\nresult = missing_until_called;\nend\n",
        )
        .unwrap();
        fs::write(
            support_directory.path().join("search_path_target.m"),
            "function [head, tail] = search_path_target(left, right)\nhead = [nargin, nargout, left];\ntail = right;\nend\n",
        )
        .unwrap();
        let main_path = main_directory.path().join("named_handle_case.m");
        fs::write(
            &main_path,
            "local_handle = @local_target;\nreturned_handle = identity_handle(local_handle);\nlocal_result = returned_handle(4);\nsame_handle = @same_directory_target;\nsame_result = same_handle(5);\nsupport_handle = @search_path_target;\n[support_head, support_tail] = support_handle(6, 7);\nbuiltin_handle = @abs;\nbuiltin_result = builtin_handle(-8);\ndeferred_handle = @deferred_target;\nopenmat_result = [local_result, same_result, support_head, support_tail, builtin_result];\nfunction returned = identity_handle(input_handle)\nreturned = input_handle;\nend\nfunction result = local_target(value)\nresult = value * 3;\nend\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::with_search_paths([support_directory.path()]).unwrap();

        engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect("local, same-directory, search-path, and built-in handles should execute");
        assert_eq!(
            engine
                .inspect(&row_inspect("openmat_result", 7))
                .unwrap()
                .values,
            [12.0, 15.0, 2.0, 2.0, 6.0, 7.0, 8.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );

        execute(
            &mut engine,
            "persisted_local = local_handle(9);\n[persisted_head, persisted_tail] = support_handle(1, 2);\npersisted_builtin = builtin_handle(-3);\n",
        )
        .expect("workspace handles should retain their target modules and registries");
        assert_eq!(
            engine
                .inspect(&scalar_inspect("persisted_local"))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 27.0 }]
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("persisted_head", 3))
                .unwrap()
                .values,
            [2.0, 2.0, 1.0]
                .into_iter()
                .map(|value| PreviewValue::Number { value })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            engine
                .inspect(&scalar_inspect("persisted_tail"))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 2.0 }]
        );
        assert_eq!(
            engine
                .inspect(&scalar_inspect("persisted_builtin"))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 3.0 }]
        );

        let deferred_path = main_directory.path().join("deferred_target.m");
        let deferred_error = execute(&mut engine, "deferred_handle();\n")
            .expect_err("target body should run only when its handle is invoked");
        assert_eq!(deferred_error.category(), "runtime.undefinedName");
        assert_eq!(
            deferred_error.diagnostics()[0]
                .range
                .as_ref()
                .map(|range| range.source_name.as_str()),
            Some(fs::canonicalize(&deferred_path).unwrap().to_str().unwrap())
        );
    }

    #[test]
    fn real_anonymous_handles_capture_link_nest_and_persist_across_requests() {
        let main_directory = TestDirectory::new("anonymous-handles-main");
        let support_directory = TestDirectory::new("anonymous-handles-support");
        fs::write(
            support_directory.path().join("anonymous_path_target.m"),
            "function result = anonymous_path_target(value)\nresult = value + 20;\nend\n",
        )
        .unwrap();
        fs::write(
            support_directory.path().join("anonymous_path_factory.m"),
            "function result = anonymous_path_factory(offset)\nresult = @(x) x + offset;\nend\n",
        )
        .unwrap();
        let broken_factory_path = support_directory.path().join("anonymous_broken_factory.m");
        fs::write(
            &broken_factory_path,
            "function result = anonymous_broken_factory()\nresult = @() missing_in_returned_closure;\nend\n",
        )
        .unwrap();
        let main_path = main_directory.path().join("anonymous_handle_case.m");
        let source = "scale = 3;\ncaptured = @(x, y) x + y + scale;\nscale = 100;\nbuiltin_closure = @(x) abs(x);\nlocal_closure = @(x) anonymous_local_target(x);\npath_closure = @(x) anonymous_path_target(x);\nreturned_closure = anonymous_make_offset(7);\npassed_result = anonymous_apply(returned_closure, 5);\npath_returned_closure = anonymous_path_factory(11);\npath_returned_result = path_returned_closure(4);\nbroken_path_closure = anonymous_broken_factory();\nfactory = @(offset) @(x) x + offset;\nnested_closure = factory(9);\narray_value = [1 2];\narray_closure = @(index) array_value(index);\narray_value(1) = 99;\nmetadata_closure = @(x) x + nargin + nargout;\nshape_closure = @(value) size(value);\n[shape_rows, shape_columns] = shape_closure([1 2; 3 4]);\npair_closure = @(value) anonymous_pair(value);\n[pair_head, pair_tail] = pair_closure(30);\nopenmat_result = [captured(4, 5), builtin_closure(-6), local_closure(4), path_closure(5), passed_result, path_returned_result, nested_closure(5), array_closure(1), metadata_closure(1), shape_rows, shape_columns, pair_head, pair_tail];\nfunction result = anonymous_local_target(value)\nresult = value * 2;\nend\nfunction result = anonymous_make_offset(offset)\nresult = @(x) x + offset;\nend\nfunction result = anonymous_apply(handle, value)\nresult = handle(value);\nend\nfunction [head, tail] = anonymous_pair(value)\nhead = value;\ntail = value + 1;\nend\n";
        fs::write(&main_path, source).unwrap();
        let mut engine = RuntimeEngine::with_search_paths([support_directory.path()]).unwrap();

        engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect("real anonymous-function source should execute");
        assert_eq!(
            engine
                .inspect(&row_inspect("openmat_result", 13))
                .unwrap()
                .values,
            [
                12.0, 6.0, 8.0, 25.0, 12.0, 15.0, 14.0, 1.0, 3.0, 2.0, 2.0, 30.0, 31.0,
            ]
            .into_iter()
            .map(|value| PreviewValue::Number { value })
            .collect::<Vec<_>>()
        );

        execute(
            &mut engine,
            "persisted_capture = captured(1, 2);\npersisted_nested = nested_closure(3);\npersisted_path = path_closure(2);\npersisted_path_returned = path_returned_closure(1);\npersisted_array = array_closure(2);\n",
        )
        .expect("workspace closures should retain code, captures, and linked sources");
        assert_eq!(
            engine
                .inspect(&row_inspect("persisted_capture", 1))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 6.0 }]
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("persisted_nested", 1))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 12.0 }]
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("persisted_path", 1))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 22.0 }]
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("persisted_path_returned", 1))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 12.0 }]
        );
        assert_eq!(
            engine
                .inspect(&row_inspect("persisted_array", 1))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 2.0 }]
        );

        let source_error = execute(&mut engine, "broken_path_closure();\n")
            .expect_err("returned support closure should retain its source location");
        assert_eq!(source_error.category(), "runtime.undefinedName");
        assert_eq!(
            source_error.diagnostics()[0]
                .range
                .as_ref()
                .map(|range| range.source_name.as_str()),
            Some(
                fs::canonicalize(&broken_factory_path)
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
        );
    }

    #[test]
    fn undefined_anonymous_capture_constructs_then_fails_at_original_source() {
        let directory = TestDirectory::new("anonymous-missing-capture");
        let main_path = directory.path().join("missing_capture_case.m");
        let source = "missing_closure = @(x) x + missing_until_called;\nconstructed = 1;\n";
        fs::write(&main_path, source).unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect("R2022b permits construction with an unresolved free name");
        assert_eq!(
            engine
                .inspect(&scalar_inspect("constructed"))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 1.0 }]
        );

        let error = execute(
            &mut engine,
            "missing_until_called = 7;\nmissing_closure(1);\n",
        )
        .expect_err("a construction-time tombstone must hide later workspace creation");
        assert_eq!(error.category(), "runtime.undefinedName");
        assert_eq!(error.diagnostics()[0].code.as_deref(), Some("OMR0002"));
        let range = error.diagnostics()[0]
            .range
            .as_ref()
            .expect("missing capture source range");
        assert_eq!(
            range.source_name,
            fs::canonicalize(&main_path)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(
            range.start,
            source.find("missing_until_called").unwrap() as u64
        );
        assert_eq!(
            range.end,
            (source.find("missing_until_called").unwrap() + "missing_until_called".len()) as u64
        );
    }

    #[test]
    fn anonymous_capture_uses_value_copy_and_handle_identity_for_objects() {
        let directory = TestDirectory::new("anonymous-object-capture");
        fs::write(
            directory.path().join("AnonymousValueBox.m"),
            "classdef AnonymousValueBox\nproperties\nValue\nend\nmethods\nfunction object = AnonymousValueBox(value)\nobject.Value = value;\nend\nend\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("AnonymousHandleBox.m"),
            "classdef AnonymousHandleBox < handle\nproperties\nValue\nend\nmethods\nfunction object = AnonymousHandleBox(value)\nobject.Value = value;\nend\nend\nend\n",
        )
        .unwrap();
        let main = directory.path().join("anonymous_object_case.m");
        fs::write(
            &main,
            "value_box = AnonymousValueBox(1);\nvalue_closure = @() value_box.Value;\nvalue_box.Value = 9;\nhandle_box = AnonymousHandleBox(2);\nhandle_closure = @() handle_box.Value;\nhandle_box.Value = 8;\nopenmat_result = [value_closure(), handle_closure()];\n",
        )
        .unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        engine
            .execute_file(&main, &CancellationToken::new())
            .expect("object captures should use language-copy semantics");
        assert_eq!(
            engine
                .inspect(&row_inspect("openmat_result", 2))
                .unwrap()
                .values,
            vec![
                PreviewValue::Number { value: 1.0 },
                PreviewValue::Number { value: 8.0 },
            ]
        );
    }

    #[test]
    fn anonymous_callable_capture_distinguishes_resolver_only_from_captured_value() {
        let directory = TestDirectory::new("anonymous-callable-capture");
        let main_path = directory.path().join("callable_capture_case.m");
        let source = "late_wrapper = @() late_callable();\nexisting_callable = @() 3;\nvalue_wrapper = @() existing_callable();\nexisting_callable = @() 9;\nvalue_result = value_wrapper();\n";
        fs::write(&main_path, source).unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect("existing callable should be captured by value");
        assert_eq!(
            engine
                .inspect(&scalar_inspect("value_result"))
                .unwrap()
                .values,
            vec![PreviewValue::Number { value: 3.0 }]
        );

        let error = execute(&mut engine, "late_callable = @() 7;\nlate_wrapper();\n")
            .expect_err("resolver-only capture must skip a later workspace callable");
        assert_eq!(error.category(), "runtime.undefinedName");
        let range = error.diagnostics()[0]
            .range
            .as_ref()
            .expect("resolver-only source range");
        assert_eq!(
            range.source_name,
            fs::canonicalize(&main_path)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(range.start, source.find("late_callable").unwrap() as u64);
    }

    #[test]
    fn unknown_named_function_handle_ignores_workspace_shadow_and_is_located() {
        let directory = TestDirectory::new("unknown-named-handle");
        let main_path = directory.path().join("unknown_handle_case.m");
        let source = "absent_target = 7;\nmissing_handle = @absent_target;\nmissing_handle();\n";
        fs::write(&main_path, source).unwrap();
        let mut engine = RuntimeEngine::new().unwrap();

        let error = engine
            .execute_file(&main_path, &CancellationToken::new())
            .expect_err("invocation must not resolve a named handle through the workspace");
        assert_eq!(error.category(), "runtime.unknownFunctionHandleTarget");
        assert_eq!(error.diagnostics()[0].code.as_deref(), Some("OMR0018"));
        let range = error.diagnostics()[0]
            .range
            .as_ref()
            .expect("unknown target diagnostic range");
        assert_eq!(
            range.source_name,
            fs::canonicalize(&main_path)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(range.start, source.rfind("missing_handle").unwrap() as u64);
        assert_eq!(range.end, source.rfind(';').unwrap() as u64);
    }

    #[test]
    fn kernel_control_cancels_a_real_source_loop_through_anonymous_handle() {
        let mut kernel = Kernel::new(
            "runtime-session",
            RuntimeEngine::new().expect("runtime engine"),
        );
        let _ = kernel.handle_request(&initialize_request());
        let control = kernel.control();
        let started = Arc::new(AtomicBool::new(false));
        let worker_started = Arc::clone(&started);
        let execution = thread::spawn(move || {
            worker_started.store(true, Ordering::Release);
            kernel.handle_request(&request(
                "forever",
                Request::Execute(ExecuteRequest {
                    code: "forever = @() spin_forever();\nforever();\nfunction value = spin_forever()\nwhile 1\nend\nvalue = 0;\nend\n".to_owned(),
                    source_name: "forever.m".to_owned(),
                    mode: ExecutionMode::Repl,
                }),
            ))
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while (control.status() != KernelStatus::Busy || !started.load(Ordering::Acquire))
            && Instant::now() < deadline
        {
            thread::yield_now();
        }
        assert_eq!(control.status(), KernelStatus::Busy);
        let interrupted = control.handle_interrupt(&request(
            "interrupt",
            Request::Interrupt(InterruptRequest {}),
        ));
        assert!(response(&interrupted).ok);

        let execution = execution.join().expect("execution thread");
        let Some(ResponseResult::Execute(result)) = &response(&execution).result else {
            panic!("execute result");
        };
        assert!(result.interrupted);
    }

    #[test]
    fn shutdown_clears_workspace_and_rejects_direct_reuse() {
        let mut engine = RuntimeEngine::new().expect("runtime engine");
        execute(
            &mut engine,
            "classdef ShutdownValue\nproperties\nx = 1\nend\nend\n",
        )
        .unwrap();
        execute(&mut engine, "x = ShutdownValue();\n").unwrap();
        assert_eq!(engine.interpreter.class_count(), 2);
        assert!(!engine.interpreter.workspace().is_empty());
        engine.shutdown().unwrap();
        assert_eq!(engine.interpreter.class_count(), 1);
        assert_eq!(engine.interpreter.object_count(), 0);
        assert!(engine.interpreter.workspace().is_empty());
        assert_eq!(
            engine
                .execute(
                    &ExecuteRequest {
                        code: "y = 2;".to_owned(),
                        source_name: "after.m".to_owned(),
                        mode: ExecutionMode::File,
                    },
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .category(),
            "kernel.dead"
        );
    }

    #[test]
    fn classdef_value_objects_execute_across_requests() {
        let mut engine = RuntimeEngine::new().unwrap();
        execute(
            &mut engine,
            "classdef Point\nproperties\nx = 1\nend\nmethods\nfunction obj = Point(x)\nobj.x = x;\nend\nfunction obj = add(obj, delta)\nobj.x = obj.x + delta;\nend\nfunction value = read(obj)\nvalue = obj.x;\nend\nend\nend\n",
        )
        .unwrap();
        execute(
            &mut engine,
            "p = Point(5); q = p; p = p.add(2); r = q; r.x = 9; px = p.read(); qx = q.read(); rx = r.x; pc = class(p); pi = isa(p, \"Point\"); po = isobject(p);",
        )
        .unwrap();

        assert_eq!(
            engine.interpreter.workspace().get("px"),
            Some(&Value::Double(7.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("qx"),
            Some(&Value::Double(5.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("rx"),
            Some(&Value::Double(9.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("pc"),
            Some(&char_row("Point"))
        );
        assert_eq!(
            engine.interpreter.workspace().get("pi"),
            Some(&Value::Logical(true))
        );
        assert_eq!(
            engine.interpreter.workspace().get("po"),
            Some(&Value::Logical(true))
        );
        let summaries = engine.list_workspace().unwrap();
        assert_eq!(
            summaries
                .iter()
                .find(|summary| summary.name == "p")
                .map(|summary| summary.class.as_str()),
            Some("Point")
        );
    }

    #[test]
    fn classdef_handle_objects_preserve_aliases_across_requests() {
        let mut engine = RuntimeEngine::new().unwrap();
        execute(
            &mut engine,
            "classdef Counter < handle\nproperties\nvalue = 0\nend\nmethods\nfunction obj = Counter(value)\nobj.value = value;\nend\nfunction bump(obj, delta)\nobj.value = obj.value + delta;\nend\nfunction value = read(obj)\nvalue = obj.value;\nend\nend\nend\n",
        )
        .unwrap();
        execute(&mut engine, "a = Counter(3); b = a;").unwrap();
        execute(
            &mut engine,
            "a.bump(4); observed = b.read(); b.value = 11; observed_after_set = a.read(); is_handle = isa(a, \"handle\");",
        )
        .unwrap();

        assert_eq!(
            engine.interpreter.workspace().get("observed"),
            Some(&Value::Double(7.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("observed_after_set"),
            Some(&Value::Double(11.0))
        );
        assert_eq!(
            engine.interpreter.workspace().get("is_handle"),
            Some(&Value::Logical(true))
        );
    }

    #[test]
    fn kernel_classdef_lifecycle_lists_inspects_and_shuts_down_objects() {
        let mut kernel = Kernel::new(
            "runtime-session",
            RuntimeEngine::new().expect("runtime engine"),
        );
        let _ = kernel.handle_request(&initialize_request());
        let registered = kernel.handle_request(&request(
            "class-file",
            Request::Execute(ExecuteRequest {
                code: "classdef Sample\nproperties\ndata = [10 20]\nlabel = \"ready\"\nend\nend\n"
                    .to_owned(),
                source_name: "Sample.m".to_owned(),
                mode: ExecutionMode::File,
            }),
        ));
        assert!(response(&registered).ok, "{:?}", response(&registered));
        let executed = kernel.handle_request(&request(
            "use-class",
            Request::Execute(ExecuteRequest {
                code: "sample = Sample(); second = sample.data(2); label = sample.label;"
                    .to_owned(),
                source_name: "use_sample.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        ));
        assert!(response(&executed).ok);

        let listed = kernel.handle_request(&request(
            "list-class",
            Request::ListWorkspace(ListWorkspaceRequest {}),
        ));
        let Some(ResponseResult::ListWorkspace(workspace)) = &response(&listed).result else {
            panic!("workspace result");
        };
        let sample = workspace
            .variables
            .iter()
            .find(|variable| variable.name == "sample")
            .expect("sample summary");
        assert_eq!(sample.class, "Sample");
        assert_eq!(sample.dimensions, vec![1, 1]);

        let inspected = kernel.handle_request(&request(
            "inspect-class",
            Request::Inspect(scalar_inspect("sample")),
        ));
        let Some(ResponseResult::Inspect(preview)) = &response(&inspected).result else {
            panic!("object preview");
        };
        assert_eq!(preview.class, "Sample");
        assert_eq!(preview.values, vec![PreviewValue::Missing]);

        let shutdown = kernel.handle_request(&request(
            "shutdown-class",
            Request::Shutdown(ShutdownRequest {}),
        ));
        assert!(response(&shutdown).ok);
        assert_eq!(kernel.status(), KernelStatus::Dead);
    }

    #[test]
    fn method_runtime_error_keeps_class_file_range_and_stack_locations() {
        let mut engine = RuntimeEngine::new().unwrap();
        engine
            .execute(
                &ExecuteRequest {
                    code: "classdef Bomb\nmethods\nfunction obj = Bomb()\nend\nfunction fail(obj)\nmissing_inside_method;\nend\nend\nend\n"
                        .to_owned(),
                    source_name: "Bomb.m".to_owned(),
                    mode: ExecutionMode::File,
                },
                &CancellationToken::new(),
            )
            .unwrap();
        let error = engine
            .execute(
                &ExecuteRequest {
                    code: "bomb = Bomb(); bomb.fail();".to_owned(),
                    source_name: "use_bomb.m".to_owned(),
                    mode: ExecutionMode::Repl,
                },
                &CancellationToken::new(),
            )
            .expect_err("method body should reference an undefined name");
        let diagnostic = error.diagnostics().first().expect("runtime diagnostic");

        assert_eq!(error.category(), "runtime.undefinedName");
        assert_eq!(
            diagnostic
                .range
                .as_ref()
                .map(|range| range.source_name.as_str()),
            Some("Bomb.m")
        );
        assert!(diagnostic.related.iter().any(|related| {
            related.range.source_name == "Bomb.m" && related.message.contains("Bomb.fail")
        }));
        assert!(diagnostic.related.iter().any(|related| {
            related.range.source_name == "use_bomb.m" && related.message.contains("<entry>")
        }));
    }

    #[test]
    fn homogeneous_object_array_literal_executes_and_heterogeneous_literal_is_structured() {
        let mut engine = RuntimeEngine::new().unwrap();
        execute(
            &mut engine,
            "classdef ArrayMember\nproperties\nx = 1\nend\nend\n",
        )
        .unwrap();
        execute(&mut engine, "a = ArrayMember(); items = [a a];").unwrap();
        let items = engine
            .interpreter
            .workspace()
            .get("items")
            .expect("object matrix binding");
        assert_eq!(engine.interpreter.value_class_name(items), "ArrayMember");
        assert_eq!(items.dimensions(), Some([1, 2].as_slice()));

        execute(&mut engine, "classdef OtherMember\nend\n").unwrap();
        let error = execute(
            &mut engine,
            "other = OtherMember(); heterogeneous = [a other];",
        )
        .expect_err("heterogeneous object literal must fail");

        assert_eq!(error.category(), "runtime.object");
    }

    #[test]
    fn indexed_assignment_builds_homogeneous_value_object_array_end_to_end() {
        let mut engine = RuntimeEngine::new().unwrap();
        execute(
            &mut engine,
            "classdef OpenMatValueCounter\nproperties\nValue\nend\nmethods\nfunction object = OpenMatValueCounter(initialValue)\nobject.Value = initialValue;\nend\nend\nend\n",
        )
        .unwrap();
        let execution = execute(
            &mut engine,
            "objects(1)=OpenMatValueCounter(2); objects(2)=OpenMatValueCounter(5); openmat_result=objects;",
        )
        .unwrap();
        let delta = execution
            .events
            .iter()
            .find_map(|event| match event {
                Event::WorkspaceDelta(delta) => Some(delta),
                _ => None,
            })
            .expect("object-array workspace delta");
        let added_result = delta
            .added
            .iter()
            .find(|variable| variable.name == "openmat_result")
            .expect("object-array delta summary");
        assert_eq!(added_result.class, "OpenMatValueCounter");
        assert_eq!(added_result.dimensions, vec![1, 2]);

        let result = engine
            .interpreter
            .workspace()
            .get("openmat_result")
            .expect("conformance result");
        let Value::ObjectArray(array) = result else {
            panic!("indexed assignment must produce a dedicated object array");
        };
        assert_eq!(
            engine.interpreter.value_class_name(result),
            "OpenMatValueCounter"
        );
        assert_eq!(array.shape().dimensions(), &[1, 2]);
        assert_eq!(array.numel(), 2);
        assert_ne!(array.as_slice()[0], array.as_slice()[1]);

        let summary = engine
            .list_workspace()
            .unwrap()
            .into_iter()
            .find(|variable| variable.name == "openmat_result")
            .expect("object array summary");
        assert_eq!(summary.class, "OpenMatValueCounter");
        assert_eq!(summary.dimensions, vec![1, 2]);
        let preview = engine.inspect(&row_inspect("openmat_result", 2)).unwrap();
        assert_eq!(preview.class, "OpenMatValueCounter");
        assert_eq!(preview.dimensions, vec![1, 2]);
        assert_eq!(
            preview.values,
            vec![PreviewValue::Missing, PreviewValue::Missing]
        );

        execute(
            &mut engine,
            "first=objects(1); first.Value=99; unchanged=objects(1); observed=unchanged.Value;",
        )
        .unwrap();
        assert_eq!(
            engine.interpreter.workspace().get("observed"),
            Some(&Value::Double(2.0))
        );

        execute(&mut engine, "classdef OtherCounter\nend\n").unwrap();
        let error = execute(&mut engine, "other=OtherCounter(); objects(2)=other;")
            .expect_err("heterogeneous indexed assignment must fail");

        assert_eq!(error.category(), "runtime.object");
    }
}

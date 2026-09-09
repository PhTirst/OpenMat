use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    num::NonZeroUsize,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};

use openmat_array::{ArrayData, DenseArray, Logical};
use openmat_fft::{FftProvider, RustFftProvider};
use openmat_graphics_model::{
    GraphicsError, GraphicsExecution, GraphicsRequest, GraphicsResponse, GraphicsSession,
};
use openmat_linalg::{LinalgProvider, ReferenceProvider};
use openmat_text::{FancyRegexProvider, RegexProvider};
use openmat_value::{BuiltinHandle, Value};

use crate::{
    CancellationToken, DEFAULT_MAX_FILE_BYTES, DisplayFormat, FileOpenMode, FileSeekOrigin,
    FileSystemMetadata, FileSystemService, OpenFileInfo, OutputEvent, OutputSink, RandomService,
    RandomSnapshot, Workspace,
};

pub(crate) trait LanguageService {
    fn copy(&mut self, value: &Value) -> Result<Value, BuiltinError>;

    fn class_name(&self, value: &Value) -> String {
        value.class_name().to_owned()
    }

    fn invoke(
        &mut self,
        callable: &Value,
        arguments: Vec<Value>,
        requested_outputs: usize,
    ) -> BuiltinResult;

    fn display_format(&self) -> DisplayFormat;

    fn set_display_format(&mut self, format: DisplayFormat);

    fn warning_enabled(&self, identifier: &str) -> bool;

    fn set_warning_enabled(&mut self, identifier: Option<&str>, enabled: bool) -> bool;

    fn last_warning(&self) -> (&str, &str);

    fn set_last_warning(&mut self, message: String, identifier: String);
}

/// One object-lifecycle operation delegated by a built-in to the owning runtime.
///
/// The value carries only opaque runtime object handles. Implementations must
/// resolve their classes, semantics, construction state, and alias validity
/// against the runtime-owned object model; built-ins never receive that store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ObjectLifecycleRequest<'a> {
    /// Query every element of a handle scalar or homogeneous handle array.
    IsValid(&'a Value),
    /// Perform language-level explicit deletion or ordinary `delete` dispatch.
    Delete(&'a Value),
}

/// Successful acceptance result of an [`ObjectLifecycleRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectLifecycleResult {
    /// Column-major validity bits, one for each requested handle element.
    Validity(Vec<bool>),
    /// A column-major linear handle deletion was accepted by the runtime.
    ///
    /// `elements` counts processed array elements, not unique identities; all
    /// aliases of every processed identity must be invalid after completion.
    HandleDeleteRequested {
        /// Number of scalar or array elements scheduled for explicit deletion.
        elements: usize,
    },
    /// Ordinary value-class `delete` method dispatch was accepted by the runtime.
    ValueDeleteMethodRequested,
}

/// Narrow runtime-owned service for object validity and explicit deletion.
///
/// `Delete` must resolve the receiver's effective class semantics. The service
/// may execute immediately or stage work, but the owning runtime must complete
/// accepted work before committing the successful built-in invocation. For a
/// handle class it first invalidates every public alias, then invokes the
/// applicable dynamic `delete` method in column-major linear order. During
/// that method only the runtime-controlled receiver may read the invalidated
/// object's state; ordinary aliases remain invalid. For a value class it
/// performs ordinary dynamic `delete` dispatch and must neither invalidate the
/// value nor install finalizer or garbage-collection semantics. Any
/// method/deletion failure replaces the provisional built-in success.
/// Homogeneous object arrays retain their exact shape in [`Value`]; `Validity`
/// returns elements in column-major order.
pub trait ObjectLifecycleService {
    /// Executes one object-lifecycle request without exposing object storage.
    ///
    /// # Errors
    ///
    /// Returns a structured `Type` or `Domain` error for a non-object receiver,
    /// a value-class `isvalid` request, a heterogeneous or otherwise invalid
    /// array, or a rejected method dispatch. Runtime failures may use another
    /// appropriate structured category.
    fn request(
        &mut self,
        request: ObjectLifecycleRequest<'_>,
    ) -> Result<ObjectLifecycleResult, BuiltinError>;
}

#[derive(Clone, Debug)]
pub(crate) struct WarningState {
    default_enabled: bool,
    overrides: BTreeMap<String, bool>,
    last_message: String,
    last_identifier: String,
}

impl Default for WarningState {
    fn default() -> Self {
        Self {
            default_enabled: true,
            overrides: BTreeMap::new(),
            last_message: String::new(),
            last_identifier: String::new(),
        }
    }
}

impl WarningState {
    pub(crate) fn enabled(&self, identifier: &str) -> bool {
        self.overrides
            .get(identifier)
            .copied()
            .unwrap_or(self.default_enabled)
    }

    pub(crate) fn set_enabled(&mut self, identifier: Option<&str>, enabled: bool) -> bool {
        let previous = identifier
            .and_then(|identifier| self.overrides.get(identifier).copied())
            .unwrap_or(self.default_enabled);
        if let Some(identifier) = identifier.filter(|identifier| *identifier != "all") {
            self.overrides.insert(identifier.to_owned(), enabled);
        } else {
            self.default_enabled = enabled;
            self.overrides.clear();
        }
        previous
    }

    pub(crate) fn last(&self) -> (&str, &str) {
        (&self.last_message, &self.last_identifier)
    }

    pub(crate) fn set_last(&mut self, message: String, identifier: String) {
        self.last_message = message;
        self.last_identifier = identifier;
    }

    /// Records one runtime warning and returns whether it should be displayed.
    ///
    /// `lastwarn` state is updated even when the identifier is disabled. The
    /// caller uses the result solely to decide whether to emit `CommandText`.
    #[allow(dead_code)]
    pub(crate) fn record(
        &mut self,
        message: impl Into<String>,
        identifier: impl Into<String>,
    ) -> bool {
        let message = message.into();
        let identifier = identifier.into();
        let enabled = self
            .overrides
            .get(&identifier)
            .copied()
            .unwrap_or(self.default_enabled);
        self.last_message = message;
        self.last_identifier = identifier;
        enabled
    }
}

/// Narrow session graphics service injected into built-in invocations.
pub trait GraphicsService {
    /// Returns the current colormap length without creating graphics state.
    fn current_colormap_length(&self) -> Option<usize>;

    /// Executes and commits one graphics builtin transaction.
    ///
    /// # Errors
    ///
    /// Returns a structured model error without a partial transaction.
    fn execute(&mut self, request: GraphicsRequest) -> Result<GraphicsExecution, GraphicsError>;
}

impl GraphicsService for GraphicsSession {
    fn current_colormap_length(&self) -> Option<usize> {
        GraphicsSession::current_colormap_length(self)
    }

    fn execute(&mut self, request: GraphicsRequest) -> Result<GraphicsExecution, GraphicsError> {
        GraphicsSession::execute(self, request)
    }
}

static REFERENCE_LINALG_PROVIDER: ReferenceProvider = ReferenceProvider;
static DEFAULT_FFT_PROVIDER: LazyLock<RustFftProvider> = LazyLock::new(RustFftProvider::default);
static DEFAULT_REGEX_PROVIDER: FancyRegexProvider = FancyRegexProvider;

/// A broad built-in failure category suitable for structured runtime errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinErrorCategory {
    /// The number of input arguments is unsupported.
    ArgumentCount,
    /// An input has an unsupported dynamic type.
    Type,
    /// An otherwise valid input lies outside the function's domain.
    Domain,
    /// The host output boundary rejected an event.
    Output,
    /// Execution was cooperatively cancelled.
    Cancelled,
    /// The runtime's object-aware language-copy service rejected the copy.
    LanguageCopy,
    /// A required host/session service is unavailable.
    HostService,
    /// A language-level host filesystem operation failed.
    FileSystem,
    /// The graphics model rejected a request.
    Graphics,
    /// A built-in-specific failure not covered above.
    Other,
}

/// A structured failure returned by a built-in implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinError {
    /// Broad error category.
    pub category: BuiltinErrorCategory,
    /// Optional language-visible identifier preserved by `try`/`catch`.
    ///
    /// `None` uses the runtime's stable built-in identifier. `Some("")` is
    /// distinct and represents MATLAB's valid identifier-less user error.
    pub identifier: Option<String>,
    /// OpenMat-owned diagnostic detail.
    pub message: String,
}

impl BuiltinError {
    /// Creates a built-in failure.
    #[must_use]
    pub fn new(category: BuiltinErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            identifier: None,
            message: message.into(),
        }
    }

    /// Attaches the exact language-visible identifier used by a caught error.
    #[must_use]
    pub fn with_identifier(mut self, identifier: impl Into<String>) -> Self {
        self.identifier = Some(identifier.into());
        self
    }
}

impl fmt::Display for BuiltinError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for BuiltinError {}

/// Return type shared by built-in implementations.
pub type BuiltinResult = Result<Vec<Value>, BuiltinError>;

/// Process-local identity of one registered native class implementation.
///
/// OEX uses the address of a plugin-defined static type token. The runtime treats the number as
/// opaque and never dereferences it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeClassToken(NonZeroUsize);

impl NativeClassToken {
    /// Wraps one non-zero process-local class token.
    #[must_use]
    pub const fn new(identifier: NonZeroUsize) -> Self {
        Self(identifier)
    }

    /// Returns the opaque process-local token.
    #[must_use]
    pub const fn identifier(self) -> NonZeroUsize {
        self.0
    }
}

/// Opaque native instance identity owned by a registered native class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeInstance(NonZeroUsize);

impl NativeInstance {
    /// Wraps one non-zero plugin-defined identity.
    #[must_use]
    pub const fn new(identifier: NonZeroUsize) -> Self {
        Self(identifier)
    }

    /// Returns the plugin-defined identity.
    #[must_use]
    pub const fn identifier(self) -> NonZeroUsize {
        self.0
    }
}

/// Runtime-owned bridge between language object handles and native instances.
///
/// A borrowed instance remains valid only for the active built-in callback. Creating an object
/// transfers the native instance to the runtime only when this service returns successfully.
pub trait NativeObjectService {
    /// Resolves one scalar language object and verifies its exact native class token.
    ///
    /// # Errors
    ///
    /// Returns a structured type, validity, cancellation, or host-service failure.
    fn borrow_instance(
        &mut self,
        value: &Value,
        expected_class: NativeClassToken,
    ) -> Result<NativeInstance, BuiltinError>;

    /// Stages one newly allocated native instance as an ordinary language object value.
    ///
    /// # Errors
    ///
    /// Returns a structured class-token, allocation, identity, or host-service failure.
    fn create_instance(
        &mut self,
        class: NativeClassToken,
        instance: NativeInstance,
    ) -> Result<Value, BuiltinError>;
}

/// One public instance method exported by a native class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeMethod {
    name: Arc<str>,
}

/// One public dependent property exported by a native handle class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeProperty {
    name: Arc<str>,
    readable: bool,
    writable: bool,
}

impl NativeProperty {
    /// Creates property metadata. Registration rejects an empty name or a property that is
    /// neither readable nor writable.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>, readable: bool, writable: bool) -> Self {
        Self {
            name: name.into(),
            readable,
            writable,
        }
    }

    /// Returns the language-visible property name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether property reads are supported.
    #[must_use]
    pub const fn readable(&self) -> bool {
        self.readable
    }

    /// Returns whether property writes are supported.
    #[must_use]
    pub const fn writable(&self) -> bool {
        self.writable
    }
}

impl NativeMethod {
    /// Creates method metadata. Registration rejects an empty or duplicate name.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self { name: name.into() }
    }

    /// Returns the language-visible method name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Runtime-independent implementation of one native handle class.
///
/// Instances have ordinary `OpenMat` object identity and participate in the existing handle-class
/// validity and garbage-collection lifecycle. The opaque native identity is never exposed at the
/// language boundary.
pub trait NativeClass: Send + Sync {
    /// Returns the process-local identity used for checked native-object bridging.
    fn token(&self) -> NativeClassToken;

    /// Returns the language-visible class name.
    fn name(&self) -> &str;

    /// Returns all public instance methods implemented by this class.
    fn methods(&self) -> &[NativeMethod];

    /// Returns all public dependent properties implemented by this class.
    fn properties(&self) -> &[NativeProperty];

    /// Constructs one native instance from owned language arguments.
    ///
    /// # Errors
    ///
    /// Returns a structured callback failure.
    fn construct(
        &self,
        arguments: Vec<Value>,
        context: &mut BuiltinContext<'_>,
    ) -> Result<NativeInstance, BuiltinError>;

    /// Invokes one instance method with owned user arguments. The receiver is represented by
    /// `instance` and is not repeated in `arguments`.
    ///
    /// # Errors
    ///
    /// Returns a structured callback failure.
    fn invoke(
        &self,
        method: &str,
        instance: NativeInstance,
        arguments: Vec<Value>,
        context: &mut BuiltinContext<'_>,
    ) -> BuiltinResult;

    /// Reads one native dependent property.
    ///
    /// # Errors
    ///
    /// Returns a structured callback failure.
    fn get_property(
        &self,
        property: &str,
        instance: NativeInstance,
        context: &mut BuiltinContext<'_>,
    ) -> Result<Value, BuiltinError>;

    /// Writes one native dependent property.
    ///
    /// # Errors
    ///
    /// Returns a structured callback failure.
    fn set_property(
        &self,
        property: &str,
        instance: NativeInstance,
        value: Value,
        context: &mut BuiltinContext<'_>,
    ) -> Result<(), BuiltinError>;

    /// Releases one successfully constructed instance exactly once.
    fn destroy(&self, instance: NativeInstance);
}

/// Services available to one built-in invocation.
pub struct BuiltinContext<'a> {
    requested_outputs: usize,
    cancellation: &'a CancellationToken,
    output: &'a mut dyn OutputSink,
    linalg_provider: &'a dyn LinalgProvider,
    fft_provider: &'a dyn FftProvider,
    regex_provider: &'a dyn RegexProvider,
    language: Option<&'a mut dyn LanguageService>,
    graphics: Option<&'a mut dyn GraphicsService>,
    random: Option<&'a mut dyn RandomService>,
    file_system: Option<&'a mut dyn FileSystemService>,
    object_lifecycle: Option<&'a mut dyn ObjectLifecycleService>,
    native_objects: Option<&'a mut dyn NativeObjectService>,
    workspace: Option<&'a Workspace>,
    binding_effects: Option<&'a mut Vec<(String, Value)>>,
    caller_source: Option<&'a str>,
}

impl<'a> BuiltinContext<'a> {
    /// Creates an invocation context.
    #[must_use]
    pub fn new(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider: &REFERENCE_LINALG_PROVIDER,
            fft_provider: &*DEFAULT_FFT_PROVIDER,
            regex_provider: &DEFAULT_REGEX_PROVIDER,
            language: None,
            graphics: None,
            random: None,
            file_system: None,
            object_lifecycle: None,
            native_objects: None,
            workspace: None,
            binding_effects: None,
            caller_source: None,
        }
    }

    /// Creates an invocation context with an explicitly selected numerical provider.
    ///
    /// This constructor is intended for standalone hosts and focused built-in
    /// tests. Interpreter-created contexts always borrow the provider selected
    /// for that runtime session.
    #[must_use]
    pub fn with_linalg_provider(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
        linalg_provider: &'a dyn LinalgProvider,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider,
            fft_provider: &*DEFAULT_FFT_PROVIDER,
            regex_provider: &DEFAULT_REGEX_PROVIDER,
            language: None,
            graphics: None,
            random: None,
            file_system: None,
            object_lifecycle: None,
            native_objects: None,
            workspace: None,
            binding_effects: None,
            caller_source: None,
        }
    }

    /// Creates an invocation context with an explicitly selected FFT provider.
    ///
    /// This constructor is intended for standalone hosts and focused built-in
    /// tests. Interpreter-created contexts borrow the provider selected for the
    /// runtime session.
    #[must_use]
    pub fn with_fft_provider(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
        fft_provider: &'a dyn FftProvider,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider: &REFERENCE_LINALG_PROVIDER,
            fft_provider,
            regex_provider: &DEFAULT_REGEX_PROVIDER,
            language: None,
            graphics: None,
            random: None,
            file_system: None,
            object_lifecycle: None,
            native_objects: None,
            workspace: None,
            binding_effects: None,
            caller_source: None,
        }
    }

    /// Creates an invocation context with an explicitly selected regex provider.
    #[must_use]
    pub fn with_regex_provider(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
        regex_provider: &'a dyn RegexProvider,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider: &REFERENCE_LINALG_PROVIDER,
            fft_provider: &*DEFAULT_FFT_PROVIDER,
            regex_provider,
            language: None,
            graphics: None,
            random: None,
            file_system: None,
            object_lifecycle: None,
            native_objects: None,
            workspace: None,
            binding_effects: None,
            caller_source: None,
        }
    }

    /// Creates a standalone context with an explicitly owned graphics service.
    #[must_use]
    pub fn with_graphics_service(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
        graphics: &'a mut dyn GraphicsService,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider: &REFERENCE_LINALG_PROVIDER,
            fft_provider: &*DEFAULT_FFT_PROVIDER,
            regex_provider: &DEFAULT_REGEX_PROVIDER,
            language: None,
            graphics: Some(graphics),
            random: None,
            file_system: None,
            object_lifecycle: None,
            native_objects: None,
            workspace: None,
            binding_effects: None,
            caller_source: None,
        }
    }

    /// Creates a standalone context with an explicitly owned random service.
    #[must_use]
    pub fn with_random_service(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
        random: &'a mut dyn RandomService,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider: &REFERENCE_LINALG_PROVIDER,
            fft_provider: &*DEFAULT_FFT_PROVIDER,
            regex_provider: &DEFAULT_REGEX_PROVIDER,
            language: None,
            graphics: None,
            random: Some(random),
            file_system: None,
            object_lifecycle: None,
            native_objects: None,
            workspace: None,
            binding_effects: None,
            caller_source: None,
        }
    }

    /// Creates a standalone context with an explicitly selected filesystem service.
    #[must_use]
    pub fn with_file_system_service(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
        file_system: &'a mut dyn FileSystemService,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider: &REFERENCE_LINALG_PROVIDER,
            fft_provider: &*DEFAULT_FFT_PROVIDER,
            regex_provider: &DEFAULT_REGEX_PROVIDER,
            language: None,
            graphics: None,
            random: None,
            file_system: Some(file_system),
            object_lifecycle: None,
            native_objects: None,
            workspace: None,
            binding_effects: None,
            caller_source: None,
        }
    }

    /// Creates a standalone context with an explicitly selected object-lifecycle service.
    #[must_use]
    pub fn with_object_lifecycle_service(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
        object_lifecycle: &'a mut dyn ObjectLifecycleService,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider: &REFERENCE_LINALG_PROVIDER,
            fft_provider: &*DEFAULT_FFT_PROVIDER,
            regex_provider: &DEFAULT_REGEX_PROVIDER,
            language: None,
            graphics: None,
            random: None,
            file_system: None,
            object_lifecycle: Some(object_lifecycle),
            native_objects: None,
            workspace: None,
            binding_effects: None,
            caller_source: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_services(
        requested_outputs: usize,
        cancellation: &'a CancellationToken,
        output: &'a mut dyn OutputSink,
        linalg_provider: &'a dyn LinalgProvider,
        fft_provider: &'a dyn FftProvider,
        regex_provider: &'a dyn RegexProvider,
        language: &'a mut dyn LanguageService,
        graphics: &'a mut dyn GraphicsService,
        random: &'a mut dyn RandomService,
        file_system: Option<&'a mut dyn FileSystemService>,
        workspace: &'a Workspace,
        binding_effects: &'a mut Vec<(String, Value)>,
        caller_source: Option<&'a str>,
    ) -> Self {
        Self {
            requested_outputs,
            cancellation,
            output,
            linalg_provider,
            fft_provider,
            regex_provider,
            language: Some(language),
            graphics: Some(graphics),
            random: Some(random),
            file_system,
            object_lifecycle: None,
            native_objects: None,
            workspace: Some(workspace),
            binding_effects: Some(binding_effects),
            caller_source,
        }
    }

    /// Attaches the runtime-owned lifecycle service to a regular interpreter context.
    ///
    /// Standalone constructors intentionally leave this service absent. The
    /// interpreter calls this immediately after [`Self::with_services`] and
    /// before invoking the built-in function.
    #[allow(dead_code)]
    pub(crate) fn set_object_lifecycle_service(
        &mut self,
        object_lifecycle: &'a mut dyn ObjectLifecycleService,
    ) {
        self.object_lifecycle = Some(object_lifecycle);
    }

    /// Attaches the runtime-owned native-object bridge to an interpreter invocation.
    pub(crate) fn set_native_object_service(
        &mut self,
        native_objects: &'a mut dyn NativeObjectService,
    ) {
        self.native_objects = Some(native_objects);
    }

    /// Returns the number of outputs requested by the caller.
    #[must_use]
    pub const fn requested_outputs(&self) -> usize {
        self.requested_outputs
    }

    /// Returns the canonical source file that contains this invocation, when any.
    #[must_use]
    pub const fn caller_source(&self) -> Option<&str> {
        self.caller_source
    }

    /// Returns the cooperative-cancellation token.
    #[must_use]
    pub const fn cancellation(&self) -> &CancellationToken {
        self.cancellation
    }

    /// Returns the numerical provider selected for this invocation.
    #[must_use]
    pub const fn linalg_provider(&self) -> &dyn LinalgProvider {
        self.linalg_provider
    }

    /// Returns the Fourier-transform provider selected for this invocation.
    #[must_use]
    pub const fn fft_provider(&self) -> &dyn FftProvider {
        self.fft_provider
    }

    /// Returns the regular-expression provider selected for this invocation.
    #[must_use]
    pub const fn regex_provider(&self) -> &dyn RegexProvider {
        self.regex_provider
    }

    /// Borrows the provider-compatible cooperative-cancellation flag.
    #[must_use]
    pub fn cancellation_flag(&self) -> &AtomicBool {
        self.cancellation.atomic_flag()
    }

    /// Returns a cancellation error if interruption has been requested.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled` after the token has been signalled.
    pub fn check_cancelled(&self) -> Result<(), BuiltinError> {
        if self.cancellation.is_cancelled() {
            Err(BuiltinError::new(
                BuiltinErrorCategory::Cancelled,
                "execution was cancelled",
            ))
        } else {
            Ok(())
        }
    }

    /// Emits a structured output event.
    ///
    /// # Errors
    ///
    /// Maps host output failures to the built-in output category.
    pub fn emit(&mut self, event: OutputEvent) -> Result<(), BuiltinError> {
        self.output
            .emit(event)
            .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Output, error.message))
    }

    /// Returns the immutable workspace snapshot taken at built-in entry.
    ///
    /// # Errors
    ///
    /// Returns `HostService` for standalone contexts that have no owning
    /// interpreter session.
    pub fn workspace(&self) -> Result<&Workspace, BuiltinError> {
        self.workspace.ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::HostService,
                "the session workspace snapshot is unavailable in this context",
            )
        })
    }

    /// Stages one name/value update in the caller's current language scope.
    ///
    /// All staged updates are committed together only after the built-in
    /// returns successfully and cancellation remains clear. The value crosses
    /// the normal object-aware language-copy boundary before it is staged.
    ///
    /// # Errors
    ///
    /// Returns a host-service error for standalone contexts, a language-copy
    /// error when copying fails, or a domain error for an invalid name.
    pub fn stage_scope_binding(
        &mut self,
        name: impl Into<String>,
        value: &Value,
    ) -> Result<(), BuiltinError> {
        let name = name.into();
        if name.is_empty() || name.as_bytes().contains(&0) {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "a staged scope binding must have a nonempty NUL-free name",
            ));
        }
        let value = self.language_copy(value)?;
        let effects = self.binding_effects.as_deref_mut().ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::HostService,
                "current-scope mutation is unavailable in this context",
            )
        })?;
        effects.try_reserve(1).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::LanguageCopy,
                "the staged scope-binding transaction exceeded runtime limits",
            )
        })?;
        effects.push((name, value));
        Ok(())
    }

    /// Returns the current per-session command-window display settings.
    ///
    /// # Errors
    ///
    /// Returns `HostService` for standalone contexts.
    pub fn display_format(&self) -> Result<DisplayFormat, BuiltinError> {
        self.language
            .as_deref()
            .map(LanguageService::display_format)
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session display format is unavailable in this context",
                )
            })
    }

    /// Replaces the per-session command-window display settings.
    ///
    /// # Errors
    ///
    /// Returns `HostService` for standalone contexts.
    pub fn set_display_format(&mut self, format: DisplayFormat) -> Result<(), BuiltinError> {
        self.language
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session display format is unavailable in this context",
                )
            })?
            .set_display_format(format);
        Ok(())
    }

    /// Returns whether a warning identifier is enabled in this runtime session.
    ///
    /// # Errors
    ///
    /// Returns `HostService` for a standalone context without session state.
    pub fn warning_enabled(&self, identifier: &str) -> Result<bool, BuiltinError> {
        self.language
            .as_deref()
            .map(|language| language.warning_enabled(identifier))
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session warning state is unavailable in this context",
                )
            })
    }

    /// Enables or disables one identifier, or the session default when `identifier` is `None`.
    ///
    /// Returns the prior effective state. Changing the default clears identifier overrides,
    /// matching MATLAB's `warning('on'|'off','all')` reset behavior.
    ///
    /// # Errors
    ///
    /// Returns `HostService` for a standalone context without session state.
    pub fn set_warning_enabled(
        &mut self,
        identifier: Option<&str>,
        enabled: bool,
    ) -> Result<bool, BuiltinError> {
        self.language
            .as_deref_mut()
            .map(|language| language.set_warning_enabled(identifier, enabled))
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session warning state is unavailable in this context",
                )
            })
    }

    /// Returns the last warning message and identifier for this runtime session.
    ///
    /// # Errors
    ///
    /// Returns `HostService` for a standalone context without session state.
    pub fn last_warning(&self) -> Result<(&str, &str), BuiltinError> {
        self.language
            .as_deref()
            .map(LanguageService::last_warning)
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session warning state is unavailable in this context",
                )
            })
    }

    /// Replaces the session's last warning message and identifier.
    ///
    /// # Errors
    ///
    /// Returns `HostService` for a standalone context without session state.
    pub fn set_last_warning(
        &mut self,
        message: impl Into<String>,
        identifier: impl Into<String>,
    ) -> Result<(), BuiltinError> {
        self.language
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session warning state is unavailable in this context",
                )
            })?
            .set_last_warning(message.into(), identifier.into());
        Ok(())
    }

    /// Copies a value across the language assignment boundary.
    ///
    /// Interpreter-created contexts inject the session's object-aware copy
    /// transaction. Repeated calls therefore create independent value-class
    /// objects while preserving handle-class identity. Standalone contexts may
    /// copy ordinary copy-on-write values, but reject objects because they do
    /// not own an object store and must not guess object semantics.
    ///
    /// # Errors
    ///
    /// Returns a structured language-copy error when object-aware copying is
    /// unavailable or the runtime copy transaction fails.
    pub fn language_copy(&mut self, value: &Value) -> Result<Value, BuiltinError> {
        if let Some(language) = &mut self.language {
            return language.copy(value);
        }
        let mut contains_objects = matches!(value, Value::Object(_) | Value::ObjectArray(_));
        value.trace_object_handles(&mut |_| contains_objects = true);
        if contains_objects {
            Err(BuiltinError::new(
                BuiltinErrorCategory::LanguageCopy,
                "object-aware language copy is unavailable in this context",
            ))
        } else {
            Ok(value.clone())
        }
    }

    /// Returns the runtime-resolved class name, including registered objects.
    /// Standalone contexts reject objects whose class requires an interpreter.
    ///
    /// # Errors
    ///
    /// Returns `HostService` when an object class is requested without an
    /// interpreter-backed language service.
    pub fn language_class_name(&self, value: &Value) -> Result<String, BuiltinError> {
        if let Some(language) = &self.language {
            return Ok(language.class_name(value));
        }
        if matches!(value, Value::Object(_) | Value::ObjectArray(_)) {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::HostService,
                "object class lookup requires an interpreter",
            ));
        }
        Ok(value.class_name().to_owned())
    }

    /// Invokes any language-callable value in the owning interpreter.
    ///
    /// This is the low-level callback bridge used by numerical built-ins and native plugins:
    /// bytecode functions, anonymous or named function handles, built-ins, class constructors,
    /// and intrinsics all follow the same runtime dispatch as an ordinary language call.
    /// Standalone contexts reject the operation because they have no owning interpreter.
    ///
    /// A callback failure poisons the current built-in invocation. Returning success after an
    /// invocation error cannot hide the original language exception or cancellation.
    ///
    /// # Errors
    ///
    /// Returns `HostService` when no interpreter callback service is available, `Cancelled`
    /// after cooperative cancellation, or the structured callback failure.
    pub fn invoke_owned(
        &mut self,
        callable: &Value,
        arguments: Vec<Value>,
        requested_outputs: usize,
    ) -> BuiltinResult {
        self.check_cancelled()?;
        self.language
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "language callback invocation is unavailable in this context",
                )
            })?
            .invoke(callable, arguments, requested_outputs)
    }

    /// Invokes a language-callable value after cloning the argument handles.
    ///
    /// Array storage remains copy-on-write; use [`Self::invoke_owned`] when the caller can hand
    /// over an existing argument vector directly.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::invoke_owned`].
    pub fn invoke(
        &mut self,
        callable: &Value,
        arguments: &[Value],
        requested_outputs: usize,
    ) -> BuiltinResult {
        self.invoke_owned(callable, arguments.to_vec(), requested_outputs)
    }

    /// Borrows the plugin-defined identity behind one exact native handle-class value.
    ///
    /// The returned identity is valid only while this built-in invocation is active. Standalone
    /// contexts do not own an interpreter object table and reject the operation.
    ///
    /// # Errors
    ///
    /// Returns `Type` for a non-native object or class mismatch, `Domain` for an invalid object,
    /// or `HostService` when the runtime bridge is unavailable.
    pub fn borrow_native_instance(
        &mut self,
        value: &Value,
        expected_class: NativeClassToken,
    ) -> Result<NativeInstance, BuiltinError> {
        self.check_cancelled()?;
        self.native_objects
            .as_deref_mut()
            .ok_or_else(native_objects_unavailable)?
            .borrow_instance(value, expected_class)
    }

    /// Stages a newly allocated native instance as an ordinary handle-class value.
    ///
    /// Successful return transfers ownership of `instance` to the runtime. On failure the caller
    /// still owns it. The owning interpreter commits the staged object only when the complete
    /// built-in invocation succeeds.
    ///
    /// # Errors
    ///
    /// Returns `Type` for an unknown class token, `HostService` for a standalone context, or a
    /// structured allocation/identity failure.
    pub fn create_native_instance(
        &mut self,
        class: NativeClassToken,
        instance: NativeInstance,
    ) -> Result<Value, BuiltinError> {
        self.check_cancelled()?;
        self.native_objects
            .as_deref_mut()
            .ok_or_else(native_objects_unavailable)?
            .create_instance(class, instance)
    }

    /// Returns an R2022b-shaped logical validity value for a handle object.
    ///
    /// # Errors
    ///
    /// Returns `HostService` when no lifecycle service is attached or when the
    /// service violates the request/result contract. Service errors otherwise
    /// pass through unchanged.
    pub fn object_validity(&mut self, value: &Value) -> Result<Value, BuiltinError> {
        let (shape, expected) = match value {
            Value::Object(_) => (None, 1),
            Value::ObjectArray(array) => (
                Some(array.shape().clone()),
                usize::try_from(array.numel()).map_err(|_| {
                    BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "the object array is too large for `isvalid`",
                    )
                })?,
            ),
            other => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Type,
                    format!(
                        "`isvalid` requires a handle object or homogeneous handle array, got `{}`",
                        other.kind()
                    ),
                ));
            }
        };
        self.check_cancelled()?;
        let validity = match self
            .object_lifecycle_service()?
            .request(ObjectLifecycleRequest::IsValid(value))?
        {
            ObjectLifecycleResult::Validity(validity) => validity,
            ObjectLifecycleResult::HandleDeleteRequested { .. }
            | ObjectLifecycleResult::ValueDeleteMethodRequested => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the object-lifecycle service returned a delete result for an isvalid request",
                ));
            }
        };
        if validity.len() != expected {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::HostService,
                format!(
                    "the object-lifecycle service returned {} validity values for {expected} object elements",
                    validity.len()
                ),
            ));
        }
        let Some(shape) = shape else {
            return Ok(Value::Logical(validity[0]));
        };
        DenseArray::from_vec(shape, validity.into_iter().map(Logical::from).collect())
            .map(ArrayData::Logical)
            .map(Value::Array)
            .map_err(|error| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("cannot construct the `isvalid` logical result: {error}"),
                )
            })
    }

    /// Requests language-level object deletion without exposing the object store.
    ///
    /// Returns `Some(elements)` when column-major handle deletion was accepted
    /// and `None` when ordinary value-class method dispatch was accepted. This
    /// distinction prevents runtime wiring from treating a value method as
    /// invalidation.
    ///
    /// # Errors
    ///
    /// Returns `HostService` when no lifecycle service is attached or when the
    /// service violates the request/result contract. Service errors otherwise
    /// pass through unchanged.
    pub fn delete_object(&mut self, value: &Value) -> Result<Option<usize>, BuiltinError> {
        self.check_cancelled()?;
        match self
            .object_lifecycle_service()?
            .request(ObjectLifecycleRequest::Delete(value))?
        {
            ObjectLifecycleResult::HandleDeleteRequested { elements } => Ok(Some(elements)),
            ObjectLifecycleResult::ValueDeleteMethodRequested => Ok(None),
            ObjectLifecycleResult::Validity(_) => Err(BuiltinError::new(
                BuiltinErrorCategory::HostService,
                "the object-lifecycle service returned validity data for a delete request",
            )),
        }
    }

    fn object_lifecycle_service(
        &mut self,
    ) -> Result<&mut (dyn ObjectLifecycleService + 'a), BuiltinError> {
        self.object_lifecycle.as_deref_mut().ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::HostService,
                "the runtime object-lifecycle service is unavailable in this context",
            )
        })
    }

    /// Reads one file through the owning session's bounded filesystem service.
    ///
    /// # Errors
    ///
    /// Returns `HostService` when no filesystem is attached, or `FileSystem`
    /// with the complete OpenMat-owned host diagnostic when reading fails.
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session filesystem service is unavailable in this context",
                )
            })?
            .read_file(path, DEFAULT_MAX_FILE_BYTES)
            .map_err(|error| BuiltinError::new(BuiltinErrorCategory::FileSystem, error.message))
    }

    /// Creates or replaces one file through the owning session filesystem service.
    ///
    /// # Errors
    ///
    /// Returns `HostService`, `Cancelled`, or a structured `FileSystem` failure.
    pub fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        let length = u64::try_from(contents.len()).unwrap_or(u64::MAX);
        if length > DEFAULT_MAX_FILE_BYTES {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::FileSystem,
                format!(
                    "refusing to write {length} bytes because the session file transfer limit is {DEFAULT_MAX_FILE_BYTES} bytes"
                ),
            ));
        }
        self.file_system
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session filesystem service is unavailable in this context",
                )
            })?
            .write_file(path, contents)
            .map_err(|error| BuiltinError::new(BuiltinErrorCategory::FileSystem, error.message))
    }

    /// Returns the session-local current directory without consulting process-global state.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when the operation
    /// cannot complete.
    pub fn working_directory(&mut self) -> Result<std::path::PathBuf, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session filesystem service is unavailable in this context",
                )
            })?
            .working_directory()
            .map_err(|error| BuiltinError::new(BuiltinErrorCategory::FileSystem, error.message))
    }

    /// Changes the session-local current directory for subsequent relative operations.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when the path cannot
    /// be selected.
    pub fn change_working_directory(
        &mut self,
        path: &str,
    ) -> Result<std::path::PathBuf, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session filesystem service is unavailable in this context",
                )
            })?
            .change_working_directory(path)
            .map_err(|error| BuiltinError::new(BuiltinErrorCategory::FileSystem, error.message))
    }

    /// Returns the session MATLAB search path.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when the snapshot
    /// cannot be read.
    pub fn search_path(&mut self) -> Result<crate::SearchPathSnapshot, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .search_path()
            .map_err(filesystem_error)
    }

    /// Replaces the complete session MATLAB search path.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when validation or
    /// mutation fails.
    pub fn replace_search_path(
        &mut self,
        paths: &[String],
    ) -> Result<crate::SearchPathSnapshot, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .replace_search_path(paths)
            .map_err(filesystem_error)
    }

    /// Adds directories to the session MATLAB search path.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when validation or
    /// mutation fails.
    pub fn add_search_path(
        &mut self,
        paths: &[String],
        position: crate::SearchPathPosition,
    ) -> Result<crate::SearchPathSnapshot, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .add_search_path(paths, position)
            .map_err(filesystem_error)
    }

    /// Removes directories from the session MATLAB search path.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when validation or
    /// mutation fails.
    pub fn remove_search_path(
        &mut self,
        paths: &[String],
    ) -> Result<crate::SearchPathSnapshot, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .remove_search_path(paths)
            .map_err(filesystem_error)
    }

    /// Expands one directory using the session `genpath` policy.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when the directory
    /// tree cannot be enumerated.
    pub fn generate_search_path(
        &mut self,
        root: &str,
    ) -> Result<Vec<std::path::PathBuf>, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .generate_search_path(root)
            .map_err(filesystem_error)
    }

    /// Resolves one MATLAB source name through current-folder and path precedence.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when resolution or
    /// source reading fails.
    pub fn resolve_matlab_source(
        &mut self,
        caller: Option<&str>,
        name: &str,
    ) -> Result<Option<crate::ResolvedMatlabSource>, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .resolve_matlab_source(caller, name)
            .map_err(filesystem_error)
    }

    /// Inspects one exact path through the session filesystem service.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when inspection
    /// cannot complete.
    pub fn file_metadata(&mut self, path: &str) -> Result<FileSystemMetadata, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .metadata(path)
            .map_err(filesystem_error)
    }

    /// Expands one exact directory/file path or final-component wildcard.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when enumeration
    /// cannot complete.
    pub fn directory_entries(
        &mut self,
        path: &str,
    ) -> Result<Vec<FileSystemMetadata>, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .directory_entries(path)
            .map_err(filesystem_error)
    }

    /// Creates one directory tree through the session filesystem service.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when creation fails.
    pub fn create_directory(&mut self, path: &str) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .create_directory(path)
            .map_err(filesystem_error)
    }

    /// Removes one directory through the session filesystem service.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when removal fails.
    pub fn remove_directory(&mut self, path: &str, recursive: bool) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .remove_directory(path, recursive)
            .map_err(filesystem_error)
    }

    /// Removes one regular file through the session filesystem service.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when removal fails.
    pub fn remove_file(&mut self, path: &str) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .remove_file(path)
            .map_err(filesystem_error)
    }

    /// Copies one regular file or directory tree through the session service.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when copying fails.
    pub fn copy_path(
        &mut self,
        source: &str,
        destination: &str,
        force: bool,
    ) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .copy_path(source, destination, force)
            .map_err(filesystem_error)
    }

    /// Moves one regular file or directory tree through the session service.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when moving fails.
    pub fn move_path(
        &mut self,
        source: &str,
        destination: &str,
        force: bool,
    ) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .move_path(source, destination, force)
            .map_err(filesystem_error)
    }

    /// Opens one session-owned file stream.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, or `FileSystem` when opening fails.
    pub fn open_file(
        &mut self,
        path: &str,
        mode: FileOpenMode,
        machine_format: &str,
        encoding: &str,
    ) -> Result<u32, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .open_file(path, mode, machine_format, encoding)
            .map_err(filesystem_error)
    }

    /// Closes one session-owned file stream.
    ///
    /// # Errors
    ///
    /// Returns the MATLAB-compatible bad-file-identifier error when the stream is not open.
    pub fn close_open_file(&mut self, identifier: u32) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        let closed = self
            .file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .close_open_file(identifier);
        if closed {
            Ok(())
        } else {
            Err(bad_file_identifier(identifier))
        }
    }

    /// Closes every stream owned by the session filesystem service.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled` or `HostService` when the operation cannot run.
    pub fn close_all_open_files(&mut self) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .close_all_open_files();
        Ok(())
    }

    /// Returns all currently open file identifiers in ascending order.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled` or `HostService` when the operation cannot run.
    pub fn open_file_identifiers(&mut self) -> Result<Vec<u32>, BuiltinError> {
        self.check_cancelled()?;
        Ok(self
            .file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .open_file_identifiers())
    }

    /// Returns retained metadata for one open file.
    ///
    /// # Errors
    ///
    /// Returns the MATLAB-compatible bad-file-identifier error when the stream is not open.
    pub fn open_file_info(&mut self, identifier: u32) -> Result<OpenFileInfo, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .open_file_info(identifier)
            .ok_or_else(|| bad_file_identifier(identifier))
    }

    /// Reads bytes from one open file at its current position.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, `FileSystem`, or a bad-file-identifier error.
    pub fn read_open_file(
        &mut self,
        identifier: u32,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>, BuiltinError> {
        self.check_cancelled()?;
        let maximum = u64::try_from(maximum_bytes).unwrap_or(u64::MAX);
        if maximum > DEFAULT_MAX_FILE_BYTES {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::FileSystem,
                format!(
                    "refusing to read {maximum} bytes because the session file transfer limit is {DEFAULT_MAX_FILE_BYTES} bytes"
                ),
            ));
        }
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .read_open_file(identifier, maximum_bytes)
            .map_err(filesystem_error)?
            .ok_or_else(|| bad_file_identifier(identifier))
    }

    /// Writes bytes to one open file at its current position.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, `FileSystem`, or a bad-file-identifier error.
    pub fn write_open_file(
        &mut self,
        identifier: u32,
        contents: &[u8],
    ) -> Result<usize, BuiltinError> {
        self.check_cancelled()?;
        let length = u64::try_from(contents.len()).unwrap_or(u64::MAX);
        if length > DEFAULT_MAX_FILE_BYTES {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::FileSystem,
                format!(
                    "refusing to write {length} bytes because the session file transfer limit is {DEFAULT_MAX_FILE_BYTES} bytes"
                ),
            ));
        }
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .write_open_file(identifier, contents)
            .map_err(filesystem_error)?
            .ok_or_else(|| bad_file_identifier(identifier))
    }

    /// Repositions one open file and returns its new byte position.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, `FileSystem`, or a bad-file-identifier error.
    pub fn seek_open_file(
        &mut self,
        identifier: u32,
        offset: i64,
        origin: FileSeekOrigin,
    ) -> Result<u64, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .seek_open_file(identifier, offset, origin)
            .map_err(filesystem_error)?
            .ok_or_else(|| bad_file_identifier(identifier))
    }

    /// Returns the current byte position of one open file.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled`, `HostService`, `FileSystem`, or a bad-file-identifier error.
    pub fn tell_open_file(&mut self, identifier: u32) -> Result<u64, BuiltinError> {
        self.check_cancelled()?;
        self.file_system
            .as_deref_mut()
            .ok_or_else(filesystem_unavailable)?
            .tell_open_file(identifier)
            .map_err(filesystem_error)?
            .ok_or_else(|| bad_file_identifier(identifier))
    }

    /// Executes one atomic graphics request through the owning session.
    ///
    /// The model commits before the internal notice is emitted. An output
    /// failure is therefore reported without rolling back committed graphics
    /// state; reconnect snapshots remain authoritative.
    ///
    /// # Errors
    ///
    /// Returns `HostService` in standalone contexts, `Graphics` for model
    /// failures, or `Output` if notice delivery fails after commit.
    pub fn graphics(&mut self, request: GraphicsRequest) -> Result<GraphicsResponse, BuiltinError> {
        self.check_cancelled()?;
        let service = self.graphics.as_deref_mut().ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::HostService,
                "the session graphics service is unavailable in this context",
            )
        })?;
        let execution = service.execute(request).map_err(|error| {
            BuiltinError::new(BuiltinErrorCategory::Graphics, error.to_string())
        })?;
        if let Some(notice) = execution.notice {
            self.emit(OutputEvent::GraphicsNotice(notice))?;
        }
        Ok(execution.response)
    }

    /// Returns the current colormap length without requiring or mutating a
    /// graphics session.
    #[must_use]
    pub fn current_colormap_length(&self) -> Option<usize> {
        self.graphics
            .as_ref()
            .and_then(|service| service.current_colormap_length())
    }

    /// Draws one uniform random value from the session stream.
    ///
    /// # Errors
    /// Returns `Cancelled` or `HostService` when the draw cannot proceed.
    pub fn random_uniform(&mut self) -> Result<f64, BuiltinError> {
        self.check_cancelled()?;
        self.random
            .as_deref_mut()
            .map(RandomService::next_uniform)
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session random service is unavailable in this context",
                )
            })
    }

    /// Draws one standard-normal value from the session stream.
    ///
    /// # Errors
    /// Returns `Cancelled` or `HostService` when the draw cannot proceed.
    pub fn random_normal(&mut self) -> Result<f64, BuiltinError> {
        self.check_cancelled()?;
        self.random
            .as_deref_mut()
            .map(RandomService::next_normal)
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session random service is unavailable in this context",
                )
            })
    }

    /// Resets the session random stream.
    ///
    /// # Errors
    /// Returns `Cancelled` or `HostService` when the stream is unavailable.
    pub fn random_reseed(&mut self, seed: u32) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        self.random
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session random service is unavailable in this context",
                )
            })?
            .reseed(seed);
        Ok(())
    }

    /// Resets the session random stream from host entropy.
    ///
    /// # Errors
    /// Returns `Cancelled` or `HostService` when the stream is unavailable.
    pub fn random_shuffle(&mut self) -> Result<(), BuiltinError> {
        self.check_cancelled()?;
        self.random
            .as_deref_mut()
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session random service is unavailable in this context",
                )
            })?
            .shuffle();
        Ok(())
    }

    /// Captures the current session random stream.
    ///
    /// # Errors
    /// Returns `Cancelled` or `HostService` when the stream is unavailable.
    pub fn random_snapshot(&mut self) -> Result<RandomSnapshot, BuiltinError> {
        self.check_cancelled()?;
        self.random
            .as_deref_mut()
            .map(|random| random.snapshot())
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::HostService,
                    "the session random service is unavailable in this context",
                )
            })
    }
}

fn filesystem_unavailable() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::HostService,
        "the session filesystem service is unavailable in this context",
    )
}

fn native_objects_unavailable() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::HostService,
        "the session native-object bridge is unavailable in this context",
    )
}

fn filesystem_error(error: crate::FileSystemError) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::FileSystem, error.message)
}

fn bad_file_identifier(identifier: u32) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("file identifier {identifier} is not open in this session"),
    )
    .with_identifier("MATLAB:badfid_mx")
}

/// A host-implemented function callable from `OpenMat` bytecode.
pub trait BuiltinFunction: Send + Sync {
    /// Invokes the built-in.
    ///
    /// # Errors
    ///
    /// Returns a structured built-in failure.
    fn call(&self, arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult;

    /// Invokes the built-in while transferring ownership of the argument values.
    ///
    /// The default implementation preserves the ordinary borrowed built-in API. Native
    /// extensions may override this entrypoint to consume uniquely owned array storage without
    /// an intermediate clone.
    ///
    /// # Errors
    ///
    /// Returns a structured built-in failure.
    fn call_owned(&self, arguments: Vec<Value>, context: &mut BuiltinContext<'_>) -> BuiltinResult {
        self.call(&arguments, context)
    }
}

impl<F> BuiltinFunction for F
where
    F: Fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult + Send + Sync,
{
    fn call(&self, arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
        self(arguments, context)
    }
}

struct BuiltinEntry {
    name: Arc<str>,
    function: Arc<dyn BuiltinFunction>,
}

/// Registration failures for a built-in registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinRegistrationError {
    /// Empty names are not valid workspace identifiers.
    EmptyName,
    /// A function is already registered under this name.
    DuplicateName(String),
    /// No further stable registry identifiers are available.
    IdentifierExhausted,
    /// A native class descriptor is internally inconsistent.
    InvalidNativeClass(String),
}

impl fmt::Display for BuiltinRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("a built-in name cannot be empty"),
            Self::DuplicateName(name) => {
                write!(formatter, "built-in `{name}` is already registered")
            }
            Self::IdentifierExhausted => {
                formatter.write_str("the built-in handle space is exhausted")
            }
            Self::InvalidNativeClass(message) => {
                write!(formatter, "invalid native class descriptor: {message}")
            }
        }
    }
}

impl Error for BuiltinRegistrationError {}

/// A failure while dispatching a registry entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinInvocationError {
    /// The handle does not belong to this registry.
    UnknownHandle(BuiltinHandle),
    /// The built-in ran and returned a structured failure.
    Failed { name: Arc<str>, error: BuiltinError },
}

impl fmt::Display for BuiltinInvocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownHandle(handle) => write!(
                formatter,
                "built-in handle {} is not registered",
                handle.identifier()
            ),
            Self::Failed { name, error } => write!(formatter, "built-in `{name}` failed: {error}"),
        }
    }
}

impl Error for BuiltinInvocationError {}

/// A deterministic name and handle registry for host built-ins.
pub struct BuiltinRegistry {
    by_name: BTreeMap<String, BuiltinHandle>,
    entries: BTreeMap<BuiltinHandle, BuiltinEntry>,
    native_classes: BTreeMap<String, Arc<dyn NativeClass>>,
    next_identifier: u64,
}

impl BuiltinRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            by_name: BTreeMap::new(),
            entries: BTreeMap::new(),
            native_classes: BTreeMap::new(),
            next_identifier: 1,
        }
    }

    /// Registers a built-in under a workspace name.
    ///
    /// # Errors
    ///
    /// Rejects empty or duplicate names and exhausted handle identifiers.
    pub fn register<F>(
        &mut self,
        name: impl Into<String>,
        function: F,
    ) -> Result<BuiltinHandle, BuiltinRegistrationError>
    where
        F: BuiltinFunction + 'static,
    {
        self.register_entry(name, Arc::new(function))
    }

    /// Registers a callable under a name commonly used without parentheses.
    ///
    /// The entry remains callable with parentheses and through a function
    /// handle. Bare invocation is selected by the compiler/runtime name context
    /// for every callable; this method remains as a source-compatible spelling
    /// for constants and zero-input functions.
    ///
    /// # Errors
    ///
    /// Rejects empty or duplicate names and exhausted handle identifiers.
    pub fn register_bare_callable<F>(
        &mut self,
        name: impl Into<String>,
        function: F,
    ) -> Result<BuiltinHandle, BuiltinRegistrationError>
    where
        F: BuiltinFunction + 'static,
    {
        self.register_entry(name, Arc::new(function))
    }

    /// Registers a dynamically shared built-in implementation.
    ///
    /// # Errors
    ///
    /// Rejects empty or duplicate names and exhausted handle identifiers.
    pub fn register_arc(
        &mut self,
        name: impl Into<String>,
        function: Arc<dyn BuiltinFunction>,
    ) -> Result<BuiltinHandle, BuiltinRegistrationError> {
        self.register_entry(name, function)
    }

    fn register_entry(
        &mut self,
        name: impl Into<String>,
        function: Arc<dyn BuiltinFunction>,
    ) -> Result<BuiltinHandle, BuiltinRegistrationError> {
        let name = name.into();
        if name.is_empty() {
            return Err(BuiltinRegistrationError::EmptyName);
        }
        if self.by_name.contains_key(&name) || self.native_classes.contains_key(&name) {
            return Err(BuiltinRegistrationError::DuplicateName(name));
        }

        let handle = BuiltinHandle::new(self.next_identifier);
        self.next_identifier = self
            .next_identifier
            .checked_add(1)
            .ok_or(BuiltinRegistrationError::IdentifierExhausted)?;
        self.by_name.insert(name.clone(), handle);
        self.entries.insert(
            handle,
            BuiltinEntry {
                name: Arc::from(name),
                function,
            },
        );
        Ok(handle)
    }

    /// Registers one native handle class in the same language namespace as built-ins.
    ///
    /// # Errors
    ///
    /// Rejects empty or conflicting class names and inconsistent member metadata.
    pub fn register_native_class(
        &mut self,
        class: Arc<dyn NativeClass>,
    ) -> Result<(), BuiltinRegistrationError> {
        let name = class.name().to_owned();
        if name.is_empty() {
            return Err(BuiltinRegistrationError::EmptyName);
        }
        if self.by_name.contains_key(&name) || self.native_classes.contains_key(&name) {
            return Err(BuiltinRegistrationError::DuplicateName(name));
        }
        if self
            .native_classes
            .values()
            .any(|registered| registered.token() == class.token())
        {
            return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                "class `{name}` repeats a native type token"
            )));
        }
        let mut methods = BTreeSet::new();
        for method in class.methods() {
            if method.name().is_empty() {
                return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                    "class `{name}` has an empty method name"
                )));
            }
            if !methods.insert(method.name()) {
                return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                    "class `{name}` repeats method `{}`",
                    method.name()
                )));
            }
            if method.name() == name || method.name() == "delete" {
                return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                    "class `{name}` uses reserved method name `{}`",
                    method.name()
                )));
            }
        }
        let mut properties = BTreeSet::new();
        for property in class.properties() {
            if property.name().is_empty() {
                return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                    "class `{name}` has an empty property name"
                )));
            }
            if !property.readable() && !property.writable() {
                return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                    "class `{name}` property `{}` is neither readable nor writable",
                    property.name()
                )));
            }
            if !properties.insert(property.name()) {
                return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                    "class `{name}` repeats property `{}`",
                    property.name()
                )));
            }
            if methods.contains(property.name()) || property.name() == "delete" {
                return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                    "class `{name}` property `{}` conflicts with a method",
                    property.name()
                )));
            }
            for prefix in ["get", "set"] {
                let accessor = format!("{prefix}.{}", property.name());
                if methods.contains(accessor.as_str()) {
                    return Err(BuiltinRegistrationError::InvalidNativeClass(format!(
                        "class `{name}` method `{accessor}` conflicts with a property accessor"
                    )));
                }
            }
        }
        self.native_classes.insert(name, class);
        Ok(())
    }

    pub(crate) fn native_classes(&self) -> impl Iterator<Item = &Arc<dyn NativeClass>> {
        self.native_classes.values()
    }

    /// Finds a built-in by workspace name.
    #[must_use]
    pub fn handle_by_name(&self, name: &str) -> Option<BuiltinHandle> {
        self.by_name.get(name).copied()
    }

    /// Returns whether a built-in or native class occupies this language name.
    #[must_use]
    pub fn contains_name(&self, name: &str) -> bool {
        self.by_name.contains_key(name) || self.native_classes.contains_key(name)
    }

    /// Returns whether one process-local native class token is already registered.
    #[must_use]
    pub fn contains_native_class_token(&self, token: NativeClassToken) -> bool {
        self.native_classes
            .values()
            .any(|class| class.token() == token)
    }

    /// Returns the registered name for a handle.
    #[must_use]
    pub fn name(&self, handle: BuiltinHandle) -> Option<&str> {
        self.entries.get(&handle).map(|entry| entry.name.as_ref())
    }

    /// Iterates over registered function names in deterministic name order.
    /// Does not invoke functions or include separately registered native classes.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// Invokes a registered built-in.
    ///
    /// # Errors
    ///
    /// Returns an unknown-handle error or the built-in's structured failure.
    pub fn invoke(
        &self,
        handle: BuiltinHandle,
        arguments: &[Value],
        context: &mut BuiltinContext<'_>,
    ) -> Result<Vec<Value>, BuiltinInvocationError> {
        let (name, function) = self
            .resolve(handle)
            .ok_or(BuiltinInvocationError::UnknownHandle(handle))?;
        function
            .call(arguments, context)
            .map_err(|error| BuiltinInvocationError::Failed { name, error })
    }

    /// Invokes a registered built-in and transfers ownership of its arguments.
    ///
    /// # Errors
    ///
    /// Returns an unknown-handle error or the built-in's structured failure.
    pub fn invoke_owned(
        &self,
        handle: BuiltinHandle,
        arguments: Vec<Value>,
        context: &mut BuiltinContext<'_>,
    ) -> Result<Vec<Value>, BuiltinInvocationError> {
        let (name, function) = self
            .resolve(handle)
            .ok_or(BuiltinInvocationError::UnknownHandle(handle))?;
        function
            .call_owned(arguments, context)
            .map_err(|error| BuiltinInvocationError::Failed { name, error })
    }

    /// Returns the number of registered built-ins.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether no built-ins are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn resolve(
        &self,
        handle: BuiltinHandle,
    ) -> Option<(Arc<str>, Arc<dyn BuiltinFunction>)> {
        self.entries
            .get(&handle)
            .map(|entry| (Arc::clone(&entry.name), Arc::clone(&entry.function)))
    }
}

impl Default for BuiltinRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for BuiltinRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuiltinRegistry")
            .field("names", &self.by_name.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BuiltinContext, BuiltinError, ObjectLifecycleRequest, ObjectLifecycleResult,
        ObjectLifecycleService, WarningState,
    };
    use crate::{CancellationToken, NullOutput};
    use openmat_array::{ArrayData, Shape};
    use openmat_value::{ClassHandle, ObjectArray, ObjectHandle, Value};

    #[derive(Default)]
    struct FakeObjectLifecycle {
        validity: Vec<bool>,
        delete_result: Option<ObjectLifecycleResult>,
        requests: Vec<(&'static str, Value)>,
    }

    impl ObjectLifecycleService for FakeObjectLifecycle {
        fn request(
            &mut self,
            request: ObjectLifecycleRequest<'_>,
        ) -> Result<ObjectLifecycleResult, BuiltinError> {
            match request {
                ObjectLifecycleRequest::IsValid(value) => {
                    self.requests.push(("isvalid", value.clone()));
                    Ok(ObjectLifecycleResult::Validity(self.validity.clone()))
                }
                ObjectLifecycleRequest::Delete(value) => {
                    self.requests.push(("delete", value.clone()));
                    Ok(self
                        .delete_result
                        .clone()
                        .unwrap_or(ObjectLifecycleResult::HandleDeleteRequested { elements: 1 }))
                }
            }
        }
    }

    fn object_array(shape: [u64; 2], handles: &[u64]) -> Value {
        Value::ObjectArray(
            ObjectArray::from_vec(
                ClassHandle::new(7),
                Shape::new(shape).unwrap(),
                handles.iter().copied().map(ObjectHandle::new).collect(),
            )
            .unwrap(),
        )
    }

    fn validity(service: &mut FakeObjectLifecycle, value: &Value) -> Value {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        BuiltinContext::with_object_lifecycle_service(1, &cancellation, &mut output, service)
            .object_validity(value)
            .unwrap()
    }

    fn delete(
        service: &mut FakeObjectLifecycle,
        value: &Value,
    ) -> Result<Option<usize>, BuiltinError> {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        BuiltinContext::with_object_lifecycle_service(0, &cancellation, &mut output, service)
            .delete_object(value)
    }

    #[test]
    fn recording_a_disabled_warning_still_updates_lastwarn() {
        let mut state = WarningState::default();
        assert!(state.record("shown", "OpenMat:Shown"));
        assert_eq!(state.last_message, "shown");
        assert_eq!(state.last_identifier, "OpenMat:Shown");

        state.overrides.insert("OpenMat:Hidden".to_owned(), false);
        assert!(!state.record("hidden", "OpenMat:Hidden"));
        assert_eq!(state.last_message, "hidden");
        assert_eq!(state.last_identifier, "OpenMat:Hidden");
    }

    #[test]
    fn regular_context_can_attach_the_runtime_lifecycle_service() {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut service = FakeObjectLifecycle {
            validity: vec![true],
            ..FakeObjectLifecycle::default()
        };
        let object = Value::Object(ObjectHandle::new(9));
        let mut context = BuiltinContext::new(1, &cancellation, &mut output);
        context.set_object_lifecycle_service(&mut service);
        assert_eq!(
            context.object_validity(&object).unwrap(),
            Value::Logical(true)
        );
    }

    #[test]
    fn lifecycle_service_preserves_r2022b_logical_shape_and_invalid_aliases() {
        let scalar = Value::Object(ObjectHandle::new(11));
        let mut service = FakeObjectLifecycle {
            validity: vec![false],
            ..FakeObjectLifecycle::default()
        };
        assert_eq!(validity(&mut service, &scalar), Value::Logical(false));

        let array = object_array([2, 2], &[21, 22, 21, 23]);
        service.validity = vec![true, false, true, false];
        let result = validity(&mut service, &array);
        let Value::Array(ArrayData::Logical(result)) = result else {
            panic!("expected logical array");
        };
        assert_eq!(result.shape().dimensions(), [2, 2]);
        assert_eq!(
            result
                .as_slice()
                .iter()
                .map(|element| element.get())
                .collect::<Vec<_>>(),
            [true, false, true, false]
        );

        let empty = object_array([0, 3], &[]);
        service.validity.clear();
        let result = validity(&mut service, &empty);
        assert_eq!(result.dimensions(), Some([0, 3].as_slice()));
        assert_eq!(
            service.requests,
            [("isvalid", scalar), ("isvalid", array), ("isvalid", empty)]
        );
    }

    #[test]
    fn lifecycle_delete_distinguishes_linear_handle_work_from_value_dispatch() {
        let mut service = FakeObjectLifecycle::default();
        let handles = object_array([2, 2], &[31, 33, 34, 31]);
        service.delete_result = Some(ObjectLifecycleResult::HandleDeleteRequested { elements: 4 });
        assert_eq!(delete(&mut service, &handles).unwrap(), Some(4));
        let Value::ObjectArray(requested) = &service.requests[0].1 else {
            panic!("expected object-array delete request");
        };
        assert_eq!(
            requested
                .as_slice()
                .iter()
                .map(|handle| handle.identifier())
                .collect::<Vec<_>>(),
            [31, 33, 34, 31]
        );

        let value_object = Value::Object(ObjectHandle::new(41));
        service.delete_result = Some(ObjectLifecycleResult::ValueDeleteMethodRequested);
        assert_eq!(delete(&mut service, &value_object).unwrap(), None);
        assert_eq!(service.requests[1], ("delete", value_object));
    }
}

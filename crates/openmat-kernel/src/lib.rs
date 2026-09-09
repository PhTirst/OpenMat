#![doc = "Headless `OpenMat` execution kernel."]

use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use openmat_protocol::{
    Capabilities, Diagnostic, Event, EventEnvelope, ExecuteRequest, ExecuteResult,
    ImplementationInfo, InitializeRequest, InitializeResult, InspectRequest, InterruptResult,
    KernelStatus, MAX_PREVIEW_ELEMENTS, MatrixPreview, PROTOCOL_V0, ProtocolError, Request,
    RequestEnvelope, ResponseEnvelope, ResponseResult, ServerMessage, ShutdownResult, StatusEvent,
    VariableSummary, WorkspaceSummary,
};

mod file_source;
mod module_linker;
mod protocol_session;
mod runtime_engine;

pub use file_source::{FileSourceResolver, ResolvedSourceFile};
pub use openmat_runtime::{GraphicsDelta, GraphicsNotice, GraphicsNoticeKind, GraphicsSession};
pub use protocol_session::{
    KernelSession, KernelSessionControl, KernelSessionError, KernelSessionMessage,
    KernelSessionRequest,
};
pub use runtime_engine::RuntimeEngine;

/// Adapter boundary between the protocol kernel and a concrete compiler/VM.
///
/// The trait is deliberately expressed only in protocol-owned data. A future
/// runtime adapter can translate to parser, bytecode, and VM types without
/// making those crates part of the stable kernel boundary.
pub trait ExecutionEngine {
    /// Returns the concrete engine identity reported during initialization.
    fn implementation(&self) -> ImplementationInfo;

    /// Returns engine capabilities before client negotiation.
    fn capabilities(&self) -> Capabilities;

    /// Returns v1 capabilities before client negotiation.
    ///
    /// The default preserves the existing engine capability surface and adds
    /// the protocol's hard UTF-16 preview limits. Engines with stricter exact
    /// preview bounds may override this method.
    fn v1_capabilities(&self) -> openmat_protocol::kernel_v1::Capabilities {
        let capabilities = self.capabilities();
        openmat_protocol::kernel_v1::Capabilities {
            execution_modes: capabilities.execution_modes,
            display_mime_types: capabilities.display_mime_types,
            max_preview_elements: capabilities.max_preview_elements,
            max_string_element_code_units: Some(
                openmat_protocol::kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS,
            ),
            max_preview_code_units: Some(openmat_protocol::kernel_v1::MAX_PREVIEW_CODE_UNITS),
            interrupt: capabilities.interrupt,
            workspace_delta: capabilities.workspace_delta,
        }
    }

    /// Returns v2 capabilities before client negotiation.
    ///
    /// The default preserves the v1 capability surface and advertises the
    /// protocol's hard aggregate ceilings. Engines with lower recursive
    /// observation budgets may override this method.
    fn v2_capabilities(&self) -> openmat_protocol::kernel_v2::Capabilities {
        let capabilities = self.v1_capabilities();
        openmat_protocol::kernel_v2::Capabilities {
            execution_modes: capabilities.execution_modes,
            display_mime_types: capabilities.display_mime_types,
            max_preview_elements: capabilities.max_preview_elements,
            max_string_element_code_units: capabilities.max_string_element_code_units,
            max_preview_code_units: capabilities.max_preview_code_units,
            max_aggregate_nodes: Some(openmat_protocol::kernel_v2::MAX_AGGREGATE_NODES),
            max_aggregate_elements: Some(openmat_protocol::kernel_v2::MAX_AGGREGATE_ELEMENTS),
            max_aggregate_depth: Some(openmat_protocol::kernel_v2::MAX_AGGREGATE_DEPTH),
            interrupt: capabilities.interrupt,
            workspace_delta: capabilities.workspace_delta,
        }
    }

    /// Returns the optional synchronized graphics hierarchy owned by this
    /// in-process engine. The default keeps non-graphics test engines and
    /// alternative hosts source-compatible.
    #[must_use]
    fn graphics_session(&self) -> Option<Arc<std::sync::Mutex<GraphicsSession>>> {
        None
    }

    /// Drains small committed graphics notices after an execution request.
    fn take_graphics_notices(&mut self) -> Vec<GraphicsNotice> {
        Vec::new()
    }

    /// Drains exact committed graphics deltas after an execution request.
    fn take_graphics_deltas(&mut self) -> Vec<GraphicsDelta> {
        Vec::new()
    }

    /// Executes one request. The kernel never calls this concurrently with
    /// `inspect`, `list_workspace`, or `shutdown` on the same engine.
    ///
    /// Implementations should poll `cancellation` at safe points. A server may
    /// terminate the process when native code cannot cooperate.
    ///
    /// # Errors
    ///
    /// Returns an [`EngineError`] for compiler or runtime failures.
    fn execute(
        &mut self,
        request: &ExecuteRequest,
        cancellation: &CancellationToken,
    ) -> Result<ExecutionOutput, EngineError>;

    /// Produces a bounded preview for one workspace value.
    ///
    /// # Errors
    ///
    /// Returns an [`EngineError`] when the name/range is invalid or inspection
    /// fails.
    fn inspect(&mut self, request: &InspectRequest) -> Result<MatrixPreview, EngineError>;

    /// Produces a bounded, exact v1 preview for one workspace value.
    ///
    /// Existing engines remain source-compatible and report a stable
    /// unsupported-value error until they implement the exact boundary.
    ///
    /// # Errors
    ///
    /// Returns an [`EngineError`] when exact v1 inspection is unavailable, the
    /// name/range is invalid, or inspection fails.
    fn inspect_v1(
        &mut self,
        _request: &openmat_protocol::kernel_v1::InspectRequest,
        _limits: &openmat_protocol::kernel_v1::PreviewLimits,
    ) -> Result<openmat_protocol::kernel_v1::MatrixPreview, EngineError> {
        Err(EngineError::new(
            "workspace.unsupportedValue",
            "this execution engine does not implement exact kernel-v1 inspection",
        ))
    }

    /// Produces a bounded v2 matrix or recursive aggregate preview.
    ///
    /// The default delegates non-aggregate behavior to the frozen v1 adapter.
    /// Consequently existing engines retain exact scalar behavior and keep
    /// returning `workspace.unsupportedValue` for cell and struct values until
    /// they implement recursive observation.
    ///
    /// # Errors
    ///
    /// Returns an [`EngineError`] when exact inspection is unavailable, the
    /// name/range is invalid, or inspection fails.
    fn inspect_v2(
        &mut self,
        request: &openmat_protocol::kernel_v2::InspectRequest,
        limits: &openmat_protocol::kernel_v2::AggregateLimits,
    ) -> Result<openmat_protocol::kernel_v2::InspectPreview, EngineError> {
        self.inspect_v1(request, &limits.preview_limits())
            .map(openmat_protocol::kernel_v2::InspectPreview::Matrix)
    }

    /// Atomically replaces one existing scalar numeric workspace element.
    ///
    /// The protocol session checks `expectedRevision` before calling this
    /// method. Existing engines remain source-compatible and explicitly report
    /// that mutation is unavailable until they implement this boundary.
    ///
    /// # Errors
    ///
    /// Returns an [`EngineError`] when the variable, class, indices, or value
    /// cannot be edited.
    fn set_variable_element(
        &mut self,
        _request: &openmat_protocol::kernel_v3::SetVariableElementRequest,
    ) -> Result<VariableSummary, EngineError> {
        Err(EngineError::new(
            "workspace.editUnsupported",
            "this execution engine does not implement workspace element mutation",
        ))
    }

    /// Returns workspace summaries without transferring values.
    ///
    /// # Errors
    ///
    /// Returns an [`EngineError`] when workspace enumeration fails.
    fn list_workspace(&mut self) -> Result<Vec<VariableSummary>, EngineError>;

    /// Releases engine-owned resources for orderly shutdown.
    ///
    /// # Errors
    ///
    /// Returns an [`EngineError`] if orderly teardown fails.
    fn shutdown(&mut self) -> Result<(), EngineError>;
}

/// Cooperative cancellation flag shared with an execution engine.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns whether cooperative cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn reset(&self) {
        self.cancelled.store(false, Ordering::Release);
    }
}

/// Engine execution result plus protocol events generated by the adapter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecutionOutput {
    /// Completion metadata.
    pub result: ExecuteResult,
    /// Stream, display, diagnostic, or workspace events produced in order.
    pub events: Vec<Event>,
}

/// Structured error returned by an [`ExecutionEngine`].
#[derive(Clone, Debug)]
pub struct EngineError {
    category: String,
    message: String,
    diagnostics: Vec<Diagnostic>,
    events: Vec<Event>,
}

impl PartialEq for EngineError {
    fn eq(&self, other: &Self) -> bool {
        self.category == other.category
            && self.message == other.message
            && self.diagnostics == other.diagnostics
    }
}

impl Eq for EngineError {}

impl EngineError {
    /// Creates an engine error without diagnostics.
    #[must_use]
    pub fn new(category: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            message: message.into(),
            diagnostics: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Attaches structured diagnostics.
    #[must_use]
    pub fn with_diagnostics(mut self, diagnostics: Vec<Diagnostic>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// Attaches ordered events produced before the execution failed.
    #[must_use]
    pub fn with_events(mut self, events: Vec<Event>) -> Self {
        self.events = events;
        self
    }

    /// Appends ordered events produced by an in-process host adapter after the
    /// engine returned, preserving any events already attached to the error.
    #[must_use]
    pub fn extend_events(mut self, events: impl IntoIterator<Item = Event>) -> Self {
        self.events.extend(events);
        self
    }

    /// Returns the stable error category.
    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }

    /// Returns the human-readable error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns structured diagnostics attached to this error.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    fn into_protocol_error(self) -> ProtocolError {
        ProtocolError {
            category: self.category,
            message: self.message,
            diagnostics: self.diagnostics,
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl Error for EngineError {}

/// Headless, single-session kernel state machine.
///
/// Sequential methods require `&mut self`, preventing overlapping execution on
/// one engine. Clone [`KernelControl`] and route interrupts on another thread or
/// task so cancellation does not wait behind a blocked engine call.
pub struct Kernel<E> {
    engine: E,
    control: KernelControl,
    initialized: bool,
    workspace_revision: u64,
}

impl<E: ExecutionEngine> Kernel<E> {
    /// Creates a kernel in the `starting` state for one opaque session ID.
    #[must_use]
    pub fn new(session_id: impl Into<String>, engine: E) -> Self {
        Self {
            engine,
            control: KernelControl {
                session_id: session_id.into(),
                state: Arc::new(AtomicU8::new(status_to_u8(KernelStatus::Starting))),
                cancellation: CancellationToken::new(),
                next_message_id: Arc::new(AtomicU64::new(1)),
            },
            initialized: false,
            workspace_revision: 0,
        }
    }

    /// Returns a cloneable interrupt control path independent from `&mut self`.
    #[must_use]
    pub fn control(&self) -> KernelControl {
        self.control.clone()
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub fn status(&self) -> KernelStatus {
        self.control.status()
    }

    /// Emits the initial `starting` status event.
    #[must_use]
    pub fn startup_event(&self) -> ServerMessage {
        ServerMessage::Event(self.control.status_event(KernelStatus::Starting))
    }

    /// Handles a request on the sequential engine path. Interrupt transports
    /// should prefer [`KernelControl::handle_interrupt`] during execution.
    #[must_use]
    pub fn handle_request(&mut self, request: &RequestEnvelope) -> Vec<ServerMessage> {
        if let Err(error) = request.validate() {
            return vec![self.failure(
                request,
                ProtocolError::new("protocol.validation", error.to_string()),
            )];
        }
        if request.session_id != self.control.session_id {
            return vec![self.failure(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            )];
        }
        if self.status() == KernelStatus::Dead {
            return vec![self.failure(
                request,
                ProtocolError::new("kernel.dead", "kernel has already shut down"),
            )];
        }

        match &request.request {
            Request::Initialize(parameters) => self.initialize(request, parameters),
            Request::Interrupt(_) => self.control.handle_interrupt(request),
            Request::Execute(parameters) => self.execute(request, parameters),
            Request::Inspect(parameters) => self.inspect(request, parameters),
            Request::ListWorkspace(_) => self.list_workspace(request),
            Request::Shutdown(_) => self.shutdown(request),
        }
    }

    fn initialize(
        &mut self,
        request: &RequestEnvelope,
        parameters: &InitializeRequest,
    ) -> Vec<ServerMessage> {
        if self.initialized {
            return vec![self.failure(
                request,
                ProtocolError::new("kernel.alreadyInitialized", "kernel is already initialized"),
            )];
        }
        if !parameters.supported_protocols.is_empty()
            && !parameters
                .supported_protocols
                .iter()
                .any(|protocol| protocol == PROTOCOL_V0)
        {
            return vec![self.failure(
                request,
                ProtocolError::new(
                    "protocol.noCommonVersion",
                    "client does not support openmat-kernel-v0",
                ),
            )];
        }

        let capabilities = self
            .engine
            .capabilities()
            .negotiate(&parameters.capabilities);
        let response = self.success(
            request,
            ResponseResult::Initialize(InitializeResult {
                negotiated_protocol: PROTOCOL_V0.to_owned(),
                implementation: self.engine.implementation(),
                capabilities,
            }),
        );
        self.initialized = true;
        self.set_status(KernelStatus::Idle);
        vec![response, self.status_message(KernelStatus::Idle)]
    }

    fn execute(
        &mut self,
        request: &RequestEnvelope,
        parameters: &ExecuteRequest,
    ) -> Vec<ServerMessage> {
        if let Some(failure) = self.require_initialized(request) {
            return vec![failure];
        }
        self.control.cancellation.reset();
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message(KernelStatus::Busy)];
        let execution = self.engine.execute(parameters, &self.control.cancellation);
        match execution {
            Ok(output) => {
                for event in output.events {
                    messages.push(self.event_message(event));
                }
                let interrupted = output.result.interrupted
                    || self.control.cancellation.is_cancelled()
                    || self.status() == KernelStatus::Interrupted;
                if interrupted && self.status() != KernelStatus::Interrupted {
                    self.set_status(KernelStatus::Interrupted);
                    messages.push(self.status_message(KernelStatus::Interrupted));
                }
                messages.push(self.success(
                    request,
                    ResponseResult::Execute(ExecuteResult { interrupted }),
                ));
            }
            Err(mut error) => {
                for event in std::mem::take(&mut error.events) {
                    messages.push(self.event_message(event));
                }
                messages.push(self.failure(request, error.into_protocol_error()));
            }
        }
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message(KernelStatus::Idle));
        messages
    }

    fn inspect(
        &mut self,
        request: &RequestEnvelope,
        parameters: &InspectRequest,
    ) -> Vec<ServerMessage> {
        if let Some(failure) = self.require_initialized(request) {
            return vec![failure];
        }
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message(KernelStatus::Busy)];
        let response = match self.engine.inspect(parameters) {
            Ok(preview) => match preview.validate() {
                Ok(()) => self.success(request, ResponseResult::Inspect(preview)),
                Err(error) => self.failure(
                    request,
                    ProtocolError::new("engine.invalidPreview", error.to_string()),
                ),
            },
            Err(error) => self.failure(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message(KernelStatus::Idle));
        messages
    }

    fn list_workspace(&mut self, request: &RequestEnvelope) -> Vec<ServerMessage> {
        if let Some(failure) = self.require_initialized(request) {
            return vec![failure];
        }
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message(KernelStatus::Busy)];
        let response = match self.engine.list_workspace() {
            Ok(mut variables) => {
                variables.sort_by(|left, right| left.name.cmp(&right.name));
                self.success(
                    request,
                    ResponseResult::ListWorkspace(WorkspaceSummary { variables }),
                )
            }
            Err(error) => self.failure(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message(KernelStatus::Idle));
        messages
    }

    fn shutdown(&mut self, request: &RequestEnvelope) -> Vec<ServerMessage> {
        let response = match self.engine.shutdown() {
            Ok(()) => self.success(request, ResponseResult::Shutdown(ShutdownResult {})),
            Err(error) => self.failure(request, error.into_protocol_error()),
        };
        self.set_status(KernelStatus::Dead);
        vec![response, self.status_message(KernelStatus::Dead)]
    }

    fn require_initialized(&self, request: &RequestEnvelope) -> Option<ServerMessage> {
        (!self.initialized).then(|| {
            self.failure(
                request,
                ProtocolError::new(
                    "kernel.notInitialized",
                    "initialize must succeed before workspace requests",
                ),
            )
        })
    }

    fn set_status(&self, status: KernelStatus) {
        self.control
            .state
            .store(status_to_u8(status), Ordering::Release);
    }

    fn success(&self, request: &RequestEnvelope, result: ResponseResult) -> ServerMessage {
        ServerMessage::Response(ResponseEnvelope::success(
            request,
            self.control.next_id(),
            result,
        ))
    }

    fn failure(&self, request: &RequestEnvelope, error: ProtocolError) -> ServerMessage {
        ServerMessage::Response(ResponseEnvelope::failure(
            request,
            self.control.next_id(),
            error,
        ))
    }

    fn event_message(&self, event: Event) -> ServerMessage {
        ServerMessage::Event(EventEnvelope::new(
            &self.control.session_id,
            self.control.next_id(),
            event,
        ))
    }

    fn status_message(&self, status: KernelStatus) -> ServerMessage {
        ServerMessage::Event(self.control.status_event(status))
    }
}

/// Cloneable control path for cooperative interrupts.
#[derive(Clone, Debug)]
pub struct KernelControl {
    session_id: String,
    state: Arc<AtomicU8>,
    cancellation: CancellationToken,
    next_message_id: Arc<AtomicU64>,
}

impl KernelControl {
    /// Returns the latest lifecycle state without taking the sequential engine
    /// lock.
    #[must_use]
    pub fn status(&self) -> KernelStatus {
        u8_to_status(self.state.load(Ordering::Acquire))
    }

    /// Handles only an interrupt request and never borrows the execution engine.
    /// It is safe to call while [`ExecutionEngine::execute`] is running.
    #[must_use]
    pub fn handle_interrupt(&self, request: &RequestEnvelope) -> Vec<ServerMessage> {
        if let Err(error) = request.validate() {
            return vec![self.failure(
                request,
                ProtocolError::new("protocol.validation", error.to_string()),
            )];
        }
        if request.session_id != self.session_id {
            return vec![self.failure(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            )];
        }
        if !matches!(request.request, Request::Interrupt(_)) {
            return vec![self.failure(
                request,
                ProtocolError::new(
                    "protocol.controlRequest",
                    "control path accepts only interrupt requests",
                ),
            )];
        }

        let status = self.status();
        let accepted = matches!(status, KernelStatus::Busy | KernelStatus::Interrupted);
        let response = ServerMessage::Response(ResponseEnvelope::success(
            request,
            self.next_id(),
            ResponseResult::Interrupt(InterruptResult { accepted }),
        ));
        if !accepted {
            return vec![response];
        }

        self.cancellation.cancel();
        if status == KernelStatus::Busy {
            self.state
                .store(status_to_u8(KernelStatus::Interrupted), Ordering::Release);
            vec![
                response,
                ServerMessage::Event(self.status_event(KernelStatus::Interrupted)),
            ]
        } else {
            vec![response]
        }
    }

    fn next_id(&self) -> String {
        let sequence = self.next_message_id.fetch_add(1, Ordering::Relaxed);
        format!("kernel-{sequence}")
    }

    fn status_event(&self, status: KernelStatus) -> EventEnvelope {
        EventEnvelope::new(
            &self.session_id,
            self.next_id(),
            Event::Status(StatusEvent { status }),
        )
    }

    fn failure(&self, request: &RequestEnvelope, error: ProtocolError) -> ServerMessage {
        ServerMessage::Response(ResponseEnvelope::failure(request, self.next_id(), error))
    }
}

/// Placeholder adapter used by the local server before a runtime integration is
/// available. It never claims to execute source code.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableEngine;

impl ExecutionEngine for UnavailableEngine {
    fn implementation(&self) -> ImplementationInfo {
        ImplementationInfo {
            name: "openmat-runtime-boundary".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            execution_modes: Vec::new(),
            display_mime_types: vec!["text/plain".to_owned()],
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            interrupt: true,
            workspace_delta: true,
        }
    }

    fn execute(
        &mut self,
        _request: &ExecuteRequest,
        _cancellation: &CancellationToken,
    ) -> Result<ExecutionOutput, EngineError> {
        Err(EngineError::new(
            "runtime.unavailable",
            "no parser/VM adapter is linked into this kernel",
        ))
    }

    fn inspect(&mut self, _request: &InspectRequest) -> Result<MatrixPreview, EngineError> {
        Err(EngineError::new(
            "workspace.notFound",
            "no runtime workspace is linked into this kernel",
        ))
    }

    fn list_workspace(&mut self) -> Result<Vec<VariableSummary>, EngineError> {
        Ok(Vec::new())
    }

    fn shutdown(&mut self) -> Result<(), EngineError> {
        Ok(())
    }
}

const fn status_to_u8(status: KernelStatus) -> u8 {
    match status {
        KernelStatus::Starting => 0,
        KernelStatus::Idle => 1,
        KernelStatus::Busy => 2,
        KernelStatus::Interrupted => 3,
        KernelStatus::Dead => 4,
    }
}

const fn u8_to_status(status: u8) -> KernelStatus {
    match status {
        0 => KernelStatus::Starting,
        1 => KernelStatus::Idle,
        2 => KernelStatus::Busy,
        3 => KernelStatus::Interrupted,
        _ => KernelStatus::Dead,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use openmat_protocol::{
        ExecutionMode, InitializeRequest, InterruptRequest, ListWorkspaceRequest, Request,
        ShutdownRequest, StreamEvent, StreamKind,
    };

    use super::*;

    #[derive(Default)]
    struct MockEngine {
        started: Option<Arc<AtomicBool>>,
        wait_for_interrupt: bool,
        shut_down: bool,
    }

    impl ExecutionEngine for MockEngine {
        fn implementation(&self) -> ImplementationInfo {
            ImplementationInfo {
                name: "mock-engine".to_owned(),
                version: "1.0".to_owned(),
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
                max_preview_elements: 64,
                interrupt: true,
                workspace_delta: true,
            }
        }

        fn execute(
            &mut self,
            request: &ExecuteRequest,
            cancellation: &CancellationToken,
        ) -> Result<ExecutionOutput, EngineError> {
            if let Some(started) = &self.started {
                started.store(true, Ordering::Release);
            }
            while self.wait_for_interrupt && !cancellation.is_cancelled() {
                thread::yield_now();
            }
            Ok(ExecutionOutput {
                result: ExecuteResult {
                    interrupted: cancellation.is_cancelled(),
                },
                events: vec![Event::Stream(StreamEvent {
                    stream: StreamKind::Stdout,
                    text: request.code.clone(),
                })],
            })
        }

        fn inspect(&mut self, _request: &InspectRequest) -> Result<MatrixPreview, EngineError> {
            unreachable!("not used by these tests")
        }

        fn list_workspace(&mut self) -> Result<Vec<VariableSummary>, EngineError> {
            Ok(vec![
                VariableSummary {
                    name: "z".to_owned(),
                    class: "double".to_owned(),
                    dimensions: vec![1, 1],
                    complex: false,
                    bytes: Some(8),
                },
                VariableSummary {
                    name: "a".to_owned(),
                    class: "logical".to_owned(),
                    dimensions: vec![1, 1],
                    complex: false,
                    bytes: Some(1),
                },
            ])
        }

        fn shutdown(&mut self) -> Result<(), EngineError> {
            self.shut_down = true;
            Ok(())
        }
    }

    fn request(message_id: &str, request: Request) -> RequestEnvelope {
        RequestEnvelope::new("session-1", message_id, request)
    }

    fn initialize() -> RequestEnvelope {
        request(
            "initialize-1",
            Request::Initialize(InitializeRequest {
                client: ImplementationInfo {
                    name: "test-client".to_owned(),
                    version: "1.0".to_owned(),
                },
                supported_protocols: vec![PROTOCOL_V0.to_owned()],
                capabilities: Capabilities {
                    execution_modes: vec![ExecutionMode::File, ExecutionMode::Repl],
                    display_mime_types: vec!["text/plain".to_owned()],
                    max_preview_elements: 32,
                    interrupt: true,
                    workspace_delta: true,
                },
            }),
        )
    }

    fn statuses(messages: &[ServerMessage]) -> Vec<KernelStatus> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerMessage::Event(EventEnvelope {
                    event: Event::Status(StatusEvent { status }),
                    ..
                }) => Some(*status),
                _ => None,
            })
            .collect()
    }

    fn response(messages: &[ServerMessage]) -> &ResponseEnvelope {
        messages
            .iter()
            .find_map(|message| match message {
                ServerMessage::Response(response) => Some(response),
                ServerMessage::Event(_) => None,
            })
            .expect("response message")
    }

    #[test]
    fn full_lifecycle_is_ordered_and_correlated() {
        let mut kernel = Kernel::new("session-1", MockEngine::default());
        assert_eq!(
            statuses(&[kernel.startup_event()]),
            vec![KernelStatus::Starting]
        );

        let initialize_request = initialize();
        let initialize_messages = kernel.handle_request(&initialize_request);
        assert!(response(&initialize_messages).is_reply_to(&initialize_request));
        assert_eq!(statuses(&initialize_messages), vec![KernelStatus::Idle]);

        let execute_request = request(
            "execute-1",
            Request::Execute(ExecuteRequest {
                code: "disp(1)".to_owned(),
                source_name: "cell-1".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        let execute_messages = kernel.handle_request(&execute_request);
        assert!(response(&execute_messages).is_reply_to(&execute_request));
        assert_eq!(
            statuses(&execute_messages),
            vec![KernelStatus::Busy, KernelStatus::Idle]
        );
        assert!(execute_messages.iter().any(|message| matches!(
            message,
            ServerMessage::Event(EventEnvelope {
                event: Event::Stream(_),
                ..
            })
        )));

        let workspace_request = request(
            "workspace-1",
            Request::ListWorkspace(ListWorkspaceRequest {}),
        );
        let workspace_messages = kernel.handle_request(&workspace_request);
        let Some(ResponseResult::ListWorkspace(summary)) = &response(&workspace_messages).result
        else {
            panic!("expected workspace result");
        };
        assert_eq!(
            summary
                .variables
                .iter()
                .map(|variable| variable.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "z"]
        );

        let shutdown_request = request("shutdown-1", Request::Shutdown(ShutdownRequest {}));
        let shutdown_messages = kernel.handle_request(&shutdown_request);
        assert_eq!(statuses(&shutdown_messages), vec![KernelStatus::Dead]);
        assert_eq!(kernel.status(), KernelStatus::Dead);

        let after_shutdown = kernel.handle_request(&request(
            "after-death",
            Request::ListWorkspace(ListWorkspaceRequest {}),
        ));
        assert!(!response(&after_shutdown).ok);
        assert_eq!(
            response(&after_shutdown)
                .error
                .as_ref()
                .expect("dead error")
                .category,
            "kernel.dead"
        );
    }

    #[test]
    fn interrupt_uses_independent_control_path() {
        let started = Arc::new(AtomicBool::new(false));
        let engine = MockEngine {
            started: Some(Arc::clone(&started)),
            wait_for_interrupt: true,
            shut_down: false,
        };
        let mut kernel = Kernel::new("session-1", engine);
        let _ = kernel.handle_request(&initialize());
        let control = kernel.control();
        let execute_request = request(
            "execute-blocking",
            Request::Execute(ExecuteRequest {
                code: "wait".to_owned(),
                source_name: "interrupt-test".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );

        let execution = thread::spawn(move || kernel.handle_request(&execute_request));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            started.load(Ordering::Acquire),
            "mock execution did not start"
        );

        let interrupt_request = request("interrupt-1", Request::Interrupt(InterruptRequest {}));
        let control_messages = control.handle_interrupt(&interrupt_request);
        assert!(response(&control_messages).is_reply_to(&interrupt_request));
        assert_eq!(statuses(&control_messages), vec![KernelStatus::Interrupted]);
        let Some(ResponseResult::Interrupt(result)) = &response(&control_messages).result else {
            panic!("expected interrupt result");
        };
        assert!(result.accepted);

        let execution_messages = execution.join().expect("execution thread");
        let Some(ResponseResult::Execute(result)) = &response(&execution_messages).result else {
            panic!("expected execution result");
        };
        assert!(result.interrupted);
        assert_eq!(control.status(), KernelStatus::Idle);
    }

    #[test]
    fn uninitialized_and_wrong_session_requests_fail_without_engine_access() {
        let mut kernel = Kernel::new("session-1", MockEngine::default());
        let uninitialized = kernel.handle_request(&request(
            "workspace-1",
            Request::ListWorkspace(ListWorkspaceRequest {}),
        ));
        assert_eq!(
            response(&uninitialized)
                .error
                .as_ref()
                .expect("not initialized")
                .category,
            "kernel.notInitialized"
        );

        let wrong_session = RequestEnvelope::new("session-2", "initialize-2", initialize().request);
        let mismatched = kernel.handle_request(&wrong_session);
        assert_eq!(
            response(&mismatched)
                .error
                .as_ref()
                .expect("session mismatch")
                .category,
            "protocol.sessionMismatch"
        );
    }
}

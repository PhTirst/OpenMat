#![doc = "Local transport boundary for an `OpenMat` kernel session."]

use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use openmat_kernel::{
    CancellationToken, EngineError, ExecutionEngine, ExecutionOutput, Kernel, KernelControl,
    KernelSession, KernelSessionError, KernelSessionMessage, KernelSessionRequest, RuntimeEngine,
};
use openmat_openblas::OpenBlasProvider;
use openmat_protocol::kernel_v1;
use openmat_protocol::kernel_v2::{self, ProtocolVersion};
use openmat_protocol::kernel_v3;
use openmat_protocol::{
    Capabilities, ExecuteRequest, ImplementationInfo, InspectRequest, KernelStatus, MatrixPreview,
    RequestEnvelope, ServerMessage, ValidationError, VariableSummary,
};
use openmat_runtime::LinalgProvider;

pub mod graphics;
mod graphics_runtime;
mod graphics_websocket;
mod lsp_websocket;
mod lsp_workspace;
mod simulation_execution;
mod simulation_websocket;
mod websocket;
mod workspace;

const EXIT_SUCCESS: i32 = 0;
const EXIT_USAGE: i32 = 64;
const EXIT_DATA_ERROR: i32 = 65;
const EXIT_SOFTWARE: i32 = 70;
const EXIT_IO_ERROR: i32 = 74;

/// A transport-neutral session dispatcher.
pub struct SessionService<E> {
    kernel: Kernel<E>,
}

impl<E: ExecutionEngine> SessionService<E> {
    /// Creates one local session around an engine adapter.
    #[must_use]
    pub fn new(session_id: impl Into<String>, engine: E) -> Self {
        Self {
            kernel: Kernel::new(session_id, engine),
        }
    }

    /// Returns the session's initial `starting` event.
    #[must_use]
    pub fn startup_message(&self) -> ServerMessage {
        self.kernel.startup_event()
    }

    /// Returns the independent cooperative-interrupt control path.
    #[must_use]
    pub fn control(&self) -> KernelControl {
        self.kernel.control()
    }

    /// Dispatches one already-decoded request.
    #[must_use]
    pub fn dispatch(&mut self, request: &RequestEnvelope) -> Vec<ServerMessage> {
        self.kernel.handle_request(request)
    }

    /// Returns the current kernel lifecycle state.
    #[must_use]
    pub fn status(&self) -> KernelStatus {
        self.kernel.status()
    }
}

/// Failure in a local server transport.
#[derive(Debug)]
pub enum ServerError {
    /// Reading from or writing to the local stdio channel failed.
    Io(io::Error),
    /// Binding, accepting, or configuring the WebSocket channel failed.
    WebSocketIo(io::Error),
    /// One input line was not a decodable request envelope.
    Decode {
        /// One-based input line number.
        line: u64,
        /// JSON decoding failure.
        source: serde_json::Error,
    },
    /// The first decoded request failed protocol validation.
    InvalidRequest {
        /// One-based input line number.
        line: u64,
        /// Protocol validation failure.
        source: ValidationError,
    },
    /// The session engine could not be initialized.
    EngineInitialization(EngineError),
    /// A kernel message could not be encoded.
    Encode(serde_json::Error),
    /// A versioned message failed its protocol codec.
    ProtocolCodec(kernel_v1::CodecError),
    /// A v2-capable message failed its protocol codec or frame-size bound.
    ProtocolV2Codec(kernel_v2::CodecError),
    /// A versioned graphics DTO or frame failed protocol validation.
    GraphicsProtocol(openmat_plot_protocol::ValidationError),
    /// A kernel-owned graphics hub rejected an operation.
    GraphicsHub(graphics::GraphicsHubError),
    /// Server-owned graphics attachment lifecycle failed.
    GraphicsRegistry(graphics::GraphicsRegistryError),
    /// A negotiated kernel session rejected a frame that cannot receive a reply.
    KernelSession(KernelSessionError),
    /// The per-connection kernel worker stopped after an internal failure.
    Worker(String),
    /// A WebSocket handshake or frame operation failed.
    WebSocket(tungstenite::Error),
    /// Internal transport lifecycle invariant failed.
    State(&'static str),
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "stdio transport failed: {error}"),
            Self::WebSocketIo(error) => write!(formatter, "WebSocket I/O failed: {error}"),
            Self::Decode { line, source } => {
                write!(formatter, "invalid request JSON on line {line}: {source}")
            }
            Self::InvalidRequest { line, source } => {
                write!(
                    formatter,
                    "invalid initial request on line {line}: {source}"
                )
            }
            Self::EngineInitialization(error) => {
                write!(formatter, "could not initialize runtime engine: {error}")
            }
            Self::Encode(error) => write!(formatter, "could not encode kernel message: {error}"),
            Self::ProtocolCodec(error) => {
                write!(formatter, "kernel protocol codec failed: {error}")
            }
            Self::ProtocolV2Codec(error) => {
                write!(formatter, "kernel v2 protocol codec failed: {error}")
            }
            Self::GraphicsProtocol(error) => {
                write!(formatter, "graphics protocol failed: {error}")
            }
            Self::GraphicsHub(error) => write!(formatter, "graphics hub failed: {error}"),
            Self::GraphicsRegistry(error) => {
                write!(formatter, "graphics registry failed: {error}")
            }
            Self::KernelSession(error) => write!(formatter, "kernel session failed: {error}"),
            Self::Worker(error) => write!(formatter, "kernel worker failed: {error}"),
            Self::WebSocket(error) => write!(formatter, "WebSocket transport failed: {error}"),
            Self::State(message) => write!(formatter, "invalid server state: {message}"),
        }
    }
}

impl Error for ServerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) | Self::WebSocketIo(error) => Some(error),
            Self::Decode { source, .. } | Self::Encode(source) => Some(source),
            Self::InvalidRequest { source, .. } => Some(source),
            Self::EngineInitialization(error) => Some(error),
            Self::ProtocolCodec(error) => Some(error),
            Self::ProtocolV2Codec(error) => Some(error),
            Self::GraphicsProtocol(error) => Some(error),
            Self::GraphicsHub(error) => Some(error),
            Self::GraphicsRegistry(error) => Some(error),
            Self::KernelSession(error) => Some(error),
            Self::WebSocket(error) => Some(error),
            Self::Worker(_) | Self::State(_) => None,
        }
    }
}

impl ServerError {
    /// Returns the stable process exit code for this server failure category.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Decode { .. } | Self::InvalidRequest { .. } => EXIT_DATA_ERROR,
            Self::EngineInitialization(_)
            | Self::Encode(_)
            | Self::ProtocolCodec(_)
            | Self::ProtocolV2Codec(_)
            | Self::GraphicsProtocol(_)
            | Self::GraphicsHub(_)
            | Self::GraphicsRegistry(_)
            | Self::KernelSession(_)
            | Self::Worker(_)
            | Self::WebSocket(_)
            | Self::State(_) => EXIT_SOFTWARE,
            Self::Io(_) | Self::WebSocketIo(_) => EXIT_IO_ERROR,
        }
    }
}

impl From<io::Error> for ServerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<tungstenite::Error> for ServerError {
    fn from(error: tungstenite::Error) -> Self {
        Self::WebSocket(error)
    }
}

/// Serves one kernel session as newline-delimited JSON over local stdio.
///
/// No TCP listener is created in this mode. The embedding parent process owns
/// access control for the inherited pipes. The loopback/WebSocket binding uses
/// the same negotiated [`KernelSession`] boundary with asynchronous control.
///
/// The first valid request selects the opaque session ID and creates the engine.
/// Requests are then dispatched sequentially. An orderly shutdown response and
/// `dead` event end the stream immediately. Interrupt-capable asynchronous
/// transports should route interrupt frames through `KernelSessionControl`;
/// stdio remains deliberately sequential under either negotiated protocol.
///
/// # Errors
///
/// Returns [`ServerError`] on transport failure, invalid JSON, or serialization
/// failure. Malformed input terminates the stdio session because it may not
/// contain trustworthy correlation identifiers.
pub fn serve_json_lines<R, W, F, E>(
    reader: R,
    writer: &mut W,
    engine_factory: F,
) -> Result<(), ServerError>
where
    R: BufRead,
    W: Write,
    F: FnOnce() -> E,
    E: ExecutionEngine,
{
    serve_json_lines_with_factory(reader, writer, || Ok(engine_factory()))
}

/// Serves one kernel session using a fallible, lazily invoked engine factory.
///
/// The first non-empty input line is decoded as bootstrap initialize before the
/// factory is called. Engine construction must then succeed before the session's
/// `starting` event is written. Malformed first frames and invalid envelope
/// headers never announce a session; a structurally valid offer with invalid v1
/// capabilities reaches [`KernelSession`] and receives its structured failure.
///
/// The stdio binding is sequential, so its negotiated capabilities always mask
/// asynchronous interrupt support even when the underlying engine supports an
/// independent control path.
///
/// # Errors
///
/// Returns [`ServerError`] on transport failure, invalid input, engine
/// initialization failure, or message serialization failure.
pub fn serve_json_lines_with_factory<R, W, F, E>(
    mut reader: R,
    writer: &mut W,
    engine_factory: F,
) -> Result<(), ServerError>
where
    R: BufRead,
    W: Write,
    F: FnOnce() -> Result<E, EngineError>,
    E: ExecutionEngine,
{
    let mut session = None;
    let mut factory = Some(engine_factory);
    let mut line = String::new();
    let mut line_number = 0_u64;

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        line_number += 1;
        let frame = line.strip_suffix('\n').unwrap_or(&line);
        let frame = frame.strip_suffix('\r').unwrap_or(frame);
        if frame.trim().is_empty() {
            continue;
        }
        if session
            .as_ref()
            .and_then(KernelSession::negotiated_protocol)
            .is_none()
        {
            let request = decode_bootstrap_request(frame, line_number)?;
            if session.is_none() {
                let create_engine = factory.take().ok_or(ServerError::State(
                    "engine factory was consumed before session initialization",
                ))?;
                let engine = create_engine().map_err(ServerError::EngineInitialization)?;
                let new_session =
                    KernelSession::new(&request.session_id, SequentialStdioEngine::new(engine));
                write_message(writer, &starting_message(&request.session_id))?;
                session = Some(new_session);
            }
            let active_session = session
                .as_mut()
                .ok_or(ServerError::State("session was not initialized"))?;
            let messages = active_session
                .handle(KernelSessionRequest::Bootstrap(&request))
                .map_err(ServerError::KernelSession)?;
            write_bootstrap_messages(writer, messages)?;
            writer.flush()?;
            continue;
        }

        let active_session = session
            .as_mut()
            .ok_or(ServerError::State("session was not initialized"))?;
        match active_session
            .negotiated_protocol()
            .ok_or(ServerError::State("session protocol was not negotiated"))?
        {
            ProtocolVersion::V0 => {
                let request: RequestEnvelope =
                    serde_json::from_str(frame).map_err(|source| ServerError::Decode {
                        line: line_number,
                        source,
                    })?;
                let messages = active_session
                    .handle(KernelSessionRequest::V0(&request))
                    .map_err(ServerError::KernelSession)?;
                write_v0_messages(writer, messages)?;
            }
            ProtocolVersion::V1 => {
                let frames = active_session
                    .handle_v1_frame(frame)
                    .map_err(ServerError::KernelSession)?;
                for frame in frames {
                    write_frame(writer, &frame)?;
                }
            }
            ProtocolVersion::V2 => {
                let frames = active_session
                    .handle_v2_frame(frame)
                    .map_err(ServerError::KernelSession)?;
                for frame in frames {
                    write_frame(writer, &frame)?;
                }
            }
            ProtocolVersion::V3 => {
                let frames = active_session
                    .handle_v3_frame(frame)
                    .map_err(ServerError::KernelSession)?;
                for frame in frames {
                    write_frame(writer, &frame)?;
                }
            }
        }
        writer.flush()?;
        if active_session.status() == KernelStatus::Dead {
            break;
        }
    }
    Ok(())
}

struct SequentialStdioEngine<E> {
    inner: E,
}

impl<E> SequentialStdioEngine<E> {
    const fn new(inner: E) -> Self {
        Self { inner }
    }
}

impl<E: ExecutionEngine> ExecutionEngine for SequentialStdioEngine<E> {
    fn implementation(&self) -> ImplementationInfo {
        self.inner.implementation()
    }

    fn capabilities(&self) -> Capabilities {
        let mut capabilities = self.inner.capabilities();
        capabilities.interrupt = false;
        capabilities
    }

    fn v1_capabilities(&self) -> kernel_v1::Capabilities {
        let mut capabilities = self.inner.v1_capabilities();
        capabilities.interrupt = false;
        capabilities
    }

    fn v2_capabilities(&self) -> kernel_v2::Capabilities {
        let mut capabilities = self.inner.v2_capabilities();
        capabilities.interrupt = false;
        capabilities
    }

    fn execute(
        &mut self,
        request: &ExecuteRequest,
        cancellation: &CancellationToken,
    ) -> Result<ExecutionOutput, EngineError> {
        self.inner.execute(request, cancellation)
    }

    fn inspect(&mut self, request: &InspectRequest) -> Result<MatrixPreview, EngineError> {
        self.inner.inspect(request)
    }

    fn inspect_v1(
        &mut self,
        request: &kernel_v1::InspectRequest,
        limits: &kernel_v1::PreviewLimits,
    ) -> Result<kernel_v1::MatrixPreview, EngineError> {
        self.inner.inspect_v1(request, limits)
    }

    fn inspect_v2(
        &mut self,
        request: &kernel_v2::InspectRequest,
        limits: &kernel_v2::AggregateLimits,
    ) -> Result<kernel_v2::InspectPreview, EngineError> {
        self.inner.inspect_v2(request, limits)
    }

    fn set_variable_element(
        &mut self,
        request: &kernel_v3::SetVariableElementRequest,
    ) -> Result<VariableSummary, EngineError> {
        self.inner.set_variable_element(request)
    }

    fn list_workspace(&mut self) -> Result<Vec<VariableSummary>, EngineError> {
        self.inner.list_workspace()
    }

    fn shutdown(&mut self) -> Result<(), EngineError> {
        self.inner.shutdown()
    }
}

fn write_message<W: Write>(writer: &mut W, message: &ServerMessage) -> Result<(), ServerError> {
    let json = serde_json::to_string(message).map_err(ServerError::Encode)?;
    writeln!(writer, "{json}")?;
    Ok(())
}

fn decode_bootstrap_request(
    input: &str,
    line: u64,
) -> Result<kernel_v2::BootstrapRequestEnvelope, ServerError> {
    kernel_v2::decode_bootstrap_request(input)
        .map_err(|error| match error {
            kernel_v2::CodecError::Json(source) => ServerError::Decode { line, source },
            kernel_v2::CodecError::Validation(source) => {
                ServerError::InvalidRequest { line, source }
            }
            error @ (kernel_v2::CodecError::FrameTooLarge { .. }
            | kernel_v2::CodecError::Utf8(_)
            | kernel_v2::CodecError::Utf8Bom) => ServerError::ProtocolV2Codec(error),
        })
        .or_else(|error| match &error {
            ServerError::InvalidRequest { source, .. }
                if source.category() == "protocol.invalidCapabilities" =>
            {
                serde_json::from_str(input).map_err(|source| ServerError::Decode { line, source })
            }
            _ => Err(error),
        })
}

fn starting_message(session_id: &str) -> ServerMessage {
    ServerMessage::Event(openmat_protocol::EventEnvelope::new(
        session_id,
        "transport-starting",
        openmat_protocol::Event::Status(openmat_protocol::StatusEvent {
            status: KernelStatus::Starting,
        }),
    ))
}

fn write_bootstrap_messages<W: Write>(
    writer: &mut W,
    messages: Vec<KernelSessionMessage>,
) -> Result<(), ServerError> {
    for message in messages {
        let frame = match message {
            KernelSessionMessage::Bootstrap(response) => {
                kernel_v2::encode_bootstrap_response(&response)
                    .map_err(ServerError::ProtocolV2Codec)?
            }
            KernelSessionMessage::V0(message) => {
                serde_json::to_string(&message).map_err(ServerError::Encode)?
            }
            KernelSessionMessage::V1(kernel_v1::ServerMessage::Event(event)) => {
                kernel_v1::encode_event(&event).map_err(ServerError::ProtocolCodec)?
            }
            KernelSessionMessage::V1(kernel_v1::ServerMessage::Response(_)) => {
                return Err(ServerError::State(
                    "bootstrap produced an uncorrelated v1 response",
                ));
            }
            KernelSessionMessage::V2(kernel_v2::ServerMessage::Event(event)) => {
                kernel_v2::encode_event(&event).map_err(ServerError::ProtocolV2Codec)?
            }
            KernelSessionMessage::V2(kernel_v2::ServerMessage::Response(_)) => {
                return Err(ServerError::State(
                    "bootstrap produced an uncorrelated v2 response",
                ));
            }
            KernelSessionMessage::V3(kernel_v3::ServerMessage::Event(event)) => {
                kernel_v3::encode_event(&event).map_err(ServerError::ProtocolV2Codec)?
            }
            KernelSessionMessage::V3(kernel_v3::ServerMessage::Response(_)) => {
                return Err(ServerError::State(
                    "bootstrap produced an uncorrelated v3 response",
                ));
            }
        };
        write_frame(writer, &frame)?;
    }
    Ok(())
}

fn write_v0_messages<W: Write>(
    writer: &mut W,
    messages: Vec<KernelSessionMessage>,
) -> Result<(), ServerError> {
    for message in messages {
        let KernelSessionMessage::V0(message) = message else {
            return Err(ServerError::State(
                "v0 request produced a non-v0 session message",
            ));
        };
        write_message(writer, &message)?;
    }
    Ok(())
}

fn write_frame<W: Write>(writer: &mut W, frame: &str) -> Result<(), ServerError> {
    writeln!(writer, "{frame}")?;
    Ok(())
}

/// Runs the local server frontend and returns a process exit code.
///
/// With no option, or with `--stdio`, it serves JSON lines on inherited stdio.
/// `--listen` binds loopback-only WebSocket endpoints at `/kernel`, `/lsp`,
/// and the versioned `/graphics/v1` through `/graphics/v4` endpoints.
/// Supplying `--workspace-root` on that listener also enables the independent,
/// versioned `/workspace/v1` compatibility, `/workspace/v2` file APIs, and
/// `/workspace/v3` file APIs with one-use HTTP downloads under
/// `/workspace/download/`.
/// `--openblas-dll` selects a validated LP64 `OpenBLAS` provider by absolute path.
/// Each repeated `--oex-plugin` selects one trusted native library in load order.
pub fn run<I, S, R, W, E>(args: I, reader: R, writer: &mut W, errors: &mut E) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    R: BufRead,
    W: Write,
    E: Write,
{
    let arguments: Vec<String> = args.into_iter().map(Into::into).collect();
    let (arguments, oex_plugins) = match extract_oex_plugins(arguments) {
        Ok(configuration) => configuration,
        Err(message) => {
            let _ = writeln!(errors, "openmat-server: {message}; use --help");
            return EXIT_USAGE;
        }
    };
    let options = arguments.get(1..).unwrap_or_default();
    match options {
        [option, address] if option == "--listen" => {
            return run_websocket(address, None, None, oex_plugins, writer, errors);
        }
        [option, address, workspace_option, workspace_root]
            if option == "--listen" && workspace_option == "--workspace-root" =>
        {
            return run_websocket(
                address,
                Some(workspace_root),
                None,
                oex_plugins,
                writer,
                errors,
            );
        }
        [option, address, openblas_option, openblas_dll]
            if option == "--listen" && openblas_option == "--openblas-dll" =>
        {
            return run_websocket(
                address,
                None,
                Some(openblas_dll),
                oex_plugins,
                writer,
                errors,
            );
        }
        [
            option,
            address,
            workspace_option,
            workspace_root,
            openblas_option,
            openblas_dll,
        ] if option == "--listen"
            && workspace_option == "--workspace-root"
            && openblas_option == "--openblas-dll" =>
        {
            return run_websocket(
                address,
                Some(workspace_root),
                Some(openblas_dll),
                oex_plugins,
                writer,
                errors,
            );
        }
        _ => {}
    }
    run_with_engine_factory(arguments, reader, writer, errors, move || {
        // SAFETY: `--oex-plugin` is an explicit command-line trust decision.
        unsafe { RuntimeEngine::with_oex_plugins(oex_plugins) }
    })
}

fn extract_oex_plugins(arguments: Vec<String>) -> Result<(Vec<String>, Vec<PathBuf>), String> {
    let mut filtered = Vec::with_capacity(arguments.len());
    let mut plugins = Vec::new();
    let mut arguments = arguments.into_iter();
    if let Some(program) = arguments.next() {
        filtered.push(program);
    }
    while let Some(argument) = arguments.next() {
        if argument == "--oex-plugin" {
            let path = arguments
                .next()
                .ok_or_else(|| "--oex-plugin requires a library path".to_owned())?;
            if path.is_empty() || path.starts_with("--") {
                return Err("--oex-plugin requires a non-empty library path".to_owned());
            }
            plugins.push(PathBuf::from(path));
        } else {
            filtered.push(argument);
        }
    }
    Ok((filtered, plugins))
}

fn run_websocket<W: Write, D: Write>(
    address: &str,
    workspace_root: Option<&str>,
    openblas_dll: Option<&str>,
    oex_plugins: Vec<PathBuf>,
    writer: &mut W,
    errors: &mut D,
) -> i32 {
    let address = match address.parse::<SocketAddr>() {
        Ok(address) if address.ip().is_loopback() => address,
        Ok(_) => {
            let _ = writeln!(errors, "openmat-server: --listen address must be loopback");
            return EXIT_USAGE;
        }
        Err(error) => {
            let _ = writeln!(errors, "openmat-server: invalid --listen address: {error}");
            return EXIT_USAGE;
        }
    };
    let linalg_provider = match openblas_dll {
        Some(path) => match OpenBlasProvider::load(Path::new(path)) {
            Ok(provider) => {
                let provider: Arc<dyn LinalgProvider> = Arc::new(provider);
                Some(provider)
            }
            Err(error) => {
                let _ = writeln!(
                    errors,
                    "openmat-server: failed to initialize OpenBLAS: {error}"
                );
                return EXIT_SOFTWARE;
            }
        },
        None => None,
    };
    let listener = match TcpListener::bind(address) {
        Ok(listener) => listener,
        Err(error) => {
            let _ = writeln!(errors, "openmat-server: could not bind {address}: {error}");
            return EXIT_IO_ERROR;
        }
    };
    let local_address = match listener.local_addr() {
        Ok(address) => address,
        Err(error) => {
            let _ = writeln!(
                errors,
                "openmat-server: could not read listener address: {error}"
            );
            return EXIT_IO_ERROR;
        }
    };
    let workspace = match workspace_root {
        Some(root) => match workspace::WorkspaceService::new(Path::new(root)) {
            Ok(workspace) => Some(workspace),
            Err(error) => {
                let _ = writeln!(errors, "openmat-server: invalid --workspace-root: {error}");
                return EXIT_USAGE;
            }
        },
        None => None,
    };
    if writeln!(writer, "ws://{local_address}/kernel")
        .and_then(|()| writer.flush())
        .is_err()
    {
        let _ = writeln!(errors, "openmat-server: could not announce listener URL");
        return EXIT_IO_ERROR;
    }

    let graphics = graphics::GraphicsSessionRegistry::new();
    match websocket::serve(
        &listener,
        errors,
        workspace.as_ref(),
        &graphics,
        linalg_provider.as_ref(),
        oex_plugins,
    ) {
        Ok(()) => EXIT_SUCCESS,
        Err(error) => {
            let exit_code = error.exit_code();
            let _ = writeln!(errors, "openmat-server: {error}");
            exit_code
        }
    }
}

fn run_with_engine_factory<I, S, R, W, D, F, T>(
    args: I,
    reader: R,
    writer: &mut W,
    errors: &mut D,
    engine_factory: F,
) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    R: BufRead,
    W: Write,
    D: Write,
    F: FnOnce() -> Result<T, EngineError>,
    T: ExecutionEngine,
{
    let arguments: Vec<String> = args.into_iter().map(Into::into).collect();
    let options = arguments.get(1..).unwrap_or_default();
    match options {
        [] => serve_stdio(reader, writer, errors, engine_factory),
        [option] if option == "--stdio" => serve_stdio(reader, writer, errors, engine_factory),
        [option] if option == "--version" => {
            let _ = writeln!(writer, "openmat-server {}", env!("CARGO_PKG_VERSION"));
            EXIT_SUCCESS
        }
        [option] if option == "--help" || option == "-h" => {
            let _ = writeln!(
                writer,
                "Usage: openmat-server [--stdio|--listen 127.0.0.1:<port> [--workspace-root <directory>] [--openblas-dll <absolute-path>]] [--oex-plugin <library>]...|--version|--help"
            );
            let _ = writeln!(
                writer,
                "\n--stdio runs negotiated kernel JSON lines on inherited stdio."
            );
            let _ = writeln!(
                writer,
                "--listen runs loopback-only WebSocket endpoints at /kernel, /lsp (openmat-lsp-websocket-v1), /graphics/v1 (openmat-graphics-v1), /graphics/v2 (openmat-graphics-v2), /graphics/v3 (openmat-graphics-v3), and /graphics/v4 (openmat-graphics-v4).\n--workspace-root additionally enables the independent /workspace/v1 compatibility, /workspace/v2 file APIs, and /workspace/v3 file APIs with one-use HTTP downloads under /workspace/download/.\n--openblas-dll loads a trusted LP64 OpenBLAS DLL from an explicit absolute path.\n--oex-plugin loads one trusted native OEX library in process and may be repeated."
            );
            EXIT_SUCCESS
        }
        _ => {
            let _ = writeln!(errors, "openmat-server: unsupported arguments; use --help");
            EXIT_USAGE
        }
    }
}

fn serve_stdio<R, W, D, F, T>(reader: R, writer: &mut W, errors: &mut D, engine_factory: F) -> i32
where
    R: BufRead,
    W: Write,
    D: Write,
    F: FnOnce() -> Result<T, EngineError>,
    T: ExecutionEngine,
{
    match serve_json_lines_with_factory(reader, writer, engine_factory) {
        Ok(()) => EXIT_SUCCESS,
        Err(error) => {
            let exit_code = error.exit_code();
            let _ = writeln!(errors, "openmat-server: {error}");
            exit_code
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::io::Cursor;
    use std::rc::Rc;

    use openmat_protocol::kernel_v1;
    use openmat_protocol::kernel_v2;
    use openmat_protocol::{
        Event, EventEnvelope, ExecuteResult, ExecutionMode, InitializeRequest, InspectRequest,
        ListWorkspaceRequest, MatrixRange, PROTOCOL_V0, PreviewTruncation, PreviewValue, Request,
        ResponseEnvelope, ResponseResult, ShutdownRequest, StatusEvent, StreamEvent, StreamKind,
    };

    use super::*;

    #[derive(Clone)]
    struct FakeEngine {
        calls: Rc<RefCell<Vec<&'static str>>>,
    }

    impl FakeEngine {
        fn new(calls: Rc<RefCell<Vec<&'static str>>>) -> Self {
            Self { calls }
        }

        fn record(&self, call: &'static str) {
            self.calls.borrow_mut().push(call);
        }
    }

    impl ExecutionEngine for FakeEngine {
        fn implementation(&self) -> ImplementationInfo {
            self.record("implementation");
            ImplementationInfo {
                name: "openmat-server-fake".to_owned(),
                version: "1.0".to_owned(),
            }
        }

        fn capabilities(&self) -> Capabilities {
            self.record("capabilities");
            Capabilities {
                execution_modes: vec![ExecutionMode::Repl],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: 64,
                interrupt: true,
                workspace_delta: true,
            }
        }

        fn execute(
            &mut self,
            _request: &ExecuteRequest,
            _cancellation: &CancellationToken,
        ) -> Result<ExecutionOutput, EngineError> {
            self.record("execute");
            Ok(ExecutionOutput {
                result: ExecuteResult { interrupted: false },
                events: vec![Event::Stream(StreamEvent {
                    stream: StreamKind::Stdout,
                    text: "42\n".to_owned(),
                })],
            })
        }

        fn inspect(&mut self, _request: &InspectRequest) -> Result<MatrixPreview, EngineError> {
            self.record("inspect");
            Ok(MatrixPreview {
                class: "double".to_owned(),
                dimensions: vec![1, 1],
                selected_range: MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 1],
                },
                values: vec![PreviewValue::Number { value: 42.0 }],
                truncation: PreviewTruncation {
                    truncated: false,
                    omitted_elements: 0,
                },
            })
        }

        fn inspect_v1(
            &mut self,
            request: &kernel_v1::InspectRequest,
            _limits: &kernel_v1::PreviewLimits,
        ) -> Result<kernel_v1::MatrixPreview, EngineError> {
            self.record("inspect_v1");
            Ok(kernel_v1::MatrixPreview {
                class: "double".to_owned(),
                dimensions: vec![1, 1],
                complex: false,
                selected_range: request.range.clone(),
                values: vec![kernel_v1::PreviewValue::Number { value: 42.0 }],
                truncation: PreviewTruncation {
                    truncated: false,
                    omitted_elements: 0,
                },
            })
        }

        fn inspect_v2(
            &mut self,
            request: &kernel_v2::InspectRequest,
            _limits: &kernel_v2::AggregateLimits,
        ) -> Result<kernel_v2::InspectPreview, EngineError> {
            self.record("inspect_v2");
            Ok(kernel_v2::InspectPreview::Aggregate(
                kernel_v2::AggregatePreview::Cell {
                    dimensions: vec![1, 2],
                    selected_range: request.range.clone(),
                    items: vec![
                        kernel_v2::ExactValue {
                            class: "double".to_owned(),
                            size: vec![1, 1],
                            ndims: 2,
                            numel: 1,
                            complex: false,
                            payload: kernel_v2::ExactPayload::Numeric {
                                real: vec!["7".to_owned()],
                                imag: vec!["0".to_owned()],
                            },
                        },
                        kernel_v2::ExactValue {
                            class: "struct".to_owned(),
                            size: vec![0, 3],
                            ndims: 2,
                            numel: 0,
                            complex: false,
                            payload: kernel_v2::ExactPayload::Struct {
                                fields: vec!["beta".to_owned(), "alpha".to_owned()],
                                records: Vec::new(),
                            },
                        },
                    ],
                    truncation: PreviewTruncation {
                        truncated: false,
                        omitted_elements: 0,
                    },
                    usage: kernel_v2::PreviewUsage {
                        nodes: 3,
                        elements: 3,
                        code_units: 9,
                        depth: 1,
                    },
                },
            ))
        }

        fn list_workspace(&mut self) -> Result<Vec<VariableSummary>, EngineError> {
            self.record("list_workspace");
            Ok(vec![VariableSummary {
                name: "answer".to_owned(),
                class: "double".to_owned(),
                dimensions: vec![1, 1],
                complex: false,
                bytes: Some(8),
            }])
        }

        fn shutdown(&mut self) -> Result<(), EngineError> {
            self.record("shutdown");
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
                    name: "server-test".to_owned(),
                    version: "1.0".to_owned(),
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

    fn v1_initialize() -> kernel_v1::BootstrapRequestEnvelope {
        kernel_v1::BootstrapRequestEnvelope::new(
            "session-1",
            "initialize-v1",
            kernel_v1::InitializeRequest::v1(
                ImplementationInfo {
                    name: "server-v1-test".to_owned(),
                    version: "1.0".to_owned(),
                },
                kernel_v1::Capabilities {
                    execution_modes: vec![ExecutionMode::Repl],
                    display_mime_types: vec!["text/plain".to_owned()],
                    max_preview_elements: 32,
                    max_string_element_code_units: Some(1024),
                    max_preview_code_units: Some(4096),
                    interrupt: true,
                    workspace_delta: true,
                },
            ),
        )
    }

    fn v2_initialize() -> kernel_v2::BootstrapRequestEnvelope {
        kernel_v2::BootstrapRequestEnvelope::new(
            "session-1",
            "initialize-v2",
            kernel_v2::InitializeRequest::v2(
                ImplementationInfo {
                    name: "server-v2-test".to_owned(),
                    version: "2.0".to_owned(),
                },
                kernel_v2::Capabilities {
                    execution_modes: vec![ExecutionMode::Repl],
                    display_mime_types: vec!["text/plain".to_owned()],
                    max_preview_elements: 32,
                    max_string_element_code_units: Some(1024),
                    max_preview_code_units: Some(4096),
                    max_aggregate_nodes: Some(128),
                    max_aggregate_elements: Some(256),
                    max_aggregate_depth: Some(8),
                    interrupt: true,
                    workspace_delta: true,
                },
            ),
        )
    }

    fn lifecycle_requests() -> [RequestEnvelope; 5] {
        [
            initialize(),
            request(
                "execute-1",
                Request::Execute(ExecuteRequest {
                    code: "answer = 42".to_owned(),
                    source_name: "lifecycle-test".to_owned(),
                    mode: ExecutionMode::Repl,
                }),
            ),
            request(
                "inspect-1",
                Request::Inspect(InspectRequest {
                    name: "answer".to_owned(),
                    range: MatrixRange {
                        start: vec![1, 1],
                        size: vec![1, 1],
                    },
                    max_elements: 1,
                }),
            ),
            request(
                "workspace-1",
                Request::ListWorkspace(ListWorkspaceRequest {}),
            ),
            request("shutdown-1", Request::Shutdown(ShutdownRequest {})),
        ]
    }

    fn encode_requests(requests: &[RequestEnvelope]) -> String {
        requests
            .iter()
            .map(|request| serde_json::to_string(request).expect("encode request"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn decode_messages(output: &[u8]) -> Vec<ServerMessage> {
        String::from_utf8(output.to_vec())
            .expect("UTF-8 output")
            .lines()
            .map(|line| serde_json::from_str::<ServerMessage>(line).expect("server message"))
            .inspect(|message| match message {
                ServerMessage::Response(response) => {
                    response.validate().expect("valid response envelope");
                }
                ServerMessage::Event(event) => {
                    event.validate().expect("valid event envelope");
                }
            })
            .collect()
    }

    fn response_for<'a>(messages: &'a [ServerMessage], reply_to: &str) -> &'a ResponseEnvelope {
        messages
            .iter()
            .find_map(|message| match message {
                ServerMessage::Response(response) if response.reply_to == reply_to => {
                    Some(response)
                }
                ServerMessage::Response(_) | ServerMessage::Event(_) => None,
            })
            .expect("response for request")
    }

    #[test]
    fn fallible_factory_runs_complete_sequential_lifecycle_and_stops_at_shutdown() {
        let requests = lifecycle_requests();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let factory_calls = Rc::clone(&calls);
        let mut output = Vec::new();
        let input = format!("{}\nnot-json", encode_requests(&requests));

        serve_json_lines_with_factory(Cursor::new(input), &mut output, move || {
            factory_calls.borrow_mut().push("factory");
            Ok(FakeEngine::new(factory_calls))
        })
        .expect("serve JSON lines");

        assert_eq!(
            calls.borrow().as_slice(),
            [
                "factory",
                "capabilities",
                "implementation",
                "execute",
                "inspect",
                "list_workspace",
                "shutdown",
            ]
        );

        let messages = decode_messages(&output);
        assert!(matches!(
            messages.first(),
            Some(ServerMessage::Event(EventEnvelope {
                event: Event::Status(StatusEvent {
                    status: openmat_protocol::KernelStatus::Starting
                }),
                ..
            }))
        ));
        assert!(matches!(
            messages.last(),
            Some(ServerMessage::Event(EventEnvelope {
                event: Event::Status(StatusEvent {
                    status: openmat_protocol::KernelStatus::Dead
                }),
                ..
            }))
        ));
        let initialize_response = response_for(&messages, "initialize-1");
        let Some(ResponseResult::Initialize(result)) = &initialize_response.result else {
            panic!("expected initialize result");
        };
        assert!(
            !result.capabilities.interrupt,
            "sequential stdio must not negotiate asynchronous interrupt"
        );
        assert!(messages.iter().any(|message| matches!(
            message,
            ServerMessage::Event(EventEnvelope {
                event: Event::Stream(StreamEvent { text, .. }),
                ..
            }) if text == "42\n"
        )));
        let replies = messages
            .iter()
            .filter_map(|message| match message {
                ServerMessage::Response(ResponseEnvelope { reply_to, .. }) => {
                    Some(reply_to.as_str())
                }
                ServerMessage::Event(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            replies,
            vec![
                "initialize-1",
                "execute-1",
                "inspect-1",
                "workspace-1",
                "shutdown-1",
            ]
        );
    }

    #[test]
    fn stdio_negotiates_v1_and_uses_request_aware_frames_through_shutdown() {
        let bootstrap = v1_initialize();
        let limits = kernel_v1::PreviewLimits::new(32, 1024, 4096).unwrap();
        let execute = kernel_v1::RequestEnvelope::new(
            "session-1",
            "execute-v1",
            kernel_v1::Request::Execute(ExecuteRequest {
                code: "answer = 42".to_owned(),
                source_name: "stdio-v1.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        let inspect = kernel_v1::RequestEnvelope::new(
            "session-1",
            "inspect-v1",
            kernel_v1::Request::Inspect(kernel_v1::InspectRequest {
                name: "answer".to_owned(),
                range: kernel_v1::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 1],
                },
                max_elements: 1,
            }),
        );
        let list = kernel_v1::RequestEnvelope::new(
            "session-1",
            "list-v1",
            kernel_v1::Request::ListWorkspace(ListWorkspaceRequest {}),
        );
        let shutdown = kernel_v1::RequestEnvelope::new(
            "session-1",
            "shutdown-v1",
            kernel_v1::Request::Shutdown(ShutdownRequest {}),
        );
        let input = [
            kernel_v1::encode_bootstrap_request(&bootstrap).unwrap(),
            kernel_v1::encode_request(&execute, &limits).unwrap(),
            kernel_v1::encode_request(&inspect, &limits).unwrap(),
            kernel_v1::encode_request(&list, &limits).unwrap(),
            kernel_v1::encode_request(&shutdown, &limits).unwrap(),
        ]
        .join("\n");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let factory_calls = Rc::clone(&calls);
        let mut output = Vec::new();

        serve_json_lines_with_factory(Cursor::new(input), &mut output, move || {
            factory_calls.borrow_mut().push("factory");
            Ok(FakeEngine::new(factory_calls))
        })
        .expect("serve negotiated v1 JSON lines");

        let frames = String::from_utf8(output).unwrap();
        let frames = frames.lines().collect::<Vec<_>>();
        assert!(matches!(
            serde_json::from_str::<ServerMessage>(frames[0]).unwrap(),
            ServerMessage::Event(EventEnvelope {
                event: Event::Status(StatusEvent {
                    status: KernelStatus::Starting
                }),
                ..
            })
        ));
        let initialized = kernel_v1::decode_bootstrap_response(frames[1]).unwrap();
        let Some(kernel_v1::BootstrapResponseResult::Initialize(result)) = initialized.result
        else {
            panic!("v1 initialize result");
        };
        assert_eq!(result.negotiated_protocol, kernel_v1::PROTOCOL_V1);
        assert!(!result.capabilities.interrupt);
        assert!(matches!(
            kernel_v1::decode_event(frames[2]).unwrap().event,
            Event::Status(StatusEvent {
                status: KernelStatus::Idle
            })
        ));

        for request in [&execute, &inspect, &list, &shutdown] {
            let response = frames.iter().find_map(|frame| {
                kernel_v1::decode_response_for_request(frame, request, &limits).ok()
            });
            assert!(response.is_some_and(|response| response.ok));
        }
        assert!(frames.iter().all(|frame| {
            serde_json::from_str::<serde_json::Value>(frame)
                .unwrap()
                .get("protocol")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|protocol| {
                    protocol == openmat_protocol::PROTOCOL_V0 || protocol == kernel_v1::PROTOCOL_V1
                })
        }));
        assert_eq!(
            calls.borrow().as_slice(),
            [
                "factory",
                "capabilities",
                "implementation",
                "execute",
                "inspect_v1",
                "list_workspace",
                "shutdown",
            ]
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One typed stdio lifecycle verifies every v2 phase in order.
    fn stdio_v2_lifecycle_routes_typed_aggregate_frames_through_shutdown() {
        let bootstrap = v2_initialize();
        let limits = kernel_v2::AggregateLimits::new(32, 1024, 4096, 128, 256, 8).unwrap();
        let execute = kernel_v2::RequestEnvelope::new(
            "session-1",
            "execute-v2",
            kernel_v2::Request::Execute(ExecuteRequest {
                code: "aggregate = {7, shaped};".to_owned(),
                source_name: "stdio-v2.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        let inspect = kernel_v2::RequestEnvelope::new(
            "session-1",
            "inspect-v2",
            kernel_v2::Request::Inspect(kernel_v2::InspectRequest {
                name: "aggregate".to_owned(),
                range: kernel_v2::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 2],
                },
                max_elements: 2,
            }),
        );
        let list = kernel_v2::RequestEnvelope::new(
            "session-1",
            "list-v2",
            kernel_v2::Request::ListWorkspace(ListWorkspaceRequest {}),
        );
        let shutdown = kernel_v2::RequestEnvelope::new(
            "session-1",
            "shutdown-v2",
            kernel_v2::Request::Shutdown(ShutdownRequest {}),
        );
        let input = [
            kernel_v2::encode_bootstrap_request(&bootstrap).unwrap(),
            kernel_v2::encode_request(&execute, &limits).unwrap(),
            kernel_v2::encode_request(&inspect, &limits).unwrap(),
            kernel_v2::encode_request(&list, &limits).unwrap(),
            kernel_v2::encode_request(&shutdown, &limits).unwrap(),
        ]
        .join("\n");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let factory_calls = Rc::clone(&calls);
        let mut output = Vec::new();

        serve_json_lines_with_factory(Cursor::new(input), &mut output, move || {
            factory_calls.borrow_mut().push("factory");
            Ok(FakeEngine::new(factory_calls))
        })
        .expect("serve negotiated v2 JSON lines");

        let frames = String::from_utf8(output).unwrap();
        let frames = frames.lines().collect::<Vec<_>>();
        assert!(matches!(
            serde_json::from_str::<ServerMessage>(frames[0]).unwrap(),
            ServerMessage::Event(EventEnvelope {
                event: Event::Status(StatusEvent {
                    status: KernelStatus::Starting
                }),
                ..
            })
        ));
        let initialized = kernel_v2::decode_bootstrap_response(frames[1]).unwrap();
        let Some(kernel_v2::BootstrapResponseResult::Initialize(result)) = initialized.result
        else {
            panic!("v2 initialize result");
        };
        assert_eq!(result.negotiated_protocol, kernel_v2::PROTOCOL_V2);
        assert!(!result.capabilities.interrupt);
        assert!(matches!(
            kernel_v2::decode_event(frames[2]).unwrap().event,
            Event::Status(StatusEvent {
                status: KernelStatus::Idle
            })
        ));

        for request in [&execute, &inspect, &list, &shutdown] {
            assert!(frames.iter().any(|frame| {
                kernel_v2::decode_response_for_request(frame, request, &limits)
                    .is_ok_and(|response| response.ok)
            }));
        }
        let inspect_frame = frames
            .iter()
            .find(|frame| {
                serde_json::from_str::<serde_json::Value>(frame)
                    .ok()
                    .and_then(|value| value.get("replyTo").cloned())
                    .is_some_and(|reply_to| reply_to == "inspect-v2")
            })
            .expect("correlated aggregate inspect frame");
        assert!(inspect_frame.len() <= kernel_v2::MAX_JSON_FRAME_BYTES);
        let inspected = kernel_v2::decode_response_for_request(inspect_frame, &inspect, &limits)
            .expect("typed aggregate response");
        let Some(kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Aggregate(
            kernel_v2::AggregatePreview::Cell { items, usage, .. },
        ))) = inspected.result
        else {
            panic!("aggregate inspect result");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(usage.nodes, 3);
        assert!(frames.iter().skip(2).all(|frame| {
            serde_json::from_str::<serde_json::Value>(frame)
                .unwrap()
                .get("protocol")
                .and_then(serde_json::Value::as_str)
                == Some(kernel_v2::PROTOCOL_V2)
        }));
        assert_eq!(
            calls.borrow().as_slice(),
            [
                "factory",
                "capabilities",
                "implementation",
                "execute",
                "inspect_v2",
                "list_workspace",
                "shutdown",
            ]
        );
    }

    #[test]
    fn invalid_first_request_does_not_call_fallible_factory_or_announce_session() {
        let mut invalid = initialize();
        invalid.session_id.clear();
        let input = serde_json::to_string(&invalid).expect("encode invalid request");
        let factory_called = Cell::new(false);
        let mut output = Vec::new();

        let error = serve_json_lines_with_factory(Cursor::new(input), &mut output, || {
            factory_called.set(true);
            Ok(FakeEngine::new(Rc::new(RefCell::new(Vec::new()))))
        })
        .expect_err("initial request must be validated");

        assert!(matches!(error, ServerError::InvalidRequest { line: 1, .. }));
        assert!(!factory_called.get());
        assert!(output.is_empty(), "startup must not precede validation");
    }

    #[test]
    fn invalid_v1_capabilities_receive_bootstrap_failure_and_allow_v0_retry() {
        let mut invalid = v1_initialize();
        let kernel_v1::BootstrapRequest::Initialize(parameters) = &mut invalid.request;
        parameters.capabilities.max_string_element_code_units = None;
        parameters.capabilities.max_preview_code_units = None;
        let retry = kernel_v1::BootstrapRequestEnvelope::new(
            "session-1",
            "initialize-v0-retry",
            kernel_v1::InitializeRequest {
                client: ImplementationInfo {
                    name: "retry-client".to_owned(),
                    version: "1".to_owned(),
                },
                supported_protocols: vec![openmat_protocol::PROTOCOL_V0.to_owned()],
                capabilities: kernel_v1::Capabilities {
                    max_preview_elements: 16,
                    ..kernel_v1::Capabilities::default()
                },
            },
        );
        let input = format!(
            "{}\n{}",
            serde_json::to_string(&invalid).unwrap(),
            kernel_v1::encode_bootstrap_request(&retry).unwrap()
        );
        let factory_called = Cell::new(false);
        let mut output = Vec::new();

        serve_json_lines_with_factory(Cursor::new(input), &mut output, || {
            factory_called.set(true);
            Ok(FakeEngine::new(Rc::new(RefCell::new(Vec::new()))))
        })
        .expect("retry bootstrap after invalid capabilities");

        assert!(factory_called.get());
        let frames = String::from_utf8(output).unwrap();
        let frames = frames.lines().collect::<Vec<_>>();
        let failure = kernel_v1::decode_bootstrap_response(frames[1]).unwrap();
        assert_eq!(
            failure.error.expect("invalid capability error").category,
            "protocol.invalidCapabilities"
        );
        let success = kernel_v1::decode_bootstrap_response(frames[2]).unwrap();
        let Some(kernel_v1::BootstrapResponseResult::Initialize(result)) = success.result else {
            panic!("retry initialize result");
        };
        assert_eq!(result.negotiated_protocol, openmat_protocol::PROTOCOL_V0);
        assert!(matches!(
            serde_json::from_str::<ServerMessage>(frames[3]).unwrap(),
            ServerMessage::Event(EventEnvelope {
                event: Event::Status(StatusEvent {
                    status: KernelStatus::Idle
                }),
                ..
            })
        ));
    }

    #[test]
    fn malformed_first_frame_does_not_call_fallible_factory() {
        let factory_called = Cell::new(false);
        let mut output = Vec::new();

        let error = serve_json_lines_with_factory(Cursor::new("not-json\n"), &mut output, || {
            factory_called.set(true);
            Ok(FakeEngine::new(Rc::new(RefCell::new(Vec::new()))))
        })
        .expect_err("initial frame must decode");

        assert!(matches!(error, ServerError::Decode { line: 1, .. }));
        assert!(!factory_called.get());
        assert!(output.is_empty());
    }

    #[test]
    fn factory_error_is_typed_and_has_no_startup_output() {
        let mut output = Vec::new();
        let error = serve_json_lines_with_factory(
            Cursor::new(encode_requests(&[initialize()])),
            &mut output,
            || -> Result<FakeEngine, EngineError> {
                Err(EngineError::new(
                    "engine.bootstrap",
                    "missing runtime service",
                ))
            },
        )
        .expect_err("engine initialization must fail");

        assert!(matches!(error, ServerError::EngineInitialization(_)));
        assert_eq!(error.exit_code(), EXIT_SOFTWARE);
        assert!(error.to_string().contains("engine.bootstrap"));
        assert!(output.is_empty(), "startup must follow engine construction");
    }

    #[test]
    fn infallible_serve_json_lines_api_remains_compatible() {
        let requests = [
            initialize(),
            request("shutdown-1", Request::Shutdown(ShutdownRequest {})),
        ];
        let calls = Rc::new(RefCell::new(Vec::new()));
        let factory_calls = Rc::clone(&calls);
        let mut output = Vec::new();

        serve_json_lines(
            Cursor::new(encode_requests(&requests)),
            &mut output,
            move || FakeEngine::new(factory_calls),
        )
        .expect("infallible compatibility wrapper");

        assert_eq!(
            calls.borrow().as_slice(),
            ["capabilities", "implementation", "shutdown"]
        );
        let messages = decode_messages(&output);
        assert!(response_for(&messages, "initialize-1").ok);
        assert!(response_for(&messages, "shutdown-1").ok);
    }

    #[test]
    fn default_and_explicit_stdio_modes_use_runtime_engine_boundary() {
        let requests = [
            initialize(),
            request("shutdown-1", Request::Shutdown(ShutdownRequest {})),
        ];
        let input = encode_requests(&requests);
        let runtime_implementation = RuntimeEngine::new()
            .expect("runtime engine construction boundary")
            .implementation();

        for arguments in [vec!["openmat-server"], vec!["openmat-server", "--stdio"]] {
            let mut output = Vec::new();
            let mut errors = Vec::new();
            let exit = run(
                arguments,
                Cursor::new(input.as_bytes()),
                &mut output,
                &mut errors,
            );

            assert_eq!(exit, EXIT_SUCCESS);
            assert!(errors.is_empty());
            let messages = decode_messages(&output);
            let response = response_for(&messages, "initialize-1");
            let Some(ResponseResult::Initialize(result)) = &response.result else {
                panic!("expected initialize result");
            };
            assert_eq!(result.implementation, runtime_implementation);
        }
    }

    #[test]
    fn malformed_json_has_deterministic_data_error_exit_code() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            ["openmat-server", "--stdio"],
            Cursor::new("not-json\n"),
            &mut output,
            &mut errors,
        );
        assert_eq!(exit, EXIT_DATA_ERROR);
        assert!(output.is_empty());
        assert!(
            String::from_utf8(errors)
                .expect("UTF-8 error")
                .contains("line 1")
        );
    }

    #[test]
    fn engine_initialization_failure_has_stable_software_exit_code() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run_with_engine_factory(
            ["openmat-server", "--stdio"],
            Cursor::new(encode_requests(&[initialize()])),
            &mut output,
            &mut errors,
            || -> Result<FakeEngine, EngineError> {
                Err(EngineError::new("engine.bootstrap", "test failure"))
            },
        );

        assert_eq!(exit, EXIT_SOFTWARE);
        assert!(output.is_empty());
        assert!(
            String::from_utf8(errors)
                .expect("UTF-8 error")
                .contains("engine.bootstrap")
        );
    }

    #[test]
    fn oex_plugin_options_preserve_order_and_require_paths() {
        let (filtered, plugins) = extract_oex_plugins(vec![
            "openmat-server".to_owned(),
            "--oex-plugin".to_owned(),
            "first.oex.dll".to_owned(),
            "--listen".to_owned(),
            "127.0.0.1:0".to_owned(),
            "--oex-plugin".to_owned(),
            "second.oex.dll".to_owned(),
        ])
        .expect("valid plugin options");
        assert_eq!(filtered, ["openmat-server", "--listen", "127.0.0.1:0"]);
        assert_eq!(
            plugins,
            [
                PathBuf::from("first.oex.dll"),
                PathBuf::from("second.oex.dll")
            ]
        );
        assert!(
            extract_oex_plugins(vec![
                "openmat-server".to_owned(),
                "--stdio".to_owned(),
                "--oex-plugin".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn stdio_reports_missing_oex_plugin_before_session_startup() {
        let missing = std::env::temp_dir().join(format!(
            "openmat-missing-oex-{}-{}.dll",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-server".to_owned(),
                "--stdio".to_owned(),
                "--oex-plugin".to_owned(),
                missing.to_string_lossy().into_owned(),
            ],
            Cursor::new(encode_requests(&[initialize()])),
            &mut output,
            &mut errors,
        );
        assert_eq!(exit, EXIT_SOFTWARE);
        assert!(output.is_empty());
        let errors = String::from_utf8(errors).expect("UTF-8 errors");
        assert!(errors.contains("oex.load"), "{errors}");
        assert!(errors.contains("failed to resolve OEX plugin"), "{errors}");
    }

    #[test]
    fn version_help_and_bad_arguments_have_stable_outputs_and_exit_codes() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(
            run(
                ["openmat-server", "--version"],
                Cursor::new(Vec::<u8>::new()),
                &mut output,
                &mut errors,
            ),
            EXIT_SUCCESS
        );
        assert!(
            String::from_utf8(output)
                .expect("UTF-8 output")
                .starts_with("openmat-server ")
        );
        assert!(errors.is_empty());

        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(
            run(
                ["openmat-server", "--help"],
                Cursor::new(Vec::<u8>::new()),
                &mut output,
                &mut errors,
            ),
            EXIT_SUCCESS
        );
        let help = String::from_utf8(output).expect("UTF-8 help");
        assert!(help.contains("JSON lines"));
        assert!(help.contains("WebSocket"));
        assert!(help.contains("127.0.0.1:<port>"));
        assert!(help.contains("/workspace/v2"));
        assert!(help.contains("/workspace/v3"));
        assert!(help.contains("/lsp (openmat-lsp-websocket-v1)"));
        assert!(help.contains("/graphics/v2 (openmat-graphics-v2)"));
        assert!(help.contains("/graphics/v3 (openmat-graphics-v3)"));
        assert!(help.contains("/graphics/v4 (openmat-graphics-v4)"));
        assert!(help.contains("--openblas-dll <absolute-path>"));
        assert!(help.contains("--oex-plugin <library>"));
        assert!(errors.is_empty());

        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(
            run(
                ["openmat-server", "--listen", "0.0.0.0:9999"],
                Cursor::new(Vec::<u8>::new()),
                &mut output,
                &mut errors,
            ),
            EXIT_USAGE
        );
        assert!(output.is_empty());
        assert!(
            String::from_utf8(errors)
                .expect("UTF-8 error")
                .contains("must be loopback")
        );

        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(
            run(
                [
                    "openmat-server",
                    "--listen",
                    "127.0.0.1:0",
                    "--openblas-dll",
                    "libopenblas.dll",
                ],
                Cursor::new(Vec::<u8>::new()),
                &mut output,
                &mut errors,
            ),
            EXIT_SOFTWARE
        );
        assert!(output.is_empty());
        assert!(
            String::from_utf8(errors)
                .expect("UTF-8 error")
                .contains("OpenBLAS DLL path must be absolute")
        );
    }
}

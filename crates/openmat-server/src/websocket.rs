use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use openmat_kernel::{
    CancellationToken, EngineError, ExecutionEngine, ExecutionOutput, KernelSession,
    KernelSessionControl, KernelSessionMessage, KernelSessionRequest, RuntimeEngine,
};
use openmat_protocol::kernel_v1;
use openmat_protocol::kernel_v2::{self, ProtocolVersion};
use openmat_protocol::kernel_v3;
use openmat_protocol::{
    Capabilities, DisplayEvent, Event, EventEnvelope, ExecuteRequest, ImplementationInfo,
    InspectRequest, KernelStatus, MatrixPreview, ProtocolError, Request, RequestEnvelope,
    ServerMessage, ShutdownRequest, StatusEvent, VariableSummary,
};
use openmat_runtime::{LinalgProvider, WorkingDirectory};
use tungstenite::handshake::HandshakeError;
use tungstenite::handshake::server::{ErrorResponse, Request as HandshakeRequest};
use tungstenite::http::{StatusCode, Uri};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Message, WebSocket, WebSocketConfig};
use tungstenite::{Error as WebSocketError, accept_hdr_with_config};

use crate::graphics::{GraphicsRegistration, GraphicsSessionRegistry};
use crate::graphics_runtime::RuntimeGraphicsHub;
use crate::workspace::{WorkspaceProtocol, WorkspaceService, entry_value_v2};
use crate::{ServerError, starting_message};

pub(crate) const MAX_MESSAGE_BYTES: usize = kernel_v2::MAX_JSON_FRAME_BYTES;
const MAX_WIRE_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PENDING_REQUESTS: usize = 32;
pub(crate) const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const DOWNLOAD_WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const UPLOAD_READ_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const WORKSPACE_CHANGE_DEBOUNCE: Duration = Duration::from_millis(120);
const MAX_HTTP_HEAD_BYTES: usize = 16 * 1024;
const DOWNLOAD_PATH_PREFIX: &str = "/workspace/download/";
const UPLOAD_PATH_PREFIX: &str = "/workspace/upload/";

fn performance_logging_enabled() -> bool {
    std::env::var_os("OPENMAT_PERF_LOG").is_some_and(|value| value != "0")
}

fn duration_milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

pub(crate) fn serve<D: io::Write>(
    listener: &TcpListener,
    errors: &mut D,
    workspace: Option<&WorkspaceService>,
    graphics: &GraphicsSessionRegistry,
    linalg_provider: Option<&Arc<dyn LinalgProvider>>,
    oex_plugins: Vec<PathBuf>,
) -> Result<(), ServerError> {
    listener
        .set_nonblocking(true)
        .map_err(ServerError::WebSocketIo)?;
    let (error_sender, error_receiver) = mpsc::channel::<String>();
    let oex_plugins: Arc<[PathBuf]> = Arc::from(oex_plugins);

    loop {
        while let Ok(error) = error_receiver.try_recv() {
            writeln!(errors, "openmat-server: connection failed: {error}")?;
            errors.flush()?;
        }

        match listener.accept() {
            Ok((stream, _peer)) => {
                let errors = error_sender.clone();
                let workspace = workspace.cloned();
                let kernel_working_directory =
                    workspace.as_ref().map(WorkspaceService::working_directory);
                let kernel_search_path = workspace.as_ref().map(WorkspaceService::search_path);
                let graphics = graphics.clone();
                let linalg_provider = linalg_provider.cloned();
                let oex_plugins = Arc::clone(&oex_plugins);
                thread::Builder::new()
                    .name("openmat-ws-connection".to_owned())
                    .spawn(move || {
                        if let Err(error) = serve_connection_with_graphics(
                            stream,
                            move || {
                                let working_directory = if let Some(working_directory) =
                                    kernel_working_directory
                                {
                                    working_directory
                                } else {
                                    let path = std::env::current_dir().map_err(|error| {
                                        EngineError::new(
                                            "filesystem.workingDirectory",
                                            format!(
                                                "failed to resolve the session working directory: {error}"
                                            ),
                                        )
                                    })?;
                                    WorkingDirectory::new(path).map_err(|error| {
                                        EngineError::new(
                                            "filesystem.workingDirectory",
                                            format!(
                                                "failed to initialize the session filesystem: {error}"
                                            ),
                                        )
                                    })?
                                };
                                let search_path = kernel_search_path.unwrap_or_default();
                                match linalg_provider {
                                    Some(provider) => {
                                        // SAFETY: Server startup accepts OEX paths only through
                                        // the explicit trusted-native command-line option.
                                        unsafe {
                                            RuntimeEngine::with_shared_session_paths_linalg_provider_and_oex_plugins(
                                                working_directory,
                                                search_path,
                                                provider,
                                                oex_plugins.iter(),
                                            )
                                        }
                                    }
                                    None => {
                                        // SAFETY: The same explicit server option establishes
                                        // trust for every supplied native library.
                                        unsafe {
                                            RuntimeEngine::with_shared_session_paths_and_oex_plugins(
                                                working_directory,
                                                search_path,
                                                oex_plugins.iter(),
                                            )
                                        }
                                    }
                                }
                            },
                            workspace,
                            &graphics,
                        ) {
                            let _ = errors.send(error.to_string());
                        }
                    })
                    .map_err(ServerError::WebSocketIo)?;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(SOCKET_POLL_INTERVAL);
            }
            Err(error) => return Err(ServerError::WebSocketIo(error)),
        }
    }
}

#[cfg(test)]
fn serve_connection<F, E>(
    stream: TcpStream,
    engine_factory: F,
    workspace: Option<WorkspaceService>,
) -> Result<(), ServerError>
where
    F: FnOnce() -> Result<E, EngineError>,
    E: ExecutionEngine + Send + 'static,
{
    serve_connection_with_graphics(
        stream,
        engine_factory,
        workspace,
        &GraphicsSessionRegistry::new(),
    )
}

pub(crate) fn serve_connection_with_graphics<F, E>(
    stream: TcpStream,
    engine_factory: F,
    workspace: Option<WorkspaceService>,
    graphics: &GraphicsSessionRegistry,
) -> Result<(), ServerError>
where
    F: FnOnce() -> Result<E, EngineError>,
    E: ExecutionEngine + Send + 'static,
{
    // A nonblocking listener can yield an inherited nonblocking accepted
    // socket on Windows. Force connection sockets back to blocking mode before
    // applying bounded read timeouts; otherwise every endpoint can spin on
    // immediate WouldBlock errors and starve kernel/graphics work.
    stream
        .set_nonblocking(false)
        .map_err(ServerError::WebSocketIo)?;
    stream.set_nodelay(true).map_err(ServerError::WebSocketIo)?;
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(ServerError::WebSocketIo)?;
    stream
        .set_write_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(ServerError::WebSocketIo)?;
    if targets_workspace_http(&stream)? {
        return serve_workspace_http(stream, workspace.as_ref());
    }
    let config = WebSocketConfig::default()
        .read_buffer_size(16 * 1024)
        .write_buffer_size(0)
        .max_write_buffer_size(2 * MAX_WIRE_MESSAGE_BYTES)
        .max_message_size(Some(MAX_WIRE_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WIRE_MESSAGE_BYTES));
    let endpoint = Cell::new(None);
    let workspace_available = workspace.is_some();
    let mut socket = match accept_hdr_with_config(
        stream,
        |request: &HandshakeRequest, response| {
            validate_handshake(request, response, &endpoint, workspace_available)
        },
        Some(config),
    ) {
        Ok(socket) => socket,
        Err(HandshakeError::Failure(WebSocketError::Http(_))) => return Ok(()),
        Err(error) => return Err(handshake_error(error)),
    };
    socket
        .get_mut()
        .set_read_timeout(None)
        .map_err(ServerError::WebSocketIo)?;
    socket
        .get_mut()
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .map_err(ServerError::WebSocketIo)?;

    if matches!(
        endpoint.get(),
        Some(Endpoint::Workspace(
            WorkspaceProtocol::V1 | WorkspaceProtocol::V2 | WorkspaceProtocol::V3
        ))
    ) {
        socket
            .get_mut()
            .set_nonblocking(false)
            .map_err(ServerError::WebSocketIo)?;
        socket
            .get_mut()
            .set_read_timeout(Some(SOCKET_POLL_INTERVAL))
            .map_err(ServerError::WebSocketIo)?;
        let service = workspace.ok_or(ServerError::State(
            "workspace handshake succeeded without a configured workspace",
        ))?;
        let Endpoint::Workspace(protocol) = endpoint.get().ok_or(ServerError::State(
            "workspace handshake did not retain its protocol",
        ))?
        else {
            return Err(ServerError::State(
                "workspace endpoint changed after handshake",
            ));
        };
        return run_workspace_connection(&mut socket, &service, protocol);
    }
    if endpoint.get() == Some(Endpoint::Lsp) {
        socket
            .get_mut()
            .set_nonblocking(false)
            .map_err(ServerError::WebSocketIo)?;
        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(ServerError::WebSocketIo)?;
        return crate::lsp_websocket::serve(&mut socket, workspace.as_ref());
    }
    if let Some(Endpoint::Graphics(protocol)) = endpoint.get() {
        // Graphics connections poll the session hub between client frames, but
        // their binary buffer transfers can be much larger than the OS send
        // buffer. A nonblocking socket turns normal backpressure into a fatal
        // WouldBlock from tungstenite::send. Keep writes blocking and bound
        // only reads so the hub still receives regular polling opportunities.
        socket
            .get_mut()
            .set_nonblocking(false)
            .map_err(ServerError::WebSocketIo)?;
        socket
            .get_mut()
            .set_read_timeout(Some(SOCKET_POLL_INTERVAL))
            .map_err(ServerError::WebSocketIo)?;
        return crate::graphics_websocket::serve(&mut socket, graphics, protocol);
    }

    // Kernel execution happens on a worker thread. Nonblocking reads let this
    // connection deliver worker events without waiting for another client
    // frame; the poll branch below bounds idle CPU.
    socket
        .get_mut()
        .set_nonblocking(true)
        .map_err(ServerError::WebSocketIo)?;
    let mut factory = Some(engine_factory);
    let mut session = None;
    let outcome = run_connection(&mut socket, &mut factory, &mut session, graphics);
    if let Some(active_session) = session.as_mut() {
        active_session.stop();
    }
    outcome
}

fn targets_workspace_http(stream: &TcpStream) -> Result<bool, ServerError> {
    let started = Instant::now();
    let mut bytes = [0_u8; 4096];
    loop {
        let count = stream.peek(&mut bytes).map_err(ServerError::WebSocketIo)?;
        if count == 0 {
            return Ok(false);
        }
        if let Some(line_end) = bytes[..count].windows(2).position(|pair| pair == b"\r\n") {
            let Ok(line) = std::str::from_utf8(&bytes[..line_end]) else {
                return Ok(false);
            };
            let mut parts = line.split_ascii_whitespace();
            let method = parts.next();
            let target = parts.next();
            return Ok(matches!(method, Some("GET"))
                && target.is_some_and(|target| target.starts_with(DOWNLOAD_PATH_PREFIX))
                || matches!(method, Some("PUT" | "OPTIONS"))
                    && target.is_some_and(|target| target.starts_with(UPLOAD_PATH_PREFIX)));
        }
        if count == bytes.len() || started.elapsed() >= HANDSHAKE_TIMEOUT {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(1));
    }
}

struct HttpHead {
    text: String,
    body_prefix: Vec<u8>,
}

fn serve_workspace_http(
    mut stream: TcpStream,
    workspace: Option<&WorkspaceService>,
) -> Result<(), ServerError> {
    stream
        .set_write_timeout(Some(DOWNLOAD_WRITE_TIMEOUT))
        .map_err(ServerError::WebSocketIo)?;
    let head = match read_http_head(&mut stream) {
        Ok(head) => head,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return write_http_empty_response(&mut stream, "431 Request Header Fields Too Large");
        }
        Err(error) => return Err(ServerError::WebSocketIo(error)),
    };
    let Some(request_line) = head.text.split("\r\n").next() else {
        return write_http_empty_response(&mut stream, "400 Bad Request");
    };
    let mut parts = request_line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return write_http_empty_response(&mut stream, "400 Bad Request");
    };
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return write_http_empty_response(&mut stream, "400 Bad Request");
    }
    match (method, target) {
        ("GET", target) if target.starts_with(DOWNLOAD_PATH_PREFIX) => {
            serve_workspace_download(stream, workspace, target)
        }
        ("OPTIONS", target) if target.starts_with(UPLOAD_PATH_PREFIX) => {
            write_upload_preflight(&mut stream)
        }
        ("PUT", target) if target.starts_with(UPLOAD_PATH_PREFIX) => {
            stream
                .set_read_timeout(Some(UPLOAD_READ_TIMEOUT))
                .map_err(ServerError::WebSocketIo)?;
            serve_workspace_upload(stream, workspace, target, &head.text, &head.body_prefix)
        }
        _ => write_http_empty_response(&mut stream, "404 Not Found"),
    }
}

fn serve_workspace_download(
    mut stream: TcpStream,
    workspace: Option<&WorkspaceService>,
    target: &str,
) -> Result<(), ServerError> {
    let Some(ticket) = target.strip_prefix(DOWNLOAD_PATH_PREFIX) else {
        return write_http_empty_response(&mut stream, "404 Not Found");
    };
    let Some(workspace) = workspace else {
        return write_http_empty_response(&mut stream, "404 Not Found");
    };
    let Some(download) = workspace.consume_download(ticket) else {
        return write_http_empty_response(&mut stream, "404 Not Found");
    };
    let disposition = content_disposition(&download.name);
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: {disposition}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        download.size,
    )
    .map_err(ServerError::WebSocketIo)?;
    let expected_size = download.size;
    let mut limited = Read::take(download.file, expected_size);
    let copied = io::copy(&mut limited, &mut stream).map_err(ServerError::WebSocketIo)?;
    if copied != expected_size {
        return Err(ServerError::WebSocketIo(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "workspace download changed size while it was streaming",
        )));
    }
    stream.flush().map_err(ServerError::WebSocketIo)
}

fn serve_workspace_upload(
    mut stream: TcpStream,
    workspace: Option<&WorkspaceService>,
    target: &str,
    head: &str,
    body_prefix: &[u8],
) -> Result<(), ServerError> {
    let Some(ticket) = target.strip_prefix(UPLOAD_PATH_PREFIX) else {
        return write_http_json_response(
            &mut stream,
            "404 Not Found",
            &serde_json::json!({ "error": { "code": "workspace.uploadNotFound", "message": "Upload ticket was not found." } }),
        );
    };
    let content_length = match parse_content_length(head) {
        Ok(length) => length,
        Err(message) => {
            return write_http_json_response(
                &mut stream,
                "400 Bad Request",
                &serde_json::json!({ "error": { "code": "workspace.invalidUpload", "message": message } }),
            );
        }
    };
    let Some(workspace) = workspace else {
        return write_http_json_response(
            &mut stream,
            "404 Not Found",
            &serde_json::json!({ "error": { "code": "workspace.uploadNotFound", "message": "Upload ticket was not found." } }),
        );
    };
    let mut upload = match workspace.consume_upload(ticket, content_length) {
        Ok(Some(upload)) => upload,
        Ok(None) => {
            return write_http_json_response(
                &mut stream,
                "404 Not Found",
                &serde_json::json!({ "error": { "code": "workspace.uploadNotFound", "message": "Upload ticket was not found or has expired." } }),
            );
        }
        Err(error) => {
            return write_http_json_response(
                &mut stream,
                error.http_status(),
                &serde_json::json!({ "error": error.value() }),
            );
        }
    };
    let prefix_size = u64::try_from(body_prefix.len()).unwrap_or(u64::MAX);
    if prefix_size > upload.size() {
        return write_http_json_response(
            &mut stream,
            "400 Bad Request",
            &serde_json::json!({ "error": { "code": "workspace.uploadSizeMismatch", "message": "Upload body exceeds its declared size." } }),
        );
    }
    upload
        .writer()
        .write_all(body_prefix)
        .map_err(ServerError::WebSocketIo)?;
    let remaining = upload.size() - prefix_size;
    let mut limited = Read::take(&mut stream, remaining);
    let copied = io::copy(&mut limited, upload.writer()).map_err(ServerError::WebSocketIo)?;
    if copied != remaining {
        return write_http_json_response(
            &mut stream,
            "400 Bad Request",
            &serde_json::json!({ "error": { "code": "workspace.uploadSizeMismatch", "message": "Upload ended before its declared size was received." } }),
        );
    }
    match upload.commit() {
        Ok(entry) => write_http_json_response(
            &mut stream,
            "201 Created",
            &serde_json::json!({ "entry": entry_value_v2(&entry) }),
        ),
        Err(error) => write_http_json_response(
            &mut stream,
            error.http_status(),
            &serde_json::json!({ "error": error.value() }),
        ),
    }
}

fn parse_content_length(head: &str) -> Result<u64, &'static str> {
    let mut content_length = None;
    for line in head.split("\r\n").skip(1) {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err("Upload request contains a malformed HTTP header.");
        };
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("Chunked upload transfer encoding is not supported.");
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("Upload request contains more than one Content-Length header.");
            }
            content_length = value.trim().parse::<u64>().ok();
            if content_length.is_none() {
                return Err("Upload Content-Length must be a non-negative integer.");
            }
        }
    }
    content_length.ok_or("Upload request requires Content-Length.")
}

fn read_http_head(stream: &mut TcpStream) -> io::Result<HttpHead> {
    let mut bytes = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP request ended before its headers",
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let head_end = end + 4;
            if head_end > MAX_HTTP_HEAD_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP headers exceed the configured limit",
                ));
            }
            let text = String::from_utf8(bytes[..head_end].to_vec()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "HTTP headers are not UTF-8")
            })?;
            return Ok(HttpHead {
                text,
                body_prefix: bytes[head_end..].to_vec(),
            });
        }
        if bytes.len() >= MAX_HTTP_HEAD_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP headers exceed the configured limit",
            ));
        }
    }
}

fn write_upload_preflight(stream: &mut TcpStream) -> Result<(), ServerError> {
    write!(
        stream,
        "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: PUT, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nAccess-Control-Max-Age: 600\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    )
    .and_then(|()| stream.flush())
    .map_err(ServerError::WebSocketIo)
}

fn write_http_json_response(
    stream: &mut TcpStream,
    status: &str,
    value: &serde_json::Value,
) -> Result<(), ServerError> {
    let body = value.to_string().into_bytes();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len(),
    )
    .and_then(|()| stream.write_all(&body))
    .and_then(|()| stream.flush())
    .map_err(ServerError::WebSocketIo)
}

fn write_http_empty_response(stream: &mut TcpStream, status: &str) -> Result<(), ServerError> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    )
    .and_then(|()| stream.flush())
    .map_err(ServerError::WebSocketIo)
}

fn content_disposition(name: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let fallback = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ' ' | '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let fallback = if fallback.is_empty() {
        "download".to_owned()
    } else {
        fallback
    };
    let mut encoded = String::with_capacity(name.len() * 3);
    for byte in name.as_bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                *byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
        {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Endpoint {
    Kernel,
    Lsp,
    Graphics(openmat_plot_protocol::GraphicsProtocol),
    Workspace(WorkspaceProtocol),
}

struct WorkspaceChangeMonitor {
    watcher: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    watched_target: Option<(u64, PathBuf)>,
    watch_active: bool,
    pending_since: Option<Instant>,
}

impl WorkspaceChangeMonitor {
    fn new(service: &WorkspaceService) -> notify::Result<Self> {
        let (sender, events) = mpsc::channel();
        let watcher = notify::recommended_watcher(sender)?;
        let mut monitor = Self {
            watcher,
            events,
            watched_target: None,
            watch_active: false,
            pending_since: None,
        };
        monitor.synchronize_target(service);
        Ok(monitor)
    }

    fn synchronize_target(&mut self, service: &WorkspaceService) {
        let Ok(target) = service.current_directory_watch_target() else {
            return;
        };
        if self.watched_target.as_ref() == Some(&target) {
            return;
        }
        if self.watch_active
            && let Some((_, previous_path)) = self.watched_target.as_ref()
        {
            let _ = self.watcher.unwatch(previous_path);
        }
        while self.events.try_recv().is_ok() {}
        self.pending_since = None;
        self.watch_active = self
            .watcher
            .watch(&target.1, RecursiveMode::Recursive)
            .is_ok();
        self.watched_target = Some(target);
    }

    fn poll(&mut self, service: &WorkspaceService, protocol: WorkspaceProtocol) -> Option<String> {
        self.synchronize_target(service);
        let now = Instant::now();
        while let Ok(event) = self.events.try_recv() {
            if event.is_err()
                || event.is_ok_and(|event| !matches!(event.kind, EventKind::Access(_)))
            {
                self.pending_since = Some(now);
            }
        }
        let pending_since = self.pending_since?;
        if now.duration_since(pending_since) < WORKSPACE_CHANGE_DEBOUNCE {
            return None;
        }
        self.pending_since = None;
        if !self.watch_active {
            return None;
        }
        self.watched_target
            .as_ref()
            .map(|(generation, _)| crate::workspace::workspace_changed_event(*generation, protocol))
    }
}

#[allow(clippy::too_many_lines)]
fn run_workspace_connection(
    socket: &mut WebSocket<TcpStream>,
    service: &WorkspaceService,
    protocol: WorkspaceProtocol,
) -> Result<(), ServerError> {
    let mut request_ids = HashSet::new();
    let modern_protocol = protocol != WorkspaceProtocol::V1;
    let mut change_monitor = modern_protocol
        .then(|| WorkspaceChangeMonitor::new(service).ok())
        .flatten();
    let mut observed_directory_generation = service
        .current_directory_event(protocol)
        .ok()
        .map(|(generation, _)| generation);
    let mut observed_search_path_generation = service
        .search_path_event(protocol)
        .ok()
        .map(|(generation, _)| generation);
    loop {
        if modern_protocol
            && let Ok((generation, event)) = service.current_directory_event(protocol)
            && observed_directory_generation != Some(generation)
        {
            write_frames(socket, [event])?;
            observed_directory_generation = Some(generation);
        }
        if modern_protocol
            && let Ok((generation, event)) = service.search_path_event(protocol)
            && observed_search_path_generation != Some(generation)
        {
            write_frames(socket, [event])?;
            observed_search_path_generation = Some(generation);
        }
        if let Some(event) = change_monitor
            .as_mut()
            .and_then(|monitor| monitor.poll(service, protocol))
        {
            write_frames(socket, [event])?;
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if text.len() > MAX_MESSAGE_BYTES {
                    close(socket, CloseCode::Size, "message exceeds size limit");
                    return Ok(());
                }
                let request_id = serde_json::from_str::<serde_json::Value>(text.as_str())
                    .ok()
                    .and_then(|value| {
                        value
                            .get("requestId")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    });
                if request_id
                    .as_ref()
                    .is_some_and(|request_id| !request_ids.insert(request_id.clone()))
                {
                    let response = serde_json::json!({
                        "protocol": match protocol {
                            WorkspaceProtocol::V1 => crate::workspace::PROTOCOL_V1,
                            WorkspaceProtocol::V2 => crate::workspace::PROTOCOL_V2,
                            WorkspaceProtocol::V3 => crate::workspace::PROTOCOL_V3,
                        },
                        "requestId": request_id.unwrap_or_default(),
                        "ok": false,
                        "error": {
                            "code": "workspace.duplicateRequestId",
                            "message": "Workspace requestId was already used on this connection.",
                            "field": "requestId"
                        }
                    })
                    .to_string();
                    write_frames(socket, [response])?;
                } else {
                    write_frames(
                        socket,
                        [service.handle_text_for_protocol(text.as_str(), protocol)],
                    )?;
                }
            }
            Ok(Message::Binary(_)) => {
                close(
                    socket,
                    CloseCode::Unsupported,
                    "binary frames are not supported",
                );
                return Ok(());
            }
            Ok(Message::Close(_)) => {
                let _ = socket.flush();
                return Ok(());
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => socket.flush()?,
            Ok(Message::Frame(_)) => {
                close(socket, CloseCode::Protocol, "unexpected raw frame");
                return Ok(());
            }
            Err(WebSocketError::Io(error)) if is_poll_timeout(&error) => {
                thread::sleep(SOCKET_POLL_INTERVAL);
            }
            Err(WebSocketError::Capacity(_)) => {
                close(socket, CloseCode::Size, "message exceeds size limit");
                return Ok(());
            }
            Err(WebSocketError::Utf8(_)) => {
                close(socket, CloseCode::Invalid, "text frame is not UTF-8");
                return Ok(());
            }
            Err(WebSocketError::Protocol(_)) => {
                close(socket, CloseCode::Protocol, "WebSocket protocol violation");
                return Ok(());
            }
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => return Ok(()),
            Err(error) => return Err(ServerError::WebSocket(error)),
        }
    }
}

fn run_connection<F, E>(
    socket: &mut WebSocket<TcpStream>,
    factory: &mut Option<F>,
    session: &mut Option<ActiveSession>,
    graphics: &GraphicsSessionRegistry,
) -> Result<(), ServerError>
where
    F: FnOnce() -> Result<E, EngineError>,
    E: ExecutionEngine + Send + 'static,
{
    loop {
        if let Some(active_session) = session.as_mut()
            && drain_worker_events(socket, active_session)?
        {
            close(socket, CloseCode::Normal, "kernel shut down");
            return Ok(());
        }
        if session
            .as_ref()
            .is_some_and(|active_session| active_session.shutdown_queued)
        {
            thread::sleep(SOCKET_POLL_INTERVAL);
            continue;
        }

        match socket.read() {
            Ok(message) => {
                if handle_message(socket, message, factory, session, graphics)? {
                    return Ok(());
                }
            }
            Err(WebSocketError::Io(error)) if is_poll_timeout(&error) => {
                thread::sleep(SOCKET_POLL_INTERVAL);
            }
            Err(WebSocketError::Capacity(_)) => {
                close(socket, CloseCode::Size, "message exceeds size limit");
                return Ok(());
            }
            Err(WebSocketError::Utf8(_)) => {
                close(socket, CloseCode::Invalid, "text frame is not UTF-8");
                return Ok(());
            }
            Err(WebSocketError::Protocol(_)) => {
                close(socket, CloseCode::Protocol, "WebSocket protocol violation");
                return Ok(());
            }
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => return Ok(()),
            Err(error) => return Err(ServerError::WebSocket(error)),
        }
    }
}

fn handle_message<F, E>(
    socket: &mut WebSocket<TcpStream>,
    message: Message,
    factory: &mut Option<F>,
    session: &mut Option<ActiveSession>,
    graphics: &GraphicsSessionRegistry,
) -> Result<bool, ServerError>
where
    F: FnOnce() -> Result<E, EngineError>,
    E: ExecutionEngine + Send + 'static,
{
    match message {
        Message::Text(text) => handle_text(socket, text.as_str(), factory, session, graphics),
        Message::Binary(_) => {
            close(
                socket,
                CloseCode::Unsupported,
                "binary frames are not supported",
            );
            Ok(true)
        }
        Message::Close(_) => {
            let _ = socket.flush();
            Ok(true)
        }
        Message::Ping(_) | Message::Pong(_) => {
            socket.flush()?;
            Ok(false)
        }
        Message::Frame(_) => {
            close(socket, CloseCode::Protocol, "unexpected raw frame");
            Ok(true)
        }
    }
}

fn handle_text<F, E>(
    socket: &mut WebSocket<TcpStream>,
    text: &str,
    factory: &mut Option<F>,
    session: &mut Option<ActiveSession>,
    graphics: &GraphicsSessionRegistry,
) -> Result<bool, ServerError>
where
    F: FnOnce() -> Result<E, EngineError>,
    E: ExecutionEngine + Send + 'static,
{
    if text.len() > MAX_MESSAGE_BYTES {
        close(socket, CloseCode::Size, "message exceeds size limit");
        return Ok(true);
    }

    if session
        .as_ref()
        .and_then(|active_session| active_session.protocol)
        .is_none()
    {
        return handle_bootstrap_text(socket, text, factory, session, graphics);
    }
    handle_negotiated_text(socket, text, session)
}

fn handle_bootstrap_text<F, E>(
    socket: &mut WebSocket<TcpStream>,
    text: &str,
    factory: &mut Option<F>,
    session: &mut Option<ActiveSession>,
    graphics: &GraphicsSessionRegistry,
) -> Result<bool, ServerError>
where
    F: FnOnce() -> Result<E, EngineError>,
    E: ExecutionEngine + Send + 'static,
{
    let request = match kernel_v2::decode_bootstrap_request(text) {
        Ok(request) => request,
        Err(kernel_v2::CodecError::Validation(error))
            if error.category() == "protocol.invalidCapabilities" =>
        {
            if let Ok(request) = serde_json::from_str(text) {
                request
            } else {
                close(socket, CloseCode::Invalid, "invalid bootstrap request JSON");
                return Ok(true);
            }
        }
        Err(kernel_v2::CodecError::Json(_)) => {
            close(socket, CloseCode::Invalid, "invalid bootstrap request JSON");
            return Ok(true);
        }
        Err(kernel_v2::CodecError::Validation(_)) => {
            close(socket, CloseCode::Policy, "invalid bootstrap initialize");
            return Ok(true);
        }
        Err(
            kernel_v2::CodecError::FrameTooLarge { .. }
            | kernel_v2::CodecError::Utf8(_)
            | kernel_v2::CodecError::Utf8Bom,
        ) => {
            close(socket, CloseCode::Size, "message exceeds size limit");
            return Ok(true);
        }
    };
    if let Some(active_session) = session.as_ref()
        && request.session_id != active_session.session_id
    {
        close(socket, CloseCode::Policy, "session mismatch");
        return Ok(true);
    }
    if session.is_none() {
        let create_engine = factory.take().ok_or(ServerError::State(
            "WebSocket engine factory was consumed before session initialization",
        ))?;
        let engine = match create_engine() {
            Ok(engine) => engine,
            Err(error) => {
                let response = kernel_v2::BootstrapResponseEnvelope::failure(
                    &request,
                    "transport-bootstrap-error",
                    ProtocolError::new(error.category(), error.message()),
                );
                let frame = kernel_v2::encode_bootstrap_response(&response)
                    .map_err(ServerError::ProtocolV2Codec)?;
                write_frames(socket, [frame])?;
                close(socket, CloseCode::Error, "kernel initialization failed");
                return Ok(true);
            }
        };
        let active_session = ActiveSession::start(&request.session_id, engine, graphics)?;
        let startup = serde_json::to_string(&starting_message(&request.session_id))
            .map_err(ServerError::Encode)?;
        write_frames(socket, [startup])?;
        *session = Some(active_session);
    }
    let active_session = session
        .as_mut()
        .ok_or(ServerError::State("WebSocket session was not initialized"))?;
    match active_session
        .commands
        .try_send(WorkerCommand::Bootstrap(Box::new(request)))
    {
        Ok(()) => Ok(false),
        Err(TrySendError::Full(_)) => {
            close(socket, CloseCode::Policy, "too many pending requests");
            Ok(true)
        }
        Err(TrySendError::Disconnected(_)) => {
            close(socket, CloseCode::Error, "kernel worker stopped");
            Ok(true)
        }
    }
}

#[allow(clippy::too_many_lines)] // Keep all locked-version decode and close-policy branches adjacent.
fn handle_negotiated_text(
    socket: &mut WebSocket<TcpStream>,
    text: &str,
    session: &mut Option<ActiveSession>,
) -> Result<bool, ServerError> {
    let active_session = session
        .as_mut()
        .ok_or(ServerError::State("WebSocket session was not initialized"))?;
    let protocol = active_session
        .protocol
        .ok_or(ServerError::State("WebSocket protocol was not negotiated"))?;
    let (command, session_id, interrupt, shutdown) = match protocol {
        ProtocolVersion::V0 => {
            let Ok(request) = serde_json::from_str::<RequestEnvelope>(text) else {
                close(socket, CloseCode::Invalid, "invalid v0 request JSON");
                return Ok(true);
            };
            if request.protocol != openmat_protocol::PROTOCOL_V0 {
                close(socket, CloseCode::Policy, "negotiated protocol mismatch");
                return Ok(true);
            }
            let session_id = request.session_id.clone();
            let interrupt = matches!(request.request, Request::Interrupt(_));
            let shutdown = matches!(request.request, Request::Shutdown(_));
            (
                WorkerCommand::V0(Box::new(request)),
                session_id,
                interrupt,
                shutdown,
            )
        }
        ProtocolVersion::V1 => {
            let Ok(request) = serde_json::from_str::<kernel_v1::RequestEnvelope>(text) else {
                close(socket, CloseCode::Invalid, "invalid v1 request JSON");
                return Ok(true);
            };
            if request.protocol != kernel_v1::PROTOCOL_V1 {
                close(socket, CloseCode::Policy, "negotiated protocol mismatch");
                return Ok(true);
            }
            let session_id = request.session_id.clone();
            let interrupt = matches!(request.request, kernel_v1::Request::Interrupt(_));
            let shutdown = matches!(request.request, kernel_v1::Request::Shutdown(_));
            (
                WorkerCommand::V1 {
                    request: Box::new(request),
                    frame: text.to_owned(),
                },
                session_id,
                interrupt,
                shutdown,
            )
        }
        ProtocolVersion::V2 => {
            let Ok(request) = serde_json::from_str::<kernel_v2::RequestEnvelope>(text) else {
                close(socket, CloseCode::Invalid, "invalid v2 request JSON");
                return Ok(true);
            };
            if request.protocol != kernel_v2::PROTOCOL_V2 {
                close(socket, CloseCode::Policy, "negotiated protocol mismatch");
                return Ok(true);
            }
            let session_id = request.session_id.clone();
            let interrupt = matches!(request.request, kernel_v2::Request::Interrupt(_));
            let shutdown = matches!(request.request, kernel_v2::Request::Shutdown(_));
            (
                WorkerCommand::V2 {
                    request: Box::new(request),
                    frame: text.to_owned(),
                },
                session_id,
                interrupt,
                shutdown,
            )
        }
        ProtocolVersion::V3 => {
            let Ok(request) = serde_json::from_str::<kernel_v3::RequestEnvelope>(text) else {
                close(socket, CloseCode::Invalid, "invalid v3 request JSON");
                return Ok(true);
            };
            if request.protocol != kernel_v3::PROTOCOL_V3 {
                close(socket, CloseCode::Policy, "negotiated protocol mismatch");
                return Ok(true);
            }
            let session_id = request.session_id.clone();
            let interrupt = matches!(request.request, kernel_v3::Request::Interrupt(_));
            let shutdown = matches!(request.request, kernel_v3::Request::Shutdown(_));
            (
                WorkerCommand::V3 {
                    request: Box::new(request),
                    frame: text.to_owned(),
                },
                session_id,
                interrupt,
                shutdown,
            )
        }
    };
    if session_id != active_session.session_id {
        close(socket, CloseCode::Policy, "session mismatch");
        return Ok(true);
    }
    if interrupt {
        let control = active_session.control.as_ref().ok_or(ServerError::State(
            "negotiated WebSocket session is missing its control path",
        ))?;
        let frames = match &command {
            WorkerCommand::V0(request) => encode_v0_messages(
                control
                    .handle(KernelSessionRequest::V0(request))
                    .map_err(ServerError::KernelSession)?,
            )?,
            WorkerCommand::V1 { frame, .. } => control
                .handle_v1_frame(frame)
                .map_err(ServerError::KernelSession)?,
            WorkerCommand::V2 { frame, .. } => control
                .handle_v2_frame(frame)
                .map_err(ServerError::KernelSession)?,
            WorkerCommand::V3 { frame, .. } => control
                .handle_v3_frame(frame)
                .map_err(ServerError::KernelSession)?,
            WorkerCommand::Bootstrap(_) | WorkerCommand::Stop => {
                return Err(ServerError::State("invalid interrupt command"));
            }
        };
        write_frames(socket, frames)?;
        return Ok(false);
    }

    match active_session.commands.try_send(command) {
        Ok(()) => active_session.shutdown_queued = shutdown,
        Err(TrySendError::Full(_)) => {
            close(socket, CloseCode::Policy, "too many pending requests");
            return Ok(true);
        }
        Err(TrySendError::Disconnected(_)) => {
            close(socket, CloseCode::Error, "kernel worker stopped");
            return Ok(true);
        }
    }
    Ok(false)
}

#[allow(clippy::result_large_err)] // Required by tungstenite's handshake callback ABI.
fn validate_handshake(
    request: &HandshakeRequest,
    response: tungstenite::handshake::server::Response,
    endpoint: &Cell<Option<Endpoint>>,
    workspace_available: bool,
) -> Result<tungstenite::handshake::server::Response, ErrorResponse> {
    let selected = match (request.uri().path(), request.uri().query()) {
        ("/kernel", None) => Endpoint::Kernel,
        ("/lsp", None) => Endpoint::Lsp,
        ("/graphics/v1", None) => Endpoint::Graphics(openmat_plot_protocol::GraphicsProtocol::V1),
        ("/graphics/v2", None) => Endpoint::Graphics(openmat_plot_protocol::GraphicsProtocol::V2),
        ("/graphics/v3", None) => Endpoint::Graphics(openmat_plot_protocol::GraphicsProtocol::V3),
        ("/graphics/v4", None) => Endpoint::Graphics(openmat_plot_protocol::GraphicsProtocol::V4),
        ("/workspace/v1", None) if workspace_available => {
            Endpoint::Workspace(WorkspaceProtocol::V1)
        }
        ("/workspace/v2", None) if workspace_available => {
            Endpoint::Workspace(WorkspaceProtocol::V2)
        }
        ("/workspace/v3", None) if workspace_available => {
            Endpoint::Workspace(WorkspaceProtocol::V3)
        }
        _ => {
            return Err(handshake_rejection(
                StatusCode::NOT_FOUND,
                "WebSocket endpoints are /kernel, /lsp, /graphics/v1, /graphics/v2, /graphics/v3, /graphics/v4, and configured /workspace/v1, /workspace/v2, or /workspace/v3",
            ));
        }
    };
    if request.uri().query().is_some() {
        return Err(handshake_rejection(
            StatusCode::NOT_FOUND,
            "WebSocket endpoints do not accept query strings",
        ));
    }
    let origins = request.headers().get_all("origin");
    let mut origins = origins.iter();
    if let Some(origin) = origins.next() {
        let allowed = origins.next().is_none()
            && origin
                .to_str()
                .ok()
                .and_then(|origin| origin.parse::<Uri>().ok())
                .and_then(|origin| origin.host().map(str::to_owned))
                .is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || host.eq_ignore_ascii_case("tauri.localhost")
                        || host == "127.0.0.1"
                });
        if !allowed {
            return Err(handshake_rejection(
                StatusCode::FORBIDDEN,
                "Origin is not allowed",
            ));
        }
    }
    endpoint.set(Some(selected));
    Ok(response)
}

fn handshake_rejection(status: StatusCode, message: &str) -> ErrorResponse {
    tungstenite::http::Response::builder()
        .status(status)
        .body(Some(message.to_owned()))
        .expect("static WebSocket rejection response")
}

fn handshake_error<R: tungstenite::handshake::HandshakeRole>(
    error: HandshakeError<R>,
) -> ServerError {
    match error {
        HandshakeError::Failure(error) => ServerError::WebSocket(error),
        HandshakeError::Interrupted(_) => {
            ServerError::State("blocking WebSocket handshake was unexpectedly interrupted")
        }
    }
}

fn is_poll_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn close(socket: &mut WebSocket<TcpStream>, code: CloseCode, reason: &'static str) {
    let _ = socket.close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }));
    let _ = socket.flush();
}

fn write_frames(
    socket: &mut WebSocket<TcpStream>,
    frames: impl IntoIterator<Item = String>,
) -> Result<(), ServerError> {
    for frame in frames {
        socket.send(Message::text(frame))?;
    }
    Ok(())
}

struct ActiveSession {
    session_id: String,
    commands: SyncSender<WorkerCommand>,
    events: Receiver<WorkerEvent>,
    control: Option<KernelSessionControl>,
    protocol: Option<ProtocolVersion>,
    worker: Option<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
    next_transport_id: u64,
    shutdown_queued: bool,
    graphics_registry: GraphicsSessionRegistry,
    graphics_registration: Option<GraphicsRegistration>,
}

impl ActiveSession {
    fn start<E: ExecutionEngine + Send + 'static>(
        session_id: &str,
        engine: E,
        graphics_registry: &GraphicsSessionRegistry,
    ) -> Result<Self, ServerError> {
        let (commands, command_receiver) = mpsc::sync_channel(MAX_PENDING_REQUESTS);
        let (event_sender, events) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let graphics_hub = engine
            .graphics_session()
            .map(|session| Arc::new(RuntimeGraphicsHub::new(session)));
        let graphics_registration = graphics_hub
            .as_ref()
            .map(|hub| {
                let hub: Arc<dyn crate::graphics::GraphicsSessionHub> = Arc::clone(hub) as Arc<_>;
                graphics_registry.register_session(session_id, hub)
            })
            .transpose()
            .map_err(ServerError::GraphicsRegistry)?;
        let notifying_engine = NotifyingEngine {
            inner: engine,
            events: event_sender.clone(),
            graphics_hub,
            graphics_registration: graphics_registration.clone(),
        };
        let session = KernelSession::new(session_id, notifying_engine);
        let worker_session_id = session_id.to_owned();
        let worker_stopping = Arc::clone(&stopping);
        let worker = thread::Builder::new()
            .name("openmat-kernel-session".to_owned())
            .spawn(move || {
                worker_loop(
                    session,
                    &worker_session_id,
                    &command_receiver,
                    &event_sender,
                    &worker_stopping,
                );
            })
            .map_err(ServerError::WebSocketIo)?;
        Ok(Self {
            session_id: session_id.to_owned(),
            commands,
            events,
            control: None,
            protocol: None,
            worker: Some(worker),
            stopping,
            next_transport_id: 1,
            shutdown_queued: false,
            graphics_registry: graphics_registry.clone(),
            graphics_registration,
        })
    }

    fn busy_frame(&mut self) -> Result<String, ServerError> {
        let message_id = format!("transport-{}", self.next_transport_id);
        self.next_transport_id += 1;
        match self.protocol.ok_or(ServerError::State(
            "execution started before protocol negotiation",
        ))? {
            ProtocolVersion::V0 => {
                serde_json::to_string(&ServerMessage::Event(EventEnvelope::new(
                    &self.session_id,
                    message_id,
                    Event::Status(StatusEvent {
                        status: KernelStatus::Busy,
                    }),
                )))
                .map_err(ServerError::Encode)
            }
            ProtocolVersion::V1 => kernel_v1::encode_event(&kernel_v1::EventEnvelope::new(
                &self.session_id,
                message_id,
                Event::Status(StatusEvent {
                    status: KernelStatus::Busy,
                }),
            ))
            .map_err(ServerError::ProtocolCodec),
            ProtocolVersion::V2 => kernel_v2::encode_event(&kernel_v2::EventEnvelope::new(
                &self.session_id,
                message_id,
                Event::Status(StatusEvent {
                    status: KernelStatus::Busy,
                }),
            ))
            .map_err(ServerError::ProtocolV2Codec),
            ProtocolVersion::V3 => kernel_v3::encode_event(&kernel_v3::EventEnvelope::new(
                &self.session_id,
                message_id,
                Event::Status(StatusEvent {
                    status: KernelStatus::Busy,
                }),
            ))
            .map_err(ServerError::ProtocolV2Codec),
        }
    }

    fn stop(&mut self) {
        if self
            .control
            .as_ref()
            .is_none_or(|control| control.status() != KernelStatus::Dead)
        {
            self.stopping.store(true, Ordering::Release);
            if let (Some(control), Some(protocol)) = (&self.control, self.protocol) {
                match protocol {
                    ProtocolVersion::V0 => {
                        let interrupt = RequestEnvelope::new(
                            &self.session_id,
                            "transport-disconnect-interrupt",
                            Request::Interrupt(openmat_protocol::InterruptRequest {}),
                        );
                        let _ = control.handle(KernelSessionRequest::V0(&interrupt));
                    }
                    ProtocolVersion::V1 => {
                        let interrupt = kernel_v1::RequestEnvelope::new(
                            &self.session_id,
                            "transport-disconnect-interrupt",
                            kernel_v1::Request::Interrupt(openmat_protocol::InterruptRequest {}),
                        );
                        let _ = control.handle(KernelSessionRequest::V1(&interrupt));
                    }
                    ProtocolVersion::V2 => {
                        let interrupt = kernel_v2::RequestEnvelope::new(
                            &self.session_id,
                            "transport-disconnect-interrupt",
                            kernel_v2::Request::Interrupt(openmat_protocol::InterruptRequest {}),
                        );
                        let _ = control.handle(KernelSessionRequest::V2(&interrupt));
                    }
                    ProtocolVersion::V3 => {
                        let interrupt = kernel_v3::RequestEnvelope::new(
                            &self.session_id,
                            "transport-disconnect-interrupt",
                            kernel_v3::Request::Interrupt(openmat_protocol::InterruptRequest {}),
                        );
                        let _ = control.handle(KernelSessionRequest::V3(&interrupt));
                    }
                }
            }
            let _ = self.commands.try_send(WorkerCommand::Stop);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(registration) = self.graphics_registration.take() {
            let _ = self.graphics_registry.unregister_session(&registration);
        }
    }
}

enum WorkerCommand {
    Bootstrap(Box<kernel_v2::BootstrapRequestEnvelope>),
    V0(Box<RequestEnvelope>),
    V1 {
        request: Box<kernel_v1::RequestEnvelope>,
        frame: String,
    },
    V2 {
        request: Box<kernel_v2::RequestEnvelope>,
        frame: String,
    },
    V3 {
        request: Box<kernel_v3::RequestEnvelope>,
        frame: String,
    },
    Stop,
}

enum WorkerEvent {
    ExecutionStarted,
    Bootstrap {
        frames: Vec<String>,
        protocol: Option<ProtocolVersion>,
        control: Option<KernelSessionControl>,
    },
    Messages {
        frames: Vec<String>,
        execution: bool,
        dead: bool,
    },
    Failure(String),
}

struct NotifyingEngine<E> {
    inner: E,
    events: mpsc::Sender<WorkerEvent>,
    graphics_hub: Option<Arc<RuntimeGraphicsHub>>,
    graphics_registration: Option<GraphicsRegistration>,
}

impl<E: ExecutionEngine> ExecutionEngine for NotifyingEngine<E> {
    fn implementation(&self) -> ImplementationInfo {
        self.inner.implementation()
    }

    fn capabilities(&self) -> Capabilities {
        let mut capabilities = self.inner.capabilities();
        self.advertise_graphics_mime(&mut capabilities.display_mime_types);
        capabilities
    }

    fn v1_capabilities(&self) -> kernel_v1::Capabilities {
        let mut capabilities = self.inner.v1_capabilities();
        self.advertise_graphics_mime(&mut capabilities.display_mime_types);
        capabilities
    }

    fn v2_capabilities(&self) -> kernel_v2::Capabilities {
        let mut capabilities = self.inner.v2_capabilities();
        self.advertise_graphics_mime(&mut capabilities.display_mime_types);
        capabilities
    }

    fn graphics_session(&self) -> Option<Arc<std::sync::Mutex<openmat_kernel::GraphicsSession>>> {
        self.inner.graphics_session()
    }

    fn take_graphics_notices(&mut self) -> Vec<openmat_kernel::GraphicsNotice> {
        self.inner.take_graphics_notices()
    }

    fn take_graphics_deltas(&mut self) -> Vec<openmat_kernel::GraphicsDelta> {
        self.inner.take_graphics_deltas()
    }

    fn execute(
        &mut self,
        request: &ExecuteRequest,
        cancellation: &CancellationToken,
    ) -> Result<ExecutionOutput, EngineError> {
        let _ = self.events.send(WorkerEvent::ExecutionStarted);
        let started = Instant::now();
        let result = self.inner.execute(request, cancellation);
        let engine_finished = Instant::now();
        let notices = self.inner.take_graphics_notices();
        let deltas = self.inner.take_graphics_deltas();
        let display_events = self.publish_graphics(notices, deltas)?;
        if performance_logging_enabled() {
            let finished = Instant::now();
            eprintln!(
                "OPENMAT_PERF server engine_ms={:.3} graphics_publish_ms={:.3} total_ms={:.3} graphics_events={}",
                duration_milliseconds(engine_finished.duration_since(started)),
                duration_milliseconds(finished.duration_since(engine_finished)),
                duration_milliseconds(finished.duration_since(started)),
                display_events.len(),
            );
        }
        match result {
            Ok(mut output) => {
                output.events.extend(display_events);
                Ok(output)
            }
            Err(error) => Err(error.extend_events(display_events)),
        }
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

impl<E: ExecutionEngine> NotifyingEngine<E> {
    fn advertise_graphics_mime(&self, mime_types: &mut Vec<String>) {
        if self.graphics_registration.is_some()
            && !mime_types
                .iter()
                .any(|mime| mime == crate::graphics::discovery_mime_type())
        {
            mime_types.push(crate::graphics::discovery_mime_type().to_owned());
        }
    }

    fn publish_graphics(
        &self,
        notices: Vec<openmat_kernel::GraphicsNotice>,
        deltas: Vec<openmat_kernel::GraphicsDelta>,
    ) -> Result<Vec<Event>, EngineError> {
        if notices.is_empty() && deltas.is_empty() {
            return Ok(Vec::new());
        }
        let hub = self.graphics_hub.as_ref().ok_or_else(|| {
            EngineError::new(
                "graphics.hostUnavailable",
                "graphics transactions committed without a registered host adapter",
            )
        })?;
        hub.publish(deltas).map_err(|error| {
            EngineError::new(
                "graphics.adapter",
                format!("graphics adapter failed: {error}"),
            )
        })?;
        let registration = self.graphics_registration.as_ref().ok_or_else(|| {
            EngineError::new(
                "graphics.hostUnavailable",
                "graphics session is missing its attachment registration",
            )
        })?;
        notices
            .into_iter()
            .filter(|notice| notice.discovery)
            .map(|notice| {
                let discovery = registration
                    .discovery(notice.figure_id, notice.revision)
                    .map_err(|error| {
                        EngineError::new(
                            "graphics.discovery",
                            format!("graphics discovery failed: {error}"),
                        )
                    })?;
                let encoded = crate::graphics::encode_discovery(&discovery).map_err(|error| {
                    EngineError::new(
                        "graphics.discovery",
                        format!("graphics discovery encoding failed: {error}"),
                    )
                })?;
                let mut representations = BTreeMap::new();
                representations.insert(crate::graphics::discovery_mime_type().to_owned(), encoded);
                Ok(Event::Display(DisplayEvent { representations }))
            })
            .collect()
    }
}

#[allow(clippy::too_many_lines)] // Keep every negotiated protocol worker branch visibly symmetric.
fn worker_loop<E: ExecutionEngine>(
    mut session: KernelSession<E>,
    session_id: &str,
    commands: &Receiver<WorkerCommand>,
    events: &mpsc::Sender<WorkerEvent>,
    stopping: &AtomicBool,
) {
    while let Ok(command) = commands.recv() {
        if stopping.load(Ordering::Acquire) && !matches!(&command, WorkerCommand::Stop) {
            orderly_stop(&mut session, session_id);
            break;
        }
        match command {
            WorkerCommand::Bootstrap(request) => {
                let result = session
                    .handle(KernelSessionRequest::Bootstrap(request.as_ref()))
                    .map_err(ServerError::KernelSession)
                    .and_then(encode_bootstrap_messages);
                let frames = match result {
                    Ok(frames) => frames,
                    Err(error) => {
                        let _ = events.send(WorkerEvent::Failure(error.to_string()));
                        break;
                    }
                };
                let protocol = session.negotiated_protocol();
                let control = protocol.and_then(|_| session.control().ok());
                if events
                    .send(WorkerEvent::Bootstrap {
                        frames,
                        protocol,
                        control,
                    })
                    .is_err()
                {
                    orderly_stop(&mut session, session_id);
                    break;
                }
            }
            WorkerCommand::V0(request) => {
                let execution = matches!(request.request, Request::Execute(_));
                let result = session
                    .handle(KernelSessionRequest::V0(request.as_ref()))
                    .map_err(ServerError::KernelSession)
                    .and_then(encode_v0_messages);
                if !finish_worker_request(
                    result,
                    execution,
                    &mut session,
                    session_id,
                    events,
                    stopping,
                ) {
                    break;
                }
            }
            WorkerCommand::V1 { request, frame } => {
                let execution = matches!(request.request, kernel_v1::Request::Execute(_));
                let result = session
                    .handle_v1_frame(&frame)
                    .map_err(ServerError::KernelSession);
                if !finish_worker_request(
                    result,
                    execution,
                    &mut session,
                    session_id,
                    events,
                    stopping,
                ) {
                    break;
                }
            }
            WorkerCommand::V2 { request, frame } => {
                let execution = matches!(request.request, kernel_v2::Request::Execute(_));
                let result = session
                    .handle_v2_frame(&frame)
                    .map_err(ServerError::KernelSession);
                if !finish_worker_request(
                    result,
                    execution,
                    &mut session,
                    session_id,
                    events,
                    stopping,
                ) {
                    break;
                }
            }
            WorkerCommand::V3 { request, frame } => {
                let execution = matches!(request.request, kernel_v3::Request::Execute(_));
                let result = session
                    .handle_v3_frame(&frame)
                    .map_err(ServerError::KernelSession);
                if !finish_worker_request(
                    result,
                    execution,
                    &mut session,
                    session_id,
                    events,
                    stopping,
                ) {
                    break;
                }
            }
            WorkerCommand::Stop => {
                orderly_stop(&mut session, session_id);
                break;
            }
        }
    }
}

fn finish_worker_request<E: ExecutionEngine>(
    result: Result<Vec<String>, ServerError>,
    execution: bool,
    session: &mut KernelSession<E>,
    session_id: &str,
    events: &mpsc::Sender<WorkerEvent>,
    stopping: &AtomicBool,
) -> bool {
    let frames = match result {
        Ok(frames) => frames,
        Err(error) => {
            let _ = events.send(WorkerEvent::Failure(error.to_string()));
            orderly_stop(session, session_id);
            return false;
        }
    };
    let dead = session.status() == KernelStatus::Dead;
    if events
        .send(WorkerEvent::Messages {
            frames,
            execution,
            dead,
        })
        .is_err()
    {
        orderly_stop(session, session_id);
        return false;
    }
    if dead {
        return false;
    }
    if stopping.load(Ordering::Acquire) {
        orderly_stop(session, session_id);
        return false;
    }
    true
}

fn orderly_stop<E: ExecutionEngine>(session: &mut KernelSession<E>, session_id: &str) {
    if session.status() == KernelStatus::Dead {
        return;
    }
    match session.negotiated_protocol() {
        Some(ProtocolVersion::V0) => {
            let shutdown = RequestEnvelope::new(
                session_id,
                "transport-disconnect-shutdown",
                Request::Shutdown(ShutdownRequest {}),
            );
            let _ = session.handle(KernelSessionRequest::V0(&shutdown));
        }
        Some(ProtocolVersion::V1) => {
            let shutdown = kernel_v1::RequestEnvelope::new(
                session_id,
                "transport-disconnect-shutdown",
                kernel_v1::Request::Shutdown(ShutdownRequest {}),
            );
            let _ = session.handle(KernelSessionRequest::V1(&shutdown));
        }
        Some(ProtocolVersion::V2) => {
            let shutdown = kernel_v2::RequestEnvelope::new(
                session_id,
                "transport-disconnect-shutdown",
                kernel_v2::Request::Shutdown(ShutdownRequest {}),
            );
            let _ = session.handle(KernelSessionRequest::V2(&shutdown));
        }
        Some(ProtocolVersion::V3) => {
            let shutdown = kernel_v3::RequestEnvelope::new(
                session_id,
                "transport-disconnect-shutdown",
                kernel_v3::Request::Shutdown(ShutdownRequest {}),
            );
            let _ = session.handle(KernelSessionRequest::V3(&shutdown));
        }
        None => {}
    }
}

fn drain_worker_events(
    socket: &mut WebSocket<TcpStream>,
    session: &mut ActiveSession,
) -> Result<bool, ServerError> {
    loop {
        match session.events.try_recv() {
            Ok(WorkerEvent::ExecutionStarted) => {
                let frame = session.busy_frame()?;
                write_frames(socket, [frame])?;
            }
            Ok(WorkerEvent::Bootstrap {
                frames,
                protocol,
                control,
            }) => {
                write_frames(socket, frames)?;
                session.protocol = protocol;
                session.control = control;
            }
            Ok(WorkerEvent::Messages {
                frames,
                execution,
                dead,
            }) => {
                let protocol = session.protocol.ok_or(ServerError::State(
                    "worker produced messages before protocol negotiation",
                ))?;
                let frames = frames
                    .into_iter()
                    .filter(|frame| !(execution && is_busy_frame(protocol, frame)));
                write_frames(socket, frames)?;
                if dead {
                    return Ok(true);
                }
            }
            Ok(WorkerEvent::Failure(error)) => {
                close(socket, CloseCode::Error, "kernel worker failed");
                return Err(ServerError::Worker(error));
            }
            Err(TryRecvError::Empty) => return Ok(false),
            Err(TryRecvError::Disconnected) => {
                if session
                    .control
                    .as_ref()
                    .is_some_and(|control| control.status() == KernelStatus::Dead)
                {
                    return Ok(true);
                }
                return Err(ServerError::State("kernel worker stopped unexpectedly"));
            }
        }
    }
}

fn encode_bootstrap_messages(
    messages: Vec<KernelSessionMessage>,
) -> Result<Vec<String>, ServerError> {
    messages
        .into_iter()
        .map(|message| match message {
            KernelSessionMessage::Bootstrap(response) => {
                kernel_v2::encode_bootstrap_response(&response)
                    .map_err(ServerError::ProtocolV2Codec)
            }
            KernelSessionMessage::V0(message) => {
                serde_json::to_string(&message).map_err(ServerError::Encode)
            }
            KernelSessionMessage::V1(kernel_v1::ServerMessage::Event(event)) => {
                kernel_v1::encode_event(&event).map_err(ServerError::ProtocolCodec)
            }
            KernelSessionMessage::V1(kernel_v1::ServerMessage::Response(_)) => Err(
                ServerError::State("bootstrap produced an uncorrelated v1 response"),
            ),
            KernelSessionMessage::V2(kernel_v2::ServerMessage::Event(event)) => {
                kernel_v2::encode_event(&event).map_err(ServerError::ProtocolV2Codec)
            }
            KernelSessionMessage::V2(kernel_v2::ServerMessage::Response(_)) => Err(
                ServerError::State("bootstrap produced an uncorrelated v2 response"),
            ),
            KernelSessionMessage::V3(kernel_v3::ServerMessage::Event(event)) => {
                kernel_v3::encode_event(&event).map_err(ServerError::ProtocolV2Codec)
            }
            KernelSessionMessage::V3(kernel_v3::ServerMessage::Response(_)) => Err(
                ServerError::State("bootstrap produced an uncorrelated v3 response"),
            ),
        })
        .collect()
}

fn encode_v0_messages(messages: Vec<KernelSessionMessage>) -> Result<Vec<String>, ServerError> {
    messages
        .into_iter()
        .map(|message| {
            let KernelSessionMessage::V0(message) = message else {
                return Err(ServerError::State(
                    "v0 request produced a non-v0 session message",
                ));
            };
            serde_json::to_string(&message).map_err(ServerError::Encode)
        })
        .collect()
}

fn is_busy_frame(protocol: ProtocolVersion, frame: &str) -> bool {
    match protocol {
        ProtocolVersion::V0 => serde_json::from_str::<ServerMessage>(frame).is_ok_and(|message| {
            matches!(
                message,
                ServerMessage::Event(EventEnvelope {
                    event: Event::Status(StatusEvent {
                        status: KernelStatus::Busy
                    }),
                    ..
                })
            )
        }),
        ProtocolVersion::V1 => kernel_v1::decode_event(frame).is_ok_and(|event| {
            matches!(
                event.event,
                Event::Status(StatusEvent {
                    status: KernelStatus::Busy
                })
            )
        }),
        ProtocolVersion::V2 => kernel_v2::decode_event(frame).is_ok_and(|event| {
            matches!(
                event.event,
                Event::Status(StatusEvent {
                    status: KernelStatus::Busy
                })
            )
        }),
        ProtocolVersion::V3 => kernel_v3::decode_event(frame).is_ok_and(|event| {
            matches!(
                event.event,
                Event::Status(StatusEvent {
                    status: KernelStatus::Busy
                })
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use openmat_protocol::kernel_v1::{self, PreviewValue};
    use openmat_protocol::kernel_v2;
    use openmat_protocol::{
        ExecuteResult, ExecutionMode, InterruptRequest, ListWorkspaceRequest, PreviewTruncation,
    };
    use tungstenite::protocol::frame::coding::CloseCode;
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket, connect};

    use super::*;

    type ClientSocket = WebSocket<MaybeTlsStream<TcpStream>>;

    struct WorkspaceTestRoot(PathBuf);

    impl WorkspaceTestRoot {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "openmat-workspace-tcp-test-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for WorkspaceTestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct ExactEngine {
        execution_started: Arc<AtomicBool>,
        shutdown: Arc<AtomicBool>,
    }

    impl ExecutionEngine for ExactEngine {
        fn implementation(&self) -> ImplementationInfo {
            ImplementationInfo {
                name: "exact-transport-test".to_owned(),
                version: "1".to_owned(),
            }
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                execution_modes: vec![ExecutionMode::Repl],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: 32,
                interrupt: true,
                workspace_delta: true,
            }
        }

        fn execute(
            &mut self,
            request: &ExecuteRequest,
            cancellation: &CancellationToken,
        ) -> Result<ExecutionOutput, EngineError> {
            self.execution_started.store(true, Ordering::Release);
            while request.code == "wait" && !cancellation.is_cancelled() {
                thread::yield_now();
            }
            Ok(ExecutionOutput {
                result: ExecuteResult {
                    interrupted: cancellation.is_cancelled(),
                },
                events: Vec::new(),
            })
        }

        fn inspect(&mut self, _request: &InspectRequest) -> Result<MatrixPreview, EngineError> {
            Err(EngineError::new(
                "workspace.unsupportedValue",
                "exact values require kernel-v1",
            ))
        }

        fn inspect_v1(
            &mut self,
            request: &kernel_v1::InspectRequest,
            _limits: &kernel_v1::PreviewLimits,
        ) -> Result<kernel_v1::MatrixPreview, EngineError> {
            let (class, dimensions, values) = match request.name.as_str() {
                "wide" => (
                    "uint64",
                    vec![1, 1],
                    vec![PreviewValue::Integer {
                        real: u64::MAX.to_string(),
                        imaginary: "0".to_owned(),
                    }],
                ),
                "isolated" => (
                    "char",
                    vec![1, 1],
                    vec![PreviewValue::CharCodeUnit { value: 0xD83D }],
                ),
                "strings" => (
                    "string",
                    vec![1, 3],
                    vec![
                        PreviewValue::String {
                            code_units: vec![0xD83D],
                            missing: false,
                        },
                        PreviewValue::String {
                            code_units: Vec::new(),
                            missing: false,
                        },
                        PreviewValue::String {
                            code_units: Vec::new(),
                            missing: true,
                        },
                    ],
                ),
                _ => {
                    return Err(EngineError::new(
                        "workspace.unknownVariable",
                        "unknown exact test value",
                    ));
                }
            };
            Ok(kernel_v1::MatrixPreview {
                class: class.to_owned(),
                dimensions,
                complex: false,
                selected_range: request.range.clone(),
                values,
                truncation: PreviewTruncation {
                    truncated: false,
                    omitted_elements: 0,
                },
            })
        }

        fn inspect_v2(
            &mut self,
            request: &kernel_v2::InspectRequest,
            limits: &kernel_v2::AggregateLimits,
        ) -> Result<kernel_v2::InspectPreview, EngineError> {
            if request.name != "aggregate" {
                return self
                    .inspect_v1(request, &limits.preview_limits())
                    .map(kernel_v2::InspectPreview::Matrix);
            }
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
            Ok(vec![
                VariableSummary {
                    name: "strings".to_owned(),
                    class: "string".to_owned(),
                    dimensions: vec![1, 3],
                    complex: false,
                    bytes: None,
                },
                VariableSummary {
                    name: "isolated".to_owned(),
                    class: "char".to_owned(),
                    dimensions: vec![1, 1],
                    complex: false,
                    bytes: Some(2),
                },
                VariableSummary {
                    name: "wide".to_owned(),
                    class: "uint64".to_owned(),
                    dimensions: vec![1, 1],
                    complex: false,
                    bytes: Some(8),
                },
            ])
        }

        fn shutdown(&mut self) -> Result<(), EngineError> {
            self.shutdown.store(true, Ordering::Release);
            Ok(())
        }
    }

    fn start_exact_server(
        started: Arc<AtomicBool>,
    ) -> (
        String,
        Arc<AtomicBool>,
        thread::JoinHandle<Result<(), ServerError>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let engine_shutdown = Arc::clone(&shutdown);
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(
                stream,
                || {
                    Ok(ExactEngine {
                        execution_started: started,
                        shutdown: engine_shutdown,
                    })
                },
                None,
            )
        });
        (format!("ws://{address}/kernel"), shutdown, worker)
    }

    fn start_runtime_graphics_server() -> (
        String,
        GraphicsSessionRegistry,
        thread::JoinHandle<Result<(), ServerError>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let registry = GraphicsSessionRegistry::new();
        let server_registry = registry.clone();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection_with_graphics(stream, RuntimeEngine::new, None, &server_registry)
        });
        (format!("ws://{address}/kernel"), registry, worker)
    }

    fn connect_client(url: &str) -> ClientSocket {
        let (mut socket, _) = connect(url).unwrap();
        let MaybeTlsStream::Plain(stream) = socket.get_mut() else {
            panic!("loopback test uses plain TCP");
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
    }

    fn start_workspace_server(
        root: &Path,
    ) -> (String, thread::JoinHandle<Result<(), ServerError>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let service = WorkspaceService::new(root).unwrap();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(
                stream,
                || -> Result<ExactEngine, EngineError> {
                    panic!("workspace endpoint must not construct a kernel engine")
                },
                Some(service),
            )
        });
        (format!("ws://{address}/workspace/v2"), worker)
    }

    #[allow(clippy::needless_pass_by_value)] // Inline JSON keeps socket exchanges linear.
    fn workspace_exchange(
        socket: &mut ClientSocket,
        request_id: &str,
        request_type: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        socket
            .send(Message::text(
                serde_json::json!({
                    "protocol": crate::workspace::PROTOCOL_V2,
                    "requestId": request_id,
                    "request": { "type": request_type, "params": params },
                })
                .to_string(),
            ))
            .unwrap();
        for _ in 0..16 {
            let response: serde_json::Value = serde_json::from_str(&read_text(socket)).unwrap();
            if response["requestId"] == request_id {
                return response;
            }
        }
        panic!("workspace-v2 request did not receive a correlated response");
    }

    fn read_text(socket: &mut ClientSocket) -> String {
        loop {
            match socket.read().unwrap() {
                Message::Text(text) => return text.to_string(),
                Message::Ping(_) | Message::Pong(_) => {}
                message => panic!("expected text frame, got {message:?}"),
            }
        }
    }

    fn negotiate(socket: &mut ClientSocket) -> kernel_v1::PreviewLimits {
        let request = kernel_v1::BootstrapRequestEnvelope::new(
            "exact-session",
            "initialize",
            kernel_v1::InitializeRequest::v1(
                ImplementationInfo {
                    name: "exact-client".to_owned(),
                    version: "1".to_owned(),
                },
                kernel_v1::Capabilities {
                    execution_modes: vec![ExecutionMode::Repl],
                    display_mime_types: vec![
                        "text/plain".to_owned(),
                        crate::graphics::discovery_mime_type().to_owned(),
                    ],
                    max_preview_elements: 32,
                    max_string_element_code_units: Some(1024),
                    max_preview_code_units: Some(4096),
                    interrupt: true,
                    workspace_delta: true,
                },
            ),
        );
        socket
            .send(Message::text(
                kernel_v1::encode_bootstrap_request(&request).unwrap(),
            ))
            .unwrap();
        let starting: ServerMessage = serde_json::from_str(&read_text(socket)).unwrap();
        assert!(matches!(
            starting,
            ServerMessage::Event(EventEnvelope {
                event: Event::Status(StatusEvent {
                    status: KernelStatus::Starting
                }),
                ..
            })
        ));
        let response = kernel_v1::decode_bootstrap_response(&read_text(socket)).unwrap();
        let Some(kernel_v1::BootstrapResponseResult::Initialize(result)) = response.result else {
            panic!("v1 bootstrap result");
        };
        assert_eq!(result.negotiated_protocol, kernel_v1::PROTOCOL_V1);
        let limits = result.capabilities.preview_limits().unwrap();
        assert!(matches!(
            kernel_v1::decode_event(&read_text(socket)).unwrap().event,
            Event::Status(StatusEvent {
                status: KernelStatus::Idle
            })
        ));
        limits
    }

    fn negotiate_v2(socket: &mut ClientSocket) -> kernel_v2::AggregateLimits {
        let request = kernel_v2::BootstrapRequestEnvelope::new(
            "exact-session",
            "initialize-v2",
            kernel_v2::InitializeRequest::v2(
                ImplementationInfo {
                    name: "exact-v2-client".to_owned(),
                    version: "2".to_owned(),
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
        );
        socket
            .send(Message::text(
                kernel_v2::encode_bootstrap_request(&request).unwrap(),
            ))
            .unwrap();
        let starting: ServerMessage = serde_json::from_str(&read_text(socket)).unwrap();
        assert!(matches!(
            starting,
            ServerMessage::Event(EventEnvelope {
                event: Event::Status(StatusEvent {
                    status: KernelStatus::Starting
                }),
                ..
            })
        ));
        let response = kernel_v2::decode_bootstrap_response(&read_text(socket)).unwrap();
        assert_eq!(response.protocol, openmat_protocol::PROTOCOL_V0);
        let Some(kernel_v2::BootstrapResponseResult::Initialize(result)) = response.result else {
            panic!("v2 bootstrap result");
        };
        assert_eq!(result.negotiated_protocol, kernel_v2::PROTOCOL_V2);
        let limits = result.capabilities.aggregate_limits().unwrap();
        assert!(matches!(
            kernel_v2::decode_event(&read_text(socket)).unwrap().event,
            Event::Status(StatusEvent {
                status: KernelStatus::Idle
            })
        ));
        limits
    }

    fn exchange(
        socket: &mut ClientSocket,
        request: &kernel_v1::RequestEnvelope,
        limits: &kernel_v1::PreviewLimits,
    ) -> kernel_v1::ResponseEnvelope {
        socket
            .send(Message::text(
                kernel_v1::encode_request(request, limits).unwrap(),
            ))
            .unwrap();
        let terminal = if matches!(request.request, kernel_v1::Request::Shutdown(_)) {
            KernelStatus::Dead
        } else {
            KernelStatus::Idle
        };
        let mut response = None;
        for _ in 0..64 {
            let frame = read_text(socket);
            if let Ok(decoded) = kernel_v1::decode_response_for_request(&frame, request, limits) {
                response = Some(decoded);
                continue;
            }
            let event = kernel_v1::decode_event(&frame).unwrap();
            if matches!(event.event, Event::Status(StatusEvent { status }) if status == terminal)
                && let Some(response) = response.take()
            {
                return response;
            }
        }
        panic!("request did not terminate");
    }

    fn exchange_v2(
        socket: &mut ClientSocket,
        request: &kernel_v2::RequestEnvelope,
        limits: &kernel_v2::AggregateLimits,
    ) -> kernel_v2::ResponseEnvelope {
        socket
            .send(Message::text(
                kernel_v2::encode_request(request, limits).unwrap(),
            ))
            .unwrap();
        let terminal = if matches!(request.request, kernel_v2::Request::Shutdown(_)) {
            KernelStatus::Dead
        } else {
            KernelStatus::Idle
        };
        let mut response = None;
        for _ in 0..64 {
            let frame = read_text(socket);
            if let Ok(decoded) = kernel_v2::decode_response_for_request(&frame, request, limits) {
                response = Some(decoded);
                continue;
            }
            let event = kernel_v2::decode_event(&frame).unwrap();
            if matches!(event.event, Event::Status(StatusEvent { status }) if status == terminal)
                && let Some(response) = response.take()
            {
                return response;
            }
        }
        panic!("v2 request did not terminate");
    }

    #[test]
    fn real_tcp_workspace_v2_reports_debounced_external_filesystem_changes() {
        let root = WorkspaceTestRoot::new();
        let (url, server) = start_workspace_server(&root.0);
        let mut socket = connect_client(&url);
        let current = workspace_exchange(
            &mut socket,
            "current",
            "currentDirectory",
            serde_json::json!({}),
        );
        let generation = current["result"]["data"]["generation"].as_u64().unwrap();

        fs::write(root.0.join("save1.mat"), b"MAT-file placeholder").unwrap();
        let event: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(event["protocol"], crate::workspace::PROTOCOL_V2);
        assert_eq!(event["event"]["type"], "workspaceChanged");
        assert_eq!(event["event"]["data"]["rootGeneration"], generation);

        socket.close(None).unwrap();
        drop(socket);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn real_tcp_workspace_v3_accepts_download_ticket_requests() {
        let root = WorkspaceTestRoot::new();
        fs::write(root.0.join("result.mat"), b"MAT-file placeholder").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let generation = service.current_directory_watch_target().unwrap().0;
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(
                stream,
                || -> Result<ExactEngine, EngineError> {
                    panic!("workspace endpoint must not construct a kernel engine")
                },
                Some(service),
            )
        });
        let mut socket = connect_client(&format!("ws://{address}/workspace/v3"));
        socket
            .send(Message::text(
                serde_json::json!({
                    "protocol": crate::workspace::PROTOCOL_V3,
                    "requestId": "download",
                    "request": {
                        "type": "prepareDownload",
                        "params": { "path": "result.mat", "rootGeneration": generation }
                    }
                })
                .to_string(),
            ))
            .unwrap();
        let response: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(response["protocol"], crate::workspace::PROTOCOL_V3);
        assert_eq!(response["ok"], true);
        assert_eq!(
            response["result"]["data"]["ticket"].as_str().unwrap().len(),
            64
        );

        socket.close(None).unwrap();
        drop(socket);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn download_ticket_serves_exact_bytes_over_http() {
        let root = WorkspaceTestRoot::new();
        let expected = [0x00, 0xff, 0x7f, 0x80, b'M', b'A', b'T'];
        fs::write(root.0.join("result.mat"), expected).unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let current: serde_json::Value = serde_json::from_str(
            &service.handle_text_for_protocol(
                &serde_json::json!({
                    "protocol": crate::workspace::PROTOCOL_V3,
                    "requestId": "current",
                    "request": { "type": "currentDirectory", "params": {} }
                })
                .to_string(),
                WorkspaceProtocol::V3,
            ),
        )
        .unwrap();
        let generation = current["result"]["data"]["generation"].as_u64().unwrap();
        let prepared: serde_json::Value = serde_json::from_str(
            &service.handle_text_for_protocol(
                &serde_json::json!({
                    "protocol": crate::workspace::PROTOCOL_V3,
                    "requestId": "download",
                    "request": {
                        "type": "prepareDownload",
                        "params": { "path": "result.mat", "rootGeneration": generation }
                    }
                })
                .to_string(),
                WorkspaceProtocol::V3,
            ),
        )
        .unwrap();
        let ticket = prepared["result"]["data"]["ticket"]
            .as_str()
            .unwrap()
            .to_owned();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(
                stream,
                || -> Result<ExactEngine, EngineError> {
                    panic!("HTTP download must not construct a kernel engine")
                },
                Some(service),
            )
        });
        let mut client = TcpStream::connect(address).unwrap();
        write!(
            client,
            "GET {DOWNLOAD_PATH_PREFIX}{ticket} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        worker.join().unwrap().unwrap();

        let separator = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let headers = std::str::from_utf8(&response[..separator]).unwrap();
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(headers.contains("Content-Disposition: attachment; filename=\"result.mat\""));
        assert!(headers.contains("Content-Type: application/octet-stream"));
        assert_eq!(&response[separator + 4..], expected);
    }

    #[test]
    fn upload_ticket_accepts_cors_preflight_and_streams_exact_binary_bytes() {
        let root = WorkspaceTestRoot::new();
        let expected = [0x00, 0xff, 0x7f, 0x80, b'M', b'A', b'T'];
        let service = WorkspaceService::new(&root.0).unwrap();
        let current: serde_json::Value = serde_json::from_str(
            &service.handle_text_for_protocol(
                &serde_json::json!({
                    "protocol": crate::workspace::PROTOCOL_V3,
                    "requestId": "current",
                    "request": { "type": "currentDirectory", "params": {} }
                })
                .to_string(),
                WorkspaceProtocol::V3,
            ),
        )
        .unwrap();
        let generation = current["result"]["data"]["generation"].as_u64().unwrap();
        let prepared: serde_json::Value = serde_json::from_str(
            &service.handle_text_for_protocol(
                &serde_json::json!({
                    "protocol": crate::workspace::PROTOCOL_V3,
                    "requestId": "upload",
                    "request": {
                        "type": "prepareUpload",
                        "params": {
                            "path": "uploaded.mat",
                            "size": expected.len(),
                            "rootGeneration": generation,
                            "overwrite": false
                        }
                    }
                })
                .to_string(),
                WorkspaceProtocol::V3,
            ),
        )
        .unwrap();
        let ticket = prepared["result"]["data"]["ticket"]
            .as_str()
            .unwrap()
            .to_owned();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            for service in [service.clone(), service] {
                let (stream, _) = listener.accept().unwrap();
                serve_connection(
                    stream,
                    || -> Result<ExactEngine, EngineError> {
                        panic!("HTTP upload must not construct a kernel engine")
                    },
                    Some(service),
                )?;
            }
            Ok::<(), ServerError>(())
        });

        let mut preflight = TcpStream::connect(address).unwrap();
        write!(
            preflight,
            "OPTIONS {UPLOAD_PATH_PREFIX}{ticket} HTTP/1.1\r\nHost: {address}\r\nOrigin: http://localhost:5173\r\nAccess-Control-Request-Method: PUT\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut preflight_response = String::new();
        preflight.read_to_string(&mut preflight_response).unwrap();
        assert!(preflight_response.starts_with("HTTP/1.1 204 No Content\r\n"));
        assert!(preflight_response.contains("Access-Control-Allow-Origin: *"));
        assert!(preflight_response.contains("Access-Control-Allow-Methods: PUT, OPTIONS"));

        let mut upload = TcpStream::connect(address).unwrap();
        write!(
            upload,
            "PUT {UPLOAD_PATH_PREFIX}{ticket} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            expected.len(),
        )
        .unwrap();
        upload.write_all(&expected).unwrap();
        let mut response = Vec::new();
        upload.read_to_end(&mut response).unwrap();
        worker.join().unwrap().unwrap();

        let separator = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let headers = std::str::from_utf8(&response[..separator]).unwrap();
        assert!(headers.starts_with("HTTP/1.1 201 Created\r\n"));
        assert!(headers.contains("Access-Control-Allow-Origin: *"));
        let body: serde_json::Value = serde_json::from_slice(&response[separator + 4..]).unwrap();
        assert_eq!(body["entry"]["path"], "uploaded.mat");
        assert_eq!(fs::read(root.0.join("uploaded.mat")).unwrap(), expected);
    }

    #[test]
    fn download_filename_has_safe_ascii_and_utf8_content_disposition_forms() {
        assert_eq!(
            content_disposition("结果 1.mat"),
            "attachment; filename=\"__ 1.mat\"; filename*=UTF-8''%E7%BB%93%E6%9E%9C%201.mat"
        );
        assert_eq!(
            content_disposition("bad\r\nname.dat"),
            "attachment; filename=\"bad__name.dat\"; filename*=UTF-8''bad%0D%0Aname.dat"
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One real socket lifecycle verifies ordered file mutations.
    fn real_tcp_workspace_v2_completes_file_lifecycle_and_preserves_conflicts() {
        let root = WorkspaceTestRoot::new();
        let (url, server) = start_workspace_server(&root.0);
        let mut socket = connect_client(&url);

        assert_eq!(
            workspace_exchange(
                &mut socket,
                "create-dir",
                "create",
                serde_json::json!({ "path": "src", "kind": "directory" }),
            )["ok"],
            true
        );
        assert_eq!(
            workspace_exchange(
                &mut socket,
                "create-file",
                "create",
                serde_json::json!({ "path": "src/live.m", "kind": "file" }),
            )["ok"],
            true
        );
        let opened = workspace_exchange(
            &mut socket,
            "read",
            "read",
            serde_json::json!({ "path": "src/live.m" }),
        );
        let revision = opened["result"]["data"]["revision"].as_str().unwrap();
        let saved = workspace_exchange(
            &mut socket,
            "write",
            "write",
            serde_json::json!({
                "path": "src/live.m",
                "content": "live = 42;\n",
                "expectedRevision": revision,
            }),
        );
        assert_eq!(saved["ok"], true);
        let stale_revision = saved["result"]["data"]["entry"]["revision"]
            .as_str()
            .unwrap();
        fs::write(root.0.join("src/live.m"), "external = 7;\n").unwrap();
        let conflict = workspace_exchange(
            &mut socket,
            "conflict",
            "write",
            serde_json::json!({
                "path": "src/live.m",
                "content": "must_not_win = 1;\n",
                "expectedRevision": stale_revision,
            }),
        );
        assert_eq!(conflict["error"]["code"], "workspace.revisionConflict");
        assert_eq!(
            fs::read_to_string(root.0.join("src/live.m")).unwrap(),
            "external = 7;\n"
        );

        let moved = workspace_exchange(
            &mut socket,
            "move",
            "move",
            serde_json::json!({
                "path": "src/live.m", "targetPath": "live.m"
            }),
        );
        assert_eq!(moved["result"]["data"]["entry"]["path"], "live.m");
        let listed = workspace_exchange(
            &mut socket,
            "list",
            "list",
            serde_json::json!({ "path": "", "recursive": true }),
        );
        assert!(
            listed["result"]["data"]["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["path"] == "live.m")
        );
        assert_eq!(
            workspace_exchange(
                &mut socket,
                "delete-file",
                "delete",
                serde_json::json!({
                    "path": "live.m", "recursive": false, "confirmPath": null
                }),
            )["ok"],
            true
        );
        assert_eq!(
            workspace_exchange(
                &mut socket,
                "delete-dir",
                "delete",
                serde_json::json!({
                    "path": "src", "recursive": false, "confirmPath": null
                }),
            )["ok"],
            true
        );
        socket.close(None).unwrap();
        drop(socket);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn real_tcp_v2_routes_execute_list_aggregate_inspect_and_shutdown() {
        let started = Arc::new(AtomicBool::new(false));
        let (url, shutdown_called, server) = start_exact_server(started);
        let mut socket = connect_client(&url);
        let limits = negotiate_v2(&mut socket);

        let execute = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "execute-v2",
            kernel_v2::Request::Execute(ExecuteRequest {
                code: "seed".to_owned(),
                source_name: "v2-lifecycle.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        assert!(exchange_v2(&mut socket, &execute, &limits).ok);

        let list = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "list-v2",
            kernel_v2::Request::ListWorkspace(ListWorkspaceRequest {}),
        );
        let response = exchange_v2(&mut socket, &list, &limits);
        let Some(kernel_v2::ResponseResult::ListWorkspace(workspace)) = response.result else {
            panic!("v2 workspace result");
        };
        assert_eq!(workspace.variables.len(), 3);

        let inspect = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "inspect-aggregate-v2",
            kernel_v2::Request::Inspect(kernel_v2::InspectRequest {
                name: "aggregate".to_owned(),
                range: kernel_v2::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 2],
                },
                max_elements: 2,
            }),
        );
        let response = exchange_v2(&mut socket, &inspect, &limits);
        let Some(kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Aggregate(
            kernel_v2::AggregatePreview::Cell { items, usage, .. },
        ))) = response.result
        else {
            panic!("v2 aggregate inspect result");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(usage.nodes, 3);

        let shutdown = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "shutdown-v2",
            kernel_v2::Request::Shutdown(ShutdownRequest {}),
        );
        assert!(exchange_v2(&mut socket, &shutdown, &limits).ok);
        match socket.read().unwrap() {
            Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Normal),
            message => panic!("expected normal close, got {message:?}"),
        }
        server.join().unwrap().unwrap();
        assert!(shutdown_called.load(Ordering::Acquire));
    }

    #[test]
    fn real_tcp_v2_interrupt_precedes_interrupted_execution_completion() {
        let started = Arc::new(AtomicBool::new(false));
        let (url, shutdown_called, server) = start_exact_server(Arc::clone(&started));
        let mut socket = connect_client(&url);
        let limits = negotiate_v2(&mut socket);
        let wait = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "wait-v2",
            kernel_v2::Request::Execute(ExecuteRequest {
                code: "wait".to_owned(),
                source_name: "v2-wait.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        socket
            .send(Message::text(
                kernel_v2::encode_request(&wait, &limits).unwrap(),
            ))
            .unwrap();
        let busy = kernel_v2::decode_event(&read_text(&mut socket)).unwrap();
        assert!(matches!(
            busy.event,
            Event::Status(StatusEvent {
                status: KernelStatus::Busy
            })
        ));
        assert!(started.load(Ordering::Acquire));

        let interrupt = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "interrupt-v2",
            kernel_v2::Request::Interrupt(InterruptRequest {}),
        );
        socket
            .send(Message::text(
                kernel_v2::encode_request(&interrupt, &limits).unwrap(),
            ))
            .unwrap();
        let accepted =
            kernel_v2::decode_response_for_request(&read_text(&mut socket), &interrupt, &limits)
                .unwrap();
        let Some(kernel_v2::ResponseResult::Interrupt(result)) = accepted.result else {
            panic!("v2 interrupt result");
        };
        assert!(result.accepted);
        let interrupted = kernel_v2::decode_event(&read_text(&mut socket)).unwrap();
        assert!(matches!(
            interrupted.event,
            Event::Status(StatusEvent {
                status: KernelStatus::Interrupted
            })
        ));

        let mut execute_interrupted = false;
        let mut idle = false;
        for _ in 0..16 {
            let frame = read_text(&mut socket);
            if let Ok(response) = kernel_v2::decode_response_for_request(&frame, &wait, &limits) {
                let Some(kernel_v2::ResponseResult::Execute(result)) = response.result else {
                    panic!("v2 execute result");
                };
                execute_interrupted = result.interrupted;
            } else {
                let event = kernel_v2::decode_event(&frame).unwrap();
                idle |= matches!(
                    event.event,
                    Event::Status(StatusEvent {
                        status: KernelStatus::Idle
                    })
                );
            }
            if execute_interrupted && idle {
                break;
            }
        }
        assert!(execute_interrupted && idle);

        let shutdown = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "shutdown-after-interrupt-v2",
            kernel_v2::Request::Shutdown(ShutdownRequest {}),
        );
        assert!(exchange_v2(&mut socket, &shutdown, &limits).ok);
        let _ = socket.read();
        server.join().unwrap().unwrap();
        assert!(shutdown_called.load(Ordering::Acquire));
    }

    #[test]
    fn negotiated_v2_websocket_closes_on_v1_protocol_mixing() {
        let (url, shutdown_called, server) = start_exact_server(Arc::new(AtomicBool::new(false)));
        let mut socket = connect_client(&url);
        let limits = negotiate_v2(&mut socket);
        let mixed = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "mixed-v1",
            kernel_v1::Request::ListWorkspace(ListWorkspaceRequest {}),
        );
        socket
            .send(Message::text(
                kernel_v1::encode_request(&mixed, &limits.preview_limits()).unwrap(),
            ))
            .unwrap();
        match socket.read().unwrap() {
            Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Policy),
            message => panic!("expected policy close, got {message:?}"),
        }
        server.join().unwrap().unwrap();
        assert!(shutdown_called.load(Ordering::Acquire));
    }

    #[test]
    fn runtime_plot_emits_server_owned_discovery_and_registers_exact_hub() {
        let (url, registry, server) = start_runtime_graphics_server();
        let mut socket = connect_client(&url);
        let limits = negotiate(&mut socket);
        let execute = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "plot-execute",
            kernel_v1::Request::Execute(ExecuteRequest {
                code: "plot([1 2 3], [1 4 9]);".to_owned(),
                source_name: "graphics-e2e.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        socket
            .send(Message::text(
                kernel_v1::encode_request(&execute, &limits).unwrap(),
            ))
            .unwrap();
        let mut response_seen = false;
        let mut idle_seen = false;
        let mut discovery = None;
        for _ in 0..64 {
            let frame = read_text(&mut socket);
            if kernel_v1::decode_response_for_request(&frame, &execute, &limits).is_ok() {
                response_seen = true;
                continue;
            }
            let event = kernel_v1::decode_event(&frame).unwrap();
            match event.event {
                Event::Display(DisplayEvent { representations }) => {
                    if let Some(encoded) =
                        representations.get(crate::graphics::discovery_mime_type())
                    {
                        discovery =
                            Some(serde_json::from_str::<serde_json::Value>(encoded).unwrap());
                    }
                }
                Event::Status(StatusEvent {
                    status: KernelStatus::Idle,
                }) => idle_seen = true,
                _ => {}
            }
            if response_seen && idle_seen && discovery.is_some() {
                break;
            }
        }
        assert!(response_seen && idle_seen);
        let discovery = discovery.expect("one first-Figure discovery Display");
        assert_eq!(
            discovery["graphicsProtocol"],
            openmat_plot_protocol::PROTOCOL_V4
        );
        assert_eq!(discovery["schemaVersion"], 4);
        assert_eq!(discovery["endpoint"], "/graphics/v4");
        let token = openmat_plot_protocol::AttachToken::new(
            discovery["attachToken"].as_str().unwrap().to_owned(),
        );
        let attachment = registry.attach("exact-session", &token).unwrap();
        let initialized = attachment.hub().initialize().unwrap();
        assert_eq!(initialized.figures.len(), 1);
        assert_eq!(
            initialized.figures[0].figure_id,
            discovery["figureId"].as_str().unwrap()
        );
        assert_eq!(initialized.figures[0].revision, 1);
        drop(attachment);

        let shutdown = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "shutdown-graphics",
            kernel_v1::Request::Shutdown(ShutdownRequest {}),
        );
        assert!(exchange(&mut socket, &shutdown, &limits).ok);
        let _ = socket.read();
        server.join().unwrap().unwrap();
        assert!(registry.attach("exact-session", &token).is_err());
    }

    #[test]
    fn runtime_xy_plot_terminates_over_the_v2_websocket() {
        let (url, _registry, server) = start_runtime_graphics_server();
        let mut socket = connect_client(&url);
        let limits = negotiate_v2(&mut socket);
        let execute = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "plot-v2-execute",
            kernel_v2::Request::Execute(ExecuteRequest {
                code: "plot([1 2 3], [1 4 9]);".to_owned(),
                source_name: "graphics-v2-e2e.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        assert!(exchange_v2(&mut socket, &execute, &limits).ok);
        let shutdown = kernel_v2::RequestEnvelope::new(
            "exact-session",
            "shutdown-graphics-v2",
            kernel_v2::Request::Shutdown(ShutdownRequest {}),
        );
        assert!(exchange_v2(&mut socket, &shutdown, &limits).ok);
        let _ = socket.read();
        server.join().unwrap().unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One end-to-end socket lifecycle is clearer kept linear.
    fn real_tcp_v1_exact_values_interrupt_and_shutdown_share_one_locked_session() {
        let started = Arc::new(AtomicBool::new(false));
        let (url, shutdown_called, server) = start_exact_server(Arc::clone(&started));
        let mut socket = connect_client(&url);
        let limits = negotiate(&mut socket);

        let execute = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "execute",
            kernel_v1::Request::Execute(ExecuteRequest {
                code: "seed".to_owned(),
                source_name: "exact-test.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        assert!(exchange(&mut socket, &execute, &limits).ok);

        let list = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "list",
            kernel_v1::Request::ListWorkspace(ListWorkspaceRequest {}),
        );
        let response = exchange(&mut socket, &list, &limits);
        let Some(kernel_v1::ResponseResult::ListWorkspace(workspace)) = response.result else {
            panic!("workspace result");
        };
        assert_eq!(
            workspace
                .variables
                .iter()
                .map(|variable| variable.name.as_str())
                .collect::<Vec<_>>(),
            ["isolated", "strings", "wide"]
        );

        let inspect = |message_id: &str, name: &str, size: Vec<u64>, max_elements: u64| {
            kernel_v1::RequestEnvelope::new(
                "exact-session",
                message_id,
                kernel_v1::Request::Inspect(kernel_v1::InspectRequest {
                    name: name.to_owned(),
                    range: kernel_v1::MatrixRange {
                        start: vec![1, 1],
                        size,
                    },
                    max_elements,
                }),
            )
        };
        let wide = inspect("inspect-wide", "wide", vec![1, 1], 1);
        let response = exchange(&mut socket, &wide, &limits);
        let Some(kernel_v1::ResponseResult::Inspect(preview)) = response.result else {
            panic!("wide preview");
        };
        assert_eq!(
            preview.values,
            [PreviewValue::Integer {
                real: u64::MAX.to_string(),
                imaginary: "0".to_owned(),
            }]
        );

        let isolated = inspect("inspect-isolated", "isolated", vec![1, 1], 1);
        let response = exchange(&mut socket, &isolated, &limits);
        let Some(kernel_v1::ResponseResult::Inspect(preview)) = response.result else {
            panic!("isolated preview");
        };
        assert_eq!(
            preview.values,
            [PreviewValue::CharCodeUnit { value: 0xD83D }]
        );

        let strings = inspect("inspect-strings", "strings", vec![1, 3], 3);
        let response = exchange(&mut socket, &strings, &limits);
        let Some(kernel_v1::ResponseResult::Inspect(preview)) = response.result else {
            panic!("string preview");
        };
        assert_eq!(
            preview.values,
            [
                PreviewValue::String {
                    code_units: vec![0xD83D],
                    missing: false,
                },
                PreviewValue::String {
                    code_units: Vec::new(),
                    missing: false,
                },
                PreviewValue::String {
                    code_units: Vec::new(),
                    missing: true,
                },
            ]
        );

        started.store(false, Ordering::Release);
        let wait = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "wait",
            kernel_v1::Request::Execute(ExecuteRequest {
                code: "wait".to_owned(),
                source_name: "wait.m".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        socket
            .send(Message::text(
                kernel_v1::encode_request(&wait, &limits).unwrap(),
            ))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let event = kernel_v1::decode_event(&read_text(&mut socket)).unwrap();
            if matches!(
                event.event,
                Event::Status(StatusEvent {
                    status: KernelStatus::Busy
                })
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "busy event was not early");
        }
        assert!(started.load(Ordering::Acquire));

        let interrupt = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "interrupt",
            kernel_v1::Request::Interrupt(InterruptRequest {}),
        );
        socket
            .send(Message::text(
                kernel_v1::encode_request(&interrupt, &limits).unwrap(),
            ))
            .unwrap();
        let mut interrupt_accepted = false;
        let mut execute_interrupted = false;
        let mut idle = false;
        for _ in 0..32 {
            let frame = read_text(&mut socket);
            let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
            match value.get("replyTo").and_then(serde_json::Value::as_str) {
                Some("interrupt") => {
                    let response =
                        kernel_v1::decode_response_for_request(&frame, &interrupt, &limits)
                            .unwrap();
                    let Some(kernel_v1::ResponseResult::Interrupt(result)) = response.result else {
                        panic!("interrupt result");
                    };
                    interrupt_accepted = result.accepted;
                }
                Some("wait") => {
                    let response =
                        kernel_v1::decode_response_for_request(&frame, &wait, &limits).unwrap();
                    let Some(kernel_v1::ResponseResult::Execute(result)) = response.result else {
                        panic!("execute result");
                    };
                    execute_interrupted = result.interrupted;
                }
                _ => {
                    let event = kernel_v1::decode_event(&frame).unwrap();
                    idle |= matches!(
                        event.event,
                        Event::Status(StatusEvent {
                            status: KernelStatus::Idle
                        })
                    );
                }
            }
            if interrupt_accepted && execute_interrupted && idle {
                break;
            }
        }
        assert!(interrupt_accepted && execute_interrupted && idle);

        let shutdown = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "shutdown",
            kernel_v1::Request::Shutdown(ShutdownRequest {}),
        );
        assert!(exchange(&mut socket, &shutdown, &limits).ok);
        match socket.read().unwrap() {
            Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Normal),
            message => panic!("expected normal close, got {message:?}"),
        }
        server.join().unwrap().unwrap();
        assert!(shutdown_called.load(Ordering::Acquire));
    }

    #[test]
    fn disconnect_orders_engine_shutdown_after_v1_negotiation() {
        let (url, shutdown_called, server) = start_exact_server(Arc::new(AtomicBool::new(false)));
        let mut socket = connect_client(&url);
        negotiate(&mut socket);
        socket.close(None).unwrap();
        drop(socket);

        server.join().unwrap().unwrap();
        assert!(shutdown_called.load(Ordering::Acquire));
    }

    #[test]
    fn invalid_negotiated_v1_request_closes_when_the_worker_rejects_it() {
        let (url, shutdown_called, server) = start_exact_server(Arc::new(AtomicBool::new(false)));
        let mut socket = connect_client(&url);
        negotiate(&mut socket);
        let invalid = kernel_v1::RequestEnvelope::new(
            "exact-session",
            "over-limit",
            kernel_v1::Request::Inspect(kernel_v1::InspectRequest {
                name: "wide".to_owned(),
                range: kernel_v1::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 1],
                },
                max_elements: 33,
            }),
        );
        socket
            .send(Message::text(serde_json::to_string(&invalid).unwrap()))
            .unwrap();
        match socket.read().unwrap() {
            Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Error),
            message => panic!("expected worker failure close, got {message:?}"),
        }

        let error = server.join().unwrap().expect_err("worker rejection");
        assert!(matches!(error, ServerError::Worker(_)));
        assert!(shutdown_called.load(Ordering::Acquire));
    }

    #[test]
    fn websocket_engine_factory_failure_is_correlated_and_never_starts_a_session() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(
                stream,
                || -> Result<ExactEngine, EngineError> {
                    Err(EngineError::new("engine.bootstrap", "test failure"))
                },
                None,
            )
        });
        let mut socket = connect_client(&format!("ws://{address}/kernel"));
        let request = kernel_v1::BootstrapRequestEnvelope::new(
            "factory-failure-session",
            "initialize-failure",
            kernel_v1::InitializeRequest::v1(
                ImplementationInfo {
                    name: "factory-failure-client".to_owned(),
                    version: "1".to_owned(),
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
        );
        socket
            .send(Message::text(
                kernel_v1::encode_bootstrap_request(&request).unwrap(),
            ))
            .unwrap();
        let response = kernel_v1::decode_bootstrap_response(&read_text(&mut socket)).unwrap();
        assert_eq!(response.reply_to, "initialize-failure");
        assert_eq!(
            response.error.expect("factory failure").category,
            "engine.bootstrap"
        );
        match socket.read().unwrap() {
            Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Error),
            message => panic!("expected factory failure close, got {message:?}"),
        }
        server.join().unwrap().unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One complete real JSON-RPC/LSP socket lifecycle is clearest kept linear.
    fn lsp_websocket_routes_real_completion_diagnostic_changes_and_close_cleanup() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(
                stream,
                || -> Result<ExactEngine, EngineError> {
                    panic!("the LSP endpoint must not initialize the kernel engine")
                },
                None,
            )
        });
        let mut socket = connect_client(&format!("ws://{address}/lsp"));
        let send = |socket: &mut ClientSocket, value: serde_json::Value| {
            socket
                .send(Message::text(value.to_string()))
                .expect("send LSP JSON-RPC message");
        };

        socket
            .send(Message::text("{"))
            .expect("send malformed LSP JSON");
        let parse_error: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(parse_error["id"], serde_json::Value::Null);
        assert_eq!(parse_error["error"]["code"], -32700);

        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": "initialize-1",
                "method": "initialize",
                "params": {
                    "clientInfo": { "name": "openmat-server-test", "version": "1" },
                    "capabilities": {}
                }
            }),
        );
        let initialized: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(initialized["id"], "initialize-1");
        assert_eq!(initialized["result"]["serverInfo"]["name"], "openmat-lsp");
        assert_eq!(
            initialized["result"]["capabilities"]["completionProvider"]["resolveProvider"],
            true
        );
        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {}
            }),
        );

        let uri = "file:///workspace/browser-demo.m";
        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": "openmat",
                        "version": 1,
                        "text": "function y = calculate(x)\ny = x;\nend\ncal"
                    }
                }
            }),
        );
        let opened: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(opened["method"], "textDocument/publishDiagnostics");
        assert_eq!(opened["params"]["version"], 1);

        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 41,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": { "uri": uri },
                    "position": { "line": 3, "character": 3 }
                }
            }),
        );
        let completion: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(completion["id"], 41);
        let calculate = completion["result"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["label"] == "calculate"))
            .expect("completion must come from the open document in openmat-lsp");
        assert_eq!(calculate["detail"], "function");
        assert_eq!(calculate["textEdit"]["newText"], "calculate");

        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": { "uri": uri },
                    "position": { "line": 0, "character": 1 }
                }
            }),
        );
        let builtins: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(builtins["id"], 42);
        let fft = builtins["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == "fft")
            .expect("registered fft completion over WebSocket");
        assert_eq!(fft["kind"], 3);
        assert_eq!(fft["detail"], "built-in function");
        assert_eq!(fft["textEdit"]["newText"], "fft");
        assert_eq!(fft["textEdit"]["range"]["end"]["character"], 1);
        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0", "id": 43,
                "method": "completionItem/resolve", "params": fft
            }),
        );
        let resolved: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(resolved["id"], 43);
        assert_eq!(resolved["result"]["textEdit"], fft["textEdit"]);
        assert!(
            resolved["result"]["documentation"]["value"]
                .as_str()
                .unwrap()
                .contains("core runtime")
        );

        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": uri, "version": 2 },
                    "contentChanges": [{ "text": "😀" }]
                }
            }),
        );
        let invalid: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(invalid["params"]["version"], 2);
        assert!(
            !invalid["params"]["diagnostics"]
                .as_array()
                .expect("diagnostic array")
                .is_empty()
        );

        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": uri, "version": 3 },
                    "contentChanges": [{ "text": "value = 1;\n" }]
                }
            }),
        );
        let valid: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(valid["params"]["version"], 3);
        assert_eq!(valid["params"]["diagnostics"], serde_json::json!([]));

        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didClose",
                "params": { "textDocument": { "uri": uri } }
            }),
        );
        let closed: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(closed["params"]["uri"], uri);
        assert_eq!(closed["params"]["diagnostics"], serde_json::json!([]));

        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": "shutdown-1",
                "method": "shutdown",
                "params": null
            }),
        );
        let shutdown: serde_json::Value = serde_json::from_str(&read_text(&mut socket)).unwrap();
        assert_eq!(shutdown["id"], "shutdown-1");
        assert_eq!(shutdown["result"], serde_json::Value::Null);
        send(
            &mut socket,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "exit",
                "params": null
            }),
        );
        match socket.read().unwrap() {
            Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Normal),
            message => panic!("expected normal LSP close, got {message:?}"),
        }
        server.join().unwrap().unwrap();
    }

    #[test]
    fn lsp_websocket_enforces_its_one_mib_text_message_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(
                stream,
                || -> Result<ExactEngine, EngineError> {
                    panic!("the LSP endpoint must not initialize the kernel engine")
                },
                None,
            )
        });
        let mut socket = connect_client(&format!("ws://{address}/lsp"));
        socket
            .send(Message::text(
                "x".repeat(crate::lsp_websocket::MAX_LSP_MESSAGE_BYTES + 1),
            ))
            .expect("send oversized LSP text message");
        match socket.read().unwrap() {
            Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Size),
            message => panic!("expected LSP size close, got {message:?}"),
        }
        server.join().unwrap().unwrap();
    }

    struct WorkspaceLspClient {
        socket: ClientSocket,
        worker: Option<thread::JoinHandle<Result<(), ServerError>>>,
        service: WorkspaceService,
        next_id: u32,
    }

    impl WorkspaceLspClient {
        fn new(root: &Path) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let service = WorkspaceService::new(root).unwrap();
            let server_service = service.clone();
            let worker = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                serve_connection(
                    stream,
                    || -> Result<ExactEngine, EngineError> {
                        panic!("LSP indexing must not start the kernel engine")
                    },
                    Some(server_service),
                )
            });
            let mut client = Self {
                socket: connect_client(&format!("ws://{address}/lsp")),
                worker: Some(worker),
                service,
                next_id: 0,
            };
            let initialized =
                client.request("initialize", serde_json::json!({ "capabilities": {} }));
            assert!(initialized.get("result").is_some());
            client.notify("initialized", serde_json::json!({}));
            client
        }

        fn uri(&self, path: &str) -> String {
            let (generation, root) = self.service.current_directory_watch_target().unwrap();
            format!(
                "{}{}",
                crate::lsp_workspace::workspace_root_uri(generation, root.to_str().unwrap()),
                crate::lsp_workspace::encoded_relative_path(path)
            )
        }

        #[allow(clippy::needless_pass_by_value)] // Inline JSON keeps socket exchanges readable.
        fn notify(&mut self, method: &str, params: serde_json::Value) {
            self.socket
                .send(Message::text(
                    serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params })
                        .to_string(),
                ))
                .unwrap();
        }

        fn open(&mut self, uri: &str, text: &str) {
            self.notify(
                "textDocument/didOpen",
                serde_json::json!({ "textDocument": {
                    "uri": uri, "version": 1, "languageId": "openmat", "text": text
                }}),
            );
        }

        #[allow(clippy::needless_pass_by_value)] // Inline JSON keeps socket exchanges readable.
        fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
            self.next_id += 1;
            self.socket.send(Message::text(serde_json::json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params }).to_string())).unwrap();
            loop {
                let response: serde_json::Value =
                    serde_json::from_str(&read_text(&mut self.socket)).unwrap();
                if response["id"] == self.next_id {
                    return response;
                }
            }
        }

        fn definition(&mut self, uri: &str) -> serde_json::Value {
            let response = self.request(
                "textDocument/definition",
                serde_json::json!({
                    "textDocument": { "uri": uri }, "position": { "line": 0, "character": 7 }
                }),
            );
            assert!(response.get("error").is_none(), "{response}");
            response["result"].clone()
        }

        fn wait_definition(&mut self, uri: &str, target: Option<(&str, u32)>) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let actual = self.definition(uri);
                let location = if actual.is_array() {
                    &actual[0]
                } else {
                    &actual
                };
                let matches = match target {
                    Some((target, line)) => {
                        location["uri"] == target && location["range"]["start"]["line"] == line
                    }
                    None => actual.as_array().is_some_and(Vec::is_empty) || actual.is_null(),
                };
                if matches {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "definition did not become {target:?}: {actual}"
                );
                thread::sleep(Duration::from_millis(75));
            }
        }
    }

    impl Drop for WorkspaceLspClient {
        fn drop(&mut self) {
            let _ = self.socket.close(None);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    #[test]
    fn lsp_websocket_indexes_unopened_files_and_keeps_editor_overlays_until_close() {
        let root = WorkspaceTestRoot::new();
        fs::write(
            root.0.join("helper.m"),
            "function y = helper(x)\ny = x;\nend\n",
        )
        .unwrap();
        let mut client = WorkspaceLspClient::new(&root.0);
        let main = client.uri("main.m");
        let helper = client.uri("helper.m");
        client.open(&main, "out = helper(2);\n");
        client.wait_definition(&main, Some((&helper, 0)));
        client.open(&helper, "\n\nfunction y = helper(x)\ny = x;\nend\n");
        client.wait_definition(&main, Some((&helper, 2)));
        // Force refresh through rename so the changed disk revision has been
        // read while the unsaved overlay still owns effective source text.
        fs::write(
            root.0.join("helper.m"),
            "\nfunction y = helper(x)\ny = x + 1;\nend\n",
        )
        .unwrap();
        let rename = client.request("textDocument/rename", serde_json::json!({
            "textDocument": { "uri": main }, "position": { "line": 0, "character": 7 }, "newName": "calculate"
        }));
        assert!(rename.get("error").is_none(), "{rename}");
        client.wait_definition(&main, Some((&helper, 2)));
        client.notify(
            "textDocument/didClose",
            serde_json::json!({ "textDocument": { "uri": helper } }),
        );
        client.wait_definition(&main, Some((&helper, 1)));
    }

    #[test]
    fn lsp_websocket_refreshes_created_modified_and_deleted_disk_files() {
        let root = WorkspaceTestRoot::new();
        let mut client = WorkspaceLspClient::new(&root.0);
        let main = client.uri("main.m");
        let helper = client.uri("helper.m");
        client.open(&main, "out = helper(2);\n");
        client.wait_definition(&main, None);
        fs::write(
            root.0.join("helper.m"),
            "function y = helper(x)\ny = x;\nend\n",
        )
        .unwrap();
        client.wait_definition(&main, Some((&helper, 0)));
        fs::write(
            root.0.join("helper.m"),
            "\nfunction y = helper(x)\ny = x + 1;\nend\n",
        )
        .unwrap();
        client.wait_definition(&main, Some((&helper, 1)));
        fs::remove_file(root.0.join("helper.m")).unwrap();
        client.wait_definition(&main, None);
    }

    #[test]
    fn lsp_websocket_root_changes_isolate_old_drafts_and_search_paths_stay_inside_root() {
        let first = WorkspaceTestRoot::new();
        let second = WorkspaceTestRoot::new();
        fs::write(
            first.0.join("helper.m"),
            "function y = helper(x)\ny = x;\nend\n",
        )
        .unwrap();
        fs::create_dir(second.0.join("lib")).unwrap();
        fs::write(
            second.0.join("lib/helper.m"),
            "function y = helper(x)\ny = x + 1;\nend\n",
        )
        .unwrap();
        let mut client = WorkspaceLspClient::new(&first.0);
        let old_main = client.uri("main.m");
        let old_helper = client.uri("helper.m");
        client.open(&old_main, "out = helper(2);\n");
        client.wait_definition(&old_main, Some((&old_helper, 0)));
        client
            .service
            .working_directory()
            .change(&second.0)
            .unwrap();
        let main = client.uri("main.m");
        let helper = client.uri("lib/helper.m");
        client.open(&main, "out = helper(3);\n");
        client.wait_definition(&old_main, None);
        // Arbitrary recursively indexed directories and outside-root MATLAB
        // search paths must not become globally visible project declarations.
        client
            .service
            .search_path()
            .add_from([&first.0], None, openmat_runtime::SearchPathPosition::End)
            .unwrap();
        client.wait_definition(&main, None);
        client
            .service
            .search_path()
            .add_from(
                [second.0.join("lib")],
                None,
                openmat_runtime::SearchPathPosition::End,
            )
            .unwrap();
        client.wait_definition(&main, Some((&helper, 0)));
        client.wait_definition(&old_main, None);
    }

    #[test]
    fn lsp_websocket_disables_rename_for_unreadable_or_oversized_index_sources() {
        let root = WorkspaceTestRoot::new();
        fs::write(
            root.0.join("helper.m"),
            "function y = helper(x)\ny = x;\nend\n",
        )
        .unwrap();
        let mut client = WorkspaceLspClient::new(&root.0);
        let main = client.uri("main.m");
        client.open(&main, "out = helper(2);\n");
        let params = serde_json::json!({ "textDocument": { "uri": main }, "position": { "line": 0, "character": 7 }, "newName": "calculate" });
        fs::write(root.0.join("unknown.m"), [0xff, 0xfe]).unwrap();
        let invalid = client.request("textDocument/rename", params.clone());
        assert_eq!(invalid["error"]["code"], -32803, "{invalid}");
        fs::write(root.0.join("unknown.m"), " ".repeat(128 * 1024 + 1)).unwrap();
        let oversized = client.request("textDocument/rename", params.clone());
        assert_eq!(oversized["error"]["code"], -32803, "{oversized}");
        fs::remove_file(root.0.join("unknown.m")).unwrap();
        let repaired = client.request("textDocument/rename", params);
        assert!(repaired.get("error").is_none(), "{repaired}");
    }

    #[test]
    fn real_tcp_workspace_rename_can_target_a_documents_previous_root() {
        let first = WorkspaceTestRoot::new();
        let second = WorkspaceTestRoot::new();
        fs::write(first.0.join("helper.m"), "original=1;\n").unwrap();
        fs::write(second.0.join("helper.m"), "unrelated=2;\n").unwrap();
        let (url, worker) = start_workspace_server(&first.0);
        let mut socket = connect_client(&url);
        let original = workspace_exchange(
            &mut socket,
            "original",
            "currentDirectory",
            serde_json::json!({}),
        );
        let generation = original["result"]["data"]["generation"].as_u64().unwrap();
        let changed = workspace_exchange(
            &mut socket,
            "change",
            "changeDirectory",
            serde_json::json!({ "path": second.0.to_string_lossy() }),
        );
        assert_eq!(changed["ok"], true);
        let renamed = workspace_exchange(
            &mut socket,
            "rename-original",
            "rename",
            serde_json::json!({
                "path": "helper.m", "newName": "calculate.m", "rootGeneration": generation
            }),
        );
        assert_eq!(renamed["ok"], true, "{renamed}");
        assert!(!first.0.join("helper.m").exists());
        assert_eq!(
            fs::read_to_string(first.0.join("calculate.m")).unwrap(),
            "original=1;\n"
        );
        assert_eq!(
            fs::read_to_string(second.0.join("helper.m")).unwrap(),
            "unrelated=2;\n"
        );
        assert!(!second.0.join("calculate.m").exists());
        socket.close(None).unwrap();
        worker.join().unwrap().unwrap();
    }
}

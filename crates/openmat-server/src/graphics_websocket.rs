//! Versioned graphics WebSocket transport over a shared session registry.

use std::collections::HashSet;
use std::io;
use std::net::TcpStream;
use std::thread;

use openmat_plot_protocol::{
    BufferChunkHeader, BufferReleasedEvent, BufferTransfer, CloseFigureResult, DecodedRequest,
    ErrorCategory, Event, EventEnvelope, FigureClosedEvent, GetSnapshotResult, GraphicsProtocol,
    InitializeResult, ProtocolFailure, ReleaseBufferResult, Request, RequestEnvelope,
    ResponseEnvelope, ResyncFigureResult, ServerCapabilities, SessionClosedEvent,
    SetAxesCameraResult, SetAxesLimitsResult, ShutdownResult, ValidationError, encode_binary_frame,
    encode_event, encode_response,
};
use serde_json::Value;
use tungstenite::Error as WebSocketError;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Message, WebSocket};

use crate::ServerError;
use crate::graphics::{GraphicsAttachment, GraphicsHubError, GraphicsSessionRegistry};
use crate::websocket::{MAX_MESSAGE_BYTES, SOCKET_POLL_INTERVAL};

const MAX_CHUNK_PAYLOAD_BYTES: u64 = 1024 * 1024;

struct ConnectionState {
    protocol: GraphicsProtocol,
    attachment: Option<GraphicsAttachment>,
    limits: openmat_plot_protocol::GraphicsLimits,
    request_ids: HashSet<String>,
    next_server_message: u64,
    next_transfer: u64,
    event_cursor: u64,
}

impl ConnectionState {
    fn new(protocol: GraphicsProtocol) -> Self {
        Self {
            protocol,
            attachment: None,
            limits: openmat_plot_protocol::GraphicsLimits::default(),
            request_ids: HashSet::new(),
            next_server_message: 1,
            next_transfer: 1,
            event_cursor: 0,
        }
    }

    fn next_message_id(&mut self) -> String {
        let value = self.next_server_message;
        self.next_server_message = self.next_server_message.saturating_add(1);
        format!("graphics-server-{value}")
    }

    fn next_transfer_id(&mut self) -> String {
        let value = self.next_transfer;
        self.next_transfer = self.next_transfer.saturating_add(1);
        format!("graphics-transfer-{value}")
    }
}

pub(crate) fn serve(
    socket: &mut WebSocket<TcpStream>,
    registry: &GraphicsSessionRegistry,
    protocol: GraphicsProtocol,
) -> Result<(), ServerError> {
    let mut state = ConnectionState::new(protocol);
    loop {
        if poll_hub_events(socket, &mut state)? {
            return Ok(());
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if text.len() > MAX_MESSAGE_BYTES {
                    close(
                        socket,
                        CloseCode::Size,
                        "graphics text message exceeds 1 MiB",
                    );
                    return Ok(());
                }
                if handle_text(socket, text.as_str(), registry, &mut state)? {
                    return Ok(());
                }
            }
            Ok(Message::Binary(_)) => {
                close(
                    socket,
                    CloseCode::Unsupported,
                    "client binary graphics frames are not supported",
                );
                return Ok(());
            }
            Ok(Message::Close(_)) => {
                let _ = socket.flush();
                return Ok(());
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => socket.flush()?,
            Ok(Message::Frame(_)) => {
                close(socket, CloseCode::Protocol, "unexpected raw graphics frame");
                return Ok(());
            }
            Err(WebSocketError::Io(error)) if is_poll_timeout(&error) => {
                thread::sleep(SOCKET_POLL_INTERVAL);
            }
            Err(WebSocketError::Capacity(_)) => {
                close(
                    socket,
                    CloseCode::Size,
                    "graphics message exceeds transport limit",
                );
                return Ok(());
            }
            Err(WebSocketError::Utf8(_)) => {
                close(
                    socket,
                    CloseCode::Invalid,
                    "graphics text frame is not UTF-8",
                );
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

fn handle_text(
    socket: &mut WebSocket<TcpStream>,
    text: &str,
    registry: &GraphicsSessionRegistry,
    state: &mut ConnectionState,
) -> Result<bool, ServerError> {
    let decoded = match openmat_plot_protocol::decode_request_for(
        text,
        MAX_MESSAGE_BYTES as u64,
        state.protocol,
    ) {
        Ok(decoded) => decoded,
        Err(error) => {
            let Some((session_id, message_id)) = correlation(text) else {
                close(socket, CloseCode::Invalid, "invalid graphics request");
                return Ok(true);
            };
            send_failure(
                socket,
                state,
                &session_id,
                &message_id,
                ProtocolFailure::from(error),
            )?;
            return Ok(false);
        }
    };
    let (session_id, message_id) = match &decoded {
        DecodedRequest::Known(request) => (&request.session_id, &request.message_id),
        DecodedRequest::Unsupported(request) => (&request.session_id, &request.message_id),
    };
    if !state.request_ids.insert(message_id.clone()) {
        send_failure(
            socket,
            state,
            session_id,
            message_id,
            ProtocolFailure::new(
                ErrorCategory::InvalidRequest,
                "messageId was already used on this graphics connection",
            ),
        )?;
        return Ok(false);
    }

    match decoded {
        DecodedRequest::Unsupported(request) => {
            let category = if state.attachment.is_some() {
                ErrorCategory::UnsupportedRequest
            } else {
                ErrorCategory::InvalidRequest
            };
            let message = if state.attachment.is_some() {
                "graphics request type is not supported"
            } else {
                "initialize must be the first graphics request"
            };
            send_failure(
                socket,
                state,
                &request.session_id,
                &request.message_id,
                ProtocolFailure::new(category, message),
            )?;
            Ok(false)
        }
        DecodedRequest::Known(request) => handle_request(socket, registry, state, request),
    }
}

#[allow(clippy::too_many_lines)] // Keep request result/event ordering in one auditable match.
fn handle_request(
    socket: &mut WebSocket<TcpStream>,
    registry: &GraphicsSessionRegistry,
    state: &mut ConnectionState,
    request: RequestEnvelope,
) -> Result<bool, ServerError> {
    if let Request::Initialize(initialize) = &request.request {
        if state.attachment.is_some() {
            send_failure(
                socket,
                state,
                &request.session_id,
                &request.message_id,
                ProtocolFailure::new(
                    ErrorCategory::InvalidRequest,
                    "graphics connection is already initialized",
                ),
            )?;
            return Ok(false);
        }
        let attachment = match registry.attach(&request.session_id, &initialize.attach_token) {
            Ok(attachment) => attachment,
            Err(error) => {
                send_failure(
                    socket,
                    state,
                    &request.session_id,
                    &request.message_id,
                    error,
                )?;
                close(socket, CloseCode::Policy, "graphics attachment rejected");
                return Ok(true);
            }
        };
        let view = match attachment.hub().initialize() {
            Ok(view) => view,
            Err(error) => {
                send_failure(
                    socket,
                    state,
                    &request.session_id,
                    &request.message_id,
                    error.into(),
                )?;
                return Ok(true);
            }
        };
        let limits = match openmat_plot_protocol::GraphicsLimits::negotiate(
            view.limits,
            &initialize.capabilities,
        ) {
            Ok(limits) => limits,
            Err(error) => {
                send_failure(
                    socket,
                    state,
                    &request.session_id,
                    &request.message_id,
                    error.into(),
                )?;
                return Ok(true);
            }
        };
        validate_initialize_view(&view.figures)?;
        let result = InitializeResult {
            result_type: "initialize".to_owned(),
            implementation: view.implementation,
            capabilities: ServerCapabilities::from(limits),
            figures: view.figures,
        };
        result.validate().map_err(ServerError::GraphicsProtocol)?;
        send_success(
            socket,
            state,
            &request.session_id,
            &request.message_id,
            result,
            limits.max_text_frame_bytes,
        )?;
        state.limits = limits;
        state.event_cursor = view.event_cursor;
        state.attachment = Some(attachment);
        return Ok(false);
    }

    let Some(attachment) = state.attachment.as_mut() else {
        send_failure(
            socket,
            state,
            &request.session_id,
            &request.message_id,
            ProtocolFailure::new(
                ErrorCategory::InvalidRequest,
                "initialize must be the first graphics request",
            ),
        )?;
        return Ok(false);
    };
    if request.session_id != attachment.session_id() {
        send_failure(
            socket,
            state,
            &request.session_id,
            &request.message_id,
            ProtocolFailure::new(
                ErrorCategory::Unauthorized,
                "graphics request session does not match the attachment",
            ),
        )?;
        close(socket, CloseCode::Policy, "graphics session mismatch");
        return Ok(true);
    }

    let session_id = request.session_id.clone();
    let request_id = request.message_id.clone();

    match request.request {
        Request::Initialize(_) => unreachable!("initialize handled above"),
        Request::GetSnapshot(parameters) => {
            let snapshot = match attachment.hub().get_snapshot(&parameters.figure_id) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    send_hub_failure(socket, state, &session_id, &request_id, error)?;
                    return Ok(false);
                }
            };
            snapshot
                .validate_for(state.limits, state.protocol)
                .map_err(ServerError::GraphicsProtocol)?;
            let result = GetSnapshotResult {
                result_type: "getSnapshot".to_owned(),
                snapshot,
            };
            result
                .validate_for(state.limits, state.protocol)
                .map_err(ServerError::GraphicsProtocol)?;
            send_success(
                socket,
                state,
                &session_id,
                &request_id,
                result,
                state.limits.max_text_frame_bytes,
            )?;
        }
        Request::ResyncFigure(parameters) => {
            let snapshot = match attachment
                .hub()
                .resync_figure(&parameters.figure_id, parameters.known_revision)
            {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    send_hub_failure(socket, state, &session_id, &request_id, error)?;
                    return Ok(false);
                }
            };
            snapshot
                .validate_for(state.limits, state.protocol)
                .map_err(ServerError::GraphicsProtocol)?;
            let result = ResyncFigureResult {
                result_type: "resyncFigure".to_owned(),
                snapshot,
            };
            result
                .validate_for(state.limits, state.protocol)
                .map_err(ServerError::GraphicsProtocol)?;
            send_success(
                socket,
                state,
                &session_id,
                &request_id,
                result,
                state.limits.max_text_frame_bytes,
            )?;
        }
        Request::GetBuffer(parameters) => {
            send_buffer(
                socket,
                state,
                &session_id,
                &request_id,
                &parameters.buffer_id,
            )?;
        }
        Request::ReleaseBuffer(parameters) => {
            let (released, outcome) = match attachment.release_buffer(&parameters.buffer_id) {
                Ok(outcome) => outcome,
                Err(error) => {
                    send_hub_failure(socket, state, &session_id, &request_id, error)?;
                    return Ok(false);
                }
            };
            let result = ReleaseBufferResult {
                result_type: "releaseBuffer".to_owned(),
                released,
            };
            result.validate().map_err(ServerError::GraphicsProtocol)?;
            send_success(
                socket,
                state,
                &session_id,
                &request_id,
                result,
                state.limits.max_text_frame_bytes,
            )?;
            if outcome.reclaimed {
                send_event(
                    socket,
                    state,
                    Event::BufferReleased(BufferReleasedEvent {
                        buffer_id: parameters.buffer_id,
                    }),
                )?;
            }
        }
        Request::CloseFigure(parameters) => {
            let outcome = match attachment
                .hub()
                .close_figure(&parameters.figure_id, parameters.expected_revision)
            {
                Ok(outcome) => outcome,
                Err(error) => {
                    send_hub_failure(socket, state, &session_id, &request_id, error)?;
                    return Ok(false);
                }
            };
            let result = CloseFigureResult {
                result_type: "closeFigure".to_owned(),
                closed_revision: outcome.closed_revision,
            };
            result.validate().map_err(ServerError::GraphicsProtocol)?;
            send_success(
                socket,
                state,
                &session_id,
                &request_id,
                result,
                state.limits.max_text_frame_bytes,
            )?;
            send_event(
                socket,
                state,
                Event::FigureClosed(FigureClosedEvent {
                    figure_id: parameters.figure_id,
                    closed_revision: outcome.closed_revision,
                }),
            )?;
            state.event_cursor = state.event_cursor.max(outcome.event_cursor);
        }
        Request::SetAxesLimits(parameters) => {
            let outcome = match attachment.hub().set_axes_limits(
                &parameters.figure_id,
                parameters.axes_id.as_deref(),
                parameters.expected_revision,
                parameters.x_limits,
                parameters.y_limits,
            ) {
                Ok(outcome) => outcome,
                Err(error) => {
                    send_hub_failure(socket, state, &session_id, &request_id, error)?;
                    return Ok(false);
                }
            };
            if parameters.expected_revision.checked_add(1) != Some(outcome.committed_revision)
                || outcome.event_cursor <= state.event_cursor
            {
                return Err(ServerError::State(
                    "graphics hub returned an invalid semantic limits outcome",
                ));
            }
            let result = SetAxesLimitsResult {
                result_type: "setAxesLimits".to_owned(),
                committed_revision: outcome.committed_revision,
            };
            result.validate().map_err(ServerError::GraphicsProtocol)?;
            send_success(
                socket,
                state,
                &session_id,
                &request_id,
                result,
                state.limits.max_text_frame_bytes,
            )?;
            return poll_hub_events(socket, state);
        }
        Request::SetAxesCamera(parameters) => {
            let outcome = match attachment.hub().set_axes_camera(
                &parameters.figure_id,
                parameters.axes_id.as_deref(),
                parameters.expected_revision,
                parameters.view,
                parameters.camera_scale,
            ) {
                Ok(outcome) => outcome,
                Err(error) => {
                    send_hub_failure(socket, state, &session_id, &request_id, error)?;
                    return Ok(false);
                }
            };
            if parameters.expected_revision.checked_add(1) != Some(outcome.committed_revision)
                || outcome.event_cursor <= state.event_cursor
            {
                return Err(ServerError::State(
                    "graphics hub returned an invalid camera outcome",
                ));
            }
            let result = SetAxesCameraResult {
                result_type: "setAxesCamera".to_owned(),
                committed_revision: outcome.committed_revision,
            };
            result.validate().map_err(ServerError::GraphicsProtocol)?;
            send_success(
                socket,
                state,
                &session_id,
                &request_id,
                result,
                state.limits.max_text_frame_bytes,
            )?;
            return poll_hub_events(socket, state);
        }
        Request::Shutdown(_) => {
            let result = ShutdownResult {
                result_type: "shutdown".to_owned(),
            };
            result.validate().map_err(ServerError::GraphicsProtocol)?;
            send_success(
                socket,
                state,
                &session_id,
                &request_id,
                result,
                state.limits.max_text_frame_bytes,
            )?;
            close(socket, CloseCode::Normal, "graphics connection detached");
            return Ok(true);
        }
    }
    Ok(false)
}

#[allow(clippy::too_many_lines)] // Keep response ordering and lease rollback together.
fn send_buffer(
    socket: &mut WebSocket<TcpStream>,
    state: &mut ConnectionState,
    session_id: &str,
    request_id: &str,
    buffer_id: &str,
) -> Result<(), ServerError> {
    let (buffer, newly_leased) = state
        .attachment
        .as_mut()
        .expect("initialized connection has attachment")
        .get_buffer(buffer_id)
        .map_err(hub_as_server_error)?;
    if buffer.buffer_id != buffer_id {
        state
            .attachment
            .as_mut()
            .expect("attachment remains present")
            .cancel_new_lease(buffer_id, newly_leased);
        return Err(ServerError::State(
            "graphics hub returned a mismatched buffer identifier",
        ));
    }
    let total_bytes = u64::try_from(buffer.bytes.len()).map_err(|_| {
        ServerError::GraphicsProtocol(ValidationError::new(
            ErrorCategory::PayloadLimit,
            "graphics buffer length does not fit u64",
        ))
    })?;
    if total_bytes > state.limits.max_buffer_bytes {
        state
            .attachment
            .as_mut()
            .expect("attachment remains present")
            .cancel_new_lease(buffer_id, newly_leased);
        send_failure(
            socket,
            state,
            session_id,
            request_id,
            ProtocolFailure::new(
                ErrorCategory::PayloadLimit,
                "graphics buffer exceeds the negotiated limit",
            ),
        )?;
        return Ok(());
    }

    let transfer_id = state.next_transfer_id();
    let (chunk_bytes, chunk_count) = if total_bytes == 0 {
        (0, 0)
    } else {
        let capacity = state
            .limits
            .max_binary_frame_bytes
            .saturating_sub(
                u64::try_from(
                    openmat_plot_protocol::BINARY_PREFIX_LEN
                        + openmat_plot_protocol::BINARY_HEADER_LIMIT,
                )
                .expect("binary framing constants fit u64"),
            )
            .min(MAX_CHUNK_PAYLOAD_BYTES);
        if capacity == 0 {
            state
                .attachment
                .as_mut()
                .expect("attachment remains present")
                .cancel_new_lease(buffer_id, newly_leased);
            send_failure(
                socket,
                state,
                session_id,
                request_id,
                ProtocolFailure::new(
                    ErrorCategory::PayloadLimit,
                    "negotiated binary limit cannot fit an OMGP chunk",
                ),
            )?;
            return Ok(());
        }
        let chunk_bytes = total_bytes.min(capacity);
        (chunk_bytes, total_bytes.div_ceil(chunk_bytes))
    };
    let result = BufferTransfer {
        result_type: "getBuffer".to_owned(),
        transfer_id: transfer_id.clone(),
        buffer_id: buffer_id.to_owned(),
        total_bytes,
        chunk_bytes,
        chunk_count,
    };
    if let Err(error) = result.validate(state.limits) {
        state
            .attachment
            .as_mut()
            .expect("attachment remains present")
            .cancel_new_lease(buffer_id, newly_leased);
        return Err(ServerError::GraphicsProtocol(error));
    }
    if let Err(error) = send_success(
        socket,
        state,
        session_id,
        request_id,
        result,
        state.limits.max_text_frame_bytes,
    ) {
        state
            .attachment
            .as_mut()
            .expect("attachment remains present")
            .cancel_new_lease(buffer_id, newly_leased);
        return Err(error);
    }

    let chunk_size = usize::try_from(chunk_bytes).map_err(|_| {
        ServerError::GraphicsProtocol(ValidationError::new(
            ErrorCategory::PayloadLimit,
            "graphics chunk length does not fit usize",
        ))
    })?;
    if chunk_size == 0 {
        return Ok(());
    }
    for (index, payload) in buffer.bytes.chunks(chunk_size).enumerate() {
        let offset = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(chunk_bytes))
            .ok_or_else(|| {
                ServerError::GraphicsProtocol(ValidationError::new(
                    ErrorCategory::InvalidBinaryFrame,
                    "graphics transfer offset overflowed",
                ))
            })?;
        let payload_len = u64::try_from(payload.len()).map_err(|_| {
            ServerError::GraphicsProtocol(ValidationError::new(
                ErrorCategory::InvalidBinaryFrame,
                "graphics transfer payload length does not fit u64",
            ))
        })?;
        let final_chunk = offset.checked_add(payload_len) == Some(total_bytes);
        let frame = encode_binary_frame(
            &BufferChunkHeader::new(&transfer_id, buffer_id, offset, total_bytes, final_chunk),
            payload,
            state.limits.max_binary_frame_bytes,
        )
        .map_err(ServerError::GraphicsProtocol)?;
        if let Err(error) = socket.send(Message::binary(frame)) {
            state
                .attachment
                .as_mut()
                .expect("attachment remains present")
                .cancel_new_lease(buffer_id, newly_leased);
            return Err(ServerError::WebSocket(error));
        }
    }
    Ok(())
}

fn poll_hub_events(
    socket: &mut WebSocket<TcpStream>,
    state: &mut ConnectionState,
) -> Result<bool, ServerError> {
    let Some(attachment) = state.attachment.as_ref() else {
        return Ok(false);
    };
    if !attachment.is_live() {
        send_event(socket, state, Event::SessionClosed(SessionClosedEvent {}))?;
        close(socket, CloseCode::Normal, "graphics session closed");
        return Ok(true);
    }
    let mut events = match attachment.hub().events_after(state.event_cursor) {
        Ok(events) => events,
        Err(error) if error.category() == ErrorCategory::SessionClosed => {
            send_event(socket, state, Event::SessionClosed(SessionClosedEvent {}))?;
            close(socket, CloseCode::Normal, "graphics session closed");
            return Ok(true);
        }
        Err(error) => return Err(hub_as_server_error(error)),
    };
    events.sort_by_key(|event| event.sequence);
    for event in events {
        if event.sequence <= state.event_cursor {
            return Err(ServerError::State(
                "graphics hub returned a non-increasing event sequence",
            ));
        }
        send_event(socket, state, event.event)?;
        state.event_cursor = event.sequence;
    }
    Ok(false)
}

fn send_hub_failure(
    socket: &mut WebSocket<TcpStream>,
    state: &mut ConnectionState,
    session_id: &str,
    request_id: &str,
    error: GraphicsHubError,
) -> Result<(), ServerError> {
    send_failure(socket, state, session_id, request_id, error.into())
}

fn send_success<T: serde::Serialize>(
    socket: &mut WebSocket<TcpStream>,
    state: &mut ConnectionState,
    session_id: &str,
    reply_to: &str,
    result: T,
    max_text_bytes: u64,
) -> Result<(), ServerError> {
    let response = ResponseEnvelope::success_for(
        state.protocol,
        session_id,
        state.next_message_id(),
        reply_to,
        result,
    );
    let text = encode_response(&response, max_text_bytes).map_err(ServerError::GraphicsProtocol)?;
    socket.send(Message::text(text))?;
    Ok(())
}

fn send_failure(
    socket: &mut WebSocket<TcpStream>,
    state: &mut ConnectionState,
    session_id: &str,
    reply_to: &str,
    error: ProtocolFailure,
) -> Result<(), ServerError> {
    let response = ResponseEnvelope::<Value>::failure_for(
        state.protocol,
        session_id,
        state.next_message_id(),
        reply_to,
        error,
    );
    let text = encode_response(&response, state.limits.max_text_frame_bytes)
        .map_err(ServerError::GraphicsProtocol)?;
    socket.send(Message::text(text))?;
    Ok(())
}

fn send_event(
    socket: &mut WebSocket<TcpStream>,
    state: &mut ConnectionState,
    event: Event,
) -> Result<(), ServerError> {
    let session_id = state
        .attachment
        .as_ref()
        .map_or("graphics-session-closed", GraphicsAttachment::session_id)
        .to_owned();
    let envelope =
        EventEnvelope::new_for(state.protocol, session_id, state.next_message_id(), event);
    let text = encode_event(&envelope, state.limits.max_text_frame_bytes, state.limits)
        .map_err(ServerError::GraphicsProtocol)?;
    socket.send(Message::text(text))?;
    Ok(())
}

fn validate_initialize_view(
    figures: &[openmat_plot_protocol::FigureSummary],
) -> Result<(), ServerError> {
    let mut ids = HashSet::with_capacity(figures.len());
    for figure in figures {
        if figure.figure_id.is_empty()
            || figure.revision == 0
            || figure.revision > openmat_plot_protocol::MAX_SAFE_INTEGER
            || !ids.insert(figure.figure_id.as_str())
        {
            return Err(ServerError::State(
                "graphics hub returned an invalid initialize Figure set",
            ));
        }
    }
    Ok(())
}

fn correlation(text: &str) -> Option<(String, String)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let session_id = value.get("sessionId")?.as_str()?.to_owned();
    let message_id = value.get("messageId")?.as_str()?.to_owned();
    if session_id.is_empty() || message_id.is_empty() {
        None
    } else {
        Some((session_id, message_id))
    }
}

fn hub_as_server_error(error: GraphicsHubError) -> ServerError {
    ServerError::GraphicsHub(error)
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

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use openmat_kernel::{EngineError, RuntimeEngine};
    use openmat_plot_protocol::{
        Axes2DProperties, AxesLimitMode, ClientCapabilities, ClientInfo, CloseFigureRequest,
        DeltaOperation, FigureDelta, FigureNextPlot, FigureProperties, FigureSnapshot,
        FigureSummary, GetBufferRequest, GraphicsLimits, GraphicsObject, GraphicsProtocol,
        NextPlot, Nullable, ObjectFields, ReleaseBufferRequest, RenderBackend, Request,
        RequestEnvelope, Rgba, Scale, SetAxesCameraRequest, SetAxesLimitsRequest, ShutdownRequest,
        decode_binary_frame,
    };
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{WebSocket, connect};

    use crate::graphics::{
        BufferLeaseRelease, CloseFigureOutcome, GraphicsBuffer, GraphicsHubInitialization,
        GraphicsSessionHub, SequencedGraphicsEvent, SetAxesCameraOutcome, SetAxesLimitsOutcome,
    };

    use super::*;

    type ClientSocket = WebSocket<MaybeTlsStream<TcpStream>>;

    struct FixtureHub {
        leases: AtomicUsize,
        releases: AtomicUsize,
        buffer_bytes: Arc<[u8]>,
        snapshot: Mutex<FigureSnapshot>,
        events: Mutex<Vec<SequencedGraphicsEvent>>,
    }

    impl FixtureHub {
        #[allow(clippy::too_many_lines)]
        fn new() -> Self {
            Self {
                leases: AtomicUsize::new(0),
                releases: AtomicUsize::new(0),
                buffer_bytes: Arc::from(*b"abcde"),
                snapshot: Mutex::new(FigureSnapshot {
                    snapshot_type: "figureSnapshot".to_owned(),
                    figure_id: "figure-1".to_owned(),
                    revision: 1,
                    root_id: "figure-object".to_owned(),
                    objects: vec![
                        GraphicsObject::Figure {
                            fields: ObjectFields {
                                id: "figure-object".to_owned(),
                                generation: 1,
                                object_revision: 1,
                                parent_id: None,
                                children: vec!["axes-1".to_owned()],
                            },
                            properties: FigureProperties {
                                number: 1,
                                name_code_units: vec![],
                                number_title: true,
                                visible: true,
                                background_rgba: Rgba([1.0, 1.0, 1.0, 1.0]),
                                initial_logical_size_css_pixels: [640.0, 480.0],
                                position_css_pixels: [100.0, 100.0, 640.0, 480.0],
                                next_plot: FigureNextPlot::Add,
                            },
                        },
                        GraphicsObject::Axes2d {
                            fields: ObjectFields {
                                id: "axes-1".to_owned(),
                                generation: 1,
                                object_revision: 1,
                                parent_id: Some("figure-object".to_owned()),
                                children: vec![],
                            },
                            properties: Axes2DProperties {
                                coordinate_system:
                                    openmat_plot_protocol::AxesCoordinateSystem::Cartesian,
                                position_normalized: [0.1, 0.1, 0.8, 0.8],
                                background_rgba: openmat_plot_protocol::Nullable(Some(Rgba([
                                    1.0, 1.0, 1.0, 1.0,
                                ]))),
                                theta_axis_units: openmat_plot_protocol::ThetaAxisUnits::Degrees,
                                theta_direction:
                                    openmat_plot_protocol::ThetaDirection::Counterclockwise,
                                theta_zero_location:
                                    openmat_plot_protocol::ThetaZeroLocation::Right,
                                r_axis_location: 80.0,
                                x_scale: Scale::Linear,
                                y_scale: Scale::Linear,
                                z_scale: Scale::Linear,
                                x_direction: openmat_plot_protocol::AxisDirection::Normal,
                                y_direction: openmat_plot_protocol::AxisDirection::Normal,
                                z_direction: openmat_plot_protocol::AxisDirection::Normal,
                                visible: true,
                                x_limits: [0.0, 1.0],
                                y_limits: [0.0, 1.0],
                                z_limits: [0.0, 1.0],
                                x_limits_mode: AxesLimitMode::Auto,
                                y_limits_mode: AxesLimitMode::Auto,
                                z_limits_mode: AxesLimitMode::Auto,
                                next_plot: NextPlot::Replace,
                                grid_x: false,
                                grid_y: false,
                                grid_z: false,
                                minor_grid_x: false,
                                minor_grid_y: false,
                                minor_grid_z: false,
                                color_order_index: 1,
                                x_tick: Vec::new(),
                                y_tick: Vec::new(),
                                z_tick: Vec::new(),
                                x_tick_label_code_units: Vec::new(),
                                y_tick_label_code_units: Vec::new(),
                                z_tick_label_code_units: Vec::new(),
                                x_tick_mode: openmat_plot_protocol::AxesTickMode::Auto,
                                y_tick_mode: openmat_plot_protocol::AxesTickMode::Auto,
                                z_tick_mode: openmat_plot_protocol::AxesTickMode::Auto,
                                x_tick_label_mode: openmat_plot_protocol::AxesTickMode::Auto,
                                y_tick_label_mode: openmat_plot_protocol::AxesTickMode::Auto,
                                z_tick_label_mode: openmat_plot_protocol::AxesTickMode::Auto,
                                view: [0.0, 90.0],
                                camera_scale: 1.0,
                                projection: openmat_plot_protocol::Projection::Orthographic,
                                data_aspect_ratio: [1.0, 1.0, 1.0],
                                data_aspect_ratio_mode: AxesLimitMode::Auto,
                                plot_box_aspect_ratio: [1.0, 1.0, 1.0],
                                plot_box_aspect_ratio_mode: AxesLimitMode::Auto,
                                c_limits: [0.0, 1.0],
                                c_limits_mode: AxesLimitMode::Auto,
                                colormap: None,
                                colorbar_visible: false,
                                box_enabled: false,
                                font_size_css_px: 10.0,
                                font_family_code_units: Vec::new(),
                                tick_direction: openmat_plot_protocol::TickDirection::In,
                                line_width_css_px: 2.0 / 3.0,
                                tick_label_interpreter: openmat_plot_protocol::TextInterpreter::Tex,
                                title_id: Nullable(None),
                                x_label_id: Nullable(None),
                                y_label_id: Nullable(None),
                                z_label_id: Nullable(None),
                            },
                        },
                    ],
                    referenced_buffers: vec![],
                }),
                events: Mutex::new(vec![]),
            }
        }

        fn with_buffer(buffer_bytes: Vec<u8>) -> Self {
            Self {
                buffer_bytes: buffer_bytes.into(),
                ..Self::new()
            }
        }
    }

    impl GraphicsSessionHub for FixtureHub {
        fn initialize(&self) -> Result<GraphicsHubInitialization, GraphicsHubError> {
            Ok(GraphicsHubInitialization {
                implementation: openmat_plot_protocol::ImplementationInfo {
                    name: "openmat-server-fixture".to_owned(),
                    version: "1".to_owned(),
                },
                limits: GraphicsLimits::default(),
                figures: vec![FigureSummary {
                    figure_id: "figure-1".to_owned(),
                    revision: self.snapshot.lock().unwrap().revision,
                }],
                event_cursor: 0,
            })
        }

        fn get_snapshot(&self, figure_id: &str) -> Result<FigureSnapshot, GraphicsHubError> {
            if figure_id == "figure-1" {
                Ok(self.snapshot.lock().unwrap().clone())
            } else {
                Err(GraphicsHubError::new(
                    ErrorCategory::UnknownFigure,
                    "unknown fixture Figure",
                ))
            }
        }

        fn get_buffer(
            &self,
            buffer_id: &str,
            acquire_lease: bool,
        ) -> Result<GraphicsBuffer, GraphicsHubError> {
            if buffer_id != "buffer-1" {
                return Err(GraphicsHubError::new(
                    ErrorCategory::UnknownBuffer,
                    "unknown fixture buffer",
                ));
            }
            if acquire_lease {
                self.leases.fetch_add(1, Ordering::Relaxed);
            }
            Ok(GraphicsBuffer {
                buffer_id: buffer_id.to_owned(),
                bytes: Arc::clone(&self.buffer_bytes),
            })
        }

        fn release_buffer_lease(
            &self,
            _buffer_id: &str,
        ) -> Result<BufferLeaseRelease, GraphicsHubError> {
            self.releases.fetch_add(1, Ordering::Relaxed);
            Ok(BufferLeaseRelease { reclaimed: false })
        }

        fn close_figure(
            &self,
            figure_id: &str,
            expected_revision: u64,
        ) -> Result<CloseFigureOutcome, GraphicsHubError> {
            if figure_id != "figure-1" {
                return Err(GraphicsHubError::new(
                    ErrorCategory::UnknownFigure,
                    "unknown fixture Figure",
                ));
            }
            if expected_revision != 1 {
                return Err(GraphicsHubError::new(
                    ErrorCategory::RevisionConflict,
                    "fixture revision conflict",
                ));
            }
            Ok(CloseFigureOutcome {
                closed_revision: 2,
                event_cursor: 1,
            })
        }

        fn resync_figure(
            &self,
            figure_id: &str,
            _known_revision: u64,
        ) -> Result<FigureSnapshot, GraphicsHubError> {
            self.get_snapshot(figure_id)
        }

        fn set_axes_limits(
            &self,
            figure_id: &str,
            _axes_id: Option<&str>,
            expected_revision: u64,
            x_limits: [f64; 2],
            y_limits: [f64; 2],
        ) -> Result<SetAxesLimitsOutcome, GraphicsHubError> {
            if figure_id != "figure-1" {
                return Err(GraphicsHubError::new(
                    ErrorCategory::UnknownFigure,
                    "unknown fixture Figure",
                ));
            }
            let mut snapshot = self.snapshot.lock().unwrap();
            if snapshot.revision != expected_revision {
                return Err(GraphicsHubError::new(
                    ErrorCategory::RevisionConflict,
                    "fixture revision conflict",
                ));
            }
            let axes = snapshot
                .objects
                .iter_mut()
                .find(|object| matches!(object, GraphicsObject::Axes2d { .. }))
                .expect("fixture contains Axes");
            let GraphicsObject::Axes2d { fields, properties } = axes else {
                unreachable!("matched Axes")
            };
            fields.object_revision += 1;
            properties.x_limits = x_limits;
            properties.y_limits = y_limits;
            properties.x_limits_mode = AxesLimitMode::Manual;
            properties.y_limits_mode = AxesLimitMode::Manual;
            let operation = DeltaOperation::UpsertObject {
                object: axes.clone(),
            };
            snapshot.revision += 1;
            let committed_revision = snapshot.revision;
            drop(snapshot);
            let mut events = self.events.lock().unwrap();
            let sequence = u64::try_from(events.len()).unwrap() + 1;
            let event = SequencedGraphicsEvent {
                sequence,
                event: Event::FigureDelta(FigureDelta {
                    figure_id: figure_id.to_owned(),
                    base_revision: expected_revision,
                    revision: committed_revision,
                    operations: vec![operation],
                    added_buffers: vec![],
                    released_buffer_ids: vec![],
                }),
            };
            events.push(event);
            Ok(SetAxesLimitsOutcome {
                committed_revision,
                event_cursor: sequence,
            })
        }

        fn set_axes_camera(
            &self,
            figure_id: &str,
            _axes_id: Option<&str>,
            expected_revision: u64,
            view: [f64; 2],
            camera_scale: f64,
        ) -> Result<SetAxesCameraOutcome, GraphicsHubError> {
            if figure_id != "figure-1" {
                return Err(GraphicsHubError::new(
                    ErrorCategory::UnknownFigure,
                    "unknown fixture Figure",
                ));
            }
            let mut snapshot = self.snapshot.lock().unwrap();
            if snapshot.revision != expected_revision {
                return Err(GraphicsHubError::new(
                    ErrorCategory::RevisionConflict,
                    "fixture revision conflict",
                ));
            }
            let axes = snapshot
                .objects
                .iter_mut()
                .find(|object| matches!(object, GraphicsObject::Axes2d { .. }))
                .expect("fixture contains Axes");
            let GraphicsObject::Axes2d { fields, properties } = axes else {
                unreachable!("matched Axes")
            };
            fields.object_revision += 1;
            properties.view = view;
            properties.camera_scale = camera_scale;
            let operation = DeltaOperation::UpsertObject {
                object: axes.clone(),
            };
            snapshot.revision += 1;
            let committed_revision = snapshot.revision;
            drop(snapshot);
            let mut events = self.events.lock().unwrap();
            let sequence = u64::try_from(events.len()).unwrap() + 1;
            events.push(SequencedGraphicsEvent {
                sequence,
                event: Event::FigureDelta(FigureDelta {
                    figure_id: figure_id.to_owned(),
                    base_revision: expected_revision,
                    revision: committed_revision,
                    operations: vec![operation],
                    added_buffers: vec![],
                    released_buffer_ids: vec![],
                }),
            });
            Ok(SetAxesCameraOutcome {
                committed_revision,
                event_cursor: sequence,
            })
        }

        fn events_after(
            &self,
            cursor: u64,
        ) -> Result<Vec<SequencedGraphicsEvent>, GraphicsHubError> {
            Ok(self
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.sequence > cursor)
                .cloned()
                .collect())
        }
    }

    fn read_text(socket: &mut ClientSocket) -> Value {
        let Message::Text(text) = socket.read().unwrap() else {
            panic!("expected graphics text message");
        };
        serde_json::from_str(text.as_str()).unwrap()
    }

    fn send_request(socket: &mut ClientSocket, request: &RequestEnvelope) {
        socket
            .send(Message::text(serde_json::to_string(request).unwrap()))
            .unwrap();
    }

    #[test]
    fn correlation_never_extracts_attach_token() {
        let text = serde_json::json!({
            "sessionId": "session-1",
            "messageId": "request-1",
            "request": { "type": "initialize", "attachToken": "secret" }
        })
        .to_string();
        assert_eq!(
            correlation(&text),
            Some(("session-1".to_owned(), "request-1".to_owned()))
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One real socket proves required graphics ordering/lifecycle.
    fn real_tcp_graphics_attach_snapshot_buffer_close_and_disconnect_cleanup() {
        const LARGE_BUFFER_BYTES: usize = 4 * 1024 * 1024;
        let registry = GraphicsSessionRegistry::new();
        let hub = Arc::new(FixtureHub::with_buffer(vec![0x5a; LARGE_BUFFER_BYTES]));
        let registration = registry.register_session("session-1", hub.clone()).unwrap();
        let discovery = registration.discovery("figure-1", 1).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_registry = registry.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            crate::websocket::serve_connection_with_graphics(
                stream,
                || -> Result<RuntimeEngine, EngineError> {
                    panic!("graphics endpoint must not construct the kernel engine")
                },
                None,
                &server_registry,
            )
            .unwrap();
        });

        let (mut socket, _) = connect(format!("ws://{address}/graphics/v1")).unwrap();
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
        }
        send_request(
            &mut socket,
            &RequestEnvelope::new(
                "session-1",
                "initialize-1",
                Request::Initialize(openmat_plot_protocol::InitializeRequest {
                    attach_token: discovery.attach_token,
                    client: ClientInfo {
                        name: "openmat-web-test".to_owned(),
                        version: "1".to_owned(),
                    },
                    capabilities: ClientCapabilities {
                        render_backend: RenderBackend::Webgpu,
                        max_text_frame_bytes: openmat_plot_protocol::MAX_TEXT_FRAME_BYTES,
                        max_binary_frame_bytes: openmat_plot_protocol::MAX_BINARY_FRAME_BYTES,
                        max_buffer_bytes: openmat_plot_protocol::MAX_BUFFER_BYTES,
                        max_resident_bytes: openmat_plot_protocol::MAX_RESIDENT_BYTES,
                    },
                }),
            ),
        );
        let initialized = read_text(&mut socket);
        assert_eq!(initialized["ok"], true);
        assert_eq!(initialized["result"]["figures"][0]["revision"], 1);

        send_request(
            &mut socket,
            &RequestEnvelope::new(
                "session-1",
                "snapshot-1",
                Request::GetSnapshot(openmat_plot_protocol::GetSnapshotRequest {
                    figure_id: "figure-1".to_owned(),
                }),
            ),
        );
        let snapshot = read_text(&mut socket);
        assert_eq!(snapshot["result"]["type"], "getSnapshot");
        assert_eq!(snapshot["result"]["snapshot"]["revision"], 1);

        for message_id in ["buffer-1", "buffer-repeat"] {
            send_request(
                &mut socket,
                &RequestEnvelope::new(
                    "session-1",
                    message_id,
                    Request::GetBuffer(GetBufferRequest {
                        buffer_id: "buffer-1".to_owned(),
                    }),
                ),
            );
            let transfer = read_text(&mut socket);
            assert_eq!(transfer["result"]["type"], "getBuffer");
            let chunk_count = transfer["result"]["chunkCount"].as_u64().unwrap();
            let mut received = 0;
            for _ in 0..chunk_count {
                let Message::Binary(frame) = socket.read().unwrap() else {
                    panic!("text response must precede binary payload");
                };
                let decoded = decode_binary_frame(
                    frame.as_ref(),
                    openmat_plot_protocol::MAX_BINARY_FRAME_BYTES,
                )
                .unwrap();
                assert!(decoded.payload.iter().all(|byte| *byte == 0x5a));
                received += decoded.payload.len();
            }
            assert_eq!(received, LARGE_BUFFER_BYTES);
        }
        assert_eq!(hub.leases.load(Ordering::Relaxed), 1);

        for (message_id, expected) in [("release-1", true), ("release-2", false)] {
            send_request(
                &mut socket,
                &RequestEnvelope::new(
                    "session-1",
                    message_id,
                    Request::ReleaseBuffer(ReleaseBufferRequest {
                        buffer_id: "buffer-1".to_owned(),
                    }),
                ),
            );
            assert_eq!(read_text(&mut socket)["result"]["released"], expected);
        }

        send_request(
            &mut socket,
            &RequestEnvelope::new(
                "session-1",
                "close-1",
                Request::CloseFigure(CloseFigureRequest {
                    figure_id: "figure-1".to_owned(),
                    expected_revision: 1,
                }),
            ),
        );
        assert_eq!(read_text(&mut socket)["result"]["type"], "closeFigure");
        assert_eq!(read_text(&mut socket)["event"]["type"], "figureClosed");

        send_request(
            &mut socket,
            &RequestEnvelope::new(
                "session-1",
                "shutdown-1",
                Request::Shutdown(ShutdownRequest {}),
            ),
        );
        assert_eq!(read_text(&mut socket)["result"]["type"], "shutdown");
        drop(socket);
        server.join().unwrap();
        assert_eq!(hub.releases.load(Ordering::Relaxed), 1);
        registry.unregister_session(&registration).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One real v2 socket proves commit/response/delta/conflict ordering.
    fn real_tcp_graphics_v2_commits_atomic_limits_and_rejects_stale_revision() {
        let registry = GraphicsSessionRegistry::new();
        let hub = Arc::new(FixtureHub::new());
        let registration = registry.register_session("session-1", hub).unwrap();
        let discovery = registration.discovery("figure-1", 1).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_registry = registry.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            crate::websocket::serve_connection_with_graphics(
                stream,
                || -> Result<RuntimeEngine, EngineError> {
                    panic!("graphics endpoint must not construct the kernel engine")
                },
                None,
                &server_registry,
            )
            .unwrap();
        });

        let (mut socket, _) = connect(format!("ws://{address}/graphics/v2")).unwrap();
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
        }
        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V2,
                "session-1",
                "initialize-v2",
                Request::Initialize(openmat_plot_protocol::InitializeRequest {
                    attach_token: discovery.attach_token,
                    client: ClientInfo {
                        name: "openmat-web-test".to_owned(),
                        version: "1".to_owned(),
                    },
                    capabilities: ClientCapabilities {
                        render_backend: RenderBackend::Webgpu,
                        max_text_frame_bytes: openmat_plot_protocol::MAX_TEXT_FRAME_BYTES,
                        max_binary_frame_bytes: openmat_plot_protocol::MAX_BINARY_FRAME_BYTES,
                        max_buffer_bytes: openmat_plot_protocol::MAX_BUFFER_BYTES,
                        max_resident_bytes: openmat_plot_protocol::MAX_RESIDENT_BYTES,
                    },
                }),
            ),
        );
        let initialized = read_text(&mut socket);
        assert_eq!(initialized["protocol"], openmat_plot_protocol::PROTOCOL_V2);
        assert_eq!(initialized["ok"], true);

        let limits_request = RequestEnvelope::new_for(
            GraphicsProtocol::V2,
            "session-1",
            "limits-1",
            Request::SetAxesLimits(SetAxesLimitsRequest {
                figure_id: "figure-1".to_owned(),
                axes_id: None,
                expected_revision: 1,
                x_limits: [-2.0, 8.0],
                y_limits: [0.25, 16.0],
            }),
        );
        send_request(&mut socket, &limits_request);
        let committed = read_text(&mut socket);
        assert_eq!(committed["replyTo"], "limits-1");
        assert_eq!(committed["result"]["type"], "setAxesLimits");
        assert_eq!(committed["result"]["committedRevision"], 2);
        let delta = read_text(&mut socket);
        assert_eq!(delta["event"]["type"], "figureDelta");
        assert_eq!(delta["event"]["baseRevision"], 1);
        assert_eq!(delta["event"]["revision"], 2);
        assert_eq!(
            delta["event"]["operations"][0]["object"]["properties"]["xLimits"],
            serde_json::json!([-2.0, 8.0])
        );
        assert_eq!(
            delta["event"]["operations"][0]["object"]["properties"]["yLimitsMode"],
            "manual"
        );

        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V2,
                "session-1",
                "camera-1",
                Request::SetAxesCamera(SetAxesCameraRequest {
                    figure_id: "figure-1".to_owned(),
                    axes_id: None,
                    expected_revision: 2,
                    view: [55.0, 24.0],
                    camera_scale: 0.8,
                }),
            ),
        );
        let camera_committed = read_text(&mut socket);
        assert_eq!(camera_committed["replyTo"], "camera-1");
        assert_eq!(camera_committed["result"]["type"], "setAxesCamera");
        assert_eq!(camera_committed["result"]["committedRevision"], 3);
        let camera_delta = read_text(&mut socket);
        assert_eq!(camera_delta["event"]["baseRevision"], 2);
        assert_eq!(camera_delta["event"]["revision"], 3);
        assert_eq!(
            camera_delta["event"]["operations"][0]["object"]["properties"]["view"],
            serde_json::json!([55.0, 24.0])
        );
        assert_eq!(
            camera_delta["event"]["operations"][0]["object"]["properties"]["cameraScale"],
            0.8
        );

        send_request(&mut socket, &limits_request);
        let duplicate_id = read_text(&mut socket);
        assert_eq!(duplicate_id["error"]["category"], "graphics.invalidRequest");
        let stale = RequestEnvelope::new_for(
            GraphicsProtocol::V2,
            "session-1",
            "limits-stale",
            Request::SetAxesLimits(SetAxesLimitsRequest {
                figure_id: "figure-1".to_owned(),
                axes_id: None,
                expected_revision: 1,
                x_limits: [0.0, 4.0],
                y_limits: [0.0, 4.0],
            }),
        );
        send_request(&mut socket, &stale);
        let conflict = read_text(&mut socket);
        assert_eq!(conflict["error"]["category"], "graphics.revisionConflict");

        socket
            .send(Message::text(
                serde_json::json!({
                    "protocol": openmat_plot_protocol::PROTOCOL_V2,
                    "sessionId": "session-1",
                    "messageId": "unknown-v2",
                    "kind": "request",
                    "request": { "type": "rotateView" }
                })
                .to_string(),
            ))
            .unwrap();
        let unsupported = read_text(&mut socket);
        assert_eq!(
            unsupported["error"]["category"],
            "graphics.unsupportedRequest"
        );

        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V2,
                "session-1",
                "shutdown-v2",
                Request::Shutdown(ShutdownRequest {}),
            ),
        );
        assert_eq!(read_text(&mut socket)["result"]["type"], "shutdown");
        drop(socket);
        server.join().unwrap();
        registry.unregister_session(&registration).unwrap();
    }

    #[test]
    fn real_tcp_graphics_v3_routes_an_explicit_axes_target() {
        let registry = GraphicsSessionRegistry::new();
        let hub = Arc::new(FixtureHub::new());
        let registration = registry.register_session("session-1", hub).unwrap();
        let discovery = registration.discovery("figure-1", 1).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_registry = registry.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            crate::websocket::serve_connection_with_graphics(
                stream,
                || -> Result<RuntimeEngine, EngineError> {
                    panic!("graphics endpoint must not construct the kernel engine")
                },
                None,
                &server_registry,
            )
            .unwrap();
        });

        let (mut socket, _) = connect(format!("ws://{address}/graphics/v3")).unwrap();
        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V3,
                "session-1",
                "initialize-v3",
                Request::Initialize(openmat_plot_protocol::InitializeRequest {
                    attach_token: discovery.attach_token,
                    client: ClientInfo {
                        name: "openmat-web-test".to_owned(),
                        version: "1".to_owned(),
                    },
                    capabilities: ClientCapabilities {
                        render_backend: RenderBackend::Webgpu,
                        max_text_frame_bytes: openmat_plot_protocol::MAX_TEXT_FRAME_BYTES,
                        max_binary_frame_bytes: openmat_plot_protocol::MAX_BINARY_FRAME_BYTES,
                        max_buffer_bytes: openmat_plot_protocol::MAX_BUFFER_BYTES,
                        max_resident_bytes: openmat_plot_protocol::MAX_RESIDENT_BYTES,
                    },
                }),
            ),
        );
        assert_eq!(
            read_text(&mut socket)["protocol"],
            openmat_plot_protocol::PROTOCOL_V3
        );

        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V3,
                "session-1",
                "limits-v3",
                Request::SetAxesLimits(SetAxesLimitsRequest {
                    figure_id: "figure-1".to_owned(),
                    axes_id: Some("axes-1".to_owned()),
                    expected_revision: 1,
                    x_limits: [-2.0, 8.0],
                    y_limits: [0.25, 16.0],
                }),
            ),
        );
        assert_eq!(read_text(&mut socket)["result"]["committedRevision"], 2);
        assert_eq!(read_text(&mut socket)["event"]["revision"], 2);

        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V3,
                "session-1",
                "shutdown-v3",
                Request::Shutdown(ShutdownRequest {}),
            ),
        );
        assert_eq!(read_text(&mut socket)["result"]["type"], "shutdown");
        drop(socket);
        server.join().unwrap();
        registry.unregister_session(&registration).unwrap();
    }

    #[test]
    fn real_tcp_graphics_v4_routes_an_explicit_axes_target() {
        let registry = GraphicsSessionRegistry::new();
        let hub = Arc::new(FixtureHub::new());
        let registration = registry.register_session("session-1", hub).unwrap();
        let discovery = registration.discovery("figure-1", 1).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_registry = registry.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            crate::websocket::serve_connection_with_graphics(
                stream,
                || -> Result<RuntimeEngine, EngineError> {
                    panic!("graphics endpoint must not construct the kernel engine")
                },
                None,
                &server_registry,
            )
            .unwrap();
        });

        let (mut socket, _) = connect(format!("ws://{address}/graphics/v4")).unwrap();
        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V4,
                "session-1",
                "initialize-v4",
                Request::Initialize(openmat_plot_protocol::InitializeRequest {
                    attach_token: discovery.attach_token,
                    client: ClientInfo {
                        name: "openmat-web-test".to_owned(),
                        version: "1".to_owned(),
                    },
                    capabilities: ClientCapabilities {
                        render_backend: RenderBackend::Webgpu,
                        max_text_frame_bytes: openmat_plot_protocol::MAX_TEXT_FRAME_BYTES,
                        max_binary_frame_bytes: openmat_plot_protocol::MAX_BINARY_FRAME_BYTES,
                        max_buffer_bytes: openmat_plot_protocol::MAX_BUFFER_BYTES,
                        max_resident_bytes: openmat_plot_protocol::MAX_RESIDENT_BYTES,
                    },
                }),
            ),
        );
        assert_eq!(
            read_text(&mut socket)["protocol"],
            openmat_plot_protocol::PROTOCOL_V4
        );

        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V4,
                "session-1",
                "limits-v4",
                Request::SetAxesLimits(SetAxesLimitsRequest {
                    figure_id: "figure-1".to_owned(),
                    axes_id: Some("axes-1".to_owned()),
                    expected_revision: 1,
                    x_limits: [-2.0, 8.0],
                    y_limits: [0.25, 16.0],
                }),
            ),
        );
        assert_eq!(read_text(&mut socket)["result"]["committedRevision"], 2);
        assert_eq!(read_text(&mut socket)["event"]["revision"], 2);

        send_request(
            &mut socket,
            &RequestEnvelope::new_for(
                GraphicsProtocol::V4,
                "session-1",
                "shutdown-v4",
                Request::Shutdown(ShutdownRequest {}),
            ),
        );
        assert_eq!(read_text(&mut socket)["result"]["type"], "shutdown");
        drop(socket);
        server.join().unwrap();
        registry.unregister_session(&registration).unwrap();
    }
}

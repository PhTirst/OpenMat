//! WebSocket transport adapter for one isolated `openmat-lsp` session.

use std::io;
use std::net::TcpStream;

use openmat_lsp::{RunOutcome, Server};
use tungstenite::Error as WebSocketError;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Message, WebSocket};

use crate::ServerError;
use crate::lsp_workspace::WorkspaceIndex;
use crate::workspace::WorkspaceService;

/// The browser-facing LSP transport has a deliberately smaller bound than the
/// stdio frontend. The limit applies independently to every incoming and
/// outgoing JSON-RPC message.
pub(crate) const MAX_LSP_MESSAGE_BYTES: usize = 1024 * 1024;

pub(crate) fn serve(
    socket: &mut WebSocket<TcpStream>,
    workspace: Option<&WorkspaceService>,
) -> Result<(), ServerError> {
    let mut server = Server::new();
    let mut workspace_index = workspace.cloned().map(WorkspaceIndex::new);
    loop {
        if let Some(index) = &mut workspace_index
            && let Some(status) = index.refresh(&mut server, false)
        {
            send_index_status(socket, &status)?;
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if text.len() > MAX_LSP_MESSAGE_BYTES {
                    close(socket, CloseCode::Size, "LSP message exceeds 1 MiB");
                    return Ok(());
                }
                // A project rename gets a fresh bounded disk snapshot even when
                // an OS change notification is still in the debounce interval.
                let changes_workspace = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .is_some_and(|message| {
                        matches!(
                            message.get("method").and_then(serde_json::Value::as_str),
                            Some("textDocument/prepareRename" | "textDocument/rename")
                        )
                    });
                if changes_workspace
                    && let Some(index) = &mut workspace_index
                    && let Some(status) = index.refresh(&mut server, true)
                {
                    send_index_status(socket, &status)?;
                }
                for response in server.handle_json(text.as_bytes()) {
                    let response = serde_json::to_string(&response).map_err(ServerError::Encode)?;
                    if response.len() > MAX_LSP_MESSAGE_BYTES {
                        close(socket, CloseCode::Size, "LSP response exceeds 1 MiB");
                        return Ok(());
                    }
                    socket.send(Message::text(response))?;
                }
                if server.should_exit() {
                    let (code, reason) = match server.exit_outcome() {
                        RunOutcome::CleanExit => (CloseCode::Normal, "LSP session exited"),
                        RunOutcome::ExitWithoutShutdown | RunOutcome::EndOfStream => {
                            (CloseCode::Policy, "LSP exit requires prior shutdown")
                        }
                    };
                    close(socket, code, reason);
                    return Ok(());
                }
            }
            Ok(Message::Binary(_)) => {
                close(
                    socket,
                    CloseCode::Unsupported,
                    "binary LSP frames are not supported",
                );
                return Ok(());
            }
            Ok(Message::Close(_)) => {
                let _ = socket.flush();
                return Ok(());
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => socket.flush()?,
            Ok(Message::Frame(_)) => {
                close(socket, CloseCode::Protocol, "unexpected raw LSP frame");
                return Ok(());
            }
            Err(WebSocketError::Io(error)) if is_poll_timeout(&error) => {}
            Err(WebSocketError::Capacity(_)) => {
                close(socket, CloseCode::Size, "LSP message exceeds 1 MiB");
                return Ok(());
            }
            Err(WebSocketError::Utf8(_)) => {
                close(socket, CloseCode::Invalid, "LSP text frame is not UTF-8");
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

fn send_index_status(socket: &mut WebSocket<TcpStream>, status: &str) -> Result<(), ServerError> {
    socket.send(Message::text(
        serde_json::json!({
            "jsonrpc": "2.0", "method": "window/logMessage",
            "params": { "type": 2, "message": status }
        })
        .to_string(),
    ))?;
    Ok(())
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

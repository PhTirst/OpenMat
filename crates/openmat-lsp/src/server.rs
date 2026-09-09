//! JSON-RPC/LSP state machine and connection loop.

use crate::document::{DocumentError, DocumentStore, WorkspaceSnapshot};
use crate::index::{SymbolIndex, is_valid_identifier};
use crate::language;
use crate::protocol::{
    CodeAction, CompletionItem, Diagnostic, DiagnosticSeverity, DocumentEdit, DocumentSymbol,
    Hover, Location, Position, PrepareRename, Range, SEMANTIC_TOKEN_TYPES, TextEdit, WorkspaceEdit,
    WorkspaceSymbol,
};
use crate::transport::{FramingError, read_message, write_message};
use serde_json::{Value, json};
use std::fmt;
use std::io::{self, BufRead, Write};

const JSON_RPC_VERSION: &str = "2.0";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ServerState {
    #[default]
    Uninitialized,
    Running,
    Shutdown,
    Exited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    CleanExit,
    ExitWithoutShutdown,
    EndOfStream,
}

#[derive(Debug)]
pub enum ConnectionError {
    Framing(FramingError),
    Json(serde_json::Error),
    Io(io::Error),
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Framing(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ConnectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Framing(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

impl From<FramingError> for ConnectionError {
    fn from(error: FramingError) -> Self {
        Self::Framing(error)
    }
}

impl From<serde_json::Error> for ConnectionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<io::Error> for ConnectionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// An in-memory, transport-neutral LSP server state machine.
#[derive(Debug, Default)]
pub struct Server {
    documents: DocumentStore,
    symbols: SymbolIndex,
    state: ServerState,
    clean_exit: bool,
}

impl Server {
    #[must_use]
    pub fn new() -> Self {
        Self {
            documents: DocumentStore::new(),
            symbols: SymbolIndex::default(),
            state: ServerState::Uninitialized,
            clean_exit: false,
        }
    }

    #[must_use]
    pub const fn state(&self) -> ServerState {
        self.state
    }

    #[must_use]
    pub const fn should_exit(&self) -> bool {
        matches!(self.state, ServerState::Exited)
    }

    #[must_use]
    pub const fn exit_outcome(&self) -> RunOutcome {
        if self.clean_exit {
            RunOutcome::CleanExit
        } else {
            RunOutcome::ExitWithoutShutdown
        }
    }

    #[must_use]
    pub const fn documents(&self) -> &DocumentStore {
        &self.documents
    }

    /// Refreshes the native host's disk index without replacing unsaved editor
    /// buffers. Unchanged snapshots do not rebuild the symbol index.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::SourceIdExhausted`] if a new source cannot be
    /// assigned a stable identifier.
    pub fn replace_workspace_documents(
        &mut self,
        snapshot: WorkspaceSnapshot,
    ) -> Result<bool, DocumentError> {
        let changed = self.documents.replace_workspace_documents(snapshot)?;
        if changed {
            self.symbols.rebuild(&self.documents);
        }
        Ok(changed)
    }

    /// Parses and handles one JSON-RPC message body.
    #[must_use]
    pub fn handle_json(&mut self, body: &[u8]) -> Vec<Value> {
        match serde_json::from_slice(body) {
            Ok(message) => self.handle_message(&message),
            Err(_) => vec![error_response(&Value::Null, -32700, "Parse error")],
        }
    }

    /// Handles one decoded JSON-RPC message.
    #[must_use]
    pub fn handle_message(&mut self, message: &Value) -> Vec<Value> {
        let Some(object) = message.as_object() else {
            return vec![error_response(&Value::Null, -32600, "Invalid Request")];
        };
        if object.get("jsonrpc").and_then(Value::as_str) != Some(JSON_RPC_VERSION) {
            return vec![error_response(
                object.get("id").unwrap_or(&Value::Null),
                -32600,
                "Invalid Request",
            )];
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return vec![error_response(
                object.get("id").unwrap_or(&Value::Null),
                -32600,
                "Invalid Request",
            )];
        };
        let id = object.get("id");
        let params = object.get("params").unwrap_or(&Value::Null);

        match self.state {
            ServerState::Uninitialized => self.handle_uninitialized(method, id, params),
            ServerState::Running => self.handle_running(method, id, params),
            ServerState::Shutdown => self.handle_shutdown(method, id),
            ServerState::Exited => Vec::new(),
        }
    }

    fn handle_uninitialized(
        &mut self,
        method: &str,
        id: Option<&Value>,
        _params: &Value,
    ) -> Vec<Value> {
        match method {
            "initialize" if id.is_some() => {
                self.state = ServerState::Running;
                vec![success_response(
                    id.unwrap_or(&Value::Null),
                    &initialize_result(),
                )]
            }
            "exit" => {
                self.state = ServerState::Exited;
                self.clean_exit = false;
                Vec::new()
            }
            _ => request_error(id, -32002, "Server not initialized"),
        }
    }

    fn handle_running(&mut self, method: &str, id: Option<&Value>, params: &Value) -> Vec<Value> {
        match method {
            "initialize" => request_error(id, -32600, "Initialize request already received"),
            "shutdown" if id.is_some() => {
                self.state = ServerState::Shutdown;
                vec![success_response(id.unwrap_or(&Value::Null), &Value::Null)]
            }
            "exit" => {
                self.state = ServerState::Exited;
                self.clean_exit = false;
                Vec::new()
            }
            "textDocument/didOpen" => self.did_open(params),
            "textDocument/didChange" => self.did_change(params),
            "textDocument/didClose" => self.did_close(params),
            "textDocument/documentSymbol" => self.document_symbol(id, params),
            "textDocument/hover" => self.hover(id, params),
            "textDocument/completion" => self.completion(id, params),
            "textDocument/definition" => self.definition(id, params),
            "textDocument/references" => self.references(id, params),
            "textDocument/prepareRename" => self.prepare_rename(id, params),
            "textDocument/rename" => self.rename(id, params),
            "textDocument/semanticTokens/full" => self.semantic_tokens(id, params),
            "workspace/symbol" => self.workspace_symbol(id, params),
            "textDocument/formatting" => self.formatting(id, params),
            "textDocument/codeAction" => self.code_action(id, params),
            "completionItem/resolve" => self.resolve_completion(id, params),
            _ => request_error(id, -32601, "Method not found"),
        }
    }

    fn handle_shutdown(&mut self, method: &str, id: Option<&Value>) -> Vec<Value> {
        if method == "exit" {
            self.state = ServerState::Exited;
            self.clean_exit = true;
            Vec::new()
        } else {
            request_error(id, -32600, "Server has shut down")
        }
    }

    fn did_open(&mut self, params: &Value) -> Vec<Value> {
        let Some(text_document) = params.get("textDocument") else {
            return Vec::new();
        };
        let (Some(uri), Some(version), Some(text)) = (
            text_document.get("uri").and_then(Value::as_str),
            text_document
                .get("version")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok()),
            text_document.get("text").and_then(Value::as_str),
        ) else {
            return Vec::new();
        };
        if self.documents.did_open(uri, version, text).is_err() {
            return Vec::new();
        }
        self.symbols.rebuild(&self.documents);
        self.documents
            .get(uri)
            .map_or_else(Vec::new, |document| vec![publish_diagnostics(document)])
    }

    fn did_change(&mut self, params: &Value) -> Vec<Value> {
        let Some(text_document) = params.get("textDocument") else {
            return Vec::new();
        };
        let (Some(uri), Some(version)) = (
            text_document.get("uri").and_then(Value::as_str),
            text_document
                .get("version")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok()),
        ) else {
            return Vec::new();
        };
        let Some(changes) = params.get("contentChanges").and_then(Value::as_array) else {
            return Vec::new();
        };
        let [change] = changes.as_slice() else {
            return Vec::new();
        };
        if change.get("range").is_some_and(|range| !range.is_null()) {
            return Vec::new();
        }
        let Some(text) = change.get("text").and_then(Value::as_str) else {
            return Vec::new();
        };
        if self.documents.did_change(uri, version, text).is_err() {
            return Vec::new();
        }
        self.symbols.rebuild(&self.documents);
        self.documents
            .get(uri)
            .map_or_else(Vec::new, |document| vec![publish_diagnostics(document)])
    }

    fn did_close(&mut self, params: &Value) -> Vec<Value> {
        let Some(uri) = document_uri(params) else {
            return Vec::new();
        };
        if self.documents.did_close(uri).is_err() {
            return Vec::new();
        }
        self.symbols.rebuild(&self.documents);
        vec![json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "diagnostics": []
            }
        })]
    }

    fn document_symbol(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let Some(uri) = document_uri(params) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        let result = self.documents.get(uri).map_or(Value::Null, |document| {
            Value::Array(
                language::document_symbols(document)
                    .iter()
                    .map(document_symbol_value)
                    .collect(),
            )
        });
        vec![success_response(id, &result)]
    }

    fn hover(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let (Some(uri), Some(position)) = (document_uri(params), request_position(params)) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        let result = self
            .documents
            .get(uri)
            .and_then(|document| language::hover(document, position))
            .map_or(Value::Null, |hover| hover_value(&hover));
        vec![success_response(id, &result)]
    }

    fn completion(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let (Some(uri), Some(position)) = (document_uri(params), request_position(params)) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        let result = self.documents.get(uri).map_or(Value::Null, |document| {
            Value::Array(
                language::completion(document, position)
                    .iter()
                    .map(completion_item_value)
                    .collect(),
            )
        });
        vec![success_response(id, &result)]
    }

    fn definition(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let (Some(uri), Some(position)) = (document_uri(params), request_position(params)) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        if self.documents.get_effective(uri).is_none() {
            return vec![success_response(id, &Value::Null)];
        }
        let result = self
            .symbols
            .definition(uri, position)
            .map_or(Value::Null, |location| location_value(&location));
        vec![success_response(id, &result)]
    }

    fn references(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let (Some(uri), Some(position), Some(include_declaration)) = (
            document_uri(params),
            request_position(params),
            params
                .get("context")
                .and_then(|context| context.get("includeDeclaration"))
                .and_then(Value::as_bool),
        ) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        if self.documents.get_effective(uri).is_none() {
            return vec![success_response(id, &Value::Null)];
        }
        let result = self
            .symbols
            .references(uri, position, include_declaration)
            .map_or(Value::Null, |locations| {
                Value::Array(locations.iter().map(location_value).collect())
            });
        vec![success_response(id, &result)]
    }

    fn prepare_rename(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let (Some(uri), Some(position)) = (document_uri(params), request_position(params)) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        if !self.documents.workspace_index_complete() {
            return request_error(
                Some(id),
                -32803,
                "Workspace index is incomplete; rename is temporarily unavailable",
            );
        }
        if let Some(reason) = self.symbols.rename_block_reason(uri, position) {
            return request_error(Some(id), -32803, reason);
        }
        if self.documents.get_effective(uri).is_none() {
            return vec![success_response(id, &Value::Null)];
        }
        let result = self
            .symbols
            .prepare_rename(uri, position)
            .map_or(Value::Null, |prepared| prepare_rename_value(&prepared));
        vec![success_response(id, &result)]
    }

    fn rename(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let (Some(uri), Some(position), Some(new_name)) = (
            document_uri(params),
            request_position(params),
            params.get("newName").and_then(Value::as_str),
        ) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        if !is_valid_identifier(new_name) {
            return request_error(Some(id), -32602, "Invalid rename identifier");
        }
        if !self.documents.workspace_index_complete() {
            return request_error(
                Some(id),
                -32803,
                "Workspace index is incomplete; rename is temporarily unavailable",
            );
        }
        if let Some(reason) = self
            .symbols
            .rename_target_block_reason(uri, position, new_name)
        {
            return request_error(Some(id), -32803, reason);
        }
        if self.documents.get_effective(uri).is_none() {
            return vec![success_response(id, &Value::Null)];
        }
        let result = self
            .symbols
            .rename(&self.documents, uri, position, new_name)
            .map_or(Value::Null, |edit| workspace_edit_value(&edit));
        vec![success_response(id, &result)]
    }

    fn semantic_tokens(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let Some(uri) = document_uri(params) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        let result = self.documents.get(uri).map_or(
            Value::Null,
            |document| json!({ "data": language::semantic_tokens(&self.documents, document) }),
        );
        vec![success_response(id, &result)]
    }

    fn workspace_symbol(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let Some(query) = params.get("query").and_then(Value::as_str) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        let result = Value::Array(
            language::workspace_symbols(&self.documents, query)
                .iter()
                .map(workspace_symbol_value)
                .collect(),
        );
        vec![success_response(id, &result)]
    }

    fn formatting(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let Some(uri) = document_uri(params) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        if !params.get("options").is_some_and(Value::is_object) {
            return request_error(Some(id), -32602, "Invalid params");
        }
        let result = self.documents.get(uri).map_or(Value::Null, |document| {
            Value::Array(
                language::formatting(document)
                    .iter()
                    .map(text_edit_value)
                    .collect(),
            )
        });
        vec![success_response(id, &result)]
    }

    fn code_action(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let (Some(uri), Some(range), Some(context)) = (
            document_uri(params),
            params.get("range").and_then(range_from_value),
            params.get("context"),
        ) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        let Some(context_diagnostics) = context.get("diagnostics").and_then(Value::as_array) else {
            return request_error(Some(id), -32602, "Invalid params");
        };
        let permits_quick_fix = context
            .get("only")
            .and_then(Value::as_array)
            .is_none_or(|only| only.iter().any(|kind| kind.as_str() == Some("quickfix")));
        let result = if permits_quick_fix {
            self.documents.get(uri).map_or(Value::Null, |document| {
                let diagnostics = context_diagnostics
                    .iter()
                    .filter_map(diagnostic_from_value)
                    .collect::<Vec<_>>();
                Value::Array(
                    language::code_actions(document, range, &diagnostics)
                        .iter()
                        .map(code_action_value)
                        .collect(),
                )
            })
        } else {
            Value::Array(Vec::new())
        };
        vec![success_response(id, &result)]
    }

    fn resolve_completion(&self, id: Option<&Value>, params: &Value) -> Vec<Value> {
        let Some(id) = id else {
            return Vec::new();
        };
        let (Some(label), Some(data)) = (
            params.get("label").and_then(Value::as_str),
            params.get("data"),
        ) else {
            return request_error(Some(id), -32602, "Invalid completion item");
        };
        let (Some(uri), Some(version)) = (
            data.get("uri").and_then(Value::as_str),
            data.get("version")
                .and_then(Value::as_i64)
                .and_then(|version| i32::try_from(version).ok()),
        ) else {
            return request_error(Some(id), -32602, "Invalid completion item data");
        };
        let Some(document) = self.documents.get(uri) else {
            return request_error(Some(id), -32602, "Completion document is not open");
        };
        if document.version() != version {
            return request_error(Some(id), -32602, "Stale completion item");
        }
        // Completion edits end at the original cursor. Resolve in that same
        // lexical scope rather than letting another function's locals win.
        let Some(range) = params
            .get("textEdit")
            .and_then(|edit| edit.get("range"))
            .and_then(range_from_value)
            .filter(|range| {
                range.start <= range.end
                    && document.position_to_byte(range.start).is_some()
                    && document.position_to_byte(range.end).is_some()
            })
        else {
            return request_error(Some(id), -32602, "Invalid completion edit range");
        };

        let mut result = params.clone();
        if let Some(resolution) = language::resolve_completion(document, label, range.end) {
            result["detail"] = Value::String(resolution.detail);
            result["documentation"] = json!({
                "kind": "markdown",
                "value": resolution.markdown
            });
        }
        vec![success_response(id, &result)]
    }
}

/// Runs a server on any buffered input and output transport.
///
/// # Errors
///
/// Returns framing, JSON serialization, or I/O failures. A clean EOF between
/// frames returns [`RunOutcome::EndOfStream`].
pub fn run_connection(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
) -> Result<RunOutcome, ConnectionError> {
    let mut server = Server::new();
    loop {
        let Some(body) = read_message(reader)? else {
            return Ok(RunOutcome::EndOfStream);
        };
        for response in server.handle_json(&body) {
            let body = serde_json::to_vec(&response)?;
            write_message(writer, &body)?;
            writer.flush()?;
        }
        if server.should_exit() {
            return Ok(server.exit_outcome());
        }
    }
}

fn initialize_result() -> Value {
    json!({
        "capabilities": {
            "positionEncoding": "utf-16",
            "textDocumentSync": {
                "openClose": true,
                "change": 1
            },
            "documentSymbolProvider": true,
            "hoverProvider": true,
            "completionProvider": {
                "resolveProvider": true
            },
            "definitionProvider": true,
            "referencesProvider": true,
            "renameProvider": {
                "prepareProvider": true
            },
            "semanticTokensProvider": {
                "legend": {
                    "tokenTypes": SEMANTIC_TOKEN_TYPES,
                    "tokenModifiers": []
                },
                "range": false,
                "full": true
            },
            "workspaceSymbolProvider": true,
            "documentFormattingProvider": true,
            "codeActionProvider": {
                "codeActionKinds": ["quickfix"],
                "resolveProvider": false
            }
        },
        "serverInfo": {
            "name": "openmat-lsp",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn publish_diagnostics(document: &crate::document::Document) -> Value {
    json!({
        "jsonrpc": JSON_RPC_VERSION,
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": document.uri(),
            "version": document.version(),
            "diagnostics": language::diagnostics(document)
                .iter()
                .map(diagnostic_value)
                .collect::<Vec<_>>()
        }
    })
}

fn success_response(id: &Value, result: &Value) -> Value {
    json!({
        "jsonrpc": JSON_RPC_VERSION,
        "id": id,
        "result": result
    })
}

fn error_response(id: &Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": JSON_RPC_VERSION,
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn request_error(id: Option<&Value>, code: i32, message: &str) -> Vec<Value> {
    id.map_or_else(Vec::new, |id| vec![error_response(id, code, message)])
}

fn document_uri(params: &Value) -> Option<&str> {
    params.get("textDocument")?.get("uri")?.as_str()
}

fn request_position(params: &Value) -> Option<Position> {
    position_from_value(params.get("position")?)
}

fn position_from_value(position: &Value) -> Option<Position> {
    Some(Position::new(
        u32::try_from(position.get("line")?.as_u64()?).ok()?,
        u32::try_from(position.get("character")?.as_u64()?).ok()?,
    ))
}

fn range_from_value(range: &Value) -> Option<Range> {
    Some(Range::new(
        position_from_value(range.get("start")?)?,
        position_from_value(range.get("end")?)?,
    ))
}

fn position_value(position: Position) -> Value {
    json!({
        "line": position.line,
        "character": position.character
    })
}

fn range_value(range: Range) -> Value {
    json!({
        "start": position_value(range.start),
        "end": position_value(range.end)
    })
}

fn diagnostic_value(diagnostic: &Diagnostic) -> Value {
    let mut value = json!({
        "range": range_value(diagnostic.range),
        "severity": diagnostic.severity.lsp_value(),
        "source": "openmat",
        "message": diagnostic.message
    });
    if let Some(code) = &diagnostic.code {
        value["code"] = Value::String(code.clone());
    }
    value
}

fn document_symbol_value(symbol: &DocumentSymbol) -> Value {
    let mut value = json!({
        "name": symbol.name,
        "kind": symbol.kind.lsp_value(),
        "range": range_value(symbol.range),
        "selectionRange": range_value(symbol.selection_range),
        "children": symbol.children.iter().map(document_symbol_value).collect::<Vec<_>>()
    });
    if let Some(detail) = &symbol.detail {
        value["detail"] = Value::String(detail.clone());
    }
    value
}

fn hover_value(hover: &Hover) -> Value {
    json!({
        "contents": {
            "kind": "markdown",
            "value": hover.markdown
        },
        "range": range_value(hover.range)
    })
}

fn completion_item_value(item: &CompletionItem) -> Value {
    json!({
        "label": item.label,
        "kind": item.kind.lsp_value(),
        "detail": item.detail,
        "textEdit": text_edit_value(&item.text_edit),
        "data": {
            "uri": item.data.uri,
            "version": item.data.version
        }
    })
}

fn workspace_symbol_value(symbol: &WorkspaceSymbol) -> Value {
    let mut value = json!({
        "name": symbol.name,
        "kind": symbol.kind.lsp_value(),
        "location": location_value(&symbol.location)
    });
    if let Some(container_name) = &symbol.container_name {
        value["containerName"] = Value::String(container_name.clone());
    }
    value
}

fn code_action_value(action: &CodeAction) -> Value {
    json!({
        "title": action.title,
        "kind": action.kind,
        "diagnostics": action.diagnostics.iter().map(diagnostic_value).collect::<Vec<_>>(),
        "isPreferred": action.is_preferred,
        "edit": workspace_edit_value(&action.edit)
    })
}

fn diagnostic_from_value(value: &Value) -> Option<Diagnostic> {
    let severity = match value.get("severity").and_then(Value::as_u64) {
        Some(1) => DiagnosticSeverity::Error,
        Some(2) => DiagnosticSeverity::Warning,
        _ => DiagnosticSeverity::Information,
    };
    Some(Diagnostic {
        range: range_from_value(value.get("range")?)?,
        severity,
        code: value.get("code").and_then(Value::as_str).map(str::to_owned),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

fn text_edit_value(edit: &TextEdit) -> Value {
    json!({
        "range": range_value(edit.range),
        "newText": edit.new_text
    })
}

fn location_value(location: &Location) -> Value {
    json!({
        "uri": location.uri,
        "range": range_value(location.range)
    })
}

fn prepare_rename_value(prepared: &PrepareRename) -> Value {
    json!({
        "range": range_value(prepared.range),
        "placeholder": prepared.placeholder
    })
}

fn workspace_edit_value(edit: &WorkspaceEdit) -> Value {
    json!({
        "documentChanges": edit
            .document_changes
            .iter()
            .map(document_edit_value)
            .chain(edit.file_renames.iter().map(|rename| json!({
                "kind": "rename",
                "oldUri": rename.old_uri,
                "newUri": rename.new_uri,
                "options": {"overwrite": false}
            })))
            .collect::<Vec<_>>()
    })
}

fn document_edit_value(edit: &DocumentEdit) -> Value {
    json!({
        "textDocument": {
            "uri": edit.uri,
            "version": edit.version
        },
        "edits": edit.edits.iter().map(text_edit_value).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::{RunOutcome, Server, ServerState, run_connection};
    use crate::transport::{read_message, write_message};
    use serde_json::{Value, json};
    use std::io::{BufReader, Cursor};

    fn request(id: i32, method: &str, params: impl Into<Value>) -> Value {
        let params = params.into();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        })
    }

    fn notification(method: &str, params: impl Into<Value>) -> Value {
        let params = params.into();
        json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        })
    }

    fn advanced_code_action_request() -> Value {
        request(
            5,
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": "file:///advanced.m" },
                "range": {
                    "start": { "line": 4, "character": 0 },
                    "end": { "line": 4, "character": 3 }
                },
                "context": {
                    "diagnostics": [{
                        "range": {
                            "start": { "line": 4, "character": 0 },
                            "end": { "line": 4, "character": 3 }
                        },
                        "severity": 1,
                        "code": "OMP0002",
                        "message": "unmatched `end`"
                    }]
                }
            }),
        )
    }

    fn advanced_resolve_request() -> Value {
        request(
            6,
            "completionItem/resolve",
            json!({
                "label": "calc",
                "kind": 3,
                "detail": "function",
                "textEdit": {
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 0 }
                    },
                    "newText": "calc"
                },
                "data": { "uri": "file:///advanced.m", "version": 11 }
            }),
        )
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn workspace_disk_navigation_rename_and_overlay_lifecycle() {
        use crate::{WorkspaceDocument, WorkspaceSnapshot};

        let root = "openmat-workspace://root-1/project/";
        let main = format!("{root}main.m");
        let helper = format!("{root}helper.m");
        let other = format!("{root}other.m");
        let snapshot = |helper_text: Option<&str>| WorkspaceSnapshot {
            root_uri: root.to_owned(),
            current_directory: String::new(),
            search_paths: Vec::new(),
            complete: true,
            documents: [
                ("helper.m", helper_text),
                ("other.m", Some("value = helper(3);\n")),
            ]
            .into_iter()
            .filter_map(|(path, text)| {
                text.map(|text| WorkspaceDocument {
                    uri: format!("{root}{path}"),
                    relative_path: path.to_owned(),
                    text: text.to_owned(),
                })
            })
            .collect(),
        };
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        assert!(
            server
                .replace_workspace_documents(snapshot(Some(
                    "function y = helper(x)\ny = x;\nend\n"
                )))
                .unwrap()
        );
        let _ = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {"uri": main, "version": 9, "text": "answer = helper(2);\n"}
            }),
        ));
        let definition = |server: &mut Server| {
            server.handle_message(&request(
                2,
                "textDocument/definition",
                json!({
                    "textDocument": {"uri": main}, "position": {"line": 0, "character": 11}
                }),
            ))
        };
        assert_eq!(definition(&mut server)[0]["result"]["uri"], helper);
        let references = server.handle_message(&request(
            3,
            "textDocument/references",
            json!({
                "textDocument": {"uri": main}, "position": {"line": 0, "character": 11},
                "context": {"includeDeclaration": true}
            }),
        ));
        assert_eq!(references[0]["result"].as_array().unwrap().len(), 3);
        let rename = server.handle_message(&request(4, "textDocument/rename", json!({
            "textDocument": {"uri": main}, "position": {"line": 0, "character": 11}, "newName": "processSignal"
        })));
        let changes = rename[0]["result"]["documentChanges"].as_array().unwrap();
        assert_eq!(changes.len(), 4);
        assert_eq!(
            changes[3],
            json!({
                "kind": "rename", "oldUri": helper,
                "newUri": format!("{root}processSignal.m"), "options": {"overwrite": false}
            })
        );
        for change in &changes[..3] {
            let identity = &change["textDocument"];
            if identity["uri"] == main {
                assert_eq!(identity["version"], 9);
            } else {
                assert!(identity["uri"] == helper || identity["uri"] == other);
                assert!(identity["version"].is_null());
            }
        }
        let _ = server.handle_message(&notification("textDocument/didOpen", json!({
            "textDocument": {"uri": helper, "version": 4, "text": "\nfunction y = helper(x)\ny = x + 1;\nend\n"}
        })));
        server
            .replace_workspace_documents(snapshot(Some(
                "\n\nfunction y = helper(x)\ny = x + 2;\nend\n",
            )))
            .unwrap();
        assert_eq!(
            definition(&mut server)[0]["result"]["range"]["start"]["line"],
            1
        );
        let _ = server.handle_message(&notification(
            "textDocument/didClose",
            json!({"textDocument": {"uri": helper}}),
        ));
        assert_eq!(
            definition(&mut server)[0]["result"]["range"]["start"]["line"],
            2
        );
        server.replace_workspace_documents(snapshot(None)).unwrap();
        assert!(definition(&mut server)[0]["result"].is_null());
        server
            .replace_workspace_documents(snapshot(Some("function y = helper(x)\ny = x;\nend\n")))
            .unwrap();
        assert_eq!(definition(&mut server)[0]["result"]["uri"], helper);
    }

    #[test]
    fn incomplete_workspace_refuses_rename_until_recovery() {
        use crate::WorkspaceSnapshot;

        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let uri = "openmat-workspace://root-1/project/main.m";
        let _ = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {"uri": uri, "version": 1, "text": "value = 1;\nresult = value;\n"}
            }),
        ));
        let mut snapshot = WorkspaceSnapshot {
            root_uri: "openmat-workspace://root-1/project/".to_owned(),
            current_directory: String::new(),
            search_paths: Vec::new(),
            complete: false,
            documents: Vec::new(),
        };
        server
            .replace_workspace_documents(snapshot.clone())
            .unwrap();
        for method in ["textDocument/prepareRename", "textDocument/rename"] {
            let response = server.handle_message(&request(
                5,
                method,
                json!({
                    "textDocument": {"uri": uri}, "position": {"line": 0, "character": 2},
                    "newName": "input"
                }),
            ));
            assert_eq!(response[0]["error"]["code"], -32803);
        }
        snapshot.complete = true;
        server.replace_workspace_documents(snapshot).unwrap();
        let prepared = server.handle_message(&request(
            6,
            "textDocument/prepareRename",
            json!({
                "textDocument": {"uri": uri}, "position": {"line": 0, "character": 2}
            }),
        ));
        assert_eq!(prepared[0]["result"]["placeholder"], "value");
    }

    #[test]
    fn case_only_primary_function_rename_fails_before_editing() {
        use crate::{WorkspaceDocument, WorkspaceSnapshot};

        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let uri = "openmat-workspace://root-1/project/helper.m";
        server
            .replace_workspace_documents(WorkspaceSnapshot {
                root_uri: "openmat-workspace://root-1/project/".to_owned(),
                current_directory: String::new(),
                search_paths: Vec::new(),
                complete: true,
                documents: vec![WorkspaceDocument {
                    uri: uri.to_owned(),
                    relative_path: "helper.m".to_owned(),
                    text: "function y = helper(x)\ny = x;\nend\n".to_owned(),
                }],
            })
            .unwrap();
        let response = server.handle_message(&request(
            2,
            "textDocument/rename",
            json!({
                "textDocument": {"uri": uri}, "position": {"line": 0, "character": 15},
                "newName": "HELPER"
            }),
        ));
        assert_eq!(response[0]["error"]["code"], -32803);
        assert!(
            response[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("letter case")
        );
        assert!(response[0].get("result").is_none());
        assert!(
            server
                .symbols
                .rename(
                    &server.documents,
                    uri,
                    crate::protocol::Position::new(0, 15),
                    "HELPER"
                )
                .is_none()
        );
    }

    #[test]
    fn workspace_class_rename_reports_related_resource_boundary() {
        use crate::{WorkspaceDocument, WorkspaceSnapshot};

        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let uri = "openmat-workspace://root-1/project/Demo.m";
        server
            .replace_workspace_documents(WorkspaceSnapshot {
                root_uri: "openmat-workspace://root-1/project/".to_owned(),
                current_directory: String::new(),
                search_paths: Vec::new(),
                complete: true,
                documents: vec![WorkspaceDocument {
                    uri: uri.to_owned(),
                    relative_path: "Demo.m".to_owned(),
                    text: "classdef Demo\nend\n".to_owned(),
                }],
            })
            .unwrap();
        for method in ["textDocument/prepareRename", "textDocument/rename"] {
            let response = server.handle_message(&request(
                2,
                method,
                json!({
                    "textDocument": {"uri": uri}, "position": {"line": 0, "character": 10},
                    "newName": "RenamedDemo"
                }),
            ));
            assert_eq!(response[0]["error"]["code"], -32803);
            assert!(
                response[0]["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("class")
            );
        }
    }

    #[test]
    fn initialize_advertises_only_implemented_capabilities() {
        let mut server = Server::new();
        let responses = server.handle_message(&request(1, "initialize", json!({})));
        let capabilities = &responses[0]["result"]["capabilities"];
        assert_eq!(server.state(), ServerState::Running);
        assert_eq!(capabilities["positionEncoding"], "utf-16");
        assert_eq!(capabilities["textDocumentSync"]["change"], 1);
        assert_eq!(capabilities["documentSymbolProvider"], true);
        assert_eq!(capabilities["hoverProvider"], true);
        assert_eq!(capabilities["definitionProvider"], true);
        assert_eq!(capabilities["referencesProvider"], true);
        assert_eq!(capabilities["renameProvider"]["prepareProvider"], true);
        assert_eq!(capabilities["semanticTokensProvider"]["full"], true);
        assert_eq!(
            capabilities["semanticTokensProvider"]["legend"]["tokenTypes"]
                .as_array()
                .map(Vec::len),
            Some(11)
        );
        assert_eq!(capabilities["workspaceSymbolProvider"], true);
        assert_eq!(capabilities["documentFormattingProvider"], true);
        assert_eq!(
            capabilities["codeActionProvider"]["codeActionKinds"][0],
            "quickfix"
        );
        assert_eq!(capabilities["completionProvider"]["resolveProvider"], true);
        assert!(capabilities.get("executeCommandProvider").is_none());
    }

    #[test]
    fn open_change_close_publish_versioned_diagnostics() {
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let opened = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///demo.m",
                    "languageId": "matlab",
                    "version": 1,
                    "text": "😀"
                }
            }),
        ));
        assert_eq!(opened[0]["method"], "textDocument/publishDiagnostics");
        assert_eq!(opened[0]["params"]["version"], 1);
        assert_eq!(
            opened[0]["params"]["diagnostics"][0]["range"]["end"]["character"],
            2
        );

        let changed = server.handle_message(&notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": "file:///demo.m", "version": 2 },
                "contentChanges": [{ "text": "x = 1;" }]
            }),
        ));
        assert_eq!(changed[0]["params"]["version"], 2);
        assert_eq!(
            changed[0]["params"]["diagnostics"],
            Value::Array(Vec::new())
        );

        let closed = server.handle_message(&notification(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": "file:///demo.m" } }),
        ));
        assert_eq!(closed[0]["params"]["diagnostics"], Value::Array(Vec::new()));
        assert!(server.documents().is_empty());
    }

    #[test]
    fn serves_symbols_hover_and_completion_requests() {
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let _ = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///demo.m",
                    "version": 1,
                    "text": "function y = calculate(x)\ny = x + 1;\nend\ncal"
                }
            }),
        ));

        let symbols = server.handle_message(&request(
            2,
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": "file:///demo.m" } }),
        ));
        assert_eq!(symbols[0]["result"][0]["name"], "calculate");
        assert!(symbols[0]["result"][0].get("range").is_some());

        let hover = server.handle_message(&request(
            3,
            "textDocument/hover",
            json!({
                "textDocument": { "uri": "file:///demo.m" },
                "position": { "line": 1, "character": 6 }
            }),
        ));
        assert!(
            hover[0]["result"]["contents"]["value"]
                .as_str()
                .is_some_and(|value| value.contains("addition"))
        );
        assert!(hover[0]["result"].get("range").is_some());

        let completion = server.handle_message(&request(
            4,
            "textDocument/completion",
            json!({
                "textDocument": { "uri": "file:///demo.m" },
                "position": { "line": 3, "character": 3 }
            }),
        ));
        let item = completion[0]["result"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["label"] == "calculate"))
            .expect("function completion");
        assert!(item["textEdit"].get("range").is_some());
    }

    #[test]
    fn serves_semantic_workspace_formatting_and_safe_code_actions() {
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let opened = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///demo.m",
                    "version": 7,
                    "text": "function y = calc(x)  \r\ny = x + 1; \r\nend\r\nend\r\n"
                }
            }),
        ));
        let _ = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///holder.m",
                    "version": 2,
                    "text": "classdef Holder\nproperties\nvalue = 1\nend\nend\n"
                }
            }),
        ));

        let semantic = server.handle_message(&request(
            2,
            "textDocument/semanticTokens/full",
            json!({ "textDocument": { "uri": "file:///demo.m" } }),
        ));
        let data = semantic[0]["result"]["data"]
            .as_array()
            .expect("semantic token data");
        assert!(!data.is_empty());
        assert_eq!(data.len() % 5, 0);

        let workspace =
            server.handle_message(&request(3, "workspace/symbol", json!({ "query": "value" })));
        assert_eq!(workspace[0]["result"][0]["name"], "value");
        assert_eq!(workspace[0]["result"][0]["containerName"], "Holder");

        let formatted = server.handle_message(&request(
            4,
            "textDocument/formatting",
            json!({
                "textDocument": { "uri": "file:///demo.m" },
                "options": { "tabSize": 4, "insertSpaces": true }
            }),
        ));
        let new_text = formatted[0]["result"][0]["newText"]
            .as_str()
            .expect("formatting edit");
        assert!(!new_text.contains("  \n"));
        assert!(new_text.ends_with('\n'));

        let diagnostic = opened[0]["params"]["diagnostics"]
            .as_array()
            .and_then(|diagnostics| {
                diagnostics
                    .iter()
                    .find(|diagnostic| diagnostic["code"] == "OMP0002")
            })
            .expect("unmatched end diagnostic")
            .clone();
        let actions = server.handle_message(&request(
            5,
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": "file:///demo.m" },
                "range": diagnostic["range"],
                "context": { "diagnostics": [diagnostic] }
            }),
        ));
        assert_eq!(actions[0]["result"][0]["kind"], "quickfix");
        assert_eq!(
            actions[0]["result"][0]["edit"]["documentChanges"][0]["textDocument"]["version"],
            7
        );
        assert_eq!(
            actions[0]["result"][0]["edit"]["documentChanges"][0]["edits"][0]["newText"],
            ""
        );
    }

    #[test]
    fn resolves_versioned_completion_without_changing_edit() {
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let _ = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///demo.m",
                    "version": 7,
                    "text": "function y = calc(x)\ny = x;\nend\n"
                }
            }),
        ));
        let completion = server.handle_message(&request(
            2,
            "textDocument/completion",
            json!({
                "textDocument": { "uri": "file:///demo.m" },
                "position": { "line": 0, "character": 0 }
            }),
        ));
        let item = completion[0]["result"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["label"] == "calc"))
            .expect("resolvable completion")
            .clone();
        let original_edit = item["textEdit"].clone();
        let resolved = server.handle_message(&request(3, "completionItem/resolve", item));
        assert!(
            resolved[0]["result"]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("function"))
        );
        assert!(
            resolved[0]["result"]["documentation"]["value"]
                .as_str()
                .is_some_and(|documentation| documentation.contains("current document"))
        );
        assert_eq!(resolved[0]["result"]["textEdit"], original_edit);

        let stale = server.handle_message(&request(
            4,
            "completionItem/resolve",
            json!({
                "label": "calc",
                "data": { "uri": "file:///demo.m", "version": 6 }
            }),
        ));
        assert_eq!(stale[0]["error"]["code"], -32602);

        let unknown = server.handle_message(&request(5, "openmat/unknown", Value::Null));
        assert_eq!(unknown[0]["error"]["code"], -32601);
    }

    #[test]
    fn completion_resolve_and_hover_retain_the_original_function_scope() {
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let _ = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///scopes.m",
                    "version": 1,
                    "text": concat!(
                        "function y = first(input)\r\n",
                        "  y = fft(input);\r\n",
                        "end\r\n",
                        "function y = second(fft)\r\n",
                        "  y = fft(1);\r\n",
                        "end\r\n",
                    )
                }
            }),
        ));
        for (line, expected_kind, expected_detail) in
            [(1, 3, "built-in function"), (4, 6, "function parameter")]
        {
            let completion = server.handle_message(&request(
                2,
                "textDocument/completion",
                json!({
                    "textDocument": { "uri": "file:///scopes.m" },
                    "position": { "line": line, "character": 7 }
                }),
            ));
            let item = completion[0]["result"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["label"] == "fft")
                .unwrap()
                .clone();
            assert_eq!(item["kind"], expected_kind);
            assert_eq!(item["detail"], expected_detail);
            let resolved =
                server.handle_message(&request(3, "completionItem/resolve", item.clone()));
            assert_eq!(resolved[0]["result"]["textEdit"], item["textEdit"]);
            assert_eq!(resolved[0]["result"]["data"], item["data"]);
            assert!(
                resolved[0]["result"]["detail"]
                    .as_str()
                    .unwrap()
                    .contains(expected_detail)
            );

            let hover = server.handle_message(&request(
                4,
                "textDocument/hover",
                json!({
                    "textDocument": { "uri": "file:///scopes.m" },
                    "position": { "line": line, "character": 7 }
                }),
            ));
            assert!(
                hover[0]["result"]["contents"]["value"]
                    .as_str()
                    .unwrap()
                    .contains(expected_detail)
            );

            for range in [
                Value::Null,
                json!({"start": {"line": 99, "character": 0}, "end": {"line": 99, "character": 1}}),
                json!({"start": {"line": line, "character": 7}, "end": {"line": line, "character": 6}}),
            ] {
                let mut malformed = item.clone();
                malformed["textEdit"]["range"] = range;
                let result =
                    server.handle_message(&request(5, "completionItem/resolve", malformed));
                assert_eq!(result[0]["error"]["code"], -32602);
            }
        }
    }

    #[test]
    fn serves_cross_document_navigation_and_versioned_rename() {
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        let _ = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///a-helper.m",
                    "languageId": "matlab",
                    "version": 4,
                    "text": "function y = helper(x)\r\ny = x;\r\nend\r\n"
                }
            }),
        ));
        let _ = server.handle_message(&notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///z-use.m",
                    "languageId": "matlab",
                    "version": 9,
                    "text": "% 😀\r\nresult = helper(1);\r\n"
                }
            }),
        ));

        let definition = server.handle_message(&request(
            2,
            "textDocument/definition",
            json!({
                "textDocument": { "uri": "file:///z-use.m" },
                "position": { "line": 1, "character": 11 }
            }),
        ));
        assert_eq!(definition[0]["result"]["uri"], "file:///a-helper.m");
        assert_eq!(definition[0]["result"]["range"]["start"]["character"], 13);

        let references = server.handle_message(&request(
            3,
            "textDocument/references",
            json!({
                "textDocument": { "uri": "file:///a-helper.m" },
                "position": { "line": 0, "character": 15 },
                "context": { "includeDeclaration": true }
            }),
        ));
        assert_eq!(references[0]["result"].as_array().map(Vec::len), Some(2));

        let prepared = server.handle_message(&request(
            4,
            "textDocument/prepareRename",
            json!({
                "textDocument": { "uri": "file:///z-use.m" },
                "position": { "line": 1, "character": 11 }
            }),
        ));
        assert_eq!(prepared[0]["result"]["placeholder"], "helper");
        assert_eq!(prepared[0]["result"]["range"]["start"]["character"], 9);
        assert_eq!(prepared[0]["result"]["range"]["end"]["character"], 15);

        let renamed = server.handle_message(&request(
            5,
            "textDocument/rename",
            json!({
                "textDocument": { "uri": "file:///z-use.m" },
                "position": { "line": 1, "character": 11 },
                "newName": "renamed"
            }),
        ));
        let changes = renamed[0]["result"]["documentChanges"]
            .as_array()
            .expect("document changes");
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0]["textDocument"]["version"], 4);
        assert_eq!(changes[1]["textDocument"]["version"], 9);

        let invalid = server.handle_message(&request(
            6,
            "textDocument/rename",
            json!({
                "textDocument": { "uri": "file:///z-use.m" },
                "position": { "line": 1, "character": 11 },
                "newName": "while"
            }),
        ));
        assert_eq!(invalid[0]["error"]["code"], -32602);
    }

    #[test]
    fn symbol_index_tracks_versions_changes_and_close_lifecycle() {
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        for (uri, version, text) in [
            (
                "file:///definition.m",
                2,
                "function y = target(x)\ny = x;\nend\n",
            ),
            ("file:///use.m", 5, "value = target(1);\n"),
        ] {
            let _ = server.handle_message(&notification(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": "matlab",
                        "version": version,
                        "text": text
                    }
                }),
            ));
        }

        let stale = server.handle_message(&notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": "file:///use.m", "version": 4 },
                "contentChanges": [{ "text": "value = other(1);\n" }]
            }),
        ));
        assert!(stale.is_empty());
        let still_indexed = server.handle_message(&request(
            2,
            "textDocument/definition",
            json!({
                "textDocument": { "uri": "file:///use.m" },
                "position": { "line": 0, "character": 10 }
            }),
        ));
        assert_eq!(still_indexed[0]["result"]["uri"], "file:///definition.m");

        let changed = server.handle_message(&notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": "file:///use.m", "version": 6 },
                "contentChanges": [{ "text": "value = other(1);\n" }]
            }),
        ));
        assert_eq!(changed[0]["params"]["version"], 6);
        let removed_reference = server.handle_message(&request(
            3,
            "textDocument/references",
            json!({
                "textDocument": { "uri": "file:///definition.m" },
                "position": { "line": 0, "character": 15 },
                "context": { "includeDeclaration": false }
            }),
        ));
        assert_eq!(removed_reference[0]["result"], Value::Array(Vec::new()));

        let _ = server.handle_message(&notification(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": "file:///definition.m" } }),
        ));
        let closed = server.handle_message(&request(
            4,
            "textDocument/definition",
            json!({
                "textDocument": { "uri": "file:///definition.m" },
                "position": { "line": 0, "character": 15 }
            }),
        ));
        assert_eq!(closed[0]["result"], Value::Null);
    }

    #[test]
    fn content_length_connection_serves_navigation_end_to_end() {
        let messages = [
            request(1, "initialize", json!({})),
            notification(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": "file:///定义.m",
                        "languageId": "matlab",
                        "version": 2,
                        "text": "function y = helper(x)\r\ny = x;\r\nend\r\n"
                    }
                }),
            ),
            notification(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": "file:///use.m",
                        "languageId": "matlab",
                        "version": 5,
                        "text": "% 😀\r\nvalue = helper(1);\r\n"
                    }
                }),
            ),
            request(
                2,
                "textDocument/definition",
                json!({
                    "textDocument": { "uri": "file:///use.m" },
                    "position": { "line": 1, "character": 10 }
                }),
            ),
            request(3, "shutdown", Value::Null),
            notification("exit", Value::Null),
        ];
        let mut input = Vec::new();
        for message in messages {
            let body = serde_json::to_vec(&message).expect("serialize request");
            write_message(&mut input, &body).expect("Content-Length frame");
        }
        assert!(
            String::from_utf8_lossy(&input).contains("Content-Length:"),
            "input should use Content-Length framing"
        );

        let mut reader = BufReader::with_capacity(3, Cursor::new(input));
        let mut output = Vec::new();
        assert_eq!(
            run_connection(&mut reader, &mut output).expect("connection"),
            RunOutcome::CleanExit
        );

        let mut output = Cursor::new(output);
        let mut responses = Vec::new();
        while let Some(body) = read_message(&mut output).expect("response frame") {
            responses.push(serde_json::from_slice::<Value>(&body).expect("response JSON"));
        }
        let definition = responses
            .iter()
            .find(|response| response["id"] == 2)
            .expect("definition response");
        assert_eq!(definition["result"]["uri"], "file:///定义.m");
        assert_eq!(definition["result"]["range"]["start"]["character"], 13);
        assert!(responses.iter().any(|response| response["id"] == 3));
    }

    #[test]
    fn content_length_connection_serves_advanced_ide_features() {
        let messages = [
            request(1, "initialize", json!({})),
            notification(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": "file:///advanced.m",
                        "languageId": "matlab",
                        "version": 11,
                        "text": "% 😀\r\nfunction y = calc(x)  \r\ny = x + 1; \r\nend\r\nend\r\n"
                    }
                }),
            ),
            request(
                2,
                "textDocument/semanticTokens/full",
                json!({ "textDocument": { "uri": "file:///advanced.m" } }),
            ),
            request(3, "workspace/symbol", json!({ "query": "calc" })),
            request(
                4,
                "textDocument/formatting",
                json!({
                    "textDocument": { "uri": "file:///advanced.m" },
                    "options": { "tabSize": 4, "insertSpaces": true }
                }),
            ),
            advanced_code_action_request(),
            advanced_resolve_request(),
            request(7, "shutdown", Value::Null),
            notification("exit", Value::Null),
        ];
        let mut input = Vec::new();
        for message in messages {
            write_message(
                &mut input,
                &serde_json::to_vec(&message).expect("serialize advanced request"),
            )
            .expect("advanced Content-Length frame");
        }
        let mut reader = BufReader::with_capacity(1, Cursor::new(input));
        let mut output = Vec::new();
        assert_eq!(
            run_connection(&mut reader, &mut output).expect("advanced connection"),
            RunOutcome::CleanExit
        );

        let mut output = Cursor::new(output);
        let mut responses = Vec::new();
        while let Some(body) = read_message(&mut output).expect("advanced response frame") {
            responses.push(serde_json::from_slice::<Value>(&body).expect("advanced response JSON"));
        }
        let response = |id| {
            responses
                .iter()
                .find(|response| response["id"] == id)
                .expect("response id")
        };
        assert!(
            !response(2)["result"]["data"]
                .as_array()
                .expect("semantic data")
                .is_empty()
        );
        assert_eq!(response(3)["result"][0]["name"], "calc");
        assert!(
            response(4)["result"][0]["newText"]
                .as_str()
                .is_some_and(|text| text.ends_with('\n') && !text.contains("  \n"))
        );
        assert_eq!(
            response(5)["result"][0]["edit"]["documentChanges"][0]["textDocument"]["version"],
            11
        );
        assert!(
            response(6)["result"]["documentation"]["value"]
                .as_str()
                .is_some_and(|documentation| documentation.contains("current document"))
        );
        assert_eq!(response(6)["result"]["textEdit"]["newText"], "calc");
        assert!(responses.iter().any(|response| response["id"] == 7));
    }

    #[test]
    fn connection_obeys_shutdown_then_exit_lifecycle() {
        let messages = [
            request(1, "initialize", json!({})),
            request(2, "shutdown", Value::Null),
            notification("exit", Value::Null),
        ];
        let mut input = Vec::new();
        for message in messages {
            write_message(
                &mut input,
                &serde_json::to_vec(&message).expect("serialize request"),
            )
            .expect("frame request");
        }
        let mut reader = BufReader::with_capacity(2, Cursor::new(input));
        let mut output = Vec::new();
        let outcome = run_connection(&mut reader, &mut output).expect("connection should run");
        assert_eq!(outcome, RunOutcome::CleanExit);

        let mut output = Cursor::new(output);
        let first = read_message(&mut output)
            .expect("read initialize response")
            .expect("initialize response");
        let second = read_message(&mut output)
            .expect("read shutdown response")
            .expect("shutdown response");
        assert_eq!(
            serde_json::from_slice::<Value>(&first).expect("JSON")["id"],
            1
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&second).expect("JSON")["id"],
            2
        );
        assert!(read_message(&mut output).expect("output EOF").is_none());
    }

    #[test]
    fn exit_without_shutdown_is_reported() {
        let mut server = Server::new();
        let _ = server.handle_message(&request(1, "initialize", json!({})));
        assert!(
            server
                .handle_message(&notification("exit", Value::Null))
                .is_empty()
        );
        assert_eq!(server.state(), ServerState::Exited);
        assert_eq!(server.exit_outcome(), RunOutcome::ExitWithoutShutdown);
    }

    #[test]
    fn malformed_json_receives_parse_error() {
        let mut server = Server::new();
        let response = server.handle_json(b"{");
        assert_eq!(response[0]["error"]["code"], -32700);
        assert_eq!(response[0]["id"], Value::Null);
    }
}

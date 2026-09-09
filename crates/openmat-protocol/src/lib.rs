#![doc = "Versioned logical messages shared by clients, servers, and kernels."]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Additive typed model, codec, and validators for `openmat-kernel-v1`.
pub mod kernel_v1;
/// Additive aggregate-observation model and codec for `openmat-kernel-v2`.
pub mod kernel_v2;
/// Optimistic workspace-mutation messages for `openmat-kernel-v3`.
pub mod kernel_v3;

/// The protocol identifier frozen by `spec/protocol/kernel-v0.md`.
pub const PROTOCOL_V0: &str = "openmat-kernel-v0";

/// A defensive upper bound for values carried by a matrix preview.
pub const MAX_PREVIEW_ELEMENTS: u64 = 4_096;

/// The top-level kind of a protocol envelope.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageKind {
    /// A client request.
    Request,
    /// A response correlated to a request.
    Response,
    /// An unsolicited kernel event.
    Event,
}

/// A request envelope sent by a client.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    /// Protocol identifier.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque identifier unique within the session.
    pub message_id: String,
    /// Always [`MessageKind::Request`].
    pub kind: MessageKind,
    /// Typed request data.
    pub request: Request,
}

impl RequestEnvelope {
    /// Creates a v0 request envelope.
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        request: Request,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V0.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Request,
            request,
        }
    }

    /// Validates envelope invariants and bounded request fields.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when a required identifier, kind, protocol,
    /// or request-specific bound is invalid.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Request,
        )?;
        self.request.validate()
    }
}

/// A response envelope sent by a kernel.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    /// Protocol identifier.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque response identifier.
    pub message_id: String,
    /// Always [`MessageKind::Response`].
    pub kind: MessageKind,
    /// Request message identifier being answered.
    pub reply_to: String,
    /// Whether the request succeeded.
    pub ok: bool,
    /// Successful result, when `ok` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResponseResult>,
    /// Structured failure, when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl ResponseEnvelope {
    /// Creates a successful response correlated to `request`.
    #[must_use]
    pub fn success(
        request: &RequestEnvelope,
        message_id: impl Into<String>,
        result: ResponseResult,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V0.to_owned(),
            session_id: request.session_id.clone(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: request.message_id.clone(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Creates a failed response correlated to `request`.
    #[must_use]
    pub fn failure(
        request: &RequestEnvelope,
        message_id: impl Into<String>,
        error: ProtocolError,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V0.to_owned(),
            session_id: request.session_id.clone(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: request.message_id.clone(),
            ok: false,
            result: None,
            error: Some(error),
        }
    }

    /// Returns whether this response is correlated to `request` in the same
    /// session and protocol.
    #[must_use]
    pub fn is_reply_to(&self, request: &RequestEnvelope) -> bool {
        self.protocol == request.protocol
            && self.session_id == request.session_id
            && self.reply_to == request.message_id
    }

    /// Validates envelope and success/failure invariants.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] if the envelope is malformed or its result
    /// and error fields disagree with `ok`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Response,
        )?;
        require_non_empty("replyTo", &self.reply_to)?;
        match (self.ok, &self.result, &self.error) {
            (true, Some(result), None) => result.validate(),
            (false, None, Some(_)) => Ok(()),
            _ => Err(ValidationError::new(
                "response_shape",
                "successful responses require only result; failed responses require only error",
            )),
        }
    }
}

/// An event envelope sent by a kernel.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    /// Protocol identifier.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque event identifier.
    pub message_id: String,
    /// Always [`MessageKind::Event`].
    pub kind: MessageKind,
    /// Typed or forward-compatible unknown event payload.
    pub event: Event,
}

impl EventEnvelope {
    /// Creates a v0 event envelope.
    #[must_use]
    pub fn new(session_id: impl Into<String>, message_id: impl Into<String>, event: Event) -> Self {
        Self {
            protocol: PROTOCOL_V0.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Event,
            event,
        }
    }

    /// Validates event envelope invariants.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when a header field is malformed.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Event,
        )
    }
}

/// Any server-to-client protocol message.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ServerMessage {
    /// A response to a request.
    Response(ResponseEnvelope),
    /// An unsolicited event.
    Event(EventEnvelope),
}

/// Kernel request payloads.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "params", rename_all = "camelCase")]
pub enum Request {
    /// Negotiate the protocol and capabilities.
    Initialize(InitializeRequest),
    /// Execute UTF-8 source.
    Execute(ExecuteRequest),
    /// Cooperatively interrupt an active execution.
    Interrupt(InterruptRequest),
    /// Inspect a bounded part of a workspace value.
    Inspect(InspectRequest),
    /// List workspace summaries.
    ListWorkspace(ListWorkspaceRequest),
    /// Terminate the kernel orderly.
    Shutdown(ShutdownRequest),
}

impl Request {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Initialize(request) => {
                require_non_empty("client.name", &request.client.name)?;
                require_non_empty("client.version", &request.client.version)?;
                if request.supported_protocols.iter().any(String::is_empty) {
                    return Err(ValidationError::new(
                        "supported_protocols",
                        "supported protocol identifiers must not be empty",
                    ));
                }
                request.capabilities.validate()
            }
            Self::Execute(request) => require_non_empty("sourceName", &request.source_name),
            Self::Interrupt(_) | Self::ListWorkspace(_) | Self::Shutdown(_) => Ok(()),
            Self::Inspect(request) => {
                require_non_empty("name", &request.name)?;
                request.range.validate()?;
                if request.max_elements == 0 || request.max_elements > MAX_PREVIEW_ELEMENTS {
                    return Err(ValidationError::new(
                        "preview_bound",
                        format!("maxElements must be between 1 and {MAX_PREVIEW_ELEMENTS}"),
                    ));
                }
                Ok(())
            }
        }
    }

    /// Returns true for requests that may bypass the sequential execution
    /// queue.
    #[must_use]
    pub const fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Initialize(_) | Self::Interrupt(_) | Self::Shutdown(_)
        )
    }
}

/// Initialization and capability-negotiation request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    /// Client implementation identity.
    pub client: ImplementationInfo,
    /// Protocol identifiers the client can speak, in preference order.
    #[serde(default)]
    pub supported_protocols: Vec<String>,
    /// Client-supported features and limits.
    #[serde(default)]
    pub capabilities: Capabilities,
}

/// An implementation name and version.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImplementationInfo {
    /// Human-readable implementation name.
    pub name: String,
    /// Implementation version.
    pub version: String,
}

/// Negotiable kernel capabilities.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Supported execution modes.
    #[serde(default)]
    pub execution_modes: Vec<ExecutionMode>,
    /// Supported MIME representations for display events.
    #[serde(default)]
    pub display_mime_types: Vec<String>,
    /// Maximum preview values the peer accepts.
    #[serde(default = "default_preview_elements")]
    pub max_preview_elements: u64,
    /// Cooperative interrupt support.
    #[serde(default)]
    pub interrupt: bool,
    /// Workspace delta event support.
    #[serde(default)]
    pub workspace_delta: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            execution_modes: Vec::new(),
            display_mime_types: Vec::new(),
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            interrupt: false,
            workspace_delta: false,
        }
    }
}

impl Capabilities {
    /// Computes the common capability subset and the stricter preview limit.
    #[must_use]
    pub fn negotiate(&self, peer: &Self) -> Self {
        let execution_modes = self
            .execution_modes
            .iter()
            .copied()
            .filter(|mode| peer.execution_modes.contains(mode))
            .collect();
        let display_mime_types = self
            .display_mime_types
            .iter()
            .filter(|mime| peer.display_mime_types.contains(mime))
            .cloned()
            .collect();
        Self {
            execution_modes,
            display_mime_types,
            max_preview_elements: self
                .max_preview_elements
                .min(peer.max_preview_elements)
                .min(MAX_PREVIEW_ELEMENTS),
            interrupt: self.interrupt && peer.interrupt,
            workspace_delta: self.workspace_delta && peer.workspace_delta,
        }
    }

    fn validate(&self) -> Result<(), ValidationError> {
        if self.max_preview_elements == 0 || self.max_preview_elements > MAX_PREVIEW_ELEMENTS {
            return Err(ValidationError::new(
                "preview_bound",
                format!("maxPreviewElements must be between 1 and {MAX_PREVIEW_ELEMENTS}"),
            ));
        }
        Ok(())
    }
}

const fn default_preview_elements() -> u64 {
    MAX_PREVIEW_ELEMENTS
}

/// Source execution request.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRequest {
    /// UTF-8 source text.
    pub code: String,
    /// Logical source identifier used by diagnostics.
    pub source_name: String,
    /// Requested execution behavior.
    pub mode: ExecutionMode,
}

/// Execution mode frozen by v0.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionMode {
    /// Execute as a file.
    File,
    /// Execute as an interactive cell.
    Cell,
    /// Execute using REPL semantics.
    Repl,
}

/// Cooperative interrupt request.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct InterruptRequest {}

/// Bounded workspace inspection request.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectRequest {
    /// Workspace variable name.
    pub name: String,
    /// One-based selected range.
    pub range: MatrixRange,
    /// Maximum number of scalar values returned.
    #[serde(default = "default_preview_elements")]
    pub max_elements: u64,
}

/// Workspace-list request.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct ListWorkspaceRequest {}

/// Orderly shutdown request.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct ShutdownRequest {}

/// Successful response payloads.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ResponseResult {
    /// Negotiated initialization result.
    Initialize(InitializeResult),
    /// Execution completion result.
    Execute(ExecuteResult),
    /// Interrupt acknowledgement.
    Interrupt(InterruptResult),
    /// Bounded inspection result.
    Inspect(MatrixPreview),
    /// Workspace summaries.
    ListWorkspace(WorkspaceSummary),
    /// Shutdown acknowledgement.
    Shutdown(ShutdownResult),
}

impl ResponseResult {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Inspect(preview) => preview.validate(),
            Self::Initialize(_)
            | Self::Execute(_)
            | Self::Interrupt(_)
            | Self::ListWorkspace(_)
            | Self::Shutdown(_) => Ok(()),
        }
    }
}

/// Successful initialization result.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// Selected protocol identifier.
    pub negotiated_protocol: String,
    /// Kernel implementation identity.
    pub implementation: ImplementationInfo,
    /// Negotiated common capabilities.
    pub capabilities: Capabilities,
}

/// Successful source execution result.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteResult {
    /// Whether cooperative cancellation ended execution.
    pub interrupted: bool,
}

/// Interrupt acknowledgement.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptResult {
    /// True when an active execution accepted cancellation.
    pub accepted: bool,
}

/// Shutdown acknowledgement.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct ShutdownResult {}

/// A stable structured protocol or engine error.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolError {
    /// Stable `OpenMat`-owned error category.
    pub category: String,
    /// Human-readable, non-ABI error prose.
    pub message: String,
    /// Optional compiler or runtime diagnostics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl ProtocolError {
    /// Creates an error without diagnostics.
    #[must_use]
    pub fn new(category: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            message: message.into(),
            diagnostics: Vec::new(),
        }
    }
}

/// A structured compiler or runtime diagnostic.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// Stable `OpenMat` diagnostic code, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Diagnostic severity.
    pub severity: DiagnosticSeverity,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Primary source range, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<SourceRange>,
    /// Additional labeled ranges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedDiagnostic>,
}

/// Diagnostic severity levels.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    /// Execution-stopping error.
    Error,
    /// Non-fatal warning.
    Warning,
    /// Informational message.
    Information,
    /// Optional hint.
    Hint,
}

/// A byte range in a named UTF-8 source.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRange {
    /// Logical source identifier.
    pub source_name: String,
    /// Inclusive byte offset.
    pub start: u64,
    /// Exclusive byte offset.
    pub end: u64,
}

/// A labeled secondary diagnostic location.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedDiagnostic {
    /// Human-readable label.
    pub message: String,
    /// Related source range.
    pub range: SourceRange,
}

/// A summary of workspace variables without their values.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    /// Variables in deterministic name order.
    pub variables: Vec<VariableSummary>,
}

/// Metadata for one workspace variable.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableSummary {
    /// Workspace variable name.
    pub name: String,
    /// `OpenMat`/MATLAB-style value class.
    pub class: String,
    /// Full array dimensions.
    pub dimensions: Vec<u64>,
    /// Whether the value has a complex representation.
    #[serde(default)]
    pub complex: bool,
    /// Estimated storage size, when cheaply available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

/// A one-based multidimensional matrix selection.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixRange {
    /// One-based start index per dimension.
    pub start: Vec<u64>,
    /// Selected extent per dimension.
    pub size: Vec<u64>,
}

impl MatrixRange {
    /// Validates rank and one-based indices.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] for a rank mismatch, zero start index, or
    /// an extent whose product overflows `u64`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.start.len() != self.size.len() || self.start.is_empty() {
            return Err(ValidationError::new(
                "matrix_range",
                "start and size must have the same non-zero rank",
            ));
        }
        if self.start.contains(&0) {
            return Err(ValidationError::new(
                "matrix_range",
                "matrix range starts are one-based",
            ));
        }
        checked_element_count(&self.size).map(|_| ())
    }
}

/// A bounded JSON matrix preview.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixPreview {
    /// Value class.
    pub class: String,
    /// Full value dimensions.
    pub dimensions: Vec<u64>,
    /// One-based range represented by `values`.
    pub selected_range: MatrixRange,
    /// Column-major scalar values, bounded by [`MAX_PREVIEW_ELEMENTS`].
    pub values: Vec<PreviewValue>,
    /// Truncation metadata for omitted values.
    pub truncation: PreviewTruncation,
}

impl MatrixPreview {
    /// Validates dimensions, selected range, value count, and global bounds.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] if this preview could transfer an
    /// unbounded value or contains inconsistent shape metadata.
    pub fn validate(&self) -> Result<(), ValidationError> {
        require_non_empty("class", &self.class)?;
        self.selected_range.validate()?;
        if self.dimensions.len() != self.selected_range.start.len() {
            return Err(ValidationError::new(
                "preview_shape",
                "dimensions and selectedRange must have the same rank",
            ));
        }
        for ((start, size), dimension) in self
            .selected_range
            .start
            .iter()
            .zip(&self.selected_range.size)
            .zip(&self.dimensions)
        {
            let end = start
                .checked_add(*size)
                .and_then(|value| value.checked_sub(1))
                .ok_or_else(|| ValidationError::new("preview_shape", "selected range overflows"))?;
            if *size > 0 && end > *dimension {
                return Err(ValidationError::new(
                    "preview_shape",
                    "selected range exceeds full dimensions",
                ));
            }
        }
        let selected_count = checked_element_count(&self.selected_range.size)?;
        let value_count = u64::try_from(self.values.len()).map_err(|_| {
            ValidationError::new("preview_bound", "preview value count does not fit u64")
        })?;
        if value_count > MAX_PREVIEW_ELEMENTS || value_count > selected_count {
            return Err(ValidationError::new(
                "preview_bound",
                format!(
                    "preview has {value_count} values for a selection of {selected_count}; limit is {MAX_PREVIEW_ELEMENTS}"
                ),
            ));
        }
        for value in &self.values {
            value.validate()?;
        }
        let missing = selected_count - value_count;
        if self.truncation.truncated != (missing > 0) || self.truncation.omitted_elements != missing
        {
            return Err(ValidationError::new(
                "preview_truncation",
                "truncation metadata does not match the selected range",
            ));
        }
        Ok(())
    }
}

/// A JSON-safe scalar preview value.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PreviewValue {
    /// A finite real number.
    Number {
        /// Numeric value.
        value: f64,
    },
    /// A complex number.
    Complex {
        /// Real component.
        real: f64,
        /// Imaginary component.
        imaginary: f64,
    },
    /// A logical scalar.
    Logical {
        /// Boolean value.
        value: bool,
    },
    /// A UTF-8 string scalar.
    Text {
        /// Text value.
        value: String,
    },
    /// A numeric value that JSON cannot encode directly, such as `NaN`.
    Special {
        /// Stable spelling such as `nan`, `infinity`, or `negativeInfinity`.
        value: String,
    },
    /// A missing or unsupported value whose full payload was not transferred.
    Missing,
}

impl PreviewValue {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Number { value } if !value.is_finite() => Err(ValidationError::new(
                "preview_value",
                "number preview values must be finite; use a stable special spelling",
            )),
            Self::Complex { real, imaginary } if !real.is_finite() || !imaginary.is_finite() => {
                Err(ValidationError::new(
                    "preview_value",
                    "complex preview components must be finite; use stable special values instead",
                ))
            }
            Self::Special { value }
                if !matches!(value.as_str(), "nan" | "infinity" | "negativeInfinity") =>
            {
                Err(ValidationError::new(
                    "preview_value",
                    "special preview values must be nan, infinity, or negativeInfinity",
                ))
            }
            Self::Number { .. }
            | Self::Complex { .. }
            | Self::Logical { .. }
            | Self::Text { .. }
            | Self::Special { .. }
            | Self::Missing => Ok(()),
        }
    }
}

/// Matrix-preview truncation metadata.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTruncation {
    /// Whether values were omitted from the selected range.
    pub truncated: bool,
    /// Number of omitted scalar values.
    pub omitted_elements: u64,
}

/// Kernel event payloads. Unknown types deserialize to [`Event::Unknown`] so a
/// v0 peer can ignore them without rejecting the envelope.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Kernel lifecycle state changed.
    Status(StatusEvent),
    /// UTF-8 standard output or error output.
    Stream(StreamEvent),
    /// MIME-keyed display data.
    Display(DisplayEvent),
    /// Structured compiler or runtime diagnostic.
    Diagnostic(Diagnostic),
    /// Workspace variables changed.
    WorkspaceDelta(WorkspaceDeltaEvent),
    /// A forward-compatible event type unknown to this implementation.
    Unknown {
        /// Unrecognized event type.
        event_type: String,
        /// Uninterpreted JSON data.
        data: Value,
    },
}

impl Event {
    /// Returns the serialized event type.
    #[must_use]
    pub fn event_type(&self) -> &str {
        match self {
            Self::Status(_) => "status",
            Self::Stream(_) => "stream",
            Self::Display(_) => "display",
            Self::Diagnostic(_) => "diagnostic",
            Self::WorkspaceDelta(_) => "workspaceDelta",
            Self::Unknown { event_type, .. } => event_type,
        }
    }

    /// Returns true when this implementation recognizes the event type.
    #[must_use]
    pub const fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown { .. })
    }
}

impl Serialize for Event {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", self.event_type())?;
        match self {
            Self::Status(data) => map.serialize_entry("data", data)?,
            Self::Stream(data) => map.serialize_entry("data", data)?,
            Self::Display(data) => map.serialize_entry("data", data)?,
            Self::Diagnostic(data) => map.serialize_entry("data", data)?,
            Self::WorkspaceDelta(data) => map.serialize_entry("data", data)?,
            Self::Unknown { data, .. } => map.serialize_entry("data", data)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawEvent {
            #[serde(rename = "type")]
            event_type: String,
            #[serde(default)]
            data: Value,
        }

        let raw = RawEvent::deserialize(deserializer)?;
        match raw.event_type.as_str() {
            "status" => serde_json::from_value(raw.data)
                .map(Self::Status)
                .map_err(D::Error::custom),
            "stream" => serde_json::from_value(raw.data)
                .map(Self::Stream)
                .map_err(D::Error::custom),
            "display" => serde_json::from_value(raw.data)
                .map(Self::Display)
                .map_err(D::Error::custom),
            "diagnostic" => serde_json::from_value(raw.data)
                .map(Self::Diagnostic)
                .map_err(D::Error::custom),
            "workspaceDelta" => serde_json::from_value(raw.data)
                .map(Self::WorkspaceDelta)
                .map_err(D::Error::custom),
            _ => Ok(Self::Unknown {
                event_type: raw.event_type,
                data: raw.data,
            }),
        }
    }
}

/// Kernel lifecycle event.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEvent {
    /// New kernel status.
    pub status: KernelStatus,
}

/// Kernel lifecycle states frozen by v0.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KernelStatus {
    /// Kernel process or session is starting.
    Starting,
    /// Kernel can accept sequential work.
    Idle,
    /// Kernel is executing a request.
    Busy,
    /// Active execution has received an interrupt.
    Interrupted,
    /// Kernel has terminated.
    Dead,
}

/// UTF-8 stream output event.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEvent {
    /// Output stream.
    pub stream: StreamKind,
    /// UTF-8 text chunk.
    pub text: String,
}

/// Stream output channel.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// MIME-keyed display data.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayEvent {
    /// Display representations keyed by MIME type.
    pub representations: BTreeMap<String, String>,
}

/// Summary-only workspace change event.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDeltaEvent {
    /// Newly created variables.
    #[serde(default)]
    pub added: Vec<VariableSummary>,
    /// Existing variables whose summaries changed.
    #[serde(default)]
    pub changed: Vec<VariableSummary>,
    /// Removed variable names.
    #[serde(default)]
    pub removed: Vec<String>,
}

/// Protocol validation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    category: &'static str,
    message: String,
}

impl ValidationError {
    fn new(category: &'static str, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    /// Returns the stable validation category.
    #[must_use]
    pub const fn category(&self) -> &'static str {
        self.category
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl Error for ValidationError {}

fn validate_header(
    protocol: &str,
    session_id: &str,
    message_id: &str,
    actual_kind: MessageKind,
    expected_kind: MessageKind,
) -> Result<(), ValidationError> {
    if protocol != PROTOCOL_V0 {
        return Err(ValidationError::new(
            "protocol",
            format!("expected {PROTOCOL_V0}, got {protocol}"),
        ));
    }
    require_non_empty("sessionId", session_id)?;
    require_non_empty("messageId", message_id)?;
    if actual_kind != expected_kind {
        return Err(ValidationError::new(
            "kind",
            format!("expected {expected_kind:?}, got {actual_kind:?}"),
        ));
    }
    Ok(())
}

fn require_non_empty(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        Err(ValidationError::new(
            "required_field",
            format!("{field} must not be empty"),
        ))
    } else {
        Ok(())
    }
}

fn checked_element_count(dimensions: &[u64]) -> Result<u64, ValidationError> {
    dimensions.iter().try_fold(1_u64, |product, dimension| {
        product.checked_mul(*dimension).ok_or_else(|| {
            ValidationError::new("matrix_range", "matrix element count overflows u64")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize_request() -> RequestEnvelope {
        RequestEnvelope::new(
            "session-1",
            "request-1",
            Request::Initialize(InitializeRequest {
                client: ImplementationInfo {
                    name: "protocol-test".to_owned(),
                    version: "1.0".to_owned(),
                },
                supported_protocols: vec![PROTOCOL_V0.to_owned()],
                capabilities: Capabilities {
                    execution_modes: vec![ExecutionMode::File, ExecutionMode::Repl],
                    display_mime_types: vec!["text/plain".to_owned()],
                    max_preview_elements: 128,
                    interrupt: true,
                    workspace_delta: true,
                },
            }),
        )
    }

    fn single_value_preview(value: PreviewValue) -> MatrixPreview {
        MatrixPreview {
            class: "double".to_owned(),
            dimensions: vec![1, 1],
            selected_range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            values: vec![value],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        }
    }

    #[test]
    fn request_json_round_trip_ignores_unknown_fields() {
        let request = initialize_request();
        let mut json = serde_json::to_value(&request).expect("serialize request");
        json.as_object_mut()
            .expect("request object")
            .insert("futureEnvelopeField".to_owned(), Value::Bool(true));
        json.get_mut("request")
            .and_then(Value::as_object_mut)
            .expect("request payload")
            .insert("futureRequestField".to_owned(), Value::Bool(true));

        let decoded: RequestEnvelope = serde_json::from_value(json).expect("decode request");
        assert_eq!(decoded, request);
        decoded.validate().expect("valid request");
    }

    #[test]
    fn response_round_trip_preserves_reply_correlation() {
        let request = initialize_request();
        let response = ResponseEnvelope::success(
            &request,
            "response-1",
            ResponseResult::Initialize(InitializeResult {
                negotiated_protocol: PROTOCOL_V0.to_owned(),
                implementation: ImplementationInfo {
                    name: "openmat-kernel".to_owned(),
                    version: "0.1.0".to_owned(),
                },
                capabilities: Capabilities::default(),
            }),
        );
        let json = serde_json::to_string(&response).expect("serialize response");
        let decoded: ResponseEnvelope = serde_json::from_str(&json).expect("decode response");

        decoded.validate().expect("valid response");
        assert!(decoded.is_reply_to(&request));
        assert_eq!(decoded.reply_to, "request-1");
    }

    #[test]
    fn unknown_event_type_is_preserved_and_ignored_by_known_handlers() {
        let json = r#"{
            "protocol":"openmat-kernel-v0",
            "sessionId":"session-1",
            "messageId":"event-1",
            "kind":"event",
            "event":{"type":"futureEvent","data":{"answer":42}},
            "futureEnvelopeField":true
        }"#;
        let event: EventEnvelope = serde_json::from_str(json).expect("decode future event");
        event.validate().expect("valid event envelope");
        assert!(!event.event.is_known());
        assert_eq!(event.event.event_type(), "futureEvent");
        assert!(matches!(event.event, Event::Unknown { .. }));
    }

    #[test]
    fn validation_rejects_bad_kind_and_response_shape() {
        let mut request = initialize_request();
        request.kind = MessageKind::Event;
        assert_eq!(request.validate().expect_err("bad kind").category(), "kind");

        let original = initialize_request();
        let mut response = ResponseEnvelope::success(
            &original,
            "response-1",
            ResponseResult::Shutdown(ShutdownResult {}),
        );
        response.error = Some(ProtocolError::new("test", "invalid shape"));
        assert_eq!(
            response.validate().expect_err("bad shape").category(),
            "response_shape"
        );
    }

    #[test]
    fn bounded_preview_validation_catches_shape_and_limit_errors() {
        let preview = MatrixPreview {
            class: "double".to_owned(),
            dimensions: vec![100, 100],
            selected_range: MatrixRange {
                start: vec![1, 1],
                size: vec![2, 3],
            },
            values: vec![PreviewValue::Number { value: 1.0 }; 5],
            truncation: PreviewTruncation {
                truncated: true,
                omitted_elements: 1,
            },
        };
        preview.validate().expect("valid bounded preview");

        let mut too_large = preview;
        too_large.dimensions = vec![MAX_PREVIEW_ELEMENTS + 1, 100];
        too_large.selected_range.size = vec![MAX_PREVIEW_ELEMENTS + 1, 1];
        too_large.values = vec![
            PreviewValue::Missing;
            usize::try_from(MAX_PREVIEW_ELEMENTS + 1)
                .expect("test limit fits usize")
        ];
        too_large.truncation = PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        };
        assert_eq!(
            too_large
                .validate()
                .expect_err("preview is too large")
                .category(),
            "preview_bound"
        );
    }

    #[test]
    fn preview_values_reject_non_finite_numbers_and_unstable_specials() {
        let invalid_values = [
            PreviewValue::Number { value: f64::NAN },
            PreviewValue::Number {
                value: f64::INFINITY,
            },
            PreviewValue::Number {
                value: f64::NEG_INFINITY,
            },
            PreviewValue::Complex {
                real: f64::INFINITY,
                imaginary: 0.0,
            },
            PreviewValue::Complex {
                real: 0.0,
                imaginary: f64::NAN,
            },
            PreviewValue::Special {
                value: "NaN".to_owned(),
            },
        ];
        for value in invalid_values {
            assert_eq!(
                single_value_preview(value)
                    .validate()
                    .expect_err("non-JSON-safe preview value")
                    .category(),
                "preview_value"
            );
        }

        for spelling in ["nan", "infinity", "negativeInfinity"] {
            let preview = single_value_preview(PreviewValue::Special {
                value: spelling.to_owned(),
            });
            preview.validate().expect("stable special spelling");
            let json = serde_json::to_string(&preview).expect("serialize special preview");
            let decoded: MatrixPreview =
                serde_json::from_str(&json).expect("deserialize special preview");
            assert_eq!(decoded, preview);
        }
    }

    #[test]
    fn response_validation_recursively_rejects_invalid_inspect_preview() {
        let request = initialize_request();
        let response = ResponseEnvelope::success(
            &request,
            "response-invalid-preview",
            ResponseResult::Inspect(single_value_preview(PreviewValue::Number {
                value: f64::NAN,
            })),
        );

        assert_eq!(
            response
                .validate()
                .expect_err("inspect response must validate its preview")
                .category(),
            "preview_value"
        );
    }

    #[test]
    fn capability_negotiation_uses_intersection_and_stricter_bound() {
        let server = Capabilities {
            execution_modes: vec![ExecutionMode::File, ExecutionMode::Cell],
            display_mime_types: vec!["text/plain".to_owned(), "image/png".to_owned()],
            max_preview_elements: 512,
            interrupt: true,
            workspace_delta: true,
        };
        let client = Capabilities {
            execution_modes: vec![ExecutionMode::Cell, ExecutionMode::Repl],
            display_mime_types: vec!["text/plain".to_owned()],
            max_preview_elements: 64,
            interrupt: false,
            workspace_delta: true,
        };

        assert_eq!(
            server.negotiate(&client),
            Capabilities {
                execution_modes: vec![ExecutionMode::Cell],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: 64,
                interrupt: false,
                workspace_delta: true,
            }
        );
    }
}

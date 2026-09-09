use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::limits::{validate_positive_safe, validate_safe};
use crate::{
    ErrorCategory, FigureDelta, FigureSnapshot, GraphicsLimits, ProtocolFailure, RenderBackend,
    ValidationError,
};

/// Frozen graphics-v1 protocol identifier.
pub const PROTOCOL_V1: &str = "openmat-graphics-v1";
/// Graphics-v2 protocol identifier.
pub const PROTOCOL_V2: &str = "openmat-graphics-v2";
/// Graphics-v3 protocol identifier.
pub const PROTOCOL_V3: &str = "openmat-graphics-v3";
/// Graphics-v4 protocol identifier.
pub const PROTOCOL_V4: &str = "openmat-graphics-v4";
/// Backward-compatible alias for the frozen graphics-v1 identifier.
pub const PROTOCOL: &str = PROTOCOL_V1;

/// Version selected by the WebSocket endpoint and every text envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphicsProtocol {
    /// Frozen `/graphics/v1` contract.
    V1,
    /// `/graphics/v2`, a strict scene/binary superset of v1.
    V2,
    /// `/graphics/v3`, adding explicit multi-Axes interaction targets.
    V3,
    /// `/graphics/v4`, adding logarithmic axes and axis presentation state.
    V4,
}

impl GraphicsProtocol {
    /// Returns the exact text-envelope protocol identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::V1 => PROTOCOL_V1,
            Self::V2 => PROTOCOL_V2,
            Self::V3 => PROTOCOL_V3,
            Self::V4 => PROTOCOL_V4,
        }
    }

    /// Returns the origin-relative WebSocket endpoint.
    #[must_use]
    pub const fn endpoint(self) -> &'static str {
        match self {
            Self::V1 => "/graphics/v1",
            Self::V2 => "/graphics/v2",
            Self::V3 => "/graphics/v3",
            Self::V4 => "/graphics/v4",
        }
    }

    /// Parses one exact supported protocol identifier.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        match identifier {
            PROTOCOL_V1 => Some(Self::V1),
            PROTOCOL_V2 => Some(Self::V2),
            PROTOCOL_V3 => Some(Self::V3),
            PROTOCOL_V4 => Some(Self::V4),
            _ => None,
        }
    }
}

/// Common envelope kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    /// Client request.
    Request,
    /// Server response.
    Response,
    /// Server event.
    Event,
}

/// Secret attachment capability carried only by initialize. Its `Debug`
/// implementation is permanently redacted.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AttachToken(String);

impl AttachToken {
    /// Creates a wire token from the Server-owned secret representation.
    #[must_use]
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// Borrows the secret for constant-time Server validation or MIME output.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AttachToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AttachToken([REDACTED])")
    }
}

/// Graphics client implementation identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    /// Non-empty implementation name.
    pub name: String,
    /// Non-empty implementation version.
    pub version: String,
}

/// Server implementation identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImplementationInfo {
    /// Non-empty implementation name.
    pub name: String,
    /// Non-empty implementation version.
    pub version: String,
}

/// Initialize request body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    /// Server-owned attachment capability.
    pub attach_token: AttachToken,
    /// Client identity.
    pub client: ClientInfo,
    /// Proposed graphics-v1 bounds.
    pub capabilities: crate::ClientCapabilities,
}

/// Complete request set. Unknown request types are represented separately by
/// [`DecodedRequest::Unsupported`].
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Request {
    /// First attachment request.
    Initialize(InitializeRequest),
    /// Complete snapshot request.
    GetSnapshot(GetSnapshotRequest),
    /// Immutable-buffer transfer request.
    GetBuffer(GetBufferRequest),
    /// Idempotent lease release.
    ReleaseBuffer(ReleaseBufferRequest),
    /// Interactive Figure close.
    CloseFigure(CloseFigureRequest),
    /// V2-only atomic semantic X/Y limit commit.
    SetAxesLimits(SetAxesLimitsRequest),
    /// V2-only atomic 3D camera commit.
    SetAxesCamera(SetAxesCameraRequest),
    /// Snapshot recovery after a gap.
    ResyncFigure(ResyncFigureRequest),
    /// Detach this graphics socket only.
    Shutdown(ShutdownRequest),
}

/// Complete client request envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    /// Exact identifier selected by the WebSocket endpoint.
    pub protocol: String,
    /// Opaque owning kernel session identifier.
    pub session_id: String,
    /// Non-empty, connection-unique request identifier.
    pub message_id: String,
    /// Exactly `request`.
    pub kind: MessageKind,
    /// Typed request.
    pub request: Request,
}

impl RequestEnvelope {
    /// Creates one typed request envelope.
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        request: Request,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Request,
            request,
        }
    }

    /// Creates one typed request envelope for an explicit graphics version.
    #[must_use]
    pub fn new_for(
        protocol: GraphicsProtocol,
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        request: Request,
    ) -> Self {
        Self {
            protocol: protocol.identifier().to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Request,
            request,
        }
    }

    /// Validates envelope and request cross-fields without exposing secrets.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let protocol = parse_protocol(&self.protocol)?;
        self.validate_for(protocol)
    }

    /// Validates against the exact protocol selected by the endpoint.
    pub fn validate_for(&self, protocol: GraphicsProtocol) -> Result<(), ValidationError> {
        validate_common_for(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Request,
            protocol,
        )?;
        match &self.request {
            Request::Initialize(request) => {
                if request.attach_token.expose_secret().is_empty() {
                    return Err(invalid_request("initialize attachToken must be non-empty"));
                }
                validate_identity(&request.client.name, &request.client.version, "client")?;
                request.capabilities.validate()
            }
            Request::GetSnapshot(request) => validate_id(&request.figure_id, "figureId"),
            Request::GetBuffer(request) => validate_id(&request.buffer_id, "bufferId"),
            Request::ReleaseBuffer(request) => validate_id(&request.buffer_id, "bufferId"),
            Request::CloseFigure(request) => {
                validate_id(&request.figure_id, "figureId")?;
                validate_positive_safe(request.expected_revision, "expectedRevision")
            }
            Request::SetAxesLimits(request) => {
                if protocol == GraphicsProtocol::V1 {
                    return Err(invalid_request(
                        "setAxesLimits is unavailable in graphics-v1",
                    ));
                }
                validate_id(&request.figure_id, "figureId")?;
                validate_axes_target(protocol, request.axes_id.as_deref())?;
                validate_positive_safe(request.expected_revision, "expectedRevision")?;
                validate_axis_limits(request.x_limits, "xLimits")?;
                validate_axis_limits(request.y_limits, "yLimits")
            }
            Request::SetAxesCamera(request) => {
                if protocol == GraphicsProtocol::V1 {
                    return Err(invalid_request(
                        "setAxesCamera is unavailable in graphics-v1",
                    ));
                }
                validate_id(&request.figure_id, "figureId")?;
                validate_axes_target(protocol, request.axes_id.as_deref())?;
                validate_positive_safe(request.expected_revision, "expectedRevision")?;
                if !request.view.iter().all(|value| value.is_finite())
                    || !(-90.0..=90.0).contains(&request.view[1])
                {
                    return Err(invalid_request(
                        "view must contain finite azimuth and bounded elevation",
                    ));
                }
                if !request.camera_scale.is_finite()
                    || !(1.0 / 45.0..=170.0 / 45.0).contains(&request.camera_scale)
                {
                    return Err(invalid_request(
                        "cameraScale is outside the supported range",
                    ));
                }
                Ok(())
            }
            Request::ResyncFigure(request) => {
                validate_id(&request.figure_id, "figureId")?;
                validate_positive_safe(request.known_revision, "knownRevision")
            }
            Request::Shutdown(_) => Ok(()),
        }
    }
}

/// Snapshot request body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSnapshotRequest {
    /// Figure identifier.
    pub figure_id: String,
}

/// Buffer request body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBufferRequest {
    /// Immutable buffer identifier.
    pub buffer_id: String,
}

/// Lease-release request body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseBufferRequest {
    /// Immutable buffer identifier.
    pub buffer_id: String,
}

/// Figure-close request body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseFigureRequest {
    /// Figure identifier.
    pub figure_id: String,
    /// Exact current revision.
    pub expected_revision: u64,
}

/// V2 semantic limits commit body. Both axes are validated and committed as
/// one observable Figure transaction.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAxesLimitsRequest {
    /// Figure whose owning session performs the mutation.
    pub figure_id: String,
    /// Explicit target Axes. Forbidden in v2 and required in v3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes_id: Option<String>,
    /// Exact current Figure revision.
    pub expected_revision: u64,
    /// Finite strictly increasing horizontal limits.
    pub x_limits: [f64; 2],
    /// Finite strictly increasing vertical limits.
    pub y_limits: [f64; 2],
}

/// V2 camera gesture commit body. View and scale publish in one Figure revision.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAxesCameraRequest {
    pub figure_id: String,
    /// Explicit target Axes. Forbidden in v2 and required in v3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes_id: Option<String>,
    pub expected_revision: u64,
    pub view: [f64; 2],
    pub camera_scale: f64,
}

/// Resynchronization request body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResyncFigureRequest {
    /// Figure identifier.
    pub figure_id: String,
    /// Client's last complete revision.
    pub known_revision: u64,
}

/// Shutdown has no required members; unknown fields remain forward-compatible.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShutdownRequest {}

/// Request decode outcome preserving correlation for unsupported request types.
#[derive(Clone, Debug, PartialEq)]
pub enum DecodedRequest {
    /// Fully typed known request.
    Known(RequestEnvelope),
    /// Structurally valid envelope with an unknown request type.
    Unsupported(UnsupportedRequest),
}

/// Correlation data for a structured `graphics.unsupportedRequest` response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedRequest {
    /// Session identifier.
    pub session_id: String,
    /// Request message identifier.
    pub message_id: String,
    /// Unknown non-empty type spelling.
    pub request_type: String,
}

/// Decodes one bounded request, ignoring unknown object fields and preserving
/// unknown request types for a structured rejection.
pub fn decode_request(text: &str, max_text_bytes: u64) -> Result<DecodedRequest, ValidationError> {
    decode_request_for(text, max_text_bytes, GraphicsProtocol::V1)
}

/// Decodes one bounded request for the exact version selected by its endpoint.
pub fn decode_request_for(
    text: &str,
    max_text_bytes: u64,
    expected_protocol: GraphicsProtocol,
) -> Result<DecodedRequest, ValidationError> {
    validate_text_size(text, max_text_bytes)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|_| invalid_request("graphics request is not valid UTF-8 JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid_request("graphics request envelope must be an object"))?;
    let protocol = required_string(object, "protocol")?;
    let session_id = required_string(object, "sessionId")?;
    let message_id = required_string(object, "messageId")?;
    let kind = required_string(object, "kind")?;
    if kind != "request" {
        return Err(invalid_request("graphics request kind must be request"));
    }
    validate_common_for(
        protocol,
        session_id,
        message_id,
        MessageKind::Request,
        MessageKind::Request,
        expected_protocol,
    )?;
    let request = object
        .get("request")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_request("graphics request member must be an object"))?;
    let request_type = required_string(request, "type")?;
    if !is_known_request(request_type, expected_protocol) {
        return Ok(DecodedRequest::Unsupported(UnsupportedRequest {
            session_id: session_id.to_owned(),
            message_id: message_id.to_owned(),
            request_type: request_type.to_owned(),
        }));
    }
    let envelope: RequestEnvelope = serde_json::from_value(value)
        .map_err(|_| invalid_request("known graphics request has an invalid schema"))?;
    envelope.validate_for(expected_protocol)?;
    Ok(DecodedRequest::Known(envelope))
}

/// Negotiated server capabilities returned by initialize.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    /// Exactly WebGPU.
    pub render_backend: RenderBackend,
    /// Negotiated text bound.
    pub max_text_frame_bytes: u64,
    /// Negotiated binary bound.
    pub max_binary_frame_bytes: u64,
    /// Negotiated buffer bound.
    pub max_buffer_bytes: u64,
    /// Negotiated resident bound.
    pub max_resident_bytes: u64,
    /// Negotiated object bound.
    pub max_objects: u64,
}

impl From<GraphicsLimits> for ServerCapabilities {
    fn from(limits: GraphicsLimits) -> Self {
        Self {
            render_backend: RenderBackend::Webgpu,
            max_text_frame_bytes: limits.max_text_frame_bytes,
            max_binary_frame_bytes: limits.max_binary_frame_bytes,
            max_buffer_bytes: limits.max_buffer_bytes,
            max_resident_bytes: limits.max_resident_bytes,
            max_objects: limits.max_objects,
        }
    }
}

impl ServerCapabilities {
    /// Returns the transport limit subset.
    #[must_use]
    pub const fn limits(self) -> GraphicsLimits {
        GraphicsLimits {
            max_text_frame_bytes: self.max_text_frame_bytes,
            max_binary_frame_bytes: self.max_binary_frame_bytes,
            max_buffer_bytes: self.max_buffer_bytes,
            max_resident_bytes: self.max_resident_bytes,
            max_objects: self.max_objects,
        }
    }
}

/// Figure revision visible at the initialize serialization point.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigureSummary {
    /// Figure identifier.
    pub figure_id: String,
    /// Exact current revision and subsequent delta base.
    pub revision: u64,
}

/// Successful initialize result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// Exactly `initialize`.
    #[serde(rename = "type")]
    pub result_type: String,
    /// Server identity.
    pub implementation: ImplementationInfo,
    /// Negotiated capabilities.
    pub capabilities: ServerCapabilities,
    /// Figures/revisions serialized atomically with attachment.
    pub figures: Vec<FigureSummary>,
}

impl InitializeResult {
    /// Validates the complete initialize result and its exact revision bases.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.result_type != "initialize" {
            return Err(invalid_request("initialize result type is invalid"));
        }
        validate_identity(
            &self.implementation.name,
            &self.implementation.version,
            "implementation",
        )?;
        self.capabilities.limits().validate()?;
        let mut ids = std::collections::HashSet::with_capacity(self.figures.len());
        for figure in &self.figures {
            validate_id(&figure.figure_id, "figureId")?;
            validate_positive_safe(figure.revision, "figure revision")?;
            if !ids.insert(figure.figure_id.as_str()) {
                return Err(invalid_request(
                    "initialize figures contain a duplicate figureId",
                ));
            }
        }
        Ok(())
    }
}

/// Successful direct snapshot result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSnapshotResult {
    /// Exactly `getSnapshot`.
    #[serde(rename = "type")]
    pub result_type: String,
    /// Complete snapshot.
    pub snapshot: FigureSnapshot,
}

impl GetSnapshotResult {
    /// Validates result discriminant and complete scene.
    pub fn validate(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        if self.result_type != "getSnapshot" {
            return Err(invalid_request("getSnapshot result type is invalid"));
        }
        self.snapshot.validate(limits)
    }

    /// Validates a snapshot result for an exact graphics endpoint.
    pub fn validate_for(
        &self,
        limits: GraphicsLimits,
        protocol: GraphicsProtocol,
    ) -> Result<(), ValidationError> {
        if self.result_type != "getSnapshot" {
            return Err(invalid_request("getSnapshot result type is invalid"));
        }
        self.snapshot.validate_for(limits, protocol)
    }
}

/// Successful direct resynchronization result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResyncFigureResult {
    /// Exactly `resyncFigure`.
    #[serde(rename = "type")]
    pub result_type: String,
    /// Complete authoritative snapshot.
    pub snapshot: FigureSnapshot,
}

impl ResyncFigureResult {
    /// Validates result discriminant and complete scene.
    pub fn validate(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        if self.result_type != "resyncFigure" {
            return Err(invalid_request("resyncFigure result type is invalid"));
        }
        self.snapshot.validate(limits)
    }

    /// Validates a resynchronization result for an exact graphics endpoint.
    pub fn validate_for(
        &self,
        limits: GraphicsLimits,
        protocol: GraphicsProtocol,
    ) -> Result<(), ValidationError> {
        if self.result_type != "resyncFigure" {
            return Err(invalid_request("resyncFigure result type is invalid"));
        }
        self.snapshot.validate_for(limits, protocol)
    }
}

/// Successful idempotent lease release.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseBufferResult {
    /// Exactly `releaseBuffer`.
    #[serde(rename = "type")]
    pub result_type: String,
    /// True only if this request removed the connection's lease.
    pub released: bool,
}

impl ReleaseBufferResult {
    /// Validates the exact result discriminant.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.result_type == "releaseBuffer" {
            Ok(())
        } else {
            Err(invalid_request("releaseBuffer result type is invalid"))
        }
    }
}

/// Successful close result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseFigureResult {
    /// Exactly `closeFigure`.
    #[serde(rename = "type")]
    pub result_type: String,
    /// Exact close transaction revision.
    pub closed_revision: u64,
}

/// Successful v2 atomic semantic limits commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAxesLimitsResult {
    /// Exactly `setAxesLimits`.
    #[serde(rename = "type")]
    pub result_type: String,
    /// Exact revision committed by the owning graphics session.
    pub committed_revision: u64,
}

impl SetAxesLimitsResult {
    /// Validates the result discriminant and committed revision.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.result_type != "setAxesLimits" {
            return Err(invalid_request("setAxesLimits result type is invalid"));
        }
        validate_positive_safe(self.committed_revision, "committedRevision")
    }
}

/// Successful v2 atomic 3D camera commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAxesCameraResult {
    #[serde(rename = "type")]
    pub result_type: String,
    pub committed_revision: u64,
}

impl SetAxesCameraResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.result_type != "setAxesCamera" {
            return Err(invalid_request("setAxesCamera result type is invalid"));
        }
        validate_positive_safe(self.committed_revision, "committedRevision")
    }
}

impl CloseFigureResult {
    /// Validates the exact result discriminant and revision.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.result_type != "closeFigure" {
            return Err(invalid_request("closeFigure result type is invalid"));
        }
        validate_positive_safe(self.closed_revision, "closedRevision")
    }
}

/// Successful graphics-socket shutdown result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShutdownResult {
    /// Exactly `shutdown`.
    #[serde(rename = "type")]
    pub result_type: String,
}

impl ShutdownResult {
    /// Validates the exact result discriminant.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.result_type == "shutdown" {
            Ok(())
        } else {
            Err(invalid_request("shutdown result type is invalid"))
        }
    }
}

/// Typed response envelope. `result` and `error` are mutually exclusive.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope<T> {
    /// Exact identifier selected by the WebSocket endpoint.
    pub protocol: String,
    /// Session identifier.
    pub session_id: String,
    /// Server-generated message identifier.
    pub message_id: String,
    /// Exactly `response`.
    pub kind: MessageKind,
    /// Correlated request identifier.
    pub reply_to: String,
    /// Success flag.
    pub ok: bool,
    /// Present exactly on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
    /// Present exactly on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolFailure>,
}

impl<T> ResponseEnvelope<T> {
    /// Creates a successful response.
    #[must_use]
    pub fn success(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        reply_to: impl Into<String>,
        result: T,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: reply_to.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Creates a successful response for an explicit graphics version.
    #[must_use]
    pub fn success_for(
        protocol: GraphicsProtocol,
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        reply_to: impl Into<String>,
        result: T,
    ) -> Self {
        Self {
            protocol: protocol.identifier().to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: reply_to.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Validates the common and success/error envelope fields.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_common(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Response,
        )?;
        validate_id(&self.reply_to, "replyTo")?;
        if self.ok != self.result.is_some() || self.ok == self.error.is_some() {
            return Err(invalid_request(
                "response success must contain result and failure must contain error",
            ));
        }
        Ok(())
    }
}

impl ResponseEnvelope<Value> {
    /// Creates a failed response without a result.
    #[must_use]
    pub fn failure(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        reply_to: impl Into<String>,
        error: ProtocolFailure,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: reply_to.into(),
            ok: false,
            result: None,
            error: Some(error),
        }
    }

    /// Creates a failed response for an explicit graphics version.
    #[must_use]
    pub fn failure_for(
        protocol: GraphicsProtocol,
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        reply_to: impl Into<String>,
        error: ProtocolFailure,
    ) -> Self {
        Self {
            protocol: protocol.identifier().to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: reply_to.into(),
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

/// Figure-close event body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigureClosedEvent {
    /// Figure identifier.
    pub figure_id: String,
    /// Exact close transaction revision.
    pub closed_revision: u64,
}

/// Buffer-reclamation event body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferReleasedEvent {
    /// Reclaimed immutable resource identifier.
    pub buffer_id: String,
}

/// Terminal session event has no additional required member.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionClosedEvent {}

/// Known graphics-v1 server events.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    /// Exact next-revision transaction.
    FigureDelta(FigureDelta),
    /// Figure close, emitted after its successful response when client-driven.
    FigureClosed(FigureClosedEvent),
    /// Resource reclaimed after both references and leases reach zero.
    BufferReleased(BufferReleasedEvent),
    /// Owning kernel graphics session ended.
    SessionClosed(SessionClosedEvent),
}

/// Server event envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    /// Exact identifier selected by the WebSocket endpoint.
    pub protocol: String,
    /// Session identifier.
    pub session_id: String,
    /// Server-generated event identifier.
    pub message_id: String,
    /// Exactly `event`.
    pub kind: MessageKind,
    /// Typed event.
    pub event: Event,
}

impl EventEnvelope {
    /// Creates one event envelope.
    #[must_use]
    pub fn new(session_id: impl Into<String>, message_id: impl Into<String>, event: Event) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Event,
            event,
        }
    }

    /// Creates one event envelope for an explicit graphics version.
    #[must_use]
    pub fn new_for(
        protocol: GraphicsProtocol,
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        event: Event,
    ) -> Self {
        Self {
            protocol: protocol.identifier().to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Event,
            event,
        }
    }

    /// Validates known event cross-fields.
    pub fn validate(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        let protocol = parse_protocol(&self.protocol)?;
        self.validate_for(limits, protocol)
    }

    /// Validates one event for the exact endpoint selected by the connection.
    pub fn validate_for(
        &self,
        limits: GraphicsLimits,
        protocol: GraphicsProtocol,
    ) -> Result<(), ValidationError> {
        validate_common_for(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Event,
            protocol,
        )?;
        match &self.event {
            Event::FigureDelta(delta) => delta.validate_structure_for(limits, protocol),
            Event::FigureClosed(event) => {
                validate_id(&event.figure_id, "figureId")?;
                validate_positive_safe(event.closed_revision, "closedRevision")
            }
            Event::BufferReleased(event) => validate_id(&event.buffer_id, "bufferId"),
            Event::SessionClosed(_) => Ok(()),
        }
    }
}

/// Decodes a bounded known event. Structurally valid unknown event types return
/// `Ok(None)` for forward compatibility.
pub fn decode_event(
    text: &str,
    max_text_bytes: u64,
    limits: GraphicsLimits,
) -> Result<Option<EventEnvelope>, ValidationError> {
    decode_event_for(text, max_text_bytes, limits, GraphicsProtocol::V1)
}

/// Decodes a bounded event for the exact version selected by its endpoint.
pub fn decode_event_for(
    text: &str,
    max_text_bytes: u64,
    limits: GraphicsLimits,
    expected_protocol: GraphicsProtocol,
) -> Result<Option<EventEnvelope>, ValidationError> {
    validate_text_size(text, max_text_bytes)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|_| invalid_request("graphics event is not valid UTF-8 JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid_request("graphics event envelope must be an object"))?;
    validate_common_for(
        required_string(object, "protocol")?,
        required_string(object, "sessionId")?,
        required_string(object, "messageId")?,
        parse_kind(required_string(object, "kind")?)?,
        MessageKind::Event,
        expected_protocol,
    )?;
    let event = object
        .get("event")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_request("graphics event member must be an object"))?;
    let event_type = required_string(event, "type")?;
    if !matches!(
        event_type,
        "figureDelta" | "figureClosed" | "bufferReleased" | "sessionClosed"
    ) {
        return Ok(None);
    }
    let envelope: EventEnvelope = serde_json::from_value(value)
        .map_err(|_| invalid_request("known graphics event has an invalid schema"))?;
    envelope.validate_for(limits, expected_protocol)?;
    Ok(Some(envelope))
}

/// Encodes and bounds one response.
pub fn encode_response<T: Serialize>(
    response: &ResponseEnvelope<T>,
    max_text_bytes: u64,
) -> Result<String, ValidationError> {
    response.validate()?;
    encode_bounded(response, max_text_bytes)
}

/// Encodes and bounds one event.
pub fn encode_event(
    event: &EventEnvelope,
    max_text_bytes: u64,
    limits: GraphicsLimits,
) -> Result<String, ValidationError> {
    event.validate(limits)?;
    encode_bounded(event, max_text_bytes)
}

fn encode_bounded<T: Serialize>(value: &T, max_text_bytes: u64) -> Result<String, ValidationError> {
    let encoded = serde_json::to_string(value)
        .map_err(|_| invalid_request("graphics message could not be encoded"))?;
    validate_text_size(&encoded, max_text_bytes)?;
    Ok(encoded)
}

fn validate_common(
    protocol: &str,
    session_id: &str,
    message_id: &str,
    actual_kind: MessageKind,
    expected_kind: MessageKind,
) -> Result<(), ValidationError> {
    parse_protocol(protocol)?;
    validate_id(session_id, "sessionId")?;
    validate_id(message_id, "messageId")?;
    if actual_kind != expected_kind {
        return Err(invalid_request("graphics envelope kind is invalid"));
    }
    Ok(())
}

fn validate_common_for(
    protocol: &str,
    session_id: &str,
    message_id: &str,
    actual_kind: MessageKind,
    expected_kind: MessageKind,
    expected_protocol: GraphicsProtocol,
) -> Result<(), ValidationError> {
    if protocol != expected_protocol.identifier() {
        return Err(invalid_request(
            "graphics protocol identifier does not match the endpoint",
        ));
    }
    validate_common(protocol, session_id, message_id, actual_kind, expected_kind)
}

fn parse_protocol(protocol: &str) -> Result<GraphicsProtocol, ValidationError> {
    GraphicsProtocol::from_identifier(protocol)
        .ok_or_else(|| invalid_request("graphics protocol identifier is invalid"))
}

fn validate_identity(name: &str, version: &str, field: &str) -> Result<(), ValidationError> {
    if name.is_empty() || version.is_empty() {
        return Err(invalid_request(format!(
            "{field} name and version must be non-empty"
        )));
    }
    Ok(())
}

fn validate_id(value: &str, field: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        Err(invalid_request(format!("{field} must be non-empty")))
    } else {
        Ok(())
    }
}

fn validate_text_size(text: &str, maximum: u64) -> Result<(), ValidationError> {
    validate_safe(maximum, "text frame limit")?;
    let length = u64::try_from(text.len())
        .map_err(|_| payload_limit("text frame length does not fit u64"))?;
    if length > maximum || length > crate::MAX_TEXT_FRAME_BYTES {
        return Err(payload_limit(
            "graphics text frame exceeds the negotiated limit",
        ));
    }
    Ok(())
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, ValidationError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_request(format!("{field} must be a non-empty string")))
}

fn parse_kind(kind: &str) -> Result<MessageKind, ValidationError> {
    match kind {
        "request" => Ok(MessageKind::Request),
        "response" => Ok(MessageKind::Response),
        "event" => Ok(MessageKind::Event),
        _ => Err(invalid_request("graphics envelope kind is unknown")),
    }
}

fn is_known_request(request_type: &str, protocol: GraphicsProtocol) -> bool {
    matches!(
        request_type,
        "initialize"
            | "getSnapshot"
            | "getBuffer"
            | "releaseBuffer"
            | "closeFigure"
            | "resyncFigure"
            | "shutdown"
    ) || (matches!(
        protocol,
        GraphicsProtocol::V2 | GraphicsProtocol::V3 | GraphicsProtocol::V4
    ) && matches!(request_type, "setAxesLimits" | "setAxesCamera"))
}

fn validate_axes_target(
    protocol: GraphicsProtocol,
    axes_id: Option<&str>,
) -> Result<(), ValidationError> {
    match (protocol, axes_id) {
        (GraphicsProtocol::V2, None) => Ok(()),
        (GraphicsProtocol::V2, Some(_)) => {
            Err(invalid_request("axesId is unavailable in graphics-v2"))
        }
        (GraphicsProtocol::V3 | GraphicsProtocol::V4, Some(axes_id)) => {
            validate_id(axes_id, "axesId")
        }
        (GraphicsProtocol::V3 | GraphicsProtocol::V4, None) => Err(invalid_request(
            "axesId is required in graphics-v3 and graphics-v4 interaction requests",
        )),
        (GraphicsProtocol::V1, _) => Err(invalid_request(
            "explicit Axes interaction targets are unavailable in graphics-v1",
        )),
    }
}

fn validate_axis_limits(limits: [f64; 2], field: &str) -> Result<(), ValidationError> {
    if limits.iter().all(|limit| limit.is_finite()) && limits[0] < limits[1] {
        Ok(())
    } else {
        Err(invalid_request(format!(
            "{field} must contain two finite strictly increasing numbers"
        )))
    }
}

fn invalid_request(message: impl Into<String>) -> ValidationError {
    ValidationError::new(ErrorCategory::InvalidRequest, message)
}

fn payload_limit(message: &str) -> ValidationError {
    ValidationError::new(ErrorCategory::PayloadLimit, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientCapabilities, MAX_BINARY_FRAME_BYTES, MAX_BUFFER_BYTES, MAX_RESIDENT_BYTES};

    fn initialize_json() -> String {
        serde_json::json!({
            "protocol": PROTOCOL,
            "sessionId": "session-1",
            "messageId": "init-1",
            "kind": "request",
            "request": {
                "type": "initialize",
                "attachToken": "do-not-print-this-token",
                "client": { "name": "openmat-web", "version": "0.1.0" },
                "capabilities": {
                    "renderBackend": "webgpu",
                    "maxTextFrameBytes": crate::MAX_TEXT_FRAME_BYTES,
                    "maxBinaryFrameBytes": MAX_BINARY_FRAME_BYTES,
                    "maxBufferBytes": MAX_BUFFER_BYTES,
                    "maxResidentBytes": MAX_RESIDENT_BYTES
                }
            },
            "futureField": true
        })
        .to_string()
    }

    #[test]
    fn decodes_initialize_and_redacts_token_debug() {
        let DecodedRequest::Known(request) =
            decode_request(&initialize_json(), crate::MAX_TEXT_FRAME_BYTES).unwrap()
        else {
            panic!("known initialize");
        };
        assert!(format!("{request:?}").contains("[REDACTED]"));
        assert!(!format!("{request:?}").contains("do-not-print"));
    }

    #[test]
    fn unknown_request_is_correlatable_and_known_malformed_is_invalid() {
        let unknown = serde_json::json!({
            "protocol": PROTOCOL,
            "sessionId": "session-1",
            "messageId": "future-1",
            "kind": "request",
            "request": { "type": "rotateView", "degrees": 30 }
        })
        .to_string();
        let DecodedRequest::Unsupported(unsupported) =
            decode_request(&unknown, crate::MAX_TEXT_FRAME_BYTES).unwrap()
        else {
            panic!("unsupported request");
        };
        assert_eq!(unsupported.request_type, "rotateView");

        let malformed = initialize_json().replace("\"webgpu\"", "\"webgl2\"");
        assert_eq!(
            decode_request(&malformed, crate::MAX_TEXT_FRAME_BYTES)
                .unwrap_err()
                .category(),
            ErrorCategory::InvalidRequest
        );
    }

    #[test]
    fn v2_adds_atomic_limits_without_changing_v1_request_meanings() {
        let v2 = serde_json::json!({
            "protocol": PROTOCOL_V2,
            "sessionId": "session-1",
            "messageId": "limits-1",
            "kind": "request",
            "request": {
                "type": "setAxesLimits",
                "figureId": "figure-1",
                "expectedRevision": 4,
                "xLimits": [-2.0, 8.0],
                "yLimits": [0.25, 16.0]
            }
        })
        .to_string();
        let DecodedRequest::Known(envelope) =
            decode_request_for(&v2, crate::MAX_TEXT_FRAME_BYTES, GraphicsProtocol::V2).unwrap()
        else {
            panic!("v2 setAxesLimits must be known");
        };
        assert!(matches!(
            envelope.request,
            Request::SetAxesLimits(SetAxesLimitsRequest {
                figure_id,
                axes_id: None,
                expected_revision: 4,
                x_limits: [-2.0, 8.0],
                y_limits: [0.25, 16.0],
            }) if figure_id == "figure-1"
        ));

        let v1 = v2.replace(PROTOCOL_V2, PROTOCOL_V1);
        let DecodedRequest::Unsupported(unsupported) =
            decode_request(&v1, crate::MAX_TEXT_FRAME_BYTES).unwrap()
        else {
            panic!("frozen v1 must reject the new request as unsupported");
        };
        assert_eq!(unsupported.request_type, "setAxesLimits");
        assert!(
            decode_request_for(&v1, crate::MAX_TEXT_FRAME_BYTES, GraphicsProtocol::V2).is_err()
        );
    }

    #[test]
    fn v2_limits_require_finite_strictly_increasing_pairs() {
        let request = RequestEnvelope::new_for(
            GraphicsProtocol::V2,
            "session-1",
            "limits-1",
            Request::SetAxesLimits(SetAxesLimitsRequest {
                figure_id: "figure-1".to_owned(),
                axes_id: None,
                expected_revision: 1,
                x_limits: [2.0, 2.0],
                y_limits: [0.0, 1.0],
            }),
        );
        assert_eq!(
            request.validate().unwrap_err().category(),
            ErrorCategory::InvalidRequest
        );
    }

    #[test]
    fn v2_camera_is_atomic_bounded_and_unknown_to_v1() {
        let request = RequestEnvelope::new_for(
            GraphicsProtocol::V2,
            "session-1",
            "camera-1",
            Request::SetAxesCamera(SetAxesCameraRequest {
                figure_id: "figure-1".to_owned(),
                axes_id: None,
                expected_revision: 4,
                view: [55.0, 24.0],
                camera_scale: 0.8,
            }),
        );
        request.validate().unwrap();
        let encoded = serde_json::to_string(&request).unwrap();
        let DecodedRequest::Known(decoded) =
            decode_request_for(&encoded, crate::MAX_TEXT_FRAME_BYTES, GraphicsProtocol::V2)
                .unwrap()
        else {
            panic!("v2 setAxesCamera must be known");
        };
        assert_eq!(decoded, request);

        let v1 = encoded.replace(PROTOCOL_V2, PROTOCOL_V1);
        let DecodedRequest::Unsupported(unsupported) =
            decode_request(&v1, crate::MAX_TEXT_FRAME_BYTES).unwrap()
        else {
            panic!("frozen v1 must reject setAxesCamera");
        };
        assert_eq!(unsupported.request_type, "setAxesCamera");

        for (view, scale) in [
            ([0.0, 91.0], 1.0),
            ([f64::NAN, 0.0], 1.0),
            ([0.0, 0.0], 0.0),
        ] {
            let invalid = RequestEnvelope::new_for(
                GraphicsProtocol::V2,
                "session-1",
                "camera-invalid",
                Request::SetAxesCamera(SetAxesCameraRequest {
                    figure_id: "figure-1".to_owned(),
                    axes_id: None,
                    expected_revision: 4,
                    view,
                    camera_scale: scale,
                }),
            );
            assert_eq!(
                invalid.validate().unwrap_err().category(),
                ErrorCategory::InvalidRequest
            );
        }
    }

    #[test]
    fn v3_requires_an_explicit_axes_target_without_relaxing_v2() {
        let request = RequestEnvelope::new_for(
            GraphicsProtocol::V3,
            "session-1",
            "limits-axes-1",
            Request::SetAxesLimits(SetAxesLimitsRequest {
                figure_id: "figure-1".to_owned(),
                axes_id: Some("axes-2".to_owned()),
                expected_revision: 8,
                x_limits: [-1.0, 1.0],
                y_limits: [10.0, 20.0],
            }),
        );
        request.validate().unwrap();
        let encoded = serde_json::to_string(&request).unwrap();
        let DecodedRequest::Known(decoded) =
            decode_request_for(&encoded, crate::MAX_TEXT_FRAME_BYTES, GraphicsProtocol::V3)
                .unwrap()
        else {
            panic!("v3 setAxesLimits must be known");
        };
        assert_eq!(decoded, request);

        let missing_target = RequestEnvelope::new_for(
            GraphicsProtocol::V3,
            "session-1",
            "limits-missing-axes",
            Request::SetAxesLimits(SetAxesLimitsRequest {
                figure_id: "figure-1".to_owned(),
                axes_id: None,
                expected_revision: 8,
                x_limits: [-1.0, 1.0],
                y_limits: [10.0, 20.0],
            }),
        );
        assert_eq!(
            missing_target.validate().unwrap_err().category(),
            ErrorCategory::InvalidRequest
        );

        let v2_with_target = RequestEnvelope::new_for(
            GraphicsProtocol::V2,
            "session-1",
            "limits-v2-axes",
            Request::SetAxesLimits(SetAxesLimitsRequest {
                figure_id: "figure-1".to_owned(),
                axes_id: Some("axes-2".to_owned()),
                expected_revision: 8,
                x_limits: [-1.0, 1.0],
                y_limits: [10.0, 20.0],
            }),
        );
        assert_eq!(
            v2_with_target.validate().unwrap_err().category(),
            ErrorCategory::InvalidRequest
        );
    }

    #[test]
    fn v2_response_uses_the_selected_protocol() {
        let response = ResponseEnvelope::success_for(
            GraphicsProtocol::V2,
            "session-1",
            "server-1",
            "limits-1",
            SetAxesLimitsResult {
                result_type: "setAxesLimits".to_owned(),
                committed_revision: 2,
            },
        );
        assert_eq!(response.protocol, PROTOCOL_V2);
        response.result.as_ref().unwrap().validate().unwrap();
        assert!(encode_response(&response, crate::MAX_TEXT_FRAME_BYTES).is_ok());
    }

    #[test]
    fn oversized_text_is_payload_limit() {
        let oversized = "x".repeat(usize::try_from(crate::MAX_TEXT_FRAME_BYTES).unwrap() + 1);
        assert_eq!(
            decode_request(&oversized, crate::MAX_TEXT_FRAME_BYTES)
                .unwrap_err()
                .category(),
            ErrorCategory::PayloadLimit
        );
    }

    #[test]
    fn response_requires_exact_result_error_cross_fields() {
        let response = ResponseEnvelope::success(
            "session-1",
            "server-1",
            "request-1",
            ShutdownResult {
                result_type: "shutdown".to_owned(),
            },
        );
        assert!(encode_response(&response, crate::MAX_TEXT_FRAME_BYTES).is_ok());

        let invalid = ResponseEnvelope::<Value> {
            protocol: PROTOCOL.to_owned(),
            session_id: "session-1".to_owned(),
            message_id: "server-2".to_owned(),
            kind: MessageKind::Response,
            reply_to: "request-1".to_owned(),
            ok: true,
            result: None,
            error: None,
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn capabilities_schema_uses_exact_max_objects_name() {
        let capabilities = ServerCapabilities::from(GraphicsLimits::default());
        let encoded = serde_json::to_value(capabilities).unwrap();
        assert_eq!(encoded["maxObjects"], crate::MAX_LIVE_OBJECTS);
        assert!(encoded.get("maxLiveObjects").is_none());
        let _ = ClientCapabilities {
            render_backend: RenderBackend::Webgpu,
            max_text_frame_bytes: crate::MAX_TEXT_FRAME_BYTES,
            max_binary_frame_bytes: MAX_BINARY_FRAME_BYTES,
            max_buffer_bytes: MAX_BUFFER_BYTES,
            max_resident_bytes: MAX_RESIDENT_BYTES,
        };
    }
}

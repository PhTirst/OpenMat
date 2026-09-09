use serde::{Deserialize, Serialize};

use super::{
    MAX_PREVIEW_CODE_UNITS, MAX_SAFE_JSON_INTEGER, MAX_STRING_ELEMENT_CODE_UNITS, NegotiationError,
    PROTOCOL_V1, ProtocolVersion,
};
use crate::{
    Diagnostic, Event, ExecuteRequest, ExecuteResult, ExecutionMode, ImplementationInfo,
    InterruptRequest, InterruptResult, KernelStatus, ListWorkspaceRequest, MAX_PREVIEW_ELEMENTS,
    MessageKind, PROTOCOL_V0, ProtocolError, ShutdownRequest, ShutdownResult, ValidationError,
    VariableSummary, WorkspaceSummary,
};

/// Bootstrap initialization capabilities. The two v1-only limits are absent
/// for a v0-only offer or a v0-selected response.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Supported execution modes.
    #[serde(default)]
    pub execution_modes: Vec<ExecutionMode>,
    /// Supported display MIME types.
    #[serde(default)]
    pub display_mime_types: Vec<String>,
    /// Maximum preview element count.
    #[serde(default = "default_preview_elements")]
    pub max_preview_elements: u64,
    /// Maximum UTF-16 code-unit count in one string element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_string_element_code_units: Option<u64>,
    /// Maximum aggregate UTF-16 code-unit count in one preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_preview_code_units: Option<u64>,
    /// Cooperative interrupt support.
    #[serde(default)]
    pub interrupt: bool,
    /// Workspace-delta event support.
    #[serde(default)]
    pub workspace_delta: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            execution_modes: Vec::new(),
            display_mime_types: Vec::new(),
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            max_string_element_code_units: None,
            max_preview_code_units: None,
            interrupt: false,
            workspace_delta: false,
        }
    }
}

impl Capabilities {
    /// Returns default capabilities with all v1 hard limits advertised.
    #[must_use]
    pub fn v1_hard_limits() -> Self {
        Self {
            max_string_element_code_units: Some(MAX_STRING_ELEMENT_CODE_UNITS),
            max_preview_code_units: Some(MAX_PREVIEW_CODE_UNITS),
            ..Self::default()
        }
    }

    /// Returns the selected v1 preview limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] if either v1 capability is absent or any
    /// selected limit violates a hard bound.
    pub fn preview_limits(&self) -> Result<PreviewLimits, ValidationError> {
        let max_string_element_code_units =
            self.max_string_element_code_units.ok_or_else(|| {
                ValidationError::new(
                    "protocol.invalidCapabilities",
                    "maxStringElementCodeUnits is required for v1",
                )
            })?;
        let max_preview_code_units = self.max_preview_code_units.ok_or_else(|| {
            ValidationError::new(
                "protocol.invalidCapabilities",
                "maxPreviewCodeUnits is required for v1",
            )
        })?;
        PreviewLimits::new(
            self.max_preview_elements,
            max_string_element_code_units,
            max_preview_code_units,
        )
    }

    /// Computes the component-wise common capability set for `selected`.
    ///
    /// # Errors
    ///
    /// Returns [`NegotiationError`] when either peer's capabilities are invalid
    /// for the selected protocol.
    pub fn negotiate(
        client: &Self,
        kernel: &Self,
        selected: ProtocolVersion,
    ) -> Result<Self, NegotiationError> {
        client
            .validate_common()
            .map_err(|error| invalid_capabilities_error(&error))?;
        kernel
            .validate_common()
            .map_err(|error| invalid_capabilities_error(&error))?;
        let negotiated_v1_limits = if selected == ProtocolVersion::V1 {
            let client_limits = client
                .preview_limits()
                .map_err(|error| invalid_capabilities_error(&error))?;
            let kernel_limits = kernel
                .preview_limits()
                .map_err(|error| invalid_capabilities_error(&error))?;
            Some((client_limits, kernel_limits))
        } else {
            None
        };

        let execution_modes = kernel
            .execution_modes
            .iter()
            .copied()
            .filter(|mode| client.execution_modes.contains(mode))
            .collect();
        let display_mime_types = kernel
            .display_mime_types
            .iter()
            .filter(|mime| client.display_mime_types.contains(mime))
            .cloned()
            .collect();
        let (max_string_element_code_units, max_preview_code_units) =
            if let Some((client_limits, kernel_limits)) = negotiated_v1_limits {
                (
                    Some(
                        client_limits
                            .max_string_element_code_units
                            .min(kernel_limits.max_string_element_code_units)
                            .min(MAX_STRING_ELEMENT_CODE_UNITS),
                    ),
                    Some(
                        client_limits
                            .max_preview_code_units
                            .min(kernel_limits.max_preview_code_units)
                            .min(MAX_PREVIEW_CODE_UNITS),
                    ),
                )
            } else {
                (None, None)
            };
        let negotiated = Self {
            execution_modes,
            display_mime_types,
            max_preview_elements: client
                .max_preview_elements
                .min(kernel.max_preview_elements)
                .min(MAX_PREVIEW_ELEMENTS),
            max_string_element_code_units,
            max_preview_code_units,
            interrupt: client.interrupt && kernel.interrupt,
            workspace_delta: client.workspace_delta && kernel.workspace_delta,
        };
        negotiated
            .validate_for_selection(selected)
            .map_err(|error| invalid_capabilities_error(&error))?;
        Ok(negotiated)
    }

    pub(super) fn validate_for_selection(
        &self,
        selected: ProtocolVersion,
    ) -> Result<(), ValidationError> {
        self.validate_common()?;
        match selected {
            ProtocolVersion::V0 => {
                if self.max_string_element_code_units.is_some()
                    || self.max_preview_code_units.is_some()
                {
                    return Err(ValidationError::new(
                        "protocol.invalidCapabilities",
                        "v0-selected capabilities must omit v1 code-unit limits",
                    ));
                }
            }
            ProtocolVersion::V1 => self.validate_v1_fields()?,
        }
        Ok(())
    }

    fn validate_common(&self) -> Result<(), ValidationError> {
        validate_bounded_positive(
            "maxPreviewElements",
            self.max_preview_elements,
            MAX_PREVIEW_ELEMENTS,
        )?;
        for mime in &self.display_mime_types {
            require_non_empty("displayMimeTypes entry", mime)?;
        }
        Ok(())
    }

    fn validate_v1_fields(&self) -> Result<(), ValidationError> {
        self.preview_limits().map(|_| ())
    }
}

fn invalid_capabilities_error(error: &ValidationError) -> NegotiationError {
    NegotiationError::new("protocol.invalidCapabilities", error.to_string())
}

/// Negotiated v1 preview limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewLimits {
    /// Maximum number of returned values.
    pub max_preview_elements: u64,
    /// Maximum code units in one string element.
    pub max_string_element_code_units: u64,
    /// Maximum aggregate string code units in one preview.
    pub max_preview_code_units: u64,
}

impl Default for PreviewLimits {
    fn default() -> Self {
        Self {
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            max_string_element_code_units: MAX_STRING_ELEMENT_CODE_UNITS,
            max_preview_code_units: MAX_PREVIEW_CODE_UNITS,
        }
    }
}

impl PreviewLimits {
    /// Constructs and validates negotiated preview limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] when a limit is zero, above its hard bound,
    /// or when the aggregate limit is below the per-element limit.
    pub fn new(
        max_preview_elements: u64,
        max_string_element_code_units: u64,
        max_preview_code_units: u64,
    ) -> Result<Self, ValidationError> {
        let limits = Self {
            max_preview_elements,
            max_string_element_code_units,
            max_preview_code_units,
        };
        limits.validate()?;
        Ok(limits)
    }

    pub(super) fn validate(&self) -> Result<(), ValidationError> {
        validate_bounded_positive(
            "maxPreviewElements",
            self.max_preview_elements,
            MAX_PREVIEW_ELEMENTS,
        )?;
        validate_bounded_positive(
            "maxStringElementCodeUnits",
            self.max_string_element_code_units,
            MAX_STRING_ELEMENT_CODE_UNITS,
        )?;
        validate_bounded_positive(
            "maxPreviewCodeUnits",
            self.max_preview_code_units,
            MAX_PREVIEW_CODE_UNITS,
        )?;
        if self.max_preview_code_units < self.max_string_element_code_units {
            return Err(ValidationError::new(
                "protocol.invalidCapabilities",
                "maxPreviewCodeUnits must not be less than maxStringElementCodeUnits",
            ));
        }
        Ok(())
    }
}

/// Bootstrap request envelope. Its outer protocol is always v0.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapRequestEnvelope {
    /// Must be `openmat-kernel-v0`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque request identifier.
    pub message_id: String,
    /// Must be `request`.
    pub kind: MessageKind,
    /// Initialize request.
    pub request: BootstrapRequest,
}

impl BootstrapRequestEnvelope {
    /// Constructs a bootstrap-v0 initialize request.
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        initialize: InitializeRequest,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V0.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Request,
            request: BootstrapRequest::Initialize(initialize),
        }
    }

    /// Validates bootstrap envelope and initialize-offer invariants.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for a non-v0 envelope, malformed header,
    /// duplicate offer, or invalid v1 capabilities.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Request,
            PROTOCOL_V0,
        )?;
        let BootstrapRequest::Initialize(request) = &self.request;
        request
            .validate()
            .map_err(|error| ValidationError::new(error.category(), error.to_string()))
    }
}

/// The only request allowed in the bootstrap exchange.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "params", rename_all = "camelCase")]
pub enum BootstrapRequest {
    /// Negotiate protocol and capabilities.
    Initialize(InitializeRequest),
}

/// V1-capable initialize parameters transported in a v0 envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    /// Client implementation identity.
    pub client: ImplementationInfo,
    /// Unique protocol identifiers in preference order.
    #[serde(default)]
    pub supported_protocols: Vec<String>,
    /// Client capability offer.
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl InitializeRequest {
    /// Constructs a production v1 offer with `[v1, v0]` protocol order.
    #[must_use]
    pub fn v1(client: ImplementationInfo, capabilities: Capabilities) -> Self {
        Self {
            client,
            supported_protocols: super::production_protocol_offer(),
            capabilities,
        }
    }

    /// Validates the initialize request for negotiation.
    ///
    /// # Errors
    ///
    /// Returns [`NegotiationError`] for malformed identity, protocol list, or
    /// capability limits.
    pub fn validate_for_negotiation(&self) -> Result<(), NegotiationError> {
        self.validate()
    }

    fn validate(&self) -> Result<(), NegotiationError> {
        if self.client.name.is_empty() || self.client.version.is_empty() {
            return Err(NegotiationError::new(
                "protocol.invalidOffer",
                "client name and version must not be empty",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for protocol in &self.supported_protocols {
            if protocol.is_empty() || !seen.insert(protocol.as_str()) {
                return Err(NegotiationError::new(
                    "protocol.invalidOffer",
                    "supportedProtocols entries must be non-empty and unique",
                ));
            }
        }
        self.capabilities
            .validate_common()
            .map_err(|error| invalid_capabilities_error(&error))?;
        if self
            .supported_protocols
            .iter()
            .any(|protocol| protocol == PROTOCOL_V1)
        {
            self.capabilities
                .validate_v1_fields()
                .map_err(|error| invalid_capabilities_error(&error))?;
        }
        Ok(())
    }
}

/// Bootstrap initialize response envelope. It is the last v0 envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResponseEnvelope {
    /// Must be `openmat-kernel-v0`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque response identifier.
    pub message_id: String,
    /// Must be `response`.
    pub kind: MessageKind,
    /// Bootstrap request identifier.
    pub reply_to: String,
    /// Whether initialization succeeded.
    pub ok: bool,
    /// Successful initialize result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<BootstrapResponseResult>,
    /// Structured initialization error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl BootstrapResponseEnvelope {
    /// Constructs the last bootstrap-v0 envelope for successful negotiation.
    #[must_use]
    pub fn success(
        request: &BootstrapRequestEnvelope,
        message_id: impl Into<String>,
        result: InitializeResult,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V0.to_owned(),
            session_id: request.session_id.clone(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: request.message_id.clone(),
            ok: true,
            result: Some(BootstrapResponseResult::Initialize(result)),
            error: None,
        }
    }

    /// Constructs a failed bootstrap-v0 initialize response.
    #[must_use]
    pub fn failure(
        request: &BootstrapRequestEnvelope,
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

    /// Validates bootstrap protocol, header, response shape, and result.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] when any bootstrap invariant is violated.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Response,
            PROTOCOL_V0,
        )?;
        require_non_empty("replyTo", &self.reply_to)?;
        match (self.ok, &self.result, &self.error) {
            (true, Some(BootstrapResponseResult::Initialize(result)), None) => result.validate(),
            (false, None, Some(error)) => validate_protocol_error(error),
            _ => Err(ValidationError::new(
                "response_shape",
                "successful responses require only result; failed responses require only error",
            )),
        }
    }
}

/// Successful bootstrap response result.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum BootstrapResponseResult {
    /// Negotiated initialization result.
    Initialize(InitializeResult),
}

/// Successful v1-capable initialization result.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// Selected protocol identifier.
    pub negotiated_protocol: String,
    /// Kernel implementation identity.
    pub implementation: ImplementationInfo,
    /// Component-wise negotiated capabilities.
    pub capabilities: Capabilities,
}

impl InitializeResult {
    fn validate(&self) -> Result<(), ValidationError> {
        require_non_empty("implementation.name", &self.implementation.name)?;
        require_non_empty("implementation.version", &self.implementation.version)?;
        let selected =
            ProtocolVersion::from_identifier(&self.negotiated_protocol).ok_or_else(|| {
                ValidationError::new(
                    "protocol.invalidNegotiation",
                    "negotiatedProtocol must name v0 or v1",
                )
            })?;
        self.capabilities.validate_for_selection(selected)
    }
}

/// A post-bootstrap v1 request envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    /// Must be `openmat-kernel-v1`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque request identifier.
    pub message_id: String,
    /// Must be `request`.
    pub kind: MessageKind,
    /// Typed non-bootstrap request.
    pub request: Request,
}

impl RequestEnvelope {
    /// Constructs a post-bootstrap v1 request.
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

    /// Validates the v1 envelope and request against negotiated limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for a mixed version, malformed header, or
    /// invalid request-specific bound.
    pub fn validate(&self, limits: &PreviewLimits) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Request,
            PROTOCOL_V1,
        )?;
        limits.validate()?;
        self.request.validate(limits)
    }
}

/// Post-bootstrap v1 request payloads. Initialize is intentionally absent: it
/// is legal only in a bootstrap-v0 envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "params", rename_all = "camelCase")]
pub enum Request {
    /// Execute UTF-8 source.
    Execute(ExecuteRequest),
    /// Cooperatively interrupt active execution.
    Interrupt(InterruptRequest),
    /// Inspect a bounded workspace range.
    Inspect(InspectRequest),
    /// List workspace summaries.
    ListWorkspace(ListWorkspaceRequest),
    /// Terminate the kernel orderly.
    Shutdown(ShutdownRequest),
}

impl Request {
    fn validate(&self, limits: &PreviewLimits) -> Result<(), ValidationError> {
        match self {
            Self::Execute(request) => require_non_empty("sourceName", &request.source_name),
            Self::Inspect(request) => request.validate(limits),
            Self::Interrupt(_) | Self::ListWorkspace(_) | Self::Shutdown(_) => Ok(()),
        }
    }

    /// Returns true for requests that may bypass the sequential execution
    /// queue.
    #[must_use]
    pub const fn is_control(&self) -> bool {
        matches!(self, Self::Interrupt(_) | Self::Shutdown(_))
    }
}

/// Bounded v1 workspace inspection request.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectRequest {
    /// Workspace variable name.
    pub name: String,
    /// One-based selected range.
    pub range: MatrixRange,
    /// Requested preview element limit.
    #[serde(default = "default_preview_elements")]
    pub max_elements: u64,
}

impl InspectRequest {
    /// Validates name, range, and negotiated element limit.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for an invalid shape or bound.
    pub fn validate(&self, limits: &PreviewLimits) -> Result<(), ValidationError> {
        require_non_empty("name", &self.name)?;
        limits.validate()?;
        self.range.validate()?;
        validate_request_element_bound(self.max_elements, limits.max_preview_elements)
    }
}

/// A post-bootstrap v1 response envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    /// Must be `openmat-kernel-v1`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque response identifier.
    pub message_id: String,
    /// Must be `response`.
    pub kind: MessageKind,
    /// Request identifier being answered.
    pub reply_to: String,
    /// Whether the request succeeded.
    pub ok: bool,
    /// Successful result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResponseResult>,
    /// Structured failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl ResponseEnvelope {
    /// Constructs a successful v1 response.
    #[must_use]
    pub fn success(
        request: &RequestEnvelope,
        message_id: impl Into<String>,
        result: ResponseResult,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            session_id: request.session_id.clone(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: request.message_id.clone(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Constructs a failed v1 response.
    #[must_use]
    pub fn failure(
        request: &RequestEnvelope,
        message_id: impl Into<String>,
        error: ProtocolError,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            session_id: request.session_id.clone(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: request.message_id.clone(),
            ok: false,
            result: None,
            error: Some(error),
        }
    }

    /// Returns whether this response is correlated to `request`.
    #[must_use]
    pub fn is_reply_to(&self, request: &RequestEnvelope) -> bool {
        self.protocol == request.protocol
            && self.session_id == request.session_id
            && self.reply_to == request.message_id
    }

    /// Validates the v1 response against negotiated preview limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for a mixed version, malformed response
    /// shape, invalid error, or invalid result payload.
    pub fn validate(&self, limits: &PreviewLimits) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Response,
            PROTOCOL_V1,
        )?;
        require_non_empty("replyTo", &self.reply_to)?;
        limits.validate()?;
        match (self.ok, &self.result, &self.error) {
            (true, Some(result), None) => result.validate(limits),
            (false, None, Some(error)) => validate_protocol_error(error),
            _ => Err(ValidationError::new(
                "response_shape",
                "successful responses require only result; failed responses require only error",
            )),
        }
    }

    /// Validates this response against its originating request, including
    /// correlation, successful result type, and request-specific inspect bound.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for any ordinary response violation or when
    /// the response does not match the supplied request.
    pub fn validate_for_request(
        &self,
        request: &RequestEnvelope,
        limits: &PreviewLimits,
    ) -> Result<(), ValidationError> {
        request.validate(limits)?;
        self.validate(limits)?;
        if !self.is_reply_to(request) {
            return Err(ValidationError::new(
                "response_correlation",
                "response protocol, sessionId, or replyTo does not match request",
            ));
        }
        if !self.ok {
            return Ok(());
        }
        let Some(result) = &self.result else {
            return Err(ValidationError::new(
                "response_shape",
                "successful response is missing its result",
            ));
        };
        match (&request.request, result) {
            (Request::Execute(_), ResponseResult::Execute(_))
            | (Request::Interrupt(_), ResponseResult::Interrupt(_))
            | (Request::ListWorkspace(_), ResponseResult::ListWorkspace(_))
            | (Request::Shutdown(_), ResponseResult::Shutdown(_)) => Ok(()),
            (Request::Inspect(inspect), ResponseResult::Inspect(preview)) => {
                if preview.selected_range != inspect.range {
                    return Err(ValidationError::new(
                        "response_shape",
                        "inspect result selectedRange does not match the request range",
                    ));
                }
                preview.validate_for_request(limits, inspect.max_elements)
            }
            _ => Err(ValidationError::new(
                "response_shape",
                "successful response result type does not match request type",
            )),
        }
    }
}

/// Successful post-bootstrap v1 response payloads. Initialize is absent because
/// its response is the final bootstrap-v0 envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ResponseResult {
    /// Execution completion result.
    Execute(ExecuteResult),
    /// Interrupt acknowledgement.
    Interrupt(InterruptResult),
    /// Bounded exact preview.
    Inspect(MatrixPreview),
    /// Workspace summaries.
    ListWorkspace(WorkspaceSummary),
    /// Shutdown acknowledgement.
    Shutdown(ShutdownResult),
}

impl ResponseResult {
    fn validate(&self, limits: &PreviewLimits) -> Result<(), ValidationError> {
        match self {
            Self::Inspect(preview) => preview.validate(limits),
            Self::ListWorkspace(workspace) => validate_workspace_summary(workspace),
            Self::Execute(_) | Self::Interrupt(_) | Self::Shutdown(_) => Ok(()),
        }
    }
}

/// A post-bootstrap v1 event envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    /// Must be `openmat-kernel-v1`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque event identifier.
    pub message_id: String,
    /// Must be `event`.
    pub kind: MessageKind,
    /// Known or explicitly forward-compatible unknown event.
    pub event: Event,
}

impl EventEnvelope {
    /// Constructs a v1 event envelope.
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

    /// Validates the v1 header and semantic fields of known events. Unknown
    /// event types remain valid extensions.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for a mixed version, malformed header, or
    /// malformed known event payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Event,
            PROTOCOL_V1,
        )?;
        validate_event(&self.event)
    }
}

/// Any post-bootstrap server-to-client v1 message.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ServerMessage {
    /// A response to a request.
    Response(ResponseEnvelope),
    /// An unsolicited event.
    Event(EventEnvelope),
}

impl ServerMessage {
    /// Validates this message against negotiated preview limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] when the contained envelope is invalid.
    pub fn validate(&self, limits: &PreviewLimits) -> Result<(), ValidationError> {
        match self {
            Self::Response(response) => response.validate(limits),
            Self::Event(event) => event.validate(),
        }
    }
}

/// A one-based multidimensional selection with at least two dimensions.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixRange {
    /// One-based start index per dimension.
    pub start: Vec<u64>,
    /// Selected extent per dimension.
    pub size: Vec<u64>,
}

impl MatrixRange {
    /// Validates rank, safe integers, one-based starts, and checked size product.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for a rank below two, a mismatch, a zero
    /// start, an unsafe JSON integer, or product overflow.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.start.len() != self.size.len() || self.start.len() < 2 {
            return Err(ValidationError::new(
                "preview_shape",
                "range start and size must have the same rank of at least two",
            ));
        }
        for start in &self.start {
            validate_safe_json_integer("selectedRange.start", *start)?;
            if *start == 0 {
                return Err(ValidationError::new(
                    "preview_shape",
                    "selected range starts are one-based",
                ));
            }
        }
        for size in &self.size {
            validate_safe_json_integer("selectedRange.size", *size)?;
        }
        checked_product(&self.size, "selectedRange.size").map(|_| ())
    }
}

/// A bounded, exact v1 matrix preview.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixPreview {
    /// Exact MATLAB-style class.
    pub class: String,
    /// Full array dimensions.
    pub dimensions: Vec<u64>,
    /// Whether the value has complex storage.
    pub complex: bool,
    /// One-based selected range.
    pub selected_range: MatrixRange,
    /// Longest whole-element column-major prefix that fits the budgets.
    pub values: Vec<PreviewValue>,
    /// Exact omitted count and truncation marker.
    pub truncation: crate::PreviewTruncation,
}

impl MatrixPreview {
    /// Validates this preview against negotiated limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for unsafe structural integers, inconsistent
    /// shape/count/truncation metadata, class/kind/complex disagreement,
    /// non-canonical integer strings, or exceeded code-unit budgets.
    pub fn validate(&self, limits: &PreviewLimits) -> Result<(), ValidationError> {
        self.validate_with_element_limit(limits, limits.max_preview_elements)
    }

    /// Validates this preview against both negotiated limits and the inspect
    /// request's `maxElements` value.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] when `request_max_elements` is invalid or
    /// the preview violates any v1 invariant.
    pub fn validate_for_request(
        &self,
        limits: &PreviewLimits,
        request_max_elements: u64,
    ) -> Result<(), ValidationError> {
        validate_request_element_bound(request_max_elements, limits.max_preview_elements)?;
        self.validate_with_element_limit(limits, request_max_elements)
    }

    fn validate_with_element_limit(
        &self,
        limits: &PreviewLimits,
        element_limit: u64,
    ) -> Result<(), ValidationError> {
        limits.validate()?;
        let class = ValueClass::parse(&self.class).ok_or_else(unsupported_value)?;
        let _full_count = validate_dimensions(&self.dimensions)?;
        self.selected_range.validate()?;
        if self.dimensions.len() != self.selected_range.start.len() {
            return Err(ValidationError::new(
                "preview_shape",
                "dimensions, selectedRange.start, and selectedRange.size must have equal rank",
            ));
        }
        let selected_count = checked_product(&self.selected_range.size, "selectedRange.size")?;
        if selected_count != 0 {
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
                    .ok_or_else(|| {
                        ValidationError::new("preview_shape", "selected range overflows")
                    })?;
                validate_safe_json_integer("selected range end", end)?;
                if end > *dimension {
                    return Err(ValidationError::new(
                        "preview_shape",
                        "nonempty selected range exceeds full dimensions",
                    ));
                }
            }
        }

        validate_safe_json_integer(
            "truncation.omittedElements",
            self.truncation.omitted_elements,
        )?;
        let value_count = u64::try_from(self.values.len()).map_err(|_| {
            ValidationError::new("preview_bound", "preview value count does not fit u64")
        })?;
        let effective_limit = element_limit.min(limits.max_preview_elements);
        if value_count > effective_limit {
            return Err(ValidationError::new(
                "preview_bound",
                format!("preview value count exceeds effective limit {effective_limit}"),
            ));
        }
        let represented_count = value_count
            .checked_add(self.truncation.omitted_elements)
            .ok_or_else(|| {
                ValidationError::new("preview_truncation", "represented element count overflows")
            })?;
        if represented_count != selected_count
            || self.truncation.truncated != (self.truncation.omitted_elements != 0)
        {
            return Err(ValidationError::new(
                "preview_truncation",
                "values and truncation metadata do not match selectedRange.size",
            ));
        }

        validate_preview_values(class, self.complex, &self.values, limits)
    }
}

/// A canonical v1 preview scalar. Legacy v0 `text` and bare `missing` values
/// remain decodable as known kinds but validation rejects them in v1 previews.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PreviewValue {
    /// Exact UTF-16 code unit for a char array element.
    CharCodeUnit {
        /// Code unit in `0..=65535`.
        value: u16,
    },
    /// Exact fixed-width integer components as canonical decimal strings.
    Integer {
        /// Real component.
        real: String,
        /// Imaginary component.
        imaginary: String,
    },
    /// Exact string element code units and independent missing marker.
    String {
        /// UTF-16 code units; isolated surrogates are valid.
        #[serde(rename = "codeUnits")]
        code_units: Vec<u16>,
        /// Whether this is the missing string value.
        missing: bool,
    },
    /// Finite real floating-point value.
    Number {
        /// Numeric value.
        value: f64,
    },
    /// Finite complex floating-point value.
    Complex {
        /// Real component.
        real: f64,
        /// Imaginary component.
        imaginary: f64,
    },
    /// Logical scalar.
    Logical {
        /// Boolean value.
        value: bool,
    },
    /// Canonical non-finite real floating-point spelling.
    Special {
        /// `nan`, `infinity`, or `negativeInfinity`.
        value: String,
    },
    /// Legacy v0 UTF-8 text; non-canonical in v1.
    Text {
        /// Legacy text value.
        value: String,
    },
    /// Legacy v0 missing marker; non-canonical in v1.
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueClass {
    Char,
    String,
    Logical,
    Integer(IntegerClass),
    Double,
    Single,
}

impl ValueClass {
    fn parse(class: &str) -> Option<Self> {
        match class {
            "char" => Some(Self::Char),
            "string" => Some(Self::String),
            "logical" => Some(Self::Logical),
            "double" => Some(Self::Double),
            "single" => Some(Self::Single),
            "int8" => Some(Self::Integer(IntegerClass::new(true, 8))),
            "uint8" => Some(Self::Integer(IntegerClass::new(false, 8))),
            "int16" => Some(Self::Integer(IntegerClass::new(true, 16))),
            "uint16" => Some(Self::Integer(IntegerClass::new(false, 16))),
            "int32" => Some(Self::Integer(IntegerClass::new(true, 32))),
            "uint32" => Some(Self::Integer(IntegerClass::new(false, 32))),
            "int64" => Some(Self::Integer(IntegerClass::new(true, 64))),
            "uint64" => Some(Self::Integer(IntegerClass::new(false, 64))),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IntegerClass {
    signed: bool,
    bits: u32,
}

impl IntegerClass {
    const fn new(signed: bool, bits: u32) -> Self {
        Self { signed, bits }
    }

    const fn positive_max(self) -> u128 {
        if self.signed {
            (1_u128 << (self.bits - 1)) - 1
        } else {
            (1_u128 << self.bits) - 1
        }
    }

    const fn negative_magnitude_max(self) -> u128 {
        1_u128 << (self.bits - 1)
    }
}

fn validate_preview_values(
    class: ValueClass,
    complex: bool,
    values: &[PreviewValue],
    limits: &PreviewLimits,
) -> Result<(), ValidationError> {
    match class {
        ValueClass::Char => {
            require_real_storage(complex)?;
            if values
                .iter()
                .all(|value| matches!(value, PreviewValue::CharCodeUnit { .. }))
            {
                Ok(())
            } else {
                Err(unsupported_value())
            }
        }
        ValueClass::String => {
            require_real_storage(complex)?;
            validate_string_values(values, limits)
        }
        ValueClass::Logical => {
            require_real_storage(complex)?;
            if values
                .iter()
                .all(|value| matches!(value, PreviewValue::Logical { .. }))
            {
                Ok(())
            } else {
                Err(unsupported_value())
            }
        }
        ValueClass::Integer(integer_class) => {
            validate_integer_values(integer_class, complex, values)
        }
        ValueClass::Double | ValueClass::Single if complex => {
            for value in values {
                match value {
                    PreviewValue::Complex { real, imaginary }
                        if real.is_finite() && imaginary.is_finite() => {}
                    _ => return Err(unsupported_value()),
                }
            }
            Ok(())
        }
        ValueClass::Double | ValueClass::Single => {
            for value in values {
                match value {
                    PreviewValue::Number { value } if value.is_finite() => {}
                    PreviewValue::Special { value }
                        if matches!(value.as_str(), "nan" | "infinity" | "negativeInfinity") => {}
                    _ => return Err(unsupported_value()),
                }
            }
            Ok(())
        }
    }
}

fn require_real_storage(complex: bool) -> Result<(), ValidationError> {
    if complex {
        Err(unsupported_value())
    } else {
        Ok(())
    }
}

fn validate_string_values(
    values: &[PreviewValue],
    limits: &PreviewLimits,
) -> Result<(), ValidationError> {
    let mut aggregate = 0_u64;
    for value in values {
        let PreviewValue::String {
            code_units,
            missing,
        } = value
        else {
            return Err(unsupported_value());
        };
        if *missing && !code_units.is_empty() {
            return Err(unsupported_value());
        }
        let count = u64::try_from(code_units.len()).map_err(|_| unsupported_value())?;
        if count > limits.max_string_element_code_units {
            return Err(unsupported_value());
        }
        aggregate = aggregate.checked_add(count).ok_or_else(unsupported_value)?;
        if aggregate > limits.max_preview_code_units {
            return Err(ValidationError::new(
                "preview_bound",
                "aggregate string code-unit budget exceeded",
            ));
        }
    }
    Ok(())
}

fn validate_integer_values(
    class: IntegerClass,
    complex: bool,
    values: &[PreviewValue],
) -> Result<(), ValidationError> {
    let mut any_imaginary = false;
    for value in values {
        let PreviewValue::Integer { real, imaginary } = value else {
            return Err(unsupported_value());
        };
        validate_integer_component(real, class)?;
        validate_integer_component(imaginary, class)?;
        if imaginary != "0" {
            any_imaginary = true;
        }
        if !complex && imaginary != "0" {
            return Err(unsupported_value());
        }
    }
    if complex && !any_imaginary {
        return Err(unsupported_value());
    }
    Ok(())
}

fn validate_integer_component(component: &str, class: IntegerClass) -> Result<(), ValidationError> {
    let bytes = component.as_bytes();
    let (negative, digits) = match bytes {
        [b'-', rest @ ..] => (true, rest),
        _ => (false, bytes),
    };
    let canonical = match digits {
        [b'0'] => !negative,
        [first, rest @ ..] => matches!(first, b'1'..=b'9') && rest.iter().all(u8::is_ascii_digit),
        [] => false,
    };
    if !canonical || (negative && !class.signed) {
        return Err(unsupported_value());
    }
    let magnitude = std::str::from_utf8(digits)
        .ok()
        .and_then(|digits| digits.parse::<u128>().ok())
        .ok_or_else(unsupported_value)?;
    let maximum = if negative {
        class.negative_magnitude_max()
    } else {
        class.positive_max()
    };
    if magnitude > maximum {
        return Err(unsupported_value());
    }
    Ok(())
}

fn validate_event(event: &Event) -> Result<(), ValidationError> {
    match event {
        Event::Diagnostic(diagnostic) => validate_diagnostic(diagnostic),
        Event::WorkspaceDelta(delta) => {
            for variable in delta.added.iter().chain(&delta.changed) {
                validate_variable_summary(variable)?;
            }
            for name in &delta.removed {
                require_non_empty("workspaceDelta.removed entry", name)?;
            }
            Ok(())
        }
        Event::Unknown { event_type, .. } => require_non_empty("event.type", event_type),
        Event::Status(status) => match status.status {
            KernelStatus::Starting
            | KernelStatus::Idle
            | KernelStatus::Busy
            | KernelStatus::Interrupted
            | KernelStatus::Dead => Ok(()),
        },
        Event::Stream(_) | Event::Display(_) => Ok(()),
    }
}

fn validate_protocol_error(error: &ProtocolError) -> Result<(), ValidationError> {
    require_non_empty("error.category", &error.category)?;
    for diagnostic in &error.diagnostics {
        validate_diagnostic(diagnostic)?;
    }
    Ok(())
}

fn validate_diagnostic(diagnostic: &Diagnostic) -> Result<(), ValidationError> {
    if let Some(range) = &diagnostic.range {
        validate_source_range(range)?;
    }
    for related in &diagnostic.related {
        validate_source_range(&related.range)?;
    }
    Ok(())
}

fn validate_source_range(range: &crate::SourceRange) -> Result<(), ValidationError> {
    require_non_empty("sourceName", &range.source_name)?;
    validate_safe_json_integer("source range start", range.start)?;
    validate_safe_json_integer("source range end", range.end)?;
    if range.start > range.end {
        return Err(ValidationError::new(
            "protocol.invalidMessage",
            "source range start must not exceed end",
        ));
    }
    Ok(())
}

fn validate_workspace_summary(summary: &WorkspaceSummary) -> Result<(), ValidationError> {
    for variable in &summary.variables {
        validate_variable_summary(variable)?;
    }
    Ok(())
}

fn validate_variable_summary(variable: &VariableSummary) -> Result<(), ValidationError> {
    require_non_empty("variable.name", &variable.name)?;
    require_non_empty("variable.class", &variable.class)?;
    validate_dimensions(&variable.dimensions)?;
    if let Some(bytes) = variable.bytes {
        validate_safe_json_integer("variable.bytes", bytes)?;
    }
    Ok(())
}

fn validate_dimensions(dimensions: &[u64]) -> Result<u64, ValidationError> {
    if dimensions.len() < 2 {
        return Err(ValidationError::new(
            "preview_shape",
            "dimensions must contain at least two entries",
        ));
    }
    for dimension in dimensions {
        validate_safe_json_integer("dimensions", *dimension)?;
    }
    checked_product(dimensions, "dimensions")
}

fn checked_product(values: &[u64], field: &'static str) -> Result<u64, ValidationError> {
    values.iter().try_fold(1_u64, |product, value| {
        product.checked_mul(*value).ok_or_else(|| {
            ValidationError::new(
                "preview_shape",
                format!("checked product of {field} overflows u64"),
            )
        })
    })
}

fn validate_bounded_positive(
    field: &'static str,
    value: u64,
    maximum: u64,
) -> Result<(), ValidationError> {
    validate_safe_json_integer(field, value)?;
    if value == 0 || value > maximum {
        return Err(ValidationError::new(
            "protocol.invalidCapabilities",
            format!("{field} must be between 1 and {maximum}"),
        ));
    }
    Ok(())
}

fn validate_request_element_bound(value: u64, maximum: u64) -> Result<(), ValidationError> {
    validate_safe_json_integer("maxElements", value)?;
    if value == 0 || value > maximum {
        return Err(ValidationError::new(
            "preview_bound",
            format!("maxElements must be between 1 and {maximum}"),
        ));
    }
    Ok(())
}

fn validate_safe_json_integer(field: &'static str, value: u64) -> Result<(), ValidationError> {
    if value > MAX_SAFE_JSON_INTEGER {
        Err(ValidationError::new(
            "protocol.invalidMessage",
            format!("{field} exceeds the maximum safe JSON integer"),
        ))
    } else {
        Ok(())
    }
}

fn validate_header(
    protocol: &str,
    session_id: &str,
    message_id: &str,
    actual_kind: MessageKind,
    expected_kind: MessageKind,
    expected_protocol: &'static str,
) -> Result<(), ValidationError> {
    if protocol != expected_protocol {
        return Err(ValidationError::new(
            "protocol",
            format!("expected {expected_protocol}, got {protocol}"),
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

fn unsupported_value() -> ValidationError {
    ValidationError::new(
        "workspace.unsupportedValue",
        "preview class, complex marker, value kind, or exact value is not representable in v1",
    )
}

const fn default_preview_elements() -> u64 {
    MAX_PREVIEW_ELEMENTS
}

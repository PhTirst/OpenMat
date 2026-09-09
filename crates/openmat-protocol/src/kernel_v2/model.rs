use std::collections::{BTreeMap, BTreeSet};

use serde::de::{Error as _, MapAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, SerializeStruct};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{
    MAX_AGGREGATE_DEPTH, MAX_AGGREGATE_ELEMENTS, MAX_AGGREGATE_NODES, NegotiationError,
    PROTOCOL_V2, ProtocolVersion,
};
use crate::kernel_v1::{
    MAX_PREVIEW_CODE_UNITS, MAX_SAFE_JSON_INTEGER, MAX_STRING_ELEMENT_CODE_UNITS, PreviewLimits,
};
use crate::{
    Diagnostic, Event, ExecuteRequest, ExecuteResult, ExecutionMode, ImplementationInfo,
    InterruptRequest, InterruptResult, KernelStatus, ListWorkspaceRequest, MAX_PREVIEW_ELEMENTS,
    MessageKind, PROTOCOL_V0, PreviewTruncation, ProtocolError, ShutdownRequest, ShutdownResult,
    ValidationError, VariableSummary, WorkspaceSummary,
};

pub use crate::kernel_v1::{InspectRequest, MatrixRange};

/// Bootstrap capabilities for v2/v1/v0 negotiation.
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
    /// Maximum UTF-16 units in one string element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_string_element_code_units: Option<u64>,
    /// Maximum semantic UTF-16 units in one preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_preview_code_units: Option<u64>,
    /// Maximum recursive exact-value node count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_aggregate_nodes: Option<u64>,
    /// Maximum aggregate semantic element count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_aggregate_elements: Option<u64>,
    /// Maximum recursive exact-value depth, with root depth zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_aggregate_depth: Option<u64>,
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
            max_aggregate_nodes: None,
            max_aggregate_elements: None,
            max_aggregate_depth: None,
            interrupt: false,
            workspace_delta: false,
        }
    }
}

impl Capabilities {
    /// Returns default capabilities advertising all v2 hard limits.
    #[must_use]
    pub fn v2_hard_limits() -> Self {
        Self {
            max_string_element_code_units: Some(MAX_STRING_ELEMENT_CODE_UNITS),
            max_preview_code_units: Some(MAX_PREVIEW_CODE_UNITS),
            max_aggregate_nodes: Some(MAX_AGGREGATE_NODES),
            max_aggregate_elements: Some(MAX_AGGREGATE_ELEMENTS),
            max_aggregate_depth: Some(MAX_AGGREGATE_DEPTH),
            ..Self::default()
        }
    }

    /// Extracts and validates all six negotiated v2 limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] when a required field is absent, malformed,
    /// inconsistent, or outside its contract range.
    pub fn aggregate_limits(&self) -> Result<AggregateLimits, ValidationError> {
        AggregateLimits::new(
            self.max_preview_elements,
            required_limit(
                self.max_string_element_code_units,
                "maxStringElementCodeUnits",
                "v2",
            )?,
            required_limit(self.max_preview_code_units, "maxPreviewCodeUnits", "v2")?,
            required_limit(self.max_aggregate_nodes, "maxAggregateNodes", "v2")?,
            required_limit(self.max_aggregate_elements, "maxAggregateElements", "v2")?,
            required_limit(self.max_aggregate_depth, "maxAggregateDepth", "v2")?,
        )
    }

    fn preview_limits(&self, version: &'static str) -> Result<PreviewLimits, ValidationError> {
        PreviewLimits::new(
            self.max_preview_elements,
            required_limit(
                self.max_string_element_code_units,
                "maxStringElementCodeUnits",
                version,
            )?,
            required_limit(self.max_preview_code_units, "maxPreviewCodeUnits", version)?,
        )
    }

    /// Computes the component-wise common capabilities for `selected`.
    ///
    /// # Errors
    ///
    /// Returns [`NegotiationError`] if either side is invalid for the selected
    /// version.
    #[allow(clippy::too_many_lines)]
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

        let client_v1 = if selected == ProtocolVersion::V0 {
            None
        } else {
            Some(
                client
                    .preview_limits("v1 or v2")
                    .map_err(|error| invalid_capabilities_error(&error))?,
            )
        };
        let kernel_v1 = if selected == ProtocolVersion::V0 {
            None
        } else {
            Some(
                kernel
                    .preview_limits("v1 or v2")
                    .map_err(|error| invalid_capabilities_error(&error))?,
            )
        };
        let client_v2 = if matches!(selected, ProtocolVersion::V2 | ProtocolVersion::V3) {
            Some(
                client
                    .aggregate_limits()
                    .map_err(|error| invalid_capabilities_error(&error))?,
            )
        } else {
            None
        };
        let kernel_v2 = if matches!(selected, ProtocolVersion::V2 | ProtocolVersion::V3) {
            Some(
                kernel
                    .aggregate_limits()
                    .map_err(|error| invalid_capabilities_error(&error))?,
            )
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
        let (max_string_element_code_units, max_preview_code_units) = match (client_v1, kernel_v1) {
            (Some(client_limits), Some(kernel_limits)) => (
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
            ),
            _ => (None, None),
        };
        let (max_aggregate_nodes, max_aggregate_elements, max_aggregate_depth) =
            match (client_v2, kernel_v2) {
                (Some(client_limits), Some(kernel_limits)) => (
                    Some(
                        client_limits
                            .max_aggregate_nodes
                            .min(kernel_limits.max_aggregate_nodes)
                            .min(MAX_AGGREGATE_NODES),
                    ),
                    Some(
                        client_limits
                            .max_aggregate_elements
                            .min(kernel_limits.max_aggregate_elements)
                            .min(MAX_AGGREGATE_ELEMENTS),
                    ),
                    Some(
                        client_limits
                            .max_aggregate_depth
                            .min(kernel_limits.max_aggregate_depth)
                            .min(MAX_AGGREGATE_DEPTH),
                    ),
                ),
                _ => (None, None, None),
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
            max_aggregate_nodes,
            max_aggregate_elements,
            max_aggregate_depth,
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
                if self.has_v1_fields() || self.has_v2_fields() {
                    return Err(invalid_capabilities(
                        "v0-selected capabilities must omit v1 and v2 limits",
                    ));
                }
            }
            ProtocolVersion::V1 => {
                self.preview_limits("v1")?;
                if self.has_v2_fields() {
                    return Err(invalid_capabilities(
                        "v1-selected capabilities must omit v2 aggregate limits",
                    ));
                }
            }
            ProtocolVersion::V2 | ProtocolVersion::V3 => {
                self.aggregate_limits()?;
            }
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

    const fn has_v1_fields(&self) -> bool {
        self.max_string_element_code_units.is_some() || self.max_preview_code_units.is_some()
    }

    const fn has_v2_fields(&self) -> bool {
        self.max_aggregate_nodes.is_some()
            || self.max_aggregate_elements.is_some()
            || self.max_aggregate_depth.is_some()
    }
}

fn required_limit(
    value: Option<u64>,
    field: &'static str,
    version: &'static str,
) -> Result<u64, ValidationError> {
    value.ok_or_else(|| {
        invalid_capabilities(format!(
            "{field} is required when {version} is selected or offered"
        ))
    })
}

fn invalid_capabilities_error(error: &ValidationError) -> NegotiationError {
    NegotiationError::new("protocol.invalidCapabilities", error.to_string())
}

fn invalid_capabilities(message: impl Into<String>) -> ValidationError {
    ValidationError::new("protocol.invalidCapabilities", message)
}

/// The six negotiated v2 semantic limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AggregateLimits {
    /// Per-node and top-level preview element maximum.
    pub max_preview_elements: u64,
    /// Maximum UTF-16 units in one string element.
    pub max_string_element_code_units: u64,
    /// Maximum semantic UTF-16 units over the returned tree.
    pub max_preview_code_units: u64,
    /// Maximum node count including the aggregate-preview root.
    pub max_aggregate_nodes: u64,
    /// Maximum semantic element count.
    pub max_aggregate_elements: u64,
    /// Maximum node depth, with the preview root at zero.
    pub max_aggregate_depth: u64,
}

impl Default for AggregateLimits {
    fn default() -> Self {
        Self {
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            max_string_element_code_units: MAX_STRING_ELEMENT_CODE_UNITS,
            max_preview_code_units: MAX_PREVIEW_CODE_UNITS,
            max_aggregate_nodes: MAX_AGGREGATE_NODES,
            max_aggregate_elements: MAX_AGGREGATE_ELEMENTS,
            max_aggregate_depth: MAX_AGGREGATE_DEPTH,
        }
    }
}

impl AggregateLimits {
    /// Constructs and validates all v2 limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for an unsafe or out-of-range integer, or
    /// when the aggregate code-unit limit is below the per-string limit.
    pub fn new(
        max_preview_elements: u64,
        max_string_element_code_units: u64,
        max_preview_code_units: u64,
        max_aggregate_nodes: u64,
        max_aggregate_elements: u64,
        max_aggregate_depth: u64,
    ) -> Result<Self, ValidationError> {
        let limits = Self {
            max_preview_elements,
            max_string_element_code_units,
            max_preview_code_units,
            max_aggregate_nodes,
            max_aggregate_elements,
            max_aggregate_depth,
        };
        limits.validate()?;
        Ok(limits)
    }

    /// Returns the inherited v1 subset.
    #[must_use]
    pub const fn preview_limits(self) -> PreviewLimits {
        PreviewLimits {
            max_preview_elements: self.max_preview_elements,
            max_string_element_code_units: self.max_string_element_code_units,
            max_preview_code_units: self.max_preview_code_units,
        }
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
        validate_bounded_positive(
            "maxAggregateNodes",
            self.max_aggregate_nodes,
            MAX_AGGREGATE_NODES,
        )?;
        validate_bounded_positive(
            "maxAggregateElements",
            self.max_aggregate_elements,
            MAX_AGGREGATE_ELEMENTS,
        )?;
        validate_safe_json_integer("maxAggregateDepth", self.max_aggregate_depth)?;
        if self.max_aggregate_depth > MAX_AGGREGATE_DEPTH {
            return Err(invalid_capabilities(format!(
                "maxAggregateDepth must be between 0 and {MAX_AGGREGATE_DEPTH}"
            )));
        }
        if self.max_preview_code_units < self.max_string_element_code_units {
            return Err(invalid_capabilities(
                "maxPreviewCodeUnits must not be less than maxStringElementCodeUnits",
            ));
        }
        Ok(())
    }
}

/// Bootstrap request envelope; its outer protocol is always v0.
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

    /// Validates bootstrap envelope and offer invariants.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for any malformed bootstrap field.
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

/// The only legal bootstrap request.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "params", rename_all = "camelCase")]
pub enum BootstrapRequest {
    /// Negotiate protocol and capabilities.
    Initialize(InitializeRequest),
}

/// V2-capable initialize parameters transported in a v0 envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    /// Client implementation identity.
    pub client: ImplementationInfo,
    /// Unique protocol identifiers in client preference order.
    #[serde(default)]
    pub supported_protocols: Vec<String>,
    /// Client capability offer.
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl InitializeRequest {
    /// Constructs a production `[v2, v1, v0]` offer.
    #[must_use]
    pub fn v2(client: ImplementationInfo, capabilities: Capabilities) -> Self {
        Self {
            client,
            supported_protocols: super::production_protocol_offer(),
            capabilities,
        }
    }

    /// Validates this request before protocol selection.
    ///
    /// # Errors
    ///
    /// Returns [`NegotiationError`] for malformed identity, protocol list, or
    /// any capability required by an offered version.
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
        let mut seen = BTreeSet::new();
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
            .any(|protocol| protocol == crate::kernel_v1::PROTOCOL_V1)
        {
            self.capabilities
                .preview_limits("v1")
                .map_err(|error| invalid_capabilities_error(&error))?;
        }
        if self
            .supported_protocols
            .iter()
            .any(|protocol| protocol == PROTOCOL_V2)
        {
            self.capabilities
                .aggregate_limits()
                .map_err(|error| invalid_capabilities_error(&error))?;
        }
        Ok(())
    }
}

/// Bootstrap initialize response envelope; it remains a v0 envelope.
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
    /// Structured initialization failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl BootstrapResponseEnvelope {
    /// Constructs the last bootstrap-v0 envelope after successful negotiation.
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

    /// Validates the bootstrap envelope and result shape.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for any malformed envelope or result.
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

/// Successful v2-capable initialization result.
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
                    "negotiatedProtocol must name v0, v1, or v2",
                )
            })?;
        self.capabilities.validate_for_selection(selected)
    }
}

/// A post-bootstrap v2 request envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    /// Must be `openmat-kernel-v2`.
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
    /// Constructs a post-bootstrap v2 request.
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        request: Request,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V2.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Request,
            request,
        }
    }

    /// Validates the v2 envelope and request against negotiated limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for protocol mixing or malformed fields.
    pub fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Request,
            PROTOCOL_V2,
        )?;
        limits.validate()?;
        self.request.validate(limits)
    }
}

/// Post-bootstrap v2 requests. Initialize remains bootstrap-only.
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
    fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        match self {
            Self::Execute(request) => require_non_empty("sourceName", &request.source_name),
            Self::Inspect(request) => request.validate(&limits.preview_limits()),
            Self::Interrupt(_) | Self::ListWorkspace(_) | Self::Shutdown(_) => Ok(()),
        }
    }

    /// Returns true for requests that may bypass the sequential queue.
    #[must_use]
    pub const fn is_control(&self) -> bool {
        matches!(self, Self::Interrupt(_) | Self::Shutdown(_))
    }
}

/// A post-bootstrap v2 response envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    /// Must be `openmat-kernel-v2`.
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
    /// Constructs a successful v2 response.
    #[must_use]
    pub fn success(
        request: &RequestEnvelope,
        message_id: impl Into<String>,
        result: ResponseResult,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V2.to_owned(),
            session_id: request.session_id.clone(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: request.message_id.clone(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Constructs a failed v2 response.
    #[must_use]
    pub fn failure(
        request: &RequestEnvelope,
        message_id: impl Into<String>,
        error: ProtocolError,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V2.to_owned(),
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

    /// Validates the response against negotiated limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for protocol mixing or malformed content.
    pub fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Response,
            PROTOCOL_V2,
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

    /// Validates correlation, result type, selected range, and request limit.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for an ordinary response error or any
    /// mismatch with `request`.
    pub fn validate_for_request(
        &self,
        request: &RequestEnvelope,
        limits: &AggregateLimits,
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
                if preview.selected_range() != &inspect.range {
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

/// Successful post-bootstrap v2 response payloads.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ResponseResult {
    /// Execution completion result.
    Execute(ExecuteResult),
    /// Interrupt acknowledgement.
    Interrupt(InterruptResult),
    /// Matrix or recursive aggregate inspection result.
    Inspect(InspectPreview),
    /// Workspace summaries.
    ListWorkspace(WorkspaceSummary),
    /// Shutdown acknowledgement.
    Shutdown(ShutdownResult),
}

impl ResponseResult {
    fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        match self {
            Self::Inspect(preview) => preview.validate(limits),
            Self::ListWorkspace(workspace) => validate_workspace_summary(workspace),
            Self::Execute(_) | Self::Interrupt(_) | Self::Shutdown(_) => Ok(()),
        }
    }
}

/// A post-bootstrap v2 event envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    /// Must be `openmat-kernel-v2`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque event identifier.
    pub message_id: String,
    /// Must be `event`.
    pub kind: MessageKind,
    /// Known or forward-compatible unknown event.
    pub event: Event,
}

impl EventEnvelope {
    /// Constructs a v2 event envelope.
    #[must_use]
    pub fn new(session_id: impl Into<String>, message_id: impl Into<String>, event: Event) -> Self {
        Self {
            protocol: PROTOCOL_V2.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Event,
            event,
        }
    }

    /// Validates the v2 header and known event payload.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for protocol mixing or malformed known data.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Event,
            PROTOCOL_V2,
        )?;
        validate_event(&self.event)
    }
}

/// Any post-bootstrap server-to-client v2 message.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ServerMessage {
    /// A response to a request.
    Response(ResponseEnvelope),
    /// An unsolicited event.
    Event(EventEnvelope),
}

impl ServerMessage {
    /// Validates the contained envelope.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] when the contained message is invalid.
    pub fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        match self {
            Self::Response(response) => response.validate(limits),
            Self::Event(event) => event.validate(),
        }
    }
}

/// Successful v2 inspect result: the exact v1 matrix shape or a v2 aggregate.
#[derive(Clone, Debug, PartialEq)]
pub enum InspectPreview {
    /// Non-aggregate value using the frozen v1 JSON shape.
    Matrix(crate::kernel_v1::MatrixPreview),
    /// Cell, struct, or table value using a recursive v2 preview.
    Aggregate(AggregatePreview),
}

impl InspectPreview {
    /// Returns the one-based selected range for request correlation.
    #[must_use]
    pub const fn selected_range(&self) -> &MatrixRange {
        match self {
            Self::Matrix(preview) => &preview.selected_range,
            Self::Aggregate(
                AggregatePreview::Cell { selected_range, .. }
                | AggregatePreview::Struct { selected_range, .. }
                | AggregatePreview::Table { selected_range, .. },
            ) => selected_range,
        }
    }

    /// Validates the preview against negotiated limits.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for any matrix-v1 or aggregate-v2 invariant
    /// violation.
    pub fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        match self {
            Self::Matrix(preview) => preview.validate(&limits.preview_limits()),
            Self::Aggregate(preview) => preview.validate(limits),
        }
    }

    /// Validates the preview against the originating request's element bound.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] if the request bound or preview is invalid.
    pub fn validate_for_request(
        &self,
        limits: &AggregateLimits,
        request_max_elements: u64,
    ) -> Result<(), ValidationError> {
        match self {
            Self::Matrix(preview) => {
                preview.validate_for_request(&limits.preview_limits(), request_max_elements)
            }
            Self::Aggregate(preview) => preview.validate_for_request(limits, request_max_elements),
        }
    }
}

impl Serialize for InspectPreview {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Matrix(preview) => preview.serialize(serializer),
            Self::Aggregate(preview) => preview.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for InspectPreview {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match RawInspectPreview::deserialize(deserializer)? {
            RawInspectPreview::Aggregate(preview) => Ok(Self::Aggregate(preview)),
            RawInspectPreview::Matrix(raw) => {
                if raw.extra.contains_key("kind") {
                    return Err(D::Error::custom(
                        "unknown or matrix preview kind is not permitted",
                    ));
                }
                Ok(Self::Matrix(crate::kernel_v1::MatrixPreview {
                    class: raw.class,
                    dimensions: raw.dimensions,
                    complex: raw.complex,
                    selected_range: raw.selected_range,
                    values: raw.values,
                    truncation: raw.truncation,
                }))
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawInspectPreview {
    Aggregate(AggregatePreview),
    Matrix(RawMatrixPreview),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMatrixPreview {
    class: String,
    dimensions: Vec<u64>,
    complex: bool,
    selected_range: MatrixRange,
    values: Vec<crate::kernel_v1::PreviewValue>,
    truncation: PreviewTruncation,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

/// Recursive v2 preview for a selected cell, struct, or table range.
#[derive(Clone, Debug, PartialEq)]
pub enum AggregatePreview {
    /// Cell preview with whole exact top-level items.
    Cell {
        /// Complete workspace value dimensions.
        dimensions: Vec<u64>,
        /// One-based selected range.
        selected_range: MatrixRange,
        /// Longest complete top-level prefix.
        items: Vec<ExactValue>,
        /// Omitted top-level element metadata.
        truncation: PreviewTruncation,
        /// Recomputed semantic usage claim.
        usage: PreviewUsage,
    },
    /// Struct preview with ordered schema and whole records.
    Struct {
        /// Complete workspace value dimensions.
        dimensions: Vec<u64>,
        /// One-based selected range.
        selected_range: MatrixRange,
        /// Unique ordered ASCII field schema.
        fields: Vec<String>,
        /// Longest complete top-level record prefix.
        records: Vec<StructRecord>,
        /// Omitted top-level element metadata.
        truncation: PreviewTruncation,
        /// Recomputed semantic usage claim.
        usage: PreviewUsage,
    },
    /// Table preview with an ordered prefix of complete selected variables.
    Table {
        /// Complete table dimensions `[rows, variables]`.
        dimensions: Vec<u64>,
        /// One-based `[rows, variables]` selection shared by every variable.
        selected_range: MatrixRange,
        /// Names of the returned variable prefix, in table order.
        variable_names: Vec<String>,
        /// Complete row slices for the returned variable prefix.
        variables: Vec<ExactValue>,
        /// Omitted selected-variable metadata. `omittedElements` is measured
        /// in table variables, not table cells or exact-value elements.
        truncation: PreviewTruncation,
        /// Recomputed semantic usage claim.
        usage: PreviewUsage,
    },
}

impl AggregatePreview {
    /// Validates the preview against negotiated v2 limits.
    ///
    /// Validation uses an explicit work stack, so a programmatically built
    /// deeply nested tree is rejected at the depth boundary before recursive
    /// serde serialization can exhaust the host stack.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] for non-canonical metadata, limit overflow,
    /// malformed truncation, field-set mismatch, or usage disagreement.
    pub fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        self.validate_with_element_limit(limits, limits.max_preview_elements)
    }

    /// Validates with the originating inspect request's top-level limit.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] when the request limit or preview is invalid.
    pub fn validate_for_request(
        &self,
        limits: &AggregateLimits,
        request_max_elements: u64,
    ) -> Result<(), ValidationError> {
        validate_request_element_bound(request_max_elements, limits.max_preview_elements)?;
        self.validate_with_element_limit(limits, request_max_elements)
    }

    fn validate_with_element_limit(
        &self,
        limits: &AggregateLimits,
        element_limit: u64,
    ) -> Result<(), ValidationError> {
        limits.validate()?;
        match self {
            Self::Cell {
                dimensions,
                selected_range,
                items,
                truncation,
                usage,
            } => validate_aggregate(
                AggregateView::Cell(items),
                dimensions,
                selected_range,
                truncation,
                usage,
                limits,
                element_limit,
            ),
            Self::Struct {
                dimensions,
                selected_range,
                fields,
                records,
                truncation,
                usage,
            } => validate_aggregate(
                AggregateView::Struct { fields, records },
                dimensions,
                selected_range,
                truncation,
                usage,
                limits,
                element_limit,
            ),
            Self::Table {
                dimensions,
                selected_range,
                variable_names,
                variables,
                truncation,
                usage,
            } => validate_table_aggregate(
                dimensions,
                selected_range,
                variable_names,
                variables,
                truncation,
                usage,
                limits,
                element_limit,
            ),
        }
    }
}

/// Semantic usage of the committed returned aggregate tree.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewUsage {
    /// Aggregate root plus every returned exact node.
    pub nodes: u64,
    /// Returned cell elements, struct records, or table variables, plus every
    /// exact node's `numel`.
    pub elements: u64,
    /// Char, string, field-schema, and table-variable-name UTF-16 code units.
    pub code_units: u64,
    /// Greatest committed exact-node depth.
    pub depth: u64,
}

/// A complete recursive schema-v2 exact value.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactValue {
    /// Exact MATLAB-style class.
    pub class: String,
    /// Complete shape with at least two dimensions.
    pub size: Vec<u64>,
    /// Must equal `size.len()`.
    pub ndims: u64,
    /// Checked product of `size`.
    pub numel: u64,
    /// Whether numeric or integer storage is complex.
    pub complex: bool,
    /// Canonical payload and exact-value kind.
    pub payload: ExactPayload,
}

/// Canonical payload union for [`ExactValue`].
#[derive(Clone, Debug, PartialEq)]
pub enum ExactPayload {
    /// `double` or `single` arrays using normalized number strings.
    Numeric {
        /// Real components in column-major order.
        real: Vec<String>,
        /// Imaginary components in column-major order.
        imag: Vec<String>,
    },
    /// Fixed-width integer arrays using exact decimal component strings.
    Integer {
        /// Components in column-major order.
        integer: Vec<IntegerValue>,
    },
    /// Logical array payload.
    Logical {
        /// Boolean elements in column-major order.
        logical: Vec<bool>,
    },
    /// Character array payload.
    Char {
        /// Exact UTF-16 code units in column-major order.
        code_units: Vec<u16>,
    },
    /// String array payload.
    String {
        /// Exact UTF-16 code-unit sequences in column-major order.
        string_code_units: Vec<Vec<u16>>,
        /// Parallel missing markers.
        missing: Vec<bool>,
    },
    /// Cell array payload.
    Cell {
        /// Complete contained values in column-major order.
        items: Vec<ExactValue>,
    },
    /// Struct array payload.
    Struct {
        /// Unique ordered ASCII field schema.
        fields: Vec<String>,
        /// Complete records in column-major order.
        records: Vec<StructRecord>,
    },
    /// Heterogeneous table payload.
    Table {
        /// Unique ordered variable names.
        variable_names: Vec<String>,
        /// Complete variables in schema order.
        variables: Vec<ExactValue>,
    },
}

/// Exact fixed-width integer components.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntegerValue {
    /// Real component.
    pub real: String,
    /// Imaginary component.
    pub imaginary: String,
}

/// One struct record represented internally without observable member order.
///
/// The entries may arrive in any JSON member order. Parent exact/preview
/// serializers always look values up and emit them in the corresponding
/// `fields` order.
#[derive(Clone, Debug, PartialEq)]
pub struct StructRecord {
    /// Field/value entries. Names are validated against the parent schema.
    pub entries: Vec<(String, ExactValue)>,
}

impl StructRecord {
    /// Constructs a record from named values.
    #[must_use]
    pub const fn new(entries: Vec<(String, ExactValue)>) -> Self {
        Self { entries }
    }

    fn get(&self, field: &str) -> Option<&ExactValue> {
        self.entries
            .iter()
            .find_map(|(name, value)| (name == field).then_some(value))
    }
}

impl Serialize for StructRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (name, value) in &self.entries {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for StructRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RecordVisitor;

        impl<'de> Visitor<'de> for RecordVisitor {
            type Value = StructRecord;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON object containing exact struct fields")
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut entries = Vec::with_capacity(access.size_hint().unwrap_or(0));
                while let Some((name, value)) = access.next_entry::<String, ExactValue>()? {
                    if entries
                        .iter()
                        .any(|(existing, _): &(String, ExactValue)| existing == &name)
                    {
                        return Err(M::Error::custom(format!(
                            "duplicate struct record member {name}"
                        )));
                    }
                    entries.push((name, value));
                }
                Ok(StructRecord { entries })
            }
        }

        deserializer.deserialize_map(RecordVisitor)
    }
}

struct OrderedRecord<'a> {
    fields: &'a [String],
    record: &'a StructRecord,
}

impl Serialize for OrderedRecord<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for field in self.fields {
            let value = self.record.get(field).ok_or_else(|| {
                serde::ser::Error::custom(format!("record is missing field {field}"))
            })?;
            map.serialize_entry(field, value)?;
        }
        map.end()
    }
}

struct OrderedRecords<'a> {
    fields: &'a [String],
    records: &'a [StructRecord],
}

impl Serialize for OrderedRecords<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.records.len()))?;
        for record in self.records {
            sequence.serialize_element(&OrderedRecord {
                fields: self.fields,
                record,
            })?;
        }
        sequence.end()
    }
}

impl Serialize for ExactValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let payload_fields = match &self.payload {
            ExactPayload::Integer { .. }
            | ExactPayload::Logical { .. }
            | ExactPayload::Char { .. }
            | ExactPayload::Cell { .. } => 1,
            ExactPayload::Numeric { .. }
            | ExactPayload::String { .. }
            | ExactPayload::Struct { .. }
            | ExactPayload::Table { .. } => 2,
        };
        let mut state = serializer.serialize_struct("ExactValue", 6 + payload_fields)?;
        state.serialize_field("class", &self.class)?;
        state.serialize_field("size", &self.size)?;
        state.serialize_field("ndims", &self.ndims)?;
        state.serialize_field("numel", &self.numel)?;
        state.serialize_field("complex", &self.complex)?;
        match &self.payload {
            ExactPayload::Numeric { real, imag } => {
                state.serialize_field("kind", "numeric")?;
                state.serialize_field("real", real)?;
                state.serialize_field("imag", imag)?;
            }
            ExactPayload::Integer { integer } => {
                state.serialize_field("kind", "integer")?;
                state.serialize_field("integer", integer)?;
            }
            ExactPayload::Logical { logical } => {
                state.serialize_field("kind", "logical")?;
                state.serialize_field("logical", logical)?;
            }
            ExactPayload::Char { code_units } => {
                state.serialize_field("kind", "char")?;
                state.serialize_field("code_units", code_units)?;
            }
            ExactPayload::String {
                string_code_units,
                missing,
            } => {
                state.serialize_field("kind", "string")?;
                state.serialize_field("string_code_units", string_code_units)?;
                state.serialize_field("missing", missing)?;
            }
            ExactPayload::Cell { items } => {
                state.serialize_field("kind", "cell")?;
                state.serialize_field("items", items)?;
            }
            ExactPayload::Struct { fields, records } => {
                state.serialize_field("kind", "struct")?;
                state.serialize_field("fields", fields)?;
                state.serialize_field("records", &OrderedRecords { fields, records })?;
            }
            ExactPayload::Table {
                variable_names,
                variables,
            } => {
                state.serialize_field("kind", "table")?;
                state.serialize_field("variableNames", variable_names)?;
                state.serialize_field("variables", variables)?;
            }
        }
        state.end()
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum RawExactValue {
    #[serde(rename = "numeric")]
    Numeric(RawExactNumeric),
    #[serde(rename = "integer")]
    Integer(RawExactInteger),
    #[serde(rename = "logical")]
    Logical(RawExactLogical),
    #[serde(rename = "char")]
    Char(RawExactChar),
    #[serde(rename = "string")]
    String(RawExactString),
    #[serde(rename = "cell")]
    Cell(RawExactCell),
    #[serde(rename = "struct")]
    Struct(RawExactStruct),
    #[serde(rename = "table")]
    Table(RawExactTable),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExactNumeric {
    class: String,
    size: Vec<u64>,
    ndims: u64,
    numel: u64,
    complex: bool,
    real: Vec<String>,
    imag: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExactInteger {
    class: String,
    size: Vec<u64>,
    ndims: u64,
    numel: u64,
    complex: bool,
    integer: Vec<IntegerValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExactLogical {
    class: String,
    size: Vec<u64>,
    ndims: u64,
    numel: u64,
    complex: bool,
    logical: Vec<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExactChar {
    class: String,
    size: Vec<u64>,
    ndims: u64,
    numel: u64,
    complex: bool,
    code_units: Vec<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExactString {
    class: String,
    size: Vec<u64>,
    ndims: u64,
    numel: u64,
    complex: bool,
    string_code_units: Vec<Vec<u16>>,
    missing: Vec<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExactCell {
    class: String,
    size: Vec<u64>,
    ndims: u64,
    numel: u64,
    complex: bool,
    items: Vec<ExactValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExactStruct {
    class: String,
    size: Vec<u64>,
    ndims: u64,
    numel: u64,
    complex: bool,
    fields: Vec<String>,
    records: Vec<StructRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawExactTable {
    class: String,
    size: Vec<u64>,
    ndims: u64,
    numel: u64,
    complex: bool,
    variable_names: Vec<String>,
    variables: Vec<ExactValue>,
}

impl<'de> Deserialize<'de> for ExactValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawExactValue::deserialize(deserializer)?;
        Ok(match raw {
            RawExactValue::Numeric(raw) => Self {
                class: raw.class,
                size: raw.size,
                ndims: raw.ndims,
                numel: raw.numel,
                complex: raw.complex,
                payload: ExactPayload::Numeric {
                    real: raw.real,
                    imag: raw.imag,
                },
            },
            RawExactValue::Integer(raw) => Self {
                class: raw.class,
                size: raw.size,
                ndims: raw.ndims,
                numel: raw.numel,
                complex: raw.complex,
                payload: ExactPayload::Integer {
                    integer: raw.integer,
                },
            },
            RawExactValue::Logical(raw) => Self {
                class: raw.class,
                size: raw.size,
                ndims: raw.ndims,
                numel: raw.numel,
                complex: raw.complex,
                payload: ExactPayload::Logical {
                    logical: raw.logical,
                },
            },
            RawExactValue::Char(raw) => Self {
                class: raw.class,
                size: raw.size,
                ndims: raw.ndims,
                numel: raw.numel,
                complex: raw.complex,
                payload: ExactPayload::Char {
                    code_units: raw.code_units,
                },
            },
            RawExactValue::String(raw) => Self {
                class: raw.class,
                size: raw.size,
                ndims: raw.ndims,
                numel: raw.numel,
                complex: raw.complex,
                payload: ExactPayload::String {
                    string_code_units: raw.string_code_units,
                    missing: raw.missing,
                },
            },
            RawExactValue::Cell(raw) => Self {
                class: raw.class,
                size: raw.size,
                ndims: raw.ndims,
                numel: raw.numel,
                complex: raw.complex,
                payload: ExactPayload::Cell { items: raw.items },
            },
            RawExactValue::Struct(raw) => Self {
                class: raw.class,
                size: raw.size,
                ndims: raw.ndims,
                numel: raw.numel,
                complex: raw.complex,
                payload: ExactPayload::Struct {
                    fields: raw.fields,
                    records: raw.records,
                },
            },
            RawExactValue::Table(raw) => Self {
                class: raw.class,
                size: raw.size,
                ndims: raw.ndims,
                numel: raw.numel,
                complex: raw.complex,
                payload: ExactPayload::Table {
                    variable_names: raw.variable_names,
                    variables: raw.variables,
                },
            },
        })
    }
}

impl Serialize for AggregatePreview {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Cell {
                dimensions,
                selected_range,
                items,
                truncation,
                usage,
            } => {
                let mut state = serializer.serialize_struct("CellAggregatePreview", 8)?;
                state.serialize_field("class", "cell")?;
                state.serialize_field("dimensions", dimensions)?;
                state.serialize_field("complex", &false)?;
                state.serialize_field("selectedRange", selected_range)?;
                state.serialize_field("kind", "cell")?;
                state.serialize_field("items", items)?;
                state.serialize_field("truncation", truncation)?;
                state.serialize_field("usage", usage)?;
                state.end()
            }
            Self::Struct {
                dimensions,
                selected_range,
                fields,
                records,
                truncation,
                usage,
            } => {
                let mut state = serializer.serialize_struct("StructAggregatePreview", 9)?;
                state.serialize_field("class", "struct")?;
                state.serialize_field("dimensions", dimensions)?;
                state.serialize_field("complex", &false)?;
                state.serialize_field("selectedRange", selected_range)?;
                state.serialize_field("kind", "struct")?;
                state.serialize_field("fields", fields)?;
                state.serialize_field("records", &OrderedRecords { fields, records })?;
                state.serialize_field("truncation", truncation)?;
                state.serialize_field("usage", usage)?;
                state.end()
            }
            Self::Table {
                dimensions,
                selected_range,
                variable_names,
                variables,
                truncation,
                usage,
            } => {
                let mut state = serializer.serialize_struct("TableAggregatePreview", 9)?;
                state.serialize_field("class", "table")?;
                state.serialize_field("dimensions", dimensions)?;
                state.serialize_field("complex", &false)?;
                state.serialize_field("selectedRange", selected_range)?;
                state.serialize_field("kind", "table")?;
                state.serialize_field("variableNames", variable_names)?;
                state.serialize_field("variables", variables)?;
                state.serialize_field("truncation", truncation)?;
                state.serialize_field("usage", usage)?;
                state.end()
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum RawAggregatePreview {
    #[serde(rename = "cell")]
    Cell(RawCellAggregatePreview),
    #[serde(rename = "struct")]
    Struct(RawStructAggregatePreview),
    #[serde(rename = "table")]
    Table(RawTableAggregatePreview),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCellAggregatePreview {
    class: String,
    dimensions: Vec<u64>,
    complex: bool,
    selected_range: MatrixRange,
    items: Vec<ExactValue>,
    truncation: PreviewTruncation,
    usage: PreviewUsage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawStructAggregatePreview {
    class: String,
    dimensions: Vec<u64>,
    complex: bool,
    selected_range: MatrixRange,
    fields: Vec<String>,
    records: Vec<StructRecord>,
    truncation: PreviewTruncation,
    usage: PreviewUsage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTableAggregatePreview {
    class: String,
    dimensions: Vec<u64>,
    complex: bool,
    selected_range: MatrixRange,
    variable_names: Vec<String>,
    variables: Vec<ExactValue>,
    truncation: PreviewTruncation,
    usage: PreviewUsage,
}

impl<'de> Deserialize<'de> for AggregatePreview {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match RawAggregatePreview::deserialize(deserializer)? {
            RawAggregatePreview::Cell(raw) => {
                if raw.class != "cell" || raw.complex {
                    return Err(D::Error::custom(
                        "cell aggregate requires class cell and complex false",
                    ));
                }
                Ok(Self::Cell {
                    dimensions: raw.dimensions,
                    selected_range: raw.selected_range,
                    items: raw.items,
                    truncation: raw.truncation,
                    usage: raw.usage,
                })
            }
            RawAggregatePreview::Struct(raw) => {
                if raw.class != "struct" || raw.complex {
                    return Err(D::Error::custom(
                        "struct aggregate requires class struct and complex false",
                    ));
                }
                Ok(Self::Struct {
                    dimensions: raw.dimensions,
                    selected_range: raw.selected_range,
                    fields: raw.fields,
                    records: raw.records,
                    truncation: raw.truncation,
                    usage: raw.usage,
                })
            }
            RawAggregatePreview::Table(raw) => {
                if raw.class != "table" || raw.complex {
                    return Err(D::Error::custom(
                        "table aggregate requires class table and complex false",
                    ));
                }
                Ok(Self::Table {
                    dimensions: raw.dimensions,
                    selected_range: raw.selected_range,
                    variable_names: raw.variable_names,
                    variables: raw.variables,
                    truncation: raw.truncation,
                    usage: raw.usage,
                })
            }
        }
    }
}

#[derive(Clone, Copy)]
enum AggregateView<'a> {
    Cell(&'a [ExactValue]),
    Struct {
        fields: &'a [String],
        records: &'a [StructRecord],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsageCounter {
    nodes: u64,
    elements: u64,
    code_units: u64,
    depth: u64,
}

impl UsageCounter {
    fn root(limits: &AggregateLimits) -> Result<Self, ValidationError> {
        if limits.max_aggregate_nodes < 1 {
            return Err(preview_limit("aggregate root exceeds the node limit"));
        }
        Ok(Self {
            nodes: 1,
            elements: 0,
            code_units: 0,
            depth: 0,
        })
    }

    fn charge_node(
        &mut self,
        numel: u64,
        depth: u64,
        limits: &AggregateLimits,
    ) -> Result<(), ValidationError> {
        self.nodes = checked_add_safe(self.nodes, 1, "usage.nodes")?;
        self.elements = checked_add_safe(self.elements, numel, "usage.elements")?;
        self.depth = self.depth.max(depth);
        if self.nodes > limits.max_aggregate_nodes {
            return Err(preview_limit("aggregate node limit exceeded"));
        }
        if self.elements > limits.max_aggregate_elements {
            return Err(preview_limit("aggregate element limit exceeded"));
        }
        Ok(())
    }

    fn charge_root_element(&mut self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        self.elements = checked_add_safe(self.elements, 1, "usage.elements")?;
        if self.elements > limits.max_aggregate_elements {
            return Err(preview_limit("aggregate element limit exceeded"));
        }
        Ok(())
    }

    fn charge_code_units(
        &mut self,
        count: u64,
        limits: &AggregateLimits,
    ) -> Result<(), ValidationError> {
        self.code_units = checked_add_safe(self.code_units, count, "usage.codeUnits")?;
        if self.code_units > limits.max_preview_code_units {
            return Err(preview_limit("aggregate code-unit limit exceeded"));
        }
        Ok(())
    }

    const fn into_public(self) -> PreviewUsage {
        PreviewUsage {
            nodes: self.nodes,
            elements: self.elements,
            code_units: self.code_units,
            depth: self.depth,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_aggregate(
    aggregate: AggregateView<'_>,
    dimensions: &[u64],
    selected_range: &MatrixRange,
    truncation: &PreviewTruncation,
    reported_usage: &PreviewUsage,
    limits: &AggregateLimits,
    element_limit: u64,
) -> Result<(), ValidationError> {
    validate_shape(dimensions, "dimensions")?;
    selected_range.validate()?;
    if dimensions.len() != selected_range.start.len() {
        return Err(invalid_preview(
            "dimensions and selectedRange must have the same rank",
        ));
    }
    let selected_count = checked_product_safe(&selected_range.size, "selectedRange.size")?;
    if selected_count != 0 {
        for ((start, size), dimension) in selected_range
            .start
            .iter()
            .zip(&selected_range.size)
            .zip(dimensions)
        {
            let end = start
                .checked_add(*size)
                .and_then(|value| value.checked_sub(1))
                .ok_or_else(|| invalid_preview("selected range end overflows"))?;
            validate_safe_json_integer("selected range end", end)?;
            if end > *dimension {
                return Err(invalid_preview(
                    "nonempty selected range exceeds full dimensions",
                ));
            }
        }
    }

    let returned = match aggregate {
        AggregateView::Cell(items) => usize_to_safe(items.len(), "items.length")?,
        AggregateView::Struct { records, .. } => usize_to_safe(records.len(), "records.length")?,
    };
    let effective_limit = element_limit.min(limits.max_preview_elements);
    if returned > effective_limit {
        return Err(preview_limit(format!(
            "returned top-level element count exceeds effective limit {effective_limit}"
        )));
    }
    validate_safe_json_integer("truncation.omittedElements", truncation.omitted_elements)?;
    let represented = checked_add_safe(
        returned,
        truncation.omitted_elements,
        "returned plus omitted elements",
    )?;
    if represented != selected_count || truncation.truncated != (truncation.omitted_elements != 0) {
        return Err(ValidationError::new(
            "preview_truncation",
            "returned prefix and truncation do not match selectedRange.size",
        ));
    }

    let mut usage = UsageCounter::root(limits)?;
    match aggregate {
        AggregateView::Cell(items) => {
            for item in items {
                usage.charge_root_element(limits)?;
                validate_exact_tree(item, 1, &mut usage, limits)?;
            }
        }
        AggregateView::Struct { fields, records } => {
            validate_fields(fields, &mut usage, limits)?;
            for record in records {
                validate_record(record, fields)?;
                usage.charge_root_element(limits)?;
                for field in fields {
                    let value = record
                        .get(field)
                        .ok_or_else(|| invalid_preview("struct record field set mismatch"))?;
                    validate_exact_tree(value, 1, &mut usage, limits)?;
                }
            }
        }
    }
    let recomputed = usage.into_public();
    validate_usage_safe(reported_usage)?;
    if &recomputed != reported_usage {
        return Err(invalid_preview(format!(
            "reported usage {reported_usage:?} does not match recomputed usage {recomputed:?}"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_table_aggregate(
    dimensions: &[u64],
    selected_range: &MatrixRange,
    variable_names: &[String],
    variables: &[ExactValue],
    truncation: &PreviewTruncation,
    reported_usage: &PreviewUsage,
    limits: &AggregateLimits,
    element_limit: u64,
) -> Result<(), ValidationError> {
    validate_shape(dimensions, "dimensions")?;
    if dimensions.len() != 2 {
        return Err(invalid_preview(
            "table dimensions must be exactly [rows, variables]",
        ));
    }
    selected_range.validate()?;
    if selected_range.start.len() != 2 {
        return Err(invalid_preview(
            "table selectedRange must be exactly [rows, variables]",
        ));
    }
    for ((start, size), dimension) in selected_range
        .start
        .iter()
        .zip(&selected_range.size)
        .zip(dimensions)
    {
        if *size == 0 {
            continue;
        }
        let end = start
            .checked_add(*size)
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| invalid_preview("selected range end overflows"))?;
        validate_safe_json_integer("selected range end", end)?;
        if end > *dimension {
            return Err(invalid_preview(
                "nonempty selected range exceeds full table dimensions",
            ));
        }
    }

    let returned = usize_to_safe(variables.len(), "variables.length")?;
    if usize_to_safe(variable_names.len(), "variableNames.length")? != returned {
        return Err(invalid_preview(
            "table variableNames and variables counts must match",
        ));
    }
    let effective_limit = element_limit.min(limits.max_preview_elements);
    if returned > effective_limit {
        return Err(preview_limit(format!(
            "returned table variable count exceeds effective limit {effective_limit}"
        )));
    }
    validate_safe_json_integer("truncation.omittedElements", truncation.omitted_elements)?;
    let represented = checked_add_safe(
        returned,
        truncation.omitted_elements,
        "returned plus omitted table variables",
    )?;
    let selected_variables = selected_range.size[1];
    if represented != selected_variables
        || truncation.truncated != (truncation.omitted_elements != 0)
    {
        return Err(ValidationError::new(
            "preview_truncation",
            "returned variable prefix and truncation do not match selectedRange variable extent",
        ));
    }

    let mut usage = UsageCounter::root(limits)?;
    validate_table_schema(variable_names, returned, &mut usage, limits)?;
    let selected_rows = selected_range.size[0];
    for variable in variables {
        if variable.size.first().copied() != Some(selected_rows) {
            return Err(invalid_preview(
                "table preview variable first dimension must equal selected row count",
            ));
        }
        usage.charge_root_element(limits)?;
        validate_exact_tree(variable, 1, &mut usage, limits)?;
    }
    let recomputed = usage.into_public();
    validate_usage_safe(reported_usage)?;
    if &recomputed != reported_usage {
        return Err(invalid_preview(format!(
            "reported usage {reported_usage:?} does not match recomputed usage {recomputed:?}"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_exact_tree(
    root: &ExactValue,
    root_depth: u64,
    usage: &mut UsageCounter,
    limits: &AggregateLimits,
) -> Result<(), ValidationError> {
    let mut work = vec![(root, root_depth)];
    while let Some((value, depth)) = work.pop() {
        validate_exact_kind_metadata(value)?;
        if depth > limits.max_aggregate_depth {
            return Err(ValidationError::new(
                "workspace.previewDepth",
                "exact value exceeds the negotiated depth limit",
            ));
        }
        let product = validate_shape(&value.size, "size")?;
        validate_safe_json_integer("ndims", value.ndims)?;
        validate_safe_json_integer("numel", value.numel)?;
        if value.ndims != usize_to_safe(value.size.len(), "size.length")? || value.numel != product
        {
            return Err(invalid_preview(
                "ndims and numel must match the exact value shape",
            ));
        }
        if value.numel > limits.max_preview_elements {
            return Err(preview_limit("exact node exceeds maxPreviewElements"));
        }
        usage.charge_node(value.numel, depth, limits)?;

        match &value.payload {
            ExactPayload::Numeric { real, imag } => {
                validate_payload_count(real.len(), value.numel, "real")?;
                validate_payload_count(imag.len(), value.numel, "imag")?;
                let mut any_imaginary = false;
                for component in real {
                    validate_number_string(component)?;
                }
                for component in imag {
                    validate_number_string(component)?;
                    any_imaginary |= !number_string_is_zero(component);
                    if !value.complex && component != "0" {
                        return Err(invalid_preview(
                            "real numeric storage requires every imag component to be exactly 0",
                        ));
                    }
                }
                if value.complex && !any_imaginary {
                    return Err(invalid_preview(
                        "complex numeric storage requires a nonzero imaginary component",
                    ));
                }
            }
            ExactPayload::Integer { integer } => {
                validate_payload_count(integer.len(), value.numel, "integer")?;
                let class = IntegerClass::parse(&value.class)
                    .ok_or_else(|| invalid_preview("invalid integer class"))?;
                let mut any_imaginary = false;
                for component in integer {
                    validate_integer_component(&component.real, class)?;
                    validate_integer_component(&component.imaginary, class)?;
                    any_imaginary |= component.imaginary != "0";
                    if !value.complex && component.imaginary != "0" {
                        return Err(invalid_preview(
                            "real integer storage requires every imaginary component to be 0",
                        ));
                    }
                }
                if value.complex && !any_imaginary {
                    return Err(invalid_preview(
                        "complex integer storage requires a nonzero imaginary component",
                    ));
                }
            }
            ExactPayload::Logical { logical } => {
                validate_payload_count(logical.len(), value.numel, "logical")?;
            }
            ExactPayload::Char { code_units } => {
                validate_payload_count(code_units.len(), value.numel, "code_units")?;
                usage.charge_code_units(
                    usize_to_safe(code_units.len(), "code_units.length")?,
                    limits,
                )?;
            }
            ExactPayload::String {
                string_code_units,
                missing,
            } => {
                validate_payload_count(string_code_units.len(), value.numel, "string_code_units")?;
                validate_payload_count(missing.len(), value.numel, "missing")?;
                for (code_units, is_missing) in string_code_units.iter().zip(missing) {
                    if *is_missing && !code_units.is_empty() {
                        return Err(invalid_preview(
                            "missing string elements require empty code-unit arrays",
                        ));
                    }
                    let count = usize_to_safe(code_units.len(), "string element code units")?;
                    if count > limits.max_string_element_code_units {
                        return Err(preview_limit(
                            "string element exceeds maxStringElementCodeUnits",
                        ));
                    }
                    usage.charge_code_units(count, limits)?;
                }
            }
            ExactPayload::Cell { items } => {
                validate_payload_count(items.len(), value.numel, "items")?;
                let child_depth = depth
                    .checked_add(1)
                    .ok_or_else(|| preview_limit("exact depth overflows"))?;
                for item in items.iter().rev() {
                    work.push((item, child_depth));
                }
            }
            ExactPayload::Struct { fields, records } => {
                validate_payload_count(records.len(), value.numel, "records")?;
                validate_fields(fields, usage, limits)?;
                let child_depth = depth
                    .checked_add(1)
                    .ok_or_else(|| preview_limit("exact depth overflows"))?;
                for record in records.iter().rev() {
                    validate_record(record, fields)?;
                    for field in fields.iter().rev() {
                        let child = record
                            .get(field)
                            .ok_or_else(|| invalid_preview("struct record field set mismatch"))?;
                        work.push((child, child_depth));
                    }
                }
            }
            ExactPayload::Table {
                variable_names,
                variables,
            } => {
                let height = value.size[0];
                let width = value.size[1];
                validate_table_schema(variable_names, width, usage, limits)?;
                validate_payload_count(variables.len(), width, "variables")?;
                let child_depth = depth
                    .checked_add(1)
                    .ok_or_else(|| preview_limit("exact depth overflows"))?;
                for variable in variables.iter().rev() {
                    if variable.size.first().copied() != Some(height) {
                        return Err(invalid_preview(
                            "table variable first dimension must equal table height",
                        ));
                    }
                    work.push((variable, child_depth));
                }
            }
        }
    }
    Ok(())
}

fn validate_exact_kind_metadata(value: &ExactValue) -> Result<(), ValidationError> {
    require_non_empty("class", &value.class)?;
    match &value.payload {
        ExactPayload::Numeric { .. } => {
            if !matches!(value.class.as_str(), "double" | "single") {
                return Err(invalid_preview(
                    "numeric exact values require class double or single",
                ));
            }
        }
        ExactPayload::Integer { .. } => {
            if IntegerClass::parse(&value.class).is_none() {
                return Err(invalid_preview(
                    "integer exact value has an unsupported fixed-width class",
                ));
            }
        }
        ExactPayload::Logical { .. } => require_exact_real_class(value, "logical")?,
        ExactPayload::Char { .. } => require_exact_real_class(value, "char")?,
        ExactPayload::String { .. } => require_exact_real_class(value, "string")?,
        ExactPayload::Cell { .. } => require_exact_real_class(value, "cell")?,
        ExactPayload::Struct { .. } => require_exact_real_class(value, "struct")?,
        ExactPayload::Table { .. } => require_exact_real_class(value, "table")?,
    }
    Ok(())
}

fn require_exact_real_class(
    value: &ExactValue,
    class: &'static str,
) -> Result<(), ValidationError> {
    if value.class != class || value.complex {
        Err(invalid_preview(format!(
            "exact {class} value requires class {class} and complex false"
        )))
    } else {
        Ok(())
    }
}

fn validate_fields(
    fields: &[String],
    usage: &mut UsageCounter,
    limits: &AggregateLimits,
) -> Result<(), ValidationError> {
    let mut unique = BTreeSet::new();
    for field in fields {
        if !valid_ascii_field_name(field) {
            return Err(ValidationError::new(
                "workspace.unsupportedValue",
                "struct field name is outside the accepted ASCII identifier subset",
            ));
        }
        if !unique.insert(field.as_str()) {
            return Err(invalid_preview("struct field names must be unique"));
        }
        usage.charge_code_units(usize_to_safe(field.len(), "field name length")?, limits)?;
    }
    Ok(())
}

fn validate_table_schema(
    variable_names: &[String],
    width: u64,
    usage: &mut UsageCounter,
    limits: &AggregateLimits,
) -> Result<(), ValidationError> {
    validate_payload_count(variable_names.len(), width, "variableNames")?;
    let mut unique = BTreeSet::new();
    for name in variable_names {
        require_non_empty("variableNames entry", name)?;
        if !unique.insert(name.as_str()) {
            return Err(invalid_preview("table variable names must be unique"));
        }
        let code_units = usize_to_safe(name.encode_utf16().count(), "variable name code units")?;
        usage.charge_code_units(code_units, limits)?;
    }
    Ok(())
}

fn validate_record(record: &StructRecord, fields: &[String]) -> Result<(), ValidationError> {
    if record.entries.len() != fields.len() {
        return Err(invalid_preview("struct record field set mismatch"));
    }
    let mut keys = BTreeSet::new();
    for (name, _) in &record.entries {
        if !keys.insert(name.as_str()) || !fields.iter().any(|field| field == name) {
            return Err(invalid_preview("struct record field set mismatch"));
        }
    }
    Ok(())
}

fn valid_ascii_field_name(field: &str) -> bool {
    let mut bytes = field.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn validate_number_string(value: &str) -> Result<(), ValidationError> {
    if matches!(value, "NaN" | "+Inf" | "-Inf") || valid_finite_number_string(value) {
        Ok(())
    } else {
        Err(invalid_preview(
            "numeric payload has a non-canonical number string",
        ))
    }
}

fn valid_finite_number_string(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = usize::from(bytes.first() == Some(&b'-'));
    if index >= bytes.len() {
        return false;
    }
    match bytes[index] {
        b'0' => {
            index += 1;
            if bytes.get(index).is_some_and(u8::is_ascii_digit) {
                return false;
            }
        }
        b'1'..=b'9' => {
            index += 1;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

fn number_string_is_zero(value: &str) -> bool {
    if !valid_finite_number_string(value) {
        return false;
    }
    value
        .bytes()
        .take_while(|byte| !matches!(byte, b'e' | b'E'))
        .all(|byte| matches!(byte, b'-' | b'0' | b'.'))
}

#[derive(Clone, Copy)]
struct IntegerClass {
    signed: bool,
    bits: u32,
}

impl IntegerClass {
    fn parse(class: &str) -> Option<Self> {
        match class {
            "int8" => Some(Self::new(true, 8)),
            "uint8" => Some(Self::new(false, 8)),
            "int16" => Some(Self::new(true, 16)),
            "uint16" => Some(Self::new(false, 16)),
            "int32" => Some(Self::new(true, 32)),
            "uint32" => Some(Self::new(false, 32)),
            "int64" => Some(Self::new(true, 64)),
            "uint64" => Some(Self::new(false, 64)),
            _ => None,
        }
    }

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

fn validate_integer_component(value: &str, class: IntegerClass) -> Result<(), ValidationError> {
    let bytes = value.as_bytes();
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
        return Err(invalid_preview(
            "integer component is non-canonical or has invalid signedness",
        ));
    }
    let magnitude = std::str::from_utf8(digits)
        .ok()
        .and_then(|digits| digits.parse::<u128>().ok())
        .ok_or_else(|| invalid_preview("integer component is outside the transport range"))?;
    let maximum = if negative {
        class.negative_magnitude_max()
    } else {
        class.positive_max()
    };
    if magnitude > maximum {
        return Err(invalid_preview(
            "integer component is outside its fixed-width class range",
        ));
    }
    Ok(())
}

fn validate_payload_count(
    actual: usize,
    expected: u64,
    field: &'static str,
) -> Result<(), ValidationError> {
    if usize_to_safe(actual, field)? == expected {
        Ok(())
    } else {
        Err(invalid_preview(format!(
            "{field} payload count does not equal numel"
        )))
    }
}

fn validate_usage_safe(usage: &PreviewUsage) -> Result<(), ValidationError> {
    validate_safe_json_integer("usage.nodes", usage.nodes)?;
    validate_safe_json_integer("usage.elements", usage.elements)?;
    validate_safe_json_integer("usage.codeUnits", usage.code_units)?;
    validate_safe_json_integer("usage.depth", usage.depth)
}

fn validate_shape(values: &[u64], field: &'static str) -> Result<u64, ValidationError> {
    if values.len() < 2 {
        return Err(invalid_preview(format!(
            "{field} must contain at least two dimensions"
        )));
    }
    for value in values {
        validate_safe_json_integer(field, *value)?;
    }
    checked_product_safe(values, field)
}

fn checked_product_safe(values: &[u64], field: &'static str) -> Result<u64, ValidationError> {
    let product = values.iter().try_fold(1_u64, |product, value| {
        product
            .checked_mul(*value)
            .ok_or_else(|| preview_limit(format!("checked product of {field} overflows")))
    })?;
    validate_safe_json_integer(field, product)?;
    Ok(product)
}

fn checked_add_safe(left: u64, right: u64, field: &'static str) -> Result<u64, ValidationError> {
    let value = left
        .checked_add(right)
        .ok_or_else(|| preview_limit(format!("checked addition for {field} overflows")))?;
    validate_safe_json_integer(field, value)?;
    Ok(value)
}

fn usize_to_safe(value: usize, field: &'static str) -> Result<u64, ValidationError> {
    let value = u64::try_from(value)
        .map_err(|_| preview_limit(format!("{field} does not fit a wire integer")))?;
    validate_safe_json_integer(field, value)?;
    Ok(value)
}

fn validate_bounded_positive(
    field: &'static str,
    value: u64,
    maximum: u64,
) -> Result<(), ValidationError> {
    validate_safe_json_integer(field, value)?;
    if value == 0 || value > maximum {
        return Err(invalid_capabilities(format!(
            "{field} must be between 1 and {maximum}"
        )));
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

fn invalid_preview(message: impl Into<String>) -> ValidationError {
    ValidationError::new("engine.invalidPreview", message)
}

fn preview_limit(message: impl Into<String>) -> ValidationError {
    ValidationError::new("workspace.previewLimit", message)
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
    validate_shape(&variable.dimensions, "variable.dimensions")?;
    if let Some(bytes) = variable.bytes {
        validate_safe_json_integer("variable.bytes", bytes)?;
    }
    Ok(())
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

const fn default_preview_elements() -> u64 {
    MAX_PREVIEW_ELEMENTS
}

#[cfg(test)]
mod table_tests {
    use super::*;

    fn numeric(size: Vec<u64>) -> ExactValue {
        let numel = size.iter().product::<u64>();
        ExactValue {
            class: "double".to_owned(),
            ndims: u64::try_from(size.len()).expect("test rank"),
            size,
            numel,
            complex: false,
            payload: ExactPayload::Numeric {
                real: vec!["0".to_owned(); usize::try_from(numel).expect("test numel")],
                imag: vec!["0".to_owned(); usize::try_from(numel).expect("test numel")],
            },
        }
    }

    fn logical(size: Vec<u64>) -> ExactValue {
        let numel = size.iter().product::<u64>();
        ExactValue {
            class: "logical".to_owned(),
            ndims: u64::try_from(size.len()).expect("test rank"),
            size,
            numel,
            complex: false,
            payload: ExactPayload::Logical {
                logical: vec![false; usize::try_from(numel).expect("test numel")],
            },
        }
    }

    fn char_scalar(code_unit: u16) -> ExactValue {
        ExactValue {
            class: "char".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            payload: ExactPayload::Char {
                code_units: vec![code_unit],
            },
        }
    }

    fn table(size: Vec<u64>, variable_names: Vec<&str>, variables: Vec<ExactValue>) -> ExactValue {
        let numel = size.iter().product::<u64>();
        ExactValue {
            class: "table".to_owned(),
            ndims: u64::try_from(size.len()).expect("test rank"),
            size,
            numel,
            complex: false,
            payload: ExactPayload::Table {
                variable_names: variable_names.into_iter().map(str::to_owned).collect(),
                variables,
            },
        }
    }

    fn cell_preview(item: ExactValue, usage: PreviewUsage) -> AggregatePreview {
        AggregatePreview::Cell {
            dimensions: vec![1, 1],
            selected_range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            items: vec![item],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage,
        }
    }

    fn wide_variable_table() -> ExactValue {
        table(
            vec![2, 2],
            vec!["温度", "Flag😀"],
            vec![numeric(vec![2, 2, 2]), logical(vec![2, 1])],
        )
    }

    fn wide_table_usage() -> PreviewUsage {
        PreviewUsage {
            nodes: 4,
            elements: 15,
            code_units: 8,
            depth: 2,
        }
    }

    fn top_level_table_preview() -> AggregatePreview {
        AggregatePreview::Table {
            dimensions: vec![4, 3],
            selected_range: MatrixRange {
                start: vec![2, 1],
                size: vec![2, 3],
            },
            variable_names: vec!["温度".to_owned(), "Flag😀".to_owned()],
            variables: vec![numeric(vec![2, 2, 2]), logical(vec![2, 1])],
            truncation: PreviewTruncation {
                truncated: true,
                omitted_elements: 1,
            },
            usage: PreviewUsage {
                nodes: 3,
                elements: 12,
                code_units: 8,
                depth: 1,
            },
        }
    }

    #[test]
    fn table_exact_value_serde_roundtrip_uses_camel_case_wire_fields() {
        let table = wide_variable_table();
        let encoded = serde_json::to_value(&table).expect("serialize table exact value");
        assert_eq!(encoded["kind"], "table");
        assert_eq!(
            encoded["variableNames"],
            serde_json::json!(["温度", "Flag😀"])
        );
        assert!(encoded.get("variable_names").is_none());
        assert_eq!(
            encoded["variables"][0]["size"],
            serde_json::json!([2, 2, 2])
        );

        let decoded: ExactValue =
            serde_json::from_value(encoded).expect("deserialize table exact value");
        assert_eq!(decoded, table);
        cell_preview(decoded, wide_table_usage())
            .validate(&AggregateLimits::default())
            .expect("validate round-tripped table");
    }

    #[test]
    fn table_exact_value_requires_table_class_and_real_storage() {
        let mut wrong_class = wide_variable_table();
        wrong_class.class = "struct".to_owned();
        assert_eq!(
            cell_preview(wrong_class, wide_table_usage())
                .validate(&AggregateLimits::default())
                .expect_err("wrong table class")
                .category(),
            "engine.invalidPreview"
        );

        let mut complex = wide_variable_table();
        complex.complex = true;
        assert_eq!(
            cell_preview(complex, wide_table_usage())
                .validate(&AggregateLimits::default())
                .expect_err("complex table")
                .category(),
            "engine.invalidPreview"
        );
    }

    #[test]
    fn table_exact_value_rejects_empty_or_duplicate_variable_names() {
        for names in [vec!["A", "A"], vec!["", "B"]] {
            let malformed = table(
                vec![2, 2],
                names,
                vec![numeric(vec![2, 1]), logical(vec![2, 1])],
            );
            assert!(
                cell_preview(malformed, wide_table_usage())
                    .validate(&AggregateLimits::default())
                    .is_err()
            );
        }
    }

    #[test]
    fn table_exact_value_requires_schema_and_payload_width_to_match_size() {
        let missing_name = table(
            vec![2, 2],
            vec!["A"],
            vec![numeric(vec![2, 1]), logical(vec![2, 1])],
        );
        assert!(
            cell_preview(missing_name, wide_table_usage())
                .validate(&AggregateLimits::default())
                .is_err()
        );

        let missing_variable = table(vec![2, 2], vec!["A", "B"], vec![numeric(vec![2, 1])]);
        assert!(
            cell_preview(missing_variable, wide_table_usage())
                .validate(&AggregateLimits::default())
                .is_err()
        );
    }

    #[test]
    fn table_exact_value_requires_every_variable_to_match_table_height() {
        let malformed = table(
            vec![2, 2],
            vec!["A", "B"],
            vec![numeric(vec![3, 1]), logical(vec![2, 1])],
        );
        assert_eq!(
            cell_preview(malformed, wide_table_usage())
                .validate(&AggregateLimits::default())
                .expect_err("mismatched table height")
                .category(),
            "engine.invalidPreview"
        );
    }

    #[test]
    fn nested_table_variables_contribute_usage_and_depth() {
        let variable = ExactValue {
            class: "cell".to_owned(),
            size: vec![2, 1],
            ndims: 2,
            numel: 2,
            complex: false,
            payload: ExactPayload::Cell {
                items: vec![char_scalar(b'x'.into()), char_scalar(b'y'.into())],
            },
        };
        let preview = cell_preview(
            table(vec![2, 1], vec!["A"], vec![variable]),
            PreviewUsage {
                nodes: 5,
                elements: 7,
                code_units: 3,
                depth: 3,
            },
        );
        preview
            .validate(&AggregateLimits::default())
            .expect("nested table usage");

        let shallow_limits = AggregateLimits {
            max_aggregate_depth: 2,
            ..AggregateLimits::default()
        };
        assert_eq!(
            preview
                .validate(&shallow_limits)
                .expect_err("nested table depth limit")
                .category(),
            "workspace.previewDepth"
        );
    }

    #[test]
    fn top_level_table_preview_roundtrips_with_canonical_wire_fields() {
        let preview = top_level_table_preview();
        preview
            .validate(&AggregateLimits::default())
            .expect("valid table preview");

        let encoded = serde_json::to_value(&preview).expect("serialize table preview");
        assert_eq!(encoded["class"], "table");
        assert_eq!(encoded["kind"], "table");
        assert_eq!(encoded["complex"], false);
        assert_eq!(encoded["dimensions"], serde_json::json!([4, 3]));
        assert_eq!(
            encoded["selectedRange"],
            serde_json::json!({"start":[2,1],"size":[2,3]})
        );
        assert_eq!(
            encoded["variableNames"],
            serde_json::json!(["温度", "Flag😀"])
        );
        assert_eq!(
            encoded["variables"][0]["size"],
            serde_json::json!([2, 2, 2])
        );
        assert!(encoded.get("variable_names").is_none());

        let decoded: AggregatePreview =
            serde_json::from_value(encoded).expect("deserialize table preview");
        assert_eq!(decoded, preview);
        assert_eq!(
            InspectPreview::Aggregate(decoded).selected_range(),
            &MatrixRange {
                start: vec![2, 1],
                size: vec![2, 3],
            }
        );
    }

    #[test]
    fn table_truncation_counts_complete_selected_variables() {
        let preview = top_level_table_preview();
        preview
            .validate(&AggregateLimits::default())
            .expect("two returned variables plus one omitted variable");

        let mut malformed = preview.clone();
        let AggregatePreview::Table { truncation, .. } = &mut malformed else {
            unreachable!()
        };
        truncation.omitted_elements = 4;
        assert_eq!(
            malformed
                .validate(&AggregateLimits::default())
                .expect_err("omitted count uses variables, not table cells")
                .category(),
            "preview_truncation"
        );

        let mut malformed = preview;
        let AggregatePreview::Table { truncation, .. } = &mut malformed else {
            unreachable!()
        };
        truncation.truncated = false;
        assert_eq!(
            malformed
                .validate(&AggregateLimits::default())
                .expect_err("truncation flag must agree with omitted variables")
                .category(),
            "preview_truncation"
        );
    }

    #[test]
    fn table_preview_validates_schema_counts_rows_and_two_dimensional_shape() {
        let mut duplicate = top_level_table_preview();
        let AggregatePreview::Table { variable_names, .. } = &mut duplicate else {
            unreachable!()
        };
        variable_names[1] = variable_names[0].clone();
        assert_eq!(
            duplicate
                .validate(&AggregateLimits::default())
                .expect_err("duplicate table variable name")
                .category(),
            "engine.invalidPreview"
        );

        let mut mismatched_count = top_level_table_preview();
        let AggregatePreview::Table { variable_names, .. } = &mut mismatched_count else {
            unreachable!()
        };
        variable_names.pop();
        assert_eq!(
            mismatched_count
                .validate(&AggregateLimits::default())
                .expect_err("name and variable counts differ")
                .category(),
            "engine.invalidPreview"
        );

        let mut mismatched_rows = top_level_table_preview();
        let AggregatePreview::Table { variables, .. } = &mut mismatched_rows else {
            unreachable!()
        };
        variables[0] = numeric(vec![3, 1]);
        assert_eq!(
            mismatched_rows
                .validate(&AggregateLimits::default())
                .expect_err("variable row count differs from selected rows")
                .category(),
            "engine.invalidPreview"
        );

        let mut rank_three = top_level_table_preview();
        let AggregatePreview::Table { dimensions, .. } = &mut rank_three else {
            unreachable!()
        };
        dimensions.push(1);
        assert_eq!(
            rank_three
                .validate(&AggregateLimits::default())
                .expect_err("table root must be two-dimensional")
                .category(),
            "engine.invalidPreview"
        );
    }

    #[test]
    fn zero_row_table_still_checks_the_selected_variable_range() {
        let preview = AggregatePreview::Table {
            dimensions: vec![0, 3],
            selected_range: MatrixRange {
                start: vec![1, 2],
                size: vec![0, 2],
            },
            variable_names: vec!["A".to_owned(), "B".to_owned()],
            variables: vec![numeric(vec![0, 2]), logical(vec![0, 1])],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: PreviewUsage {
                nodes: 3,
                elements: 2,
                code_units: 2,
                depth: 1,
            },
        };
        preview
            .validate(&AggregateLimits::default())
            .expect("zero rows do not erase the variable selection");

        let mut out_of_bounds = preview;
        let AggregatePreview::Table { selected_range, .. } = &mut out_of_bounds else {
            unreachable!()
        };
        selected_range.start[1] = 3;
        assert_eq!(
            out_of_bounds
                .validate(&AggregateLimits::default())
                .expect_err("variable extent exceeds full table width")
                .category(),
            "engine.invalidPreview"
        );
    }

    #[test]
    fn table_preview_recomputes_usage_and_enforces_negotiated_limits() {
        let mut wrong_usage = top_level_table_preview();
        let AggregatePreview::Table { usage, .. } = &mut wrong_usage else {
            unreachable!()
        };
        usage.elements -= 1;
        assert_eq!(
            wrong_usage
                .validate(&AggregateLimits::default())
                .expect_err("usage is recomputed")
                .category(),
            "engine.invalidPreview"
        );

        let node_limited = AggregateLimits {
            max_aggregate_nodes: 2,
            ..AggregateLimits::default()
        };
        assert_eq!(
            top_level_table_preview()
                .validate(&node_limited)
                .expect_err("two exact variables plus aggregate root exceed node limit")
                .category(),
            "workspace.previewLimit"
        );

        let code_unit_limited = AggregateLimits::new(
            MAX_PREVIEW_ELEMENTS,
            1,
            7,
            MAX_AGGREGATE_NODES,
            MAX_AGGREGATE_ELEMENTS,
            MAX_AGGREGATE_DEPTH,
        )
        .expect("valid narrow code-unit limit");
        assert_eq!(
            top_level_table_preview()
                .validate(&code_unit_limited)
                .expect_err("returned variable names contribute UTF-16 usage")
                .category(),
            "workspace.previewLimit"
        );
    }

    #[test]
    fn table_preview_request_limit_counts_returned_variables() {
        let preview = top_level_table_preview();
        preview
            .validate_for_request(&AggregateLimits::default(), 2)
            .expect("two complete returned variables fit maxElements two");
        assert_eq!(
            preview
                .validate_for_request(&AggregateLimits::default(), 1)
                .expect_err("request permits only one complete variable")
                .category(),
            "workspace.previewLimit"
        );
        assert_eq!(
            preview
                .validate_for_request(&AggregateLimits::default(), 0)
                .expect_err("zero request limit is invalid")
                .category(),
            "preview_bound"
        );
    }

    #[test]
    fn table_preview_deserialization_rejects_unknown_or_noncanonical_fields() {
        let mut unknown = serde_json::to_value(top_level_table_preview()).expect("table JSON");
        unknown
            .as_object_mut()
            .expect("table object")
            .insert("futureField".to_owned(), serde_json::json!(true));
        assert!(serde_json::from_value::<AggregatePreview>(unknown).is_err());

        let mut wrong_class = serde_json::to_value(top_level_table_preview()).expect("table JSON");
        wrong_class["class"] = serde_json::json!("struct");
        assert!(serde_json::from_value::<AggregatePreview>(wrong_class).is_err());

        let mut complex = serde_json::to_value(top_level_table_preview()).expect("table JSON");
        complex["complex"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AggregatePreview>(complex).is_err());
    }
}

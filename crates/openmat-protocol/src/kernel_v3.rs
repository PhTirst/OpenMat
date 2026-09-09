//! Typed `openmat-kernel-v3` messages for optimistic scalar workspace writes.
//!
//! Bootstrap continues to use the v0 envelope and v2 capability limits. V3
//! wraps workspace reads with a monotonic revision and adds one atomic,
//! revision-checked numeric element mutation. V0 through v2 wire shapes remain
//! unchanged.

use serde::{Deserialize, Serialize};

use crate::kernel_v1::MAX_SAFE_JSON_INTEGER;
use crate::kernel_v2::{self, AggregateLimits, InspectPreview};
use crate::{
    Event, ExecuteRequest, ExecuteResult, InterruptRequest, InterruptResult, ListWorkspaceRequest,
    MessageKind, ProtocolError, ShutdownRequest, ShutdownResult, ValidationError, VariableSummary,
};

pub use crate::kernel_v2::{
    BootstrapRequest, BootstrapRequestEnvelope, BootstrapResponseEnvelope, BootstrapResponseResult,
    Capabilities, CodecError, InitializeRequest, InitializeResult,
};

/// The negotiated v3 protocol identifier.
pub const PROTOCOL_V3: &str = "openmat-kernel-v3";

/// Production v3 client offer in preference order.
pub const SUPPORTED_PROTOCOLS: [&str; 4] = [
    PROTOCOL_V3,
    kernel_v2::PROTOCOL_V2,
    crate::kernel_v1::PROTOCOL_V1,
    crate::PROTOCOL_V0,
];

/// Returns the production v3 protocol offer.
#[must_use]
pub fn production_protocol_offer() -> Vec<String> {
    SUPPORTED_PROTOCOLS
        .iter()
        .map(|protocol| (*protocol).to_owned())
        .collect()
}

/// Exact, protocol-safe real and imaginary scalar text.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumericScalar {
    /// Real component using the v2 number-string vocabulary.
    pub real: String,
    /// Imaginary component using the v2 number-string vocabulary.
    pub imaginary: String,
}

impl NumericScalar {
    /// Parses both components as binary64 values after lexical validation.
    ///
    /// # Errors
    ///
    /// Returns a validation error for whitespace, non-number text, or another
    /// token outside the canonical v2 number-string vocabulary.
    pub fn components(&self) -> Result<(f64, f64), ValidationError> {
        Ok((
            parse_number_string("real", &self.real)?,
            parse_number_string("imaginary", &self.imaginary)?,
        ))
    }
}

/// Atomic update of one existing numeric workspace element.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetVariableElementRequest {
    /// Existing workspace variable name.
    pub name: String,
    /// One-based subscript per existing dimension.
    pub indices: Vec<u64>,
    /// Replacement scalar.
    pub value: NumericScalar,
    /// Workspace revision returned by the inspect that supplied the cell.
    pub expected_revision: u64,
}

impl SetVariableElementRequest {
    /// Validates the bounded request independently of runtime shape and class.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the name, indices, revision, or numeric
    /// component text is not a canonical v3 value.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !is_identifier(&self.name) {
            return Err(ValidationError::new(
                "workspace_name",
                "name must use the supported ASCII identifier repertoire",
            ));
        }
        if self.indices.len() < 2
            || self
                .indices
                .iter()
                .any(|index| *index == 0 || *index > MAX_SAFE_JSON_INTEGER)
        {
            return Err(ValidationError::new(
                "workspace_indices",
                "indices must contain at least two one-based safe integers",
            ));
        }
        validate_revision(self.expected_revision)?;
        self.value.components().map(|_| ())
    }
}

/// Post-bootstrap v3 requests.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "params", rename_all = "camelCase")]
pub enum Request {
    /// Execute UTF-8 source.
    Execute(ExecuteRequest),
    /// Cooperatively interrupt active execution.
    Interrupt(InterruptRequest),
    /// Inspect a bounded workspace range.
    Inspect(kernel_v2::InspectRequest),
    /// List workspace summaries.
    ListWorkspace(ListWorkspaceRequest),
    /// Atomically replace one numeric element.
    SetVariableElement(SetVariableElementRequest),
    /// Terminate the kernel orderly.
    Shutdown(ShutdownRequest),
}

impl Request {
    fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        match self {
            Self::Execute(request) if request.source_name.is_empty() => Err(ValidationError::new(
                "source_name",
                "sourceName must not be empty",
            )),
            Self::Inspect(request) => request.validate(&limits.preview_limits()),
            Self::SetVariableElement(request) => request.validate(),
            Self::Execute(_) | Self::Interrupt(_) | Self::ListWorkspace(_) | Self::Shutdown(_) => {
                Ok(())
            }
        }
    }

    /// Returns true for requests allowed to bypass the sequential queue.
    #[must_use]
    pub const fn is_control(&self) -> bool {
        matches!(self, Self::Interrupt(_) | Self::Shutdown(_))
    }
}

/// One v3 request envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestEnvelope {
    /// Must be `openmat-kernel-v3`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque request identifier.
    pub message_id: String,
    /// Must be `request`.
    pub kind: MessageKind,
    /// Typed request.
    pub request: Request,
}

impl RequestEnvelope {
    /// Constructs a v3 request.
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        request: Request,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V3.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Request,
            request,
        }
    }

    /// Validates envelope and request invariants.
    ///
    /// # Errors
    ///
    /// Returns a validation error for a malformed envelope or request payload.
    pub fn validate(&self, limits: &AggregateLimits) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Request,
        )?;
        self.request.validate(limits)
    }
}

/// A revision paired with a bounded v2 preview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionedInspectPreview {
    /// Monotonic workspace revision observed by this request.
    pub revision: u64,
    /// Existing v2 inspect payload.
    pub preview: InspectPreview,
}

/// A revision paired with deterministic workspace summaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionedWorkspaceSummary {
    /// Monotonic workspace revision observed by this request.
    pub revision: u64,
    /// Variables in deterministic name order.
    pub variables: Vec<VariableSummary>,
}

/// Successful element mutation result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetVariableElementResult {
    /// New workspace revision after commit.
    pub revision: u64,
    /// Updated variable metadata.
    pub variable: VariableSummary,
}

/// Successful v3 response payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ResponseResult {
    /// Execution completion.
    Execute(ExecuteResult),
    /// Interrupt acknowledgement.
    Interrupt(InterruptResult),
    /// Versioned inspect result.
    Inspect(VersionedInspectPreview),
    /// Versioned workspace list.
    ListWorkspace(VersionedWorkspaceSummary),
    /// Committed element write.
    SetVariableElement(SetVariableElementResult),
    /// Shutdown acknowledgement.
    Shutdown(ShutdownResult),
}

/// One v3 response envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseEnvelope {
    /// Must be `openmat-kernel-v3`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque response identifier.
    pub message_id: String,
    /// Must be `response`.
    pub kind: MessageKind,
    /// Correlated request identifier.
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
    /// Constructs a successful response.
    #[must_use]
    pub fn success(
        request: &RequestEnvelope,
        message_id: impl Into<String>,
        result: ResponseResult,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V3.to_owned(),
            session_id: request.session_id.clone(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: request.message_id.clone(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Constructs a failed response.
    #[must_use]
    pub fn failure(
        request: &RequestEnvelope,
        message_id: impl Into<String>,
        error: ProtocolError,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V3.to_owned(),
            session_id: request.session_id.clone(),
            message_id: message_id.into(),
            kind: MessageKind::Response,
            reply_to: request.message_id.clone(),
            ok: false,
            result: None,
            error: Some(error),
        }
    }

    /// Validates this response against its originating request.
    ///
    /// # Errors
    ///
    /// Returns a validation error when correlation, response shape, revision,
    /// or bounded result data is invalid for the request.
    pub fn validate_for_request(
        &self,
        request: &RequestEnvelope,
        limits: &AggregateLimits,
    ) -> Result<(), ValidationError> {
        request.validate(limits)?;
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Response,
        )?;
        if self.reply_to.is_empty()
            || self.session_id != request.session_id
            || self.reply_to != request.message_id
        {
            return Err(ValidationError::new(
                "response_correlation",
                "response sessionId or replyTo does not match request",
            ));
        }
        match (self.ok, &self.result, &self.error) {
            (false, None, Some(error))
                if !error.category.is_empty() && !error.message.is_empty() =>
            {
                Ok(())
            }
            (true, Some(result), None) => validate_result_for_request(result, request, limits),
            _ => Err(ValidationError::new(
                "response_shape",
                "successful responses require only result; failed responses require only error",
            )),
        }
    }
}

/// One v3 event envelope. Event payloads are unchanged from v2.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventEnvelope {
    /// Must be `openmat-kernel-v3`.
    pub protocol: String,
    /// Opaque session identifier.
    pub session_id: String,
    /// Opaque event identifier.
    pub message_id: String,
    /// Must be `event`.
    pub kind: MessageKind,
    /// Existing event payload.
    pub event: Event,
}

impl EventEnvelope {
    /// Constructs a v3 event.
    #[must_use]
    pub fn new(session_id: impl Into<String>, message_id: impl Into<String>, event: Event) -> Self {
        Self {
            protocol: PROTOCOL_V3.to_owned(),
            session_id: session_id.into(),
            message_id: message_id.into(),
            kind: MessageKind::Event,
            event,
        }
    }

    /// Validates the v3 header and unchanged v2 event payload.
    ///
    /// # Errors
    ///
    /// Returns a validation error for an invalid envelope or event payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_header(
            &self.protocol,
            &self.session_id,
            &self.message_id,
            self.kind,
            MessageKind::Event,
        )?;
        kernel_v2::EventEnvelope::new(
            self.session_id.clone(),
            self.message_id.clone(),
            self.event.clone(),
        )
        .validate()
    }
}

/// One server-to-client v3 message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ServerMessage {
    /// Correlated response.
    Response(ResponseEnvelope),
    /// Unsolicited event.
    Event(EventEnvelope),
}

/// Decodes and validates a v3 request.
///
/// # Errors
///
/// Returns a codec error for an oversized, malformed, or invalid frame.
pub fn decode_request(
    input: &str,
    limits: &AggregateLimits,
) -> Result<RequestEnvelope, CodecError> {
    check_frame(input)?;
    let request: RequestEnvelope = serde_json::from_str(input)?;
    request.validate(limits)?;
    Ok(request)
}

/// Encodes and validates a v3 request.
///
/// # Errors
///
/// Returns a codec error when validation, serialization, or the frame budget
/// fails.
pub fn encode_request(
    request: &RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<String, CodecError> {
    request.validate(limits)?;
    encode_checked(request)
}

/// Decodes a response and checks it against its request.
///
/// # Errors
///
/// Returns a codec error for an oversized or malformed frame, or when the
/// response is not valid for the supplied request.
pub fn decode_response_for_request(
    input: &str,
    request: &RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<ResponseEnvelope, CodecError> {
    check_frame(input)?;
    let response: ResponseEnvelope = serde_json::from_str(input)?;
    response.validate_for_request(request, limits)?;
    Ok(response)
}

/// Encodes a response after request-aware validation.
///
/// # Errors
///
/// Returns a codec error when request-aware validation, serialization, or the
/// frame budget fails.
pub fn encode_response_for_request(
    response: &ResponseEnvelope,
    request: &RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<String, CodecError> {
    response.validate_for_request(request, limits)?;
    encode_checked(response)
}

/// Decodes and validates an event.
///
/// # Errors
///
/// Returns a codec error for an oversized, malformed, or invalid event frame.
pub fn decode_event(input: &str) -> Result<EventEnvelope, CodecError> {
    check_frame(input)?;
    let event: EventEnvelope = serde_json::from_str(input)?;
    event.validate()?;
    Ok(event)
}

/// Encodes and validates an event.
///
/// # Errors
///
/// Returns a codec error when event validation, serialization, or the frame
/// budget fails.
pub fn encode_event(event: &EventEnvelope) -> Result<String, CodecError> {
    event.validate()?;
    encode_checked(event)
}

fn validate_result_for_request(
    result: &ResponseResult,
    request: &RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<(), ValidationError> {
    match (&request.request, result) {
        (Request::Execute(_), ResponseResult::Execute(_))
        | (Request::Interrupt(_), ResponseResult::Interrupt(_))
        | (Request::Shutdown(_), ResponseResult::Shutdown(_)) => Ok(()),
        (Request::Inspect(inspect), ResponseResult::Inspect(result)) => {
            validate_revision(result.revision)?;
            if result.preview.selected_range() != &inspect.range {
                return Err(ValidationError::new(
                    "response_shape",
                    "inspect selectedRange does not match request",
                ));
            }
            result
                .preview
                .validate_for_request(limits, inspect.max_elements)
        }
        (Request::ListWorkspace(_), ResponseResult::ListWorkspace(result)) => {
            validate_revision(result.revision)?;
            validate_workspace_variables(&result.variables)
        }
        (Request::SetVariableElement(_), ResponseResult::SetVariableElement(result)) => {
            validate_revision(result.revision)?;
            validate_workspace_variables(std::slice::from_ref(&result.variable))
        }
        _ => Err(ValidationError::new(
            "response_shape",
            "successful response result type does not match request type",
        )),
    }
}

fn validate_workspace_variables(variables: &[VariableSummary]) -> Result<(), ValidationError> {
    let request = kernel_v2::RequestEnvelope::new(
        "validation",
        "validation-request",
        kernel_v2::Request::ListWorkspace(ListWorkspaceRequest {}),
    );
    let response = kernel_v2::ResponseEnvelope::success(
        &request,
        "validation-response",
        kernel_v2::ResponseResult::ListWorkspace(crate::WorkspaceSummary {
            variables: variables.to_vec(),
        }),
    );
    response.validate_for_request(&request, &AggregateLimits::default())
}

fn validate_header(
    protocol: &str,
    session_id: &str,
    message_id: &str,
    actual_kind: MessageKind,
    expected_kind: MessageKind,
) -> Result<(), ValidationError> {
    if protocol != PROTOCOL_V3 {
        return Err(ValidationError::new(
            "protocol",
            "envelope protocol must be openmat-kernel-v3",
        ));
    }
    if session_id.is_empty() || message_id.is_empty() {
        return Err(ValidationError::new(
            "identifier",
            "sessionId and messageId must not be empty",
        ));
    }
    if actual_kind != expected_kind {
        return Err(ValidationError::new(
            "message_kind",
            "envelope kind does not match the message shape",
        ));
    }
    Ok(())
}

fn validate_revision(revision: u64) -> Result<(), ValidationError> {
    if revision > MAX_SAFE_JSON_INTEGER {
        Err(ValidationError::new(
            "workspace_revision",
            "workspace revision must be a safe JSON integer",
        ))
    } else {
        Ok(())
    }
}

fn is_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn parse_number_string(field: &'static str, value: &str) -> Result<f64, ValidationError> {
    let parsed = match value {
        "NaN" => Some(f64::NAN),
        "+Inf" => Some(f64::INFINITY),
        "-Inf" => Some(f64::NEG_INFINITY),
        _ if valid_decimal_number(value) => value.parse::<f64>().ok(),
        _ => None,
    };
    parsed.ok_or_else(|| {
        ValidationError::new(
            "workspace_value",
            format!("{field} must use the canonical number-string vocabulary"),
        )
    })
}

fn valid_decimal_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut index = usize::from(bytes[0] == b'-');
    if index >= bytes.len() {
        return false;
    }
    if bytes[index] == b'0' {
        index += 1;
        if index < bytes.len() && bytes[index].is_ascii_digit() {
            return false;
        }
    } else if bytes[index].is_ascii_digit() && bytes[index] != b'0' {
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
    } else {
        return false;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let fraction_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if fraction_start == index {
            return false;
        }
    }
    if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
        index += 1;
        if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
            index += 1;
        }
        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if exponent_start == index {
            return false;
        }
    }
    index == bytes.len()
}

fn check_frame(input: &str) -> Result<(), CodecError> {
    kernel_v2::check_json_frame_bytes(input.as_bytes()).map(|_| ())
}

fn encode_checked<T: Serialize>(value: &T) -> Result<String, CodecError> {
    let encoded = serde_json::to_string(value)?;
    kernel_v2::check_json_frame_bytes(encoded.as_bytes())?;
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_exact_numeric_write_and_rejects_stale_unsafe_revision() {
        let valid = SetVariableElementRequest {
            name: "signal".to_owned(),
            indices: vec![2, 3],
            value: NumericScalar {
                real: "-1.25e2".to_owned(),
                imaginary: "0".to_owned(),
            },
            expected_revision: 4,
        };
        assert_eq!(valid.value.components().unwrap(), (-125.0, 0.0));
        assert!(valid.validate().is_ok());

        let invalid = SetVariableElementRequest {
            expected_revision: MAX_SAFE_JSON_INTEGER + 1,
            ..valid
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn request_codec_round_trips_the_v3_mutation() {
        let request = RequestEnvelope::new(
            "session",
            "write-1",
            Request::SetVariableElement(SetVariableElementRequest {
                name: "A".to_owned(),
                indices: vec![1, 2],
                value: NumericScalar {
                    real: "7".to_owned(),
                    imaginary: "-3".to_owned(),
                },
                expected_revision: 9,
            }),
        );
        let limits = AggregateLimits::default();
        let encoded = encode_request(&request, &limits).unwrap();
        assert_eq!(decode_request(&encoded, &limits).unwrap(), request);
    }
}

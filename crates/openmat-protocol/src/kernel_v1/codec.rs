use std::error::Error;
use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::model::{
    BootstrapRequestEnvelope, BootstrapResponseEnvelope, EventEnvelope, PreviewLimits,
    RequestEnvelope, ResponseEnvelope, ServerMessage,
};
use crate::ValidationError;

/// A JSON syntax/shape failure or a semantic protocol-validation failure.
#[derive(Debug)]
pub enum CodecError {
    /// Malformed JSON, an unknown required enum tag, or a field type mismatch.
    Json(serde_json::Error),
    /// A typed message that violates a cross-field or negotiated invariant.
    Validation(ValidationError),
}

impl CodecError {
    /// Returns the semantic validation category, when decoding reached semantic
    /// validation.
    #[must_use]
    pub fn validation_category(&self) -> Option<&'static str> {
        match self {
            Self::Json(_) => None,
            Self::Validation(error) => Some(error.category()),
        }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "json: {error}"),
            Self::Validation(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for CodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Validation(error) => Some(error),
        }
    }
}

impl From<serde_json::Error> for CodecError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<ValidationError> for CodecError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

/// Decodes and validates the first bootstrap-v0 initialize request.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed JSON, non-initialize shapes, a non-v0
/// envelope, duplicate protocol offers, or invalid v1 capability limits.
pub fn decode_bootstrap_request(input: &str) -> Result<BootstrapRequestEnvelope, CodecError> {
    decode_and_validate(input, BootstrapRequestEnvelope::validate)
}

/// Encodes a validated bootstrap-v0 initialize request.
///
/// # Errors
///
/// Returns [`CodecError`] if the typed request is invalid or JSON encoding
/// fails.
pub fn encode_bootstrap_request(request: &BootstrapRequestEnvelope) -> Result<String, CodecError> {
    validate_and_encode(request, BootstrapRequestEnvelope::validate)
}

/// Decodes and validates the final bootstrap-v0 initialize response.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed JSON or an invalid bootstrap response.
pub fn decode_bootstrap_response(input: &str) -> Result<BootstrapResponseEnvelope, CodecError> {
    decode_and_validate(input, BootstrapResponseEnvelope::validate)
}

/// Encodes a validated bootstrap-v0 initialize response.
///
/// # Errors
///
/// Returns [`CodecError`] if the typed response is invalid or JSON encoding
/// fails.
pub fn encode_bootstrap_response(
    response: &BootstrapResponseEnvelope,
) -> Result<String, CodecError> {
    validate_and_encode(response, BootstrapResponseEnvelope::validate)
}

/// Decodes and validates one post-bootstrap v1 request.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed or unknown request shapes, version
/// mixing, or negotiated-limit violations.
pub fn decode_request(input: &str, limits: &PreviewLimits) -> Result<RequestEnvelope, CodecError> {
    let request: RequestEnvelope = serde_json::from_str(input)?;
    request.validate(limits)?;
    Ok(request)
}

/// Encodes one validated post-bootstrap v1 request.
///
/// # Errors
///
/// Returns [`CodecError`] if validation or JSON encoding fails.
pub fn encode_request(
    request: &RequestEnvelope,
    limits: &PreviewLimits,
) -> Result<String, CodecError> {
    request.validate(limits)?;
    serde_json::to_string(request).map_err(CodecError::from)
}

/// Decodes and validates one post-bootstrap v1 response.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed or unknown response-result shapes,
/// version mixing, or invalid preview/error payloads.
pub fn decode_response(
    input: &str,
    limits: &PreviewLimits,
) -> Result<ResponseEnvelope, CodecError> {
    let response: ResponseEnvelope = serde_json::from_str(input)?;
    response.validate(limits)?;
    Ok(response)
}

/// Decodes a v1 response and validates correlation, result type, and the
/// inspect request's `maxElements` limit against its originating request.
///
/// # Errors
///
/// Returns [`CodecError`] for any ordinary response failure plus correlation,
/// result-type, or request-specific preview-bound violations.
pub fn decode_response_for_request(
    input: &str,
    request: &RequestEnvelope,
    limits: &PreviewLimits,
) -> Result<ResponseEnvelope, CodecError> {
    let response: ResponseEnvelope = serde_json::from_str(input)?;
    response.validate_for_request(request, limits)?;
    Ok(response)
}

/// Encodes one validated post-bootstrap v1 response.
///
/// # Errors
///
/// Returns [`CodecError`] if validation or JSON encoding fails.
pub fn encode_response(
    response: &ResponseEnvelope,
    limits: &PreviewLimits,
) -> Result<String, CodecError> {
    response.validate(limits)?;
    serde_json::to_string(response).map_err(CodecError::from)
}

/// Encodes a v1 response after validating it against the originating request.
///
/// # Errors
///
/// Returns [`CodecError`] for correlation, result-type, request-specific bound,
/// semantic, or JSON-encoding failures.
pub fn encode_response_for_request(
    response: &ResponseEnvelope,
    request: &RequestEnvelope,
    limits: &PreviewLimits,
) -> Result<String, CodecError> {
    response.validate_for_request(request, limits)?;
    serde_json::to_string(response).map_err(CodecError::from)
}

/// Decodes and validates one post-bootstrap v1 event. Known malformed events
/// fail; unknown event types retain their uninterpreted data.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed JSON, version mixing, or an invalid
/// known event payload.
pub fn decode_event(input: &str) -> Result<EventEnvelope, CodecError> {
    decode_and_validate(input, EventEnvelope::validate)
}

/// Encodes one validated post-bootstrap v1 event.
///
/// # Errors
///
/// Returns [`CodecError`] if validation or JSON encoding fails.
pub fn encode_event(event: &EventEnvelope) -> Result<String, CodecError> {
    validate_and_encode(event, EventEnvelope::validate)
}

/// Decodes and validates a post-bootstrap server response or event.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed JSON, unknown required shapes, version
/// mixing, or semantic validation failures.
pub fn decode_server_message(
    input: &str,
    limits: &PreviewLimits,
) -> Result<ServerMessage, CodecError> {
    let message: ServerMessage = serde_json::from_str(input)?;
    message.validate(limits)?;
    Ok(message)
}

/// Encodes a validated post-bootstrap server response or event.
///
/// # Errors
///
/// Returns [`CodecError`] if validation or JSON encoding fails.
pub fn encode_server_message(
    message: &ServerMessage,
    limits: &PreviewLimits,
) -> Result<String, CodecError> {
    message.validate(limits)?;
    serde_json::to_string(message).map_err(CodecError::from)
}

fn decode_and_validate<T>(
    input: &str,
    validate: impl FnOnce(&T) -> Result<(), ValidationError>,
) -> Result<T, CodecError>
where
    T: DeserializeOwned,
{
    let value = serde_json::from_str(input)?;
    validate(&value)?;
    Ok(value)
}

fn validate_and_encode<T>(
    value: &T,
    validate: impl FnOnce(&T) -> Result<(), ValidationError>,
) -> Result<String, CodecError>
where
    T: Serialize,
{
    validate(value)?;
    serde_json::to_string(value).map_err(CodecError::from)
}

use std::error::Error;
use std::fmt;
use std::str::Utf8Error;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::MAX_JSON_FRAME_BYTES;
use super::model::{
    AggregateLimits, BootstrapRequestEnvelope, BootstrapResponseEnvelope, EventEnvelope,
    RequestEnvelope, ResponseEnvelope, ServerMessage,
};
use crate::ValidationError;

/// JSON, semantic, UTF-8, or complete-frame-bound failure.
#[derive(Debug)]
pub enum CodecError {
    /// Malformed JSON, unknown required tag, or field type mismatch.
    Json(serde_json::Error),
    /// Typed message violates a cross-field or negotiated invariant.
    Validation(ValidationError),
    /// Complete UTF-8 JSON frame exceeds one mebibyte.
    FrameTooLarge {
        /// Actual frame byte count.
        actual: usize,
        /// Contract maximum.
        maximum: usize,
    },
    /// Complete frame bytes are not valid UTF-8.
    Utf8(Utf8Error),
    /// Complete frame begins with a forbidden UTF-8 BOM.
    Utf8Bom,
}

impl CodecError {
    /// Returns the semantic validation category, when available.
    #[must_use]
    pub fn validation_category(&self) -> Option<&'static str> {
        match self {
            Self::Validation(error) => Some(error.category()),
            Self::Json(_) | Self::FrameTooLarge { .. } | Self::Utf8(_) | Self::Utf8Bom => None,
        }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "json: {error}"),
            Self::Validation(error) => write!(formatter, "{error}"),
            Self::FrameTooLarge { actual, maximum } => {
                write!(
                    formatter,
                    "JSON frame has {actual} bytes; maximum is {maximum}"
                )
            }
            Self::Utf8(error) => write!(formatter, "frame is not UTF-8: {error}"),
            Self::Utf8Bom => formatter.write_str("UTF-8 JSON frames must not contain a BOM"),
        }
    }
}

impl Error for CodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Validation(error) => Some(error),
            Self::Utf8(error) => Some(error),
            Self::FrameTooLarge { .. } | Self::Utf8Bom => None,
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

/// Checks one complete frame as BOM-free UTF-8 JSON within the byte bound.
///
/// This helper is the narrow transport integration point: callers pass the
/// actual complete bytes, including the envelope. It performs no socket I/O.
///
/// # Errors
///
/// Returns [`CodecError`] for one-byte-over frames, a BOM, invalid UTF-8, or a
/// syntactically incomplete/non-JSON text.
pub fn check_json_frame_bytes(input: &[u8]) -> Result<&str, CodecError> {
    check_frame_size(input.len())?;
    if input.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(CodecError::Utf8Bom);
    }
    let text = std::str::from_utf8(input).map_err(CodecError::Utf8)?;
    serde_json::from_str::<serde::de::IgnoredAny>(text)?;
    Ok(text)
}

/// Decodes and validates the first bootstrap-v0 initialize request.
///
/// # Errors
///
/// Returns [`CodecError`] for frame, JSON, envelope, offer, or capability
/// failures.
pub fn decode_bootstrap_request(input: &str) -> Result<BootstrapRequestEnvelope, CodecError> {
    decode_and_validate(input, BootstrapRequestEnvelope::validate)
}

/// Encodes a validated bootstrap-v0 initialize request.
///
/// # Errors
///
/// Returns [`CodecError`] if validation, encoding, or the frame bound fails.
pub fn encode_bootstrap_request(request: &BootstrapRequestEnvelope) -> Result<String, CodecError> {
    validate_and_encode(request, BootstrapRequestEnvelope::validate)
}

/// Decodes and validates the final bootstrap-v0 initialize response.
///
/// # Errors
///
/// Returns [`CodecError`] for any malformed response or frame.
pub fn decode_bootstrap_response(input: &str) -> Result<BootstrapResponseEnvelope, CodecError> {
    decode_and_validate(input, BootstrapResponseEnvelope::validate)
}

/// Encodes a validated bootstrap-v0 initialize response.
///
/// # Errors
///
/// Returns [`CodecError`] if validation, encoding, or the frame bound fails.
pub fn encode_bootstrap_response(
    response: &BootstrapResponseEnvelope,
) -> Result<String, CodecError> {
    validate_and_encode(response, BootstrapResponseEnvelope::validate)
}

/// Decodes and validates one post-bootstrap v2 request.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed JSON, protocol mixing, unknown request
/// kinds, or negotiated-limit violations.
pub fn decode_request(
    input: &str,
    limits: &AggregateLimits,
) -> Result<RequestEnvelope, CodecError> {
    check_text_frame(input)?;
    let request: RequestEnvelope = serde_json::from_str(input)?;
    request.validate(limits)?;
    Ok(request)
}

/// Encodes one validated post-bootstrap v2 request.
///
/// # Errors
///
/// Returns [`CodecError`] for validation, JSON, or frame-size failure.
pub fn encode_request(
    request: &RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<String, CodecError> {
    request.validate(limits)?;
    encode_checked(request)
}

/// Decodes and validates one post-bootstrap v2 response.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed required kinds, protocol mixing,
/// semantic inconsistency, or frame failure.
pub fn decode_response(
    input: &str,
    limits: &AggregateLimits,
) -> Result<ResponseEnvelope, CodecError> {
    check_text_frame(input)?;
    let response: ResponseEnvelope = serde_json::from_str(input)?;
    response.validate(limits)?;
    Ok(response)
}

/// Decodes a response and validates it against its originating request.
///
/// # Errors
///
/// Returns [`CodecError`] for ordinary decode failures plus correlation,
/// result-type, selected-range, or request-specific bound violations.
pub fn decode_response_for_request(
    input: &str,
    request: &RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<ResponseEnvelope, CodecError> {
    check_text_frame(input)?;
    let response: ResponseEnvelope = serde_json::from_str(input)?;
    response.validate_for_request(request, limits)?;
    Ok(response)
}

/// Encodes one validated post-bootstrap v2 response.
///
/// # Errors
///
/// Returns [`CodecError`] for semantic, JSON, or complete-frame failure.
pub fn encode_response(
    response: &ResponseEnvelope,
    limits: &AggregateLimits,
) -> Result<String, CodecError> {
    response.validate(limits)?;
    encode_checked(response)
}

/// Encodes a response after request-aware validation.
///
/// # Errors
///
/// Returns [`CodecError`] for correlation, result-type, selected-range,
/// request-specific bound, semantic, JSON, or frame-size failure.
pub fn encode_response_for_request(
    response: &ResponseEnvelope,
    request: &RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<String, CodecError> {
    response.validate_for_request(request, limits)?;
    encode_checked(response)
}

/// Decodes and validates one post-bootstrap v2 event.
///
/// Unknown event types remain forward-compatible and preserve their data.
///
/// # Errors
///
/// Returns [`CodecError`] for malformed frames, protocol mixing, or invalid
/// known event data.
pub fn decode_event(input: &str) -> Result<EventEnvelope, CodecError> {
    decode_and_validate(input, EventEnvelope::validate)
}

/// Encodes one validated post-bootstrap v2 event.
///
/// # Errors
///
/// Returns [`CodecError`] for validation, JSON, or frame-size failure.
pub fn encode_event(event: &EventEnvelope) -> Result<String, CodecError> {
    validate_and_encode(event, EventEnvelope::validate)
}

/// Decodes a post-bootstrap server response or event.
///
/// # Errors
///
/// Returns [`CodecError`] for invalid required kinds, version mixing, semantic
/// inconsistency, or frame failure.
pub fn decode_server_message(
    input: &str,
    limits: &AggregateLimits,
) -> Result<ServerMessage, CodecError> {
    check_text_frame(input)?;
    let message: ServerMessage = serde_json::from_str(input)?;
    message.validate(limits)?;
    Ok(message)
}

/// Encodes a validated post-bootstrap server message.
///
/// # Errors
///
/// Returns [`CodecError`] for validation, JSON, or frame-size failure.
pub fn encode_server_message(
    message: &ServerMessage,
    limits: &AggregateLimits,
) -> Result<String, CodecError> {
    message.validate(limits)?;
    encode_checked(message)
}

fn decode_and_validate<T>(
    input: &str,
    validate: impl FnOnce(&T) -> Result<(), ValidationError>,
) -> Result<T, CodecError>
where
    T: DeserializeOwned,
{
    check_text_frame(input)?;
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
    encode_checked(value)
}

fn encode_checked<T: Serialize>(value: &T) -> Result<String, CodecError> {
    let encoded = serde_json::to_string(value)?;
    check_frame_size(encoded.len())?;
    Ok(encoded)
}

fn check_text_frame(input: &str) -> Result<(), CodecError> {
    check_frame_size(input.len())?;
    if input.as_bytes().starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(CodecError::Utf8Bom);
    }
    Ok(())
}

fn check_frame_size(actual: usize) -> Result<(), CodecError> {
    if actual > MAX_JSON_FRAME_BYTES {
        Err(CodecError::FrameTooLarge {
            actual,
            maximum: MAX_JSON_FRAME_BYTES,
        })
    } else {
        Ok(())
    }
}

//! Typed `openmat-kernel-v1` messages and bootstrap negotiation.
//!
//! The v1 model is deliberately separate from the frozen v0 public model. The
//! initialize exchange uses [`kernel_v1::BootstrapRequestEnvelope`] and
//! [`kernel_v1::BootstrapResponseEnvelope`], both of which require a v0 outer
//! envelope. After initialization, the transport selects one
//! [`kernel_v1::ProtocolVersion`] and can use
//! [`kernel_v1::validate_session_protocol`] to enforce that selection without
//! putting socket or session state in this crate.
//!
//! [`kernel_v1::BootstrapRequestEnvelope`]: crate::kernel_v1::BootstrapRequestEnvelope
//! [`kernel_v1::BootstrapResponseEnvelope`]: crate::kernel_v1::BootstrapResponseEnvelope
//! [`kernel_v1::ProtocolVersion`]: crate::kernel_v1::ProtocolVersion
//! [`kernel_v1::validate_session_protocol`]: crate::kernel_v1::validate_session_protocol

mod codec;
mod model;

pub use codec::{
    CodecError, decode_bootstrap_request, decode_bootstrap_response, decode_event, decode_request,
    decode_response, decode_response_for_request, decode_server_message, encode_bootstrap_request,
    encode_bootstrap_response, encode_event, encode_request, encode_response,
    encode_response_for_request, encode_server_message,
};
pub use model::{
    BootstrapRequest, BootstrapRequestEnvelope, BootstrapResponseEnvelope, BootstrapResponseResult,
    Capabilities, EventEnvelope, InitializeRequest, InitializeResult, InspectRequest,
    MatrixPreview, MatrixRange, PreviewLimits, PreviewValue, Request, RequestEnvelope,
    ResponseEnvelope, ResponseResult, ServerMessage,
};

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use crate::{ImplementationInfo, PROTOCOL_V0, ProtocolError};

/// The negotiated v1 protocol identifier.
pub const PROTOCOL_V1: &str = "openmat-kernel-v1";

/// Largest exact integer that every JSON implementation must preserve.
pub const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

/// Hard maximum UTF-16 code-unit count for one string element.
pub const MAX_STRING_ELEMENT_CODE_UNITS: u64 = 16_384;

/// Hard maximum UTF-16 code-unit count for one preview.
pub const MAX_PREVIEW_CODE_UNITS: u64 = 65_536;

/// Production v1 client offer, in preference order.
pub const SUPPORTED_PROTOCOLS: [&str; 2] = [PROTOCOL_V1, PROTOCOL_V0];

/// A protocol version understood by this codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolVersion {
    /// Frozen compatibility protocol.
    V0,
    /// Exact char, string, and integer preview protocol.
    V1,
}

impl ProtocolVersion {
    /// Returns the stable wire identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V0 => PROTOCOL_V0,
            Self::V1 => PROTOCOL_V1,
        }
    }

    /// Parses a known protocol identifier.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        match identifier {
            PROTOCOL_V0 => Some(Self::V0),
            PROTOCOL_V1 => Some(Self::V1),
            _ => None,
        }
    }
}

/// A deterministic initialization-negotiation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NegotiationError {
    category: &'static str,
    message: String,
}

impl NegotiationError {
    fn new(category: &'static str, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    /// Returns the stable protocol error category.
    #[must_use]
    pub const fn category(&self) -> &'static str {
        self.category
    }

    /// Converts this failure to the shared structured wire error.
    #[must_use]
    pub fn to_protocol_error(&self) -> ProtocolError {
        ProtocolError::new(self.category, self.message.clone())
    }
}

impl fmt::Display for NegotiationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl Error for NegotiationError {}

/// Result of deterministic protocol and capability negotiation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NegotiatedInitialization {
    /// Selected transport protocol for every post-bootstrap envelope.
    pub protocol: ProtocolVersion,
    /// Component-wise common capabilities for the initialize response.
    pub capabilities: Capabilities,
}

/// Returns the production v1 client's ordered protocol offer.
#[must_use]
pub fn production_protocol_offer() -> Vec<String> {
    SUPPORTED_PROTOCOLS
        .iter()
        .map(|protocol| (*protocol).to_owned())
        .collect()
}

/// Selects the first offered protocol supported by a v1 kernel.
///
/// Unknown offered identifiers are allowed for forward negotiation. Empty or
/// duplicate entries are malformed, and an offer with no known identifier
/// returns `protocol.noCommonVersion`.
///
/// # Errors
///
/// Returns [`NegotiationError`] when the ordered offer is malformed or has no
/// common version.
pub fn select_protocol(offered: &[String]) -> Result<ProtocolVersion, NegotiationError> {
    let mut unique = BTreeSet::new();
    for identifier in offered {
        if identifier.is_empty() || !unique.insert(identifier.as_str()) {
            return Err(NegotiationError::new(
                "protocol.invalidOffer",
                "supportedProtocols entries must be non-empty and unique",
            ));
        }
    }

    offered
        .iter()
        .find_map(|identifier| ProtocolVersion::from_identifier(identifier))
        .ok_or_else(|| {
            NegotiationError::new(
                "protocol.noCommonVersion",
                "client and kernel have no common protocol version",
            )
        })
}

/// Selects a protocol and computes its negotiated capabilities.
///
/// The client offer is validated before selection, so offering v1 with missing
/// or invalid v1 limits fails instead of silently falling back to v0.
///
/// # Errors
///
/// Returns [`NegotiationError`] for malformed capabilities, malformed protocol
/// offers, or the absence of a common version.
pub fn negotiate_initialize(
    request: &InitializeRequest,
    kernel_capabilities: &Capabilities,
) -> Result<NegotiatedInitialization, NegotiationError> {
    request.validate_for_negotiation()?;
    let protocol = select_protocol(&request.supported_protocols)?;
    let capabilities =
        Capabilities::negotiate(&request.capabilities, kernel_capabilities, protocol)?;
    Ok(NegotiatedInitialization {
        protocol,
        capabilities,
    })
}

/// Ensures one post-bootstrap envelope matches the transport's selected
/// protocol. This helper is stateless; the transport owns the selected value.
///
/// # Errors
///
/// Returns [`NegotiationError`] for a mismatched or unknown identifier.
pub fn validate_session_protocol(
    selected: ProtocolVersion,
    envelope_protocol: &str,
) -> Result<(), NegotiationError> {
    if envelope_protocol == selected.as_str() {
        Ok(())
    } else {
        Err(NegotiationError::new(
            "protocol.mismatch",
            format!("expected {}, got {envelope_protocol}", selected.as_str()),
        ))
    }
}

/// Validates a successful bootstrap response against its request and returns
/// the version the outer transport must lock for subsequent envelopes.
///
/// # Errors
///
/// Returns [`NegotiationError`] if correlation, bootstrap versions, selected
/// protocol, or selected-version capability shape is inconsistent.
pub fn validate_initialize_exchange(
    request: &BootstrapRequestEnvelope,
    response: &BootstrapResponseEnvelope,
) -> Result<ProtocolVersion, NegotiationError> {
    request
        .validate()
        .map_err(|error| validation_as_negotiation(&error))?;
    response
        .validate()
        .map_err(|error| validation_as_negotiation(&error))?;
    if request.session_id != response.session_id || request.message_id != response.reply_to {
        return Err(NegotiationError::new(
            "protocol.invalidNegotiation",
            "initialize response does not correlate to the bootstrap request",
        ));
    }
    if !response.ok {
        return Err(NegotiationError::new(
            "protocol.initializationFailed",
            "initialize response reports failure",
        ));
    }
    let Some(BootstrapResponseResult::Initialize(result)) = &response.result else {
        return Err(NegotiationError::new(
            "protocol.invalidNegotiation",
            "successful bootstrap response must contain initialize data",
        ));
    };
    let BootstrapRequest::Initialize(initialize) = &request.request;
    let selected =
        ProtocolVersion::from_identifier(&result.negotiated_protocol).ok_or_else(|| {
            NegotiationError::new(
                "protocol.invalidNegotiation",
                "initialize response selected an unknown protocol",
            )
        })?;
    if !initialize
        .supported_protocols
        .iter()
        .any(|offered| offered == selected.as_str())
    {
        return Err(NegotiationError::new(
            "protocol.invalidNegotiation",
            "initialize response selected a protocol the client did not offer",
        ));
    }
    result
        .capabilities
        .validate_for_selection(selected)
        .map_err(|error| validation_as_negotiation(&error))?;
    Ok(selected)
}

fn validation_as_negotiation(error: &crate::ValidationError) -> NegotiationError {
    NegotiationError::new(error.category(), error.to_string())
}

/// Builds the typed initialize result after negotiation.
#[must_use]
pub fn initialize_result(
    negotiated: NegotiatedInitialization,
    implementation: ImplementationInfo,
) -> InitializeResult {
    InitializeResult {
        negotiated_protocol: negotiated.protocol.as_str().to_owned(),
        implementation,
        capabilities: negotiated.capabilities,
    }
}

#[cfg(test)]
mod tests;

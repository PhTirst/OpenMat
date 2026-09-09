//! Typed `openmat-kernel-v2` messages, aggregate previews, and negotiation.
//!
//! Initialization continues to use v0 envelopes. Post-bootstrap messages use
//! exactly the selected protocol and v2 adds recursive aggregate inspection
//! without changing the frozen v0 or v1 public models.

mod codec;
mod model;

pub use crate::kernel_v1::{MatrixPreview, PreviewValue};
pub use codec::{
    CodecError, check_json_frame_bytes, decode_bootstrap_request, decode_bootstrap_response,
    decode_event, decode_request, decode_response, decode_response_for_request,
    decode_server_message, encode_bootstrap_request, encode_bootstrap_response, encode_event,
    encode_request, encode_response, encode_response_for_request, encode_server_message,
};
pub use model::{
    AggregateLimits, AggregatePreview, BootstrapRequest, BootstrapRequestEnvelope,
    BootstrapResponseEnvelope, BootstrapResponseResult, Capabilities, EventEnvelope, ExactPayload,
    ExactValue, InitializeRequest, InitializeResult, InspectPreview, InspectRequest, IntegerValue,
    MatrixRange, PreviewUsage, Request, RequestEnvelope, ResponseEnvelope, ResponseResult,
    ServerMessage, StructRecord,
};

use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::fmt;
use std::hash::Hash;

use crate::{ImplementationInfo, PROTOCOL_V0, ProtocolError};

/// The negotiated v2 protocol identifier.
pub const PROTOCOL_V2: &str = "openmat-kernel-v2";

/// Hard maximum number of recursive exact-value nodes in one preview.
pub const MAX_AGGREGATE_NODES: u64 = 16_384;

/// Hard maximum aggregate element count in one preview.
pub const MAX_AGGREGATE_ELEMENTS: u64 = 65_536;

/// Hard maximum exact-value depth, with the aggregate root at depth zero.
pub const MAX_AGGREGATE_DEPTH: u64 = 32;

/// Hard maximum byte length of one complete UTF-8 JSON frame.
pub const MAX_JSON_FRAME_BYTES: usize = 1_048_576;

/// Production v2 client offer, in preference order.
pub const SUPPORTED_PROTOCOLS: [&str; 3] =
    [PROTOCOL_V2, crate::kernel_v1::PROTOCOL_V1, PROTOCOL_V0];

/// A protocol version understood by the v2 codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolVersion {
    /// Frozen compatibility protocol.
    V0,
    /// Exact scalar matrix-preview protocol.
    V1,
    /// Recursive aggregate-preview protocol.
    V2,
    /// Optimistic workspace-element mutation protocol.
    V3,
}

impl ProtocolVersion {
    /// Returns the stable wire identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V0 => PROTOCOL_V0,
            Self::V1 => crate::kernel_v1::PROTOCOL_V1,
            Self::V2 => PROTOCOL_V2,
            Self::V3 => crate::kernel_v3::PROTOCOL_V3,
        }
    }

    /// Parses a known protocol identifier.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        match identifier {
            PROTOCOL_V0 => Some(Self::V0),
            crate::kernel_v1::PROTOCOL_V1 => Some(Self::V1),
            PROTOCOL_V2 => Some(Self::V2),
            crate::kernel_v3::PROTOCOL_V3 => Some(Self::V3),
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
    pub(super) fn new(category: &'static str, message: impl Into<String>) -> Self {
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
    /// Component-wise negotiated capabilities for the initialize response.
    pub capabilities: Capabilities,
}

/// Returns the production v2 client's ordered protocol offer.
#[must_use]
pub fn production_protocol_offer() -> Vec<String> {
    SUPPORTED_PROTOCOLS
        .iter()
        .map(|protocol| (*protocol).to_owned())
        .collect()
}

/// Selects the first offered protocol supported by a v2 implementation.
///
/// # Errors
///
/// Returns [`NegotiationError`] for empty or duplicate entries, or when no
/// common protocol exists.
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
/// Capability validation happens before selection. Consequently an invalid v2
/// offer is fatal and is never repaired by silently selecting v1 or v0.
///
/// # Errors
///
/// Returns [`NegotiationError`] for malformed offers or capabilities, or when
/// no common version exists.
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

/// Ensures one post-bootstrap envelope matches the selected protocol.
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

/// Validates a successful bootstrap exchange and returns the selected version.
///
/// # Errors
///
/// Returns [`NegotiationError`] for malformed envelopes, failed correlation,
/// unknown or unoffered selections, or incorrect selected capability shape.
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

/// Builds a typed initialize result after negotiation.
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

/// Producer-side active recursion-path guard for adapter identities.
///
/// The wire tree has no identity, but runtime adapters can use this explicit
/// enter/leave interface while traversing with their own work stack. An
/// identity may reappear after it has left the active path, so harmless sibling
/// sharing is accepted.
#[derive(Debug)]
pub struct ActivePathGuard<I> {
    active: HashSet<I>,
    stack: Vec<I>,
}

impl<I> Default for ActivePathGuard<I> {
    fn default() -> Self {
        Self {
            active: HashSet::new(),
            stack: Vec::new(),
        }
    }
}

impl<I> ActivePathGuard<I>
where
    I: Clone + Eq + Hash,
{
    /// Enters an identity at `depth` after checking cycle and depth precedence.
    ///
    /// # Errors
    ///
    /// Returns `workspace.cyclicValue` for an active identity repetition, or
    /// `workspace.previewDepth` when `depth` exceeds `maximum_depth`.
    pub fn enter(
        &mut self,
        identity: I,
        depth: u64,
        maximum_depth: u64,
    ) -> Result<(), ProducerTraversalError> {
        if self.active.contains(&identity) {
            return Err(ProducerTraversalError::new(
                "workspace.cyclicValue",
                "value identity repeats on the active recursion path",
            ));
        }
        if depth > maximum_depth {
            return Err(ProducerTraversalError::new(
                "workspace.previewDepth",
                "exact value exceeds the negotiated depth limit",
            ));
        }
        self.active.insert(identity.clone());
        self.stack.push(identity);
        Ok(())
    }

    /// Leaves the most recently entered identity.
    ///
    /// # Errors
    ///
    /// Returns `engine.invalidPreview` for an unbalanced producer traversal.
    pub fn leave(&mut self, identity: &I) -> Result<(), ProducerTraversalError> {
        if self.stack.last() != Some(identity) {
            return Err(ProducerTraversalError::new(
                "engine.invalidPreview",
                "active-path traversal leave order is unbalanced",
            ));
        }
        let Some(popped) = self.stack.pop() else {
            return Err(ProducerTraversalError::new(
                "engine.invalidPreview",
                "active-path traversal stack is unexpectedly empty",
            ));
        };
        self.active.remove(&popped);
        Ok(())
    }

    /// Returns the number of active identities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// Returns whether the active path is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

/// Stable producer traversal failure exposed by [`ActivePathGuard`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProducerTraversalError {
    category: &'static str,
    message: &'static str,
}

impl ProducerTraversalError {
    const fn new(category: &'static str, message: &'static str) -> Self {
        Self { category, message }
    }

    /// Returns the stable producer error category.
    #[must_use]
    pub const fn category(&self) -> &'static str {
        self.category
    }
}

impl fmt::Display for ProducerTraversalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl Error for ProducerTraversalError {}

#[cfg(test)]
mod tests;

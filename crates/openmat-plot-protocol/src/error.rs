use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable graphics-v1 error categories.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ErrorCategory {
    /// The request or envelope is malformed.
    #[serde(rename = "graphics.invalidRequest")]
    InvalidRequest,
    /// The request type is unknown to graphics-v1.
    #[serde(rename = "graphics.unsupportedRequest")]
    UnsupportedRequest,
    /// Attachment credentials or active-client policy rejected the request.
    #[serde(rename = "graphics.unauthorized")]
    Unauthorized,
    /// Another graphics client is already attached to this live session.
    #[serde(rename = "graphics.sessionInUse")]
    SessionInUse,
    /// The requested Figure does not exist.
    #[serde(rename = "graphics.unknownFigure")]
    UnknownFigure,
    /// The requested immutable buffer does not exist.
    #[serde(rename = "graphics.unknownBuffer")]
    UnknownBuffer,
    /// The expected Figure revision is stale.
    #[serde(rename = "graphics.revisionConflict")]
    RevisionConflict,
    /// A negotiated frame or immutable-buffer bound was exceeded.
    #[serde(rename = "graphics.payloadLimit")]
    PayloadLimit,
    /// The session resident-data limit was exceeded.
    #[serde(rename = "graphics.residentLimit")]
    ResidentLimit,
    /// A snapshot or delta violates graphics-v1 scene invariants.
    #[serde(rename = "graphics.invalidScene")]
    InvalidScene,
    /// An OMGP frame or transfer violates binary framing invariants.
    #[serde(rename = "graphics.invalidBinaryFrame")]
    InvalidBinaryFrame,
    /// The owning kernel graphics session has ended.
    #[serde(rename = "graphics.sessionClosed")]
    SessionClosed,
}

impl ErrorCategory {
    /// Returns the stable wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "graphics.invalidRequest",
            Self::UnsupportedRequest => "graphics.unsupportedRequest",
            Self::Unauthorized => "graphics.unauthorized",
            Self::SessionInUse => "graphics.sessionInUse",
            Self::UnknownFigure => "graphics.unknownFigure",
            Self::UnknownBuffer => "graphics.unknownBuffer",
            Self::RevisionConflict => "graphics.revisionConflict",
            Self::PayloadLimit => "graphics.payloadLimit",
            Self::ResidentLimit => "graphics.residentLimit",
            Self::InvalidScene => "graphics.invalidScene",
            Self::InvalidBinaryFrame => "graphics.invalidBinaryFrame",
            Self::SessionClosed => "graphics.sessionClosed",
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A non-secret protocol validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    category: ErrorCategory,
    message: String,
}

impl ValidationError {
    /// Creates one validation failure. Callers must never place an attachment
    /// token in `message`.
    #[must_use]
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    /// Returns the stable category.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    /// Returns non-normative diagnostic prose.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl Error for ValidationError {}

/// Error payload used by failed response envelopes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolFailure {
    /// Stable OpenMat-owned category.
    pub category: ErrorCategory,
    /// Non-normative, non-secret prose.
    pub message: String,
}

impl ProtocolFailure {
    /// Creates a wire failure.
    #[must_use]
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }
}

impl From<ValidationError> for ProtocolFailure {
    fn from(error: ValidationError) -> Self {
        Self::new(error.category, error.message)
    }
}

use serde::{Deserialize, Serialize};

use crate::{ErrorCategory, ValidationError};

/// Largest integer represented exactly by every JSON/JavaScript consumer.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
/// Hard graphics-v1 text-frame maximum (1 MiB).
pub const MAX_TEXT_FRAME_BYTES: u64 = 1_048_576;
/// Hard graphics-v1 complete binary-message maximum (8 MiB).
pub const MAX_BINARY_FRAME_BYTES: u64 = 8_388_608;
/// Hard graphics-v1 immutable-buffer maximum (256 MiB).
pub const MAX_BUFFER_BYTES: u64 = 268_435_456;
/// Hard graphics-v1 per-session resident-data maximum (512 MiB).
pub const MAX_RESIDENT_BYTES: u64 = 536_870_912;
/// Hard graphics-v1 live-object maximum.
pub const MAX_LIVE_OBJECTS: u64 = 100_000;

/// Renderer backend negotiated by graphics-v1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderBackend {
    /// Browser WebGPU. V1 has no WebGL fallback.
    Webgpu,
}

/// Limits proposed by a graphics client during initialization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    /// Must be exactly `webgpu` in v1.
    pub render_backend: RenderBackend,
    /// Proposed text-message maximum.
    pub max_text_frame_bytes: u64,
    /// Proposed complete binary-message maximum.
    pub max_binary_frame_bytes: u64,
    /// Proposed immutable-buffer maximum.
    pub max_buffer_bytes: u64,
    /// Proposed resident-data maximum.
    pub max_resident_bytes: u64,
}

impl ClientCapabilities {
    /// Validates nonzero safe integers and hard protocol ceilings.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_bound(
            self.max_text_frame_bytes,
            MAX_TEXT_FRAME_BYTES,
            "maxTextFrameBytes",
        )?;
        validate_bound(
            self.max_binary_frame_bytes,
            MAX_BINARY_FRAME_BYTES,
            "maxBinaryFrameBytes",
        )?;
        validate_bound(self.max_buffer_bytes, MAX_BUFFER_BYTES, "maxBufferBytes")?;
        validate_bound(
            self.max_resident_bytes,
            MAX_RESIDENT_BYTES,
            "maxResidentBytes",
        )?;
        Ok(())
    }
}

/// Common limits selected by client/server negotiation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsLimits {
    /// Text-message limit.
    pub max_text_frame_bytes: u64,
    /// Complete binary-message limit.
    pub max_binary_frame_bytes: u64,
    /// One immutable-buffer limit.
    pub max_buffer_bytes: u64,
    /// Session resident-data limit.
    pub max_resident_bytes: u64,
    /// Session live-object limit.
    #[serde(rename = "maxObjects")]
    pub max_objects: u64,
}

impl Default for GraphicsLimits {
    fn default() -> Self {
        Self::protocol_maxima()
    }
}

impl GraphicsLimits {
    /// Returns graphics-v1's hard maxima.
    #[must_use]
    pub const fn protocol_maxima() -> Self {
        Self {
            max_text_frame_bytes: MAX_TEXT_FRAME_BYTES,
            max_binary_frame_bytes: MAX_BINARY_FRAME_BYTES,
            max_buffer_bytes: MAX_BUFFER_BYTES,
            max_resident_bytes: MAX_RESIDENT_BYTES,
            max_objects: MAX_LIVE_OBJECTS,
        }
    }

    /// Validates one server-side limit set.
    pub fn validate(self) -> Result<(), ValidationError> {
        validate_bound(
            self.max_text_frame_bytes,
            MAX_TEXT_FRAME_BYTES,
            "maxTextFrameBytes",
        )?;
        validate_bound(
            self.max_binary_frame_bytes,
            MAX_BINARY_FRAME_BYTES,
            "maxBinaryFrameBytes",
        )?;
        validate_bound(self.max_buffer_bytes, MAX_BUFFER_BYTES, "maxBufferBytes")?;
        validate_bound(
            self.max_resident_bytes,
            MAX_RESIDENT_BYTES,
            "maxResidentBytes",
        )?;
        validate_bound(self.max_objects, MAX_LIVE_OBJECTS, "maxLiveObjects")
    }

    /// Selects nonzero common bounds, never exceeding either peer or v1.
    pub fn negotiate(server: Self, client: &ClientCapabilities) -> Result<Self, ValidationError> {
        server.validate()?;
        client.validate()?;
        Ok(Self {
            max_text_frame_bytes: server.max_text_frame_bytes.min(client.max_text_frame_bytes),
            max_binary_frame_bytes: server
                .max_binary_frame_bytes
                .min(client.max_binary_frame_bytes),
            max_buffer_bytes: server.max_buffer_bytes.min(client.max_buffer_bytes),
            max_resident_bytes: server.max_resident_bytes.min(client.max_resident_bytes),
            max_objects: server.max_objects,
        })
    }
}

pub(crate) fn validate_safe(value: u64, field: &str) -> Result<(), ValidationError> {
    if value > MAX_SAFE_INTEGER {
        return Err(ValidationError::new(
            ErrorCategory::InvalidRequest,
            format!("{field} must be a JSON safe integer"),
        ));
    }
    Ok(())
}

pub(crate) fn validate_positive_safe(value: u64, field: &str) -> Result<(), ValidationError> {
    validate_safe(value, field)?;
    if value == 0 {
        return Err(ValidationError::new(
            ErrorCategory::InvalidRequest,
            format!("{field} must be positive"),
        ));
    }
    Ok(())
}

fn validate_bound(value: u64, maximum: u64, field: &str) -> Result<(), ValidationError> {
    validate_positive_safe(value, field)?;
    if value > maximum {
        return Err(ValidationError::new(
            ErrorCategory::InvalidRequest,
            format!("{field} exceeds the graphics-v1 maximum"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_selects_nonzero_common_v1_bounds() {
        let client = ClientCapabilities {
            render_backend: RenderBackend::Webgpu,
            max_text_frame_bytes: 64 * 1024,
            max_binary_frame_bytes: 2 * 1024 * 1024,
            max_buffer_bytes: 32 * 1024 * 1024,
            max_resident_bytes: 64 * 1024 * 1024,
        };
        let negotiated = GraphicsLimits::negotiate(GraphicsLimits::default(), &client).unwrap();
        assert_eq!(negotiated.max_text_frame_bytes, 64 * 1024);
        assert_eq!(negotiated.max_binary_frame_bytes, 2 * 1024 * 1024);
        assert_eq!(negotiated.max_objects, MAX_LIVE_OBJECTS);
    }

    #[test]
    fn zero_and_above_protocol_maxima_are_rejected() {
        let limits = GraphicsLimits {
            max_text_frame_bytes: 0,
            ..GraphicsLimits::default()
        };
        assert!(limits.validate().is_err());
        let limits = GraphicsLimits {
            max_buffer_bytes: MAX_BUFFER_BYTES + 1,
            ..GraphicsLimits::default()
        };
        assert!(limits.validate().is_err());
    }
}

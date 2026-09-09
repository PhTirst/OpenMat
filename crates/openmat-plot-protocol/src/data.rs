use serde::{Deserialize, Serialize};

use crate::limits::{MAX_SAFE_INTEGER, validate_safe};
use crate::{ErrorCategory, GraphicsLimits, ValidationError};

/// Numeric element representation carried by graphics-v1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    /// IEEE-754 binary32.
    F32,
    /// IEEE-754 binary64.
    F64,
}

impl DataType {
    /// Returns the element width in bytes.
    #[must_use]
    pub const fn byte_width(self) -> u64 {
        match self {
            Self::F32 => 4,
            Self::F64 => 8,
        }
    }
}

/// Array element order. V1 permits only MATLAB column-major order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageOrder {
    /// MATLAB-compatible column-major order.
    ColumnMajor,
}

/// Byte order. OMGP and v1 data are canonical little-endian.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Endianness {
    /// Little-endian scalar bytes.
    Little,
}

/// Immutable graphics numerical resource descriptor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataRef {
    /// Opaque, non-empty resource identifier.
    pub buffer_id: String,
    /// Scalar representation.
    pub dtype: DataType,
    /// Exact MATLAB dimensions.
    pub shape: Vec<u64>,
    /// Exact element order.
    pub order: StorageOrder,
    /// Exact byte order.
    pub endianness: Endianness,
    /// V1 requires zero and has no stride metadata.
    pub byte_offset: u64,
    /// Complete resource byte count.
    pub byte_length: u64,
}

impl DataRef {
    /// Returns the checked number of elements.
    pub fn element_count(&self) -> Result<u64, ValidationError> {
        if self.shape.len() < 2 {
            return Err(invalid_scene(
                "DataRef shape must contain at least two dimensions",
            ));
        }
        let mut count = 1_u64;
        for extent in &self.shape {
            validate_safe(*extent, "DataRef shape extent")
                .map_err(|_| invalid_scene("DataRef shape extent is not JSON-safe"))?;
            count = count
                .checked_mul(*extent)
                .ok_or_else(|| invalid_scene("DataRef element-count multiplication overflowed"))?;
            if count > MAX_SAFE_INTEGER {
                return Err(invalid_scene("DataRef element count is not JSON-safe"));
            }
        }
        Ok(count)
    }

    /// Validates general v1 descriptor invariants.
    pub fn validate(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        if self.buffer_id.is_empty() {
            return Err(invalid_scene("DataRef bufferId must be non-empty"));
        }
        if self.byte_offset != 0 {
            return Err(invalid_scene("DataRef byteOffset must be zero in v1"));
        }
        validate_safe(self.byte_length, "DataRef byteLength")
            .map_err(|_| invalid_scene("DataRef byteLength is not JSON-safe"))?;
        let expected = self
            .element_count()?
            .checked_mul(self.dtype.byte_width())
            .ok_or_else(|| invalid_scene("DataRef byteLength multiplication overflowed"))?;
        if expected != self.byte_length {
            return Err(invalid_scene(
                "DataRef byteLength does not match shape and dtype",
            ));
        }
        if self.byte_length > limits.max_buffer_bytes {
            return Err(ValidationError::new(
                ErrorCategory::PayloadLimit,
                "DataRef exceeds the negotiated immutable-buffer limit",
            ));
        }
        Ok(())
    }

    /// Validates the first-tranche per-series contiguous 1-by-N restriction.
    pub fn validate_series(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        self.validate(limits)?;
        if self.shape.len() != 2 || self.shape[0] != 1 {
            return Err(invalid_scene(
                "first-tranche series DataRef shape must be exactly 1-by-N",
            ));
        }
        Ok(())
    }

    /// Validates one contiguous two-dimensional matrix resource.
    pub fn validate_matrix(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        self.validate(limits)?;
        if self.shape.len() != 2 {
            return Err(invalid_scene(
                "matrix DataRef shape must contain exactly two dimensions",
            ));
        }
        Ok(())
    }
}

/// Text result preceding one server-to-client binary transfer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferTransfer {
    /// Exactly `getBuffer`.
    #[serde(rename = "type")]
    pub result_type: String,
    /// Opaque per-connection transfer identifier.
    pub transfer_id: String,
    /// Immutable resource identifier.
    pub buffer_id: String,
    /// Complete resource length.
    pub total_bytes: u64,
    /// Nominal non-final chunk length, or zero for an empty resource.
    pub chunk_bytes: u64,
    /// Exact number of following OMGP binary messages.
    pub chunk_count: u64,
}

impl BufferTransfer {
    /// Validates transfer descriptor/count arithmetic.
    pub fn validate(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        if self.result_type != "getBuffer" {
            return Err(invalid_request("buffer transfer type must be getBuffer"));
        }
        if self.transfer_id.is_empty() || self.buffer_id.is_empty() {
            return Err(invalid_request("transferId and bufferId must be non-empty"));
        }
        for (value, field) in [
            (self.total_bytes, "totalBytes"),
            (self.chunk_bytes, "chunkBytes"),
            (self.chunk_count, "chunkCount"),
        ] {
            validate_safe(value, field)?;
        }
        if self.total_bytes > limits.max_buffer_bytes {
            return Err(ValidationError::new(
                ErrorCategory::PayloadLimit,
                "buffer transfer exceeds the negotiated buffer limit",
            ));
        }
        if self.total_bytes == 0 {
            if self.chunk_bytes != 0 || self.chunk_count != 0 {
                return Err(invalid_request(
                    "empty transfers require zero chunkBytes and chunkCount",
                ));
            }
            return Ok(());
        }
        if self.chunk_bytes == 0 || self.chunk_count == 0 {
            return Err(invalid_request(
                "non-empty transfers require positive chunkBytes and chunkCount",
            ));
        }
        let payload_capacity = limits
            .max_binary_frame_bytes
            .saturating_sub(crate::BINARY_PREFIX_LEN as u64 + crate::BINARY_HEADER_LIMIT as u64);
        if self.chunk_bytes > payload_capacity {
            return Err(ValidationError::new(
                ErrorCategory::PayloadLimit,
                "chunkBytes cannot fit in one negotiated binary message",
            ));
        }
        let expected_count = self.total_bytes.div_ceil(self.chunk_bytes);
        if self.chunk_count != expected_count {
            return Err(invalid_request(
                "chunkCount does not cover totalBytes using chunkBytes",
            ));
        }
        Ok(())
    }
}

/// JSON header embedded in one OMGP binary message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferChunkHeader {
    /// Exactly `bufferChunk`.
    #[serde(rename = "type")]
    pub message_type: String,
    /// Correlates with the preceding `getBuffer` result.
    pub transfer_id: String,
    /// Must match that transfer's immutable resource.
    pub buffer_id: String,
    /// Byte offset in the complete resource.
    pub offset: u64,
    /// Must match the transfer descriptor.
    pub total_bytes: u64,
    /// True exactly when this payload ends at `totalBytes`.
    #[serde(rename = "final")]
    pub final_chunk: bool,
}

impl BufferChunkHeader {
    /// Creates a typed v1 buffer-chunk header.
    #[must_use]
    pub fn new(
        transfer_id: impl Into<String>,
        buffer_id: impl Into<String>,
        offset: u64,
        total_bytes: u64,
        final_chunk: bool,
    ) -> Self {
        Self {
            message_type: "bufferChunk".to_owned(),
            transfer_id: transfer_id.into(),
            buffer_id: buffer_id.into(),
            offset,
            total_bytes,
            final_chunk,
        }
    }

    pub(crate) fn validate_shape(&self, payload_len: u64) -> Result<(), ValidationError> {
        if self.message_type != "bufferChunk" {
            return Err(invalid_binary("binary header type must be bufferChunk"));
        }
        if self.transfer_id.is_empty() || self.buffer_id.is_empty() {
            return Err(invalid_binary(
                "binary transferId and bufferId must be non-empty",
            ));
        }
        validate_safe(self.offset, "binary offset")
            .map_err(|_| invalid_binary("binary offset is not JSON-safe"))?;
        validate_safe(self.total_bytes, "binary totalBytes")
            .map_err(|_| invalid_binary("binary totalBytes is not JSON-safe"))?;
        let end = self
            .offset
            .checked_add(payload_len)
            .ok_or_else(|| invalid_binary("binary range overflowed"))?;
        if end > self.total_bytes {
            return Err(invalid_binary("binary range exceeds totalBytes"));
        }
        if self.final_chunk != (end == self.total_bytes) {
            return Err(invalid_binary(
                "binary final flag does not match the ending offset",
            ));
        }
        Ok(())
    }
}

fn invalid_request(message: &str) -> ValidationError {
    ValidationError::new(ErrorCategory::InvalidRequest, message)
}

fn invalid_scene(message: &str) -> ValidationError {
    ValidationError::new(ErrorCategory::InvalidScene, message)
}

fn invalid_binary(message: &str) -> ValidationError {
    ValidationError::new(ErrorCategory::InvalidBinaryFrame, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(shape: Vec<u64>, byte_length: u64) -> DataRef {
        DataRef {
            buffer_id: "buffer-1".to_owned(),
            dtype: DataType::F64,
            shape,
            order: StorageOrder::ColumnMajor,
            endianness: Endianness::Little,
            byte_offset: 0,
            byte_length,
        }
    }

    #[test]
    fn empty_series_is_exact_one_by_zero_without_payload() {
        let empty = descriptor(vec![1, 0], 0);
        empty.validate_series(GraphicsLimits::default()).unwrap();
        assert_eq!(empty.element_count().unwrap(), 0);
    }

    #[test]
    fn rejects_offset_shape_byte_length_and_safe_integer_overflow() {
        let mut offset = descriptor(vec![1, 2], 16);
        offset.byte_offset = 8;
        assert!(offset.validate_series(GraphicsLimits::default()).is_err());
        assert!(
            descriptor(vec![2, 1], 16)
                .validate_series(GraphicsLimits::default())
                .is_err()
        );
        assert!(
            descriptor(vec![1, 2], 8)
                .validate_series(GraphicsLimits::default())
                .is_err()
        );
        assert!(
            descriptor(vec![1, MAX_SAFE_INTEGER], MAX_SAFE_INTEGER)
                .validate_series(GraphicsLimits::default())
                .is_err()
        );
    }

    #[test]
    fn transfer_count_and_empty_rules_are_exact() {
        let limits = GraphicsLimits::default();
        let valid = BufferTransfer {
            result_type: "getBuffer".to_owned(),
            transfer_id: "transfer-1".to_owned(),
            buffer_id: "buffer-1".to_owned(),
            total_bytes: 10,
            chunk_bytes: 4,
            chunk_count: 3,
        };
        valid.validate(limits).unwrap();
        let mut invalid = valid;
        invalid.chunk_count = 2;
        assert!(invalid.validate(limits).is_err());

        invalid.total_bytes = 0;
        invalid.chunk_bytes = 0;
        invalid.chunk_count = 1;
        assert!(invalid.validate(limits).is_err());
    }
}

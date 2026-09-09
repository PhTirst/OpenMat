use std::collections::HashMap;

use crate::{BufferChunkHeader, BufferTransfer, ErrorCategory, GraphicsLimits, ValidationError};

/// OMGP four-byte magic.
pub const OMGP_MAGIC: [u8; 4] = *b"OMGP";
/// Supported protocol major.
pub const PROTOCOL_MAJOR: u16 = 1;
/// Supported protocol minor.
pub const PROTOCOL_MINOR: u16 = 0;
/// Exact binary prefix width.
pub const BINARY_PREFIX_LEN: usize = 16;
/// Maximum UTF-8 JSON header width.
pub const BINARY_HEADER_LIMIT: usize = 65_536;

/// One decoded OMGP message. The payload borrows the validated complete frame.
#[derive(Debug)]
pub struct DecodedBinaryFrame<'a> {
    /// Typed JSON header.
    pub header: BufferChunkHeader,
    /// Raw resource range.
    pub payload: &'a [u8],
}

/// Encodes one canonical little-endian OMGP message.
pub fn encode_binary_frame(
    header: &BufferChunkHeader,
    payload: &[u8],
    max_frame_bytes: u64,
) -> Result<Vec<u8>, ValidationError> {
    let payload_len = u64::try_from(payload.len())
        .map_err(|_| invalid_binary("binary payload length does not fit u64"))?;
    header.validate_shape(payload_len)?;
    let header_bytes = serde_json::to_vec(header)
        .map_err(|_| invalid_binary("binary header could not be encoded as JSON"))?;
    if header_bytes.len() > BINARY_HEADER_LIMIT {
        return Err(invalid_binary("binary JSON header exceeds 65536 bytes"));
    }
    let header_len = u32::try_from(header_bytes.len())
        .map_err(|_| invalid_binary("binary header length does not fit u32"))?;
    let payload_len_u32 = u32::try_from(payload.len())
        .map_err(|_| invalid_binary("binary payload length does not fit u32"))?;
    let complete_len = BINARY_PREFIX_LEN
        .checked_add(header_bytes.len())
        .and_then(|value| value.checked_add(payload.len()))
        .ok_or_else(|| invalid_binary("complete binary message length overflowed"))?;
    let complete_len_u64 = u64::try_from(complete_len)
        .map_err(|_| invalid_binary("complete binary message length does not fit u64"))?;
    if complete_len_u64 > max_frame_bytes {
        return Err(ValidationError::new(
            ErrorCategory::PayloadLimit,
            "complete OMGP message exceeds the negotiated binary limit",
        ));
    }

    let mut encoded = Vec::with_capacity(complete_len);
    encoded.extend_from_slice(&OMGP_MAGIC);
    encoded.extend_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    encoded.extend_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    encoded.extend_from_slice(&header_len.to_le_bytes());
    encoded.extend_from_slice(&payload_len_u32.to_le_bytes());
    encoded.extend_from_slice(&header_bytes);
    encoded.extend_from_slice(payload);
    Ok(encoded)
}

/// Decodes and validates one complete OMGP WebSocket binary message.
pub fn decode_binary_frame(
    frame: &[u8],
    max_frame_bytes: u64,
) -> Result<DecodedBinaryFrame<'_>, ValidationError> {
    let frame_len = u64::try_from(frame.len())
        .map_err(|_| invalid_binary("binary frame length does not fit u64"))?;
    if frame_len > max_frame_bytes {
        return Err(ValidationError::new(
            ErrorCategory::PayloadLimit,
            "complete OMGP message exceeds the negotiated binary limit",
        ));
    }
    if frame.len() < BINARY_PREFIX_LEN {
        return Err(invalid_binary(
            "binary frame is shorter than the OMGP prefix",
        ));
    }
    if frame[0..4] != OMGP_MAGIC {
        return Err(invalid_binary("binary frame has invalid OMGP magic"));
    }
    let major = u16::from_le_bytes([frame[4], frame[5]]);
    let minor = u16::from_le_bytes([frame[6], frame[7]]);
    if major != PROTOCOL_MAJOR || minor != PROTOCOL_MINOR {
        return Err(invalid_binary("binary frame has an unsupported version"));
    }
    let header_len = u32::from_le_bytes([frame[8], frame[9], frame[10], frame[11]]);
    let payload_len = u32::from_le_bytes([frame[12], frame[13], frame[14], frame[15]]);
    let header_len = usize::try_from(header_len)
        .map_err(|_| invalid_binary("binary header length does not fit usize"))?;
    let payload_len = usize::try_from(payload_len)
        .map_err(|_| invalid_binary("binary payload length does not fit usize"))?;
    if header_len > BINARY_HEADER_LIMIT {
        return Err(invalid_binary("binary JSON header exceeds 65536 bytes"));
    }
    let expected = BINARY_PREFIX_LEN
        .checked_add(header_len)
        .and_then(|value| value.checked_add(payload_len))
        .ok_or_else(|| invalid_binary("binary prefix lengths overflowed"))?;
    if expected != frame.len() {
        return Err(invalid_binary(
            "binary prefix lengths do not equal the complete message length",
        ));
    }
    let header_end = BINARY_PREFIX_LEN + header_len;
    let header: BufferChunkHeader =
        serde_json::from_slice(&frame[BINARY_PREFIX_LEN..header_end])
            .map_err(|_| invalid_binary("binary header is not valid UTF-8 JSON"))?;
    let payload = &frame[header_end..];
    header.validate_shape(
        u64::try_from(payload.len())
            .map_err(|_| invalid_binary("binary payload length does not fit u64"))?,
    )?;
    Ok(DecodedBinaryFrame { header, payload })
}

/// Complete immutable bytes, exposed only after every descriptor and range has
/// validated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedBuffer {
    /// Resource identifier.
    pub buffer_id: String,
    /// Complete bytes.
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
struct PendingTransfer {
    descriptor: BufferTransfer,
    bytes: Vec<u8>,
    chunks: u64,
}

/// Bounded client-side transfer assembler. Any malformed binary input clears
/// affected partial state; callers then close/reconnect as required by v1.
#[derive(Debug)]
pub struct BufferTransferAssembler {
    limits: GraphicsLimits,
    transfers: HashMap<String, PendingTransfer>,
    resident_bytes: u64,
}

impl BufferTransferAssembler {
    /// Creates an empty assembler under negotiated limits.
    pub fn new(limits: GraphicsLimits) -> Result<Self, ValidationError> {
        limits.validate()?;
        Ok(Self {
            limits,
            transfers: HashMap::new(),
            resident_bytes: 0,
        })
    }

    /// Registers the preceding successful `getBuffer` text result. Empty
    /// buffers complete immediately and require no OMGP message.
    pub fn begin(
        &mut self,
        descriptor: BufferTransfer,
    ) -> Result<Option<CompletedBuffer>, ValidationError> {
        descriptor.validate(self.limits)?;
        if self.transfers.contains_key(&descriptor.transfer_id) {
            return Err(invalid_binary("duplicate transferId"));
        }
        if descriptor.total_bytes == 0 {
            return Ok(Some(CompletedBuffer {
                buffer_id: descriptor.buffer_id,
                bytes: Vec::new(),
            }));
        }
        let new_resident = self
            .resident_bytes
            .checked_add(descriptor.total_bytes)
            .ok_or_else(|| resident_limit("in-flight transfer bytes overflowed"))?;
        if new_resident > self.limits.max_resident_bytes {
            return Err(resident_limit(
                "in-flight transfers exceed the negotiated resident limit",
            ));
        }
        let capacity = usize::try_from(descriptor.total_bytes)
            .map_err(|_| resident_limit("transfer size does not fit this platform"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(capacity).map_err(|_| {
            resident_limit("could not reserve complete transfer without partial exposure")
        })?;
        self.resident_bytes = new_resident;
        self.transfers.insert(
            descriptor.transfer_id.clone(),
            PendingTransfer {
                descriptor,
                bytes,
                chunks: 0,
            },
        );
        Ok(None)
    }

    /// Decodes and adds one OMGP message. On any failure, no partial buffer is
    /// returned and the affected transfer is aborted.
    pub fn push_frame(&mut self, frame: &[u8]) -> Result<Option<CompletedBuffer>, ValidationError> {
        let decoded = match decode_binary_frame(frame, self.limits.max_binary_frame_bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                self.abort_all();
                return Err(error);
            }
        };
        let transfer_id = decoded.header.transfer_id.clone();
        let Some(pending) = self.transfers.get_mut(&transfer_id) else {
            self.abort_all();
            return Err(invalid_binary("binary frame names an unknown transfer"));
        };
        let result = validate_and_append(pending, &decoded);
        if let Err(error) = result {
            self.abort(&transfer_id);
            return Err(error);
        }
        if !decoded.header.final_chunk {
            return Ok(None);
        }
        let Some(pending) = self.transfers.remove(&transfer_id) else {
            self.abort_all();
            return Err(invalid_binary(
                "completed transfer disappeared before publication",
            ));
        };
        self.resident_bytes = self
            .resident_bytes
            .saturating_sub(pending.descriptor.total_bytes);
        Ok(Some(CompletedBuffer {
            buffer_id: pending.descriptor.buffer_id,
            bytes: pending.bytes,
        }))
    }

    /// Aborts one transfer without exposing accumulated bytes.
    pub fn abort(&mut self, transfer_id: &str) {
        if let Some(pending) = self.transfers.remove(transfer_id) {
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(pending.descriptor.total_bytes);
        }
    }

    /// Aborts every transfer, as required when the graphics connection closes.
    pub fn abort_all(&mut self) {
        self.transfers.clear();
        self.resident_bytes = 0;
    }
}

fn validate_and_append(
    pending: &mut PendingTransfer,
    frame: &DecodedBinaryFrame<'_>,
) -> Result<(), ValidationError> {
    if frame.header.buffer_id != pending.descriptor.buffer_id
        || frame.header.total_bytes != pending.descriptor.total_bytes
    {
        return Err(invalid_binary("binary transfer descriptor mismatch"));
    }
    let current_offset = u64::try_from(pending.bytes.len())
        .map_err(|_| invalid_binary("assembled offset does not fit u64"))?;
    if frame.header.offset != current_offset {
        return Err(invalid_binary(
            "binary chunk is duplicate, overlapping, gapped, or out of order",
        ));
    }
    let remaining = pending.descriptor.total_bytes - current_offset;
    let expected_payload = pending.descriptor.chunk_bytes.min(remaining);
    let actual_payload = u64::try_from(frame.payload.len())
        .map_err(|_| invalid_binary("binary chunk length does not fit u64"))?;
    if actual_payload != expected_payload {
        return Err(invalid_binary(
            "binary chunk length differs from descriptor",
        ));
    }
    pending.chunks = pending
        .chunks
        .checked_add(1)
        .ok_or_else(|| invalid_binary("binary chunk count overflowed"))?;
    if pending.chunks > pending.descriptor.chunk_count {
        return Err(invalid_binary("binary transfer has too many chunks"));
    }
    pending.bytes.extend_from_slice(frame.payload);
    if frame.header.final_chunk
        && (pending.chunks != pending.descriptor.chunk_count
            || u64::try_from(pending.bytes.len()).ok() != Some(pending.descriptor.total_bytes))
    {
        return Err(invalid_binary(
            "final binary chunk does not complete transfer",
        ));
    }
    Ok(())
}

fn invalid_binary(message: &str) -> ValidationError {
    ValidationError::new(ErrorCategory::InvalidBinaryFrame, message)
}

fn resident_limit(message: &str) -> ValidationError {
    ValidationError::new(ErrorCategory::ResidentLimit, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> BufferTransfer {
        BufferTransfer {
            result_type: "getBuffer".to_owned(),
            transfer_id: "transfer-1".to_owned(),
            buffer_id: "buffer-1".to_owned(),
            total_bytes: 5,
            chunk_bytes: 3,
            chunk_count: 2,
        }
    }

    fn frame(offset: u64, payload: &[u8], final_chunk: bool) -> Vec<u8> {
        encode_binary_frame(
            &BufferChunkHeader::new("transfer-1", "buffer-1", offset, 5, final_chunk),
            payload,
            crate::MAX_BINARY_FRAME_BYTES,
        )
        .unwrap()
    }

    #[test]
    fn prefix_is_exact_little_endian_and_round_trips() {
        let encoded = frame(0, &[1, 2, 3], false);
        assert_eq!(&encoded[0..4], b"OMGP");
        assert_eq!(&encoded[4..6], &1_u16.to_le_bytes());
        assert_eq!(&encoded[6..8], &0_u16.to_le_bytes());
        assert_eq!(&encoded[12..16], &3_u32.to_le_bytes());
        let decoded = decode_binary_frame(&encoded, crate::MAX_BINARY_FRAME_BYTES).unwrap();
        assert_eq!(decoded.header.offset, 0);
        assert_eq!(decoded.payload, [1, 2, 3]);
    }

    #[test]
    fn rejects_magic_version_lengths_header_and_limit() {
        let valid = frame(0, &[1, 2, 3], false);
        for index in [0, 4, 8, 12] {
            let mut invalid = valid.clone();
            invalid[index] ^= 0xff;
            assert!(decode_binary_frame(&invalid, crate::MAX_BINARY_FRAME_BYTES).is_err());
        }
        assert!(decode_binary_frame(&valid, 8).is_err());

        let mut invalid_json = valid;
        invalid_json[BINARY_PREFIX_LEN] = 0xff;
        assert!(decode_binary_frame(&invalid_json, crate::MAX_BINARY_FRAME_BYTES).is_err());
    }

    #[test]
    fn assembles_ordered_chunks_without_exposing_partial_bytes() {
        let mut assembler = BufferTransferAssembler::new(GraphicsLimits::default()).unwrap();
        assert!(assembler.begin(descriptor()).unwrap().is_none());
        assert!(
            assembler
                .push_frame(&frame(0, b"abc", false))
                .unwrap()
                .is_none()
        );
        let complete = assembler
            .push_frame(&frame(3, b"de", true))
            .unwrap()
            .unwrap();
        assert_eq!(complete.buffer_id, "buffer-1");
        assert_eq!(complete.bytes, b"abcde");
    }

    #[test]
    fn rejects_gap_overlap_duplicate_and_descriptor_mismatch_and_aborts() {
        for bad in [
            frame(1, b"bcd", false),
            encode_binary_frame(
                &BufferChunkHeader::new("transfer-1", "other", 0, 5, false),
                b"abc",
                crate::MAX_BINARY_FRAME_BYTES,
            )
            .unwrap(),
        ] {
            let mut assembler = BufferTransferAssembler::new(GraphicsLimits::default()).unwrap();
            assembler.begin(descriptor()).unwrap();
            assert!(assembler.push_frame(&bad).is_err());
            assert!(assembler.push_frame(&frame(0, b"abc", false)).is_err());
        }

        let mut assembler = BufferTransferAssembler::new(GraphicsLimits::default()).unwrap();
        assembler.begin(descriptor()).unwrap();
        assembler.push_frame(&frame(0, b"abc", false)).unwrap();
        assert!(assembler.push_frame(&frame(0, b"abc", false)).is_err());
    }

    #[test]
    fn empty_transfer_completes_without_binary() {
        let mut assembler = BufferTransferAssembler::new(GraphicsLimits::default()).unwrap();
        let complete = assembler
            .begin(BufferTransfer {
                result_type: "getBuffer".to_owned(),
                transfer_id: "empty-transfer".to_owned(),
                buffer_id: "empty".to_owned(),
                total_bytes: 0,
                chunk_bytes: 0,
                chunk_count: 0,
            })
            .unwrap()
            .unwrap();
        assert!(complete.bytes.is_empty());
    }
}

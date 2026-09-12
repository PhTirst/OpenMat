use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use zip::{CompressionMethod, ZipArchive};

use crate::{Error, Limits, uri::validate_part_name};

// Bound metadata allocation before entering zip-rs. Its reader can search earlier
// end records after a malformed directory; this profile rejects multiple possible
// end records so it cannot fall back to an unchecked archive or ZIP64 record.
fn preflight(bytes: &[u8], limits: Limits) -> Result<usize, Error> {
    if bytes.len() > limits.archive_bytes {
        return Err(Error::new("archive_limit", "archive exceeds byte limit"));
    }
    let end = (bytes.len().saturating_sub(65557)..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            bytes.get(i..i + 4) == Some(b"PK\x05\x06")
                && i + 22 + usize::from(u16::from_le_bytes([bytes[i + 20], bytes[i + 21]]))
                    == bytes.len()
        })
        .ok_or_else(|| Error::new("zip", "ZIP end record is missing"))?;
    for (offset, signature) in bytes.windows(4).enumerate() {
        if signature == b"PK\x05\x06" && offset != end && offset + 22 <= bytes.len() {
            let comment = usize::from(u16::from_le_bytes([bytes[offset + 20], bytes[offset + 21]]));
            if offset + 22 + comment <= bytes.len() {
                return Err(Error::new(
                    "zip_profile",
                    "multiple possible ZIP end records are unsupported",
                ));
            }
        }
    }
    let field = |offset| u16::from_le_bytes([bytes[end + offset], bytes[end + offset + 1]]);
    let count = usize::from(field(10));
    if field(4) != 0
        || field(6) != 0
        || field(8) != field(10)
        || field(10) == u16::MAX
        || end
            .checked_sub(20)
            .is_some_and(|offset| bytes.get(offset..offset + 4) == Some(b"PK\x06\x07"))
    {
        return Err(Error::new(
            "zip_profile",
            "multi-volume and ZIP64 packages are unsupported in v0",
        ));
    }
    if count > limits.parts {
        return Err(Error::new("part_limit", "too many archive entries"));
    }
    let directory_bytes = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap());
    let directory_offset = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap());
    if directory_bytes == u32::MAX
        || directory_offset == u32::MAX
        || u64::from(directory_offset) + u64::from(directory_bytes)
            != u64::try_from(end).unwrap_or(u64::MAX)
    {
        return Err(Error::new(
            "zip_profile",
            "invalid or unsupported central directory extent",
        ));
    }
    Ok(count)
}

pub(crate) fn read(bytes: &[u8], limits: Limits) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let count = preflight(bytes, limits)?;
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::new("zip", e.to_string()))?;
    if archive.len() != count {
        return Err(Error::new("zip_entries", "ambiguous archive entry count"));
    }
    let mut names = BTreeSet::new();
    let mut parts = BTreeMap::new();
    let mut total = 0usize;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|e| Error::new("zip_entry", e.to_string()))?;
        let name = std::str::from_utf8(file.name_raw())
            .map_err(|_| Error::new("part_name", "part names must be UTF-8"))?;
        if name.starts_with('/') {
            return Err(Error::new(
                "part_name",
                "ZIP entries must be package-relative",
            ));
        }
        let name = format!("/{name}");
        // Directory markers are unnecessary in OPC; tolerate empty ZIP markers.
        if file.is_dir() {
            validate_part_name(name.trim_end_matches('/'))?;
            if file.size() != 0 {
                return Err(Error::new("zip_entry", "nonempty directory entry"));
            }
            continue;
        }
        validate_part_name(&name)?;
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(Error::new("duplicate_part", "ambiguous or duplicate part name").at(&name));
        }
        if file.is_symlink()
            || file.encrypted()
            || !matches!(
                file.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            )
        {
            return Err(Error::new(
                "zip_profile",
                "unsupported ZIP entry type, encryption or compression",
            )
            .at(&name));
        }
        let size = usize::try_from(file.size())
            .map_err(|_| Error::new("part_limit", "unrepresentable part size"))?;
        if size > limits.part_bytes || total.saturating_add(size) > limits.total_bytes {
            return Err(Error::new("part_limit", "expanded package exceeds size limit").at(&name));
        }
        let mut content = Vec::new();
        file.by_ref()
            .take(
                u64::try_from(limits.part_bytes)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
            )
            .read_to_end(&mut content)
            .map_err(|e| Error::new("zip_data", e.to_string()).at(&name))?;
        if content.len() != size {
            return Err(
                Error::new("part_size", "expanded size disagrees with ZIP metadata").at(&name),
            );
        }
        total += content.len();
        parts.insert(name, content);
    }
    Ok(parts)
}

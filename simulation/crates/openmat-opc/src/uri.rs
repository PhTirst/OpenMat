use crate::Error;

pub(crate) fn validate_part_name(name: &str) -> Result<(), Error> {
    if !name.starts_with('/')
        || name.len() > 2048
        || name.contains(['\\', '?', '#', ':'])
        || name.bytes().any(|b| b <= 0x20 || b == 0x7f)
        || name[1..]
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(Error::new("part_name", "invalid package part name").at(name));
    }
    let bytes = name.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let encoded = name
                .get(index + 1..index + 3)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .ok_or_else(|| Error::new("part_name", "invalid URI escape").at(name))?;
            if encoded.is_ascii_alphanumeric() || b"/\\._~-".contains(&encoded) || encoded < 0x20 {
                return Err(
                    Error::new("part_name", "escaped separator or unreserved character").at(name),
                );
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

/// Resolve an internal relative relationship against its source part, not the OS.
///
/// # Errors
/// Rejects external URIs, invalid names and package-root traversal.
pub fn resolve_target(source: &str, target: &str) -> Result<String, Error> {
    if source != "/" {
        validate_part_name(source)?;
    }
    if target.is_empty() || target.contains(['\\', ':', '?', '#']) || target.starts_with("//") {
        return Err(Error::new(
            "target_uri",
            "invalid internal relationship URI",
        ));
    }
    let mut segments: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        source.trim_start_matches('/').split('/').collect()
    };
    if !target.starts_with('/') {
        segments.pop();
    }
    for segment in target.trim_start_matches('/').split('/') {
        match segment {
            "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(Error::new(
                        "target_uri",
                        "relationship escapes package root",
                    ));
                }
            }
            "" => return Err(Error::new("target_uri", "empty relationship path segment")),
            _ => segments.push(segment),
        }
    }
    let name = format!("/{}", segments.join("/"));
    validate_part_name(&name)?;
    Ok(name)
}

pub(crate) fn relationship_source(name: &str) -> Result<Option<String>, Error> {
    if name == "/_rels/.rels" {
        return Ok(Some("/".into()));
    }
    if let Some((directory, file)) = name.rsplit_once("/_rels/") {
        if let Some(stem) = file.strip_suffix(".rels") {
            let source = format!("{directory}/{stem}");
            validate_part_name(&source)?;
            return Ok(Some(source));
        }
        return Err(Error::new("relationship_name", "invalid relationships part name").at(name));
    }
    Ok(None)
}

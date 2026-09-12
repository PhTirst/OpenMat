//! Bounded, platform-independent reading of ZIP-based Open Packaging Conventions.
#![forbid(unsafe_code)]

mod archive;
mod uri;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use roxmltree::Document;
use serde::Serialize;

pub use uri::resolve_target;

const TYPES_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub archive_bytes: usize,
    pub parts: usize,
    pub part_bytes: usize,
    pub total_bytes: usize,
    pub xml_nodes: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            archive_bytes: 64 * 1024 * 1024,
            parts: 4096,
            part_bytes: 32 * 1024 * 1024,
            total_bytes: 128 * 1024 * 1024,
            xml_nodes: 200_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}
impl Error {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            part: None,
        }
    }
    pub(crate) fn at(mut self, part: &str) -> Self {
        self.part = Some(part.into());
        self
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(part) = &self.part {
            write!(f, " [part={part}]")?;
        }
        Ok(())
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relationship {
    pub id: String,
    pub relationship_type: String,
    /// Absolute package part name for internal targets; unchanged URI for external targets.
    pub target: String,
    pub external: bool,
}

#[derive(Debug)]
pub struct Part {
    name: String,
    content_type: String,
    bytes: Vec<u8>,
}
impl Part {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn content_type(&self) -> &str {
        &self.content_type
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug)]
pub struct Package {
    parts: BTreeMap<String, Part>,
    relationships: BTreeMap<String, Vec<Relationship>>,
    limits: Limits,
}
impl Package {
    /// Read a bounded package without extracting files or following external links.
    ///
    /// # Errors
    /// Rejects malformed ZIP/XML, ambiguous names, dangling relationships and limits.
    pub fn read(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
        let mut raw = archive::read(bytes, limits)?;
        let types = raw
            .remove("/[Content_Types].xml")
            .ok_or_else(|| Error::new("content_types", "missing [Content_Types].xml"))?;
        let (defaults, overrides) = content_types(&types, limits)?;
        let mut parts = BTreeMap::new();
        for (name, bytes) in raw {
            let extension = name.rsplit_once('.').map_or("", |(_, e)| e);
            let content_type = overrides
                .get(&name.to_ascii_lowercase())
                .or_else(|| defaults.get(&extension.to_ascii_lowercase()))
                .ok_or_else(|| {
                    Error::new("content_type", "part has no declared content type").at(&name)
                })?;
            parts.insert(
                name.to_ascii_lowercase(),
                Part {
                    name,
                    content_type: content_type.clone(),
                    bytes,
                },
            );
        }
        for name in overrides.keys() {
            if !parts.contains_key(name) {
                return Err(
                    Error::new("content_type", "override refers to a missing part").at(name),
                );
            }
        }
        let mut package = Self {
            parts,
            relationships: BTreeMap::new(),
            limits,
        };
        package.read_relationships()?;
        Ok(package)
    }

    pub fn parts(&self) -> impl Iterator<Item = &Part> {
        self.parts.values()
    }
    #[must_use]
    pub fn part(&self, name: &str) -> Option<&Part> {
        self.parts.get(&name.to_ascii_lowercase())
    }
    #[must_use]
    pub fn relationships(&self, source: &str) -> &[Relationship] {
        self.relationships
            .get(&source.to_ascii_lowercase())
            .map_or(&[], Vec::as_slice)
    }
    /// Parse a UTF-8 XML part with entity resolution disabled and a node bound.
    ///
    /// # Errors
    /// Returns missing-part, encoding or XML diagnostics.
    pub fn xml(&self, name: &str) -> Result<Document<'_>, Error> {
        let part = self
            .part(name)
            .ok_or_else(|| Error::new("missing_part", "part does not exist").at(name))?;
        parse_xml(&part.bytes, self.limits).map_err(|error| error.at(name))
    }

    fn read_relationships(&mut self) -> Result<(), Error> {
        for part in self.parts.values() {
            let Some(source) = uri::relationship_source(&part.name)? else {
                continue;
            };
            if part.content_type != "application/vnd.openxmlformats-package.relationships+xml" {
                return Err(Error::new(
                    "relationship_type",
                    "incorrect relationships content type",
                )
                .at(&part.name));
            }
            if source != "/" && self.part(&source).is_none() {
                return Err(
                    Error::new("relationship_source", "relationship source is missing")
                        .at(&part.name),
                );
            }
            let doc = self.xml(&part.name)?;
            if !doc.root_element().has_tag_name((RELS_NS, "Relationships")) {
                return Err(
                    Error::new("relationships", "invalid relationships root or namespace")
                        .at(&part.name),
                );
            }
            let mut ids = BTreeSet::new();
            let mut relations = Vec::new();
            for node in doc
                .root_element()
                .children()
                .filter(roxmltree::Node::is_element)
            {
                if !node.has_tag_name((RELS_NS, "Relationship")) {
                    return Err(
                        Error::new("relationships", "unknown relationship element").at(&part.name)
                    );
                }
                let id = required(node, "Id")?;
                if !ids.insert(id.to_owned()) {
                    return Err(
                        Error::new("duplicate_relationship", "duplicate relationship ID")
                            .at(&part.name),
                    );
                }
                let kind = required(node, "Type")?;
                let target = required(node, "Target")?;
                let external = match node.attribute("TargetMode").unwrap_or("Internal") {
                    "Internal" => false,
                    "External" => true,
                    _ => {
                        return Err(
                            Error::new("target_mode", "invalid relationship target mode")
                                .at(&part.name),
                        );
                    }
                };
                let target = if external {
                    target.to_owned()
                } else {
                    resolve_target(&source, target)?
                };
                if !external && self.part(&target).is_none() {
                    return Err(Error::new(
                        "missing_target",
                        "internal relationship target is missing",
                    )
                    .at(&target));
                }
                relations.push(Relationship {
                    id: id.into(),
                    relationship_type: kind.into(),
                    target,
                    external,
                });
            }
            self.relationships
                .insert(source.to_ascii_lowercase(), relations);
        }
        Ok(())
    }
}

/// Parse XML without DTDs, external entities or unbounded node allocation.
///
/// # Errors
/// Rejects invalid UTF-8/XML or the configured limits.
pub fn parse_xml(bytes: &[u8], limits: Limits) -> Result<Document<'_>, Error> {
    if bytes.len() > limits.part_bytes {
        return Err(Error::new("xml_limit", "XML part exceeds byte limit"));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| Error::new("xml_encoding", "XML must be UTF-8"))?;
    if text.contains("<!DOCTYPE") {
        return Err(Error::new("xml_dtd", "DTD declarations are unsupported"));
    }
    Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: limits.xml_nodes,
            ..Default::default()
        },
    )
    .map_err(|error| Error::new("xml", error.to_string()))
}

fn required<'a>(node: roxmltree::Node<'a, '_>, name: &str) -> Result<&'a str, Error> {
    node.attribute(name)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::new("xml_attribute", format!("missing {name} attribute")))
}

type ContentTypes = (BTreeMap<String, String>, BTreeMap<String, String>);
fn content_types(bytes: &[u8], limits: Limits) -> Result<ContentTypes, Error> {
    let doc = parse_xml(bytes, limits)?;
    if !doc.root_element().has_tag_name((TYPES_NS, "Types")) {
        return Err(Error::new(
            "content_types",
            "invalid content types root or namespace",
        ));
    }
    let (mut defaults, mut overrides) = (BTreeMap::new(), BTreeMap::new());
    for node in doc
        .root_element()
        .children()
        .filter(roxmltree::Node::is_element)
    {
        let content_type = required(node, "ContentType")?;
        let previous = if node.has_tag_name((TYPES_NS, "Default")) {
            defaults.insert(
                required(node, "Extension")?.to_ascii_lowercase(),
                content_type.into(),
            )
        } else if node.has_tag_name((TYPES_NS, "Override")) {
            let name = required(node, "PartName")?;
            uri::validate_part_name(name)?;
            overrides.insert(name.to_ascii_lowercase(), content_type.into())
        } else {
            return Err(Error::new("content_types", "unknown content type element"));
        };
        if previous.is_some() {
            return Err(Error::new(
                "content_types",
                "duplicate content type declaration",
            ));
        }
    }
    Ok((defaults, overrides))
}

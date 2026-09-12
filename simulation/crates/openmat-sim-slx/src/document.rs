use std::collections::{BTreeMap, BTreeSet};

use openmat_opc::Package;
use roxmltree::Node;
use serde::Serialize;

use crate::Issue;

pub type Properties = BTreeMap<String, String>;
const MODEL_REL: &str = "http://schemas.mathworks.com/simulink/2010/relationships/blockDiagram";
const SYSTEM_REL: &str = "http://schemas.mathworks.com/simulink/2010/relationships/system";
const CORE_REL: &str = "http://schemas.mathworks.com/package/2012/relationships/coreProperties";

#[derive(Clone, Debug, Serialize)]
pub struct Source {
    pub part: String,
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub schema_version: u32,
    pub name: String,
    pub matlab_release: Option<String>,
    pub kind: String,
    pub properties: Properties,
    pub model_elements: Vec<String>,
    pub configuration: Properties,
    pub configuration_objects: Vec<crate::configuration::ConfigurationObject>,
    pub systems: Vec<System>,
    pub parts: Vec<PartInfo>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartInfo {
    pub name: String,
    pub content_type: String,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct System {
    pub source: Source,
    pub parent_block: Option<String>,
    pub properties: Properties,
    pub blocks: Vec<Block>,
    pub lines: Vec<Line>,
    pub extra_elements: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub sid: String,
    pub name: String,
    pub block_type: String,
    pub attributes: Properties,
    pub properties: Properties,
    pub port_counts: Properties,
    pub port_properties: Vec<PortProperties>,
    pub extra_elements: Vec<String>,
    pub source: Source,
}
#[derive(Clone, Debug, Serialize)]
pub struct Line {
    pub properties: Properties,
    pub branches: Vec<Line>,
    pub extra_elements: Vec<String>,
    pub source: Source,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortProperties {
    pub attributes: Properties,
    pub properties: Properties,
    pub extra_elements: Vec<String>,
    pub source: Source,
}

pub(crate) fn location(node: Node<'_, '_>, part: &str) -> Source {
    let range = node.range();
    Source {
        part: part.into(),
        start: range.start,
        end: range.end,
    }
}
pub(crate) fn properties(node: Node<'_, '_>) -> Result<Properties, Issue> {
    let mut values = Properties::new();
    for child in node.children().filter(|n| n.has_tag_name("P")) {
        let key = attribute(child, "Name")?;
        if values
            .insert(key.into(), child.text().unwrap_or("").into())
            .is_some()
        {
            return Err(Issue::new(
                "duplicate_property",
                format!("duplicate property {key}"),
            ));
        }
    }
    Ok(values)
}
pub(crate) fn attribute<'a>(node: Node<'a, '_>, name: &str) -> Result<&'a str, Issue> {
    node.attribute(name)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Issue::new("slx_attribute", format!("missing {name}")))
}
pub(crate) fn related<'a>(
    package: &'a Package,
    source: &str,
    kind: &str,
) -> Result<&'a str, Issue> {
    let relations: Vec<_> = package
        .relationships(source)
        .iter()
        .filter(|r| r.relationship_type == kind)
        .collect();
    if relations.len() != 1 || relations[0].external {
        return Err(Issue::new(
            "slx_relationship",
            format!("expected one internal {kind} relationship"),
        ));
    }
    Ok(&relations[0].target)
}

pub(crate) fn read(package: &Package, name: &str) -> Result<Document, Issue> {
    let main = related(package, "/", MODEL_REL)?;
    let xml = package.xml(main)?;
    if !xml.root_element().has_tag_name("ModelInformation") {
        return Err(Issue::new("slx_root", "expected ModelInformation"));
    }
    let models: Vec<_> = xml
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("Model") || n.has_tag_name("Library"))
        .collect();
    if models.len() != 1 {
        return Err(Issue::new("slx_root", "expected one Model or Library"));
    }
    let model = models[0];
    let roots: Vec<_> = model
        .children()
        .filter(|n| n.has_tag_name("System"))
        .collect();
    if roots.len() != 1 {
        return Err(Issue::new("slx_system", "expected one root System"));
    }
    let mut collector = Collector {
        package,
        systems: Vec::new(),
        used_parts: BTreeSet::new(),
        ids: BTreeSet::new(),
    };
    collector.system(roots[0], main, None, 0)?;
    let matlab_release = package
        .relationships("/")
        .iter()
        .find(|r| r.relationship_type == CORE_REL && !r.external)
        .map(|r| package.xml(&r.target))
        .transpose()?
        .and_then(|doc| {
            doc.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "matlabRelease")
                .and_then(|n| n.text())
                .map(str::to_owned)
        });
    let (configuration, configuration_objects) = crate::configuration::read(package)?;
    Ok(Document {
        schema_version: 1,
        name: name.into(),
        matlab_release,
        kind: model.tag_name().name().into(),
        properties: properties(model)?,
        configuration,
        configuration_objects,
        model_elements: model
            .children()
            .filter(|n| n.is_element() && !n.has_tag_name("P") && !n.has_tag_name("System"))
            .map(|n| n.tag_name().name().into())
            .collect(),
        systems: collector.systems,
        parts: package
            .parts()
            .map(|p| PartInfo {
                name: p.name().into(),
                content_type: p.content_type().into(),
                bytes: p.bytes().len(),
            })
            .collect(),
    })
}

struct Collector<'a> {
    package: &'a Package,
    systems: Vec<System>,
    used_parts: BTreeSet<String>,
    ids: BTreeSet<String>,
}
impl Collector<'_> {
    fn system(
        &mut self,
        node: Node<'_, '_>,
        part: &str,
        parent: Option<String>,
        depth: usize,
    ) -> Result<(), Issue> {
        if depth > 32 || self.systems.len() >= 1024 {
            return Err(Issue::new("slx_limit", "system hierarchy exceeds limit"));
        }
        if let Some(reference) = node.attribute("Ref") {
            if node.children().any(|n| n.is_element())
                || node.attributes().any(|a| a.name() != "Ref")
            {
                return Err(Issue::new(
                    "slx_system",
                    "System Ref cannot also define inline content",
                ));
            }
            let relation = self
                .package
                .relationships(part)
                .iter()
                .find(|r| r.id == reference && r.relationship_type == SYSTEM_REL && !r.external)
                .ok_or_else(|| Issue::new("slx_system", "unresolved System Ref"))?;
            if !self.used_parts.insert(relation.target.to_ascii_lowercase()) {
                return Err(Issue::new("slx_system", "cyclic or reused system part"));
            }
            let xml = self.package.xml(&relation.target)?;
            if !xml.root_element().has_tag_name("System") {
                return Err(Issue::new("slx_system", "referenced part is not a System"));
            }
            return self.system(xml.root_element(), &relation.target, parent, depth + 1);
        }
        let mut system = System {
            source: location(node, part),
            parent_block: parent,
            properties: properties(node)?,
            blocks: Vec::new(),
            lines: Vec::new(),
            extra_elements: Vec::new(),
        };
        let mut nested = Vec::new();
        for child in node.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "P" => {}
                "Block" => {
                    let sid = attribute(child, "SID")?.to_owned();
                    if self.ids.len() >= 10_000 || !self.ids.insert(sid.clone()) {
                        return Err(Issue::new("slx_block", "duplicate SID or block limit"));
                    }
                    let port_counts = child
                        .children()
                        .find(|n| n.has_tag_name("PortCounts"))
                        .map(|n| {
                            n.attributes()
                                .map(|a| (a.name().into(), a.value().into()))
                                .collect()
                        })
                        .unwrap_or_default();
                    system.blocks.push(Block {
                        sid: sid.clone(),
                        name: attribute(child, "Name")?.into(),
                        block_type: attribute(child, "BlockType")?.into(),
                        attributes: child
                            .attributes()
                            .map(|a| (a.name().into(), a.value().into()))
                            .collect(),
                        properties: properties(child)?,
                        port_counts,
                        port_properties: port_properties(child, part)?,
                        source: location(child, part),
                        extra_elements: child
                            .children()
                            .filter(|n| n.is_element() && !n.has_tag_name("P"))
                            .map(|n| n.tag_name().name().into())
                            .collect(),
                    });
                    for sub in child.children().filter(|n| n.has_tag_name("System")) {
                        nested.push((sub, sid.clone()));
                    }
                }
                "Line" => system.lines.push(line(child, part, 0)?),
                other => system.extra_elements.push(other.into()),
            }
        }
        self.systems.push(system);
        for (sub, sid) in nested {
            self.system(sub, part, Some(sid), depth + 1)?;
        }
        Ok(())
    }
}

fn line(node: Node<'_, '_>, part: &str, depth: usize) -> Result<Line, Issue> {
    if depth > 64 {
        return Err(Issue::new("slx_limit", "line branch depth exceeds limit"));
    }
    Ok(Line {
        properties: properties(node)?,
        source: location(node, part),
        extra_elements: node
            .children()
            .filter(|n| n.is_element() && !n.has_tag_name("P") && !n.has_tag_name("Branch"))
            .map(|n| n.tag_name().name().into())
            .collect(),
        branches: node
            .children()
            .filter(|n| n.has_tag_name("Branch"))
            .map(|n| line(n, part, depth + 1))
            .collect::<Result<_, _>>()?,
    })
}

fn port_properties(node: Node<'_, '_>, part: &str) -> Result<Vec<PortProperties>, Issue> {
    node.children()
        .filter(|n| n.has_tag_name("PortProperties"))
        .flat_map(|n| n.children().filter(Node::is_element))
        .map(|n| {
            if !n.has_tag_name("Port") {
                return Err(Issue::new(
                    "port_properties",
                    "unexpected port properties element",
                ));
            }
            Ok(PortProperties {
                attributes: n
                    .attributes()
                    .map(|a| (a.name().into(), a.value().into()))
                    .collect(),
                properties: properties(n)?,
                extra_elements: n
                    .children()
                    .filter(|c| c.is_element() && !c.has_tag_name("P"))
                    .map(|c| c.tag_name().name().into())
                    .collect(),
                source: location(n, part),
            })
        })
        .collect()
}

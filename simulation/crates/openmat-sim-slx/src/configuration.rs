use openmat_opc::Package;
use serde::Serialize;

use crate::document::{attribute, location, properties, related};
use crate::{Issue, Properties, Source};

const INFO_REL: &str = "http://schemas.mathworks.com/simulink/2014/relationships/configSetInfo";
const SET_REL: &str = "http://schemas.mathworks.com/simulink/2014/relationships/configSet";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationObject {
    pub class_name: String,
    pub properties: Properties,
    pub source: Source,
}

// Select execution properties from their owning configuration component. Other
// objects can legitimately reuse names (notably Name and Description).
pub(crate) fn execution_owner(name: &str) -> Option<&'static str> {
    match name {
        "SolverName"
        | "StartTime"
        | "StopTime"
        | "FixedStep"
        | "EnableMultiTasking"
        | "ConcurrentTasks"
        | "AutoInsertRateTranBlk"
        | "EnableFixedStepZeroCrossing"
        | "DecoupledContinuousIntegration"
        | "MinimalZcImpactIntegration"
        | "SampleTimeConstraint"
        | "SampleTimeProperty"
        | "AllowMultiTaskInputOutput" => Some("Simulink.SolverCC"),
        "LoadExternalInput" | "LoadInitialState" => Some("Simulink.DataIOCC"),
        "DefaultUnderspecifiedDataType" | "DenormalBehavior" => Some("Simulink.OptimizationCC"),
        "UseModelRefSolver" => Some("Simulink.ModelReferenceCC"),
        _ => None,
    }
}

pub(crate) fn read(package: &Package) -> Result<(Properties, Vec<ConfigurationObject>), Issue> {
    let info_name = related(package, "/", INFO_REL)?;
    let info = package.xml(info_name)?;
    if !info.root_element().has_tag_name("ConfigSetInfo") {
        return Err(Issue::new("slx_configuration", "expected ConfigSetInfo"));
    }
    let active: Vec<_> = info
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("ConfigSet") && n.attribute("Active") == Some("true"))
        .collect();
    if active.len() != 1 {
        return Err(Issue::new(
            "slx_configuration",
            "expected one active configuration",
        ));
    }
    let part = attribute(active[0], "PartName")?;
    if !package.relationships(info_name).iter().any(|r| {
        !r.external && r.target.eq_ignore_ascii_case(part) && r.relationship_type == SET_REL
    }) {
        return Err(Issue::new(
            "slx_configuration",
            "active configuration has no matching relationship",
        ));
    }
    let xml = package.xml(part)?;
    if !xml.root_element().has_tag_name("ConfigSet") {
        return Err(Issue::new("slx_configuration", "expected ConfigSet"));
    }
    let mut values = Properties::new();
    let mut objects = Vec::new();
    for node in xml.descendants().filter(|n| n.has_tag_name("Object")) {
        let class_name = attribute(node, "ClassName")?;
        let props = properties(node)?;
        for (name, value) in &props {
            if execution_owner(name) == Some(class_name)
                && values.insert(name.clone(), value.clone()).is_some()
            {
                return Err(Issue::new(
                    "slx_configuration",
                    format!("ambiguous execution property {name}"),
                ));
            }
        }
        objects.push(ConfigurationObject {
            class_name: class_name.into(),
            properties: props,
            source: location(node, part),
        });
    }
    Ok((values, objects))
}

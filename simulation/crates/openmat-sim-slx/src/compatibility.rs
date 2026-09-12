use openmat_opc::Package;

use crate::{Document, Issue};

pub(crate) fn global_checks(document: &Document, package: &Package) -> Vec<Issue> {
    let mut issues = Vec::new();
    if document.matlab_release.as_deref() != Some("R2022b") {
        issues.push(Issue::new(
            "slx_release",
            "execution is currently validated only for R2022b SLX models",
        ));
    }
    if document.kind != "Model" {
        issues.push(Issue::new(
            "slx_kind",
            "libraries cannot be simulated directly",
        ));
    }
    if document.systems.len() != 1 {
        issues.push(Issue::new(
            "subsystem",
            "subsystem structure is retained but hierarchical execution is not supported in v0",
        ));
    }
    for (name, text) in &document.properties {
        if (name.ends_with("Fcn") || matches!(name.as_str(), "DataDictionary" | "ExternalSources"))
            && !text.trim().is_empty()
        {
            issues.push(Issue::new(
                "model_dependency",
                format!("{name} requires model initialization or dependency resolution"),
            ));
        }
    }
    if document.model_elements.iter().any(|e| {
        ![
            "ConfigManagerSettings",
            "SimulationSettings",
            "EngineSettings",
            "ConcurrentExecutionSettings",
        ]
        .contains(&e.as_str())
    }) {
        issues.push(Issue::new(
            "model_feature",
            "model contains unsupported workspace or execution elements",
        ));
    }
    for system in &document.systems {
        if system.extra_elements.iter().any(|e| e != "Annotation") {
            issues.push(Issue::new(
                "system_feature",
                "system contains unsupported execution elements",
            ));
        }
    }
    for source in std::iter::once("/").chain(package.parts().map(openmat_opc::Part::name)) {
        if package.relationships(source).iter().any(|r| r.external) {
            issues.push(Issue::new(
                "external_dependency",
                "external package relationships require explicit dependency resolution",
            ));
        }
    }
    check_package_xml(package, &mut issues);
    check_configuration(document, &mut issues);
    issues
}

fn check_configuration(document: &Document, issues: &mut Vec<Issue>) {
    for object in &document.configuration_objects {
        if ![
            "Simulink.ConfigSet",
            "Simulink.SolverCC",
            "Simulink.DataIOCC",
            "Simulink.OptimizationCC",
            "Simulink.DebuggingCC",
            "Simulink.HardwareCC",
            "Simulink.ModelReferenceCC",
            "Simulink.SFSimCC",
            "Simulink.RTWCC",
            "Simulink.CodeAppCC",
            "Simulink.GRTTargetCC",
            "SlCovCC.ConfigComp",
            "hdlcoderui.hdlcc",
        ]
        .contains(&object.class_name.as_str())
        {
            issues.push(Issue::new(
                "configuration_class",
                format!("unrecognized configuration component {}", object.class_name),
            ));
        }
        if object.class_name != "Simulink.SolverCC" {
            continue;
        }
        for key in object.properties.keys() {
            // These are inactive settings for other solvers or descriptive UI
            // metadata in the fixed-step, no-event execution profile.
            let inactive = [
                "Description",
                "Name",
                "DisabledProps",
                "Components",
                "AbsTol",
                "AutoScaleAbsTol",
                "InitialStep",
                "MaxOrder",
                "ZcThreshold",
                "ConsecutiveZCsStepRelTol",
                "MaxConsecutiveZCs",
                "ExtrapolationOrder",
                "NumberNewtonIterations",
                "MaxStep",
                "MinStep",
                "MaxConsecutiveMinStep",
                "RelTol",
                "SolverJacobianMethodControl",
                "DaesscMode",
                "ShapePreserveControl",
                "ZeroCrossControl",
                "ZeroCrossAlgorithm",
                "AlgebraicLoopSolver",
                "SolverInfoToggleStatus",
                "IsAutoAppliedInSIP",
                "SolverResetMethod",
                "PositivePriorityOrder",
                "InsertRTBMode",
                "ODENIntegrationMethod",
                "MaxZcPerStep",
                "MaxZcBracketingIterations",
            ]
            .contains(&key.as_str());
            if !inactive && crate::configuration::execution_owner(key) != Some("Simulink.SolverCC")
            {
                let mut issue = Issue::new(
                    "configuration_parameter",
                    format!("unrecognized solver setting {key}"),
                );
                issue.parameter = Some(key.clone());
                issue.part = Some(object.source.part.clone());
                issues.push(issue);
            }
        }
    }
}

fn check_package_xml(package: &Package, issues: &mut Vec<Issue>) {
    for part in package.parts().filter(|p| p.content_type().contains("xml")) {
        let xml = match package.xml(part.name()) {
            Ok(xml) => xml,
            Err(error) => {
                issues.push(error.into());
                continue;
            }
        };
        for node in xml.descendants().filter(roxmltree::Node::is_element) {
            if node.has_tag_name("BlockParameterDefaults")
                || node.has_tag_name("ModelWorkspace")
                || (node.has_tag_name("P")
                    && node.parent().is_some_and(|p| {
                        p.has_tag_name("Model")
                            || p.has_tag_name("Block")
                            || p.has_tag_name("System")
                    })
                    && node.attribute("Name").is_some_and(|n| {
                        n.ends_with("Fcn") && !node.text().unwrap_or("").trim().is_empty()
                    }))
            {
                issues.push(Issue::new(
                    "initialization",
                    "custom defaults, workspaces and callbacks require an initialization stage",
                ));
                break;
            }
            if node.has_tag_name("P")
                && node.attribute("Name") == Some("SimulationMode")
                && node.text() != Some("normal")
            {
                issues.push(Issue::new(
                    "simulation_mode",
                    "only normal simulation mode is supported",
                ));
            }
            if node.has_tag_name("ModelInformation") && node.attribute("Version") != Some("1.0") {
                issues.push(Issue::new(
                    "model_version",
                    "unsupported ModelInformation version",
                ));
            }
            let config_property = node
                .attribute("Name")
                .and_then(crate::configuration::execution_owner)
                .is_some_and(|owner| {
                    node.parent()
                        .is_some_and(|p| p.attribute("ClassName") == Some(owner))
                });
            if node.has_tag_name("P")
                && node.parent().is_some_and(|p| {
                    p.has_tag_name("Block") || p.has_tag_name("Port") || config_property
                })
                && (node.children().any(|c| c.is_element())
                    || node.attributes().any(|a| {
                        a.name() != "Name"
                            && !(config_property && a.name() == "Class" && a.value() == "double")
                    }))
            {
                let mut issue = Issue::new(
                    "property_encoding",
                    "structured or referenced execution properties are unsupported",
                );
                issue.part = Some(part.name().into());
                issue.parameter = node.attribute("Name").map(str::to_owned);
                issues.push(issue);
            }
        }
        if xml.root_element().has_tag_name("BlockDiagramDefaults")
            && xml
                .root_element()
                .children()
                .filter(roxmltree::Node::is_element)
                .any(|n| !n.has_tag_name("MaskDefaults"))
        {
            issues.push(Issue::new(
                "custom_defaults",
                "custom block defaults require release-specific default resolution",
            ));
        }
    }
    for relation in package.relationships("/").iter().filter(|r| {
        r.relationship_type
            == "http://schemas.mathworks.com/simulinkModel/2016/relationships/modelDictionary"
            && !r.external
    }) {
        if let Ok(xml) = package.xml(&relation.target)
            && xml
                .descendants()
                .filter(roxmltree::Node::is_element)
                .any(|n| !["MF0", "System", "Interface"].contains(&n.tag_name().name()))
        {
            issues.push(Issue::new(
                "model_dictionary",
                "nonempty embedded model dictionaries are unsupported",
            ));
        }
    }
}

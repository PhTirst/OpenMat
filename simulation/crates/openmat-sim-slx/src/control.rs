//! Explicit control-v1 importer; the original `lower()` profile remains unchanged.
use crate::{
    ImportedSlx, Issue, Parameters, compatibility,
    control_blocks::{self, Kind},
    control_emit, control_graph, lower,
};
use openmat_sim::{SourceBundle, model::Model};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlModel {
    pub model: Model,
    pub sources: SourceBundle,
    pub parameters: Parameters,
    pub block_paths: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<openmat_sim::sampling::SamplingPlan>,
}

impl ImportedSlx {
    /// Resolve explicit constant parameters and lower ordinary hierarchical control models.
    /// # Errors
    /// Reports unsupported semantics, dependencies, dimensions and numeric compilation failures.
    pub fn lower_control(&self, parameter_text: &str) -> Result<ControlModel, Vec<Issue>> {
        let result = self.control_inner(parameter_text);
        result.map_err(|issues| {
            let paths = self.block_paths();
            issues
                .into_iter()
                .map(|mut issue| {
                    if let Some(sid) = &issue.block
                        && let Some(path) = paths.get(sid)
                    {
                        issue.message = format!("{path}: {}", issue.message);
                    }
                    issue
                })
                .collect()
        })
    }

    fn control_inner(&self, parameter_text: &str) -> Result<ControlModel, Vec<Issue>> {
        let mut issues = compatibility::global_checks(&self.document, &self.package)
            .into_iter()
            .filter(|i| i.code != "subsystem")
            .collect::<Vec<_>>();
        let parameters = Parameters::from_script(parameter_text).map_err(|e| vec![e])?;
        let mut props = self.document.configuration.clone();
        let mut settings_resolved = true;
        for (key, default) in [
            ("FixedStep", "auto"),
            ("StartTime", "0"),
            ("StopTime", "10"),
        ] {
            let array = parameters
                .evaluate(lower::value(&props, key, default))
                .and_then(|a| a.scalar());
            match array {
                Ok(v) => {
                    props.insert(key.into(), v.to_string());
                }
                Err(mut e) => {
                    settings_resolved = false;
                    e.parameter = Some(key.into());
                    issues.push(e);
                }
            }
        }
        let settings = if settings_resolved {
            lower::settings(&props).map_err(|e| issues.push(e)).ok()
        } else {
            None
        };
        let nodes = control_blocks::prepare(&self.document, &parameters)
            .map_err(|e| issues.extend(e))
            .ok();
        if !issues.is_empty() {
            return Err(issues);
        }
        let mut settings = settings.expect("validated settings");
        let nodes = nodes.expect("validated blocks");
        let mut sample = None;
        for node in &nodes {
            if node.source.block_type == "UnitDelay" {
                let period = parameters
                    .evaluate(lower::value(&node.source.properties, "SampleTime", "-1"))
                    .and_then(|a| a.scalar())
                    .map_err(|e| vec![e.at(node.source, Some("SampleTime"))])?;
                let ratio = period / settings.max_step;
                if period <= 0.0
                    || !ratio.is_finite()
                    || ratio < 1.0
                    || (ratio - ratio.round()).abs() > 1e-9
                    || sample.is_some_and(|old: f64| old.to_bits() != period.to_bits())
                {
                    return Err(vec![Issue::new("sample_time", "UnitDelay requires one explicit shared positive period on the FixedStep grid").at(node.source,Some("SampleTime"))]);
                }
                sample = Some(period);
            }
            if lower::value(&props, "SolverName", "") == "FixedStepDiscrete"
                && (matches!(&node.kind, Kind::StateSpace { initial, .. } if !initial.is_empty())
                    || node.source.block_type == "Integrator")
            {
                return Err(vec![
                    Issue::new(
                        "solver",
                        "FixedStepDiscrete cannot execute continuous states",
                    )
                    .at(node.source, None),
                ]);
            }
        }
        settings.sample_time = sample;
        let edges = control_graph::connections(&self.document, &nodes).map_err(|e| vec![e])?;
        let widths = control_graph::widths(&nodes, &edges).map_err(|e| vec![e])?;
        let (model, sources) =
            control_emit::emit(&self.document.name, settings, &nodes, &edges, &widths)
                .map_err(|e| vec![e])?;
        if let Err(error) = openmat_sim::compile_with_sources(&model, &sources) {
            let mut issue = Issue::new(&error.0.code, error.0.message);
            issue.parameter = error.0.port;
            if let Some(block) = nodes
                .iter()
                .find(|n| lower::runtime_id(&n.source.sid).ok() == error.0.block)
            {
                issue = issue.at(block.source, None);
            }
            return Err(vec![issue]);
        }
        Ok(ControlModel {
            model,
            sources,
            parameters,
            block_paths: self.block_paths(),
            sampling: None,
        })
    }

    /// Original SID to display path. These are model identities, never filesystem paths.
    #[must_use]
    pub fn block_paths(&self) -> BTreeMap<String, String> {
        let names: BTreeMap<_, _> = self
            .document
            .systems
            .iter()
            .flat_map(|s| {
                s.blocks
                    .iter()
                    .map(|b| (b.sid.as_str(), (b.name.as_str(), s.parent_block.as_deref())))
            })
            .collect();
        names
            .iter()
            .map(|(&sid, _)| {
                let mut parts = Vec::new();
                let mut next = Some(sid);
                while let Some(id) = next {
                    let Some(&(name, parent)) = names.get(id) else {
                        break;
                    };
                    parts.push(name.replace('/', "//"));
                    next = parent;
                }
                parts.push(self.document.name.clone());
                parts.reverse();
                (sid.into(), parts.join("/"))
            })
            .collect()
    }
}

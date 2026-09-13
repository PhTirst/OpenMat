//! Versioned multirate lowering; normalization never mutates the source document.
mod normalize;
use crate::{
    ControlModel, ImportedSlx, Issue, Parameters, compatibility, control_blocks, control_emit,
    control_graph, lower,
};
use openmat_sim::{
    component::Callback,
    model::{BlockKind, SampleTime},
};
use std::collections::BTreeMap;

impl ImportedSlx {
    /// Lower a bounded synchronous R2022b model with explicit parameter assignments.
    /// # Errors
    /// Reports unsupported rates, block configurations, dependencies and numerical graphs.
    pub fn lower_multirate(&self, parameter_text: &str) -> Result<ControlModel, Vec<Issue>> {
        self.multirate_inner(parameter_text).map_err(|issues| {
            let paths = self.block_paths();
            issues
                .into_iter()
                .map(|mut issue| {
                    if let Some(path) = issue.block.as_ref().and_then(|sid| paths.get(sid)) {
                        issue.message = format!("{path}: {}", issue.message);
                    }
                    issue
                })
                .collect()
        })
    }

    #[allow(clippy::too_many_lines)]
    fn multirate_inner(&self, parameter_text: &str) -> Result<ControlModel, Vec<Issue>> {
        let mut issues: Vec<_> = compatibility::global_checks(&self.document, &self.package)
            .into_iter()
            .filter(|i| i.code != "subsystem")
            .collect();
        let parameters = Parameters::from_script(parameter_text).map_err(|e| vec![e])?;
        let mut properties = self.document.configuration.clone();
        for (key, default) in [
            ("FixedStep", "auto"),
            ("StartTime", "0"),
            ("StopTime", "10"),
        ] {
            match parameters
                .evaluate(lower::value(&properties, key, default))
                .and_then(|a| a.scalar())
            {
                Ok(value) => {
                    properties.insert(key.into(), value.to_string());
                }
                Err(mut error) => {
                    error.parameter = Some(key.into());
                    issues.push(error);
                }
            }
        }
        if !issues.is_empty() {
            return Err(issues);
        }
        let settings = lower::settings(&properties).map_err(|e| vec![e])?;
        let mut normalized = self.document.clone();
        let mut adaptations = BTreeMap::new();
        for system in &mut normalized.systems {
            for block in &mut system.blocks {
                match normalize::block(block, &parameters) {
                    Ok(adaptation) => {
                        adaptations.insert(
                            lower::runtime_id(&block.sid).map_err(|e| vec![e])?,
                            adaptation,
                        );
                    }
                    Err(error) => issues.push(error.at(block, None)),
                }
            }
        }
        if !issues.is_empty() {
            return Err(issues);
        }
        let nodes = control_blocks::prepare(&normalized, &parameters)?;
        let edges = control_graph::connections(&normalized, &nodes).map_err(|e| vec![e])?;
        let widths = control_graph::widths(&nodes, &edges).map_err(|e| vec![e])?;
        let (mut model, mut sources) =
            control_emit::emit(&self.document.name, settings, &nodes, &edges, &widths)
                .map_err(|e| vec![e])?;
        model.schema_version = 5;
        for block in &mut model.blocks {
            let adapter = &adaptations[&block.id];
            if let Some(rate) = adapter.rate {
                model.sample_times.insert(block.id.clone(), rate);
            }
            match &adapter.kind {
                normalize::AdaptKind::Same => {}
                normalize::AdaptKind::Hold => block.kind = BlockKind::ZeroOrderHold,
                normalize::AdaptKind::Transition {
                    initial,
                    deterministic,
                } => {
                    block.kind = BlockKind::RateTransition {
                        initial: initial.clone(),
                        deterministic: *deterministic,
                    }
                }
                normalize::AdaptKind::Integrator { gain } => {
                    let BlockKind::Integrator { initial } = &block.kind else {
                        unreachable!()
                    };
                    block.kind = BlockKind::DiscreteIntegrator {
                        initial: initial.clone(),
                        gain: *gain,
                    };
                }
                normalize::AdaptKind::StateSpace { initial } => {
                    let BlockKind::Component { component, .. } = &block.kind else {
                        unreachable!()
                    };
                    let definition = model
                        .components
                        .iter_mut()
                        .find(|d| &d.id == component)
                        .expect("generated definition");
                    let count = definition.continuous_states;
                    definition.name.clone_from(&adapter.name);
                    definition.category = "SLX multirate-v1".into();
                    definition.icon = "filter".into();
                    definition.discrete_states = count;
                    definition.continuous_states = 0;
                    definition.sample_time = if count == 0 {
                        None
                    } else {
                        Some(match adapter.rate {
                            Some(SampleTime::Discrete { period }) => period,
                            _ => -1.0,
                        })
                    };
                    for callback in definition.callbacks() {
                        let text = sources
                            .get_mut(&callback.source)
                            .expect("generated callback");
                        *text = text.replace("x(", "q(");
                    }
                    if let Some(derivatives) = definition.derivatives.take() {
                        let source = derivatives.source.replace("_derivatives.m", "_update.m");
                        let entry = "om_update".to_owned();
                        let text = sources
                            .remove(&derivatives.source)
                            .expect("generated derivatives")
                            .replace("om_derivatives", &entry);
                        sources.insert(source.clone(), text);
                        definition.update = Some(Callback { source, entry });
                    }
                    if let Some(initial) = initial
                        && count > 0
                    {
                        let values =
                            control_blocks::expand(initial.clone(), count).map_err(|e| vec![e])?;
                        let callback = definition
                            .initialize
                            .as_ref()
                            .expect("generated initial state");
                        sources.insert(callback.source.clone(), format!(
                            "% OpenMat-authored discrete state initialization.\nfunction y = {}(p)\ny = [{}];\nend\n",
                            callback.entry, values.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")));
                    }
                }
            }
        }
        let plan = openmat_sim::compile_with_sources(&model, &sources).map_err(|e| {
            let mut issue = Issue::new(&e.0.code, e.0.message);
            issue.parameter = e.0.port;
            if let Some(block) = self
                .document
                .systems
                .iter()
                .flat_map(|s| &s.blocks)
                .find(|b| lower::runtime_id(&b.sid).ok() == e.0.block)
            {
                issue = issue.at(block, None);
            }
            vec![issue]
        })?;
        if lower::value(&properties, "SolverName", "") == "FixedStepDiscrete"
            && plan.continuous_state_count() > 0
        {
            return Err(vec![Issue::new(
                "solver",
                "FixedStepDiscrete cannot execute continuous states",
            )]);
        }
        Ok(ControlModel {
            model,
            sources,
            parameters,
            block_paths: self.block_paths(),
            sampling: plan.sampling().cloned(),
        })
    }
}

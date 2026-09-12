use std::collections::BTreeMap;

use openmat_opc::Package;
use openmat_sim::model::{Block as RuntimeBlock, BlockKind, Connection, Model, Port, Settings};

use crate::blocks::translate;
use crate::compatibility::global_checks;
use crate::{Block, Document, Issue, Line, Properties, literal};

pub(crate) fn lower(document: &Document, package: &Package) -> Result<Model, Vec<Issue>> {
    let mut issues = global_checks(document, package);
    let settings = settings(&document.configuration)
        .map_err(|e| issues.push(e))
        .ok();
    let mut blocks = Vec::new();
    let mut periods = Vec::new();
    for system in &document.systems {
        for block in &system.blocks {
            match translate(block) {
                Ok(runtime) => {
                    if matches!(runtime.kind, BlockKind::UnitDelay { .. }) {
                        match literal::scalar(value(&block.properties, "SampleTime", "-1")) {
                            Ok(period) if period > 0.0 => periods.push(period),
                            _ => issues.push(Issue::new("sample_time", "UnitDelay requires an explicit positive scalar SampleTime in SLX v0").at(block, Some("SampleTime"))),
                        }
                    }
                    blocks.push(runtime);
                }
                Err(error) => issues.push(error),
            }
        }
    }
    let source_blocks: BTreeMap<_, _> = document
        .systems
        .iter()
        .flat_map(|s| &s.blocks)
        .map(|b| (b.sid.as_str(), b))
        .collect();
    let mut connections = Vec::new();
    for system in &document.systems {
        for line in &system.lines {
            if let Err(error) = connections_from_line(line, None, &source_blocks, &mut connections)
            {
                issues.push(error);
            }
        }
    }
    let Some(mut settings) = settings else {
        return Err(issues);
    };
    if let Some(&period) = periods.first() {
        if periods.iter().any(|p| p.to_bits() != period.to_bits()) {
            issues.push(Issue::new(
                "multiple_rates",
                "all UnitDelay blocks must share one sample period",
            ));
        }
        let ratio = period / settings.max_step;
        if !ratio.is_finite() || ratio < 1.0 || (ratio - ratio.round()).abs() > 1e-9 {
            issues.push(Issue::new(
                "sample_time",
                "SampleTime must be an integer multiple of FixedStep",
            ));
        }
        settings.sample_time = Some(period);
    }
    if value(&document.configuration, "SolverName", "") == "FixedStepDiscrete"
        && blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Integrator { .. }))
    {
        issues.push(Issue::new(
            "solver",
            "the discrete solver cannot execute Integrator blocks",
        ));
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    if let Err(error) = expand_initial_states(&mut blocks, &connections) {
        return Err(vec![error]);
    }
    let model = Model {
        components: Vec::new(),
        schema_version: 1,
        name: document.name.clone(),
        settings,
        blocks,
        connections,
    };
    if let Err(error) = openmat_sim::compile(&model) {
        let mut issue = Issue {
            code: error.0.code,
            message: error.0.message,
            block: error.0.block,
            parameter: error.0.port,
            part: None,
        };
        if let Some(source) = source_blocks
            .values()
            .find(|b| runtime_id(&b.sid).ok() == issue.block)
        {
            issue = issue.at(source, None);
        }
        return Err(vec![issue]);
    }
    Ok(model)
}

pub(crate) fn settings(props: &Properties) -> Result<Settings, Issue> {
    let solver = value(props, "SolverName", "");
    if !["ode4", "FixedStepDiscrete"].contains(&solver) {
        return Err(Issue::new(
            "solver",
            "SLX v0 supports explicit ode4 or FixedStepDiscrete; the source solver is never silently replaced",
        ));
    }
    for key in [
        "LoadExternalInput",
        "LoadInitialState",
        "EnableMultiTasking",
        "ConcurrentTasks",
        "AutoInsertRateTranBlk",
        "EnableFixedStepZeroCrossing",
        "UseModelRefSolver",
        "DecoupledContinuousIntegration",
        "MinimalZcImpactIntegration",
        "AllowMultiTaskInputOutput",
    ] {
        if value(props, key, "off") != "off" {
            return Err(Issue::new(
                "configuration",
                format!("unsupported configuration {key}"),
            ));
        }
    }
    for (key, default) in [
        ("SampleTimeConstraint", "Unconstrained"),
        ("SampleTimeProperty", "[]"),
        ("DenormalBehavior", "GradualUnderflow"),
    ] {
        if value(props, key, default) != default {
            return Err(Issue::new(
                "configuration",
                format!("unsupported configuration {key}"),
            ));
        }
    }
    if value(props, "DefaultUnderspecifiedDataType", "double") != "double" {
        return Err(Issue::new(
            "data_type",
            "only real double signals are supported",
        ));
    }
    let start_time = literal::scalar(value(props, "StartTime", "0"))?;
    if start_time != 0.0 {
        return Err(Issue::new(
            "start_time",
            "SLX v0 requires StartTime zero to preserve sampling phase",
        ));
    }
    let stop_time = literal::scalar(value(props, "StopTime", "10"))?;
    let max_step = literal::scalar(value(props, "FixedStep", "auto"))?;
    let steps = stop_time / max_step;
    if max_step <= 0.0
        || stop_time < 0.0
        || !steps.is_finite()
        || (steps - steps.round()).abs() > 1e-9
    {
        return Err(Issue::new(
            "stop_time",
            "SLX v0 requires nonnegative StopTime on the positive FixedStep grid",
        ));
    }
    Ok(Settings {
        start_time,
        stop_time,
        max_step,
        sample_time: None,
    })
}

pub(crate) fn value<'a>(props: &'a Properties, key: &str, default: &'a str) -> &'a str {
    props.get(key).map_or(default, |v| v.trim())
}
pub(crate) fn runtime_id(sid: &str) -> Result<String, Issue> {
    if sid.is_empty()
        || sid.len() > 48
        || !sid
            .split(':')
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Issue::new("block_sid", "unsupported block SID syntax"));
    }
    Ok(format!("slx_{}", sid.replace(':', "_")))
}

fn connections_from_line(
    line: &Line,
    inherited: Option<&str>,
    blocks: &BTreeMap<&str, &Block>,
    output: &mut Vec<Connection>,
) -> Result<(), Issue> {
    if !line.extra_elements.is_empty()
        || line
            .properties
            .keys()
            .any(|k| !["Src", "Dst", "Name", "ZOrder", "Points", "Labels"].contains(&k.as_str()))
    {
        return Err(Issue::new(
            "line_feature",
            "line contains unsupported signal properties",
        ));
    }
    if inherited.is_some() && line.properties.contains_key("Src") {
        return Err(Issue::new(
            "line_source",
            "a branch cannot override its inherited source",
        ));
    }
    let source = line
        .properties
        .get("Src")
        .map(String::as_str)
        .or(inherited)
        .ok_or_else(|| Issue::new("line_source", "line branch has no source"))?;
    if let Some(destination) = line.properties.get("Dst") {
        if output.len() >= 640_000 {
            return Err(Issue::new("connection_limit", "too many connections"));
        }
        output.push(Connection {
            from: endpoint(source, true, blocks)?,
            to: endpoint(destination, false, blocks)?,
        });
    } else if line.branches.is_empty() {
        return Err(Issue::new("line_destination", "line has no destination"));
    }
    for branch in &line.branches {
        connections_from_line(branch, Some(source), blocks, output)?;
    }
    Ok(())
}
fn endpoint(text: &str, output: bool, blocks: &BTreeMap<&str, &Block>) -> Result<Port, Issue> {
    let (sid, port) = text
        .split_once('#')
        .ok_or_else(|| Issue::new("endpoint", "invalid line endpoint"))?;
    let block = blocks
        .get(sid)
        .ok_or_else(|| Issue::new("endpoint", "line refers to a missing block SID"))?;
    let (kind, index) = port
        .split_once(':')
        .ok_or_else(|| Issue::new("endpoint", "invalid endpoint port"))?;
    let index = index
        .parse::<usize>()
        .map_err(|_| Issue::new("endpoint", "invalid port number"))?;
    let port = if output && kind == "out" && index == 1 {
        "out".into()
    } else if !output && kind == "in" && index > 0 && index <= 64 {
        if block.block_type == "Sum" {
            format!("in{}", index - 1)
        } else if index == 1 {
            "in".into()
        } else {
            return Err(Issue::new("endpoint", "unsupported input port"));
        }
    } else {
        return Err(Issue::new(
            "endpoint",
            "unsupported port direction or number",
        ));
    };
    Ok(Port {
        block: runtime_id(sid)?,
        port,
    })
}

// Supported blocks preserve width. Unifying those constraints also implements
// Simulink's scalar initial-condition expansion without changing the runtime IR.
fn expand_initial_states(
    blocks: &mut [RuntimeBlock],
    connections: &[Connection],
) -> Result<(), Issue> {
    let ids: BTreeMap<_, _> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.clone(), i))
        .collect();
    let mut parent: Vec<_> = (0..blocks.len()).collect();
    for edge in connections {
        let a = root(&mut parent, ids[&edge.from.block]);
        let b = root(&mut parent, ids[&edge.to.block]);
        parent[a] = b;
    }
    let mut widths = BTreeMap::new();
    for (index, block) in blocks.iter().enumerate() {
        let width = match &block.kind {
            BlockKind::Constant { value } => Some(value.len()),
            BlockKind::Gain { gain } if gain.len() > 1 => Some(gain.len()),
            BlockKind::Integrator { initial } | BlockKind::UnitDelay { initial }
                if initial.len() > 1 =>
            {
                Some(initial.len())
            }
            _ => None,
        };
        if let Some(width) = width
            && widths
                .insert(root(&mut parent, index), width)
                .is_some_and(|old| old != width)
        {
            return Err(Issue::new(
                "signal_width",
                "inconsistent signal widths; implicit Sum broadcasting is not supported in SLX v0",
            ));
        }
    }
    // Bound scalar-to-vector state expansion before allocating its buffers.
    let mut components = 0usize;
    for (index, block) in blocks.iter_mut().enumerate() {
        let width = widths.get(&root(&mut parent, index)).copied().unwrap_or(1);
        if let BlockKind::Integrator { initial } | BlockKind::UnitDelay { initial } =
            &mut block.kind
        {
            components = components.saturating_add(width);
            if components > 262_144 {
                return Err(Issue::new(
                    "state_limit",
                    "expanded states exceed the SLX v0 component limit",
                ));
            }
            if initial.len() == 1 {
                initial.resize(width, initial[0]);
            }
        }
    }
    Ok(())
}
fn root(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        parent[index] = parent[parent[index]];
        index = parent[index];
    }
    index
}

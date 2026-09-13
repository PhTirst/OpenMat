use super::normalize::Adaptation;
use crate::{
    Block, Document, Issue, Line, Parameters, blocks,
    control_blocks::{Kind, Node},
    control_graph::{self, Edge},
    lower::{runtime_id, value},
};
use openmat_sim::{
    conditional::{ConditionalOutput, Execution, HoldReset},
    hybrid::ResetMode,
    model::{Block as RuntimeBlock, BlockKind, Connection, Model, Port, Position, SampleTime},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Domain {
    sid: String,
    source: Option<String>,
    execution: Execution,
}

fn hold_reset(block: &Block, key: &str, default: &str) -> Result<HoldReset, Issue> {
    match value(&block.properties, key, default) {
        "held" => Ok(HoldReset::Held),
        "reset" => Ok(HoldReset::Reset),
        _ => {
            Err(Issue::new("conditional_parameter", "expected held or reset").at(block, Some(key)))
        }
    }
}

pub(super) fn prepare(
    document: &mut Document,
    parameters: &Parameters,
) -> Result<Vec<Domain>, Issue> {
    if value(
        &document.configuration,
        "UnderspecifiedInitializationDetection",
        "Simplified",
    ) != "Simplified"
    {
        let mut issue = Issue::new(
            "conditional_initialization",
            "conditional-v1 requires simplified initialization",
        );
        issue.parameter = Some("UnderspecifiedInitializationDetection".into());
        return Err(issue);
    }
    let mut domains = Vec::new();
    for system in &mut document.systems {
        let controls: Vec<_> = system
            .blocks
            .iter()
            .filter(|b| matches!(b.block_type.as_str(), "EnablePort" | "TriggerPort"))
            .cloned()
            .collect();
        if controls.is_empty() {
            continue;
        }
        if controls.len() != 1 || system.parent_block.is_none() {
            return Err(Issue::new(
                "conditional_control",
                "one EnablePort or TriggerPort is required inside a subsystem",
            )
            .at(&controls[0], None));
        }
        let control = &controls[0];
        let enabled = control.block_type == "EnablePort";
        validate_control(control, enabled)?;
        let mut output_ports = system
            .blocks
            .iter_mut()
            .filter(|b| b.block_type == "Outport")
            .collect::<Vec<_>>();
        let mut outputs = BTreeMap::new();
        for output in &mut output_ports {
            let (number, policy) = read_output(output, parameters, enabled)?;
            if outputs.insert(number, policy).is_some() {
                return Err(Issue::new("subsystem_port", "duplicate output number")
                    .at(output, Some("Port")));
            }
        }
        if outputs.keys().copied().ne(1..=outputs.len()) {
            return Err(
                Issue::new("subsystem_port", "output numbers must be contiguous").at(control, None),
            );
        }
        let outputs = outputs.into_values().collect();
        let execution = if enabled {
            Execution::Enabled {
                period: 1.0,
                states_when_enabling: hold_reset(control, "StatesWhenEnabling", "held")?,
                outputs,
            }
        } else {
            let edge = match value(&control.properties, "TriggerType", "rising") {
                "rising" => ResetMode::Rising,
                "falling" => ResetMode::Falling,
                "either" => ResetMode::Either,
                _ => unreachable!(),
            };
            Execution::Triggered {
                period: 1.0,
                edge,
                outputs,
            }
        };
        domains.push(Domain {
            sid: system.parent_block.clone().expect("parent"),
            source: None,
            execution,
        });
        system.blocks.retain(|b| b.sid != control.sid);
    }
    if domains.len() > 64 {
        return Err(Issue::new(
            "conditional_limit",
            "at most 64 conditional domains are supported",
        ));
    }
    normalize_connections(document, &mut domains)?;
    Ok(domains)
}

fn normalize_connections(document: &mut Document, domains: &mut [Domain]) -> Result<(), Issue> {
    for system in &mut document.systems {
        let local: BTreeSet<_> = system.blocks.iter().map(|b| b.sid.clone()).collect();
        for b in &mut system.blocks {
            if let Some(domain) = domains.iter().find(|d| d.sid == b.sid) {
                if b.port_counts
                    .remove(domain.execution.control_port())
                    .is_some_and(|v| v != "1")
                {
                    return Err(
                        Issue::new("port_count", "expected one conditional control port")
                            .at(b, None),
                    );
                }
                if let Some(atomic) = b.properties.get_mut("TreatAsAtomicUnit") {
                    if !["on", "off"].contains(&atomic.as_str()) {
                        return Err(
                            Issue::new("conditional_parameter", "invalid atomic setting")
                                .at(b, Some("TreatAsAtomicUnit")),
                        );
                    }
                    *atomic = "off".into();
                }
            }
        }
        let mut lines = Vec::new();
        for mut line in std::mem::take(&mut system.lines) {
            if strip_control(&mut line, None, &local, domains)? {
                lines.push(line);
            }
        }
        system.lines = lines;
    }
    if let Some(domain) = domains.iter().find(|d| d.source.is_none()) {
        let mut error = Issue::new("missing_input", "conditional control port is not connected");
        error.block = Some(domain.sid.clone());
        return Err(error);
    }
    Ok(())
}

fn strip_control(
    line: &mut Line,
    inherited: Option<&str>,
    local: &BTreeSet<String>,
    domains: &mut [Domain],
) -> Result<bool, Issue> {
    if !line.properties.contains_key("Dst") && line.branches.is_empty() {
        return Err(Issue::new(
            "line_destination",
            "connection needs a destination",
        ));
    }
    let source = line
        .properties
        .get("Src")
        .map(String::as_str)
        .or(inherited)
        .ok_or_else(|| Issue::new("line_source", "missing source"))?
        .to_owned();
    if inherited.is_some() && line.properties.contains_key("Src")
        || !line.extra_elements.is_empty()
        || line
            .properties
            .keys()
            .any(|k| !["Src", "Dst", "Name", "ZOrder", "Points", "Labels"].contains(&k.as_str()))
    {
        return Err(Issue::new(
            "line_feature",
            "unsupported conditional connection structure",
        ));
    }
    if let Some(dst) = line.properties.get("Dst")
        && let Some(domain) = domains
            .iter_mut()
            .find(|d| *dst == format!("{}#{}", d.sid, d.execution.control_port()))
    {
        if !local.contains(&domain.sid)
            || !source
                .split_once('#')
                .is_some_and(|(sid, _)| local.contains(sid))
            || domain.source.replace(source.clone()).is_some()
        {
            return Err(Issue::new(
                "conditional_connection",
                "conditional control must have one local driver",
            ));
        }
        line.properties.remove("Dst");
    }
    let mut branches = Vec::new();
    for mut branch in std::mem::take(&mut line.branches) {
        if strip_control(&mut branch, Some(&source), local, domains)? {
            branches.push(branch);
        }
    }
    line.branches = branches;
    Ok(line.properties.contains_key("Dst") || !line.branches.is_empty())
}

// Grid ratios are finite integral values in 1..=1e9 before conversion.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn control_rate(
    index: usize,
    nodes: &[Node<'_>],
    edges: &[Edge],
    adapters: &BTreeMap<String, Adaptation>,
    visiting: &mut BTreeSet<usize>,
    depth: usize,
    base: f64,
) -> Result<SampleTime, Issue> {
    if depth > 128 {
        return Err(Issue::new(
            "conditional_rate",
            "control rate dependency depth exceeded",
        ));
    }
    let adapter = &adapters[&runtime_id(&nodes[index].source.sid)?];
    if let Some(rate) = adapter.rate
        && rate != SampleTime::Inherited
    {
        return Ok(rate);
    }
    if !visiting.insert(index) {
        return Ok(SampleTime::Inherited);
    }
    let mut ticks = None;
    let mut continuous = false;
    let mut unresolved = false;
    for e in edges.iter().filter(|e| e.to.block == index) {
        match control_rate(
            e.from.block,
            nodes,
            edges,
            adapters,
            visiting,
            depth + 1,
            base,
        )? {
            SampleTime::Discrete { period } => {
                let ratio = period / base;
                if !ratio.is_finite()
                    || (ratio - ratio.round()).abs() > 1e-9
                    || !(1.0..=1e9).contains(&ratio.round())
                {
                    return Err(Issue::new(
                        "sample_grid",
                        "control period is not on the base grid",
                    ));
                }
                let mut n = ratio.round() as u32;
                if let Some(mut old) = ticks {
                    while n != 0 {
                        let r = old % n;
                        old = n;
                        n = r;
                    }
                    n = old;
                }
                ticks = Some(n);
            }
            SampleTime::Continuous => continuous = true,
            SampleTime::Inherited => unresolved = true,
            SampleTime::Constant => {}
        }
    }
    visiting.remove(&index);
    Ok(if continuous {
        SampleTime::Continuous
    } else if let Some(n) = ticks {
        SampleTime::Discrete {
            period: f64::from(n) * base,
        }
    } else if unresolved {
        SampleTime::Inherited
    } else if matches!(nodes[index].kind, Kind::Ground) || edges.iter().any(|e| e.to.block == index)
    {
        SampleTime::Constant
    } else {
        SampleTime::Continuous
    })
}

pub(super) fn restore(
    document: &Document,
    nodes: &[Node<'_>],
    flat_edges: &[Edge],
    adapters: &BTreeMap<String, Adaptation>,
    domains: &mut [Domain],
    model: &mut Model,
) -> Result<(), Issue> {
    let local_edges = control_graph::local_connections(document, nodes)?;
    resolve_periods(
        document,
        nodes,
        flat_edges,
        &local_edges,
        adapters,
        domains,
        model.settings.max_step,
    )?;
    let parents: BTreeMap<_, _> = document
        .systems
        .iter()
        .flat_map(|s| {
            s.blocks
                .iter()
                .map(|b| (b.sid.as_str(), s.parent_block.as_deref()))
        })
        .collect();
    for node in nodes {
        let id = runtime_id(&node.source.sid)?;
        let parent = parents[node.source.sid.as_str()]
            .map(runtime_id)
            .transpose()?;
        if let Some(block) = model.blocks.iter_mut().find(|b| b.id == id) {
            block.parent = parent;
            continue;
        }
        let kind = match node.kind {
            Kind::Subsystem => BlockKind::Subsystem {
                inputs: node.inputs,
                outputs: node.outputs,
                execution: domains
                    .iter()
                    .find(|d| d.sid == node.source.sid)
                    .map(|d| d.execution.clone()),
            },
            Kind::Inport(port) => BlockKind::Inport { port, data: None },
            Kind::Outport(port) => BlockKind::Outport { port },
            _ => continue,
        };
        let position = node
            .source
            .properties
            .get("Position")
            .map(|p| crate::literal::vector(p))
            .transpose()?
            .and_then(|v| (v.len() == 4).then(|| Position { x: v[0], y: v[1] }));
        model.blocks.push(RuntimeBlock {
            id,
            parent,
            position,
            kind,
        });
    }
    model.connections = local_edges
        .iter()
        .map(|e| {
            Ok(Connection {
                from: Port {
                    block: runtime_id(&nodes[e.from.block].source.sid)?,
                    port: nodes[e.from.block].output_name(e.from.port),
                },
                to: Port {
                    block: runtime_id(&nodes[e.to.block].source.sid)?,
                    port: nodes[e.to.block].input_name(e.to.port),
                },
            })
        })
        .collect::<Result<_, Issue>>()?;
    for domain in domains {
        let source = control_graph::source_pin(domain.source.as_ref().expect("source"), nodes)?;
        model.connections.push(Connection {
            from: Port {
                block: runtime_id(&nodes[source.block].source.sid)?,
                port: nodes[source.block].output_name(source.port),
            },
            to: Port {
                block: runtime_id(&domain.sid)?,
                port: domain.execution.control_port().into(),
            },
        });
    }
    model.schema_version = 8;
    Ok(())
}

fn validate_control(control: &Block, enabled: bool) -> Result<(), Issue> {
    for (key, text) in &control.properties {
        let accepted = match key.as_str() {
            "StatesWhenEnabling" if enabled => ["held", "reset"].contains(&text.as_str()),
            "TriggerType" if !enabled => ["rising", "falling", "either"].contains(&text.as_str()),
            "VariantControl" if !enabled => text == "(inherit)",
            "ShowOutputPort" => text == "off",
            "ZeroCross" => ["on", "off"].contains(&text.as_str()),
            _ => blocks::parameter_supported(control, key, text),
        };
        if !accepted {
            return Err(Issue::new(
                "conditional_parameter",
                "unsupported conditional control setting",
            )
            .at(control, Some(key)));
        }
    }
    if control
        .attributes
        .keys()
        .any(|k| !["BlockType", "Name", "SID"].contains(&k.as_str()))
        || control.extra_elements.iter().any(|e| e != "PortCounts")
        || !control.port_properties.is_empty()
        || control.port_counts.values().any(|v| v != "0")
    {
        return Err(Issue::new(
            "conditional_control",
            "conditional control output/extra execution structures are not supported",
        )
        .at(control, None));
    }
    Ok(())
}

// Output numbers are checked to be integral and in 1..=64 before conversion.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn read_output(
    output: &mut Block,
    parameters: &Parameters,
    enabled: bool,
) -> Result<(usize, ConditionalOutput), Issue> {
    let number = parameters
        .evaluate(value(&output.properties, "Port", "1"))
        .and_then(|p| p.scalar())?;
    if number.fract() != 0.0 || !(1.0..=64.0).contains(&number) {
        return Err(Issue::new("subsystem_port", "invalid output number").at(output, Some("Port")));
    }
    let invalid_initial = || {
        Issue::new("conditional_initial_output", "conditional-v1 requires an explicit finite InitialOutput; inherited [] is not yet supported").at(output, Some("InitialOutput"))
    };
    let initial = parameters
        .evaluate(value(&output.properties, "InitialOutput", "[]"))
        .and_then(|p| p.vector())
        .map_err(|_| invalid_initial())?;
    if initial.is_empty() || initial.iter().any(|v| !v.is_finite()) {
        return Err(invalid_initial());
    }
    let when_disabled = hold_reset(output, "OutputWhenDisabled", "held")?;
    if !enabled && when_disabled != HoldReset::Held {
        return Err(Issue::new(
            "conditional_output",
            "triggered outputs must hold between invocations",
        )
        .at(output, Some("OutputWhenDisabled")));
    }

    output
        .properties
        .insert("InitialOutput".into(), "[]".into());
    output
        .properties
        .insert("OutputWhenDisabled".into(), "held".into());
    Ok((
        number as usize,
        ConditionalOutput {
            initial,
            when_disabled,
        },
    ))
}

fn resolve_periods(
    document: &Document,
    nodes: &[Node<'_>],
    flat_edges: &[Edge],
    local_edges: &[Edge],
    adapters: &BTreeMap<String, Adaptation>,
    domains: &mut [Domain],
    base: f64,
) -> Result<(), Issue> {
    for domain in domains.iter_mut() {
        let source = control_graph::resolved_source(
            document,
            nodes,
            local_edges,
            domain.source.as_ref().expect("control source"),
        )?;
        let rate = control_rate(
            source.block,
            nodes,
            flat_edges,
            adapters,
            &mut BTreeSet::new(),
            0,
            base,
        )?;
        let period = match rate {
            SampleTime::Discrete { period } => period,
            SampleTime::Constant => {
                let system = document
                    .systems
                    .iter()
                    .find(|s| s.parent_block.as_ref() == Some(&domain.sid))
                    .expect("body");
                let rates: BTreeSet<_> = system
                    .blocks
                    .iter()
                    .filter_map(|b| {
                        adapters
                            .get(&runtime_id(&b.sid).ok()?)
                            .and_then(|a| match a.rate {
                                Some(SampleTime::Discrete { period }) => Some(period.to_bits()),
                                _ => None,
                            })
                    })
                    .collect();
                if rates.len() > 1 {
                    return Err(Issue::new(
                        "conditional_rate",
                        "constant enable with multiple body clocks is unsupported",
                    ));
                }
                rates.first().map_or(base, |bits| f64::from_bits(*bits))
            }
            _ => {
                return Err(Issue::new(
                    "conditional_rate",
                    "conditional-v1 requires an explicit or resolvable discrete control signal",
                )
                .at(nodes[source.block].source, Some("SampleTime")));
            }
        };
        match &mut domain.execution {
            Execution::Enabled { period: p, .. } | Execution::Triggered { period: p, .. } => {
                *p = period;
            }
        }
    }
    Ok(())
}

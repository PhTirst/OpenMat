use crate::{
    Document, Issue, Line,
    control_blocks::{Kind, Node},
};
use openmat_sim::model::BlockKind;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Pin {
    pub block: usize,
    pub port: usize,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Edge {
    pub from: Pin,
    pub to: Pin,
}

pub(crate) fn connections(document: &Document, nodes: &[Node<'_>]) -> Result<Vec<Edge>, Issue> {
    let by_sid: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.source.sid.as_str(), i))
        .collect();
    let mut parents = BTreeMap::new();
    let mut outputs = BTreeMap::new();
    let mut drivers = BTreeMap::new();
    for system in &document.systems {
        let local: BTreeSet<_> = system.blocks.iter().map(|b| b.sid.as_str()).collect();
        for block in &system.blocks {
            let index = by_sid[block.sid.as_str()];
            if let Some(parent) = &system.parent_block {
                let parent = by_sid[parent.as_str()];
                parents.insert(index, parent);
                if let Kind::Outport(port) = nodes[index].kind {
                    outputs.insert(
                        Pin {
                            block: parent,
                            port: port - 1,
                        },
                        index,
                    );
                }
            }
        }
        for line in &system.lines {
            collect_line(line, None, &local, &by_sid, nodes, &mut drivers)?;
        }
    }
    for (i, node) in nodes.iter().enumerate() {
        for port in 0..node.inputs {
            if !drivers.contains_key(&Pin { block: i, port }) {
                return Err(
                    Issue::new("missing_input", "required input is not connected")
                        .at(node.source, Some(&format!("in:{}", port + 1))),
                );
            }
        }
    }
    let mut edges = Vec::new();
    for (&to, &from) in &drivers {
        if !nodes[to.block].executable() && !matches!(nodes[to.block].kind, Kind::Terminator) {
            continue;
        }
        let from = resolve(from, nodes, &drivers, &parents, &outputs)?;
        edges.push(Edge { from, to });
    }
    Ok(edges)
}

fn collect_line(
    line: &Line,
    inherited: Option<&str>,
    local: &BTreeSet<&str>,
    ids: &BTreeMap<&str, usize>,
    nodes: &[Node<'_>],
    drivers: &mut BTreeMap<Pin, Pin>,
) -> Result<(), Issue> {
    if !line.extra_elements.is_empty()
        || line
            .properties
            .keys()
            .any(|k| !["Src", "Dst", "Name", "ZOrder", "Points", "Labels"].contains(&k.as_str()))
    {
        return Err(Issue::new(
            "line_feature",
            "line has unsupported execution properties",
        ));
    }
    if inherited.is_some() && line.properties.contains_key("Src") {
        return Err(Issue::new(
            "line_source",
            "a branch cannot override its source",
        ));
    }
    let source = line
        .properties
        .get("Src")
        .map(String::as_str)
        .or(inherited)
        .ok_or_else(|| Issue::new("line_source", "missing line source"))?;
    let from = endpoint(source, true, local, ids, nodes)?;
    if let Some(dst) = line.properties.get("Dst") {
        let to = endpoint(dst, false, local, ids, nodes)?;
        if drivers.len() >= 640_000 {
            return Err(Issue::new("connection_limit", "too many connections"));
        }
        if drivers.insert(to, from).is_some() {
            return Err(
                Issue::new("multiple_drivers", "input has multiple connections")
                    .at(nodes[to.block].source, Some(&format!("in:{}", to.port + 1))),
            );
        }
    } else if line.branches.is_empty() {
        return Err(Issue::new("line_destination", "line has no destination"));
    }
    for branch in &line.branches {
        collect_line(branch, Some(source), local, ids, nodes, drivers)?;
    }
    Ok(())
}
fn endpoint(
    text: &str,
    output: bool,
    local: &BTreeSet<&str>,
    ids: &BTreeMap<&str, usize>,
    nodes: &[Node<'_>],
) -> Result<Pin, Issue> {
    let invalid = || Issue::new("endpoint", "invalid or cross-system line endpoint");
    let (sid, port) = text.split_once('#').ok_or_else(invalid)?;
    if !local.contains(sid) {
        return Err(invalid());
    }
    let block = *ids.get(sid).ok_or_else(invalid)?;
    let (direction, number) = port.split_once(':').ok_or_else(invalid)?;
    let number = number.parse::<usize>().map_err(|_| invalid())?;
    let count = if output {
        nodes[block].outputs
    } else {
        nodes[block].inputs
    };
    if direction != if output { "out" } else { "in" } || number == 0 || number > count {
        return Err(invalid().at(nodes[block].source, Some(port)));
    }
    Ok(Pin {
        block,
        port: number - 1,
    })
}
fn resolve(
    mut pin: Pin,
    nodes: &[Node<'_>],
    drivers: &BTreeMap<Pin, Pin>,
    parents: &BTreeMap<usize, usize>,
    outputs: &BTreeMap<Pin, usize>,
) -> Result<Pin, Issue> {
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(pin) || visited.len() > 128 {
            return Err(Issue::new(
                "subsystem_cycle",
                "cyclic or excessively deep virtual boundary connection",
            )
            .at(nodes[pin.block].source, None));
        }
        let target = match nodes[pin.block].kind {
            Kind::Inport(number) => Pin {
                block: *parents
                    .get(&pin.block)
                    .ok_or_else(|| Issue::new("subsystem_port", "Inport is missing its parent"))?,
                port: number - 1,
            },
            Kind::Subsystem => Pin {
                block: *outputs.get(&pin).ok_or_else(|| {
                    Issue::new("subsystem_port", "subsystem output has no matching Outport")
                })?,
                port: 0,
            },
            _ => return Ok(pin),
        };
        pin = *drivers.get(&target).ok_or_else(|| {
            Issue::new("missing_input", "virtual boundary is not connected")
                .at(nodes[target.block].source, None)
        })?;
    }
}

pub(crate) struct Widths {
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
}
struct Union {
    parent: Vec<usize>,
    sizes: Vec<Option<usize>>,
}
impl Union {
    fn root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }
    fn get(&mut self, i: usize) -> Option<usize> {
        let r = self.root(i);
        self.sizes[r]
    }
    fn set(&mut self, i: usize, width: usize) -> Result<bool, Issue> {
        if !(1..=4096).contains(&width) {
            return Err(Issue::new(
                "signal_width",
                "signal width must be in 1..4096",
            ));
        }
        let r = self.root(i);
        if self.sizes[r].is_some_and(|old| old != width) {
            return Err(Issue::new(
                "signal_width",
                "connected signal width constraints disagree",
            ));
        }
        let changed = self.sizes[r].is_none();
        self.sizes[r] = Some(width);
        Ok(changed)
    }
    fn join(&mut self, a: usize, b: usize) -> Result<(), Issue> {
        let a = self.root(a);
        let b = self.root(b);
        if a == b {
            return Ok(());
        }
        if let Some(width) = self.sizes[a] {
            self.set(b, width)?;
        }
        self.parent[a] = b;
        Ok(())
    }
}

pub(crate) fn widths(nodes: &[Node<'_>], edges: &[Edge]) -> Result<Vec<Widths>, Issue> {
    let mut count = 0;
    let ports: Vec<_> = nodes
        .iter()
        .map(|n| {
            let inputs = (count..count + n.inputs).collect();
            count += n.inputs;
            let outputs = (count..count + n.outputs).collect();
            count += n.outputs;
            Widths { inputs, outputs }
        })
        .collect();
    let mut union = Union {
        parent: (0..count).collect(),
        sizes: vec![None; count],
    };
    for edge in edges {
        union.join(
            ports[edge.from.block].outputs[edge.from.port],
            ports[edge.to.block].inputs[edge.to.port],
        )?;
    }
    for (node, p) in nodes.iter().zip(&ports) {
        constraints(node, p, &mut union).map_err(|e| e.at(node.source, None))?;
    }
    for _ in 0..128 {
        let mut changed = false;
        for (node, p) in nodes.iter().zip(&ports) {
            changed |= propagate(node, p, &mut union).map_err(|e| e.at(node.source, None))?;
        }
        if !changed {
            break;
        }
    }
    // Only unconstrained Ground sources and legacy scalar-state networks have
    // a default scalar size. Routing ambiguity is an explicit diagnostic.
    for (node, p) in nodes.iter().zip(&ports) {
        if matches!(node.kind, Kind::Ground)
            || matches!(&node.kind,Kind::Legacy(b) if matches!(b.kind,BlockKind::Integrator { .. } | BlockKind::UnitDelay { .. }))
        {
            for &i in p.inputs.iter().chain(&p.outputs) {
                if union.get(i).is_none() {
                    union.set(i, 1)?;
                }
            }
        }
    }
    for _ in 0..128 {
        let mut changed = false;
        for (node, p) in nodes.iter().zip(&ports) {
            changed |= propagate(node, p, &mut union).map_err(|e| e.at(node.source, None))?;
        }
        if !changed {
            break;
        }
    }
    let mut total = 0usize;
    nodes
        .iter()
        .zip(ports)
        .map(|(node, p)| {
            if !node.executable() && !matches!(node.kind, Kind::Terminator) {
                return Ok(Widths {
                    inputs: vec![],
                    outputs: vec![],
                });
            }
            let mut resolve = |items: Vec<usize>| -> Result<Vec<usize>, Issue> {
                items
                    .into_iter()
                    .map(|i| {
                        let width = union.get(i).ok_or_else(|| {
                            Issue::new(
                                "signal_width",
                                "signal width is ambiguous; specify routing widths",
                            )
                            .at(node.source, None)
                        })?;
                        total = total.saturating_add(width);
                        if total > 262_144 {
                            return Err(Issue::new(
                                "signal_limit",
                                "total signal width budget exceeded",
                            ));
                        }
                        Ok(width)
                    })
                    .collect()
            };
            Ok(Widths {
                inputs: resolve(p.inputs)?,
                outputs: resolve(p.outputs)?,
            })
        })
        .collect()
}

fn constraints(node: &Node<'_>, p: &Widths, u: &mut Union) -> Result<(), Issue> {
    match &node.kind {
        Kind::Hybrid(BlockKind::ResetIntegrator { initial, .. }) => {
            u.join(p.inputs[0], p.outputs[0])?;
            u.set(p.inputs[1], 1)?;
            if initial.len() > 1 {
                u.set(p.outputs[0], initial.len())?;
            }
        }
        Kind::Legacy(b) => {
            let out = p.outputs.first().copied();
            if let Some(out) = out {
                for &input in &p.inputs {
                    u.join(input, out)?;
                }
                let width = match &b.kind {
                    BlockKind::Constant { value } => Some(value.len()),
                    BlockKind::Gain { gain } if gain.len() > 1 => Some(gain.len()),
                    BlockKind::Integrator { initial } | BlockKind::UnitDelay { initial }
                        if initial.len() > 1 =>
                    {
                        Some(initial.len())
                    }
                    _ => None,
                };
                if let Some(width) = width {
                    u.set(out, width)?;
                }
            }
        }
        Kind::Bias(b) => {
            if b.len() == 1 {
                u.join(p.inputs[0], p.outputs[0])?;
            } else {
                u.set(p.outputs[0], b.len())?;
            }
        }
        Kind::Sine {
            amplitude,
            bias,
            frequency,
            phase,
        } => {
            let width = [amplitude.len(), bias.len(), frequency.len(), phase.len()]
                .into_iter()
                .max()
                .unwrap();
            if [amplitude, bias, frequency, phase]
                .iter()
                .any(|v| v.len() != 1 && v.len() != width)
            {
                return Err(Issue::new(
                    "signal_width",
                    "Sine Wave parameters must be scalars or matching vectors",
                ));
            }
            u.set(p.outputs[0], width)?;
        }
        Kind::StateSpace { b, c, .. } => {
            u.set(p.inputs[0], b.columns)?;
            u.set(p.outputs[0], c.rows)?;
        }
        Kind::Mux(sizes) | Kind::Demux(sizes) => {
            let ids = if matches!(node.kind, Kind::Mux(_)) {
                &p.inputs
            } else {
                &p.outputs
            };
            for (&i, size) in ids.iter().zip(sizes) {
                if let Some(size) = size {
                    u.set(i, *size)?;
                }
            }
        }
        Kind::Step { before, .. } => {
            u.set(p.outputs[0], before.len())?;
        }
        _ => {}
    }
    Ok(())
}

fn propagate(node: &Node<'_>, p: &Widths, u: &mut Union) -> Result<bool, Issue> {
    match &node.kind {
        Kind::Mux(_) | Kind::Demux(_) => {
            let (parts, whole) = if matches!(node.kind, Kind::Mux(_)) {
                (&p.inputs, p.outputs[0])
            } else {
                (&p.outputs, p.inputs[0])
            };
            let mut known = 0;
            let mut missing = Vec::new();
            for &i in parts {
                if let Some(w) = u.get(i) {
                    known += w;
                } else {
                    missing.push(i);
                }
            }
            if missing.is_empty() {
                return u.set(whole, known);
            }
            if let Some(width) = u.get(whole) {
                if width <= known {
                    return Err(Issue::new(
                        "signal_width",
                        "routing widths exceed their combined signal width",
                    ));
                }
                if missing.len() == 1 || matches!(node.kind, Kind::Demux(_)) {
                    let remaining = width - known;
                    if remaining % missing.len() != 0 {
                        return Err(Issue::new(
                            "signal_width",
                            "uneven inferred Demux widths require an explicit Outputs vector",
                        ));
                    }
                    let mut changed = false;
                    let width = remaining / missing.len();
                    for i in missing {
                        changed |= u.set(i, width)?;
                    }
                    return Ok(changed);
                }
            }
        }
        Kind::Hybrid(BlockKind::Control {
            operation: openmat_sim::hybrid::ControlOp::MinMax { inputs: 1, .. },
            ..
        }) => return u.set(p.outputs[0], 1),
        Kind::Product(_) | Kind::Hybrid(BlockKind::Control { .. }) => {
            let inputs: Option<Vec<_>> = p.inputs.iter().map(|&i| u.get(i)).collect();
            if let Some(widths) = inputs {
                let width = *widths.iter().max().unwrap();
                if widths.iter().any(|w| *w != 1 && *w != width) {
                    return Err(Issue::new(
                        "signal_width",
                        "Product requires scalar expansion or matching input widths",
                    ));
                }
                return u.set(p.outputs[0], width);
            }
        }
        Kind::Bias(b) if b.len() > 1 => {
            if let Some(width) = u.get(p.inputs[0])
                && width != 1
                && width != b.len()
            {
                return Err(Issue::new(
                    "signal_width",
                    "Bias requires scalar expansion or matching vector width",
                ));
            }
        }
        _ => {}
    }
    Ok(false)
}

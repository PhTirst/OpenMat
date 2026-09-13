use crate::{
    ModelError,
    model::{Block, BlockKind, Connection, Model, Port},
};
use std::collections::{BTreeMap, BTreeSet};

type Key = (String, String);
fn key(p: &Port) -> Key {
    (p.block.clone(), p.port.clone())
}
fn fail(b: &str, message: &str) -> ModelError {
    ModelError::new("hierarchy", message).at(b, None)
}

fn collect_domains(
    model: &Model,
    blocks: &Blocks<'_>,
    boundaries: &Boundaries<'_>,
    drivers: &BTreeMap<Key, Port>,
) -> Result<Vec<crate::conditional::DomainSpec>, ModelError> {
    let mut domains = Vec::new();
    for group in &model.blocks {
        let BlockKind::Subsystem {
            execution: Some(execution),
            outputs,
            ..
        } = &group.kind
        else {
            continue;
        };
        let mut members = Vec::new();
        let mut scopes = Vec::new();
        for b in &model.blocks {
            let mut parent = b.parent.as_deref();
            while let Some(p) = parent {
                if p == group.id {
                    if matches!(
                        b.kind,
                        BlockKind::Subsystem {
                            execution: Some(_),
                            ..
                        }
                    ) {
                        return Err(fail(
                            &b.id,
                            "nested conditional domains are not supported in conditional-v1",
                        ));
                    }
                    if !matches!(
                        b.kind,
                        BlockKind::Subsystem { .. }
                            | BlockKind::Inport { .. }
                            | BlockKind::Outport { .. }
                    ) {
                        members.push(b.id.clone());
                        if matches!(b.kind, BlockKind::Scope) {
                            scopes.push(b.id.clone());
                        }
                    }
                    break;
                }
                parent = blocks[p].parent.as_deref();
            }
        }
        let outports: Vec<_> = (1..=*outputs)
            .map(|p| boundaries[&(Some(group.id.as_str()), false)][&p].id.clone())
            .collect();
        members.extend(outports.iter().cloned());
        if members.is_empty() || domains.len() >= 64 {
            return Err(fail(
                &group.id,
                "conditional domains need a computational body; at most 64 domains are supported",
            ));
        }
        domains.push(crate::conditional::DomainSpec {
            id: group.id.clone(),
            execution: execution.clone(),
            control: drivers[&(group.id.clone(), execution.control_port().into())].clone(),
            members,
            outports,
            scopes,
        });
    }
    Ok(domains)
}

pub(crate) fn flatten(
    model: &Model,
) -> Result<(Model, Vec<crate::conditional::DomainSpec>), ModelError> {
    let blocks: BTreeMap<_, _> = model.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
    let boundaries = validate_hierarchy(model, &blocks)?;
    let drivers = validate_connections(model, &blocks)?;
    let mut domains = collect_domains(model, &blocks, &boundaries, &drivers)?;
    let conditional_outputs: BTreeSet<_> = domains
        .iter()
        .flat_map(|d| d.outports.iter().cloned())
        .collect();
    let conditional_scopes: BTreeSet<_> = domains
        .iter()
        .flat_map(|d| d.scopes.iter().cloned())
        .collect();
    let aliases = boundary_aliases(model, &boundaries, &drivers, &conditional_outputs);
    let mut cache: BTreeMap<Key, Port> = BTreeMap::new();
    let mut resolve = |source: &Port| resolve_alias(source, &aliases, &mut cache);
    // Validate every alias even if no observable consumes it.
    for (block, port) in aliases.keys() {
        resolve(&Port {
            block: block.clone(),
            port: port.clone(),
        })?;
    }
    for domain in &mut domains {
        domain.control = resolve(&domain.control)?;
    }
    let mut flat = model.clone();
    flat.blocks.retain(|b| {
        !(matches!(b.kind, BlockKind::Subsystem { .. })
            || b.parent.is_some()
                && matches!(b.kind, BlockKind::Inport { .. } | BlockKind::Outport { .. })
                && !conditional_outputs.contains(&b.id))
    });
    let retained: BTreeSet<_> = flat.blocks.iter().map(|b| b.id.clone()).collect();
    flat.connections = model
        .connections
        .iter()
        .filter(|e| retained.contains(&e.to.block))
        .map(|e| {
            Ok(Connection {
                from: resolve(&e.from)?,
                to: e.to.clone(),
            })
        })
        .collect::<Result<_, ModelError>>()?;
    flat.sample_times.retain(|id, _| retained.contains(id));
    for b in &mut flat.blocks {
        b.parent = None;
        if conditional_scopes.contains(&b.id) {
            b.kind = BlockKind::Outport { port: 0 };
        } else if matches!(b.kind, BlockKind::Outport { .. })
            && !conditional_outputs.contains(&b.id)
        {
            b.kind = BlockKind::Scope;
        }
    }
    Ok((flat, domains))
}

fn boundary_aliases(
    model: &Model,
    boundaries: &Boundaries<'_>,
    drivers: &BTreeMap<Key, Port>,
    conditional_outputs: &BTreeSet<String>,
) -> BTreeMap<Key, Port> {
    let mut aliases = BTreeMap::new();
    for block in &model.blocks {
        match &block.kind {
            BlockKind::Inport { port, .. } if block.parent.is_some() => {
                aliases.insert(
                    (block.id.clone(), "out".into()),
                    drivers[&(block.parent.clone().expect("parent"), format!("in{port}"))].clone(),
                );
            }
            BlockKind::Subsystem { outputs, .. } => {
                for port in 1..=*outputs {
                    let child = boundaries[&(Some(block.id.as_str()), false)][&port];
                    aliases.insert(
                        (block.id.clone(), format!("out{port}")),
                        if conditional_outputs.contains(&child.id) {
                            Port {
                                block: child.id.clone(),
                                port: "out".into(),
                            }
                        } else {
                            drivers[&(child.id.clone(), "in".into())].clone()
                        },
                    );
                }
            }
            _ => {}
        }
    }
    aliases
}

fn resolve_alias(
    source: &Port,
    aliases: &BTreeMap<Key, Port>,
    cache: &mut BTreeMap<Key, Port>,
) -> Result<Port, ModelError> {
    let mut p = source.clone();
    let mut visited = BTreeSet::new();
    loop {
        let k = key(&p);
        if let Some(found) = cache.get(&k) {
            p = found.clone();
            break;
        }
        if let Some(next) = aliases.get(&k) {
            if !visited.insert(k) {
                return Err(fail(
                    &p.block,
                    "instantaneous cycle through subsystem boundary ports",
                ));
            }
            p = next.clone();
        } else {
            break;
        }
    }
    for k in visited {
        cache.insert(k, p.clone());
    }
    Ok(p)
}

type Blocks<'a> = BTreeMap<&'a str, &'a Block>;
type Boundaries<'a> = BTreeMap<(Option<&'a str>, bool), BTreeMap<usize, &'a Block>>;

fn validate_parent(block: &Block, blocks: &Blocks<'_>) -> Result<(), ModelError> {
    let mut ancestor = block.parent.as_deref();
    let mut seen = BTreeSet::from([block.id.as_str()]);
    while let Some(id) = ancestor {
        let parent = blocks
            .get(id)
            .ok_or_else(|| fail(&block.id, "parent system is missing"))?;
        if !matches!(parent.kind, BlockKind::Subsystem { .. })
            || !seen.insert(id)
            || seen.len() > 33
        {
            return Err(fail(
                &block.id,
                "parent must be a subsystem without cycles, at most 32 levels deep",
            ));
        }
        ancestor = parent.parent.as_deref();
    }
    Ok(())
}

fn validate_hierarchy<'a>(
    model: &'a Model,
    blocks: &Blocks<'a>,
) -> Result<Boundaries<'a>, ModelError> {
    if let Some(id) = model
        .sample_times
        .keys()
        .find(|id| !blocks.contains_key(id.as_str()))
    {
        return Err(
            ModelError::new("sample_time", "sampleTimes references an unknown block").at(id, None),
        );
    }
    let mut boundaries = Boundaries::new();
    for block in &model.blocks {
        validate_parent(block, blocks)?;
        let boundary = match block.kind {
            BlockKind::Inport { port, .. } => Some((true, port)),
            BlockKind::Outport { port } => Some((false, port)),
            _ => None,
        };
        if let Some((input, port)) = boundary
            && boundaries
                .entry((block.parent.as_deref(), input))
                .or_default()
                .insert(port, block)
                .is_some()
        {
            return Err(fail(&block.id, "boundary port number is duplicated"));
        }
        if let BlockKind::Inport { data, .. } = &block.kind {
            if block.parent.is_some() && data.is_some() {
                return Err(fail(
                    &block.id,
                    "internal Inport takes its signal from the parent boundary, not embedded data",
                ));
            }
            if block.parent.is_none() && data.is_none() {
                return Err(ModelError::new(
                    "input_data",
                    "root Inport needs an embedded time series",
                )
                .at(&block.id, None));
            }
        }
        if model.sample_times.contains_key(&block.id)
            && (matches!(block.kind, BlockKind::Subsystem { .. })
                || boundary.is_some() && block.parent.is_some())
        {
            return Err(fail(
                &block.id,
                "virtual subsystem and internal port rates are inherited; configure a numerical block instead",
            ));
        }
    }
    for ((parent, input), ports) in &boundaries {
        let expected = match parent.and_then(|id| blocks.get(id)).map(|b| &b.kind) {
            Some(BlockKind::Subsystem {
                inputs, outputs, ..
            }) => {
                if *input {
                    *inputs
                } else {
                    *outputs
                }
            }
            _ => ports.len(),
        };
        if ports.len() != expected || !ports.keys().copied().eq(1..=expected) {
            return Err(fail(
                parent.unwrap_or("root"),
                "boundary ports must be unique and contiguous and match the subsystem interface",
            ));
        }
    }
    for b in &model.blocks {
        if let BlockKind::Subsystem {
            inputs, outputs, ..
        } = b.kind
        {
            for (input, count) in [(true, inputs), (false, outputs)] {
                if boundaries
                    .get(&(Some(b.id.as_str()), input))
                    .map_or(0, BTreeMap::len)
                    != count
                {
                    return Err(fail(
                        &b.id,
                        "subsystem is missing an internal boundary port",
                    ));
                }
            }
        }
    }
    Ok(boundaries)
}

fn validate_connections(
    model: &Model,
    blocks: &Blocks<'_>,
) -> Result<BTreeMap<Key, Port>, ModelError> {
    let definitions: BTreeMap<_, _> = model
        .components
        .iter()
        .map(|d| (d.id.as_str(), d))
        .collect();
    let ports = |b: &Block, output: bool| {
        if let BlockKind::Component { component, .. } = &b.kind {
            return definitions
                .get(component.as_str())
                .map_or_else(Vec::new, |d| {
                    if output { &d.outputs } else { &d.inputs }
                        .iter()
                        .map(|p| p.name.clone())
                        .collect()
                });
        }
        if output {
            super::output_names(&b.kind)
        } else {
            b.kind.inputs()
        }
    };
    let mut drivers = BTreeMap::new();
    for edge in &model.connections {
        let source = blocks
            .get(edge.from.block.as_str())
            .ok_or_else(|| fail(&edge.from.block, "connection source is missing"))?;
        let target = blocks
            .get(edge.to.block.as_str())
            .ok_or_else(|| fail(&edge.to.block, "connection target is missing"))?;
        if source.parent != target.parent {
            return Err(fail(
                &target.id,
                "connections cannot cross system boundaries; use Inport/Outport",
            ));
        }
        if !ports(source, true).contains(&edge.from.port)
            || !ports(target, false).contains(&edge.to.port)
        {
            return Err(fail(&target.id, "connection references an unknown port"));
        }
        if drivers.insert(key(&edge.to), edge.from.clone()).is_some() {
            return Err(fail(&target.id, "input has multiple drivers"));
        }
    }
    for block in &model.blocks {
        for port in ports(block, false) {
            if !drivers.contains_key(&(block.id.clone(), port.clone())) {
                return Err(
                    ModelError::new("missing_input", "required input is not connected")
                        .at(&block.id, Some(&port)),
                );
            }
        }
    }
    Ok(drivers)
}

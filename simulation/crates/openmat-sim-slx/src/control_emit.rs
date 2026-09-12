use crate::{
    Issue, ParameterArray,
    control_blocks::{Kind, Node, expand},
    control_graph::{Edge, Widths},
    literal,
    lower::runtime_id,
};
use openmat_sim::{
    SourceBundle,
    component::{Callback, ComponentDefinition},
    model::{Block, BlockKind, Connection, FunctionInput, Model, Port, Position, Settings},
};
use std::collections::BTreeMap;

pub(crate) fn emit(
    name: &str,
    settings: Settings,
    nodes: &[Node<'_>],
    edges: &[Edge],
    widths: &[Widths],
) -> Result<(Model, SourceBundle), Issue> {
    let mut model = Model {
        schema_version: 4,
        name: name.into(),
        settings,
        blocks: vec![],
        connections: vec![],
        components: vec![],
    };
    let mut sources = SourceBundle::new();
    for (node, width) in nodes.iter().zip(widths) {
        if !node.executable() {
            continue;
        }
        let block = emit_block(node, width, &mut model.components, &mut sources)
            .map_err(|e| e.at(node.source, None))?;
        model.blocks.push(block);
    }
    for edge in edges {
        let from = &nodes[edge.from.block];
        let to = &nodes[edge.to.block];
        if !to.executable() {
            continue;
        }
        model.connections.push(Connection {
            from: Port {
                block: runtime_id(&from.source.sid)?,
                port: from.output_name(edge.from.port),
            },
            to: Port {
                block: runtime_id(&to.source.sid)?,
                port: to.input_name(edge.to.port),
            },
        });
    }
    if model.components.len() > 64
        || sources.len() > 64
        || sources.values().map(String::len).sum::<usize>() > 1024 * 1024
    {
        return Err(Issue::new(
            "control_limit",
            "generated component definitions or source bundle exceed the bounded compiler profile",
        ));
    }
    Ok((model, sources))
}

fn callback(
    id: &str,
    role: &str,
    body: &str,
    initial: bool,
    sources: &mut SourceBundle,
) -> Callback {
    let source = format!("slx_generated/{id}_{role}.m");
    let entry = format!("om_{role}");
    let signature = if initial { "p" } else { "t,x,q,u,p" };
    sources.insert(source.clone(),format!("% Generated from an OpenMat SLX control-v1 numerical snapshot.\nfunction y = {entry}({signature})\ny = {body};\nend\n"));
    Callback { source, entry }
}
fn column(values: impl Iterator<Item = String>) -> String {
    format!("[{}]", values.collect::<Vec<_>>().join("; "))
}
fn literal_column(values: &[f64]) -> String {
    column(values.iter().map(ToString::to_string))
}
fn coefficient(v: f64, variable: &str, index: usize) -> String {
    format!("({v})*{variable}({})", index + 1)
}
fn linear(a: &ParameterArray, b: &ParameterArray, row: usize) -> String {
    let terms = (0..a.columns)
        .filter_map(|c| {
            let v = a.values[row + c * a.rows];
            (v != 0.0).then(|| coefficient(v, "x", c))
        })
        .chain((0..b.columns).filter_map(|c| {
            let v = b.values[row + c * b.rows];
            (v != 0.0).then(|| coefficient(v, "u", c))
        }))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        "0".into()
    } else {
        terms.join(" + ")
    }
}

#[allow(clippy::too_many_lines)]
fn emit_block(
    node: &Node<'_>,
    width: &Widths,
    definitions: &mut Vec<ComponentDefinition>,
    sources: &mut SourceBundle,
) -> Result<Block, Issue> {
    let id = runtime_id(&node.source.sid)?;
    let position = node
        .source
        .properties
        .get("Position")
        .map(|text| {
            let v = literal::vector(text)?;
            if v.len() != 4 {
                return Err(Issue::new(
                    "position",
                    "block position requires four literal numbers",
                ));
            }
            Ok(Position { x: v[0], y: v[1] })
        })
        .transpose()?;
    let mut x = 0;
    let mut init = None;
    let mut derivatives = None;
    let output = match &node.kind {
        Kind::Legacy(block) => {
            let mut block = block.clone();
            if let BlockKind::Integrator { initial } | BlockKind::UnitDelay { initial } =
                &mut block.kind
            {
                *initial = expand(initial.clone(), width.outputs[0])?;
            }
            return Ok(block);
        }
        Kind::Ground => {
            return Ok(Block {
                id,
                position,
                kind: BlockKind::Constant {
                    value: vec![0.0; width.outputs[0]],
                },
            });
        }
        Kind::Step {
            time,
            before,
            after,
        } => {
            return Ok(Block {
                id,
                position,
                kind: BlockKind::Step {
                    time: *time,
                    before: before.clone(),
                    after: after.clone(),
                },
            });
        }
        Kind::Bias(values) => {
            let values = expand(values.clone(), width.outputs[0])?;
            column(values.iter().enumerate().map(|(i, v)| {
                format!(
                    "u({}) + ({v})",
                    if width.inputs[0] == 1 { 1 } else { i + 1 }
                )
            }))
        }
        Kind::Product(signs) => column((0..width.outputs[0]).map(|i| {
            let mut offset = 0;
            let mut value = "1".to_owned();
            for (sign, n) in signs.iter().zip(&width.inputs) {
                let index = offset + if *n == 1 { 1 } else { i + 1 };
                value = format!("({value}) {sign} u({index})");
                offset += n;
            }
            value
        })),
        Kind::Sine {
            amplitude,
            bias,
            frequency,
            phase,
        } => {
            let n = width.outputs[0];
            let amplitude = expand(amplitude.clone(), n)?;
            let bias = expand(bias.clone(), n)?;
            let frequency = expand(frequency.clone(), n)?;
            let phase = expand(phase.clone(), n)?;
            column((0..n).map(|i| {
                format!(
                    "({})*sin(({})*t+({}))+({})",
                    amplitude[i], frequency[i], phase[i], bias[i]
                )
            }))
        }
        Kind::StateSpace {
            a,
            b,
            c,
            d,
            initial,
        } => {
            x = a.rows;
            if x > 0 {
                init = Some(callback(
                    &id,
                    "initialize",
                    &literal_column(initial),
                    true,
                    sources,
                ));
                derivatives = Some(callback(
                    &id,
                    "derivatives",
                    &column((0..a.rows).map(|r| linear(a, b, r))),
                    false,
                    sources,
                ));
            }
            column((0..c.rows).map(|r| linear(c, d, r)))
        }
        Kind::Mux(_) | Kind::Demux(_) => "u".into(),
        _ => unreachable!("virtual boundaries and terminators do not emit numerical blocks"),
    };
    let outputs_function = callback(&id, "outputs", &output, false, sources);
    let component = format!("om_import_{id}");
    definitions.push(ComponentDefinition {
        id: component.clone(),
        name: node.source.block_type.clone(),
        category: "SLX control-v1".into(),
        icon: if x > 0 { "plant" } else { "function" }.into(),
        inputs: width
            .inputs
            .iter()
            .enumerate()
            .map(|(i, &width)| FunctionInput {
                name: node.input_name(i),
                width,
            })
            .collect(),
        outputs: width
            .outputs
            .iter()
            .enumerate()
            .map(|(i, &width)| FunctionInput {
                name: node.output_name(i),
                width,
            })
            .collect(),
        parameters: vec![],
        continuous_states: x,
        discrete_states: 0,
        sample_time: None,
        initialize: init,
        outputs_function,
        derivatives,
        update: None,
    });
    Ok(Block {
        id,
        position,
        kind: BlockKind::Component {
            component,
            parameters: BTreeMap::new(),
        },
    })
}

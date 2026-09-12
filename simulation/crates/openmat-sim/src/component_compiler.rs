//! Schema 3 port dependency scheduling and separate flow/update programs.
use std::collections::{BTreeMap, BTreeSet};

use crate::component::ComponentDefinition;
use crate::model::{Block, BlockKind, FunctionInput, Model, Port};
use crate::numeric::{Comparison, Instruction, Kernel, MAX_VALUES, Program, ReferenceKernel};
use crate::static_function::{self, Signature};
use crate::{CompiledModel, ModelError, ScopeInfo, SourceBundle, m_function};

#[derive(Clone, Copy)]
struct Wire {
    block: usize,
    port: usize,
}

struct Node<'a> {
    block: &'a Block,
    definition: Option<&'a ComponentDefinition>,
    inputs: Vec<FunctionInput>,
    outputs: Vec<FunctionInput>,
    drivers: Vec<Wire>,
    x: Vec<f64>,
    q: Vec<f64>,
    x_offset: usize,
    q_offset: usize,
    signals: Vec<Program>,
    derivatives: Option<Program>,
    update: Option<Program>,
    scope: Option<Program>,
}

fn failure(block: &Block, code: &str, message: &str) -> ModelError {
    ModelError::new(code, message).at(&block.id, None)
}

fn program(
    inputs: usize,
    instructions: Vec<Instruction>,
    outputs: Vec<usize>,
) -> Result<Program, ModelError> {
    Program::new(inputs, instructions, outputs)
        .map(|p| p.pruned())
        .map_err(|e| ModelError::new("numerical_ir", e.to_string()))
}

#[allow(clippy::too_many_lines)] // Ordered compilation phases share the validated graph and state layout.
pub(crate) fn compile(model: &Model, sources: &SourceBundle) -> Result<CompiledModel, ModelError> {
    let mut definitions = BTreeMap::new();
    if model.components.len() > 64 {
        return Err(ModelError::new(
            "model_limit",
            "at most 64 component definitions are supported",
        ));
    }
    for definition in &model.components {
        definition.validate()?;
        if definitions
            .insert(definition.id.as_str(), definition)
            .is_some()
        {
            return Err(ModelError::new(
                "component_definition",
                "duplicate component definition ID",
            ));
        }
    }
    let mut blocks: Vec<_> = model.blocks.iter().collect();
    blocks.sort_by(|a, b| a.id.cmp(&b.id));
    let mut nodes = blocks
        .iter()
        .map(|b| node(b, &definitions))
        .collect::<Result<Vec<_>, _>>()?;
    connect(model, &mut nodes)?;
    infer_widths(&mut nodes)?;
    let (mut x, mut q, mut work, mut ir_size) = (Vec::new(), Vec::new(), 0, 0);
    for node in &mut nodes {
        let paths = node.definition.map_or_else(
            || match &node.block.kind {
                BlockKind::MFunction { source, .. } => vec![source.as_str()],
                _ => vec![],
            },
            |d| d.callbacks().iter().map(|c| c.source.as_str()).collect(),
        );
        work += paths
            .iter()
            .filter_map(|p| sources.get(*p))
            .map(String::len)
            .sum::<usize>();
        if work > 4 * 1024 * 1024 {
            return Err(failure(
                node.block,
                "model_limit",
                "total callback lowering work exceeds 4 MiB",
            ));
        }
        if let Some(definition) = node.definition {
            if definition.sample_time.is_some()
                && definition.sample_time != model.settings.sample_time
            {
                return Err(failure(
                    node.block,
                    "sample_time",
                    "component sampleTime must equal settings.sampleTime; multirate scheduling is not supported",
                ));
            }
            lower_component(node, sources)?;
        } else {
            lower_builtin(node, sources)?;
        }
        ir_size += node
            .signals
            .iter()
            .chain(node.derivatives.iter())
            .chain(node.update.iter())
            .chain(node.scope.iter())
            .map(|p| p.instructions().len())
            .sum::<usize>();
        node.x_offset = x.len();
        node.q_offset = q.len();
        x.extend(&node.x);
        q.extend(&node.q);
        if x.len() + q.len() > 262_144 || ir_size > MAX_VALUES {
            return Err(failure(
                node.block,
                "model_limit",
                "combined state or callback instruction budget exceeded",
            ));
        }
    }
    let input_count = 1 + x.len() + q.len();
    let mut flow = Emitter::new(&nodes, x.len(), input_count);
    // Validate all output dependencies, including disconnected components.
    for (block, node) in nodes.iter().enumerate() {
        for port in 0..node.outputs.len() {
            flow.signal(Wire { block, port }, 0)?;
        }
    }
    let mut outputs = Vec::new();
    let mut origins = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if let Some(p) = &node.derivatives {
            outputs.extend(flow.callback(i, p, 0)?);
            origins.extend((0..node.x.len()).map(|_| origin(node, "derivatives")));
        }
    }
    for (i, node) in nodes.iter().enumerate() {
        for j in 0..node.q.len() {
            outputs.push(flow.push(Instruction::Input(1 + x.len() + node.q_offset + j))?);
        }
        origins.extend((0..nodes[i].q.len()).map(|_| origin(node, "state")));
    }
    let mut scopes = Vec::new();
    let mut offset = 0;
    for (i, node) in nodes.iter().enumerate() {
        if let Some(p) = &node.scope {
            let values = flow.callback(i, p, 0)?;
            scopes.push(ScopeInfo {
                block: node.block.id.clone(),
                offset,
                width: values.len(),
            });
            offset += values.len();
            origins.extend((0..values.len()).map(|_| origin(node, "in")));
            outputs.extend(values);
        }
    }
    if outputs.len() > 262_144 {
        return Err(ModelError::new(
            "model_limit",
            "too many state and scope components",
        ));
    }
    let execution_order = flow
        .order
        .iter()
        .map(|&i| nodes[i].block.id.clone())
        .collect();
    let flow_program = program(input_count, flow.instructions, outputs)?;
    let mut update = Emitter::new(&nodes, x.len(), input_count);
    let mut next = Vec::new();
    let mut update_origins = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if let Some(p) = &node.update {
            next.extend(update.callback(i, p, 0)?);
            update_origins.extend((0..node.q.len()).map(|_| origin(node, "update")));
        }
    }
    let update_program = if q.is_empty() {
        None
    } else {
        Some(program(input_count, update.instructions, next)?)
    };
    let mut time_events: Vec<_> = model
        .blocks
        .iter()
        .filter_map(|b| {
            if let BlockKind::Step { time, .. } = b.kind {
                (time > model.settings.start_time && time <= model.settings.stop_time)
                    .then_some(time)
            } else {
                None
            }
        })
        .collect();
    time_events.sort_by(f64::total_cmp);
    time_events.dedup();
    Ok(CompiledModel {
        name: model.name.clone(),
        settings: model.settings.clone(),
        program: flow_program,
        update_program,
        update_origins,
        continuous_initial: x,
        discrete_initial: q,
        scopes,
        origins,
        execution_order,
        time_events,
    })
}

fn origin(node: &Node<'_>, port: &str) -> Port {
    Port {
        block: node.block.id.clone(),
        port: port.into(),
    }
}

fn node<'a>(
    block: &'a Block,
    definitions: &BTreeMap<&str, &'a ComponentDefinition>,
) -> Result<Node<'a>, ModelError> {
    let definition = if let BlockKind::Component { component, .. } = &block.kind {
        Some(*definitions.get(component.as_str()).ok_or_else(|| {
            failure(
                block,
                "component_definition",
                "component definition is missing",
            )
        })?)
    } else {
        None
    };
    let (inputs, outputs) = if let Some(d) = definition {
        (d.inputs.clone(), d.outputs.clone())
    } else {
        let inputs = match &block.kind {
            BlockKind::MFunction { inputs, .. } => inputs.clone(),
            _ => block
                .kind
                .inputs()
                .into_iter()
                .map(|name| FunctionInput { name, width: 0 })
                .collect(),
        };
        let width = match &block.kind {
            BlockKind::Constant { value } => value.len(),
            BlockKind::Step { before, .. } => before.len(),
            BlockKind::Integrator { initial } | BlockKind::UnitDelay { initial } => initial.len(),
            BlockKind::MFunction { output_width, .. } => *output_width,
            _ => 0,
        };
        let outputs = if matches!(block.kind, BlockKind::Scope) {
            vec![]
        } else {
            vec![FunctionInput {
                name: "out".into(),
                width,
            }]
        };
        (inputs, outputs)
    };
    Ok(Node {
        block,
        definition,
        inputs,
        outputs,
        drivers: vec![],
        x: vec![],
        q: vec![],
        x_offset: 0,
        q_offset: 0,
        signals: vec![],
        derivatives: None,
        update: None,
        scope: None,
    })
}

fn connect(model: &Model, nodes: &mut [Node<'_>]) -> Result<(), ModelError> {
    let ids: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.block.id.as_str(), i))
        .collect();
    let mut wires: Vec<Vec<Option<Wire>>> =
        nodes.iter().map(|n| vec![None; n.inputs.len()]).collect();
    for connection in &model.connections {
        let lookup = |port: &Port, output: bool| -> Result<(usize, usize), ModelError> {
            let index = *ids.get(port.block.as_str()).ok_or_else(|| {
                ModelError::new("unknown_block", "connection block does not exist")
                    .at(&port.block, Some(&port.port))
            })?;
            let ports = if output {
                &nodes[index].outputs
            } else {
                &nodes[index].inputs
            };
            let p = ports
                .iter()
                .position(|p| p.name == port.port)
                .ok_or_else(|| {
                    ModelError::new("unknown_port", "connection port does not exist")
                        .at(&port.block, Some(&port.port))
                })?;
            Ok((index, p))
        };
        let (from, output) = lookup(&connection.from, true)?;
        let (to, input) = lookup(&connection.to, false)?;
        if wires[to][input]
            .replace(Wire {
                block: from,
                port: output,
            })
            .is_some()
        {
            return Err(
                ModelError::new("multiple_drivers", "input has more than one driver")
                    .at(&connection.to.block, Some(&connection.to.port)),
            );
        }
    }
    for (node, wires) in nodes.iter_mut().zip(wires) {
        node.drivers = wires
            .into_iter()
            .enumerate()
            .map(|(i, w)| {
                w.ok_or_else(|| {
                    ModelError::new("missing_input", "required input is not connected")
                        .at(&node.block.id, Some(&node.inputs[i].name))
                })
            })
            .collect::<Result<_, _>>()?;
    }
    Ok(())
}

fn infer_widths(nodes: &mut [Node<'_>]) -> Result<(), ModelError> {
    for _ in 0..nodes.len() {
        let mut changed = false;
        for i in 0..nodes.len() {
            if nodes[i].outputs.first().is_some_and(|p| p.width == 0) {
                let driver = nodes[i].drivers[0];
                let width = nodes[driver.block].outputs[driver.port].width;
                if width > 0 {
                    nodes[i].outputs[0].width = width;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut total = 0;
    for i in 0..nodes.len() {
        if nodes[i].outputs.iter().any(|p| p.width == 0) {
            return Err(failure(
                nodes[i].block,
                "algebraic_loop",
                "cannot infer width through an instantaneous cycle",
            ));
        }
        for j in 0..nodes[i].inputs.len() {
            let wire = nodes[i].drivers[j];
            let actual = nodes[wire.block].outputs[wire.port].width;
            let expected = if nodes[i].definition.is_some()
                || matches!(nodes[i].block.kind, BlockKind::MFunction { .. })
            {
                nodes[i].inputs[j].width
            } else if !matches!(nodes[i].block.kind, BlockKind::Scope) {
                nodes[i].outputs[0].width
            } else {
                actual
            };
            if actual != expected {
                return Err(ModelError::new(
                    "signal_width",
                    format!("expected width {expected}, found {actual}"),
                )
                .at(&nodes[i].block.id, Some(&nodes[i].inputs[j].name)));
            }
            nodes[i].inputs[j].width = actual;
            total += actual;
        }
        total += nodes[i].outputs.iter().map(|p| p.width).sum::<usize>();
        if total > MAX_VALUES {
            return Err(failure(
                nodes[i].block,
                "model_limit",
                "signal width budget exceeded",
            ));
        }
    }
    Ok(())
}

fn lower_component(node: &mut Node<'_>, sources: &SourceBundle) -> Result<(), ModelError> {
    let d = node.definition.expect("component definition");
    let BlockKind::Component { parameters, .. } = &node.block.kind else {
        unreachable!()
    };
    let parameters = d
        .parameter_values(parameters)
        .map_err(|e| e.at(&node.block.id, None))?;
    if let Some(callback) = &d.initialize {
        let p = static_function::compile(
            &node.block.id,
            callback,
            &Signature {
                arguments: &[],
                parameters: &parameters,
                output_width: d.continuous_states + d.discrete_states,
                conditions: true,
            },
            sources,
        )?;
        let mut values = vec![0.0; p.output_count()];
        ReferenceKernel::new(p)
            .evaluate(&[], &mut values)
            .map_err(|e| failure(node.block, "component_initialize", &e.to_string()))?;
        if values.iter().any(|v| !v.is_finite()) {
            return Err(failure(
                node.block,
                "component_initialize",
                "initial states must be finite",
            ));
        }
        node.x = values[..d.continuous_states].to_vec();
        node.q = values[d.continuous_states..].to_vec();
    }
    let args = [
        ("t", 1),
        ("x", node.x.len()),
        ("q", node.q.len()),
        ("u", d.inputs.iter().map(|p| p.width).sum()),
    ];
    let lower = |callback, width, conditions| {
        static_function::compile(
            &node.block.id,
            callback,
            &Signature {
                arguments: &args,
                parameters: &parameters,
                output_width: width,
                conditions,
            },
            sources,
        )
    };
    let outputs = lower(
        &d.outputs_function,
        d.outputs.iter().map(|p| p.width).sum(),
        false,
    )?;
    let mut offset = 0;
    let mut signal_work = 0;
    for port in &d.outputs {
        let signal = program(
            outputs.input_count(),
            outputs.instructions().to_vec(),
            outputs.outputs()[offset..offset + port.width].to_vec(),
        )?;
        signal_work += signal.instructions().len();
        if signal_work > MAX_VALUES {
            return Err(failure(
                node.block,
                "model_limit",
                "per-output callback instruction budget exceeded",
            ));
        }
        node.signals.push(signal);
        offset += port.width;
    }
    node.derivatives = d
        .derivatives
        .as_ref()
        .map(|c| lower(c, node.x.len(), false))
        .transpose()?;
    node.update = d
        .update
        .as_ref()
        .map(|c| lower(c, node.q.len(), true))
        .transpose()?;
    Ok(())
}

#[allow(clippy::too_many_lines)] // One checked adapter per supported builtin, sharing local signal offsets.
fn lower_builtin(node: &mut Node<'_>, sources: &SourceBundle) -> Result<(), ModelError> {
    match &node.block.kind {
        BlockKind::Integrator { initial } => node.x.clone_from(initial),
        BlockKind::UnitDelay { initial } => node.q.clone_from(initial),
        _ => {}
    }
    let start = 1 + node.x.len() + node.q.len();
    let count = start + node.inputs.iter().map(|p| p.width).sum::<usize>();
    let mut ops: Vec<_> = (0..count).map(Instruction::Input).collect();
    let mut offset = start;
    let inputs: Vec<Vec<usize>> = node
        .inputs
        .iter()
        .map(|p| {
            let ids = (offset..offset + p.width).collect();
            offset += p.width;
            ids
        })
        .collect();
    let push = |ops: &mut Vec<Instruction>, op| {
        let id = ops.len();
        ops.push(op);
        id
    };
    let output = match &node.block.kind {
        BlockKind::Step {
            time,
            before,
            after,
        } => {
            let event = push(&mut ops, Instruction::Constant(*time));
            let condition = push(
                &mut ops,
                Instruction::Compare(Comparison::GreaterEqual, 0, event),
            );
            before
                .iter()
                .zip(after)
                .map(|(&a, &b)| {
                    let a = push(&mut ops, Instruction::Constant(a));
                    let b = push(&mut ops, Instruction::Constant(b));
                    push(&mut ops, Instruction::Select(condition, b, a))
                })
                .collect()
        }
        BlockKind::Constant { value } => value
            .iter()
            .map(|&v| push(&mut ops, Instruction::Constant(v)))
            .collect(),
        BlockKind::Integrator { .. } => {
            node.derivatives = Some(program(count, ops.clone(), inputs[0].clone())?);
            (1..start).collect()
        }
        BlockKind::UnitDelay { .. } => {
            node.update = Some(program(count, ops.clone(), inputs[0].clone())?);
            (1..start).collect()
        }
        BlockKind::Gain { gain } => {
            if gain.len() != 1 && gain.len() != inputs[0].len() {
                return Err(failure(
                    node.block,
                    "signal_width",
                    "Gain width must be scalar or match its input",
                ));
            }
            inputs[0]
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let g = push(
                        &mut ops,
                        Instruction::Constant(gain[if gain.len() == 1 { 0 } else { i }]),
                    );
                    push(&mut ops, Instruction::Multiply(g, v))
                })
                .collect()
        }
        BlockKind::Sum { signs } => (0..inputs[0].len())
            .map(|i| {
                let mut sum = if signs[0] == 1 {
                    inputs[0][i]
                } else {
                    push(&mut ops, Instruction::Negate(inputs[0][i]))
                };
                for (input, sign) in inputs.iter().zip(signs).skip(1) {
                    sum = push(
                        &mut ops,
                        if *sign == 1 {
                            Instruction::Add(sum, input[i])
                        } else {
                            Instruction::Subtract(sum, input[i])
                        },
                    );
                }
                sum
            })
            .collect(),
        BlockKind::MFunction { .. } => m_function::lower(
            node.block,
            &inputs.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            sources,
            &mut ops,
        )?,
        BlockKind::Scope => {
            node.scope = Some(program(count, ops, inputs[0].clone())?);
            return Ok(());
        }
        BlockKind::Component { .. } => unreachable!(),
    };
    node.signals.push(program(count, ops, output)?);
    Ok(())
}

struct Emitter<'a> {
    nodes: &'a [Node<'a>],
    continuous: usize,
    input_count: usize,
    instructions: Vec<Instruction>,
    cache: Vec<Vec<Option<Vec<usize>>>>,
    visiting: BTreeSet<(usize, usize)>,
    order: Vec<usize>,
}

impl<'a> Emitter<'a> {
    fn new(nodes: &'a [Node<'a>], continuous: usize, input_count: usize) -> Self {
        Self {
            nodes,
            continuous,
            input_count,
            instructions: vec![],
            cache: nodes.iter().map(|n| vec![None; n.outputs.len()]).collect(),
            visiting: BTreeSet::new(),
            order: vec![],
        }
    }
    fn push(&mut self, instruction: Instruction) -> Result<usize, ModelError> {
        if self.instructions.len() >= MAX_VALUES {
            return Err(ModelError::new(
                "model_limit",
                "combined numerical program exceeds instruction budget",
            ));
        }
        let id = self.instructions.len();
        self.instructions.push(instruction);
        Ok(id)
    }
    fn signal(&mut self, wire: Wire, depth: usize) -> Result<Vec<usize>, ModelError> {
        if let Some(values) = &self.cache[wire.block][wire.port] {
            return Ok(values.clone());
        }
        let node = &self.nodes[wire.block];
        if !self.visiting.insert((wire.block, wire.port)) {
            return Err(ModelError::new(
                "algebraic_loop",
                "instantaneous output dependency cycle requires an algebraic solver",
            )
            .at(&node.block.id, Some(&node.outputs[wire.port].name)));
        }
        if depth > 128 {
            return Err(failure(
                node.block,
                "model_limit",
                "output dependency depth exceeds 128",
            ));
        }
        let values = self.callback(wire.block, &node.signals[wire.port], depth + 1)?;
        self.cache[wire.block][wire.port] = Some(values.clone());
        self.visiting.remove(&(wire.block, wire.port));
        if !self.order.contains(&wire.block) {
            self.order.push(wire.block);
        }
        Ok(values)
    }
    fn callback(
        &mut self,
        block: usize,
        program: &Program,
        depth: usize,
    ) -> Result<Vec<usize>, ModelError> {
        let node = &self.nodes[block];
        let header = 1 + node.x.len() + node.q.len();
        let mut inputs = BTreeMap::new();
        // Only input instructions surviving SSA pruning create feedthrough edges.
        for op in program.instructions() {
            if let Instruction::Input(i) = *op {
                if inputs.contains_key(&i) {
                    continue;
                }
                let mapped = if i < header {
                    let global = if i == 0 {
                        0
                    } else if i <= node.x.len() {
                        node.x_offset + i
                    } else {
                        1 + self.continuous + node.q_offset + i - 1 - node.x.len()
                    };
                    debug_assert!(global < self.input_count);
                    self.push(Instruction::Input(global))?
                } else {
                    let mut offset = i - header;
                    let p = node
                        .inputs
                        .iter()
                        .position(|p| {
                            if offset < p.width {
                                true
                            } else {
                                offset -= p.width;
                                false
                            }
                        })
                        .expect("verified local input");
                    self.signal(node.drivers[p], depth)?[offset]
                };
                inputs.insert(i, mapped);
            }
        }
        let mut mapping = Vec::with_capacity(program.instructions().len());
        for op in program.instructions() {
            let id = if let Instruction::Input(i) = op {
                inputs[i]
            } else {
                self.push(op.remap(|i| mapping[i]))?
            };
            mapping.push(id);
        }
        Ok(program.outputs().iter().map(|&i| mapping[i]).collect())
    }
}

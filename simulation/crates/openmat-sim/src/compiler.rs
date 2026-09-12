use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::ModelError;
use crate::model::{Block, BlockKind, Model, Port, SCHEMA_VERSION, Settings};
use crate::numeric::{Instruction, MAX_VALUES, Program};

const MAX_BLOCKS: usize = 10_000;
const MAX_COMPONENTS: usize = 262_144;
const MAX_SUM_INPUTS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeInfo {
    pub block: String,
    /// Offset relative to the flattened scope values in a frame.
    pub offset: usize,
    pub width: usize,
}

#[derive(Clone, Debug)]
pub struct CompiledModel {
    pub(crate) name: String,
    pub(crate) settings: Settings,
    pub(crate) program: Program,
    pub(crate) continuous_initial: Vec<f64>,
    pub(crate) discrete_initial: Vec<f64>,
    pub(crate) scopes: Vec<ScopeInfo>,
    pub(crate) origins: Vec<Port>,
    execution_order: Vec<String>,
}

impl CompiledModel {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.program
    }
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    #[must_use]
    pub fn scopes(&self) -> &[ScopeInfo] {
        &self.scopes
    }
    #[must_use]
    pub fn continuous_state_count(&self) -> usize {
        self.continuous_initial.len()
    }
    #[must_use]
    pub fn discrete_state_count(&self) -> usize {
        self.discrete_initial.len()
    }
    #[must_use]
    pub fn execution_order(&self) -> &[String] {
        &self.execution_order
    }
}

struct Graph<'a> {
    blocks: Vec<&'a Block>,
    inputs: Vec<Vec<usize>>,
    order: Vec<usize>,
}

/// Validate a model and lower its numerical work into verified f64 SSA.
///
/// # Errors
/// Returns a diagnostic with the relevant block and port for invalid models.
pub fn compile(model: &Model) -> Result<CompiledModel, ModelError> {
    validate_model(model)?;
    let graph = build_graph(model)?;
    let mut continuous_initial = Vec::new();
    let mut discrete_initial = Vec::new();
    let mut continuous_offsets = vec![0; graph.blocks.len()];
    let mut discrete_offsets = vec![0; graph.blocks.len()];
    for (index, block) in graph.blocks.iter().enumerate() {
        match &block.kind {
            BlockKind::Integrator { initial } => {
                continuous_offsets[index] = continuous_initial.len();
                continuous_initial.extend(initial);
            }
            BlockKind::UnitDelay { initial } => {
                discrete_offsets[index] = discrete_initial.len();
                discrete_initial.extend(initial);
            }
            _ => {}
        }
    }
    let GeneratedSignals {
        instructions,
        signals,
    } = lower_signals(
        &graph,
        continuous_initial.len(),
        &continuous_offsets,
        &discrete_offsets,
    )?;
    let mut outputs = Vec::new();
    let mut origins = Vec::new();
    for continuous in [true, false] {
        for (index, block) in graph.blocks.iter().enumerate() {
            let ((BlockKind::Integrator { initial }, true)
            | (BlockKind::UnitDelay { initial }, false)) = (&block.kind, continuous)
            else {
                continue;
            };
            let input = &signals[graph.inputs[index][0]];
            if input.len() != initial.len() {
                return Err(width_error(block, "in", initial.len(), input.len()));
            }
            check_output_budget(block, outputs.len(), input.len())?;
            outputs.extend(input);
            origins.extend((0..input.len()).map(|_| Port {
                block: block.id.clone(),
                port: "in".into(),
            }));
        }
    }
    let mut scopes = Vec::new();
    let mut scope_width = 0;
    for (index, block) in graph.blocks.iter().enumerate() {
        if matches!(block.kind, BlockKind::Scope) {
            let input = &signals[graph.inputs[index][0]];
            check_output_budget(block, outputs.len(), input.len())?;
            scopes.push(ScopeInfo {
                block: block.id.clone(),
                offset: scope_width,
                width: input.len(),
            });
            scope_width += input.len();
            outputs.extend(input);
            origins.extend((0..input.len()).map(|_| Port {
                block: block.id.clone(),
                port: "in".into(),
            }));
        }
    }
    let program = Program::new(
        1 + continuous_initial.len() + discrete_initial.len(),
        instructions,
        outputs,
    )
    .map_err(|error| ModelError::new("numerical_ir", error.to_string()))?;
    Ok(CompiledModel {
        name: model.name.clone(),
        settings: model.settings.clone(),
        program,
        continuous_initial,
        discrete_initial,
        scopes,
        origins,
        execution_order: graph
            .order
            .iter()
            .map(|&i| graph.blocks[i].id.clone())
            .collect(),
    })
}

struct GeneratedSignals {
    instructions: Vec<Instruction>,
    signals: Vec<Vec<usize>>,
}

fn lower_signals(
    graph: &Graph<'_>,
    continuous_count: usize,
    continuous_offsets: &[usize],
    discrete_offsets: &[usize],
) -> Result<GeneratedSignals, ModelError> {
    let mut instructions = Vec::new();
    let mut signals: Vec<Vec<usize>> = vec![Vec::new(); graph.blocks.len()];
    let mut signal_components = 0;
    for &index in &graph.order {
        let block = graph.blocks[index];
        let input_signals: Vec<&[usize]> = graph.inputs[index]
            .iter()
            .map(|&i| signals[i].as_slice())
            .collect();
        check_signal_budget(
            block,
            &input_signals,
            instructions.len(),
            &mut signal_components,
        )?;
        let signal = match &block.kind {
            BlockKind::Constant { value } => value
                .iter()
                .map(|&v| push(&mut instructions, Instruction::Constant(v)))
                .collect(),
            BlockKind::Integrator { initial } => (0..initial.len())
                .map(|i| {
                    push(
                        &mut instructions,
                        Instruction::Input(1 + continuous_offsets[index] + i),
                    )
                })
                .collect(),
            BlockKind::UnitDelay { initial } => (0..initial.len())
                .map(|i| {
                    push(
                        &mut instructions,
                        Instruction::Input(1 + continuous_count + discrete_offsets[index] + i),
                    )
                })
                .collect(),
            BlockKind::Gain { gain } => {
                let input = input_signals[0];
                if gain.len() != 1 && gain.len() != input.len() {
                    return Err(width_error(block, "in", input.len(), gain.len()));
                }
                input
                    .iter()
                    .enumerate()
                    .map(|(i, &value)| {
                        let coefficient = push(
                            &mut instructions,
                            Instruction::Constant(gain[if gain.len() == 1 { 0 } else { i }]),
                        );
                        push(&mut instructions, Instruction::Multiply(value, coefficient))
                    })
                    .collect()
            }
            BlockKind::Sum { signs } => {
                let width = input_signals[0].len();
                for (port, input) in input_signals.iter().enumerate() {
                    if input.len() != width {
                        return Err(width_error(block, &format!("in{port}"), width, input.len()));
                    }
                }
                (0..width)
                    .map(|i| {
                        let mut terms = input_signals
                            .iter()
                            .zip(signs)
                            .map(|(input, sign)| (input[i], *sign));
                        let (first, sign) = terms.next().expect("validated nonempty Sum");
                        let mut result = if sign == 1 {
                            first
                        } else {
                            push(&mut instructions, Instruction::Negate(first))
                        };
                        for (value, sign) in terms {
                            let term = if sign == 1 {
                                value
                            } else {
                                push(&mut instructions, Instruction::Negate(value))
                            };
                            result = push(&mut instructions, Instruction::Add(result, term));
                        }
                        result
                    })
                    .collect()
            }
            BlockKind::Scope => Vec::new(),
        };
        signals[index] = signal;
    }
    Ok(GeneratedSignals {
        instructions,
        signals,
    })
}

fn check_signal_budget(
    block: &Block,
    inputs: &[&[usize]],
    instructions: usize,
    stored: &mut usize,
) -> Result<(), ModelError> {
    let (width, operations_per_element) = match &block.kind {
        BlockKind::Constant { value } => (value.len(), 1),
        BlockKind::Integrator { initial } | BlockKind::UnitDelay { initial } => (initial.len(), 1),
        BlockKind::Gain { .. } => (inputs[0].len(), 2),
        BlockKind::Sum { signs } => (
            inputs[0].len(),
            signs.len() - 1 + signs.iter().filter(|&&s| s == -1).count(),
        ),
        BlockKind::Scope => (0, 0),
    };
    if instructions.saturating_add(width.saturating_mul(operations_per_element)) > MAX_VALUES
        || stored.saturating_add(width) > MAX_VALUES
    {
        return Err(ModelError::new(
            "model_limit",
            "model exceeds the numerical instruction or signal storage limit",
        )
        .at(&block.id, None));
    }
    *stored += width;
    Ok(())
}

fn check_output_budget(block: &Block, used: usize, added: usize) -> Result<(), ModelError> {
    if used.saturating_add(added) > MAX_COMPONENTS {
        return Err(
            ModelError::new("model_limit", "too many state and scope components")
                .at(&block.id, Some("in")),
        );
    }
    Ok(())
}

fn push(instructions: &mut Vec<Instruction>, instruction: Instruction) -> usize {
    let id = instructions.len();
    instructions.push(instruction);
    id
}

fn width_error(block: &Block, port: &str, expected: usize, actual: usize) -> ModelError {
    ModelError::new(
        "signal_width",
        format!("expected width {expected}, found {actual}"),
    )
    .at(&block.id, Some(port))
}

fn validate_model(model: &Model) -> Result<(), ModelError> {
    if model.schema_version != SCHEMA_VERSION {
        return Err(ModelError::new(
            "schema_version",
            "unsupported model schema version",
        ));
    }
    if model.blocks.is_empty()
        || model.blocks.len() > MAX_BLOCKS
        || model.connections.len() > MAX_BLOCKS * MAX_SUM_INPUTS
    {
        return Err(ModelError::new(
            "model_limit",
            "model has an invalid block or connection count",
        ));
    }
    validate_settings(&model.settings)?;
    let s = &model.settings;
    let mut components = 0;
    let mut ids = BTreeSet::new();
    for block in &model.blocks {
        if block.id.is_empty()
            || block.id.len() > 128
            || !block
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(ModelError::new(
                "block_id",
                "block IDs must contain 1..128 ASCII letters, digits, underscores or hyphens",
            )
            .at(&block.id, None));
        }
        if !ids.insert(&block.id) {
            return Err(
                ModelError::new("duplicate_block", "duplicate block ID").at(&block.id, None)
            );
        }
        if block
            .position
            .as_ref()
            .is_some_and(|p| !p.x.is_finite() || !p.y.is_finite())
        {
            return Err(
                ModelError::new("position", "editor position must be finite").at(&block.id, None),
            );
        }
        let values = match &block.kind {
            BlockKind::Constant { value } => Some(value),
            BlockKind::Gain { gain } => Some(gain),
            BlockKind::Integrator { initial } => Some(initial),
            BlockKind::UnitDelay { initial } => {
                if s.sample_time.is_none() {
                    return Err(ModelError::new(
                        "sample_time",
                        "UnitDelay requires settings.sampleTime",
                    )
                    .at(&block.id, None));
                }
                Some(initial)
            }
            BlockKind::Sum { signs } => {
                if signs.is_empty()
                    || signs.len() > MAX_SUM_INPUTS
                    || signs.iter().any(|&s| s != 1 && s != -1)
                {
                    return Err(ModelError::new(
                        "sum_signs",
                        "Sum requires 1..64 signs, each 1 or -1",
                    )
                    .at(&block.id, None));
                }
                None
            }
            BlockKind::Scope => None,
        };
        if let Some(values) = values {
            if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
                return Err(ModelError::new(
                    "parameter",
                    "values must be nonempty finite double vectors",
                )
                .at(&block.id, None));
            }
            components += values.len();
            if components > MAX_COMPONENTS {
                return Err(ModelError::new(
                    "model_limit",
                    "too many parameter and state components",
                ));
            }
        }
    }
    Ok(())
}

fn validate_settings(s: &Settings) -> Result<(), ModelError> {
    if !s.start_time.is_finite()
        || !s.stop_time.is_finite()
        || s.stop_time <= s.start_time
        || !(s.stop_time - s.start_time).is_finite()
    {
        return Err(ModelError::new(
            "time_range",
            "finite stopTime must be greater than finite startTime",
        ));
    }
    let resolution = 64.0
        * f64::EPSILON
        * s.start_time
            .abs()
            .max(s.stop_time.abs())
            .max(f64::MIN_POSITIVE);
    if !s.max_step.is_finite() || s.max_step <= 0.0 || s.max_step < resolution {
        return Err(ModelError::new(
            "time_step",
            "maxStep must be finite, positive and resolvable over the time range",
        ));
    }
    if let Some(period) = s.sample_time
        && (!period.is_finite() || period <= 0.0 || period < resolution)
    {
        return Err(ModelError::new(
            "sample_time",
            "sampleTime must be finite, positive and resolvable over the time range",
        ));
    }
    Ok(())
}

fn build_graph(model: &Model) -> Result<Graph<'_>, ModelError> {
    let mut blocks: Vec<&Block> = model.blocks.iter().collect();
    blocks.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: BTreeMap<&str, usize> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.as_str(), i))
        .collect();
    let ports: Vec<Vec<String>> = blocks.iter().map(|b| b.kind.inputs()).collect();
    let mut drivers: Vec<Vec<Option<usize>>> = ports.iter().map(|p| vec![None; p.len()]).collect();
    for connection in &model.connections {
        let from = *ids.get(connection.from.block.as_str()).ok_or_else(|| {
            ModelError::new("unknown_block", "source block does not exist")
                .at(&connection.from.block, Some(&connection.from.port))
        })?;
        let to = *ids.get(connection.to.block.as_str()).ok_or_else(|| {
            ModelError::new("unknown_block", "destination block does not exist")
                .at(&connection.to.block, Some(&connection.to.port))
        })?;
        if connection.from.port != "out" || matches!(blocks[from].kind, BlockKind::Scope) {
            return Err(
                ModelError::new("unknown_port", "source output port does not exist")
                    .at(&connection.from.block, Some(&connection.from.port)),
            );
        }
        let port = ports[to]
            .iter()
            .position(|p| p == &connection.to.port)
            .ok_or_else(|| {
                ModelError::new("unknown_port", "destination input port does not exist")
                    .at(&connection.to.block, Some(&connection.to.port))
            })?;
        if drivers[to][port].replace(from).is_some() {
            return Err(ModelError::new(
                "multiple_drivers",
                "input port has more than one connection",
            )
            .at(&connection.to.block, Some(&connection.to.port)));
        }
    }
    let mut inputs = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let mut input = Vec::new();
        for (port, driver) in drivers[index].iter().enumerate() {
            input.push(driver.ok_or_else(|| {
                ModelError::new("missing_input", "required input port is not connected")
                    .at(&block.id, Some(&ports[index][port]))
            })?);
        }
        inputs.push(input);
    }
    let mut dependents = vec![Vec::new(); blocks.len()];
    let mut indegree = vec![0; blocks.len()];
    for (index, block) in blocks.iter().enumerate() {
        if block.kind.direct_feedthrough() {
            indegree[index] = inputs[index].len();
            for &source in &inputs[index] {
                dependents[source].push(index);
            }
        }
    }
    let mut ready: BTreeSet<usize> = indegree
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| (d == 0).then_some(i))
        .collect();
    let mut order = Vec::new();
    while let Some(index) = ready.pop_first() {
        order.push(index);
        for &destination in &dependents[index] {
            indegree[destination] -= 1;
            if indegree[destination] == 0 {
                ready.insert(destination);
            }
        }
    }
    if order.len() != blocks.len() {
        let mut current = indegree
            .iter()
            .position(|&d| d > 0)
            .expect("unprocessed node");
        let mut visited = BTreeSet::new();
        loop {
            let port = inputs[current]
                .iter()
                .position(|&source| indegree[source] > 0)
                .expect("unresolved dependency");
            if !visited.insert(current) {
                return Err(ModelError::new("algebraic_loop", "instantaneous feedback requires an algebraic solver, which v0 does not support").at(&blocks[current].id, Some(&ports[current][port])));
            }
            current = inputs[current][port];
        }
    }
    Ok(Graph {
        blocks,
        inputs,
        order,
    })
}

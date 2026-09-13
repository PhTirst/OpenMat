use super::{Emitter, Node, program};
use crate::{
    ModelError,
    hybrid::{
        ControlOp, EventInfo, EventPlan, LogicOperator, ResetTarget, SignalType, SwitchCriterion,
    },
    model::{BlockKind, SampleTime},
    numeric::{Comparison, Instruction, MathFunction},
    sampling::SamplingPlan,
};
use std::collections::BTreeMap;

fn push(ops: &mut Vec<Instruction>, op: Instruction) -> usize {
    let id = ops.len();
    ops.push(op);
    id
}

pub(super) fn lower(
    op: &ControlOp,
    width: usize,
    inputs: &[Vec<usize>],
    ops: &mut Vec<Instruction>,
) -> Result<Vec<usize>, ModelError> {
    if let ControlOp::Saturation { lower, upper } = op
        && [lower.len(), upper.len()]
            .iter()
            .any(|&n| n != 1 && n != width)
    {
        return Err(ModelError::new(
            "signal_width",
            "saturation bounds must be scalar or match the signal",
        ));
    }
    let mut output = Vec::with_capacity(width);
    let zero = push(ops, Instruction::Constant(0.0));
    for i in 0..width {
        let u: Vec<_> = if matches!(op, ControlOp::MinMax { inputs: 1, .. }) {
            inputs[0].clone()
        } else {
            inputs.iter().map(|v| v[i % v.len()]).collect()
        };
        output.push(match op {
            ControlOp::Abs => push(ops, Instruction::Math(MathFunction::Abs, u[0])),
            ControlOp::Saturation { lower, upper } => {
                let a = push(ops, Instruction::Constant(lower[i % lower.len()]));
                let b = push(ops, Instruction::Constant(upper[i % upper.len()]));
                let low = push(ops, Instruction::Compare(Comparison::Less, u[0], a));
                let high = push(ops, Instruction::Compare(Comparison::Greater, u[0], b));
                let clipped = push(ops, Instruction::Select(low, a, u[0]));
                push(ops, Instruction::Select(high, b, clipped))
            }
            ControlOp::Switch {
                criterion,
                threshold,
            } => {
                let limit = push(
                    ops,
                    Instruction::Constant(if *criterion == SwitchCriterion::Nonzero {
                        0.0
                    } else {
                        *threshold
                    }),
                );
                let comparison = match criterion {
                    SwitchCriterion::Greater => Comparison::Greater,
                    SwitchCriterion::GreaterEqual => Comparison::GreaterEqual,
                    SwitchCriterion::Nonzero => Comparison::NotEqual,
                };
                let condition = push(ops, Instruction::Compare(comparison, u[1], limit));
                push(ops, Instruction::Select(condition, u[0], u[2]))
            }
            ControlOp::Relational { operator } => {
                push(ops, Instruction::Compare(*operator, u[0], u[1]))
            }
            ControlOp::Logical { operator, .. } => {
                let mut value = push(ops, Instruction::Compare(Comparison::NotEqual, u[0], zero));
                for &input in &u[1..] {
                    let bit = push(ops, Instruction::Compare(Comparison::NotEqual, input, zero));
                    value = match operator {
                        LogicOperator::And | LogicOperator::Nand => {
                            push(ops, Instruction::Multiply(value, bit))
                        }
                        LogicOperator::Or | LogicOperator::Nor => {
                            let sum = push(ops, Instruction::Add(value, bit));
                            push(ops, Instruction::Compare(Comparison::Greater, sum, zero))
                        }
                        _ => push(ops, Instruction::Compare(Comparison::NotEqual, value, bit)),
                    };
                }
                if matches!(
                    operator,
                    LogicOperator::Not
                        | LogicOperator::Nand
                        | LogicOperator::Nor
                        | LogicOperator::Nxor
                ) {
                    push(ops, Instruction::Compare(Comparison::Equal, value, zero))
                } else {
                    value
                }
            }
            ControlOp::MinMax { minimum, .. } => {
                let mut value = u[0];
                for &v in &u[1..] {
                    let c = push(
                        ops,
                        Instruction::Compare(
                            if *minimum {
                                Comparison::LessEqual
                            } else {
                                Comparison::GreaterEqual
                            },
                            value,
                            v,
                        ),
                    );
                    value = push(ops, Instruction::Select(c, value, v));
                }
                value
            }
        });
    }
    Ok(output)
}

pub(super) fn events(
    nodes: &[Node<'_>],
    sampling: Option<&SamplingPlan>,
    x: usize,
    q: usize,
    count: usize,
) -> Result<EventPlan, ModelError> {
    validate_reset_dependencies(nodes, x, count)?;
    let sampling = sampling.expect("schema 6 sampling");
    let mut emit = Emitter::new(nodes, x, count);
    emit.sampling = Some(sampling);
    emit.cache_start = 1 + x + q;
    let mut events = Vec::new();
    let mut outputs = Vec::new();
    let signals = signal_types(nodes)?;
    for node in nodes {
        let (operation, reset) = match &node.block.kind {
            BlockKind::Control {
                operation,
                zero_crossing: true,
            } => (Some(operation), None),
            BlockKind::ResetIntegrator {
                initial,
                discrete,
                reset,
                ..
            } => (
                None,
                Some(ResetTarget {
                    initial: initial.clone(),
                    offset: if *discrete {
                        node.q_offset
                    } else {
                        node.x_offset
                    },
                    discrete: *discrete,
                    mode: *reset,
                }),
            ),
            _ => continue,
        };
        let inputs = node
            .drivers
            .iter()
            .map(|&d| emit.signal(d, 0))
            .collect::<Result<Vec<_>, _>>()?;
        let width = node.outputs[0].width;
        let mut surfaces: Vec<(String, usize)> = Vec::new();
        if reset.is_some() {
            surfaces.push(("reset".into(), inputs[1][0]));
        }
        if let Some(operation) = operation {
            control_surfaces(
                operation,
                width,
                &inputs,
                &mut emit,
                &mut surfaces,
                events.len(),
                &node.block.id,
            )?;
        }
        let rate = if reset.as_ref().is_some_and(|r| !r.discrete) {
            sampling.blocks[&nodes[node.drivers[1].block].block.id]
        } else {
            sampling.blocks[&node.block.id]
        };
        for (surface, output) in surfaces {
            outputs.push(output);
            events.push(EventInfo {
                block: node.block.id.clone(),
                surface,
                sample_time: rate,
                locate: rate == SampleTime::Continuous,
                reset: reset.clone(),
            });
        }
        if events.len() > 256 {
            return Err(ModelError::new("event_limit", "at most 256 event surfaces are supported; disable unnecessary zero-crossing detection").at(&node.block.id, None));
        }
    }
    Ok(EventPlan {
        signals,
        events,
        program: program(count, emit.instructions, outputs)?,
    })
}

#[allow(clippy::too_many_arguments)] // Operates on one bounded event emitter and its surface budget.
fn control_surfaces(
    operation: &ControlOp,
    width: usize,
    inputs: &[Vec<usize>],
    emit: &mut Emitter<'_>,
    surfaces: &mut Vec<(String, usize)>,
    existing: usize,
    id: &str,
) -> Result<(), ModelError> {
    for i in 0..width {
        let u: Vec<_> = if matches!(operation, ControlOp::MinMax { inputs: 1, .. }) {
            inputs[0].clone()
        } else {
            inputs.iter().map(|v| v[i % v.len()]).collect()
        };
        let mut pairs = Vec::new();
        match operation {
            ControlOp::Saturation { lower, upper } => {
                for (label, bound) in [
                    ("lower", lower[i % lower.len()]),
                    ("upper", upper[i % upper.len()]),
                ] {
                    let v = emit.push(Instruction::Constant(bound))?;
                    pairs.push((label.to_owned(), u[0], v));
                }
            }
            ControlOp::Switch {
                criterion,
                threshold,
            } => {
                let v = emit.push(Instruction::Constant(
                    if *criterion == SwitchCriterion::Nonzero {
                        0.0
                    } else {
                        *threshold
                    },
                ))?;
                pairs.push(("threshold".into(), u[1], v));
            }
            ControlOp::Relational { .. } => pairs.push(("comparison".into(), u[0], u[1])),
            ControlOp::Abs => {
                surfaces.push((format!("zero[{i}]"), u[0]));
            }
            ControlOp::Logical { .. } => {}
            ControlOp::MinMax { .. } => {
                if u.len() * u.len().saturating_sub(1) / 2 + existing + surfaces.len() > 256 {
                    return Err(ModelError::new(
                        "event_limit",
                        "too many MinMax crossing surfaces; disable zero-crossing detection",
                    )
                    .at(id, None));
                }
                for a in 0..u.len() {
                    for b in a + 1..u.len() {
                        pairs.push((format!("input{a}-{b}"), u[a], u[b]));
                    }
                }
            }
        }
        for (label, a, b) in pairs {
            surfaces.push((
                format!("{label}[{i}]"),
                emit.push(Instruction::Subtract(a, b))?,
            ));
        }
    }
    Ok(())
}

#[allow(clippy::float_cmp)] // Logical state encodings must be exactly 0 or 1.
fn signal_types(nodes: &[Node<'_>]) -> Result<BTreeMap<String, SignalType>, ModelError> {
    let mut signals: BTreeMap<_, _> = nodes
        .iter()
        .map(|n| {
            (
                n.block.id.clone(),
                if matches!(
                    n.block.kind,
                    BlockKind::Control {
                        operation: ControlOp::Relational { .. } | ControlOp::Logical { .. },
                        ..
                    }
                ) {
                    SignalType::Logical
                } else {
                    SignalType::Double
                },
            )
        })
        .collect();
    for _ in 0..nodes.len() {
        let mut changed = false;
        for node in nodes {
            let driver = match node.block.kind {
                BlockKind::Scope
                | BlockKind::ZeroOrderHold
                | BlockKind::UnitDelay { .. }
                | BlockKind::RateTransition { .. }
                | BlockKind::Control {
                    operation: ControlOp::Switch { .. },
                    ..
                } => node.drivers.first(),
                _ => None,
            };
            if let Some(driver) = driver {
                let value = signals[&nodes[driver.block].block.id];
                if signals[&node.block.id] != value {
                    signals.insert(node.block.id.clone(), value);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    for node in nodes {
        if signals[&node.block.id] == SignalType::Logical
            && matches!(
                node.block.kind,
                BlockKind::UnitDelay { .. } | BlockKind::RateTransition { .. }
            )
            && node.q.iter().any(|&v| v != 0.0 && v != 1.0)
        {
            return Err(ModelError::new(
                "signal_type",
                "logical state initial values must be 0 or 1",
            )
            .at(&node.block.id, None));
        }
        if matches!(
            node.block.kind,
            BlockKind::Control {
                operation: ControlOp::Switch { .. },
                ..
            }
        ) && signals[&nodes[node.drivers[0].block].block.id]
            != signals[&nodes[node.drivers[2].block].block.id]
        {
            return Err(ModelError::new(
                "signal_type",
                "Switch data inputs must have the same double/logical type",
            )
            .at(&node.block.id, None));
        }
    }
    Ok(signals)
}

// Trace the pruned instantaneous expressions, including sample-and-hold inputs.
// A stateful delay breaks the path; cached outputs alone must not hide a loop.
fn validate_reset_dependencies(
    nodes: &[Node<'_>],
    x: usize,
    count: usize,
) -> Result<(), ModelError> {
    let reset_nodes: Vec<_> = nodes
        .iter()
        .filter(|n| matches!(n.block.kind, BlockKind::ResetIntegrator { .. }))
        .collect();
    if reset_nodes.len() > 256 {
        return Err(ModelError::new(
            "event_limit",
            "at most 256 reset event surfaces are supported",
        ));
    }
    let mut work = 0;
    let mut dependencies = vec![Vec::new(); reset_nodes.len()];
    for (i, node) in reset_nodes.iter().enumerate() {
        let mut emit = Emitter::new(nodes, x, count);
        let outputs = emit.signal(node.drivers[1], 0)?;
        work += emit.instructions.len();
        if work > crate::numeric::MAX_VALUES * 4 {
            return Err(ModelError::new(
                "event_limit",
                "reset dependency compilation budget exceeded",
            )
            .at(&node.block.id, Some("reset")));
        }
        let expression = program(count, emit.instructions, outputs)?;
        for (j, target) in reset_nodes.iter().enumerate() {
            let start = if target.x.is_empty() {
                1 + x + target.q_offset
            } else {
                1 + target.x_offset
            };
            let end = start + target.x.len() + target.q.len();
            if expression
                .instructions()
                .iter()
                .any(|op| matches!(op, Instruction::Input(index) if (start..end).contains(index)))
            {
                dependencies[i].push(j);
            }
        }
    }
    let mut remaining = vec![true; reset_nodes.len()];
    loop {
        let mut progress = false;
        for i in 0..remaining.len() {
            if remaining[i] && dependencies[i].iter().all(|&j| !remaining[j]) {
                remaining[i] = false;
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    if let Some(i) = remaining.iter().position(|&v| v) {
        return Err(ModelError::new(
            "reset_algebraic_loop",
            "reset input depends instantaneously on a reset state cycle; insert a stateful delay",
        )
        .at(&reset_nodes[i].block.id, Some("reset")));
    }
    Ok(())
}

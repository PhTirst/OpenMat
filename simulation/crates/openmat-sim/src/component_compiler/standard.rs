use super::{Node, failure, program};
use crate::{
    ModelError,
    authoring::{StandardOp, TimeSeries},
    numeric::{Comparison, Instruction},
};

fn push(ops: &mut Vec<Instruction>, op: Instruction) -> usize {
    let id = ops.len();
    ops.push(op);
    id
}

pub(super) fn lower(node: &mut Node<'_>, op: &StandardOp) -> Result<(), ModelError> {
    let realization = op.realization();
    if let Some(r) = &realization {
        node.x.clone_from(&r.initial);
    }
    let start = 1 + node.x.len();
    let count = start + node.inputs.iter().map(|p| p.width).sum::<usize>();
    let mut ops: Vec<_> = (0..count).map(Instruction::Input).collect();
    let mut offset = start;
    let inputs: Vec<Vec<_>> = node
        .inputs
        .iter()
        .map(|p| {
            let ids = (offset..offset + p.width).collect();
            offset += p.width;
            ids
        })
        .collect();
    let outputs: Vec<Vec<usize>> = match op {
        StandardOp::Product { operations } => {
            let width = node.outputs[0].width;
            if inputs.iter().any(|u| u.len() != 1 && u.len() != width) {
                return Err(failure(
                    node.block,
                    "signal_width",
                    "Product inputs must be scalar or equal width",
                ));
            }
            vec![
                (0..width)
                    .map(|i| {
                        let mut v = push(&mut ops, Instruction::Constant(1.0));
                        for (u, operation) in inputs.iter().zip(operations.bytes()) {
                            let x = u[i % u.len()];
                            v = push(
                                &mut ops,
                                if operation == b'*' {
                                    Instruction::Multiply(v, x)
                                } else {
                                    Instruction::Divide(v, x)
                                },
                            );
                        }
                        v
                    })
                    .collect(),
            ]
        }
        StandardOp::Mux { .. } => vec![inputs.into_iter().flatten().collect()],
        StandardOp::Demux { widths } => {
            let mut start = 0;
            widths
                .iter()
                .map(|n| {
                    let v = inputs[0][start..start + n].to_vec();
                    start += n;
                    v
                })
                .collect()
        }
        StandardOp::StateSpace { .. } | StandardOp::TransferFcn { .. } => {
            let r = realization.expect("state space");
            let mut linear = |state: &[f64], input: &[f64]| {
                let mut sum = push(&mut ops, Instruction::Constant(0.0));
                for (&coefficient, value) in state
                    .iter()
                    .zip(1..start)
                    .chain(input.iter().zip(inputs[0].iter().copied()))
                {
                    if coefficient != 0.0 {
                        let c = push(&mut ops, Instruction::Constant(coefficient));
                        let term = push(&mut ops, Instruction::Multiply(c, value));
                        sum = push(&mut ops, Instruction::Add(sum, term));
                    }
                }
                sum
            };
            let derivatives =
                r.a.iter()
                    .zip(&r.b)
                    .map(|(a, b)| linear(a, b))
                    .collect::<Vec<_>>();
            let values = r.c.iter().zip(&r.d).map(|(c, d)| linear(c, d)).collect();
            if !derivatives.is_empty() {
                node.derivatives = Some(program(count, ops.clone(), derivatives)?);
            }
            vec![values]
        }
    };
    for values in outputs {
        node.signals.push(program(count, ops.clone(), values)?);
    }
    Ok(())
}

pub(super) fn input(node: &mut Node<'_>, data: &TimeSeries) -> Result<(), ModelError> {
    let mut ops = vec![Instruction::Input(0)];
    let mut values: Vec<_> = data.values[0]
        .iter()
        .map(|v| push(&mut ops, Instruction::Constant(*v)))
        .collect();
    // Linear segments and endpoint holds are evaluated without changing state.
    for (times, rows) in data.times.windows(2).zip(data.values.windows(2)) {
        let at = push(&mut ops, Instruction::Constant(times[0]));
        let end = push(&mut ops, Instruction::Constant(times[1]));
        let active = push(
            &mut ops,
            Instruction::Compare(Comparison::GreaterEqual, 0, at),
        );
        let past = push(
            &mut ops,
            Instruction::Compare(Comparison::GreaterEqual, 0, end),
        );
        let clamped = push(&mut ops, Instruction::Select(past, end, 0));
        let clamped = push(&mut ops, Instruction::Select(active, clamped, at));
        let delta = push(&mut ops, Instruction::Subtract(clamped, at));
        for (i, (&a, &b)) in rows[0].iter().zip(&rows[1]).enumerate() {
            let slope = push(
                &mut ops,
                Instruction::Constant((b - a) / (times[1] - times[0])),
            );
            let a = push(&mut ops, Instruction::Constant(a));
            let scaled = push(&mut ops, Instruction::Multiply(slope, delta));
            let y = push(&mut ops, Instruction::Add(a, scaled));
            // The explicit endpoint avoids extrapolation and interpolation roundoff there.
            let b = push(&mut ops, Instruction::Constant(b));
            let y = push(&mut ops, Instruction::Select(past, b, y));
            values[i] = push(&mut ops, Instruction::Select(active, y, values[i]));
        }
    }
    node.signals.push(program(1, ops, values)?);
    Ok(())
}

use super::{Emitter, Node, Wire, failure};
use crate::{
    ModelError,
    conditional::{ConditionalPlan, DomainPlan, DomainSpec, Execution, HoldReset},
    model::{BlockKind, Model, SampleTime},
    numeric::{Comparison, Instruction},
    sampling::SamplingPlan,
};
use std::collections::BTreeMap;

pub(super) fn configure(
    model: &Model,
    nodes: &[Node<'_>],
    domains: &[DomainSpec],
) -> Result<Model, ModelError> {
    let mut configured = model.clone();
    let by_id: BTreeMap<_, _> = nodes.iter().map(|n| (n.block.id.as_str(), n)).collect();
    for domain in domains {
        let period = domain.execution.period();
        for id in &domain.members {
            let node = by_id[id.as_str()];
            if !node.x.is_empty()
                || matches!(
                    node.block.kind,
                    BlockKind::ResetIntegrator { .. } | BlockKind::RateTransition { .. }
                )
            {
                return Err(failure(
                    node.block,
                    "conditional_body",
                    "conditional-v1 bodies require discrete states and do not support reset integrators or rate transitions",
                ));
            }
            let declared = model.sample_times.get(id);
            if matches!(domain.execution, Execution::Triggered { .. }) {
                if declared
                    .is_some_and(|r| !matches!(r, SampleTime::Inherited | SampleTime::Constant))
                    || node
                        .definition
                        .is_some_and(|d| d.sample_time.is_some_and(|p| p > 0.0))
                    || matches!(
                        node.block.kind,
                        BlockKind::DiscreteIntegrator { .. } | BlockKind::ZeroOrderHold
                    )
                {
                    return Err(failure(
                        node.block,
                        "triggered_sample_time",
                        "triggered bodies inherit invocation; explicit periodic blocks, holds and periodic integrators are not supported",
                    ));
                }
            } else if declared.is_some_and(|r| match r {
                SampleTime::Continuous => true,
                SampleTime::Discrete { period: p } => {
                    (p - period).abs() > 32.0 * f64::EPSILON * period
                }
                _ => false,
            }) {
                return Err(failure(
                    node.block,
                    "enabled_sample_time",
                    "enabled body blocks must use the domain's single discrete period",
                ));
            }
            configured
                .sample_times
                .insert(id.clone(), SampleTime::Discrete { period });
        }
    }
    Ok(configured)
}

pub(super) fn layout(
    nodes: &[Node<'_>],
    domains: &[DomainSpec],
    plan: &mut SamplingPlan,
    states: usize,
) -> Result<(), ModelError> {
    if domains.is_empty() {
        return Ok(());
    }
    let ids: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.block.id.as_str(), i))
        .collect();
    let mut result = ConditionalPlan {
        domains: vec![],
        node_domains: vec![None; nodes.len()],
        immediate_states: vec![false; states],
        initial_cache: vec![],
        scopes: BTreeMap::new(),
    };
    for (index, domain) in domains.iter().enumerate() {
        let mut outputs = BTreeMap::new();
        for (id, policy) in domain.outports.iter().zip(domain.execution.outputs()) {
            let node = ids[id.as_str()];
            let width = nodes[node].outputs[0].width;
            if policy.initial.len() != 1 && policy.initial.len() != width {
                return Err(failure(
                    nodes[node].block,
                    "conditional_output",
                    "initial output must be scalar or match the output width",
                ));
            }
            let values = (0..width)
                .map(|i| policy.initial[i % policy.initial.len()])
                .collect();
            result
                .initial_cache
                .push((plan.output_offsets[node][0].expect("domain cache"), values));
            outputs.insert(node, policy.clone());
        }
        let clock = plan.node_clocks[ids[domain.members[0].as_str()]].expect("domain clock");
        for id in &domain.members {
            let node = ids[id.as_str()];
            result.node_domains[node] = Some(index);
            for state in nodes[node].q_offset..nodes[node].q_offset + nodes[node].q.len() {
                result.immediate_states[state] = true;
            }
        }
        for id in &domain.scopes {
            result.scopes.insert(ids[id.as_str()], domain.id.clone());
        }
        let source = ids[domain.control.block.as_str()];
        let port = nodes[source]
            .outputs
            .iter()
            .position(|p| p.name == domain.control.port)
            .ok_or_else(|| {
                failure(
                    nodes[source].block,
                    "conditional_control",
                    "control output port is missing",
                )
            })?;
        if nodes[source].outputs[port].width != 1 {
            return Err(failure(
                nodes[source].block,
                "conditional_control",
                "enable/trigger signals must be scalar",
            ));
        }
        // The declared domain period is an explicit sampling boundary for the control.
        if let SampleTime::Discrete { period } = plan.blocks[&domain.control.block]
            && (period - plan.clocks[clock].period).abs() > 32.0 * f64::EPSILON * period
        {
            return Err(failure(
                nodes[source].block,
                "conditional_control_rate",
                "sample the control at the domain period before connecting enable/trigger",
            ));
        }
        result.domains.push(DomainPlan {
            id: domain.id.clone(),
            execution: domain.execution.clone(),
            control: (source, port),
            clock,
            memory: plan.cache_count,
            outputs,
        });
        plan.cache_count += 4;
    }
    if plan.cache_count > 262_144 {
        return Err(ModelError::new(
            "model_limit",
            "conditional cache budget exceeded",
        ));
    }
    plan.conditional = Some(result);
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct Gate {
    pub active: usize,
    pub reset: usize,
    pub hit: usize,
    sign: usize,
    zero_edge: usize,
}

impl Emitter<'_> {
    // R2022b forward-Euler integration holds the last emitted integral when an
    // enabled domain stops. Its speculative next increment is discarded; a
    // Unit Delay or explicit m update instead retains its next-call state.
    pub(super) fn inactive_state(
        &mut self,
        node: usize,
        element: usize,
        old: usize,
    ) -> Result<usize, ModelError> {
        let Some(domain) = self.domain(node) else {
            return Ok(old);
        };
        let sampling = self.sampling.expect("domain sampling");
        let plan = &sampling.conditional.as_ref().expect("domains").domains[domain];
        if !matches!(
            self.nodes[node].block.kind,
            BlockKind::DiscreteIntegrator { .. }
        ) || !matches!(plan.execution, Execution::Enabled { .. })
        {
            return Ok(old);
        }
        let previous_sign = self.push(Instruction::Input(self.cache_start + plan.memory))?;
        let zero = self.push(Instruction::Constant(0.0))?;
        let was_enabled = self.push(Instruction::Compare(
            Comparison::Greater,
            previous_sign,
            zero,
        ))?;
        let hit = self.gate(domain)?.hit;
        let disabling = self.push(Instruction::Multiply(hit, was_enabled))?;
        let cached = self.push(Instruction::Input(
            self.cache_start + sampling.output_offsets[node][0].expect("integral cache") + element,
        ))?;
        self.push(Instruction::Select(disabling, cached, old))
    }

    pub(super) fn domain(&self, node: usize) -> Option<usize> {
        self.sampling
            .and_then(|s| s.conditional.as_ref())
            .and_then(|p| p.node_domains[node])
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn gate(&mut self, domain: usize) -> Result<Gate, ModelError> {
        if let Some(gate) = self.gates.get(&domain) {
            return Ok(*gate);
        }
        let sampling = self.sampling.expect("domain sampling");
        let plan = &sampling.conditional.as_ref().expect("domains").domains[domain];
        if !self.gate_visiting.insert(domain) {
            return Err(ModelError::new("conditional_cycle", "control depends instantaneously on its own conditional outputs; use a stateful delay outside the domain").at(&plan.id, None));
        }
        let u = self.signal(
            Wire {
                block: plan.control.0,
                port: plan.control.1,
            },
            0,
        )?[0];
        let zero = self.push(Instruction::Constant(0.0))?;
        let pos = self.push(Instruction::Compare(Comparison::Greater, u, zero))?;
        let neg = self.push(Instruction::Compare(Comparison::Less, u, zero))?;
        let sign = self.push(Instruction::Subtract(pos, neg))?;
        let old = self.push(Instruction::Input(self.cache_start + plan.memory))?;
        let old_edge = self.push(Instruction::Input(self.cache_start + plan.memory + 1))?;
        let hit = self.push(Instruction::Input(
            self.cache_start + sampling.cache_count + plan.clock,
        ))?;
        let rising = self.push(Instruction::Compare(Comparison::Greater, sign, old))?;
        let falling = self.push(Instruction::Compare(Comparison::Less, sign, old))?;
        let direction = self.push(Instruction::Subtract(rising, falling))?;
        let old_zero = self.push(Instruction::Compare(Comparison::Equal, old, zero))?;
        let repeated = self.push(Instruction::Compare(Comparison::Equal, old_edge, direction))?;
        let suppress = self.push(Instruction::Multiply(old_zero, repeated))?;
        let direction = self.push(Instruction::Select(suppress, zero, direction))?;
        let at_zero = self.push(Instruction::Compare(Comparison::Equal, sign, zero))?;
        let zero_edge = self.push(Instruction::Select(at_zero, direction, zero))?;
        let (active, reset) = match &plan.execution {
            Execution::Enabled {
                states_when_enabling,
                ..
            } => {
                let active = self.push(Instruction::Multiply(hit, pos))?;
                let was_off = self.push(Instruction::Compare(Comparison::LessEqual, old, zero))?;
                let reset = if *states_when_enabling == HoldReset::Reset {
                    self.push(Instruction::Multiply(active, was_off))?
                } else {
                    zero
                };
                (active, reset)
            }
            Execution::Triggered { edge, .. } => {
                let comparison = match edge {
                    crate::hybrid::ResetMode::Rising => Comparison::Greater,
                    crate::hybrid::ResetMode::Falling => Comparison::Less,
                    crate::hybrid::ResetMode::Either => Comparison::NotEqual,
                };
                let detected = self.push(Instruction::Compare(comparison, direction, zero))?;
                let time = self.push(Instruction::Input(0))?;
                let started = self.push(Instruction::Compare(Comparison::Greater, time, zero))?;
                let detected = self.push(Instruction::Multiply(detected, started))?;
                (self.push(Instruction::Multiply(detected, hit))?, zero)
            }
        };
        let sign = self.push(Instruction::Select(hit, sign, old))?;
        let zero_edge = self.push(Instruction::Select(hit, zero_edge, old_edge))?;
        let gate = Gate {
            active,
            reset,
            hit,
            sign,
            zero_edge,
        };
        self.gate_visiting.remove(&domain);
        self.gates.insert(domain, gate);
        Ok(gate)
    }

    pub(super) fn domain_memory(&mut self) -> Result<Vec<(String, [usize; 4])>, ModelError> {
        let ids: Vec<_> = self
            .sampling
            .and_then(|s| s.conditional.as_ref())
            .map_or_else(Vec::new, |p| {
                p.domains.iter().map(|d| d.id.clone()).collect()
            });
        ids.into_iter()
            .enumerate()
            .map(|(i, id)| {
                let g = self.gate(i)?;
                Ok((id, [g.sign, g.zero_edge, g.active, g.reset]))
            })
            .collect()
    }
}

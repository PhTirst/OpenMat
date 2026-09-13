use super::{Node, failure, lower_builtin};
use crate::{
    ModelError, SourceBundle,
    model::{BlockKind, Model, SampleTime},
    sampling::{SampleClock, SamplingPlan},
};
use std::collections::{BTreeMap, BTreeSet};

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ticks(period: f64, base: f64) -> Result<u64, ModelError> {
    let ratio = period / base;
    if !ratio.is_finite()
        || !(1.0..=1_000_000_000.0).contains(&ratio.round())
        || (ratio - ratio.round()).abs() > 32.0 * f64::EPSILON * ratio.abs().max(1.0)
    {
        return Err(ModelError::new(
            "sample_grid",
            "period must be a positive integer multiple of maxStep (at most 1e9 base ticks)",
        ));
    }
    Ok(ratio.round() as u64)
}

#[allow(clippy::cast_precision_loss)] // Grid multipliers are bounded to 1e9.
fn canonical(rate: SampleTime, base: f64) -> Result<SampleTime, ModelError> {
    Ok(match rate {
        SampleTime::Discrete { period } => SampleTime::Discrete {
            period: ticks(period, base)? as f64 * base,
        },
        other => other,
    })
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[allow(clippy::cast_precision_loss)]
fn combine(rates: impl Iterator<Item = SampleTime>, base: f64) -> Result<SampleTime, ModelError> {
    let mut period = None;
    let mut continuous = false;
    for rate in rates {
        match rate {
            SampleTime::Inherited => return Ok(SampleTime::Inherited),
            SampleTime::Continuous => continuous = true,
            SampleTime::Constant => {}
            SampleTime::Discrete { period: p } => {
                let next = ticks(p, base)?;
                period = Some(period.map_or(next, |old| gcd(old, next)));
            }
        }
    }
    Ok(if continuous {
        SampleTime::Continuous
    } else {
        period.map_or(SampleTime::Constant, |n| SampleTime::Discrete {
            period: n as f64 * base,
        })
    })
}

fn is_constant(node: &Node<'_>) -> bool {
    node.x.is_empty()
        && node.q.is_empty()
        && !node.signals.is_empty()
        && node.signals.iter().all(|p| {
            p.instructions()
                .iter()
                .all(|i| !matches!(i, crate::numeric::Instruction::Input(_)))
        })
}

fn must_be_discrete(node: &Node<'_>) -> bool {
    matches!(
        node.block.kind,
        BlockKind::UnitDelay { .. }
            | BlockKind::DiscreteIntegrator { .. }
            | BlockKind::ZeroOrderHold
            | BlockKind::RateTransition { .. }
    ) || (!node.q.is_empty() && node.x.is_empty())
}

#[allow(clippy::too_many_lines)]
pub(super) fn resolve(
    model: &Model,
    nodes: &mut [Node<'_>],
) -> Result<Vec<SampleTime>, ModelError> {
    if model.settings.start_time != 0.0 {
        return Err(ModelError::new(
            "sample_time",
            "schema 5 currently requires startTime=0 and zero sampling offsets",
        ));
    }
    let base = model.settings.max_step;
    ticks(model.settings.stop_time, base)?;
    let ids: BTreeSet<_> = nodes.iter().map(|n| n.block.id.as_str()).collect();
    if let Some(id) = model
        .sample_times
        .keys()
        .find(|id| !ids.contains(id.as_str()))
    {
        return Err(
            ModelError::new("sample_time", "sampleTimes references an unknown block").at(id, None),
        );
    }
    let mut rates = Vec::with_capacity(nodes.len());
    let mut inherited = Vec::with_capacity(nodes.len());
    for node in nodes.iter() {
        if !node.x.is_empty()
            && !node.q.is_empty()
            && !node
                .definition
                .and_then(|d| d.sample_time)
                .is_some_and(|p| p > 0.0)
        {
            return Err(failure(
                node.block,
                "sample_time",
                "a mixed continuous/discrete component requires an explicit positive state-update period",
            ));
        }
        let configured = model.sample_times.get(&node.block.id).copied();
        let declared = if !node.x.is_empty() {
            Some(SampleTime::Continuous)
        } else if let Some(period) = node
            .definition
            .and_then(|d| d.sample_time)
            .filter(|p| *p > 0.0)
        {
            Some(SampleTime::Discrete { period })
        } else if configured.is_none() && matches!(node.block.kind, BlockKind::UnitDelay { .. }) {
            model
                .settings
                .sample_time
                .map(|period| SampleTime::Discrete { period })
        } else if configured.is_none() && is_constant(node) {
            Some(SampleTime::Constant)
        } else {
            None
        };
        let rate = canonical(
            configured
                .filter(|r| *r != SampleTime::Inherited)
                .or(declared)
                .unwrap_or(SampleTime::Inherited),
            base,
        )
        .map_err(|e| e.at(&node.block.id, None))?;
        if !node.x.is_empty() && rate != SampleTime::Continuous {
            return Err(failure(
                node.block,
                "sample_time",
                "continuous state outputs cannot be sampled by overriding their rate; insert a sampling block",
            ));
        }
        if rate == SampleTime::Constant
            && !is_constant(node)
            && !matches!(node.block.kind, BlockKind::Scope)
        {
            return Err(failure(
                node.block,
                "sample_time",
                "a time, state or input dependent block cannot declare constant sample time",
            ));
        }
        if let Some(d) = node.definition
            && d.sample_time.is_some_and(|p| p > 0.0)
            && !node.q.is_empty()
            && node.x.is_empty()
            && rate
                != canonical(
                    SampleTime::Discrete {
                        period: d.sample_time.expect("validated component clock"),
                    },
                    base,
                )?
        {
            return Err(failure(
                node.block,
                "sample_time",
                "instance sampling must match the component's declared discrete period",
            ));
        }
        inherited.push(rate == SampleTime::Inherited);
        rates.push(rate);
    }
    let mut consumers = vec![Vec::new(); nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        for wire in &node.drivers {
            consumers[wire.block].push(i);
        }
    }
    let mut work = 0usize;
    for stage in 0..2 {
        for _ in 0..128 {
            let mut changed = false;
            for (i, node) in nodes.iter().enumerate() {
                if rates[i] != SampleTime::Inherited {
                    continue;
                }
                work += 1 + node.drivers.len() + consumers[i].len();
                if work > 4_000_000 {
                    return Err(ModelError::new(
                        "model_limit",
                        "sample-time propagation work exceeds its bounded profile",
                    ));
                }
                let forward = if node.drivers.is_empty() {
                    SampleTime::Inherited
                } else {
                    combine(node.drivers.iter().map(|w| rates[w.block]), base)?
                };
                let next = if forward != SampleTime::Inherited
                    && !(must_be_discrete(node) && forward == SampleTime::Constant)
                {
                    forward
                } else {
                    let known: Vec<_> = consumers[i]
                        .iter()
                        .map(|&j| rates[j])
                        .filter(|r| !matches!(r, SampleTime::Inherited | SampleTime::Constant))
                        .collect();
                    if known.is_empty() {
                        SampleTime::Inherited
                    } else {
                        combine(known.into_iter(), base)?
                    }
                };
                if next != SampleTime::Inherited {
                    rates[i] = next;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        if stage == 0 {
            for (i, node) in nodes.iter().enumerate() {
                if rates[i] == SampleTime::Inherited && node.drivers.is_empty() {
                    rates[i] = if is_constant(node) {
                        SampleTime::Constant
                    } else {
                        SampleTime::Continuous
                    };
                }
            }
        }
    }
    for (i, node) in nodes.iter().enumerate() {
        let rate = rates[i];
        if rate == SampleTime::Inherited
            || must_be_discrete(node) && !matches!(rate, SampleTime::Discrete { .. })
        {
            return Err(failure(
                node.block,
                "sample_time_unresolved",
                "this block requires an unambiguous positive discrete sample time; set its period explicitly",
            ));
        }
        if inherited[i] && !node.drivers.is_empty() && !must_be_discrete(node) {
            let forward = combine(node.drivers.iter().map(|w| rates[w.block]), base)?;
            if forward != SampleTime::Constant && forward != rate {
                return Err(failure(
                    node.block,
                    "sample_time_conflict",
                    "input rates disagree with the inherited downstream constraint",
                ));
            }
        }
        if must_be_discrete(node) && !matches!(node.block.kind, BlockKind::RateTransition { .. }) {
            for wire in &node.drivers {
                if matches!(rates[wire.block], SampleTime::Discrete { .. })
                    && rates[wire.block] != rate
                {
                    return Err(failure(
                        node.block,
                        "rate_transition_required",
                        "different discrete input and output rates require an explicit RateTransition block",
                    ));
                }
            }
        }
    }
    for i in 0..nodes.len() {
        if matches!(
            nodes[i].block.kind,
            BlockKind::DiscreteIntegrator { .. }
                | BlockKind::ResetIntegrator { discrete: true, .. }
        ) {
            if let SampleTime::Discrete { period } = rates[i] {
                nodes[i].sampling_period = Some(period);
            }
            lower_builtin(&mut nodes[i], &SourceBundle::new())?;
        }
        if let BlockKind::RateTransition { deterministic, .. } = nodes[i].block.kind {
            let output = rates[i];
            let input = rates[nodes[i].drivers[0].block];
            let (SampleTime::Discrete { period: source }, SampleTime::Discrete { period: target }) =
                (input, output)
            else {
                return Err(failure(
                    nodes[i].block,
                    "rate_transition",
                    "RateTransition requires two explicit periodic clock domains; use ZeroOrderHold for continuous sampling",
                ));
            };
            let (source_ticks, target_ticks) = (ticks(source, base)?, ticks(target, base)?);
            if source_ticks.max(target_ticks) % source_ticks.min(target_ticks) != 0 {
                return Err(failure(
                    nodes[i].block,
                    "rate_transition",
                    "initial RateTransition support requires integer-related periods",
                ));
            }
            if deterministic && source_ticks > target_ticks {
                nodes[i].transition_period = Some(source);
                lower_builtin(&mut nodes[i], &SourceBundle::new())?;
            }
        }
    }
    Ok(rates)
}

fn state_period(node: &Node<'_>, rate: SampleTime) -> Option<f64> {
    if node.q.is_empty() {
        None
    } else {
        node.transition_period
            .or_else(|| {
                node.definition
                    .and_then(|d| d.sample_time)
                    .filter(|p| *p > 0.0)
            })
            .or({
                if let SampleTime::Discrete { period } = rate {
                    Some(period)
                } else {
                    None
                }
            })
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn layout(
    model: &Model,
    nodes: &[Node<'_>],
    rates: &[SampleTime],
) -> Result<SamplingPlan, ModelError> {
    let base = model.settings.max_step;
    let mut periods = BTreeSet::new();
    for (node, rate) in nodes.iter().zip(rates) {
        if let SampleTime::Discrete { period } = *rate {
            periods.insert(ticks(period, base)?);
        }
        if let Some(period) = state_period(node, *rate) {
            periods.insert(ticks(period, base)?);
        }
    }
    if periods.len() > 64 {
        return Err(ModelError::new(
            "model_limit",
            "at most 64 distinct sampling clocks are supported",
        ));
    }
    let lookup: BTreeMap<_, _> = periods.iter().enumerate().map(|(i, &p)| (p, i)).collect();
    let mut result = SamplingPlan {
        clocks: periods
            .into_iter()
            .enumerate()
            .map(|(id, ticks)| SampleClock {
                id,
                ticks,
                period: ticks as f64 * base,
            })
            .collect(),
        blocks: nodes
            .iter()
            .zip(rates)
            .map(|(n, &r)| (n.block.id.clone(), r))
            .collect(),
        state_clocks: Vec::new(),
        cache_count: 0,
        output_offsets: Vec::new(),
        node_clocks: Vec::new(),
    };
    for (node, &rate) in nodes.iter().zip(rates) {
        let clock = if let SampleTime::Discrete { period } = rate {
            Some(lookup[&ticks(period, base)?])
        } else {
            None
        };
        result.node_clocks.push(clock);
        result.output_offsets.push(
            node.outputs
                .iter()
                .map(|port| {
                    clock.map(|_| {
                        let offset = result.cache_count;
                        result.cache_count += port.width;
                        offset
                    })
                })
                .collect(),
        );
        if let Some(period) = state_period(node, rate) {
            result.state_clocks.extend(std::iter::repeat_n(
                lookup[&ticks(period, base)?],
                node.q.len(),
            ));
        }
    }
    if result.cache_count > 262_144 {
        return Err(ModelError::new(
            "model_limit",
            "sampled output storage exceeds 262144 scalars",
        ));
    }
    Ok(result)
}

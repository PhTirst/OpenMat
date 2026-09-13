use super::{Frame, Kernel, Ordering, RunError, Runner, near};
use crate::{sampling::SamplingPlan, solver::OdeStep};
use std::sync::atomic::AtomicBool;

#[derive(Clone)]
pub(super) struct SampledState {
    pub extra: Vec<f64>,
    pub hits: Vec<usize>,
    next_ticks: Vec<u64>,
    base_tick: u64,
}

impl SampledState {
    pub fn new(plan: &SamplingPlan) -> Self {
        let mut extra = vec![0.0; plan.cache_count + plan.clocks.len()];
        extra[plan.cache_count..].fill(1.0);
        Self {
            extra,
            hits: (0..plan.clocks.len()).collect(),
            next_ticks: vec![1; plan.clocks.len()],
            base_tick: 0,
        }
    }
}

impl<K: Kernel> Runner<K> {
    pub(super) fn initialize_sampling(&mut self) -> Result<(), RunError> {
        let plan = self.plan.sampling.as_ref().expect("sampling plan");
        let state = self.sampling.as_mut().expect("sampling instance");
        if let Some(kernel) = &mut self.update_kernel {
            self.update_scratch.evaluate_with_extra(
                kernel,
                &self.plan.update_origins,
                self.time,
                &self.continuous,
                &self.held,
                &state.extra,
                &AtomicBool::new(false),
            )?;
            state.extra[..plan.cache_count]
                .copy_from_slice(&self.update_scratch.outputs[..plan.cache_count]);
            self.pending
                .copy_from_slice(&self.update_scratch.outputs[plan.cache_count..]);
        }
        self.sample_hit = !state.hits.is_empty();
        self.evaluate_current(&AtomicBool::new(false))?;
        self.capture_observations();
        if let Some(events) = &mut self.events {
            events.evaluate(
                self.time,
                &self.continuous,
                &self.held,
                &self.sampling.as_ref().expect("sampling").extra,
                &AtomicBool::new(false),
            )?;
            events.initialize();
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines, clippy::cast_precision_loss)] // Single transaction; counters are bounded to 1e9 base ticks.
    pub(super) fn advance_sampled(
        &mut self,
        cancel: &AtomicBool,
    ) -> Result<Option<Frame>, RunError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(RunError::new("cancelled", "simulation was cancelled"));
        }
        if self.is_finished() {
            return Ok(None);
        }
        let sampling = self.plan.sampling.as_ref().expect("sampling plan");
        let current = self.sampling.as_ref().expect("sampling instance");
        let settings = &self.plan.settings;
        let hit_times = sampling
            .clocks
            .iter()
            .zip(&current.next_ticks)
            .map(|(clock, &tick)| {
                if self.plan.events.is_some() {
                    return Ok(clock.period * tick as f64);
                }
                clock
                    .ticks
                    .checked_mul(tick)
                    .map(|t| t as f64 * settings.max_step)
                    .ok_or_else(|| RunError::new("time_progress", "sampling counter overflow"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let hit = hit_times.iter().copied().fold(f64::INFINITY, f64::min);
        let event = if self.solver.is_some() {
            self.plan
                .time_events
                .iter()
                .copied()
                .find(|t| *t > self.time)
                .unwrap_or(f64::INFINITY)
        } else {
            self.plan
                .input_knots
                .iter()
                .copied()
                .find(|t| *t > self.time)
                .unwrap_or(f64::INFINITY)
        };
        let boundary = settings.stop_time.min(hit).min(event);
        let mut next = ((current.base_tick + 1) as f64 * settings.max_step).min(boundary);
        let mut root_directions = vec![0; self.plan.events.as_ref().map_or(0, |e| e.events.len())];
        if let Some(solver) = self.solver.as_mut() {
            let scratch = &mut self.scratch;
            let kernel = &mut self.kernel;
            let plan = &self.plan;
            let held = &self.held;
            let extra = &current.extra;
            let mut rhs = |time: f64, state: &[f64], derivatives: &mut [f64]| {
                let time = if event.is_finite() && time >= event {
                    event.next_down()
                } else {
                    time
                };
                scratch.evaluate_with_extra(
                    kernel,
                    &plan.origins,
                    time,
                    state,
                    held,
                    extra,
                    cancel,
                )?;
                derivatives.copy_from_slice(&scratch.outputs[..state.len()]);
                Ok(())
            };
            let step = OdeStep {
                time: self.time,
                boundary,
                max_step: settings.max_step,
                state: &self.continuous,
                candidate: &mut self.trial,
                cancel,
                reinitialize: self.sample_hit
                    || self.event_hit
                    || self.time.to_bits() == settings.start_time.to_bits(),
            };
            if let Some(events) = &mut self.events
                && !events.located.is_empty()
            {
                let located = events.located.clone();
                let mut found = vec![0; located.len()];
                let mut roots = |time: f64, state: &[f64], values: &mut [f64]| {
                    events.evaluate(time, state, held, extra, cancel)?;
                    for (out, &index) in values.iter_mut().zip(&located) {
                        *out = events.scratch.outputs[index];
                    }
                    Ok(())
                };
                next = solver.advance_with_events(
                    step,
                    &mut rhs,
                    crate::solver::OdeEvents {
                        count: located.len(),
                        evaluate: &mut roots,
                        found: &mut found,
                    },
                )?;
                for (&index, &direction) in located.iter().zip(&found) {
                    root_directions[index] = direction;
                }
            } else {
                next = solver.advance(step, &mut rhs)?;
            }
            if next > boundary && !near(next, boundary)
                || next > self.time + settings.max_step
                    && !near(next, self.time + settings.max_step)
            {
                return Err(RunError::new(
                    "solver_boundary",
                    "ODE solver crossed a sampling boundary or maximum step",
                ));
            }
        }
        if near(next, boundary) {
            next = boundary;
        }
        if next <= self.time || !next.is_finite() {
            return Err(RunError::new(
                "time_progress",
                "simulation time did not advance",
            ));
        }
        let hits: Vec<_> = hit_times
            .iter()
            .enumerate()
            .filter_map(|(i, &time)| near(time, next).then_some(i))
            .collect();
        let event_hit = event.is_finite() && near(next, event);
        let mut candidate_sampling = current.clone();
        candidate_sampling.hits = hits;
        if self.solver.is_none() && near(next, (current.base_tick + 1) as f64 * settings.max_step) {
            candidate_sampling.base_tick += 1;
        }
        candidate_sampling.extra[sampling.cache_count..].fill(0.0);
        for &clock in &candidate_sampling.hits {
            candidate_sampling.next_ticks[clock] += 1;
            candidate_sampling.extra[sampling.cache_count + clock] = 1.0;
        }
        let mut candidate_state = self.held.clone();
        for (i, &clock) in sampling.state_clocks.iter().enumerate() {
            if candidate_sampling.hits.contains(&clock) {
                candidate_state[i] = self.pending[i];
            }
        }
        if !self.continuous.is_empty() && self.solver.is_none() {
            self.integrate(next - self.time, cancel)?;
        }
        let sampling = self.plan.sampling.as_ref().expect("sampling plan");
        let mut candidate_pending = self.pending.clone();
        if !candidate_sampling.hits.is_empty()
            && let Some(kernel) = &mut self.update_kernel
        {
            self.update_scratch.evaluate_with_extra(
                kernel,
                &self.plan.update_origins,
                next,
                &self.trial,
                &candidate_state,
                &candidate_sampling.extra,
                cancel,
            )?;
            candidate_sampling.extra[..sampling.cache_count]
                .copy_from_slice(&self.update_scratch.outputs[..sampling.cache_count]);
            for (i, &clock) in sampling.state_clocks.iter().enumerate() {
                if candidate_sampling.hits.contains(&clock) {
                    candidate_pending[i] = self.update_scratch.outputs[sampling.cache_count + i];
                }
            }
        }
        let mut event_candidate = self.events.as_ref().map(super::hybrid::EventCandidate::new);
        if let (Some(events), Some(candidate), Some(plan)) =
            (&mut self.events, &mut event_candidate, &self.plan.events)
        {
            let periods: Vec<_> = candidate_sampling
                .hits
                .iter()
                .map(|&id| sampling.clocks[id].period)
                .collect();
            for iteration in 0..17 {
                if iteration == 16 {
                    return Err(RunError::new(
                        "event_iteration",
                        "event cascade exceeded 16 reset rounds",
                    ));
                }
                events.evaluate(
                    next,
                    &self.trial,
                    &candidate_state,
                    &candidate_sampling.extra,
                    cancel,
                )?;
                let reset = candidate.apply(
                    plan,
                    &events.scratch.outputs,
                    &mut self.trial,
                    &mut candidate_state,
                    &periods,
                    &root_directions,
                    iteration == 0,
                )?;
                root_directions.fill(0);
                if !reset {
                    break;
                }
                if !candidate_sampling.hits.is_empty()
                    && let Some(kernel) = &mut self.update_kernel
                {
                    self.update_scratch.evaluate_with_extra(
                        kernel,
                        &self.plan.update_origins,
                        next,
                        &self.trial,
                        &candidate_state,
                        &candidate_sampling.extra,
                        cancel,
                    )?;
                    candidate_sampling.extra[..sampling.cache_count]
                        .copy_from_slice(&self.update_scratch.outputs[..sampling.cache_count]);
                    for (i, &clock) in sampling.state_clocks.iter().enumerate() {
                        if candidate_sampling.hits.contains(&clock) {
                            candidate_pending[i] =
                                self.update_scratch.outputs[sampling.cache_count + i];
                        }
                    }
                }
            }
        }
        self.scratch.evaluate_with_extra(
            &mut self.kernel,
            &self.plan.origins,
            next,
            &self.trial,
            &candidate_state,
            &candidate_sampling.extra,
            cancel,
        )?;
        if let (Some(events), Some(candidate)) = (&self.events, &event_candidate)
            && events.total_records + candidate.records.len() > 100_000
        {
            return Err(RunError::new(
                "event_limit",
                "event records exceeded 100000 per run",
            ));
        }
        // No externally observable mutation precedes successful candidate validation.
        self.continuous.copy_from_slice(&self.trial);
        self.held = candidate_state;
        self.pending = candidate_pending;
        self.time = next;
        self.sample_hit = !candidate_sampling.hits.is_empty();
        self.event_hit = event_hit
            || event_candidate
                .as_ref()
                .is_some_and(|e| !e.records.is_empty());
        if let (Some(events), Some(candidate)) = (&mut self.events, event_candidate) {
            events.total_records += candidate.records.len();
            events.signs = candidate.signs;
            events.zero_edges = candidate.zero_edges;
            events.records = candidate.records;
        }
        self.sampling = Some(candidate_sampling);
        self.capture_observations();
        Ok(Some(self.current_frame()))
    }
}

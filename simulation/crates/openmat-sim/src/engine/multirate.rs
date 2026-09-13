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
            f64::INFINITY
        };
        let boundary = settings.stop_time.min(hit).min(event);
        let mut next = ((current.base_tick + 1) as f64 * settings.max_step).min(boundary);
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
            next = solver.advance(
                OdeStep {
                    time: self.time,
                    boundary,
                    max_step: settings.max_step,
                    state: &self.continuous,
                    candidate: &mut self.trial,
                    cancel,
                    reinitialize: self.sample_hit
                        || self.event_hit
                        || self.time.to_bits() == settings.start_time.to_bits(),
                },
                &mut rhs,
            )?;
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
        if self.solver.is_none() {
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
        self.scratch.evaluate_with_extra(
            &mut self.kernel,
            &self.plan.origins,
            next,
            &self.trial,
            &candidate_state,
            &candidate_sampling.extra,
            cancel,
        )?;
        // No externally observable mutation precedes successful candidate validation.
        self.continuous.copy_from_slice(&self.trial);
        self.held = candidate_state;
        self.pending = candidate_pending;
        self.time = next;
        self.sample_hit = !candidate_sampling.hits.is_empty();
        self.event_hit = event_hit;
        self.sampling = Some(candidate_sampling);
        self.capture_observations();
        Ok(Some(self.current_frame()))
    }
}

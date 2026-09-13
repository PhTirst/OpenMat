use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::model::Port;
use crate::numeric::Kernel;
use crate::solver::{ContinuousSolver, OdeStep, SolverStats};
use crate::{CompiledModel, ScopeInfo};
mod hybrid;
mod multirate;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub time: f64,
    pub sample_hit: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sample_hits: Vec<usize>,
    pub values: Vec<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<crate::hybrid::EventRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub execution_hits: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub execution_events: Vec<crate::conditional::ExecutionEvent>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationResult {
    pub schema_version: u32,
    pub model: String,
    pub scopes: Vec<ScopeInfo>,
    pub frames: Vec<Frame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<crate::sampling::SamplingPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_plan: Option<crate::hybrid::EventPlan>,
}

#[derive(Clone, Copy, Debug)]
pub struct CollectionLimits {
    pub max_samples: usize,
    pub max_values: usize,
}

impl Default for CollectionLimits {
    fn default() -> Self {
        Self {
            max_samples: 100_000,
            max_values: 8_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RunError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
}

impl RunError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            block: None,
            port: None,
        }
    }
}
impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(block) = &self.block {
            write!(
                f,
                " [block={block}, port={}]",
                self.port.as_deref().unwrap_or("")
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for RunError {}

struct Scratch {
    inputs: Vec<f64>,
    outputs: Vec<f64>,
}

impl Scratch {
    fn evaluate<K: Kernel>(
        &mut self,
        kernel: &mut K,
        origins: &[Port],
        time: f64,
        continuous: &[f64],
        discrete: &[f64],
        cancel: &AtomicBool,
    ) -> Result<(), RunError> {
        self.evaluate_with_extra(kernel, origins, time, continuous, discrete, &[], cancel)
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_with_extra<K: Kernel>(
        &mut self,
        kernel: &mut K,
        origins: &[Port],
        time: f64,
        continuous: &[f64],
        discrete: &[f64],
        extra: &[f64],
        cancel: &AtomicBool,
    ) -> Result<(), RunError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(RunError::new("cancelled", "simulation was cancelled"));
        }
        self.inputs[0] = time;
        self.inputs[1..=continuous.len()].copy_from_slice(continuous);
        let end = 1 + continuous.len() + discrete.len();
        self.inputs[1 + continuous.len()..end].copy_from_slice(discrete);
        self.inputs[end..].copy_from_slice(extra);
        if self.inputs.iter().any(|v| !v.is_finite()) {
            return Err(RunError::new(
                "non_finite_state",
                format!("non-finite state at time {time}"),
            ));
        }
        kernel
            .evaluate(&self.inputs, &mut self.outputs)
            .map_err(|e| RunError::new("kernel", e.to_string()))?;
        if let Some(index) = self.outputs.iter().position(|v| !v.is_finite()) {
            let origin = &origins[index];
            return Err(RunError {
                code: "non_finite_output",
                message: format!("non-finite numerical output at time {time}"),
                block: Some(origin.block.clone()),
                port: Some(origin.port.clone()),
            });
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(RunError::new("cancelled", "simulation was cancelled"));
        }
        Ok(())
    }
}

/// A simulation instance. Numerical trials never mutate the held discrete outputs.
pub struct Runner<K> {
    plan: CompiledModel,
    kernel: K,
    update_kernel: Option<K>,
    update_scratch: Scratch,
    time: f64,
    continuous: Vec<f64>,
    held: Vec<f64>,
    pending: Vec<f64>,
    observations: Vec<f64>,
    scratch: Scratch,
    trial: Vec<f64>,
    slopes: [Vec<f64>; 4],
    next_tick: u32,
    sample_hit: bool,
    event_hit: bool,
    failed: bool,
    solver: Option<Box<dyn ContinuousSolver>>,
    sampling: Option<multirate::SampledState>,
    events: Option<hybrid::EventState>,
}

impl<K: Kernel> Runner<K> {
    /// Initialize states and capture tick-zero inputs for the first delay update.
    ///
    /// # Errors
    /// Rejects incompatible kernels and failures during initial evaluation.
    pub fn new(plan: CompiledModel, kernel: K) -> Result<Self, RunError> {
        Self::new_with_update(plan, kernel, None)
    }

    /// Initialize a model with a separate discrete update kernel.
    /// # Errors
    /// Rejects mismatched or missing kernels and invalid initial evaluations.
    pub fn new_with_update(
        plan: CompiledModel,
        kernel: K,
        update_kernel: Option<K>,
    ) -> Result<Self, RunError> {
        if kernel.program() != &plan.program {
            return Err(RunError::new(
                "kernel_mismatch",
                "kernel was compiled for a different numerical program",
            ));
        }
        if update_kernel.as_ref().map(Kernel::program) != plan.update_program.as_ref() {
            return Err(RunError::new(
                "kernel_mismatch",
                "discrete update kernel is missing or was compiled for a different program",
            ));
        }
        let count = plan.continuous_initial.len();
        let mut runner = Self {
            time: plan.settings.start_time,
            continuous: plan.continuous_initial.clone(),
            held: plan.discrete_initial.clone(),
            pending: plan.discrete_initial.clone(),
            observations: vec![
                0.0;
                plan.program.output_count() - count - plan.discrete_initial.len()
            ],
            sample_hit: !plan.discrete_initial.is_empty(),
            event_hit: false,
            scratch: Scratch {
                inputs: vec![0.0; plan.program.input_count()],
                outputs: vec![0.0; plan.program.output_count()],
            },
            update_scratch: Scratch {
                inputs: vec![0.0; plan.program.input_count()],
                outputs: vec![
                    0.0;
                    plan.update_program
                        .as_ref()
                        .map_or(0, crate::numeric::Program::output_count)
                ],
            },
            trial: vec![0.0; count],
            slopes: std::array::from_fn(|_| vec![0.0; count]),
            next_tick: 1,
            failed: false,
            solver: None,
            sampling: plan.sampling.as_ref().map(multirate::SampledState::new),
            events: plan.events.as_ref().map(hybrid::EventState::new),
            plan,
            kernel,
            update_kernel,
        };
        if runner.sampling.is_some() {
            runner.initialize_sampling()?;
            return Ok(runner);
        }
        runner.evaluate_current(&AtomicBool::new(false))?;
        if let Some(kernel) = &mut runner.update_kernel {
            runner.update_scratch.evaluate(
                kernel,
                &runner.plan.update_origins,
                runner.time,
                &runner.continuous,
                &runner.held,
                &AtomicBool::new(false),
            )?;
        }
        runner.capture_next();
        runner.capture_observations();
        Ok(runner)
    }

    /// Select a continuous solver before the first step.
    /// # Errors
    /// Rejects a run already advanced or a model without continuous states.
    pub fn with_solver(mut self, solver: Box<dyn ContinuousSolver>) -> Result<Self, RunError> {
        if self.time.to_bits() != self.plan.settings.start_time.to_bits()
            || self.failed
            || self.continuous.is_empty()
        {
            return Err(RunError::new(
                "solver_configuration",
                "a continuous solver requires continuous states and a fresh run",
            ));
        }
        self.solver = Some(solver);
        Ok(self)
    }

    #[must_use]
    pub fn solver_statistics(&self) -> Option<SolverStats> {
        self.solver.as_ref().map(|solver| solver.statistics())
    }

    #[must_use]
    pub fn time(&self) -> f64 {
        self.time
    }
    #[must_use]
    pub fn continuous_state(&self) -> &[f64] {
        &self.continuous
    }
    #[must_use]
    pub fn discrete_output(&self) -> &[f64] {
        &self.held
    }
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.time >= self.plan.settings.stop_time
    }
    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.failed
    }
    #[must_use]
    pub fn current_frame(&self) -> Frame {
        Frame {
            time: self.time,
            sample_hit: self.sample_hit,
            sample_hits: self
                .sampling
                .as_ref()
                .map_or_else(Vec::new, |s| s.hits.clone()),
            values: self.observations.clone(),
            events: self
                .events
                .as_ref()
                .map_or_else(Vec::new, |e| e.records.clone()),
            execution_hits: self
                .sampling
                .as_ref()
                .map_or_else(Vec::new, |s| s.execution_hits.clone()),
            execution_events: self
                .sampling
                .as_ref()
                .map_or_else(Vec::new, |s| s.execution_events.clone()),
        }
    }

    /// Advance one accepted step, bounded by the next sample hit and stop time.
    ///
    /// # Errors
    /// Returns terminal errors for failed numerical evaluation or invalid progress.
    pub fn advance(&mut self) -> Result<Option<Frame>, RunError> {
        self.advance_with_cancel(&AtomicBool::new(false))
    }

    /// # Errors
    /// Cancellation or any numerical failure makes this runner terminal while
    /// preserving the last accepted time, states and observations.
    pub fn advance_with_cancel(&mut self, cancel: &AtomicBool) -> Result<Option<Frame>, RunError> {
        if self.failed {
            return Err(RunError::new(
                "failed_run",
                "a failed run must be reinitialized",
            ));
        }
        let result = self.advance_inner(cancel);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    #[allow(clippy::too_many_lines)] // Validate every candidate before the single state commit.
    fn advance_inner(&mut self, cancel: &AtomicBool) -> Result<Option<Frame>, RunError> {
        if self.sampling.is_some() {
            return self.advance_sampled(cancel);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(RunError::new("cancelled", "simulation was cancelled"));
        }
        if self.is_finished() {
            return Ok(None);
        }
        let s = &self.plan.settings;
        let hit = if self.held.is_empty() {
            f64::INFINITY
        } else {
            s.start_time
                + f64::from(self.next_tick) * s.sample_time.expect("validated discrete sample time")
        };
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
        let boundary = s.stop_time.min(hit).min(event);
        let mut next = (self.time + s.max_step).min(boundary);
        if let Some(solver) = self.solver.as_mut() {
            let scratch = &mut self.scratch;
            let kernel = &mut self.kernel;
            let plan = &self.plan;
            let held = &self.held;
            let mut rhs = |time: f64, state: &[f64], derivatives: &mut [f64]| {
                // CVODE integrates up to the left limit; the accepted frame and
                // next solver history use the right limit. RK4 preserves stage-time semantics.
                let rhs_time = if event.is_finite() && time >= event {
                    event.next_down()
                } else {
                    time
                };
                scratch.evaluate(kernel, &plan.origins, rhs_time, state, held, cancel)?;
                derivatives.copy_from_slice(&scratch.outputs[..state.len()]);
                Ok(())
            };
            next = solver.advance(
                OdeStep {
                    time: self.time,
                    boundary,
                    max_step: s.max_step,
                    state: &self.continuous,
                    candidate: &mut self.trial,
                    cancel,
                    reinitialize: self.sample_hit
                        || self.event_hit
                        || self.time.to_bits() == s.start_time.to_bits(),
                },
                &mut rhs,
            )?;
            if (next > boundary && !near(next, boundary))
                || (next > self.time + s.max_step && !near(next, self.time + s.max_step))
            {
                return Err(RunError::new(
                    "solver_boundary",
                    "ODE solver crossed the scheduler boundary or maximum step",
                ));
            }
        }
        if near(next, s.stop_time) {
            next = s.stop_time;
        }
        let event_hit = event.is_finite() && near(next, event);
        if event_hit {
            next = event;
        }
        let sample_hit = hit.is_finite() && near(next, hit);
        if sample_hit && hit < s.stop_time && !near(hit, s.stop_time) {
            next = hit;
        }
        if next <= self.time || !next.is_finite() {
            return Err(RunError::new(
                "time_progress",
                "time step is not representable",
            ));
        }
        let next_tick = if sample_hit {
            self.next_tick
                .checked_add(1)
                .ok_or_else(|| RunError::new("tick_limit", "sample tick limit reached"))?
        } else {
            self.next_tick
        };
        let step = next - self.time;
        if !self.continuous.is_empty() && self.solver.is_none() {
            self.integrate(step, cancel)?;
        }
        self.scratch.evaluate(
            &mut self.kernel,
            &self.plan.origins,
            next,
            &self.trial,
            if sample_hit {
                &self.pending
            } else {
                &self.held
            },
            cancel,
        )?;
        if sample_hit && let Some(kernel) = &mut self.update_kernel {
            self.update_scratch.evaluate(
                kernel,
                &self.plan.update_origins,
                next,
                &self.trial,
                &self.pending,
                cancel,
            )?;
        }
        // Commit only after all candidate outputs are valid. Trial failures must
        // not expose partial state or observation changes to streaming callers.
        self.continuous.copy_from_slice(&self.trial);
        self.time = next;
        self.sample_hit = sample_hit;
        self.event_hit = event_hit;
        self.next_tick = next_tick;
        if sample_hit {
            self.held.copy_from_slice(&self.pending);
            self.capture_next();
        }
        self.capture_observations();
        Ok(Some(self.current_frame()))
    }

    fn evaluate_current(&mut self, cancel: &AtomicBool) -> Result<(), RunError> {
        self.scratch.evaluate_with_extra(
            &mut self.kernel,
            &self.plan.origins,
            self.time,
            &self.continuous,
            &self.held,
            self.sampling.as_ref().map_or(&[], |s| s.extra.as_slice()),
            cancel,
        )
    }

    fn capture_next(&mut self) {
        if self.update_kernel.is_some() {
            self.pending.copy_from_slice(&self.update_scratch.outputs);
            return;
        }
        let count = self.continuous.len();
        self.pending
            .copy_from_slice(&self.scratch.outputs[count..count + self.held.len()]);
    }

    fn capture_observations(&mut self) {
        self.observations
            .copy_from_slice(&self.scratch.outputs[self.continuous.len() + self.held.len()..]);
    }

    fn integrate(&mut self, step: f64, cancel: &AtomicBool) -> Result<(), RunError> {
        let count = self.continuous.len();
        self.evaluate_current(cancel)?;
        self.slopes[0].copy_from_slice(&self.scratch.outputs[..count]);
        for stage in 1..4 {
            let fraction = if stage == 3 { 1.0 } else { 0.5 };
            for (index, value) in self.trial.iter_mut().enumerate() {
                *value = self.continuous[index] + fraction * step * self.slopes[stage - 1][index];
            }
            self.scratch.evaluate_with_extra(
                &mut self.kernel,
                &self.plan.origins,
                self.time + fraction * step,
                &self.trial,
                &self.held,
                self.sampling.as_ref().map_or(&[], |s| s.extra.as_slice()),
                cancel,
            )?;
            self.slopes[stage].copy_from_slice(&self.scratch.outputs[..count]);
        }
        for (index, value) in self.trial.iter_mut().enumerate() {
            *value = self.continuous[index]
                + (step / 6.0)
                    * (self.slopes[0][index]
                        + 2.0 * self.slopes[1][index]
                        + 2.0 * self.slopes[2][index]
                        + self.slopes[3][index]);
        }
        Ok(())
    }

    /// Collect a bounded trajectory. Streaming hosts can call `advance` instead.
    ///
    /// # Errors
    /// Returns numerical errors or an explicit storage-limit error.
    pub fn collect(self, limits: CollectionLimits) -> Result<SimulationResult, RunError> {
        self.collect_with_cancel(limits, &AtomicBool::new(false))
    }

    /// # Errors
    /// Returns numerical, cancellation or trajectory storage-limit errors.
    pub fn collect_with_cancel(
        mut self,
        limits: CollectionLimits,
        cancel: &AtomicBool,
    ) -> Result<SimulationResult, RunError> {
        if self.failed {
            return Err(RunError::new(
                "failed_run",
                "a failed run must be reinitialized",
            ));
        }
        let width = self.plan.program.output_count() - self.continuous.len() - self.held.len();
        let mut frames = Vec::new();
        let mut frame = self.current_frame();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(RunError::new("cancelled", "simulation was cancelled"));
            }
            let count = frames.len() + 1;
            if count > limits.max_samples
                || count
                    .checked_mul(width)
                    .is_none_or(|size| size > limits.max_values)
            {
                return Err(RunError::new(
                    "output_limit",
                    "trajectory storage limit reached; consume accepted frames incrementally or increase the limit",
                ));
            }
            frames.push(frame);
            match self.advance_with_cancel(cancel)? {
                Some(next) => frame = next,
                None => break,
            }
        }
        Ok(SimulationResult {
            schema_version: if self.plan.events.is_some() {
                3
            } else if self.plan.sampling.is_some() {
                2
            } else {
                1
            },
            event_plan: self.plan.events,
            sampling: self.plan.sampling,
            model: self.plan.name,
            scopes: self.plan.scopes,
            frames,
        })
    }
}

fn near(a: f64, b: f64) -> bool {
    a.is_finite()
        && b.is_finite()
        && (a - b).abs() <= 8.0 * f64::EPSILON * a.abs().max(b.abs()).max(f64::MIN_POSITIVE)
}

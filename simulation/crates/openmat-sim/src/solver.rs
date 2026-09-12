//! Safe in-process boundary between the state scheduler and an ODE integrator.
use crate::RunError;
use serde::Serialize;
use std::sync::atomic::AtomicBool;

pub type OdeRhs<'a> = dyn FnMut(f64, &[f64], &mut [f64]) -> Result<(), RunError> + 'a;

pub struct OdeStep<'a> {
    pub time: f64,
    pub boundary: f64,
    pub max_step: f64,
    pub state: &'a [f64],
    pub candidate: &'a mut [f64],
    pub cancel: &'a AtomicBool,
    /// True initially and immediately after discrete outputs change.
    pub reinitialize: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SolverStats {
    pub accepted_steps: u64,
    pub rhs_evaluations: u64,
    pub error_test_failures: u64,
    pub nonlinear_iterations: u64,
    pub reinitializations: u64,
}

pub trait ContinuousSolver {
    /// Take one accepted step without crossing the scheduler's boundary.
    /// Trial RHS calls must not commit simulation state.
    /// # Errors
    /// On failure the scheduler discards the candidate and terminates the run.
    fn advance(&mut self, step: OdeStep<'_>, rhs: &mut OdeRhs<'_>) -> Result<f64, RunError>;
    fn statistics(&self) -> SolverStats;
}

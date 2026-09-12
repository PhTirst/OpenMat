//! Causal block compilation and deterministic continuous/discrete simulation.
#![forbid(unsafe_code)]

mod compiler;
mod diagnostic;
mod engine;
pub mod model;
pub mod numeric;

pub use compiler::{CompiledModel, ScopeInfo, compile};
pub use diagnostic::{Diagnostic, ModelError};
pub use engine::{CollectionLimits, Frame, RunError, Runner, SimulationResult};

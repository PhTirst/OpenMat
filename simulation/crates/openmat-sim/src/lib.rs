//! Causal block compilation and deterministic continuous/discrete simulation.
#![forbid(unsafe_code)]

mod compiler;
mod diagnostic;
mod engine;
mod m_function;
pub mod model;
pub mod numeric;
pub mod solver;

pub use compiler::{CompiledModel, ScopeInfo, compile, compile_with_sources};
pub use diagnostic::{Diagnostic, ModelError};
pub use engine::{CollectionLimits, Frame, RunError, Runner, SimulationResult};
pub use m_function::{MAX_SOURCE_BYTES, SourceBundle};

//! Causal block compilation and deterministic continuous/discrete simulation.
#![forbid(unsafe_code)]

pub mod authoring;
mod compiler;
pub mod component;
mod component_compiler;
pub mod conditional;
mod diagnostic;
mod engine;
pub mod hybrid;
mod m_function;
pub mod model;
pub mod numeric;
pub mod sampling;
pub mod solver;
mod static_function;

pub use compiler::{CompiledModel, ScopeInfo, compile, compile_with_sources};
pub use diagnostic::{Diagnostic, ModelError};
pub use engine::{CollectionLimits, Frame, RunError, Runner, SimulationResult};
pub use m_function::{MAX_SOURCE_BYTES, SourceBundle};

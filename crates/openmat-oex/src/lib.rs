#![doc = "Stable C ABI host for trusted in-process `OpenMat` extensions."]

mod abi;
mod exports;
mod host;
mod loader;

pub use abi::*;
pub use loader::{OexLoadError, OexPlugin};

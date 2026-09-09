#![doc = "Correctness-oriented in-tree implementation of `OpenMat`'s sparse provider boundary."]
#![forbid(unsafe_code)]

mod faer_provider;
mod reference;
mod scalar;

pub use faer_provider::{
    FaerCholeskyComplex64Factor, FaerCholeskyF64Factor, FaerCholeskySymbolic,
    FaerLuComplex64Factor, FaerLuF64Factor, FaerLuSymbolic, FaerOperationBackend,
    FaerQrComplex64Factor, FaerQrF64Factor, FaerQrSymbolic, FaerSparseProvider,
};
pub use reference::{
    ReferenceCholeskyFactor, ReferenceLuFactor, ReferenceQrFactor, ReferenceSparseProvider,
    ReferenceSymbolic,
};

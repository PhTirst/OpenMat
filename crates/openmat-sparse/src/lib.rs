#![doc = "Owned, normalized compressed-sparse-column arrays for `OpenMat`."]
#![forbid(unsafe_code)]

mod arithmetic;
mod error;
mod matrix;
mod structural;
mod value;

pub use arithmetic::{SparseBinaryOperator, SparseBinaryResult};
pub use error::{SparseError, SparseInvariant};
pub use matrix::{CooEntry, CscEntries, CscMatrix};
pub use value::{SparseArrayData, SparseScalarValue, SparseStoredEntry};

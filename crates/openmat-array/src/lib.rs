#![doc = "Column-major dense arrays and shape/index primitives."]
#![forbid(unsafe_code)]

mod dense;
mod dtype;
mod error;
mod index;
mod shape;

pub use dense::{Contiguity, DenseArray, SelectionStrategy};
pub use dtype::{
    ArrayData, ArrayElement, CharCodeUnit, Complex32, Complex64, ComplexInteger, DType,
    IntegerArrayData, IntegerComponent, IntegerElement, IntegerElementDecimal, IntegerElementValue,
    IntegerElements, Logical,
};
pub use error::ArrayError;
pub use index::{IndexRange, IndexSelection, ResolvedIndex, ResolvedIndices};
pub use shape::Shape;

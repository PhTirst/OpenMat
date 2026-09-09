#![doc = "Dynamic `OpenMat` values and session-stable runtime handles."]
#![forbid(unsafe_code)]

mod cell;
mod complex;
mod graphics_handle;
mod handle;
mod object_array;
mod string;
mod struct_array;
mod table;
mod value;

pub use cell::CellArray;
pub use complex::Complex64;
pub use graphics_handle::{GraphicsHandleArray, GraphicsHandleArrayError};
pub use handle::{
    ArrayHandle, BuiltinHandle, BytecodeFunctionHandle, ClassHandle, FunctionHandle,
    IntrinsicFunction, ObjectHandle,
};
pub use object_array::ObjectArray;
pub use openmat_graphics_model::{GraphicsClass, GraphicsHandle};
pub use openmat_sparse::{
    CooEntry, CscEntries, CscMatrix, SparseArrayData, SparseError, SparseInvariant,
    SparseScalarValue, SparseStoredEntry,
};
pub use string::{StringArray, StringElement, StringValue};
pub use struct_array::{FieldName, StructArray, StructSchema};
pub use table::{TableArray, TableError, TableRowName, TableSchema, TableVariableName};
pub use value::{AggregateError, ConditionError, ConditionEvaluation, Value, ValueKind};

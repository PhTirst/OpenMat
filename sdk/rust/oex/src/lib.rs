//! Rust bindings for the OEX 1.3 plugin ABI.
//!
//! Functions return [`Result`]; owned handles release themselves on drop. Views
//! borrow their backing value, and values/builders cannot escape a callback.
//! OEX indices are zero-based and arrays are contiguous, column-major.
//!
//! ```no_run
//! use oex::{Call, Result};
//! fn scale(call: &mut Call<'_>) -> Result<()> {
//!     let mut value = call.take_input(0)?;
//!     for element in value.array_mut::<f64>()?.as_mut_slice() {
//!         *element *= 2.0;
//!     }
//!     call.set_output(0, value)
//! }
//! oex::export_plugin! {
//!     name: "example", version: "1.0.0",
//!     functions: [oex::function!("scale", 1..=1, 1..=1, scale)],
//!     classes: [],
//! }
//! ```
//!
//! The generated callback boundary translates errors and catches unwinding
//! panics. `panic = "abort"` still aborts the process. Native classes require
//! `Send + 'static` state and use checked, nonblocking mutex borrows.
//! See the SDK README for linking and the complete example.

#[doc = include_str!("borrowing.md")]
pub mod borrowing {}

mod array;
mod call;
mod containers;
mod error;
mod native;
mod registration;
mod sparse;
mod types;
mod value;

pub use array::{ArrayView, ArrayViewMut, DenseBuilder};
pub use call::{Call, Cancellation};
pub use containers::StringElementRef;
pub use error::{Error, ErrorKind, Result};
pub use native::{NativeClass, NativeObject};
pub use registration::{ClassDefinition, Function, NativeMethod, NativeProperty, Plugin};
pub use sparse::{
    SparseElement, SparsePattern, SparsePatternBuilder, SparseTripletBuilder, SparseView,
};
pub use types::{Char16, Complex, DataType, Element, Logical, ValueInfo, ValueKind};
pub use value::{Value, ValueRef};

/// Raw FFI escape hatch. Every operation requires the oex.h safety contract.
pub use oex_sys as sys;

// Rc marks handles as call-thread-only. A scope lifetime also prevents escaping
// even if the callback forgets a handle instead of dropping it.
type Scope<'a> = std::marker::PhantomData<&'a std::rc::Rc<()>>;

#[doc(hidden)]
pub mod __private {
    pub use crate::call::callback;
    pub use crate::native::{constructor_callback, method_callback};
}

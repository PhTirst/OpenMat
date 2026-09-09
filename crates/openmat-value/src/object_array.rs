use openmat_array::{ArrayError, DenseArray, Shape};

use crate::{ClassHandle, ObjectHandle};

/// An owned homogeneous array of runtime object references.
///
/// The dynamic class is retained even for an empty array, while elements are
/// stored in contiguous column-major order. Cloning shares the handle buffer;
/// the first element replacement detaches that buffer. The runtime remains
/// responsible for applying value-versus-handle assignment semantics to the
/// referenced object records before inserting handles into this container.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectArray {
    class: ClassHandle,
    storage: DenseArray<ObjectHandle>,
}

impl ObjectArray {
    /// Constructs a homogeneous object-reference array from a canonical shape
    /// and column-major handles.
    ///
    /// # Errors
    ///
    /// Returns the array core's checked shape/host-length mismatch error.
    pub fn from_vec(
        class: ClassHandle,
        shape: Shape,
        elements: Vec<ObjectHandle>,
    ) -> Result<Self, ArrayError> {
        DenseArray::from_vec(shape, elements).map(|storage| Self { class, storage })
    }

    /// Returns the session-local identity of the exact dynamic element class.
    #[must_use]
    pub const fn class_handle(&self) -> ClassHandle {
        self.class
    }

    /// Returns the canonical language-visible shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        self.storage.shape()
    }

    /// Returns the number of object elements.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.storage.numel()
    }

    /// Returns object handles in column-major linear order.
    #[must_use]
    pub fn as_slice(&self) -> &[ObjectHandle] {
        self.storage.as_slice()
    }

    /// Returns mutable column-major handle storage, detaching a shared buffer.
    pub fn as_mut_slice(&mut self) -> &mut [ObjectHandle] {
        self.storage.as_mut_slice()
    }

    /// Returns whether two clones share their current handle buffer.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        self.storage.shares_storage_with(&other.storage)
    }
}

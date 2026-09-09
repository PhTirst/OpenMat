use std::sync::Arc;

use crate::{ArrayElement, ArrayError, DType, Shape};

/// Physical continuity guaranteed by [`DenseArray`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contiguity {
    /// Every element is stored contiguously in column-major linear order.
    ColumnMajor,
}

/// The release-one policy for non-scalar indexing results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionStrategy {
    /// Indexing gathers into a new contiguous buffer; strided views are not
    /// exposed by this crate.
    MaterializeCopy,
}

/// An owned, contiguous, column-major dense array with copy-on-write storage.
///
/// Cloning an array shares its `Arc` buffer. The first mutable access to either
/// clone detaches that clone with [`Arc::make_mut`]. No strided view type is
/// exposed in the first release; callers materialize indexing results into a
/// new `DenseArray`.
///
/// The generic representation is an internal Rust API, not a stable plugin,
/// C, or process ABI.
#[derive(Clone, Debug)]
pub struct DenseArray<T> {
    shape: Shape,
    data: Arc<Vec<T>>,
}

impl<T: PartialEq> PartialEq for DenseArray<T> {
    fn eq(&self, other: &Self) -> bool {
        self.shape == other.shape && self.data == other.data
    }
}

impl<T> DenseArray<T> {
    /// Constructs an array from a shape and a column-major element buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::HostLengthOverflow`] if `shape.numel()` cannot fit
    /// a `Vec`, or [`ArrayError::DataLengthMismatch`] if the lengths differ.
    pub fn from_vec(shape: Shape, data: Vec<T>) -> Result<Self, ArrayError> {
        let expected =
            usize::try_from(shape.numel()).map_err(|_| ArrayError::HostLengthOverflow {
                numel: shape.numel(),
            })?;
        if data.len() != expected {
            return Err(ArrayError::DataLengthMismatch {
                expected: shape.numel(),
                actual: data.len(),
            });
        }
        Ok(Self {
            shape,
            data: Arc::new(data),
        })
    }

    /// Returns the canonical shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Returns the number of stored elements.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.shape.numel()
    }

    /// Returns whether the array contains no elements.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.shape.is_empty()
    }

    /// Returns the contiguous column-major buffer.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        self.data.as_slice()
    }

    /// Returns this array's physical layout guarantee.
    #[must_use]
    pub const fn contiguity(&self) -> Contiguity {
        Contiguity::ColumnMajor
    }

    /// Returns the indexing-result storage policy for this milestone.
    #[must_use]
    pub const fn selection_strategy(&self) -> SelectionStrategy {
        SelectionStrategy::MaterializeCopy
    }

    /// Returns whether two clones currently share the same allocation.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }

    /// Returns the number of strong owners of the current buffer.
    #[must_use]
    pub fn storage_strong_count(&self) -> usize {
        Arc::strong_count(&self.data)
    }

    /// Returns the element at a one-based linear index.
    ///
    /// # Errors
    ///
    /// Returns an indexing error if the index is outside `1..=numel`.
    pub fn get_linear(&self, one_based_index: u64) -> Result<&T, ArrayError> {
        let offset = self.shape.linear_offset(one_based_index)?;
        self.get_offset(offset)
    }

    /// Returns the element at one-based multidimensional subscripts.
    ///
    /// # Errors
    ///
    /// Returns an indexing or checked-offset error for invalid subscripts.
    pub fn get_subscripts(&self, subscripts: &[u64]) -> Result<&T, ArrayError> {
        let offset = self.shape.offset_for_subscripts(subscripts)?;
        self.get_offset(offset)
    }

    fn get_offset(&self, offset: u64) -> Result<&T, ArrayError> {
        let offset = usize::try_from(offset).map_err(|_| ArrayError::HostLengthOverflow {
            numel: self.shape.numel(),
        })?;
        self.data.get(offset).ok_or(ArrayError::OffsetOverflow)
    }
}

impl<T: Clone> DenseArray<T> {
    /// Constructs an array whose elements are clones of `value`.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::HostLengthOverflow`] if the element count cannot
    /// fit a host `Vec`.
    pub fn from_elem(shape: Shape, value: T) -> Result<Self, ArrayError> {
        let length =
            usize::try_from(shape.numel()).map_err(|_| ArrayError::HostLengthOverflow {
                numel: shape.numel(),
            })?;
        Self::from_vec(shape, vec![value; length])
    }

    /// Returns a mutable contiguous buffer, detaching shared storage first.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        Arc::make_mut(&mut self.data).as_mut_slice()
    }

    /// Returns mutable access at a one-based linear index.
    ///
    /// Shared storage is detached only after the index has been validated.
    ///
    /// # Errors
    ///
    /// Returns an indexing error if the index is outside `1..=numel`.
    pub fn get_mut_linear(&mut self, one_based_index: u64) -> Result<&mut T, ArrayError> {
        let offset = self.shape.linear_offset(one_based_index)?;
        self.get_mut_offset(offset)
    }

    /// Returns mutable access at one-based multidimensional subscripts.
    ///
    /// Shared storage is detached only after the subscripts have been validated.
    ///
    /// # Errors
    ///
    /// Returns an indexing or checked-offset error for invalid subscripts.
    pub fn get_mut_subscripts(&mut self, subscripts: &[u64]) -> Result<&mut T, ArrayError> {
        let offset = self.shape.offset_for_subscripts(subscripts)?;
        self.get_mut_offset(offset)
    }

    /// Makes an eager, independent copy of the shape and buffer.
    #[must_use]
    pub fn deep_copy(&self) -> Self {
        Self {
            shape: self.shape.clone(),
            data: Arc::new(self.data.as_ref().clone()),
        }
    }

    fn get_mut_offset(&mut self, offset: u64) -> Result<&mut T, ArrayError> {
        let offset = usize::try_from(offset).map_err(|_| ArrayError::HostLengthOverflow {
            numel: self.shape.numel(),
        })?;
        Arc::make_mut(&mut self.data)
            .get_mut(offset)
            .ok_or(ArrayError::OffsetOverflow)
    }
}

impl<T: ArrayElement> DenseArray<T> {
    /// Returns the supported runtime class tag for this element type.
    #[must_use]
    pub const fn dtype(&self) -> DType {
        T::DTYPE
    }
}

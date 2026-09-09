use std::{marker::PhantomData, mem::MaybeUninit, ptr::NonNull, slice};

use crate::{Call, Element, Error, ErrorKind, Result, Scope, Value, sys};
use crate::{
    error::{check, count, length},
    types::check_element,
};

/// Read-only, contiguous, column-major array borrowed from a value.
#[derive(Clone, Copy, Debug)]
pub struct ArrayView<'a, T> {
    pub(crate) dimensions: &'a [u64],
    pub(crate) data: &'a [T],
}
impl<'a, T> ArrayView<'a, T> {
    pub fn dimensions(&self) -> &'a [u64] {
        self.dimensions
    }
    pub fn as_slice(&self) -> &'a [T] {
        self.data
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn get(&self, indices: &[u64]) -> Option<&'a T> {
        self.data.get(offset(self.dimensions, indices)?)
    }
}

/// Exclusive view obtained after the host performs copy-on-write detachment.
#[derive(Debug)]
pub struct ArrayViewMut<'a, T> {
    pub(crate) dimensions: &'a [u64],
    pub(crate) data: &'a mut [T],
}
impl<T> ArrayViewMut<'_, T> {
    pub fn dimensions(&self) -> &[u64] {
        self.dimensions
    }
    pub fn as_slice(&self) -> &[T] {
        self.data
    }
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.data
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn get(&self, indices: &[u64]) -> Option<&T> {
        self.data.get(offset(self.dimensions, indices)?)
    }
    pub fn get_mut(&mut self, indices: &[u64]) -> Option<&mut T> {
        self.data.get_mut(offset(self.dimensions, indices)?)
    }
}

fn offset(dimensions: &[u64], indices: &[u64]) -> Option<usize> {
    if dimensions.len() != indices.len() {
        return None;
    }
    let mut offset = 0u64;
    let mut stride = 1u64;
    for (&dimension, &index) in dimensions.iter().zip(indices) {
        if index >= dimension {
            return None;
        }
        offset = offset.checked_add(index.checked_mul(stride)?)?;
        stride = stride.checked_mul(dimension)?;
    }
    usize::try_from(offset).ok()
}

// The host guarantees allocation validity. Check the additional Rust slice
// requirements before constructing a reference, including the empty case.
fn slice_pointer<T>(pointer: *const T, len: usize) -> Result<*mut T> {
    if len > (isize::MAX as usize) / size_of::<T>() {
        return Err(Error::with_kind(
            ErrorKind::Range,
            "slice exceeds isize::MAX",
        ));
    }
    if len == 0 {
        return Ok(NonNull::<T>::dangling().as_ptr());
    }
    if pointer.is_null() || !(pointer as usize).is_multiple_of(align_of::<T>()) {
        return Err(Error::with_kind(
            ErrorKind::Abi,
            "null or misaligned array storage",
        ));
    }
    Ok(pointer.cast_mut())
}

// SAFETY obligations for callers: initialized allocation of len elements,
// valid for 'a, with no writes while the returned shared slice is live.
pub(crate) unsafe fn read_slice<'a, T>(pointer: *const T, len: usize) -> Result<&'a [T]> {
    let pointer = slice_pointer(pointer, len)?;
    // SAFETY: caller provides allocation/lifetime/aliasing; pointer checks above.
    Ok(unsafe { slice::from_raw_parts(pointer, len) })
}

// Same as read_slice, but the caller must exclusively own the allocation.
pub(crate) unsafe fn write_slice<'a, T>(pointer: *mut T, len: usize) -> Result<&'a mut [T]> {
    let pointer = slice_pointer(pointer, len)?;
    // SAFETY: caller provides exclusive allocation/lifetime; pointer checks above.
    Ok(unsafe { slice::from_raw_parts_mut(pointer, len) })
}

/// A call-scoped allocation of the final host buffer. Dropping aborts it.
/// Safe fill methods initialize every element before committing.
pub struct DenseBuilder<'call, T: Element> {
    handle: Option<NonNull<sys::OexDenseBuilder>>,
    view: sys::OexMutableDenseView,
    len: usize,
    scope: Scope<'call>,
    element: PhantomData<T>,
}

impl<'call, T: Element> DenseBuilder<'call, T> {
    pub(crate) fn new(call: &Call<'call>, dimensions: &[u64]) -> Result<Self> {
        let rank = count(dimensions.len())?;
        let mut pointer = std::ptr::null_mut();
        let mut view = sys::OexMutableDenseView::default();
        // SAFETY: call is live, dimensions and both out pointers are valid.
        let status = unsafe {
            sys::OexDenseBuilder_CreateUninitialized(
                call.as_raw(),
                T::DATA_TYPE.as_raw(),
                rank,
                dimensions.as_ptr(),
                &mut pointer,
                &mut view,
            )
        };
        let mut builder = Self {
            handle: NonNull::new(pointer),
            view,
            len: 0,
            scope: PhantomData,
            element: PhantomData,
        };
        check(status, "OexDenseBuilder_CreateUninitialized")?;
        if builder.handle.is_none() {
            return Err(Error::with_kind(ErrorKind::Abi, "null dense builder"));
        }
        check_element::<T>(view.data_type, view.element_size)?;
        builder.len = length(view.element_count)?;
        // Validate Rust slice constraints now, before an infallible accessor.
        slice_pointer(view.data.cast::<MaybeUninit<T>>(), builder.len)?;
        slice_pointer(view.dimensions, view.rank as usize)?;
        Ok(builder)
    }

    pub fn dimensions(&self) -> &[u64] {
        // SAFETY: the live builder owns immutable dimensions, checked at creation.
        unsafe { read_slice(self.view.dimensions, self.view.rank as usize) }
            .expect("validated dimensions")
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn uninit_slice(&mut self) -> &mut [MaybeUninit<T>] {
        // SAFETY: exclusive builder borrow; MaybeUninit permits unwritten elements.
        unsafe { write_slice(self.view.data.cast(), self.len) }.expect("validated builder storage")
    }
    pub fn fill(mut self, value: T) -> Result<Value<'call>> {
        for slot in self.uninit_slice() {
            slot.write(value);
        }
        // SAFETY: the loop initialized every element with a valid T.
        unsafe { self.assume_init() }
    }
    pub fn fill_with(mut self, mut f: impl FnMut(usize) -> T) -> Result<Value<'call>> {
        for (index, slot) in self.uninit_slice().iter_mut().enumerate() {
            slot.write(f(index));
        }
        // SAFETY: each slot was initialized; a panic above instead drops/aborts self.
        unsafe { self.assume_init() }
    }
    pub fn copy_from_slice(mut self, values: &[T]) -> Result<Value<'call>> {
        if values.len() != self.len {
            return Err(Error::with_kind(
                ErrorKind::Dimension,
                "array data length does not match shape",
            ));
        }
        for (slot, value) in self.uninit_slice().iter_mut().zip(values) {
            slot.write(*value);
        }
        // SAFETY: length equality and the loop initialize the entire allocation.
        unsafe { self.assume_init() }
    }
    /// Commits a buffer initialized through `uninit_slice`.
    ///
    /// # Safety
    /// Every element must contain a valid T, including canonical logical bytes.
    pub unsafe fn assume_init(mut self) -> Result<Value<'call>> {
        let pointer = self.handle.take().expect("live builder").as_ptr();
        let mut value = std::ptr::null_mut();
        // SAFETY: caller initialized all elements. Commit consumes even on failure.
        let status = unsafe { sys::OexDenseBuilder_Commit(pointer, &mut value) };
        // SAFETY: Commit's out pointer transfers an owned value, scoped to this call.
        unsafe { Value::from_out(status, value, "OexDenseBuilder_Commit") }
    }
}

impl<T: Element> Drop for DenseBuilder<'_, T> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // SAFETY: this wrapper still owns an unconsumed, call-scoped builder.
            unsafe { sys::OexDenseBuilder_Abort(handle.as_ptr()) };
        }
    }
}

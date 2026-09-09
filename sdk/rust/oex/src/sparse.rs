use std::{marker::PhantomData, mem::MaybeUninit, ptr::NonNull};

use crate::{Call, Complex, Element, Error, ErrorKind, Logical, Result, Scope, Value, sys};
use crate::{
    array::write_slice,
    error::{check, length},
    types::check_element,
};

mod sealed {
    pub trait Sealed {}
}
/// The three element types supported by the OEX sparse ABI.
pub trait SparseElement: Element + sealed::Sealed {}
impl sealed::Sealed for Logical {}
impl sealed::Sealed for f64 {}
impl sealed::Sealed for Complex<f64> {}
impl SparseElement for Logical {}
impl SparseElement for f64 {}
impl SparseElement for Complex<f64> {}

#[derive(Clone, Copy, Debug)]
pub struct SparseView<'a, T> {
    pub(crate) rows: u64,
    pub(crate) columns: u64,
    pub(crate) column_offsets: &'a [u64],
    pub(crate) row_indices: &'a [u64],
    pub(crate) values: &'a [T],
}
impl<'a, T> SparseView<'a, T> {
    pub fn rows(&self) -> u64 {
        self.rows
    }
    pub fn columns(&self) -> u64 {
        self.columns
    }
    pub fn column_offsets(&self) -> &'a [u64] {
        self.column_offsets
    }
    pub fn row_indices(&self) -> &'a [u64] {
        self.row_indices
    }
    pub fn values(&self) -> &'a [T] {
        self.values
    }
    pub fn stored_count(&self) -> usize {
        self.values.len()
    }
}

/// Immutable persistent CSC topology. Not Send/Sync: pattern API operations
/// follow the C contract's call-thread restriction, even between callbacks.
pub struct SparsePattern {
    handle: NonNull<sys::OexSparsePattern>,
    thread: PhantomData<std::rc::Rc<()>>,
}
impl SparsePattern {
    pub(crate) fn new(
        call: &Call<'_>,
        rows: u64,
        columns: u64,
        offsets: &[u64],
        indices: &[u64],
    ) -> Result<Self> {
        if columns.checked_add(1) != Some(offsets.len() as u64) {
            return Err(Error::with_kind(
                ErrorKind::Dimension,
                "CSC offsets length must equal columns + 1",
            ));
        }
        let mut pointer = std::ptr::null_mut();
        // SAFETY: offsets/indices have the exact lengths passed to the host.
        let status = unsafe {
            sys::OexSparsePattern_Create(
                call.as_raw(),
                rows,
                columns,
                indices.len() as u64,
                offsets.as_ptr(),
                indices.as_ptr(),
                &mut pointer,
            )
        };
        let pattern = NonNull::new(pointer).map(|handle| Self {
            handle,
            thread: PhantomData,
        });
        check(status, "OexSparsePattern_Create")?;
        pattern.ok_or_else(|| Error::with_kind(ErrorKind::Abi, "null sparse pattern"))
    }
    pub fn as_raw(&self) -> *const sys::OexSparsePattern {
        self.handle.as_ptr()
    }
    pub fn try_clone(&self) -> Result<Self> {
        let mut pointer = std::ptr::null_mut();
        // SAFETY: pattern is alive and thread-confined; Retain creates a new handle.
        let status = unsafe { sys::OexSparsePattern_Retain(self.as_raw(), &mut pointer) };
        let pattern = NonNull::new(pointer).map(|handle| Self {
            handle,
            thread: PhantomData,
        });
        check(status, "OexSparsePattern_Retain")?;
        pattern.ok_or_else(|| Error::with_kind(ErrorKind::Abi, "null retained pattern"))
    }
}
impl Drop for SparsePattern {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns exactly one persistent pattern handle.
        unsafe { sys::OexSparsePattern_Release(self.handle.as_ptr()) };
    }
}

/// Push initialized triplets into the host buffer, then consume with `finish`.
/// The host sorts, combines duplicates and removes zeros at commit.
pub struct SparseTripletBuilder<'call, T: SparseElement> {
    handle: Option<NonNull<sys::OexSparseTripletBuilder>>,
    view: sys::OexSparseTripletView,
    len: usize,
    capacity: usize,
    scope: Scope<'call>,
    element: PhantomData<T>,
}
impl<'call, T: SparseElement> SparseTripletBuilder<'call, T> {
    pub(crate) fn new(
        call: &Call<'call>,
        rows: u64,
        columns: u64,
        capacity: usize,
    ) -> Result<Self> {
        let mut pointer = std::ptr::null_mut();
        let mut view = sys::OexSparseTripletView::default();
        // SAFETY: call and both out pointers are live; no input buffers involved.
        let status = unsafe {
            sys::OexSparseTripletBuilder_CreateUninitialized(
                call.as_raw(),
                T::DATA_TYPE.as_raw(),
                rows,
                columns,
                capacity as u64,
                &mut pointer,
                &mut view,
            )
        };
        let builder = Self {
            handle: NonNull::new(pointer),
            view,
            len: 0,
            capacity,
            scope: PhantomData,
            element: PhantomData,
        };
        check(status, "OexSparseTripletBuilder_CreateUninitialized")?;
        if builder.handle.is_none() || view.capacity != capacity as u64 {
            return Err(Error::with_kind(
                ErrorKind::Abi,
                "invalid sparse triplet allocation",
            ));
        }
        check_element::<T>(view.data_type, view.element_size)?;
        Ok(builder)
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn push(&mut self, row: u64, column: u64, value: T) -> Result<()> {
        if self.len == self.capacity {
            return Err(Error::with_kind(
                ErrorKind::Range,
                "triplet capacity exhausted",
            ));
        }
        if row >= self.view.rows || column >= self.view.columns {
            return Err(Error::with_kind(
                ErrorKind::Range,
                "triplet coordinate out of range",
            ));
        }
        // SAFETY: len < capacity, the allocation belongs exclusively to self, and
        // all three entries are written before advancing the initialized prefix.
        unsafe {
            self.view.row_indices.add(self.len).write(row);
            self.view.column_indices.add(self.len).write(column);
            self.view.values.cast::<T>().add(self.len).write(value);
        }
        self.len += 1;
        Ok(())
    }
    pub fn extend(&mut self, entries: impl IntoIterator<Item = (u64, u64, T)>) -> Result<()> {
        for (row, column, value) in entries {
            self.push(row, column, value)?;
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<Value<'call>> {
        let pointer = self.handle.take().expect("live builder").as_ptr();
        let mut value = std::ptr::null_mut();
        // SAFETY: exactly len entries are initialized. Commit consumes on failure too.
        let status =
            unsafe { sys::OexSparseTripletBuilder_Commit(pointer, self.len as u64, &mut value) };
        // SAFETY: Commit returns an owned value for this callback.
        unsafe { Value::from_out(status, value, "OexSparseTripletBuilder_Commit") }
    }
}
impl<T: SparseElement> Drop for SparseTripletBuilder<'_, T> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // SAFETY: this is an unconsumed builder owned by this live callback.
            unsafe { sys::OexSparseTripletBuilder_Abort(handle.as_ptr()) };
        }
    }
}

/// Allocates values for a persistent pattern. The host retains the topology,
/// so the original pattern can be dropped while this builder is alive.
pub struct SparsePatternBuilder<'call, T: SparseElement> {
    handle: Option<NonNull<sys::OexSparsePatternBuilder>>,
    data: *mut MaybeUninit<T>,
    len: usize,
    scope: Scope<'call>,
}
impl<'call, T: SparseElement> SparsePatternBuilder<'call, T> {
    pub(crate) fn new(call: &Call<'call>, pattern: &SparsePattern) -> Result<Self> {
        let mut pointer = std::ptr::null_mut();
        let mut view = sys::OexSparsePatternValuesView::default();
        // SAFETY: call/pattern/out pointers are live; host retains the pattern.
        let status = unsafe {
            sys::OexSparsePatternBuilder_CreateUninitialized(
                call.as_raw(),
                pattern.as_raw(),
                T::DATA_TYPE.as_raw(),
                &mut pointer,
                &mut view,
            )
        };
        let mut builder = Self {
            handle: NonNull::new(pointer),
            data: view.values.cast(),
            len: 0,
            scope: PhantomData,
        };
        check(status, "OexSparsePatternBuilder_CreateUninitialized")?;
        if builder.handle.is_none() {
            return Err(Error::with_kind(
                ErrorKind::Abi,
                "null sparse pattern builder",
            ));
        }
        check_element::<T>(view.data_type, view.element_size)?;
        builder.len = length(view.stored_count)?;
        // SAFETY: new builder exclusively owns these possibly uninitialized slots.
        unsafe { write_slice(builder.data, builder.len)? };
        Ok(builder)
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn uninit_slice(&mut self) -> &mut [MaybeUninit<T>] {
        // SAFETY: exclusive borrow of checked, live builder storage.
        unsafe { write_slice(self.data, self.len) }.expect("validated pattern storage")
    }
    pub fn fill(mut self, value: T) -> Result<Value<'call>> {
        for slot in self.uninit_slice() {
            slot.write(value);
        }
        // SAFETY: every pattern value has been initialized.
        unsafe { self.assume_init() }
    }
    pub fn fill_with(mut self, mut f: impl FnMut(usize) -> T) -> Result<Value<'call>> {
        for (index, slot) in self.uninit_slice().iter_mut().enumerate() {
            slot.write(f(index));
        }
        // SAFETY: the entire pattern value buffer is initialized.
        unsafe { self.assume_init() }
    }
    pub fn copy_from_slice(mut self, values: &[T]) -> Result<Value<'call>> {
        if self.len != values.len() {
            return Err(Error::with_kind(
                ErrorKind::Dimension,
                "pattern value count mismatch",
            ));
        }
        for (slot, value) in self.uninit_slice().iter_mut().zip(values) {
            slot.write(*value);
        }
        // SAFETY: every value slot has been initialized.
        unsafe { self.assume_init() }
    }
    /// # Safety
    /// All slots exposed by `uninit_slice` must contain valid T values.
    pub unsafe fn assume_init(mut self) -> Result<Value<'call>> {
        let pointer = self.handle.take().expect("live builder").as_ptr();
        let mut value = std::ptr::null_mut();
        // SAFETY: caller initialized all values; Commit consumes even on failure.
        let status = unsafe { sys::OexSparsePatternBuilder_Commit(pointer, &mut value) };
        // SAFETY: Commit returns ownership for this callback.
        unsafe { Value::from_out(status, value, "OexSparsePatternBuilder_Commit") }
    }
}
impl<T: SparseElement> Drop for SparsePatternBuilder<'_, T> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // SAFETY: wrapper still owns the builder and its callback is live.
            unsafe { sys::OexSparsePatternBuilder_Abort(handle.as_ptr()) };
        }
    }
}

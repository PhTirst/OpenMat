use std::{marker::PhantomData, ptr::NonNull};

use crate::{
    ArrayView, ArrayViewMut, DataType, Element, Error, ErrorKind, Result, Scope, SparseElement,
    SparseView, ValueInfo, ValueKind, sys,
};
use crate::{
    array::{read_slice, write_slice},
    error::{check, length},
    types::check_element,
};

/// An owned OEX value. Moving transfers ownership; `try_clone` retains COW storage.
/// Values are confined to their callback and cannot be sent to another thread.
pub struct Value<'call> {
    handle: NonNull<sys::OexValue>,
    scope: Scope<'call>,
}

/// A read-only handle borrowing an input slot or owned value.
#[derive(Clone, Copy)]
pub struct ValueRef<'value, 'call> {
    pub(crate) handle: NonNull<sys::OexValue>,
    pub(crate) owner: PhantomData<&'value Value<'call>>,
}

impl<'call> Value<'call> {
    /// Takes ownership of a raw value without retaining it.
    ///
    /// # Safety
    /// The handle must be live, exclusively owned, associated with the current
    /// callback, and valid for 'call. It must not be released by anyone else.
    pub unsafe fn from_raw(handle: *mut sys::OexValue) -> Result<Self> {
        let handle = NonNull::new(handle)
            .ok_or_else(|| Error::with_kind(ErrorKind::Abi, "null owned value"))?;
        Ok(Self {
            handle,
            scope: PhantomData,
        })
    }
    pub(crate) unsafe fn from_out(
        status: u32,
        handle: *mut sys::OexValue,
        operation: &'static str,
    ) -> Result<Self> {
        // Adopt before checking the status so any returned ownership is cleaned up.
        // SAFETY: caller obtained this owned out pointer from the C API.
        let value = NonNull::new(handle)
            .map(|handle| unsafe { Self::from_raw(handle.as_ptr()).expect("nonnull") });
        check(status, operation)?;
        value.ok_or_else(|| {
            Error::with_kind(ErrorKind::Abi, "successful operation returned null value")
        })
    }
    pub fn as_raw(&self) -> *mut sys::OexValue {
        self.handle.as_ptr()
    }
    pub fn into_raw(self) -> *mut sys::OexValue {
        let pointer = self.as_raw();
        std::mem::forget(self);
        pointer
    }
    pub fn as_value_ref(&self) -> ValueRef<'_, 'call> {
        ValueRef {
            handle: self.handle,
            owner: PhantomData,
        }
    }
    pub fn info(&self) -> Result<ValueInfo> {
        self.as_value_ref().info()
    }
    pub fn kind(&self) -> Result<ValueKind> {
        self.as_value_ref().kind()
    }
    pub fn data_type(&self) -> Result<DataType> {
        self.as_value_ref().data_type()
    }
    pub fn is_dense<T: Element>(&self) -> Result<bool> {
        self.as_value_ref().is_dense::<T>()
    }
    pub fn is_scalar<T: Element>(&self) -> Result<bool> {
        self.as_value_ref().is_scalar::<T>()
    }
    pub fn is_sparse<T: SparseElement>(&self) -> Result<bool> {
        self.as_value_ref().is_sparse::<T>()
    }
    pub fn as_f64(&self) -> Result<f64> {
        self.as_value_ref().as_f64()
    }
    pub fn try_clone(&self) -> Result<Value<'call>> {
        self.as_value_ref().try_to_owned()
    }
    pub fn array<T: Element>(&self) -> Result<ArrayView<'_, T>> {
        self.as_value_ref().array()
    }
    pub fn sparse<T: SparseElement>(&self) -> Result<SparseView<'_, T>> {
        self.as_value_ref().sparse()
    }
    pub fn array_mut<T: Element>(&mut self) -> Result<ArrayViewMut<'_, T>> {
        let info = self.info()?;
        check_element::<T>(info.data_type.as_raw(), info.element_size)?;
        let mut view = sys::OexMutableDenseView::default();
        check(
            // SAFETY: exclusive ownership borrow prevents retain/release or other views.
            unsafe { sys::OexDense_GetMutableView(self.as_raw(), &mut view) },
            "OexDense_GetMutableView",
        )?;
        check_element::<T>(view.data_type, view.element_size)?;
        // SAFETY: host returns initialized, disjoint dimensions/data after COW detach.
        unsafe {
            Ok(ArrayViewMut {
                dimensions: read_slice(view.dimensions, view.rank as usize)?,
                data: write_slice(view.data.cast(), length(view.element_count)?)?,
            })
        }
    }
}

impl Drop for Value<'_> {
    fn drop(&mut self) {
        // SAFETY: this is the only wrapper owning this handle, with no live borrows.
        unsafe { sys::OexValue_Release(self.handle.as_ptr()) };
    }
}

impl<'value, 'call> ValueRef<'value, 'call> {
    pub fn as_raw(self) -> *const sys::OexValue {
        self.handle.as_ptr()
    }
    pub fn info(self) -> Result<ValueInfo> {
        let mut info = sys::OexValueInfo {
            struct_size: sys::abi_size::<sys::OexValueInfo>(),
            ..Default::default()
        };
        check(
            // SAFETY: the owner/slot borrow keeps the handle live; info is writable.
            unsafe { sys::OexValue_GetInfo(self.as_raw(), &mut info) },
            "OexValue_GetInfo",
        )?;
        Ok(ValueInfo {
            kind: ValueKind::from_raw(info.kind),
            data_type: DataType::from_raw(info.data_type),
            rank: info.rank,
            element_size: info.element_size,
            element_count: info.element_count,
            is_complex: info.flags & sys::OEX_VALUE_FLAG_COMPLEX != 0,
        })
    }
    pub fn kind(self) -> Result<ValueKind> {
        Ok(self.info()?.kind)
    }
    pub fn data_type(self) -> Result<DataType> {
        Ok(self.info()?.data_type)
    }
    pub fn is_dense<T: Element>(self) -> Result<bool> {
        let info = self.info()?;
        Ok(info.kind == ValueKind::Dense && info.data_type == T::DATA_TYPE)
    }
    pub fn is_scalar<T: Element>(self) -> Result<bool> {
        let info = self.info()?;
        Ok(info.kind == ValueKind::Dense
            && info.data_type == T::DATA_TYPE
            && info.element_count == 1)
    }
    pub fn is_sparse<T: SparseElement>(self) -> Result<bool> {
        let info = self.info()?;
        Ok(info.kind == ValueKind::Sparse && info.data_type == T::DATA_TYPE)
    }
    pub fn as_f64(self) -> Result<f64> {
        let mut value = 0.0;
        check(
            // SAFETY: the borrowed handle is live and the scalar out pointer is valid.
            unsafe { sys::OexValue_GetFloat64(self.as_raw(), &mut value) },
            "OexValue_GetFloat64",
        )?;
        Ok(value)
    }
    pub fn try_to_owned(self) -> Result<Value<'call>> {
        let mut value = std::ptr::null_mut();
        // SAFETY: retaining a live shared handle creates a distinct owned COW handle.
        let status = unsafe { sys::OexValue_Retain(self.as_raw(), &mut value) };
        // SAFETY: Retain transfers the resulting handle to this callback.
        unsafe { Value::from_out(status, value, "OexValue_Retain") }
    }
    pub fn array<T: Element>(self) -> Result<ArrayView<'value, T>> {
        let mut view = sys::OexDenseView::default();
        check(
            // SAFETY: the handle and its immutable storage are borrowed for 'value.
            unsafe { sys::OexDense_GetView(self.as_raw(), &mut view) },
            "OexDense_GetView",
        )?;
        check_element::<T>(view.data_type, view.element_size)?;
        // SAFETY: C API returns initialized read-only views for the live handle.
        unsafe {
            Ok(ArrayView {
                dimensions: read_slice(view.dimensions, view.rank as usize)?,
                data: read_slice(view.data.cast(), length(view.element_count)?)?,
            })
        }
    }
    pub fn sparse<T: SparseElement>(self) -> Result<SparseView<'value, T>> {
        let mut view = sys::OexSparseCscView::default();
        check(
            // SAFETY: the handle is borrowed for 'value; view is a writable out struct.
            unsafe { sys::OexSparse_GetCscView(self.as_raw(), &mut view) },
            "OexSparse_GetCscView",
        )?;
        check_element::<T>(view.data_type, view.element_size)?;
        let columns = view
            .columns
            .checked_add(1)
            .ok_or_else(|| Error::with_kind(ErrorKind::Range, "CSC column count overflow"))?;
        // SAFETY: the C API guarantees initialized immutable CSC arrays.
        unsafe {
            Ok(SparseView {
                rows: view.rows,
                columns: view.columns,
                column_offsets: read_slice(view.column_offsets, length(columns)?)?,
                row_indices: read_slice(view.row_indices, length(view.stored_count)?)?,
                values: read_slice(view.values.cast(), length(view.stored_count)?)?,
            })
        }
    }
}

use std::{
    cell::Cell,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
};

use crate::error::{check, count};
use crate::{
    DenseBuilder, Element, Error, ErrorKind, Result, Scope, SparseElement, SparsePattern,
    SparsePatternBuilder, SparseTripletBuilder, Value, ValueRef, sys,
};

/// The current synchronous plugin invocation. Created by the callback adapter.
pub struct Call<'call> {
    handle: NonNull<sys::OexCall>,
    invoke_failure: Cell<Option<u32>>,
    scope: Scope<'call>,
}

impl<'call> Call<'call> {
    pub fn as_raw(&self) -> *mut sys::OexCall {
        self.handle.as_ptr()
    }
    pub fn input_count(&self) -> u32 {
        // SAFETY: the adapter keeps the current call alive.
        unsafe { sys::OexCall_GetInputCount(self.as_raw()) }
    }
    pub fn output_count(&self) -> u32 {
        // SAFETY: the adapter keeps the current call alive.
        unsafe { sys::OexCall_GetOutputCount(self.as_raw()) }
    }
    /// Borrows an input. While the borrow is used, `take_input` cannot be called.
    pub fn input(&self, index: u32) -> Result<ValueRef<'_, 'call>> {
        if index >= self.input_count() {
            return Err(Error::with_kind(
                ErrorKind::Range,
                "input index out of range",
            ));
        }
        // SAFETY: the slot cannot be taken while the returned self borrow is live.
        let pointer = unsafe { sys::OexCall_BorrowInput(self.as_raw(), index) };
        let handle = NonNull::new(pointer.cast_mut()).ok_or_else(|| {
            Error::with_kind(ErrorKind::State, "input is missing or already taken")
        })?;
        Ok(ValueRef {
            handle,
            owner: PhantomData,
        })
    }
    pub fn take_input(&mut self, index: u32) -> Result<Value<'call>> {
        let mut pointer = std::ptr::null_mut();
        // SAFETY: exclusive Call borrow prevents live borrowed input handles.
        let status = unsafe { sys::OexCall_TakeInput(self.as_raw(), index, &mut pointer) };
        // SAFETY: TakeInput transfers an owned handle scoped to this call.
        unsafe { Value::from_out(status, pointer, "OexCall_TakeInput") }
    }
    /// Transfers ownership on success; drops the value on failure.
    pub fn set_output(&self, index: u32, value: Value<'call>) -> Result<()> {
        // SAFETY: value is owned with no outstanding borrows; call is live.
        let status = unsafe { sys::OexCall_SetOutput(self.as_raw(), index, value.as_raw()) };
        check(status, "OexCall_SetOutput")?;
        let _ = value.into_raw();
        Ok(())
    }
    pub fn scalar(&self, value: f64) -> Result<Value<'call>> {
        let mut pointer = std::ptr::null_mut();
        // SAFETY: call and out pointer are valid; scalar is passed by value.
        let status = unsafe { sys::OexValue_CreateFloat64(self.as_raw(), value, &mut pointer) };
        // SAFETY: CreateFloat64 returns ownership for this callback.
        unsafe { Value::from_out(status, pointer, "OexValue_CreateFloat64") }
    }
    pub fn dense_builder<T: Element>(&self, dimensions: &[u64]) -> Result<DenseBuilder<'call, T>> {
        DenseBuilder::new(self, dimensions)
    }
    pub fn array<T: Element>(&self, dimensions: &[u64], values: &[T]) -> Result<Value<'call>> {
        self.dense_builder(dimensions)?.copy_from_slice(values)
    }
    pub fn zeros<T: Element>(&self, dimensions: &[u64]) -> Result<Value<'call>> {
        self.dense_builder(dimensions)?.fill(T::default())
    }
    pub fn sparse_triplets<T: SparseElement>(
        &self,
        rows: u64,
        columns: u64,
        capacity: usize,
    ) -> Result<SparseTripletBuilder<'call, T>> {
        SparseTripletBuilder::new(self, rows, columns, capacity)
    }
    pub fn sparse_pattern(
        &self,
        rows: u64,
        columns: u64,
        column_offsets: &[u64],
        row_indices: &[u64],
    ) -> Result<SparsePattern> {
        SparsePattern::new(self, rows, columns, column_offsets, row_indices)
    }
    pub fn sparse_values<T: SparseElement>(
        &self,
        pattern: &SparsePattern,
    ) -> Result<SparsePatternBuilder<'call, T>> {
        SparsePatternBuilder::new(self, pattern)
    }
    /// Calls a language function. An original language error remains pending in
    /// the host even if plugin code handles this Result and returns Ok(()).
    pub fn invoke(
        &self,
        callable: ValueRef<'_, 'call>,
        inputs: &[ValueRef<'_, 'call>],
        output_count: u32,
    ) -> Result<Vec<Value<'call>>> {
        let input_count = count(inputs.len())?;
        let allocation = |_| {
            Error::with_kind(
                ErrorKind::Allocation,
                "cannot allocate invocation arguments/results",
            )
        };
        let mut input_pointers = Vec::new();
        input_pointers
            .try_reserve_exact(inputs.len())
            .map_err(allocation)?;
        input_pointers.extend(inputs.iter().map(|v| v.as_raw()));
        let mut output_pointers = Vec::new();
        output_pointers
            .try_reserve_exact(output_count as usize)
            .map_err(allocation)?;
        output_pointers.resize(output_count as usize, std::ptr::null_mut());
        let mut outputs = Vec::new();
        outputs
            .try_reserve_exact(output_count as usize)
            .map_err(allocation)?;
        // SAFETY: all handles are borrowed; buffers are correctly sized and live.
        let status = unsafe {
            sys::OexCall_Invoke(
                self.as_raw(),
                callable.as_raw(),
                input_count,
                input_pointers.as_ptr(),
                output_count,
                output_pointers.as_mut_ptr(),
            )
        };
        for pointer in output_pointers {
            if !pointer.is_null() {
                // SAFETY: every nonnull invocation output is newly owned.
                outputs.push(unsafe { Value::from_raw(pointer)? });
            }
        }
        if status != sys::OEX_OK {
            self.invoke_failure.set(Some(status));
        }
        check(status, "OexCall_Invoke")?;
        if outputs.len() != output_count as usize {
            return Err(Error::with_kind(
                ErrorKind::Abi,
                "missing invocation output",
            ));
        }
        Ok(outputs)
    }
    pub fn cancellation(&self) -> Cancellation<'call> {
        // SAFETY: cancellation handle has the entire callback lifetime.
        let handle = unsafe { sys::OexCall_GetCancellation(self.as_raw()) };
        Cancellation {
            handle,
            lifetime: PhantomData,
        }
    }
    pub fn check_cancelled(&self) -> Result<()> {
        self.cancellation().check()
    }
}

/// Thread-safe cancellation polling, valid only until this callback returns.
#[derive(Clone, Copy)]
pub struct Cancellation<'call> {
    handle: *const sys::OexCancellation,
    lifetime: PhantomData<&'call ()>,
}
// SAFETY: OexCancellation_IsRequested is explicitly thread-safe; lifetime is scoped.
unsafe impl Send for Cancellation<'_> {}
// SAFETY: concurrent cancellation polling is explicitly allowed by the C contract.
unsafe impl Sync for Cancellation<'_> {}
impl Cancellation<'_> {
    pub fn is_requested(self) -> bool {
        // SAFETY: the handle is live for this callback, including joined workers.
        unsafe { sys::OexCancellation_IsRequested(self.handle) != 0 }
    }
    pub fn check(self) -> Result<()> {
        if self.is_requested() {
            Err(Error::with_kind(
                ErrorKind::Cancelled,
                "operation cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

pub(crate) fn utf8(value: &str) -> sys::OexUtf8View {
    sys::OexUtf8View {
        data: value.as_ptr().cast(),
        length: value.len() as u64,
    }
}

/// Runs Rust code inside one OEX callback boundary.
///
/// # Safety
/// `raw` must be the current live host callback, with no other Call wrapper or
/// outstanding raw input borrows. The host must obey oex.h throughout the call.
pub unsafe fn callback(
    raw: *mut sys::OexCall,
    f: impl for<'call> FnOnce(&mut Call<'call>) -> Result<()>,
) -> u32 {
    let Some(handle) = NonNull::new(raw) else {
        return sys::OEX_ERROR_ARGUMENT;
    };
    let mut call = Call {
        handle,
        invoke_failure: Cell::new(None),
        scope: PhantomData,
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| f(&mut call)));
    // Preserve the original language error, including when Rust code replaces
    // its Result with another error or a panic. Never call SetError over it.
    if let Some(status) = call.invoke_failure.get() {
        if let Err(payload) = outcome {
            std::mem::forget(payload);
        }
        return status;
    }
    let error = match outcome {
        Ok(Ok(())) => return sys::OEX_OK,
        Ok(Err(error)) => error,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("Rust plugin panicked")
                .to_owned();
            // An arbitrary panic payload may itself panic in Drop. Leaking it on
            // this exceptional path prevents a second unwind crossing C.
            std::mem::forget(payload);
            Error::new("OpenMat:OEX:RustPanic", message)
        }
    };
    // SAFETY: SetError copies the strings while raw and error are still alive.
    unsafe {
        sys::OexCall_SetError(
            raw,
            utf8(error.identifier().unwrap_or("OpenMat:OEX:Rust")),
            utf8(error.message()),
        )
    };
    error.status()
}

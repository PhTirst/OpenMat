use std::{
    ffi::c_void,
    marker::PhantomData,
    ops::RangeInclusive,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{Call, Error, ErrorKind, NativeMethod, NativeProperty, Result, Value, ValueRef, sys};
use crate::{call::callback, error::check, registration::ClassDefinition};

/// A unique static registration token for a Rust native handle class.
/// Use one `static NativeClass<T>` per registered class, even when T is shared.
pub struct NativeClass<T: Send + 'static> {
    // Nonzero size ensures separate statics have distinct addresses.
    _token: u8,
    ty: PhantomData<fn() -> T>,
}
impl<T: Send + 'static> Default for NativeClass<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: Send + 'static> NativeClass<T> {
    pub const fn new() -> Self {
        Self {
            _token: 0,
            ty: PhantomData,
        }
    }
    pub const fn type_token(&'static self) -> *const c_void {
        std::ptr::from_ref(self).cast()
    }

    /// # Safety
    /// Callback must be a no-unwind adapter for this SDK's T instance layout.
    pub const unsafe fn method_definition(
        &self,
        name: &'static str,
        inputs: RangeInclusive<u32>,
        outputs: RangeInclusive<u32>,
        invoke: sys::OexNativeMethod,
    ) -> NativeMethod<T> {
        // SAFETY: forwarded from the documented caller requirement.
        unsafe { NativeMethod::from_raw_callback(name, inputs, outputs, invoke) }
    }

    /// # Safety
    /// Constructor must allocate `Box<Arc<Mutex<T>>>` using the SDK adapter, must
    /// not unwind, and must transfer that box only on success. Prefer `class!`.
    pub const unsafe fn definition(
        &'static self,
        name: &'static str,
        constructor: sys::OexNativeConstructor,
        methods: &'static [NativeMethod<T>],
        properties: &'static [NativeProperty<T>],
    ) -> ClassDefinition {
        ClassDefinition(sys::OexNativeClassDefinition {
            struct_size: sys::abi_size::<sys::OexNativeClassDefinition>(),
            semantics: sys::OEX_CLASS_HANDLE,
            name: sys::OexUtf8View {
                data: name.as_ptr().cast(),
                length: name.len() as u64,
            },
            constructor: Some(constructor),
            destructor: Some(destroy::<T>),
            methods: methods.as_ptr().cast(),
            method_count: methods.len() as u64,
            type_token: self.type_token(),
            properties: properties.as_ptr().cast(),
            property_count: properties.len() as u64,
        })
    }

    pub fn create<'call>(&'static self, call: &Call<'call>, state: T) -> Result<Value<'call>> {
        let instance = Box::into_raw(Box::new(Arc::new(Mutex::new(state))));
        let mut value = std::ptr::null_mut();
        // SAFETY: token identifies this registered T layout; instance is plugin-owned.
        let status = unsafe {
            sys::OexNativeObject_Create(
                call.as_raw(),
                self.type_token(),
                instance.cast(),
                &mut value,
            )
        };
        if status != sys::OEX_OK {
            // SAFETY: the C API leaves instance ownership with the plugin on failure.
            unsafe { drop(Box::from_raw(instance)) };
        }
        // SAFETY: successful Create transfers the instance, returning an owned value.
        unsafe { Value::from_out(status, value, "OexNativeObject_Create") }
    }

    /// Retains Rust state, after the host validates the exact registered token.
    /// The returned object owns an Arc, so nested language calls cannot free it.
    pub fn borrow(
        &'static self,
        call: &Call<'_>,
        value: ValueRef<'_, '_>,
    ) -> Result<NativeObject<T>> {
        let mut instance = std::ptr::null_mut();
        check(
            // SAFETY: call/value/token are live; output pointer is writable.
            unsafe {
                sys::OexNativeObject_BorrowInstance(
                    call.as_raw(),
                    value.as_raw(),
                    self.type_token(),
                    &mut instance,
                )
            },
            "OexNativeObject_BorrowInstance",
        )?;
        // SAFETY: exact token validation proves the registered SDK representation.
        unsafe { NativeObject::from_instance(instance) }
    }
}

/// Shared Rust ownership of native state. `try_lock` enforces exclusive access
/// across reentrant callbacks and across threads, without blocking the runtime.
pub struct NativeObject<T: Send + 'static> {
    state: Arc<Mutex<T>>,
}
impl<T: Send + 'static> Clone for NativeObject<T> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}
impl<T: Send + 'static> NativeObject<T> {
    unsafe fn from_instance(instance: *mut c_void) -> Result<Self> {
        if instance.is_null() {
            return Err(Error::with_kind(ErrorKind::Abi, "null native instance"));
        }
        // SAFETY: caller guarantees a live SDK Box<Arc<Mutex<T>>> instance.
        let state = unsafe { &*instance.cast::<Arc<Mutex<T>>>() };
        Ok(Self {
            state: Arc::clone(state),
        })
    }
    pub fn try_lock(&self) -> Result<MutexGuard<'_, T>> {
        self.state.try_lock().map_err(|_| {
            Error::with_kind(
                ErrorKind::State,
                "native object is already borrowed or poisoned",
            )
        })
    }
}

unsafe extern "C" fn destroy<T: Send + 'static>(instance: *mut c_void) {
    if !instance.is_null() {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: host returns exactly the SDK box whose ownership it accepted.
            unsafe { drop(Box::from_raw(instance.cast::<Arc<Mutex<T>>>())) };
        }));
        if let Err(payload) = outcome {
            std::mem::forget(payload);
        }
    }
}

/// # Safety
/// raw must be a host callback, output writable, and this adapter must be
/// installed only as the supplied class's native constructor.
pub unsafe fn constructor_callback<T: Send + 'static>(
    raw: *mut sys::OexCall,
    output: *mut *mut c_void,
    _class: &'static NativeClass<T>,
    constructor: impl for<'call> FnOnce(&mut Call<'call>) -> Result<T>,
) -> u32 {
    if output.is_null() {
        return sys::OEX_ERROR_ARGUMENT;
    }
    // SAFETY: host supplied the writable native-instance out pointer.
    unsafe { output.write(std::ptr::null_mut()) };
    // Keep ownership until the boundary has returned success, including when
    // the constructor swallowed a pending language error.
    let mut instance = None;
    // SAFETY: forwarded live host callback; the closure cannot escape call values.
    let status = unsafe {
        callback(raw, |call| {
            instance = Some(Box::new(Arc::new(Mutex::new(constructor(call)?))));
            Ok(())
        })
    };
    if status == sys::OEX_OK {
        if let Some(instance) = instance {
            // SAFETY: transfer to the host only after all callback checks succeeded.
            unsafe { output.write(Box::into_raw(instance).cast()) };
        }
    } else {
        // A user Drop can panic too. Keep cleanup inside an unwind boundary.
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(instance))) {
            std::mem::forget(payload);
        }
    }
    status
}

/// # Safety
/// raw and instance must be a host method/property callback for this exact
/// `NativeClass<T>`, whose instances were allocated by the SDK.
pub unsafe fn method_callback<T: Send + 'static>(
    raw: *mut sys::OexCall,
    instance: *mut c_void,
    _class: &'static NativeClass<T>,
    method: impl for<'call> FnOnce(&mut Call<'call>, &mut T) -> Result<()>,
) -> u32 {
    // SAFETY: caller provides the current host call and the exact typed instance.
    unsafe {
        callback(raw, |call| {
            let object = NativeObject::<T>::from_instance(instance)?;
            let mut state = object.try_lock()?;
            method(call, &mut state)
        })
    }
}

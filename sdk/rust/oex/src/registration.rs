use std::{marker::PhantomData, ops::RangeInclusive};

use crate::sys;

const fn text(value: &'static str) -> sys::OexUtf8View {
    sys::OexUtf8View {
        data: value.as_ptr().cast(),
        length: value.len() as u64,
    }
}

/// Immutable static function metadata. Prefer `function!` for Rust callbacks.
#[repr(transparent)]
pub struct Function(sys::OexFunctionDefinition);
// SAFETY: private fields reference only immutable static text and code.
unsafe impl Sync for Function {}
impl Function {
    /// # Safety
    /// The callback must obey oex.h and never unwind across its C boundary.
    pub const unsafe fn from_raw_callback(
        name: &'static str,
        inputs: RangeInclusive<u32>,
        outputs: RangeInclusive<u32>,
        invoke: sys::OexFunctionCallback,
    ) -> Self {
        Self(sys::OexFunctionDefinition {
            struct_size: sys::abi_size::<sys::OexFunctionDefinition>(),
            flags: 0,
            name: text(name),
            minimum_inputs: *inputs.start(),
            maximum_inputs: *inputs.end(),
            minimum_outputs: *outputs.start(),
            maximum_outputs: *outputs.end(),
            invoke: Some(invoke),
        })
    }
}

#[repr(transparent)]
pub struct NativeMethod<T> {
    raw: sys::OexNativeMethodDefinition,
    ty: PhantomData<fn() -> T>,
}
// SAFETY: immutable static text and code only; no T instance is stored here.
unsafe impl<T> Sync for NativeMethod<T> {}
impl<T> NativeMethod<T> {
    /// # Safety
    /// The callback must use the SDK's native instance representation for T and
    /// contain panics. Prefer `method!`, which enforces both requirements.
    pub const unsafe fn from_raw_callback(
        name: &'static str,
        inputs: RangeInclusive<u32>,
        outputs: RangeInclusive<u32>,
        invoke: sys::OexNativeMethod,
    ) -> Self {
        Self {
            raw: sys::OexNativeMethodDefinition {
                struct_size: sys::abi_size::<sys::OexNativeMethodDefinition>(),
                flags: 0,
                name: text(name),
                minimum_inputs: *inputs.start(),
                maximum_inputs: *inputs.end(),
                minimum_outputs: *outputs.start(),
                maximum_outputs: *outputs.end(),
                invoke: Some(invoke),
            },
            ty: PhantomData,
        }
    }
}

#[repr(transparent)]
pub struct NativeProperty<T> {
    raw: sys::OexNativePropertyDefinition,
    ty: PhantomData<fn() -> T>,
}
// SAFETY: immutable static text and code only; no T instance is stored here.
unsafe impl<T> Sync for NativeProperty<T> {}
impl<T> NativeProperty<T> {
    pub const fn new(
        name: &'static str,
        getter: Option<NativeMethod<T>>,
        setter: Option<NativeMethod<T>>,
    ) -> Self {
        Self {
            raw: sys::OexNativePropertyDefinition {
                struct_size: sys::abi_size::<sys::OexNativePropertyDefinition>(),
                flags: 0,
                name: text(name),
                getter: match &getter {
                    Some(g) => g.raw.invoke,
                    None => None,
                },
                setter: match &setter {
                    Some(s) => s.raw.invoke,
                    None => None,
                },
            },
            ty: PhantomData,
        }
    }
}

/// Type-erased static metadata for a native class (created by `class!`).
#[repr(transparent)]
pub struct ClassDefinition(pub(crate) sys::OexNativeClassDefinition);
// SAFETY: constructed from static typed descriptors, static text and a static token.
unsafe impl Sync for ClassDefinition {}

/// Process-lifetime plugin metadata. Prefer `export_plugin!` for its entrypoint.
#[repr(transparent)]
pub struct Plugin(sys::OexPluginDefinition);
// SAFETY: all pointers are restricted by new() to immutable, static metadata.
unsafe impl Sync for Plugin {}
impl Plugin {
    pub const fn new(
        name: &'static str,
        version: &'static str,
        functions: &'static [Function],
        classes: &'static [ClassDefinition],
    ) -> Self {
        Self(sys::OexPluginDefinition {
            struct_size: sys::abi_size::<sys::OexPluginDefinition>(),
            required_abi: sys::OEX_ABI_VERSION,
            flags: 0,
            reserved: 0,
            name: text(name),
            version: text(version),
            functions: functions.as_ptr().cast(),
            function_count: functions.len() as u64,
            classes: classes.as_ptr().cast(),
            class_count: classes.len() as u64,
        })
    }
    pub const fn as_raw(&self) -> *const sys::OexPluginDefinition {
        &self.0
    }
}

/// Adapts `fn(&mut Call<'_>) -> Result<()>` to a C callback and static metadata.
#[macro_export]
macro_rules! function {
    ($name:expr, $inputs:expr, $outputs:expr, $handler:path) => {{
        struct Adapter;
        impl Adapter {
            unsafe extern "C" fn invoke(raw: *mut $crate::sys::OexCall) -> u32 {
                // SAFETY: OEX invokes this trampoline with its current callback context.
                unsafe { $crate::__private::callback(raw, $handler) }
            }
        }
        // SAFETY: the generated trampoline contains panics and translates Result.
        unsafe { $crate::Function::from_raw_callback($name, $inputs, $outputs, Adapter::invoke) }
    }};
}

/// Exports the single OexPlugin_Init symbol with immutable static metadata.
#[macro_export]
macro_rules! export_plugin {
    (name: $name:expr, version: $version:expr, functions: [$($function:expr),* $(,)?], classes: [$($class:expr),* $(,)?] $(,)?) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn OexPlugin_Init() -> *const $crate::sys::OexPluginDefinition {
            static FUNCTIONS: &[$crate::Function] = &[$($function),*];
            static CLASSES: &[$crate::ClassDefinition] = &[$($class),*];
            static PLUGIN: $crate::Plugin = $crate::Plugin::new($name, $version, FUNCTIONS, CLASSES);
            PLUGIN.as_raw()
        }
    };
}

/// Adapts `fn(&mut Call<'_>, &mut T) -> Result<()>`. Reentrant access to the
/// same instance returns an error instead of aliasing &mut T or deadlocking.
#[macro_export]
macro_rules! method {
    ($class:path, $name:expr, $inputs:expr, $outputs:expr, $handler:path) => {{
        struct Adapter;
        impl Adapter {
            unsafe extern "C" fn invoke(
                raw: *mut $crate::sys::OexCall,
                instance: *mut ::std::ffi::c_void,
            ) -> u32 {
                // SAFETY: typed class metadata installs this adapter for the same T.
                unsafe { $crate::__private::method_callback(raw, instance, &$class, $handler) }
            }
        }
        // SAFETY: generated adapter uses this class's T and contains panics.
        unsafe { $class.method_definition($name, $inputs, $outputs, Adapter::invoke) }
    }};
}

/// Builds typed native class metadata. The constructor is an ordinary Rust
/// `fn(&mut Call<'_>) -> Result<T>`; allocations/destruction stay in the plugin.
#[macro_export]
macro_rules! class {
    ($class:path, $name:expr, $constructor:path, methods: $methods:expr, properties: $properties:expr $(,)?) => {{
        struct Adapter;
        impl Adapter {
            unsafe extern "C" fn construct(
                raw: *mut $crate::sys::OexCall,
                instance: *mut *mut ::std::ffi::c_void,
            ) -> u32 {
                // SAFETY: host provides writable output; constructor's T is type checked.
                unsafe {
                    $crate::__private::constructor_callback(raw, instance, &$class, $constructor)
                }
            }
        }
        // SAFETY: constructor, methods and properties all use this class's T.
        unsafe { $class.definition($name, Adapter::construct, $methods, $properties) }
    }};
}

use std::error::Error;
use std::fmt;
use std::mem::{align_of, size_of};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::slice;
use std::str;
use std::sync::Arc;

use openmat_runtime::{
    BuiltinError, BuiltinErrorCategory, BuiltinFunction, BuiltinRegistrationError, BuiltinRegistry,
    BuiltinResult, NativeClass, NativeClassToken, NativeInstance, NativeMethod, NativeProperty,
};

use crate::abi::{
    OEX_ABI_MAJOR, OEX_ABI_MINOR, OEX_ABI_VERSION, OEX_CLASS_HANDLE, OEX_ERROR_PLUGIN,
    OexFunctionDefinition, OexNativeClassDefinition, OexNativeConstructor, OexNativeDestructor,
    OexNativeMethodDefinition, OexNativePropertyDefinition, OexNativePropertyGetter,
    OexNativePropertySetter, OexPluginDefinition, OexPluginInit, OexStatus, OexUtf8View, abi_size,
};
use crate::host::{
    FunctionRegistration, MethodRegistration, invoke, invoke_native_constructor,
    invoke_native_method, invoke_native_property_get, invoke_native_property_set, invoke_owned,
};

#[cfg(windows)]
use libloading::os::windows::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library as NativeLibrary,
};

/// Failure while loading or registering a native OEX library.
#[derive(Debug)]
pub enum OexLoadError {
    Io { path: PathBuf, message: String },
    UnsupportedPlatform,
    Library { path: PathBuf, message: String },
    MissingEntrypoint { path: PathBuf },
    Initialization { path: PathBuf, status: OexStatus },
    InvalidRegistration { path: PathBuf, message: String },
    Builtin(BuiltinRegistrationError),
}

impl fmt::Display for OexLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(
                formatter,
                "failed to resolve OEX library `{}`: {message}",
                path.display()
            ),
            Self::UnsupportedPlatform => {
                formatter.write_str("native OEX loading is currently supported only on Windows")
            }
            Self::Library { path, message } => write!(
                formatter,
                "failed to load OEX library `{}`: {message}",
                path.display()
            ),
            Self::MissingEntrypoint { path } => write!(
                formatter,
                "OEX library `{}` does not export `OexPlugin_Init`",
                path.display()
            ),
            Self::Initialization { path, status } => write!(
                formatter,
                "OEX library `{}` initialization failed with status {status}",
                path.display()
            ),
            Self::InvalidRegistration { path, message } => write!(
                formatter,
                "OEX library `{}` has an invalid static descriptor: {message}",
                path.display()
            ),
            Self::Builtin(error) => write!(
                formatter,
                "failed to add an OEX function to the runtime registry: {error}"
            ),
        }
    }
}

impl Error for OexLoadError {}

impl From<BuiltinRegistrationError> for OexLoadError {
    fn from(value: BuiltinRegistrationError) -> Self {
        Self::Builtin(value)
    }
}

struct LoadedLibrary {
    path: PathBuf,
    #[cfg(windows)]
    _library: NativeLibrary,
}

impl fmt::Debug for LoadedLibrary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoadedLibrary")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// One pinned OEX module and the functions copied from its static descriptor.
#[derive(Clone)]
pub struct OexPlugin {
    library: Arc<LoadedLibrary>,
    name: Arc<str>,
    version: Arc<str>,
    functions: Arc<[FunctionRegistration]>,
    classes: Arc<[NativeClassRegistration]>,
}

#[derive(Clone)]
struct NativeClassRegistration {
    name: String,
    token: NativeClassToken,
    constructor: OexNativeConstructor,
    destructor: OexNativeDestructor,
    methods: Arc<[MethodRegistration]>,
    properties: Arc<[PropertyRegistration]>,
}

#[derive(Clone)]
struct PropertyRegistration {
    name: String,
    getter: Option<OexNativePropertyGetter>,
    setter: Option<OexNativePropertySetter>,
}

impl fmt::Debug for OexPlugin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OexPlugin")
            .field("path", &self.library.path)
            .field("name", &self.name)
            .field("version", &self.version)
            .field(
                "functions",
                &self
                    .functions
                    .iter()
                    .map(|function| &function.name)
                    .collect::<Vec<_>>(),
            )
            .field(
                "classes",
                &self
                    .classes
                    .iter()
                    .map(|class| &class.name)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl OexPlugin {
    /// Loads a trusted OEX DLL and validates its complete static descriptor.
    ///
    /// # Safety
    /// Loading a native library runs arbitrary in-process code. The library must obey the OEX
    /// C ABI, ownership rules, and call-scoped pointer lifetimes.
    ///
    /// # Errors
    ///
    /// Returns a path, loader, entrypoint, ABI, or static-descriptor validation error.
    pub unsafe fn load(path: impl AsRef<Path>) -> Result<Self, OexLoadError> {
        let supplied = path.as_ref();
        let path = supplied.canonicalize().map_err(|error| OexLoadError::Io {
            path: supplied.to_path_buf(),
            message: error.to_string(),
        })?;
        Self::load_canonical(path)
    }

    #[cfg(windows)]
    fn load_canonical(path: PathBuf) -> Result<Self, OexLoadError> {
        // SAFETY: The public load contract requires a trusted native library. Restricted
        // search flags keep dependency resolution at the plugin directory and System32.
        let library = unsafe {
            NativeLibrary::load_with_flags(
                &path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .map_err(|error| OexLoadError::Library {
            path: path.clone(),
            message: error.to_string(),
        })?;
        // SAFETY: Only the exact stable entrypoint name is accepted.
        let init = unsafe { library.get::<OexPluginInit>(b"OexPlugin_Init\0") }
            .map_err(|_| OexLoadError::MissingEntrypoint { path: path.clone() })?;
        // SAFETY: The loaded library promises that the returned static descriptor remains
        // valid while the module is loaded. The module is pinned in OexPlugin.
        let definition = unsafe { init() };
        if definition.is_null() {
            return Err(OexLoadError::Initialization {
                path,
                status: OEX_ERROR_PLUGIN,
            });
        }
        let descriptor = unsafe { &*definition };
        let (name, version, functions, classes) =
            unsafe { copy_and_validate_descriptor(&path, descriptor) }?;
        Ok(Self {
            library: Arc::new(LoadedLibrary {
                path,
                _library: library,
            }),
            name: Arc::from(name),
            version: Arc::from(version),
            functions: Arc::from(functions),
            classes: Arc::from(classes),
        })
    }

    #[cfg(not(windows))]
    fn load_canonical(_path: PathBuf) -> Result<Self, OexLoadError> {
        Err(OexLoadError::UnsupportedPlatform)
    }

    /// Registers every validated function as a runtime built-in.
    ///
    /// # Errors
    ///
    /// Returns an error if a name is already registered or the registry cannot allocate a
    /// stable built-in handle.
    pub fn register_into(&self, registry: &mut BuiltinRegistry) -> Result<(), OexLoadError> {
        for function in self.functions.iter() {
            if registry.contains_name(&function.name) {
                return Err(OexLoadError::Builtin(
                    BuiltinRegistrationError::DuplicateName(function.name.clone()),
                ));
            }
        }
        for class in self.classes.iter() {
            if registry.contains_name(&class.name) {
                return Err(OexLoadError::Builtin(
                    BuiltinRegistrationError::DuplicateName(class.name.clone()),
                ));
            }
            if registry.contains_native_class_token(class.token) {
                return Err(OexLoadError::Builtin(
                    BuiltinRegistrationError::InvalidNativeClass(format!(
                        "class `{}` repeats a native type token",
                        class.name
                    )),
                ));
            }
        }
        for function in self.functions.iter() {
            let builtin: Arc<dyn BuiltinFunction> = Arc::new(OexBuiltin {
                _library: Arc::clone(&self.library),
                registration: function.clone(),
            });
            registry.register_arc(function.name.clone(), builtin)?;
        }
        for registration in self.classes.iter() {
            let class: Arc<dyn NativeClass> = Arc::new(OexNativeClass {
                _library: Arc::clone(&self.library),
                registration: registration.clone(),
                methods: registration
                    .methods
                    .iter()
                    .map(|method| NativeMethod::new(method.name.clone()))
                    .collect(),
                properties: registration
                    .properties
                    .iter()
                    .map(|property| {
                        NativeProperty::new(
                            property.name.clone(),
                            property.getter.is_some(),
                            property.setter.is_some(),
                        )
                    })
                    .collect(),
            });
            registry.register_native_class(class)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn function_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.functions.iter().map(|function| function.name.as_str())
    }

    #[must_use]
    pub fn class_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.classes.iter().map(|class| class.name.as_str())
    }
}

struct OexNativeClass {
    _library: Arc<LoadedLibrary>,
    registration: NativeClassRegistration,
    methods: Vec<NativeMethod>,
    properties: Vec<NativeProperty>,
}

impl NativeClass for OexNativeClass {
    fn token(&self) -> NativeClassToken {
        self.registration.token
    }

    fn name(&self) -> &str {
        &self.registration.name
    }

    fn methods(&self) -> &[NativeMethod] {
        &self.methods
    }

    fn properties(&self) -> &[NativeProperty] {
        &self.properties
    }

    fn construct(
        &self,
        arguments: Vec<openmat_value::Value>,
        context: &mut openmat_runtime::BuiltinContext<'_>,
    ) -> Result<NativeInstance, BuiltinError> {
        invoke_native_constructor(
            &self.registration.name,
            self.registration.constructor,
            self.registration.destructor,
            arguments,
            context,
        )
    }

    fn invoke(
        &self,
        method: &str,
        instance: NativeInstance,
        arguments: Vec<openmat_value::Value>,
        context: &mut openmat_runtime::BuiltinContext<'_>,
    ) -> BuiltinResult {
        let registration = self
            .registration
            .methods
            .iter()
            .find(|registration| registration.name == method)
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Other,
                    format!(
                        "OEX class `{}` has no method `{method}`",
                        self.registration.name
                    ),
                )
            })?;
        invoke_native_method(
            &self.registration.name,
            registration,
            instance,
            arguments,
            context,
        )
    }

    fn get_property(
        &self,
        property: &str,
        instance: NativeInstance,
        context: &mut openmat_runtime::BuiltinContext<'_>,
    ) -> Result<openmat_value::Value, BuiltinError> {
        let registration = self
            .registration
            .properties
            .iter()
            .find(|registration| registration.name == property)
            .ok_or_else(|| missing_native_property(&self.registration.name, property))?;
        let getter = registration
            .getter
            .ok_or_else(|| missing_native_property(&self.registration.name, property))?;
        invoke_native_property_get(&self.registration.name, property, getter, instance, context)
    }

    fn set_property(
        &self,
        property: &str,
        instance: NativeInstance,
        value: openmat_value::Value,
        context: &mut openmat_runtime::BuiltinContext<'_>,
    ) -> Result<(), BuiltinError> {
        let registration = self
            .registration
            .properties
            .iter()
            .find(|registration| registration.name == property)
            .ok_or_else(|| missing_native_property(&self.registration.name, property))?;
        let setter = registration
            .setter
            .ok_or_else(|| missing_native_property(&self.registration.name, property))?;
        invoke_native_property_set(
            &self.registration.name,
            property,
            setter,
            instance,
            value,
            context,
        )
    }

    fn destroy(&self, instance: NativeInstance) {
        let instance = std::ptr::without_provenance_mut(instance.identifier().get());
        // SAFETY: The instance originated from this class's constructor and the runtime calls
        // destroy exactly once while the defining library remains pinned by this object.
        unsafe { (self.registration.destructor)(instance) };
    }
}

fn missing_native_property(class_name: &str, property: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Other,
        format!("OEX class `{class_name}` has no matching property callback for `{property}`"),
    )
}

struct OexBuiltin {
    _library: Arc<LoadedLibrary>,
    registration: FunctionRegistration,
}

impl BuiltinFunction for OexBuiltin {
    fn call(
        &self,
        arguments: &[openmat_value::Value],
        context: &mut openmat_runtime::BuiltinContext<'_>,
    ) -> openmat_runtime::BuiltinResult {
        invoke(&self.registration, arguments, context)
    }

    fn call_owned(
        &self,
        arguments: Vec<openmat_value::Value>,
        context: &mut openmat_runtime::BuiltinContext<'_>,
    ) -> openmat_runtime::BuiltinResult {
        invoke_owned(&self.registration, arguments, context)
    }
}

#[allow(clippy::too_many_lines)]
unsafe fn copy_and_validate_descriptor(
    path: &Path,
    descriptor: &OexPluginDefinition,
) -> Result<
    (
        String,
        String,
        Vec<FunctionRegistration>,
        Vec<NativeClassRegistration>,
    ),
    OexLoadError,
> {
    let invalid = |message: String| OexLoadError::InvalidRegistration {
        path: path.to_path_buf(),
        message,
    };
    if descriptor.struct_size < abi_size::<OexPluginDefinition>() {
        return Err(invalid(format!(
            "plugin descriptor is {} bytes; at least {} are required",
            descriptor.struct_size,
            size_of::<OexPluginDefinition>()
        )));
    }
    let required_major = descriptor.required_abi >> 16;
    let required_minor = descriptor.required_abi & 0xffff;
    if required_major != OEX_ABI_MAJOR || required_minor > OEX_ABI_MINOR {
        return Err(invalid(format!(
            "requires OEX ABI {required_major}.{required_minor}, host provides {OEX_ABI_MAJOR}.{OEX_ABI_MINOR}"
        )));
    }
    if descriptor.reserved != 0 {
        return Err(invalid(
            "plugin descriptor reserved field must be zero".to_owned(),
        ));
    }
    if descriptor.flags != 0 {
        return Err(invalid("plugin descriptor flags must be zero".to_owned()));
    }
    let name = unsafe { copy_utf8(descriptor.name) }
        .map_err(|message| invalid(format!("plugin name {message}")))?;
    let version = unsafe { copy_utf8(descriptor.version) }
        .map_err(|message| invalid(format!("plugin version {message}")))?;
    if name.is_empty() {
        return Err(invalid("plugin name is empty".to_owned()));
    }
    if version.is_empty() {
        return Err(invalid("plugin version is empty".to_owned()));
    }
    if descriptor.function_count == 0 && descriptor.class_count == 0 {
        return Err(invalid(
            "plugin does not describe any functions or native classes".to_owned(),
        ));
    }
    if descriptor.function_count != 0 && descriptor.functions.is_null() {
        return Err(invalid("function descriptor pointer is null".to_owned()));
    }
    let count = usize::try_from(descriptor.function_count)
        .map_err(|_| invalid("function count does not fit the host".to_owned()))?;
    let mut functions = Vec::new();
    functions
        .try_reserve_exact(count)
        .map_err(|_| invalid("could not reserve function descriptors".to_owned()))?;
    let mut definition_pointer = descriptor.functions;
    for _ in 0..count {
        // SAFETY: The trusted plugin promises function_count readable descriptors. Reading
        // structSize first lets an older host walk descriptors that append fields.
        let definition = unsafe { &*definition_pointer };
        let definition_size = definition.struct_size;
        if definition_size < abi_size::<OexFunctionDefinition>() {
            return Err(invalid(format!(
                "function descriptor is {definition_size} bytes; at least {} are required",
                size_of::<OexFunctionDefinition>()
            )));
        }
        let stride = usize::try_from(definition_size)
            .map_err(|_| invalid("function descriptor size does not fit the host".to_owned()))?;
        if stride % align_of::<OexFunctionDefinition>() != 0 {
            return Err(invalid(
                "function descriptor size does not preserve pointer alignment".to_owned(),
            ));
        }
        functions.push(unsafe { copy_function(path, definition, &functions) }?);
        // SAFETY: The plugin contract provides a static descriptor sequence with this stride.
        definition_pointer = unsafe { definition_pointer.byte_add(stride) };
    }
    if descriptor.class_count != 0 && descriptor.classes.is_null() {
        return Err(invalid(
            "native class descriptor pointer is null".to_owned(),
        ));
    }
    let count = usize::try_from(descriptor.class_count)
        .map_err(|_| invalid("native class count does not fit the host".to_owned()))?;
    let mut classes = Vec::new();
    classes
        .try_reserve_exact(count)
        .map_err(|_| invalid("could not reserve native class descriptors".to_owned()))?;
    let mut definition_pointer = descriptor.classes;
    for _ in 0..count {
        // SAFETY: The trusted plugin promises class_count readable descriptors.
        let definition = unsafe { &*definition_pointer };
        let definition_size = definition.struct_size;
        if definition_size < abi_size::<OexNativeClassDefinition>() {
            return Err(invalid(format!(
                "native class descriptor is {definition_size} bytes; at least {} are required",
                size_of::<OexNativeClassDefinition>()
            )));
        }
        let stride = usize::try_from(definition_size).map_err(|_| {
            invalid("native class descriptor size does not fit the host".to_owned())
        })?;
        if stride % align_of::<OexNativeClassDefinition>() != 0 {
            return Err(invalid(
                "native class descriptor size does not preserve pointer alignment".to_owned(),
            ));
        }
        classes.push(unsafe { copy_native_class(path, definition, &classes, &functions) }?);
        // SAFETY: The plugin contract provides a static descriptor sequence with this stride.
        definition_pointer = unsafe { definition_pointer.byte_add(stride) };
    }
    Ok((name, version, functions, classes))
}

unsafe fn copy_native_class(
    path: &Path,
    definition: &OexNativeClassDefinition,
    previous: &[NativeClassRegistration],
    functions: &[FunctionRegistration],
) -> Result<NativeClassRegistration, OexLoadError> {
    let invalid = |message: String| OexLoadError::InvalidRegistration {
        path: path.to_path_buf(),
        message,
    };
    if definition.semantics != OEX_CLASS_HANDLE {
        return Err(invalid(
            "native value classes require a clone callback and are not supported by this ABI"
                .to_owned(),
        ));
    }
    let token = NonZeroUsize::new(definition.type_token.addr())
        .map(NativeClassToken::new)
        .ok_or_else(|| invalid("native class type token is null".to_owned()))?;
    if previous.iter().any(|class| class.token == token) {
        return Err(invalid(
            "native class type token is described more than once".to_owned(),
        ));
    }
    let name = unsafe { copy_utf8(definition.name) }
        .map_err(|message| invalid(format!("native class name {message}")))?;
    if name.is_empty() {
        return Err(invalid("native class name is empty".to_owned()));
    }
    if previous.iter().any(|class| class.name == name)
        || functions.iter().any(|function| function.name == name)
    {
        return Err(invalid(format!(
            "language name `{name}` is described more than once"
        )));
    }
    let Some(constructor) = definition.constructor else {
        return Err(invalid(format!(
            "native class `{name}` has a null constructor"
        )));
    };
    let Some(destructor) = definition.destructor else {
        return Err(invalid(format!(
            "native class `{name}` has a null destructor"
        )));
    };
    if definition.method_count != 0 && definition.methods.is_null() {
        return Err(invalid(format!(
            "native class `{name}` has a null method descriptor pointer"
        )));
    }
    let count = usize::try_from(definition.method_count).map_err(|_| {
        invalid(format!(
            "native class `{name}` method count does not fit the host"
        ))
    })?;
    let mut methods = Vec::new();
    methods.try_reserve_exact(count).map_err(|_| {
        invalid(format!(
            "could not reserve methods for native class `{name}`"
        ))
    })?;
    let mut method_pointer = definition.methods;
    for _ in 0..count {
        // SAFETY: The trusted plugin promises method_count readable descriptors.
        let method = unsafe { &*method_pointer };
        let method_size = method.struct_size;
        if method_size < abi_size::<OexNativeMethodDefinition>() {
            return Err(invalid(format!(
                "native method descriptor is {method_size} bytes; at least {} are required",
                size_of::<OexNativeMethodDefinition>()
            )));
        }
        let stride = usize::try_from(method_size).map_err(|_| {
            invalid("native method descriptor size does not fit the host".to_owned())
        })?;
        if stride % align_of::<OexNativeMethodDefinition>() != 0 {
            return Err(invalid(
                "native method descriptor size does not preserve pointer alignment".to_owned(),
            ));
        }
        methods.push(unsafe { copy_native_method(path, &name, method, &methods) }?);
        // SAFETY: The plugin contract provides a static descriptor sequence with this stride.
        method_pointer = unsafe { method_pointer.byte_add(stride) };
    }
    let properties = unsafe { copy_native_properties(path, &name, definition, &methods) }?;
    Ok(NativeClassRegistration {
        name,
        token,
        constructor,
        destructor,
        methods: Arc::from(methods),
        properties: Arc::from(properties),
    })
}

unsafe fn copy_native_properties(
    path: &Path,
    class_name: &str,
    definition: &OexNativeClassDefinition,
    methods: &[MethodRegistration],
) -> Result<Vec<PropertyRegistration>, OexLoadError> {
    let invalid = |message: String| OexLoadError::InvalidRegistration {
        path: path.to_path_buf(),
        message,
    };
    if definition.property_count != 0 && definition.properties.is_null() {
        return Err(invalid(format!(
            "native class `{class_name}` has a null property descriptor pointer"
        )));
    }
    let count = usize::try_from(definition.property_count).map_err(|_| {
        invalid(format!(
            "native class `{class_name}` property count does not fit the host"
        ))
    })?;
    let mut properties = Vec::new();
    properties.try_reserve_exact(count).map_err(|_| {
        invalid(format!(
            "could not reserve properties for native class `{class_name}`"
        ))
    })?;
    let mut property_pointer = definition.properties;
    for _ in 0..count {
        // SAFETY: The trusted plugin promises property_count readable descriptors.
        let property = unsafe { &*property_pointer };
        let property_size = property.struct_size;
        if property_size < abi_size::<OexNativePropertyDefinition>() {
            return Err(invalid(format!(
                "native property descriptor is {property_size} bytes; at least {} are required",
                size_of::<OexNativePropertyDefinition>()
            )));
        }
        let stride = usize::try_from(property_size).map_err(|_| {
            invalid("native property descriptor size does not fit the host".to_owned())
        })?;
        if stride % align_of::<OexNativePropertyDefinition>() != 0 {
            return Err(invalid(
                "native property descriptor size does not preserve pointer alignment".to_owned(),
            ));
        }
        properties.push(unsafe {
            copy_native_property(path, class_name, property, &properties, methods)
        }?);
        // SAFETY: The plugin contract provides a static descriptor sequence with this stride.
        property_pointer = unsafe { property_pointer.byte_add(stride) };
    }
    Ok(properties)
}

unsafe fn copy_native_property(
    path: &Path,
    class_name: &str,
    definition: &OexNativePropertyDefinition,
    previous: &[PropertyRegistration],
    methods: &[MethodRegistration],
) -> Result<PropertyRegistration, OexLoadError> {
    let invalid = |message: String| OexLoadError::InvalidRegistration {
        path: path.to_path_buf(),
        message,
    };
    if definition.flags != 0 {
        return Err(invalid(format!(
            "native property `{class_name}` flags must be zero"
        )));
    }
    let name = unsafe { copy_utf8(definition.name) }
        .map_err(|message| invalid(format!("native property name {message}")))?;
    if name.is_empty() {
        return Err(invalid(format!(
            "native class `{class_name}` has an empty property name"
        )));
    }
    if name == "delete" {
        return Err(invalid(format!(
            "native class `{class_name}` property `{name}` conflicts with a method"
        )));
    }
    if previous.iter().any(|property| property.name == name) {
        return Err(invalid(format!(
            "native class `{class_name}` describes property `{name}` twice"
        )));
    }
    let getter_name = format!("get.{name}");
    let setter_name = format!("set.{name}");
    if methods.iter().any(|method| {
        method.name == name || method.name == getter_name || method.name == setter_name
    }) {
        return Err(invalid(format!(
            "native class `{class_name}` property `{name}` conflicts with a method"
        )));
    }
    if definition.getter.is_none() && definition.setter.is_none() {
        return Err(invalid(format!(
            "native class `{class_name}` property `{name}` has no getter or setter"
        )));
    }
    Ok(PropertyRegistration {
        name,
        getter: definition.getter,
        setter: definition.setter,
    })
}

unsafe fn copy_native_method(
    path: &Path,
    class_name: &str,
    definition: &OexNativeMethodDefinition,
    previous: &[MethodRegistration],
) -> Result<MethodRegistration, OexLoadError> {
    let invalid = |message: String| OexLoadError::InvalidRegistration {
        path: path.to_path_buf(),
        message,
    };
    if definition.flags != 0 {
        return Err(invalid(format!(
            "native method `{class_name}` flags must be zero"
        )));
    }
    let name = unsafe { copy_utf8(definition.name) }
        .map_err(|message| invalid(format!("native method name {message}")))?;
    if name.is_empty() {
        return Err(invalid(format!(
            "native class `{class_name}` has an empty method name"
        )));
    }
    if name == class_name || name == "delete" {
        return Err(invalid(format!(
            "native class `{class_name}` uses reserved method name `{name}`"
        )));
    }
    if previous.iter().any(|method| method.name == name) {
        return Err(invalid(format!(
            "native class `{class_name}` describes method `{name}` twice"
        )));
    }
    if definition.minimum_inputs > definition.maximum_inputs {
        return Err(invalid(format!(
            "native method `{class_name}.{name}` has an inverted input range"
        )));
    }
    if definition.minimum_outputs > definition.maximum_outputs {
        return Err(invalid(format!(
            "native method `{class_name}.{name}` has an inverted output range"
        )));
    }
    let Some(invoke) = definition.invoke else {
        return Err(invalid(format!(
            "native method `{class_name}.{name}` has a null callback"
        )));
    };
    Ok(MethodRegistration {
        name,
        invoke,
        minimum_inputs: definition.minimum_inputs,
        maximum_inputs: definition.maximum_inputs,
        minimum_outputs: definition.minimum_outputs,
        maximum_outputs: definition.maximum_outputs,
    })
}

unsafe fn copy_function(
    path: &Path,
    definition: &OexFunctionDefinition,
    previous: &[FunctionRegistration],
) -> Result<FunctionRegistration, OexLoadError> {
    let invalid = |message: String| OexLoadError::InvalidRegistration {
        path: path.to_path_buf(),
        message,
    };
    if definition.flags != 0 {
        return Err(invalid("function descriptor flags must be zero".to_owned()));
    }
    if definition.struct_size < abi_size::<OexFunctionDefinition>() {
        return Err(invalid(format!(
            "function descriptor is {} bytes; at least {} are required",
            definition.struct_size,
            size_of::<OexFunctionDefinition>()
        )));
    }
    let name = unsafe { copy_utf8(definition.name) }
        .map_err(|message| invalid(format!("function name {message}")))?;
    if name.is_empty() {
        return Err(invalid("function name is empty".to_owned()));
    }
    if previous.iter().any(|function| function.name == name) {
        return Err(invalid(format!("function `{name}` is described twice")));
    }
    if definition.minimum_inputs > definition.maximum_inputs {
        return Err(invalid(format!(
            "function `{name}` has an inverted input range"
        )));
    }
    if definition.minimum_outputs > definition.maximum_outputs {
        return Err(invalid(format!(
            "function `{name}` has an inverted output range"
        )));
    }
    let Some(invoke) = definition.invoke else {
        return Err(invalid(format!("function `{name}` has a null callback")));
    };
    Ok(FunctionRegistration {
        name,
        invoke,
        minimum_inputs: definition.minimum_inputs,
        maximum_inputs: definition.maximum_inputs,
        minimum_outputs: definition.minimum_outputs,
        maximum_outputs: definition.maximum_outputs,
    })
}

unsafe fn copy_utf8(view: OexUtf8View) -> Result<String, &'static str> {
    let length = usize::try_from(view.length).map_err(|_| "length does not fit the host")?;
    if length == 0 {
        return Ok(String::new());
    }
    if view.data.is_null() {
        return Err("pointer is null");
    }
    // SAFETY: Static descriptor text remains readable while the plugin is loaded.
    let bytes = unsafe { slice::from_raw_parts(view.data.cast::<u8>(), length) };
    str::from_utf8(bytes)
        .map(ToOwned::to_owned)
        .map_err(|_| "is not valid UTF-8")
}

#[allow(dead_code)]
const _: u32 = OEX_ABI_VERSION;

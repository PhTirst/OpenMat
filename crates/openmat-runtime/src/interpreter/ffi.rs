#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::wildcard_imports
)]

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use openmat_ffi::{
    Abi, CArray, CStruct, CType, CUnion, FunctionType, Library, Memory, NativeFunction,
};

use super::*;

const FFI_CLASS_NAME: &str = "OpenMatFFIObject";

#[derive(Clone, Debug)]
enum FfiObject {
    Namespace,
    Type(Arc<CType>),
    Abi(Abi),
    Library(Arc<Library>),
    Function(Arc<NativeFunction>),
    Structure {
        structure: Arc<CStruct>,
        memory: Memory,
    },
    Union {
        union: Arc<CUnion>,
        memory: Memory,
    },
    Array {
        array: Arc<CArray>,
        memory: Memory,
    },
    Pointer {
        pointee: Arc<CType>,
        memory: Memory,
    },
}

impl FfiObject {
    fn class_name(&self) -> &'static str {
        match self {
            Self::Namespace => "ffi.Namespace",
            Self::Type(_) => "ffi.Type",
            Self::Abi(_) => "ffi.Abi",
            Self::Library(_) => "ffi.Library",
            Self::Function(_) => "ffi.Function",
            Self::Structure { .. } => "ffi.StructureValue",
            Self::Union { .. } => "ffi.UnionValue",
            Self::Array { .. } => "ffi.ArrayValue",
            Self::Pointer { .. } => "ffi.Pointer",
        }
    }
}

#[derive(Default)]
pub(super) struct FfiSession {
    class: Option<ClassId>,
    namespace: Option<ObjectHandle>,
    objects: BTreeMap<ObjectHandle, FfiObject>,
    symbols: BTreeMap<(ObjectHandle, String), Arc<NativeFunction>>,
}

impl FfiSession {
    fn resource(&self, handle: ObjectHandle) -> Option<FfiObject> {
        self.objects.get(&handle).cloned()
    }

    fn retain_live(
        &mut self,
        store: &ObjectStore<Value>,
        references: &BTreeMap<ObjectHandle, ObjectRef>,
    ) {
        self.objects.retain(|handle, _| {
            references.get(handle).is_some_and(|reference| {
                store
                    .handle_state(reference)
                    .is_ok_and(|state| state == HandleState::Alive)
            })
        });
        self.symbols
            .retain(|(library, _), _| self.objects.contains_key(library));
        if self
            .namespace
            .is_some_and(|handle| !self.objects.contains_key(&handle))
        {
            self.namespace = None;
        }
    }
}

impl Interpreter {
    pub(super) fn ffi_value_class_name(&self, value: &Value) -> Option<&'static str> {
        let Value::Object(handle) = value else {
            return None;
        };
        self.ffi.objects.get(handle).map(FfiObject::class_name)
    }

    pub(super) fn retain_live_ffi_objects(&mut self) {
        self.ffi.retain_live(&self.objects, &self.object_references);
    }

    pub(super) fn ffi_namespace(
        &mut self,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        if let Some(handle) = self.ffi.namespace
            && self.ffi_live_resource(handle).is_some()
        {
            return Ok(Value::Object(handle));
        }
        let value = self.allocate_ffi_object(FfiObject::Namespace, location)?;
        let Value::Object(handle) = value else {
            return Err(self.invalid_state("FFI namespace allocation returned a non-object"));
        };
        self.ffi.namespace = Some(handle);
        Ok(value)
    }

    pub(super) fn ffi_get_field(
        &mut self,
        object: &Value,
        name: &str,
        location: Option<SourceLocation>,
    ) -> Option<RuntimeResult<Value>> {
        let Value::Object(handle) = object else {
            return None;
        };
        let resource = self.ffi.resource(*handle)?;
        if self.ffi_live_resource(*handle).is_none() {
            return Some(Err(
                self.ffi_error("FFI object is no longer valid", location)
            ));
        }
        Some(self.ffi_get_resource_field(*handle, resource, name, location))
    }

    pub(super) fn ffi_set_field(
        &mut self,
        object: &Value,
        name: &str,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> Option<RuntimeResult<Value>> {
        let Value::Object(handle) = object else {
            return None;
        };
        let resource = self.ffi.resource(*handle)?;
        if self.ffi_live_resource(*handle).is_none() {
            return Some(Err(
                self.ffi_error("FFI object is no longer valid", location)
            ));
        }
        Some(self.ffi_set_resource_field(*handle, resource, name, value, location))
    }

    pub(super) fn ffi_apply_value(
        &mut self,
        object: &Value,
        arguments: &[IndexInput],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> Option<RuntimeResult<Vec<Value>>> {
        let Value::Object(handle) = object else {
            return None;
        };
        let resource = self.ffi.resource(*handle)?;
        if self.ffi_live_resource(*handle).is_none() {
            return Some(Err(
                self.ffi_error("FFI object is no longer valid", location)
            ));
        }
        let values = match self.ffi_call_arguments(arguments, location) {
            Ok(values) => values,
            Err(error) => return Some(Err(error)),
        };
        Some(match resource {
            FfiObject::Function(function) => {
                self.call_ffi_function(&function, &values, requested_outputs, location)
            }
            FfiObject::Type(type_) => {
                self.call_ffi_type(&type_, &values, requested_outputs, location)
            }
            other => {
                Err(self.ffi_error(format!("{} is not callable", other.class_name()), location))
            }
        })
    }

    pub(super) fn ffi_apply_field(
        &mut self,
        object: &Value,
        name: &str,
        arguments: &[IndexInput],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> Option<RuntimeResult<Vec<Value>>> {
        let Value::Object(handle) = object else {
            return None;
        };
        let resource = self.ffi.resource(*handle)?;
        if self.ffi_live_resource(*handle).is_none() {
            return Some(Err(
                self.ffi_error("FFI object is no longer valid", location)
            ));
        }
        let values = match self.ffi_call_arguments(arguments, location) {
            Ok(values) => values,
            Err(error) => return Some(Err(error)),
        };
        Some(match resource {
            FfiObject::Namespace => {
                self.call_ffi_namespace_field(name, &values, requested_outputs, location)
            }
            FfiObject::Pointer { pointee, memory } if name == "offset" => self
                .call_ffi_pointer_offset(&pointee, &memory, &values, requested_outputs, location),
            FfiObject::Array { array, memory } if name == "at" => {
                self.call_ffi_array_at(&array, &memory, &values, requested_outputs, location)
            }
            FfiObject::Array { array, memory } if name == "set" => self.call_ffi_array_set(
                *handle,
                &array,
                &memory,
                &values,
                requested_outputs,
                location,
            ),
            _ => self
                .ffi_get_resource_field(*handle, resource, name, location)
                .and_then(|property| {
                    self.ffi_apply_value(&property, arguments, requested_outputs, location)
                        .unwrap_or_else(|| {
                            Err(self.ffi_error(format!("field `{name}` is not callable"), location))
                        })
                }),
        })
    }

    fn ffi_live_resource(&self, handle: ObjectHandle) -> Option<&FfiObject> {
        let resource = self.ffi.objects.get(&handle)?;
        let reference = self.object_references.get(&handle)?;
        self.objects
            .handle_state(reference)
            .is_ok_and(|state| state == HandleState::Alive)
            .then_some(resource)
    }

    fn ensure_ffi_class(&mut self, location: Option<SourceLocation>) -> RuntimeResult<ClassId> {
        if let Some(class) = self.ffi.class {
            return Ok(class);
        }
        let class = self
            .classes
            .register_class(ClassDefinition::handle(FFI_CLASS_NAME))
            .map_err(|error| {
                self.ffi_error(
                    format!("cannot register FFI object class: {error}"),
                    location,
                )
            })?;
        self.class_handles
            .insert(ClassHandle::new(class.get()), class);
        self.class_code.entry(class).or_default();
        self.ffi.class = Some(class);
        Ok(class)
    }

    fn allocate_ffi_object(
        &mut self,
        resource: FfiObject,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let class = self.ensure_ffi_class(location)?;
        let reference = self
            .objects
            .allocate_with(&self.classes, class, |_| Ok(Value::Nothing))
            .map_err(|error| {
                self.ffi_error(format!("cannot allocate FFI object: {error}"), location)
            })?;
        self.objects
            .begin_construction(&reference)
            .and_then(|()| self.objects.finish_construction(&reference))
            .map_err(|error| {
                self.ffi_error(format!("cannot initialize FFI object: {error}"), location)
            })?;
        let handle = ObjectHandle::new(reference.id().get());
        self.object_references.insert(handle, reference);
        self.ffi.objects.insert(handle, resource);
        Ok(Value::Object(handle))
    }

    fn ffi_get_resource_field(
        &mut self,
        owner: ObjectHandle,
        resource: FfiObject,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        match resource {
            FfiObject::Namespace => self.ffi_namespace_property(name, location),
            FfiObject::Type(type_) => match name {
                "name" => Ok(Value::from(type_.display_name())),
                "size" => self.uint64_scalar(type_.size() as u64, "FFI type size", location),
                "alignment" => {
                    self.uint64_scalar(type_.alignment() as u64, "FFI type alignment", location)
                }
                "length" => match &*type_ {
                    CType::Array(array) => {
                        self.uint64_scalar(array.length as u64, "FFI array length", location)
                    }
                    _ => Err(self.ffi_error("only ffi.Array types have a length", location)),
                },
                "elementType" => match &*type_ {
                    CType::Array(array) => self
                        .allocate_ffi_object(FfiObject::Type(Arc::clone(&array.element)), location),
                    _ => Err(self.ffi_error("only ffi.Array types have an elementType", location)),
                },
                "pack" => match &*type_ {
                    CType::Structure(structure) => self.uint64_scalar(
                        structure.pack.unwrap_or(0) as u64,
                        "FFI structure packing",
                        location,
                    ),
                    CType::Union(union) => self.uint64_scalar(
                        union.pack.unwrap_or(0) as u64,
                        "FFI union packing",
                        location,
                    ),
                    _ => Err(self.ffi_error(
                        "only ffi.Structure and ffi.Union types have packing",
                        location,
                    )),
                },
                _ => Err(self.ffi_error(format!("ffi.Type has no field `{name}`"), location)),
            },
            FfiObject::Abi(abi) => match name {
                "name" => Ok(Value::from(match abi {
                    Abi::Win64 => "win64",
                    Abi::Cdecl => "cdecl",
                    Abi::Stdcall => "stdcall",
                })),
                _ => Err(self.ffi_error(format!("ffi.Abi has no field `{name}`"), location)),
            },
            FfiObject::Library(library) => {
                if name == "path" {
                    return Ok(Value::from(library.path().to_string_lossy().into_owned()));
                }
                self.ffi_library_symbol(owner, library, name, location)
            }
            FfiObject::Function(function) => {
                let signature = function.signature();
                match name {
                    "address" => self.uint64_scalar(
                        function.address() as u64,
                        "FFI function address",
                        location,
                    ),
                    "argtypes" => self.ffi_type_cell(&signature.arguments, location),
                    "restype" => {
                        self.allocate_ffi_object(FfiObject::Type(signature.result), location)
                    }
                    "abi" => self.allocate_ffi_object(FfiObject::Abi(signature.abi), location),
                    "use_last_error" => Ok(Value::Logical(function.use_last_error())),
                    "last_error" => self.ffi_integer_scalar(function.last_error(), location),
                    "library" => Ok(function.library_path().map_or(Value::Nothing, |path| {
                        Value::from(path.to_string_lossy().into_owned())
                    })),
                    _ => {
                        Err(self.ffi_error(format!("ffi.Function has no field `{name}`"), location))
                    }
                }
            }
            FfiObject::Structure { structure, memory } => {
                if name == "address" {
                    return self.uint64_scalar(
                        memory.address() as u64,
                        "FFI structure address",
                        location,
                    );
                }
                let field = structure.field(name).ok_or_else(|| {
                    self.ffi_error(
                        format!("structure `{}` has no field `{name}`", structure.name),
                        location,
                    )
                })?;
                // SAFETY: structure field offsets are created by the C layout.
                let view = memory.offset(field.offset, field.type_.size());
                self.ffi_decode(&field.type_, view, location)
            }
            FfiObject::Union { union, memory } => {
                if name == "address" {
                    return self.uint64_scalar(
                        memory.address() as u64,
                        "FFI union address",
                        location,
                    );
                }
                let field = union.field(name).ok_or_else(|| {
                    self.ffi_error(
                        format!("union `{}` has no field `{name}`", union.name),
                        location,
                    )
                })?;
                self.ffi_decode(&field.type_, memory.offset(0, field.type_.size()), location)
            }
            FfiObject::Array { array, memory } => match name {
                "address" => {
                    self.uint64_scalar(memory.address() as u64, "FFI array address", location)
                }
                "length" => self.uint64_scalar(array.length as u64, "FFI array length", location),
                "elementType" => {
                    self.allocate_ffi_object(FfiObject::Type(Arc::clone(&array.element)), location)
                }
                _ => Err(self.ffi_error(format!("ffi.ArrayValue has no field `{name}`"), location)),
            },
            FfiObject::Pointer { pointee, memory } => match name {
                "address" => {
                    self.uint64_scalar(memory.address() as u64, "FFI pointer address", location)
                }
                "contents" => self.ffi_decode(&pointee, memory, location),
                "type" => self.allocate_ffi_object(FfiObject::Type(pointee), location),
                _ => Err(self.ffi_error(format!("ffi.Pointer has no field `{name}`"), location)),
            },
        }
    }

    fn ffi_set_resource_field(
        &mut self,
        owner: ObjectHandle,
        resource: FfiObject,
        name: &str,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        match resource {
            FfiObject::Library(_) => {
                let Value::Object(function_handle) = value else {
                    return Err(self.ffi_error(
                        format!("library symbol `{name}` must remain an ffi.Function"),
                        location,
                    ));
                };
                let Some(FfiObject::Function(function)) = self.ffi_live_resource(*function_handle)
                else {
                    return Err(self.ffi_error(
                        format!("library symbol `{name}` must remain an ffi.Function"),
                        location,
                    ));
                };
                self.ffi
                    .symbols
                    .insert((owner, name.to_owned()), Arc::clone(function));
            }
            FfiObject::Function(function) => match name {
                "argtypes" => function.set_arguments(self.ffi_type_list(value, location)?),
                "restype" => function.set_result(self.ffi_type_value(value, location)?),
                "abi" => function.set_abi(self.ffi_abi_value(value, location)?),
                "use_last_error" => {
                    function.set_use_last_error(self.ffi_numeric_u128(value, location)? != 0);
                }
                _ => {
                    return Err(self.ffi_error(
                        format!("ffi.Function field `{name}` is read-only or unknown"),
                        location,
                    ));
                }
            },
            FfiObject::Structure { structure, memory } => {
                let field = structure.field(name).ok_or_else(|| {
                    self.ffi_error(
                        format!("structure `{}` has no field `{name}`", structure.name),
                        location,
                    )
                })?;
                let encoded = self.ffi_encode(&field.type_, value, location)?;
                let bytes = encoded.to_vec();
                // SAFETY: the field view comes from the declared C layout.
                let target = memory.offset(field.offset, field.type_.size());
                target.write(&bytes[..field.type_.size()]);
            }
            FfiObject::Union { union, memory } => {
                let field = union.field(name).ok_or_else(|| {
                    self.ffi_error(
                        format!("union `{}` has no field `{name}`", union.name),
                        location,
                    )
                })?;
                let encoded = self.ffi_encode(&field.type_, value, location)?;
                let bytes = encoded.to_vec();
                memory.write(&bytes[..field.type_.size()]);
            }
            FfiObject::Pointer { pointee, memory } if name == "contents" => {
                let encoded = self.ffi_encode(&pointee, value, location)?;
                let bytes = encoded.to_vec();
                memory.write(&bytes[..pointee.size()]);
            }
            other => {
                return Err(self.ffi_error(
                    format!(
                        "{} field `{name}` is read-only or unknown",
                        other.class_name()
                    ),
                    location,
                ));
            }
        }
        Ok(Value::Object(owner))
    }

    fn ffi_namespace_property(
        &mut self,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let primitive = match name {
            "void" => Some(CType::Void),
            "i8" => Some(CType::I8),
            "u8" | "bool" => Some(CType::U8),
            "i16" => Some(CType::I16),
            "u16" => Some(CType::U16),
            "i32" | "bool32" => Some(CType::I32),
            "u32" => Some(CType::U32),
            "i64" | "isize" => Some(CType::I64),
            "u64" | "usize" => Some(CType::U64),
            "f32" => Some(CType::F32),
            "f64" => Some(CType::F64),
            _ => None,
        };
        if let Some(type_) = primitive {
            return self.allocate_ffi_object(FfiObject::Type(Arc::new(type_)), location);
        }
        let abi = match name {
            "win64" => Some(Abi::Win64),
            "cdecl" => Some(Abi::Cdecl),
            "stdcall" => Some(Abi::Stdcall),
            _ => None,
        };
        if let Some(abi) = abi {
            return self.allocate_ffi_object(FfiObject::Abi(abi), location);
        }
        Err(self.ffi_error(
            format!("ffi namespace has no value field `{name}`"),
            location,
        ))
    }

    fn call_ffi_namespace_field(
        &mut self,
        name: &str,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if requested_outputs > 1 {
            return Err(self.ffi_error("FFI constructors return at most one value", location));
        }
        let value = match name {
            "Library" => {
                self.ffi_expect_arity(name, arguments, 1, location)?;
                let path = self.function_name_text("ffi.Library", &arguments[0], location)?;
                // SAFETY: loading arbitrary native code is the explicit FFI operation.
                let library = Library::open(PathBuf::from(path))
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                self.allocate_ffi_object(FfiObject::Library(library), location)?
            }
            "Structure" => {
                if !(2..=3).contains(&arguments.len()) {
                    return Err(self.ffi_error(
                        "ffi.Structure expects name, fields, and optional packing",
                        location,
                    ));
                }
                let structure_name =
                    self.function_name_text("ffi.Structure", &arguments[0], location)?;
                let fields = self.ffi_structure_fields(&arguments[1], location)?;
                let pack = arguments
                    .get(2)
                    .map(|value| self.ffi_pack_value(value, location))
                    .transpose()?;
                let structure = CStruct::with_pack(structure_name, fields, pack)
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                self.allocate_ffi_object(
                    FfiObject::Type(Arc::new(CType::Structure(Arc::new(structure)))),
                    location,
                )?
            }
            "Union" => {
                if !(2..=3).contains(&arguments.len()) {
                    return Err(self.ffi_error(
                        "ffi.Union expects name, fields, and optional packing",
                        location,
                    ));
                }
                let union_name = self.function_name_text("ffi.Union", &arguments[0], location)?;
                let fields = self.ffi_structure_fields(&arguments[1], location)?;
                let pack = arguments
                    .get(2)
                    .map(|value| self.ffi_pack_value(value, location))
                    .transpose()?;
                let union = CUnion::with_pack(union_name, fields, pack)
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                self.allocate_ffi_object(
                    FfiObject::Type(Arc::new(CType::Union(Arc::new(union)))),
                    location,
                )?
            }
            "Array" => {
                self.ffi_expect_arity(name, arguments, 2, location)?;
                let element = self.ffi_type_value(&arguments[0], location)?;
                let length = usize::try_from(self.ffi_numeric_u128(&arguments[1], location)?)
                    .map_err(|_| {
                        self.ffi_error("ffi.Array length exceeds host capacity", location)
                    })?;
                let array = CArray::new(element, length)
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                self.allocate_ffi_object(
                    FfiObject::Type(Arc::new(CType::Array(Arc::new(array)))),
                    location,
                )?
            }
            "Ptr" => {
                self.ffi_expect_arity(name, arguments, 1, location)?;
                let pointee = self.ffi_type_value(&arguments[0], location)?;
                self.allocate_ffi_object(
                    FfiObject::Type(Arc::new(CType::Pointer(pointee))),
                    location,
                )?
            }
            "FunctionType" => {
                if !(2..=3).contains(&arguments.len()) {
                    return Err(self.ffi_error(
                        "ffi.FunctionType expects restype, argtypes, and optional abi",
                        location,
                    ));
                }
                let result = self.ffi_type_value(&arguments[0], location)?;
                let argument_types = self.ffi_type_list(&arguments[1], location)?;
                let abi = if arguments.len() == 3 {
                    self.ffi_abi_value(&arguments[2], location)?
                } else {
                    Abi::Win64
                };
                self.allocate_ffi_object(
                    FfiObject::Type(Arc::new(CType::Function(Arc::new(FunctionType {
                        abi,
                        arguments: argument_types,
                        result,
                    })))),
                    location,
                )?
            }
            "cast" => {
                self.ffi_expect_arity(name, arguments, 2, location)?;
                let target = self.ffi_type_value(&arguments[1], location)?;
                let address = self.ffi_address(&arguments[0], location)?;
                self.ffi_cast_address(address, &target, location)?
            }
            "addressof" => {
                self.ffi_expect_arity(name, arguments, 1, location)?;
                let address = self.ffi_address(&arguments[0], location)?;
                self.uint64_scalar(address as u64, "FFI address", location)?
            }
            "sizeof" => {
                self.ffi_expect_arity(name, arguments, 1, location)?;
                let type_ = self.ffi_type_or_value(&arguments[0], location)?;
                self.uint64_scalar(type_.size() as u64, "FFI size", location)?
            }
            "alignof" => {
                self.ffi_expect_arity(name, arguments, 1, location)?;
                let type_ = self.ffi_type_or_value(&arguments[0], location)?;
                self.uint64_scalar(type_.alignment() as u64, "FFI alignment", location)?
            }
            "alloc" => {
                if !(1..=2).contains(&arguments.len()) {
                    return Err(
                        self.ffi_error("ffi.alloc expects a type and optional count", location)
                    );
                }
                let pointee = self.ffi_type_value(&arguments[0], location)?;
                let count = if arguments.len() == 2 {
                    usize::try_from(self.ffi_numeric_u128(&arguments[1], location)?).map_err(
                        |_| self.ffi_error("ffi.alloc count exceeds host capacity", location),
                    )?
                } else {
                    1
                };
                let bytes = pointee
                    .size()
                    .checked_mul(count)
                    .ok_or_else(|| self.ffi_error("ffi.alloc size overflow", location))?;
                let memory = Memory::allocate_bytes(bytes, pointee.alignment())
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                self.allocate_ffi_object(FfiObject::Pointer { pointee, memory }, location)?
            }
            "cstr" => {
                self.ffi_expect_arity(name, arguments, 1, location)?;
                let text = self.function_name_text("ffi.cstr", &arguments[0], location)?;
                let mut bytes = text.into_bytes();
                bytes.push(0);
                self.ffi_owned_buffer_pointer(Arc::new(CType::U8), &bytes, 1, location)?
            }
            "wstr" => {
                self.ffi_expect_arity(name, arguments, 1, location)?;
                let text = self.function_name_text("ffi.wstr", &arguments[0], location)?;
                let mut words = text.encode_utf16().collect::<Vec<_>>();
                words.push(0);
                let mut bytes = Vec::with_capacity(words.len() * 2);
                for word in words {
                    bytes.extend_from_slice(&word.to_ne_bytes());
                }
                self.ffi_owned_buffer_pointer(Arc::new(CType::U16), &bytes, 2, location)?
            }
            "read_cstr" => {
                if !(1..=2).contains(&arguments.len()) {
                    return Err(self.ffi_error(
                        "ffi.read_cstr expects an address and optional maximum length",
                        location,
                    ));
                }
                let address = self.ffi_address(&arguments[0], location)?;
                let maximum = arguments
                    .get(1)
                    .map(|value| self.ffi_host_count(value, "C string length", location))
                    .transpose()?;
                let bytes = Self::ffi_read_terminated_bytes(address, maximum);
                let text = String::from_utf8(bytes).map_err(|error| {
                    self.ffi_error(
                        format!("native C string is not valid UTF-8: {error}"),
                        location,
                    )
                })?;
                Value::from(text)
            }
            "read_wstr" => {
                if !(1..=2).contains(&arguments.len()) {
                    return Err(self.ffi_error(
                        "ffi.read_wstr expects an address and optional maximum length",
                        location,
                    ));
                }
                let address = self.ffi_address(&arguments[0], location)?;
                let maximum = arguments
                    .get(1)
                    .map(|value| self.ffi_host_count(value, "wide string length", location))
                    .transpose()?;
                let words = Self::ffi_read_terminated_words(address, maximum);
                let text = String::from_utf16(&words).map_err(|error| {
                    self.ffi_error(
                        format!("native wide string is not valid UTF-16: {error}"),
                        location,
                    )
                })?;
                Value::from(text)
            }
            _ => {
                return Err(self.ffi_error(
                    format!("ffi namespace has no callable field `{name}`"),
                    location,
                ));
            }
        };
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![value])
        }
    }

    fn call_ffi_type(
        &mut self,
        type_: &Arc<CType>,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if requested_outputs > 1 {
            return Err(self.ffi_error("FFI type constructors return one value", location));
        }
        let value = match &**type_ {
            CType::Structure(structure) => {
                if !arguments.is_empty() {
                    return Err(self.ffi_error(
                        "structure construction currently expects no arguments",
                        location,
                    ));
                }
                let memory = Memory::allocate(type_)
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                self.allocate_ffi_object(
                    FfiObject::Structure {
                        structure: Arc::clone(structure),
                        memory,
                    },
                    location,
                )?
            }
            CType::Union(union) => {
                if !arguments.is_empty() {
                    return Err(self.ffi_error(
                        "union construction currently expects no arguments",
                        location,
                    ));
                }
                let memory = Memory::allocate(type_)
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                self.allocate_ffi_object(
                    FfiObject::Union {
                        union: Arc::clone(union),
                        memory,
                    },
                    location,
                )?
            }
            CType::Array(array) => {
                if !arguments.is_empty() {
                    return Err(self.ffi_error(
                        "array construction currently expects no arguments",
                        location,
                    ));
                }
                let memory = Memory::allocate(type_)
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                self.allocate_ffi_object(
                    FfiObject::Array {
                        array: Arc::clone(array),
                        memory,
                    },
                    location,
                )?
            }
            CType::Pointer(pointee) => {
                if arguments.len() > 1 {
                    return Err(self
                        .ffi_error("pointer construction expects zero or one address", location));
                }
                let address = if let Some(value) = arguments.first() {
                    self.ffi_address(value, location)?
                } else {
                    0
                };
                // SAFETY: raw address construction is the explicit operation.
                let memory = Memory::from_address(address, pointee.size());
                self.allocate_ffi_object(
                    FfiObject::Pointer {
                        pointee: Arc::clone(pointee),
                        memory,
                    },
                    location,
                )?
            }
            CType::Function(signature) => {
                self.ffi_expect_arity("function pointer", arguments, 1, location)?;
                let address = self.ffi_address(&arguments[0], location)?;
                let function = NativeFunction::new(address, None, (**signature).clone());
                self.allocate_ffi_object(FfiObject::Function(function), location)?
            }
            CType::Void => {
                return Err(self.ffi_error("void is not constructible", location));
            }
            _ => {
                self.ffi_expect_arity("primitive C type", arguments, 1, location)?;
                let memory = self.ffi_encode(type_, &arguments[0], location)?;
                self.ffi_decode(type_, memory, location)?
            }
        };
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![value])
        }
    }

    fn call_ffi_function(
        &mut self,
        function: &Arc<NativeFunction>,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        if requested_outputs > 1 {
            return Err(self.ffi_error("native functions return at most one value", location));
        }
        let signature = function.signature();
        if arguments.len() != signature.arguments.len() {
            return Err(self.ffi_error(
                format!(
                    "native function expects {} arguments, received {}",
                    signature.arguments.len(),
                    arguments.len()
                ),
                location,
            ));
        }
        let encoded = signature
            .arguments
            .iter()
            .zip(arguments)
            .map(|(type_, value)| self.ffi_encode(type_, value, location))
            .collect::<RuntimeResult<Vec<_>>>()?;
        // SAFETY: M code supplied the complete native signature and accepted
        // that a mismatch may corrupt memory or terminate the process.
        let returned = function
            .call(&encoded)
            .map_err(|error| self.ffi_error(error.to_string(), location))?;
        let Some(returned) = returned else {
            return if requested_outputs == 0 {
                Ok(Vec::new())
            } else {
                Ok(vec![Value::Nothing])
            };
        };
        let value = self.ffi_decode(&signature.result, returned, location)?;
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![value])
        }
    }

    fn call_ffi_pointer_offset(
        &mut self,
        pointee: &Arc<CType>,
        memory: &Memory,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        self.ffi_expect_arity("pointer.offset", arguments, 1, location)?;
        if requested_outputs > 1 {
            return Err(self.ffi_error("pointer.offset returns one pointer", location));
        }
        let count = self.ffi_numeric_i128(&arguments[0], location)?;
        let byte_offset = count
            .checked_mul(pointee.size() as i128)
            .and_then(|value| isize::try_from(value).ok())
            .ok_or_else(|| self.ffi_error("pointer offset exceeds host capacity", location))?;
        // SAFETY: pointer arithmetic is deliberately unchecked.
        let memory = memory.offset_signed(byte_offset, pointee.size());
        let value = self.allocate_ffi_object(
            FfiObject::Pointer {
                pointee: Arc::clone(pointee),
                memory,
            },
            location,
        )?;
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![value])
        }
    }

    fn call_ffi_array_at(
        &mut self,
        array: &Arc<CArray>,
        memory: &Memory,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        self.ffi_expect_arity("array.at", arguments, 1, location)?;
        if requested_outputs > 1 {
            return Err(self.ffi_error("array.at returns one value", location));
        }
        let byte_offset = self.ffi_element_byte_offset(
            &arguments[0],
            array.element.size(),
            "array index",
            location,
        )?;
        // SAFETY: array indexing is deliberately not bounds checked.
        let element = memory.offset_signed(byte_offset, array.element.size());
        let value = self.ffi_decode(&array.element, element, location)?;
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![value])
        }
    }

    fn call_ffi_array_set(
        &mut self,
        owner: ObjectHandle,
        array: &Arc<CArray>,
        memory: &Memory,
        arguments: &[Value],
        requested_outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        self.ffi_expect_arity("array.set", arguments, 2, location)?;
        if requested_outputs > 1 {
            return Err(self.ffi_error("array.set returns the array", location));
        }
        let byte_offset = self.ffi_element_byte_offset(
            &arguments[0],
            array.element.size(),
            "array index",
            location,
        )?;
        let encoded = self.ffi_encode(&array.element, &arguments[1], location)?;
        let bytes = encoded.to_vec();
        // SAFETY: array indexing is deliberately not bounds checked.
        memory
            .offset_signed(byte_offset, array.element.size())
            .write(&bytes[..array.element.size()]);
        if requested_outputs == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![Value::Object(owner)])
        }
    }

    fn ffi_element_byte_offset(
        &self,
        index: &Value,
        element_size: usize,
        operation: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<isize> {
        self.ffi_numeric_i128(index, location)?
            .checked_mul(element_size as i128)
            .and_then(|value| isize::try_from(value).ok())
            .ok_or_else(|| self.ffi_error(format!("{operation} exceeds host capacity"), location))
    }

    fn ffi_library_symbol(
        &mut self,
        library_handle: ObjectHandle,
        library: Arc<Library>,
        name: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let key = (library_handle, name.to_owned());
        if let Some(function) = self.ffi.symbols.get(&key).cloned() {
            return self.allocate_ffi_object(FfiObject::Function(function), location);
        }
        // SAFETY: the symbol remains untyped until its M properties are set.
        let address = library
            .symbol_address(name)
            .map_err(|error| self.ffi_error(error.to_string(), location))?;
        let function = NativeFunction::new(
            address,
            Some(library),
            FunctionType {
                abi: Abi::Win64,
                arguments: Vec::new(),
                result: Arc::new(CType::Void),
            },
        );
        self.ffi.symbols.insert(key, Arc::clone(&function));
        self.allocate_ffi_object(FfiObject::Function(function), location)
    }

    fn ffi_structure_fields(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<(String, Arc<CType>)>> {
        let Value::Cell(cell) = value else {
            return Err(self.ffi_error(
                "ffi.Structure fields must be an N-by-2 cell array",
                location,
            ));
        };
        let dimensions = cell.shape().dimensions();
        if dimensions.len() != 2 || dimensions[1] != 2 {
            return Err(self.ffi_error(
                "ffi.Structure fields must be an N-by-2 cell array",
                location,
            ));
        }
        let rows = usize::try_from(dimensions[0])
            .map_err(|_| self.ffi_error("structure field count exceeds host capacity", location))?;
        let mut fields = Vec::with_capacity(rows);
        for row in 0..rows {
            let name = self.function_name_text("ffi.Structure", &cell.values()[row], location)?;
            let type_ = self.ffi_type_value(&cell.values()[row + rows], location)?;
            fields.push((name, type_));
        }
        Ok(fields)
    }

    fn ffi_type_list(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Arc<CType>>> {
        let Value::Cell(cell) = value else {
            return Err(
                self.ffi_error("argtypes must be a cell array of ffi.Type values", location)
            );
        };
        cell.values()
            .iter()
            .map(|value| self.ffi_type_value(value, location))
            .collect()
    }

    fn ffi_pack_value(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<usize> {
        let pack = usize::try_from(self.ffi_numeric_u128(value, location)?)
            .map_err(|_| self.ffi_error("FFI packing exceeds host capacity", location))?;
        if pack == 0 || !pack.is_power_of_two() {
            return Err(self.ffi_error("FFI packing must be a positive power of two", location));
        }
        Ok(pack)
    }

    fn ffi_host_count(
        &self,
        value: &Value,
        operation: &str,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<usize> {
        usize::try_from(self.ffi_numeric_u128(value, location)?)
            .map_err(|_| self.ffi_error(format!("{operation} exceeds host capacity"), location))
    }

    fn ffi_owned_buffer_pointer(
        &mut self,
        pointee: Arc<CType>,
        bytes: &[u8],
        alignment: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let memory = Memory::allocate_bytes(bytes.len(), alignment)
            .map_err(|error| self.ffi_error(error.to_string(), location))?;
        memory.write(bytes);
        self.allocate_ffi_object(FfiObject::Pointer { pointee, memory }, location)
    }

    fn ffi_read_terminated_bytes(address: usize, maximum: Option<usize>) -> Vec<u8> {
        let mut bytes = Vec::new();
        while maximum.is_none_or(|maximum| bytes.len() < maximum) {
            // SAFETY: C-string scanning deliberately trusts the supplied address.
            let byte = Memory::from_address(address.wrapping_add(bytes.len()), 1).to_vec()[0];
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }
        bytes
    }

    fn ffi_read_terminated_words(address: usize, maximum: Option<usize>) -> Vec<u16> {
        let mut words = Vec::new();
        while maximum.is_none_or(|maximum| words.len() < maximum) {
            let offset = words.len().wrapping_mul(std::mem::size_of::<u16>());
            // SAFETY: wide-string scanning deliberately trusts the supplied address.
            let bytes =
                Memory::from_address(address.wrapping_add(offset), std::mem::size_of::<u16>())
                    .to_vec();
            let word = u16::from_ne_bytes([bytes[0], bytes[1]]);
            if word == 0 {
                break;
            }
            words.push(word);
        }
        words
    }

    fn ffi_type_value(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Arc<CType>> {
        let Value::Object(handle) = value else {
            return Err(self.ffi_error("expected an ffi.Type value", location));
        };
        match self.ffi_live_resource(*handle) {
            Some(FfiObject::Type(type_)) => Ok(Arc::clone(type_)),
            _ => Err(self.ffi_error("expected an ffi.Type value", location)),
        }
    }

    fn ffi_abi_value(&self, value: &Value, location: Option<SourceLocation>) -> RuntimeResult<Abi> {
        let Value::Object(handle) = value else {
            return Err(self.ffi_error("expected an ffi.Abi value", location));
        };
        match self.ffi_live_resource(*handle) {
            Some(FfiObject::Abi(abi)) => Ok(*abi),
            _ => Err(self.ffi_error("expected an ffi.Abi value", location)),
        }
    }

    fn ffi_type_or_value(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Arc<CType>> {
        let Value::Object(handle) = value else {
            return Err(self.ffi_error("expected an FFI type or native value", location));
        };
        match self.ffi_live_resource(*handle) {
            Some(FfiObject::Type(type_)) => Ok(Arc::clone(type_)),
            Some(FfiObject::Structure { structure, .. }) => {
                Ok(Arc::new(CType::Structure(Arc::clone(structure))))
            }
            Some(FfiObject::Union { union, .. }) => Ok(Arc::new(CType::Union(Arc::clone(union)))),
            Some(FfiObject::Array { array, .. }) => Ok(Arc::new(CType::Array(Arc::clone(array)))),
            Some(FfiObject::Pointer { pointee, .. }) => {
                Ok(Arc::new(CType::Pointer(Arc::clone(pointee))))
            }
            Some(FfiObject::Function(function)) => {
                Ok(Arc::new(CType::Function(Arc::new(function.signature()))))
            }
            _ => Err(self.ffi_error("expected an FFI type or native value", location)),
        }
    }

    fn ffi_type_cell(
        &mut self,
        types: &[Arc<CType>],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let mut values = Vec::with_capacity(types.len());
        for type_ in types {
            values.push(self.allocate_ffi_object(FfiObject::Type(Arc::clone(type_)), location)?);
        }
        let length = u64::try_from(values.len())
            .map_err(|_| self.ffi_error("FFI type list exceeds host capacity", location))?;
        let shape = Shape::new([1, length])
            .map_err(|_| self.ffi_error("FFI type list shape is invalid", location))?;
        CellArray::from_values(shape, values)
            .map(Value::Cell)
            .map_err(|error| self.ffi_error(error.to_string(), location))
    }

    fn ffi_cast_address(
        &mut self,
        address: usize,
        target: &Arc<CType>,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        match &**target {
            CType::Pointer(pointee) => {
                // SAFETY: ffi.cast explicitly creates an unchecked pointer.
                let memory = Memory::from_address(address, pointee.size());
                self.allocate_ffi_object(
                    FfiObject::Pointer {
                        pointee: Arc::clone(pointee),
                        memory,
                    },
                    location,
                )
            }
            CType::Function(signature) => self.allocate_ffi_object(
                FfiObject::Function(NativeFunction::new(address, None, (**signature).clone())),
                location,
            ),
            _ => Err(self.ffi_error(
                "ffi.cast target must be a pointer or function type",
                location,
            )),
        }
    }

    fn ffi_address(&self, value: &Value, location: Option<SourceLocation>) -> RuntimeResult<usize> {
        if let Value::Object(handle) = value {
            return match self.ffi_live_resource(*handle) {
                Some(
                    FfiObject::Structure { memory, .. }
                    | FfiObject::Union { memory, .. }
                    | FfiObject::Array { memory, .. }
                    | FfiObject::Pointer { memory, .. },
                ) => Ok(memory.address()),
                Some(FfiObject::Function(function)) => Ok(function.address()),
                _ => Err(self.ffi_error("value has no native address", location)),
            };
        }
        usize::try_from(self.ffi_numeric_u128(value, location)?)
            .map_err(|_| self.ffi_error("address exceeds host pointer width", location))
    }

    fn ffi_encode(
        &self,
        type_: &Arc<CType>,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Memory> {
        match &**type_ {
            CType::Structure(expected) => {
                let Value::Object(handle) = value else {
                    return Err(self.ffi_error("expected an ffi.StructureValue", location));
                };
                match self.ffi_live_resource(*handle) {
                    Some(FfiObject::Structure { structure, memory }) if structure == expected => {
                        Ok(memory.clone())
                    }
                    _ => Err(self.ffi_error(
                        format!("expected a `{}` structure value", expected.name),
                        location,
                    )),
                }
            }
            CType::Union(expected) => {
                let Value::Object(handle) = value else {
                    return Err(self.ffi_error("expected an ffi.UnionValue", location));
                };
                match self.ffi_live_resource(*handle) {
                    Some(FfiObject::Union { union, memory }) if union == expected => {
                        Ok(memory.clone())
                    }
                    _ => Err(self.ffi_error(
                        format!("expected a `{}` union value", expected.name),
                        location,
                    )),
                }
            }
            CType::Array(expected) => {
                let Value::Object(handle) = value else {
                    return Err(self.ffi_error("expected an ffi.ArrayValue", location));
                };
                match self.ffi_live_resource(*handle) {
                    Some(FfiObject::Array { array, memory }) if array == expected => {
                        Ok(memory.clone())
                    }
                    _ => Err(self.ffi_error("expected a matching C array value", location)),
                }
            }
            CType::Pointer(_) | CType::Function(_) => {
                let address = self.ffi_address(value, location)?;
                let memory = Memory::allocate(type_)
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                memory.write(&address.to_ne_bytes());
                Ok(memory)
            }
            CType::Void => Err(self.ffi_error("void cannot be a function argument", location)),
            primitive => {
                let memory = Memory::allocate(primitive)
                    .map_err(|error| self.ffi_error(error.to_string(), location))?;
                let bytes = self.ffi_primitive_bytes(primitive, value, location)?;
                memory.write(&bytes);
                Ok(memory)
            }
        }
    }

    fn ffi_decode(
        &mut self,
        type_: &Arc<CType>,
        memory: Memory,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        match &**type_ {
            CType::Void => Ok(Value::Nothing),
            CType::Structure(structure) => self.allocate_ffi_object(
                FfiObject::Structure {
                    structure: Arc::clone(structure),
                    memory,
                },
                location,
            ),
            CType::Union(union) => self.allocate_ffi_object(
                FfiObject::Union {
                    union: Arc::clone(union),
                    memory,
                },
                location,
            ),
            CType::Array(array) => self.allocate_ffi_object(
                FfiObject::Array {
                    array: Arc::clone(array),
                    memory,
                },
                location,
            ),
            CType::Pointer(pointee) => {
                let address = Self::ffi_read_usize(&memory);
                // SAFETY: native return/field bits are intentionally trusted.
                let target = Memory::from_address(address, pointee.size());
                self.allocate_ffi_object(
                    FfiObject::Pointer {
                        pointee: Arc::clone(pointee),
                        memory: target,
                    },
                    location,
                )
            }
            CType::Function(signature) => {
                let address = Self::ffi_read_usize(&memory);
                self.allocate_ffi_object(
                    FfiObject::Function(NativeFunction::new(address, None, (**signature).clone())),
                    location,
                )
            }
            primitive => self.ffi_decode_primitive(primitive, &memory, location),
        }
    }

    fn ffi_primitive_bytes(
        &self,
        type_: &CType,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<u8>> {
        let bytes = match type_ {
            CType::I8 => (self.ffi_numeric_i128(value, location)? as i8)
                .to_ne_bytes()
                .to_vec(),
            CType::U8 => (self.ffi_numeric_u128(value, location)? as u8)
                .to_ne_bytes()
                .to_vec(),
            CType::I16 => (self.ffi_numeric_i128(value, location)? as i16)
                .to_ne_bytes()
                .to_vec(),
            CType::U16 => (self.ffi_numeric_u128(value, location)? as u16)
                .to_ne_bytes()
                .to_vec(),
            CType::I32 => (self.ffi_numeric_i128(value, location)? as i32)
                .to_ne_bytes()
                .to_vec(),
            CType::U32 => (self.ffi_numeric_u128(value, location)? as u32)
                .to_ne_bytes()
                .to_vec(),
            CType::I64 => (self.ffi_numeric_i128(value, location)? as i64)
                .to_ne_bytes()
                .to_vec(),
            CType::U64 => (self.ffi_numeric_u128(value, location)? as u64)
                .to_ne_bytes()
                .to_vec(),
            CType::F32 => (self.ffi_numeric_f64(value, location)? as f32)
                .to_ne_bytes()
                .to_vec(),
            CType::F64 => self
                .ffi_numeric_f64(value, location)?
                .to_ne_bytes()
                .to_vec(),
            _ => return Err(self.ffi_error("expected a primitive C type", location)),
        };
        Ok(bytes)
    }

    fn ffi_decode_primitive(
        &self,
        type_: &CType,
        memory: &Memory,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let mut bytes = vec![0_u8; type_.size()];
        memory.read_into(&mut bytes);
        macro_rules! integer {
            ($type:ty) => {{
                let value =
                    <$type>::from_ne_bytes(bytes.as_slice().try_into().map_err(|_| {
                        self.ffi_error("native result has the wrong width", location)
                    })?);
                self.ffi_integer_scalar(value, location)
            }};
        }
        match type_ {
            CType::I8 => integer!(i8),
            CType::U8 => integer!(u8),
            CType::I16 => integer!(i16),
            CType::U16 => integer!(u16),
            CType::I32 => integer!(i32),
            CType::U32 => integer!(u32),
            CType::I64 => integer!(i64),
            CType::U64 => integer!(u64),
            CType::F32 => Ok(Value::Double(f64::from(f32::from_ne_bytes(
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| self.ffi_error("native result has the wrong width", location))?,
            )))),
            CType::F64 => Ok(Value::Double(f64::from_ne_bytes(
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| self.ffi_error("native result has the wrong width", location))?,
            ))),
            _ => Err(self.ffi_error("expected a primitive C type", location)),
        }
    }

    fn ffi_integer_scalar<T: openmat_array::IntegerElement>(
        &self,
        value: T,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Value> {
        let shape = Shape::new([1, 1])
            .map_err(|_| self.ffi_error("integer scalar shape is invalid", location))?;
        DenseArray::from_vec(shape, vec![value])
            .map(ArrayData::from_typed)
            .map(Value::Array)
            .map_err(|error| self.ffi_error(error.to_string(), location))
    }

    fn ffi_read_usize(memory: &Memory) -> usize {
        let mut bytes = [0_u8; std::mem::size_of::<usize>()];
        memory.read_into(&mut bytes);
        usize::from_ne_bytes(bytes)
    }

    fn ffi_numeric_f64(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<f64> {
        match value {
            Value::Double(value) => Ok(*value),
            Value::Logical(value) => Ok(f64::from(u8::from(*value))),
            Value::Array(ArrayData::F64(array)) if array.numel() == 1 => Ok(array.as_slice()[0]),
            Value::Array(ArrayData::Logical(array)) if array.numel() == 1 => {
                Ok(f64::from(u8::from(array.as_slice()[0].get())))
            }
            Value::Array(ArrayData::Integer(integer)) if integer.numel() == 1 => {
                let element = integer
                    .element(0)
                    .ok_or_else(|| self.ffi_error("integer scalar storage is empty", location))?;
                if element.is_complex() {
                    return Err(self.ffi_error("FFI numeric values must be real scalars", location));
                }
                Ok(match element.real_component() {
                    openmat_array::IntegerComponent::Signed(value) => value as f64,
                    openmat_array::IntegerComponent::Unsigned(value) => value as f64,
                })
            }
            _ => Err(self.ffi_error("FFI numeric values must be real scalars", location)),
        }
    }

    fn ffi_numeric_i128(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<i128> {
        if let Value::Array(ArrayData::Integer(integer)) = value
            && integer.numel() == 1
        {
            let element = integer
                .element(0)
                .ok_or_else(|| self.ffi_error("integer scalar storage is empty", location))?;
            if !element.is_complex() {
                return Ok(match element.real_component() {
                    openmat_array::IntegerComponent::Signed(value) => value,
                    openmat_array::IntegerComponent::Unsigned(value) => value as i128,
                });
            }
        }
        Ok(self.ffi_numeric_f64(value, location)? as i128)
    }

    fn ffi_numeric_u128(
        &self,
        value: &Value,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<u128> {
        if let Value::Array(ArrayData::Integer(integer)) = value
            && integer.numel() == 1
        {
            let element = integer
                .element(0)
                .ok_or_else(|| self.ffi_error("integer scalar storage is empty", location))?;
            if !element.is_complex() {
                return Ok(match element.real_component() {
                    openmat_array::IntegerComponent::Signed(value) => value as u128,
                    openmat_array::IntegerComponent::Unsigned(value) => value,
                });
            }
        }
        Ok(self.ffi_numeric_f64(value, location)? as u128)
    }

    fn ffi_call_arguments(
        &self,
        arguments: &[IndexInput],
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        arguments
            .iter()
            .enumerate()
            .map(|(argument, value)| match value {
                IndexInput::Value(value) => Ok(value.clone()),
                IndexInput::Colon => Err(self.error(
                    array_error(ArrayRuntimeError::ColonCallArgument { argument }),
                    location,
                )),
            })
            .collect()
    }

    fn ffi_expect_arity(
        &self,
        name: &str,
        arguments: &[Value],
        expected: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<()> {
        if arguments.len() == expected {
            Ok(())
        } else {
            Err(self.ffi_error(
                format!(
                    "ffi.{name} expects {expected} arguments, received {}",
                    arguments.len()
                ),
                location,
            ))
        }
    }

    pub(super) fn ffi_error(
        &self,
        message: impl Into<String>,
        location: Option<SourceLocation>,
    ) -> RuntimeError {
        self.error(
            RuntimeErrorKind::Object {
                operation: "ffi",
                message: message.into(),
            },
            location,
        )
    }
}

#[cfg(test)]
mod tests {
    use openmat_bytecode::{BytecodeModule, Function, FunctionId, Instruction, InstructionKind};

    use super::*;

    fn interpreter() -> Interpreter {
        let entry = Function {
            name: "<ffi-test>".to_owned(),
            register_count: 0,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: Vec::new(),
            })],
            exception_handlers: Vec::new(),
        };
        Interpreter::new(BytecodeModule::new(vec![entry], FunctionId::new(0))).unwrap()
    }

    fn arguments(values: Vec<Value>) -> Vec<IndexInput> {
        values.into_iter().map(IndexInput::Value).collect()
    }

    fn namespace_call(
        interpreter: &mut Interpreter,
        namespace: &Value,
        name: &str,
        values: Vec<Value>,
    ) -> Value {
        interpreter
            .ffi_apply_field(namespace, name, &arguments(values), 1, None)
            .unwrap()
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }

    fn object_call(
        interpreter: &mut Interpreter,
        object: &Value,
        name: &str,
        values: Vec<Value>,
        outputs: usize,
    ) -> Vec<Value> {
        interpreter
            .ffi_apply_field(object, name, &arguments(values), outputs, None)
            .unwrap()
            .unwrap()
    }

    fn type_value(interpreter: &mut Interpreter, namespace: &Value, name: &str) -> Value {
        interpreter
            .ffi_get_field(namespace, name, None)
            .unwrap()
            .unwrap()
    }

    fn field_table(rows: Vec<(&str, Value)>) -> Value {
        let row_count = rows.len();
        let mut values = Vec::with_capacity(row_count * 2);
        values.extend(rows.iter().map(|(name, _)| Value::from(*name)));
        values.extend(rows.into_iter().map(|(_, type_)| type_));
        Value::Cell(
            CellArray::from_values(Shape::new([row_count as u64, 2]).unwrap(), values).unwrap(),
        )
    }

    #[test]
    fn arrays_unions_and_packed_structures_share_native_memory() {
        let mut interpreter = interpreter();
        let namespace = interpreter.ffi_namespace(None).unwrap();
        let u8_type = type_value(&mut interpreter, &namespace, "u8");
        let u16_type = type_value(&mut interpreter, &namespace, "u16");
        let u32_type = type_value(&mut interpreter, &namespace, "u32");

        let array_type = namespace_call(
            &mut interpreter,
            &namespace,
            "Array",
            vec![u16_type.clone(), Value::Double(3.0)],
        );
        let array = interpreter
            .ffi_apply_value(&array_type, &[], 1, None)
            .unwrap()
            .unwrap()
            .remove(0);
        object_call(
            &mut interpreter,
            &array,
            "set",
            vec![Value::Double(0.0), Value::Double(17.0)],
            0,
        );
        object_call(
            &mut interpreter,
            &array,
            "set",
            vec![Value::Double(1.0), Value::Double(23.0)],
            0,
        );
        let first =
            object_call(&mut interpreter, &array, "at", vec![Value::Double(0.0)], 1).remove(0);
        let second =
            object_call(&mut interpreter, &array, "at", vec![Value::Double(1.0)], 1).remove(0);
        assert_eq!(interpreter.ffi_numeric_u128(&first, None).unwrap(), 17);
        assert_eq!(interpreter.ffi_numeric_u128(&second, None).unwrap(), 23);

        let packet_type = namespace_call(
            &mut interpreter,
            &namespace,
            "Structure",
            vec![
                Value::from("Packet"),
                field_table(vec![("values", array_type.clone())]),
            ],
        );
        let packet = interpreter
            .ffi_apply_value(&packet_type, &[], 1, None)
            .unwrap()
            .unwrap()
            .remove(0);
        let packet_values = interpreter
            .ffi_get_field(&packet, "values", None)
            .unwrap()
            .unwrap();
        object_call(
            &mut interpreter,
            &packet_values,
            "set",
            vec![Value::Double(2.0), Value::Double(31.0)],
            0,
        );
        let packet_values = interpreter
            .ffi_get_field(&packet, "values", None)
            .unwrap()
            .unwrap();
        let third = object_call(
            &mut interpreter,
            &packet_values,
            "at",
            vec![Value::Double(2.0)],
            1,
        )
        .remove(0);
        assert_eq!(interpreter.ffi_numeric_u128(&third, None).unwrap(), 31);

        let packed_layout_type = namespace_call(
            &mut interpreter,
            &namespace,
            "Structure",
            vec![
                Value::from("Packed"),
                field_table(vec![("tag", u8_type.clone()), ("value", u32_type.clone())]),
                Value::Double(1.0),
            ],
        );
        let packed_layout = interpreter
            .ffi_type_value(&packed_layout_type, None)
            .unwrap();
        assert_eq!(packed_layout.size(), 5);
        assert_eq!(packed_layout.alignment(), 1);

        let union_type = namespace_call(
            &mut interpreter,
            &namespace,
            "Union",
            vec![
                Value::from("Number"),
                field_table(vec![("wide", u32_type), ("low", u16_type)]),
            ],
        );
        let union = interpreter
            .ffi_apply_value(&union_type, &[], 1, None)
            .unwrap()
            .unwrap()
            .remove(0);
        interpreter
            .ffi_set_field(&union, "low", &Value::Double(f64::from(0x7788)), None)
            .unwrap()
            .unwrap();
        let wide = interpreter
            .ffi_get_field(&union, "wide", None)
            .unwrap()
            .unwrap();
        assert_eq!(interpreter.ffi_numeric_u128(&wide, None).unwrap(), 0x7788);
    }

    #[test]
    fn owned_utf8_and_utf16_buffers_round_trip() {
        let mut interpreter = interpreter();
        let namespace = interpreter.ffi_namespace(None).unwrap();
        let narrow = namespace_call(
            &mut interpreter,
            &namespace,
            "cstr",
            vec![Value::from("OpenMat")],
        );
        let narrow_text = namespace_call(&mut interpreter, &namespace, "read_cstr", vec![narrow]);
        assert_eq!(
            interpreter
                .function_name_text("test", &narrow_text, None)
                .unwrap(),
            "OpenMat"
        );

        let wide = namespace_call(
            &mut interpreter,
            &namespace,
            "wstr",
            vec![Value::from("ffi测试")],
        );
        let wide_text = namespace_call(&mut interpreter, &namespace, "read_wstr", vec![wide]);
        assert_eq!(
            interpreter
                .function_name_text("test", &wide_text, None)
                .unwrap(),
            "ffi测试"
        );
    }

    #[cfg(windows)]
    #[test]
    fn function_can_capture_windows_last_error_immediately() {
        let mut interpreter = interpreter();
        let namespace = interpreter.ffi_namespace(None).unwrap();
        let library = namespace_call(
            &mut interpreter,
            &namespace,
            "Library",
            vec![Value::from("kernel32.dll")],
        );
        let function = interpreter
            .ffi_get_field(&library, "SetLastError", None)
            .unwrap()
            .unwrap();
        let u32_type = type_value(&mut interpreter, &namespace, "u32");
        let void_type = type_value(&mut interpreter, &namespace, "void");
        let argtypes = Value::Cell(
            CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![u32_type]).unwrap(),
        );
        interpreter
            .ffi_set_field(&function, "argtypes", &argtypes, None)
            .unwrap()
            .unwrap();
        interpreter
            .ffi_set_field(&function, "restype", &void_type, None)
            .unwrap()
            .unwrap();
        interpreter
            .ffi_set_field(&function, "use_last_error", &Value::Logical(true), None)
            .unwrap()
            .unwrap();
        interpreter
            .ffi_apply_value(
                &function,
                &arguments(vec![Value::Double(f64::from(0x5a17))]),
                0,
                None,
            )
            .unwrap()
            .unwrap();
        let last_error = interpreter
            .ffi_get_field(&function, "last_error", None)
            .unwrap()
            .unwrap();
        assert_eq!(
            interpreter.ffi_numeric_u128(&last_error, None).unwrap(),
            0x5a17
        );
    }
}

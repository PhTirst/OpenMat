//! Low-level native-library calls for the `OpenMat` `ffi` namespace.
//!
//! This crate deliberately does not try to make a declared foreign signature
//! safe. The caller supplies the C ABI and every argument/result type. A wrong
//! declaration can corrupt memory or terminate the process.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc, handle_alloc_error},
    error::Error,
    ffi::c_void,
    fmt,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

/// One calling convention selected by M code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Abi {
    /// The single Windows x64 calling convention.
    Win64,
    /// C spelling retained as an x64 alias for `Win64`.
    Cdecl,
    /// Win32 spelling retained as an x64 alias for `Win64`.
    Stdcall,
}

/// A C field after deterministic natural layout.
#[derive(Clone, Debug, PartialEq)]
pub struct CField {
    pub name: String,
    pub type_: Arc<CType>,
    pub offset: usize,
}

/// A named C structure layout. Rust native layout is never exposed.
#[derive(Clone, Debug, PartialEq)]
pub struct CStruct {
    pub name: String,
    pub fields: Vec<CField>,
    pub size: usize,
    pub alignment: usize,
    /// Optional C packing ceiling. `None` means natural alignment.
    pub pack: Option<usize>,
}

impl CStruct {
    /// Creates a naturally aligned C structure for the Windows x64 target.
    ///
    /// # Errors
    ///
    /// Rejects `void` fields, duplicate names, and overflowing layouts.
    pub fn new(
        name: impl Into<String>,
        fields: Vec<(String, Arc<CType>)>,
    ) -> Result<Self, FfiError> {
        Self::with_pack(name, fields, None)
    }

    /// Creates a structure whose field alignment is capped by `pack`.
    ///
    /// # Errors
    ///
    /// Rejects invalid packing values, `void` fields, duplicate names, and
    /// overflowing layouts.
    pub fn with_pack(
        name: impl Into<String>,
        fields: Vec<(String, Arc<CType>)>,
        pack: Option<usize>,
    ) -> Result<Self, FfiError> {
        validate_pack(pack)?;
        let mut offset = 0_usize;
        let mut alignment = 1_usize;
        let mut laid_out = Vec::with_capacity(fields.len());
        for (field_name, type_) in fields {
            if matches!(&*type_, CType::Void) {
                return Err(FfiError::Type(format!(
                    "structure field `{field_name}` cannot have type void"
                )));
            }
            if laid_out
                .iter()
                .any(|field: &CField| field.name == field_name)
            {
                return Err(FfiError::Type(format!(
                    "duplicate structure field `{field_name}`"
                )));
            }
            let field_alignment = packed_alignment(type_.alignment(), pack);
            offset = align_up(offset, field_alignment).ok_or(FfiError::LayoutOverflow)?;
            let field_offset = offset;
            offset = offset
                .checked_add(type_.size())
                .ok_or(FfiError::LayoutOverflow)?;
            alignment = alignment.max(field_alignment);
            laid_out.push(CField {
                name: field_name,
                type_,
                offset: field_offset,
            });
        }
        let size = align_up(offset, alignment).ok_or(FfiError::LayoutOverflow)?;
        Ok(Self {
            name: name.into(),
            fields: laid_out,
            size,
            alignment,
            pack,
        })
    }

    #[must_use]
    pub fn field(&self, name: &str) -> Option<&CField> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// A named C union layout. Every field starts at offset zero.
#[derive(Clone, Debug, PartialEq)]
pub struct CUnion {
    pub name: String,
    pub fields: Vec<CField>,
    pub size: usize,
    pub alignment: usize,
    /// Optional C packing ceiling. `None` means natural alignment.
    pub pack: Option<usize>,
}

impl CUnion {
    /// Creates a naturally aligned union.
    ///
    /// # Errors
    ///
    /// Rejects `void` fields, duplicate names, and overflowing layouts.
    pub fn new(
        name: impl Into<String>,
        fields: Vec<(String, Arc<CType>)>,
    ) -> Result<Self, FfiError> {
        Self::with_pack(name, fields, None)
    }

    /// Creates a union whose member alignment is capped by `pack`.
    ///
    /// # Errors
    ///
    /// Rejects invalid packing values, `void` fields, duplicate names, and
    /// overflowing layouts.
    pub fn with_pack(
        name: impl Into<String>,
        fields: Vec<(String, Arc<CType>)>,
        pack: Option<usize>,
    ) -> Result<Self, FfiError> {
        validate_pack(pack)?;
        let mut alignment = 1_usize;
        let mut maximum_size = 0_usize;
        let mut laid_out = Vec::with_capacity(fields.len());
        for (field_name, type_) in fields {
            if matches!(&*type_, CType::Void) {
                return Err(FfiError::Type(format!(
                    "union field `{field_name}` cannot have type void"
                )));
            }
            if laid_out
                .iter()
                .any(|field: &CField| field.name == field_name)
            {
                return Err(FfiError::Type(format!(
                    "duplicate union field `{field_name}`"
                )));
            }
            alignment = alignment.max(packed_alignment(type_.alignment(), pack));
            maximum_size = maximum_size.max(type_.size());
            laid_out.push(CField {
                name: field_name,
                type_,
                offset: 0,
            });
        }
        let size = align_up(maximum_size, alignment).ok_or(FfiError::LayoutOverflow)?;
        Ok(Self {
            name: name.into(),
            fields: laid_out,
            size,
            alignment,
            pack,
        })
    }

    #[must_use]
    pub fn field(&self, name: &str) -> Option<&CField> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// One fixed-size C array type.
#[derive(Clone, Debug, PartialEq)]
pub struct CArray {
    pub element: Arc<CType>,
    pub length: usize,
    pub size: usize,
    pub alignment: usize,
}

impl CArray {
    /// Creates a fixed-size array. Flexible and zero-length members are not
    /// represented by this type.
    ///
    /// # Errors
    ///
    /// Rejects `void`, a zero length, and overflowing layouts.
    pub fn new(element: Arc<CType>, length: usize) -> Result<Self, FfiError> {
        if matches!(&*element, CType::Void) {
            return Err(FfiError::Type(
                "C array elements cannot have type void".to_owned(),
            ));
        }
        if length == 0 {
            return Err(FfiError::Type(
                "C array length must be greater than zero".to_owned(),
            ));
        }
        let size = element
            .size()
            .checked_mul(length)
            .ok_or(FfiError::LayoutOverflow)?;
        let alignment = element.alignment();
        Ok(Self {
            element,
            length,
            size,
            alignment,
        })
    }
}

/// A complete function-pointer type.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionType {
    pub abi: Abi,
    pub arguments: Vec<Arc<CType>>,
    pub result: Arc<CType>,
}

/// Stable, explicitly described C types used by the M-language API.
#[derive(Clone, Debug, PartialEq)]
pub enum CType {
    Void,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Pointer(Arc<CType>),
    Array(Arc<CArray>),
    Structure(Arc<CStruct>),
    Union(Arc<CUnion>),
    Function(Arc<FunctionType>),
}

impl CType {
    #[must_use]
    pub fn size(&self) -> usize {
        match self {
            Self::Void => 0,
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 | Self::Pointer(_) | Self::Function(_) => 8,
            Self::Array(array) => array.size,
            Self::Structure(structure) => structure.size,
            Self::Union(union) => union.size,
        }
    }

    #[must_use]
    pub fn alignment(&self) -> usize {
        match self {
            Self::Void | Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 | Self::Pointer(_) | Self::Function(_) => 8,
            Self::Array(array) => array.alignment,
            Self::Structure(structure) => structure.alignment,
            Self::Union(union) => union.alignment,
        }
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        match self {
            Self::Void => "void",
            Self::I8 => "i8",
            Self::U8 => "u8",
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Pointer(_) => "pointer",
            Self::Array(_) => "array",
            Self::Structure(structure) => &structure.name,
            Self::Union(union) => &union.name,
            Self::Function(_) => "function_pointer",
        }
    }
}

#[derive(Debug)]
struct Allocation {
    pointer: NonNull<u8>,
    layout: Layout,
}

// Native code is allowed to mutate pointed-to storage. OpenMat serializes M
// execution; these markers only allow session objects to move with the kernel.
unsafe impl Send for Allocation {}
unsafe impl Sync for Allocation {}

impl Allocation {
    fn new(size: usize, alignment: usize) -> Result<Self, FfiError> {
        let layout = Layout::from_size_align(size.max(1), alignment.max(1))
            .map_err(|_| FfiError::LayoutOverflow)?;
        // SAFETY: `layout` is non-zero and was validated above.
        let pointer = unsafe { alloc_zeroed(layout) };
        let Some(pointer) = NonNull::new(pointer) else {
            handle_alloc_error(layout);
        };
        Ok(Self { pointer, layout })
    }
}

impl Drop for Allocation {
    fn drop(&mut self) {
        // SAFETY: this allocation was created with exactly this layout and has
        // not been deallocated elsewhere.
        unsafe { dealloc(self.pointer.as_ptr(), self.layout) };
    }
}

/// A typed or untyped memory view whose address remains stable.
#[derive(Clone, Debug)]
pub struct Memory {
    address: usize,
    length: usize,
    owner: Option<Arc<Allocation>>,
}

impl Memory {
    /// Allocates zeroed, suitably aligned storage for one declared C value.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested layout cannot be represented.
    pub fn allocate(type_: &CType) -> Result<Self, FfiError> {
        Self::allocate_bytes(type_.size(), type_.alignment())
    }

    /// Allocates a zeroed block with an explicit size and alignment.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested layout cannot be represented.
    pub fn allocate_bytes(size: usize, alignment: usize) -> Result<Self, FfiError> {
        let owner = Arc::new(Allocation::new(size, alignment)?);
        Ok(Self {
            address: owner.pointer.as_ptr() as usize,
            length: size,
            owner: Some(owner),
        })
    }

    /// Creates a view of arbitrary process memory.
    ///
    #[must_use]
    pub const fn from_address(address: usize, length: usize) -> Self {
        Self {
            address,
            length,
            owner: None,
        }
    }

    #[must_use]
    pub const fn address(&self) -> usize {
        self.address
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.length
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Returns a sub-view and retains an owned allocation when one exists.
    ///
    #[must_use]
    pub fn offset(&self, offset: usize, length: usize) -> Self {
        Self {
            address: self.address.wrapping_add(offset),
            length,
            owner: self.owner.clone(),
        }
    }

    /// Returns a signed byte-offset view and retains an owned allocation.
    ///
    #[must_use]
    pub fn offset_signed(&self, offset: isize, length: usize) -> Self {
        Self {
            address: self.address.wrapping_add_signed(offset),
            length,
            owner: self.owner.clone(),
        }
    }

    /// Copies bytes from the viewed process memory.
    ///
    pub fn read_into(&self, output: &mut [u8]) {
        // SAFETY: delegated to this method's caller.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.address as *const u8,
                output.as_mut_ptr(),
                output.len(),
            );
        }
    }

    /// Copies bytes into the viewed process memory.
    ///
    pub fn write(&self, input: &[u8]) {
        // SAFETY: delegated to this method's caller.
        unsafe {
            std::ptr::copy_nonoverlapping(input.as_ptr(), self.address as *mut u8, input.len());
        }
    }

    /// Copies this view into an owned byte vector.
    ///
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        let mut bytes = vec![0_u8; self.length];
        // SAFETY: delegated to this method's caller.
        self.read_into(&mut bytes);
        bytes
    }
}

/// An open dynamic library. Functions retain an `Arc` owner to it.
pub struct Library {
    path: PathBuf,
    #[cfg(windows)]
    native: libloading::Library,
}

impl fmt::Debug for Library {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Library")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Library {
    /// Loads one dynamic library into the current process.
    ///
    /// # Errors
    ///
    /// Returns a platform or loader error when the DLL cannot be loaded.
    pub fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, FfiError> {
        let path = path.as_ref();
        #[cfg(windows)]
        {
            // SAFETY: delegated to this method's caller and the M `ffi` user.
            let native = unsafe { libloading::Library::new(path) }
                .map_err(|error| FfiError::Library(error.to_string()))?;
            Ok(Arc::new(Self {
                path: path.to_path_buf(),
                native,
            }))
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(FfiError::UnsupportedPlatform)
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolves an exported symbol without imposing a Rust function type.
    ///
    /// # Errors
    ///
    /// Returns an error when the export cannot be resolved.
    pub fn symbol_address(&self, name: &str) -> Result<usize, FfiError> {
        #[cfg(windows)]
        {
            type UntypedFunction = unsafe extern "system" fn();
            // SAFETY: the symbol is used only as an untyped address.
            let symbol = unsafe { self.native.get::<UntypedFunction>(name.as_bytes()) }.map_err(
                |error| FfiError::Symbol {
                    name: name.to_owned(),
                    message: error.to_string(),
                },
            )?;
            Ok(*symbol as *const () as usize)
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Err(FfiError::UnsupportedPlatform)
        }
    }
}

/// A callable address with mutable ctypes-style signature properties.
#[derive(Debug)]
pub struct NativeFunction {
    address: usize,
    library: Option<Arc<Library>>,
    signature: Mutex<FunctionType>,
    use_last_error: AtomicBool,
    last_error: AtomicU32,
}

impl NativeFunction {
    #[must_use]
    pub fn new(
        address: usize,
        library: Option<Arc<Library>>,
        signature: FunctionType,
    ) -> Arc<Self> {
        Arc::new(Self {
            address,
            library,
            signature: Mutex::new(signature),
            use_last_error: AtomicBool::new(false),
            last_error: AtomicU32::new(0),
        })
    }

    #[must_use]
    pub const fn address(&self) -> usize {
        self.address
    }

    #[must_use]
    pub fn library_path(&self) -> Option<&Path> {
        self.library.as_deref().map(Library::path)
    }

    #[must_use]
    pub fn signature(&self) -> FunctionType {
        self.signature
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_arguments(&self, arguments: Vec<Arc<CType>>) {
        self.signature
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .arguments = arguments;
    }

    pub fn set_result(&self, result: Arc<CType>) {
        self.signature
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .result = result;
    }

    pub fn set_abi(&self, abi: Abi) {
        self.signature
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .abi = abi;
    }

    /// Enables or disables immediate Windows `GetLastError` capture after a
    /// native call.
    pub fn set_use_last_error(&self, enabled: bool) {
        self.use_last_error.store(enabled, Ordering::Release);
        if !enabled {
            self.last_error.store(0, Ordering::Release);
        }
    }

    #[must_use]
    pub fn use_last_error(&self) -> bool {
        self.use_last_error.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn last_error(&self) -> u32 {
        self.last_error.load(Ordering::Acquire)
    }

    /// Calls the address using only the user-declared signature.
    ///
    /// # Errors
    ///
    /// Returns setup and declared-arity errors. Native faults are not caught.
    ///
    pub fn call(&self, arguments: &[Memory]) -> Result<Option<Memory>, FfiError> {
        let signature = self.signature();
        if arguments.len() != signature.arguments.len() {
            return Err(FfiError::Arity {
                expected: signature.arguments.len(),
                actual: arguments.len(),
            });
        }
        // SAFETY: delegated to the caller and implemented by the platform
        // boundary below.
        // SAFETY: the M-language caller explicitly supplied the unchecked
        // native declaration.
        let capture_last_error = self.use_last_error();
        let call =
            unsafe { platform_call(self.address, &signature, arguments, capture_last_error) }?;
        if let Some(last_error) = call.last_error {
            self.last_error.store(last_error, Ordering::Release);
        }
        Ok(call.returned)
    }
}

struct NativeCall {
    returned: Option<Memory>,
    last_error: Option<u32>,
}

#[cfg(windows)]
unsafe fn platform_call(
    address: usize,
    signature: &FunctionType,
    arguments: &[Memory],
    capture_last_error: bool,
) -> Result<NativeCall, FfiError> {
    use libffi::middle::{Arg, Cif, CodePtr, Ret};

    let argument_types = signature
        .arguments
        .iter()
        .map(|type_| libffi_type(type_))
        .collect::<Vec<_>>();
    let result_type = libffi_type(&signature.result);
    let cif = Cif::try_new_with_abi(
        argument_types,
        result_type,
        libffi::middle::ffi_abi_FFI_DEFAULT_ABI,
    )
    .map_err(|error| FfiError::Call(format!("cannot prepare call interface: {error:?}")))?;

    let argument_slices = arguments
        .iter()
        .map(|argument| {
            // SAFETY: signature validation is deliberately the FFI user's
            // responsibility. A one-byte slice is used for zero-sized values.
            unsafe {
                std::slice::from_raw_parts(argument.address() as *const u8, argument.len().max(1))
            }
        })
        .collect::<Vec<_>>();
    let ffi_arguments = argument_slices
        .iter()
        .map(|slice| Arg::new(*slice))
        .collect::<Vec<_>>();
    let code = CodePtr::from_ptr(address as *const c_void);

    if matches!(&*signature.result, CType::Void) {
        // SAFETY: the M declaration is the complete safety contract.
        unsafe { cif.call_return_into(code, &ffi_arguments, Ret::void()) };
        let last_error = capture_last_error.then(windows_last_error);
        return Ok(NativeCall {
            returned: None,
            last_error,
        });
    }

    let result = Memory::allocate_bytes(
        signature.result.size().max(std::mem::size_of::<usize>()),
        signature
            .result
            .alignment()
            .max(std::mem::align_of::<usize>()),
    )?;
    // SAFETY: the allocation is writable and large/aligned enough for both
    // the declared result and libffi's small-return convention.
    let result_slice =
        unsafe { std::slice::from_raw_parts_mut(result.address() as *mut u8, result.len().max(1)) };
    // SAFETY: the M declaration is the complete safety contract.
    unsafe { cif.call_return_into(code, &ffi_arguments, Ret::new(result_slice)) };
    let last_error = capture_last_error.then(windows_last_error);
    Ok(NativeCall {
        returned: Some(result),
        last_error,
    })
}

#[cfg(windows)]
fn windows_last_error() -> u32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLastError() -> u32;
    }

    // SAFETY: `GetLastError` has no arguments and returns thread-local state.
    unsafe { GetLastError() }
}

#[cfg(windows)]
fn libffi_type(type_: &CType) -> libffi::middle::Type {
    use libffi::middle::Type;
    match type_ {
        CType::Void => Type::void(),
        CType::I8 => Type::i8(),
        CType::U8 => Type::u8(),
        CType::I16 => Type::i16(),
        CType::U16 => Type::u16(),
        CType::I32 => Type::i32(),
        CType::U32 => Type::u32(),
        CType::I64 => Type::i64(),
        CType::U64 => Type::u64(),
        CType::F32 => Type::f32(),
        CType::F64 => Type::f64(),
        CType::Pointer(_) | CType::Function(_) => Type::pointer(),
        CType::Array(array) => Type::structure(
            std::iter::repeat_with(|| libffi_type(&array.element)).take(array.length),
        ),
        CType::Structure(structure) => Type::structure(if structure.pack.is_some() {
            aggregate_blob_fields(structure.size, structure.alignment)
        } else {
            structure
                .fields
                .iter()
                .map(|field| libffi_type(&field.type_))
                .collect::<Vec<_>>()
        }),
        CType::Union(union) => Type::structure(aggregate_blob_fields(union.size, union.alignment)),
    }
}

#[cfg(windows)]
fn aggregate_blob_fields(size: usize, alignment: usize) -> Vec<libffi::middle::Type> {
    use libffi::middle::Type;

    if size == 0 {
        return Vec::new();
    }
    let (head, head_size) = match alignment {
        8 => (Type::u64(), 8),
        4 => (Type::u32(), 4),
        2 => (Type::u16(), 2),
        _ => (Type::u8(), 1),
    };
    let mut fields = Vec::with_capacity(size.saturating_sub(head_size).saturating_add(1));
    fields.push(head);
    fields.extend(std::iter::repeat_with(Type::u8).take(size.saturating_sub(head_size)));
    fields
}

#[cfg(not(windows))]
unsafe fn platform_call(
    _address: usize,
    _signature: &FunctionType,
    _arguments: &[Memory],
    _capture_last_error: bool,
) -> Result<NativeCall, FfiError> {
    Err(FfiError::UnsupportedPlatform)
}

/// Failures in host-side setup. Native faults are intentionally not caught.
#[derive(Debug)]
pub enum FfiError {
    UnsupportedPlatform,
    Library(String),
    Symbol { name: String, message: String },
    Type(String),
    Arity { expected: usize, actual: usize },
    LayoutOverflow,
    Call(String),
}

impl fmt::Display for FfiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("FFI currently requires Windows"),
            Self::Library(message) => write!(formatter, "cannot load dynamic library: {message}"),
            Self::Symbol { name, message } => {
                write!(formatter, "cannot resolve symbol `{name}`: {message}")
            }
            Self::Type(message) | Self::Call(message) => formatter.write_str(message),
            Self::Arity { expected, actual } => {
                write!(
                    formatter,
                    "native call expects {expected} arguments, received {actual}"
                )
            }
            Self::LayoutOverflow => formatter.write_str("C type layout exceeds host capacity"),
        }
    }
}

impl Error for FfiError {}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    let mask = alignment - 1;
    value.checked_add(mask).map(|value| value & !mask)
}

fn validate_pack(pack: Option<usize>) -> Result<(), FfiError> {
    if pack.is_some_and(|value| value == 0 || !value.is_power_of_two()) {
        return Err(FfiError::Type(
            "C packing must be a positive power of two".to_owned(),
        ));
    }
    Ok(())
}

fn packed_alignment(natural: usize, pack: Option<usize>) -> usize {
    pack.map_or(natural, |pack| natural.min(pack))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_struct_layout_is_independent_of_rust_structs() {
        let structure = CStruct::new(
            "Mixed",
            vec![
                ("tag".to_owned(), Arc::new(CType::U8)),
                ("value".to_owned(), Arc::new(CType::U64)),
                ("tail".to_owned(), Arc::new(CType::U16)),
            ],
        )
        .unwrap();
        assert_eq!(structure.fields[0].offset, 0);
        assert_eq!(structure.fields[1].offset, 8);
        assert_eq!(structure.fields[2].offset, 16);
        assert_eq!(structure.size, 24);
        assert_eq!(structure.alignment, 8);
        assert_eq!(structure.pack, None);
    }

    #[test]
    fn packed_struct_layout_caps_field_alignment() {
        let structure = CStruct::with_pack(
            "Packed",
            vec![
                ("tag".to_owned(), Arc::new(CType::U8)),
                ("value".to_owned(), Arc::new(CType::U32)),
                ("tail".to_owned(), Arc::new(CType::U16)),
            ],
            Some(1),
        )
        .unwrap();
        assert_eq!(structure.fields[0].offset, 0);
        assert_eq!(structure.fields[1].offset, 1);
        assert_eq!(structure.fields[2].offset, 5);
        assert_eq!(structure.size, 7);
        assert_eq!(structure.alignment, 1);
    }

    #[test]
    fn union_and_array_layouts_are_explicit() {
        let array = Arc::new(CArray::new(Arc::new(CType::U16), 3).unwrap());
        assert_eq!(array.size, 6);
        assert_eq!(array.alignment, 2);

        let union = CUnion::new(
            "Number",
            vec![
                ("small".to_owned(), Arc::new(CType::U8)),
                ("wide".to_owned(), Arc::new(CType::U64)),
                ("words".to_owned(), Arc::new(CType::Array(array))),
            ],
        )
        .unwrap();
        assert!(union.fields.iter().all(|field| field.offset == 0));
        assert_eq!(union.size, 8);
        assert_eq!(union.alignment, 8);
    }

    #[cfg(windows)]
    #[test]
    fn calls_a_real_windows_dll_function() {
        let library = Library::open("kernel32.dll").unwrap();
        let address = library.symbol_address("GetCurrentProcessId").unwrap();
        let function = NativeFunction::new(
            address,
            Some(library),
            FunctionType {
                abi: Abi::Win64,
                arguments: Vec::new(),
                result: Arc::new(CType::U32),
            },
        );
        let result = function.call(&[]).unwrap().unwrap();
        let bytes = result.to_vec();
        assert_eq!(
            u32::from_ne_bytes(bytes[..4].try_into().unwrap()),
            std::process::id()
        );
    }

    #[cfg(windows)]
    #[test]
    fn passes_a_pointer_argument_through_libffi() {
        let library = Library::open("kernel32.dll").unwrap();
        let address = library.symbol_address("GetModuleHandleW").unwrap();
        let pointer = Arc::new(CType::Pointer(Arc::new(CType::U16)));
        let function = NativeFunction::new(
            address,
            Some(library),
            FunctionType {
                abi: Abi::Win64,
                arguments: vec![pointer.clone()],
                result: pointer.clone(),
            },
        );
        let argument = Memory::allocate(&pointer).unwrap();
        argument.write(&0_usize.to_ne_bytes());
        let result = function.call(&[argument]).unwrap().unwrap();
        let bytes = result.to_vec();
        assert_ne!(
            usize::from_ne_bytes(bytes[..std::mem::size_of::<usize>()].try_into().unwrap()),
            0
        );
    }
}

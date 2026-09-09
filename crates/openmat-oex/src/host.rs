pub(crate) mod containers;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::mem::{MaybeUninit, size_of};
use std::num::NonZeroUsize;
use std::ptr;
use std::slice;
use std::str;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32, Complex64 as ArrayComplex64, ComplexInteger, DType,
    DenseArray, IntegerArrayData, Logical, Shape,
};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult, NativeClassToken,
    NativeInstance,
};
use openmat_value::{CooEntry, CscMatrix, SparseArrayData, SparseError, Value, ValueKind};

use crate::abi::{
    OEX_ABI_VERSION, OEX_BRIDGE_MAGIC, OEX_DATA_CHAR16, OEX_DATA_COMPLEX_F32, OEX_DATA_COMPLEX_F64,
    OEX_DATA_COMPLEX_I8, OEX_DATA_COMPLEX_I16, OEX_DATA_COMPLEX_I32, OEX_DATA_COMPLEX_I64,
    OEX_DATA_COMPLEX_U8, OEX_DATA_COMPLEX_U16, OEX_DATA_COMPLEX_U32, OEX_DATA_COMPLEX_U64,
    OEX_DATA_F32, OEX_DATA_F64, OEX_DATA_I8, OEX_DATA_I16, OEX_DATA_I32, OEX_DATA_I64,
    OEX_DATA_LOGICAL, OEX_DATA_NONE, OEX_DATA_U8, OEX_DATA_U16, OEX_DATA_U32, OEX_DATA_U64,
    OEX_ERROR_ABI, OEX_ERROR_ALLOCATION, OEX_ERROR_ARGUMENT, OEX_ERROR_CALLBACK,
    OEX_ERROR_CANCELLED, OEX_ERROR_DIMENSION, OEX_ERROR_PLUGIN, OEX_ERROR_RANGE, OEX_ERROR_STATE,
    OEX_ERROR_TYPE, OEX_ERROR_UNSUPPORTED, OEX_HEADER_CALL, OEX_HEADER_CANCELLATION,
    OEX_HEADER_DENSE_BUILDER, OEX_HEADER_SPARSE_PATTERN, OEX_HEADER_SPARSE_PATTERN_BUILDER,
    OEX_HEADER_SPARSE_TRIPLET_BUILDER, OEX_HEADER_VALUE, OEX_OK, OEX_VALUE_CELL, OEX_VALUE_DENSE,
    OEX_VALUE_FLAG_COMPLEX, OEX_VALUE_FUNCTION, OEX_VALUE_GRAPHICS, OEX_VALUE_NOTHING,
    OEX_VALUE_OBJECT, OEX_VALUE_SPARSE, OEX_VALUE_STRING, OEX_VALUE_STRUCT, OEX_VALUE_TABLE,
    OexCall, OexCancellation, OexDenseBuilder, OexDenseView, OexDispatch, OexFunctionCallback,
    OexMutableDenseView, OexNativeConstructor, OexNativeDestructor, OexNativeMethod,
    OexNativePropertyGetter, OexNativePropertySetter, OexObjectHeader, OexSparseCscView,
    OexSparsePattern, OexSparsePatternBuilder, OexSparsePatternValuesView, OexSparseTripletBuilder,
    OexSparseTripletView, OexStatus, OexUtf8View, OexValue, OexValueInfo, abi_size,
};

const VALUE_BORROWED: u64 = 1;
const VALUE_OWNED: u64 = 2;
const SCALAR_DIMENSIONS: [u64; 2] = [1, 1];

#[derive(Clone)]
pub(crate) struct FunctionRegistration {
    pub(crate) name: String,
    pub(crate) invoke: OexFunctionCallback,
    pub(crate) minimum_inputs: u32,
    pub(crate) maximum_inputs: u32,
    pub(crate) minimum_outputs: u32,
    pub(crate) maximum_outputs: u32,
}

#[derive(Clone)]
pub(crate) struct MethodRegistration {
    pub(crate) name: String,
    pub(crate) invoke: OexNativeMethod,
    pub(crate) minimum_inputs: u32,
    pub(crate) maximum_inputs: u32,
    pub(crate) minimum_outputs: u32,
    pub(crate) maximum_outputs: u32,
}

#[repr(C)]
struct ValueHandle {
    header: OexObjectHeader,
    ownership: u64,
    value: Value,
}

impl ValueHandle {
    fn borrowed(value: Value) -> Self {
        Self::new(VALUE_BORROWED, normalize_scalar_storage(value))
    }

    fn owned(value: Value) -> Self {
        Self::new(VALUE_OWNED, normalize_scalar_storage(value))
    }

    fn new(ownership: u64, value: Value) -> Self {
        Self {
            header: object_header(OEX_HEADER_VALUE),
            ownership,
            value,
        }
    }

    const fn is_owned(&self) -> bool {
        self.ownership == VALUE_OWNED
    }
}

fn normalize_scalar_storage(value: Value) -> Value {
    if let Value::Logical(value) = value {
        let shape = Shape::new(SCALAR_DIMENSIONS).expect("the scalar shape is valid");
        let array = DenseArray::from_vec(shape, vec![Logical::from(value)])
            .expect("one logical value matches the scalar shape");
        Value::Array(ArrayData::Logical(array))
    } else {
        value
    }
}

struct CallError {
    identifier: Option<String>,
    message: String,
}

#[repr(C)]
struct CancellationHandle {
    header: OexObjectHeader,
    flag: *const AtomicBool,
}

#[repr(C)]
struct CallContext {
    header: OexObjectHeader,
    context: *mut c_void,
    inputs: Vec<Option<Box<ValueHandle>>>,
    outputs: Vec<Option<Box<ValueHandle>>>,
    error: Option<CallError>,
    cancellation: CancellationHandle,
    builders: BTreeMap<usize, unsafe fn(usize)>,
    text_buffers: Vec<Box<str>>,
}

impl Drop for CallContext {
    fn drop(&mut self) {
        for (address, drop_builder) in std::mem::take(&mut self.builders) {
            // SAFETY: Every registered address came from Box::into_raw and a committed or
            // explicitly aborted builder removes itself from this set first.
            unsafe { drop_builder(address) };
        }
    }
}

unsafe fn drop_dense_builder(address: usize) {
    unsafe { drop(Box::from_raw(address as *mut DenseBuilderHandle)) };
}

#[repr(C)]
struct DenseBuilderHandle {
    header: OexObjectHeader,
    call: *mut CallContext,
    shape: Shape,
    storage: Option<DenseBuilderStorage>,
}

#[derive(Debug)]
struct SparsePatternStorage {
    rows: u64,
    columns: u64,
    column_offsets: Vec<u64>,
    row_indices: Vec<u64>,
}

#[repr(C)]
struct SparsePatternHandle {
    header: OexObjectHeader,
    storage: Arc<SparsePatternStorage>,
}

#[repr(C)]
struct SparseTripletBuilderHandle {
    header: OexObjectHeader,
    call: *mut CallContext,
    rows: u64,
    columns: u64,
    capacity: usize,
    row_indices: Vec<MaybeUninit<u64>>,
    column_indices: Vec<MaybeUninit<u64>>,
    values: Option<SparseBuilderStorage>,
}

#[repr(C)]
struct SparsePatternBuilderHandle {
    header: OexObjectHeader,
    call: *mut CallContext,
    pattern: Arc<SparsePatternStorage>,
    values: Option<SparseBuilderStorage>,
}

enum SparseBuilderStorage {
    Logical(Vec<MaybeUninit<Logical>>),
    F64(Vec<MaybeUninit<f64>>),
    ComplexF64(Vec<MaybeUninit<ArrayComplex64>>),
}

enum InitializedSparseStorage {
    Logical(Vec<Logical>),
    F64(Vec<f64>),
    ComplexF64(Vec<ArrayComplex64>),
}

impl SparseBuilderStorage {
    fn allocate(data_type: u32, length: usize) -> Result<Self, OexStatus> {
        Ok(match data_type {
            OEX_DATA_LOGICAL => {
                Self::Logical(allocate_uninitialized("logical sparse values", length)?)
            }
            OEX_DATA_F64 => Self::F64(allocate_uninitialized("f64 sparse values", length)?),
            OEX_DATA_COMPLEX_F64 => {
                Self::ComplexF64(allocate_uninitialized("complex-f64 sparse values", length)?)
            }
            _ => return Err(OEX_ERROR_TYPE),
        })
    }

    fn view_parts(&mut self) -> (u32, u32, *mut c_void) {
        match self {
            Self::Logical(values) => (
                OEX_DATA_LOGICAL,
                abi_size::<Logical>(),
                values.as_mut_ptr().cast::<Logical>().cast::<c_void>(),
            ),
            Self::F64(values) => (
                OEX_DATA_F64,
                abi_size::<f64>(),
                values.as_mut_ptr().cast::<f64>().cast::<c_void>(),
            ),
            Self::ComplexF64(values) => (
                OEX_DATA_COMPLEX_F64,
                abi_size::<ArrayComplex64>(),
                values
                    .as_mut_ptr()
                    .cast::<ArrayComplex64>()
                    .cast::<c_void>(),
            ),
        }
    }

    unsafe fn into_initialized(mut self, length: usize) -> InitializedSparseStorage {
        match &mut self {
            Self::Logical(values) => values.truncate(length),
            Self::F64(values) => values.truncate(length),
            Self::ComplexF64(values) => values.truncate(length),
        }
        match self {
            Self::Logical(values) => {
                InitializedSparseStorage::Logical(unsafe { assume_init_vec(values) })
            }
            Self::F64(values) => InitializedSparseStorage::F64(unsafe { assume_init_vec(values) }),
            Self::ComplexF64(values) => {
                InitializedSparseStorage::ComplexF64(unsafe { assume_init_vec(values) })
            }
        }
    }
}

fn allocate_uninitialized<T>(
    _storage: &'static str,
    length: usize,
) -> Result<Vec<MaybeUninit<T>>, OexStatus> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| OEX_ERROR_ALLOCATION)?;
    // SAFETY: MaybeUninit<T> may remain uninitialized until builder commit.
    unsafe { values.set_len(length) };
    Ok(values)
}

unsafe fn drop_sparse_triplet_builder(address: usize) {
    unsafe { drop(Box::from_raw(address as *mut SparseTripletBuilderHandle)) };
}

unsafe fn drop_sparse_pattern_builder(address: usize) {
    unsafe { drop(Box::from_raw(address as *mut SparsePatternBuilderHandle)) };
}

enum DenseBuilderStorage {
    Logical(Vec<MaybeUninit<Logical>>),
    Char(Vec<MaybeUninit<CharCodeUnit>>),
    I8(Vec<MaybeUninit<i8>>),
    U8(Vec<MaybeUninit<u8>>),
    I16(Vec<MaybeUninit<i16>>),
    U16(Vec<MaybeUninit<u16>>),
    I32(Vec<MaybeUninit<i32>>),
    U32(Vec<MaybeUninit<u32>>),
    I64(Vec<MaybeUninit<i64>>),
    U64(Vec<MaybeUninit<u64>>),
    F32(Vec<MaybeUninit<f32>>),
    F64(Vec<MaybeUninit<f64>>),
    ComplexF32(Vec<MaybeUninit<Complex32>>),
    ComplexF64(Vec<MaybeUninit<ArrayComplex64>>),
    ComplexI8(Vec<MaybeUninit<ComplexInteger<i8>>>),
    ComplexU8(Vec<MaybeUninit<ComplexInteger<u8>>>),
    ComplexI16(Vec<MaybeUninit<ComplexInteger<i16>>>),
    ComplexU16(Vec<MaybeUninit<ComplexInteger<u16>>>),
    ComplexI32(Vec<MaybeUninit<ComplexInteger<i32>>>),
    ComplexU32(Vec<MaybeUninit<ComplexInteger<u32>>>),
    ComplexI64(Vec<MaybeUninit<ComplexInteger<i64>>>),
    ComplexU64(Vec<MaybeUninit<ComplexInteger<u64>>>),
}

macro_rules! allocate_builder_variant {
    ($variant:ident, $element:ty, $length:expr) => {{
        let mut values = Vec::<MaybeUninit<$element>>::new();
        if values.try_reserve_exact($length).is_err() {
            return Err(OEX_ERROR_ALLOCATION);
        }
        // SAFETY: MaybeUninit<T> may remain uninitialized. The plugin must initialize every
        // element before committing the builder.
        unsafe { values.set_len($length) };
        Self::$variant(values)
    }};
}

impl DenseBuilderStorage {
    fn allocate(data_type: u32, length: usize) -> Result<Self, OexStatus> {
        Ok(match data_type {
            OEX_DATA_LOGICAL => allocate_builder_variant!(Logical, Logical, length),
            OEX_DATA_CHAR16 => allocate_builder_variant!(Char, CharCodeUnit, length),
            OEX_DATA_I8 => allocate_builder_variant!(I8, i8, length),
            OEX_DATA_U8 => allocate_builder_variant!(U8, u8, length),
            OEX_DATA_I16 => allocate_builder_variant!(I16, i16, length),
            OEX_DATA_U16 => allocate_builder_variant!(U16, u16, length),
            OEX_DATA_I32 => allocate_builder_variant!(I32, i32, length),
            OEX_DATA_U32 => allocate_builder_variant!(U32, u32, length),
            OEX_DATA_I64 => allocate_builder_variant!(I64, i64, length),
            OEX_DATA_U64 => allocate_builder_variant!(U64, u64, length),
            OEX_DATA_F32 => allocate_builder_variant!(F32, f32, length),
            OEX_DATA_F64 => allocate_builder_variant!(F64, f64, length),
            OEX_DATA_COMPLEX_F32 => {
                allocate_builder_variant!(ComplexF32, Complex32, length)
            }
            OEX_DATA_COMPLEX_F64 => {
                allocate_builder_variant!(ComplexF64, ArrayComplex64, length)
            }
            OEX_DATA_COMPLEX_I8 => {
                allocate_builder_variant!(ComplexI8, ComplexInteger<i8>, length)
            }
            OEX_DATA_COMPLEX_U8 => {
                allocate_builder_variant!(ComplexU8, ComplexInteger<u8>, length)
            }
            OEX_DATA_COMPLEX_I16 => {
                allocate_builder_variant!(ComplexI16, ComplexInteger<i16>, length)
            }
            OEX_DATA_COMPLEX_U16 => {
                allocate_builder_variant!(ComplexU16, ComplexInteger<u16>, length)
            }
            OEX_DATA_COMPLEX_I32 => {
                allocate_builder_variant!(ComplexI32, ComplexInteger<i32>, length)
            }
            OEX_DATA_COMPLEX_U32 => {
                allocate_builder_variant!(ComplexU32, ComplexInteger<u32>, length)
            }
            OEX_DATA_COMPLEX_I64 => {
                allocate_builder_variant!(ComplexI64, ComplexInteger<i64>, length)
            }
            OEX_DATA_COMPLEX_U64 => {
                allocate_builder_variant!(ComplexU64, ComplexInteger<u64>, length)
            }
            _ => return Err(OEX_ERROR_TYPE),
        })
    }

    fn view_parts(&mut self) -> (u32, u32, *mut c_void) {
        macro_rules! parts {
            ($data_type:expr, $values:expr, $element:ty) => {
                (
                    $data_type,
                    u32::try_from(size_of::<$element>()).unwrap_or(u32::MAX),
                    $values.as_mut_ptr().cast::<$element>().cast::<c_void>(),
                )
            };
        }
        match self {
            Self::Logical(values) => parts!(OEX_DATA_LOGICAL, values, Logical),
            Self::Char(values) => parts!(OEX_DATA_CHAR16, values, CharCodeUnit),
            Self::I8(values) => parts!(OEX_DATA_I8, values, i8),
            Self::U8(values) => parts!(OEX_DATA_U8, values, u8),
            Self::I16(values) => parts!(OEX_DATA_I16, values, i16),
            Self::U16(values) => parts!(OEX_DATA_U16, values, u16),
            Self::I32(values) => parts!(OEX_DATA_I32, values, i32),
            Self::U32(values) => parts!(OEX_DATA_U32, values, u32),
            Self::I64(values) => parts!(OEX_DATA_I64, values, i64),
            Self::U64(values) => parts!(OEX_DATA_U64, values, u64),
            Self::F32(values) => parts!(OEX_DATA_F32, values, f32),
            Self::F64(values) => parts!(OEX_DATA_F64, values, f64),
            Self::ComplexF32(values) => parts!(OEX_DATA_COMPLEX_F32, values, Complex32),
            Self::ComplexF64(values) => parts!(OEX_DATA_COMPLEX_F64, values, ArrayComplex64),
            Self::ComplexI8(values) => parts!(OEX_DATA_COMPLEX_I8, values, ComplexInteger<i8>),
            Self::ComplexU8(values) => parts!(OEX_DATA_COMPLEX_U8, values, ComplexInteger<u8>),
            Self::ComplexI16(values) => parts!(OEX_DATA_COMPLEX_I16, values, ComplexInteger<i16>),
            Self::ComplexU16(values) => parts!(OEX_DATA_COMPLEX_U16, values, ComplexInteger<u16>),
            Self::ComplexI32(values) => parts!(OEX_DATA_COMPLEX_I32, values, ComplexInteger<i32>),
            Self::ComplexU32(values) => parts!(OEX_DATA_COMPLEX_U32, values, ComplexInteger<u32>),
            Self::ComplexI64(values) => parts!(OEX_DATA_COMPLEX_I64, values, ComplexInteger<i64>),
            Self::ComplexU64(values) => parts!(OEX_DATA_COMPLEX_U64, values, ComplexInteger<u64>),
        }
    }

    fn into_value(self, shape: Shape) -> Result<Value, OexStatus> {
        macro_rules! dense {
            ($values:expr, $wrap:expr) => {{
                let values = unsafe { assume_init_vec($values) };
                let array = DenseArray::from_vec(shape, values).map_err(|_| OEX_ERROR_DIMENSION)?;
                $wrap(array)
            }};
        }
        Ok(match self {
            Self::Logical(values) => {
                dense!(values, |array| Value::Array(ArrayData::Logical(array)))
            }
            Self::Char(values) => dense!(values, |array| Value::Array(ArrayData::Char(array))),
            Self::I8(values) => dense!(values, integer_value),
            Self::U8(values) => dense!(values, integer_value),
            Self::I16(values) => dense!(values, integer_value),
            Self::U16(values) => dense!(values, integer_value),
            Self::I32(values) => dense!(values, integer_value),
            Self::U32(values) => dense!(values, integer_value),
            Self::I64(values) => dense!(values, integer_value),
            Self::U64(values) => dense!(values, integer_value),
            Self::F32(values) => dense!(values, |array| Value::Array(ArrayData::F32(array))),
            Self::F64(values) => dense!(values, |array| Value::Array(ArrayData::F64(array))),
            Self::ComplexF32(values) => {
                dense!(values, |array| Value::Array(ArrayData::ComplexF32(array)))
            }
            Self::ComplexF64(values) => {
                dense!(values, |array| Value::Array(ArrayData::ComplexF64(array)))
            }
            Self::ComplexI8(values) => dense!(values, integer_value),
            Self::ComplexU8(values) => dense!(values, integer_value),
            Self::ComplexI16(values) => dense!(values, integer_value),
            Self::ComplexU16(values) => dense!(values, integer_value),
            Self::ComplexI32(values) => dense!(values, integer_value),
            Self::ComplexU32(values) => dense!(values, integer_value),
            Self::ComplexI64(values) => dense!(values, integer_value),
            Self::ComplexU64(values) => dense!(values, integer_value),
        })
    }
}

fn integer_value<T: openmat_array::IntegerElement>(array: DenseArray<T>) -> Value {
    Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(array)))
}

unsafe fn assume_init_vec<T>(mut values: Vec<MaybeUninit<T>>) -> Vec<T> {
    let pointer = values.as_mut_ptr().cast::<T>();
    let length = values.len();
    let capacity = values.capacity();
    std::mem::forget(values);
    // SAFETY: MaybeUninit<T> has the same layout as T and commit promises initialization.
    unsafe { Vec::from_raw_parts(pointer, length, capacity) }
}

const fn object_header(kind: u64) -> OexObjectHeader {
    OexObjectHeader {
        dispatch: &raw const HOST_DISPATCH,
        bridge_magic: OEX_BRIDGE_MAGIC,
        kind,
    }
}

pub(crate) static HOST_DISPATCH: OexDispatch = OexDispatch {
    struct_size: abi_size::<OexDispatch>(),
    abi_version: OEX_ABI_VERSION,
    call_get_input_count,
    call_get_output_count,
    call_borrow_input,
    call_take_input,
    call_set_output,
    call_set_error,
    call_get_cancellation,
    cancellation_is_requested,
    value_get_info,
    value_retain,
    value_release,
    value_get_f64,
    value_create_f64,
    dense_get_view,
    dense_get_mutable_view,
    dense_builder_create_uninitialized,
    dense_builder_commit,
    dense_builder_abort,
    sparse_get_csc_view,
    sparse_pattern_create,
    sparse_pattern_retain,
    sparse_pattern_release,
    sparse_triplet_builder_create_uninitialized,
    sparse_triplet_builder_commit,
    sparse_triplet_builder_abort,
    sparse_pattern_builder_create_uninitialized,
    sparse_pattern_builder_commit,
    sparse_pattern_builder_abort,
    native_object_borrow_instance,
    native_object_create,
    call_invoke,
    value_get_dimensions: containers::value_get_dimensions,
    value_get_class_name: containers::value_get_class_name,
    string_create: containers::string_create,
    string_create_from_utf16: containers::string_create_from_utf16,
    string_set_elements_utf16: containers::string_set_elements_utf16,
    string_create_from_utf8: containers::string_create_from_utf8,
    string_set_elements_utf8: containers::string_set_elements_utf8,
    string_get_element_view: containers::string_get_element_view,
    string_get_elements: containers::string_get_elements,
    string_copy_element_utf8: containers::string_copy_element_utf8,
    string_set_element_utf8: containers::string_set_element_utf8,
    string_set_element_utf16: containers::string_set_element_utf16,
    string_set_missing: containers::string_set_missing,
    cell_create: containers::cell_create,
    cell_get_element: containers::cell_get_element,
    cell_set_element: containers::cell_set_element,
    cell_get_elements: containers::cell_get_elements,
    cell_set_elements: containers::cell_set_elements,
    struct_create: containers::struct_create,
    struct_get_field_count: containers::struct_get_field_count,
    struct_get_field_name: containers::struct_get_field_name,
    struct_find_field: containers::struct_find_field,
    struct_get_field: containers::struct_get_field,
    struct_set_field: containers::struct_set_field,
    struct_add_field: containers::struct_add_field,
    struct_remove_field: containers::struct_remove_field,
    struct_set_field_names: containers::struct_set_field_names,
    struct_get_field_values: containers::struct_get_field_values,
    struct_set_field_values: containers::struct_set_field_values,
    table_create: containers::table_create,
    table_get_row_count: containers::table_get_row_count,
    table_get_variable_count: containers::table_get_variable_count,
    table_get_variable_name: containers::table_get_variable_name,
    table_find_variable: containers::table_find_variable,
    table_set_variable_names: containers::table_set_variable_names,
    table_get_variable: containers::table_get_variable,
    table_set_variable: containers::table_set_variable,
    table_append_variable: containers::table_append_variable,
    table_remove_variable: containers::table_remove_variable,
    table_get_row_names: containers::table_get_row_names,
    table_set_row_names: containers::table_set_row_names,
    table_clear_row_names: containers::table_clear_row_names,
};

pub(crate) fn invoke(
    registration: &FunctionRegistration,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    invoke_owned(registration, arguments.to_vec(), context)
}

pub(crate) fn invoke_owned(
    registration: &FunctionRegistration,
    arguments: Vec<Value>,
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    validate_call_counts(
        &registration.name,
        registration.minimum_inputs,
        registration.maximum_inputs,
        registration.minimum_outputs,
        registration.maximum_outputs,
        arguments.len(),
        context.requested_outputs(),
    )?;
    context.check_cancelled()?;
    let mut call = new_call(arguments, context.requested_outputs(), context);
    // SAFETY: The callback came from a validated static descriptor and the call stays live.
    let status = unsafe { (registration.invoke)(ptr::from_mut(&mut call).cast::<OexCall>()) };
    finish_call(call, status, &registration.name, context)
}

pub(crate) fn invoke_native_constructor(
    name: &str,
    constructor: OexNativeConstructor,
    destructor: OexNativeDestructor,
    arguments: Vec<Value>,
    context: &mut BuiltinContext<'_>,
) -> Result<NativeInstance, BuiltinError> {
    context.check_cancelled()?;
    let mut call = new_call(arguments, 0, context);
    let mut instance = ptr::null_mut::<c_void>();
    // SAFETY: The callback came from a validated static class descriptor and both output
    // pointers remain valid for the duration of the call.
    let status = unsafe {
        constructor(
            ptr::from_mut(&mut call).cast::<OexCall>(),
            ptr::from_mut(&mut instance),
        )
    };
    if let Err(error) = finish_call(call, status, name, context) {
        if !instance.is_null() {
            // SAFETY: A non-null failed-construction result remains plugin-owned and the paired
            // destructor came from the same validated class descriptor.
            unsafe { destructor(instance) };
        }
        return Err(error);
    }
    let identifier = NonZeroUsize::new(instance.addr()).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("OEX constructor `{name}` returned a null instance"),
        )
    })?;
    Ok(NativeInstance::new(identifier))
}

pub(crate) fn invoke_native_method(
    class_name: &str,
    registration: &MethodRegistration,
    instance: NativeInstance,
    arguments: Vec<Value>,
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = format!("{class_name}.{}", registration.name);
    invoke_native_instance_callback(
        &name,
        registration.minimum_inputs,
        registration.maximum_inputs,
        registration.minimum_outputs,
        registration.maximum_outputs,
        registration.invoke,
        instance,
        arguments,
        context,
    )
}

pub(crate) fn invoke_native_property_get(
    class_name: &str,
    property_name: &str,
    getter: OexNativePropertyGetter,
    instance: NativeInstance,
    context: &mut BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = format!("{class_name}.get.{property_name}");
    let mut returned =
        invoke_native_instance_callback(&name, 0, 0, 1, 1, getter, instance, Vec::new(), context)?;
    returned.pop().ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("OEX property getter `{name}` returned no value"),
        )
    })
}

pub(crate) fn invoke_native_property_set(
    class_name: &str,
    property_name: &str,
    setter: OexNativePropertySetter,
    instance: NativeInstance,
    value: Value,
    context: &mut BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    let name = format!("{class_name}.set.{property_name}");
    invoke_native_instance_callback(&name, 1, 1, 0, 0, setter, instance, vec![value], context)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn invoke_native_instance_callback(
    name: &str,
    minimum_inputs: u32,
    maximum_inputs: u32,
    minimum_outputs: u32,
    maximum_outputs: u32,
    invoke: OexNativeMethod,
    instance: NativeInstance,
    arguments: Vec<Value>,
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    validate_call_counts(
        name,
        minimum_inputs,
        maximum_inputs,
        minimum_outputs,
        maximum_outputs,
        arguments.len(),
        context.requested_outputs(),
    )?;
    context.check_cancelled()?;
    let mut call = new_call(arguments, context.requested_outputs(), context);
    let instance = ptr::without_provenance_mut::<c_void>(instance.identifier().get());
    // SAFETY: The callback and instance came from the same pinned native class and the call
    // context remains valid for the callback duration.
    let status = unsafe { invoke(ptr::from_mut(&mut call).cast::<OexCall>(), instance) };
    finish_call(call, status, name, context)
}

fn new_call(
    arguments: Vec<Value>,
    output_count: usize,
    context: &mut BuiltinContext<'_>,
) -> CallContext {
    let inputs = arguments
        .into_iter()
        .map(|value| Some(Box::new(ValueHandle::borrowed(value))))
        .collect();
    CallContext {
        header: object_header(OEX_HEADER_CALL),
        context: ptr::from_mut(context).cast::<c_void>(),
        inputs,
        outputs: (0..output_count).map(|_| None).collect(),
        error: None,
        cancellation: CancellationHandle {
            header: object_header(OEX_HEADER_CANCELLATION),
            flag: ptr::from_ref(context.cancellation_flag()),
        },
        builders: BTreeMap::new(),
        text_buffers: Vec::new(),
    }
}

fn finish_call(
    mut call: CallContext,
    status: OexStatus,
    name: &str,
    context: &BuiltinContext<'_>,
) -> BuiltinResult {
    if status != OEX_OK {
        return Err(call_error(status, call.error.take(), name));
    }
    context.check_cancelled()?;
    if call.error.is_some() {
        return Err(call_error(OEX_ERROR_PLUGIN, call.error.take(), name));
    }
    if call.outputs.iter().any(Option::is_none) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Output,
            format!("OEX function `{name}` did not set every requested output"),
        ));
    }
    let mut values = Vec::new();
    values.try_reserve_exact(call.outputs.len()).map_err(|_| {
        BuiltinError::new(BuiltinErrorCategory::Other, "failed to reserve OEX outputs")
    })?;
    for output in &mut call.outputs {
        let handle = output.take().expect("all output slots were validated");
        values.push(handle.value);
    }
    Ok(values)
}

fn validate_call_counts(
    name: &str,
    minimum_inputs: u32,
    maximum_inputs: u32,
    minimum_outputs: u32,
    maximum_outputs: u32,
    inputs: usize,
    outputs: usize,
) -> Result<(), BuiltinError> {
    let inputs = u32::try_from(inputs).map_err(|_| {
        BuiltinError::new(BuiltinErrorCategory::ArgumentCount, "too many OEX inputs")
    })?;
    let outputs = u32::try_from(outputs).map_err(|_| {
        BuiltinError::new(BuiltinErrorCategory::ArgumentCount, "too many OEX outputs")
    })?;
    if !(minimum_inputs..=maximum_inputs).contains(&inputs) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "OEX callback `{name}` accepts {minimum_inputs}..={maximum_inputs} inputs, received {inputs}"
            ),
        ));
    }
    if !(minimum_outputs..=maximum_outputs).contains(&outputs) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "OEX callback `{name}` accepts {minimum_outputs}..={maximum_outputs} outputs, received {outputs}"
            ),
        ));
    }
    Ok(())
}

unsafe fn checked_call<'a>(call: *const OexCall) -> Option<&'a CallContext> {
    let call = unsafe { call.cast::<CallContext>().as_ref()? };
    valid_header(&call.header, OEX_HEADER_CALL).then_some(call)
}

unsafe fn checked_call_mut<'a>(call: *mut OexCall) -> Option<&'a mut CallContext> {
    let call = unsafe { call.cast::<CallContext>().as_mut()? };
    valid_header(&call.header, OEX_HEADER_CALL).then_some(call)
}

unsafe fn checked_value<'a>(value: *const OexValue) -> Option<&'a ValueHandle> {
    let value = unsafe { value.cast::<ValueHandle>().as_ref()? };
    valid_header(&value.header, OEX_HEADER_VALUE).then_some(value)
}

unsafe fn checked_value_mut<'a>(value: *mut OexValue) -> Option<&'a mut ValueHandle> {
    let value = unsafe { value.cast::<ValueHandle>().as_mut()? };
    valid_header(&value.header, OEX_HEADER_VALUE).then_some(value)
}

fn valid_header(header: &OexObjectHeader, kind: u64) -> bool {
    header.bridge_magic == OEX_BRIDGE_MAGIC
        && header.kind == kind
        && ptr::eq(header.dispatch, &raw const HOST_DISPATCH)
}

unsafe extern "C" fn call_get_input_count(call: *const OexCall) -> u32 {
    unsafe { checked_call(call) }
        .and_then(|call| u32::try_from(call.inputs.len()).ok())
        .unwrap_or(0)
}

unsafe extern "C" fn call_get_output_count(call: *const OexCall) -> u32 {
    unsafe { checked_call(call) }
        .and_then(|call| u32::try_from(call.outputs.len()).ok())
        .unwrap_or(0)
}

unsafe extern "C" fn call_borrow_input(call: *const OexCall, index: u32) -> *const OexValue {
    let Some(call) = (unsafe { checked_call(call) }) else {
        return ptr::null();
    };
    call.inputs
        .get(index as usize)
        .and_then(Option::as_deref)
        .map_or(ptr::null(), |value| ptr::from_ref(value).cast::<OexValue>())
}

unsafe extern "C" fn call_take_input(
    call: *mut OexCall,
    index: u32,
    result: *mut *mut OexValue,
) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe { result.write(ptr::null_mut()) };
    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    let Some(slot) = call.inputs.get_mut(index as usize) else {
        return OEX_ERROR_RANGE;
    };
    let Some(mut value) = slot.take() else {
        return OEX_ERROR_STATE;
    };
    value.ownership = VALUE_OWNED;
    unsafe { result.write(Box::into_raw(value).cast::<OexValue>()) };
    OEX_OK
}

unsafe extern "C" fn call_set_output(
    call: *mut OexCall,
    index: u32,
    value: *mut OexValue,
) -> OexStatus {
    let Some(owned) = (unsafe { checked_value(value) }).map(ValueHandle::is_owned) else {
        return OEX_ERROR_ABI;
    };
    if !owned {
        return OEX_ERROR_STATE;
    }
    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    let Some(slot) = call.outputs.get_mut(index as usize) else {
        return OEX_ERROR_RANGE;
    };
    if slot.is_some() {
        return OEX_ERROR_STATE;
    }
    *slot = Some(unsafe { Box::from_raw(value.cast::<ValueHandle>()) });
    OEX_OK
}

unsafe extern "C" fn call_set_error(
    call: *mut OexCall,
    identifier: OexUtf8View,
    message: OexUtf8View,
) -> OexStatus {
    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    let Ok(identifier) = (unsafe { copy_utf8(identifier) }) else {
        return OEX_ERROR_ARGUMENT;
    };
    let Ok(message) = (unsafe { copy_utf8(message) }) else {
        return OEX_ERROR_ARGUMENT;
    };
    call.error = Some(CallError {
        identifier: Some(identifier),
        message,
    });
    OEX_ERROR_PLUGIN
}

unsafe extern "C" fn call_invoke(
    call: *mut OexCall,
    callable: *const OexValue,
    input_count: u32,
    inputs: *const *const OexValue,
    output_count: u32,
    outputs: *mut *mut OexValue,
) -> OexStatus {
    let Ok(input_count) = usize::try_from(input_count) else {
        return OEX_ERROR_RANGE;
    };
    let Ok(output_count) = usize::try_from(output_count) else {
        return OEX_ERROR_RANGE;
    };
    if (input_count != 0 && inputs.is_null()) || (output_count != 0 && outputs.is_null()) {
        return OEX_ERROR_ARGUMENT;
    }
    let output_slots = if output_count == 0 {
        &mut []
    } else {
        unsafe { slice::from_raw_parts_mut(outputs, output_count) }
    };
    output_slots.fill(ptr::null_mut());

    let Some(callable) = (unsafe { checked_value(callable) }) else {
        return OEX_ERROR_ABI;
    };
    let callable = callable.value.clone();
    let input_pointers = if input_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(inputs, input_count) }
    };
    let mut arguments = Vec::new();
    if arguments.try_reserve_exact(input_count).is_err() {
        return OEX_ERROR_ALLOCATION;
    }
    for input in input_pointers {
        let Some(input) = (unsafe { checked_value(*input) }) else {
            return OEX_ERROR_ABI;
        };
        arguments.push(input.value.clone());
    }

    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    let Some(context) = (unsafe { call.context.cast::<BuiltinContext<'_>>().as_mut() }) else {
        return OEX_ERROR_ABI;
    };
    let values = match context.invoke_owned(&callable, arguments, output_count) {
        Ok(values) => values,
        Err(error) => {
            let status = if error.category == BuiltinErrorCategory::Cancelled {
                OEX_ERROR_CANCELLED
            } else {
                OEX_ERROR_CALLBACK
            };
            call.error = Some(CallError {
                identifier: error.identifier,
                message: error.message,
            });
            return status;
        }
    };
    if values.len() != output_count {
        call.error = Some(CallError {
            identifier: Some("OpenMat:OEX:CallbackOutputs".to_owned()),
            message: format!(
                "language callback returned {} values for {output_count} requested outputs",
                values.len()
            ),
        });
        return OEX_ERROR_CALLBACK;
    }
    for (slot, value) in output_slots.iter_mut().zip(values) {
        *slot = Box::into_raw(Box::new(ValueHandle::owned(value))).cast::<OexValue>();
    }
    OEX_OK
}

unsafe extern "C" fn call_get_cancellation(call: *const OexCall) -> *const OexCancellation {
    let Some(call) = (unsafe { checked_call(call) }) else {
        return ptr::null();
    };
    ptr::from_ref(&call.cancellation).cast::<OexCancellation>()
}

unsafe extern "C" fn cancellation_is_requested(cancellation: *const OexCancellation) -> u32 {
    let Some(cancellation) = (unsafe { cancellation.cast::<CancellationHandle>().as_ref() }) else {
        return 0;
    };
    if !valid_header(&cancellation.header, OEX_HEADER_CANCELLATION) || cancellation.flag.is_null() {
        return 0;
    }
    u32::from(unsafe { &*cancellation.flag }.load(Ordering::Relaxed))
}

unsafe extern "C" fn value_get_info(value: *const OexValue, info: *mut OexValueInfo) -> OexStatus {
    if info.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    let Some(value) = (unsafe { checked_value(value) }) else {
        return OEX_ERROR_ABI;
    };
    let dimensions = value.value.dimensions();
    let result = OexValueInfo {
        struct_size: abi_size::<OexValueInfo>(),
        kind: value_kind_to_oex(value.value.kind()),
        data_type: value.value.dtype().map_or(OEX_DATA_NONE, dtype_to_oex),
        flags: u32::from(value.value.is_complex_numeric()) * OEX_VALUE_FLAG_COMPLEX,
        rank: dimensions
            .and_then(|d| u32::try_from(d.len()).ok())
            .unwrap_or(0),
        element_size: value
            .value
            .dtype()
            .and_then(|d| u32::try_from(d.element_width_bytes()).ok())
            .unwrap_or(0),
        element_count: value.value.numel().unwrap_or(0),
    };
    unsafe { info.write(result) };
    OEX_OK
}

unsafe extern "C" fn value_retain(value: *const OexValue, result: *mut *mut OexValue) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    let Some(value) = (unsafe { checked_value(value) }) else {
        return OEX_ERROR_ABI;
    };
    let retained = Box::new(ValueHandle::owned(value.value.clone()));
    unsafe { result.write(Box::into_raw(retained).cast::<OexValue>()) };
    OEX_OK
}

unsafe extern "C" fn value_release(value: *mut OexValue) {
    let Some(owned) = (unsafe { checked_value(value) }).map(ValueHandle::is_owned) else {
        return;
    };
    if owned {
        unsafe { drop(Box::from_raw(value.cast::<ValueHandle>())) };
    }
}

unsafe extern "C" fn value_get_f64(value: *const OexValue, result: *mut f64) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    let Some(value) = (unsafe { checked_value(value) }) else {
        return OEX_ERROR_ABI;
    };
    let scalar = match &value.value {
        Value::Double(value) => *value,
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => array.as_slice()[0],
        _ => return OEX_ERROR_TYPE,
    };
    unsafe { result.write(scalar) };
    OEX_OK
}

unsafe extern "C" fn value_create_f64(
    call: *mut OexCall,
    value: f64,
    result: *mut *mut OexValue,
) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    if (unsafe { checked_call_mut(call) }).is_none() {
        return OEX_ERROR_ABI;
    }
    let value = Box::new(ValueHandle::owned(Value::Double(value)));
    unsafe { result.write(Box::into_raw(value).cast::<OexValue>()) };
    OEX_OK
}

unsafe extern "C" fn dense_get_view(value: *const OexValue, view: *mut OexDenseView) -> OexStatus {
    if view.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    let Some(value) = (unsafe { checked_value(value) }) else {
        return OEX_ERROR_ABI;
    };
    let Some(result) = dense_view(&value.value) else {
        return OEX_ERROR_TYPE;
    };
    unsafe { view.write(result) };
    OEX_OK
}

unsafe extern "C" fn dense_get_mutable_view(
    value: *mut OexValue,
    view: *mut OexMutableDenseView,
) -> OexStatus {
    if view.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    let Some(value) = (unsafe { checked_value_mut(value) }) else {
        return OEX_ERROR_ABI;
    };
    if !value.is_owned() {
        return OEX_ERROR_STATE;
    }
    let Some(result) = mutable_dense_view(&mut value.value) else {
        return OEX_ERROR_TYPE;
    };
    unsafe { view.write(result) };
    OEX_OK
}

unsafe extern "C" fn dense_builder_create_uninitialized(
    call: *mut OexCall,
    data_type: u32,
    rank: u32,
    dimensions: *const u64,
    result: *mut *mut OexDenseBuilder,
    view: *mut OexMutableDenseView,
) -> OexStatus {
    if result.is_null() || view.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe {
        result.write(ptr::null_mut());
        view.write(OexMutableDenseView {
            struct_size: abi_size::<OexMutableDenseView>(),
            data_type: OEX_DATA_NONE,
            rank: 0,
            element_size: 0,
            element_count: 0,
            dimensions: ptr::null(),
            data: ptr::null_mut(),
        });
    }
    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    if rank < 2 || dimensions.is_null() {
        return OEX_ERROR_DIMENSION;
    }
    let Ok(rank_usize) = usize::try_from(rank) else {
        return OEX_ERROR_RANGE;
    };
    let dimensions = unsafe { slice::from_raw_parts(dimensions, rank_usize) };
    let Ok(shape) = Shape::new(dimensions.iter().copied()) else {
        return OEX_ERROR_DIMENSION;
    };
    if shape.ndims() != rank_usize {
        return OEX_ERROR_DIMENSION;
    }
    let Ok(length) = usize::try_from(shape.numel()) else {
        return OEX_ERROR_RANGE;
    };
    let storage = match DenseBuilderStorage::allocate(data_type, length) {
        Ok(storage) => storage,
        Err(status) => return status,
    };
    let mut builder = Box::new(DenseBuilderHandle {
        header: object_header(OEX_HEADER_DENSE_BUILDER),
        call: ptr::from_mut(call),
        shape,
        storage: Some(storage),
    });
    let (actual_type, element_size, data) = builder
        .storage
        .as_mut()
        .expect("new builder owns storage")
        .view_parts();
    let dense_view = OexMutableDenseView {
        struct_size: abi_size::<OexMutableDenseView>(),
        data_type: actual_type,
        rank,
        element_size,
        element_count: builder.shape.numel(),
        dimensions: builder.shape.dimensions().as_ptr(),
        data,
    };
    let pointer = Box::into_raw(builder);
    call.builders.insert(pointer as usize, drop_dense_builder);
    unsafe {
        result.write(pointer.cast::<OexDenseBuilder>());
        view.write(dense_view);
    }
    OEX_OK
}

unsafe extern "C" fn dense_builder_commit(
    builder: *mut OexDenseBuilder,
    result: *mut *mut OexValue,
) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe { result.write(ptr::null_mut()) };
    let Some(builder_ref) = (unsafe { builder.cast::<DenseBuilderHandle>().as_ref() }) else {
        return OEX_ERROR_ABI;
    };
    if !valid_header(&builder_ref.header, OEX_HEADER_DENSE_BUILDER) || builder_ref.call.is_null() {
        return OEX_ERROR_ABI;
    }
    let call = unsafe { &mut *builder_ref.call };
    if !valid_header(&call.header, OEX_HEADER_CALL)
        || call
            .builders
            .remove(&(builder.cast::<DenseBuilderHandle>() as usize))
            .is_none()
    {
        return OEX_ERROR_STATE;
    }
    let mut builder = unsafe { Box::from_raw(builder.cast::<DenseBuilderHandle>()) };
    let Some(storage) = builder.storage.take() else {
        return OEX_ERROR_STATE;
    };
    let value = match storage.into_value(builder.shape.clone()) {
        Ok(value) => value,
        Err(status) => return status,
    };
    let value = Box::new(ValueHandle::owned(value));
    unsafe { result.write(Box::into_raw(value).cast::<OexValue>()) };
    OEX_OK
}

unsafe extern "C" fn dense_builder_abort(builder: *mut OexDenseBuilder) {
    let Some(builder_ref) = (unsafe { builder.cast::<DenseBuilderHandle>().as_ref() }) else {
        return;
    };
    if !valid_header(&builder_ref.header, OEX_HEADER_DENSE_BUILDER) || builder_ref.call.is_null() {
        return;
    }
    let call = unsafe { &mut *builder_ref.call };
    if valid_header(&call.header, OEX_HEADER_CALL)
        && call
            .builders
            .remove(&(builder.cast::<DenseBuilderHandle>() as usize))
            .is_some()
    {
        unsafe { drop(Box::from_raw(builder.cast::<DenseBuilderHandle>())) };
    }
}

unsafe extern "C" fn sparse_get_csc_view(
    value: *const OexValue,
    view: *mut OexSparseCscView,
) -> OexStatus {
    if view.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    let Some(value) = (unsafe { checked_value(value) }) else {
        return OEX_ERROR_ABI;
    };
    let Value::Sparse(sparse) = &value.value else {
        return OEX_ERROR_TYPE;
    };
    let result = match sparse {
        SparseArrayData::Logical(matrix) => sparse_view(matrix, OEX_DATA_LOGICAL),
        SparseArrayData::F64(matrix) => sparse_view(matrix, OEX_DATA_F64),
        SparseArrayData::ComplexF64(matrix) => sparse_view(matrix, OEX_DATA_COMPLEX_F64),
    };
    unsafe { view.write(result) };
    OEX_OK
}

unsafe extern "C" fn sparse_pattern_create(
    call: *mut OexCall,
    rows: u64,
    columns: u64,
    stored_count: u64,
    column_offsets: *const u64,
    row_indices: *const u64,
    result: *mut *mut OexSparsePattern,
) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe { result.write(ptr::null_mut()) };
    if (unsafe { checked_call_mut(call) }).is_none() {
        return OEX_ERROR_ABI;
    }
    if Shape::new([rows, columns]).is_err() {
        return OEX_ERROR_DIMENSION;
    }
    let Ok(columns_length) = usize::try_from(columns) else {
        return OEX_ERROR_RANGE;
    };
    let Some(offset_count) = columns_length.checked_add(1) else {
        return OEX_ERROR_RANGE;
    };
    let Ok(stored_length) = usize::try_from(stored_count) else {
        return OEX_ERROR_RANGE;
    };
    if column_offsets.is_null() || (stored_length != 0 && row_indices.is_null()) {
        return OEX_ERROR_ARGUMENT;
    }
    let source_offsets = unsafe { slice::from_raw_parts(column_offsets, offset_count) };
    let source_rows = if stored_length == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(row_indices, stored_length) }
    };
    if source_offsets.first().copied() != Some(0)
        || source_offsets.last().copied() != Some(stored_count)
    {
        return OEX_ERROR_ARGUMENT;
    }
    for column in 0..columns_length {
        let Ok(start) = usize::try_from(source_offsets[column]) else {
            return OEX_ERROR_RANGE;
        };
        let Ok(end) = usize::try_from(source_offsets[column + 1]) else {
            return OEX_ERROR_RANGE;
        };
        if start > end || end > stored_length {
            return OEX_ERROR_ARGUMENT;
        }
        let mut previous = None;
        for row in &source_rows[start..end] {
            if *row >= rows || previous.is_some_and(|previous| *row <= previous) {
                return OEX_ERROR_ARGUMENT;
            }
            previous = Some(*row);
        }
    }
    let mut offsets = Vec::new();
    if offsets.try_reserve_exact(offset_count).is_err() {
        return OEX_ERROR_ALLOCATION;
    }
    offsets.extend_from_slice(source_offsets);
    let mut indices = Vec::new();
    if indices.try_reserve_exact(stored_length).is_err() {
        return OEX_ERROR_ALLOCATION;
    }
    indices.extend_from_slice(source_rows);
    let pattern = Box::new(SparsePatternHandle {
        header: object_header(OEX_HEADER_SPARSE_PATTERN),
        storage: Arc::new(SparsePatternStorage {
            rows,
            columns,
            column_offsets: offsets,
            row_indices: indices,
        }),
    });
    unsafe { result.write(Box::into_raw(pattern).cast::<OexSparsePattern>()) };
    OEX_OK
}

unsafe extern "C" fn sparse_pattern_retain(
    pattern: *const OexSparsePattern,
    result: *mut *mut OexSparsePattern,
) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe { result.write(ptr::null_mut()) };
    let Some(pattern) = (unsafe { pattern.cast::<SparsePatternHandle>().as_ref() }) else {
        return OEX_ERROR_ABI;
    };
    if !valid_header(&pattern.header, OEX_HEADER_SPARSE_PATTERN) {
        return OEX_ERROR_ABI;
    }
    let retained = Box::new(SparsePatternHandle {
        header: object_header(OEX_HEADER_SPARSE_PATTERN),
        storage: Arc::clone(&pattern.storage),
    });
    unsafe { result.write(Box::into_raw(retained).cast::<OexSparsePattern>()) };
    OEX_OK
}

unsafe extern "C" fn sparse_pattern_release(pattern: *mut OexSparsePattern) {
    let Some(pattern_ref) = (unsafe { pattern.cast::<SparsePatternHandle>().as_ref() }) else {
        return;
    };
    if valid_header(&pattern_ref.header, OEX_HEADER_SPARSE_PATTERN) {
        unsafe { drop(Box::from_raw(pattern.cast::<SparsePatternHandle>())) };
    }
}

unsafe extern "C" fn sparse_triplet_builder_create_uninitialized(
    call: *mut OexCall,
    data_type: u32,
    rows: u64,
    columns: u64,
    capacity: u64,
    result: *mut *mut OexSparseTripletBuilder,
    view: *mut OexSparseTripletView,
) -> OexStatus {
    if result.is_null() || view.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe {
        result.write(ptr::null_mut());
        view.write(empty_triplet_view());
    }
    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    if Shape::new([rows, columns]).is_err() {
        return OEX_ERROR_DIMENSION;
    }
    let Ok(length) = usize::try_from(capacity) else {
        return OEX_ERROR_RANGE;
    };
    let row_indices = match allocate_uninitialized("triplet row indices", length) {
        Ok(values) => values,
        Err(status) => return status,
    };
    let column_indices = match allocate_uninitialized("triplet column indices", length) {
        Ok(values) => values,
        Err(status) => return status,
    };
    let values = match SparseBuilderStorage::allocate(data_type, length) {
        Ok(values) => values,
        Err(status) => return status,
    };
    let mut builder = Box::new(SparseTripletBuilderHandle {
        header: object_header(OEX_HEADER_SPARSE_TRIPLET_BUILDER),
        call: ptr::from_mut(call),
        rows,
        columns,
        capacity: length,
        row_indices,
        column_indices,
        values: Some(values),
    });
    let (actual_type, element_size, values) = builder
        .values
        .as_mut()
        .expect("new triplet builder owns values")
        .view_parts();
    let output_view = OexSparseTripletView {
        struct_size: abi_size::<OexSparseTripletView>(),
        data_type: actual_type,
        element_size,
        reserved: 0,
        rows,
        columns,
        capacity,
        row_indices: builder.row_indices.as_mut_ptr().cast::<u64>(),
        column_indices: builder.column_indices.as_mut_ptr().cast::<u64>(),
        values,
    };
    let pointer = Box::into_raw(builder);
    call.builders
        .insert(pointer as usize, drop_sparse_triplet_builder);
    unsafe {
        result.write(pointer.cast::<OexSparseTripletBuilder>());
        view.write(output_view);
    }
    OEX_OK
}

unsafe extern "C" fn sparse_triplet_builder_commit(
    builder: *mut OexSparseTripletBuilder,
    stored_count: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe { result.write(ptr::null_mut()) };
    let Some(builder_ref) = (unsafe { builder.cast::<SparseTripletBuilderHandle>().as_ref() })
    else {
        return OEX_ERROR_ABI;
    };
    if !valid_header(&builder_ref.header, OEX_HEADER_SPARSE_TRIPLET_BUILDER)
        || builder_ref.call.is_null()
    {
        return OEX_ERROR_ABI;
    }
    let Ok(length) = usize::try_from(stored_count) else {
        return OEX_ERROR_RANGE;
    };
    if length > builder_ref.capacity {
        return OEX_ERROR_RANGE;
    }
    let call = unsafe { &mut *builder_ref.call };
    if !valid_header(&call.header, OEX_HEADER_CALL)
        || call
            .builders
            .remove(&(builder.cast::<SparseTripletBuilderHandle>() as usize))
            .is_none()
    {
        return OEX_ERROR_STATE;
    }
    let mut builder = unsafe { Box::from_raw(builder.cast::<SparseTripletBuilderHandle>()) };
    builder.row_indices.truncate(length);
    builder.column_indices.truncate(length);
    let rows = unsafe { assume_init_vec(std::mem::take(&mut builder.row_indices)) };
    let columns = unsafe { assume_init_vec(std::mem::take(&mut builder.column_indices)) };
    let Some(values) = builder.values.take() else {
        return OEX_ERROR_STATE;
    };
    let values = unsafe { values.into_initialized(length) };
    let cancellation = unsafe { call.cancellation.flag.as_ref() };
    let value = match triplet_sparse_value(
        builder.rows,
        builder.columns,
        builder.capacity,
        rows,
        columns,
        values,
        cancellation,
    ) {
        Ok(value) => value,
        Err(error) => return sparse_error_status(&error),
    };
    let value = Box::new(ValueHandle::owned(value));
    unsafe { result.write(Box::into_raw(value).cast::<OexValue>()) };
    OEX_OK
}

unsafe extern "C" fn sparse_triplet_builder_abort(builder: *mut OexSparseTripletBuilder) {
    let Some(builder_ref) = (unsafe { builder.cast::<SparseTripletBuilderHandle>().as_ref() })
    else {
        return;
    };
    if !valid_header(&builder_ref.header, OEX_HEADER_SPARSE_TRIPLET_BUILDER)
        || builder_ref.call.is_null()
    {
        return;
    }
    let call = unsafe { &mut *builder_ref.call };
    if valid_header(&call.header, OEX_HEADER_CALL)
        && call
            .builders
            .remove(&(builder.cast::<SparseTripletBuilderHandle>() as usize))
            .is_some()
    {
        unsafe { drop(Box::from_raw(builder.cast::<SparseTripletBuilderHandle>())) };
    }
}

unsafe extern "C" fn sparse_pattern_builder_create_uninitialized(
    call: *mut OexCall,
    pattern: *const OexSparsePattern,
    data_type: u32,
    result: *mut *mut OexSparsePatternBuilder,
    view: *mut OexSparsePatternValuesView,
) -> OexStatus {
    if result.is_null() || view.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe {
        result.write(ptr::null_mut());
        view.write(empty_pattern_values_view());
    }
    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    let Some(pattern) = (unsafe { pattern.cast::<SparsePatternHandle>().as_ref() }) else {
        return OEX_ERROR_ABI;
    };
    if !valid_header(&pattern.header, OEX_HEADER_SPARSE_PATTERN) {
        return OEX_ERROR_ABI;
    }
    let length = pattern.storage.row_indices.len();
    let values = match SparseBuilderStorage::allocate(data_type, length) {
        Ok(values) => values,
        Err(status) => return status,
    };
    let mut builder = Box::new(SparsePatternBuilderHandle {
        header: object_header(OEX_HEADER_SPARSE_PATTERN_BUILDER),
        call: ptr::from_mut(call),
        pattern: Arc::clone(&pattern.storage),
        values: Some(values),
    });
    let (actual_type, element_size, values) = builder
        .values
        .as_mut()
        .expect("new pattern builder owns values")
        .view_parts();
    let output_view = OexSparsePatternValuesView {
        struct_size: abi_size::<OexSparsePatternValuesView>(),
        data_type: actual_type,
        element_size,
        reserved: 0,
        stored_count: u64::try_from(length).unwrap_or(u64::MAX),
        values,
    };
    let pointer = Box::into_raw(builder);
    call.builders
        .insert(pointer as usize, drop_sparse_pattern_builder);
    unsafe {
        result.write(pointer.cast::<OexSparsePatternBuilder>());
        view.write(output_view);
    }
    OEX_OK
}

unsafe extern "C" fn sparse_pattern_builder_commit(
    builder: *mut OexSparsePatternBuilder,
    result: *mut *mut OexValue,
) -> OexStatus {
    if result.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe { result.write(ptr::null_mut()) };
    let Some(builder_ref) = (unsafe { builder.cast::<SparsePatternBuilderHandle>().as_ref() })
    else {
        return OEX_ERROR_ABI;
    };
    if !valid_header(&builder_ref.header, OEX_HEADER_SPARSE_PATTERN_BUILDER)
        || builder_ref.call.is_null()
    {
        return OEX_ERROR_ABI;
    }
    let call = unsafe { &mut *builder_ref.call };
    if !valid_header(&call.header, OEX_HEADER_CALL)
        || call
            .builders
            .remove(&(builder.cast::<SparsePatternBuilderHandle>() as usize))
            .is_none()
    {
        return OEX_ERROR_STATE;
    }
    let mut builder = unsafe { Box::from_raw(builder.cast::<SparsePatternBuilderHandle>()) };
    let Some(values) = builder.values.take() else {
        return OEX_ERROR_STATE;
    };
    let values = unsafe { values.into_initialized(builder.pattern.row_indices.len()) };
    let cancellation = unsafe { call.cancellation.flag.as_ref() };
    let value = match pattern_sparse_value(&builder.pattern, values, cancellation) {
        Ok(value) => value,
        Err(error) => return sparse_error_status(&error),
    };
    let value = Box::new(ValueHandle::owned(value));
    unsafe { result.write(Box::into_raw(value).cast::<OexValue>()) };
    OEX_OK
}

unsafe extern "C" fn sparse_pattern_builder_abort(builder: *mut OexSparsePatternBuilder) {
    let Some(builder_ref) = (unsafe { builder.cast::<SparsePatternBuilderHandle>().as_ref() })
    else {
        return;
    };
    if !valid_header(&builder_ref.header, OEX_HEADER_SPARSE_PATTERN_BUILDER)
        || builder_ref.call.is_null()
    {
        return;
    }
    let call = unsafe { &mut *builder_ref.call };
    if valid_header(&call.header, OEX_HEADER_CALL)
        && call
            .builders
            .remove(&(builder.cast::<SparsePatternBuilderHandle>() as usize))
            .is_some()
    {
        unsafe { drop(Box::from_raw(builder.cast::<SparsePatternBuilderHandle>())) };
    }
}

unsafe extern "C" fn native_object_borrow_instance(
    call: *mut OexCall,
    value: *const OexValue,
    expected_type_token: *const c_void,
    instance: *mut *mut c_void,
) -> OexStatus {
    if instance.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe { instance.write(ptr::null_mut()) };
    let Some(token) = native_class_token(expected_type_token) else {
        return OEX_ERROR_ARGUMENT;
    };
    let Some(value) = (unsafe { checked_value(value) }).map(|handle| handle.value.clone()) else {
        return OEX_ERROR_ABI;
    };
    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    let Some(context) = (unsafe { call.context.cast::<BuiltinContext<'_>>().as_mut() }) else {
        return OEX_ERROR_ABI;
    };
    match context.borrow_native_instance(&value, token) {
        Ok(native) => {
            unsafe {
                instance.write(ptr::without_provenance_mut(native.identifier().get()));
            }
            OEX_OK
        }
        Err(error) => native_object_error_status(&error),
    }
}

unsafe extern "C" fn native_object_create(
    call: *mut OexCall,
    type_token: *const c_void,
    instance: *mut c_void,
    value: *mut *mut OexValue,
) -> OexStatus {
    if value.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe { value.write(ptr::null_mut()) };
    let Some(token) = native_class_token(type_token) else {
        return OEX_ERROR_ARGUMENT;
    };
    let Some(identifier) = NonZeroUsize::new(instance.addr()) else {
        return OEX_ERROR_ARGUMENT;
    };
    let Some(call) = (unsafe { checked_call_mut(call) }) else {
        return OEX_ERROR_ABI;
    };
    let Some(context) = (unsafe { call.context.cast::<BuiltinContext<'_>>().as_mut() }) else {
        return OEX_ERROR_ABI;
    };
    let created = match context.create_native_instance(token, NativeInstance::new(identifier)) {
        Ok(created) => created,
        Err(error) => return native_object_error_status(&error),
    };
    let created = Box::new(ValueHandle::owned(created));
    unsafe { value.write(Box::into_raw(created).cast::<OexValue>()) };
    OEX_OK
}

fn native_class_token(token: *const c_void) -> Option<NativeClassToken> {
    NonZeroUsize::new(token.addr()).map(NativeClassToken::new)
}

const fn native_object_error_status(error: &BuiltinError) -> OexStatus {
    match error.category {
        BuiltinErrorCategory::Type => OEX_ERROR_TYPE,
        BuiltinErrorCategory::Domain | BuiltinErrorCategory::HostService => OEX_ERROR_STATE,
        BuiltinErrorCategory::Cancelled => OEX_ERROR_CANCELLED,
        BuiltinErrorCategory::ArgumentCount => OEX_ERROR_ARGUMENT,
        BuiltinErrorCategory::Other | BuiltinErrorCategory::LanguageCopy => OEX_ERROR_ALLOCATION,
        BuiltinErrorCategory::Output
        | BuiltinErrorCategory::FileSystem
        | BuiltinErrorCategory::Graphics => OEX_ERROR_PLUGIN,
    }
}

const fn empty_triplet_view() -> OexSparseTripletView {
    OexSparseTripletView {
        struct_size: abi_size::<OexSparseTripletView>(),
        data_type: OEX_DATA_NONE,
        element_size: 0,
        reserved: 0,
        rows: 0,
        columns: 0,
        capacity: 0,
        row_indices: ptr::null_mut(),
        column_indices: ptr::null_mut(),
        values: ptr::null_mut(),
    }
}

const fn empty_pattern_values_view() -> OexSparsePatternValuesView {
    OexSparsePatternValuesView {
        struct_size: abi_size::<OexSparsePatternValuesView>(),
        data_type: OEX_DATA_NONE,
        element_size: 0,
        reserved: 0,
        stored_count: 0,
        values: ptr::null_mut(),
    }
}

fn triplet_sparse_value(
    rows: u64,
    columns: u64,
    reserved_nnz: usize,
    row_indices: Vec<u64>,
    column_indices: Vec<u64>,
    values: InitializedSparseStorage,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, SparseError> {
    macro_rules! build {
        ($values:expr, $wrap:expr) => {{
            let values = $values;
            let mut entries = Vec::new();
            entries
                .try_reserve_exact(values.len())
                .map_err(|_| SparseError::Allocation {
                    storage: "OEX COO entries",
                    elements: values.len(),
                })?;
            for ((row, column), value) in row_indices.into_iter().zip(column_indices).zip(values) {
                entries.push(CooEntry::new(row, column, value));
            }
            CscMatrix::try_from_coo(rows, columns, entries, reserved_nnz, cancellation)
                .map($wrap)
                .map(Value::Sparse)
        }};
    }
    match values {
        InitializedSparseStorage::Logical(values) => {
            build!(values, SparseArrayData::Logical)
        }
        InitializedSparseStorage::F64(values) => build!(values, SparseArrayData::F64),
        InitializedSparseStorage::ComplexF64(values) => {
            build!(values, SparseArrayData::ComplexF64)
        }
    }
}

fn pattern_sparse_value(
    pattern: &SparsePatternStorage,
    values: InitializedSparseStorage,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, SparseError> {
    macro_rules! build {
        ($values:expr, $is_zero:expr, $wrap:expr) => {{
            let values = $values;
            let mut offsets = Vec::new();
            offsets
                .try_reserve_exact(pattern.column_offsets.len())
                .map_err(|_| SparseError::Allocation {
                    storage: "OEX pattern column offsets",
                    elements: pattern.column_offsets.len(),
                })?;
            let mut rows = Vec::new();
            rows.try_reserve_exact(pattern.row_indices.len())
                .map_err(|_| SparseError::Allocation {
                    storage: "OEX pattern row indices",
                    elements: pattern.row_indices.len(),
                })?;
            let mut stored = Vec::new();
            stored
                .try_reserve_exact(values.len())
                .map_err(|_| SparseError::Allocation {
                    storage: "OEX pattern values",
                    elements: values.len(),
                })?;
            offsets.push(0);
            let columns = usize::try_from(pattern.columns)
                .expect("validated sparse pattern column count fits the host");
            for column in 0..columns {
                let start = usize::try_from(pattern.column_offsets[column])
                    .expect("validated sparse pattern offset fits the host");
                let end = usize::try_from(pattern.column_offsets[column + 1])
                    .expect("validated sparse pattern offset fits the host");
                for index in start..end {
                    let value = values[index];
                    if !$is_zero(value) {
                        rows.push(pattern.row_indices[index]);
                        stored.push(value);
                    }
                }
                offsets.push(
                    u64::try_from(stored.len())
                        .expect("validated sparse pattern length fits the ABI"),
                );
            }
            CscMatrix::try_from_canonical_parts(
                pattern.rows,
                pattern.columns,
                offsets,
                rows,
                stored,
                pattern.row_indices.len(),
                cancellation,
            )
            .map($wrap)
            .map(Value::Sparse)
        }};
    }
    match values {
        InitializedSparseStorage::Logical(values) => build!(
            values,
            |value: Logical| !value.get(),
            SparseArrayData::Logical
        ),
        InitializedSparseStorage::F64(values) => {
            build!(values, |value: f64| value == 0.0, SparseArrayData::F64)
        }
        InitializedSparseStorage::ComplexF64(values) => build!(
            values,
            |value: ArrayComplex64| value.re == 0.0 && value.im == 0.0,
            SparseArrayData::ComplexF64
        ),
    }
}

const fn sparse_error_status(error: &SparseError) -> OexStatus {
    match error {
        SparseError::Cancelled => OEX_ERROR_CANCELLED,
        SparseError::Allocation { .. } => OEX_ERROR_ALLOCATION,
        SparseError::ShapeOverflow { .. } => OEX_ERROR_DIMENSION,
        SparseError::HostLengthOverflow { .. }
        | SparseError::CoordinateOutOfBounds { .. }
        | SparseError::OffsetOverflow => OEX_ERROR_RANGE,
        _ => OEX_ERROR_ARGUMENT,
    }
}

fn dense_view(value: &Value) -> Option<OexDenseView> {
    match value {
        Value::Double(value) => Some(scalar_dense_view(value, OEX_DATA_F64)),
        Value::Complex(value) => Some(scalar_dense_view(value, OEX_DATA_COMPLEX_F64)),
        Value::Array(ArrayData::F32(array)) => Some(array_dense_view(array, OEX_DATA_F32)),
        Value::Array(ArrayData::ComplexF32(array)) => {
            Some(array_dense_view(array, OEX_DATA_COMPLEX_F32))
        }
        Value::Array(ArrayData::F64(array)) => Some(array_dense_view(array, OEX_DATA_F64)),
        Value::Array(ArrayData::ComplexF64(array)) => {
            Some(array_dense_view(array, OEX_DATA_COMPLEX_F64))
        }
        Value::Array(ArrayData::Logical(array)) => Some(array_dense_view(array, OEX_DATA_LOGICAL)),
        Value::Array(ArrayData::Char(array)) => Some(array_dense_view(array, OEX_DATA_CHAR16)),
        Value::Array(ArrayData::Integer(integer)) => Some(integer_dense_view(integer)),
        _ => None,
    }
}

fn mutable_dense_view(value: &mut Value) -> Option<OexMutableDenseView> {
    match value {
        Value::Double(value) => Some(scalar_mutable_dense_view(value, OEX_DATA_F64)),
        Value::Complex(value) => Some(scalar_mutable_dense_view(value, OEX_DATA_COMPLEX_F64)),
        Value::Array(ArrayData::F32(array)) => Some(array_mutable_dense_view(array, OEX_DATA_F32)),
        Value::Array(ArrayData::ComplexF32(array)) => {
            Some(array_mutable_dense_view(array, OEX_DATA_COMPLEX_F32))
        }
        Value::Array(ArrayData::F64(array)) => Some(array_mutable_dense_view(array, OEX_DATA_F64)),
        Value::Array(ArrayData::ComplexF64(array)) => {
            Some(array_mutable_dense_view(array, OEX_DATA_COMPLEX_F64))
        }
        Value::Array(ArrayData::Logical(array)) => {
            Some(array_mutable_dense_view(array, OEX_DATA_LOGICAL))
        }
        Value::Array(ArrayData::Char(array)) => {
            Some(array_mutable_dense_view(array, OEX_DATA_CHAR16))
        }
        Value::Array(ArrayData::Integer(integer)) => Some(integer_mutable_dense_view(integer)),
        _ => None,
    }
}

fn array_dense_view<T>(array: &DenseArray<T>, data_type: u32) -> OexDenseView {
    OexDenseView {
        struct_size: abi_size::<OexDenseView>(),
        data_type,
        rank: u32::try_from(array.shape().ndims()).unwrap_or(u32::MAX),
        element_size: abi_size::<T>(),
        element_count: array.numel(),
        dimensions: array.shape().dimensions().as_ptr(),
        data: array.as_slice().as_ptr().cast::<c_void>(),
    }
}

fn scalar_dense_view<T>(value: &T, data_type: u32) -> OexDenseView {
    OexDenseView {
        struct_size: abi_size::<OexDenseView>(),
        data_type,
        rank: 2,
        element_size: abi_size::<T>(),
        element_count: 1,
        dimensions: SCALAR_DIMENSIONS.as_ptr(),
        data: ptr::from_ref(value).cast::<c_void>(),
    }
}

fn array_mutable_dense_view<T: Clone>(
    array: &mut DenseArray<T>,
    data_type: u32,
) -> OexMutableDenseView {
    let rank = u32::try_from(array.shape().ndims()).unwrap_or(u32::MAX);
    let element_count = array.numel();
    let dimensions = array.shape().dimensions().as_ptr();
    let data = array.as_mut_slice().as_mut_ptr().cast::<c_void>();
    OexMutableDenseView {
        struct_size: abi_size::<OexMutableDenseView>(),
        data_type,
        rank,
        element_size: abi_size::<T>(),
        element_count,
        dimensions,
        data,
    }
}

fn scalar_mutable_dense_view<T>(value: &mut T, data_type: u32) -> OexMutableDenseView {
    OexMutableDenseView {
        struct_size: abi_size::<OexMutableDenseView>(),
        data_type,
        rank: 2,
        element_size: abi_size::<T>(),
        element_count: 1,
        dimensions: SCALAR_DIMENSIONS.as_ptr(),
        data: ptr::from_mut(value).cast::<c_void>(),
    }
}

macro_rules! integer_views {
    ($integer:expr, $view:ident) => {
        match $integer {
            IntegerArrayData::I8(a) => $view(a, OEX_DATA_I8),
            IntegerArrayData::U8(a) => $view(a, OEX_DATA_U8),
            IntegerArrayData::I16(a) => $view(a, OEX_DATA_I16),
            IntegerArrayData::U16(a) => $view(a, OEX_DATA_U16),
            IntegerArrayData::I32(a) => $view(a, OEX_DATA_I32),
            IntegerArrayData::U32(a) => $view(a, OEX_DATA_U32),
            IntegerArrayData::I64(a) => $view(a, OEX_DATA_I64),
            IntegerArrayData::U64(a) => $view(a, OEX_DATA_U64),
            IntegerArrayData::ComplexI8(a) => $view(a, OEX_DATA_COMPLEX_I8),
            IntegerArrayData::ComplexU8(a) => $view(a, OEX_DATA_COMPLEX_U8),
            IntegerArrayData::ComplexI16(a) => $view(a, OEX_DATA_COMPLEX_I16),
            IntegerArrayData::ComplexU16(a) => $view(a, OEX_DATA_COMPLEX_U16),
            IntegerArrayData::ComplexI32(a) => $view(a, OEX_DATA_COMPLEX_I32),
            IntegerArrayData::ComplexU32(a) => $view(a, OEX_DATA_COMPLEX_U32),
            IntegerArrayData::ComplexI64(a) => $view(a, OEX_DATA_COMPLEX_I64),
            IntegerArrayData::ComplexU64(a) => $view(a, OEX_DATA_COMPLEX_U64),
        }
    };
}

fn integer_dense_view(integer: &IntegerArrayData) -> OexDenseView {
    integer_views!(integer, array_dense_view)
}
fn integer_mutable_dense_view(integer: &mut IntegerArrayData) -> OexMutableDenseView {
    integer_views!(integer, array_mutable_dense_view)
}

fn sparse_view<T>(matrix: &openmat_value::CscMatrix<T>, data_type: u32) -> OexSparseCscView {
    OexSparseCscView {
        struct_size: abi_size::<OexSparseCscView>(),
        data_type,
        element_size: abi_size::<T>(),
        reserved: 0,
        rows: matrix.rows(),
        columns: matrix.columns(),
        stored_count: matrix.nnz() as u64,
        column_offsets: matrix.col_offsets().as_ptr(),
        row_indices: matrix.row_indices().as_ptr(),
        values: matrix.values().as_ptr().cast::<c_void>(),
    }
}

const fn dtype_to_oex(dtype: DType) -> u32 {
    match dtype {
        DType::Logical => OEX_DATA_LOGICAL,
        DType::Char => OEX_DATA_CHAR16,
        DType::I8 => OEX_DATA_I8,
        DType::U8 => OEX_DATA_U8,
        DType::I16 => OEX_DATA_I16,
        DType::U16 => OEX_DATA_U16,
        DType::I32 => OEX_DATA_I32,
        DType::U32 => OEX_DATA_U32,
        DType::I64 => OEX_DATA_I64,
        DType::U64 => OEX_DATA_U64,
        DType::F32 => OEX_DATA_F32,
        DType::F64 => OEX_DATA_F64,
        DType::ComplexF32 => OEX_DATA_COMPLEX_F32,
        DType::ComplexF64 => OEX_DATA_COMPLEX_F64,
        DType::ComplexI8 => OEX_DATA_COMPLEX_I8,
        DType::ComplexU8 => OEX_DATA_COMPLEX_U8,
        DType::ComplexI16 => OEX_DATA_COMPLEX_I16,
        DType::ComplexU16 => OEX_DATA_COMPLEX_U16,
        DType::ComplexI32 => OEX_DATA_COMPLEX_I32,
        DType::ComplexU32 => OEX_DATA_COMPLEX_U32,
        DType::ComplexI64 => OEX_DATA_COMPLEX_I64,
        DType::ComplexU64 => OEX_DATA_COMPLEX_U64,
    }
}

const fn value_kind_to_oex(kind: ValueKind) -> u32 {
    match kind {
        ValueKind::Nothing => OEX_VALUE_NOTHING,
        ValueKind::Logical
        | ValueKind::Double
        | ValueKind::Complex
        | ValueKind::Single
        | ValueKind::Char
        | ValueKind::Integer
        | ValueKind::Array => OEX_VALUE_DENSE,
        ValueKind::Sparse => OEX_VALUE_SPARSE,
        ValueKind::String => OEX_VALUE_STRING,
        ValueKind::Cell => OEX_VALUE_CELL,
        ValueKind::Struct => OEX_VALUE_STRUCT,
        ValueKind::Table => OEX_VALUE_TABLE,
        ValueKind::Object => OEX_VALUE_OBJECT,
        ValueKind::Graphics => OEX_VALUE_GRAPHICS,
        ValueKind::Function => OEX_VALUE_FUNCTION,
    }
}

unsafe fn copy_utf8(view: OexUtf8View) -> Result<String, ()> {
    let length = usize::try_from(view.length).map_err(|_| ())?;
    if length == 0 {
        return Ok(String::new());
    }
    if view.data.is_null() {
        return Err(());
    }
    let bytes = unsafe { slice::from_raw_parts(view.data.cast::<u8>(), length) };
    str::from_utf8(bytes).map(ToOwned::to_owned).map_err(|_| ())
}

fn call_error(status: OexStatus, error: Option<CallError>, function: &str) -> BuiltinError {
    if let Some(error) = error {
        let builtin = BuiltinError::new(category_for_status(status), error.message);
        return if let Some(identifier) = error.identifier {
            builtin.with_identifier(identifier)
        } else {
            builtin
        };
    }
    BuiltinError::new(
        category_for_status(status),
        format!("OEX function `{function}` failed with status {status}"),
    )
}

const fn category_for_status(status: OexStatus) -> BuiltinErrorCategory {
    match status {
        OEX_ERROR_TYPE | crate::abi::OEX_ERROR_ENCODING => BuiltinErrorCategory::Type,
        OEX_ERROR_ARGUMENT
        | OEX_ERROR_RANGE
        | OEX_ERROR_DIMENSION
        | crate::abi::OEX_ERROR_NOT_FOUND => BuiltinErrorCategory::Domain,
        OEX_ERROR_CANCELLED => BuiltinErrorCategory::Cancelled,
        _ => BuiltinErrorCategory::Other,
    }
}

#[allow(dead_code)]
const _: OexStatus = OEX_ERROR_UNSUPPORTED;

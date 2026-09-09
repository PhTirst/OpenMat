//! Raw declarations for `include/openmat/oex.h`, ABI 1.3.
//!
//! No host/runtime implementation is linked into this crate. All pointer validity,
//! ownership, initialization and callback lifetime obligations are those of oex.h.
//! On Windows, Rust generates the import library for openmat_oex.dll itself.
#![no_std]
#![allow(non_snake_case)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("OEX ABI 1.x requires a 64-bit target");

use core::ffi::{c_char, c_void};
use core::mem::size_of;

#[allow(clippy::cast_possible_truncation)]
pub const fn abi_size<T>() -> u32 {
    // Every OEX ABI structure is fixed and far below UINT32_MAX. Keeping the one
    // audited cast here avoids scattering unchecked layout conversions.
    size_of::<T>() as u32
}

pub const OEX_ABI_VERSION: u32 = 0x0001_0003;
pub const OEX_ABI_MAJOR: u32 = 1;
pub const OEX_ABI_MINOR: u32 = 3;

pub type OexStatus = u32;

pub const OEX_OK: OexStatus = 0;
pub const OEX_ERROR_ARGUMENT: OexStatus = 1;
pub const OEX_ERROR_TYPE: OexStatus = 2;
pub const OEX_ERROR_DIMENSION: OexStatus = 3;
pub const OEX_ERROR_RANGE: OexStatus = 4;
pub const OEX_ERROR_ALLOCATION: OexStatus = 5;
pub const OEX_ERROR_CANCELLED: OexStatus = 6;
pub const OEX_ERROR_UNSUPPORTED: OexStatus = 7;
pub const OEX_ERROR_PLUGIN: OexStatus = 8;
pub const OEX_ERROR_ABI: OexStatus = 9;
pub const OEX_ERROR_STATE: OexStatus = 10;
pub const OEX_ERROR_CALLBACK: OexStatus = 11;
pub const OEX_ERROR_ENCODING: OexStatus = 12;
pub const OEX_ERROR_NOT_FOUND: OexStatus = 13;

pub const OEX_VALUE_NOTHING: u32 = 0;
pub const OEX_VALUE_DENSE: u32 = 1;
pub const OEX_VALUE_SPARSE: u32 = 2;
pub const OEX_VALUE_STRING: u32 = 3;
pub const OEX_VALUE_CELL: u32 = 4;
pub const OEX_VALUE_STRUCT: u32 = 5;
pub const OEX_VALUE_TABLE: u32 = 6;
pub const OEX_VALUE_OBJECT: u32 = 7;
pub const OEX_VALUE_FUNCTION: u32 = 8;
pub const OEX_VALUE_GRAPHICS: u32 = 9;

pub const OEX_DATA_NONE: u32 = 0;
pub const OEX_DATA_LOGICAL: u32 = 1;
pub const OEX_DATA_CHAR16: u32 = 2;
pub const OEX_DATA_I8: u32 = 3;
pub const OEX_DATA_U8: u32 = 4;
pub const OEX_DATA_I16: u32 = 5;
pub const OEX_DATA_U16: u32 = 6;
pub const OEX_DATA_I32: u32 = 7;
pub const OEX_DATA_U32: u32 = 8;
pub const OEX_DATA_I64: u32 = 9;
pub const OEX_DATA_U64: u32 = 10;
pub const OEX_DATA_F32: u32 = 11;
pub const OEX_DATA_F64: u32 = 12;
pub const OEX_DATA_COMPLEX_F32: u32 = 13;
pub const OEX_DATA_COMPLEX_F64: u32 = 14;
pub const OEX_DATA_COMPLEX_I8: u32 = 15;
pub const OEX_DATA_COMPLEX_U8: u32 = 16;
pub const OEX_DATA_COMPLEX_I16: u32 = 17;
pub const OEX_DATA_COMPLEX_U16: u32 = 18;
pub const OEX_DATA_COMPLEX_I32: u32 = 19;
pub const OEX_DATA_COMPLEX_U32: u32 = 20;
pub const OEX_DATA_COMPLEX_I64: u32 = 21;
pub const OEX_DATA_COMPLEX_U64: u32 = 22;

pub const OEX_VALUE_FLAG_COMPLEX: u32 = 1;
pub const OEX_CLASS_HANDLE: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexUtf16View {
    pub data: *const u16,
    pub length: u64,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexStringElementView {
    pub struct_size: u32,
    pub is_missing: u32,
    pub data: *const u16,
    pub length: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexUtf8View {
    pub data: *const c_char,
    pub length: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OexValueInfo {
    pub struct_size: u32,
    pub kind: u32,
    pub data_type: u32,
    pub flags: u32,
    pub rank: u32,
    pub element_size: u32,
    pub element_count: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexDenseView {
    pub struct_size: u32,
    pub data_type: u32,
    pub rank: u32,
    pub element_size: u32,
    pub element_count: u64,
    pub dimensions: *const u64,
    pub data: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexMutableDenseView {
    pub struct_size: u32,
    pub data_type: u32,
    pub rank: u32,
    pub element_size: u32,
    pub element_count: u64,
    pub dimensions: *const u64,
    pub data: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexSparseCscView {
    pub struct_size: u32,
    pub data_type: u32,
    pub element_size: u32,
    pub reserved: u32,
    pub rows: u64,
    pub columns: u64,
    pub stored_count: u64,
    pub column_offsets: *const u64,
    pub row_indices: *const u64,
    pub values: *const c_void,
}

macro_rules! complex_type {
    ($name:ident, $component:ty) => {
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub struct $name {
            pub re: $component,
            pub im: $component,
        }
    };
}

complex_type!(OexComplexF32, f32);
complex_type!(OexComplexF64, f64);
complex_type!(OexComplexI8, i8);
complex_type!(OexComplexU8, u8);
complex_type!(OexComplexI16, i16);
complex_type!(OexComplexU16, u16);
complex_type!(OexComplexI32, i32);
complex_type!(OexComplexU32, u32);
complex_type!(OexComplexI64, i64);
complex_type!(OexComplexU64, u64);

#[repr(C)]
pub struct OexCall {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OexValue {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OexCancellation {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OexDenseBuilder {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OexSparsePattern {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OexSparseTripletBuilder {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OexSparsePatternBuilder {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexSparseTripletView {
    pub struct_size: u32,
    pub data_type: u32,
    pub element_size: u32,
    pub reserved: u32,
    pub rows: u64,
    pub columns: u64,
    pub capacity: u64,
    pub row_indices: *mut u64,
    pub column_indices: *mut u64,
    pub values: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexSparsePatternValuesView {
    pub struct_size: u32,
    pub data_type: u32,
    pub element_size: u32,
    pub reserved: u32,
    pub stored_count: u64,
    pub values: *mut c_void,
}

pub type OexFunctionCallback = unsafe extern "C" fn(*mut OexCall) -> OexStatus;
pub type OexNativeConstructor = unsafe extern "C" fn(*mut OexCall, *mut *mut c_void) -> OexStatus;
pub type OexNativeMethod = unsafe extern "C" fn(*mut OexCall, *mut c_void) -> OexStatus;
pub type OexNativePropertyGetter = unsafe extern "C" fn(*mut OexCall, *mut c_void) -> OexStatus;
pub type OexNativePropertySetter = unsafe extern "C" fn(*mut OexCall, *mut c_void) -> OexStatus;
pub type OexNativeDestructor = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexFunctionDefinition {
    pub struct_size: u32,
    pub flags: u32,
    pub name: OexUtf8View,
    pub minimum_inputs: u32,
    pub maximum_inputs: u32,
    pub minimum_outputs: u32,
    pub maximum_outputs: u32,
    pub invoke: Option<OexFunctionCallback>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexNativeMethodDefinition {
    pub struct_size: u32,
    pub flags: u32,
    pub name: OexUtf8View,
    pub minimum_inputs: u32,
    pub maximum_inputs: u32,
    pub minimum_outputs: u32,
    pub maximum_outputs: u32,
    pub invoke: Option<OexNativeMethod>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexNativePropertyDefinition {
    pub struct_size: u32,
    pub flags: u32,
    pub name: OexUtf8View,
    pub getter: Option<OexNativePropertyGetter>,
    pub setter: Option<OexNativePropertySetter>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexNativeClassDefinition {
    pub struct_size: u32,
    pub semantics: u32,
    pub name: OexUtf8View,
    pub constructor: Option<OexNativeConstructor>,
    pub destructor: Option<OexNativeDestructor>,
    pub methods: *const OexNativeMethodDefinition,
    pub method_count: u64,
    pub type_token: *const c_void,
    pub properties: *const OexNativePropertyDefinition,
    pub property_count: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OexPluginDefinition {
    pub struct_size: u32,
    pub required_abi: u32,
    pub flags: u32,
    pub reserved: u32,
    pub name: OexUtf8View,
    pub version: OexUtf8View,
    pub functions: *const OexFunctionDefinition,
    pub function_count: u64,
    pub classes: *const OexNativeClassDefinition,
    pub class_count: u64,
}

pub type OexPluginInit = unsafe extern "C" fn() -> *const OexPluginDefinition;

pub type OexValueKind = u32;
pub type OexDataType = u32;

#[cfg_attr(windows, link(name = "openmat_oex", kind = "raw-dylib"))]
#[cfg_attr(not(windows), link(name = "openmat_oex"))]
unsafe extern "C" {
    pub fn OexCall_GetInputCount(call: *const OexCall) -> u32;
    pub fn OexCall_GetOutputCount(call: *const OexCall) -> u32;
    pub fn OexCall_BorrowInput(call: *const OexCall, index: u32) -> *const OexValue;
    pub fn OexCall_TakeInput(
        call: *mut OexCall,
        index: u32,
        value: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexCall_SetOutput(call: *mut OexCall, index: u32, value: *mut OexValue) -> OexStatus;
    pub fn OexCall_SetError(
        call: *mut OexCall,
        identifier: OexUtf8View,
        message: OexUtf8View,
    ) -> OexStatus;
    pub fn OexCall_Invoke(
        call: *mut OexCall,
        callable: *const OexValue,
        input_count: u32,
        inputs: *const *const OexValue,
        output_count: u32,
        outputs: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexCall_GetCancellation(call: *const OexCall) -> *const OexCancellation;
    pub fn OexCancellation_IsRequested(cancellation: *const OexCancellation) -> u32;
    pub fn OexValue_GetInfo(value: *const OexValue, info: *mut OexValueInfo) -> OexStatus;
    pub fn OexValue_Retain(value: *const OexValue, retained: *mut *mut OexValue) -> OexStatus;
    pub fn OexValue_Release(value: *mut OexValue);
    pub fn OexValue_GetFloat64(value: *const OexValue, result: *mut f64) -> OexStatus;
    pub fn OexValue_CreateFloat64(
        call: *mut OexCall,
        value: f64,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexDense_GetView(value: *const OexValue, view: *mut OexDenseView) -> OexStatus;
    pub fn OexDense_GetMutableView(
        value: *mut OexValue,
        view: *mut OexMutableDenseView,
    ) -> OexStatus;
    pub fn OexDenseBuilder_CreateUninitialized(
        call: *mut OexCall,
        data_type: u32,
        rank: u32,
        dimensions: *const u64,
        builder: *mut *mut OexDenseBuilder,
        view: *mut OexMutableDenseView,
    ) -> OexStatus;
    pub fn OexDenseBuilder_Commit(
        builder: *mut OexDenseBuilder,
        value: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexDenseBuilder_Abort(builder: *mut OexDenseBuilder);
    pub fn OexSparse_GetCscView(value: *const OexValue, view: *mut OexSparseCscView) -> OexStatus;
    pub fn OexSparsePattern_Create(
        call: *mut OexCall,
        rows: u64,
        columns: u64,
        stored_count: u64,
        column_offsets: *const u64,
        row_indices: *const u64,
        pattern: *mut *mut OexSparsePattern,
    ) -> OexStatus;
    pub fn OexSparsePattern_Retain(
        pattern: *const OexSparsePattern,
        retained: *mut *mut OexSparsePattern,
    ) -> OexStatus;
    pub fn OexSparsePattern_Release(pattern: *mut OexSparsePattern);
    pub fn OexSparseTripletBuilder_CreateUninitialized(
        call: *mut OexCall,
        data_type: u32,
        rows: u64,
        columns: u64,
        capacity: u64,
        builder: *mut *mut OexSparseTripletBuilder,
        view: *mut OexSparseTripletView,
    ) -> OexStatus;
    pub fn OexSparseTripletBuilder_Commit(
        builder: *mut OexSparseTripletBuilder,
        stored_count: u64,
        value: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexSparseTripletBuilder_Abort(builder: *mut OexSparseTripletBuilder);
    pub fn OexSparsePatternBuilder_CreateUninitialized(
        call: *mut OexCall,
        pattern: *const OexSparsePattern,
        data_type: u32,
        builder: *mut *mut OexSparsePatternBuilder,
        view: *mut OexSparsePatternValuesView,
    ) -> OexStatus;
    pub fn OexSparsePatternBuilder_Commit(
        builder: *mut OexSparsePatternBuilder,
        value: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexSparsePatternBuilder_Abort(builder: *mut OexSparsePatternBuilder);
    pub fn OexNativeObject_BorrowInstance(
        call: *mut OexCall,
        value: *const OexValue,
        expected_type_token: *const c_void,
        instance: *mut *mut c_void,
    ) -> OexStatus;
    pub fn OexNativeObject_Create(
        call: *mut OexCall,
        type_token: *const c_void,
        instance: *mut c_void,
        value: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexValue_GetDimensions(
        value: *const OexValue,
        capacity: u32,
        dimensions: *mut u64,
        rank: *mut u32,
    ) -> OexStatus;
    pub fn OexValue_GetClassName(
        call: *mut OexCall,
        value: *const OexValue,
        name: *mut OexUtf8View,
    ) -> OexStatus;
    pub fn OexString_Create(
        call: *mut OexCall,
        rank: u32,
        dimensions: *const u64,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexString_CreateFromUtf16(
        call: *mut OexCall,
        rank: u32,
        dimensions: *const u64,
        count: u64,
        elements: *const OexStringElementView,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexString_SetElementsUtf16(
        call: *mut OexCall,
        value: *mut OexValue,
        start: u64,
        count: u64,
        elements: *const OexStringElementView,
    ) -> OexStatus;
    pub fn OexString_CreateFromUtf8(
        call: *mut OexCall,
        rank: u32,
        dimensions: *const u64,
        count: u64,
        elements: *const OexUtf8View,
        missing: *const u8,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexString_SetElementsUtf8(
        call: *mut OexCall,
        value: *mut OexValue,
        start: u64,
        count: u64,
        elements: *const OexUtf8View,
        missing: *const u8,
    ) -> OexStatus;
    pub fn OexString_GetElementView(
        value: *const OexValue,
        index: u64,
        view: *mut OexStringElementView,
    ) -> OexStatus;
    pub fn OexString_GetElements(
        value: *const OexValue,
        start: u64,
        count: u64,
        views: *mut OexStringElementView,
    ) -> OexStatus;
    pub fn OexString_CopyElementUtf8(
        value: *const OexValue,
        index: u64,
        buffer: *mut c_char,
        capacity: u64,
        required: *mut u64,
        is_missing: *mut u32,
    ) -> OexStatus;
    pub fn OexString_SetElementUtf8(
        call: *mut OexCall,
        value: *mut OexValue,
        index: u64,
        text: OexUtf8View,
    ) -> OexStatus;
    pub fn OexString_SetElementUtf16(
        call: *mut OexCall,
        value: *mut OexValue,
        index: u64,
        text: OexUtf16View,
    ) -> OexStatus;
    pub fn OexString_SetMissing(call: *mut OexCall, value: *mut OexValue, index: u64) -> OexStatus;
    pub fn OexCell_Create(
        call: *mut OexCall,
        rank: u32,
        dimensions: *const u64,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexCell_GetElement(
        call: *mut OexCall,
        value: *const OexValue,
        index: u64,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexCell_SetElement(
        call: *mut OexCall,
        value: *mut OexValue,
        index: u64,
        element: *const OexValue,
    ) -> OexStatus;
    pub fn OexCell_GetElements(
        call: *mut OexCall,
        value: *const OexValue,
        start: u64,
        count: u64,
        results: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexCell_SetElements(
        call: *mut OexCall,
        value: *mut OexValue,
        start: u64,
        count: u64,
        elements: *const *const OexValue,
    ) -> OexStatus;
    pub fn OexStruct_Create(
        call: *mut OexCall,
        rank: u32,
        dimensions: *const u64,
        field_count: u64,
        field_names: *const OexUtf8View,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexStruct_GetFieldCount(value: *const OexValue, count: *mut u64) -> OexStatus;
    pub fn OexStruct_GetFieldName(
        value: *const OexValue,
        field_index: u64,
        name: *mut OexUtf8View,
    ) -> OexStatus;
    pub fn OexStruct_FindField(
        value: *const OexValue,
        name: OexUtf8View,
        index: *mut u64,
    ) -> OexStatus;
    pub fn OexStruct_GetField(
        call: *mut OexCall,
        value: *const OexValue,
        element_index: u64,
        field_index: u64,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexStruct_SetField(
        call: *mut OexCall,
        value: *mut OexValue,
        element_index: u64,
        field_index: u64,
        element: *const OexValue,
    ) -> OexStatus;
    pub fn OexStruct_AddField(
        call: *mut OexCall,
        value: *mut OexValue,
        name: OexUtf8View,
    ) -> OexStatus;
    pub fn OexStruct_RemoveField(
        call: *mut OexCall,
        value: *mut OexValue,
        field_index: u64,
    ) -> OexStatus;
    pub fn OexStruct_SetFieldNames(
        call: *mut OexCall,
        value: *mut OexValue,
        count: u64,
        names: *const OexUtf8View,
    ) -> OexStatus;
    pub fn OexStruct_GetFieldValues(
        call: *mut OexCall,
        value: *const OexValue,
        field_index: u64,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexStruct_SetFieldValues(
        call: *mut OexCall,
        value: *mut OexValue,
        field_index: u64,
        elements: *const OexValue,
    ) -> OexStatus;
    pub fn OexTable_Create(
        call: *mut OexCall,
        row_count: u64,
        variable_count: u64,
        variable_names: *const OexUtf8View,
        variables: *const *const OexValue,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexTable_GetRowCount(value: *const OexValue, count: *mut u64) -> OexStatus;
    pub fn OexTable_GetVariableCount(value: *const OexValue, count: *mut u64) -> OexStatus;
    pub fn OexTable_GetVariableName(
        value: *const OexValue,
        index: u64,
        name: *mut OexUtf8View,
    ) -> OexStatus;
    pub fn OexTable_FindVariable(
        value: *const OexValue,
        name: OexUtf8View,
        index: *mut u64,
    ) -> OexStatus;
    pub fn OexTable_SetVariableNames(
        call: *mut OexCall,
        value: *mut OexValue,
        count: u64,
        names: *const OexUtf8View,
    ) -> OexStatus;
    pub fn OexTable_GetVariable(
        call: *mut OexCall,
        value: *const OexValue,
        index: u64,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexTable_SetVariable(
        call: *mut OexCall,
        value: *mut OexValue,
        index: u64,
        variable: *const OexValue,
    ) -> OexStatus;
    pub fn OexTable_AppendVariable(
        call: *mut OexCall,
        value: *mut OexValue,
        name: OexUtf8View,
        variable: *const OexValue,
    ) -> OexStatus;
    pub fn OexTable_RemoveVariable(
        call: *mut OexCall,
        value: *mut OexValue,
        index: u64,
    ) -> OexStatus;
    pub fn OexTable_GetRowNames(
        call: *mut OexCall,
        value: *const OexValue,
        has_names: *mut u32,
        result: *mut *mut OexValue,
    ) -> OexStatus;
    pub fn OexTable_SetRowNames(
        call: *mut OexCall,
        value: *mut OexValue,
        count: u64,
        names: *const OexUtf8View,
    ) -> OexStatus;
    pub fn OexTable_ClearRowNames(call: *mut OexCall, value: *mut OexValue) -> OexStatus;
}

macro_rules! view_default {
    ($($ty:ty),* $(,)?) => { $(
        impl Default for $ty {
            fn default() -> Self {
                // SAFETY: These C output structs contain only integers and raw
                // pointers, for which all-zero bytes are valid representations.
                let mut value: Self = unsafe { core::mem::zeroed() };
                value.struct_size = abi_size::<Self>();
                value
            }
        }
    )* };
}
view_default!(
    OexStringElementView,
    OexDenseView,
    OexMutableDenseView,
    OexSparseCscView,
    OexSparseTripletView,
    OexSparsePatternValuesView
);

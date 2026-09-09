use std::ffi::{c_char, c_void};
use std::mem::size_of;

#[allow(clippy::cast_possible_truncation)]
pub(crate) const fn abi_size<T>() -> u32 {
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

pub(crate) const OEX_BRIDGE_MAGIC: u64 = 0x4f45_5842_5244_4731;
pub(crate) const OEX_HEADER_CALL: u64 = 1;
pub(crate) const OEX_HEADER_VALUE: u64 = 2;
pub(crate) const OEX_HEADER_CANCELLATION: u64 = 3;
pub(crate) const OEX_HEADER_DENSE_BUILDER: u64 = 4;
pub(crate) const OEX_HEADER_SPARSE_PATTERN: u64 = 5;
pub(crate) const OEX_HEADER_SPARSE_TRIPLET_BUILDER: u64 = 6;
pub(crate) const OEX_HEADER_SPARSE_PATTERN_BUILDER: u64 = 7;

#[repr(C)]
pub(crate) struct OexObjectHeader {
    pub(crate) dispatch: *const OexDispatch,
    pub(crate) bridge_magic: u64,
    pub(crate) kind: u64,
}

#[repr(C)]
pub(crate) struct OexDispatch {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) call_get_input_count: unsafe extern "C" fn(*const OexCall) -> u32,
    pub(crate) call_get_output_count: unsafe extern "C" fn(*const OexCall) -> u32,
    pub(crate) call_borrow_input: unsafe extern "C" fn(*const OexCall, u32) -> *const OexValue,
    pub(crate) call_take_input:
        unsafe extern "C" fn(*mut OexCall, u32, *mut *mut OexValue) -> OexStatus,
    pub(crate) call_set_output: unsafe extern "C" fn(*mut OexCall, u32, *mut OexValue) -> OexStatus,
    pub(crate) call_set_error:
        unsafe extern "C" fn(*mut OexCall, OexUtf8View, OexUtf8View) -> OexStatus,
    pub(crate) call_get_cancellation:
        unsafe extern "C" fn(*const OexCall) -> *const OexCancellation,
    pub(crate) cancellation_is_requested: unsafe extern "C" fn(*const OexCancellation) -> u32,
    pub(crate) value_get_info:
        unsafe extern "C" fn(*const OexValue, *mut OexValueInfo) -> OexStatus,
    pub(crate) value_retain: unsafe extern "C" fn(*const OexValue, *mut *mut OexValue) -> OexStatus,
    pub(crate) value_release: unsafe extern "C" fn(*mut OexValue),
    pub(crate) value_get_f64: unsafe extern "C" fn(*const OexValue, *mut f64) -> OexStatus,
    pub(crate) value_create_f64:
        unsafe extern "C" fn(*mut OexCall, f64, *mut *mut OexValue) -> OexStatus,
    pub(crate) dense_get_view:
        unsafe extern "C" fn(*const OexValue, *mut OexDenseView) -> OexStatus,
    pub(crate) dense_get_mutable_view:
        unsafe extern "C" fn(*mut OexValue, *mut OexMutableDenseView) -> OexStatus,
    pub(crate) dense_builder_create_uninitialized: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        u32,
        *const u64,
        *mut *mut OexDenseBuilder,
        *mut OexMutableDenseView,
    ) -> OexStatus,
    pub(crate) dense_builder_commit:
        unsafe extern "C" fn(*mut OexDenseBuilder, *mut *mut OexValue) -> OexStatus,
    pub(crate) dense_builder_abort: unsafe extern "C" fn(*mut OexDenseBuilder),
    pub(crate) sparse_get_csc_view:
        unsafe extern "C" fn(*const OexValue, *mut OexSparseCscView) -> OexStatus,
    pub(crate) sparse_pattern_create: unsafe extern "C" fn(
        *mut OexCall,
        u64,
        u64,
        u64,
        *const u64,
        *const u64,
        *mut *mut OexSparsePattern,
    ) -> OexStatus,
    pub(crate) sparse_pattern_retain:
        unsafe extern "C" fn(*const OexSparsePattern, *mut *mut OexSparsePattern) -> OexStatus,
    pub(crate) sparse_pattern_release: unsafe extern "C" fn(*mut OexSparsePattern),
    pub(crate) sparse_triplet_builder_create_uninitialized: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        u64,
        u64,
        u64,
        *mut *mut OexSparseTripletBuilder,
        *mut OexSparseTripletView,
    ) -> OexStatus,
    pub(crate) sparse_triplet_builder_commit:
        unsafe extern "C" fn(*mut OexSparseTripletBuilder, u64, *mut *mut OexValue) -> OexStatus,
    pub(crate) sparse_triplet_builder_abort: unsafe extern "C" fn(*mut OexSparseTripletBuilder),
    pub(crate) sparse_pattern_builder_create_uninitialized: unsafe extern "C" fn(
        *mut OexCall,
        *const OexSparsePattern,
        u32,
        *mut *mut OexSparsePatternBuilder,
        *mut OexSparsePatternValuesView,
    ) -> OexStatus,
    pub(crate) sparse_pattern_builder_commit:
        unsafe extern "C" fn(*mut OexSparsePatternBuilder, *mut *mut OexValue) -> OexStatus,
    pub(crate) sparse_pattern_builder_abort: unsafe extern "C" fn(*mut OexSparsePatternBuilder),
    pub(crate) native_object_borrow_instance: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        *const c_void,
        *mut *mut c_void,
    ) -> OexStatus,
    pub(crate) native_object_create: unsafe extern "C" fn(
        *mut OexCall,
        *const c_void,
        *mut c_void,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) call_invoke: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u32,
        *const *const OexValue,
        u32,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) value_get_dimensions:
        unsafe extern "C" fn(*const OexValue, u32, *mut u64, *mut u32) -> OexStatus,
    pub(crate) value_get_class_name:
        unsafe extern "C" fn(*mut OexCall, *const OexValue, *mut OexUtf8View) -> OexStatus,
    pub(crate) string_create:
        unsafe extern "C" fn(*mut OexCall, u32, *const u64, *mut *mut OexValue) -> OexStatus,
    pub(crate) string_create_from_utf16: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        *const u64,
        u64,
        *const OexStringElementView,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) string_set_elements_utf16: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        u64,
        u64,
        *const OexStringElementView,
    ) -> OexStatus,
    pub(crate) string_create_from_utf8: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        *const u64,
        u64,
        *const OexUtf8View,
        *const u8,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) string_set_elements_utf8: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        u64,
        u64,
        *const OexUtf8View,
        *const u8,
    ) -> OexStatus,
    pub(crate) string_get_element_view:
        unsafe extern "C" fn(*const OexValue, u64, *mut OexStringElementView) -> OexStatus,
    pub(crate) string_get_elements:
        unsafe extern "C" fn(*const OexValue, u64, u64, *mut OexStringElementView) -> OexStatus,
    pub(crate) string_copy_element_utf8: unsafe extern "C" fn(
        *const OexValue,
        u64,
        *mut c_char,
        u64,
        *mut u64,
        *mut u32,
    ) -> OexStatus,
    pub(crate) string_set_element_utf8:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, OexUtf8View) -> OexStatus,
    pub(crate) string_set_element_utf16:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, OexUtf16View) -> OexStatus,
    pub(crate) string_set_missing:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64) -> OexStatus,
    pub(crate) cell_create:
        unsafe extern "C" fn(*mut OexCall, u32, *const u64, *mut *mut OexValue) -> OexStatus,
    pub(crate) cell_get_element:
        unsafe extern "C" fn(*mut OexCall, *const OexValue, u64, *mut *mut OexValue) -> OexStatus,
    pub(crate) cell_set_element:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexValue) -> OexStatus,
    pub(crate) cell_get_elements: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u64,
        u64,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) cell_set_elements: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        u64,
        u64,
        *const *const OexValue,
    ) -> OexStatus,
    pub(crate) struct_create: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        *const u64,
        u64,
        *const OexUtf8View,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) struct_get_field_count: unsafe extern "C" fn(*const OexValue, *mut u64) -> OexStatus,
    pub(crate) struct_get_field_name:
        unsafe extern "C" fn(*const OexValue, u64, *mut OexUtf8View) -> OexStatus,
    pub(crate) struct_find_field:
        unsafe extern "C" fn(*const OexValue, OexUtf8View, *mut u64) -> OexStatus,
    pub(crate) struct_get_field: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u64,
        u64,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) struct_set_field:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, u64, *const OexValue) -> OexStatus,
    pub(crate) struct_add_field:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, OexUtf8View) -> OexStatus,
    pub(crate) struct_remove_field:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64) -> OexStatus,
    pub(crate) struct_set_field_names:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexUtf8View) -> OexStatus,
    pub(crate) struct_get_field_values:
        unsafe extern "C" fn(*mut OexCall, *const OexValue, u64, *mut *mut OexValue) -> OexStatus,
    pub(crate) struct_set_field_values:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexValue) -> OexStatus,
    pub(crate) table_create: unsafe extern "C" fn(
        *mut OexCall,
        u64,
        u64,
        *const OexUtf8View,
        *const *const OexValue,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) table_get_row_count: unsafe extern "C" fn(*const OexValue, *mut u64) -> OexStatus,
    pub(crate) table_get_variable_count:
        unsafe extern "C" fn(*const OexValue, *mut u64) -> OexStatus,
    pub(crate) table_get_variable_name:
        unsafe extern "C" fn(*const OexValue, u64, *mut OexUtf8View) -> OexStatus,
    pub(crate) table_find_variable:
        unsafe extern "C" fn(*const OexValue, OexUtf8View, *mut u64) -> OexStatus,
    pub(crate) table_set_variable_names:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexUtf8View) -> OexStatus,
    pub(crate) table_get_variable:
        unsafe extern "C" fn(*mut OexCall, *const OexValue, u64, *mut *mut OexValue) -> OexStatus,
    pub(crate) table_set_variable:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexValue) -> OexStatus,
    pub(crate) table_append_variable: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        OexUtf8View,
        *const OexValue,
    ) -> OexStatus,
    pub(crate) table_remove_variable:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64) -> OexStatus,
    pub(crate) table_get_row_names: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        *mut u32,
        *mut *mut OexValue,
    ) -> OexStatus,
    pub(crate) table_set_row_names:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexUtf8View) -> OexStatus,
    pub(crate) table_clear_row_names:
        unsafe extern "C" fn(*mut OexCall, *mut OexValue) -> OexStatus,
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use super::{
        OexComplexF32, OexComplexF64, OexDenseView, OexFunctionDefinition, OexMutableDenseView,
        OexNativeClassDefinition, OexNativeMethodDefinition, OexNativePropertyDefinition,
        OexPluginDefinition, OexSparseCscView, OexSparsePatternValuesView, OexSparseTripletView,
        OexUtf8View, OexValueInfo,
    };

    #[test]
    fn public_x64_layout_is_fixed() {
        assert_eq!(size_of::<usize>(), 8, "OEX targets a 64-bit host");
        assert_eq!(size_of::<OexUtf8View>(), 16);
        assert_eq!(size_of::<OexValueInfo>(), 32);
        assert_eq!(size_of::<OexDenseView>(), 40);
        assert_eq!(size_of::<OexMutableDenseView>(), 40);
        assert_eq!(size_of::<OexSparseCscView>(), 64);
        assert_eq!(size_of::<OexSparseTripletView>(), 64);
        assert_eq!(size_of::<OexSparsePatternValuesView>(), 32);
        assert_eq!(size_of::<OexComplexF32>(), 8);
        assert_eq!(size_of::<OexComplexF64>(), 16);
        assert_eq!(size_of::<OexFunctionDefinition>(), 48);
        assert_eq!(size_of::<OexNativeMethodDefinition>(), 48);
        assert_eq!(size_of::<OexNativePropertyDefinition>(), 40);
        assert_eq!(size_of::<OexNativeClassDefinition>(), 80);
        assert_eq!(size_of::<OexPluginDefinition>(), 80);
        assert_eq!(align_of::<OexPluginDefinition>(), 8);
    }
}

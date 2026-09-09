//! Thin exported C entrypoints.
//!
//! The public ABI is a set of ordinary functions. Opaque objects carry a private
//! host dispatch pointer so this bridge DLL never owns runtime state or plugin
//! allocations.

use std::{
    ffi::{c_char, c_void},
    mem::size_of,
};

use crate::abi::{
    OEX_ABI_VERSION, OEX_BRIDGE_MAGIC, OEX_ERROR_ABI, OEX_ERROR_ARGUMENT, OEX_HEADER_CALL,
    OEX_HEADER_CANCELLATION, OEX_HEADER_DENSE_BUILDER, OEX_HEADER_SPARSE_PATTERN,
    OEX_HEADER_SPARSE_PATTERN_BUILDER, OEX_HEADER_SPARSE_TRIPLET_BUILDER, OEX_HEADER_VALUE,
    OexCall, OexCancellation, OexDenseBuilder, OexDenseView, OexDispatch, OexMutableDenseView,
    OexObjectHeader, OexSparseCscView, OexSparsePattern, OexSparsePatternBuilder,
    OexSparsePatternValuesView, OexSparseTripletBuilder, OexSparseTripletView, OexStatus,
    OexStringElementView, OexUtf8View, OexUtf16View, OexValue, OexValueInfo,
};

unsafe fn dispatch<T>(object: *const T, expected_kind: u64) -> Option<&'static OexDispatch> {
    if object.is_null() {
        return None;
    }
    // SAFETY: Every public opaque OEX object begins with OexObjectHeader. A plugin that
    // passes an arbitrary pointer already violates the trusted in-process ABI contract.
    let header = unsafe { &*object.cast::<OexObjectHeader>() };
    if header.bridge_magic != OEX_BRIDGE_MAGIC
        || header.kind != expected_kind
        || header.dispatch.is_null()
    {
        return None;
    }
    // SAFETY: A valid host-created header points at a process-lifetime dispatch table.
    let dispatch = unsafe { &*header.dispatch };
    let required_size = u32::try_from(size_of::<OexDispatch>()).unwrap_or(u32::MAX);
    let host_major = dispatch.abi_version >> 16;
    let host_minor = dispatch.abi_version & 0xffff;
    let bridge_major = OEX_ABI_VERSION >> 16;
    let bridge_minor = OEX_ABI_VERSION & 0xffff;
    (host_major == bridge_major
        && host_minor >= bridge_minor
        && dispatch.struct_size >= required_size)
        .then_some(dispatch)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCall_GetInputCount(call: *const OexCall) -> u32 {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return 0;
    };
    unsafe { (dispatch.call_get_input_count)(call) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCall_GetOutputCount(call: *const OexCall) -> u32 {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return 0;
    };
    unsafe { (dispatch.call_get_output_count)(call) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCall_BorrowInput(call: *const OexCall, index: u32) -> *const OexValue {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return std::ptr::null();
    };
    unsafe { (dispatch.call_borrow_input)(call, index) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCall_TakeInput(
    call: *mut OexCall,
    index: u32,
    value: *mut *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.call_take_input)(call, index, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCall_SetOutput(
    call: *mut OexCall,
    index: u32,
    value: *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.call_set_output)(call, index, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCall_SetError(
    call: *mut OexCall,
    identifier: OexUtf8View,
    message: OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.call_set_error)(call, identifier, message) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCall_Invoke(
    call: *mut OexCall,
    callable: *const OexValue,
    input_count: u32,
    inputs: *const *const OexValue,
    output_count: u32,
    outputs: *mut *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.call_invoke)(call, callable, input_count, inputs, output_count, outputs) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCall_GetCancellation(call: *const OexCall) -> *const OexCancellation {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return std::ptr::null();
    };
    unsafe { (dispatch.call_get_cancellation)(call) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCancellation_IsRequested(cancellation: *const OexCancellation) -> u32 {
    let Some(dispatch) = (unsafe { dispatch(cancellation, OEX_HEADER_CANCELLATION) }) else {
        return 0;
    };
    unsafe { (dispatch.cancellation_is_requested)(cancellation) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexValue_GetInfo(
    value: *const OexValue,
    info: *mut OexValueInfo,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.value_get_info)(value, info) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexValue_Retain(
    value: *const OexValue,
    retained: *mut *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.value_retain)(value, retained) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexValue_Release(value: *mut OexValue) {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return;
    };
    unsafe { (dispatch.value_release)(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexValue_GetFloat64(
    value: *const OexValue,
    result: *mut f64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.value_get_f64)(value, result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexValue_CreateFloat64(
    call: *mut OexCall,
    value: f64,
    result: *mut *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.value_create_f64)(call, value, result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexDense_GetView(
    value: *const OexValue,
    view: *mut OexDenseView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.dense_get_view)(value, view) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexDense_GetMutableView(
    value: *mut OexValue,
    view: *mut OexMutableDenseView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.dense_get_mutable_view)(value, view) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexDenseBuilder_CreateUninitialized(
    call: *mut OexCall,
    data_type: u32,
    rank: u32,
    dimensions: *const u64,
    builder: *mut *mut OexDenseBuilder,
    view: *mut OexMutableDenseView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    if builder.is_null() || view.is_null() {
        return OEX_ERROR_ARGUMENT;
    }
    unsafe {
        (dispatch.dense_builder_create_uninitialized)(
            call, data_type, rank, dimensions, builder, view,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexDenseBuilder_Commit(
    builder: *mut OexDenseBuilder,
    value: *mut *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(builder, OEX_HEADER_DENSE_BUILDER) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.dense_builder_commit)(builder, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexDenseBuilder_Abort(builder: *mut OexDenseBuilder) {
    let Some(dispatch) = (unsafe { dispatch(builder, OEX_HEADER_DENSE_BUILDER) }) else {
        return;
    };
    unsafe { (dispatch.dense_builder_abort)(builder) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparse_GetCscView(
    value: *const OexValue,
    view: *mut OexSparseCscView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.sparse_get_csc_view)(value, view) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparsePattern_Create(
    call: *mut OexCall,
    rows: u64,
    columns: u64,
    stored_count: u64,
    column_offsets: *const u64,
    row_indices: *const u64,
    pattern: *mut *mut OexSparsePattern,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe {
        (dispatch.sparse_pattern_create)(
            call,
            rows,
            columns,
            stored_count,
            column_offsets,
            row_indices,
            pattern,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparsePattern_Retain(
    pattern: *const OexSparsePattern,
    retained: *mut *mut OexSparsePattern,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(pattern, OEX_HEADER_SPARSE_PATTERN) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.sparse_pattern_retain)(pattern, retained) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparsePattern_Release(pattern: *mut OexSparsePattern) {
    let Some(dispatch) = (unsafe { dispatch(pattern, OEX_HEADER_SPARSE_PATTERN) }) else {
        return;
    };
    unsafe { (dispatch.sparse_pattern_release)(pattern) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparseTripletBuilder_CreateUninitialized(
    call: *mut OexCall,
    data_type: u32,
    rows: u64,
    columns: u64,
    capacity: u64,
    builder: *mut *mut OexSparseTripletBuilder,
    view: *mut OexSparseTripletView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe {
        (dispatch.sparse_triplet_builder_create_uninitialized)(
            call, data_type, rows, columns, capacity, builder, view,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparseTripletBuilder_Commit(
    builder: *mut OexSparseTripletBuilder,
    stored_count: u64,
    value: *mut *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(builder, OEX_HEADER_SPARSE_TRIPLET_BUILDER) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.sparse_triplet_builder_commit)(builder, stored_count, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparseTripletBuilder_Abort(builder: *mut OexSparseTripletBuilder) {
    let Some(dispatch) = (unsafe { dispatch(builder, OEX_HEADER_SPARSE_TRIPLET_BUILDER) }) else {
        return;
    };
    unsafe { (dispatch.sparse_triplet_builder_abort)(builder) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparsePatternBuilder_CreateUninitialized(
    call: *mut OexCall,
    pattern: *const OexSparsePattern,
    data_type: u32,
    builder: *mut *mut OexSparsePatternBuilder,
    view: *mut OexSparsePatternValuesView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe {
        (dispatch.sparse_pattern_builder_create_uninitialized)(
            call, pattern, data_type, builder, view,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparsePatternBuilder_Commit(
    builder: *mut OexSparsePatternBuilder,
    value: *mut *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(builder, OEX_HEADER_SPARSE_PATTERN_BUILDER) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.sparse_pattern_builder_commit)(builder, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexSparsePatternBuilder_Abort(builder: *mut OexSparsePatternBuilder) {
    let Some(dispatch) = (unsafe { dispatch(builder, OEX_HEADER_SPARSE_PATTERN_BUILDER) }) else {
        return;
    };
    unsafe { (dispatch.sparse_pattern_builder_abort)(builder) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexNativeObject_BorrowInstance(
    call: *mut OexCall,
    value: *const OexValue,
    expected_type_token: *const c_void,
    instance: *mut *mut c_void,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.native_object_borrow_instance)(call, value, expected_type_token, instance) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexNativeObject_Create(
    call: *mut OexCall,
    type_token: *const c_void,
    instance: *mut c_void,
    value: *mut *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.native_object_create)(call, type_token, instance, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexValue_GetDimensions(
    value: *const OexValue,
    capacity: u32,
    dimensions: *mut u64,
    rank: *mut u32,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.value_get_dimensions)(value, capacity, dimensions, rank) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexValue_GetClassName(
    call: *mut OexCall,
    value: *const OexValue,
    name: *mut OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.value_get_class_name)(call, value, name) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_Create(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_create)(call, rank, dimensions, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_CreateFromUtf16(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    count: u64,
    elements: *const OexStringElementView,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_create_from_utf16)(call, rank, dimensions, count, elements, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_SetElementsUtf16(
    call: *mut OexCall,
    value: *mut OexValue,
    start: u64,
    count: u64,
    elements: *const OexStringElementView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_set_elements_utf16)(call, value, start, count, elements) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_CreateFromUtf8(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    count: u64,
    elements: *const OexUtf8View,
    missing: *const u8,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe {
        (dispatch.string_create_from_utf8)(call, rank, dimensions, count, elements, missing, result)
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_SetElementsUtf8(
    call: *mut OexCall,
    value: *mut OexValue,
    start: u64,
    count: u64,
    elements: *const OexUtf8View,
    missing: *const u8,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_set_elements_utf8)(call, value, start, count, elements, missing) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_GetElementView(
    value: *const OexValue,
    index: u64,
    view: *mut OexStringElementView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_get_element_view)(value, index, view) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_GetElements(
    value: *const OexValue,
    start: u64,
    count: u64,
    views: *mut OexStringElementView,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_get_elements)(value, start, count, views) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_CopyElementUtf8(
    value: *const OexValue,
    index: u64,
    buffer: *mut c_char,
    capacity: u64,
    required: *mut u64,
    is_missing: *mut u32,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe {
        (dispatch.string_copy_element_utf8)(value, index, buffer, capacity, required, is_missing)
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_SetElementUtf8(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    text: OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_set_element_utf8)(call, value, index, text) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_SetElementUtf16(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    text: OexUtf16View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_set_element_utf16)(call, value, index, text) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexString_SetMissing(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.string_set_missing)(call, value, index) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCell_Create(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.cell_create)(call, rank, dimensions, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCell_GetElement(
    call: *mut OexCall,
    value: *const OexValue,
    index: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.cell_get_element)(call, value, index, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCell_SetElement(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    element: *const OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.cell_set_element)(call, value, index, element) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCell_GetElements(
    call: *mut OexCall,
    value: *const OexValue,
    start: u64,
    count: u64,
    results: *mut *mut OexValue,
) -> OexStatus {
    if let Ok(out) = unsafe { crate::host::containers::output_slice(results, count) } {
        out.fill(std::ptr::null_mut());
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.cell_get_elements)(call, value, start, count, results) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexCell_SetElements(
    call: *mut OexCall,
    value: *mut OexValue,
    start: u64,
    count: u64,
    elements: *const *const OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.cell_set_elements)(call, value, start, count, elements) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_Create(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    field_count: u64,
    field_names: *const OexUtf8View,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_create)(call, rank, dimensions, field_count, field_names, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_GetFieldCount(
    value: *const OexValue,
    count: *mut u64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_get_field_count)(value, count) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_GetFieldName(
    value: *const OexValue,
    field_index: u64,
    name: *mut OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_get_field_name)(value, field_index, name) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_FindField(
    value: *const OexValue,
    name: OexUtf8View,
    index: *mut u64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_find_field)(value, name, index) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_GetField(
    call: *mut OexCall,
    value: *const OexValue,
    element_index: u64,
    field_index: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_get_field)(call, value, element_index, field_index, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_SetField(
    call: *mut OexCall,
    value: *mut OexValue,
    element_index: u64,
    field_index: u64,
    element: *const OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_set_field)(call, value, element_index, field_index, element) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_AddField(
    call: *mut OexCall,
    value: *mut OexValue,
    name: OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_add_field)(call, value, name) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_RemoveField(
    call: *mut OexCall,
    value: *mut OexValue,
    field_index: u64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_remove_field)(call, value, field_index) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_SetFieldNames(
    call: *mut OexCall,
    value: *mut OexValue,
    count: u64,
    names: *const OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_set_field_names)(call, value, count, names) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_GetFieldValues(
    call: *mut OexCall,
    value: *const OexValue,
    field_index: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_get_field_values)(call, value, field_index, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexStruct_SetFieldValues(
    call: *mut OexCall,
    value: *mut OexValue,
    field_index: u64,
    elements: *const OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.struct_set_field_values)(call, value, field_index, elements) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_Create(
    call: *mut OexCall,
    row_count: u64,
    variable_count: u64,
    variable_names: *const OexUtf8View,
    variables: *const *const OexValue,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe {
        (dispatch.table_create)(
            call,
            row_count,
            variable_count,
            variable_names,
            variables,
            result,
        )
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_GetRowCount(
    value: *const OexValue,
    count: *mut u64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_get_row_count)(value, count) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_GetVariableCount(
    value: *const OexValue,
    count: *mut u64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_get_variable_count)(value, count) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_GetVariableName(
    value: *const OexValue,
    index: u64,
    name: *mut OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_get_variable_name)(value, index, name) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_FindVariable(
    value: *const OexValue,
    name: OexUtf8View,
    index: *mut u64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(value, OEX_HEADER_VALUE) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_find_variable)(value, name, index) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_SetVariableNames(
    call: *mut OexCall,
    value: *mut OexValue,
    count: u64,
    names: *const OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_set_variable_names)(call, value, count, names) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_GetVariable(
    call: *mut OexCall,
    value: *const OexValue,
    index: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_get_variable)(call, value, index, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_SetVariable(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    variable: *const OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_set_variable)(call, value, index, variable) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_AppendVariable(
    call: *mut OexCall,
    value: *mut OexValue,
    name: OexUtf8View,
    variable: *const OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_append_variable)(call, value, name, variable) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_RemoveVariable(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_remove_variable)(call, value, index) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_GetRowNames(
    call: *mut OexCall,
    value: *const OexValue,
    has_names: *mut u32,
    result: *mut *mut OexValue,
) -> OexStatus {
    if !result.is_null() {
        unsafe { result.write(std::ptr::null_mut()) };
    }
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_get_row_names)(call, value, has_names, result) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_SetRowNames(
    call: *mut OexCall,
    value: *mut OexValue,
    count: u64,
    names: *const OexUtf8View,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_set_row_names)(call, value, count, names) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn OexTable_ClearRowNames(
    call: *mut OexCall,
    value: *mut OexValue,
) -> OexStatus {
    let Some(dispatch) = (unsafe { dispatch(call, OEX_HEADER_CALL) }) else {
        return OEX_ERROR_ABI;
    };
    unsafe { (dispatch.table_clear_row_names)(call, value) }
}

//! Typed container operations. Stage mutations before replacing the owned handle.
//!
//! The trusted C ABI requires live, aligned handles and valid buffers of the
//! declared lengths. Input views are borrowed only during a synchronous call;
//! payloads are copied before replacing a target that might alias its source.
use super::{
    BuiltinContext, OEX_ERROR_ABI, OEX_ERROR_ALLOCATION, OEX_ERROR_ARGUMENT, OEX_ERROR_CANCELLED,
    OEX_ERROR_DIMENSION, OEX_ERROR_PLUGIN, OEX_ERROR_RANGE, OEX_ERROR_STATE, OEX_ERROR_TYPE,
    OEX_OK, OexCall, OexStatus, OexUtf8View, OexValue, Ordering, Shape, Value, ValueHandle,
    abi_size, checked_call, checked_call_mut, checked_value, checked_value_mut,
    native_object_error_status, size_of, slice, str,
};
use crate::abi::{OEX_ERROR_ENCODING, OEX_ERROR_NOT_FOUND, OexStringElementView, OexUtf16View};
use openmat_array::ArrayError;
use openmat_value::{
    AggregateError, CellArray, FieldName, StringArray, StringElement, StringValue, StructArray,
    TableArray, TableError, TableRowName, TableVariableName,
};
use std::ffi::c_char;

type ApiResult<T = ()> = Result<T, OexStatus>;

fn boundary(f: impl FnOnce() -> ApiResult) -> OexStatus {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .map_or(OEX_ERROR_PLUGIN, |r| r.err().unwrap_or(OEX_OK))
}
fn usize_count(n: u64) -> ApiResult<usize> {
    usize::try_from(n).map_err(|_| OEX_ERROR_RANGE)
}
fn reserve<T>(v: &mut Vec<T>, n: usize) -> ApiResult {
    v.try_reserve_exact(n).map_err(|_| OEX_ERROR_ALLOCATION)
}
fn vec_filled<T: Clone>(n: usize, element: T) -> ApiResult<Vec<T>> {
    let mut v = Vec::new();
    reserve(&mut v, n)?;
    v.resize(n, element);
    Ok(v)
}
fn require_out<T>(p: *mut T) -> ApiResult {
    if p.is_null() {
        Err(OEX_ERROR_ARGUMENT)
    } else {
        Ok(())
    }
}
unsafe fn write_out<T>(p: *mut T, value: T) -> ApiResult {
    require_out(p)?;
    unsafe { p.write(value) };
    Ok(())
}
unsafe fn input_slice<'a, T>(p: *const T, n: u64) -> ApiResult<&'a [T]> {
    let n = usize_count(n)?;
    if n == 0 {
        return Ok(&[]);
    }
    if p.is_null() || !p.is_aligned() {
        return Err(OEX_ERROR_ARGUMENT);
    }
    if n.checked_mul(size_of::<T>())
        .filter(|n| isize::try_from(*n).is_ok())
        .is_none()
    {
        return Err(OEX_ERROR_RANGE);
    }
    Ok(unsafe { slice::from_raw_parts(p, n) })
}
pub(crate) unsafe fn output_slice<'a, T>(p: *mut T, n: u64) -> ApiResult<&'a mut [T]> {
    let count = usize_count(n)?;
    if count == 0 {
        return Ok(&mut []);
    }
    if p.is_null() || !p.is_aligned() {
        return Err(OEX_ERROR_ARGUMENT);
    }
    if count
        .checked_mul(size_of::<T>())
        .filter(|n| isize::try_from(*n).is_ok())
        .is_none()
    {
        return Err(OEX_ERROR_RANGE);
    }
    Ok(unsafe { slice::from_raw_parts_mut(p, count) })
}
unsafe fn copy_buffer<T: Copy>(source: &[T], out: *mut T, capacity: u64) -> ApiResult {
    if capacity == 0 && out.is_null() {
        return Ok(());
    }
    if capacity < (source.len() as u64) {
        return Err(OEX_ERROR_RANGE);
    }
    let out = unsafe { output_slice(out, source.len() as u64)? };
    out.copy_from_slice(source);
    Ok(())
}
unsafe fn read_value<'a>(value: *const OexValue) -> ApiResult<&'a Value> {
    Ok(&unsafe { checked_value(value) }.ok_or(OEX_ERROR_ABI)?.value)
}
unsafe fn active(call: *mut OexCall) -> ApiResult {
    let c = unsafe { checked_call(call) }.ok_or(OEX_ERROR_ABI)?;
    if unsafe { &*c.cancellation.flag }.load(Ordering::Relaxed) {
        return Err(OEX_ERROR_CANCELLED);
    }
    Ok(())
}
unsafe fn context<'a>(call: *mut OexCall) -> ApiResult<&'a mut BuiltinContext<'a>> {
    unsafe { active(call)? };
    let call = unsafe { checked_call(call) }.ok_or(OEX_ERROR_ABI)?;
    unsafe { call.context.cast::<BuiltinContext<'a>>().as_mut() }.ok_or(OEX_ERROR_ABI)
}
unsafe fn language_copy(call: *mut OexCall, value: &Value) -> ApiResult<Value> {
    unsafe { context(call)? }
        .language_copy(value)
        .map_err(|e| native_object_error_status(&e))
}
unsafe fn output(call: *mut OexCall, out: *mut *mut OexValue, value: Value) -> ApiResult {
    require_out(out)?;
    unsafe { active(call)? };
    unsafe { out.write(Box::into_raw(Box::new(ValueHandle::owned(value))).cast()) };
    Ok(())
}
unsafe fn edit(
    call: *mut OexCall,
    value: *mut OexValue,
    f: impl FnOnce(&mut Value) -> ApiResult,
) -> ApiResult {
    unsafe { active(call)? };
    let original = unsafe { checked_value(value) }.ok_or(OEX_ERROR_ABI)?;
    if !original.is_owned() {
        return Err(OEX_ERROR_STATE);
    }
    let mut staged = original.value.clone();
    f(&mut staged)?;
    unsafe { active(call)? };
    unsafe { checked_value_mut(value) }
        .ok_or(OEX_ERROR_ABI)?
        .value = staged;
    Ok(())
}
unsafe fn text8<'a>(text: OexUtf8View) -> ApiResult<&'a str> {
    str::from_utf8(unsafe { input_slice(text.data.cast::<u8>(), text.length)? })
        .map_err(|_| OEX_ERROR_ENCODING)
}
fn utf8_view(text: &str) -> OexUtf8View {
    OexUtf8View {
        data: text.as_ptr().cast(),
        length: text.len() as u64,
    }
}
unsafe fn input_shape(rank: u32, dimensions: *const u64) -> ApiResult<Shape> {
    if rank < 2 || dimensions.is_null() {
        return Err(OEX_ERROR_DIMENSION);
    }
    let shape = Shape::new(
        unsafe { input_slice(dimensions, u64::from(rank))? }
            .iter()
            .copied(),
    )
    .map_err(array_status)?;
    if shape.ndims() != rank as usize {
        return Err(OEX_ERROR_DIMENSION);
    }
    Ok(shape)
}
fn range(start: u64, count: u64, total: u64) -> ApiResult<std::ops::Range<usize>> {
    let end = start
        .checked_add(count)
        .filter(|n| *n <= total)
        .ok_or(OEX_ERROR_RANGE)?;
    Ok(usize_count(start)?..usize_count(end)?)
}
#[expect(
    clippy::needless_pass_by_value,
    reason = "Used directly as a Result::map_err callback, which consumes its error."
)]
fn array_status(e: ArrayError) -> OexStatus {
    match e {
        ArrayError::HostLengthOverflow { .. } => OEX_ERROR_ALLOCATION,
        _ => OEX_ERROR_DIMENSION,
    }
}
fn aggregate_status(e: AggregateError) -> OexStatus {
    match e {
        AggregateError::Array(e) => array_status(e),
        AggregateError::FieldOutOfBounds { .. } | AggregateError::OffsetOutOfBounds { .. } => {
            OEX_ERROR_RANGE
        }
        AggregateError::FieldColumnCountMismatch { .. }
        | AggregateError::FieldColumnLengthMismatch { .. } => OEX_ERROR_DIMENSION,
        _ => OEX_ERROR_ARGUMENT,
    }
}
fn table_status(e: TableError) -> OexStatus {
    match e {
        TableError::Array(e) => array_status(e),
        TableError::VariableOutOfBounds { .. } => OEX_ERROR_RANGE,
        TableError::UnsupportedVariable { .. } => OEX_ERROR_TYPE,
        TableError::RowCountMismatch { .. }
        | TableError::RowNameCountMismatch { .. }
        | TableError::VariableCountMismatch { .. } => OEX_ERROR_DIMENSION,
        _ => OEX_ERROR_ARGUMENT,
    }
}
macro_rules! accessor {
    ($get:ident,$get_mut:ident,$variant:ident,$ty:ty) => {
        fn $get(value: &Value) -> ApiResult<&$ty> {
            if let Value::$variant(v) = value {
                Ok(v)
            } else {
                Err(OEX_ERROR_TYPE)
            }
        }
        fn $get_mut(value: &mut Value) -> ApiResult<&mut $ty> {
            if let Value::$variant(v) = value {
                Ok(v)
            } else {
                Err(OEX_ERROR_TYPE)
            }
        }
    };
}
accessor!(string, string_mut, String, StringValue);
accessor!(cell, cell_mut, Cell, CellArray);
accessor!(structure, structure_mut, Struct, StructArray);
accessor!(table, table_mut, Table, TableArray);
unsafe fn input_values(values: *const *const OexValue, count: u64) -> ApiResult<Vec<Value>> {
    let input = unsafe { input_slice(values, count)? };
    let mut values = Vec::new();
    reserve(&mut values, input.len())?;
    for v in input {
        values.push(unsafe { read_value(*v)? }.clone());
    }
    Ok(values)
}
unsafe fn parse_fields(names: *const OexUtf8View, count: u64) -> ApiResult<Vec<FieldName>> {
    unsafe { input_slice(names, count)? }
        .iter()
        .map(|n| FieldName::new(unsafe { text8(*n)? }).map_err(aggregate_status))
        .collect()
}
unsafe fn parse_variable_names(
    names: *const OexUtf8View,
    count: u64,
) -> ApiResult<Vec<TableVariableName>> {
    unsafe { input_slice(names, count)? }
        .iter()
        .map(|n| TableVariableName::new(unsafe { text8(*n)? }).map_err(table_status))
        .collect()
}
fn string_view(e: &StringElement) -> OexStringElementView {
    OexStringElementView {
        struct_size: abi_size::<OexStringElementView>(),
        is_missing: u32::from(e.is_missing()),
        data: e.code_units().as_ptr(),
        length: e.code_unit_len() as u64,
    }
}
unsafe fn string_elements16(
    elements: *const OexStringElementView,
    count: u64,
) -> ApiResult<Vec<StringElement>> {
    let input = unsafe { input_slice(elements, count)? };
    let mut values = Vec::new();
    reserve(&mut values, input.len())?;
    for e in input {
        if e.struct_size != abi_size::<OexStringElementView>() {
            return Err(OEX_ERROR_ABI);
        }
        if e.is_missing > 1 || (e.is_missing == 1 && e.length != 0) {
            return Err(OEX_ERROR_ARGUMENT);
        }
        values.push(if e.is_missing == 1 {
            StringElement::missing()
        } else {
            StringElement::from_code_units(unsafe { input_slice(e.data, e.length)? }.to_vec())
        });
    }
    Ok(values)
}
unsafe fn string_elements8(
    elements: *const OexUtf8View,
    missing: *const u8,
    count: u64,
) -> ApiResult<Vec<StringElement>> {
    let input = unsafe { input_slice(elements, count)? };
    let mask = if missing.is_null() {
        None
    } else {
        Some(unsafe { input_slice(missing, count)? })
    };
    let mut values = Vec::new();
    reserve(&mut values, input.len())?;
    for (i, e) in input.iter().enumerate() {
        let m = mask.map_or(0, |v| v[i]);
        if m > 1 || (m == 1 && e.length != 0) {
            return Err(OEX_ERROR_ARGUMENT);
        }
        values.push(if m == 1 {
            StringElement::missing()
        } else {
            StringElement::from_utf8(unsafe { text8(*e)? })
        });
    }
    Ok(values)
}
fn string_elements_owned(string: &StringValue) -> ApiResult<Vec<StringElement>> {
    let count = usize_count(string.numel())?;
    let mut values = Vec::new();
    reserve(&mut values, count)?;
    for i in 0..count {
        values.push(string.element(i).unwrap().clone());
    }
    Ok(values)
}
unsafe fn set_string(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    element: StringElement,
) -> ApiResult {
    unsafe {
        edit(call, value, |v| {
            let s = string_mut(v)?;
            if index >= s.numel() {
                return Err(OEX_ERROR_RANGE);
            }
            match s {
                StringValue::Scalar(v) => *v = element,
                StringValue::Array(v) => {
                    v.replace_linear(index + 1, element).map_err(array_status)?;
                }
            }
            Ok(())
        })
    }
}
pub(super) unsafe extern "C" fn value_get_dimensions(
    value: *const OexValue,
    capacity: u32,
    dimensions: *mut u64,
    rank: *mut u32,
) -> OexStatus {
    boundary(|| unsafe {
        let dims = read_value(value)?.dimensions().ok_or(OEX_ERROR_TYPE)?;
        write_out(
            rank,
            u32::try_from(dims.len()).map_err(|_| OEX_ERROR_DIMENSION)?,
        )?;
        copy_buffer(dims, dimensions, u64::from(capacity))
    })
}
pub(super) unsafe extern "C" fn value_get_class_name(
    call: *mut OexCall,
    value: *const OexValue,
    name: *mut OexUtf8View,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(name)?;
        let v = read_value(value)?.clone();
        let text = context(call)?
            .language_class_name(&v)
            .map_err(|e| native_object_error_status(&e))?;
        let call = checked_call_mut(call).ok_or(OEX_ERROR_ABI)?;
        call.text_buffers.push(text.into_boxed_str());
        write_out(name, utf8_view(call.text_buffers.last().unwrap()))
    })
}
pub(super) unsafe extern "C" fn string_create(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let shape = input_shape(rank, dimensions)?;
        let values = vec_filled(usize_count(shape.numel())?, StringElement::default())?;
        output(
            call,
            result,
            Value::from(StringArray::from_elements(shape, values).map_err(array_status)?),
        )
    })
}
pub(super) unsafe extern "C" fn string_create_from_utf16(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    count: u64,
    elements: *const OexStringElementView,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let shape = input_shape(rank, dimensions)?;
        if shape.numel() != count {
            return Err(OEX_ERROR_DIMENSION);
        }
        let elements = string_elements16(elements, count)?;
        output(
            call,
            result,
            Value::from(StringArray::from_elements(shape, elements).map_err(array_status)?),
        )
    })
}
pub(super) unsafe extern "C" fn string_set_elements_utf16(
    call: *mut OexCall,
    value: *mut OexValue,
    start: u64,
    count: u64,
    elements: *const OexStringElementView,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let elements = string_elements16(elements, count)?;
        edit(call, value, |v| {
            let string = string_mut(v)?;
            let range = range(start, count, string.numel())?;
            let shape = Shape::new(string.dimensions().iter().copied()).map_err(array_status)?;
            let mut values = string_elements_owned(string)?;
            values[range].clone_from_slice(&elements);
            *string =
                StringValue::from(StringArray::from_elements(shape, values).map_err(array_status)?);
            Ok(())
        })
    })
}
pub(super) unsafe extern "C" fn string_create_from_utf8(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    count: u64,
    elements: *const OexUtf8View,
    missing: *const u8,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let shape = input_shape(rank, dimensions)?;
        if shape.numel() != count {
            return Err(OEX_ERROR_DIMENSION);
        }
        let elements = string_elements8(elements, missing, count)?;
        output(
            call,
            result,
            Value::from(StringArray::from_elements(shape, elements).map_err(array_status)?),
        )
    })
}
pub(super) unsafe extern "C" fn string_set_elements_utf8(
    call: *mut OexCall,
    value: *mut OexValue,
    start: u64,
    count: u64,
    elements: *const OexUtf8View,
    missing: *const u8,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let elements = string_elements8(elements, missing, count)?;
        edit(call, value, |v| {
            let string = string_mut(v)?;
            let range = range(start, count, string.numel())?;
            let shape = Shape::new(string.dimensions().iter().copied()).map_err(array_status)?;
            let mut values = string_elements_owned(string)?;
            values[range].clone_from_slice(&elements);
            *string =
                StringValue::from(StringArray::from_elements(shape, values).map_err(array_status)?);
            Ok(())
        })
    })
}
pub(super) unsafe extern "C" fn string_get_element_view(
    value: *const OexValue,
    index: u64,
    view: *mut OexStringElementView,
) -> OexStatus {
    boundary(|| unsafe {
        let element = string(read_value(value)?)?
            .element(usize_count(index)?)
            .ok_or(OEX_ERROR_RANGE)?;
        write_out(view, string_view(element))
    })
}
pub(super) unsafe extern "C" fn string_get_elements(
    value: *const OexValue,
    start: u64,
    count: u64,
    views: *mut OexStringElementView,
) -> OexStatus {
    boundary(|| unsafe {
        let string = string(read_value(value)?)?;
        let range = range(start, count, string.numel())?;
        let out = output_slice(views, count)?;
        for (slot, index) in out.iter_mut().zip(range) {
            *slot = string_view(string.element(index).unwrap());
        }
        Ok(())
    })
}
pub(super) unsafe extern "C" fn string_copy_element_utf8(
    value: *const OexValue,
    index: u64,
    buffer: *mut c_char,
    capacity: u64,
    required: *mut u64,
    is_missing: *mut u32,
) -> OexStatus {
    boundary(|| unsafe {
        require_out(required)?;
        require_out(is_missing)?;
        let element = string(read_value(value)?)?
            .element(usize_count(index)?)
            .ok_or(OEX_ERROR_RANGE)?;
        write_out(is_missing, u32::from(element.is_missing()))?;
        let text = String::from_utf16(element.code_units()).map_err(|_| OEX_ERROR_ENCODING)?;
        write_out(required, text.len() as u64)?;
        copy_buffer(text.as_bytes(), buffer.cast::<u8>(), capacity)
    })
}
pub(super) unsafe extern "C" fn string_set_element_utf8(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    text: OexUtf8View,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let element = StringElement::from_utf8(text8(text)?);
        set_string(call, value, index, element)
    })
}
pub(super) unsafe extern "C" fn string_set_element_utf16(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    text: OexUtf16View,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let element = StringElement::from_code_units(input_slice(text.data, text.length)?.to_vec());
        set_string(call, value, index, element)
    })
}
pub(super) unsafe extern "C" fn string_set_missing(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        set_string(call, value, index, StringElement::missing())
    })
}
pub(super) unsafe extern "C" fn cell_create(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let shape = input_shape(rank, dimensions)?;
        output(
            call,
            result,
            Value::Cell(
                CellArray::from_values(
                    shape.clone(),
                    vec_filled(usize_count(shape.numel())?, Value::empty_double())?,
                )
                .map_err(aggregate_status)?,
            ),
        )
    })
}
pub(super) unsafe extern "C" fn cell_get_element(
    call: *mut OexCall,
    value: *const OexValue,
    index: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let v = cell(read_value(value)?)?
            .value_at_offset(usize_count(index)?)
            .ok_or(OEX_ERROR_RANGE)?
            .clone();
        let v = language_copy(call, &v)?;
        output(call, result, v)
    })
}
pub(super) unsafe extern "C" fn cell_set_element(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    element: *const OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let source = read_value(element)?.clone();
        edit(call, value, |v| {
            let c = cell_mut(v)?;
            let offset = usize_count(index)?;
            if offset >= c.values().len() {
                return Err(OEX_ERROR_RANGE);
            }
            c.replace_at_offset(offset, language_copy(call, &source)?)
                .map_err(aggregate_status)?;
            Ok(())
        })
    })
}
pub(super) unsafe extern "C" fn cell_get_elements(
    call: *mut OexCall,
    value: *const OexValue,
    start: u64,
    count: u64,
    results: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let c = cell(read_value(value)?)?;
        let range = range(start, count, c.numel())?;
        let sources = c.values()[range].to_vec();
        let mut values = Vec::new();
        reserve(&mut values, sources.len())?;
        for v in &sources {
            values.push(Box::new(ValueHandle::owned(language_copy(call, v)?)));
        }
        let outputs = output_slice(results, count)?;
        for (slot, value) in outputs.iter_mut().zip(values) {
            *slot = Box::into_raw(value).cast();
        }
        Ok(())
    })
}
pub(super) unsafe extern "C" fn cell_set_elements(
    call: *mut OexCall,
    value: *mut OexValue,
    start: u64,
    count: u64,
    elements: *const *const OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let sources = input_values(elements, count)?;
        edit(call, value, |v| {
            let c = cell_mut(v)?;
            let range = range(start, count, c.numel())?;
            for (index, source) in range.zip(&sources) {
                c.replace_at_offset(index, language_copy(call, source)?)
                    .map_err(aggregate_status)?;
            }
            Ok(())
        })
    })
}
pub(super) unsafe extern "C" fn struct_create(
    call: *mut OexCall,
    rank: u32,
    dimensions: *const u64,
    field_count: u64,
    field_names: *const OexUtf8View,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let shape = input_shape(rank, dimensions)?;
        let fields = parse_fields(field_names, field_count)?;
        let mut columns = Vec::new();
        reserve(&mut columns, fields.len())?;
        let count = usize_count(shape.numel())?;
        for _ in &fields {
            active(call)?;
            columns.push(vec_filled(count, Value::empty_double())?);
        }
        output(
            call,
            result,
            Value::Struct(
                StructArray::from_columns(shape, fields, columns).map_err(aggregate_status)?,
            ),
        )
    })
}
pub(super) unsafe extern "C" fn struct_get_field_count(
    value: *const OexValue,
    count: *mut u64,
) -> OexStatus {
    boundary(|| unsafe { write_out(count, structure(read_value(value)?)?.field_count() as u64) })
}
pub(super) unsafe extern "C" fn struct_get_field_name(
    value: *const OexValue,
    field_index: u64,
    name: *mut OexUtf8View,
) -> OexStatus {
    boundary(|| unsafe {
        let field = structure(read_value(value)?)?
            .field_names()
            .get(usize_count(field_index)?)
            .ok_or(OEX_ERROR_RANGE)?;
        write_out(name, utf8_view(field.as_str()))
    })
}
pub(super) unsafe extern "C" fn struct_find_field(
    value: *const OexValue,
    name: OexUtf8View,
    index: *mut u64,
) -> OexStatus {
    boundary(|| unsafe {
        let index_value = structure(read_value(value)?)?
            .field_index(text8(name)?)
            .ok_or(OEX_ERROR_NOT_FOUND)?;
        write_out(index, index_value as u64)
    })
}
pub(super) unsafe extern "C" fn struct_get_field(
    call: *mut OexCall,
    value: *const OexValue,
    element_index: u64,
    field_index: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let v = structure(read_value(value)?)?
            .value_at(usize_count(field_index)?, usize_count(element_index)?)
            .ok_or(OEX_ERROR_RANGE)?
            .clone();
        let v = language_copy(call, &v)?;
        output(call, result, v)
    })
}
pub(super) unsafe extern "C" fn struct_set_field(
    call: *mut OexCall,
    value: *mut OexValue,
    element_index: u64,
    field_index: u64,
    element: *const OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let source = read_value(element)?.clone();
        edit(call, value, |v| {
            let s = structure_mut(v)?;
            let field = usize_count(field_index)?;
            let index = usize_count(element_index)?;
            if s.value_at(field, index).is_none() {
                return Err(OEX_ERROR_RANGE);
            }
            s.replace_at(field, index, language_copy(call, &source)?)
                .map_err(aggregate_status)?;
            Ok(())
        })
    })
}
pub(super) unsafe extern "C" fn struct_add_field(
    call: *mut OexCall,
    value: *mut OexValue,
    name: OexUtf8View,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let name = FieldName::new(text8(name)?).map_err(aggregate_status)?;
        edit(call, value, |v| {
            structure_mut(v)?
                .append_empty_field(name)
                .map_err(aggregate_status)
        })
    })
}
pub(super) unsafe extern "C" fn struct_remove_field(
    call: *mut OexCall,
    value: *mut OexValue,
    field_index: u64,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        edit(call, value, |v| {
            structure_mut(v)?
                .remove_field(usize_count(field_index)?)
                .map_err(aggregate_status)
        })
    })
}
pub(super) unsafe extern "C" fn struct_set_field_names(
    call: *mut OexCall,
    value: *mut OexValue,
    count: u64,
    names: *const OexUtf8View,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let names = parse_fields(names, count)?;
        edit(call, value, |v| {
            structure_mut(v)?
                .rename_fields(names)
                .map_err(aggregate_status)
        })
    })
}
pub(super) unsafe extern "C" fn struct_get_field_values(
    call: *mut OexCall,
    value: *const OexValue,
    field_index: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let s = structure(read_value(value)?)?;
        let shape = s.shape().clone();
        let sources = s
            .field_values(usize_count(field_index)?)
            .ok_or(OEX_ERROR_RANGE)?
            .to_vec();
        let mut values = Vec::new();
        reserve(&mut values, sources.len())?;
        for v in &sources {
            values.push(language_copy(call, v)?);
        }
        output(
            call,
            result,
            Value::Cell(CellArray::from_values(shape, values).map_err(aggregate_status)?),
        )
    })
}
pub(super) unsafe extern "C" fn struct_set_field_values(
    call: *mut OexCall,
    value: *mut OexValue,
    field_index: u64,
    elements: *const OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let source = cell(read_value(elements)?)?.clone();
        edit(call, value, |v| {
            let s = structure_mut(v)?;
            let field = usize_count(field_index)?;
            if field >= s.field_count() {
                return Err(OEX_ERROR_RANGE);
            }
            if s.shape() != source.shape() {
                return Err(OEX_ERROR_DIMENSION);
            }
            for (i, element) in source.values().iter().enumerate() {
                s.replace_at(field, i, language_copy(call, element)?)
                    .map_err(aggregate_status)?;
            }
            Ok(())
        })
    })
}
pub(super) unsafe extern "C" fn table_create(
    call: *mut OexCall,
    row_count: u64,
    variable_count: u64,
    variable_names: *const OexUtf8View,
    variables: *const *const OexValue,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let names = parse_variable_names(variable_names, variable_count)?;
        let sources = input_values(variables, variable_count)?;
        let mut table = TableArray::from_parts(row_count, names, sources).map_err(table_status)?;
        for i in 0..table.variable_count() {
            let v = language_copy(call, table.variable(i).unwrap())?;
            table.replace_variable(i, v).map_err(table_status)?;
        }
        output(call, result, Value::Table(table))
    })
}
pub(super) unsafe extern "C" fn table_get_row_count(
    value: *const OexValue,
    count: *mut u64,
) -> OexStatus {
    boundary(|| unsafe { write_out(count, table(read_value(value)?)?.row_count()) })
}
pub(super) unsafe extern "C" fn table_get_variable_count(
    value: *const OexValue,
    count: *mut u64,
) -> OexStatus {
    boundary(|| unsafe { write_out(count, table(read_value(value)?)?.variable_count() as u64) })
}
pub(super) unsafe extern "C" fn table_get_variable_name(
    value: *const OexValue,
    index: u64,
    name: *mut OexUtf8View,
) -> OexStatus {
    boundary(|| unsafe {
        let name_value = table(read_value(value)?)?
            .variable_names()
            .get(usize_count(index)?)
            .ok_or(OEX_ERROR_RANGE)?;
        write_out(name, utf8_view(name_value.as_str()))
    })
}
pub(super) unsafe extern "C" fn table_find_variable(
    value: *const OexValue,
    name: OexUtf8View,
    index: *mut u64,
) -> OexStatus {
    boundary(|| unsafe {
        let i = table(read_value(value)?)?
            .variable_index(text8(name)?)
            .ok_or(OEX_ERROR_NOT_FOUND)?;
        write_out(index, i as u64)
    })
}
pub(super) unsafe extern "C" fn table_set_variable_names(
    call: *mut OexCall,
    value: *mut OexValue,
    count: u64,
    names: *const OexUtf8View,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let names = parse_variable_names(names, count)?;
        edit(call, value, |v| {
            table_mut(v)?.rename_variables(names).map_err(table_status)
        })
    })
}
pub(super) unsafe extern "C" fn table_get_variable(
    call: *mut OexCall,
    value: *const OexValue,
    index: u64,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        let v = table(read_value(value)?)?
            .variable(usize_count(index)?)
            .ok_or(OEX_ERROR_RANGE)?
            .clone();
        let v = language_copy(call, &v)?;
        output(call, result, v)
    })
}
pub(super) unsafe extern "C" fn table_set_variable(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
    variable: *const OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let source = read_value(variable)?.clone();
        edit(call, value, |v| {
            let t = table_mut(v)?;
            let i = usize_count(index)?;
            if i >= t.variable_count() {
                return Err(OEX_ERROR_RANGE);
            }
            if source.dimensions().and_then(|d| d.first()).copied() != Some(t.row_count()) {
                return Err(OEX_ERROR_DIMENSION);
            }
            t.replace_variable(i, language_copy(call, &source)?)
                .map_err(table_status)?;
            Ok(())
        })
    })
}
pub(super) unsafe extern "C" fn table_append_variable(
    call: *mut OexCall,
    value: *mut OexValue,
    name: OexUtf8View,
    variable: *const OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let name = TableVariableName::new(text8(name)?).map_err(table_status)?;
        let source = read_value(variable)?.clone();
        edit(call, value, |v| {
            let t = table_mut(v)?;
            if source.dimensions().and_then(|d| d.first()).copied() != Some(t.row_count()) {
                return Err(OEX_ERROR_DIMENSION);
            }
            if t.variable_index(name.as_str()).is_some() {
                return Err(OEX_ERROR_ARGUMENT);
            }
            t.append_variable(name, language_copy(call, &source)?)
                .map_err(table_status)
        })
    })
}
pub(super) unsafe extern "C" fn table_remove_variable(
    call: *mut OexCall,
    value: *mut OexValue,
    index: u64,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        edit(call, value, |v| {
            table_mut(v)?
                .remove_variable(usize_count(index)?)
                .map_err(table_status)?;
            Ok(())
        })
    })
}
pub(super) unsafe extern "C" fn table_get_row_names(
    call: *mut OexCall,
    value: *const OexValue,
    has_names: *mut u32,
    result: *mut *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        require_out(result)?;
        require_out(has_names)?;
        let t = table(read_value(value)?)?;
        let present = t.row_names().is_some();
        let elements = t
            .row_names()
            .unwrap_or(&[])
            .iter()
            .map(|n| StringElement::from_utf8(n.as_str()))
            .collect::<Vec<_>>();
        let shape = Shape::new([elements.len() as u64, 1]).map_err(array_status)?;
        let v = Value::from(StringArray::from_elements(shape, elements).map_err(array_status)?);
        output(call, result, v)?;
        write_out(has_names, u32::from(present))
    })
}
pub(super) unsafe extern "C" fn table_set_row_names(
    call: *mut OexCall,
    value: *mut OexValue,
    count: u64,
    names: *const OexUtf8View,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        let names = input_slice(names, count)?
            .iter()
            .map(|n| TableRowName::new(text8(*n)?).map_err(table_status))
            .collect::<Result<Vec<_>, _>>()?;
        edit(call, value, |v| {
            table_mut(v)?.set_row_names(names).map_err(table_status)
        })
    })
}
pub(super) unsafe extern "C" fn table_clear_row_names(
    call: *mut OexCall,
    value: *mut OexValue,
) -> OexStatus {
    boundary(|| unsafe {
        active(call)?;
        edit(call, value, |v| {
            table_mut(v)?.clear_row_names();
            Ok(())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::*;
    use crate::host::new_call;
    use openmat_runtime::{CancellationToken, NullOutput};
    use std::ptr;

    #[test]
    fn rejected_and_cancelled_writes_preserve_owned_and_borrowed_values() {
        let cancellation = CancellationToken::new();
        let mut sink = NullOutput;
        let mut context = BuiltinContext::new(0, &cancellation, &mut sink);
        let mut call = new_call(Vec::new(), 0, &mut context);
        let raw_call = ptr::from_mut(&mut call).cast();
        let initial = Value::String(StringValue::Scalar(StringElement::from_utf8("original")));
        let mut owned = ValueHandle::owned(initial.clone());
        let mut borrowed = ValueHandle::borrowed(initial.clone());
        let raw_owned = ptr::from_mut(&mut owned).cast();
        let raw_borrowed = ptr::from_mut(&mut borrowed).cast();

        // SAFETY: Handles and buffers are live, aligned locals throughout each call.
        unsafe {
            assert_eq!(
                OexString_SetMissing(raw_call, raw_borrowed, 0),
                OEX_ERROR_STATE
            );
            assert_eq!(
                OexString_SetMissing(raw_call, raw_owned, u64::MAX),
                OEX_ERROR_RANGE
            );
            assert_eq!(owned.value, initial);
            assert_eq!(borrowed.value, initial);

            cancellation.cancel();
            assert_eq!(
                OexString_SetMissing(raw_call, raw_owned, 0),
                OEX_ERROR_CANCELLED
            );
            assert_eq!(owned.value, initial);
            let mut output = raw_owned;
            assert_eq!(
                OexCell_Create(raw_call, 2, [1, 1].as_ptr(), &raw mut output),
                OEX_ERROR_CANCELLED
            );
            assert!(output.is_null());
        }
    }

    #[test]
    fn invalid_inputs_clear_owned_outputs_without_partial_buffer_writes() {
        let cancellation = CancellationToken::new();
        let mut sink = NullOutput;
        let mut context = BuiltinContext::new(0, &cancellation, &mut sink);
        let mut call = new_call(Vec::new(), 0, &mut context);
        let raw_call = ptr::from_mut(&mut call).cast();
        let mut sentinel = ValueHandle::owned(Value::empty_double());
        let raw_sentinel = ptr::from_mut(&mut sentinel).cast();

        // SAFETY: All pointers refer to live local buffers/handles; NULL is tested only
        // where the public C API specifies validation or an empty slice/query.
        unsafe {
            for dimensions in [&[1_u64][..], &[1, 1, 1][..], &[u64::MAX, 2][..]] {
                let mut result = raw_sentinel;
                assert_eq!(
                    OexCell_Create(
                        raw_call,
                        u32::try_from(dimensions.len()).expect("test rank fits u32"),
                        dimensions.as_ptr(),
                        &raw mut result
                    ),
                    OEX_ERROR_DIMENSION
                );
                assert!(result.is_null());
            }
            let invalid = OexUtf8View {
                data: [0xff_u8].as_ptr().cast(),
                length: 1,
            };
            let mut result = raw_sentinel;
            assert_eq!(
                OexString_CreateFromUtf8(
                    raw_call,
                    2,
                    [1, 1].as_ptr(),
                    1,
                    &raw const invalid,
                    ptr::null(),
                    &raw mut result
                ),
                OEX_ERROR_ENCODING
            );
            assert!(result.is_null());
            assert_eq!(
                OexCell_Create(raw_call, 2, [1, 1].as_ptr(), ptr::null_mut()),
                OEX_ERROR_ARGUMENT
            );

            assert_eq!(
                OexCell_Create(raw_call, 2, [0, 3].as_ptr(), &raw mut result),
                OEX_OK
            );
            let mut rank = 0;
            let mut dimensions = [42_u64; 2];
            assert_eq!(
                OexValue_GetDimensions(result, 1, dimensions.as_mut_ptr(), &raw mut rank),
                OEX_ERROR_RANGE
            );
            assert_eq!(rank, 2);
            assert_eq!(dimensions, [42; 2]);
            assert_eq!(
                OexCell_GetElements(raw_call, result, 0, 0, ptr::null_mut()),
                OEX_OK
            );
            let mut outputs = [raw_sentinel; 2];
            assert_eq!(
                OexCell_GetElements(raw_call, result, u64::MAX, 2, outputs.as_mut_ptr()),
                OEX_ERROR_RANGE
            );
            assert!(outputs.iter().all(|p| p.is_null()));
            OexValue_Release(result);
        }
    }
}

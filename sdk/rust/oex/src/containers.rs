//! String and heterogeneous container access, with callback-scoped ownership.
use crate::array::read_slice;
use crate::error::{check, count, length};
use crate::{Call, Error, ErrorKind, Result, Value, ValueRef, sys};
use std::ptr;

/// An exact string element. Missing is independent of an empty code-unit slice.
#[derive(Clone, Copy, Debug)]
pub struct StringElementRef<'a> {
    pub code_units: &'a [u16],
    pub is_missing: bool,
}
fn utf8(s: &str) -> sys::OexUtf8View {
    sys::OexUtf8View {
        data: s.as_ptr().cast(),
        length: s.len() as u64,
    }
}
fn names(values: &[&str]) -> Vec<sys::OexUtf8View> {
    values.iter().map(|s| utf8(s)).collect()
}
fn pointers(values: &[ValueRef<'_, '_>]) -> Vec<*const sys::OexValue> {
    values.iter().map(|v| v.as_raw()).collect()
}
unsafe fn value_out<'call>(
    op: &'static str,
    f: impl FnOnce(*mut *mut sys::OexValue) -> u32,
) -> Result<Value<'call>> {
    let mut out = ptr::null_mut();
    let status = f(&mut out);
    // SAFETY: each caller supplies an owned-output C API with callback lifetime.
    unsafe { Value::from_out(status, out, op) }
}
unsafe fn text_view<'a>(v: sys::OexUtf8View) -> Result<&'a str> {
    // SAFETY: callers borrow the value holding these bytes for the result lifetime.
    let bytes = unsafe { read_slice(v.data.cast::<u8>(), length(v.length)?)? };
    std::str::from_utf8(bytes)
        .map_err(|_| Error::with_kind(ErrorKind::Encoding, "invalid UTF-8 from host"))
}
fn string_inputs16(values: &[Option<&[u16]>]) -> Vec<sys::OexStringElementView> {
    values
        .iter()
        .map(|v| sys::OexStringElementView {
            struct_size: sys::abi_size::<sys::OexStringElementView>(),
            is_missing: u32::from(v.is_none()),
            data: v.unwrap_or(&[]).as_ptr(),
            length: v.unwrap_or(&[]).len() as u64,
        })
        .collect()
}
fn string_inputs8(values: &[Option<&str>]) -> (Vec<sys::OexUtf8View>, Vec<u8>) {
    (
        values.iter().map(|v| utf8(v.unwrap_or(""))).collect(),
        values.iter().map(|v| u8::from(v.is_none())).collect(),
    )
}
impl<'a, 'call> ValueRef<'a, 'call> {
    pub fn dimensions(self) -> Result<Vec<u64>> {
        let mut rank = 0;
        check(
            // SAFETY: live value and a writable rank; zero capacity requests length only.
            unsafe { sys::OexValue_GetDimensions(self.as_raw(), 0, ptr::null_mut(), &mut rank) },
            "OexValue_GetDimensions",
        )?;
        let mut dims = vec![0; rank as usize];
        check(
            // SAFETY: capacity matches the writable dimensions vector.
            unsafe {
                sys::OexValue_GetDimensions(self.as_raw(), rank, dims.as_mut_ptr(), &mut rank)
            },
            "OexValue_GetDimensions",
        )?;
        Ok(dims)
    }
    pub fn class_name(self, call: &Call<'call>) -> Result<String> {
        let mut view = sys::OexUtf8View {
            data: ptr::null(),
            length: 0,
        };
        check(
            // SAFETY: call and value are live; class text is copied before returning.
            unsafe { sys::OexValue_GetClassName(call.as_raw(), self.as_raw(), &mut view) },
            "OexValue_GetClassName",
        )?;
        // SAFETY: host keeps the view alive for this callback.
        Ok(unsafe { text_view(view)? }.to_owned())
    }
    pub fn string_element(self, index: u64) -> Result<StringElementRef<'a>> {
        let mut view = sys::OexStringElementView::default();
        check(
            // SAFETY: immutable value borrow keeps the exact UTF-16 payload live.
            unsafe { sys::OexString_GetElementView(self.as_raw(), index, &mut view) },
            "OexString_GetElementView",
        )?;
        Ok(StringElementRef {
            // SAFETY: host returned initialized code units owned by the borrowed value.
            code_units: unsafe { read_slice(view.data, length(view.length)?)? },
            is_missing: view.is_missing != 0,
        })
    }
    pub fn string_elements(self, start: u64, count: usize) -> Result<Vec<StringElementRef<'a>>> {
        let mut views = vec![sys::OexStringElementView::default(); count];
        check(
            // SAFETY: descriptor buffer has exactly count slots; value remains borrowed.
            unsafe {
                sys::OexString_GetElements(self.as_raw(), start, count as u64, views.as_mut_ptr())
            },
            "OexString_GetElements",
        )?;
        views
            .into_iter()
            .map(|v| {
                Ok(StringElementRef {
                    // SAFETY: all code-unit views borrow the unchanged source value.
                    code_units: unsafe { read_slice(v.data, length(v.length)?)? },
                    is_missing: v.is_missing != 0,
                })
            })
            .collect()
    }
    pub fn string_utf8(self, index: u64) -> Result<Option<String>> {
        let mut required = 0;
        let mut missing = 0;
        check(
            // SAFETY: size-only query; missing is reported independently.
            unsafe {
                sys::OexString_CopyElementUtf8(
                    self.as_raw(),
                    index,
                    ptr::null_mut(),
                    0,
                    &mut required,
                    &mut missing,
                )
            },
            "OexString_CopyElementUtf8",
        )?;
        if missing != 0 {
            return Ok(None);
        }
        let mut bytes = vec![0u8; length(required)?];
        check(
            // SAFETY: byte buffer is writable for its stated length.
            unsafe {
                sys::OexString_CopyElementUtf8(
                    self.as_raw(),
                    index,
                    bytes.as_mut_ptr().cast(),
                    required,
                    &mut required,
                    &mut missing,
                )
            },
            "OexString_CopyElementUtf8",
        )?;
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| Error::with_kind(ErrorKind::Encoding, "invalid UTF-8 from host"))
    }
    pub fn cell_elements(
        self,
        call: &Call<'call>,
        start: u64,
        count: usize,
    ) -> Result<Vec<Value<'call>>> {
        let mut outputs = vec![ptr::null_mut(); count];
        // SAFETY: all output slots are writable; GetElements transfers ownership.
        let status = unsafe {
            sys::OexCell_GetElements(
                call.as_raw(),
                self.as_raw(),
                start,
                count as u64,
                outputs.as_mut_ptr(),
            )
        };
        let values: Result<Vec<_>> = outputs
            .into_iter()
            .map(|out| {
                // SAFETY: adopt all returned outputs, even when the operation failed.
                unsafe { Value::from_out(status, out, "OexCell_GetElements") }
            })
            .collect();
        check(status, "OexCell_GetElements")?;
        values
    }
    pub fn table_row_names(self, call: &Call<'call>) -> Result<Option<Value<'call>>> {
        let mut present = 0;
        // SAFETY: getter returns a new owned string array; presence is a separate flag.
        let value = unsafe {
            value_out("OexTable_GetRowNames", |out| {
                sys::OexTable_GetRowNames(call.as_raw(), self.as_raw(), &mut present, out)
            })
        }?;
        Ok((present != 0).then_some(value))
    }
}

impl<'call> Call<'call> {
    pub fn strings(&self, dimensions: &[u64]) -> Result<Value<'call>> {
        let rank = count(dimensions.len())?;
        // SAFETY: shape slice and owned output are valid for this synchronous call.
        unsafe {
            value_out("OexString_Create", |out| {
                sys::OexString_Create(self.as_raw(), rank, dimensions.as_ptr(), out)
            })
        }
    }
    pub fn strings_utf8(
        &self,
        dimensions: &[u64],
        elements: &[Option<&str>],
    ) -> Result<Value<'call>> {
        let rank = count(dimensions.len())?;
        let (views, missing) = string_inputs8(elements);
        // SAFETY: descriptors borrow live input strings and mask; host copies contents.
        unsafe {
            value_out("OexString_CreateFromUtf8", |out| {
                sys::OexString_CreateFromUtf8(
                    self.as_raw(),
                    rank,
                    dimensions.as_ptr(),
                    elements.len() as u64,
                    views.as_ptr(),
                    missing.as_ptr(),
                    out,
                )
            })
        }
    }
    pub fn strings_utf16(
        &self,
        dimensions: &[u64],
        elements: &[Option<&[u16]>],
    ) -> Result<Value<'call>> {
        let rank = count(dimensions.len())?;
        let views = string_inputs16(elements);
        // SAFETY: descriptors borrow live code units; host copies exact data.
        unsafe {
            value_out("OexString_CreateFromUtf16", |out| {
                sys::OexString_CreateFromUtf16(
                    self.as_raw(),
                    rank,
                    dimensions.as_ptr(),
                    elements.len() as u64,
                    views.as_ptr(),
                    out,
                )
            })
        }
    }
    pub fn string(&self, text: &str) -> Result<Value<'call>> {
        self.strings_utf8(&[1, 1], &[Some(text)])
    }
    pub fn cell(&self, dimensions: &[u64]) -> Result<Value<'call>> {
        let rank = count(dimensions.len())?;
        // SAFETY: shape slice is live; constructor returns ownership.
        unsafe {
            value_out("OexCell_Create", |out| {
                sys::OexCell_Create(self.as_raw(), rank, dimensions.as_ptr(), out)
            })
        }
    }
    pub fn structure(&self, dimensions: &[u64], fields: &[&str]) -> Result<Value<'call>> {
        let rank = count(dimensions.len())?;
        let fields = names(fields);
        // SAFETY: names and shape remain live; constructor copies inputs.
        unsafe {
            value_out("OexStruct_Create", |out| {
                sys::OexStruct_Create(
                    self.as_raw(),
                    rank,
                    dimensions.as_ptr(),
                    fields.len() as u64,
                    fields.as_ptr(),
                    out,
                )
            })
        }
    }
    pub fn table(
        &self,
        rows: u64,
        variables: &[(&str, ValueRef<'_, 'call>)],
    ) -> Result<Value<'call>> {
        let names: Vec<_> = variables.iter().map(|(name, _)| utf8(name)).collect();
        let values: Vec<_> = variables.iter().map(|(_, v)| v.as_raw()).collect();
        // SAFETY: descriptor and value arrays have the same length and remain borrowed.
        unsafe {
            value_out("OexTable_Create", |out| {
                sys::OexTable_Create(
                    self.as_raw(),
                    rows,
                    variables.len() as u64,
                    names.as_ptr(),
                    values.as_ptr(),
                    out,
                )
            })
        }
    }
}
impl<'call> Value<'call> {
    pub fn set_string_utf8(&mut self, call: &Call<'call>, index: u64, text: &str) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow, borrowed UTF-8 input, synchronous call.
            unsafe {
                sys::OexString_SetElementUtf8(call.as_raw(), self.as_raw(), index, utf8(text))
            },
            "OexString_SetElementUtf8",
        )
    }
    pub fn set_string_utf16(&mut self, call: &Call<'call>, index: u64, text: &[u16]) -> Result<()> {
        check(
            // SAFETY: host copies the borrowed code-unit slice before returning.
            unsafe {
                sys::OexString_SetElementUtf16(
                    call.as_raw(),
                    self.as_raw(),
                    index,
                    sys::OexUtf16View {
                        data: text.as_ptr(),
                        length: text.len() as u64,
                    },
                )
            },
            "OexString_SetElementUtf16",
        )
    }
    pub fn set_strings_utf8(
        &mut self,
        call: &Call<'call>,
        start: u64,
        elements: &[Option<&str>],
    ) -> Result<()> {
        let (views, missing) = string_inputs8(elements);
        check(
            // SAFETY: descriptors and mask remain valid; target is exclusively borrowed.
            unsafe {
                sys::OexString_SetElementsUtf8(
                    call.as_raw(),
                    self.as_raw(),
                    start,
                    elements.len() as u64,
                    views.as_ptr(),
                    missing.as_ptr(),
                )
            },
            "OexString_SetElementsUtf8",
        )
    }
    pub fn set_strings_utf16(
        &mut self,
        call: &Call<'call>,
        start: u64,
        elements: &[Option<&[u16]>],
    ) -> Result<()> {
        let views = string_inputs16(elements);
        check(
            // SAFETY: descriptors remain valid; target is exclusively borrowed.
            unsafe {
                sys::OexString_SetElementsUtf16(
                    call.as_raw(),
                    self.as_raw(),
                    start,
                    elements.len() as u64,
                    views.as_ptr(),
                )
            },
            "OexString_SetElementsUtf16",
        )
    }
    pub fn set_cell_elements(
        &mut self,
        call: &Call<'call>,
        start: u64,
        elements: &[ValueRef<'_, 'call>],
    ) -> Result<()> {
        let elements = pointers(elements);
        check(
            // SAFETY: input handles are borrowed, target exclusive, pointers copied synchronously.
            unsafe {
                sys::OexCell_SetElements(
                    call.as_raw(),
                    self.as_raw(),
                    start,
                    elements.len() as u64,
                    elements.as_ptr(),
                )
            },
            "OexCell_SetElements",
        )
    }
}

impl<'a, 'call> ValueRef<'a, 'call> {
    pub fn cell_element(self, call: &Call<'call>, index: u64) -> Result<Value<'call>> {
        // SAFETY: borrowed inputs stay live; the API returns an owned child in this callback.
        unsafe {
            value_out("OexCell_GetElement", |out| {
                sys::OexCell_GetElement(call.as_raw(), self.as_raw(), index, out)
            })
        }
    }
    pub fn struct_field(
        self,
        call: &Call<'call>,
        element: u64,
        field: u64,
    ) -> Result<Value<'call>> {
        // SAFETY: borrowed inputs stay live; the API returns an owned child in this callback.
        unsafe {
            value_out("OexStruct_GetField", |out| {
                sys::OexStruct_GetField(call.as_raw(), self.as_raw(), element, field, out)
            })
        }
    }
    pub fn struct_field_values(self, call: &Call<'call>, field: u64) -> Result<Value<'call>> {
        // SAFETY: borrowed inputs stay live; the API returns an owned child in this callback.
        unsafe {
            value_out("OexStruct_GetFieldValues", |out| {
                sys::OexStruct_GetFieldValues(call.as_raw(), self.as_raw(), field, out)
            })
        }
    }
    pub fn table_variable(self, call: &Call<'call>, index: u64) -> Result<Value<'call>> {
        // SAFETY: borrowed inputs stay live; the API returns an owned child in this callback.
        unsafe {
            value_out("OexTable_GetVariable", |out| {
                sys::OexTable_GetVariable(call.as_raw(), self.as_raw(), index, out)
            })
        }
    }
    pub fn struct_field_count(self) -> Result<u64> {
        let mut count = 0;
        check(
            // SAFETY: live handle and writable scalar output.
            unsafe { sys::OexStruct_GetFieldCount(self.as_raw(), &mut count) },
            "OexStruct_GetFieldCount",
        )?;
        Ok(count)
    }
    pub fn table_row_count(self) -> Result<u64> {
        let mut count = 0;
        check(
            // SAFETY: live handle and writable scalar output.
            unsafe { sys::OexTable_GetRowCount(self.as_raw(), &mut count) },
            "OexTable_GetRowCount",
        )?;
        Ok(count)
    }
    pub fn table_variable_count(self) -> Result<u64> {
        let mut count = 0;
        check(
            // SAFETY: live handle and writable scalar output.
            unsafe { sys::OexTable_GetVariableCount(self.as_raw(), &mut count) },
            "OexTable_GetVariableCount",
        )?;
        Ok(count)
    }
    pub fn struct_field_name(self, index: u64) -> Result<&'a str> {
        let mut v = sys::OexUtf8View {
            data: ptr::null(),
            length: 0,
        };
        check(
            // SAFETY: name view borrows the live source schema.
            unsafe { sys::OexStruct_GetFieldName(self.as_raw(), index, &mut v) },
            "OexStruct_GetFieldName",
        )?;
        // SAFETY: source value remains borrowed for 'a.
        unsafe { text_view(v) }
    }
    pub fn table_variable_name(self, index: u64) -> Result<&'a str> {
        let mut v = sys::OexUtf8View {
            data: ptr::null(),
            length: 0,
        };
        check(
            // SAFETY: name view borrows the live source schema.
            unsafe { sys::OexTable_GetVariableName(self.as_raw(), index, &mut v) },
            "OexTable_GetVariableName",
        )?;
        // SAFETY: source value remains borrowed for 'a.
        unsafe { text_view(v) }
    }
    pub fn find_struct_field(self, name: &str) -> Result<Option<u64>> {
        let mut index = 0;
        // SAFETY: the value and UTF-8 name are borrowed for the synchronous call.
        let status = unsafe { sys::OexStruct_FindField(self.as_raw(), utf8(name), &mut index) };
        if status == sys::OEX_ERROR_NOT_FOUND {
            return Ok(None);
        }
        check(status, "OexStruct_FindField")?;
        Ok(Some(index))
    }
    pub fn find_table_variable(self, name: &str) -> Result<Option<u64>> {
        let mut index = 0;
        // SAFETY: the value and UTF-8 name are borrowed for the synchronous call.
        let status = unsafe { sys::OexTable_FindVariable(self.as_raw(), utf8(name), &mut index) };
        if status == sys::OEX_ERROR_NOT_FOUND {
            return Ok(None);
        }
        check(status, "OexTable_FindVariable")?;
        Ok(Some(index))
    }
}
impl<'call> Value<'call> {
    pub fn set_missing(&mut self, call: &Call<'call>, index: u64) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow; source handles stay live and are not consumed.
            unsafe { sys::OexString_SetMissing(call.as_raw(), self.as_raw(), index) },
            "OexString_SetMissing",
        )
    }
    pub fn set_cell_element(
        &mut self,
        call: &Call<'call>,
        index: u64,
        source: ValueRef<'_, 'call>,
    ) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow; source handles stay live and are not consumed.
            unsafe {
                sys::OexCell_SetElement(call.as_raw(), self.as_raw(), index, source.as_raw())
            },
            "OexCell_SetElement",
        )
    }
    pub fn set_struct_field(
        &mut self,
        call: &Call<'call>,
        element: u64,
        field: u64,
        source: ValueRef<'_, 'call>,
    ) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow; source handles stay live and are not consumed.
            unsafe {
                sys::OexStruct_SetField(
                    call.as_raw(),
                    self.as_raw(),
                    element,
                    field,
                    source.as_raw(),
                )
            },
            "OexStruct_SetField",
        )
    }
    pub fn set_struct_field_values(
        &mut self,
        call: &Call<'call>,
        field: u64,
        source: ValueRef<'_, 'call>,
    ) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow; source handles stay live and are not consumed.
            unsafe {
                sys::OexStruct_SetFieldValues(call.as_raw(), self.as_raw(), field, source.as_raw())
            },
            "OexStruct_SetFieldValues",
        )
    }
    pub fn remove_struct_field(&mut self, call: &Call<'call>, field: u64) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow; source handles stay live and are not consumed.
            unsafe { sys::OexStruct_RemoveField(call.as_raw(), self.as_raw(), field) },
            "OexStruct_RemoveField",
        )
    }
    pub fn set_table_variable(
        &mut self,
        call: &Call<'call>,
        index: u64,
        source: ValueRef<'_, 'call>,
    ) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow; source handles stay live and are not consumed.
            unsafe {
                sys::OexTable_SetVariable(call.as_raw(), self.as_raw(), index, source.as_raw())
            },
            "OexTable_SetVariable",
        )
    }
    pub fn remove_table_variable(&mut self, call: &Call<'call>, index: u64) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow; source handles stay live and are not consumed.
            unsafe { sys::OexTable_RemoveVariable(call.as_raw(), self.as_raw(), index) },
            "OexTable_RemoveVariable",
        )
    }
    pub fn clear_table_row_names(&mut self, call: &Call<'call>) -> Result<()> {
        check(
            // SAFETY: exclusive target borrow; source handles stay live and are not consumed.
            unsafe { sys::OexTable_ClearRowNames(call.as_raw(), self.as_raw()) },
            "OexTable_ClearRowNames",
        )
    }
    pub fn set_struct_field_names(&mut self, call: &Call<'call>, values: &[&str]) -> Result<()> {
        let names = names(values);
        check(
            // SAFETY: UTF-8 names are borrowed, target is exclusive, host copies synchronously.
            unsafe {
                sys::OexStruct_SetFieldNames(
                    call.as_raw(),
                    self.as_raw(),
                    names.len() as u64,
                    names.as_ptr(),
                )
            },
            "OexStruct_SetFieldNames",
        )
    }
    pub fn set_table_variable_names(&mut self, call: &Call<'call>, values: &[&str]) -> Result<()> {
        let names = names(values);
        check(
            // SAFETY: UTF-8 names are borrowed, target is exclusive, host copies synchronously.
            unsafe {
                sys::OexTable_SetVariableNames(
                    call.as_raw(),
                    self.as_raw(),
                    names.len() as u64,
                    names.as_ptr(),
                )
            },
            "OexTable_SetVariableNames",
        )
    }
    pub fn set_table_row_names(&mut self, call: &Call<'call>, values: &[&str]) -> Result<()> {
        let names = names(values);
        check(
            // SAFETY: UTF-8 names are borrowed, target is exclusive, host copies synchronously.
            unsafe {
                sys::OexTable_SetRowNames(
                    call.as_raw(),
                    self.as_raw(),
                    names.len() as u64,
                    names.as_ptr(),
                )
            },
            "OexTable_SetRowNames",
        )
    }
    pub fn add_struct_field(&mut self, call: &Call<'call>, name: &str) -> Result<()> {
        check(
            // SAFETY: target is exclusive; the name and optional source are borrowed and copied.
            unsafe { sys::OexStruct_AddField(call.as_raw(), self.as_raw(), utf8(name)) },
            "OexStruct_AddField",
        )
    }
    pub fn append_table_variable(
        &mut self,
        call: &Call<'call>,
        name: &str,
        source: ValueRef<'_, 'call>,
    ) -> Result<()> {
        check(
            // SAFETY: target is exclusive; the name and optional source are borrowed and copied.
            unsafe {
                sys::OexTable_AppendVariable(
                    call.as_raw(),
                    self.as_raw(),
                    utf8(name),
                    source.as_raw(),
                )
            },
            "OexTable_AppendVariable",
        )
    }
    pub fn cell_element(&self, call: &Call<'call>, index: u64) -> Result<Value<'call>> {
        self.as_value_ref().cell_element(call, index)
    }
    pub fn struct_field(
        &self,
        call: &Call<'call>,
        element: u64,
        field: u64,
    ) -> Result<Value<'call>> {
        self.as_value_ref().struct_field(call, element, field)
    }
    pub fn struct_field_values(&self, call: &Call<'call>, field: u64) -> Result<Value<'call>> {
        self.as_value_ref().struct_field_values(call, field)
    }
    pub fn table_variable(&self, call: &Call<'call>, index: u64) -> Result<Value<'call>> {
        self.as_value_ref().table_variable(call, index)
    }
    pub fn struct_field_count(&self) -> Result<u64> {
        self.as_value_ref().struct_field_count()
    }
    pub fn table_row_count(&self) -> Result<u64> {
        self.as_value_ref().table_row_count()
    }
    pub fn table_variable_count(&self) -> Result<u64> {
        self.as_value_ref().table_variable_count()
    }
    pub fn struct_field_name(&self, index: u64) -> Result<&str> {
        self.as_value_ref().struct_field_name(index)
    }
    pub fn table_variable_name(&self, index: u64) -> Result<&str> {
        self.as_value_ref().table_variable_name(index)
    }
    pub fn find_struct_field(&self, name: &str) -> Result<Option<u64>> {
        self.as_value_ref().find_struct_field(name)
    }
    pub fn find_table_variable(&self, name: &str) -> Result<Option<u64>> {
        self.as_value_ref().find_table_variable(name)
    }
    pub fn dimensions(&self) -> Result<Vec<u64>> {
        self.as_value_ref().dimensions()
    }
    pub fn class_name(&self, call: &Call<'call>) -> Result<String> {
        self.as_value_ref().class_name(call)
    }
    pub fn string_element(&self, index: u64) -> Result<StringElementRef<'_>> {
        self.as_value_ref().string_element(index)
    }
    pub fn string_elements(&self, start: u64, count: usize) -> Result<Vec<StringElementRef<'_>>> {
        self.as_value_ref().string_elements(start, count)
    }
    pub fn string_utf8(&self, index: u64) -> Result<Option<String>> {
        self.as_value_ref().string_utf8(index)
    }
    pub fn cell_elements(
        &self,
        call: &Call<'call>,
        start: u64,
        count: usize,
    ) -> Result<Vec<Value<'call>>> {
        self.as_value_ref().cell_elements(call, start, count)
    }
    pub fn table_row_names(&self, call: &Call<'call>) -> Result<Option<Value<'call>>> {
        self.as_value_ref().table_row_names(call)
    }
}

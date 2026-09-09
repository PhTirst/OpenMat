//! ABI checks use the actual public C header as the independent reference.
use oex_sys::*;
use std::ffi::c_void;
use std::{fmt::Write, fs, path::PathBuf, process::Command};

#[test]
fn rust_declarations_match_the_c_header() {
    let mut c = String::from("#include <openmat/oex.h>\n#include <stdio.h>\nint main(void) {\n");
    let mut expected = String::new();
    let mut symbols = Vec::new();
    macro_rules! constant {
        ($name:ident) => {{
            symbols.push(stringify!($name));
            writeln!(
                c,
                "printf(\"{}=%llu\\n\", (unsigned long long){});",
                stringify!($name),
                stringify!($name)
            )
            .unwrap();
            writeln!(expected, "{}={}", stringify!($name), $name).unwrap();
        }};
    }
    macro_rules! layout {
        ($ty:ty, $($field:ident => $c_field:literal),* $(,)?) => {{
            writeln!(c, "printf(\"{}=%llu,%llu\\n\", (unsigned long long)sizeof({}), (unsigned long long)_Alignof({}));", stringify!($ty), stringify!($ty), stringify!($ty)).unwrap();
            writeln!(expected, "{}={},{}", stringify!($ty), size_of::<$ty>(), align_of::<$ty>()).unwrap();
            $(
                writeln!(c, "printf(\"{}={}:%llu\\n\", (unsigned long long)offsetof({}, {}));", stringify!($ty), $c_field, stringify!($ty), $c_field).unwrap();
                writeln!(expected, "{}={}:{}", stringify!($ty), $c_field, std::mem::offset_of!($ty, $field)).unwrap();
            )*
        }};
    }
    constant!(OEX_ABI_VERSION);
    constant!(OEX_ABI_MAJOR);
    constant!(OEX_ABI_MINOR);
    constant!(OEX_OK);
    constant!(OEX_ERROR_ARGUMENT);
    constant!(OEX_ERROR_TYPE);
    constant!(OEX_ERROR_DIMENSION);
    constant!(OEX_ERROR_RANGE);
    constant!(OEX_ERROR_ALLOCATION);
    constant!(OEX_ERROR_CANCELLED);
    constant!(OEX_ERROR_UNSUPPORTED);
    constant!(OEX_ERROR_PLUGIN);
    constant!(OEX_ERROR_ABI);
    constant!(OEX_ERROR_STATE);
    constant!(OEX_ERROR_CALLBACK);
    constant!(OEX_ERROR_ENCODING);
    constant!(OEX_ERROR_NOT_FOUND);
    constant!(OEX_VALUE_NOTHING);
    constant!(OEX_VALUE_DENSE);
    constant!(OEX_VALUE_SPARSE);
    constant!(OEX_VALUE_STRING);
    constant!(OEX_VALUE_CELL);
    constant!(OEX_VALUE_STRUCT);
    constant!(OEX_VALUE_TABLE);
    constant!(OEX_VALUE_OBJECT);
    constant!(OEX_VALUE_FUNCTION);
    constant!(OEX_VALUE_GRAPHICS);
    constant!(OEX_DATA_NONE);
    constant!(OEX_DATA_LOGICAL);
    constant!(OEX_DATA_CHAR16);
    constant!(OEX_DATA_I8);
    constant!(OEX_DATA_U8);
    constant!(OEX_DATA_I16);
    constant!(OEX_DATA_U16);
    constant!(OEX_DATA_I32);
    constant!(OEX_DATA_U32);
    constant!(OEX_DATA_I64);
    constant!(OEX_DATA_U64);
    constant!(OEX_DATA_F32);
    constant!(OEX_DATA_F64);
    constant!(OEX_DATA_COMPLEX_F32);
    constant!(OEX_DATA_COMPLEX_F64);
    constant!(OEX_DATA_COMPLEX_I8);
    constant!(OEX_DATA_COMPLEX_U8);
    constant!(OEX_DATA_COMPLEX_I16);
    constant!(OEX_DATA_COMPLEX_U16);
    constant!(OEX_DATA_COMPLEX_I32);
    constant!(OEX_DATA_COMPLEX_U32);
    constant!(OEX_DATA_COMPLEX_I64);
    constant!(OEX_DATA_COMPLEX_U64);
    constant!(OEX_VALUE_FLAG_COMPLEX);
    constant!(OEX_CLASS_HANDLE);
    layout!(OexUtf16View, data => "data", length => "length");
    layout!(OexStringElementView, struct_size => "structSize", is_missing => "isMissing", data => "data", length => "length");
    layout!(OexUtf8View, data => "data", length => "length");
    layout!(OexValueInfo, struct_size => "structSize", kind => "kind", data_type => "dataType", flags => "flags", rank => "rank", element_size => "elementSize", element_count => "elementCount");
    layout!(OexDenseView, struct_size => "structSize", data_type => "dataType", rank => "rank", element_size => "elementSize", element_count => "elementCount", dimensions => "dimensions", data => "data");
    layout!(OexMutableDenseView, struct_size => "structSize", data_type => "dataType", rank => "rank", element_size => "elementSize", element_count => "elementCount", dimensions => "dimensions", data => "data");
    layout!(OexSparseCscView, struct_size => "structSize", data_type => "dataType", element_size => "elementSize", reserved => "reserved", rows => "rows", columns => "columns", stored_count => "storedCount", column_offsets => "columnOffsets", row_indices => "rowIndices", values => "values");
    layout!(OexSparseTripletView, struct_size => "structSize", data_type => "dataType", element_size => "elementSize", reserved => "reserved", rows => "rows", columns => "columns", capacity => "capacity", row_indices => "rowIndices", column_indices => "columnIndices", values => "values");
    layout!(OexSparsePatternValuesView, struct_size => "structSize", data_type => "dataType", element_size => "elementSize", reserved => "reserved", stored_count => "storedCount", values => "values");
    layout!(OexFunctionDefinition, struct_size => "structSize", flags => "flags", name => "name", minimum_inputs => "minimumInputs", maximum_inputs => "maximumInputs", minimum_outputs => "minimumOutputs", maximum_outputs => "maximumOutputs", invoke => "invoke");
    layout!(OexNativeMethodDefinition, struct_size => "structSize", flags => "flags", name => "name", minimum_inputs => "minimumInputs", maximum_inputs => "maximumInputs", minimum_outputs => "minimumOutputs", maximum_outputs => "maximumOutputs", invoke => "invoke");
    layout!(OexNativePropertyDefinition, struct_size => "structSize", flags => "flags", name => "name", getter => "getter", setter => "setter");
    layout!(OexNativeClassDefinition, struct_size => "structSize", semantics => "semantics", name => "name", constructor => "constructor", destructor => "destructor", methods => "methods", method_count => "methodCount", type_token => "typeToken", properties => "properties", property_count => "propertyCount");
    layout!(OexPluginDefinition, struct_size => "structSize", required_abi => "requiredAbi", flags => "flags", reserved => "reserved", name => "name", version => "version", functions => "functions", function_count => "functionCount", classes => "classes", class_count => "classCount");
    layout!(OexComplexF32, re => "re", im => "im");
    layout!(OexComplexF64, re => "re", im => "im");
    layout!(OexComplexI8, re => "re", im => "im");
    layout!(OexComplexU8, re => "re", im => "im");
    layout!(OexComplexI16, re => "re", im => "im");
    layout!(OexComplexU16, re => "re", im => "im");
    layout!(OexComplexI32, re => "re", im => "im");
    layout!(OexComplexU32, re => "re", im => "im");
    layout!(OexComplexI64, re => "re", im => "im");
    layout!(OexComplexU64, re => "re", im => "im");
    let _: unsafe extern "C" fn(*const OexCall) -> u32 = OexCall_GetInputCount;
    c.push_str(r#"typedef uint32_t (OEX_CALL *Expected_OexCall_GetInputCount)(const OexCall *); _Static_assert(_Generic(&OexCall_GetInputCount, Expected_OexCall_GetInputCount: 1, default: 0), "signature: OexCall_GetInputCount");"#);
    let _: unsafe extern "C" fn(*const OexCall) -> u32 = OexCall_GetOutputCount;
    c.push_str(r#"typedef uint32_t (OEX_CALL *Expected_OexCall_GetOutputCount)(const OexCall *); _Static_assert(_Generic(&OexCall_GetOutputCount, Expected_OexCall_GetOutputCount: 1, default: 0), "signature: OexCall_GetOutputCount");"#);
    let _: unsafe extern "C" fn(*const OexCall, u32) -> *const OexValue = OexCall_BorrowInput;
    c.push_str(r#"typedef const OexValue * (OEX_CALL *Expected_OexCall_BorrowInput)(const OexCall *, uint32_t); _Static_assert(_Generic(&OexCall_BorrowInput, Expected_OexCall_BorrowInput: 1, default: 0), "signature: OexCall_BorrowInput");"#);
    let _: unsafe extern "C" fn(*mut OexCall, u32, *mut *mut OexValue) -> OexStatus =
        OexCall_TakeInput;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCall_TakeInput)(OexCall *, uint32_t, OexValue * *); _Static_assert(_Generic(&OexCall_TakeInput, Expected_OexCall_TakeInput: 1, default: 0), "signature: OexCall_TakeInput");"#);
    let _: unsafe extern "C" fn(*mut OexCall, u32, *mut OexValue) -> OexStatus = OexCall_SetOutput;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCall_SetOutput)(OexCall *, uint32_t, OexValue *); _Static_assert(_Generic(&OexCall_SetOutput, Expected_OexCall_SetOutput: 1, default: 0), "signature: OexCall_SetOutput");"#);
    let _: unsafe extern "C" fn(*mut OexCall, OexUtf8View, OexUtf8View) -> OexStatus =
        OexCall_SetError;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCall_SetError)(OexCall *, OexUtf8View, OexUtf8View); _Static_assert(_Generic(&OexCall_SetError, Expected_OexCall_SetError: 1, default: 0), "signature: OexCall_SetError");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u32,
        *const *const OexValue,
        u32,
        *mut *mut OexValue,
    ) -> OexStatus = OexCall_Invoke;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCall_Invoke)(OexCall *, const OexValue *, uint32_t, const OexValue * const *, uint32_t, OexValue * *); _Static_assert(_Generic(&OexCall_Invoke, Expected_OexCall_Invoke: 1, default: 0), "signature: OexCall_Invoke");"#);
    let _: unsafe extern "C" fn(*const OexCall) -> *const OexCancellation = OexCall_GetCancellation;
    c.push_str(r#"typedef const OexCancellation * (OEX_CALL *Expected_OexCall_GetCancellation)(const OexCall *); _Static_assert(_Generic(&OexCall_GetCancellation, Expected_OexCall_GetCancellation: 1, default: 0), "signature: OexCall_GetCancellation");"#);
    let _: unsafe extern "C" fn(*const OexCancellation) -> u32 = OexCancellation_IsRequested;
    c.push_str(r#"typedef uint32_t (OEX_CALL *Expected_OexCancellation_IsRequested)(const OexCancellation *); _Static_assert(_Generic(&OexCancellation_IsRequested, Expected_OexCancellation_IsRequested: 1, default: 0), "signature: OexCancellation_IsRequested");"#);
    let _: unsafe extern "C" fn(*const OexValue, *mut OexValueInfo) -> OexStatus = OexValue_GetInfo;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexValue_GetInfo)(const OexValue *, OexValueInfo *); _Static_assert(_Generic(&OexValue_GetInfo, Expected_OexValue_GetInfo: 1, default: 0), "signature: OexValue_GetInfo");"#);
    let _: unsafe extern "C" fn(*const OexValue, *mut *mut OexValue) -> OexStatus = OexValue_Retain;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexValue_Retain)(const OexValue *, OexValue * *); _Static_assert(_Generic(&OexValue_Retain, Expected_OexValue_Retain: 1, default: 0), "signature: OexValue_Retain");"#);
    let _: unsafe extern "C" fn(*mut OexValue) = OexValue_Release;
    c.push_str(r#"typedef void (OEX_CALL *Expected_OexValue_Release)(OexValue *); _Static_assert(_Generic(&OexValue_Release, Expected_OexValue_Release: 1, default: 0), "signature: OexValue_Release");"#);
    let _: unsafe extern "C" fn(*const OexValue, *mut f64) -> OexStatus = OexValue_GetFloat64;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexValue_GetFloat64)(const OexValue *, double *); _Static_assert(_Generic(&OexValue_GetFloat64, Expected_OexValue_GetFloat64: 1, default: 0), "signature: OexValue_GetFloat64");"#);
    let _: unsafe extern "C" fn(*mut OexCall, f64, *mut *mut OexValue) -> OexStatus =
        OexValue_CreateFloat64;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexValue_CreateFloat64)(OexCall *, double, OexValue * *); _Static_assert(_Generic(&OexValue_CreateFloat64, Expected_OexValue_CreateFloat64: 1, default: 0), "signature: OexValue_CreateFloat64");"#);
    let _: unsafe extern "C" fn(*const OexValue, *mut OexDenseView) -> OexStatus = OexDense_GetView;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexDense_GetView)(const OexValue *, OexDenseView *); _Static_assert(_Generic(&OexDense_GetView, Expected_OexDense_GetView: 1, default: 0), "signature: OexDense_GetView");"#);
    let _: unsafe extern "C" fn(*mut OexValue, *mut OexMutableDenseView) -> OexStatus =
        OexDense_GetMutableView;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexDense_GetMutableView)(OexValue *, OexMutableDenseView *); _Static_assert(_Generic(&OexDense_GetMutableView, Expected_OexDense_GetMutableView: 1, default: 0), "signature: OexDense_GetMutableView");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        u32,
        *const u64,
        *mut *mut OexDenseBuilder,
        *mut OexMutableDenseView,
    ) -> OexStatus = OexDenseBuilder_CreateUninitialized;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexDenseBuilder_CreateUninitialized)(OexCall *, uint32_t, uint32_t, const uint64_t *, OexDenseBuilder * *, OexMutableDenseView *); _Static_assert(_Generic(&OexDenseBuilder_CreateUninitialized, Expected_OexDenseBuilder_CreateUninitialized: 1, default: 0), "signature: OexDenseBuilder_CreateUninitialized");"#);
    let _: unsafe extern "C" fn(*mut OexDenseBuilder, *mut *mut OexValue) -> OexStatus =
        OexDenseBuilder_Commit;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexDenseBuilder_Commit)(OexDenseBuilder *, OexValue * *); _Static_assert(_Generic(&OexDenseBuilder_Commit, Expected_OexDenseBuilder_Commit: 1, default: 0), "signature: OexDenseBuilder_Commit");"#);
    let _: unsafe extern "C" fn(*mut OexDenseBuilder) = OexDenseBuilder_Abort;
    c.push_str(r#"typedef void (OEX_CALL *Expected_OexDenseBuilder_Abort)(OexDenseBuilder *); _Static_assert(_Generic(&OexDenseBuilder_Abort, Expected_OexDenseBuilder_Abort: 1, default: 0), "signature: OexDenseBuilder_Abort");"#);
    let _: unsafe extern "C" fn(*const OexValue, *mut OexSparseCscView) -> OexStatus =
        OexSparse_GetCscView;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexSparse_GetCscView)(const OexValue *, OexSparseCscView *); _Static_assert(_Generic(&OexSparse_GetCscView, Expected_OexSparse_GetCscView: 1, default: 0), "signature: OexSparse_GetCscView");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        u64,
        u64,
        u64,
        *const u64,
        *const u64,
        *mut *mut OexSparsePattern,
    ) -> OexStatus = OexSparsePattern_Create;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexSparsePattern_Create)(OexCall *, uint64_t, uint64_t, uint64_t, const uint64_t *, const uint64_t *, OexSparsePattern * *); _Static_assert(_Generic(&OexSparsePattern_Create, Expected_OexSparsePattern_Create: 1, default: 0), "signature: OexSparsePattern_Create");"#);
    let _: unsafe extern "C" fn(*const OexSparsePattern, *mut *mut OexSparsePattern) -> OexStatus =
        OexSparsePattern_Retain;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexSparsePattern_Retain)(const OexSparsePattern *, OexSparsePattern * *); _Static_assert(_Generic(&OexSparsePattern_Retain, Expected_OexSparsePattern_Retain: 1, default: 0), "signature: OexSparsePattern_Retain");"#);
    let _: unsafe extern "C" fn(*mut OexSparsePattern) = OexSparsePattern_Release;
    c.push_str(r#"typedef void (OEX_CALL *Expected_OexSparsePattern_Release)(OexSparsePattern *); _Static_assert(_Generic(&OexSparsePattern_Release, Expected_OexSparsePattern_Release: 1, default: 0), "signature: OexSparsePattern_Release");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        u64,
        u64,
        u64,
        *mut *mut OexSparseTripletBuilder,
        *mut OexSparseTripletView,
    ) -> OexStatus = OexSparseTripletBuilder_CreateUninitialized;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexSparseTripletBuilder_CreateUninitialized)(OexCall *, uint32_t, uint64_t, uint64_t, uint64_t, OexSparseTripletBuilder * *, OexSparseTripletView *); _Static_assert(_Generic(&OexSparseTripletBuilder_CreateUninitialized, Expected_OexSparseTripletBuilder_CreateUninitialized: 1, default: 0), "signature: OexSparseTripletBuilder_CreateUninitialized");"#);
    let _: unsafe extern "C" fn(
        *mut OexSparseTripletBuilder,
        u64,
        *mut *mut OexValue,
    ) -> OexStatus = OexSparseTripletBuilder_Commit;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexSparseTripletBuilder_Commit)(OexSparseTripletBuilder *, uint64_t, OexValue * *); _Static_assert(_Generic(&OexSparseTripletBuilder_Commit, Expected_OexSparseTripletBuilder_Commit: 1, default: 0), "signature: OexSparseTripletBuilder_Commit");"#);
    let _: unsafe extern "C" fn(*mut OexSparseTripletBuilder) = OexSparseTripletBuilder_Abort;
    c.push_str(r#"typedef void (OEX_CALL *Expected_OexSparseTripletBuilder_Abort)(OexSparseTripletBuilder *); _Static_assert(_Generic(&OexSparseTripletBuilder_Abort, Expected_OexSparseTripletBuilder_Abort: 1, default: 0), "signature: OexSparseTripletBuilder_Abort");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexSparsePattern,
        u32,
        *mut *mut OexSparsePatternBuilder,
        *mut OexSparsePatternValuesView,
    ) -> OexStatus = OexSparsePatternBuilder_CreateUninitialized;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexSparsePatternBuilder_CreateUninitialized)(OexCall *, const OexSparsePattern *, uint32_t, OexSparsePatternBuilder * *, OexSparsePatternValuesView *); _Static_assert(_Generic(&OexSparsePatternBuilder_CreateUninitialized, Expected_OexSparsePatternBuilder_CreateUninitialized: 1, default: 0), "signature: OexSparsePatternBuilder_CreateUninitialized");"#);
    let _: unsafe extern "C" fn(*mut OexSparsePatternBuilder, *mut *mut OexValue) -> OexStatus =
        OexSparsePatternBuilder_Commit;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexSparsePatternBuilder_Commit)(OexSparsePatternBuilder *, OexValue * *); _Static_assert(_Generic(&OexSparsePatternBuilder_Commit, Expected_OexSparsePatternBuilder_Commit: 1, default: 0), "signature: OexSparsePatternBuilder_Commit");"#);
    let _: unsafe extern "C" fn(*mut OexSparsePatternBuilder) = OexSparsePatternBuilder_Abort;
    c.push_str(r#"typedef void (OEX_CALL *Expected_OexSparsePatternBuilder_Abort)(OexSparsePatternBuilder *); _Static_assert(_Generic(&OexSparsePatternBuilder_Abort, Expected_OexSparsePatternBuilder_Abort: 1, default: 0), "signature: OexSparsePatternBuilder_Abort");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        *const c_void,
        *mut *mut c_void,
    ) -> OexStatus = OexNativeObject_BorrowInstance;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexNativeObject_BorrowInstance)(OexCall *, const OexValue *, const void *, void * *); _Static_assert(_Generic(&OexNativeObject_BorrowInstance, Expected_OexNativeObject_BorrowInstance: 1, default: 0), "signature: OexNativeObject_BorrowInstance");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const c_void,
        *mut c_void,
        *mut *mut OexValue,
    ) -> OexStatus = OexNativeObject_Create;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexNativeObject_Create)(OexCall *, const void *, void *, OexValue * *); _Static_assert(_Generic(&OexNativeObject_Create, Expected_OexNativeObject_Create: 1, default: 0), "signature: OexNativeObject_Create");"#);
    let _: Option<unsafe extern "C" fn(*mut OexCall) -> OexStatus> = None::<OexFunctionCallback>;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexFunctionCallback)(OexCall *); _Static_assert(_Generic((OexFunctionCallback)0, Expected_OexFunctionCallback: 1, default: 0), "callback signature: OexFunctionCallback");"#);
    let _: Option<unsafe extern "C" fn(*mut OexCall, *mut *mut c_void) -> OexStatus> =
        None::<OexNativeConstructor>;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexNativeConstructor)(OexCall *, void * *); _Static_assert(_Generic((OexNativeConstructor)0, Expected_OexNativeConstructor: 1, default: 0), "callback signature: OexNativeConstructor");"#);
    let _: Option<unsafe extern "C" fn(*mut OexCall, *mut c_void) -> OexStatus> =
        None::<OexNativeMethod>;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexNativeMethod)(OexCall *, void *); _Static_assert(_Generic((OexNativeMethod)0, Expected_OexNativeMethod: 1, default: 0), "callback signature: OexNativeMethod");"#);
    let _: Option<unsafe extern "C" fn(*mut OexCall, *mut c_void) -> OexStatus> =
        None::<OexNativePropertyGetter>;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexNativePropertyGetter)(OexCall *, void *); _Static_assert(_Generic((OexNativePropertyGetter)0, Expected_OexNativePropertyGetter: 1, default: 0), "callback signature: OexNativePropertyGetter");"#);
    let _: Option<unsafe extern "C" fn(*mut OexCall, *mut c_void) -> OexStatus> =
        None::<OexNativePropertySetter>;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexNativePropertySetter)(OexCall *, void *); _Static_assert(_Generic((OexNativePropertySetter)0, Expected_OexNativePropertySetter: 1, default: 0), "callback signature: OexNativePropertySetter");"#);
    let _: Option<unsafe extern "C" fn(*mut c_void)> = None::<OexNativeDestructor>;
    c.push_str(r#"typedef void (OEX_CALL *Expected_OexNativeDestructor)(void *); _Static_assert(_Generic((OexNativeDestructor)0, Expected_OexNativeDestructor: 1, default: 0), "callback signature: OexNativeDestructor");"#);
    let _: Option<unsafe extern "C" fn() -> *const OexPluginDefinition> = None::<OexPluginInit>;
    c.push_str(r#"typedef const OexPluginDefinition * (OEX_CALL *Expected_OexPluginInit)(void); _Static_assert(_Generic(&OexPlugin_Init, Expected_OexPluginInit: 1, default: 0), "callback signature: OexPluginInit");"#);
    c.push_str("return 0; }\n");
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let header = fs::read_to_string(repository.join("include/openmat/oex.h")).unwrap();
    let header_constants: Vec<_> = header
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            if words.next()? != "#define" {
                return None;
            }
            let name = words.next()?;
            let value = words.next()?;
            value
                .starts_with(|c: char| c.is_ascii_digit())
                .then_some(name)
        })
        .collect();
    assert_eq!(
        symbols, header_constants,
        "update bindings/tests for changed public constants"
    );
    let _: unsafe extern "C" fn(*const OexValue, u32, *mut u64, *mut u32) -> OexStatus =
        OexValue_GetDimensions;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexValue_GetDimensions)(const OexValue *,uint32_t,uint64_t *,uint32_t *); _Static_assert(_Generic(&OexValue_GetDimensions, Expected_OexValue_GetDimensions: 1, default: 0), "signature: OexValue_GetDimensions");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *const OexValue, *mut OexUtf8View) -> OexStatus =
        OexValue_GetClassName;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexValue_GetClassName)(OexCall *,const OexValue *,OexUtf8View *); _Static_assert(_Generic(&OexValue_GetClassName, Expected_OexValue_GetClassName: 1, default: 0), "signature: OexValue_GetClassName");"#);
    let _: unsafe extern "C" fn(*mut OexCall, u32, *const u64, *mut *mut OexValue) -> OexStatus =
        OexString_Create;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_Create)(OexCall *,uint32_t,const uint64_t *,OexValue **); _Static_assert(_Generic(&OexString_Create, Expected_OexString_Create: 1, default: 0), "signature: OexString_Create");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        *const u64,
        u64,
        *const OexStringElementView,
        *mut *mut OexValue,
    ) -> OexStatus = OexString_CreateFromUtf16;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_CreateFromUtf16)(OexCall *,uint32_t,const uint64_t *,uint64_t,const OexStringElementView *,OexValue **); _Static_assert(_Generic(&OexString_CreateFromUtf16, Expected_OexString_CreateFromUtf16: 1, default: 0), "signature: OexString_CreateFromUtf16");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        u64,
        u64,
        *const OexStringElementView,
    ) -> OexStatus = OexString_SetElementsUtf16;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_SetElementsUtf16)(OexCall *,OexValue *,uint64_t,uint64_t,const OexStringElementView *); _Static_assert(_Generic(&OexString_SetElementsUtf16, Expected_OexString_SetElementsUtf16: 1, default: 0), "signature: OexString_SetElementsUtf16");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        *const u64,
        u64,
        *const OexUtf8View,
        *const u8,
        *mut *mut OexValue,
    ) -> OexStatus = OexString_CreateFromUtf8;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_CreateFromUtf8)(OexCall *,uint32_t,const uint64_t *,uint64_t,const OexUtf8View *,const uint8_t *,OexValue **); _Static_assert(_Generic(&OexString_CreateFromUtf8, Expected_OexString_CreateFromUtf8: 1, default: 0), "signature: OexString_CreateFromUtf8");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        u64,
        u64,
        *const OexUtf8View,
        *const u8,
    ) -> OexStatus = OexString_SetElementsUtf8;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_SetElementsUtf8)(OexCall *,OexValue *,uint64_t,uint64_t,const OexUtf8View *,const uint8_t *); _Static_assert(_Generic(&OexString_SetElementsUtf8, Expected_OexString_SetElementsUtf8: 1, default: 0), "signature: OexString_SetElementsUtf8");"#);
    let _: unsafe extern "C" fn(*const OexValue, u64, *mut OexStringElementView) -> OexStatus =
        OexString_GetElementView;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_GetElementView)(const OexValue *,uint64_t,OexStringElementView *); _Static_assert(_Generic(&OexString_GetElementView, Expected_OexString_GetElementView: 1, default: 0), "signature: OexString_GetElementView");"#);
    let _: unsafe extern "C" fn(*const OexValue, u64, u64, *mut OexStringElementView) -> OexStatus =
        OexString_GetElements;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_GetElements)(const OexValue *,uint64_t,uint64_t,OexStringElementView *); _Static_assert(_Generic(&OexString_GetElements, Expected_OexString_GetElements: 1, default: 0), "signature: OexString_GetElements");"#);
    let _: unsafe extern "C" fn(
        *const OexValue,
        u64,
        *mut std::ffi::c_char,
        u64,
        *mut u64,
        *mut u32,
    ) -> OexStatus = OexString_CopyElementUtf8;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_CopyElementUtf8)(const OexValue *,uint64_t,char *,uint64_t,uint64_t *,uint32_t *); _Static_assert(_Generic(&OexString_CopyElementUtf8, Expected_OexString_CopyElementUtf8: 1, default: 0), "signature: OexString_CopyElementUtf8");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, OexUtf8View) -> OexStatus =
        OexString_SetElementUtf8;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_SetElementUtf8)(OexCall *,OexValue *,uint64_t,OexUtf8View); _Static_assert(_Generic(&OexString_SetElementUtf8, Expected_OexString_SetElementUtf8: 1, default: 0), "signature: OexString_SetElementUtf8");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, OexUtf16View) -> OexStatus =
        OexString_SetElementUtf16;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_SetElementUtf16)(OexCall *,OexValue *,uint64_t,OexUtf16View); _Static_assert(_Generic(&OexString_SetElementUtf16, Expected_OexString_SetElementUtf16: 1, default: 0), "signature: OexString_SetElementUtf16");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64) -> OexStatus =
        OexString_SetMissing;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexString_SetMissing)(OexCall *,OexValue *,uint64_t); _Static_assert(_Generic(&OexString_SetMissing, Expected_OexString_SetMissing: 1, default: 0), "signature: OexString_SetMissing");"#);
    let _: unsafe extern "C" fn(*mut OexCall, u32, *const u64, *mut *mut OexValue) -> OexStatus =
        OexCell_Create;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCell_Create)(OexCall *,uint32_t,const uint64_t *,OexValue **); _Static_assert(_Generic(&OexCell_Create, Expected_OexCell_Create: 1, default: 0), "signature: OexCell_Create");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u64,
        *mut *mut OexValue,
    ) -> OexStatus = OexCell_GetElement;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCell_GetElement)(OexCall *,const OexValue *,uint64_t,OexValue **); _Static_assert(_Generic(&OexCell_GetElement, Expected_OexCell_GetElement: 1, default: 0), "signature: OexCell_GetElement");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexValue) -> OexStatus =
        OexCell_SetElement;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCell_SetElement)(OexCall *,OexValue *,uint64_t,const OexValue *); _Static_assert(_Generic(&OexCell_SetElement, Expected_OexCell_SetElement: 1, default: 0), "signature: OexCell_SetElement");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u64,
        u64,
        *mut *mut OexValue,
    ) -> OexStatus = OexCell_GetElements;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCell_GetElements)(OexCall *,const OexValue *,uint64_t,uint64_t,OexValue **); _Static_assert(_Generic(&OexCell_GetElements, Expected_OexCell_GetElements: 1, default: 0), "signature: OexCell_GetElements");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        u64,
        u64,
        *const *const OexValue,
    ) -> OexStatus = OexCell_SetElements;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexCell_SetElements)(OexCall *,OexValue *,uint64_t,uint64_t,const OexValue *const *); _Static_assert(_Generic(&OexCell_SetElements, Expected_OexCell_SetElements: 1, default: 0), "signature: OexCell_SetElements");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        u32,
        *const u64,
        u64,
        *const OexUtf8View,
        *mut *mut OexValue,
    ) -> OexStatus = OexStruct_Create;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_Create)(OexCall *,uint32_t,const uint64_t *,uint64_t,const OexUtf8View *,OexValue **); _Static_assert(_Generic(&OexStruct_Create, Expected_OexStruct_Create: 1, default: 0), "signature: OexStruct_Create");"#);
    let _: unsafe extern "C" fn(*const OexValue, *mut u64) -> OexStatus = OexStruct_GetFieldCount;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_GetFieldCount)(const OexValue *,uint64_t *); _Static_assert(_Generic(&OexStruct_GetFieldCount, Expected_OexStruct_GetFieldCount: 1, default: 0), "signature: OexStruct_GetFieldCount");"#);
    let _: unsafe extern "C" fn(*const OexValue, u64, *mut OexUtf8View) -> OexStatus =
        OexStruct_GetFieldName;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_GetFieldName)(const OexValue *,uint64_t,OexUtf8View *); _Static_assert(_Generic(&OexStruct_GetFieldName, Expected_OexStruct_GetFieldName: 1, default: 0), "signature: OexStruct_GetFieldName");"#);
    let _: unsafe extern "C" fn(*const OexValue, OexUtf8View, *mut u64) -> OexStatus =
        OexStruct_FindField;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_FindField)(const OexValue *,OexUtf8View,uint64_t *); _Static_assert(_Generic(&OexStruct_FindField, Expected_OexStruct_FindField: 1, default: 0), "signature: OexStruct_FindField");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u64,
        u64,
        *mut *mut OexValue,
    ) -> OexStatus = OexStruct_GetField;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_GetField)(OexCall *,const OexValue *,uint64_t,uint64_t,OexValue **); _Static_assert(_Generic(&OexStruct_GetField, Expected_OexStruct_GetField: 1, default: 0), "signature: OexStruct_GetField");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        u64,
        u64,
        *const OexValue,
    ) -> OexStatus = OexStruct_SetField;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_SetField)(OexCall *,OexValue *,uint64_t,uint64_t,const OexValue *); _Static_assert(_Generic(&OexStruct_SetField, Expected_OexStruct_SetField: 1, default: 0), "signature: OexStruct_SetField");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, OexUtf8View) -> OexStatus =
        OexStruct_AddField;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_AddField)(OexCall *,OexValue *,OexUtf8View); _Static_assert(_Generic(&OexStruct_AddField, Expected_OexStruct_AddField: 1, default: 0), "signature: OexStruct_AddField");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64) -> OexStatus =
        OexStruct_RemoveField;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_RemoveField)(OexCall *,OexValue *,uint64_t); _Static_assert(_Generic(&OexStruct_RemoveField, Expected_OexStruct_RemoveField: 1, default: 0), "signature: OexStruct_RemoveField");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexUtf8View) -> OexStatus =
        OexStruct_SetFieldNames;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_SetFieldNames)(OexCall *,OexValue *,uint64_t,const OexUtf8View *); _Static_assert(_Generic(&OexStruct_SetFieldNames, Expected_OexStruct_SetFieldNames: 1, default: 0), "signature: OexStruct_SetFieldNames");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u64,
        *mut *mut OexValue,
    ) -> OexStatus = OexStruct_GetFieldValues;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_GetFieldValues)(OexCall *,const OexValue *,uint64_t,OexValue **); _Static_assert(_Generic(&OexStruct_GetFieldValues, Expected_OexStruct_GetFieldValues: 1, default: 0), "signature: OexStruct_GetFieldValues");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexValue) -> OexStatus =
        OexStruct_SetFieldValues;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexStruct_SetFieldValues)(OexCall *,OexValue *,uint64_t,const OexValue *); _Static_assert(_Generic(&OexStruct_SetFieldValues, Expected_OexStruct_SetFieldValues: 1, default: 0), "signature: OexStruct_SetFieldValues");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        u64,
        u64,
        *const OexUtf8View,
        *const *const OexValue,
        *mut *mut OexValue,
    ) -> OexStatus = OexTable_Create;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_Create)(OexCall *,uint64_t,uint64_t,const OexUtf8View *,const OexValue *const *,OexValue **); _Static_assert(_Generic(&OexTable_Create, Expected_OexTable_Create: 1, default: 0), "signature: OexTable_Create");"#);
    let _: unsafe extern "C" fn(*const OexValue, *mut u64) -> OexStatus = OexTable_GetRowCount;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_GetRowCount)(const OexValue *,uint64_t *); _Static_assert(_Generic(&OexTable_GetRowCount, Expected_OexTable_GetRowCount: 1, default: 0), "signature: OexTable_GetRowCount");"#);
    let _: unsafe extern "C" fn(*const OexValue, *mut u64) -> OexStatus = OexTable_GetVariableCount;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_GetVariableCount)(const OexValue *,uint64_t *); _Static_assert(_Generic(&OexTable_GetVariableCount, Expected_OexTable_GetVariableCount: 1, default: 0), "signature: OexTable_GetVariableCount");"#);
    let _: unsafe extern "C" fn(*const OexValue, u64, *mut OexUtf8View) -> OexStatus =
        OexTable_GetVariableName;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_GetVariableName)(const OexValue *,uint64_t,OexUtf8View *); _Static_assert(_Generic(&OexTable_GetVariableName, Expected_OexTable_GetVariableName: 1, default: 0), "signature: OexTable_GetVariableName");"#);
    let _: unsafe extern "C" fn(*const OexValue, OexUtf8View, *mut u64) -> OexStatus =
        OexTable_FindVariable;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_FindVariable)(const OexValue *,OexUtf8View,uint64_t *); _Static_assert(_Generic(&OexTable_FindVariable, Expected_OexTable_FindVariable: 1, default: 0), "signature: OexTable_FindVariable");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexUtf8View) -> OexStatus =
        OexTable_SetVariableNames;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_SetVariableNames)(OexCall *,OexValue *,uint64_t,const OexUtf8View *); _Static_assert(_Generic(&OexTable_SetVariableNames, Expected_OexTable_SetVariableNames: 1, default: 0), "signature: OexTable_SetVariableNames");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        u64,
        *mut *mut OexValue,
    ) -> OexStatus = OexTable_GetVariable;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_GetVariable)(OexCall *,const OexValue *,uint64_t,OexValue **); _Static_assert(_Generic(&OexTable_GetVariable, Expected_OexTable_GetVariable: 1, default: 0), "signature: OexTable_GetVariable");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexValue) -> OexStatus =
        OexTable_SetVariable;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_SetVariable)(OexCall *,OexValue *,uint64_t,const OexValue *); _Static_assert(_Generic(&OexTable_SetVariable, Expected_OexTable_SetVariable: 1, default: 0), "signature: OexTable_SetVariable");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *mut OexValue,
        OexUtf8View,
        *const OexValue,
    ) -> OexStatus = OexTable_AppendVariable;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_AppendVariable)(OexCall *,OexValue *,OexUtf8View,const OexValue *); _Static_assert(_Generic(&OexTable_AppendVariable, Expected_OexTable_AppendVariable: 1, default: 0), "signature: OexTable_AppendVariable");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64) -> OexStatus =
        OexTable_RemoveVariable;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_RemoveVariable)(OexCall *,OexValue *,uint64_t); _Static_assert(_Generic(&OexTable_RemoveVariable, Expected_OexTable_RemoveVariable: 1, default: 0), "signature: OexTable_RemoveVariable");"#);
    let _: unsafe extern "C" fn(
        *mut OexCall,
        *const OexValue,
        *mut u32,
        *mut *mut OexValue,
    ) -> OexStatus = OexTable_GetRowNames;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_GetRowNames)(OexCall *,const OexValue *,uint32_t *,OexValue **); _Static_assert(_Generic(&OexTable_GetRowNames, Expected_OexTable_GetRowNames: 1, default: 0), "signature: OexTable_GetRowNames");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue, u64, *const OexUtf8View) -> OexStatus =
        OexTable_SetRowNames;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_SetRowNames)(OexCall *,OexValue *,uint64_t,const OexUtf8View *); _Static_assert(_Generic(&OexTable_SetRowNames, Expected_OexTable_SetRowNames: 1, default: 0), "signature: OexTable_SetRowNames");"#);
    let _: unsafe extern "C" fn(*mut OexCall, *mut OexValue) -> OexStatus = OexTable_ClearRowNames;
    c.push_str(r#"typedef OexStatus (OEX_CALL *Expected_OexTable_ClearRowNames)(OexCall *,OexValue *); _Static_assert(_Generic(&OexTable_ClearRowNames, Expected_OexTable_ClearRowNames: 1, default: 0), "signature: OexTable_ClearRowNames");"#);
    let functions = [
        "OexValue_GetDimensions",
        "OexValue_GetClassName",
        "OexString_Create",
        "OexString_CreateFromUtf16",
        "OexString_SetElementsUtf16",
        "OexString_CreateFromUtf8",
        "OexString_SetElementsUtf8",
        "OexString_GetElementView",
        "OexString_GetElements",
        "OexString_CopyElementUtf8",
        "OexString_SetElementUtf8",
        "OexString_SetElementUtf16",
        "OexString_SetMissing",
        "OexCell_Create",
        "OexCell_GetElement",
        "OexCell_SetElement",
        "OexCell_GetElements",
        "OexCell_SetElements",
        "OexStruct_Create",
        "OexStruct_GetFieldCount",
        "OexStruct_GetFieldName",
        "OexStruct_FindField",
        "OexStruct_GetField",
        "OexStruct_SetField",
        "OexStruct_AddField",
        "OexStruct_RemoveField",
        "OexStruct_SetFieldNames",
        "OexStruct_GetFieldValues",
        "OexStruct_SetFieldValues",
        "OexTable_Create",
        "OexTable_GetRowCount",
        "OexTable_GetVariableCount",
        "OexTable_GetVariableName",
        "OexTable_FindVariable",
        "OexTable_SetVariableNames",
        "OexTable_GetVariable",
        "OexTable_SetVariable",
        "OexTable_AppendVariable",
        "OexTable_RemoveVariable",
        "OexTable_GetRowNames",
        "OexTable_SetRowNames",
        "OexTable_ClearRowNames",
        "OexCall_GetInputCount",
        "OexCall_GetOutputCount",
        "OexCall_BorrowInput",
        "OexCall_TakeInput",
        "OexCall_SetOutput",
        "OexCall_SetError",
        "OexCall_Invoke",
        "OexCall_GetCancellation",
        "OexCancellation_IsRequested",
        "OexValue_GetInfo",
        "OexValue_Retain",
        "OexValue_Release",
        "OexValue_GetFloat64",
        "OexValue_CreateFloat64",
        "OexDense_GetView",
        "OexDense_GetMutableView",
        "OexDenseBuilder_CreateUninitialized",
        "OexDenseBuilder_Commit",
        "OexDenseBuilder_Abort",
        "OexSparse_GetCscView",
        "OexSparsePattern_Create",
        "OexSparsePattern_Retain",
        "OexSparsePattern_Release",
        "OexSparseTripletBuilder_CreateUninitialized",
        "OexSparseTripletBuilder_Commit",
        "OexSparseTripletBuilder_Abort",
        "OexSparsePatternBuilder_CreateUninitialized",
        "OexSparsePatternBuilder_Commit",
        "OexSparsePatternBuilder_Abort",
        "OexNativeObject_BorrowInstance",
        "OexNativeObject_Create",
        "OexPlugin_Init",
    ];
    for suffix in header.split("OEX_CALL ").skip(1) {
        let name = suffix
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next()
            .unwrap();
        if name.starts_with("Oex") {
            assert!(functions.contains(&name), "unbound public function {name}");
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("oex-rust-abi-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let input = directory.join("abi.c");
    let output = directory.join(if cfg!(windows) { "abi.exe" } else { "abi" });
    fs::write(&input, c).unwrap();
    let compiler = std::env::var_os("CC").unwrap_or_else(|| "gcc".into());
    let compilation = Command::new(compiler)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(repository.join("include"))
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()
        .expect("C compiler required for ABI test (set CC)");
    assert!(
        compilation.status.success(),
        "{}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success());
    assert_eq!(
        String::from_utf8(result.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        expected
    );
    fs::remove_dir_all(directory).unwrap();
}

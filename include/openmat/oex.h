#ifndef OPENMAT_OEX_H
#define OPENMAT_OEX_H

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32)
#define OEX_CALL __cdecl
#define OEX_PLUGIN_EXPORT __declspec(dllexport)
#if defined(OEX_BUILDING_BRIDGE)
#define OEX_API __declspec(dllexport)
#else
#define OEX_API __declspec(dllimport)
#endif
#else
#define OEX_CALL
#define OEX_PLUGIN_EXPORT __attribute__((visibility("default")))
#define OEX_API __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define OEX_ABI_VERSION 0x00010003u
#define OEX_ABI_MAJOR 1u
#define OEX_ABI_MINOR 3u

/*
 * OEX ABI v1 rules:
 * - Static descriptors and referenced text/arrays have process lifetime.
 * - Every flags/reserved field is zero unless a later ABI explicitly defines it.
 * - Callbacks are synchronous and must not unwind or throw across the C boundary.
 * - OEX owns no plugin allocation; use paired callbacks for native instances.
 */

typedef uint32_t OexStatus;

#define OEX_OK 0u
#define OEX_ERROR_ARGUMENT 1u
#define OEX_ERROR_TYPE 2u
#define OEX_ERROR_DIMENSION 3u
#define OEX_ERROR_RANGE 4u
#define OEX_ERROR_ALLOCATION 5u
#define OEX_ERROR_CANCELLED 6u
#define OEX_ERROR_UNSUPPORTED 7u
#define OEX_ERROR_PLUGIN 8u
#define OEX_ERROR_ABI 9u
#define OEX_ERROR_STATE 10u
#define OEX_ERROR_CALLBACK 11u
#define OEX_ERROR_ENCODING 12u
#define OEX_ERROR_NOT_FOUND 13u

typedef uint32_t OexValueKind;

#define OEX_VALUE_NOTHING 0u
#define OEX_VALUE_DENSE 1u
#define OEX_VALUE_SPARSE 2u
#define OEX_VALUE_STRING 3u
#define OEX_VALUE_CELL 4u
#define OEX_VALUE_STRUCT 5u
#define OEX_VALUE_TABLE 6u
#define OEX_VALUE_OBJECT 7u
#define OEX_VALUE_FUNCTION 8u
#define OEX_VALUE_GRAPHICS 9u

typedef uint32_t OexDataType;

#define OEX_DATA_NONE 0u
#define OEX_DATA_LOGICAL 1u
#define OEX_DATA_CHAR16 2u
#define OEX_DATA_I8 3u
#define OEX_DATA_U8 4u
#define OEX_DATA_I16 5u
#define OEX_DATA_U16 6u
#define OEX_DATA_I32 7u
#define OEX_DATA_U32 8u
#define OEX_DATA_I64 9u
#define OEX_DATA_U64 10u
#define OEX_DATA_F32 11u
#define OEX_DATA_F64 12u
#define OEX_DATA_COMPLEX_F32 13u
#define OEX_DATA_COMPLEX_F64 14u
#define OEX_DATA_COMPLEX_I8 15u
#define OEX_DATA_COMPLEX_U8 16u
#define OEX_DATA_COMPLEX_I16 17u
#define OEX_DATA_COMPLEX_U16 18u
#define OEX_DATA_COMPLEX_I32 19u
#define OEX_DATA_COMPLEX_U32 20u
#define OEX_DATA_COMPLEX_I64 21u
#define OEX_DATA_COMPLEX_U64 22u

#define OEX_VALUE_FLAG_COMPLEX 1u
#define OEX_CLASS_HANDLE 1u

typedef struct OexCall OexCall;
typedef struct OexCancellation OexCancellation;
typedef struct OexDenseBuilder OexDenseBuilder;
typedef struct OexSparsePattern OexSparsePattern;
typedef struct OexSparseTripletBuilder OexSparseTripletBuilder;
typedef struct OexSparsePatternBuilder OexSparsePatternBuilder;
typedef struct OexValue OexValue;

typedef struct OexUtf8View {
    const char *data;
    uint64_t length;
} OexUtf8View;

#define OEX_UTF8_LITERAL(value) {(value), (uint64_t)(sizeof(value) - 1u)}

typedef struct OexUtf16View {
    const uint16_t *data;
    uint64_t length;
} OexUtf16View;
typedef struct OexStringElementView {
    uint32_t structSize;
    uint32_t isMissing;
    const uint16_t *data;
    uint64_t length;
} OexStringElementView;

typedef struct OexValueInfo {
    uint32_t structSize;
    OexValueKind kind;
    OexDataType dataType;
    uint32_t flags;
    uint32_t rank;
    uint32_t elementSize;
    uint64_t elementCount;
} OexValueInfo;

/* Dense storage is contiguous and column-major. dimensions has rank entries. */
typedef struct OexDenseView {
    uint32_t structSize;
    OexDataType dataType;
    uint32_t rank;
    uint32_t elementSize;
    uint64_t elementCount;
    const uint64_t *dimensions;
    const void *data;
} OexDenseView;

typedef struct OexMutableDenseView {
    uint32_t structSize;
    OexDataType dataType;
    uint32_t rank;
    uint32_t elementSize;
    uint64_t elementCount;
    const uint64_t *dimensions;
    void *data;
} OexMutableDenseView;

/* CSC indices and offsets are zero-based; columnOffsets has columns + 1 entries. */
typedef struct OexSparseCscView {
    uint32_t structSize;
    OexDataType dataType;
    uint32_t elementSize;
    uint32_t reserved;
    uint64_t rows;
    uint64_t columns;
    uint64_t storedCount;
    const uint64_t *columnOffsets;
    const uint64_t *rowIndices;
    const void *values;
} OexSparseCscView;

typedef struct OexSparseTripletView {
    uint32_t structSize;
    OexDataType dataType;
    uint32_t elementSize;
    uint32_t reserved;
    uint64_t rows;
    uint64_t columns;
    uint64_t capacity;
    uint64_t *rowIndices;
    uint64_t *columnIndices;
    void *values;
} OexSparseTripletView;

typedef struct OexSparsePatternValuesView {
    uint32_t structSize;
    OexDataType dataType;
    uint32_t elementSize;
    uint32_t reserved;
    uint64_t storedCount;
    void *values;
} OexSparsePatternValuesView;

#define OEX_DECLARE_COMPLEX_TYPE(name, componentType) \
    typedef struct name {                            \
        componentType re;                            \
        componentType im;                            \
    } name

OEX_DECLARE_COMPLEX_TYPE(OexComplexF32, float);
OEX_DECLARE_COMPLEX_TYPE(OexComplexF64, double);
OEX_DECLARE_COMPLEX_TYPE(OexComplexI8, int8_t);
OEX_DECLARE_COMPLEX_TYPE(OexComplexU8, uint8_t);
OEX_DECLARE_COMPLEX_TYPE(OexComplexI16, int16_t);
OEX_DECLARE_COMPLEX_TYPE(OexComplexU16, uint16_t);
OEX_DECLARE_COMPLEX_TYPE(OexComplexI32, int32_t);
OEX_DECLARE_COMPLEX_TYPE(OexComplexU32, uint32_t);
OEX_DECLARE_COMPLEX_TYPE(OexComplexI64, int64_t);
OEX_DECLARE_COMPLEX_TYPE(OexComplexU64, uint64_t);

#undef OEX_DECLARE_COMPLEX_TYPE

/*
 * OEX_DATA_LOGICAL elements are uint8_t values restricted to 0 and 1.
 * OEX_DATA_CHAR16 elements are exact uint16_t UTF-16 code units.
 * Complex elements use the interleaved OexComplex* structures above.
 */

typedef OexStatus(OEX_CALL *OexFunctionCallback)(OexCall *call);
typedef OexStatus(OEX_CALL *OexNativeConstructor)(OexCall *call, void **instance);
typedef OexStatus(OEX_CALL *OexNativeMethod)(OexCall *call, void *instance);
typedef OexStatus(OEX_CALL *OexNativePropertyGetter)(OexCall *call, void *instance);
typedef OexStatus(OEX_CALL *OexNativePropertySetter)(OexCall *call, void *instance);
typedef void(OEX_CALL *OexNativeDestructor)(void *instance);

typedef struct OexFunctionDefinition {
    uint32_t structSize;
    uint32_t flags;
    OexUtf8View name;
    uint32_t minimumInputs;
    uint32_t maximumInputs;
    uint32_t minimumOutputs;
    uint32_t maximumOutputs;
    OexFunctionCallback invoke;
} OexFunctionDefinition;

typedef struct OexNativeMethodDefinition {
    uint32_t structSize;
    uint32_t flags;
    OexUtf8View name;
    uint32_t minimumInputs;
    uint32_t maximumInputs;
    uint32_t minimumOutputs;
    uint32_t maximumOutputs;
    OexNativeMethod invoke;
} OexNativeMethodDefinition;

typedef struct OexNativePropertyDefinition {
    uint32_t structSize;
    uint32_t flags;
    OexUtf8View name;
    /* Getter: zero inputs, one output. Setter: one input, zero outputs. */
    OexNativePropertyGetter getter;
    OexNativePropertySetter setter;
} OexNativePropertyDefinition;

typedef struct OexNativeClassDefinition {
    uint32_t structSize;
    uint32_t semantics;
    OexUtf8View name;
    OexNativeConstructor constructor;
    OexNativeDestructor destructor;
    const OexNativeMethodDefinition *methods;
    uint64_t methodCount;
    /* Unique non-null process-lifetime address; OpenMat never dereferences it. */
    const void *typeToken;
    const OexNativePropertyDefinition *properties;
    uint64_t propertyCount;
} OexNativeClassDefinition;

typedef struct OexPluginDefinition {
    uint32_t structSize;
    uint32_t requiredAbi;
    uint32_t flags;
    uint32_t reserved;
    OexUtf8View name;
    OexUtf8View version;
    const OexFunctionDefinition *functions;
    uint64_t functionCount;
    const OexNativeClassDefinition *classes;
    uint64_t classCount;
} OexPluginDefinition;

/*
 * OexCall, its cancellation handle, borrowed inputs, and all views are valid
 * only during the callback. BorrowInput returns NULL for an invalid, missing,
 * or already-taken slot. Never release a borrowed value.
 */
OEX_API uint32_t OEX_CALL OexCall_GetInputCount(const OexCall *call);
OEX_API uint32_t OEX_CALL OexCall_GetOutputCount(const OexCall *call);
OEX_API const OexValue *OEX_CALL OexCall_BorrowInput(const OexCall *call, uint32_t index);

/*
 * TakeInput empties the input slot and returns an owned value on success.
 * SetOutput consumes an owned value only on success. On failure the plugin
 * still owns it and must release it. A successful callback must set every
 * requested output exactly once.
 */
OEX_API OexStatus OEX_CALL OexCall_TakeInput(OexCall *call, uint32_t index, OexValue **value);
OEX_API OexStatus OEX_CALL OexCall_SetOutput(OexCall *call, uint32_t index, OexValue *value);
/* Copies both strings and returns OEX_ERROR_PLUGIN for direct callback return. */
OEX_API OexStatus OEX_CALL OexCall_SetError(OexCall *call, OexUtf8View identifier, OexUtf8View message);

/*
 * Synchronously invokes a language-callable value through the owning runtime.
 * callable and inputs are borrowed and remain plugin-owned. On success each
 * requested output receives a new owned value. On failure every output remains
 * NULL and the original language error is propagated out of the active plugin
 * callback; returning OEX_OK cannot suppress it.
 */
OEX_API OexStatus OEX_CALL OexCall_Invoke(
    OexCall *call, const OexValue *callable, uint32_t inputCount,
    const OexValue *const *inputs, uint32_t outputCount, OexValue **outputs);
OEX_API const OexCancellation *OEX_CALL OexCall_GetCancellation(const OexCall *call);
OEX_API uint32_t OEX_CALL OexCancellation_IsRequested(const OexCancellation *cancellation);

OEX_API OexStatus OEX_CALL OexValue_GetInfo(const OexValue *value, OexValueInfo *info);
/* Retain returns a new owned handle. Release accepts owned handles only. */
OEX_API OexStatus OEX_CALL OexValue_Retain(const OexValue *value, OexValue **retained);
OEX_API void OEX_CALL OexValue_Release(OexValue *value);
OEX_API OexStatus OEX_CALL OexValue_GetFloat64(const OexValue *value, double *result);
OEX_API OexStatus OEX_CALL OexValue_CreateFloat64(OexCall *call, double value, OexValue **result);

/*
 * Views alias OpenMat storage and are never retained independently. Mutable
 * views require an owned value and may detach its COW storage before return.
 */
OEX_API OexStatus OEX_CALL OexDense_GetView(const OexValue *value, OexDenseView *view);
OEX_API OexStatus OEX_CALL OexDense_GetMutableView(OexValue *value, OexMutableDenseView *view);

/*
 * The builder allocates the final OpenMat array buffer without initialization.
 * Every element must be initialized before Commit. Commit consumes the builder
 * and returns an owned value; Abort consumes it. Uncommitted builders are
 * automatically aborted when the callback returns.
 */
OEX_API OexStatus OEX_CALL OexDenseBuilder_CreateUninitialized(
    OexCall *call,
    OexDataType dataType,
    uint32_t rank,
    const uint64_t *dimensions,
    OexDenseBuilder **builder,
    OexMutableDenseView *view);
OEX_API OexStatus OEX_CALL OexDenseBuilder_Commit(OexDenseBuilder *builder, OexValue **value);
OEX_API void OEX_CALL OexDenseBuilder_Abort(OexDenseBuilder *builder);

OEX_API OexStatus OEX_CALL OexSparse_GetCscView(const OexValue *value, OexSparseCscView *view);

/*
 * Sparse indices are zero based. A pattern is an owned, persistent handle
 * independent of OexCall; retain/release it explicitly and release the
 * original once when no longer needed. Offsets must start at zero, end at
 * storedCount, and be monotone. Rows must be in range and strictly increasing
 * within each column.
 */
OEX_API OexStatus OEX_CALL OexSparsePattern_Create(
    OexCall *call, uint64_t rows, uint64_t columns, uint64_t storedCount,
    const uint64_t *columnOffsets, const uint64_t *rowIndices,
    OexSparsePattern **pattern);
OEX_API OexStatus OEX_CALL OexSparsePattern_Retain(
    const OexSparsePattern *pattern, OexSparsePattern **retained);
OEX_API void OEX_CALL OexSparsePattern_Release(OexSparsePattern *pattern);

/*
 * Only logical, f64, and complex-f64 sparse value storage is currently
 * supported. Triplet Commit consumes the builder, sorts by column then row,
 * combines duplicates, and removes explicit zeros. Initialize the first
 * storedCount entries of all three buffers before committing. Uncommitted
 * call-scoped builders are aborted automatically when the callback returns.
 */
OEX_API OexStatus OEX_CALL OexSparseTripletBuilder_CreateUninitialized(
    OexCall *call, OexDataType dataType, uint64_t rows, uint64_t columns,
    uint64_t capacity, OexSparseTripletBuilder **builder,
    OexSparseTripletView *view);
OEX_API OexStatus OEX_CALL OexSparseTripletBuilder_Commit(
    OexSparseTripletBuilder *builder, uint64_t storedCount, OexValue **value);
OEX_API void OEX_CALL OexSparseTripletBuilder_Abort(OexSparseTripletBuilder *builder);

/*
 * A pattern builder reuses immutable offsets and rows and allocates only its
 * final value buffer. Initialize all storedCount values before Commit. Commit
 * removes explicit zeros while preserving the pattern's canonical ordering.
 */
OEX_API OexStatus OEX_CALL OexSparsePatternBuilder_CreateUninitialized(
    OexCall *call, const OexSparsePattern *pattern, OexDataType dataType,
    OexSparsePatternBuilder **builder, OexSparsePatternValuesView *view);
OEX_API OexStatus OEX_CALL OexSparsePatternBuilder_Commit(
    OexSparsePatternBuilder *builder, OexValue **value);
OEX_API void OEX_CALL OexSparsePatternBuilder_Abort(OexSparsePatternBuilder *builder);

/*
 * Resolves a scalar native object after checking its exact registered type token.
 * The borrowed instance remains valid only for this callback. The type token is
 * the same pointer supplied by OexNativeClassDefinition.typeToken.
 */
OEX_API OexStatus OEX_CALL OexNativeObject_BorrowInstance(
    OexCall *call, const OexValue *value, const void *expectedTypeToken,
    void **instance);

/*
 * Wraps a newly allocated instance as an ordinary OpenMat handle object.
 * Success transfers instance ownership to OpenMat immediately, even if a later
 * operation in the callback fails. Failure leaves ownership with the plugin.
 * The registered class destructor releases each transferred instance once.
 */
OEX_API OexStatus OEX_CALL OexNativeObject_Create(
    OexCall *call, const void *typeToken, void *instance, OexValue **value);

/* Container API (ABI 1.3).
 * All element/field/variable indices are zero-based; arrays are column-major.
 * Shapes use canonical rank >= 2, as for dense arrays: no trailing singleton
 * dimensions beyond the second. Zero-length dimensions are supported.
 * Create/Get outputs are owned handles; release them. Set borrows its inputs
 * and requires an owned target. Failed operations leave the target unchanged.
 * Gets follow language copy semantics; mutating a returned child requires Set
 * to write it back. Handle objects retain their shared-instance semantics.
 * Views borrow storage: discard before source mutation/release or runtime
 * reentry, and no later than callback return. Class-name text is call-owned.
 * NULL + zero capacity queries GetDimensions/CopyElementUtf8 required length;
 * insufficient capacity returns RANGE and writes no buffer data. UTF-8 lengths
 * count bytes, UTF-16 lengths count code units; neither includes a terminator.
 * UTF-8 conversion is strict (ENCODING on invalid UTF-8/unpaired surrogate).
 * Missing is independent of empty text. CopyElementUtf8 reports it separately.
 * String arrays start with empty strings; Cell/Struct entries start with [].
 * Batch string missing masks may be NULL (all present), else contain only 0/1.
 * UTF-16 input views require structSize and isMissing=0/1; missing payload is empty.
 * Find returns NOT_FOUND when absent. Names follow current runtime validation.
 * Table creation/append/replacement require matching first dimensions, including
 * zero-variable tables. Row-name presence is separate from a zero-length list.
 * Struct field-values APIs exchange a Cell with the exact struct array shape.
 * Output handles (including GetElements results) are NULL on failure.
 */

OEX_API OexStatus OEX_CALL OexValue_GetDimensions(
    const OexValue *value, uint32_t capacity, uint64_t *dimensions, uint32_t *rank);
OEX_API OexStatus OEX_CALL OexValue_GetClassName(
    OexCall *call, const OexValue *value, OexUtf8View *name);
OEX_API OexStatus OEX_CALL OexString_Create(
    OexCall *call, uint32_t rank, const uint64_t *dimensions, OexValue **result);
OEX_API OexStatus OEX_CALL OexString_CreateFromUtf16(
    OexCall *call, uint32_t rank, const uint64_t *dimensions, uint64_t count,
    const OexStringElementView *elements, OexValue **result);
OEX_API OexStatus OEX_CALL OexString_SetElementsUtf16(
    OexCall *call, OexValue *value, uint64_t start, uint64_t count,
    const OexStringElementView *elements);
OEX_API OexStatus OEX_CALL OexString_CreateFromUtf8(
    OexCall *call, uint32_t rank, const uint64_t *dimensions, uint64_t count,
    const OexUtf8View *elements, const uint8_t *missing, OexValue **result);
OEX_API OexStatus OEX_CALL OexString_SetElementsUtf8(
    OexCall *call, OexValue *value, uint64_t start, uint64_t count, const OexUtf8View *elements,
    const uint8_t *missing);
OEX_API OexStatus OEX_CALL OexString_GetElementView(
    const OexValue *value, uint64_t index, OexStringElementView *view);
OEX_API OexStatus OEX_CALL OexString_GetElements(
    const OexValue *value, uint64_t start, uint64_t count, OexStringElementView *views);
OEX_API OexStatus OEX_CALL OexString_CopyElementUtf8(
    const OexValue *value, uint64_t index, char *buffer, uint64_t capacity, uint64_t *required,
    uint32_t *isMissing);
OEX_API OexStatus OEX_CALL OexString_SetElementUtf8(
    OexCall *call, OexValue *value, uint64_t index, OexUtf8View text);
OEX_API OexStatus OEX_CALL OexString_SetElementUtf16(
    OexCall *call, OexValue *value, uint64_t index, OexUtf16View text);
OEX_API OexStatus OEX_CALL OexString_SetMissing(
    OexCall *call, OexValue *value, uint64_t index);
OEX_API OexStatus OEX_CALL OexCell_Create(
    OexCall *call, uint32_t rank, const uint64_t *dimensions, OexValue **result);
OEX_API OexStatus OEX_CALL OexCell_GetElement(
    OexCall *call, const OexValue *value, uint64_t index, OexValue **result);
OEX_API OexStatus OEX_CALL OexCell_SetElement(
    OexCall *call, OexValue *value, uint64_t index, const OexValue *element);
OEX_API OexStatus OEX_CALL OexCell_GetElements(
    OexCall *call, const OexValue *value, uint64_t start, uint64_t count, OexValue **results);
OEX_API OexStatus OEX_CALL OexCell_SetElements(
    OexCall *call, OexValue *value, uint64_t start, uint64_t count,
    const OexValue *const *elements);
OEX_API OexStatus OEX_CALL OexStruct_Create(
    OexCall *call, uint32_t rank, const uint64_t *dimensions, uint64_t fieldCount,
    const OexUtf8View *fieldNames, OexValue **result);
OEX_API OexStatus OEX_CALL OexStruct_GetFieldCount(
    const OexValue *value, uint64_t *count);
OEX_API OexStatus OEX_CALL OexStruct_GetFieldName(
    const OexValue *value, uint64_t fieldIndex, OexUtf8View *name);
OEX_API OexStatus OEX_CALL OexStruct_FindField(
    const OexValue *value, OexUtf8View name, uint64_t *index);
OEX_API OexStatus OEX_CALL OexStruct_GetField(
    OexCall *call, const OexValue *value, uint64_t elementIndex, uint64_t fieldIndex,
    OexValue **result);
OEX_API OexStatus OEX_CALL OexStruct_SetField(
    OexCall *call, OexValue *value, uint64_t elementIndex, uint64_t fieldIndex,
    const OexValue *element);
OEX_API OexStatus OEX_CALL OexStruct_AddField(
    OexCall *call, OexValue *value, OexUtf8View name);
OEX_API OexStatus OEX_CALL OexStruct_RemoveField(
    OexCall *call, OexValue *value, uint64_t fieldIndex);
OEX_API OexStatus OEX_CALL OexStruct_SetFieldNames(
    OexCall *call, OexValue *value, uint64_t count, const OexUtf8View *names);
OEX_API OexStatus OEX_CALL OexStruct_GetFieldValues(
    OexCall *call, const OexValue *value, uint64_t fieldIndex, OexValue **result);
OEX_API OexStatus OEX_CALL OexStruct_SetFieldValues(
    OexCall *call, OexValue *value, uint64_t fieldIndex, const OexValue *elements);
OEX_API OexStatus OEX_CALL OexTable_Create(
    OexCall *call, uint64_t rowCount, uint64_t variableCount, const OexUtf8View *variableNames,
    const OexValue *const *variables, OexValue **result);
OEX_API OexStatus OEX_CALL OexTable_GetRowCount(
    const OexValue *value, uint64_t *count);
OEX_API OexStatus OEX_CALL OexTable_GetVariableCount(
    const OexValue *value, uint64_t *count);
OEX_API OexStatus OEX_CALL OexTable_GetVariableName(
    const OexValue *value, uint64_t index, OexUtf8View *name);
OEX_API OexStatus OEX_CALL OexTable_FindVariable(
    const OexValue *value, OexUtf8View name, uint64_t *index);
OEX_API OexStatus OEX_CALL OexTable_SetVariableNames(
    OexCall *call, OexValue *value, uint64_t count, const OexUtf8View *names);
OEX_API OexStatus OEX_CALL OexTable_GetVariable(
    OexCall *call, const OexValue *value, uint64_t index, OexValue **result);
OEX_API OexStatus OEX_CALL OexTable_SetVariable(
    OexCall *call, OexValue *value, uint64_t index, const OexValue *variable);
OEX_API OexStatus OEX_CALL OexTable_AppendVariable(
    OexCall *call, OexValue *value, OexUtf8View name, const OexValue *variable);
OEX_API OexStatus OEX_CALL OexTable_RemoveVariable(
    OexCall *call, OexValue *value, uint64_t index);
OEX_API OexStatus OEX_CALL OexTable_GetRowNames(
    OexCall *call, const OexValue *value, uint32_t *hasNames, OexValue **result);
OEX_API OexStatus OEX_CALL OexTable_SetRowNames(
    OexCall *call, OexValue *value, uint64_t count, const OexUtf8View *names);
OEX_API OexStatus OEX_CALL OexTable_ClearRowNames(
    OexCall *call, OexValue *value);

/* Every plugin exports exactly this ordinary entrypoint and returns static data. */
OEX_PLUGIN_EXPORT const OexPluginDefinition *OEX_CALL OexPlugin_Init(void);

#define OEX_TRY(expression)                    \
    do {                                       \
        OexStatus oexTryStatus = (expression); \
        if (oexTryStatus != OEX_OK) {           \
            return oexTryStatus;                \
        }                                      \
    } while (0)

#if defined(__cplusplus)
static_assert(sizeof(OexUtf16View) == 16, "OEX ABI layout mismatch");
static_assert(sizeof(OexStringElementView) == 24, "OEX ABI layout mismatch");
static_assert(sizeof(OexUtf8View) == 16, "OEX ABI layout mismatch");
static_assert(sizeof(OexValueInfo) == 32, "OEX ABI layout mismatch");
static_assert(sizeof(OexDenseView) == 40, "OEX ABI layout mismatch");
static_assert(sizeof(OexMutableDenseView) == 40, "OEX ABI layout mismatch");
static_assert(sizeof(OexSparseCscView) == 64, "OEX ABI layout mismatch");
static_assert(sizeof(OexSparseTripletView) == 64, "OEX ABI layout mismatch");
static_assert(sizeof(OexSparsePatternValuesView) == 32, "OEX ABI layout mismatch");
static_assert(sizeof(OexFunctionDefinition) == 48, "OEX ABI layout mismatch");
static_assert(sizeof(OexNativeMethodDefinition) == 48, "OEX ABI layout mismatch");
static_assert(sizeof(OexNativePropertyDefinition) == 40, "OEX ABI layout mismatch");
static_assert(sizeof(OexNativeClassDefinition) == 80, "OEX ABI layout mismatch");
static_assert(sizeof(OexPluginDefinition) == 80, "OEX ABI layout mismatch");
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(OexUtf16View) == 16, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexStringElementView) == 24, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexUtf8View) == 16, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexValueInfo) == 32, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexDenseView) == 40, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexMutableDenseView) == 40, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexSparseCscView) == 64, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexSparseTripletView) == 64, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexSparsePatternValuesView) == 32, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexFunctionDefinition) == 48, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexNativeMethodDefinition) == 48, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexNativePropertyDefinition) == 40, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexNativeClassDefinition) == 80, "OEX ABI layout mismatch");
_Static_assert(sizeof(OexPluginDefinition) == 80, "OEX ABI layout mismatch");
#endif

#ifdef __cplusplus
}
#endif

#endif

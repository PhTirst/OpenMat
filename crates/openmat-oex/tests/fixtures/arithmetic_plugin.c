#include <openmat/oex.h>
#include <stdlib.h>
#include <string.h>

typedef struct NativeCounter {
    double value;
    OexSparsePattern *pattern;
} NativeCounter;

typedef struct NativeReceiver {
    double observed;
} NativeReceiver;

static uint64_t DESTROYED_COUNTERS = 0;
static uint64_t DESTROYED_RECEIVERS = 0;
static const char COUNTER_TYPE_TOKEN = 0;
static const char RECEIVER_TYPE_TOKEN = 0;

static OexStatus OEX_CALL add2(OexCall *call) {
    double left = 0.0;
    double right = 0.0;
    OexValue *result = NULL;

    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &left));
    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 1), &right));
    OEX_TRY(OexValue_CreateFloat64(call, left + right, &result));
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL scale2_builder(OexCall *call) {
    const OexValue *input = OexCall_BorrowInput(call, 0);
    OexDenseView source;
    OexMutableDenseView destination;
    OexDenseBuilder *builder = NULL;
    OexValue *result = NULL;
    uint64_t index;

    OEX_TRY(OexDense_GetView(input, &source));
    if (source.dataType != OEX_DATA_F64) {
        return OEX_ERROR_TYPE;
    }
    OEX_TRY(OexDenseBuilder_CreateUninitialized(
        call,
        OEX_DATA_F64,
        source.rank,
        source.dimensions,
        &builder,
        &destination));
    for (index = 0; index < source.elementCount; ++index) {
        ((double *)destination.data)[index] = 2.0 * ((const double *)source.data)[index];
    }
    OEX_TRY(OexDenseBuilder_Commit(builder, &result));
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL scale2_taken(OexCall *call) {
    OexValue *value = NULL;
    OexMutableDenseView view;
    uint64_t index;
    OexStatus status;

    OEX_TRY(OexCall_TakeInput(call, 0, &value));
    status = OexDense_GetMutableView(value, &view);
    if (status != OEX_OK) {
        OexValue_Release(value);
        return status;
    }
    if (view.dataType != OEX_DATA_F64) {
        OexValue_Release(value);
        return OEX_ERROR_TYPE;
    }
    for (index = 0; index < view.elementCount; ++index) {
        ((double *)view.data)[index] *= 2.0;
    }
    return OexCall_SetOutput(call, 0, value);
}

static OexStatus OEX_CALL copy_dense(OexCall *call) {
    OexDenseView source;
    OexMutableDenseView destination;
    OexDenseBuilder *builder = NULL;
    OexValue *result = NULL;
    size_t bytes;

    OEX_TRY(OexDense_GetView(OexCall_BorrowInput(call, 0), &source));
    if (source.elementSize != 0 && source.elementCount > SIZE_MAX / source.elementSize) {
        return OEX_ERROR_RANGE;
    }
    OEX_TRY(OexDenseBuilder_CreateUninitialized(
        call,
        source.dataType,
        source.rank,
        source.dimensions,
        &builder,
        &destination));
    bytes = (size_t)source.elementCount * source.elementSize;
    if (bytes != 0) {
        memcpy(destination.data, source.data, bytes);
    }
    OEX_TRY(OexDenseBuilder_Commit(builder, &result));
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL fail_with_error(OexCall *call) {
    const OexUtf8View identifier = OEX_UTF8_LITERAL("oex:testFailure");
    const OexUtf8View message = OEX_UTF8_LITERAL("intentional plugin failure");
    OexSparseTripletBuilder *unfinished = NULL;
    OexSparseTripletView view;
    OEX_TRY(OexSparseTripletBuilder_CreateUninitialized(
        call, OEX_DATA_F64, 2, 2, 4, &unfinished, &view));
    return OexCall_SetError(call, identifier, message);
}

static OexStatus OEX_CALL missing_output(OexCall *call) {
    (void)call;
    return OEX_OK;
}

static OexStatus OEX_CALL invoke_callback(OexCall *call) {
    const OexValue *callback_inputs[1];
    OexValue *callback_outputs[2] = {NULL, NULL};
    uint32_t output_count = OexCall_GetOutputCount(call);
    uint32_t index;
    OexStatus status;

    if (OexCall_GetInputCount(call) != 2 || output_count > 2) {
        return OEX_ERROR_ARGUMENT;
    }
    callback_inputs[0] = OexCall_BorrowInput(call, 1);
    status = OexCall_Invoke(
        call,
        OexCall_BorrowInput(call, 0),
        1,
        callback_inputs,
        output_count,
        callback_outputs);
    if (status != OEX_OK) {
        return status;
    }
    for (index = 0; index < output_count; ++index) {
        status = OexCall_SetOutput(call, index, callback_outputs[index]);
        if (status != OEX_OK) {
            uint32_t remaining;
            OexValue_Release(callback_outputs[index]);
            for (remaining = index + 1; remaining < output_count; ++remaining) {
                OexValue_Release(callback_outputs[remaining]);
            }
            return status;
        }
    }
    return OEX_OK;
}

static OexStatus OEX_CALL destroyed_count(OexCall *call) {
    OexValue *result = NULL;
    OEX_TRY(OexValue_CreateFloat64(call, (double)DESTROYED_COUNTERS, &result));
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL destroyed_receiver_count(OexCall *call) {
    OexValue *result = NULL;
    OEX_TRY(OexValue_CreateFloat64(call, (double)DESTROYED_RECEIVERS, &result));
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL sparse_triplet(OexCall *call) {
    OexSparseTripletBuilder *builder = NULL;
    OexSparseTripletView view;
    OexSparseCscView csc;
    OexValue *result = NULL;
    double *values;

    OEX_TRY(OexSparseTripletBuilder_CreateUninitialized(
        call, OEX_DATA_F64, 3, 3, 5, &builder, &view));
    values = (double *)view.values;

    view.rowIndices[0] = 2; view.columnIndices[0] = 0; values[0] = 4.0;
    view.rowIndices[1] = 0; view.columnIndices[1] = 1; values[1] = 2.0;
    view.rowIndices[2] = 2; view.columnIndices[2] = 0; values[2] = -1.0;
    view.rowIndices[3] = 1; view.columnIndices[3] = 2; values[3] = 0.0;
    view.rowIndices[4] = 0; view.columnIndices[4] = 0; values[4] = 5.0;

    OEX_TRY(OexSparseTripletBuilder_Commit(builder, 5, &result));
    OEX_TRY(OexSparse_GetCscView(result, &csc));
    if (csc.rows != 3 || csc.columns != 3 || csc.storedCount != 3 ||
        csc.columnOffsets[0] != 0 || csc.columnOffsets[1] != 2 ||
        csc.columnOffsets[2] != 3 || csc.columnOffsets[3] != 3) {
        OexValue_Release(result);
        return OEX_ERROR_PLUGIN;
    }
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL counter_construct(OexCall *call, void **instance) {
    NativeCounter *counter;
    OexSparsePattern *pattern = NULL;
    OexStatus status;
    static const uint64_t columnOffsets[] = {0, 2, 3, 4};
    static const uint64_t rowIndices[] = {0, 2, 1, 2};
    double initial = 0.0;
    if (OexCall_GetInputCount(call) != 1) {
        return OEX_ERROR_ARGUMENT;
    }
    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &initial));
    counter = (NativeCounter *)malloc(sizeof(*counter));
    if (counter == NULL) {
        return OEX_ERROR_ALLOCATION;
    }
    counter->value = initial;
    counter->pattern = NULL;
    status = OexSparsePattern_Create(
        call, 3, 3, 4, columnOffsets, rowIndices, &pattern);
    if (status == OEX_OK) {
        status = OexSparsePattern_Retain(pattern, &counter->pattern);
        OexSparsePattern_Release(pattern);
    }
    if (status != OEX_OK) {
        free(counter);
        return status;
    }
    *instance = counter;
    return OEX_OK;
}

static OexStatus OEX_CALL counter_add(OexCall *call, void *instance) {
    NativeCounter *counter = (NativeCounter *)instance;
    OexValue *result = NULL;
    double delta = 0.0;
    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &delta));
    counter->value += delta;
    OEX_TRY(OexValue_CreateFloat64(call, counter->value, &result));
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL counter_value(OexCall *call, void *instance) {
    NativeCounter *counter = (NativeCounter *)instance;
    OexValue *result = NULL;
    OEX_TRY(OexValue_CreateFloat64(call, counter->value, &result));
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL counter_set_value(OexCall *call, void *instance) {
    NativeCounter *counter = (NativeCounter *)instance;
    double value = 0.0;
    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &value));
    counter->value = value;
    return OEX_OK;
}

static OexStatus OEX_CALL counter_assemble(OexCall *call, void *instance) {
    NativeCounter *counter = (NativeCounter *)instance;
    OexSparsePatternBuilder *builder = NULL;
    OexSparsePatternValuesView view;
    OexValue *result = NULL;
    double scale = 0.0;
    double *values;

    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &scale));
    OEX_TRY(OexSparsePatternBuilder_CreateUninitialized(
        call, counter->pattern, OEX_DATA_F64, &builder, &view));
    values = (double *)view.values;
    values[0] = scale;
    values[1] = 0.0;
    values[2] = -scale;
    values[3] = 2.0 * scale;
    OEX_TRY(OexSparsePatternBuilder_Commit(builder, &result));
    return OexCall_SetOutput(call, 0, result);
}

static OexStatus OEX_CALL counter_clone_scaled(OexCall *call, void *instance) {
    NativeCounter *counter = (NativeCounter *)instance;
    NativeCounter *clone;
    OexValue *result = NULL;
    OexStatus status;
    double scale = 0.0;

    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &scale));
    clone = (NativeCounter *)malloc(sizeof(*clone));
    if (clone == NULL) {
        return OEX_ERROR_ALLOCATION;
    }
    clone->value = counter->value * scale;
    clone->pattern = NULL;
    status = OexSparsePattern_Retain(counter->pattern, &clone->pattern);
    if (status != OEX_OK) {
        free(clone);
        return status;
    }
    status = OexNativeObject_Create(call, &COUNTER_TYPE_TOKEN, clone, &result);
    if (status != OEX_OK) {
        OexSparsePattern_Release(clone->pattern);
        free(clone);
        return status;
    }
    status = OexCall_SetOutput(call, 0, result);
    if (status != OEX_OK) {
        OexValue_Release(result);
    }
    return status;
}

static OexStatus OEX_CALL counter_clone_fail(OexCall *call, void *instance) {
    const OexUtf8View identifier = OEX_UTF8_LITERAL("oex:cloneFailure");
    const OexUtf8View message = OEX_UTF8_LITERAL("failure after native object creation");
    NativeCounter *counter = (NativeCounter *)instance;
    NativeCounter *clone;
    OexValue *created = NULL;
    OexStatus status;

    clone = (NativeCounter *)malloc(sizeof(*clone));
    if (clone == NULL) {
        return OEX_ERROR_ALLOCATION;
    }
    clone->value = counter->value;
    clone->pattern = NULL;
    status = OexSparsePattern_Retain(counter->pattern, &clone->pattern);
    if (status != OEX_OK) {
        free(clone);
        return status;
    }
    status = OexNativeObject_Create(call, &COUNTER_TYPE_TOKEN, clone, &created);
    if (status != OEX_OK) {
        OexSparsePattern_Release(clone->pattern);
        free(clone);
        return status;
    }
    OexValue_Release(created);
    return OexCall_SetError(call, identifier, message);
}

static OexStatus OEX_CALL receiver_construct(OexCall *call, void **instance) {
    NativeCounter *counter = NULL;
    NativeReceiver *receiver;

    OEX_TRY(OexNativeObject_BorrowInstance(
        call, OexCall_BorrowInput(call, 0), &COUNTER_TYPE_TOKEN, (void **)&counter));
    receiver = (NativeReceiver *)malloc(sizeof(*receiver));
    if (receiver == NULL) {
        return OEX_ERROR_ALLOCATION;
    }
    receiver->observed = counter->value;
    *instance = receiver;
    return OEX_OK;
}

static OexStatus OEX_CALL receiver_value(OexCall *call, void *instance) {
    NativeReceiver *receiver = (NativeReceiver *)instance;
    OexValue *result = NULL;
    OEX_TRY(OexValue_CreateFloat64(call, receiver->observed, &result));
    return OexCall_SetOutput(call, 0, result);
}

static void OEX_CALL receiver_destroy(void *instance) {
    ++DESTROYED_RECEIVERS;
    free(instance);
}

static void OEX_CALL counter_destroy(void *instance) {
    NativeCounter *counter = (NativeCounter *)instance;
    ++DESTROYED_COUNTERS;
    OexSparsePattern_Release(counter->pattern);
    free(instance);
}

static const OexFunctionDefinition FUNCTIONS[] = {
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("add2"), 2, 2, 1, 1, add2},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("scale2_builder"), 1, 1, 1, 1, scale2_builder},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("scale2_taken"), 1, 1, 1, 1, scale2_taken},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("copy_dense"), 1, 1, 1, 1, copy_dense},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("fail_with_error"), 0, 0, 1, 1, fail_with_error},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("missing_output"), 0, 0, 1, 1, missing_output},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("invoke_callback"), 2, 2, 0, 2, invoke_callback},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("destroyed_count"), 0, 0, 1, 1, destroyed_count},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("sparse_triplet"), 0, 0, 1, 1, sparse_triplet},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("destroyed_receiver_count"), 0, 0, 1, 1, destroyed_receiver_count},
};

static const OexNativeMethodDefinition COUNTER_METHODS[] = {
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("add"), 1, 1, 1, 1, counter_add},
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("value"), 0, 0, 1, 1, counter_value},
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("assemble"), 1, 1, 1, 1, counter_assemble},
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("cloneScaled"), 1, 1, 1, 1, counter_clone_scaled},
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("cloneFail"), 0, 0, 0, 0, counter_clone_fail},
};

static const OexNativeMethodDefinition RECEIVER_METHODS[] = {
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("value"), 0, 0, 1, 1, receiver_value},
};

static const OexNativePropertyDefinition COUNTER_PROPERTIES[] = {
    {sizeof(OexNativePropertyDefinition), 0, OEX_UTF8_LITERAL("Value"), counter_value,
     counter_set_value},
    {sizeof(OexNativePropertyDefinition), 0, OEX_UTF8_LITERAL("ReadOnly"), counter_value, NULL},
    {sizeof(OexNativePropertyDefinition), 0, OEX_UTF8_LITERAL("WriteOnly"), NULL,
     counter_set_value},
};

static const OexNativeClassDefinition CLASSES[] = {
    {sizeof(OexNativeClassDefinition), OEX_CLASS_HANDLE, OEX_UTF8_LITERAL("NativeCounter"),
     counter_construct, counter_destroy, COUNTER_METHODS,
     sizeof(COUNTER_METHODS) / sizeof(COUNTER_METHODS[0]), &COUNTER_TYPE_TOKEN, COUNTER_PROPERTIES,
     sizeof(COUNTER_PROPERTIES) / sizeof(COUNTER_PROPERTIES[0])},
    {sizeof(OexNativeClassDefinition), OEX_CLASS_HANDLE, OEX_UTF8_LITERAL("NativeReceiver"),
     receiver_construct, receiver_destroy, RECEIVER_METHODS,
     sizeof(RECEIVER_METHODS) / sizeof(RECEIVER_METHODS[0]), &RECEIVER_TYPE_TOKEN, NULL, 0},
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition),
    OEX_ABI_VERSION,
    0,
    0,
    OEX_UTF8_LITERAL("OpenMat OEX test plugin"),
    OEX_UTF8_LITERAL("1.0.0"),
    FUNCTIONS,
    sizeof(FUNCTIONS) / sizeof(FUNCTIONS[0]),
    CLASSES,
    sizeof(CLASSES) / sizeof(CLASSES[0]),
};

OEX_PLUGIN_EXPORT const OexPluginDefinition *OEX_CALL OexPlugin_Init(void) {
    return &PLUGIN;
}

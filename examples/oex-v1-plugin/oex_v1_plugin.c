#include <openmat/oex.h>

#include <stdint.h>
#include <stdlib.h>

typedef struct OexPatternAssembler {
    OexSparsePattern *pattern;
    double scale;
} OexPatternAssembler;

static const char ASSEMBLER_TYPE_TOKEN = 0;

static OexStatus set_float64_output(OexCall *call, uint32_t index, double scalar) {
    OexValue *value = NULL;
    OexStatus status = OexValue_CreateFloat64(call, scalar, &value);
    if (status != OEX_OK) {
        return status;
    }
    status = OexCall_SetOutput(call, index, value);
    if (status != OEX_OK) {
        OexValue_Release(value);
    }
    return status;
}

/* Reuses an owned input buffer after OpenMat performs any required COW detach. */
static OexStatus OEX_CALL scale_in_place(OexCall *call) {
    OexMutableDenseView view;
    const OexCancellation *cancellation;
    OexValue *value = NULL;
    OexStatus status;
    double factor = 0.0;
    double *elements;
    uint64_t index;

    status = OexCall_TakeInput(call, 0, &value);
    if (status != OEX_OK) {
        return status;
    }
    status = OexValue_GetFloat64(OexCall_BorrowInput(call, 1), &factor);
    if (status != OEX_OK) {
        OexValue_Release(value);
        return status;
    }
    status = OexDense_GetMutableView(value, &view);
    if (status != OEX_OK) {
        OexValue_Release(value);
        return status;
    }
    if (view.dataType != OEX_DATA_F64) {
        OexValue_Release(value);
        return OEX_ERROR_TYPE;
    }

    cancellation = OexCall_GetCancellation(call);
    elements = (double *)view.data;
    for (index = 0; index < view.elementCount; ++index) {
        if ((index & 4095u) == 0u && OexCancellation_IsRequested(cancellation) != 0u) {
            OexValue_Release(value);
            return OEX_ERROR_CANCELLED;
        }
        elements[index] *= factor;
    }
    status = OexCall_SetOutput(call, 0, value);
    if (status != OEX_OK) {
        OexValue_Release(value);
    }
    return status;
}

/* Calls an arbitrary m-language function handle with one input and one output. */
static OexStatus OEX_CALL apply1(OexCall *call) {
    const OexValue *inputs[1];
    OexValue *output = NULL;
    OexStatus status;

    inputs[0] = OexCall_BorrowInput(call, 1);
    status = OexCall_Invoke(
        call, OexCall_BorrowInput(call, 0), 1, inputs, 1, &output);
    if (status != OEX_OK) {
        return status;
    }
    status = OexCall_SetOutput(call, 0, output);
    if (status != OEX_OK) {
        OexValue_Release(output);
    }
    return status;
}

/* Demonstrates one-off COO-style filling followed by canonical CSC commit. */
static OexStatus OEX_CALL triplet_demo(OexCall *call) {
    OexSparseTripletBuilder *builder = NULL;
    OexSparseTripletView view;
    OexValue *value = NULL;
    OexStatus status;
    double *entries;

    OEX_TRY(OexSparseTripletBuilder_CreateUninitialized(
        call, OEX_DATA_F64, 3, 3, 5, &builder, &view));
    entries = (double *)view.values;

    view.rowIndices[0] = 0;
    view.columnIndices[0] = 0;
    entries[0] = 2.0;
    view.rowIndices[1] = 0;
    view.columnIndices[1] = 0;
    entries[1] = 3.0;
    view.rowIndices[2] = 1;
    view.columnIndices[2] = 1;
    entries[2] = 4.0;
    view.rowIndices[3] = 2;
    view.columnIndices[3] = 2;
    entries[3] = 5.0;
    view.rowIndices[4] = 2;
    view.columnIndices[4] = 0;
    entries[4] = 0.0;

    status = OexSparseTripletBuilder_Commit(builder, 5, &value);
    if (status != OEX_OK) {
        return status;
    }
    status = OexCall_SetOutput(call, 0, value);
    if (status != OEX_OK) {
        OexValue_Release(value);
    }
    return status;
}

static OexStatus OEX_CALL assembler_construct(OexCall *call, void **instance) {
    static const uint64_t column_offsets[] = {0, 1, 2, 3};
    static const uint64_t row_indices[] = {0, 1, 2};
    const OexUtf8View identifier = OEX_UTF8_LITERAL("oex:example:constructor");
    const OexUtf8View message = OEX_UTF8_LITERAL("OexPatternAssembler requires one scale");
    OexPatternAssembler *assembler;
    OexStatus status;
    double scale = 0.0;

    *instance = NULL;
    if (OexCall_GetInputCount(call) != 1u) {
        return OexCall_SetError(call, identifier, message);
    }
    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &scale));

    assembler = (OexPatternAssembler *)calloc(1, sizeof(*assembler));
    if (assembler == NULL) {
        return OEX_ERROR_ALLOCATION;
    }
    status = OexSparsePattern_Create(
        call, 3, 3, 3, column_offsets, row_indices, &assembler->pattern);
    if (status != OEX_OK) {
        free(assembler);
        return status;
    }
    assembler->scale = scale;
    *instance = assembler;
    return OEX_OK;
}

static void OEX_CALL assembler_destroy(void *instance) {
    OexPatternAssembler *assembler = (OexPatternAssembler *)instance;
    if (assembler != NULL) {
        OexSparsePattern_Release(assembler->pattern);
        free(assembler);
    }
}

static OexStatus OEX_CALL assembler_get_scale(OexCall *call, void *instance) {
    const OexPatternAssembler *assembler = (const OexPatternAssembler *)instance;
    return set_float64_output(call, 0, assembler->scale);
}

static OexStatus OEX_CALL assembler_set_scale(OexCall *call, void *instance) {
    OexPatternAssembler *assembler = (OexPatternAssembler *)instance;
    double scale = 0.0;
    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &scale));
    assembler->scale = scale;
    return OEX_OK;
}

static OexStatus OEX_CALL assembler_assemble(OexCall *call, void *instance) {
    const OexPatternAssembler *assembler = (const OexPatternAssembler *)instance;
    OexSparsePatternBuilder *builder = NULL;
    OexSparsePatternValuesView view;
    OexValue *value = NULL;
    OexStatus status;
    double *entries;

    OEX_TRY(OexSparsePatternBuilder_CreateUninitialized(
        call, assembler->pattern, OEX_DATA_F64, &builder, &view));
    entries = (double *)view.values;
    entries[0] = assembler->scale;
    entries[1] = 2.0 * assembler->scale;
    entries[2] = 3.0 * assembler->scale;

    status = OexSparsePatternBuilder_Commit(builder, &value);
    if (status != OEX_OK) {
        return status;
    }
    status = OexCall_SetOutput(call, 0, value);
    if (status != OEX_OK) {
        OexValue_Release(value);
    }
    return status;
}

static OexStatus OEX_CALL assembler_scaled_copy(OexCall *call, void *instance) {
    const OexPatternAssembler *source = (const OexPatternAssembler *)instance;
    OexPatternAssembler *copy;
    OexValue *value = NULL;
    OexStatus status;
    double factor = 0.0;

    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &factor));
    copy = (OexPatternAssembler *)calloc(1, sizeof(*copy));
    if (copy == NULL) {
        return OEX_ERROR_ALLOCATION;
    }
    copy->scale = source->scale * factor;
    status = OexSparsePattern_Retain(source->pattern, &copy->pattern);
    if (status != OEX_OK) {
        free(copy);
        return status;
    }

    status = OexNativeObject_Create(call, &ASSEMBLER_TYPE_TOKEN, copy, &value);
    if (status != OEX_OK) {
        assembler_destroy(copy);
        return status;
    }
    status = OexCall_SetOutput(call, 0, value);
    if (status != OEX_OK) {
        /* OpenMat owns copy after Create succeeds; release only the value wrapper. */
        OexValue_Release(value);
    }
    return status;
}

static OexStatus OEX_CALL assembler_scale(OexCall *call) {
    OexPatternAssembler *assembler = NULL;
    OEX_TRY(OexNativeObject_BorrowInstance(
        call,
        OexCall_BorrowInput(call, 0),
        &ASSEMBLER_TYPE_TOKEN,
        (void **)&assembler));
    return set_float64_output(call, 0, assembler->scale);
}

static const OexFunctionDefinition FUNCTIONS[] = {
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("oex_scale_in_place"), 2, 2, 1, 1,
     scale_in_place},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("oex_apply1"), 2, 2, 1, 1, apply1},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("oex_triplet_demo"), 0, 0, 1, 1,
     triplet_demo},
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("oex_assembler_scale"), 1, 1, 1, 1,
     assembler_scale},
};

static const OexNativeMethodDefinition ASSEMBLER_METHODS[] = {
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("assemble"), 0, 0, 1, 1,
     assembler_assemble},
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("scaledCopy"), 1, 1, 1, 1,
     assembler_scaled_copy},
};

static const OexNativePropertyDefinition ASSEMBLER_PROPERTIES[] = {
    {sizeof(OexNativePropertyDefinition), 0, OEX_UTF8_LITERAL("Scale"), assembler_get_scale,
     assembler_set_scale},
};

static const OexNativeClassDefinition CLASSES[] = {
    {sizeof(OexNativeClassDefinition), OEX_CLASS_HANDLE, OEX_UTF8_LITERAL("OexPatternAssembler"),
     assembler_construct, assembler_destroy, ASSEMBLER_METHODS,
     sizeof(ASSEMBLER_METHODS) / sizeof(ASSEMBLER_METHODS[0]), &ASSEMBLER_TYPE_TOKEN,
     ASSEMBLER_PROPERTIES, sizeof(ASSEMBLER_PROPERTIES) / sizeof(ASSEMBLER_PROPERTIES[0])},
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition),
    OEX_ABI_VERSION,
    0,
    0,
    OEX_UTF8_LITERAL("OpenMat OEX v1 example"),
    OEX_UTF8_LITERAL("1.0.0"),
    FUNCTIONS,
    sizeof(FUNCTIONS) / sizeof(FUNCTIONS[0]),
    CLASSES,
    sizeof(CLASSES) / sizeof(CLASSES[0]),
};

OEX_PLUGIN_EXPORT const OexPluginDefinition *OEX_CALL OexPlugin_Init(void) {
    return &PLUGIN;
}

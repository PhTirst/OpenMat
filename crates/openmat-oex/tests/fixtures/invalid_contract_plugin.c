#include <openmat/oex.h>

#include <stdlib.h>

#ifndef OEX_INVALID_CASE
#error "compile with OEX_INVALID_CASE"
#endif

#if OEX_INVALID_CASE == 1

static OexStatus OEX_CALL valid_callback(OexCall *call) {
    (void)call;
    return OEX_OK;
}

static const OexFunctionDefinition FUNCTIONS[] = {
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("valid_first"), 0, 0, 0, 0,
     valid_callback},
    {sizeof(OexFunctionDefinition), 1, OEX_UTF8_LITERAL("invalid_flags"), 0, 0, 0, 0,
     valid_callback},
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition), OEX_ABI_VERSION, 0, 0,
    OEX_UTF8_LITERAL("invalid function flags"), OEX_UTF8_LITERAL("1.0.0"),
    FUNCTIONS, sizeof(FUNCTIONS) / sizeof(FUNCTIONS[0]), NULL, 0,
};

#elif OEX_INVALID_CASE == 2

static const char TYPE_TOKEN = 0;

static OexStatus OEX_CALL construct(OexCall *call, void **instance) {
    (void)call;
    *instance = malloc(1);
    return *instance == NULL ? OEX_ERROR_ALLOCATION : OEX_OK;
}

static void OEX_CALL destroy(void *instance) {
    free(instance);
}

static const OexNativePropertyDefinition PROPERTIES[] = {
    {sizeof(OexNativePropertyDefinition), 0, OEX_UTF8_LITERAL("Invalid"), NULL, NULL},
};

static const OexNativeClassDefinition CLASSES[] = {
    {sizeof(OexNativeClassDefinition), OEX_CLASS_HANDLE, OEX_UTF8_LITERAL("InvalidProperty"),
     construct, destroy, NULL, 0, &TYPE_TOKEN, PROPERTIES,
     sizeof(PROPERTIES) / sizeof(PROPERTIES[0])},
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition), OEX_ABI_VERSION, 0, 0,
    OEX_UTF8_LITERAL("invalid empty property"), OEX_UTF8_LITERAL("1.0.0"),
    NULL, 0, CLASSES, sizeof(CLASSES) / sizeof(CLASSES[0]),
};

#elif OEX_INVALID_CASE == 3

static const char TYPE_TOKEN = 0;

static OexStatus OEX_CALL construct(OexCall *call, void **instance) {
    (void)call;
    *instance = malloc(1);
    return *instance == NULL ? OEX_ERROR_ALLOCATION : OEX_OK;
}

static void OEX_CALL destroy(void *instance) {
    free(instance);
}

static OexStatus OEX_CALL member(OexCall *call, void *instance) {
    (void)call;
    (void)instance;
    return OEX_OK;
}

static const OexNativeMethodDefinition METHODS[] = {
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("Value"), 0, 0, 0, 0, member},
};

static const OexNativePropertyDefinition PROPERTIES[] = {
    {sizeof(OexNativePropertyDefinition), 0, OEX_UTF8_LITERAL("Value"), member, NULL},
};

static const OexNativeClassDefinition CLASSES[] = {
    {sizeof(OexNativeClassDefinition), OEX_CLASS_HANDLE, OEX_UTF8_LITERAL("ConflictingMembers"),
     construct, destroy, METHODS, sizeof(METHODS) / sizeof(METHODS[0]), &TYPE_TOKEN, PROPERTIES,
     sizeof(PROPERTIES) / sizeof(PROPERTIES[0])},
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition), OEX_ABI_VERSION, 0, 0,
    OEX_UTF8_LITERAL("invalid member conflict"), OEX_UTF8_LITERAL("1.0.0"),
    NULL, 0, CLASSES, sizeof(CLASSES) / sizeof(CLASSES[0]),
};

#elif OEX_INVALID_CASE == 4

static const OexFunctionDefinition FUNCTIONS[] = {
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("null_callback"), 0, 0, 0, 0, NULL},
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition), OEX_ABI_VERSION, 0, 0,
    OEX_UTF8_LITERAL("invalid null callback"), OEX_UTF8_LITERAL("1.0.0"),
    FUNCTIONS, sizeof(FUNCTIONS) / sizeof(FUNCTIONS[0]), NULL, 0,
};

#elif OEX_INVALID_CASE == 5

static OexStatus OEX_CALL valid_callback(OexCall *call) {
    (void)call;
    return OEX_OK;
}

static const OexFunctionDefinition FUNCTIONS[] = {
    {sizeof(OexFunctionDefinition) - 8, 0, OEX_UTF8_LITERAL("undersized"), 0, 0, 0, 0,
     valid_callback},
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition), OEX_ABI_VERSION, 0, 0,
    OEX_UTF8_LITERAL("invalid descriptor size"), OEX_UTF8_LITERAL("1.0.0"),
    FUNCTIONS, sizeof(FUNCTIONS) / sizeof(FUNCTIONS[0]), NULL, 0,
};

#else
#error "unknown OEX_INVALID_CASE"
#endif

OEX_PLUGIN_EXPORT const OexPluginDefinition *OEX_CALL OexPlugin_Init(void) {
    return &PLUGIN;
}

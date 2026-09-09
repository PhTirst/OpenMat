#include <openmat/oex.h>

static OexStatus OEX_CALL never_called(OexCall *call) {
    (void)call;
    return OEX_OK;
}

static const OexFunctionDefinition FUNCTION = {
    sizeof(OexFunctionDefinition),
    0,
    OEX_UTF8_LITERAL("invalid"),
    0,
    0,
    0,
    0,
    never_called,
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition),
    0x00020000u,
    0,
    0,
    OEX_UTF8_LITERAL("invalid ABI plugin"),
    OEX_UTF8_LITERAL("1.0.0"),
    &FUNCTION,
    1,
    NULL,
    0,
};

OEX_PLUGIN_EXPORT const OexPluginDefinition *OEX_CALL OexPlugin_Init(void) {
    return &PLUGIN;
}

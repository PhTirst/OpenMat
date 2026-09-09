#include <openmat/oex.h>
#include <string.h>

#define CHECK(x)                                                                               \
    do {                                                                                       \
        if (!(x))                                                                              \
            return OexCall_SetError(call, (OexUtf8View)OEX_UTF8_LITERAL("OEX:ContainerTest"),  \
                                    (OexUtf8View)OEX_UTF8_LITERAL(#x));                        \
    } while (0)

static OexStatus OEX_CALL exercise(OexCall* call) {
    const uint64_t shape[] = {2, 2};
    const uint16_t exact[] = {0xD800, 0, 0x0061};
    const OexStringElementView initial[] = {{sizeof(OexStringElementView), 0, exact, 3},
                                            {sizeof(OexStringElementView), 0, NULL, 0},
                                            {sizeof(OexStringElementView), 1, NULL, 0},
                                            {sizeof(OexStringElementView), 0, exact + 2, 1}};
    OexValue *strings = NULL, *copy = NULL, *cells = NULL, *s = NULL, *table = NULL;
    OEX_TRY(OexString_CreateFromUtf16(call, 2, shape, 4, initial, &strings));
    OEX_TRY(OexValue_Retain(strings, &copy));
    OexStringElementView view = {0};
    OEX_TRY(OexString_GetElementView(strings, 0, &view));
    CHECK(view.length == 3 && view.data[0] == 0xD800 && view.data[1] == 0);
    uint64_t required = 0;
    uint32_t missing = 0;
    CHECK(OexString_CopyElementUtf8(strings, 0, NULL, 0, &required, &missing) ==
          OEX_ERROR_ENCODING);
    OEX_TRY(OexString_CopyElementUtf8(strings, 2, NULL, 0, &required, &missing));
    CHECK(required == 0 && missing == 1);
    const char utf8_text[] = "\xE4\xB8\xAD\xF0\x9F\x99\x82\0x";
    OEX_TRY(OexString_SetElementUtf8(call, copy, 0, (OexUtf8View){utf8_text, 9}));
    OEX_TRY(OexString_CopyElementUtf8(copy, 0, NULL, 0, &required, &missing));
    CHECK(required == 9 && missing == 0);
    char buffer[10];
    memset(buffer, '!', sizeof(buffer));
    CHECK(OexString_CopyElementUtf8(copy, 0, buffer, 8, &required, &missing) ==
          OEX_ERROR_RANGE);
    CHECK(buffer[0] == '!');
    OEX_TRY(OexString_CopyElementUtf8(copy, 0, buffer, 9, &required, &missing));
    CHECK(memcmp(buffer, utf8_text, 9) == 0 && buffer[9] == '!');
    const OexUtf8View invalid[] = {{"good", 4}, {"\xff", 1}};
    CHECK(OexString_SetElementsUtf8(call, copy, 0, 2, invalid, NULL) == OEX_ERROR_ENCODING);
    OEX_TRY(OexString_CopyElementUtf8(copy, 0, buffer, 9, &required, &missing));
    CHECK(memcmp(buffer, utf8_text, 9) == 0);
    OEX_TRY(OexString_GetElementView(strings, 0, &view));
    CHECK(view.data[0] == 0xD800);
    OexStringElementView batch[4];
    OEX_TRY(OexString_GetElements(strings, 0, 4, batch));
    CHECK(batch[1].isMissing == 0 && batch[2].isMissing == 1);
    OEX_TRY(OexString_SetElementsUtf16(call, copy, 0, 4, initial));
    OEX_TRY(OexString_SetElementUtf16(call, copy, 0, (OexUtf16View){exact, 3}));
    OEX_TRY(OexString_SetMissing(call, copy, 3));

    OEX_TRY(OexCell_Create(call, 2, shape, &cells));
    const OexValue* elements[] = {strings, copy};
    OEX_TRY(OexCell_SetElements(call, cells, 1, 2, elements));
    OexValue* got[2] = {NULL, NULL};
    OEX_TRY(OexCell_GetElements(call, cells, 1, 2, got));
    OEX_TRY(OexString_SetMissing(call, got[0], 0));
    OexValue* child = NULL;
    OEX_TRY(OexCell_GetElement(call, cells, 1, &child));
    OEX_TRY(OexString_GetElementView(child, 0, &view));
    CHECK(!view.isMissing);
    OEX_TRY(OexCell_SetElement(call, cells, 1, got[0]));
    OexValue_Release(child);
    OexValue_Release(got[0]);
    OexValue_Release(got[1]);
    const OexValue* bad_elements[] = {strings, NULL};
    CHECK(OexCell_SetElements(call, cells, 0, 2, bad_elements) == OEX_ERROR_ABI);
    OEX_TRY(OexCell_GetElement(call, cells, 0, &child));
    OexValueInfo info = {0};
    OEX_TRY(OexValue_GetInfo(child, &info));
    CHECK(info.kind == OEX_VALUE_DENSE && info.elementCount == 0);
    OexValue_Release(child);

    const OexUtf8View fields[] = {OEX_UTF8_LITERAL("a"), OEX_UTF8_LITERAL("b")};
    OEX_TRY(OexStruct_Create(call, 2, shape, 2, fields, &s));
    OEX_TRY(OexStruct_SetFieldValues(call, s, 0, cells));
    OEX_TRY(OexStruct_GetFieldValues(call, s, 0, &child));
    OEX_TRY(OexValue_GetInfo(child, &info));
    CHECK(info.kind == OEX_VALUE_CELL && info.elementCount == 4);
    OexValue_Release(child);
    OEX_TRY(OexStruct_SetField(call, s, 3, 1, copy));
    OEX_TRY(OexStruct_GetField(call, s, 3, 1, &child));
    OexValue_Release(child);
    OEX_TRY(OexStruct_AddField(call, s, (OexUtf8View)OEX_UTF8_LITERAL("c")));
    OEX_TRY(OexStruct_RemoveField(call, s, 2));
    const OexUtf8View renamed[] = {OEX_UTF8_LITERAL("b"), OEX_UTF8_LITERAL("a")};
    OEX_TRY(OexStruct_SetFieldNames(call, s, 2, renamed));
    uint64_t index = 0, count = 0;
    OEX_TRY(OexStruct_FindField(s, fields[0], &index));
    CHECK(index == 1);
    CHECK(OexStruct_FindField(s, (OexUtf8View)OEX_UTF8_LITERAL("absent"), &index) ==
          OEX_ERROR_NOT_FOUND);
    OEX_TRY(OexStruct_GetFieldCount(s, &count));
    CHECK(count == 2);
    OexUtf8View name = {0};
    OEX_TRY(OexStruct_GetFieldName(s, 0, &name));
    CHECK(name.length == 1 && name.data[0] == 'b');

    const OexUtf8View names[] = {OEX_UTF8_LITERAL("text data"), OEX_UTF8_LITERAL("nested")};
    const OexValue* variables[] = {strings, s};
    OEX_TRY(OexTable_Create(call, 2, 2, names, variables, &table));
    OEX_TRY(OexTable_GetRowCount(table, &count));
    CHECK(count == 2);
    OEX_TRY(OexTable_GetVariableCount(table, &count));
    CHECK(count == 2);
    OEX_TRY(OexTable_GetVariableName(table, 0, &name));
    CHECK(name.length == 9);
    OEX_TRY(OexTable_FindVariable(table, names[1], &index));
    CHECK(index == 1);
    OEX_TRY(OexTable_SetVariable(call, table, 0, copy));
    OEX_TRY(OexTable_AppendVariable(call, table, (OexUtf8View)OEX_UTF8_LITERAL("more"), cells));
    OEX_TRY(OexTable_RemoveVariable(call, table, 2));
    OEX_TRY(OexTable_SetVariableNames(call, table, 2, fields));
    OEX_TRY(OexTable_SetRowNames(call, table, 2, names));
    uint32_t present = 0;
    OEX_TRY(OexTable_GetRowNames(call, table, &present, &child));
    CHECK(present);
    OexValue_Release(child);
    OEX_TRY(OexTable_ClearRowNames(call, table));
    OEX_TRY(OexTable_GetRowNames(call, table, &present, &child));
    CHECK(!present);
    OexValue_Release(child);
    const OexUtf8View duplicate[] = {OEX_UTF8_LITERAL("x"), OEX_UTF8_LITERAL("x")};
    CHECK(OexTable_SetVariableNames(call, table, 2, duplicate) == OEX_ERROR_ARGUMENT);
    OEX_TRY(OexTable_GetVariableName(table, 0, &name));
    CHECK(name.data[0] == 'a');
    OEX_TRY(OexTable_GetVariable(call, table, 1, &child));
    uint64_t dims[2] = {0};
    uint32_t rank = 0;
    OEX_TRY(OexValue_GetDimensions(child, 0, NULL, &rank));
    CHECK(rank == 2);
    OEX_TRY(OexValue_GetDimensions(child, 2, dims, &rank));
    CHECK(dims[0] == 2 && dims[1] == 2);
    OEX_TRY(OexValue_GetClassName(call, child, &name));
    CHECK(name.length == 6 && memcmp(name.data, "struct", 6) == 0);
    OexValue_Release(child);
    OexValue* empty = NULL;
    OEX_TRY(OexTable_Create(call, 7, 0, NULL, NULL, &empty));
    CHECK(OexTable_AppendVariable(call, empty, fields[0], strings) == OEX_ERROR_DIMENSION);
    OEX_TRY(OexTable_GetRowCount(empty, &count));
    CHECK(count == 7);
    OexValue_Release(empty);
    OexValue_Release(strings);
    OexValue_Release(copy);
    OexValue_Release(cells);
    OexValue_Release(s);
    return OexCall_SetOutput(call, 0, table);
}
static const OexFunctionDefinition functions[] = {{sizeof(OexFunctionDefinition), 0,
                                                   OEX_UTF8_LITERAL("oex_container_test"), 0, 0,
                                                   1, 1, exercise}};
static const OexPluginDefinition plugin = {
    sizeof(OexPluginDefinition), OEX_ABI_VERSION, 0, 0,    OEX_UTF8_LITERAL("containers"),
    OEX_UTF8_LITERAL("1.0"),     functions,       1, NULL, 0};
OEX_PLUGIN_EXPORT const OexPluginDefinition* OEX_CALL OexPlugin_Init(void) { return &plugin; }

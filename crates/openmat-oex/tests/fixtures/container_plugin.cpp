#include <cassert>
#include <openmat/oex.hpp>

static OexStatus OEX_CALL exercise_cpp(OexCall* raw) noexcept {
    try {
        oex::Call call(raw);
        const std::array<std::uint64_t, 2> shape{2, 1};
        const std::array<std::optional<std::string_view>, 2> text{"hello", std::nullopt};
        auto strings = call.make_strings_utf8(shape, text);
        assert(strings.class_name(call) == "string");
        assert(strings.dimensions() == std::vector<std::uint64_t>({2, 1}));
        assert(strings.string_utf8(0) == "hello");
        assert(!strings.string_utf8(1));
        auto copy = strings;
        const std::array<std::uint16_t, 2> exact{0xd800, 0};
        copy.set_string_utf16(call, 0, exact);
        assert(copy.string_element(0).code_units[0] == 0xd800);
        bool encoding_error = false;
        try {
            (void)copy.string_utf8(0);
        } catch (const oex::Error& e) {
            encoding_error = e.status() == OEX_ERROR_ENCODING;
        }
        assert(encoding_error);
        assert(strings.string_utf8(0) == "hello");
        copy.set_strings_utf8(call, 0, text);
        const std::array<oex::StringElementView, 2> elements{{{exact, false}, {{}, true}}};
        auto exact_strings = call.make_strings_utf16(shape, elements);
        copy.set_strings_utf16(call, 0, elements);
        assert(exact_strings.string_elements(0, 2)[1].is_missing);
        copy.set_missing(call, 0);
        auto cells = call.make_cell(shape);
        const std::array<oex::ValueView, 2> values{strings.view(), copy.view()};
        cells.set_cell_elements(call, 0, values);
        auto child = cells.cell_element(call, 0);
        child.set_string_utf8(call, 0, "changed");
        assert(cells.cell_element(call, 0).string_utf8(0) == "hello");
        cells.set_cell_element(call, 0, child.view());
        assert(cells.cell_elements(call, 0, 2).size() == 2);
        const std::array<std::string_view, 2> fields{"a", "b"};
        auto structure = call.make_struct(shape, fields);
        structure.set_struct_field_values(call, 0, cells.view());
        assert(structure.struct_field_values(call, 0).kind() == oex::ValueKind::Cell);
        structure.set_struct_field(call, 1, 1, strings.view());
        assert(structure.struct_field(call, 1, 1).string_utf8(0) == "hello");
        structure.add_struct_field(call, "c");
        structure.remove_struct_field(call, 2);
        structure.set_struct_field_names(call, fields);
        assert(structure.struct_field_count() == 2);
        assert(structure.struct_field_name(1) == "b");
        assert(structure.find_struct_field("a") == 0);
        assert(!structure.find_struct_field("missing"));
        const std::array<oex::ValueView, 2> variables{strings.view(), structure.view()};
        auto table = call.make_table(2, fields, variables);
        assert(table.table_row_count() == 2 && table.table_variable_count() == 2);
        assert(table.table_variable_name(0) == "a");
        assert(table.find_table_variable("b") == 1);
        assert(!table.find_table_variable("missing"));
        table.set_table_variable(call, 0, copy.view());
        table.append_table_variable(call, "more", cells.view());
        table.remove_table_variable(call, 2);
        table.set_table_variable_names(call, fields);
        table.set_table_row_names(call, fields);
        assert(table.table_row_names(call)->string_utf8(0) == "a");
        table.clear_table_row_names(call);
        assert(!table.table_row_names(call));
        assert(table.table_variable(call, 1).kind() == oex::ValueKind::Struct);
        assert(call.make_string("scalar").string_utf8(0) == "scalar");
        assert(call.make_strings(shape).string_utf8(0) == "");
        call.output(0, std::move(table));
        return OEX_OK;
    } catch (const std::exception& e) {
        return OexCall_SetError(raw, OEX_UTF8_LITERAL("OEX:CppContainerTest"),
                                oex::detail::utf8(e.what()));
    } catch (...) {
        return OEX_ERROR_PLUGIN;
    }
}
static const OexFunctionDefinition functions[] = {{sizeof(OexFunctionDefinition), 0,
                                                   OEX_UTF8_LITERAL("oex_cpp_container_test"),
                                                   0, 0, 1, 1, exercise_cpp}};
static const OexPluginDefinition plugin = {sizeof(OexPluginDefinition),
                                           OEX_ABI_VERSION,
                                           0,
                                           0,
                                           OEX_UTF8_LITERAL("cpp containers"),
                                           OEX_UTF8_LITERAL("1.0"),
                                           functions,
                                           1,
                                           nullptr,
                                           0};
OEX_PLUGIN_EXPORT const OexPluginDefinition* OEX_CALL OexPlugin_Init(void) { return &plugin; }

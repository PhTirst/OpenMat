#ifndef OPENMAT_OEX_HPP
#define OPENMAT_OEX_HPP

#if defined(_MSVC_LANG)
#if _MSVC_LANG < 202002L
#error "openmat/oex.hpp requires C++20 or later"
#endif
#elif !defined(__cplusplus) || __cplusplus < 202002L
#error "openmat/oex.hpp requires C++20 or later"
#endif

#if !defined(__cpp_exceptions) && !defined(_CPPUNWIND)
#error "openmat/oex.hpp requires C++ exception support"
#endif

#include "oex.h"

#include <algorithm>
#include <array>
#include <concepts>
#include <cstddef>
#include <cstdint>
#include <initializer_list>
#include <limits>
#include <memory>
#include <optional>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <type_traits>
#include <utility>
#include <vector>

/*
 * C++20 source interface to oex.h; no C++ object crosses the C ABI.
 *
 * Operations throw Error (or a standard C++ allocation exception). The plugin
 * author must catch exceptions at EVERY C callback, including constructors,
 * methods, properties and OexPlugin_Init. Registration helpers do not install
 * an exception policy or catch user exceptions.
 *
 * Call, ValueView, Cancellation, builders and all array views are call-scoped.
 * A view also expires when its backing handle is released, transferred, taken
 * from an input slot, or its storage is invalidated. End mutable access before
 * copying/retaining the value, transferring it, or reentering the runtime.
 * Spans do not extend lifetimes or perform further COW checks. Worker threads
 * may use permitted data views/cancellation only and must join before return.
 *
 * Value and SparsePattern own handles. Copies retain handles; moves transfer
 * them. An empty C++ handle is distinct from an OpenMat Nothing/empty array.
 * Builders must finish/abort/be destroyed before their callback returns.
 * Initialize every dense/pattern element (or the committed triplet prefix).
 * Logical storage must contain only 0 and 1. All C++ indices are zero-based.
 *
 * Descriptors, NativeClass objects, and referenced names/arrays must have
 * process lifetime. Use static storage and string literals for registration.
 *
 * Example C callback (the exception mapping belongs to the plugin):
 *
 *   static OexStatus OEX_CALL scale_entry(OexCall* raw) noexcept {
 *       try {
 *           oex::Call call(raw);
 *           const double factor = call.input(1).as_double();
 *           auto value = call.take_input(0);
 *           {
 *               auto array = value.array<double>();
 *               for (auto& element : array) element *= factor;
 *           }
 *           call.output(0, std::move(value));
 *           return OEX_OK;
 *       } catch (const oex::Error& error) {
 *           return error.status();
 *       } catch (const std::bad_alloc&) {
 *           return OEX_ERROR_ALLOCATION;
 *       } catch (...) {
 *           return OEX_ERROR_PLUGIN;
 *       }
 *   }
 *
 *   static constexpr auto functions = oex::functions(
 *       oex::function("scale", scale_entry, oex::inputs(2), oex::outputs(1)));
 *   static constexpr auto plugin = oex::plugin("example", "1.0", functions);
 *   extern "C" OEX_PLUGIN_EXPORT const OexPluginDefinition* OEX_CALL
 *   OexPlugin_Init() { return &plugin; }
 */

namespace oex {

enum class ValueKind : std::uint32_t {
    Nothing = OEX_VALUE_NOTHING,
    Dense = OEX_VALUE_DENSE,
    Sparse = OEX_VALUE_SPARSE,
    String = OEX_VALUE_STRING,
    Cell = OEX_VALUE_CELL,
    Struct = OEX_VALUE_STRUCT,
    Table = OEX_VALUE_TABLE,
    Object = OEX_VALUE_OBJECT,
    Function = OEX_VALUE_FUNCTION,
    Graphics = OEX_VALUE_GRAPHICS,
};

enum class DataType : std::uint32_t {
    None = OEX_DATA_NONE,
    Logical = OEX_DATA_LOGICAL,
    Char16 = OEX_DATA_CHAR16,
    Int8 = OEX_DATA_I8,
    UInt8 = OEX_DATA_U8,
    Int16 = OEX_DATA_I16,
    UInt16 = OEX_DATA_U16,
    Int32 = OEX_DATA_I32,
    UInt32 = OEX_DATA_U32,
    Int64 = OEX_DATA_I64,
    UInt64 = OEX_DATA_U64,
    Float32 = OEX_DATA_F32,
    Float64 = OEX_DATA_F64,
    ComplexFloat32 = OEX_DATA_COMPLEX_F32,
    ComplexFloat64 = OEX_DATA_COMPLEX_F64,
    ComplexInt8 = OEX_DATA_COMPLEX_I8,
    ComplexUInt8 = OEX_DATA_COMPLEX_U8,
    ComplexInt16 = OEX_DATA_COMPLEX_I16,
    ComplexUInt16 = OEX_DATA_COMPLEX_U16,
    ComplexInt32 = OEX_DATA_COMPLEX_I32,
    ComplexUInt32 = OEX_DATA_COMPLEX_U32,
    ComplexInt64 = OEX_DATA_COMPLEX_I64,
    ComplexUInt64 = OEX_DATA_COMPLEX_U64,
};

struct ValueInfo {
    ValueKind kind;
    DataType data_type;
    std::uint32_t rank;
    std::uint32_t element_size;
    std::uint64_t element_count;
    bool complex;
};

class Error : public std::runtime_error {
  public:
    Error(OexStatus status, std::string message)
        : std::runtime_error(std::move(message)), status_(status) {}

    [[nodiscard]] OexStatus status() const noexcept { return status_; }

  private:
    OexStatus status_;
};

// Tags distinguish logical/UTF-16 storage from ordinary uint8/uint16 arrays.
struct logical {};
struct char16 {};

using ComplexF32 = OexComplexF32;
using ComplexF64 = OexComplexF64;
using ComplexI8 = OexComplexI8;
using ComplexU8 = OexComplexU8;
using ComplexI16 = OexComplexI16;
using ComplexU16 = OexComplexU16;
using ComplexI32 = OexComplexI32;
using ComplexU32 = OexComplexU32;
using ComplexI64 = OexComplexI64;
using ComplexU64 = OexComplexU64;

template <class T> struct ElementTraits;

#define OPENMAT_OEX_ELEMENT(cpp_type, storage_type, oex_type)                                      \
    template <> struct ElementTraits<cpp_type> {                                                   \
        using Storage = storage_type;                                                              \
        static constexpr DataType type = DataType::oex_type;                                       \
    }

OPENMAT_OEX_ELEMENT(logical, std::uint8_t, Logical);
OPENMAT_OEX_ELEMENT(char16, std::uint16_t, Char16);
OPENMAT_OEX_ELEMENT(std::int8_t, std::int8_t, Int8);
OPENMAT_OEX_ELEMENT(std::uint8_t, std::uint8_t, UInt8);
OPENMAT_OEX_ELEMENT(std::int16_t, std::int16_t, Int16);
OPENMAT_OEX_ELEMENT(std::uint16_t, std::uint16_t, UInt16);
OPENMAT_OEX_ELEMENT(std::int32_t, std::int32_t, Int32);
OPENMAT_OEX_ELEMENT(std::uint32_t, std::uint32_t, UInt32);
OPENMAT_OEX_ELEMENT(std::int64_t, std::int64_t, Int64);
OPENMAT_OEX_ELEMENT(std::uint64_t, std::uint64_t, UInt64);
OPENMAT_OEX_ELEMENT(float, float, Float32);
OPENMAT_OEX_ELEMENT(double, double, Float64);
OPENMAT_OEX_ELEMENT(ComplexF32, ComplexF32, ComplexFloat32);
OPENMAT_OEX_ELEMENT(ComplexF64, ComplexF64, ComplexFloat64);
OPENMAT_OEX_ELEMENT(ComplexI8, ComplexI8, ComplexInt8);
OPENMAT_OEX_ELEMENT(ComplexU8, ComplexU8, ComplexUInt8);
OPENMAT_OEX_ELEMENT(ComplexI16, ComplexI16, ComplexInt16);
OPENMAT_OEX_ELEMENT(ComplexU16, ComplexU16, ComplexUInt16);
OPENMAT_OEX_ELEMENT(ComplexI32, ComplexI32, ComplexInt32);
OPENMAT_OEX_ELEMENT(ComplexU32, ComplexU32, ComplexUInt32);
OPENMAT_OEX_ELEMENT(ComplexI64, ComplexI64, ComplexInt64);
OPENMAT_OEX_ELEMENT(ComplexU64, ComplexU64, ComplexUInt64);

#undef OPENMAT_OEX_ELEMENT

template <class T>
concept Element = !std::is_volatile_v<T> && requires {
    typename ElementTraits<std::remove_const_t<T>>::Storage;
    ElementTraits<std::remove_const_t<T>>::type;
};

template <Element T> using storage_t = typename ElementTraits<std::remove_const_t<T>>::Storage;

template <Element T>
inline constexpr DataType data_type_v = ElementTraits<std::remove_const_t<T>>::type;

template <class T>
concept MutableElement = Element<T> && !std::is_const_v<T>;

template <class T>
concept SparseElement =
    Element<T> && (data_type_v<T> == DataType::Logical || data_type_v<T> == DataType::Float64 ||
                   data_type_v<T> == DataType::ComplexFloat64);

namespace detail {

inline void check(OexStatus status, std::string_view operation) {
    if (status != OEX_OK)
        throw Error(status, std::string(operation));
}

constexpr void require(bool condition, OexStatus status, std::string_view message) {
    if (!condition)
        throw Error(status, std::string(message));
}

template <class To, class From> constexpr To narrow(From value) {
    require(std::in_range<To>(value), OEX_ERROR_RANGE, "OEX size is out of range");
    return static_cast<To>(value);
}

constexpr OexUtf8View utf8(std::string_view text) {
    return {text.data(), narrow<std::uint64_t>(text.size())};
}

template <class T> std::span<T> span(T* data, std::uint64_t count) {
    const auto size = narrow<std::size_t>(count);
    require(size <=
                static_cast<std::size_t>(std::numeric_limits<std::ptrdiff_t>::max()) / sizeof(T),
            OEX_ERROR_RANGE, "OEX buffer is too large for a C++ view");
    require(data != nullptr || size == 0, OEX_ERROR_ABI, "OEX returned a null buffer");
    if (size == 0)
        return {};
    return {data, size};
}

inline std::uint64_t element_count(std::span<const std::uint64_t> shape) {
    require(shape.size() >= 2, OEX_ERROR_DIMENSION, "An array needs at least two dimensions");
    if (std::ranges::find(shape, std::uint64_t{0}) != shape.end())
        return 0;
    std::uint64_t count = 1;
    for (const auto dimension : shape) {
        require(dimension <= std::numeric_limits<std::uint64_t>::max() / count, OEX_ERROR_RANGE,
                "Array element count overflows");
        count *= dimension;
    }
    return count;
}

template <Element T> void check_element(OexDataType type, std::uint32_t size) {
    require(type == static_cast<OexDataType>(data_type_v<T>), OEX_ERROR_TYPE,
            "Array element type does not match the requested C++ type");
    require(size == sizeof(storage_t<T>), OEX_ERROR_ABI, "OEX element size mismatch");
}

template <class T, auto Release> struct Deleter {
    void operator()(T* pointer) const noexcept { Release(pointer); }
};

template <class T, auto Release> using Owner = std::unique_ptr<T, Deleter<T, Release>>;

} // namespace detail

template <Element T> class ArrayView {
  public:
    using value_type = storage_t<T>;
    using element_type = std::conditional_t<std::is_const_v<T>, const value_type, value_type>;

    ArrayView() noexcept = default;
    ArrayView(std::span<element_type> elements, std::span<const std::uint64_t> shape)
        : elements_(elements), shape_(shape) {
        detail::require(detail::element_count(shape) == elements.size(), OEX_ERROR_DIMENSION,
                        "Array shape does not match its buffer");
    }

    template <Element U>
        requires(std::is_const_v<T> && !std::is_const_v<U> &&
                 std::same_as<std::remove_const_t<T>, U>)
    ArrayView(const ArrayView<U>& other) noexcept
        : elements_(other.elements()), shape_(other.shape()) {}

    [[nodiscard]] std::span<element_type> elements() const noexcept { return elements_; }
    [[nodiscard]] std::span<const std::uint64_t> shape() const noexcept { return shape_; }
    [[nodiscard]] std::size_t size() const noexcept { return elements_.size(); }
    [[nodiscard]] std::size_t rank() const noexcept { return shape_.size(); }
    [[nodiscard]] bool empty() const noexcept { return elements_.empty(); }
    [[nodiscard]] element_type* data() const noexcept { return elements_.data(); }
    auto begin() const noexcept { return elements_.begin(); }
    auto end() const noexcept { return elements_.end(); }

    element_type& operator[](std::size_t index) const noexcept { return elements_[index]; }
    element_type& at(std::size_t index) const {
        detail::require(index < size(), OEX_ERROR_RANGE, "Array index is out of range");
        return elements_[index];
    }
    element_type& at(std::size_t row, std::size_t column) const {
        detail::require(rank() == 2, OEX_ERROR_DIMENSION, "Expected a two-dimensional array");
        detail::require(row < shape_[0] && column < shape_[1], OEX_ERROR_RANGE,
                        "Array subscript is out of range");
        return elements_[row + column * detail::narrow<std::size_t>(shape_[0])];
    }

  private:
    std::span<element_type> elements_;
    std::span<const std::uint64_t> shape_;
};

template <SparseElement T> class SparseCscView {
  public:
    explicit SparseCscView(const OexSparseCscView& view)
        : rows_(view.rows), columns_(view.columns) {
        detail::check_element<T>(view.dataType, view.elementSize);
        detail::require(columns_ < std::numeric_limits<std::uint64_t>::max(), OEX_ERROR_RANGE,
                        "Sparse column count overflows");
        offsets_ = detail::span(view.columnOffsets, columns_ + 1);
        indices_ = detail::span(view.rowIndices, view.storedCount);
        values_ = detail::span(static_cast<const storage_t<T>*>(view.values), view.storedCount);
        detail::require(offsets_.front() == 0 && offsets_.back() == view.storedCount, OEX_ERROR_ABI,
                        "Invalid CSC endpoints");
    }

    [[nodiscard]] std::uint64_t rows() const noexcept { return rows_; }
    [[nodiscard]] std::uint64_t columns() const noexcept { return columns_; }
    [[nodiscard]] std::size_t stored_count() const noexcept { return values_.size(); }
    [[nodiscard]] std::span<const std::uint64_t> column_offsets() const noexcept {
        return offsets_;
    }
    [[nodiscard]] std::span<const std::uint64_t> row_indices() const noexcept { return indices_; }
    [[nodiscard]] std::span<const storage_t<T>> values() const noexcept { return values_; }

  private:
    std::uint64_t rows_;
    std::uint64_t columns_;
    std::span<const std::uint64_t> offsets_;
    std::span<const std::uint64_t> indices_;
    std::span<const storage_t<T>> values_;
};

class Value;
class Call;

class Call;
struct StringElementView {
    std::span<const std::uint16_t> code_units;
    bool is_missing = false;
};
class ValueView {
  public:
    ValueView() noexcept = default;
    // Borrow only: this never retains or releases the C handle.
    explicit ValueView(const OexValue* borrowed) : handle_(borrowed) { require_handle(); }

    [[nodiscard]] const OexValue* native_handle() const noexcept { return handle_; }
    explicit operator bool() const noexcept { return handle_ != nullptr; }

    [[nodiscard]] ValueInfo info() const {
        require_handle();
        OexValueInfo raw{};
        raw.structSize = sizeof(raw);
        detail::check(OexValue_GetInfo(handle_, &raw), "Cannot query value information");
        return {static_cast<ValueKind>(raw.kind),
                static_cast<DataType>(raw.dataType),
                raw.rank,
                raw.elementSize,
                raw.elementCount,
                (raw.flags & OEX_VALUE_FLAG_COMPLEX) != 0};
    }
    [[nodiscard]] ValueKind kind() const { return info().kind; }
    [[nodiscard]] DataType data_type() const { return info().data_type; }

    template <Element T> [[nodiscard]] bool is_dense() const {
        const auto value = info();
        return value.kind == ValueKind::Dense && value.data_type == data_type_v<T>;
    }
    template <Element T> [[nodiscard]] bool is_scalar() const {
        const auto value = info();
        return value.kind == ValueKind::Dense && value.data_type == data_type_v<T> &&
               value.element_count == 1;
    }
    template <Element T> [[nodiscard]] bool is_sparse() const {
        const auto value = info();
        return value.kind == ValueKind::Sparse && value.data_type == data_type_v<T>;
    }

    [[nodiscard]] double as_double() const {
        require_handle();
        double result = 0;
        detail::check(OexValue_GetFloat64(handle_, &result),
                      "Cannot read value as a double scalar");
        return result;
    }

    template <Element T> [[nodiscard]] ArrayView<const std::remove_const_t<T>> array() const {
        require_handle();
        OexDenseView raw{};
        raw.structSize = sizeof(raw);
        detail::check(OexDense_GetView(handle_, &raw), "Cannot read a dense array");
        detail::check_element<T>(raw.dataType, raw.elementSize);
        return {detail::span(static_cast<const storage_t<T>*>(raw.data), raw.elementCount),
                detail::span(raw.dimensions, raw.rank)};
    }

    template <SparseElement T> [[nodiscard]] SparseCscView<T> sparse() const {
        require_handle();
        OexSparseCscView raw{};
        raw.structSize = sizeof(raw);
        detail::check(OexSparse_GetCscView(handle_, &raw), "Cannot read a sparse array");
        return SparseCscView<T>(raw);
    }

    [[nodiscard]] Value cell_element(Call& call, std::uint64_t index) const;
    [[nodiscard]] Value struct_field(Call& call, std::uint64_t element,
                                     std::uint64_t field) const;
    [[nodiscard]] Value struct_field_values(Call& call, std::uint64_t field) const;
    [[nodiscard]] Value table_variable(Call& call, std::uint64_t index) const;
    [[nodiscard]] std::uint64_t struct_field_count() const {
        std::uint64_t out = 0;
        detail::check(OexStruct_GetFieldCount(handle_, &out), "OexStruct_GetFieldCount");
        return out;
    }
    [[nodiscard]] std::uint64_t table_row_count() const {
        std::uint64_t out = 0;
        detail::check(OexTable_GetRowCount(handle_, &out), "OexTable_GetRowCount");
        return out;
    }
    [[nodiscard]] std::uint64_t table_variable_count() const {
        std::uint64_t out = 0;
        detail::check(OexTable_GetVariableCount(handle_, &out), "OexTable_GetVariableCount");
        return out;
    }
    [[nodiscard]] std::string struct_field_name(std::uint64_t index) const {
        OexUtf8View out{};
        detail::check(OexStruct_GetFieldName(handle_, index, &out), "OexStruct_GetFieldName");
        return std::string(out.data, detail::narrow<std::size_t>(out.length));
    }
    [[nodiscard]] std::string table_variable_name(std::uint64_t index) const {
        OexUtf8View out{};
        detail::check(OexTable_GetVariableName(handle_, index, &out),
                      "OexTable_GetVariableName");
        return std::string(out.data, detail::narrow<std::size_t>(out.length));
    }
    [[nodiscard]] std::optional<std::uint64_t> find_struct_field(std::string_view name) const {
        std::uint64_t index = 0;
        const auto status = OexStruct_FindField(handle_, detail::utf8(name), &index);
        if (status == OEX_ERROR_NOT_FOUND)
            return std::nullopt;
        detail::check(status, "OexStruct_FindField");
        return index;
    }
    [[nodiscard]] std::optional<std::uint64_t>
    find_table_variable(std::string_view name) const {
        std::uint64_t index = 0;
        const auto status = OexTable_FindVariable(handle_, detail::utf8(name), &index);
        if (status == OEX_ERROR_NOT_FOUND)
            return std::nullopt;
        detail::check(status, "OexTable_FindVariable");
        return index;
    }
    [[nodiscard]] std::vector<std::uint64_t> dimensions() const {
        std::uint32_t rank = 0;
        detail::check(OexValue_GetDimensions(handle_, 0, nullptr, &rank),
                      "Cannot query dimensions");
        std::vector<std::uint64_t> result(rank);
        detail::check(OexValue_GetDimensions(handle_, rank, result.data(), &rank),
                      "Cannot read dimensions");
        return result;
    }
    [[nodiscard]] std::string class_name(Call& call) const;
    [[nodiscard]] StringElementView string_element(std::uint64_t index) const {
        OexStringElementView raw{};
        raw.structSize = sizeof(raw);
        detail::check(OexString_GetElementView(handle_, index, &raw), "Cannot read string");
        return {detail::span(raw.data, raw.length), raw.isMissing != 0};
    }
    [[nodiscard]] std::vector<StringElementView> string_elements(std::uint64_t start,
                                                                 std::uint64_t count) const {
        std::vector<OexStringElementView> raw(detail::narrow<std::size_t>(count));
        detail::check(OexString_GetElements(handle_, start, count, raw.data()),
                      "Cannot read strings");
        std::vector<StringElementView> result;
        result.reserve(raw.size());
        for (auto v : raw)
            result.push_back({detail::span(v.data, v.length), v.isMissing != 0});
        return result;
    }
    [[nodiscard]] std::optional<std::string> string_utf8(std::uint64_t index) const {
        std::uint64_t required = 0;
        std::uint32_t missing = 0;
        detail::check(
            OexString_CopyElementUtf8(handle_, index, nullptr, 0, &required, &missing),
            "Cannot convert string to UTF-8");
        if (missing)
            return std::nullopt;
        std::string result(detail::narrow<std::size_t>(required), '\0');
        detail::check(OexString_CopyElementUtf8(handle_, index, result.data(), required,
                                                &required, &missing),
                      "Cannot copy UTF-8 string");
        return result;
    }
    [[nodiscard]] std::vector<Value> cell_elements(Call& call, std::uint64_t start,
                                                   std::uint64_t count) const;
    [[nodiscard]] std::optional<Value> table_row_names(Call& call) const;

    [[nodiscard]] Value retain() const;

  private:
    const OexValue* handle_ = nullptr;
    void require_handle() const {
        detail::require(handle_ != nullptr, OEX_ERROR_STATE, "Value view is empty");
    }
};

class Value {
  public:
    Value() noexcept = default;
    ~Value() noexcept {
        if (handle_)
            OexValue_Release(handle_);
    }
    Value(const Value& other) {
        if (other.handle_)
            detail::check(OexValue_Retain(other.handle_, &handle_), "Cannot retain value");
    }
    Value(Value&& other) noexcept : handle_(std::exchange(other.handle_, nullptr)) {}
    Value& operator=(Value other) noexcept {
        swap(other);
        return *this;
    }
    void swap(Value& other) noexcept { std::swap(handle_, other.handle_); }
    friend void swap(Value& left, Value& right) noexcept { left.swap(right); }
    explicit operator bool() const noexcept { return handle_ != nullptr; }

    // Explicit C interop: adopt accepts OWNED handles only; release transfers one.
    [[nodiscard]] static Value adopt(OexValue* owned) noexcept { return Value(owned); }
    [[nodiscard]] OexValue* release() noexcept { return std::exchange(handle_, nullptr); }
    [[nodiscard]] const OexValue* native_handle() const noexcept { return handle_; }

    [[nodiscard]] ValueView view() const& { return ValueView(handle_); }
    ValueView view() const&& = delete;
    [[nodiscard]] ValueInfo info() const { return ValueView(handle_).info(); }
    [[nodiscard]] ValueKind kind() const { return info().kind; }
    [[nodiscard]] DataType data_type() const { return info().data_type; }
    [[nodiscard]] double as_double() const { return ValueView(handle_).as_double(); }
    template <Element T> [[nodiscard]] bool is_dense() const {
        return ValueView(handle_).is_dense<T>();
    }
    template <Element T> [[nodiscard]] bool is_scalar() const {
        return ValueView(handle_).is_scalar<T>();
    }
    template <Element T> [[nodiscard]] bool is_sparse() const {
        return ValueView(handle_).is_sparse<T>();
    }

    template <Element T> [[nodiscard]] ArrayView<T> array() & {
        if constexpr (std::is_const_v<T>) {
            return view().array<T>();
        } else {
            // Reject a type mismatch before asking the host to detach COW storage.
            detail::require(is_dense<T>(), OEX_ERROR_TYPE, "Expected a matching dense array");
            OexMutableDenseView raw{};
            raw.structSize = sizeof(raw);
            detail::check(OexDense_GetMutableView(handle_, &raw), "Cannot access a writable array");
            detail::check_element<T>(raw.dataType, raw.elementSize);
            return {detail::span(static_cast<storage_t<T>*>(raw.data), raw.elementCount),
                    detail::span(raw.dimensions, raw.rank)};
        }
    }
    template <Element T> [[nodiscard]] ArrayView<const std::remove_const_t<T>> array() const& {
        return view().array<T>();
    }
    template <Element T> ArrayView<T> array() && = delete;
    template <Element T> ArrayView<const std::remove_const_t<T>> array() const&& = delete;

    template <SparseElement T> [[nodiscard]] SparseCscView<T> sparse() const& {
        return view().sparse<T>();
    }
    template <SparseElement T> SparseCscView<T> sparse() const&& = delete;

    [[nodiscard]] Value cell_element(Call& call, std::uint64_t index) const {
        return view().cell_element(call, index);
    }
    [[nodiscard]] Value struct_field(Call& call, std::uint64_t element,
                                     std::uint64_t field) const {
        return view().struct_field(call, element, field);
    }
    [[nodiscard]] Value struct_field_values(Call& call, std::uint64_t field) const {
        return view().struct_field_values(call, field);
    }
    [[nodiscard]] Value table_variable(Call& call, std::uint64_t index) const {
        return view().table_variable(call, index);
    }
    [[nodiscard]] std::uint64_t struct_field_count() const {
        return view().struct_field_count();
    }
    [[nodiscard]] std::uint64_t table_row_count() const { return view().table_row_count(); }
    [[nodiscard]] std::uint64_t table_variable_count() const {
        return view().table_variable_count();
    }
    [[nodiscard]] std::string struct_field_name(std::uint64_t index) const {
        return view().struct_field_name(index);
    }
    [[nodiscard]] std::string table_variable_name(std::uint64_t index) const {
        return view().table_variable_name(index);
    }
    [[nodiscard]] std::optional<std::uint64_t> find_struct_field(std::string_view name) const {
        return view().find_struct_field(name);
    }
    [[nodiscard]] std::optional<std::uint64_t>
    find_table_variable(std::string_view name) const {
        return view().find_table_variable(name);
    }
    void set_missing(Call& call, std::uint64_t index);
    void set_cell_element(Call& call, std::uint64_t index, ValueView source);
    void set_struct_field(Call& call, std::uint64_t element, std::uint64_t field,
                          ValueView source);
    void set_struct_field_values(Call& call, std::uint64_t field, ValueView source);
    void remove_struct_field(Call& call, std::uint64_t field);
    void set_table_variable(Call& call, std::uint64_t index, ValueView source);
    void remove_table_variable(Call& call, std::uint64_t index);
    void clear_table_row_names(Call& call);
    void set_struct_field_names(Call& call, std::span<const std::string_view> names);
    void set_table_variable_names(Call& call, std::span<const std::string_view> names);
    void set_table_row_names(Call& call, std::span<const std::string_view> names);
    void add_struct_field(Call& call, std::string_view name);
    void append_table_variable(Call& call, std::string_view name, ValueView source);
    [[nodiscard]] std::vector<std::uint64_t> dimensions() const { return view().dimensions(); }
    [[nodiscard]] std::string class_name(Call& call) const { return view().class_name(call); }
    [[nodiscard]] StringElementView string_element(std::uint64_t index) const& {
        return view().string_element(index);
    }
    StringElementView string_element(std::uint64_t) const&& = delete;
    [[nodiscard]] std::vector<StringElementView> string_elements(std::uint64_t start,
                                                                 std::uint64_t count) const& {
        return view().string_elements(start, count);
    }
    std::vector<StringElementView> string_elements(std::uint64_t,
                                                   std::uint64_t) const&& = delete;
    [[nodiscard]] std::optional<std::string> string_utf8(std::uint64_t index) const {
        return view().string_utf8(index);
    }
    [[nodiscard]] std::vector<Value> cell_elements(Call& call, std::uint64_t start,
                                                   std::uint64_t count) const {
        return view().cell_elements(call, start, count);
    }
    [[nodiscard]] std::optional<Value> table_row_names(Call& call) const {
        return view().table_row_names(call);
    }
    void set_string_utf8(Call& call, std::uint64_t index, std::string_view text);
    void set_string_utf16(Call& call, std::uint64_t index, std::span<const std::uint16_t> text);
    void set_strings_utf8(Call& call, std::uint64_t start,
                          std::span<const std::optional<std::string_view>> text);
    void set_strings_utf16(Call& call, std::uint64_t start,
                           std::span<const StringElementView> text);
    void set_cell_elements(Call& call, std::uint64_t start,
                           std::span<const ValueView> elements);

  private:
    OexValue* handle_ = nullptr;
    explicit Value(OexValue* owned) noexcept : handle_(owned) {}
    friend class Call;
};

inline Value ValueView::retain() const {
    require_handle();
    OexValue* retained = nullptr;
    detail::check(OexValue_Retain(handle_, &retained), "Cannot retain value");
    return Value::adopt(retained);
}

template <MutableElement T> class ArrayBuilder {
  public:
    ArrayBuilder() noexcept = default;
    ArrayBuilder(ArrayBuilder&&) noexcept = default;
    ArrayBuilder& operator=(ArrayBuilder&&) noexcept = default;
    ArrayBuilder(const ArrayBuilder&) = delete;
    ArrayBuilder& operator=(const ArrayBuilder&) = delete;

    [[nodiscard]] ArrayView<T> view() & {
        require_active();
        return view_;
    }
    ArrayView<T> view() && = delete;
    [[nodiscard]] Value finish() {
        require_active();
        auto* builder = handle_.release();
        view_ = {};
        OexValue* value = nullptr;
        detail::check(OexDenseBuilder_Commit(builder, &value), "Cannot finish array");
        return Value::adopt(value);
    }
    void abort() noexcept {
        handle_.reset();
        view_ = {};
    }

  private:
    detail::Owner<OexDenseBuilder, OexDenseBuilder_Abort> handle_;
    ArrayView<T> view_;
    void require_active() const {
        detail::require(bool(handle_), OEX_ERROR_STATE, "Array builder is empty");
    }
    ArrayBuilder(OexDenseBuilder* builder, const OexMutableDenseView& raw) : handle_(builder) {
        detail::check_element<T>(raw.dataType, raw.elementSize);
        view_ = ArrayView<T>(detail::span(static_cast<storage_t<T>*>(raw.data), raw.elementCount),
                             detail::span(raw.dimensions, raw.rank));
    }
    friend class Call;
};

class SparsePattern {
  public:
    SparsePattern() noexcept = default;
    SparsePattern(const SparsePattern& other)
        : rows_(other.rows_), columns_(other.columns_), stored_(other.stored_) {
        if (other.handle_) {
            OexSparsePattern* retained = nullptr;
            detail::check(OexSparsePattern_Retain(other.handle_.get(), &retained),
                          "Cannot retain sparse pattern");
            handle_.reset(retained);
        }
    }
    SparsePattern(SparsePattern&& other) noexcept { swap(other); }
    SparsePattern& operator=(SparsePattern other) noexcept {
        swap(other);
        return *this;
    }
    void swap(SparsePattern& other) noexcept {
        handle_.swap(other.handle_);
        std::swap(rows_, other.rows_);
        std::swap(columns_, other.columns_);
        std::swap(stored_, other.stored_);
    }
    friend void swap(SparsePattern& left, SparsePattern& right) noexcept { left.swap(right); }
    explicit operator bool() const noexcept { return bool(handle_); }
    [[nodiscard]] std::uint64_t rows() const noexcept { return rows_; }
    [[nodiscard]] std::uint64_t columns() const noexcept { return columns_; }
    [[nodiscard]] std::uint64_t stored_count() const noexcept { return stored_; }
    [[nodiscard]] const OexSparsePattern* native_handle() const noexcept { return handle_.get(); }

  private:
    detail::Owner<OexSparsePattern, OexSparsePattern_Release> handle_;
    std::uint64_t rows_ = 0, columns_ = 0, stored_ = 0;
    SparsePattern(OexSparsePattern* pattern, std::uint64_t rows, std::uint64_t columns,
                  std::uint64_t stored) noexcept
        : handle_(pattern), rows_(rows), columns_(columns), stored_(stored) {}
    friend class Call;
};

template <MutableElement T>
    requires SparseElement<T>
class SparseTripletBuilder {
  public:
    SparseTripletBuilder() noexcept = default;
    SparseTripletBuilder(SparseTripletBuilder&&) noexcept = default;
    SparseTripletBuilder& operator=(SparseTripletBuilder&&) noexcept = default;
    SparseTripletBuilder(const SparseTripletBuilder&) = delete;
    SparseTripletBuilder& operator=(const SparseTripletBuilder&) = delete;

    [[nodiscard]] std::span<std::uint64_t> row_indices() & {
        require_active();
        return rows_;
    }
    [[nodiscard]] std::span<std::uint64_t> column_indices() & {
        require_active();
        return columns_;
    }
    [[nodiscard]] std::span<storage_t<T>> values() & {
        require_active();
        return values_;
    }
    std::span<std::uint64_t> row_indices() && = delete;
    std::span<std::uint64_t> column_indices() && = delete;
    std::span<storage_t<T>> values() && = delete;
    [[nodiscard]] std::size_t capacity() const {
        require_active();
        return values_.size();
    }

    [[nodiscard]] Value finish(std::uint64_t stored_count) {
        require_active();
        // The C implementation checks capacity BEFORE consuming the builder.
        // Perform that check here; subsequent failures consume the C handle.
        detail::require(stored_count <= values_.size(), OEX_ERROR_RANGE,
                        "Triplet count exceeds the builder capacity");
        (void)detail::narrow<std::size_t>(stored_count);
        auto* builder = handle_.release();
        clear_views();
        OexValue* value = nullptr;
        detail::check(OexSparseTripletBuilder_Commit(builder, stored_count, &value),
                      "Cannot finish sparse triplets");
        return Value::adopt(value);
    }
    void abort() noexcept {
        handle_.reset();
        clear_views();
    }

  private:
    detail::Owner<OexSparseTripletBuilder, OexSparseTripletBuilder_Abort> handle_;
    std::span<std::uint64_t> rows_, columns_;
    std::span<storage_t<T>> values_;
    void clear_views() noexcept {
        rows_ = {};
        columns_ = {};
        values_ = {};
    }
    void require_active() const {
        detail::require(bool(handle_), OEX_ERROR_STATE, "Sparse triplet builder is empty");
    }
    SparseTripletBuilder(OexSparseTripletBuilder* builder, const OexSparseTripletView& raw)
        : handle_(builder) {
        detail::check_element<T>(raw.dataType, raw.elementSize);
        rows_ = detail::span(raw.rowIndices, raw.capacity);
        columns_ = detail::span(raw.columnIndices, raw.capacity);
        values_ = detail::span(static_cast<storage_t<T>*>(raw.values), raw.capacity);
    }
    friend class Call;
};

template <MutableElement T>
    requires SparseElement<T>
class SparsePatternBuilder {
  public:
    SparsePatternBuilder() noexcept = default;
    SparsePatternBuilder(SparsePatternBuilder&&) noexcept = default;
    SparsePatternBuilder& operator=(SparsePatternBuilder&&) noexcept = default;
    SparsePatternBuilder(const SparsePatternBuilder&) = delete;
    SparsePatternBuilder& operator=(const SparsePatternBuilder&) = delete;

    [[nodiscard]] std::span<storage_t<T>> values() & {
        require_active();
        return values_;
    }
    std::span<storage_t<T>> values() && = delete;
    [[nodiscard]] Value finish() {
        require_active();
        auto* builder = handle_.release();
        values_ = {};
        OexValue* value = nullptr;
        detail::check(OexSparsePatternBuilder_Commit(builder, &value),
                      "Cannot finish sparse pattern values");
        return Value::adopt(value);
    }
    void abort() noexcept {
        handle_.reset();
        values_ = {};
    }

  private:
    detail::Owner<OexSparsePatternBuilder, OexSparsePatternBuilder_Abort> handle_;
    std::span<storage_t<T>> values_;
    void require_active() const {
        detail::require(bool(handle_), OEX_ERROR_STATE, "Sparse pattern builder is empty");
    }
    SparsePatternBuilder(OexSparsePatternBuilder* builder, const OexSparsePatternValuesView& raw)
        : handle_(builder) {
        detail::check_element<T>(raw.dataType, raw.elementSize);
        values_ = detail::span(static_cast<storage_t<T>*>(raw.values), raw.storedCount);
    }
    friend class Call;
};

class Cancellation {
  public:
    explicit Cancellation(const OexCancellation* handle) : handle_(handle) {
        detail::require(handle != nullptr, OEX_ERROR_STATE, "Cancellation handle is empty");
    }
    [[nodiscard]] bool requested() const noexcept {
        return OexCancellation_IsRequested(handle_) != 0;
    }
    void check() const {
        detail::require(!requested(), OEX_ERROR_CANCELLED, "OEX call was cancelled");
    }

  private:
    const OexCancellation* handle_;
};

class Call {
  public:
    explicit Call(OexCall* call) : handle_(call) {
        detail::require(call != nullptr, OEX_ERROR_STATE, "Call handle is empty");
    }
    Call(const Call&) = delete;
    Call& operator=(const Call&) = delete;
    [[nodiscard]] OexCall* native_handle() const noexcept { return handle_; }
    [[nodiscard]] std::uint32_t input_count() const noexcept {
        return OexCall_GetInputCount(handle_);
    }
    [[nodiscard]] std::uint32_t output_count() const noexcept {
        return OexCall_GetOutputCount(handle_);
    }

    [[nodiscard]] ValueView input(std::uint32_t index) const {
        detail::require(index < input_count(), OEX_ERROR_RANGE, "Input index is out of range");
        return ValueView(OexCall_BorrowInput(handle_, index));
    }
    [[nodiscard]] Value take_input(std::uint32_t index) {
        OexValue* value = nullptr;
        detail::check(OexCall_TakeInput(handle_, index, &value), "Cannot take input");
        return Value::adopt(value);
    }
    [[nodiscard]] Value scalar(double scalar) {
        OexValue* value = nullptr;
        detail::check(OexValue_CreateFloat64(handle_, scalar, &value),
                      "Cannot create double scalar");
        return Value::adopt(value);
    }
    void output(std::uint32_t index, Value value) {
        detail::require(bool(value), OEX_ERROR_STATE, "Cannot output an empty value handle");
        detail::check(OexCall_SetOutput(handle_, index, value.handle_), "Cannot set output");
        (void)value.release();
    }
    [[nodiscard]] Value make_strings(std::span<const std::uint64_t> shape) {
        OexValue* out = nullptr;
        detail::check(OexString_Create(handle_, detail::narrow<std::uint32_t>(shape.size()),
                                       shape.data(), &out),
                      "Cannot create strings");
        return Value::adopt(out);
    }
    [[nodiscard]] Value
    make_strings_utf8(std::span<const std::uint64_t> shape,
                      std::span<const std::optional<std::string_view>> text) {
        std::vector<OexUtf8View> raw;
        std::vector<std::uint8_t> missing;
        raw.reserve(text.size());
        missing.reserve(text.size());
        for (auto v : text) {
            raw.push_back(detail::utf8(v.value_or("")));
            missing.push_back(!v.has_value());
        }
        OexValue* out = nullptr;
        detail::check(OexString_CreateFromUtf8(
                          handle_, detail::narrow<std::uint32_t>(shape.size()), shape.data(),
                          raw.size(), raw.data(), missing.data(), &out),
                      "Cannot create UTF-8 strings");
        return Value::adopt(out);
    }
    [[nodiscard]] Value make_strings_utf16(std::span<const std::uint64_t> shape,
                                           std::span<const StringElementView> text) {
        std::vector<OexStringElementView> raw;
        raw.reserve(text.size());
        for (auto v : text)
            raw.push_back({sizeof(OexStringElementView), v.is_missing, v.code_units.data(),
                           v.code_units.size()});
        OexValue* out = nullptr;
        detail::check(OexString_CreateFromUtf16(handle_,
                                                detail::narrow<std::uint32_t>(shape.size()),
                                                shape.data(), raw.size(), raw.data(), &out),
                      "Cannot create UTF-16 strings");
        return Value::adopt(out);
    }
    [[nodiscard]] Value make_string(std::string_view text) {
        const std::array<std::uint64_t, 2> shape{1, 1};
        const std::array<std::optional<std::string_view>, 1> elements{text};
        return make_strings_utf8(shape, elements);
    }
    [[nodiscard]] Value make_cell(std::span<const std::uint64_t> shape) {
        OexValue* out = nullptr;
        detail::check(OexCell_Create(handle_, detail::narrow<std::uint32_t>(shape.size()),
                                     shape.data(), &out),
                      "Cannot create cell");
        return Value::adopt(out);
    }
    [[nodiscard]] Value make_struct(std::span<const std::uint64_t> shape,
                                    std::span<const std::string_view> names) {
        std::vector<OexUtf8View> raw;
        raw.reserve(names.size());
        for (auto n : names)
            raw.push_back(detail::utf8(n));
        OexValue* out = nullptr;
        detail::check(OexStruct_Create(handle_, detail::narrow<std::uint32_t>(shape.size()),
                                       shape.data(), raw.size(), raw.data(), &out),
                      "Cannot create struct");
        return Value::adopt(out);
    }
    [[nodiscard]] Value make_table(std::uint64_t rows, std::span<const std::string_view> names,
                                   std::span<const ValueView> variables) {
        detail::require(names.size() == variables.size(), OEX_ERROR_DIMENSION,
                        "Table name/value count mismatch");
        std::vector<OexUtf8View> raw_names;
        std::vector<const OexValue*> raw_values;
        raw_names.reserve(names.size());
        raw_values.reserve(variables.size());
        for (auto n : names) {
            raw_names.push_back(detail::utf8(n));
        }
        for (auto v : variables) {
            raw_values.push_back(v.native_handle());
        }
        OexValue* out = nullptr;
        detail::check(OexTable_Create(handle_, rows, names.size(), raw_names.data(),
                                      raw_values.data(), &out),
                      "Cannot create table");
        return Value::adopt(out);
    }

    [[nodiscard]] Cancellation cancellation() const {
        return Cancellation(OexCall_GetCancellation(handle_));
    }
    void check_cancelled() const { cancellation().check(); }

    [[noreturn]] void fail(std::string_view identifier, std::string_view message) {
        const auto status =
            OexCall_SetError(handle_, detail::utf8(identifier), detail::utf8(message));
        throw Error(status == OEX_OK ? OEX_ERROR_PLUGIN : status, std::string(message));
    }

    template <MutableElement T>
    [[nodiscard]] ArrayBuilder<T> make_uninitialized_array(std::span<const std::uint64_t> shape) {
        (void)detail::element_count(shape);
        OexDenseBuilder* builder = nullptr;
        OexMutableDenseView raw{};
        raw.structSize = sizeof(raw);
        detail::check(
            OexDenseBuilder_CreateUninitialized(handle_, static_cast<OexDataType>(data_type_v<T>),
                                                detail::narrow<std::uint32_t>(shape.size()),
                                                shape.data(), &builder, &raw),
            "Cannot create array builder");
        return ArrayBuilder<T>(builder, raw);
    }
    template <MutableElement T>
    [[nodiscard]] ArrayBuilder<T>
    make_uninitialized_array(std::initializer_list<std::uint64_t> shape) {
        return make_uninitialized_array<T>(
            std::span<const std::uint64_t>(shape.begin(), shape.size()));
    }
    template <MutableElement T>
    [[nodiscard]] Value make_array(std::span<const std::uint64_t> shape,
                                   storage_t<T> initial = {}) {
        auto builder = make_uninitialized_array<T>(shape);
        std::ranges::fill(builder.view().elements(), initial);
        return builder.finish();
    }
    template <MutableElement T>
    [[nodiscard]] Value make_array(std::initializer_list<std::uint64_t> shape,
                                   storage_t<T> initial = {}) {
        return make_array<T>(std::span<const std::uint64_t>(shape.begin(), shape.size()), initial);
    }

    template <MutableElement T>
        requires SparseElement<T>
    [[nodiscard]] SparseTripletBuilder<T>
    make_sparse_triplets(std::uint64_t rows, std::uint64_t columns, std::uint64_t capacity) {
        OexSparseTripletBuilder* builder = nullptr;
        OexSparseTripletView raw{};
        raw.structSize = sizeof(raw);
        detail::check(OexSparseTripletBuilder_CreateUninitialized(
                          handle_, static_cast<OexDataType>(data_type_v<T>), rows, columns,
                          capacity, &builder, &raw),
                      "Cannot create sparse triplet builder");
        return SparseTripletBuilder<T>(builder, raw);
    }
    [[nodiscard]] SparsePattern make_sparse_pattern(std::uint64_t rows, std::uint64_t columns,
                                                    std::span<const std::uint64_t> column_offsets,
                                                    std::span<const std::uint64_t> row_indices) {
        detail::require(columns < std::numeric_limits<std::uint64_t>::max() &&
                            columns + 1 == column_offsets.size(),
                        OEX_ERROR_DIMENSION, "Sparse column offsets must have columns + 1 entries");
        OexSparsePattern* pattern = nullptr;
        const auto stored = detail::narrow<std::uint64_t>(row_indices.size());
        detail::check(OexSparsePattern_Create(handle_, rows, columns, stored, column_offsets.data(),
                                              row_indices.data(), &pattern),
                      "Cannot create sparse pattern");
        return SparsePattern(pattern, rows, columns, stored);
    }
    template <MutableElement T>
        requires SparseElement<T>
    [[nodiscard]] SparsePatternBuilder<T> make_sparse_values(const SparsePattern& pattern) {
        detail::require(bool(pattern), OEX_ERROR_STATE, "Sparse pattern is empty");
        OexSparsePatternBuilder* builder = nullptr;
        OexSparsePatternValuesView raw{};
        raw.structSize = sizeof(raw);
        detail::check(OexSparsePatternBuilder_CreateUninitialized(
                          handle_, pattern.native_handle(),
                          static_cast<OexDataType>(data_type_v<T>), &builder, &raw),
                      "Cannot create sparse pattern builder");
        return SparsePatternBuilder<T>(builder, raw);
    }

    template <std::size_t Outputs>
    [[nodiscard]] std::array<Value, Outputs> invoke(ValueView callable,
                                                    std::span<const ValueView> arguments) {
        static_assert(Outputs <= std::numeric_limits<std::uint32_t>::max());
        auto inputs = invocation_inputs(callable, arguments);
        std::array<OexValue*, Outputs> raw{};
        std::array<Value, Outputs> result;
        const auto status = OexCall_Invoke(
            handle_, callable.native_handle(), detail::narrow<std::uint32_t>(inputs.size()),
            inputs.data(), static_cast<std::uint32_t>(Outputs), raw.data());
        // Adopt without allocation BEFORE any throwing operation after Invoke.
        for (std::size_t i = 0; i < Outputs; ++i)
            result[i] = Value::adopt(raw[i]);
        check_invocation(status, result);
        return result;
    }
    template <std::size_t Outputs>
    [[nodiscard]] std::array<Value, Outputs> invoke(ValueView callable,
                                                    std::initializer_list<ValueView> arguments) {
        return invoke<Outputs>(callable,
                               std::span<const ValueView>(arguments.begin(), arguments.size()));
    }
    [[nodiscard]] std::vector<Value>
    invoke(ValueView callable, std::span<const ValueView> arguments, std::size_t outputs) {
        const auto count = detail::narrow<std::uint32_t>(outputs);
        auto inputs = invocation_inputs(callable, arguments);
        std::vector<OexValue*> raw(outputs, nullptr);
        std::vector<Value> result(outputs);
        const auto status = OexCall_Invoke(handle_, callable.native_handle(),
                                           detail::narrow<std::uint32_t>(inputs.size()),
                                           inputs.data(), count, raw.data());
        for (std::size_t i = 0; i < outputs; ++i)
            result[i] = Value::adopt(raw[i]);
        check_invocation(status, result);
        return result;
    }
    [[nodiscard]] std::vector<Value>
    invoke(ValueView callable, std::initializer_list<ValueView> arguments, std::size_t outputs) {
        return invoke(callable, std::span<const ValueView>(arguments.begin(), arguments.size()),
                      outputs);
    }

  private:
    OexCall* handle_;
    static std::vector<const OexValue*> invocation_inputs(ValueView callable,
                                                          std::span<const ValueView> arguments) {
        detail::require(bool(callable), OEX_ERROR_STATE, "Callable value is empty");
        (void)detail::narrow<std::uint32_t>(arguments.size());
        std::vector<const OexValue*> inputs;
        inputs.reserve(arguments.size());
        for (const auto argument : arguments) {
            detail::require(bool(argument), OEX_ERROR_STATE, "Callback argument is empty");
            inputs.push_back(argument.native_handle());
        }
        return inputs;
    }
    template <class Values> static void check_invocation(OexStatus status, const Values& values) {
        // The original language error is already held by the host. Do not SetError.
        detail::check(status, "Language callback failed");
        for (const auto& value : values)
            detail::require(bool(value), OEX_ERROR_ABI,
                            "Language callback returned a null value handle");
    }
};
inline Value ValueView::cell_element(Call& call, std::uint64_t index) const {
    OexValue* out = nullptr;
    detail::check(OexCell_GetElement(call.native_handle(), handle_, index, &out),
                  "OexCell_GetElement");
    return Value::adopt(out);
}
inline Value ValueView::struct_field(Call& call, std::uint64_t element,
                                     std::uint64_t field) const {
    OexValue* out = nullptr;
    detail::check(OexStruct_GetField(call.native_handle(), handle_, element, field, &out),
                  "OexStruct_GetField");
    return Value::adopt(out);
}
inline Value ValueView::struct_field_values(Call& call, std::uint64_t field) const {
    OexValue* out = nullptr;
    detail::check(OexStruct_GetFieldValues(call.native_handle(), handle_, field, &out),
                  "OexStruct_GetFieldValues");
    return Value::adopt(out);
}
inline Value ValueView::table_variable(Call& call, std::uint64_t index) const {
    OexValue* out = nullptr;
    detail::check(OexTable_GetVariable(call.native_handle(), handle_, index, &out),
                  "OexTable_GetVariable");
    return Value::adopt(out);
}
inline void Value::set_missing(Call& call, std::uint64_t index) {
    detail::check(OexString_SetMissing(call.native_handle(), handle_, index),
                  "OexString_SetMissing");
}
inline void Value::set_cell_element(Call& call, std::uint64_t index, ValueView source) {
    detail::check(
        OexCell_SetElement(call.native_handle(), handle_, index, source.native_handle()),
        "OexCell_SetElement");
}
inline void Value::set_struct_field(Call& call, std::uint64_t element, std::uint64_t field,
                                    ValueView source) {
    detail::check(OexStruct_SetField(call.native_handle(), handle_, element, field,
                                     source.native_handle()),
                  "OexStruct_SetField");
}
inline void Value::set_struct_field_values(Call& call, std::uint64_t field, ValueView source) {
    detail::check(
        OexStruct_SetFieldValues(call.native_handle(), handle_, field, source.native_handle()),
        "OexStruct_SetFieldValues");
}
inline void Value::remove_struct_field(Call& call, std::uint64_t field) {
    detail::check(OexStruct_RemoveField(call.native_handle(), handle_, field),
                  "OexStruct_RemoveField");
}
inline void Value::set_table_variable(Call& call, std::uint64_t index, ValueView source) {
    detail::check(
        OexTable_SetVariable(call.native_handle(), handle_, index, source.native_handle()),
        "OexTable_SetVariable");
}
inline void Value::remove_table_variable(Call& call, std::uint64_t index) {
    detail::check(OexTable_RemoveVariable(call.native_handle(), handle_, index),
                  "OexTable_RemoveVariable");
}
inline void Value::clear_table_row_names(Call& call) {
    detail::check(OexTable_ClearRowNames(call.native_handle(), handle_),
                  "OexTable_ClearRowNames");
}
inline void Value::set_struct_field_names(Call& call, std::span<const std::string_view> names) {
    std::vector<OexUtf8View> raw;
    raw.reserve(names.size());
    for (auto n : names)
        raw.push_back(detail::utf8(n));
    detail::check(
        OexStruct_SetFieldNames(call.native_handle(), handle_, raw.size(), raw.data()),
        "OexStruct_SetFieldNames");
}
inline void Value::set_table_variable_names(Call& call,
                                            std::span<const std::string_view> names) {
    std::vector<OexUtf8View> raw;
    raw.reserve(names.size());
    for (auto n : names)
        raw.push_back(detail::utf8(n));
    detail::check(
        OexTable_SetVariableNames(call.native_handle(), handle_, raw.size(), raw.data()),
        "OexTable_SetVariableNames");
}
inline void Value::set_table_row_names(Call& call, std::span<const std::string_view> names) {
    std::vector<OexUtf8View> raw;
    raw.reserve(names.size());
    for (auto n : names)
        raw.push_back(detail::utf8(n));
    detail::check(OexTable_SetRowNames(call.native_handle(), handle_, raw.size(), raw.data()),
                  "OexTable_SetRowNames");
}
inline void Value::add_struct_field(Call& call, std::string_view name) {
    detail::check(OexStruct_AddField(call.native_handle(), handle_, detail::utf8(name)),
                  "OexStruct_AddField");
}
inline void Value::append_table_variable(Call& call, std::string_view name, ValueView source) {
    detail::check(OexTable_AppendVariable(call.native_handle(), handle_, detail::utf8(name),
                                          source.native_handle()),
                  "OexTable_AppendVariable");
}
inline std::string ValueView::class_name(Call& call) const {
    OexUtf8View raw{};
    detail::check(OexValue_GetClassName(call.native_handle(), handle_, &raw),
                  "Cannot query class name");
    return std::string(raw.data, detail::narrow<std::size_t>(raw.length));
}
inline std::vector<Value> ValueView::cell_elements(Call& call, std::uint64_t start,
                                                   std::uint64_t count) const {
    std::vector<OexValue*> raw(detail::narrow<std::size_t>(count));
    // Allocate the owner vector before C transfers handles; no allocation can leak them.
    std::vector<Value> result(raw.size());
    const auto status =
        OexCell_GetElements(call.native_handle(), handle_, start, count, raw.data());
    for (std::size_t i = 0; i < raw.size(); ++i)
        result[i] = Value::adopt(raw[i]);
    detail::check(status, "Cannot read cell elements");
    return result;
}
inline std::optional<Value> ValueView::table_row_names(Call& call) const {
    OexValue* raw = nullptr;
    std::uint32_t present = 0;
    const auto status = OexTable_GetRowNames(call.native_handle(), handle_, &present, &raw);
    auto owned = Value::adopt(raw);
    detail::check(status, "Cannot read row names");
    if (!present) {
        return std::nullopt;
    }
    return owned;
}
inline void Value::set_string_utf8(Call& call, std::uint64_t index, std::string_view text) {
    detail::check(
        OexString_SetElementUtf8(call.native_handle(), handle_, index, detail::utf8(text)),
        "Cannot set UTF-8 string");
}
inline void Value::set_string_utf16(Call& call, std::uint64_t index,
                                    std::span<const std::uint16_t> text) {
    detail::check(OexString_SetElementUtf16(call.native_handle(), handle_, index,
                                            {text.data(), text.size()}),
                  "Cannot set UTF-16 string");
}
inline void Value::set_strings_utf8(Call& call, std::uint64_t start,
                                    std::span<const std::optional<std::string_view>> text) {
    std::vector<OexUtf8View> raw;
    std::vector<std::uint8_t> missing;
    raw.reserve(text.size());
    missing.reserve(text.size());
    for (auto v : text) {
        raw.push_back(detail::utf8(v.value_or("")));
        missing.push_back(!v.has_value());
    }
    detail::check(OexString_SetElementsUtf8(call.native_handle(), handle_, start, raw.size(),
                                            raw.data(), missing.data()),
                  "Cannot set UTF-8 strings");
}
inline void Value::set_strings_utf16(Call& call, std::uint64_t start,
                                     std::span<const StringElementView> text) {
    std::vector<OexStringElementView> raw;
    raw.reserve(text.size());
    for (auto v : text)
        raw.push_back({sizeof(OexStringElementView), v.is_missing, v.code_units.data(),
                       v.code_units.size()});
    detail::check(OexString_SetElementsUtf16(call.native_handle(), handle_, start, raw.size(),
                                             raw.data()),
                  "Cannot set UTF-16 strings");
}
inline void Value::set_cell_elements(Call& call, std::uint64_t start,
                                     std::span<const ValueView> elements) {
    std::vector<const OexValue*> raw;
    raw.reserve(elements.size());
    for (auto v : elements)
        raw.push_back(v.native_handle());
    detail::check(
        OexCell_SetElements(call.native_handle(), handle_, start, raw.size(), raw.data()),
        "Cannot set cell elements");
}

struct Inputs {
    std::uint32_t minimum, maximum;
};
struct Outputs {
    std::uint32_t minimum, maximum;
};

constexpr Inputs inputs(std::uint32_t minimum, std::uint32_t maximum) {
    detail::require(minimum <= maximum, OEX_ERROR_ARGUMENT, "Invalid input count range");
    return {minimum, maximum};
}
constexpr Inputs inputs(std::uint32_t count) { return inputs(count, count); }
constexpr Outputs outputs(std::uint32_t minimum, std::uint32_t maximum) {
    detail::require(minimum <= maximum, OEX_ERROR_ARGUMENT, "Invalid output count range");
    return {minimum, maximum};
}
constexpr Outputs outputs(std::uint32_t count) { return outputs(count, count); }

// These accept C callbacks supplied by the author, not throwing C++ callbacks.
constexpr OexFunctionDefinition function(std::string_view name, OexFunctionCallback invoke,
                                         Inputs in, Outputs out) {
    detail::require(invoke != nullptr, OEX_ERROR_ARGUMENT, "Function callback is null");
    (void)inputs(in.minimum, in.maximum);
    (void)outputs(out.minimum, out.maximum);
    return {sizeof(OexFunctionDefinition),
            0,
            detail::utf8(name),
            in.minimum,
            in.maximum,
            out.minimum,
            out.maximum,
            invoke};
}
constexpr OexNativeMethodDefinition method(std::string_view name, OexNativeMethod invoke, Inputs in,
                                           Outputs out) {
    detail::require(invoke != nullptr, OEX_ERROR_ARGUMENT, "Method callback is null");
    (void)inputs(in.minimum, in.maximum);
    (void)outputs(out.minimum, out.maximum);
    return {sizeof(OexNativeMethodDefinition),
            0,
            detail::utf8(name),
            in.minimum,
            in.maximum,
            out.minimum,
            out.maximum,
            invoke};
}
constexpr OexNativePropertyDefinition property(std::string_view name,
                                               OexNativePropertyGetter getter,
                                               OexNativePropertySetter setter = nullptr) {
    detail::require(getter != nullptr || setter != nullptr, OEX_ERROR_ARGUMENT,
                    "Property has no callback");
    return {sizeof(OexNativePropertyDefinition), 0, detail::utf8(name), getter, setter};
}

template <class... D>
    requires(std::same_as<D, OexFunctionDefinition> && ...)
constexpr auto functions(D... definitions) {
    return std::array<OexFunctionDefinition, sizeof...(D)>{definitions...};
}
template <class... D>
    requires(std::same_as<D, OexNativeMethodDefinition> && ...)
constexpr auto methods(D... definitions) {
    return std::array<OexNativeMethodDefinition, sizeof...(D)>{definitions...};
}
template <class... D>
    requires(std::same_as<D, OexNativePropertyDefinition> && ...)
constexpr auto properties(D... definitions) {
    return std::array<OexNativePropertyDefinition, sizeof...(D)>{definitions...};
}
template <class... D>
    requires(std::same_as<D, OexNativeClassDefinition> && ...)
constexpr auto classes(D... definitions) {
    return std::array<OexNativeClassDefinition, sizeof...(D)>{definitions...};
}

template <class T> class NativeClass {
    static_assert(std::is_nothrow_destructible_v<T>, "A native class destructor must not throw");

  public:
    // Constructor callbacks must return an instance allocated with new T.
    constexpr NativeClass(std::string_view name, OexNativeConstructor constructor,
                          std::span<const OexNativeMethodDefinition> methods = {},
                          std::span<const OexNativePropertyDefinition> properties = {})
        : definition_{sizeof(OexNativeClassDefinition),
                      OEX_CLASS_HANDLE,
                      detail::utf8(name),
                      constructor,
                      &destroy,
                      methods.data(),
                      detail::narrow<std::uint64_t>(methods.size()),
                      &token_,
                      properties.data(),
                      detail::narrow<std::uint64_t>(properties.size())} {
        detail::require(constructor != nullptr, OEX_ERROR_ARGUMENT, "Native constructor is null");
    }
    NativeClass(const NativeClass&) = delete;
    NativeClass& operator=(const NativeClass&) = delete;
    NativeClass(NativeClass&&) = delete;
    NativeClass& operator=(NativeClass&&) = delete;

    [[nodiscard]] constexpr const OexNativeClassDefinition& definition() const& noexcept {
        return definition_;
    }
    const OexNativeClassDefinition& definition() const&& = delete;
    [[nodiscard]] constexpr const void* type_token() const noexcept { return &token_; }

    [[nodiscard]] T& borrow(Call& call, ValueView object) const {
        detail::require(bool(object), OEX_ERROR_STATE, "Native object value is empty");
        void* instance = nullptr;
        detail::check(OexNativeObject_BorrowInstance(call.native_handle(), object.native_handle(),
                                                     type_token(), &instance),
                      "Cannot borrow native object of the registered type");
        detail::require(instance != nullptr, OEX_ERROR_ABI, "Native instance is null");
        return *static_cast<T*>(instance);
    }
    [[nodiscard]] Value create(Call& call, std::unique_ptr<T> instance) const {
        detail::require(bool(instance), OEX_ERROR_ARGUMENT, "Native instance is null");
        OexValue* value = nullptr;
        detail::check(
            OexNativeObject_Create(call.native_handle(), type_token(), instance.get(), &value),
            "Cannot create native object");
        (void)instance.release(); // Host owns the instance immediately on success.
        return Value::adopt(value);
    }

  private:
    char token_ = 0;
    OexNativeClassDefinition definition_;
    static void OEX_CALL destroy(void* instance) noexcept { delete static_cast<T*>(instance); }
};

namespace detail {
inline constexpr std::array<OexFunctionDefinition, 0> no_functions{};
inline constexpr std::array<OexNativeClassDefinition, 0> no_classes{};
} // namespace detail

template <std::size_t F, std::size_t C>
constexpr OexPluginDefinition plugin(std::string_view name, std::string_view version,
                                     const std::array<OexFunctionDefinition, F>& functions,
                                     const std::array<OexNativeClassDefinition, C>& classes) {
    return {sizeof(OexPluginDefinition),
            OEX_ABI_VERSION,
            0,
            0,
            detail::utf8(name),
            detail::utf8(version),
            functions.data(),
            detail::narrow<std::uint64_t>(F),
            classes.data(),
            detail::narrow<std::uint64_t>(C)};
}
template <std::size_t F>
constexpr OexPluginDefinition plugin(std::string_view name, std::string_view version,
                                     const std::array<OexFunctionDefinition, F>& functions) {
    return plugin(name, version, functions, detail::no_classes);
}
constexpr OexPluginDefinition plugin(std::string_view name, std::string_view version) {
    return plugin(name, version, detail::no_functions, detail::no_classes);
}

// Catch the easy dangling-descriptor mistake: passing a temporary array.
template <std::size_t F>
OexPluginDefinition plugin(std::string_view, std::string_view,
                           const std::array<OexFunctionDefinition, F>&&) = delete;
template <std::size_t F, std::size_t C>
OexPluginDefinition plugin(std::string_view, std::string_view,
                           const std::array<OexFunctionDefinition, F>&&,
                           const std::array<OexNativeClassDefinition, C>&) = delete;
template <std::size_t F, std::size_t C>
OexPluginDefinition plugin(std::string_view, std::string_view,
                           const std::array<OexFunctionDefinition, F>&,
                           const std::array<OexNativeClassDefinition, C>&&) = delete;

} // namespace oex

#endif // OPENMAT_OEX_HPP

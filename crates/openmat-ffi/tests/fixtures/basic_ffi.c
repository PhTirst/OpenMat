#include <stdint.h>
#include <stddef.h>

#if defined(_WIN32)
#define OPENMAT_FFI_EXPORT __declspec(dllexport)
#else
#define OPENMAT_FFI_EXPORT __attribute__((visibility("default")))
#endif

typedef struct openmat_ffi_point {
    int32_t x;
    int32_t y;
} openmat_ffi_point;

typedef int32_t (*openmat_ffi_binary)(int32_t, int32_t);

#pragma pack(push, 1)
typedef struct openmat_ffi_packed {
    uint8_t tag;
    uint32_t value;
} openmat_ffi_packed;
#pragma pack(pop)

typedef union openmat_ffi_number {
    uint32_t bits;
    float real;
} openmat_ffi_number;

typedef struct openmat_ffi_words {
    uint16_t values[3];
} openmat_ffi_words;

OPENMAT_FFI_EXPORT int32_t ffi_add_i32(int32_t left, int32_t right) {
    return left + right;
}

OPENMAT_FFI_EXPORT openmat_ffi_point ffi_translate_point(
    openmat_ffi_point point,
    int32_t delta_x,
    int32_t delta_y
) {
    point.x += delta_x;
    point.y += delta_y;
    return point;
}

OPENMAT_FFI_EXPORT void ffi_translate_point_in_place(
    openmat_ffi_point *point,
    int32_t delta_x,
    int32_t delta_y
) {
    point->x += delta_x;
    point->y += delta_y;
}

OPENMAT_FFI_EXPORT openmat_ffi_binary ffi_get_add_i32(void) {
    return ffi_add_i32;
}

OPENMAT_FFI_EXPORT int32_t ffi_call_binary(
    openmat_ffi_binary function,
    int32_t left,
    int32_t right
) {
    return function(left, right);
}

OPENMAT_FFI_EXPORT uint32_t ffi_sizeof_packed(void) {
    return (uint32_t)sizeof(openmat_ffi_packed);
}

OPENMAT_FFI_EXPORT uint32_t ffi_offsetof_packed_value(void) {
    return (uint32_t)offsetof(openmat_ffi_packed, value);
}

OPENMAT_FFI_EXPORT void ffi_write_packed(
    openmat_ffi_packed *value,
    uint8_t tag,
    uint32_t payload
) {
    value->tag = tag;
    value->value = payload;
}

OPENMAT_FFI_EXPORT uint32_t ffi_read_packed(openmat_ffi_packed value) {
    return (uint32_t)value.tag + value.value;
}

OPENMAT_FFI_EXPORT void ffi_write_union_bits(
    openmat_ffi_number *value,
    uint32_t bits
) {
    value->bits = bits;
}

OPENMAT_FFI_EXPORT uint32_t ffi_read_union_bits(openmat_ffi_number value) {
    return value.bits;
}

OPENMAT_FFI_EXPORT uint32_t ffi_sum_words(openmat_ffi_words value) {
    return (uint32_t)value.values[0]
        + (uint32_t)value.values[1]
        + (uint32_t)value.values[2];
}

#ifndef OPENMAT_SIM_KERNEL_H
#define OPENMAT_SIM_KERNEL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif
/* ABI v1: host-owned double buffers, element counts (not byte sizes).
 * The host guarantees valid storage for the supplied lengths. No allocations,
 * callbacks, Rust types or exceptions cross this boundary. All results are
 * computed before output stores. Code must remain loaded during every call.
 * 0 = success, 1 = ABI mismatch, 2 = length mismatch, 3 = null buffer.
 */
#define OPENMAT_SIM_KERNEL_ABI_VERSION UINT32_C(1)
typedef uint32_t (*OpenMatSimKernelV1)(uint32_t abi_version,
                                     const double *inputs, uint64_t input_count,
                                     double *outputs, uint64_t output_count);

#ifdef __cplusplus
}
#endif

#endif

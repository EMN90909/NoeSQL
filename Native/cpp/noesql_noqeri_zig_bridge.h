#ifndef NOESQL_NOQERI_ZIG_BRIDGE_H
#define NOESQL_NOQERI_ZIG_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Linux x86-64 qualification shim.
 *
 * Noqeri's current AOT backend uses its own internal native calling
 * convention. For six integer/scalar arguments on Linux x86-64 that contract
 * maps to rdi/rsi/rdx/rcx/r8/r9 with rax return, so this shim is the only
 * component allowed to call the experimental Noqeri symbol directly.
 *
 * Rust calls this stable C surface. The shim owns C-string handling and Zig
 * pager interaction; Noqeri sees only six scalar integers.
 */
int32_t noesql_cpp_noqeri_bool_row(
    const char *path,
    uint32_t page_id,
    int64_t row_id,
    uint8_t enabled,
    int32_t operation,
    uint8_t *out_enabled
);

uint32_t noesql_cpp_noqeri_bridge_version(void);

#ifdef __cplusplus
}
#endif

#endif

#ifndef NOESQL_ZIG_BRIDGE_H
#define NOESQL_ZIG_BRIDGE_H

#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
#define NOESQL_ZIG_ABI_VERSION 1u
#define NOESQL_ZIG_MAX_SOURCE_BYTES (1024u * 1024u)
#define NOESQL_ZIG_MAX_OUTPUT_BYTES (16u * 1024u * 1024u)
typedef int32_t (*noesql_zig_execute_fn)(
    const uint8_t *source, size_t source_length,
    uint8_t *output, size_t output_capacity, size_t *output_length);
typedef struct noesql_zig_engine_v1 {
    uint32_t abi_version;
    noesql_zig_execute_fn execute;
} noesql_zig_engine_v1;
int32_t noesql_zig_validate_engine(const noesql_zig_engine_v1 *engine);
int32_t noesql_zig_execute(
    const noesql_zig_engine_v1 *engine, const uint8_t *source,
    size_t source_length, uint8_t *output,
    size_t output_capacity, size_t *output_length);
#ifdef __cplusplus
}
#endif
#endif

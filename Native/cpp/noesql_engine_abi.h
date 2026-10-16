#ifndef NOESQL_ENGINE_ABI_H
#define NOESQL_ENGINE_ABI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define NOESQL_ENGINE_ABI_VERSION 1u
#define NOESQL_MAX_SOURCE_BYTES (1024u * 1024u)
#define NOESQL_MAX_OUTPUT_BYTES (16u * 1024u * 1024u)

enum noesql_bridge_status {
    NOESQL_BRIDGE_OK = 0,
    NOESQL_BRIDGE_INVALID_ARGUMENT = -1,
    NOESQL_BRIDGE_UNSUPPORTED_ABI = -2,
    NOESQL_BRIDGE_LIMIT_EXCEEDED = -3
};

/*
 * A Noqeri-compiled engine exports this table through a C-compatible ABI.
 * The callback owns query semantics; this C++ bridge only validates buffers
 * and forwards the request. The output is caller-owned and must not outlive
 * the call. Return NOESQL_BRIDGE_OK on success, or a stable negative error.
 */
typedef int32_t (*noesql_engine_execute_fn)(
    const uint8_t *source,
    size_t source_length,
    uint8_t *output,
    size_t output_capacity,
    size_t *output_length
);

typedef struct noesql_engine_v1 {
    uint32_t abi_version;
    noesql_engine_execute_fn execute;
} noesql_engine_v1;

/* Validate the engine table without executing a query. */
int32_t noesql_cpp_validate_engine(const noesql_engine_v1 *engine);

/* Validate input/output bounds, then forward to the Noqeri-owned engine. */
int32_t noesql_cpp_execute(
    const noesql_engine_v1 *engine,
    const uint8_t *source,
    size_t source_length,
    uint8_t *output,
    size_t output_capacity,
    size_t *output_length
);

#ifdef __cplusplus
}
#endif

#endif

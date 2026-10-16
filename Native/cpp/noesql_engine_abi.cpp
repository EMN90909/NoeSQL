#include "noesql_engine_abi.h"

extern "C" int32_t noesql_cpp_validate_engine(const noesql_engine_v1 *engine) {
    if (engine == nullptr || engine->execute == nullptr) {
        return NOESQL_BRIDGE_INVALID_ARGUMENT;
    }
    if (engine->abi_version != NOESQL_ENGINE_ABI_VERSION) {
        return NOESQL_BRIDGE_UNSUPPORTED_ABI;
    }
    return NOESQL_BRIDGE_OK;
}

extern "C" int32_t noesql_cpp_execute(
    const noesql_engine_v1 *engine,
    const uint8_t *source,
    size_t source_length,
    uint8_t *output,
    size_t output_capacity,
    size_t *output_length
) {
    if (output_length == nullptr) {
        return NOESQL_BRIDGE_INVALID_ARGUMENT;
    }
    *output_length = 0;

    const int32_t engine_status = noesql_cpp_validate_engine(engine);
    if (engine_status != NOESQL_BRIDGE_OK) {
        return engine_status;
    }
    if ((source == nullptr && source_length != 0) ||
        (output == nullptr && output_capacity != 0)) {
        return NOESQL_BRIDGE_INVALID_ARGUMENT;
    }
    if (source_length > NOESQL_MAX_SOURCE_BYTES ||
        output_capacity > NOESQL_MAX_OUTPUT_BYTES) {
        return NOESQL_BRIDGE_LIMIT_EXCEEDED;
    }

    const int32_t status = engine->execute(
        source, source_length, output, output_capacity, output_length
    );
    if (status != NOESQL_BRIDGE_OK) {
        *output_length = 0;
        return status;
    }
    if (*output_length > output_capacity) {
        *output_length = 0;
        return NOESQL_BRIDGE_LIMIT_EXCEEDED;
    }
    return NOESQL_BRIDGE_OK;
}

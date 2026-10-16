#include "noesql_engine_abi.h"

#include <cassert>
#include <cstdint>
#include <cstring>

static int32_t echo(
    const uint8_t *source,
    size_t source_length,
    uint8_t *output,
    size_t output_capacity,
    size_t *output_length
) {
    if (source_length > output_capacity) return NOESQL_BRIDGE_LIMIT_EXCEEDED;
    if (source_length != 0) std::memcpy(output, source, source_length);
    *output_length = source_length;
    return NOESQL_BRIDGE_OK;
}

static int32_t bad_length(
    const uint8_t *, size_t, uint8_t *, size_t, size_t *output_length
) {
    *output_length = NOESQL_MAX_OUTPUT_BYTES + 1u;
    return NOESQL_BRIDGE_OK;
}

int main() {
    const uint8_t source[] = {'d', 'b', ' ', 's', 'c', 'h', 'o', 'o', 'l'};
    uint8_t output[32] = {};
    size_t output_length = 99;

    noesql_engine_v1 engine{NOESQL_ENGINE_ABI_VERSION, echo};
    assert(noesql_cpp_validate_engine(&engine) == NOESQL_BRIDGE_OK);
    assert(noesql_cpp_execute(&engine, source, sizeof(source), output,
                              sizeof(output), &output_length) == NOESQL_BRIDGE_OK);
    assert(output_length == sizeof(source));
    assert(std::memcmp(source, output, sizeof(source)) == 0);

    noesql_engine_v1 wrong_version{NOESQL_ENGINE_ABI_VERSION + 1u, echo};
    assert(noesql_cpp_validate_engine(&wrong_version) == NOESQL_BRIDGE_UNSUPPORTED_ABI);
    assert(noesql_cpp_execute(&wrong_version, source, sizeof(source), output,
                              sizeof(output), &output_length) == NOESQL_BRIDGE_UNSUPPORTED_ABI);
    assert(output_length == 0);

    assert(noesql_cpp_execute(&engine, nullptr, 1, output, sizeof(output),
                              &output_length) == NOESQL_BRIDGE_INVALID_ARGUMENT);
    assert(output_length == 0);
    assert(noesql_cpp_execute(&engine, source, NOESQL_MAX_SOURCE_BYTES + 1u,
                              output, sizeof(output), &output_length) == NOESQL_BRIDGE_LIMIT_EXCEEDED);
    assert(output_length == 0);

    noesql_engine_v1 malicious_length{NOESQL_ENGINE_ABI_VERSION, bad_length};
    assert(noesql_cpp_execute(&malicious_length, source, sizeof(source), output,
                              sizeof(output), &output_length) == NOESQL_BRIDGE_LIMIT_EXCEEDED);
    assert(output_length == 0);
    assert(noesql_cpp_validate_engine(nullptr) == NOESQL_BRIDGE_INVALID_ARGUMENT);
    return 0;
}

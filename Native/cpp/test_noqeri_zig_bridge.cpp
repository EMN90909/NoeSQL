#include "noesql_noqeri_zig_bridge.h"
#include "../zig/noesql_zig_pages.h"

#include <array>
#include <cassert>
#include <cstdint>
#include <cstring>

namespace {
std::array<std::uint8_t, NOESQL_ZIG_PAGE_BYTES> g_page{};
bool g_present = false;
}

extern "C" int32_t noesql_zig_page_open(uint32_t version, const char *path, void **output) {
    if (version != NOESQL_ZIG_PAGE_ABI_VERSION || !path || !*path || !output) return NOESQL_ZIG_PAGE_INVALID;
    *output = reinterpret_cast<void *>(0x1);
    return NOESQL_ZIG_PAGE_OK;
}
extern "C" void noesql_zig_page_close(void *) {}
extern "C" int32_t noesql_zig_page_stage(void *, uint32_t, const uint8_t *data, size_t length) {
    if (!data || length != g_page.size()) return NOESQL_ZIG_PAGE_INVALID;
    std::memcpy(g_page.data(), data, g_page.size());
    g_present = true;
    return NOESQL_ZIG_PAGE_OK;
}
extern "C" int32_t noesql_zig_page_commit(void *) { return NOESQL_ZIG_PAGE_OK; }
extern "C" int32_t noesql_zig_page_rollback(void *) { return NOESQL_ZIG_PAGE_OK; }
extern "C" int32_t noesql_zig_page_get(void *, uint32_t, uint8_t *output, size_t capacity) {
    if (!g_present) return NOESQL_ZIG_PAGE_NOT_FOUND;
    if (!output || capacity < g_page.size()) return NOESQL_ZIG_PAGE_BUFFER_TOO_SMALL;
    std::memcpy(output, g_page.data(), g_page.size());
    return NOESQL_ZIG_PAGE_OK;
}

// Stand-in for the six-scalar Noqeri AOT symbol. The production native-chain
// target links the real assembly produced from noesql_stage2_bridge.noqeri.
extern "C" std::int64_t noesqlStage2BoolRow(
    std::int64_t operation,
    std::int64_t present,
    std::int64_t current_id,
    std::int64_t current_enabled,
    std::int64_t requested_id,
    std::int64_t requested_enabled
) {
    if (operation == 1) {
        if (!present || current_id != requested_id) return 110;
        return current_enabled ? 101 : 100;
    }
    if (operation == 2) {
        if (present) return 210;
        return requested_enabled ? 201 : 200;
    }
    if (operation == 3) {
        if (!present || current_id != requested_id) return 310;
        return requested_enabled ? 301 : 300;
    }
    if (operation == 4) {
        if (!present || current_id != requested_id) return 410;
        return 400;
    }
    return -1;
}

int main() {
    assert(noesql_cpp_noqeri_bridge_version() == 1u);

    uint8_t enabled = 0;
    assert(noesql_cpp_noqeri_bool_row("bridge.noedb", 3, 44, 1, 2, &enabled) == 0);
    assert(noesql_cpp_noqeri_bool_row("bridge.noedb", 3, 44, 0, 1, &enabled) == 0);
    assert(enabled == 1);

    assert(noesql_cpp_noqeri_bool_row("bridge.noedb", 3, 44, 0, 3, &enabled) == 0);
    assert(noesql_cpp_noqeri_bool_row("bridge.noedb", 3, 44, 0, 1, &enabled) == 0);
    assert(enabled == 0);

    assert(noesql_cpp_noqeri_bool_row("bridge.noedb", 3, 44, 0, 4, &enabled) == 0);
    return 0;
}

#include "noesql_noqeri_zig_bridge.h"
#include "../zig/noesql_zig_pages.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>

#if !defined(__linux__) || !(defined(__x86_64__) || defined(_M_X64))
#error "NoeSQL Noqeri/Zig qualification shim currently supports Linux x86-64 only."
#endif

namespace {

constexpr std::array<std::uint8_t, 8> kMagic{{'N','Q','N','R','0','0','0','1'}};
constexpr std::size_t kPageBytes = NOESQL_ZIG_PAGE_BYTES;

// Qualified backend-specific call boundary. Keep this declaration scalar-only.
extern "C" std::int64_t noesqlStage2BoolRow(
    std::int64_t operation,
    std::int64_t present,
    std::int64_t current_id,
    std::int64_t current_enabled,
    std::int64_t requested_id,
    std::int64_t requested_enabled
);

struct Row {
    std::int64_t id;
    bool enabled;
};

bool page_is_zero(const std::array<std::uint8_t, kPageBytes>& page) {
    for (const auto byte : page) {
        if (byte != 0) return false;
    }
    return true;
}

int32_t decode_row(const std::array<std::uint8_t, kPageBytes>& page, Row *out, bool *present) {
    if (!out || !present) return NOESQL_ZIG_PAGE_INVALID;
    if (page_is_zero(page)) {
        *present = false;
        *out = Row{0, false};
        return NOESQL_ZIG_PAGE_OK;
    }
    if (std::memcmp(page.data(), kMagic.data(), kMagic.size()) != 0) {
        return NOESQL_ZIG_PAGE_FAILURE;
    }
    std::int64_t id = 0;
    std::memcpy(&id, page.data() + 8, sizeof(id));
    if (page[16] > 1) return NOESQL_ZIG_PAGE_FAILURE;
    *present = true;
    *out = Row{id, page[16] != 0};
    return NOESQL_ZIG_PAGE_OK;
}

std::array<std::uint8_t, kPageBytes> encode_row(const Row *row) {
    std::array<std::uint8_t, kPageBytes> page{};
    if (!row) return page;
    std::memcpy(page.data(), kMagic.data(), kMagic.size());
    std::memcpy(page.data() + 8, &row->id, sizeof(row->id));
    page[16] = row->enabled ? 1u : 0u;
    return page;
}

int32_t stage_and_commit(void *handle, std::uint32_t page_id, const Row *row) {
    const auto page = encode_row(row);
    const int32_t staged = noesql_zig_page_stage(handle, page_id, page.data(), page.size());
    if (staged != NOESQL_ZIG_PAGE_OK) {
        (void)noesql_zig_page_rollback(handle);
        return staged;
    }
    const int32_t committed = noesql_zig_page_commit(handle);
    if (committed != NOESQL_ZIG_PAGE_OK) {
        (void)noesql_zig_page_rollback(handle);
        return committed;
    }
    return NOESQL_ZIG_PAGE_OK;
}

}  // namespace

extern "C" uint32_t noesql_cpp_noqeri_bridge_version(void) {
    return 1u;
}

extern "C" int32_t noesql_cpp_noqeri_bool_row(
    const char *path,
    uint32_t page_id,
    int64_t row_id,
    uint8_t enabled,
    int32_t operation,
    uint8_t *out_enabled
) {
    if (!path || !*path || !out_enabled) return NOESQL_ZIG_PAGE_INVALID;
    if (page_id == 0 || page_id >= 256) return NOESQL_ZIG_PAGE_INVALID;
    if (operation < 1 || operation > 4) return NOESQL_ZIG_PAGE_INVALID;
    if (enabled > 1) return NOESQL_ZIG_PAGE_INVALID;

    *out_enabled = 0;

    void *handle = nullptr;
    const int32_t opened = noesql_zig_page_open(NOESQL_ZIG_PAGE_ABI_VERSION, path, &handle);
    if (opened != NOESQL_ZIG_PAGE_OK || !handle) return opened == NOESQL_ZIG_PAGE_OK ? NOESQL_ZIG_PAGE_FAILURE : opened;

    std::array<std::uint8_t, kPageBytes> page{};
    const int32_t got = noesql_zig_page_get(handle, page_id, page.data(), page.size());

    Row current{0, false};
    bool present = false;
    if (got == NOESQL_ZIG_PAGE_OK) {
        const int32_t decoded = decode_row(page, &current, &present);
        if (decoded != NOESQL_ZIG_PAGE_OK) {
            noesql_zig_page_close(handle);
            return decoded;
        }
    } else if (got != NOESQL_ZIG_PAGE_NOT_FOUND) {
        noesql_zig_page_close(handle);
        return got;
    }

    const std::int64_t result = noesqlStage2BoolRow(
        operation,
        present ? 1 : 0,
        current.id,
        current.enabled ? 1 : 0,
        row_id,
        enabled ? 1 : 0
    );

    int32_t status = NOESQL_ZIG_PAGE_OK;
    switch (operation) {
        case 1:
            if (result == 100 || result == 101) {
                *out_enabled = result == 101 ? 1u : 0u;
            } else if (result == 110) {
                status = NOESQL_ZIG_PAGE_NOT_FOUND;
            } else {
                status = static_cast<int32_t>(result);
            }
            break;
        case 2:
            if (result == 200 || result == 201) {
                const Row next{row_id, result == 201};
                status = stage_and_commit(handle, page_id, &next);
            } else {
                status = static_cast<int32_t>(result);
            }
            break;
        case 3:
            if (result == 300 || result == 301) {
                const Row next{row_id, result == 301};
                status = stage_and_commit(handle, page_id, &next);
            } else {
                status = static_cast<int32_t>(result);
            }
            break;
        case 4:
            if (result == 400) {
                status = stage_and_commit(handle, page_id, nullptr);
            } else {
                status = static_cast<int32_t>(result);
            }
            break;
        default:
            status = NOESQL_ZIG_PAGE_INVALID;
            break;
    }

    noesql_zig_page_close(handle);
    return status;
}

#include "../zig/noesql_zig_pages.h"
#include <stdint.h>
#include <stdio.h>
// This symbol must originate from a Noqeri-generated machine-code object,
// never from a substitute C/JS implementation.
extern int64_t noeNativeIndexSmoke(const char *);
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    const int64_t status = noeNativeIndexSmoke(argv[1]);
    if (status != 0) {
        fprintf(stderr, "Noqeri native index smoke failed: %lld\n", (long long)status);
        return 1;
    }
    void *handle = NULL;
    if (noesql_zig_page_open(NOESQL_ZIG_PAGE_ABI_VERSION, argv[1], &handle) != 0 || !handle) return 3;
    uint32_t page = 0;
    const int result = noesql_zig_index_find(handle, 5u, -3, &page);
    int good = result == NOESQL_ZIG_PAGE_OK && page == 9u;
    if (good) good = noesql_zig_page_checkpoint(handle) == NOESQL_ZIG_PAGE_OK;
    noesql_zig_page_close(handle);
    if (!good) return 4;
    // Cold reopen verifies the checkpoint rather than merely the WAL cache.
    handle = NULL;
    if (noesql_zig_page_open(NOESQL_ZIG_PAGE_ABI_VERSION, argv[1], &handle) != 0 || !handle) return 5;
    page = 0;
    const int recovered = noesql_zig_index_find(handle, 5u, -3, &page);
    noesql_zig_page_close(handle);
    return recovered == NOESQL_ZIG_PAGE_OK && page == 9u ? 0 : 6;
}

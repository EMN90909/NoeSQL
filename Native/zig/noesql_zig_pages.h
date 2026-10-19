#ifndef NOESQL_ZIG_PAGES_H
#define NOESQL_ZIG_PAGES_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Experimental Linux-only Zig native pager ABI v2.
 * Not a SQL engine and not authenticated. Trusted callers only.
 * Treat handles as opaque and never reuse after close. */
#define NOESQL_ZIG_PAGER_COMPONENT_VERSION "1.0.0"
#define NOESQL_ZIG_PAGE_ABI_VERSION 2u
#define NOESQL_ZIG_PAGE_BYTES 4096u
#define NOESQL_ZIG_PAGE_OK 0
#define NOESQL_ZIG_PAGE_NOT_FOUND 1
#define NOESQL_ZIG_PAGE_INVALID (-1)
#define NOESQL_ZIG_PAGE_FAILURE (-2)
#define NOESQL_ZIG_PAGE_BUFFER_TOO_SMALL (-3)
int32_t noesql_zig_page_open(uint32_t version,const char* path,void** output);
void noesql_zig_page_close(void* handle);
int32_t noesql_zig_page_stage(void* handle,uint32_t id,const uint8_t* data,size_t length);
int32_t noesql_zig_page_commit(void* handle);
int32_t noesql_zig_page_rollback(void* handle);
int32_t noesql_zig_page_get(void* handle,uint32_t id,uint8_t* output,size_t capacity);
uint64_t noesql_zig_page_generation(void* handle);
int32_t noesql_zig_page_validate_reference_state(void* handle);
int32_t noesql_zig_page_checkpoint(void* handle);
#ifdef __cplusplus
}
#endif
#endif

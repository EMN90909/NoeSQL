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
#define NOESQL_ZIG_INDEX_DUPLICATE (-4)
#define NOESQL_ZIG_INDEX_FULL (-5)
#define NOESQL_ZIG_INDEX_CAPACITY 337u
int32_t noesql_zig_page_open(uint32_t version,const char* path,void** output);
void noesql_zig_page_close(void* handle);
int32_t noesql_zig_page_stage(void* handle,uint32_t id,const uint8_t* data,size_t length);
int32_t noesql_zig_page_commit(void* handle);
int32_t noesql_zig_page_rollback(void* handle);
int32_t noesql_zig_page_get(void* handle,uint32_t id,uint8_t* output,size_t capacity);
uint64_t noesql_zig_page_generation(void* handle);
int32_t noesql_zig_page_validate_reference_state(void* handle);
int32_t noesql_zig_page_checkpoint(void* handle);
int32_t noesql_zig_snapshot_write(void* handle,const uint8_t* input,size_t size);
int32_t noesql_zig_snapshot_read(void* handle,uint8_t* output,size_t capacity,size_t* written);
/* Trusted Noqeri transaction handle only. Reserve each index page, never use
 * page zero or reuse a catalog/data page. Operations are staged and require
 * noesql_zig_page_commit(); they are discarded by rollback/close.
 * Single-page integer index only: 337 unique keys, no B+tree or SQL planning. */
int32_t noesql_zig_index_find(void* handle,uint32_t index_page,int64_t key,uint32_t* out_page);
int32_t noesql_zig_index_insert(void* handle,uint32_t index_page,int64_t key,uint32_t data_page);
int32_t noesql_zig_index_delete(void* handle,uint32_t index_page,int64_t key);
int32_t noesql_zig_index_range(void* handle,uint32_t index_page,int64_t minimum,int64_t maximum,int64_t* keys,uint32_t* pages,size_t capacity,size_t* written);
#ifdef __cplusplus
}
#endif
#endif

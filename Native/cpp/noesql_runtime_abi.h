#ifndef NOESQL_RUNTIME_ABI_H
#define NOESQL_RUNTIME_ABI_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* NoeSQL owns the engine. This table is supplied by an actual NoeSQL provider,
 * never by the Noqeri compiler. Handles are opaque provider-owned values.
 * Input strings are pointer+length, not null-terminated. All callbacks are
 * synchronous. Result bytes belong to the caller and never alias inputs. */
#define NOESQL_RUNTIME_ABI_VERSION 1u
#define NOESQL_RUNTIME_MAX_INPUT (1024u * 1024u)
#define NOESQL_RUNTIME_MAX_RESULT (16u * 1024u * 1024u)
typedef uint64_t noesql_handle;
typedef uint64_t noesql_statement;
enum noesql_runtime_status {
 NOESQL_RUNTIME_OK = 0,
 NOESQL_RUNTIME_INVALID = -1,
 NOESQL_RUNTIME_VERSION = -2,
 NOESQL_RUNTIME_LIMIT = -3,
 NOESQL_RUNTIME_UNAVAILABLE = -4
};
typedef int32_t (*noesql_open_fn)(void *,const char *,size_t,noesql_handle *);
typedef int32_t (*noesql_close_fn)(void *,noesql_handle);
typedef int32_t (*noesql_prepare_fn)(void *,noesql_handle,const uint8_t *,size_t,noesql_statement *);
typedef int32_t (*noesql_run_fn)(void *,noesql_handle,noesql_statement,const uint8_t *,size_t,uint8_t *,size_t,size_t *);
typedef int32_t (*noesql_finish_fn)(void *,noesql_handle,noesql_statement);
typedef int32_t (*noesql_transaction_fn)(void *,noesql_handle);
typedef int32_t (*noesql_error_fn)(void *,noesql_handle,char *,size_t,size_t *);
typedef struct noesql_runtime_v1 {
 uint32_t abi_version;
 uint32_t struct_size;
 void *context;
 noesql_open_fn open;
 noesql_close_fn close;
 noesql_prepare_fn prepare;
 noesql_run_fn execute;
 noesql_run_fn query;
 noesql_finish_fn finalize;
 noesql_transaction_fn begin;
 noesql_transaction_fn commit;
 noesql_transaction_fn rollback;
 noesql_error_fn last_error;
} noesql_runtime_v1;
int32_t noesql_runtime_validate(const noesql_runtime_v1 *);
int32_t noesql_runtime_open(const noesql_runtime_v1 *,const char *,size_t,noesql_handle *);
int32_t noesql_runtime_close(const noesql_runtime_v1 *,noesql_handle);
int32_t noesql_runtime_prepare(const noesql_runtime_v1 *,noesql_handle,const uint8_t *,size_t,noesql_statement *);
int32_t noesql_runtime_execute(const noesql_runtime_v1 *,noesql_handle,noesql_statement,const uint8_t *,size_t,uint8_t *,size_t,size_t *);
int32_t noesql_runtime_query(const noesql_runtime_v1 *,noesql_handle,noesql_statement,const uint8_t *,size_t,uint8_t *,size_t,size_t *);
int32_t noesql_runtime_finalize(const noesql_runtime_v1 *,noesql_handle,noesql_statement);
int32_t noesql_runtime_begin(const noesql_runtime_v1 *,noesql_handle);
int32_t noesql_runtime_commit(const noesql_runtime_v1 *,noesql_handle);
int32_t noesql_runtime_rollback(const noesql_runtime_v1 *,noesql_handle);
int32_t noesql_runtime_last_error(const noesql_runtime_v1 *,noesql_handle,char *,size_t,size_t *);
#ifdef __cplusplus
}
#endif
#endif

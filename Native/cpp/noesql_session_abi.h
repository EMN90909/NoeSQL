#ifndef NOESQL_SESSION_ABI_H
#define NOESQL_SESSION_ABI_H
#include <stddef.h>
#include <stdint.h>
#include "noesql_engine_abi.h"
#ifdef __cplusplus
extern "C" {
#endif

/* Stable narrow v2 connection contract. Ownership of connections and statements
 * stays with the NoeSQL provider. V1 execute-only ABI remains unchanged.
 * This header DOES NOT claim a qualifying native implementation exists.
 */
#define NOESQL_SESSION_ABI_VERSION 2u
typedef void *noesql_connection_handle;
typedef void *noesql_statement_handle;

typedef struct noesql_session_v2 {
  uint32_t abi_version;
  void *context;
  int32_t (*open)(void *context, const uint8_t *path, size_t path_len, noesql_connection_handle *out);
  int32_t (*close)(void *context, noesql_connection_handle conn);
  int32_t (*prepare)(void *context, noesql_connection_handle conn,
                     const uint8_t *query, size_t query_len, noesql_statement_handle *out);
  int32_t (*release)(void *context, noesql_connection_handle conn, noesql_statement_handle stmt);
  /* Inputs and outputs are length-delimited UTF-8 JSON/NoeSQL data contracts.
   * Provider validates binding types; callers cannot interpolate trusted SQL.
   */
  int32_t (*execute)(void *context, noesql_connection_handle conn, noesql_statement_handle stmt,
                     const uint8_t *params, size_t params_len,
                     uint8_t *output, size_t cap, size_t *out_len);
  int32_t (*query)(void *context, noesql_connection_handle conn, noesql_statement_handle stmt,
                   const uint8_t *params, size_t params_len,
                   uint8_t *output, size_t cap, size_t *out_len);
  int32_t (*begin)(void *context, noesql_connection_handle conn);
  int32_t (*commit)(void *context, noesql_connection_handle conn);
  int32_t (*rollback)(void *context, noesql_connection_handle conn);
  int32_t (*last_error)(void *context, noesql_connection_handle conn,
                        uint8_t *output, size_t cap, size_t *out_len);
} noesql_session_v2;

int32_t noesql_cpp_validate_session(const noesql_session_v2 *session);
int32_t noesql_cpp_open(const noesql_session_v2 *session,const uint8_t *path,size_t path_len,noesql_connection_handle *out);
int32_t noesql_cpp_close(const noesql_session_v2 *session,noesql_connection_handle conn);
int32_t noesql_cpp_prepare(const noesql_session_v2 *session,noesql_connection_handle conn,const uint8_t *source,size_t size,noesql_statement_handle *out);
int32_t noesql_cpp_release(const noesql_session_v2 *session,noesql_connection_handle conn,noesql_statement_handle stmt);
int32_t noesql_cpp_execute(const noesql_session_v2 *session,noesql_connection_handle conn,noesql_statement_handle stmt,const uint8_t *params,size_t size,uint8_t *buffer,size_t cap,size_t *written);
int32_t noesql_cpp_query(const noesql_session_v2 *session,noesql_connection_handle conn,noesql_statement_handle stmt,const uint8_t *params,size_t size,uint8_t *buffer,size_t cap,size_t *written);
int32_t noesql_cpp_begin(const noesql_session_v2 *session,noesql_connection_handle conn);
int32_t noesql_cpp_commit(const noesql_session_v2 *session,noesql_connection_handle conn);
int32_t noesql_cpp_rollback(const noesql_session_v2 *session,noesql_connection_handle conn);
int32_t noesql_cpp_last_error(const noesql_session_v2 *session,noesql_connection_handle conn,uint8_t *output,size_t cap,size_t *written);
#ifdef __cplusplus
}
#endif
#endif

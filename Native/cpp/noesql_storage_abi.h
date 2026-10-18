#ifndef NOESQL_STORAGE_ABI_H
#define NOESQL_STORAGE_ABI_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
// Versioned native storage ABI for a future Noqeri-owned query executor.
// Not a replacement for the separate noesql_engine_v1 language callback.
#define NOESQL_STORAGE_ABI_VERSION 1u
typedef struct noesql_storage noesql_storage;
typedef struct noesql_session noesql_session;
enum noesql_storage_status {
  NOESQL_STORAGE_OK=0, NOESQL_STORAGE_NOT_FOUND=1,
  NOESQL_STORAGE_ERROR=-1, NOESQL_STORAGE_BUFFER_TOO_SMALL=-2,
  NOESQL_STORAGE_INVALID_ARGUMENT=-3
};
int32_t noesql_storage_open(uint32_t abi_version,const char* path,noesql_storage** out);
void noesql_storage_close(noesql_storage* s);
int32_t noesql_storage_initialize(noesql_storage* s,const uint8_t* password,size_t password_len);
int32_t noesql_storage_login(noesql_storage* s,const char* user,const uint8_t* password,size_t password_len,noesql_session** out);
void noesql_storage_logout(noesql_session* session);
int32_t noesql_storage_get(noesql_storage* s,const noesql_session* session,
     const uint8_t* key,size_t key_len,uint8_t* output,size_t capacity,size_t* required);
int32_t noesql_storage_put(noesql_storage* s,const noesql_session* session,
     const uint8_t* key,size_t key_len,const uint8_t* value,size_t value_len);
int32_t noesql_storage_delete(noesql_storage* s,const noesql_session* session,
     const uint8_t* key,size_t key_len);
// Thread-local diagnostics contain no passwords or record contents.
const char* noesql_storage_last_error(void);
#ifdef __cplusplus
}
#endif
#endif

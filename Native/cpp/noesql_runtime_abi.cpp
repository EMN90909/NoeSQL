#include "noesql_runtime_abi.h"
extern "C" int32_t noesql_runtime_validate(const noesql_runtime_v1 *v) {
 if (!v) return NOESQL_RUNTIME_INVALID;
 if (v->abi_version != NOESQL_RUNTIME_ABI_VERSION || v->struct_size < sizeof(noesql_runtime_v1)) return NOESQL_RUNTIME_VERSION;
 return v->open && v->close && v->prepare && v->execute && v->query && v->finalize &&
   v->begin && v->commit && v->rollback && v->last_error ? NOESQL_RUNTIME_OK : NOESQL_RUNTIME_UNAVAILABLE;
}
extern "C" int32_t noesql_runtime_open(const noesql_runtime_v1 *v,const char *p,size_t n,noesql_handle *h) {
 if (!h) return NOESQL_RUNTIME_INVALID; *h=0;
 int32_t status=noesql_runtime_validate(v); if(status) return status;
 if (!p || !n || n>4096) return NOESQL_RUNTIME_INVALID;
 return v->open(v->context,p,n,h);
}
extern "C" int32_t noesql_runtime_close(const noesql_runtime_v1 *v,noesql_handle h) {
 int32_t s=noesql_runtime_validate(v);return s?s:!h?NOESQL_RUNTIME_INVALID:v->close(v->context,h);
}
extern "C" int32_t noesql_runtime_prepare(const noesql_runtime_v1 *v,noesql_handle h,const uint8_t *p,size_t n,noesql_statement *s) {
 if(!s) return NOESQL_RUNTIME_INVALID; *s=0;
 int32_t r=noesql_runtime_validate(v);if(r)return r;
 if(!h||!p||!n||n>NOESQL_RUNTIME_MAX_INPUT)return NOESQL_RUNTIME_INVALID;
 return v->prepare(v->context,h,p,n,s);
}
static int32_t run(const noesql_runtime_v1 *v,noesql_run_fn fn,noesql_handle h,noesql_statement s,
 const uint8_t *params,size_t pn,uint8_t *result,size_t capacity,size_t *used) {
 if(!used)return NOESQL_RUNTIME_INVALID;*used=0;
 int32_t r=noesql_runtime_validate(v);if(r)return r;
 if(!h||!s||(pn&&!params)||(capacity&&!result))return NOESQL_RUNTIME_INVALID;
 if(pn>NOESQL_RUNTIME_MAX_INPUT||capacity>NOESQL_RUNTIME_MAX_RESULT)return NOESQL_RUNTIME_LIMIT;
 r=fn(v->context,h,s,params,pn,result,capacity,used);
 if(r||*used>capacity){*used=0;return r?r:NOESQL_RUNTIME_LIMIT;}return NOESQL_RUNTIME_OK;
}
extern "C" int32_t noesql_runtime_execute(const noesql_runtime_v1 *v,noesql_handle h,noesql_statement s,const uint8_t *p,size_t n,uint8_t *out,size_t cap,size_t *used) {
 return run(v,v?v->execute:nullptr,h,s,p,n,out,cap,used);
}
extern "C" int32_t noesql_runtime_query(const noesql_runtime_v1 *v,noesql_handle h,noesql_statement s,const uint8_t *p,size_t n,uint8_t *out,size_t cap,size_t *used) {
 return run(v,v?v->query:nullptr,h,s,p,n,out,cap,used);
}
extern "C" int32_t noesql_runtime_finalize(const noesql_runtime_v1 *v,noesql_handle h,noesql_statement s) {
 int32_t r=noesql_runtime_validate(v);return r?r:(!h||!s)?NOESQL_RUNTIME_INVALID:v->finalize(v->context,h,s);
}
extern "C" int32_t noesql_runtime_begin(const noesql_runtime_v1 *v,noesql_handle h) {
 int32_t r=noesql_runtime_validate(v);return r?r:!h?NOESQL_RUNTIME_INVALID:v->begin(v->context,h);
}
extern "C" int32_t noesql_runtime_commit(const noesql_runtime_v1 *v,noesql_handle h) {
 int32_t r=noesql_runtime_validate(v);return r?r:!h?NOESQL_RUNTIME_INVALID:v->commit(v->context,h);
}
extern "C" int32_t noesql_runtime_rollback(const noesql_runtime_v1 *v,noesql_handle h) {
 int32_t r=noesql_runtime_validate(v);return r?r:!h?NOESQL_RUNTIME_INVALID:v->rollback(v->context,h);
}
extern "C" int32_t noesql_runtime_last_error(const noesql_runtime_v1 *v,noesql_handle h,char *out,size_t cap,size_t *used) {
 if(!used)return NOESQL_RUNTIME_INVALID;*used=0;
 int32_t r=noesql_runtime_validate(v);if(r)return r;
 if(!h||(cap&&!out)||cap>NOESQL_RUNTIME_MAX_RESULT)return NOESQL_RUNTIME_INVALID;
 r=v->last_error(v->context,h,out,cap,used);
 if(r||*used>cap){*used=0;return r?r:NOESQL_RUNTIME_LIMIT;}return NOESQL_RUNTIME_OK;
}

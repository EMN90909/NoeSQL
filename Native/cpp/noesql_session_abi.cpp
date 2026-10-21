#include "noesql_session_abi.h"
#include <cstring>
namespace {
bool valid(const noesql_session_v2 *s){
 return s&&s->abi_version==NOESQL_SESSION_ABI_VERSION&&s->open&&s->close&&s->prepare&&s->release&&s->execute&&s->query&&s->begin&&s->commit&&s->rollback&&s->last_error;
}
int32_t forward(const noesql_session_v2 *s,noesql_connection_handle conn,noesql_statement_handle stmt,const uint8_t *params,size_t size,uint8_t *out,size_t cap,size_t *written,bool query) {
 if(!written)return NOESQL_BRIDGE_INVALID_ARGUMENT;
 *written=0;
 if(!valid(s))return s?NOESQL_BRIDGE_UNSUPPORTED_ABI:NOESQL_BRIDGE_INVALID_ARGUMENT;
 if(!conn||!stmt||(size&&!params)||(cap&&!out))return NOESQL_BRIDGE_INVALID_ARGUMENT;
 if(size>NOESQL_MAX_SOURCE_BYTES||cap>NOESQL_MAX_OUTPUT_BYTES)return NOESQL_BRIDGE_LIMIT_EXCEEDED;
 const int32_t status=query?s->query(s->context,conn,stmt,params,size,out,cap,written):s->execute(s->context,conn,stmt,params,size,out,cap,written);
 if(status!=NOESQL_BRIDGE_OK||*written>cap){
   *written=0;
   return status==NOESQL_BRIDGE_OK?NOESQL_BRIDGE_LIMIT_EXCEEDED:status;
 }
 return NOESQL_BRIDGE_OK;
}
}
extern "C" int32_t noesql_cpp_session_validate(const noesql_session_v2 *s){
 if(!s)return NOESQL_BRIDGE_INVALID_ARGUMENT;
 if(s->abi_version!=NOESQL_SESSION_ABI_VERSION)return NOESQL_BRIDGE_UNSUPPORTED_ABI;
 return valid(s)?NOESQL_BRIDGE_OK:NOESQL_BRIDGE_INVALID_ARGUMENT;
}
extern "C" int32_t noesql_cpp_session_open(const noesql_session_v2 *s,const uint8_t *path,size_t n,noesql_connection_handle *out){
 if(!out)return NOESQL_BRIDGE_INVALID_ARGUMENT;
 *out=nullptr;
 const int32_t status=noesql_cpp_session_validate(s);
 if(status!=NOESQL_BRIDGE_OK)return status;
 if(!path||n==0||n>4096||std::memchr(path,0,n))return NOESQL_BRIDGE_INVALID_ARGUMENT;
 return s->open(s->context,path,n,out);
}
extern "C" int32_t noesql_cpp_session_close(const noesql_session_v2 *s,noesql_connection_handle conn){
 const int32_t status=noesql_cpp_session_validate(s);
 return status!=NOESQL_BRIDGE_OK?status:(!conn?NOESQL_BRIDGE_INVALID_ARGUMENT:s->close(s->context,conn));
}
extern "C" int32_t noesql_cpp_session_prepare(const noesql_session_v2 *s,noesql_connection_handle conn,const uint8_t *source,size_t size,noesql_statement_handle *out){
 if(!out)return NOESQL_BRIDGE_INVALID_ARGUMENT;
 *out=nullptr;
 const int32_t status=noesql_cpp_session_validate(s);
 if(status!=NOESQL_BRIDGE_OK)return status;
 if(!conn||!source||size==0)return NOESQL_BRIDGE_INVALID_ARGUMENT;
 if(size>NOESQL_MAX_SOURCE_BYTES)return NOESQL_BRIDGE_LIMIT_EXCEEDED;
 return s->prepare(s->context,conn,source,size,out);
}
extern "C" int32_t noesql_cpp_session_release(const noesql_session_v2 *s,noesql_connection_handle conn,noesql_statement_handle stmt){
 const int32_t status=noesql_cpp_session_validate(s);
 return status!=NOESQL_BRIDGE_OK?status:(!conn||!stmt?NOESQL_BRIDGE_INVALID_ARGUMENT:s->release(s->context,conn,stmt));
}
extern "C" int32_t noesql_cpp_session_execute(const noesql_session_v2 *s,noesql_connection_handle conn,noesql_statement_handle stmt,const uint8_t *params,size_t n,uint8_t *out,size_t cap,size_t *written){
 return forward(s,conn,stmt,params,n,out,cap,written,false);
}
extern "C" int32_t noesql_cpp_session_query(const noesql_session_v2 *s,noesql_connection_handle conn,noesql_statement_handle stmt,const uint8_t *params,size_t n,uint8_t *out,size_t cap,size_t *written){
 return forward(s,conn,stmt,params,n,out,cap,written,true);
}
extern "C" int32_t noesql_cpp_session_begin(const noesql_session_v2 *s,noesql_connection_handle conn){
 const int32_t status=noesql_cpp_session_validate(s);
 return status!=NOESQL_BRIDGE_OK?status:(!conn?NOESQL_BRIDGE_INVALID_ARGUMENT:s->begin(s->context,conn));
}
extern "C" int32_t noesql_cpp_session_commit(const noesql_session_v2 *s,noesql_connection_handle conn){
 const int32_t status=noesql_cpp_session_validate(s);
 return status!=NOESQL_BRIDGE_OK?status:(!conn?NOESQL_BRIDGE_INVALID_ARGUMENT:s->commit(s->context,conn));
}
extern "C" int32_t noesql_cpp_session_rollback(const noesql_session_v2 *s,noesql_connection_handle conn){
 const int32_t status=noesql_cpp_session_validate(s);
 return status!=NOESQL_BRIDGE_OK?status:(!conn?NOESQL_BRIDGE_INVALID_ARGUMENT:s->rollback(s->context,conn));
}
extern "C" int32_t noesql_cpp_session_last_error(const noesql_session_v2 *s,noesql_connection_handle conn,uint8_t *out,size_t cap,size_t *written){
 if(!written)return NOESQL_BRIDGE_INVALID_ARGUMENT;
 *written=0;
 const int32_t status=noesql_cpp_session_validate(s);
 if(status!=NOESQL_BRIDGE_OK)return status;
 if(!conn||(cap&&!out))return NOESQL_BRIDGE_INVALID_ARGUMENT;
 if(cap>NOESQL_MAX_OUTPUT_BYTES)return NOESQL_BRIDGE_LIMIT_EXCEEDED;
 const int32_t result=s->last_error(s->context,conn,out,cap,written);
 if(result!=NOESQL_BRIDGE_OK||*written>cap){*written=0;return result==NOESQL_BRIDGE_OK?NOESQL_BRIDGE_LIMIT_EXCEEDED:result;}
 return NOESQL_BRIDGE_OK;
}

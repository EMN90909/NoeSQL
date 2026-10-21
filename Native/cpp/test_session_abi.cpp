#include "noesql_session_abi.h"
#include <cassert>
#include <cstring>
#include <cstdint>
static int fake_connection = 1;
static int fake_statement = 2;
static int32_t open_fn(void*,const uint8_t*,size_t,noesql_connection_handle*out){*out=&fake_connection;return 0;}
static int32_t close_fn(void*,noesql_connection_handle){return 0;}
static int32_t prepare_fn(void*,noesql_connection_handle,const uint8_t*,size_t,noesql_statement_handle*out){*out=&fake_statement;return 0;}
static int32_t release_fn(void*,noesql_connection_handle,noesql_statement_handle){return 0;}
static int32_t statement_fn(void*,noesql_connection_handle,noesql_statement_handle,const uint8_t*,size_t,uint8_t*out,size_t cap,size_t*written){if(cap<2)return -3;out[0]='[';out[1]=']';*written=2;return 0;}
static int32_t trans_fn(void*,noesql_connection_handle){return 0;}
static int32_t error_fn(void*,noesql_connection_handle,uint8_t*,size_t,size_t*out){*out=0;return 0;}
int main(){
 noesql_session_v2 api{NOESQL_SESSION_ABI_VERSION,nullptr,open_fn,close_fn,prepare_fn,release_fn,statement_fn,statement_fn,trans_fn,trans_fn,trans_fn,error_fn};
 assert(noesql_cpp_session_validate(&api)==0);
 noesql_connection_handle conn=nullptr;
 assert(noesql_cpp_session_open(&api,(const uint8_t*)"app.noedb",9,&conn)==0 && conn);
 noesql_statement_handle stmt=nullptr;
 assert(noesql_cpp_session_prepare(&api,conn,(const uint8_t*)"select",6,&stmt)==0 && stmt);
 uint8_t result[4]{};size_t written=999;
 assert(noesql_cpp_session_query(&api,conn,stmt,nullptr,0,result,sizeof(result),&written)==0);
 assert(written==2 && result[0]=='[' && result[1]==']');
 assert(noesql_cpp_session_begin(&api,conn)==0 && noesql_cpp_session_commit(&api,conn)==0 && noesql_cpp_session_rollback(&api,conn)==0);
 assert(noesql_cpp_session_query(&api,conn,stmt,nullptr,0,result,1,&written)==-3 && written==0);
 assert(noesql_cpp_session_prepare(&api,conn,(const uint8_t*)"x",NOESQL_MAX_SOURCE_BYTES+1,&stmt)==NOESQL_BRIDGE_LIMIT_EXCEEDED);
 const uint8_t nulpath[]={'x',0,'y'};
 assert(noesql_cpp_session_open(&api,nulpath,3,&conn)==NOESQL_BRIDGE_INVALID_ARGUMENT);
 api.abi_version=3;
 assert(noesql_cpp_session_validate(&api)==NOESQL_BRIDGE_UNSUPPORTED_ABI);
 api.abi_version=NOESQL_SESSION_ABI_VERSION;
 assert(noesql_cpp_session_release(&api,&fake_connection,&fake_statement)==0);
 assert(noesql_cpp_session_close(&api,&fake_connection)==0);
 return 0;
}

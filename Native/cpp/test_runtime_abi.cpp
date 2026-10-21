#include "noesql_runtime_abi.h"
#include <cassert>
#include <cstring>
static int32_t open_db(void*,const char *p,size_t n,noesql_handle *h){assert(n==8 && std::memcmp(p,"db.noedb",8)==0);*h=42;return 0;}
static int32_t close_db(void*,noesql_handle h){return h==42?0:-5;}
static int32_t prepare(void*,noesql_handle,const uint8_t*,size_t,noesql_statement *s){*s=7;return 0;}
static int32_t run(void*,noesql_handle,noesql_statement,const uint8_t*,size_t,uint8_t *out,size_t cap,size_t *used){
 if(cap<2)return -7;out[0]='[';out[1]=']';*used=2;return 0;
}
static int32_t finish(void*,noesql_handle,noesql_statement){return 0;}
static int32_t tx(void*,noesql_handle){return 0;}
static int32_t err(void*,noesql_handle,char*,size_t,size_t *n){*n=0;return 0;}
int main(){
 noesql_runtime_v1 api={NOESQL_RUNTIME_ABI_VERSION,sizeof(noesql_runtime_v1),nullptr,open_db,close_db,prepare,run,run,finish,tx,tx,tx,err};
 assert(noesql_runtime_validate(&api)==0);
 noesql_handle db=0;assert(noesql_runtime_open(&api,"db.noedb",8,&db)==0 && db==42);
 noesql_statement statement=0;
 const uint8_t sql[]="items get *";
 assert(noesql_runtime_prepare(&api,db,sql,sizeof(sql)-1,&statement)==0);
 uint8_t output[4]{};size_t used=99;
 assert(noesql_runtime_query(&api,db,statement,nullptr,0,output,sizeof(output),&used)==0&&used==2);
 assert(noesql_runtime_execute(&api,db,statement,nullptr,0,output,sizeof(output),&used)==0);
 assert(noesql_runtime_begin(&api,db)==0&&noesql_runtime_commit(&api,db)==0&&noesql_runtime_rollback(&api,db)==0);
 assert(noesql_runtime_last_error(&api,db,nullptr,0,&used)==0);
 assert(noesql_runtime_finalize(&api,db,statement)==0&&noesql_runtime_close(&api,db)==0);
 api.abi_version=99;assert(noesql_runtime_validate(&api)==NOESQL_RUNTIME_VERSION);
 api.abi_version=1;api.query=nullptr;assert(noesql_runtime_validate(&api)==NOESQL_RUNTIME_UNAVAILABLE);
 return 0;
}

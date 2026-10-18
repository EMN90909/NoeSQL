#include "noesql_storage_abi.h"
#include "noesql_page_store.h"
#include <cstring>
#include <exception>
#include <memory>
#include <string>
#include <vector>
struct noesql_storage {noesql::Store store;explicit noesql_storage(std::string path):store(std::move(path)) {}};
struct noesql_session {noesql::Session session;noesql_storage* owner;noesql_session(noesql::Session token,noesql_storage* s):session(std::move(token)),owner(s){}};
namespace {
thread_local std::string last_error;
int32_t rejected(const char* why){last_error=why;return NOESQL_STORAGE_INVALID_ARGUMENT;}
template<class F>int32_t guarded(F&& f){try{last_error.clear();return f();}catch(const std::exception& e){last_error=e.what();return NOESQL_STORAGE_ERROR;}catch(...){last_error="unknown storage failure";return NOESQL_STORAGE_ERROR;}}
std::string bytes(const uint8_t* p,size_t n){return n==0?std::string():std::string(reinterpret_cast<const char*>(p),n);}
bool sane(const uint8_t* p,size_t n){return p!=nullptr||n==0;}
}
extern "C" int32_t noesql_storage_open(uint32_t version,const char* path,noesql_storage** out){
 if(!out)return rejected("missing out parameter");
 *out=nullptr;
 if(version!=NOESQL_STORAGE_ABI_VERSION||!path)return rejected("ABI version or path mismatch");
 return guarded([&]{*out=new noesql_storage(path);return NOESQL_STORAGE_OK;});
}
extern "C" void noesql_storage_close(noesql_storage* s){delete s;}
extern "C" int32_t noesql_storage_initialize(noesql_storage* s,const uint8_t* p,size_t n){
 if(!s||!sane(p,n)||n>1024)return rejected("invalid initialization arguments");
 return guarded([&]{s->store.initialize(bytes(p,n));return NOESQL_STORAGE_OK;});
}
extern "C" int32_t noesql_storage_login(noesql_storage* s,const char* user,const uint8_t* p,size_t n,noesql_session** out){
 if(!s||!user||!out||!sane(p,n)||n>1024)return rejected("invalid login arguments");
 *out=nullptr;
 return guarded([&]{*out=new noesql_session(s->store.login(user,bytes(p,n)),s);return NOESQL_STORAGE_OK;});
}
extern "C" void noesql_storage_logout(noesql_session* session){delete session;}
extern "C" int32_t noesql_storage_get(noesql_storage* s,const noesql_session* session,const uint8_t* key,size_t key_len,uint8_t* out,size_t capacity,size_t* required){
 if(!required)return rejected("missing required length");
 *required=0;
 if(!s||!session||session->owner!=s||!sane(key,key_len)||key_len>128||(out==nullptr&&capacity!=0))return rejected("invalid get arguments");
 return guarded([&]{auto v=s->store.get(session->session,bytes(key,key_len));if(!v)return NOESQL_STORAGE_NOT_FOUND;
     *required=v->size();if(*required>capacity)return NOESQL_STORAGE_BUFFER_TOO_SMALL;
     if(!v->empty())std::memcpy(out,v->data(),v->size());
     return NOESQL_STORAGE_OK;});
}
extern "C" int32_t noesql_storage_put(noesql_storage* s,const noesql_session* session,const uint8_t* key,size_t key_len,const uint8_t* val,size_t val_len){
 if(!s||!session||session->owner!=s||!sane(key,key_len)||!sane(val,val_len)||key_len>128||val_len>1024)return rejected("invalid put arguments");
 return guarded([&]{s->store.transact(session->session,{{noesql::Mutation::Kind::Put,bytes(key,key_len),bytes(val,val_len)}});return NOESQL_STORAGE_OK;});
}
extern "C" int32_t noesql_storage_delete(noesql_storage* s,const noesql_session* session,const uint8_t* key,size_t key_len){
 if(!s||!session||session->owner!=s||!sane(key,key_len)||key_len>128)return rejected("invalid delete arguments");
 return guarded([&]{s->store.transact(session->session,{{noesql::Mutation::Kind::Delete,bytes(key,key_len),""}});return NOESQL_STORAGE_OK;});
}
extern "C" const char* noesql_storage_last_error(void){return last_error.c_str();}

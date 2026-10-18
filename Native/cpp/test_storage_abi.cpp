#include "noesql_storage_abi.h"
#include <cassert>
#include <cstdlib>
#include <filesystem>
#include <string>
int main(){
 char templ[]="/tmp/noesql-api-XXXXXX";auto dir=std::string(mkdtemp(templ));auto path=dir+"/test.nqd";
 noesql_storage* db=nullptr;noesql_session* session=nullptr;
 assert(noesql_storage_open(100,path.c_str(),&db)==NOESQL_STORAGE_INVALID_ARGUMENT&&db==nullptr);
 assert(noesql_storage_open(NOESQL_STORAGE_ABI_VERSION,path.c_str(),&db)==NOESQL_STORAGE_OK);
 const std::string pw="ABI integration test password 2026";
 const auto* p=reinterpret_cast<const uint8_t*>(pw.data());
 assert(noesql_storage_initialize(db,p,pw.size())==NOESQL_STORAGE_OK);
 assert(noesql_storage_login(db,"admin",p,pw.size(),&session)==NOESQL_STORAGE_OK);
 const uint8_t key[]={'x'};const uint8_t value[]={'y','z'};size_t needed=42;
 assert(noesql_storage_get(db,session,key,sizeof(key),nullptr,0,&needed)==NOESQL_STORAGE_NOT_FOUND);
 assert(noesql_storage_put(db,session,key,sizeof(key),value,sizeof(value))==NOESQL_STORAGE_OK);
 assert(noesql_storage_get(db,session,key,sizeof(key),nullptr,0,&needed)==NOESQL_STORAGE_BUFFER_TOO_SMALL&&needed==2);
 uint8_t buf[2]={};assert(noesql_storage_get(db,session,key,sizeof(key),buf,sizeof(buf),&needed)==NOESQL_STORAGE_OK);
 assert(buf[0]=='y'&&buf[1]=='z');
 assert(noesql_storage_delete(db,session,key,sizeof(key))==NOESQL_STORAGE_OK);
 assert(noesql_storage_get(db,session,key,sizeof(key),buf,sizeof(buf),&needed)==NOESQL_STORAGE_NOT_FOUND);
 noesql_storage_logout(session);noesql_storage_close(db);std::filesystem::remove_all(dir);
}

#include "noesql_page_store.h"
#include <cstdlib>
#include <filesystem>
#include <stdexcept>
#include <string>
#include <unistd.h>
int main(){
 char pattern[]="/tmp/noesql-prod-fault-XXXXXX";
 char* d=::mkdtemp(pattern);if(!d)throw std::runtime_error("mkdtemp failed");
 std::string dir=d;
 try{
  // Ordinary binaries must ignore environment-controlled test termination.
  ::setenv("NOESQL_TEST_CRASH_AFTER_WAL","1",1);
  ::setenv("NOESQL_TEST_CRASH_AFTER_CHECKPOINT","1",1);
  noesql::Store db(dir+"/db.nqd");
  db.initialize("testing password 12345678");
  auto s=db.login("admin","testing password 12345678");
  db.transact(s,{{noesql::Mutation::Kind::Put,"safe","survived"}});
  if(db.get(s,"safe")!=std::optional<std::string>("survived"))throw std::runtime_error("write missing");
 }catch(...){std::filesystem::remove_all(dir);throw;}
 std::filesystem::remove_all(dir);
}

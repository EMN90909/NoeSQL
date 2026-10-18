#include "noesql_page_store.h"
#include <cassert>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <functional>
#include <cstdio>
#include <fcntl.h>
#include <iostream>
#include <stdexcept>
#include <sys/wait.h>
#include <unistd.h>
#include <vector>

using noesql::Mutation;
const std::string pass="correct horse battery staple 2026";
const std::string readerPass="read only passphrase 2026";
static void must_fail(const std::function<void()>& f){try{f();}catch(const std::exception&){return;}throw std::runtime_error("expected rejection");}
static std::string testdir(){char tmpl[]="/tmp/noesql-native-XXXXXX";char* p=::mkdtemp(tmpl);if(!p)throw std::runtime_error("mkdtemp");return p;}
int main(){
 std::string dir=testdir(),path=dir+"/db.nqd";
 try{
  noesql::Store db(path);db.initialize(pass);assert(db.verify()==1);
  must_fail([&]{db.initialize(pass);});
  db.add_user(pass,"reader",readerPass,false);
  auto readerSession=db.login("reader",readerPass);
  assert(!db.get(readerSession,"none"));
  must_fail([&]{db.add_user("wrong password phrase","attacker",readerPass,true);});
  must_fail([&]{db.transact("reader",readerPass,{{Mutation::Kind::Put,"x","b"}});});
  must_fail([&]{db.get("admin","wrong password phrase","x");});
  must_fail([&]{db.transact("admin",pass,{{Mutation::Kind::Put,"\x01" "user/admin","bad"}});});
  must_fail([&]{db.transact("admin",pass,{{Mutation::Kind::Put,"x",std::string(1025,'x')}});});
  db.transact("admin",pass,{{Mutation::Kind::Put,"a","first"},{Mutation::Kind::Put,"b","second"}});
  assert(db.get("reader",readerPass,"a")==std::optional<std::string>("first"));
  assert(!db.get("reader",readerPass,"absent"));
  std::vector<Mutation> many;for(int i=0;i<800;++i){char key[32];std::snprintf(key,sizeof(key),"row-%04d",i);many.push_back({Mutation::Kind::Put,key,std::string(120,'v')});}
  db.transact("admin",pass,many);
  {noesql::Store reopened(path);assert(reopened.get("reader",readerPass,"row-0799")==std::optional<std::string>(std::string(120,'v')));assert(reopened.scan("reader",readerPass,"row-").size()==800);}
  db.transact("admin",pass,{{Mutation::Kind::Delete,"row-0000",""}});
  assert(!db.get("admin",pass,"row-0000"));
  noesql::Limits small;small.max_mutations=1;noesql::Store limited(path,small);
  small.max_scan_rows=2;noesql::Store limitedReads(path,small);
  must_fail([&]{limitedReads.scan("admin",pass,"row-");});
  must_fail([&]{limited.transact("admin",pass,{{Mutation::Kind::Put,"a","1"},{Mutation::Kind::Put,"b","2"}});});
  assert(db.get("admin",pass,"a")==std::optional<std::string>("first"));
  // Explicit readers serialize with concurrent writers, preventing dirty/partial reads.
  pid_t child=fork();if(child==0){try{noesql::Store other(path);other.transact("admin",pass,{{Mutation::Kind::Put,"forked","yes"}});_exit(0);}catch(...){_exit(1);}}
  int status=0;assert(waitpid(child,&status,0)==child&&WIFEXITED(status)&&WEXITSTATUS(status)==0);
  assert(db.get("admin",pass,"forked")==std::optional<std::string>("yes"));
  // Independent processes serialize writes without losing either transaction.
  std::vector<pid_t> children;
  for(int i=0;i<4;++i){
    pid_t worker=fork();
    if(worker==0){
      try{
        noesql::Store other(path);auto token=other.login("admin",pass);
        for(int j=0;j<3;++j){char key[32];std::snprintf(key,sizeof(key),"worker-%d-%d",i,j);
          other.transact(token,{{Mutation::Kind::Put,key,"committed"}});
        }
        _exit(0);
      }catch(...){_exit(1);}
    }
    children.push_back(worker);
  }
  for(auto worker:children){assert(waitpid(worker,&status,0)==worker&&WIFEXITED(status)&&WEXITSTATUS(status)==0);}
  for(int i=0;i<4;++i)for(int j=0;j<3;++j){char key[32];std::snprintf(key,sizeof(key),"worker-%d-%d",i,j);
    assert(db.get("admin",pass,key)==std::optional<std::string>("committed"));}
  // Deterministic termination immediately after WAL is synced, before checkpoint.
  child=fork();if(child==0){setenv("NOESQL_TEST_CRASH_AFTER_WAL","1",1);noesql::Store other(path);other.transact("admin",pass,{{Mutation::Kind::Put,"crash","committed"}});_exit(0);}
  assert(waitpid(child,&status,0)==child&&WIFEXITED(status)&&WEXITSTATUS(status)==93);
  assert(db.get("admin",pass,"crash")==std::optional<std::string>("committed"));
  // Repair a torn checkpoint using the committed and checksummed WAL image.
  {int f=open(path.c_str(),O_WRONLY);assert(f>=0);char corruption='X';
    assert(pwrite(f,&corruption,1,4096+64)==1);close(f);}
  assert(db.get("admin",pass,"crash")==std::optional<std::string>("committed"));
  db.transact("admin",pass,{{Mutation::Kind::Put,"post","recovered"}});
  // After checkpoint rename, redo record is idempotent.
  child=fork();if(child==0){setenv("NOESQL_TEST_CRASH_AFTER_CHECKPOINT","1",1);noesql::Store other(path);other.transact("admin",pass,{{Mutation::Kind::Put,"after","checkpoint"}});_exit(0);}
  assert(waitpid(child,&status,0)==child&&WIFEXITED(status)&&WEXITSTATUS(status)==94);
  assert(db.get("admin",pass,"after")==std::optional<std::string>("checkpoint"));
  db.transact("admin",pass,{{Mutation::Kind::Put,"clean","yes"}});
  db.remove_user(pass,"reader");
  must_fail([&]{db.get(readerSession,"a");});
  // Page corruption must never be decoded as valid committed data.
  int fd=open(path.c_str(),O_WRONLY);assert(fd>=0);char bad='X';assert(pwrite(fd,&bad,1,4096+64)==1);close(fd);
  must_fail([&]{db.verify();});
  std::cout<<"native page/index/security/quota/recovery tests PASS\n";
 }catch(...){std::filesystem::remove_all(dir);throw;}
 std::filesystem::remove_all(dir);
}

// Local, single-process benchmark. Both engines use durable WAL and 120-byte values.
// SQLite does not provide embedded authentication, so access paths differ.
#include "noesql_page_store.h"
#include <sqlite3.h>
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
#include <unistd.h>
using Clock=std::chrono::steady_clock;
static double elapsed(Clock::time_point at){return std::chrono::duration<double,std::milli>(Clock::now()-at).count();}
static std::string key(int i){char s[32];std::snprintf(s,sizeof(s),"bench%08d",i);return s;}
static double pct(std::vector<double> x,int percentile){std::sort(x.begin(),x.end());return x[(x.size()-1)*size_t(percentile)/100];}
static void check(int rc,sqlite3 *db){if(rc!=SQLITE_OK&&rc!=SQLITE_DONE&&rc!=SQLITE_ROW)throw std::runtime_error(sqlite3_errmsg(db));}
struct Sqlite {sqlite3 *db=nullptr;explicit Sqlite(const std::string& path){check(sqlite3_open(path.c_str(),&db),db);}~Sqlite(){if(db)sqlite3_close(db);}void exec(const char *sql){char* error=nullptr;if(sqlite3_exec(db,sql,nullptr,nullptr,&error)!=SQLITE_OK){std::string msg=error?error:"sqlite failure";sqlite3_free(error);throw std::runtime_error(msg);}}};
struct Result {double batch_ms,single_ms,p50,p95,p99;};
static Result run_noesql(const std::string &path,int n){
 const std::string password="benchmark password deliberately long 2026";
 noesql::Store store(path);store.initialize(password);auto session=store.login("admin",password);
 std::vector<noesql::Mutation> batch;batch.reserve(size_t(n));
 for(int i=0;i<n;++i)batch.push_back({noesql::Mutation::Kind::Put,key(i),std::string(120,'x')});
 auto at=Clock::now();store.transact(session,batch);double batchms=elapsed(at);
 std::vector<double> lookups;lookups.reserve(size_t(n));
 for(int i=0;i<n;++i){at=Clock::now();auto value=store.get(session,key(i));if(!value||value->size()!=120)throw std::runtime_error("noesql lookup incorrect");lookups.push_back(elapsed(at));}
 at=Clock::now();for(int i=0;i<20;++i)store.transact(session,{{noesql::Mutation::Kind::Put,"single"+std::to_string(i),"one"}});
 return {batchms,elapsed(at),pct(lookups,50),pct(lookups,95),pct(lookups,99)};
}
static Result run_sqlite(const std::string &path,int n){
 Sqlite s(path);s.exec("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE kv (k TEXT PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID;");
 sqlite3_stmt *insert=nullptr,*select=nullptr;
 check(sqlite3_prepare_v2(s.db,"INSERT INTO kv(k,v) VALUES(?1,?2)",-1,&insert,nullptr),s.db);
 check(sqlite3_prepare_v2(s.db,"SELECT v FROM kv WHERE k=?1",-1,&select,nullptr),s.db);
 auto put=[&](const std::string &k,const std::string &v){sqlite3_reset(insert);sqlite3_clear_bindings(insert);check(sqlite3_bind_text(insert,1,k.c_str(),int(k.size()),SQLITE_TRANSIENT),s.db);check(sqlite3_bind_blob(insert,2,v.data(),int(v.size()),SQLITE_TRANSIENT),s.db);check(sqlite3_step(insert),s.db);};
 auto at=Clock::now();s.exec("BEGIN IMMEDIATE");for(int i=0;i<n;++i)put(key(i),std::string(120,'x'));s.exec("COMMIT");double batchms=elapsed(at);
 std::vector<double> lookups;lookups.reserve(size_t(n));
 for(int i=0;i<n;++i){std::string k=key(i);at=Clock::now();sqlite3_reset(select);sqlite3_clear_bindings(select);check(sqlite3_bind_text(select,1,k.c_str(),int(k.size()),SQLITE_TRANSIENT),s.db);check(sqlite3_step(select),s.db);if(sqlite3_column_bytes(select,0)!=120)throw std::runtime_error("sqlite lookup incorrect");lookups.push_back(elapsed(at));}
 // Close the SELECT cursor before measuring durable independent transactions.
 check(sqlite3_reset(select),s.db);
 if(sqlite3_get_autocommit(s.db)!=1)throw std::runtime_error("SQLite read transaction still active");
 at=Clock::now();
 for(int i=0;i<20;++i){
   s.exec("BEGIN IMMEDIATE");
   put("single"+std::to_string(i),"one");
   s.exec("COMMIT");
 }
 double singlems=elapsed(at);
 sqlite3_finalize(insert);sqlite3_finalize(select);
 return {batchms,singlems,pct(lookups,50),pct(lookups,95),pct(lookups,99)};
}
static void print(const char *name,const Result &r,int n){
 std::cout<<std::fixed<<std::setprecision(4)<<"{\"engine\":\""<<name<<"\",\"rows\":"<<n
 <<",\"batch_rows_per_second\":"<<1000.0*n/r.batch_ms
 <<",\"single_commits_per_second\":"<<20000.0/r.single_ms
 <<",\"lookup_p50_ms\":"<<r.p50<<",\"lookup_p95_ms\":"<<r.p95<<",\"lookup_p99_ms\":"<<r.p99<<"}\n";
}
int main(int argc,char **argv){
 int n=argc>1?std::stoi(argv[1]):500;if(n<1||n>5000){std::cerr<<"count must be 1..5000\n";return 2;}
 char templ[]="/tmp/noesql-compare-XXXXXX";char *d=mkdtemp(templ);if(!d)throw std::runtime_error("mkdtemp failed");std::string dir=d;
 try{auto a=run_noesql(dir+"/native.nqd",n);auto b=run_sqlite(dir+"/sqlite.db",n);print("noesql-native",a,n);print("sqlite-wal-full",b,n);}catch(...){std::filesystem::remove_all(dir);throw;}
 std::filesystem::remove_all(dir);
}

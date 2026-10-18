#include "noesql_page_store.h"
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <stdexcept>
#include <cstdlib>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <string>
#include <vector>
using Clock=std::chrono::steady_clock;
const std::string pass="benchmark password deliberately long 2026";
static double ms(Clock::time_point since){return std::chrono::duration<double,std::milli>(Clock::now()-since).count();}
int main(int argc,char**argv){
 int n=argc>1?std::stoi(argv[1]):500;
 if(n<1||n>5000){std::cerr<<"benchmark count must be 1..5000\n";return 1;}
 char dirTemplate[]="/tmp/noesql-bench-XXXXXX";
 char* ptr=mkdtemp(dirTemplate);if(!ptr)return 1;std::string dir=ptr;
 try{
  noesql::Store store(dir+"/bench.nqd");auto before=Clock::now();store.initialize(pass);double init=ms(before);
  std::vector<noesql::Mutation> rows;rows.reserve(n);
  for(int i=0;i<n;++i){char key[32];std::snprintf(key,sizeof(key),"bench%08d",i);rows.push_back({noesql::Mutation::Kind::Put,key,std::string(120,'x')});}
  before=Clock::now();store.transact("admin",pass,rows);double commit=ms(before);
  auto session=store.login("admin",pass);
  std::vector<double> latency;latency.reserve(n);
  for(int i=0;i<n;++i){auto at=Clock::now();auto got=store.get(session,rows[size_t(i)].key);if(!got||got->size()!=120)throw std::runtime_error("bad lookup");latency.push_back(ms(at));}
  std::sort(latency.begin(),latency.end());double p50=latency[latency.size()/2],p95=latency[(latency.size()-1)*95/100],p99=latency[(latency.size()-1)*99/100];
  // Single-operation committed writes include full image WAL and checkpoint.
  before=Clock::now();
  constexpr int independentCommits=20;
  for(int j=0;j<independentCommits;++j){char key[32];std::snprintf(key,sizeof(key),"single%08d",j);
    store.transact(session,{{noesql::Mutation::Kind::Put,key,"one"}});
  }
  double singleCommitMs=ms(before);
  auto bytes=std::filesystem::file_size(dir+"/bench.nqd");
  std::cout<<std::fixed<<std::setprecision(3)<<"{\"engine\":\"native-page-v1\",\"rows\":"<<n<<",\"init_ms\":"<<init<<",\"batch_commit_ms\":"<<commit<<",\"batch_rows_per_second\":"<<(n*1000/commit)<<",\"single_commit_rows_per_second\":"<<(independentCommits*1000/singleCommitMs)<<",\"lookup_p50_ms\":"<<p50<<",\"lookup_p95_ms\":"<<p95<<",\"lookup_p99_ms\":"<<p99<<",\"file_bytes\":"<<bytes<<"}\n";
 }catch(...){std::filesystem::remove_all(dir);throw;}
 std::filesystem::remove_all(dir);
}

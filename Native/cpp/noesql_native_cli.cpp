#include "noesql_page_store.h"
#include <cstdlib>
#include <iostream>
#include <stdexcept>
#include <string>
int main(int argc,char** argv){
  try{
    if(argc<3)throw std::runtime_error("usage: noesql-native <init|put|get|delete|scan|useradd|verify> path [user args]; set NOESQL_PASSWORD");
    noesql::Store s(argv[2]);std::string cmd=argv[1];const char* secret=std::getenv("NOESQL_PASSWORD");
    if(cmd=="verify"){std::cout<<"generation="<<s.verify()<<'\n';return 0;}
    if(!secret)throw std::runtime_error("NOESQL_PASSWORD required (never pass secrets in command line)");
    if(cmd=="init"&&argc==3)s.initialize(secret);
    else if(cmd=="useradd"&&argc==5){const char* np=std::getenv("NOESQL_NEW_PASSWORD");if(!np)throw std::runtime_error("NOESQL_NEW_PASSWORD required");s.add_user(secret,argv[3],np,std::string(argv[4])=="writer");}
    else if(cmd=="put"&&argc==6)s.transact(argv[3],secret,{{noesql::Mutation::Kind::Put,argv[4],argv[5]}});
    else if(cmd=="delete"&&argc==5)s.transact(argv[3],secret,{{noesql::Mutation::Kind::Delete,argv[4],""}});
    else if(cmd=="get"&&argc==5){auto v=s.get(argv[3],secret,argv[4]);if(!v)return 2;std::cout<<*v<<'\n';}
    else if(cmd=="scan"&&(argc==4||argc==5)){for(const auto& [k,v]:s.scan(argv[3],secret,argc==5?argv[4]:""))std::cout<<k<<'\t'<<v<<'\n';}
    else throw std::runtime_error("invalid arguments");
    return 0;
  }catch(const std::exception& e){std::cerr<<"NOESQL: "<<e.what()<<'\n';return 1;}
}

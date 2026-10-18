#include "noesql_page_store.h"
#include <openssl/evp.h>
#include <openssl/crypto.h>
#include <openssl/rand.h>
#include <algorithm>
#include <array>
#include <chrono>
#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <filesystem>
#include <functional>
#include <cstdlib>
#include <limits>
#include <stdexcept>
#include <sys/file.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

namespace noesql {
namespace {
using Bytes=std::vector<uint8_t>;
using Map=std::map<std::string,std::string>;
constexpr size_t H=48;
constexpr char kMagic[]="NQPG0001";
constexpr char kWalMagic[]="NQWL0001";
const std::string kAuth="\x01" "user/";
[[noreturn]] void fail(const char* what){throw std::runtime_error(what);}
void check(bool yes,const char* what){if(!yes)fail(what);}
void u16(Bytes& b,size_t at,uint16_t x){b.at(at)=uint8_t(x);b.at(at+1)=uint8_t(x>>8);}
void u32(Bytes& b,size_t at,uint32_t x){for(int i=0;i<4;++i)b.at(at+i)=uint8_t(x>>(8*i));}
void u64(Bytes& b,size_t at,uint64_t x){for(int i=0;i<8;++i)b.at(at+i)=uint8_t(x>>(8*i));}
uint16_t r16(const Bytes& b,size_t at){check(at+2<=b.size(),"truncated u16");return uint16_t(b[at]|uint16_t(b[at+1])<<8);}
uint32_t r32(const Bytes& b,size_t at){check(at+4<=b.size(),"truncated u32");uint32_t x=0;for(int i=0;i<4;++i)x|=uint32_t(b[at+i])<<(8*i);return x;}
uint64_t r64(const Bytes& b,size_t at){check(at+8<=b.size(),"truncated u64");uint64_t x=0;for(int i=0;i<8;++i)x|=uint64_t(b[at+i])<<(8*i);return x;}
std::array<uint8_t,32> sha(const uint8_t* p,size_t n){std::array<uint8_t,32> out{};unsigned len=0;check(EVP_Digest(p,n,out.data(),&len,EVP_sha256(),nullptr)==1&&len==32,"sha256 unavailable");return out;}
void seal(Bytes& b,size_t at){std::fill(b.begin()+at+8,b.begin()+at+40,0);auto hash=sha(b.data()+at,kPageSize);std::copy(hash.begin(),hash.end(),b.begin()+at+8);}
void validate_page(const Bytes& b,size_t at){check(at+kPageSize<=b.size(),"truncated page");Bytes p(b.begin()+at,b.begin()+at+kPageSize);auto existing=std::array<uint8_t,32>{};std::copy(p.begin()+8,p.begin()+40,existing.begin());std::fill(p.begin()+8,p.begin()+40,0);check(CRYPTO_memcmp(existing.data(),sha(p.data(),p.size()).data(),32)==0,"page checksum mismatch");}
struct Fd {int fd=-1;Fd()=default;explicit Fd(int f):fd(f){} Fd(const Fd&)=delete;Fd& operator=(const Fd&)=delete;~Fd(){if(fd>=0)::close(fd);}int get()const{return fd;}};
void write_all(int fd,const uint8_t* p,size_t n){while(n){auto w=::write(fd,p,n);if(w<0&&errno==EINTR)continue;check(w>0,"write failed");p+=w;n-=size_t(w);}}
Bytes read_file(const std::string& path,uint64_t max,bool optional=false){
    int f=::open(path.c_str(),O_RDONLY|O_CLOEXEC|O_NOFOLLOW);
    if(f<0){if(errno==ENOENT&&optional)return {};fail("open failed or unsafe symlink");}Fd fd(f);
    struct stat st{};check(::fstat(fd.get(),&st)==0&&S_ISREG(st.st_mode)&&st.st_nlink==1&&st.st_size>=0&&uint64_t(st.st_size)<=max,"unsafe file or size limit");
    Bytes b(size_t(st.st_size));size_t at=0;
    while(at<b.size()){
        ssize_t n=::read(fd.get(),b.data()+at,b.size()-at);
        if(n<0&&errno==EINTR)continue;
        check(n>0,"file truncated during read");at+=size_t(n);
    }
    struct stat after{};
    check(::fstat(fd.get(),&after)==0&&after.st_size==st.st_size&&after.st_ino==st.st_ino&&
          after.st_mtim.tv_sec==st.st_mtim.tv_sec&&after.st_mtim.tv_nsec==st.st_mtim.tv_nsec,
          "file changed while reading");
    return b;
}
void fsync_dir(const std::string& path){auto dir=std::filesystem::path(path).parent_path();if(dir.empty())dir=".";Fd fd(::open(dir.c_str(),O_RDONLY|O_DIRECTORY|O_CLOEXEC|O_NOFOLLOW));check(fd.get()>=0&&::fsync(fd.get())==0,"directory fsync failed");}
void atomic_file(const std::string& target,const Bytes& bytes){
    std::string tmp=target+".tmp."+std::to_string(::getpid())+"."+std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    Fd fd(::open(tmp.c_str(),O_CREAT|O_EXCL|O_WRONLY|O_CLOEXEC|O_NOFOLLOW,0600));check(fd.get()>=0,"temp create failed");
    try{write_all(fd.get(),bytes.data(),bytes.size());check(::fsync(fd.get())==0,"temp sync failed");
        struct stat st{};if(::lstat(target.c_str(),&st)==0)check(S_ISREG(st.st_mode)&&st.st_nlink==1,"unsafe checkpoint target");else check(errno==ENOENT,"checkpoint stat failure");
        check(::rename(tmp.c_str(),target.c_str())==0,"checkpoint rename failed");fsync_dir(target);
    }catch(...){::unlink(tmp.c_str());throw;}
}
class Lock {Fd f_;public:Lock(const std::string& path,bool exclusive):f_(::open((path+".lck").c_str(),O_CREAT|O_RDWR|O_NOFOLLOW|O_CLOEXEC,0600)){
    check(f_.get()>=0,"lock open failed");struct stat st{};check(::fstat(f_.get(),&st)==0&&S_ISREG(st.st_mode)&&st.st_nlink==1,"unsafe lock file");
    int how=exclusive?LOCK_EX:LOCK_SH;while(::flock(f_.get(),how)<0){if(errno==EINTR)continue;fail("lock failed");}
}};
void reserved(const std::string& s){check(!s.empty()&&s[0]!='\x01',"reserved system key");}
void key_valid(const std::string& s,const Limits& l){reserved(s);check(s.size()<=l.max_key_bytes,"key too large");}
void username_valid(const std::string& s){
    check(!s.empty()&&s.size()<=64,"invalid username");
    for(unsigned char c:s)check((c>='a'&&c<='z')||(c>='A'&&c<='Z')||(c>='0'&&c<='9')||c=='_'||c=='-',"invalid username");
}
void password_valid(const std::string& s){check(s.size()>=12&&s.size()<=1024,"password length must be 12..1024 bytes");}
std::string hash_password(const std::string& password,const std::array<uint8_t,16>& salt){
    std::array<unsigned char,32> key{};
    // OpenSSL scrypt memory bound 32 MiB, N=2^15, r=8, p=1.
    check(EVP_PBE_scrypt(password.data(),password.size(),salt.data(),salt.size(),1<<15,8,1,64ull*1024*1024,key.data(),key.size())==1,"scrypt unavailable");
    return std::string(reinterpret_cast<char*>(key.data()),key.size());
}
std::string credential(const std::string& password,bool write){password_valid(password);std::array<uint8_t,16> salt{};check(RAND_bytes(salt.data(),salt.size())==1,"random failure");
    auto hash=hash_password(password,salt);std::string result(1,write?'w':'r');result.append(reinterpret_cast<const char*>(salt.data()),salt.size());result+=hash;return result;
}
void authenticate(const Map& m,const std::string& name,const std::string& password,bool write){
    username_valid(name);
    auto it=m.find(kAuth+name);check(it!=m.end(),"access denied");auto& rec=it->second;check(rec.size()==49,"corrupt credential record");
    std::array<uint8_t,16> salt{};std::copy(rec.begin()+1,rec.begin()+17,salt.begin());
    auto computed=hash_password(password,salt);
    check(CRYPTO_memcmp(computed.data(),rec.data()+17,32)==0 && (!write||rec[0]=='w'||rec[0]=='a'),"access denied");
}
void authorize_session(const Map& map,const std::string& name,const std::array<uint8_t,32>& expected,bool write){
    auto it=map.find(kAuth+name);
    check(it!=map.end()&&it->second.size()==49,"session revoked");
    auto dig=sha(reinterpret_cast<const uint8_t*>(it->second.data()),it->second.size());
    check(CRYPTO_memcmp(dig.data(),expected.data(),32)==0 && (!write||it->second[0]=='w'||it->second[0]=='a'),"session revoked or insufficient permission");
}
std::string page_key(const Bytes& image,size_t& pos,size_t end);
// Read-only on-disk B+ tree reader. Under shared flock each page remains stable.
// It verifies checksums for every page traversed; full integrity uses verify().
class PageReader {
    Fd fd_;
    uint32_t pages_=0;
    uint32_t root_=0;
    Bytes page(uint32_t id)const{
        check(id<pages_,"physical page outside database");
        Bytes b(kPageSize);size_t at=0;
        while(at<b.size()){
            ssize_t n=::pread(fd_.get(),b.data()+at,b.size()-at,off_t(uint64_t(id)*kPageSize+at));
            if(n<0&&errno==EINTR)continue;
            check(n>0,"short physical page");at+=size_t(n);
        }
        validate_page(b,0);return b;
    }
public:
    PageReader(const std::string& path,const Limits& lim):fd_(::open(path.c_str(),O_RDONLY|O_CLOEXEC|O_NOFOLLOW)){
        check(fd_.get()>=0,"page file open failed");
        struct stat st{};
        check(::fstat(fd_.get(),&st)==0&&S_ISREG(st.st_mode)&&st.st_nlink==1&&st.st_size>=off_t(2*kPageSize)&&
              uint64_t(st.st_size)%kPageSize==0&&uint64_t(st.st_size)<=lim.max_file_bytes,"unsafe page file");
        pages_=uint32_t(uint64_t(st.st_size)/kPageSize);
        auto header=page(0);
        check(std::memcmp(header.data(),kMagic,8)==0&&r32(header,40)==1&&r32(header,44)==kPageSize,"invalid native page header");
        root_=r32(header,56);check(root_>0&&root_<pages_,"invalid index root");
    }
    std::optional<std::string> find(const std::string& target)const{
        uint32_t id=root_;
        for(unsigned depth=0;depth<16;++depth){
            auto b=page(id);size_t pos=H;uint16_t count=r16(b,2);
            check(count>0,"empty physical page");
            if(b[0]==1){
                std::string prev;
                for(uint16_t i=0;i<count;++i){
                    auto key=page_key(b,pos,kPageSize);uint32_t len=r32(b,pos);pos+=4;
                    check(pos+len<=kPageSize,"invalid physical leaf length");
                    check(i==0||prev<key,"unsorted physical leaf");
                    if(key==target)return std::string(reinterpret_cast<const char*>(b.data()+pos),len);
                    if(key>target)return std::nullopt;
                    prev=key;pos+=len;
                }
                return std::nullopt;
            }
            check(b[0]==2,"bad internal page type");
            uint32_t child=r32(b,pos);pos+=4;
            std::string prev;
            for(uint16_t i=1;i<count;++i){
                auto sep=page_key(b,pos,kPageSize);auto next=r32(b,pos);pos+=4;
                check(i==1||prev<sep,"unsorted internal separators");
                if(target<sep)break;
                child=next;prev=sep;
            }
            check(child>0&&child<pages_,"invalid child pointer");id=child;
        }
        fail("physical index depth limit");
    }
};

struct Parsed {Map map;uint64_t generation=0;uint32_t root=0;uint32_t count=0;};
std::string page_key(const Bytes& image,size_t& pos,size_t end){uint16_t len=r16(image,pos);pos+=2;check(len>0&&pos+len<=end,"bad page key");auto k=std::string(reinterpret_cast<const char*>(image.data()+pos),len);pos+=len;return k;}
// Persisted B+ tree lookup: internal separators choose children, leaves hold rows.
// The v1 loader validates all page checksums first; this read traverses pages.
std::optional<std::string> index_get(const Bytes& image,uint32_t root,const std::string& target){
    uint32_t page=root;
    for(size_t steps=0;steps<image.size()/kPageSize;++steps){
        size_t at=size_t(page)*kPageSize;
        check(at+kPageSize<=image.size(),"index page outside database");
        uint8_t type=image[at];uint16_t count=r16(image,at+2);size_t pos=at+H;
        if(type==1){
            for(uint16_t i=0;i<count;++i){
                auto key=page_key(image,pos,at+kPageSize);
                uint32_t n=r32(image,pos);pos+=4;
                check(pos+n<=at+kPageSize,"invalid indexed row");
                if(key==target)return std::string(reinterpret_cast<const char*>(image.data()+pos),n);
                if(key>target)return std::nullopt;
                pos+=n;
            }
            return std::nullopt;
        }
        check(type==2,"invalid index type");
        uint32_t child=r32(image,pos);pos+=4;
        for(uint16_t i=1;i<count;++i){
            auto sep=page_key(image,pos,at+kPageSize);
            uint32_t next=r32(image,pos);pos+=4;
            if(target<sep)break;
            child=next;
        }
        page=child;
    }
    fail("index cycle");
}

Parsed decode(const Bytes& image,const Limits& lim){
    check(image.size()>=kPageSize&&image.size()%kPageSize==0&&image.size()<=lim.max_file_bytes,"invalid page file length");
    uint32_t pages=uint32_t(image.size()/kPageSize);
    check(std::memcmp(image.data(),kMagic,8)==0,"incompatible native page format");
    validate_page(image,0);check(r32(image,40)==1&&r32(image,44)==kPageSize,"unsupported page version");
    Parsed p;p.generation=r64(image,48);p.root=r32(image,56);p.count=r32(image,60);
    check(p.root>0&&p.root<pages,"invalid root page");
    // DFS traverses the persisted physical tree, validating ordering, fanout,
    // checksum, reachability and duplicate/cyclic references.
    std::vector<uint8_t> seen(pages,0);uint32_t visited=0;
    std::function<std::pair<std::string,std::string>(uint32_t,unsigned)> walk=[&](uint32_t id,unsigned depth)->std::pair<std::string,std::string>{
        check(depth<=16,"physical index depth limit");
        check(id>0&&id<pages&&!seen[id],"index cycle or invalid page");seen[id]=1;++visited;
        size_t at=size_t(id)*kPageSize;validate_page(image,at);uint8_t type=image[at];uint16_t count=r16(image,at+2);size_t pos=at+H;
        check(count>0,"empty index page");
        std::string first,last;
        if(type==1){
            for(uint16_t i=0;i<count;++i){
                std::string key=page_key(image,pos,at+kPageSize);uint32_t length=r32(image,pos);pos+=4;
                check(length<=lim.max_value_bytes&&pos+length<=at+kPageSize&&key.size()<=lim.max_key_bytes,"invalid leaf entry");
                std::string val(reinterpret_cast<const char*>(image.data()+pos),length);pos+=length;
                check(last.empty()||last<key,"unsorted leaf entries");if(i==0)first=key;last=key;
                check(p.map.emplace(std::move(key),std::move(val)).second,"duplicate leaf key");
            }
        }else if(type==2){
            uint32_t child=r32(image,pos);pos+=4;auto bounds=walk(child,depth+1);first=bounds.first;last=bounds.second;
            for(uint16_t i=1;i<count;++i){
                std::string separator=page_key(image,pos,at+kPageSize);child=r32(image,pos);pos+=4;
                auto next=walk(child,depth+1);check(last<next.first&&separator==next.first,"invalid branch separator");last=next.second;
            }
        }else fail("unknown page type");
        return {first,last};
    };
    walk(p.root,0);check(visited==pages-1&&p.map.size()==p.count,"page reachability or row count mismatch");return p;
}
struct Node {uint32_t id;std::string first;};
Bytes encode(const Map& map,uint64_t generation,const Limits& lim){
    check(map.size()<=std::numeric_limits<uint32_t>::max(),"too many rows");
    Bytes image(kPageSize,0);std::copy(kMagic,kMagic+8,image.begin());u32(image,40,1);u32(image,44,kPageSize);u64(image,48,generation);u32(image,60,uint32_t(map.size()));
    check(!map.empty(),"empty index must include bootstrap admin");
    std::vector<Node> layer;
    auto it=map.begin();while(it!=map.end()){
        Bytes page(kPageSize,0);page[0]=1;size_t pos=H;uint16_t n=0;std::string first;
        while(it!=map.end()){
            const auto& [k,v]=*it;check(k.size()<=lim.max_key_bytes&&v.size()<=lim.max_value_bytes,"row size limit");
            size_t need=2+k.size()+4+v.size();check(need<=kPageSize-H,"row too large for page");
            if(n&&pos+need>kPageSize)break;
            if(n==0)first=k;
            u16(page,pos,uint16_t(k.size()));pos+=2;std::copy(k.begin(),k.end(),page.begin()+pos);pos+=k.size();
            u32(page,pos,uint32_t(v.size()));pos+=4;std::copy(v.begin(),v.end(),page.begin()+pos);pos+=v.size();++it;++n;
        }
        u16(page,2,n);seal(page,0);uint32_t id=uint32_t(image.size()/kPageSize);image.insert(image.end(),page.begin(),page.end());layer.push_back({id,first});
        check(image.size()<=lim.max_file_bytes,"database file size limit");
    }
    while(layer.size()>1){
        std::vector<Node> upper;size_t i=0;
        while(i<layer.size()){
            Bytes page(kPageSize,0);page[0]=2;size_t pos=H;uint16_t n=1;
            Node start=layer[i++];u32(page,pos,start.id);pos+=4;
            while(i<layer.size()){
                auto& e=layer[i];size_t need=2+e.first.size()+4;
                if(pos+need>kPageSize)break;
                u16(page,pos,uint16_t(e.first.size()));pos+=2;
                std::copy(e.first.begin(),e.first.end(),page.begin()+pos);pos+=e.first.size();u32(page,pos,e.id);pos+=4;++i;++n;
            }
            u16(page,2,n);seal(page,0);uint32_t id=uint32_t(image.size()/kPageSize);image.insert(image.end(),page.begin(),page.end());upper.push_back({id,start.first});
            check(image.size()<=lim.max_file_bytes,"database file size limit");
        }
        layer=std::move(upper);
    }
    u32(image,56,layer.front().id);seal(image,0);return image;
}
Bytes build_wal(const Bytes& image,uint64_t generation){
    Bytes b(64,0);std::copy(kWalMagic,kWalMagic+8,b.begin());u64(b,8,generation);u64(b,16,image.size());
    auto digest=sha(image.data(),image.size());std::copy(digest.begin(),digest.end(),b.begin()+24);
    b.insert(b.end(),image.begin(),image.end());
    const char marker[]="COMMIT01";b.insert(b.end(),marker,marker+8);
    auto final=sha(b.data(),b.size());b.insert(b.end(),final.begin(),final.end());return b;
}
std::optional<Bytes> parse_wal(const Bytes& b,const Limits& limits){
    if(b.empty())return std::nullopt;
    // An incomplete WAL is uncommitted. Never apply a truncated tail.
    if(b.size()<64+8+32)return std::nullopt;
    check(std::memcmp(b.data(),kWalMagic,8)==0,"WAL header corruption");
    uint64_t size=r64(b,16);check(size<=limits.max_file_bytes,"WAL image size limit");
    if(size>std::numeric_limits<size_t>::max()-104||b.size()<size+104)return std::nullopt;
    check(b.size()==size+104,"WAL trailing data");
    check(std::memcmp(b.data()+64+size,"COMMIT01",8)==0,"WAL commit marker corruption");
    auto digest=sha(b.data()+64,size);check(CRYPTO_memcmp(digest.data(),b.data()+24,32)==0,"WAL image checksum mismatch");
    auto final=sha(b.data(),b.size()-32);check(CRYPTO_memcmp(final.data(),b.data()+b.size()-32,32)==0,"WAL commit checksum mismatch");
    return Bytes(b.begin()+64,b.begin()+64+size);
}
struct State {Parsed parsed;Bytes image;bool wal=false;};
State load(const std::string& path,const Limits& limits,bool allow_missing=false){
    auto main=read_file(path,limits.max_file_bytes,true);
    auto journal=read_file(path+".wal",limits.max_file_bytes+104,true);
    std::optional<Bytes> wal=parse_wal(journal,limits);
    State s;
    std::optional<Parsed> walParsed;
    if(wal){
        walParsed=decode(*wal,limits);
        check(r64(journal,8)==walParsed->generation,"WAL generation mismatch");
    }
    if(!main.empty()){
        try{s.parsed=decode(main,limits);s.image=std::move(main);}
        catch(const std::exception&){
            // A fully committed, independently checked WAL can recover a torn
            // checkpoint even when its base file cannot be parsed.
            if(!wal)throw;
        }
    }
    if(wal && (s.image.empty() || walParsed->generation>=s.parsed.generation)){
        s.parsed=std::move(*walParsed);s.image=std::move(*wal);s.wal=true;
    }
    if(!allow_missing)check(!s.image.empty(),"database not initialized");
    return s;
}
void commit(const std::string& path,const Bytes& image,uint64_t generation){
    Bytes wal=build_wal(image,generation);
    // Any old WAL must have been checkpointed under writer lock first.
    auto previous=read_file(path+".wal",wal.size(),true);check(previous.empty(),"pending WAL checkpoint required");
    {
        Fd file(::open((path+".wal").c_str(),O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600));check(file.get()>=0,"WAL create failed");
        write_all(file.get(),wal.data(),wal.size());check(::fsync(file.get())==0,"WAL fsync failed");
    }
    fsync_dir(path);
#ifdef NOESQL_TEST_FAULT_INJECTION
    if(std::getenv("NOESQL_TEST_CRASH_AFTER_WAL")!=nullptr)::_exit(93);
#endif
    atomic_file(path,image);
#ifdef NOESQL_TEST_FAULT_INJECTION
    if(std::getenv("NOESQL_TEST_CRASH_AFTER_CHECKPOINT")!=nullptr)::_exit(94);
#endif
    check(::unlink((path+".wal").c_str())==0,"WAL unlink failed");fsync_dir(path);
}
void recover(const std::string& path,const Limits& limits,State& state){
    // Always checkpoint or clear complete stale journal while exclusively locked.
    auto journal=read_file(path+".wal",limits.max_file_bytes+104,true);
    if(journal.empty())return;
    auto wal=parse_wal(journal,limits);
    if(wal && state.wal)atomic_file(path,state.image);
    check(::unlink((path+".wal").c_str())==0,"WAL cleanup failed");fsync_dir(path);
}
uint64_t elapsed_ms(std::chrono::steady_clock::time_point since){return uint64_t(std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now()-since).count());}
} // namespace
Store::Store(std::string path,Limits limits):path_(std::move(path)),limits_(limits){
    check(std::filesystem::path(path_).extension()==".nqd","native store requires .nqd file");
    check(limits_.max_file_bytes>=2*kPageSize && limits_.max_file_bytes<=1024ull*1024*1024,"invalid file quota");
    check(limits_.max_key_bytes>0&&limits_.max_key_bytes<=128&&limits_.max_value_bytes>0&&limits_.max_value_bytes<=1024,"invalid row quota");
}
void Store::initialize(const std::string& admin_password){
    Lock lock(path_,true);auto state=load(path_,limits_,true);check(state.image.empty(),"database already initialized");
    Map m;m[kAuth+"admin"]=credential(admin_password,true);auto image=encode(m,1,limits_);commit(path_,image,1);
}
void Store::add_user(const std::string& admin_password,const std::string& name,const std::string& password,bool can_write){
    username_valid(name);check(name!="admin","invalid username");
    Lock lock(path_,true);auto state=load(path_,limits_);recover(path_,limits_,state);
    authenticate(state.parsed.map,"admin",admin_password,true);
    auto m=state.parsed.map;check(m.find(kAuth+name)==m.end(),"user already exists");m[kAuth+name]=credential(password,can_write);
    auto image=encode(m,state.parsed.generation+1,limits_);commit(path_,image,state.parsed.generation+1);
}
std::optional<std::string> Store::get(const std::string& name,const std::string& password,const std::string& key)const{
    key_valid(key,limits_);Lock lock(path_,false);auto state=load(path_,limits_);authenticate(state.parsed.map,name,password,false);
    return index_get(state.image,state.parsed.root,key);
}
std::vector<std::pair<std::string,std::string>> Store::scan(const std::string& name,const std::string& password,const std::string& prefix)const{
    if(!prefix.empty())key_valid(prefix,limits_);
    Lock lock(path_,false);auto state=load(path_,limits_);authenticate(state.parsed.map,name,password,false);
    std::vector<std::pair<std::string,std::string>> out;
    for(auto it=state.parsed.map.lower_bound(prefix);it!=state.parsed.map.end();++it){
        if(it->first[0]=='\x01')continue;
        if(!prefix.empty()&&it->first.compare(0,prefix.size(),prefix)!=0)break;
        check(out.size()<limits_.max_scan_rows,"scan row limit exceeded");out.push_back(*it);
    }return out;
}
void Store::transact(const std::string& name,const std::string& password,const std::vector<Mutation>& changes)const{
    check(!changes.empty()&&changes.size()<=limits_.max_mutations,"mutation count limit exceeded");
    auto started=std::chrono::steady_clock::now();Lock lock(path_,true);auto state=load(path_,limits_);
    recover(path_,limits_,state);authenticate(state.parsed.map,name,password,true);
    Map m=state.parsed.map;
    for(const auto& change:changes){
        check(elapsed_ms(started)<=limits_.max_transaction_ms,"transaction deadline exceeded");key_valid(change.key,limits_);
        if(change.kind==Mutation::Kind::Put){check(change.value.size()<=limits_.max_value_bytes,"value too large");m[change.key]=change.value;}
        else if(change.kind==Mutation::Kind::Delete)m.erase(change.key);
        else fail("invalid mutation kind");
    }
    check(elapsed_ms(started)<=limits_.max_transaction_ms,"transaction deadline exceeded");
    auto image=encode(m,state.parsed.generation+1,limits_);
    check(elapsed_ms(started)<=limits_.max_transaction_ms,"transaction deadline exceeded");commit(path_,image,state.parsed.generation+1);
}
uint64_t Store::verify()const{Lock lock(path_,false);auto s=load(path_,limits_);return s.parsed.generation;}
} // namespace noesql

namespace noesql {
Session Store::login(const std::string& name,const std::string& password)const{
    Lock lock(path_,false);auto state=load(path_,limits_);authenticate(state.parsed.map,name,password,false);
    const auto& rec=state.parsed.map.at(kAuth+name);
    return Session(name,sha(reinterpret_cast<const uint8_t*>(rec.data()),rec.size()));
}
std::optional<std::string> Store::get(const Session& session,const std::string& key)const{
    key_valid(key,limits_);Lock lock(path_,false);
    struct stat wal{};
    if(::lstat((path_+".wal").c_str(),&wal)==0){
        // A committed but not checkpointed WAL is the authoritative image.
        auto s=load(path_,limits_);
        authorize_session(s.parsed.map,session.name_,session.verifier_,false);
        return index_get(s.image,s.parsed.root,key);
    }
    check(errno==ENOENT,"WAL status unavailable");
    PageReader tree(path_,limits_);
    auto rec=tree.find(kAuth+session.name_);
    check(rec.has_value()&&rec->size()==49,"session revoked");
    auto actual=sha(reinterpret_cast<const uint8_t*>(rec->data()),rec->size());
    check(CRYPTO_memcmp(actual.data(),session.verifier_.data(),32)==0,"session revoked");
    return tree.find(key);
}
std::vector<std::pair<std::string,std::string>> Store::scan(const Session& session,const std::string& prefix)const{
    if(!prefix.empty())key_valid(prefix,limits_);
    Lock lock(path_,false);auto s=load(path_,limits_);
    authorize_session(s.parsed.map,session.name_,session.verifier_,false);
    std::vector<std::pair<std::string,std::string>> out;
    for(auto it=s.parsed.map.lower_bound(prefix);it!=s.parsed.map.end();++it){
        if(it->first[0]=='\x01')continue;
        if(!prefix.empty()&&it->first.compare(0,prefix.size(),prefix)!=0)break;
        check(out.size()<limits_.max_scan_rows,"scan row limit exceeded");out.push_back(*it);
    }
    return out;
}
void Store::transact(const Session& session,const std::vector<Mutation>& changes)const{
    check(!changes.empty()&&changes.size()<=limits_.max_mutations,"mutation count limit exceeded");
    auto started=std::chrono::steady_clock::now();Lock lock(path_,true);auto state=load(path_,limits_);
    recover(path_,limits_,state);authorize_session(state.parsed.map,session.name_,session.verifier_,true);
    Map m=state.parsed.map;
    for(const auto& change:changes){
        check(elapsed_ms(started)<=limits_.max_transaction_ms,"transaction deadline exceeded");key_valid(change.key,limits_);
        if(change.kind==Mutation::Kind::Put){check(change.value.size()<=limits_.max_value_bytes,"value too large");m[change.key]=change.value;}
        else if(change.kind==Mutation::Kind::Delete)m.erase(change.key);
        else fail("invalid mutation kind");
    }
    check(elapsed_ms(started)<=limits_.max_transaction_ms,"transaction deadline exceeded");
    auto image=encode(m,state.parsed.generation+1,limits_);
    check(elapsed_ms(started)<=limits_.max_transaction_ms,"transaction deadline exceeded");
    commit(path_,image,state.parsed.generation+1);
}
void Store::remove_user(const std::string& admin_password,const std::string& name){
    username_valid(name);check(name!="admin","cannot remove admin");Lock lock(path_,true);
    auto state=load(path_,limits_);recover(path_,limits_,state);authenticate(state.parsed.map,"admin",admin_password,true);
    Map m=state.parsed.map;check(m.erase(kAuth+name)==1,"unknown user");
    auto image=encode(m,state.parsed.generation+1,limits_);commit(path_,image,state.parsed.generation+1);
}
}

#include "../zig/noesql_zig_pages.h"
#include <stdio.h>
#include <stdint.h>
#include <string.h>
extern int64_t noeZigRelationalSmoke(const char *);
static int verify(const char *path, int checkpoint) {
    void *handle = NULL;
    if (noesql_zig_page_open(NOESQL_ZIG_PAGE_ABI_VERSION,path,&handle)!=0 || !handle) return 21;
    uint8_t page[NOESQL_ZIG_PAGE_BYTES];
    int ok = noesql_zig_page_get(handle,3u,page,sizeof(page))==0 && memcmp(page,"NROW",4)==0;
    if (ok && checkpoint) ok = noesql_zig_page_checkpoint(handle)==0;
    noesql_zig_page_close(handle);
    if (!ok) return 22;
    if (checkpoint) {
        FILE *f=fopen(path,"rb");
        if (!f) return 23;
        char magic[8]; size_t n=fread(magic,1,8,f); fclose(f);
        if (n!=8 || memcmp(magic,"NQDB0002",8)!=0) return 24;
    }
    return 0;
}
int main(int argc,char **argv){
    if(argc!=3) return 2;
    if(strcmp(argv[1],"write")==0){
        int64_t rc=noeZigRelationalSmoke(argv[2]);
        if(rc!=0){fprintf(stderr,"Noqeri smoke failed %lld\n",(long long)rc);return 11;}
        return verify(argv[2],0);
    }
    if(strcmp(argv[1],"verify")==0)return verify(argv[2],1);
    return 2;
}

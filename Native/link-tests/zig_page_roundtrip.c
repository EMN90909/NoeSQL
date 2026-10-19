#include "noesql_zig_pages.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

static int fail(const char* msg){fprintf(stderr,"Zig/Node round-trip: %s\n",msg);return 1;}
int main(int argc,char** argv){
    if(argc!=3 && argc!=4)return fail("usage: verify <data.noedb> | write <data.noedb> <pages-prefix>");
    void* h=NULL;
    if(noesql_zig_page_open(NOESQL_ZIG_PAGE_ABI_VERSION,argv[2],&h)!=NOESQL_ZIG_PAGE_OK||!h)return fail("native Zig pager open failed");
    int failed=0;
    if(strcmp(argv[1],"verify")==0 && argc==3){
        uint8_t buf[NOESQL_ZIG_PAGE_BYTES];
        if(noesql_zig_page_get(h,0,buf,sizeof(buf))!=NOESQL_ZIG_PAGE_OK || memcmp(buf,"NQRS0001",8)!=0)failed=fail("native pager did not read Node state header");
        else if(noesql_zig_page_get(h,1,buf,sizeof(buf))!=NOESQL_ZIG_PAGE_OK)failed=fail("native pager did not read Node payload page");
        else if(noesql_zig_page_generation(h)!=1)failed=fail("native pager generation differs from Node");
        else if(noesql_zig_page_validate_reference_state(h)!=NOESQL_ZIG_PAGE_OK)failed=fail("Zig payload manifest validation failed");
    } else if(strcmp(argv[1],"write")==0 && argc==4){
        int count=0;
        for(int id=0;id<16;id++){
            char path[4096];
            const int n=snprintf(path,sizeof(path),"%s.%d",argv[3],id);
            if(n<=0||(size_t)n>=sizeof(path)){failed=fail("fixture path exceeds bound");break;}
            FILE* f=fopen(path,"rb");
            if(!f){if(id==0)failed=fail("missing manifest fixture");break;}
            uint8_t buf[NOESQL_ZIG_PAGE_BYTES];
            const size_t read=fread(buf,1,sizeof(buf),f);
            const int beyond=fgetc(f);
            fclose(f);
            if(read!=sizeof(buf)||beyond!=EOF){failed=fail("invalid fixture page size");break;}
            if(noesql_zig_page_stage(h,(uint32_t)id,buf,sizeof(buf))!=NOESQL_ZIG_PAGE_OK){failed=fail("native stage failed");break;}
            count++;
        }
        if(!failed && (!count||noesql_zig_page_commit(h)!=NOESQL_ZIG_PAGE_OK))failed=fail("native Zig commit failed");
        if(!failed && noesql_zig_page_checkpoint(h)!=NOESQL_ZIG_PAGE_OK)failed=fail("native Zig checkpoint failed");
        if(!failed && noesql_zig_page_validate_reference_state(h)!=NOESQL_ZIG_PAGE_OK)failed=fail("Zig payload digest did not match staged fixture");
    }else failed=fail("invalid command");
    noesql_zig_page_close(h);
    return failed;
}

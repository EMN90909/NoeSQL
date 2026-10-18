#define _POSIX_C_SOURCE 200809L
#include "noesql_zig_pages.h"
#include <sqlite3.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

/* Experimental single-writer 4 KiB fsynced page commits vs SQLite WAL/FULL.
 * No comparisons for relational SQL semantics, multi-reader concurrency or
 * PostgreSQL. Includes different engine/file-format semantics, so results
 * only quantify the narrow durable write API overhead.
 * Usage: ./benchmark-zig-v2 /absolute/private/workdir [transactions] */
static double now(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return (double)t.tv_sec+(double)t.tv_nsec*1e-9;}
static int compare(const void*a,const void*b){double x=*(const double*)a,y=*(const double*)b;return(x>y)-(x<y);}
static void summary(const char*name,double*times,size_t count){
  qsort(times,count,sizeof(double),compare);
  double total=0;for(size_t i=0;i<count;i++)total+=times[i];
  size_t p95=(size_t)((count-1)*.95),p99=(size_t)((count-1)*.99);
  printf("%s,transactions=%zu,txn_per_s=%.2f,p50_ms=%.4f,p95_ms=%.4f,p99_ms=%.4f\n",
    name,count,count/total,1000*times[count/2],1000*times[p95],1000*times[p99]);
}
static void sqlcheck(int code,sqlite3*db){if(code!=SQLITE_OK&&code!=SQLITE_DONE){fprintf(stderr,"SQLite %s\n",sqlite3_errmsg(db));exit(1);}}
int main(int argc,char**argv){
 if(argc<2||argc>3){fprintf(stderr,"Usage: benchmark-zig-v2 /absolute/private/dir [transactions]\n");return 2;}
 size_t count=argc==3?strtoul(argv[2],NULL,10):250;
 if(count<10||count>5000){fprintf(stderr,"transactions must be 10..5000\n");return 2;}
 char path[4096],other[4096];
 if(snprintf(path,sizeof(path),"%s/native.nqdb",argv[1])>=(int)sizeof(path)||
    snprintf(other,sizeof(other),"%s/sqlite.sqlite",argv[1])>=(int)sizeof(other))return 2;
 void *page_store=NULL;
 if(noesql_zig_page_open(NOESQL_ZIG_PAGE_ABI_VERSION,path,&page_store)!=0){fprintf(stderr,"Zig pager open failed\n");return 1;}
 double*zig=calloc(count,sizeof(double));double*sqlite_times=calloc(count,sizeof(double));
 if(!zig||!sqlite_times)return 1;
 unsigned char blob[NOESQL_ZIG_PAGE_BYTES]={0};
 for(size_t i=0;i<count;i++){
   memcpy(blob,&i,sizeof(i));
   const double begin=now();
   if(noesql_zig_page_stage(page_store,(uint32_t)(i%128),blob,sizeof(blob))!=0||
      noesql_zig_page_commit(page_store)!=0){fprintf(stderr,"Zig durable commit failed at %zu\n",i);return 1;}
   zig[i]=now()-begin;
 }
 noesql_zig_page_close(page_store);
 sqlite3*db=NULL;sqlcheck(sqlite3_open(other,&db),db);
 sqlite3_stmt*check=NULL;
 sqlcheck(sqlite3_prepare_v2(db,"PRAGMA journal_mode=WAL",-1,&check,NULL),db);
 if(sqlite3_step(check)!=SQLITE_ROW||strcmp((const char*)sqlite3_column_text(check,0),"wal")!=0){fprintf(stderr,"SQLite WAL not available\n");return 1;}
 sqlite3_finalize(check);
 sqlcheck(sqlite3_exec(db,"PRAGMA synchronous=FULL;CREATE TABLE pages(id INTEGER PRIMARY KEY, body BLOB NOT NULL)",NULL,NULL,NULL),db);
 sqlcheck(sqlite3_prepare_v2(db,"INSERT INTO pages(id,body) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body",-1,&check,NULL),db);
 for(size_t i=0;i<count;i++){
   memcpy(blob,&i,sizeof(i));
   const double begin=now();
   sqlcheck(sqlite3_exec(db,"BEGIN IMMEDIATE",NULL,NULL,NULL),db);
   sqlcheck(sqlite3_bind_int(check,1,(int)(i%128)),db);
   sqlcheck(sqlite3_bind_blob(check,2,blob,sizeof(blob),SQLITE_STATIC),db);
   sqlcheck(sqlite3_step(check),db);
   sqlcheck(sqlite3_reset(check),db);
   sqlcheck(sqlite3_clear_bindings(check),db);
   sqlcheck(sqlite3_exec(db,"COMMIT",NULL,NULL,NULL),db);
   sqlite_times[i]=now()-begin;
 }
 sqlcheck(sqlite3_finalize(check),db);sqlcheck(sqlite3_close(db),db);
 summary("Zig-pager-v2",zig,count);
 summary("SQLite-WAL-FULL",sqlite_times,count);
 free(zig);free(sqlite_times);
 return 0;
}

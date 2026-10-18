#!/usr/bin/env node
// NoeSQL vs SQLite benchmark gate. SQLite-only baseline never implies native parity.
import {DatabaseSync} from 'node:sqlite'
import {createHash} from 'node:crypto'
import {spawnSync} from 'node:child_process'
import {mkdtempSync,rmSync,existsSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join,resolve} from 'node:path'
import {fileURLToPath} from 'node:url'
const now=()=>process.hrtime.bigint()
const ms=ns=>Number(ns)/1e6
const sha=rows=>createHash('sha256').update(JSON.stringify(rows)).digest('hex')
const p=(list,q)=>list[Math.min(list.length-1,Math.ceil(q*list.length)-1)]
function integer(value,defaultValue,min,max){
  const x=value===undefined?defaultValue:Number(value)
  if(!Number.isSafeInteger(x)||x<min||x>max)throw Error('invalid workload parameter')
  return x
}
export function baselineTrial(dir,{transactions=100,rowsPerTransaction=1,blobBytes=4096}={}){
  transactions=integer(transactions,100,10,100000)
  rowsPerTransaction=integer(rowsPerTransaction,1,1,16)
  blobBytes=integer(blobBytes,4096,0,8192)
  const db=new DatabaseSync(join(dir,'baseline.sqlite'))
  let summary
  try{
    if(db.prepare('PRAGMA journal_mode=WAL').get().journal_mode!=='wal')
      throw Error('SQLite WAL mode was not established')
    db.exec('PRAGMA synchronous=FULL;PRAGMA foreign_keys=ON')
    if(db.prepare('PRAGMA synchronous').get().synchronous!==2)
      throw Error('SQLite FULL synchronous mode unavailable')
    db.exec('CREATE TABLE records (id INTEGER PRIMARY KEY,version INTEGER NOT NULL,payload BLOB NOT NULL)')
    const stmt=db.prepare('INSERT INTO records(id,version,payload) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET version=excluded.version,payload=excluded.payload')
    const payload=Buffer.alloc(blobBytes),times=[],start=now()
    for(let i=0;i<transactions;i++){
      const t=now()
      db.exec('BEGIN IMMEDIATE')
      try{
        for(let k=0;k<rowsPerTransaction;k++){
          const id=(i*rowsPerTransaction+k)%128
          if(blobBytes>=8)payload.writeBigUInt64LE(BigInt(i*rowsPerTransaction+k))
          stmt.run(id,i,payload)
        }
        db.exec('COMMIT')
      }catch(e){db.exec('ROLLBACK');throw e}
      times.push(ms(now()-t))
    }
    const elapsedMs=ms(now()-start)
    times.sort((a,b)=>a-b)
    const rows=db.prepare('SELECT id,version,hex(payload) AS payload FROM records ORDER BY id').all()
    if(db.prepare('PRAGMA quick_check').get().quick_check!=='ok')
      throw Error('SQLite integrity check failed')
    summary={engine:'SQLite',durability:'WAL/FULL',transactions,rowsPerTransaction,blobBytes,
      storedRows:rows.length,elapsedMs:+elapsedMs.toFixed(3),
      transactionsPerSecond:+(1000*transactions/elapsedMs).toFixed(1),
      p50Ms:+p(times,.5).toFixed(4),p95Ms:+p(times,.95).toFixed(4),p99Ms:+p(times,.99).toFixed(4),
      resultSha256:sha(rows),restartVerified:false}
  }finally{db.close()}
  const reopened=new DatabaseSync(join(dir,'baseline.sqlite'))
  try{
    const rows=reopened.prepare('SELECT id,version,hex(payload) AS payload FROM records ORDER BY id').all()
    if(sha(rows)!==summary.resultSha256||reopened.prepare('PRAGMA quick_check').get().quick_check!=='ok')
      throw Error('SQLite cold-restart contents differ')
    summary.restartVerified=true
  }finally{reopened.close()}
  return summary
}
export function nativeRequired(env=process.env){
  for(const k of ['NOESQL_NATIVE_BIN','NOESQL_NATIVE_BIN_SHA256','NOQERI_ROOT','NOQERI_COMMIT_SHA','NOQERI_SOURCE_CLOSURE_SHA256']){
    if(!env[k])throw Error('native Noqeri/Zig benchmark BLOCKED: '+k+' is not configured')
  }
  const check=fileURLToPath(new URL('../scripts/verify-native-e2e.mjs',import.meta.url))
  if(!existsSync(check))throw Error('native E2E qualification is absent')
  const r=spawnSync(process.execPath,[check],{cwd:fileURLToPath(new URL('../',import.meta.url)),
    env,timeout:120000,encoding:'utf8',shell:false})
  if(r.error||r.status!==0)throw Error('native Noqeri/Zig benchmark BLOCKED: compiler and Zig E2E proof failed')
  return true
}
export function sqliteOnly({transactions=100,rowsPerTransaction=1,blobBytes=4096,repeats=5}={}){
  repeats=integer(repeats,5,3,20)
  const trials=[]
  for(let i=0;i<repeats;i++){
    const dir=mkdtempSync(join(tmpdir(),'noesql-sqlite-bench-'))
    try{trials.push(baselineTrial(dir,{transactions,rowsPerTransaction,blobBytes}))}
    finally{rmSync(dir,{recursive:true,force:true})}
  }
  const rates=trials.map(x=>x.transactionsPerSecond).sort((a,b)=>a-b)
  return {status:'sqlite-baseline-only',nativeComparisonQualified:false,
    workload:{transactions,rowsPerTransaction,blobBytes,repeats,sqliteDurability:'WAL/FULL'},
    medianSqliteTransactionsPerSecond:rates[Math.floor(rates.length/2)],
    trials,warning:'NoeSQL native throughput is NOT measured. Do not infer native speed from SQLite-only trials.'}
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{
    const args=process.argv.slice(2),options={}
    for(const name of ['transactions','rowsPerTransaction','blobBytes','repeats']){
      const key='--'+name.replace(/[A-Z]/g,s=>'-'+s.toLowerCase())+'='
      const found=args.find(a=>a.startsWith(key))
      if(found)options[name]=Number(found.slice(key.length))
    }
    if(!args.includes('--sqlite-only')){
      nativeRequired()
      throw Error('native relational benchmark endpoint is not implemented; native and SQLite performance cannot be fairly compared')
    }
    console.log(JSON.stringify(sqliteOnly(options),null,2))
  }catch(e){console.error(e.message);process.exitCode=1}
}

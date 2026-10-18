// Same SQLite connection, same dataset. Measures added NoeSQL compiler/API overhead,
// NOT a NoeSQL native page kernel benchmark or an independent SQLite competitor.
import {DatabaseSync} from 'node:sqlite'
import {performance} from 'node:perf_hooks'
import {fromSQLiteDatabase} from '../Runtime/noesql-embedded.mjs'
const N = Number(process.argv[2] || 10000)
const LOOKUPS = Number(process.argv[3] || 3000)
if(!Number.isInteger(N)||N<100||N>100000||!Number.isInteger(LOOKUPS)||LOOKUPS<50||LOOKUPS>100000)throw Error('usage: node embedded-vs-sqlite.mjs [rows:100-100000] [lookups:50-100000]')
const sqlite = new DatabaseSync(':memory:')
sqlite.exec('CREATE TABLE docs (id INTEGER PRIMARY KEY, tag TEXT NOT NULL, payload TEXT NOT NULL);CREATE INDEX docs_tag ON docs(tag);')
const insert = sqlite.prepare('INSERT INTO docs(id,tag,payload) VALUES (?,?,?)')
sqlite.exec('BEGIN IMMEDIATE')
for(let i=1;i<=N;i++)insert.run(i,'tag'+(i%100),`payload-${i}`.padEnd(120,'.'))
sqlite.exec('COMMIT')
const noesql=fromSQLiteDatabase(sqlite,{maxRows:10})
const direct=sqlite.prepare('SELECT id, payload FROM docs WHERE id=? LIMIT 1 OFFSET 0')
const query='from docs\nselect id, payload\nwhere id == $id\ntake 1'
const percentiles = a=>{a.sort((x,y)=>x-y);return {p50:a[Math.floor(a.length*0.5)],p95:a[Math.floor(a.length*0.95)],p99:a[Math.floor(a.length*0.99)]}}
function bench(fn){
  for(let i=0;i<500;i++)fn(i%N+1)
  const durations=[];let checks=0
  const t0=performance.now()
  for(let i=0;i<LOOKUPS;i++){
    const id=1+((i*7919)%N),start=performance.now()
    const row=fn(id)
    durations.push(performance.now()-start)
    if(row?.id===id)checks++
  }
  const total=performance.now()-t0
  if(checks!==LOOKUPS)throw Error('incorrect results')
  return {opsPerSec:Math.round(LOOKUPS*1000/total),latencyMs:percentiles(durations)}
}
const sqliteResult=bench(id=>direct.get(id))
const noeResult=bench(id=>noesql.query(query,{id})[0])
const N_WRITES=Math.min(N,5000)
const writer=new DatabaseSync(':memory:')
writer.exec('CREATE TABLE baseline_batch (id INTEGER PRIMARY KEY, value TEXT);CREATE TABLE noe_batch (id INTEGER PRIMARY KEY, value TEXT)')
const rawInsert=writer.prepare('INSERT INTO baseline_batch (id, value) VALUES (?, ?)')
const value='x'.repeat(120)
const t1=performance.now()
writer.exec('BEGIN IMMEDIATE')
for(let i=0;i<N_WRITES;i++)rawInsert.run(i,value)
writer.exec('COMMIT')
const directDuration=performance.now()-t1
const wrapper=fromSQLiteDatabase(writer,{maxMutations:N_WRITES})
const t2=performance.now()
wrapper.transaction(tx=>{for(let i=0;i<N_WRITES;i++)tx.insert('noe_batch',{id:i,value})})
const wrapperDuration=performance.now()-t2
const batchWrites={rows:N_WRITES,directRowsPerSec:Math.round(N_WRITES*1000/directDuration),noesqlRowsPerSec:Math.round(N_WRITES*1000/wrapperDuration)}
console.log(JSON.stringify({engine:`SQLite ${process.versions.sqlite} via node:sqlite`,datasetRows:N,lookups:LOOKUPS,comparison:'same SQLite engine, API and language overhead only; in-memory for repeatability',directSQLite:sqliteResult,noesqlEmbedded:noeResult,batchWrites},null,2))
noesql.close();wrapper.close()

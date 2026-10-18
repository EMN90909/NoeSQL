// Embedded NoeSQL for Node.js websites and desktop/server applications.
// Explicit SQLite-backed mode: this does NOT change the native .nqd page format.
import { existsSync, lstatSync, openSync, closeSync, chmodSync } from 'node:fs'
import { dirname, isAbsolute, extname } from 'node:path'
import { createAdvancedQueryExecutor } from './noesql-advanced.mjs'
const VALID = /^[A-Za-z_][A-Za-z0-9_]*$/
const q = s => {if (typeof s !== 'string' || !VALID.test(s) || (s.toLowerCase().startsWith('sqlite_') || s.toLowerCase().startsWith('_noesql_'))) throw Error('NOE-EMBED: invalid identifier');return '"'+s+'"'}
const assert = (ok,msg) => {if(!ok)throw Error('NOE-EMBED: '+msg)}
const bind = v => {
  if(typeof v==='boolean')return Number(v)
  assert(v===null || typeof v==='string' || (typeof v==='number' && Number.isSafeInteger(v)) || (v instanceof Uint8Array && v.length <= 256*1024),'invalid bound value')
  if(typeof v==='string')assert(Buffer.byteLength(v,'utf8')<=256*1024,'bound value exceeds 256KiB')
  return v
}
function buildAPI(db,{adapter=null,maxRows=500,maxMutations=1000,backend='sqlite'}={}) {
  assert(Number.isSafeInteger(maxMutations)&&maxMutations>0&&maxMutations<=10000,'invalid mutation quota')
  const queries=createAdvancedQueryExecutor(db,{maxRows})
  let closed=false,inside=false
  const columnCache=new Map(),statementCache=new Map()
  const versionStmt=db.prepare('PRAGMA schema_version')
  let schemaVersion=versionStmt.get().schema_version
  function invalidate(){columnCache.clear();statementCache.clear();queries.clearCache();schemaVersion=versionStmt.get().schema_version}
  function checkVersion(){
    if(inside)return
    const current=versionStmt.get().schema_version
    if(current!==schemaVersion)invalidate()
  }
  function cols(table){
    q(table);checkVersion()
    if(columnCache.has(table))return columnCache.get(table)
    const fields=new Set(db.prepare(`PRAGMA table_info(${q(table)})`).all().map(x=>x.name))
    assert(fields.size>0,`unknown table ${table}`)
    columnCache.set(table,fields)
    return fields
  }
  function fields(table,names){const c=cols(table);for(const key of names){q(key);assert(c.has(key),`unknown column ${table}.${key}`)}}
  function recordFields(table,data){
    assert(data&&typeof data==='object'&&Object.getPrototypeOf(data)===Object.prototype,'record must be a plain object')
    const keys=Object.keys(data)
    assert(keys.length>0&&keys.length<=64,'record requires 1..64 fields')
    fields(table,keys)
    return {keys,values:keys.map(k=>bind(data[k]))}
  }
  function whereFields(table,data){
    assert(data&&typeof data==='object'&&Object.getPrototypeOf(data)===Object.prototype,'where must be a plain object')
    const keys=Object.keys(data)
    assert(keys.length>0&&keys.length<=16,'writes need 1..16 equality predicates')
    fields(table,keys)
    return {sql:keys.map(k=>`${q(k)} = ?`).join(' AND '),values:keys.map(k=>bind(data[k]))}
  }
  function stmt(sql){
    if(statementCache.has(sql))return statementCache.get(sql)
    const prepared=db.prepare(sql)
    statementCache.set(sql,prepared)
    if(statementCache.size>256)statementCache.delete(statementCache.keys().next().value)
    return prepared
  }
  const check=()=>assert(!closed,'database closed')
  const actions={
    insert(table,record) {check();const {keys,values}=recordFields(table,record);return stmt(`INSERT INTO ${q(table)} (${keys.map(q).join(', ')}) VALUES (${keys.map(()=>'?').join(', ')})`).run(...values).changes},
    update(table,data,where) {check();const d=recordFields(table,data);const w=whereFields(table,where);return stmt(`UPDATE ${q(table)} SET ${d.keys.map(k=>`${q(k)} = ?`).join(', ')} WHERE ${w.sql}`).run(...d.values,...w.values).changes},
    remove(table,where) {check();const w=whereFields(table,where);return stmt(`DELETE FROM ${q(table)} WHERE ${w.sql}`).run(...w.values).changes},
    query(source,params={}) {check();return queries.query(source,params)},
    explain(source,params={}) {check();return queries.explain(source,params)}
  }
  const api={
    ...actions,
    plan(source,params={}){check();return queries.compile(source,params)},
    run(source,params={}) {check();assert(!inside,'run .noe statements outside transaction()');assert(adapter,'NoeSQL interpreter not attached');const out=adapter.run(source,params);invalidate();return out},
    named(name,params={}) {check();assert(adapter,'NoeSQL interpreter not attached');return adapter.query(name,params)},
    createIndex(table,columns,{name,unique=false}={}) {
      check();assert(!inside,'schema modifications outside transaction()')
      assert(Array.isArray(columns)&&columns.length>0&&columns.length<=8,'index requires 1..8 columns')
      fields(table,columns)
      const indexName=name||`noe_idx_${table}_${columns.join('_')}`
      db.exec(`CREATE ${unique?'UNIQUE ':''}INDEX IF NOT EXISTS ${q(indexName)} ON ${q(table)} (${columns.map(q).join(', ')})`)
      invalidate()
    },
    transaction(fn){
      check();assert(!inside,'nested transactions are not supported')
      assert(typeof fn==='function','transaction requires callback')
      checkVersion()
      inside=true;let operations=0
      try {
        db.exec('BEGIN IMMEDIATE')
        const tx=Object.freeze({
          query:actions.query,explain:actions.explain,
          insert(...args){assert(++operations<=maxMutations,'transaction mutation quota');return actions.insert(...args)},
          update(...args){assert(++operations<=maxMutations,'transaction mutation quota');return actions.update(...args)},
          remove(...args){assert(++operations<=maxMutations,'transaction mutation quota');return actions.remove(...args)}
        })
        const result=fn(tx)
        assert(!(result&&typeof result.then==='function'),'transaction callback must be synchronous')
        db.exec('COMMIT');return result
      }catch(e){try{db.exec('ROLLBACK')}catch{};throw e}
      finally{inside=false}
    },
    verify(){check();const result=db.prepare('PRAGMA quick_check').all();return {ok:result.every(x=>Object.values(x)[0]==='ok'),details:result}},
    close(){if(!closed){closed=true;invalidate();if(adapter)adapter.close();else db.close()}},
    // Trusted server-side migrations and diagnostics only: NEVER expose over HTTP.
    database:db,
    backend
  }
  return Object.freeze(api)
}
/** Compose with the repo's established .noe SQLite dialect adapter. */
export async function openNoeSQLEmbedded({file=':memory:',maxRows=500,maxMutations=1000,busyTimeoutMs=5000}={}) {
  assert(file===':memory:' || (typeof file==='string'&&isAbsolute(file)&&extname(file)==='.sqlite'),'use :memory: or an absolute .sqlite filename; do not reuse native .nqd files')
  assert(Number.isSafeInteger(busyTimeoutMs)&&busyTimeoutMs>=0&&busyTimeoutMs<=30000,'busy timeout must be 0..30000ms')
  if(file!==':memory:') {
    if(existsSync(file)){const st=lstatSync(file);assert(st.isFile()&&!st.isSymbolicLink()&&st.nlink===1,'unsafe file type or hard link')}
    else {const fd=openSync(file,'wx',0o600);closeSync(fd)}
    chmodSync(file,0o600)
  }
  const {createSqliteAdapter}=await import('./noesql-adapters.mjs')
  // The adapter invokes this hook before its SQLite COMMIT. Schema metadata and
  // physical DDL therefore commit or roll back as one unit.
  const persistCatalog=(code,db)=>{
    const structural=[]
    const collect=code=>{for(const s of code){if(s.k==='atomic')collect(s.body);else if(['db','table','alter','index','query'].includes(s.k))structural.push(s)}}
    collect(code)
    if(structural.length===0)return
    const statement=db.prepare('INSERT INTO "_noesql_catalog" (definition) VALUES (?)')
    for(const ast of structural)statement.run(JSON.stringify(ast))
  }
  const adapter=await createSqliteAdapter(file,{afterSuccessfulRun:persistCatalog})
  try {
    const db=adapter.db
    db.exec(`PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA busy_timeout=${busyTimeoutMs}; PRAGMA synchronous=FULL;`)
    if(file!==':memory:')db.exec('PRAGMA journal_mode=WAL')
    const journal=db.prepare('PRAGMA journal_mode').get().journal_mode
    const synchronous=db.prepare('PRAGMA synchronous').get().synchronous
    if(file!==':memory:')assert(journal==='wal'&&synchronous===2,'durable WAL/FULL mode not available')
    db.exec('CREATE TABLE IF NOT EXISTS "_noesql_catalog" (seq INTEGER PRIMARY KEY, definition TEXT NOT NULL)')
    for(const row of db.prepare('SELECT definition FROM "_noesql_catalog" ORDER BY seq').all()) {
      const definition=JSON.parse(row.definition)
      adapter.compiler.compile(definition)
    }
    return buildAPI(db,{adapter,maxRows,maxMutations,backend:file===':memory:'?'sqlite-memory':'sqlite-wal'})
  }catch(e){adapter.close();throw e}
}
/** Construct from a caller-owned DatabaseSync for integration tests or application-managed pooling. */
export function fromSQLiteDatabase(db,options={}) {
  const journal=db.prepare('PRAGMA journal_mode').get().journal_mode
  return buildAPI(db,{...options,backend:'sqlite-'+journal})
}

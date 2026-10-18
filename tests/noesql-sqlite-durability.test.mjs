import {test} from 'node:test'
import {strict as assert} from 'node:assert'
import {DatabaseSync} from 'node:sqlite'
import {mkdtemp,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {spawn} from 'node:child_process'
import {fromSQLiteDatabase} from '../Runtime/noesql-embedded.mjs'

async function workspace(fn){
  const dir=await mkdtemp(join(tmpdir(),'noe-sqlite-test-'))
  try{return await fn(join(dir,'data.sqlite'))}finally{await rm(dir,{force:true,recursive:true})}
}
test('file WAL restart preserves committed data and durable physical index',()=>workspace(async path=>{
  const conn=new DatabaseSync(path)
  conn.exec('PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;CREATE TABLE users(id INTEGER PRIMARY KEY,name TEXT NOT NULL)')
  const first=fromSQLiteDatabase(conn)
  first.transaction(tx=>{for(let i=1;i<=40;i++)tx.insert('users',{id:i,name:`person-${i}`})})
  first.createIndex('users',['name'])
  first.close()
  const conn2=new DatabaseSync(path)
  const second=fromSQLiteDatabase(conn2)
  assert.equal(second.database.prepare('PRAGMA integrity_check').get().integrity_check,'ok')
  assert.deepEqual(second.query('from users\nselect id\nwhere name == $name',{name:'person-31'}),[{id:31}])
  assert.ok(second.explain('from users\nselect id\nwhere name == $name',{name:'person-31'}).some(row=>String(row.detail).includes('noe_idx_users_name')))
  second.close()
}))
test('WAL reader sees transaction snapshot while another connection commits',()=>workspace(async path=>{
  const db1=new DatabaseSync(path),db2=new DatabaseSync(path)
  try{
    db1.exec('PRAGMA journal_mode=WAL;CREATE TABLE events(id INTEGER PRIMARY KEY, title TEXT);INSERT INTO events VALUES(1,\'First\')')
    db2.exec('PRAGMA journal_mode=WAL')
    db1.exec('BEGIN')
    assert.equal(db1.prepare('SELECT count(*) AS n FROM events').get().n,1)
    db2.exec('INSERT INTO events VALUES(2,\'Second\')')
    assert.equal(db1.prepare('SELECT count(*) AS n FROM events').get().n,1)
    db1.exec('COMMIT')
    assert.equal(db1.prepare('SELECT count(*) AS n FROM events').get().n,2)
  }finally{db1.close();db2.close()}
}))
test('committed SQLite WAL state survives subprocess SIGKILL', {skip:process.platform==='win32'},()=>workspace(async path=>{
  const db=new DatabaseSync(path)
  db.exec('PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE records(id INTEGER PRIMARY KEY, text TEXT); INSERT INTO records VALUES(1,\'baseline\')')
  db.close()
  const script=`import {DatabaseSync} from 'node:sqlite'; const db=new DatabaseSync(${JSON.stringify(path)}); db.exec(\"PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; INSERT INTO records VALUES(2,'committed')\");process.kill(process.pid,'SIGKILL')`
  const status=await new Promise((resolve,reject)=>{const child=spawn(process.execPath,['--input-type=module','-e',script],{stdio:'ignore'});child.once('error',reject);child.once('close',(code,signal)=>resolve({code,signal}))})
  assert.equal(status.signal,'SIGKILL')
  const reopened=new DatabaseSync(path)
  try{assert.equal(reopened.prepare('SELECT count(*) AS n FROM records').get().n,2);assert.equal(reopened.prepare('PRAGMA integrity_check').get().integrity_check,'ok')}
  finally{reopened.close()}
}))

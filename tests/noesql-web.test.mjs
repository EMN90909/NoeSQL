import {test} from 'node:test'
import {strict as assert} from 'node:assert'
import {DatabaseSync} from 'node:sqlite'
import {createServer} from 'node:http'
import {fromSQLiteDatabase} from '../Runtime/noesql-embedded.mjs'
import {createNoeSQLHttpHandler} from '../Runtime/noesql-web.mjs'
const READER='reader-key-change-me-very-long-secret-1234'
const WRITER='writer-key-change-me-very-long-secret-1234'
async function fixture(t,{rate=100,limit=1024*64}={}){
  const sqlite=new DatabaseSync(':memory:')
  sqlite.exec('CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL); INSERT INTO notes VALUES (1,\'Hello\')')
  const db=fromSQLiteDatabase(sqlite)
  const server=createServer(createNoeSQLHttpHandler({database:db,keys:[{token:READER,role:'reader'},{token:WRITER,role:'writer'}],requestsPerMinute:rate,maxBodyBytes:limit}))
  await new Promise(ok=>server.listen(0,'127.0.0.1',ok))
  t.after(async()=>{await new Promise(ok=>server.close(ok));db.close()})
  const url=`http://127.0.0.1:${server.address().port}`
  return {db,url,post:(endpoint,body,token=READER)=>fetch(url+endpoint,{method:'POST',headers:{Authorization:`Bearer ${token}`,'Content-Type':'application/json'},body:JSON.stringify(body)})}
}
test('web API limits reader to prepared reads; writer uses atomic transactions',async t=>{
  const {post,db}=await fixture(t)
  let r=await post('/v1/query',{query:'from notes\nselect id, title\nsort id'})
  assert.equal(r.status,200)
  assert.deepEqual((await r.json()).rows,[{id:1,title:'Hello'}])
  r=await post('/v1/mutate',{operations:[{kind:'insert',table:'notes',values:{id:2,title:'New'}}]})
  assert.equal(r.status,403)
  r=await post('/v1/mutate',{operations:[{kind:'insert',table:'notes',values:{id:2,title:'New'}},{kind:'insert',table:'notes',values:{id:3,title:'Another'}}]},WRITER)
  assert.equal(r.status,200)
  assert.deepEqual((await r.json()).affected,[1,1])
  r=await post('/v1/mutate',{operations:[{kind:'insert',table:'notes',values:{id:4,title:'Rollback'}},{kind:'insert',table:'notes',values:{id:2,title:'duplicate'}}]},WRITER)
  assert.equal(r.status,400)
  assert.deepEqual(db.query('from notes\nselect id\nsort id'),[{id:1},{id:2},{id:3}])
})
test('HTTP auth, method, content type, and SQL injection defenses',async t=>{
  const {post,url}=await fixture(t)
  assert.equal((await fetch(url+'/health')).status,200)
  assert.equal((await fetch(url+'/v1/query')).status,405)
  assert.equal((await fetch(url+'/foo')).status,404)
  assert.equal((await post('/v1/query',{query:'from notes\nselect id'},'wrong-token-which-is-long-enough-here')).status,401)
  assert.equal((await post('/v1/query',{query:'from notes\nselect id'},'')).status,401)
  const injected=await post('/v1/query',{query:'from notes\nselect title\nwhere title == $value',params:{value:"Hello' OR 1=1 --"}})
  assert.equal(injected.status,200)
  assert.deepEqual((await injected.json()).rows,[])
  assert.equal((await post('/v1/query',{query:'from notes\nselect *; DROP TABLE notes'})).status,400)
})
test('HTTP enforces request budgets and rate limit',async t=>{
  const {post}=await fixture(t,{rate:2,limit:1024})
  assert.equal((await post('/v1/query',{query:'from notes\nselect id'})).status,200)
  assert.equal((await post('/v1/query',{query:'from notes\nselect id'})).status,200)
  assert.equal((await post('/v1/query',{query:'from notes\nselect id'})).status,429)
  assert.equal((await post('/v1/mutate',{operations:[{kind:'insert',table:'notes',values:{id:2,title:'x'.repeat(2000)}}]},WRITER)).status,413)
})

test('per-credential table allowlist blocks cross-table reads and writes',async t=>{
  const sqlite=new DatabaseSync(':memory:')
  sqlite.exec("CREATE TABLE public_notes (id INTEGER PRIMARY KEY,title TEXT); CREATE TABLE private_notes(id INTEGER PRIMARY KEY, secret TEXT); INSERT INTO public_notes VALUES (1,'safe'); INSERT INTO private_notes VALUES(1,'secret')")
  const db=fromSQLiteDatabase(sqlite)
  const handler=createNoeSQLHttpHandler({database:db,keys:[{token:READER,role:'reader',tables:['public_notes']},{token:WRITER,role:'writer',tables:['public_notes']}],requestsPerMinute:100})
  const server=createServer(handler)
  await new Promise(ok=>server.listen(0,'127.0.0.1',ok))
  t.after(async()=>{await new Promise(ok=>server.close(ok));db.close()})
  const url=`http://127.0.0.1:${server.address().port}`
  const post=(endpoint,body,token=READER)=>fetch(url+endpoint,{method:'POST',headers:{Authorization:`Bearer ${token}`,'Content-Type':'application/json'},body:JSON.stringify(body)})
  assert.equal((await post('/v1/query',{query:'from public_notes\nselect id, title'})).status,200)
  assert.equal((await post('/v1/query',{query:'from private_notes\nselect id, secret'})).status,403)
  assert.equal((await post('/v1/mutate',{operations:[{kind:'insert',table:'private_notes',values:{id:2,secret:'bad'}}]},WRITER)).status,403)
  assert.equal(sqlite.prepare('SELECT count(*) AS n FROM private_notes').get().n,1)
})

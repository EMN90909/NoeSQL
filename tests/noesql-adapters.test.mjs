import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createSqlCompiler, createSqliteAdapter, createPostgresAdapter } from '../Runtime/noesql-adapters.mjs'
import { parse, execute } from '../Runtime/noesql-reference.mjs'

const schema=`table users {
 id: int key auto
 name: text required
 age: int
 active: bool = true
}
index age_idx on users (age)
query adults(minAge: int = 18) {
 users get name, age where age >= $minAge sort name
}`

test('SQLite adapter runs schema, index, CRUD and named typed query with prepared params',async()=>{
 const a=await createSqliteAdapter()
 try {
  const result=a.run(schema+'\nusers add {name: "Grace", age: 40}\nusers add {name: "Ada", age: 22}\nusers add {name: "Lin", age: 15}')
  assert.equal(result.at(-1).rows.length,1)
  assert.deepEqual(a.query('adults').rows.map(r=>r.name),['Ada','Grace'])
  assert.deepEqual(a.query('adults',{minAge:30}).rows.map(r=>r.name),['Grace'])
  assert.equal(a.run('users where age >= 40 set active = false')[0].affected,1)
  assert.equal(a.run('users where age < 18 delete')[0].affected,1)
  assert.deepEqual(a.run('users get name where active == false')[0].rows.map(r=>r.name),['Grace'])
  assert.throws(()=>a.query('adults',{minAge:'wrong'}),/NOE440/)
  assert.throws(()=>a.run('users get unknown'),/NOE430/)
 }finally{a.close()}
})
test('SQLite prepared statements safely bind hostile strings',async()=>{
 const a=await createSqliteAdapter()
 try {
  a.run('table people { id: int key auto\n name: text required\n }')
  const attack='x\"; DROP TABLE people; --'
  const expr=`people add {name: ${JSON.stringify(attack)}}`
  a.run(expr)
  assert.deepEqual(a.run('people get name')[0].rows[0].name,attack)
  assert.equal(a.run('people get name')[0].rows.length,1)
 }finally{a.close()}
})
test('SQLite rolls back failed multi-step mutation',async()=>{
 const a=await createSqliteAdapter()
 try {
  a.run('table people { id: int key auto\n name: text required unique\n}')
  assert.throws(()=>a.run('people add {name: "A"}\npeople add {name: "A"}'),/UNIQUE constraint failed/)
  assert.deepEqual(a.run('people get name')[0].rows,[])
 }finally{a.close()}
})
test('PostgreSQL compiler uses placeholders and validates columns',()=>{
 const c=createSqlCompiler('postgres')
 const p=parse('table learners {id: int key auto\nname: text required\nage: int\n}\nlearners add {name: "X", age: 21}\nlearners get name where age >= $cutoff')
 const built=p.map(s=>c.compile(s,{cutoff:18}))
 assert.match(built[0].sql,/BIGSERIAL PRIMARY KEY/)
 assert.match(built[1].sql,/\$1/)
 assert.match(built[2].sql,/\$1/)
 assert.deepEqual(built[2].values,[18])
 assert.throws(()=>c.compile(parse('learners get nope')[0]),/NOE430/)
})
test('no hard dependency on Postgres; unavailable driver gives explicit error',async()=>{
 await assert.rejects(createPostgresAdapter({}),/optional npm package|ECONNREFUSED/)
})


test('SQLite restores in-memory schema metadata on rollback', async()=>{
 const a=await createSqliteAdapter()
 try {
  assert.throws(()=>a.run('table people { id: int key auto\n name: text required\n}\npeople add {name: "A"}\npeople add {id: 1, name: "B"}'),/UNIQUE constraint failed/)
  // The rolled back CREATE TABLE must not remain registered as valid catalog metadata.
  assert.equal(a.compiler.tables.people, undefined)
  a.run('table people { id: int key auto\n name: text required\n}')
  assert.deepEqual(a.run('people get name')[0].rows,[])
 } finally {a.close()}
})

test('SQLite relationship restrict and boolean result type', async()=>{
 const a=await createSqliteAdapter()
 try {
  a.run('table parents {id: int key auto\n active: bool required\n}\ntable children {id: int key auto\n parent: ref parents required\n}\nparents add {active: true}\nchildren add {parent: 1}')
  assert.equal(a.run('parents get active')[0].rows[0].active, true)
  assert.throws(()=>a.run('parents where id == 1 delete'),/FOREIGN KEY constraint failed/)
  assert.equal(a.run('children where id == 1 delete')[0].affected,1)
  assert.equal(a.run('parents where id == 1 delete')[0].affected,1)
 }finally {a.close()}
})


test('SQLite and independent reference agree on null-last deterministic sorting',async()=>{
 const source=`table users {id: int key auto
 name: text required
 age: int
}
users add {name:"Nil"}
users add {name:"B",age:20}
users add {name:"A",age:20}
users get name,age sort age take 2`
 const expected=execute(source).results.at(-1).rows
 const adapter=await createSqliteAdapter()
 try { const got=adapter.run(source).at(-1).rows;assert.deepEqual(got,expected) }
 finally { adapter.close() }
})

test('SQL compiler refuses invalid column references without contacting database',()=>{
 const compiler=createSqlCompiler('postgres')
 compiler.compile(parse('table users {id: int key auto\n name: text required\n}')[0])
 assert.throws(()=>compiler.compile(parse('users where unknown == 1 delete')[0]),/NOE402/)
 assert.throws(()=>compiler.compile(parse('users where id == $unset delete')[0]),/NOE401/)
})

import {test} from 'node:test'
import assert from 'node:assert/strict'
import {execute,runNamed} from '../Runtime/noesql-reference.mjs'

const fixture=()=>{
  const sql='table people {id:int key auto\n name:text required}\n' +
    'table jobs {id:int key auto\n person:ref people required\n salary:int required}\n' +
    'people add {name:"Ada"}\npeople add {name:"Bo"}\n' +
    'jobs add {person:1,salary:25}\njobs add {person:1,salary:15}'
  return execute(sql).database
}
test('pure get reuses the same database object instead of deep cloning',()=>{
  const db=fixture()
  const out=execute('people get id,name sort name',db)
  assert.equal(out.database,db)
  assert.deepEqual(out.results[0].rows,[{id:1,name:'Ada'},{id:2,name:'Bo'}])
})
test('read result objects never alias underlying state',()=>{
  const db=fixture()
  const row=execute('people get name take 1',db).results[0].rows[0]
  row.name='modified externally'
  assert.equal(db.tables.people.rows[0].name,'Ada')
  const grouped=execute('jobs get count(*) as n, sum(salary) as total',db).results[0].rows
  grouped[0].total=9999
  assert.equal(db.tables.jobs.rows[0].salary,25)
})
test('read-only join and group preserve SQL output and source state',()=>{
  const db=fixture(),snapshot=JSON.stringify(db)
  const r=execute('jobs get people.name, count(*) as n, sum(salary) as total join people on person == people.id group people.name sort people.name',db)
  assert.deepEqual(r.results[0].rows,[{'people.name':'Ada',n:2,total:40}])
  assert.equal(JSON.stringify(db),snapshot)
  assert.equal(r.database,db)
})
test('writes still clone and failure leaves caller database untouched',()=>{
  const db=fixture(),snapshot=JSON.stringify(db)
  const out=execute('people add {name:"Cleo"}',db)
  assert.notEqual(out.database,db)
  assert.equal(db.tables.people.rows.length,2)
  assert.equal(out.database.tables.people.rows.length,3)
  assert.throws(()=>execute('atomic {\npeople add {name:"New"}\npeople add {id:1,name:"Duplicate"}\n}',db),{code:'NOE203'})
  assert.equal(JSON.stringify(db),snapshot)
})
test('named read-only queries still bind parameters without mutating catalog',()=>{
  let db=fixture()
  db=execute('query paid(minimum:int=20) {jobs get count(*) as n where salary >= $minimum}',db).database
  const old=JSON.stringify(db)
  assert.deepEqual(runNamed(db,'paid').rows,[{n:1}])
  assert.deepEqual(runNamed(db,'paid',{minimum:10}).rows,[{n:2}])
  assert.equal(JSON.stringify(db),old)
})

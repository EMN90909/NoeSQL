import {test} from 'node:test'
import assert from 'node:assert/strict'
import {execute,emptyDatabase} from '../Runtime/noesql-reference.mjs'

const schema=`table people { id: int key auto\n name: text required\n}
table events { id: int key auto\n person_id: ref people required\n score: int required\n}`
const database=(n=100)=>{
  let db=execute(schema,emptyDatabase()).database
  let batch=''
  for(let i=1;i<=n;i++)batch+=`people add {name:"P${i}"}\nevents add {person_id:${i},score:${i}}\n`
  return execute(batch,db).database
}

test('hash equality extracted from AND preserves compound ON and aggregates',()=>{
 const db=database()
 const query='people get people.name, sum(events.score) as total join events on people.id == events.person_id && events.score >= 50 group people.name having total >= 50 sort people.name'
 const got=execute(query,db).results[0].rows
 assert.equal(got.length,51)
 assert.deepEqual(got.find(r=>r['people.name']==='P50'),{'people.name':'P50',total:50})
 assert.equal(got.some(r=>r['people.name']==='P49'),false)
})
test('small relations preserve nested-loop equality and left-null rows',()=>{
 const db=execute('table a {id:int key auto\n name:text}\ntable b {id:int key auto\n a_id:int}',emptyDatabase()).database
 const inserted=execute('a add {name:"x"}\na add {name:"y"}\nb add {a_id:1}',db).database
 const got=execute('a get a.name, b.a_id left join b on a.id == b.a_id sort a.name',inserted).results[0].rows
 assert.deepEqual(got,[{'a.name':'x','b.a_id':1},{'a.name':'y','b.a_id':null}])
})
test('base-only WHERE can filter joined rows while leaving left unmatched behavior correct',()=>{
 const db=database(100)
 const result=execute('people get people.name, events.score left join events on people.id == events.person_id where people.id > 90 sort people.name',db).results[0].rows
 assert.equal(result.length,10)
 assert(result.every(r=>Number(r['people.name'].slice(1))>90))
})
test('OR conditions are never treated as equality-only join predicates',()=>{
 const db=execute('table a {id:int key auto}\ntable b {id:int key auto\n a_id:int}\na add {}\na add {}\nb add {a_id:1}\nb add {a_id:2}',emptyDatabase()).database
 const rows=execute('a get a.id, b.id join b on a.id == b.a_id || a.id == b.id',db).results[0].rows
 assert.deepEqual(rows,[{'a.id':1,'b.id':1},{'a.id':2,'b.id':2}])
})
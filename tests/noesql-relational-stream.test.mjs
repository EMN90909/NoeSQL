import {test} from 'node:test'
import assert from 'node:assert/strict'
import {execute,emptyDatabase} from '../Runtime/noesql-reference.mjs'
const apply=(sql,db=emptyDatabase())=>execute(sql,db)
test('streamed grouping preserves null-aware count, sum, min and max',()=>{
  let {database}=apply('table x { id:int key auto\n group_id:int\n value:int }')
  ;({database}=apply('x add {group_id:1,value:10}\nx add {group_id:1}\nx add {group_id:2,value:-2}',database))
  const sql='x get group_id, count(*) as n, count(value) as nonnull, sum(value) as total, min(value) as low, max(value) as high group group_id sort group_id'
  assert.deepEqual(apply(sql,database).results[0].rows,[
    {group_id:1,n:2,nonnull:1,total:10,low:10,high:10},
    {group_id:2,n:1,nonnull:1,total:-2,low:-2,high:-2}
  ])
  assert.deepEqual(apply('x get count(*) as n, sum(value) as total',database).results[0].rows,[{n:3,total:8}])
})
test('safe WHERE pushdown preserves both base and joined filters',()=>{
  let {database}=apply('table p {id:int key auto\n name:text}\ntable c {id:int key auto\n pid:int\n amount:int}')
  ;({database}=apply('p add {name:"A"}\np add {name:"B"}\nc add {pid:1,amount:7}\nc add {pid:2,amount:9}',database))
  const base=apply('p get p.name, c.amount join c on p.id == c.pid where p.name == "A"',database).results[0].rows
  assert.deepEqual(base,[{'p.name':'A','c.amount':7}])
  const other=apply('p get p.name, c.amount join c on p.id == c.pid where c.amount >= 9',database).results[0].rows
  assert.deepEqual(other,[{'p.name':'B','c.amount':9}])
})
test('empty aggregates remain one row and integer overflow fails closed',()=>{
  let {database}=apply('table x {id:int key auto\n value:int}')
  assert.deepEqual(apply('x get count(*) as n, sum(value) as s, min(value) as mi, max(value) as ma',database).results[0].rows,[{n:0,s:null,mi:null,ma:null}])
  ;({database}=apply('x add {value:9007199254740991}\nx add {value:1}',database))
  assert.throws(()=>apply('x get sum(value) as total',database),e=>e.code==='NOE212')
})

import {test} from 'node:test'
import assert from 'node:assert/strict'
import {execute} from '../Runtime/noesql-reference.mjs'
const schema='table customers {id:int key auto\n name:text required}\ntable orders {id:int key auto\n customer_id:int\n amount:int}\ncustomers add {name:"Ada"}\ncustomers add {name:"Bea"}\norders add {customer_id:1,amount:10}\norders add {customer_id:1,amount:20}'
test('left join preserves unmatched rows and HAVING evaluates aggregate aliases',()=>{
  const {database}=execute(schema)
  const sql='customers get name, count(orders.id) as n, sum(orders.amount) as total left join orders on id == orders.customer_id group name having n >= 0 sort name'
  assert.deepEqual(execute(sql,database).results[0].rows,[
    {name:'Ada',n:2,total:30},{name:'Bea',n:0,total:null}
  ])
  assert.deepEqual(execute('customers get name, count(orders.id) as n left join orders on id == orders.customer_id group name having n > 0 sort name',database).results[0].rows,[{name:'Ada',n:2}])
  assert.deepEqual(execute('customers get name join orders on id == orders.customer_id',database).results[0].rows,[{name:'Ada'},{name:'Ada'}])
})
test('HAVING rejects unaggregated input, unknown alias and incorrect types',()=>{
 const {database}=execute(schema)
 assert.throws(()=>execute('customers get name having name == "Ada"',database),e=>e.code==='NOE245')
 assert.throws(()=>execute('customers get count(*) as n having missing >= 1',database),e=>e.code==='NOE120')
 assert.throws(()=>execute('customers get count(*) as n having n + 1',database),e=>e.code==='NOE246')
})

import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { parse, execute, emptyDatabase, runNamed } from '../Runtime/noesql-reference.mjs'

const fixture=`table customers {
  id: int key auto
  name: text required
}
table orders {
  id: int key auto
  customer_id: ref customers required
  amount: int
}
customers add {name:"Ada"}
customers add {name:"Lin"}
orders add {customer_id:1,amount:10}
orders add {customer_id:1,amount:25}
orders add {customer_id:2,amount:15}
`
const apply=(sql,db=emptyDatabase())=>execute(sql,db)
test('joined grouped count/sum and sorting agree with expected totals',()=>{
  const {database}=apply(fixture)
  const sql='orders get customers.name, count(*) as purchases, sum(amount) as total join customers on customer_id == customers.id group customers.name sort customers.name'
  assert.deepEqual(apply(sql,database).results[0].rows,[
    {'customers.name':'Ada',purchases:2,total:35},
    {'customers.name':'Lin',purchases:1,total:15}
  ])
})
test('join is an inner join and limits/filters are enforced',()=>{
  const {database}=apply(fixture)
  const sql='orders get customers.name, amount join customers on customer_id == customers.id where amount >= 15 sort amount take 1'
  assert.deepEqual(apply(sql,database).results[0].rows,[{'customers.name':'Lin',amount:15}])
})
test('aggregate NULL semantics and empty input',()=>{
  let {database}=apply('table records { id:int key auto\n value:int }')
  assert.deepEqual(apply('records get count(*) as n, sum(value) as total, min(value) as lowest',database).results[0].rows,
    [{n:0,total:null,lowest:null}])
  ;({database}=apply('records add {}\nrecords add {value:4}',database))
  assert.deepEqual(apply('records get count(*) as n, count(value) as nonnull, sum(value) as total, max(value) as high',database).results[0].rows,
    [{n:2,nonnull:1,total:4,high:4}])
})
test('group projection validation, aliases, unsupported self joins and unknown relations',()=>{
  const {database}=apply(fixture)
  const invalid=[
    ['orders get amount, count(*) as n group customer_id','NOE235'],
    ['orders get count(*) as n, count(*) as n','NOE234'],
    ['orders get nope join customers on customer_id == customers.id','NOE119'],
    ['orders get * join orders on id == orders.id','NOE230'],
    ['orders get * join missing on id == missing.id','NOE100']
  ]
  for(const [sql,code] of invalid)assert.throws(()=>apply(sql,database),e=>e.code===code,sql)
})
test('typed named grouped query with a bound filter',()=>{
  let {database}=apply(fixture)
  ;({database}=apply('query totals(minAmount:int=15) { orders get count(*) as n, sum(amount) as total where amount >= $minAmount }',database))
  assert.deepEqual(runNamed(database,'totals').rows,[{n:2,total:40}])
  assert.deepEqual(runNamed(database,'totals',{minAmount:25}).rows,[{n:1,total:25}])
})
test('additive migrations preserve rows, update catalog version and reject unsafe additions',()=>{
  let {database}=apply('table people { id:int key auto\n name:text required }\npeople add {name:"Ada"}')
  const v=database.version
  ;({database}=apply('alter people add { active:bool required = true\n score:int }',database))
  assert.equal(database.version,v+1)
  assert.deepEqual(apply('people get name, active, score',database).results[0].rows,[{name:'Ada',active:true,score:null}])
  assert.throws(()=>apply('alter people add { new_name:text required }',database),e=>e.code==='NOE244')
  assert.throws(()=>apply('alter people add { age:int key auto }',database),e=>e.code==='NOE242')
})
test('malformed aggregates and unsafe column lookups fail closed',()=>{
  for(const sql of ['orders get sum(*)','orders get max()','orders get unknown()'])
    assert.throws(()=>parse(sql))
  const {database}=apply(fixture)
  assert.throws(()=>apply('orders get toString',database),e=>e.code==='NOE119')
})
test('aggregate overflow aborts the entire transaction',()=>{
  let {database}=apply('table x {id:int key auto\n value:int}')
  ;({database}=apply('x add {value:9007199254740991}\nx add {value:1}',database))
  assert.throws(()=>apply('x get sum(value) as total',database),e=>e.code==='NOE212')
})

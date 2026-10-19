#!/usr/bin/env node
// Read-only reference hot-path profiler. Builds synthetic valid state directly
// to avoid mixing O(n^2) reference insert validation with SELECT timings.
// NOT a native Noqeri/Zig measurement or a SQLite comparison.
import {performance} from 'node:perf_hooks'
import {execute} from '../Runtime/noesql-reference.mjs'

const n=Number(process.argv[2]||1000)
const trials=Number(process.argv[3]||7)
if(!Number.isSafeInteger(n)||n<1||n>10000)throw Error('rows must be integer 1..10000')
if(!Number.isSafeInteger(trials)||trials<3||trials>30)throw Error('trials must be integer 3..30')
const sql=`table customers {id:int key auto
 name:text required}
table orders {id:int key auto
 customer_id:ref customers required
 amount:int required}`
const db=execute(sql).database
for(let i=1;i<=n;i++){
  db.tables.customers.rows.push({id:i,name:'person_'+i})
  db.tables.orders.rows.push({id:i,customer_id:i,amount:(i%20)+1})
}
db.tables.customers.nextId=n+1
db.tables.orders.nextId=n+1
const queries=[
  {name:'point-lookup-scan',sql:`customers get id,name where id == ${Math.ceil(n/2)}`,expected:1},
  {name:'filter-scan',sql:'orders get id,amount where amount >= 15 take 250',
   expected:Math.min(250,Math.floor(n/20)*6+Math.max(0,n%20-14))},
  {name:'join-and-group',sql:'orders get customers.name, count(*) as n, sum(amount) as total join customers on customer_id == customers.id group customers.name sort customers.name',expected:n},
]
function measure(fn){
  fn()
  const samples=[]
  for(let i=0;i<trials;i++){
    const begin=performance.now()
    const count=fn()
    samples.push({ms:performance.now()-begin,count})
  }
  samples.sort((a,b)=>a.ms-b.ms)
  return {medianMs:+samples[Math.floor(samples.length/2)].ms.toFixed(3),
    minMs:+samples[0].ms.toFixed(3),
    maxMs:+samples.at(-1).ms.toFixed(3),
    trials}
}
const measurements=[]
for(const item of queries){
  const run=()=>{
    const result=execute(item.sql,db)
    if(result.database!==db)throw Error('read unexpectedly cloned entire database')
    const count=result.results[0].rows.length
    if(count!==item.expected)throw Error('query result mismatch: '+item.name+' expected '+item.expected+' got '+count)
    return count
  }
  measurements.push({name:item.name,rowsReturned:item.expected,...measure(run)})
}
const copy=measure(()=>structuredClone(db))
console.log(JSON.stringify({
  kind:'noesql-reference-read-path-profile',
  node:process.version,platform:process.platform,
  rowsPerTable:n,trials,
  measurements,
  formerDatabaseClone:copy,
  note:'Reference runtime only. No physical persisted SQL indexes, no native Noqeri executable, no SQLite comparison.'
},null,2))

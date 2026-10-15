// Opt-in comparative reference benchmark. This reports measurements, not a
// production performance claim. Query results are checked before timings.
import { performance } from 'node:perf_hooks'
import { execute } from '../Runtime/noesql-reference.mjs'
import { createSqliteAdapter } from '../Runtime/noesql-adapters.mjs'

const n=Number(process.argv[2]||250)
if(!Number.isInteger(n)||n<1||n>1000)throw Error('rows must be integer 1..1000')
const schema=`table customers { id:int key auto
 name:text required }
table orders { id:int key auto
 customer_id:ref customers required
 amount:int required }`
const seed=[schema]
for(let i=1;i<=n;i++)seed.push('customers add {name:"user'+i+'"}')
for(let i=1;i<=n;i++)seed.push('orders add {customer_id:'+i+',amount:10}')
const source=seed.join('\n')
const query='orders get customers.name, count(*) as n, sum(amount) as total join customers on customer_id == customers.id group customers.name sort customers.name'
const oracle=execute(source)
const expected=execute(query,oracle.database).results[0].rows
const adapter=await createSqliteAdapter()
try{
  adapter.run(source)
  const sqliteRows=adapter.run(query)[0].rows
  if(JSON.stringify(sqliteRows)!==JSON.stringify(expected))
    throw Error('reference and SQLite results disagree; measurements invalid')
  const bench=(work,iterations=5)=>{
    work()
    const ms=[]
    for(let i=0;i<iterations;i++){
      const start=performance.now()
      work()
      ms.push(performance.now()-start)
    }
    ms.sort((a,b)=>a-b)
    return {medianMs:Number(ms[2].toFixed(3)),minMs:Number(ms[0].toFixed(3)),trials:iterations}
  }
  console.log(JSON.stringify({
    kind:'noesql-experimental-query-benchmark',node:process.version,platform:process.platform,
    customers:n,orders:n,outputRows:expected.length,
    reference:bench(()=>execute(query,oracle.database)),
    sqlite:bench(()=>adapter.run(query)),
    note:'Not a native Noqeri measurement. JS cloning, interpreter overhead and SQLite memory backend are included.'
  },null,2))
}finally{adapter.close()}

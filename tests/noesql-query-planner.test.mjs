import {test} from 'node:test'
import assert from 'node:assert/strict'
import {chooseRelationalPlan,safePredicate,baseOnly} from '../Runtime/noesql-query-planner.mjs'
const col=name=>({k:'column',name})
const lit=value=>({k:'literal',value})
const binary=(op,left,right)=>({k:'binary',op,left,right})
const eq=(a,b)=>binary('==',col(a),col(b))
const left= n=>({rows:Array.from({length:n},(_,i)=>({id:i}))})
const selection=(on,where=null,joinType='inner')=>({table:'users',join:{table:'orders',type:joinType,on},where})
test('hash joins use qualified equi-join keys above cost threshold',()=>{
 const plan=chooseRelationalPlan(selection(eq('id','orders.user_id')),left(100),left(100))
 assert.equal(plan.join,'hash');assert.equal(plan.leftKey,'id');assert.equal(plan.rightKey,'user_id')
})
test('low cardinality uses allocation-free nested-loop plan',()=>{
 const plan=chooseRelationalPlan(selection(eq('users.id','orders.user_id')),left(2),left(3))
 assert.equal(plan.join,'nested');assert.equal(plan.leftKey,'id')
})
test('extracts equality from safe conjunctions and swapped operands',()=>{
 const predicate=binary('&&',eq('orders.user_id','users.id'),binary('>=',col('orders.price'),lit(10)))
 const plan=chooseRelationalPlan(selection(predicate),left(100),left(100))
 assert.equal(plan.join,'hash');assert.equal(plan.leftKey,'id');assert.equal(plan.rightKey,'user_id')
})
test('unsafe ON arithmetic keeps full evaluation and nested loop',()=>{
 const predicate=binary('&&',eq('id','orders.user_id'),binary('>',binary('/',col('orders.price'),lit(0)),lit(1)))
 const plan=chooseRelationalPlan(selection(predicate),left(100),left(100))
 assert.equal(plan.join,'nested');assert.equal(plan.leftKey,null)
})
test('only pure, base-only WHERE expressions qualify for pre-join filtering',()=>{
 const good=binary('&&',eq('users.id','users.id'),binary('>',col('id'),lit(0)))
 assert.equal(chooseRelationalPlan(selection(eq('id','orders.user_id'),good,'left'),left(20),left(20)).pushWhere,true)
 const joined=eq('orders.user_id','id')
 assert.equal(chooseRelationalPlan(selection(eq('id','orders.user_id'),joined),left(20),left(20)).pushWhere,false)
 assert.equal(baseOnly(joined,'users'),false)
 assert.equal(safePredicate(binary('/',lit(2),lit(0))),false)
})
test('invalid join form does not infer an optimization',()=>{
 const plan=chooseRelationalPlan(selection(binary('||',eq('users.id','orders.user_id'),eq('users.id','orders.id'))),left(50),left(50))
 assert.equal(plan.join,'nested');assert.equal(plan.leftKey,null)
})
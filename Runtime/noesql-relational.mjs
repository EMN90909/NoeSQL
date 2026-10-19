import { chooseRelationalPlan } from './noesql-query-planner.mjs'
// Experimental, bounded relational SELECT executor. It is a conformance
// reference, NOT a native database engine or a performance claim.
const MAX_PAIRS=1_000_000
const MAX_ROWS=100_000
export function relationalSelect(s,db,params,types,validateOnly,{fail,infer,value,fieldType}) {
  const base=db.tables[s.table]
  if(!Object.hasOwn(db.tables,s.table))fail('NOE100',s.at,'unknown table '+s.table)
  const fields=Object.fromEntries(base.fields.map(f=>[f.name,f]))
  for(const [k,f] of Object.entries({...fields}))fields[s.table+'.'+k]=f
  let other=null
  if(s.join){
    if(s.join.table===s.table)fail('NOE230',s.join.at,'self-joins require aliases (not supported)')
    if(!Object.hasOwn(db.tables,s.join.table))fail('NOE100',s.join.at,'unknown joined table '+s.join.table)
    other=db.tables[s.join.table]
    for(const f of other.fields)fields[s.join.table+'.'+f.name]=f
    if(infer(s.join.on,fields,types)!=='bool')fail('NOE231',s.join.at,'join on must be boolean')
  }
  if(s.where&&infer(s.where,fields,types)!=='bool')fail('NOE124',s.where.at,'where must be boolean')
  const groups=s.group||[]
  if(new Set(groups).size!==groups.length)fail('NOE232',s.at,'duplicate group column')
  for(const g of groups)if(!Object.hasOwn(fields,g))fail('NOE120',s.at,'unknown group field '+g)
  const aggregate=groups.length>0||s.columns.some(p=>typeof p==='object')
  if(aggregate&&s.columns.includes('*'))fail('NOE233',s.at,'group cannot select *')
  const output=new Set()
  for(const p of s.columns){
    if(p==='*')continue
    const name=typeof p==='string'?p:p.alias
    if(output.has(name))fail('NOE234',s.at,'duplicate output name '+name)
    output.add(name)
    if(typeof p==='string'){
      if(!Object.hasOwn(fields,p))fail('NOE119',s.at,'unknown projection '+p)
      if(aggregate&&!groups.includes(p))fail('NOE235',s.at,'projection '+p+' must be grouped')
    }else{
      if(p.arg!==null&&p.arg!=='*'&&!Object.hasOwn(fields,p.arg))fail('NOE120',p.at,'unknown aggregate field '+p.arg)
      if(p.fn==='sum'&&fieldType(fields[p.arg])!=='int')fail('NOE236',p.at,'sum requires int column')
    }
  }
  if(s.having){
    if(!aggregate)fail('NOE245',s.having.at,'having requires grouping or aggregation')
    const columns={}
    for(const p of s.columns){
      if(typeof p==='string')columns[p]=fields[p]
      else columns[p.alias]={type:p.fn==='count'?'int':p.fn==='sum'?'int':fieldType(fields[p.arg])}
    }
    if(infer(s.having,columns,types)!=='bool')fail('NOE246',s.having.at,'having must be boolean')
  }
  if(s.sort){
    if(aggregate&&!output.has(s.sort))fail('NOE237',s.at,'grouped sort must name an output column or alias')
    if(!aggregate&&!Object.hasOwn(fields,s.sort))fail('NOE119',s.at,'unknown sort field '+s.sort)
  }
  for(const limit of [s.skip,s.take])if(limit&&infer(limit,{},types)!=='int')fail('NOE124',limit.at,'skip/take must be integer constants or parameters')
  if(validateOnly)return
  // Choose a bounded cardinality-aware join and recheck every ON candidate.
  const plan=chooseRelationalPlan(s,base,other)
  const joinKey=plan.leftKey!==null?[plan.leftKey,plan.rightKey]:null
  let hash=null
  if(plan.join==='hash'){
    hash=new Map()
    for(const row of other.rows){
      const v=row[joinKey[1]]
      if(v===null)continue
      const key=JSON.stringify([typeof v,v])
      const bucket=hash.get(key)
      if(bucket)bucket.push(row)
      else hash.set(key,[row])
    }
  }
  // Aggregate on the fly: O(distinct groups) state, not O(join output) rows.
  // This remains a bounded reference executor, not a cost-based native optimizer.
  const rows=[],buckets=new Map()
  const startValues=()=>s.columns.map(p=>typeof p==='string'?null:p.fn==='count'?0:null)
  if(aggregate&&!groups.length)buckets.set('[]',{first:null,values:startValues()})
  let pairs=0,matched=0
  function emit(row){
    if(++matched>MAX_ROWS)fail('NOE239',s.at,'query exceeds 100000 matching rows')
    if(!aggregate){rows.push(row);return}
    const key=JSON.stringify(groups.map(g=>row[g]))
    let state=buckets.get(key)
    if(!state){
      if(buckets.size===MAX_ROWS)fail('NOE239',s.at,'query exceeds 100000 groups')
      state={first:row,values:startValues()}
      buckets.set(key,state)
    }
    if(state.first===null)state.first=row
    for(let i=0;i<s.columns.length;i++){
      const p=s.columns[i]
      if(typeof p==='string')continue
      const val=p.arg===null||p.arg==='*'?1:row[p.arg]
      if(p.fn==='count'){
        if(val!==null)state.values[i]++
      }else if(val!==null){
        const old=state.values[i]
        if(p.fn==='sum'){
          const sum=(old===null?0:old)+val
          if(!Number.isSafeInteger(sum))fail('NOE212',p.at,'sum integer overflow')
          state.values[i]=sum
        }else if(old===null||(p.fn==='min'?val<old:val>old))state.values[i]=val
      }
    }
  }
  const pushdown=plan.pushWhere
  for(const source of base.rows){
    const baseRow={...source}
    for(const [k,v] of Object.entries(source))baseRow[s.table+'.'+k]=v
    if(pushdown&&value(s.where,baseRow,params)!==true)continue
    if(other){
      const leftValue=joinKey?source[joinKey[0]]:null
      const candidates=plan.join==='hash'?(leftValue===null?[]:(hash.get(JSON.stringify([typeof leftValue,leftValue]))||[])):other.rows
      let matchedJoin=false
      for(const target of candidates){
        if(++pairs>MAX_PAIRS)fail('NOE238',s.at,'join exceeds one million evaluated row pairs')
        const joined={...baseRow}
        for(const [k,v] of Object.entries(target))joined[s.join.table+'.'+k]=v
        if(value(s.join.on,joined,params)!==true)continue
        matchedJoin=true
        if(!pushdown&&s.where&&value(s.where,joined,params)!==true)continue
        emit(joined)
      }
      if(s.join.type==='left'&&!matchedJoin){
        const joined={...baseRow}
        for(const field of other.fields)joined[s.join.table+'.'+field.name]=null
        if(pushdown||!s.where||value(s.where,joined,params)===true)emit(joined)
      }
    }else if(!s.where||value(s.where,baseRow,params)===true)emit(baseRow)
  }
  let result
  if(aggregate){
    result=[]
    for(const state of buckets.values()){
      const out={}
      for(let i=0;i<s.columns.length;i++){
        const p=s.columns[i]
        if(typeof p==='string')out[p]=state.first[p]
        else out[p.alias]=state.values[i]
      }
      if(!s.having||value(s.having,out,params)===true)result.push(out)
    }
  }else{
    result=rows
    if(s.sort){
      const pk=base.fields.find(f=>f.flags.includes('key')).name
      const fk=other?.fields.find(f=>f.flags.includes('key'))?.name
      result=result.slice().sort((a,b)=>{
        const x=a[s.sort],y=b[s.sort]
        if(x!==y)return x===null?1:y===null?-1:x<y?-1:1
        if(a[pk]!==b[pk])return a[pk]<b[pk]?-1:1
        if(fk){
          const ka=a[s.join.table+'.'+fk],kb=b[s.join.table+'.'+fk]
          return ka===kb?0:ka===null?1:kb===null?-1:ka<kb?-1:1
        }
        return 0
      })
    }
    result=result.map(r=>s.columns[0]==='*'
      ? Object.fromEntries(Object.entries(r).filter(([k])=>!k.includes('.')))
      : Object.fromEntries(s.columns.map(c=>[c,r[c]])))
  }
  if(aggregate&&s.sort)result.sort((a,b)=>{
    const x=a[s.sort],y=b[s.sort]
    return x===y?0:x===null?1:y===null?-1:x<y?-1:1
  })
  const skip=s.skip?value(s.skip,{},params):0
  const take=s.take?value(s.take,{},params):result.length
  if(!Number.isSafeInteger(skip)||!Number.isSafeInteger(take)||skip<0||take<0)
    fail('NOE206',s.at,'skip/take must be nonnegative integers')
  return {kind:'get',rows:structuredClone(result.slice(skip,skip+take))}
}

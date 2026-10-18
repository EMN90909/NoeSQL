// Minimal server-side HTTP adapter. NEVER expose NoeSQL credentials in browser JS.
// Authentication is scoped to API-key roles; browser identity must be verified by
// the host application before reaching this handler.
import {createHash,timingSafeEqual} from 'node:crypto'
const hash = value => createHash('sha256').update(value).digest()
const JSON_HEADERS = {'content-type':'application/json; charset=utf-8','cache-control':'no-store','x-content-type-options':'nosniff'}
function reply(res,code,body){res.writeHead(code,JSON_HEADERS);res.end(JSON.stringify(body))}
function same(a,b){return a.length===b.length&&timingSafeEqual(a,b)}
export function createNoeSQLHttpHandler({database,keys=[],maxBodyBytes=64*1024,requestsPerMinute=120}={}){
  if(!database||typeof database.query!=='function'||typeof database.transaction!=='function')throw Error('NOE-WEB: embedded database required')
  if(!Array.isArray(keys)||keys.length===0||keys.length>20||keys.some(k=>typeof k.token!=='string'||k.token.length<32||!['reader','writer'].includes(k.role)||(k.tables!==undefined&&(!Array.isArray(k.tables)||k.tables.length===0||k.tables.length>100||k.tables.some(t=>typeof t!=='string'||!/^[A-Za-z_][A-Za-z0-9_]*$/.test(t))))))throw Error('NOE-WEB: specify strong server-side reader/writer API keys')
  if(!Number.isInteger(maxBodyBytes)||maxBodyBytes<1024||maxBodyBytes>1048576)throw Error('NOE-WEB: invalid body limit')
  if(!Number.isInteger(requestsPerMinute)||requestsPerMinute<1||requestsPerMinute>100000)throw Error('NOE-WEB: invalid request rate')
  const credentials=keys.map(({token,role,tables})=>({digest:hash(token),role,tables:tables?new Set(tables):null}))
  const uses=new Map()
  return async function handler(req,res){
    try{
      res.setHeader('X-Content-Type-Options','nosniff')
      if(req.method==='GET'&&req.url==='/health'){reply(res,200,{ok:true});return}
      if(req.url!=='/v1/query'&&req.url!=='/v1/mutate'){reply(res,404,{error:'not_found'});return}
      if(req.method!=='POST'){reply(res,405,{error:'method_not_allowed'});return}
      const auth=/^Bearer ([^\s]{32,1024})$/.exec(req.headers.authorization||'')
      if(!auth){reply(res,401,{error:'unauthorized'});return}
      const digest=hash(auth[1]);let role=null;let index=-1
      for(let i=0;i<credentials.length;i++)if(same(digest,credentials[i].digest)){role=credentials[i].role;index=i}
      if(!role){reply(res,401,{error:'unauthorized'});return}
      if(req.url==='/v1/mutate'&&role!=='writer'){reply(res,403,{error:'forbidden'});return}
      const now=Date.now();const usage=uses.get(index)||{at:now,count:0}
      if(now-usage.at>=60000){usage.at=now;usage.count=0}
      if(++usage.count>requestsPerMinute){reply(res,429,{error:'rate_limited'});return}
      uses.set(index,usage)
      if(!/\bapplication\/json\b/i.test(req.headers['content-type']||'')){reply(res,415,{error:'json_required'});return}
      let bytes=0;const chunks=[]
      for await(const part of req){bytes+=part.length;if(bytes>maxBodyBytes){reply(res,413,{error:'body_too_large'});return}chunks.push(part)}
      let body
      try{body=JSON.parse(Buffer.concat(chunks).toString('utf8'))}catch{reply(res,400,{error:'invalid_json'});return}
      if(!body||Array.isArray(body)||typeof body!=='object'){reply(res,400,{error:'invalid_request'});return}
      try{
        if(req.url==='/v1/query'){
          if(typeof body.query!=='string'){reply(res,400,{error:'query_required'});return}
          const policy=credentials[index].tables
          if(policy && database.plan(body.query,body.params||{}).tables.some(t=>!policy.has(t))){reply(res,403,{error:'forbidden_table'});return}
          const rows=database.query(body.query,body.params||{})
          reply(res,200,{rows})
        }else{
          if(!Array.isArray(body.operations)||body.operations.length===0||body.operations.length>100){reply(res,400,{error:'operations_required'});return}
          const policy=credentials[index].tables
          if(policy && body.operations.some(op=>!policy.has(op?.table))){reply(res,403,{error:'forbidden_table'});return}
          const affected=database.transaction(tx=>body.operations.map(op=>{
            if(!op||typeof op!=='object')throw Error('invalid operation')
            if(op.kind==='insert')return tx.insert(op.table,op.values)
            if(op.kind==='update')return tx.update(op.table,op.values,op.where)
            if(op.kind==='delete')return tx.remove(op.table,op.where)
            throw Error('invalid mutation kind')
          }))
          reply(res,200,{affected})
        }
      }catch(e){reply(res,400,{error:'invalid_query_or_mutation',message:String(e.message).slice(0,300)})}
    }catch{if(!res.writableEnded)reply(res,500,{error:'server_error'})}
  }
}

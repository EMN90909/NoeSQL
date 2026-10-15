// NoeSQL experimental reference executor. This is an independent in-memory
// conformance oracle, NOT the production Noqeri-owned engine or durable .nqd format.
// No external SQL runtime or npm dependency is used.
export class NoeError extends Error {
  constructor(code, token, message) {
    super(`${code} at ${token.line}:${token.col}: ${message}`)
    this.name = 'NoeError'; this.code = code; this.line = token.line; this.column = token.col
  }
}
const START = /[A-Za-z_]/, REST = /[A-Za-z_0-9]/, DIGIT = /[0-9]/
export function lex(source) {
  if (typeof source !== "string" || Buffer.byteLength(source, "utf8") > 1048576) throw new NoeError("NOE005", {line:1,col:1}, "source exceeds 1 MiB limit")
  const tokens = []; let i = 0, line = 1, col = 1, depth = 0
  const MAX_TOKENS = 100000, MAX_DEPTH = 64
  const emit = (value, kind, start, atLine, atCol) => {
    if (tokens.length >= MAX_TOKENS) throw new NoeError("NOE006", {line:atLine,col:atCol}, "too many tokens")
    if (value === "(" || value === "{") { depth++; if (depth > MAX_DEPTH) throw new NoeError("NOE007", {line:atLine,col:atCol}, "nesting exceeds maximum") }
    if (value === ")" || value === "}") depth = Math.max(0, depth - 1)
    tokens.push({ value, kind, start, end: i, line: atLine, col: atCol })
  }
  const advance = () => { const c = source[i++]; if (c === '\n') { line++; col = 1 } else col++; return c }
  while (i < source.length) {
    const start = i, atLine = line, atCol = col, ch = source[i]
    if (ch === ' ' || ch === '\t' || ch === '\r') { advance(); continue }
    if (ch === '\n' || ch === ';') { advance(); emit(';', 'sep', start, atLine, atCol); continue }
    if (source.slice(i, i + 2) === '//') { while (i < source.length && source[i] !== '\n') advance(); continue }
    if (source.slice(i, i + 2) === '/*') {
      advance(); advance()
      while (i < source.length && source.slice(i, i + 2) !== '*/') advance()
      if (i === source.length) throw new NoeError('NOE001', {line:atLine,col:atCol}, 'unterminated comment')
      advance(); advance(); continue
    }
    if (START.test(ch)) { let s = ''; do { s += advance() } while (i < source.length && REST.test(source[i])); emit(s, 'id', start, atLine, atCol); continue }
    if (DIGIT.test(ch)) { let s = ''; do { s += advance() } while (i < source.length && DIGIT.test(source[i])); emit(s, 'int', start, atLine, atCol); continue }
    if (ch === '"') {
      let s = '"'; advance(); let complete = false
      while (i < source.length) {
        const c = advance(); s += c
        if (c === '\\') { if (i === source.length) break; s += advance() }
        else if (c === '"') { complete = true; break }
        else if (c === '\n') break
      }
      if (!complete) throw new NoeError('NOE002', {line:atLine,col:atCol}, 'unterminated string')
      try { emit(JSON.parse(s), 'string', start, atLine, atCol) }
      catch { throw new NoeError('NOE003', {line:atLine,col:atCol}, 'invalid string escape') }
      continue
    }
    const duo = source.slice(i, i + 2)
    if (['==','!=','<=','>=','&&','||'].includes(duo)) { advance(); advance(); emit(duo, 'op', start, atLine, atCol); continue }
    if ('{}():,.?=$+-*/!<>'.includes(ch)) { advance(); emit(ch, 'op', start, atLine, atCol); continue }
    throw new NoeError('NOE004', {line:atLine,col:atCol}, `unexpected character ${JSON.stringify(ch)}`)
  }
  tokens.push({value:'<eof>',kind:'eof',start:i,end:i,line,col})
  return tokens
}
const precedence = {'||':1,'&&':2,'==':3,'!=':3,'<':4,'<=':4,'>':4,'>=':4,'+':5,'-':5,'*':6,'/':6}
export function parse(source) {
  const t = lex(source); let p = 0
  const cur = () => t[p], is = s => cur().value === s, take = () => t[p++]
  const err = (code, message) => { throw new NoeError(code, cur(), message) }
  const eat = s => is(s) ? (take(), true) : false
  const need = s => { if (!eat(s)) err('NOE010', `expected ${s}, found ${cur().value}`) }
  const id = () => { if (cur().kind !== 'id') err('NOE011', 'expected identifier'); if (['__proto__','constructor','prototype'].includes(cur().value)) err('NOE019','reserved identifier'); return take().value }
  const nl = () => { while (is(';')) take() }
  const boundary = () => { if (!is(';') && !is('}') && cur().kind !== 'eof') err('NOE012','expected newline or semicolon'); nl() }
  const expr = (min=0) => {
    let first = take(), left
    if (first.value === '(') { left = expr(); need(')') }
    else if (first.value === '!' || first.value === '-') { left = {k:'unary',op:first.value,arg:expr(7),at:first} }
    else if (first.value === '$') { left = {k:'param',name:id(),at:first} }
    else if (first.kind === 'string') left = {k:'literal',value:first.value,at:first}
    else if (first.kind === 'int') { const value = Number(first.value); if (!Number.isSafeInteger(value)) throw new NoeError('NOE014',first,'integer exceeds safe reference-engine range'); left = {k:'literal',value,at:first} }
    else if (first.value === 'true' || first.value === 'false') left = {k:'literal',value:first.value === 'true',at:first}
    else if (first.value === 'null') left = {k:'literal',value:null,at:first}
    else if (first.kind === 'id' && !['where','sort','take','skip','set','delete','group','get'].includes(first.value)) {
      let name = first.value
      while (eat('.')) name += '.' + id()
      left = {k:'column',name,at:first}
    } else throw new NoeError('NOE015',first,'expected expression')
    while (true) {
      const op = cur().value, priority = precedence[op]
      if (priority === undefined || priority < min) break
      const at = take(); left = {k:'binary',op,left,right:expr(priority+1),at}
    }
    return left
  }
  const fields = () => {
    need('{'); nl(); const out = []
    while (!is('}')) {
      const at = cur(), name = id(); need(':'); let type = id(), ref = null
      if (type === 'ref') ref = id()
      let nullable = eat('?'), flags = []
      while (['required','key','auto','unique'].includes(cur().value)) flags.push(take().value)
      let fallback; if (eat('=')) fallback = expr()
      out.push({name,type,ref,nullable,flags,default:fallback,at})
      if (eat(',')) { nl(); continue }
      boundary()
    }
    need('}'); return out
  }
  const obj = () => {
    need('{'); nl(); const entries = []
    while (!is('}')) {
      const at=cur(), field=id(); need(':'); const value=expr(); entries.push({field,value,at})
      if (eat(',')) { nl(); continue }
      if (!is('}')) boundary()
    }
    need('}'); return entries
  }
  const select = (table, at) => {
    need('get'); const columns = []
    if (eat('*')) columns.push('*')
    else { columns.push(id()); while (eat(',')) columns.push(id()) }
    let where=null, sort=null, skip=null, takeCount=null
    if (eat('where')) where = expr()
    if (eat('sort')) sort = id()
    if (eat('skip')) skip = expr()
    if (eat('take')) takeCount = expr()
    return {k:'get',table,columns,where,sort,skip,take:takeCount,at}
  }
  const stmt = () => {
    nl(); const at = cur()
    if (eat('db')) return {k:'db',name:id(),at}
    if (eat('table')) return {k:'table',name:id(),fields:fields(),at}
    if (eat('index')) {
      const name=id(); need('on'); const table=id(); need('('); const columns=[id()]
      while (eat(',')) columns.push(id()); need(')'); return {k:'index',name,table,columns,at}
    }
    if (eat('query')) {
      const name=id(); need('('); const params=[]
      while (!is(')')) {
        const at=cur(), name=id(); need(':'); const type=id(); const fallback=eat('=')?expr():null
        params.push({name,type,default:fallback,at}); if (!eat(',')) break
      }
      need(')'); need('{'); nl(); const query=stmt(); nl(); need('}')
      if (query.k !== 'get') err('NOE016','named query must be a get statement')
      return {k:'query',name,params,query,at}
    }
    if (eat('atomic')) {
      need('{'); nl(); const body=[]
      while (!is('}')) { if (cur().kind==='eof') err('NOE017','unterminated atomic scope'); body.push(stmt()); boundary() }
      need('}'); return {k:'atomic',body,at}
    }
    const table=id()
    if (eat('add')) return {k:'add',table,entries:obj(),at}
    if (is('get')) return select(table,at)
    if (eat('where')) {
      const where=expr()
      if (eat('delete')) return {k:'delete',table,where,at}
      need('set'); const entries=[]
      do { const at=cur(), field=id(); need('='); entries.push({field,value:expr(),at}) } while (eat(','))
      return {k:'set',table,where,entries,at}
    }
    err('NOE018','expected add, get, or where after table name')
  }
  const statements=[]; nl()
  while (cur().kind !== 'eof') { statements.push(stmt()); boundary() }
  return statements
}
function fail(code, at, message) { throw new NoeError(code,at,message) }
const atUnknown = {line:1,col:1}
const fieldType = f => f.type === 'ref' ? 'int' : f.type
function matches(value, type) { return value===null || (type==='int'?Number.isSafeInteger(value):type==='bool'?typeof value==='boolean':type==='text'?typeof value==='string':false) }
function typeOf(v) { return v===null?'null':typeof v==='number'?'int':typeof v==='boolean'?'bool':typeof v==='string'?'text':'invalid' }
function infer(node, columns, params) {
  if (node.k==='literal') return typeOf(node.value)
  if (node.k==='column') { if (!columns?.[node.name]) fail('NOE120',node.at,`unknown field ${node.name}`); return fieldType(columns[node.name]) }
  if (node.k==='param') { if (!Object.hasOwn(params,node.name)) fail('NOE121',node.at,`unknown parameter $${node.name}`); return params[node.name] }
  if (node.k==='unary') { const t=infer(node.arg,columns,params); if (node.op==='!'&&t==='bool') return 'bool'; if(node.op==='-'&&t==='int') return 'int'; fail('NOE122',node.at,'invalid unary operand') }
  const l=infer(node.left,columns,params),r=infer(node.right,columns,params)
  if (['+','-','*','/'].includes(node.op) && l==='int'&&r==='int') return 'int'
  if (['&&','||'].includes(node.op) && l==='bool'&&r==='bool') return 'bool'
  if (['==','!=','<','<=','>','>='].includes(node.op) && l===r && l!=='null') return 'bool'
  fail('NOE123',node.at,`incompatible ${l} ${node.op} ${r}`)
}
function value(node,row,params) {
  if (node.k==='literal') return node.value
  if (node.k==='column') return row[node.name]
  if (node.k==='param') return params[node.name]
  if (node.k==='unary') { const v=value(node.arg,row,params); return v===null?null:node.op==='!'?!v:-v }
  const l=value(node.left,row,params)
  if (node.op==='&&' && l===false) return false
  if (node.op==='||' && l===true) return true
  const r=value(node.right,row,params)
  if (['&&','||'].includes(node.op)) return node.op==='&&'?(l===false||r===false?false:l===null||r===null?null:true):(l===true||r===true?true:l===null||r===null?null:false)
  if (l===null||r===null) return null
  let result
  switch(node.op) {
    case '==': result=l===r;break;case '!=':result=l!==r;break
    case '<':result=l<r;break;case '<=':result=l<=r;break;case '>':result=l>r;break;case '>=':result=l>=r;break
    case '+':result=l+r;break;case '-':result=l-r;break;case '*':result=l*r;break
    case '/':if(r===0)fail('NOE211',node.at,'division by zero');result=Math.trunc(l/r);break
  }
  if(typeof result==='number'&&!Number.isSafeInteger(result))fail('NOE212',node.at,'integer overflow')
  return result
}
export function emptyDatabase() { return {format:'noesql-reference-1',name:null,version:1,tables:{},indexes:{},queries:{}} }
export function execute(source, database=emptyDatabase(), bindings={}) {
  const program = typeof source==='string'?parse(source):source
  if (database.format!=='noesql-reference-1') fail('NOE200',atUnknown,'unsupported database format')
  const results=[]
  // The caller owns persistence. Execute against a clone, then publish only on success.
  const db=structuredClone(database)
  function tableFor(s) { const t=db.tables[s.table]; if(!t) fail('NOE100',s.at,`unknown table ${s.table}`);return t }
  function fieldsFor(t) { return Object.fromEntries(t.fields.map(f=>[f.name,f])) }
  function validExpr(e,t,params,expected) {const got=infer(e,fieldsFor(t),params);if(expected&&got!==expected&&got!=='null')fail('NOE124',e.at,`expected ${expected}, got ${got}`)}
  function tableConstraints(t,row,old=null,at=atUnknown) {
    for (const f of t.fields) {
      const v=row[f.name]
      if ((v===null||v===undefined) && (f.flags.includes('required')||f.flags.includes('key'))) fail('NOE201',at,`${f.name} is required`)
      if (!matches(v,fieldType(f))) fail('NOE202',at,`invalid ${f.name}: expected ${fieldType(f)}`)
      if (f.flags.includes('unique')||f.flags.includes('key')) for (const x of t.rows) if (x!==old && v!==null && x[f.name]===v) fail('NOE203',at,`duplicate unique value for ${f.name}`)
      if (f.ref && v!==null) {
        const target=db.tables[f.ref]
        if (!target) fail('NOE204',at,`unknown reference ${f.ref}`)
        const key=target.fields.find(g=>g.flags.includes('key'))
        if (!key || !target.rows.some(x=>x[key.name]===v)) fail('NOE205',at,`foreign key ${f.name} refers to missing ${f.ref} row`)
      }
    }
  }
  function apply(s,params,types) {
    if (s.k==='db') { if(db.name&&db.name!==s.name)fail('NOE101',s.at,'cannot change database identity');db.name=s.name;return {kind:'db',name:s.name} }
    if (s.k==='table') {
      if(db.tables[s.name])fail('NOE102',s.at,`duplicate table ${s.name}`)
      const known=new Set();let keys=0,autos=0
      for(const f of s.fields) {
        if(known.has(f.name))fail('NOE103',f.at,`duplicate field ${f.name}`); known.add(f.name)
        if(!['int','text','bool','ref'].includes(f.type))fail('NOE104',f.at,`unsupported type ${f.type}`)
        if(f.flags.includes('key')){keys++;if(f.type!=='int')fail('NOE105',f.at,'reference engine only supports int keys')}
        if(f.flags.includes('auto')){autos++;if(!f.flags.includes('key')||f.type!=='int')fail('NOE106',f.at,'auto requires int key')}
        if(f.default){if(f.ref)fail('NOE107',f.at,'references cannot have defaults');validExpr(f.default,{fields:[]},{},fieldType(f))}
      }
      if(keys!==1||autos>1)fail('NOE108',s.at,'exactly one integer primary key required')
      if(s.fields.length===0||s.fields.length>256)fail('NOE109',s.at,'table requires 1..256 fields')
      db.tables[s.name]={fields:s.fields,rows:[],nextId:1};return {kind:'table',name:s.name}
    }
    if (s.k==='index') {
      const t=tableFor(s);if(db.indexes[s.name])fail('NOE110',s.at,'duplicate index name')
      if(s.columns.length<1||s.columns.length>16||new Set(s.columns).size!==s.columns.length)fail('NOE111',s.at,'invalid index columns')
      for(const c of s.columns)if(!fieldsFor(t)[c])fail('NOE112',s.at,`unknown index field ${c}`)
      db.indexes[s.name]={table:s.table,columns:s.columns};return {kind:'index',name:s.name}
    }
    if (s.k==='query') {
      if(db.queries[s.name])fail('NOE113',s.at,'duplicate query name')
      const declared={};for(const p of s.params){if(Object.hasOwn(declared,p.name))fail('NOE114',p.at,'duplicate parameter');if(!['int','text','bool'].includes(p.type))fail('NOE115',p.at,'unsupported parameter type');declared[p.name]=p.type;if(p.default)validExpr(p.default,{fields:[]},{},p.type)}
      validateStatement(s.query,declared)
      db.queries[s.name]=s;return {kind:'query',name:s.name}
    }
    if(s.k==='atomic') { const out=[];for(const x of s.body)out.push(apply(x,params,types));return {kind:'atomic',results:out} }
    const t=tableFor(s), col=fieldsFor(t), key=t.fields.find(f=>f.flags.includes('key'))
    function assertAssignments(entries,allowKey){let seen=new Set();for(const e of entries){if(seen.has(e.field))fail('NOE116',e.at,'duplicate assignment');seen.add(e.field);if(!col[e.field])fail('NOE117',e.at,`unknown field ${e.field}`);if(!allowKey&&e.field===key.name)fail('NOE118',e.at,'cannot update primary key');validExpr(e.value,t,types,fieldType(col[e.field]))}}
    if (s.k==='add') {
      assertAssignments(s.entries,true)
      const row=Object.fromEntries(t.fields.map(f=>[f.name,f.default?value(f.default,{},params):null]))
      for(const e of s.entries)row[e.field]=value(e.value,{},params)
      if(row[key.name]===null&&key.flags.includes('auto'))row[key.name]=t.nextId
      tableConstraints(t,row,null,s.at)
      t.rows.push(row)
      if(row[key.name]>=t.nextId)t.nextId=row[key.name]+1
      return {kind:'add',affected:1,row:structuredClone(row)}
    }
    if(s.k==='get') {
      for(const c of s.columns)if(c!=='*'&&!col[c])fail('NOE119',s.at,`unknown projection ${c}`)
      if(s.where)validExpr(s.where,t,types,'bool')
      if(s.sort&&!col[s.sort])fail('NOE119',s.at,`unknown sort field ${s.sort}`)
      for(const limit of [s.skip,s.take])if(limit)validExpr(limit,t,types,'int')
      let rows=t.rows.filter(r=>!s.where||value(s.where,r,params)===true)
      if(s.sort)rows=rows.slice().sort((a,b)=>a[s.sort]===b[s.sort]?a[key.name]-b[key.name]:a[s.sort]===null?1:b[s.sort]===null?-1:a[s.sort]<b[s.sort]?-1:1)
      let skip=s.skip?value(s.skip,{},params):0, take=s.take?value(s.take,{},params):rows.length
      if(!Number.isSafeInteger(skip)||!Number.isSafeInteger(take)||skip<0||take<0)fail('NOE206',s.at,'skip/take must be nonnegative integers')
      rows=rows.slice(skip,skip+take)
      return {kind:'get',rows:rows.map(r=>structuredClone(s.columns[0]==='*'?r:Object.fromEntries(s.columns.map(c=>[c,r[c]]))))}
    }
    if(s.k==='set'||s.k==='delete') {
      if(!s.where)fail('NOE207',s.at,'write requires where')
      validExpr(s.where,t,types,'bool')
      if(s.k==='set')assertAssignments(s.entries,false)
      const hit=t.rows.filter(r=>value(s.where,r,params)===true)
      if(s.k==='delete') {
        for(const other of Object.values(db.tables))for(const f of other.fields)if(f.ref===s.table)for(const x of other.rows)for(const r of hit)if(x[f.name]===r[key.name])fail('NOE208',s.at,'delete blocked by reference')
        t.rows=t.rows.filter(x=>!hit.includes(x))
      } else {
        for(const r of hit){const candidate={...r};for(const e of s.entries)candidate[e.field]=value(e.value,r,params);tableConstraints(t,candidate,r,s.at);Object.assign(r,candidate)}
      }
      return {kind:s.k,affected:hit.length}
    }
    fail('NOE999',s.at,'unsupported statement')
  }
  function validateStatement(s,types) {
    if(s.k!=='get')fail('NOE130',s.at,'named query must select')
    const t=tableFor(s),f=fieldsFor(t)
    for(const c of s.columns)if(c!=='*'&&!f[c])fail('NOE119',s.at,`unknown projection ${c}`)
    if(s.where)validExpr(s.where,t,types,'bool')
    if(s.sort&&!f[s.sort])fail('NOE119',s.at,'unknown sort field')
    for(const l of [s.skip,s.take])if(l)validExpr(l,t,types,'int')
  }
  for(const s of program)results.push(apply(s,bindings,Object.fromEntries(Object.entries(bindings).map(([k,v])=>[k,typeOf(v)]))))
  return {database:db,results}
}
export function runNamed(database,name,args={}) {
  const definition=database.queries[name]
  if(!definition)fail('NOE131',atUnknown,`unknown query ${name}`)
  const params={};for(const p of definition.params){const v=Object.hasOwn(args,p.name)?args[p.name]:p.default?value(p.default,{},{}):undefined;if(!matches(v,p.type)||v===undefined)fail('NOE132',p.at,`invalid parameter ${p.name}`);params[p.name]=v}
  for(const k of Object.keys(args))if(!definition.params.some(p=>p.name===k))fail('NOE133',definition.at,`unknown parameter ${k}`)
  return execute([definition.query],database,params).results[0]
}

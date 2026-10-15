// Optional prepared-statement interoperability for SQLite and PostgreSQL.
// Does not participate in the native NoeSQL .nqd storage/executor pipeline.
// Language parsing lives in the reference grammar oracle until native compilation is verified.
import { parse, NoeError } from './noesql-reference.mjs'

const IDENT = /^[A-Za-z_][A-Za-z0-9_]*$/
const at = { line: 1, col: 1 }
function fault(code, node, msg) { throw new NoeError(code, node?.at || at, msg) }
function quote(name) { if (!IDENT.test(name) || ['constructor','prototype','__proto__'].includes(name)) fault('NOE400',null,`invalid SQL identifier ${name}`); return `"${name}"` }
const typeOf = value => value===null ? 'null' : typeof value==='boolean' ? 'bool' : Number.isSafeInteger(value) ? 'int' : typeof value==='string' ? 'text' : 'invalid'
const nativeType = x => x.type==='ref' ? 'int' : x.type
function fieldMap(table) { return Object.fromEntries(table.fields.map(f=>[f.name,f])) }
function validateType(expr,fields,types) {
  if(expr.k==='literal') return typeOf(expr.value)
  if(expr.k==='param') { if(!Object.hasOwn(types,expr.name))fault('NOE401',expr,`unknown parameter $${expr.name}`); return types[expr.name] }
  if(expr.k==='column') { const f=fields[expr.name];if(!f)fault('NOE402',expr,`unknown column ${expr.name}`);return nativeType(f) }
  if(expr.k==='unary') {const t=validateType(expr.arg,fields,types);if(expr.op==='!'&&t==='bool')return 'bool';if(expr.op==='-'&&t==='int')return 'int';fault('NOE403',expr,'invalid unary expression')}
  if(expr.k==='binary') {
    const l=validateType(expr.left,fields,types),r=validateType(expr.right,fields,types)
    if(['&&','||'].includes(expr.op)&&l==='bool'&&r==='bool')return 'bool'
    if(['+','-','*','/'].includes(expr.op)&&l==='int'&&r==='int')return 'int'
    if(['==','!=','<','<=','>','>='].includes(expr.op)&&(l===r||l==='null'||r==='null'))return 'bool'
    fault('NOE404',expr,`incompatible ${l} ${expr.op} ${r}`)
  }
  fault('NOE405',expr,'unsupported expression')
}
function expect(expr,fields,types,type) {const actual=validateType(expr,fields,types);if(actual!==type&&actual!=='null')fault('NOE406',expr,`expected ${type}; got ${actual}`)}
export function createSqlCompiler(dialect='sqlite',existing={}) {
  if(!['sqlite','postgres'].includes(dialect))throw Error(`unsupported SQL dialect ${dialect}`)
  const tables=Object.assign(Object.create(null),existing), named=Object.create(null)
  function compileExpression(node,ctx) {
    if(node.k==='column')return quote(node.name)
    if(node.k==='literal'||node.k==='param') {
      const value=node.k==='literal'?node.value:ctx.params[node.name]
      if(node.k==='param'&&!Object.hasOwn(ctx.params,node.name))fault('NOE407',node,`missing binding $${node.name}`)
      if(!['int','text','bool','null'].includes(typeOf(value)))fault('NOE408',node,'invalid bound parameter type')
      ctx.values.push(dialect==='sqlite'&&typeof value==='boolean'?Number(value):value)
      return dialect==='sqlite'?'?':`$${ctx.values.length}`
    }
    if(node.k==='unary')return `(${node.op==='!'?'NOT ':node.op}${compileExpression(node.arg,ctx)})`
    if(node.k==='binary') {
      const left=compileExpression(node.left,ctx),right=compileExpression(node.right,ctx)
      const op=({'&&':'AND','||':'OR','==':'=','!=':'<>'})[node.op]||node.op
      // No implicit arbitrary SQL strings; AST operators are enumerated by the parser.
      if(!['AND','OR','=','<>','<','<=','>','>=','+','-','*','/'].includes(op))fault('NOE409',node,'unsupported SQL operator')
      return `(${left} ${op} ${right})`
    }
    fault('NOE410',node,'unsupported expression')
  }
  function valuesFor(node,parameters) {
    const ctx={params:parameters,values:[]};return {text:compileExpression(node,ctx),params:ctx.values}
  }
  function tableFor(stmt) {const table=tables[stmt.table];if(!table)fault('NOE411',stmt,`unknown table ${stmt.table}`);return table}
  function compile(stmt,parameters={},parameterTypes=Object.fromEntries(Object.entries(parameters).map(([k,v])=>[k,typeOf(v)]))) {
    if(stmt.k==='db')return {kind:'db',name:stmt.name}
    if(stmt.k==='atomic')return {kind:'atomic',statements:stmt.body.map(s=>compile(s,parameters,parameterTypes))}
    if(stmt.k==='table') {
      if(tables[stmt.name])fault('NOE412',stmt,`duplicate table ${stmt.name}`)
      if(!stmt.fields.length||stmt.fields.length>256)fault('NOE413',stmt,'invalid field count')
      const seen=new Set(), keys=stmt.fields.filter(f=>f.flags.includes('key'))
      if(keys.length!==1||keys[0].type!=='int')fault('NOE414',stmt,'one int primary key required')
      const fieldSql=[]
      for(const f of stmt.fields) {
        if(seen.has(f.name))fault('NOE415',f,'duplicate column');seen.add(f.name)
        if(!['int','bool','text','ref'].includes(f.type))fault('NOE416',f,`unsupported type ${f.type}`)
        if(f.flags.includes('auto')&&(!f.flags.includes('key')||f.type!=='int'))fault('NOE417',f,'auto requires an int key')
        const reference=f.type==='ref'?tables[f.ref]:null
        if(f.type==='ref'&&!reference)fault('NOE418',f,`reference to unknown table ${f.ref}`)
        const primary=f.flags.includes('key'), auto=f.flags.includes('auto')
        let base=primary&&auto?(dialect==='sqlite'?'INTEGER PRIMARY KEY AUTOINCREMENT':'BIGSERIAL PRIMARY KEY'):
          `${f.type==='text'?'TEXT':f.type==='bool'?'BOOLEAN':'BIGINT'}${primary?' PRIMARY KEY':''}`
        if(!primary&&f.flags.includes('required'))base+=' NOT NULL'
        if(!primary&&f.flags.includes('unique'))base+=' UNIQUE'
        if(f.type==='bool')base+=dialect==='sqlite'?` CHECK (${quote(f.name)} IN (0,1))`:''
        if(f.default){if(f.default.k!=='literal')fault('NOE419',f,'SQL defaults must be constants');const v=f.default.value;if(typeOf(v)!==nativeType(f))fault('NOE420',f,'bad default type');base+=' DEFAULT '+(typeof v==='string'?`'${v.replaceAll("'","''")}'`:typeof v==='boolean'?(v?'TRUE':'FALSE'):v)}
        if(reference){const pk=reference.fields.find(g=>g.flags.includes('key'));base+=` REFERENCES ${quote(f.ref)}(${quote(pk.name)}) ON DELETE RESTRICT`}
        fieldSql.push(`${quote(f.name)} ${base}`)
      }
      const sql=`CREATE TABLE ${quote(stmt.name)} (${fieldSql.join(', ')})`
      tables[stmt.name]={fields:stmt.fields}
      return {kind:'table',sql,values:[]}
    }
    if(stmt.k==='index') {
      const table=tableFor(stmt), map=fieldMap(table)
      if(!stmt.columns.length||stmt.columns.length>16||new Set(stmt.columns).size!==stmt.columns.length)fault('NOE421',stmt,'invalid index definition')
      for(const c of stmt.columns)if(!map[c])fault('NOE422',stmt,`unknown index column ${c}`)
      return {kind:'index',sql:`CREATE INDEX ${quote(stmt.name)} ON ${quote(stmt.table)} (${stmt.columns.map(quote).join(', ')})`,values:[]}
    }
    if(stmt.k==='query') {
      if(named[stmt.name])fault('NOE423',stmt,'duplicate named query')
      const defaults=Object.create(null),types=Object.create(null)
      for(const p of stmt.params){if(Object.hasOwn(types,p.name))fault('NOE424',p,'duplicate query parameter');if(!['int','text','bool'].includes(p.type))fault('NOE425',p,'invalid parameter type');types[p.name]=p.type;if(p.default){if(p.default.k!=='literal'||typeOf(p.default.value)!==p.type)fault('NOE426',p,'bad query default');defaults[p.name]=p.default.value}}
      // Fully validate references and types without executing any query.
      compile(stmt.query,Object.fromEntries(Object.entries(types).map(([k,t])=>[k, t==='int'?0:t==='bool'?false:''])),types)
      named[stmt.name]={stmt,defaults,types};return {kind:'query',name:stmt.name}
    }
    const table=tableFor(stmt),fields=fieldMap(table)
    const ctx={params:parameters,values:[]}
    if(stmt.k==='add') {
      if(!stmt.entries.length)fault('NOE427',stmt,'insert needs columns')
      const names=[],expressions=[],seen=new Set()
      for(const entry of stmt.entries){const f=fields[entry.field];if(!f)fault('NOE428',entry,`unknown column ${entry.field}`);if(seen.has(entry.field))fault('NOE429',entry,'duplicate assignment');seen.add(entry.field);expect(entry.value,{},parameterTypes,nativeType(f));names.push(quote(entry.field));expressions.push(compileExpression(entry.value,ctx))}
      return {kind:'add',table:stmt.table,sql:`INSERT INTO ${quote(stmt.table)} (${names.join(', ')}) VALUES (${expressions.join(', ')}) RETURNING *`,values:ctx.values}
    }
    if(stmt.k==='get') {
      for(const c of stmt.columns)if(c!=='*'&&!fields[c])fault('NOE430',stmt,`unknown projection ${c}`)
      let sql=`SELECT ${stmt.columns.map(x=>x==='*'?'*':quote(x)).join(', ')} FROM ${quote(stmt.table)}`
      if(stmt.where){expect(stmt.where,fields,parameterTypes,'bool');sql+=' WHERE '+compileExpression(stmt.where,ctx)}
      if(stmt.sort){
        if(!fields[stmt.sort])fault('NOE431',stmt,`unknown sort column ${stmt.sort}`)
        const pk=table.fields.find(f=>f.flags.includes('key')).name
        sql+=' ORDER BY ('+quote(stmt.sort)+' IS NULL), '+quote(stmt.sort)
        if(stmt.sort!==pk)sql+=', '+quote(pk)
      }
      if(stmt.skip||stmt.take){if(stmt.take){expect(stmt.take,{},parameterTypes,'int');sql+=' LIMIT '+compileExpression(stmt.take,ctx)}else sql+=dialect==='sqlite'?' LIMIT -1':' LIMIT ALL';if(stmt.skip){expect(stmt.skip,{},parameterTypes,'int');sql+=' OFFSET '+compileExpression(stmt.skip,ctx)}}
      return {kind:'get',table:stmt.table,sql,values:ctx.values}
    }
    if(stmt.k==='set'||stmt.k==='delete') {
      if(!stmt.where)fault('NOE432',stmt,'write requires a where predicate')
      if(stmt.k==='set') {
        if(!stmt.entries.length)fault('NOE433',stmt,'update requires assignments')
        const assignments=[],seen=new Set()
        for(const entry of stmt.entries){const f=fields[entry.field];if(!f)fault('NOE434',entry,`unknown column ${entry.field}`);if(f.flags.includes('key'))fault('NOE435',entry,'primary key update forbidden');if(seen.has(entry.field))fault('NOE436',entry,'duplicate assignment');seen.add(entry.field);expect(entry.value,fields,parameterTypes,nativeType(f));assignments.push(`${quote(entry.field)} = ${compileExpression(entry.value,ctx)}`)}
        expect(stmt.where,fields,parameterTypes,'bool');return {kind:'set',sql:`UPDATE ${quote(stmt.table)} SET ${assignments.join(', ')} WHERE ${compileExpression(stmt.where,ctx)}`,values:ctx.values}
      }
      expect(stmt.where,fields,parameterTypes,'bool');return {kind:'delete',sql:`DELETE FROM ${quote(stmt.table)} WHERE ${compileExpression(stmt.where,ctx)}`,values:ctx.values}
    }
    fault('NOE437',stmt,`unsupported statement ${stmt.k}`)
  }
  function namedQuery(name,inputs={}) {
    const definition=named[name];if(!definition)fault('NOE438',null,`unknown query ${name}`)
    for(const k of Object.keys(inputs))if(!Object.hasOwn(definition.types,k))fault('NOE439',null,`unknown query argument ${k}`)
    const params={...definition.defaults,...inputs}
    for(const [k,type] of Object.entries(definition.types))if(!Object.hasOwn(params,k)||typeOf(params[k])!==type)fault('NOE440',null,`missing/invalid query argument ${k}`)
    return compile(definition.stmt.query,params,definition.types)
  }
  return {compile, namedQuery, tables, named, parse}
}

// SQLite and PostgreSQL are optional compatibility targets; NoeSQL .nqd
// uses its own reference/native storage and has no dependency on either engine.
export function createSqliteAdapter(databaseFile=':memory:') {
  return import('node:sqlite').then(({DatabaseSync})=> {
    const db=new DatabaseSync(databaseFile)
    db.exec('PRAGMA foreign_keys=ON')
    const compiler=createSqlCompiler('sqlite')
    let ended=false
    function checked(){if(ended)throw Error('SQLite adapter is closed')}
    function executePlan(plan) {
      if(plan.kind==='atomic')return {kind:'atomic',results:plan.statements.map(executePlan)}
      if(!plan.sql)return plan
      const statement=db.prepare(plan.sql)
      if(plan.kind==='get'||plan.kind==='add') {
        const columnTypes=fieldMap(compiler.tables[plan.table])
        const rows=statement.all(...plan.values).map(x=>Object.fromEntries(Object.entries(x).map(([k,v])=> {
          if(typeof v==='bigint') {if(v>BigInt(Number.MAX_SAFE_INTEGER)||v<BigInt(Number.MIN_SAFE_INTEGER))throw Error('NOE441: SQLite integer exceeds safe NoeSQL range');v=Number(v)}
          if(columnTypes[k]?.type==='bool'&&v!==null)v=Boolean(v)
          return [k,v]
        })))
        return {kind:plan.kind,rows}
      }
      const result=statement.run(...plan.values)
      return {kind:plan.kind,affected:Number(result.changes||0)}
    }
    function run(source,params={}) {
      checked();const code=typeof source==='string'?parse(source):source
      const tableNames=new Set(Object.keys(compiler.tables)), queryNames=new Set(Object.keys(compiler.named))
      db.exec('BEGIN IMMEDIATE')
      try {const out=code.map(x=>executePlan(compiler.compile(x,params)));db.exec('COMMIT');return out}
      catch(e){db.exec('ROLLBACK');for(const key of Object.keys(compiler.tables))if(!tableNames.has(key))delete compiler.tables[key];for(const key of Object.keys(compiler.named))if(!queryNames.has(key))delete compiler.named[key];throw e}
    }
    function query(name,params={}){checked();const plan=compiler.namedQuery(name,params);return executePlan(plan)}
    return {run,query,db,close(){if(!ended){db.close();ended=true}},compiler}
  })
}

export async function createPostgresAdapter(config) {
  let Client
  try {({Client}=await import('pg'))}catch{throw Error('PostgreSQL compatibility requires optional npm package "pg"; install it explicitly')}
  const client=new Client(config);await client.connect()
  const compiler=createSqlCompiler('postgres')
  let ended=false
  async function executePlan(plan) {
    if(plan.kind==='atomic'){const results=[];for(const stmt of plan.statements)results.push(await executePlan(stmt));return {kind:'atomic',results}}
    if(!plan.sql)return plan
    const result=await client.query(plan.sql,plan.values)
    return {kind:plan.kind,...(['get','add'].includes(plan.kind)?{rows:result.rows}:{affected:result.rowCount})}
  }
  async function run(source,params={}) {
    if(ended)throw Error('PostgreSQL adapter is closed')
    const program=typeof source==='string'?parse(source):source
    const tableNames=new Set(Object.keys(compiler.tables)), queryNames=new Set(Object.keys(compiler.named))
    await client.query('BEGIN')
    try {const out=[];for(const x of program)out.push(await executePlan(compiler.compile(x,params)));await client.query('COMMIT');return out}
    catch(e){await client.query('ROLLBACK');for(const key of Object.keys(compiler.tables))if(!tableNames.has(key))delete compiler.tables[key];for(const key of Object.keys(compiler.named))if(!queryNames.has(key))delete compiler.named[key];throw e}
  }
  return {run,query:(name,params={})=>executePlan(compiler.namedQuery(name,params)),close:async()=>{if(!ended){await client.end();ended=true}},compiler}
}

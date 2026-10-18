// NoeSQL readable reporting/query compiler for the optional SQLite embedded backend.
// No SQL fragments from callers are ever interpolated: identifiers are validated
// against the live schema and values are bound through prepared statements.
const IDENT = /^[A-Za-z_][A-Za-z0-9_]*$/
const NAME = /^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?$/
const COL_ALIAS = /^([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?)(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?$/i
const AGG = /^(count|sum|avg|min|max)\((\*|[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?)\)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)$/i
const FILTER = /^([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?)\s*(==|!=|>=|<=|>|<|is null|is not null)\s*(?:\$([A-Za-z_][A-Za-z0-9_]*))?$/i
const JOIN = /^(?:(left|right|full|inner)\s+)?join\s+([A-Za-z_][A-Za-z0-9_]*)\s+on\s+([A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*)\s*==\s*([A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*)$/i
const MAX_CHARS = 64 * 1024
const quote = s => '"' + s.replaceAll('"', '""') + '"'
function expect(ok, message) { if (!ok) throw new Error(`NOE-QUERY: ${message}`) }
function id(s) { expect(IDENT.test(s) && !s.toLowerCase().startsWith('sqlite_') && !s.toLowerCase().startsWith('_noesql_'), `invalid identifier ${s}`); return s }
function components(s) { expect(NAME.test(s), `invalid column ${s}`); return s.split('.').map(id) }
function columnsOf(db, table) {
  id(table)
  const rows = db.prepare(`PRAGMA table_info(${quote(table)})`).all()
  expect(rows.length > 0, `unknown table ${table}`)
  return new Set(rows.map(r => r.name))
}
function getSchema(db, tables) {
  return new Map([...tables].map(table => [table, columnsOf(db, table)]))
}
function resolveColumn(token, schema, defaultTable) {
  const parts = components(token)
  if (parts.length === 2) {
    const [table, col] = parts
    expect(schema.get(table)?.has(col), `unknown column ${token}`)
    return `${quote(table)}.${quote(col)}`
  }
  const found = [...schema].filter(([, cols]) => cols.has(parts[0]))
  expect(found.length === 1, found.length ? `ambiguous column ${token}; qualify it` : `unknown column ${token}`)
  return schema.size === 1 && defaultTable === found[0][0]
    ? quote(parts[0]) : `${quote(found[0][0])}.${quote(parts[0])}`
}
function splitList(text) {
  const bits = text.split(/,(?![^()]*\))/).map(s => s.trim())
  expect(bits.length > 0 && bits.every(Boolean) && bits.length <= 64, 'invalid or excessive column list')
  return bits
}
function bindParams(names, params) {
  const bound = []
  for (const name of names) {
    expect(Object.hasOwn(params, name), `missing parameter $${name}`)
    const v = params[name]
    expect(v === null || typeof v === 'string' || typeof v === 'boolean' || (typeof v === 'number' && Number.isFinite(v) && Number.isSafeInteger(v)), `invalid parameter $${name}`)
    if(typeof v === 'string')expect(Buffer.byteLength(v,'utf8')<=256*1024,`parameter $${name} too large`)
    bound.push(typeof v === 'boolean' ? Number(v) : v)
  }
  return bound
}
function parseFilter(text, schema, aliases, params, bindings, allowAliases = false) {
  const parts = text.split(/\s+and\s+/i)
  expect(parts.length <= 16, 'too many predicates')
  const predicates = []
  for (const part of parts) {
    const match = FILTER.exec(part.trim())
    expect(match, `invalid predicate ${part.trim()}`)
    const [, column, op, name] = match
    let expr
    if (allowAliases && aliases.has(column)) expr = quote(column)
    else expr = resolveColumn(column, schema)
    if (/^is (?:not )?null$/i.test(op)) {
      expect(name === undefined, 'IS NULL does not accept a parameter')
      predicates.push(`${expr} ${op.toUpperCase()}`)
    } else {
      expect(name, 'comparisons must use named parameters ($name)')
      bindings.push(name)
      predicates.push(`${expr} ${op === '==' ? '=' : op === '!=' ? '<>' : op} ?`)
    }
  }
  return predicates.join(' AND ')
}
/**
 * Readable query grammar, one clause per line:
 *   from tickets
 *   left join teams on tickets.team_id == teams.id
 *   select teams.name as team, count(tickets.id) as tickets
 *   where tickets.closed == $closed
 *   group teams.name
 *   having tickets >= $minimum
 *   sort tickets desc
 *   take 20
 * SQL parameters are required for predicate values. No arbitrary SQL accepted.
 */
export function compileAdvanced(db, source, params = {}, opts = {}) {
  expect(typeof source === 'string' && source.length > 0 && Buffer.byteLength(source, 'utf8') <= MAX_CHARS, 'query source must be 1..65536 bytes')
  expect(params && Object.getPrototypeOf(params) === Object.prototype, 'parameters must be a plain object')
  const maxRows = opts.maxRows ?? 500
  expect(Number.isSafeInteger(maxRows) && maxRows > 0 && maxRows <= 10000, 'invalid maximum rows')
  const clauses = source.split(/\r?\n/).map(line => line.trim()).filter(line => line && !line.startsWith('#'))
  expect(clauses.length > 0 && clauses.length <= 40, 'invalid number of clauses')
  const first = /^from\s+([A-Za-z_][A-Za-z0-9_]*)$/i.exec(clauses.shift())
  expect(first, 'query must start with `from table`')
  const base = id(first[1]); const joinDefs = []; const tableNames = new Set([base]); const other = new Map()
  for (const line of clauses) {
    const j = JOIN.exec(line)
    if (!j) continue
    expect(joinDefs.length < 4 && !tableNames.has(j[2]), 'too many joins or repeated table')
    const table = id(j[2]); tableNames.add(table)
    joinDefs.push({kind: (j[1] || 'inner').toUpperCase(), table, left: j[3], right: j[4]})
    other.set(line, true)
  }
  const schema = getSchema(db, tableNames)
  // Joins must connect the newly introduced table to one already in scope.
  const reachable = new Set([base]);const joins = []
  for (const j of joinDefs) {
    const l = components(j.left), r = components(j.right)
    expect(l.length === 2 && r.length === 2 && ((l[0] === j.table && reachable.has(r[0])) || (r[0] === j.table && reachable.has(l[0]))), `join ${j.table} must connect to a preceding table`)
    joins.push(`${j.kind} JOIN ${quote(j.table)} ON ${resolveColumn(j.left, schema)} = ${resolveColumn(j.right, schema)}`)
    reachable.add(j.table)
  }
  let select = null, where = [], group = null, having = [], order = null, take = null, skip = null, distinct = false
  const paramNames = [];const aliases = new Set(); const unaggregated = [];let hasAggregates = false
  for (const line of clauses) {
    if (other.has(line)) continue
    let m
    if ((m = /^select\s+(distinct\s+)?(.+)$/i.exec(line))) {
      expect(select === null, 'duplicate select')
      distinct = !!m[1]
      select = splitList(m[2]).map(item => {
        if (item === '*') { expect(tableNames.size === 1, 'select * with joins is ambiguous'); unaggregated.push('*'); return '*' }
        const agg = AGG.exec(item)
        if (agg) {
          const [,fn,arg,alias] = agg; id(alias)
          expect(!aliases.has(alias), 'duplicate output name');aliases.add(alias);hasAggregates = true
          if (arg === '*') expect(fn.toLowerCase() === 'count', 'only count(*) supports *')
          return `${fn.toUpperCase()}(${arg === '*' ? '*' : resolveColumn(arg, schema)}) AS ${quote(alias)}`
        }
        const col = COL_ALIAS.exec(item)
        expect(col, `invalid select entry ${item}`)
        const [,name,alias] = col
        const out = alias || name.split('.').at(-1)
        expect(!aliases.has(out), `duplicate or ambiguous output name ${out}`)
        aliases.add(out)
        unaggregated.push(name)
        return `${resolveColumn(name,schema)}${alias ? ` AS ${quote(alias)}` : ''}`
      })
    } else if ((m = /^where\s+(.+)$/i.exec(line))) where.push(m[1])
    else if ((m = /^group\s+(.+)$/i.exec(line))) {expect(group === null, 'duplicate group');group = splitList(m[1])}
    else if ((m = /^having\s+(.+)$/i.exec(line))) having.push(m[1])
    else if ((m = /^sort\s+(.+)$/i.exec(line))) {expect(order === null, 'duplicate sort'); order = splitList(m[1])}
    else if ((m = /^take\s+(\d+|\$[A-Za-z_][A-Za-z0-9_]*)$/i.exec(line))) {expect(take === null, 'duplicate take');take = m[1]}
    else if ((m = /^skip\s+(\d+|\$[A-Za-z_][A-Za-z0-9_]*)$/i.exec(line))) {expect(skip === null, 'duplicate skip');skip = m[1]}
    else expect(false, `unsupported clause ${line}`)
  }
  expect(select && select.length > 0, 'select clause required')
  expect(where.length <= 8 && having.length <= 8, 'too many filters')
  if (group) {
    const grouped = new Set(group)
    for (const col of unaggregated) expect(grouped.has(col), `nonaggregated field ${col} must be grouped`)
  } else expect(!hasAggregates || unaggregated.length === 0, 'aggregated select requires group for plain columns')
  expect(having.length === 0 || hasAggregates || group, 'having requires group or aggregate')
  let sql = `SELECT ${distinct ? 'DISTINCT ' : ''}${select.join(', ')} FROM ${quote(base)}`
  if (joins.length) sql += ` ${joins.join(' ')}`
  if (where.length) sql += ` WHERE ${where.map(s=>`(${parseFilter(s, schema, aliases, params, paramNames)})`).join(' AND ')}`
  if (group) sql += ` GROUP BY ${group.map(c=>resolveColumn(c,schema)).join(', ')}`
  if (having.length) sql += ` HAVING ${having.map(s=>`(${parseFilter(s, schema, aliases, params, paramNames, true)})`).join(' AND ')}`
  if (order) sql += ` ORDER BY ${order.map(t=>{
    const m=/^([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?)(?:\s+(asc|desc))?$/i.exec(t)
    expect(m, `invalid sort ${t}`)
    return `${aliases.has(m[1]) ? quote(m[1]) : resolveColumn(m[1],schema)} ${m[2]?.toUpperCase()||'ASC'}`
  }).join(', ')}`
  function countLimit(expr, fallback, upperBound) {
    if(expr===null)return fallback
    const raw=expr.startsWith('$')?params[expr.slice(1)]:Number(expr)
    expect(Number.isSafeInteger(raw) && raw >= 0 && raw <= upperBound, `invalid take/skip ${expr}`)
    return raw
  }
  const count = countLimit(take,maxRows,maxRows)
  const offset = countLimit(skip,0,1000000)
  sql += ` LIMIT ? OFFSET ?`
  return {sql, values:[...bindParams(paramNames,params),count,offset], maxRows, tables:[...tableNames], paramNames:[...paramNames], takeExpr:take, skipExpr:skip}
}
function limitValue(expr,params,fallback,upperBound) {
  if(expr === null)return fallback
  const raw=expr.startsWith('$')?params[expr.slice(1)]:Number(expr)
  expect(Number.isSafeInteger(raw)&&raw>=0&&raw<=upperBound,`invalid take/skip ${expr}`)
  return raw
}
function refreshValues(plan,params) {
  expect(params&&Object.getPrototypeOf(params)===Object.prototype,'parameters must be a plain object')
  return {...plan,values:[...bindParams(plan.paramNames,params),limitValue(plan.takeExpr,params,plan.maxRows,plan.maxRows),limitValue(plan.skipExpr,params,0,1000000)]}
}


export function createAdvancedQueryExecutor(db,{maxRows=500,maxQueries=200}={}) {
  expect(Number.isSafeInteger(maxQueries)&&maxQueries>=1&&maxQueries<=1000,'invalid statement-cache limit')
  const cached = new Map()
  const plans = new Map()
  const schemaStmt=db.prepare('PRAGMA schema_version')
  let schemaVersion=schemaStmt.get().schema_version
  function statement(sql) {
    if(cached.has(sql)) {const old=cached.get(sql);cached.delete(sql);cached.set(sql,old);return old}
    const stmt=db.prepare(sql);cached.set(sql,stmt)
    if(cached.size>maxQueries)cached.delete(cached.keys().next().value)
    return stmt
  }
  function compile(source,params={}){
    const current=schemaStmt.get().schema_version
    if(current!==schemaVersion){cached.clear();plans.clear();schemaVersion=current}
    if(plans.has(source))return refreshValues(plans.get(source),params)
    const plan=compileAdvanced(db,source,params,{maxRows})
    plans.set(source,{...plan,values:[]})
    if(plans.size>maxQueries)plans.delete(plans.keys().next().value)
    return plan
  }
  return Object.freeze({
    compile,
    query(source,params={}) {const plan=compile(source,params);return statement(plan.sql).all(...plan.values).map(row=>({...row}))},
    explain(source,params={}) {const plan=compile(source,params);return statement('EXPLAIN QUERY PLAN '+plan.sql).all(...plan.values)},
    clearCache(){cached.clear();plans.clear()}
  })
}

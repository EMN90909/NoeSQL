import test from 'node:test'
import assert from 'node:assert/strict'
import {readFileSync} from 'node:fs'
const source=p=>readFileSync(new URL('../'+p,import.meta.url),'utf8')
test('Noqeri owns authorization, typed IR execution and calls Zig pager for commits',()=>{
  const s=source('Database/noesql_zig_relational.nqr')
  assert.match(s,/noeApiValidate\(/)
  assert.match(s,/noeExecute\(/)
  assert.match(s,/noeRowDecode\(/)
  assert.match(s,/noeRowEncode\(/)
  assert.match(s,/noesql_zig_page_get\(/)
  assert.match(s,/noesql_zig_page_stage\(/)
  assert.match(s,/noesql_zig_page_commit\(/)
  assert.match(s,/noesql_zig_page_close\(/)
  assert.doesNotMatch(s,/sqlite3_|DatabaseSync|createSqliteAdapter/)
})
test('compiled smoke input requires signed-in scope before storage access',()=>{
  const f=source('tests/noesql_zig_relational.nqr')
  assert.match(f,/noeZigRelationalSmoke/)
  assert.match(f,/ctx\.principal\.authenticated = false/)
  assert.match(f,/status != -83/)
})

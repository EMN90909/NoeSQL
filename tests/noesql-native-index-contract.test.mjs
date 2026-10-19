import {test} from 'node:test'
import assert from 'node:assert/strict'
import {readFileSync} from 'node:fs'
const load=path=>readFileSync(new URL('../'+path,import.meta.url),'utf8')
const functions=['find','insert','delete','range']
test('C ABI, Zig implementation and Noqeri declarations agree on index exports',()=>{
  const header=load('Native/zig/noesql_zig_pages.h')
  const zig=load('Native/zig/src/storage_abi.zig')
  const nqr=load('Database/noesql_pager_abi.nqr')
  for(const op of functions){
    const id='noesql_zig_index_'+op
    assert.match(header,new RegExp('int32_t\\s+'+id+'\\('))
    assert.match(zig,new RegExp('pub export fn '+id+'\\('))
    assert.match(nqr,new RegExp('extern function '+id+'\\('))
  }
  assert.match(zig,/pager\.stage\(index_page/)
  assert.match(zig,/pager\.readPending\(page_id\)/)
  assert.match(zig,/index\.Index\.decode\(bytes\)/)
  assert.match(load('Native/zig/src/pager_v2.zig'),/pub fn readPending\(/)
})
test('Noqeri wrapper does not route native index updates through SQLite or JavaScript',()=>{
  const code=load('Database/noesql_native_index.nqr')
  assert.match(code,/noeIndexPut\(/)
  assert.match(code,/noeIndexLookup\(/)
  assert.match(code,/noeIndexRemove\(/)
  assert.doesNotMatch(code,/sqlite3_|DatabaseSync|createSqliteAdapter|node:fs/)
  const fixture=load('tests/noesql_native_index.nqr')
  assert.match(fixture,/noeNativeIndexSmoke/)
  assert.match(fixture,/noesql_zig_page_commit\(/)
  assert.match(fixture,/noesql_zig_page_close\(/)
})

import test from 'node:test'
import assert from 'node:assert/strict'
import {readFileSync,existsSync} from 'node:fs'
import {fileURLToPath} from 'node:url'
const root=fileURLToPath(new URL('../',import.meta.url))
const source=p=>readFileSync(new URL('../'+p,import.meta.url),'utf8')
test('0.7.6 identity is npm packaging-ready but not accidentally published',()=>{
  const pkg=JSON.parse(source('package.json'))
  assert.equal(pkg.version,'0.7.6')
  assert.equal(source('VERSION').trim(),pkg.version)
  assert.equal(pkg.private,true)
  for(const value of Object.values(pkg.exports)) assert.equal(existsSync(root+value.slice(1)),true)
})
test('benchmark links the actual native pager and component identity is explicit',()=>{
  assert.match(source('Native/zig/build.zig'),/name = "noesql_zig_pages"/)
  assert.match(source('scripts/benchmark-zig-v2.sh'),/libnoesql_zig_pages\.a/)
  assert.doesNotMatch(source('scripts/benchmark-zig-v2.sh'),/libnoesql_zig_bridge\.a/)
  assert.match(source('Native/zig/src/pager_v2.zig'),/COMPONENT_VERSION = "1\.0\.0"/)
  assert.match(source('Native/zig/src/index_v2.zig'),/COMPONENT_VERSION = "1\.0\.0"/)
  assert.match(source('scripts/verify-noqeri-bootstrap.mjs'),/COMPONENT_VERSION = '1\.0\.0'/)
  assert.match(source('Benchmarks/benchmark-zig-v2.c'),/BENCHMARK_COMPONENT_VERSION "1\.0\.0"/)
})

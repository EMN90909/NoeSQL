import test from 'node:test'
import assert from 'node:assert/strict'
import {readFileSync} from 'node:fs'
import {checkNativeCompiler} from '../scripts/build-noqeri-zig-link.mjs'

test('unverified compiler and missing provenance fail before creating an output',()=>{
 assert.throws(()=>checkNativeCompiler({}),/NOQERI_ROOT/)
 assert.throws(()=>checkNativeCompiler({NOQERI_ROOT:'/tmp/example'}),/NOQERI_NATIVE_COMPILER/)
})
test('build artifact retains machine code and verifiable hash manifest',()=>{
 const code=readFileSync(new URL('../scripts/build-noqeri-zig-link.mjs',import.meta.url),'utf8')
 for(const symbol of ['relational.o','relational.s','relational-smoke','provenance.json',
    'noesql_zig_page_stage','noesql_zig_page_commit','NQDB0002','NOESQL_NATIVE_OUT_DIR',
    'producer_stage2_sha256','sha256','cold_restart'])assert.match(code,new RegExp(symbol.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')))
 assert.match(code,/verified:true/)
 assert.match(code,/rmSync\(dir,\{recursive:true,force:true\}\)/)
 assert.doesNotMatch(code,/libnoesql_zig_bridge\.a/)
})
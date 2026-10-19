import test from 'node:test'
import assert from 'node:assert/strict'
import {readFileSync} from 'node:fs'
import {checkNativeCompiler} from '../scripts/build-noqeri-zig-link.mjs'
test('missing native self-hosted compiler cannot pass link qualification',()=>{
 assert.throws(()=>checkNativeCompiler({}),/NOQERI_ROOT/)
 assert.throws(()=>checkNativeCompiler({NOQERI_ROOT:'/tmp/noqeri'}),/NOQERI_NATIVE_COMPILER/)
})
test('source uses actual machine-code object and Zig ABI, not JS reference engine',()=>{
 const s=readFileSync(new URL('../scripts/build-noqeri-zig-link.mjs',import.meta.url),'utf8')
 const c=readFileSync(new URL('../Native/link-tests/noqeri_link_smoke.c',import.meta.url),'utf8')
 assert.match(s,/clang.*assembler/)
 assert.match(s,/libnoesql_zig_pages\.a/)
 assert.doesNotMatch(s,/libnoesql_zig_bridge\.a/)
 assert.match(c,/noeZigRelationalSmoke/)
 assert.match(c,/NQDB0002/)
})

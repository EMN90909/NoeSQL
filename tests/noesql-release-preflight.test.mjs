import test from 'node:test'
import assert from 'node:assert/strict'
import {readFileSync} from 'node:fs'
import {releasePolicy} from '../scripts/release-preflight.mjs'
const pkg=JSON.parse(readFileSync(new URL('../package.json',import.meta.url),'utf8'))
const licenseText=readFileSync(new URL('../LICENSE',import.meta.url),'utf8')
const approved={
NOESQL_APPROVE_PUBLICATION:'publish-noesql@0.7.6',
NOESQL_LICENSE_REVIEWED:'GPL-3.0-only',
NOESQL_APPROVED_COMMIT_SHA:'a'.repeat(40),
NOQERI_ROOT:'/absolute/noqeri',
NOQERI_COMMIT_SHA:'b'.repeat(40),
NOQERI_SOURCE_CLOSURE_SHA256:'c'.repeat(64),
NOESQL_NATIVE_BIN:'/absolute/bin',
NOESQL_NATIVE_BIN_SHA256:'d'.repeat(64),
NOQERI_STAGE0:'/absolute/seed',
NOQERI_STAGE0_SHA256:'e'.repeat(64)
}
test('public npm publish requires separate release and legal approvals',()=>{
assert.throws(()=>releasePolicy({manifest:pkg,licenseText,env:{}}),/publication approval/)
assert.throws(()=>releasePolicy({manifest:pkg,licenseText,env:{...approved,NOESQL_LICENSE_REVIEWED:''}}),/legal review/)
})
test('cannot redirect public publishing to another registry or dist-tag',()=>{
assert.throws(()=>releasePolicy({manifest:{...pkg,publishConfig:{...pkg.publishConfig,tag:'latest'}},licenseText,env:approved}),/registry/)
assert.throws(()=>releasePolicy({manifest:{...pkg,repository:{url:'https://evil.example/x'}},licenseText,env:approved}),/repository/)
})
test('policy is necessary but not a compiler/runtime artifact',()=>{
assert.equal(releasePolicy({manifest:pkg,licenseText,env:approved}).sha,'a'.repeat(40))
assert.throws(()=>releasePolicy({manifest:pkg,licenseText,env:{...approved,NOESQL_NATIVE_BIN:''}}),/NOESQL_NATIVE_BIN/)
})

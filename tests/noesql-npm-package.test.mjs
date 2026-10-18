import test from 'node:test'
import assert from 'node:assert/strict'
import {verifyPackageContents} from '../scripts/test-npm-package.mjs'
test('npm tarball rejects leaked credentials and unexpected absence of runtime',()=>{
  const required=['package.json','Runtime/noesql-embedded.mjs','Runtime/noesql-adapters.mjs','Runtime/noesql-reference.mjs'].map(path=>({path}))
  assert.equal(verifyPackageContents(required),true)
  assert.throws(()=>verifyPackageContents([...required,{path:'.npmrc'}]),/sensitive/)
  assert.throws(()=>verifyPackageContents([...required,{path:'Runtime/.env.production'}]),/sensitive/)
  assert.throws(()=>verifyPackageContents([{path:'package.json'}]),/missing/)
})

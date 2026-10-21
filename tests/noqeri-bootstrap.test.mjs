import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { verifyNoqeriProof } from '../scripts/verify-noqeri-bootstrap.mjs'

const h = bytes => createHash('sha256').update(bytes).digest('hex')
const pin = 'a'.repeat(20) // intentionally malformed git revision
const commit = 'b'.repeat(40)
const suiteNames = ['frontend','nir','optimizer','object-writers','formatter','lsp','package-manager','fuzz','race','memory','overflow','reproducible-build','practical-corpus']
const stage = name => ({ name, configured: true, status: 0, passed: true })
test('bootstrap proof verifier rejects missing or unpinned provenance', () => {
  assert.throws(() => verifyNoqeriProof({root:tmpdir(),expectedCommit:pin,expectedClosure:'a'.repeat(64),actualCommit:pin}),/pin both/)
  assert.throws(() => verifyNoqeriProof({root:tmpdir(),expectedCommit:commit,expectedClosure:'a'.repeat(64),actualCommit:'c'.repeat(64)}),/Git commit/)
})
test('bootstrap proof checks actual source closure and artifact bytes, not status alone', () => {
  const root = mkdtempSync(join(tmpdir(),'noesql-bootstrap-'))
  try {
    mkdirSync(join(root,'Compiler/selfhost'),{recursive:true})
    mkdirSync(join(root,'build'),{recursive:true})
    writeFileSync(join(root,'Compiler/selfhost/compiler-sources.txt'),'Compiler/selfhost/main.noqeri\n')
    writeFileSync(join(root,'Compiler/selfhost/main.noqeri'),'module test')
    const closure = h('Compiler/selfhost/main.noqeri\0module test\0')
    writeFileSync(join(root,'build/stage2'),'compiler-a')
    writeFileSync(join(root,'build/rebuilt'),'compiler-a')
    const artifactSha = h('compiler-a')
    const proof = {
      schema:1, verified:true,release_needs_cpp:false,
      stage0_built_stage1:true,stage1_built_stage2:true,stage2_rebuilt_compiler:true,
      outputs_equivalent:true,frontend_passed:true,nir_passed:true,optimizer_passed:true,
      object_writers_passed:true,formatter_passed:true,lsp_passed:true,package_manager_passed:true,
      hardening: Object.fromEntries(['fuzz_passed','race_passed','memory_passed','overflow_passed','reproducible_build_passed','practical_corpus_passed'].map(x=>[x,true])),
      stages:[stage('stage1-builds-stage2'),stage('stage2-rebuilds-compiler')],
      suites:suiteNames.map(stage),source_files:['Compiler/selfhost/main.noqeri'],source_closure_sha256:closure,
      stage2_output_sha256:artifactSha,rebuilt_output_sha256:artifactSha,
      artifacts:{stage2:{path:'build/stage2'},rebuilt:{path:'build/rebuilt'}}
    }
    const args = {root,expectedCommit:commit,expectedClosure:closure,actualCommit:commit}
    const location = join(root,'build/stage2-proof.json')
    writeFileSync(location,JSON.stringify(proof))
    // This tests consistency only; the fabricated fixture is NOT genuine compiler provenance.
    assert.equal(verifyNoqeriProof(args).verified,true)
    proof.stage0_built_stage1=false
    writeFileSync(location,JSON.stringify(proof))
    assert.throws(()=>verifyNoqeriProof(args),/seed/)
    proof.stage0_built_stage1=true
    writeFileSync(location,JSON.stringify(proof))
    writeFileSync(join(root,'build/rebuilt'),'compiler-b')
    assert.throws(()=>verifyNoqeriProof(args),/artifact/)
    writeFileSync(join(root,'build/rebuilt'),'compiler-a')
    writeFileSync(join(root,'Compiler/selfhost/main.noqeri'),'changed')
    assert.throws(()=>verifyNoqeriProof(args),/closure/)
  }finally{rmSync(root,{recursive:true,force:true})}
})

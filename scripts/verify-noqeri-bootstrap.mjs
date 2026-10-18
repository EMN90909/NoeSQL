#!/usr/bin/env node
// Verify Noqeri's Stage-2 proof against the exact source closure and pinned
// artifacts. This is an integrity/provenance *precondition*, not permission to
// mark the NoeSQL native SQL execution stack as qualified.
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, realpathSync, statSync } from 'node:fs'
import { resolve, relative, isAbsolute } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const requiredSuiteNames = [
  'frontend', 'nir', 'optimizer', 'object-writers', 'formatter', 'lsp',
  'package-manager', 'fuzz', 'race', 'memory', 'overflow',
  'reproducible-build', 'practical-corpus'
]
const requiredFlags = [
  'verified', 'stage0_built_stage1', 'stage1_built_stage2',
  'stage2_rebuilt_compiler', 'outputs_equivalent', 'frontend_passed',
  'nir_passed', 'optimizer_passed', 'object_writers_passed',
  'formatter_passed', 'lsp_passed', 'package_manager_passed'
]
const requiredHardening = [
  'fuzz_passed', 'race_passed', 'memory_passed', 'overflow_passed',
  'reproducible_build_passed', 'practical_corpus_passed'
]
const SHA256 = /^[0-9a-f]{64}$/
const GIT_COMMIT = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/
const digest = bytes => createHash('sha256').update(bytes).digest('hex')

function safePath(root, value) {
  if (typeof value !== 'string' || !value || value.includes('\0')) throw Error('invalid proof/artifact path')
  const absolute = resolve(root, value)
  const rel = relative(root, absolute)
  if (rel.startsWith('..') || isAbsolute(rel)) throw Error('proof/artifact path escapes Noqeri checkout')
  const real = realpathSync(absolute)
  const resolvedRelative = relative(realpathSync(root), real)
  if (resolvedRelative.startsWith('..') || isAbsolute(resolvedRelative)) throw Error('symlink escapes Noqeri checkout')
  if (!statSync(real).isFile()) throw Error('proof/artifact must be a regular file')
  return real
}

export function verifyNoqeriProof({ root, proofFile, expectedCommit, expectedClosure, actualCommit }) {
  if (!GIT_COMMIT.test(expectedCommit || '') || !SHA256.test(expectedClosure || '')) throw Error('pin both Noqeri git commit and compiler source SHA-256')
  if (actualCommit !== expectedCommit) throw Error('Noqeri Git commit does not match approved pin')
  const file = safePath(root, proofFile || 'build/stage2-proof.json')
  const proof = JSON.parse(readFileSync(file, 'utf8'))
  if (proof.schema !== 1 || proof.release_needs_cpp !== false) throw Error('unqualified Noqeri proof schema/release dependency')
  if (!requiredFlags.every(x => proof[x] === true)) throw Error('Noqeri Stage-2/Stage-3, seed and frontend proof incomplete')
  if (!proof.hardening || !requiredHardening.every(x => proof.hardening[x] === true)) throw Error('Noqeri hardening suites incomplete')
  const suites = proof.suites || []
  if (!requiredSuiteNames.every(name => suites.some(s => s.name === name && s.passed === true && s.configured === true && s.status === 0))) {
    throw Error('all actual compiler verification suites must exit zero')
  }
  if (!Array.isArray(proof.stages) || !['stage1-builds-stage2', 'stage2-rebuilds-compiler'].every(name => proof.stages.some(s => s.name === name && s.passed === true && s.configured === true && s.status === 0))) {
    throw Error('actual compiler rebuild stages must execute')
  }
  const sourceList = safePath(root, 'Compiler/selfhost/compiler-sources.txt')
  const files = readFileSync(sourceList, 'utf8').split(/\r?\n/).map(x => x.trim()).filter(Boolean)
  const hash = createHash('sha256')
  for (const relativeSource of files) {
    hash.update(relativeSource)
    hash.update('\0')
    hash.update(readFileSync(safePath(root, relativeSource)))
    hash.update('\0')
  }
  const sourceSha = hash.digest('hex')
  if (sourceSha !== expectedClosure || sourceSha !== proof.source_closure_sha256 ||
    JSON.stringify(files) !== JSON.stringify(proof.source_files)) throw Error('compiler source closure differs from proof')
  const stage2 = safePath(root, proof.artifacts?.stage2?.path)
  const rebuilt = safePath(root, proof.artifacts?.rebuilt?.path)
  const stage2Sha = digest(readFileSync(stage2))
  const rebuiltSha = digest(readFileSync(rebuilt))
  if (stage2Sha !== proof.stage2_output_sha256 || rebuiltSha !== proof.rebuilt_output_sha256 ||
    stage2Sha !== rebuiltSha) throw Error('compiler output artifacts lack byte-equivalent hashes')
  return { verified: true, noqeri_commit: expectedCommit, source_closure_sha256: sourceSha, stage2_sha256: stage2Sha }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const root = realpathSync(resolve(process.env.NOQERI_ROOT || ''))
    if (!process.env.NOQERI_ROOT) throw Error('NOQERI_ROOT is required')
    const proc = spawnSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8', timeout: 10000, shell: false })
    if (proc.status !== 0) throw Error('Cannot determine Noqeri checked-out revision')
    const result = verifyNoqeriProof({
      root,
      proofFile: process.env.NOQERI_STAGE2_PROOF || 'build/stage2-proof.json',
      expectedCommit: process.env.NOQERI_COMMIT_SHA,
      expectedClosure: process.env.NOQERI_SOURCE_CLOSURE_SHA256,
      actualCommit: proc.stdout.trim()
    })
    console.log(JSON.stringify(result, null, 2))
  } catch (error) {
    console.error(`Noqeri bootstrap BLOCKED: ${error.message}`)
    process.exitCode = 1
  }
}

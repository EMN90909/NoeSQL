import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawn } from 'node:child_process'
import { readNqd, runNqd, verifyNqd } from '../Runtime/noesql-store.mjs'

const schema = `db acceptance
table users {
  id: int key auto
  name: text required
  age: int required
}`

async function fixture(fn) {
  const dir = await mkdtemp(join(tmpdir(), 'noesql-acceptance-'))
  const path = join(dir, 'acceptance.nqd')
  try { await fn(path, dir) } finally { await rm(dir, { recursive: true, force: true }) }
}

test('acceptance: create, insert, read, update, delete and persist across reopen', () => fixture(async path => {
  await runNqd(path, schema)
  await runNqd(path, 'users add {name:"Ada", age:20}')
  assert.deepEqual((await readNqd(path)).tables.users.rows, [{id:1,name:'Ada',age:20}])

  await runNqd(path, 'users where id == 1 set age = 21')
  assert.deepEqual((await readNqd(path)).tables.users.rows, [{id:1,name:'Ada',age:21}])

  // A fresh read from disk must recover the last committed state.
  const reopened = await readNqd(path)
  assert.deepEqual(reopened.tables.users.rows, [{id:1,name:'Ada',age:21}])
  await runNqd(path, 'users where id == 1 delete')
  assert.deepEqual((await readNqd(path)).tables.users.rows, [])
  assert.equal((await verifyNqd(path)).ok, true)
}))

test('acceptance: failed multi-statement transaction preserves prior committed state', () => fixture(async path => {
  await runNqd(path, schema + '\\nusers add {name:"Committed", age:30}')
  const before = await readNqd(path)
  await assert.rejects(runNqd(path, `atomic {
    users add {name:"Temporary", age:31}
    users add {name:"Invalid", age:"not-an-int"}
  }`))
  assert.deepEqual((await readNqd(path)), before)
  assert.deepEqual((await readNqd(path)).tables.users.rows, [{id:1,name:'Committed',age:30}])
}))

const phases = [
  ['afterBeginWrite', false],
  ['afterImageWrite', false],
  ['afterCommitWrite', true],
  ['afterWalSync', true],
  ['afterWalDirectorySync', true],
  ['afterSnapshotSync', true],
  ['afterSnapshotRename', true],
  ['afterDirectorySync', true],
  ['afterCheckpoint', true]
]

for (const [phase, expectedCommit] of phases) {
  test(`acceptance: SIGKILL at ${phase} recovers a consistent database`, { skip: process.platform === 'win32' }, () => fixture(async (path, dir) => {
    await runNqd(path, schema + '\\nusers add {name:"Before", age:10}')
    const moduleUrl = new URL('../Runtime/noesql-store.mjs', import.meta.url).href
    const childScript = `
      import { runNqd } from ${JSON.stringify(moduleUrl)};
      await runNqd(${JSON.stringify(path)}, 'users add {name:"During", age:11}', {}, {
        inject: phase => { if (phase === ${JSON.stringify(phase)}) process.kill(process.pid, 'SIGKILL') }
      });
    `
    const result = await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, ['--input-type=module', '-e', childScript], { cwd: dir, stdio: ['ignore','ignore','pipe'] })
      let stderr = ''
      child.stderr.on('data', chunk => { stderr += chunk })
      child.once('error', reject)
      child.once('close', (code, signal) => resolve({ code, signal, stderr }))
    })
    assert.equal(result.signal, 'SIGKILL', result.stderr)
    const state = await readNqd(path)
    assert.deepEqual(state.tables.users.rows.map(row => row.name), expectedCommit ? ['Before','During'] : ['Before'])
    assert.equal((await verifyNqd(path)).ok, true)
    // A killed writer leaves its lock behind; recovery must not silently steal it.
    await assert.rejects(runNqd(path, 'users add {name:"After", age:12}'), /locked/)
  }))
}

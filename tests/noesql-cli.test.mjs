import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { runNqd } from '../Runtime/noesql-store.mjs'

const exec = promisify(execFile)
const cli = new URL('../Runtime/noesql-cli.mjs', import.meta.url).pathname

test('CLI verify reports a valid reference snapshot', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'noesql-verify-'))
  try {
    const path = join(directory, 'test.nqd')
    await runNqd(path, 'table users { id: int key auto }')
    const { stdout } = await exec(process.execPath, [cli, 'verify', path])
    assert.deepEqual(JSON.parse(stdout), { ok: true, tables: 1, walPresent: false })
  } finally { await rm(directory, { recursive: true, force: true }) }
})

test('CLI verify fails on corrupted snapshot', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'noesql-verify-'))
  try {
    const path = join(directory, 'test.nqd')
    await writeFile(path, 'corrupt')
    await assert.rejects(exec(process.execPath, [cli, 'verify', path]), error =>
      error.code === 1 && /invalid JSON/.test(error.stderr))
  } finally { await rm(directory, { recursive: true, force: true }) }
})

test('CLI verify rejects excess arguments', async () => {
  const { stderr } = await exec(process.execPath, [cli, 'verify']).catch(e => e)
  assert.match(stderr, /Usage:/)
})

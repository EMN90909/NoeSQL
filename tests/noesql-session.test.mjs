import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { database } from '../Runtime/noesql-session.mjs'
test('NoeSQL session delegates to .noedb durable reference store', { skip: process.platform !== 'linux' }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'noesql-session-'))
  try {
    const db = database(join(dir, 'app.noedb'))
    await db.execute('table users { id: int key auto\n name: text required }\nquery by_name() { users get id, name sort name }')
    await db.atomic('users add {name:"Manu"}\nusers add {name:"Ada"}')
    assert.deepEqual((await db.query('by_name')).rows.map(x => x.name), ['Ada','Manu'])
    assert.equal((await readFile(db.path)).toString('ascii',0,8), 'NQDB0002')
    assert.equal((await db.verify()).format, 'NQDB0002')
    assert.throws(() => db.begin(), /native session/)
    assert.throws(() => database(join(dir, 'old.nqdb')), /migrate/)
  } finally { await rm(dir, { recursive: true, force: true }) }
})

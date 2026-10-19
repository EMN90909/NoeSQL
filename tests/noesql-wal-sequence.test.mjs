import {test} from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {runNqd,readNqd,verifyNqd} from '../Runtime/noesql-store.mjs'
async function dbTest(fn) {
  const dir=await mkdtemp(join(tmpdir(),'noesql-sequence-'))
  const path=join(dir,'db.noedb')
  try {await fn(path)} finally {await rm(dir,{recursive:true,force:true})}
}
const source='table t {id:int key auto\n name:text required}'
test('commit sequence advances on writes but not reads',()=>dbTest(async db=>{
  await runNqd(db,source)
  const first=(await verifyNqd(db)).storageSequence
  await runNqd(db,'t get name')
  assert.equal((await verifyNqd(db)).storageSequence,first)
  await runNqd(db,'t add {name:"A"}')
  assert.equal((await verifyNqd(db)).storageSequence,first+1)
}))
test('replayed previously committed WAL cannot roll back newer snapshot',()=>dbTest(async db=>{
  await runNqd(db,source)
  await assert.rejects(runNqd(db,'t add {name:"A"}',{},{
    inject:p=>{if(p==='afterWalDirectorySync')throw Error('fault')}
  }),/fault/)
  const oldWal=await readFile(db+'.wal')
  await runNqd(db,'t add {name:"B"}')
  assert.deepEqual((await readNqd(db)).tables.t.rows.map(x=>x.name),['A','B'])
  await writeFile(db+'.wal',oldWal)
  await assert.rejects(readNqd(db),/stale WAL generation/)
  await assert.rejects(verifyNqd(db),/stale WAL generation/)
  await assert.rejects(runNqd(db,'t add {name:"C"}'),/stale WAL generation/)
}))
test('WAL image and renamed checkpoint with equal sequence recover idempotently',()=>dbTest(async db=>{
  await runNqd(db,source)
  await assert.rejects(runNqd(db,'t add {name:"A"}',{},{
    inject:p=>{if(p==='afterSnapshotRename')throw Error('checkpoint interruption')}
  }),/checkpoint interruption/)
  assert.deepEqual((await readNqd(db)).tables.t.rows.map(x=>x.name),['A'])
  assert.equal((await verifyNqd(db)).walPresent,true)
  await runNqd(db,'t add {name:"B"}')
  assert.deepEqual((await readNqd(db)).tables.t.rows.map(x=>x.name),['A','B'])
}))

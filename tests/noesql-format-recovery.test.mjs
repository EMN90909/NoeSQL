import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { runNqd, readNqd, verifyNqd, namedNqd } from '../Runtime/noesql-store.mjs'
const schema='table people { id:int key auto\n name:text required }\nquery names() { people get name sort name }'
async function workspace(fn) {
  const dir=await mkdtemp(join(tmpdir(),'noesql-formats-'))
  try { await fn(dir) } finally { await rm(dir,{recursive:true,force:true}) }
}
test('.noedb accepts .nqds schema and named queries after restart',()=>workspace(async dir=>{
  const db=join(dir,'people.noedb'),script=join(dir,'schema.nqds')
  await writeFile(script,schema)
  const cli=new URL('../Runtime/noesql-cli.mjs',import.meta.url)
  const run=spawnSync(process.execPath,[cli.pathname,'run',db,script],{encoding:'utf8'})
  assert.equal(run.status,0,run.stderr)
  await runNqd(db,'people add {name:"Ada"}')
  assert.deepEqual((await namedNqd(db,'names')).rows,[{name:'Ada'}])
  assert.equal((await verifyNqd(db)).ok,true)
}))
test('reference read-only query leaves checkpoint bytes unchanged',()=>workspace(async dir=>{
  const db=join(dir,'read.noedb')
  await runNqd(db,schema+'\npeople add {name:"A"}')
  const original=await readFile(db)
  await runNqd(db,'people get name')
  assert.deepEqual(await readFile(db),original)
}))
test('failed multi-write transaction does not mutate committed .noedb bytes',()=>workspace(async dir=>{
  const db=join(dir,'atomic.noedb')
  await runNqd(db,schema+'\npeople add {name:"Ada"}')
  const original=await readFile(db)
  await assert.rejects(runNqd(db,'people add {name:"Bea"}\npeople add {name:7}'),/expected text/)
  assert.deepEqual(await readFile(db),original)
  assert.deepEqual((await readNqd(db)).tables.people.rows.map(r=>r.name),['Ada'])
}))
test('durable WAL recovers after checkpoint failure and later writer reopens',()=>workspace(async dir=>{
  const db=join(dir,'restart.noedb')
  await runNqd(db,schema+'\npeople add {name:"Ada"}')
  await assert.rejects(runNqd(db,'people add {name:"Bea"}',{},{
    inject:phase=>{if(phase==='afterWalDirectorySync')throw Error('power-cut simulation')}
  }),/power-cut/)
  assert.deepEqual((await readNqd(db)).tables.people.rows.map(r=>r.name),['Ada','Bea'])
  await runNqd(db,'people add {name:"Cam"}')
  assert.deepEqual((await readNqd(db)).tables.people.rows.map(r=>r.name),['Ada','Bea','Cam'])
  const journal=await readFile(db+'.wal').catch(e=>e.code==='ENOENT'?null:Promise.reject(e))
  assert.equal(journal,null)
}))

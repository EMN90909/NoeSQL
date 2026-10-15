import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { mkdtemp, rm, symlink, link, writeFile, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { readNqd, runNqd } from '../Runtime/noesql-store.mjs'
const schema = 'table users { id: int key auto\n name: text required\n}'
async function fixture() {
  const dir=await mkdtemp(join(tmpdir(),'noesql-sec-'))
  return {dir,path:join(dir,'data.nqd'),cleanup:()=>rm(dir,{recursive:true,force:true})}
}
test('storage rejects oversized snapshot before decode',async()=>{
  const f=await fixture()
  try {await writeFile(f.path,Buffer.alloc(16*1024*1024+1))
    await assert.rejects(readNqd(f.path),/file size/)}
  finally {await f.cleanup()}
})
test('storage rejects oversized WAL before allocation',async()=>{
  const f=await fixture()
  try {await writeFile(f.path+'.wal',Buffer.alloc(16*1024*1024+300))
    await assert.rejects(readNqd(f.path),/file size/)}
  finally {await f.cleanup()}
})
test('symlinks and hardlinks to snapshot are rejected',async()=>{
  const f=await fixture()
  try {
    const target=join(f.dir,'target');await writeFile(target,'PRIVATE DATA')
    await symlink(target,f.path)
    await assert.rejects(readNqd(f.path),/ELOOP|invalid file/)
    await rm(f.path)
    await link(target,f.path)
    await assert.rejects(readNqd(f.path),/hard-link count/)
    assert.equal(await readFile(target,'utf8'),'PRIVATE DATA')
  } finally {await f.cleanup()}
})
test('oversized query rejected before writer lock',async()=>{
  const f=await fixture()
  try {
    await assert.rejects(runNqd(f.path,'x'.repeat(1024*1024+1)),/query source/)
    await runNqd(f.path,schema)
    assert.equal((await readNqd(f.path)).tables.users.rows.length,0)
  } finally {await f.cleanup()}
})
test('concurrent writers exclude overlapping transactions',async()=>{
  const f=await fixture()
  try {
    await runNqd(f.path,schema)
    let unblock, reached
    const hold=new Promise(resolve=>unblock=resolve)
    const entered=new Promise(resolve=>reached=resolve)
    const first=runNqd(f.path,'users add {name:"A"}',{}, {inject:async phase=>{if(phase==='afterBeginWrite'){reached();await hold}}})
    await entered
    await assert.rejects(runNqd(f.path,'users add {name:"B"}'),/locked/)
    unblock();await first
    await runNqd(f.path,'users add {name:"B"}')
    assert.deepEqual((await readNqd(f.path)).tables.users.rows.map(r=>r.name),['A','B'])
  } finally {await f.cleanup()}
})

import { strict as assert } from 'node:assert'
import { test } from 'node:test'
import { mkdtemp, readFile, writeFile, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { readNqd, runNqd, verifyNqd } from '../Runtime/noesql-store.mjs'
import { newWalTransaction, replayWal } from '../Runtime/noesql-wal.mjs'
const schema='table users { id: int key auto\n name: text required\n}'
async function fixture() {
  const dir=await mkdtemp(join(tmpdir(),'noe-wal-')), path=join(dir,'school.nqd')
  await runNqd(path,schema+'\nusers add {name:"A"}')
  return {dir,path,cleanup:()=>rm(dir,{recursive:true,force:true})}
}
test('WAL format validates sequence, digest and full committed image',()=>{
  const tx=newWalTransaction(Buffer.from('test-image'),1n)
  assert.equal(replayWal(tx.bytes).committed.toString(),'test-image')
  assert.equal(replayWal(Buffer.concat(tx.frames.slice(0,2))).committed,null)
  const corrupt=Buffer.from(tx.bytes);corrupt[90]^=1
  assert.throws(()=>replayWal(corrupt),/checksum|magic|sequence/)
  const truncated=tx.bytes.subarray(0,tx.bytes.length-1)
  assert.equal(replayWal(truncated).committed,null)
  assert.throws(()=>replayWal(Buffer.concat([tx.bytes,Buffer.from('x')])),/trailing bytes/)
})
for(const [phase,committed] of [['afterBeginWrite',false],['afterImageWrite',false],['afterCommitWrite',true],['afterWalSync',true],['afterWalDirectorySync',true],['afterSnapshotSync',true],['afterSnapshotRename',true],['afterDirectorySync',true],['afterCheckpoint',true]]) {
  test(`fault at ${phase} recovers ${committed?'new':'previous'} state`, async()=>{
    const f=await fixture()
    try {
      await assert.rejects(runNqd(f.path,'users add {name:"B"}',{}, {inject:at=>{if(at===phase)throw Error('injected fault')}}),/injected fault/)
      assert.deepEqual((await readNqd(f.path)).tables.users.rows.map(x=>x.name),committed?['A','B']:['A'])
      assert.equal((await verifyNqd(f.path)).ok,true)
      // On the next writer invocation, outstanding committed WAL is checkpointed.
      await runNqd(f.path,'users add {name:"C"}')
      assert.deepEqual((await readNqd(f.path)).tables.users.rows.map(x=>x.name),committed?['A','B','C']:['A','C'])
    } finally {await f.cleanup()}
  })
}
test('bad WAL checksum fails closed and read-only verifier leaves bytes unchanged',async()=>{
  const f=await fixture()
  try {
    await assert.rejects(runNqd(f.path,'users add {name:"B"}',{}, {inject:phase=>{if(phase==='afterWalSync')throw Error('stop')}}))
    const p=f.path+'.wal', original=await readFile(p)
    assert.equal((await verifyNqd(f.path)).walPresent,true)
    assert.deepEqual(await readFile(p),original)
    const altered=Buffer.from(original);altered[60]^=0x10;await writeFile(p,altered)
    await assert.rejects(readNqd(f.path),/checksum mismatch|magic.*mismatch/)
    await assert.rejects(runNqd(f.path,'users add {name:"C"}'),/checksum mismatch|magic.*mismatch/)
    assert.deepEqual(await readFile(p),altered)
  } finally {await f.cleanup()}
})
test('full WAL snapshot can recover missing checkpoint base',async()=>{
  const f=await fixture()
  try {
    await assert.rejects(runNqd(f.path,'users add {name:"B"}',{}, {inject:phase=>{if(phase==='afterWalSync')throw Error('stop')}}))
    await rm(f.path)
    assert.deepEqual((await readNqd(f.path)).tables.users.rows.map(x=>x.name),['A','B'])
  } finally {await f.cleanup()}
})


test('SIGKILL after WAL fsync retains committed image on POSIX', {skip:process.platform==='win32'}, async()=>{
  const {spawn}=await import('node:child_process')
  const f=await fixture()
  try {
    const moduleUrl=new URL('../Runtime/noesql-store.mjs',import.meta.url).href
    const childScript=`import {runNqd} from ${JSON.stringify(moduleUrl)};\nawait runNqd(${JSON.stringify(f.path)},'users add {name:"B"}',{}, {inject:phase=>{if(phase==='afterWalDirectorySync')process.kill(process.pid,'SIGKILL')}})`
    const result=await new Promise((resolve,reject)=>{
      const child=spawn(process.execPath,['--input-type=module','-e',childScript],{stdio:['ignore','pipe','pipe']})
      let errors='';child.stderr.on('data',chunk=>errors+=chunk)
      child.once('error',reject)
      child.once('close',(code,signal)=>resolve({code,signal,errors}))
    })
    assert.equal(result.signal,'SIGKILL',result.errors)
    assert.deepEqual((await readNqd(f.path)).tables.users.rows.map(x=>x.name),['A','B'])
    // The lock file intentionally remains after a hard kill: fail closed.
    await assert.rejects(runNqd(f.path,'users add {name:"C"}'),/locked/)
  } finally {await f.cleanup()}
})

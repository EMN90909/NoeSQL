// Linux local-filesystem stress: transaction snapshots, cross-process flock,
// process termination, and restart. NOT a power-loss/ACID certification.
import test from 'node:test'
import assert from 'node:assert/strict'
import {spawn} from 'node:child_process'
import {mkdtemp,rm,readFile} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {readNqd,runNqd,verifyNqd} from '../Runtime/noesql-store.mjs'
const moduleUrl=new URL('../Runtime/noesql-store.mjs',import.meta.url).href
const schema='table items {id:int key auto\n name:text required}'
async function fixture(fn){
 const dir=await mkdtemp(join(tmpdir(),'noesql-acid-matrix-'))
 const path=join(dir,'items.noedb')
 try{await runNqd(path,schema);await fn(path)}finally{await rm(dir,{recursive:true,force:true})}
}
function launch(script){
 return new Promise((resolve,reject)=>{
  const child=spawn(process.execPath,['--input-type=module','-e',script],{stdio:['ignore','pipe','pipe']})
  let stderr=''
  child.stderr.on('data',c=>stderr+=c.toString())
  child.once('error',reject)
  child.once('close',(code,signal)=>resolve({code,signal,stderr}))
 })
}
test('atomic batch rollback preserves checkpoint and generation',()=>fixture(async path=>{
 await runNqd(path,'items add {name:"A"}')
 const before=(await verifyNqd(path)).storageSequence,bytes=await readFile(path)
 await assert.rejects(runNqd(path,'items add {name:"B"}\nitems add {name:19}'),/expected text/)
 assert.deepEqual((await readNqd(path)).tables.items.rows.map(x=>x.name),['A'])
 assert.equal((await verifyNqd(path)).storageSequence,before)
 assert.deepEqual(await readFile(path),bytes)
}))
for(const [phase,shouldCommit] of [['afterBeginWrite',false],['afterImageWrite',false],['afterWalDirectorySync',true],['afterSnapshotRename',true],['afterCheckpoint',true]]){
 test(`SIGKILL at ${phase} restores ${shouldCommit?'committed':'previous'} generation`,{skip:process.platform!=='linux'},()=>fixture(async path=>{
  await runNqd(path,'items add {name:"A"}')
  const original=(await verifyNqd(path)).storageSequence
  const script=`import {runNqd} from ${JSON.stringify(moduleUrl)};await runNqd(${JSON.stringify(path)},'items add {name:"B"}',{}, {inject:p=>{if(p===${JSON.stringify(phase)})process.kill(process.pid,'SIGKILL')}})`
  const result=await launch(script)
  assert.equal(result.signal,'SIGKILL',result.stderr)
  assert.deepEqual((await readNqd(path)).tables.items.rows.map(r=>r.name),shouldCommit?['A','B']:['A'])
  assert.equal((await verifyNqd(path)).storageSequence,original+(shouldCommit?1:0))
  await runNqd(path,'items add {name:"C"}')
  assert.deepEqual((await readNqd(path)).tables.items.rows.map(r=>r.name),shouldCommit?['A','B','C']:['A','C'])
 }))
}
test('competing processes serialize on Zig .lck inode without lost updates',{skip:process.platform!=='linux'},()=>fixture(async path=>{
 const count=8
 const worker=index=>`import {runNqd} from ${JSON.stringify(moduleUrl)};let wrote=false;for(let tries=0;tries<160&&!wrote;tries++){try{await runNqd(${JSON.stringify(path)},'items add {name:"P${index}"}');wrote=true}catch(e){if(!/locked/.test(e.message))throw e;await new Promise(r=>setTimeout(r,25))}}if(!wrote)throw Error('retry budget exhausted')`
 const results=await Promise.all(Array.from({length:count},(_,i)=>launch(worker(i))))
 for(const [i,result] of results.entries())assert.equal(result.code,0,`process ${i}: ${result.stderr}`)
 const state=await readNqd(path)
 assert.equal(state.tables.items.rows.length,count)
 assert.deepEqual(state.tables.items.rows.map(r=>r.name).sort(),Array.from({length:count},(_,i)=>'P'+i).sort())
 assert.equal((await verifyNqd(path)).storageSequence,count+1)
}))
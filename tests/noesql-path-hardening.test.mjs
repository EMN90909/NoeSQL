import {test} from 'node:test'
import {strict as assert} from 'node:assert'
import {mkdtemp,rm,symlink,link,writeFile,readFile} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {runNqd,readNqd} from '../Runtime/noesql-store.mjs'
const schema='table users {id:int key auto\n name:text required}'
async function fixture(fn) {const dir=await mkdtemp(join(tmpdir(),'noe-hardening-'));try{await fn(join(dir,'database.nqd'),dir)}finally{await rm(dir,{recursive:true,force:true})}}
test('a new writer cannot overwrite a snapshot symlink',()=>fixture(async(path,dir)=>{
  const victim=join(dir,'victim.txt')
  await writeFile(victim,'DO NOT CHANGE')
  await symlink(victim,path)
  await assert.rejects(runNqd(path,schema),/NOE-SEC|ELOOP|invalid file/)
  assert.equal(await readFile(victim,'utf8'),'DO NOT CHANGE')
}))
test('a new writer cannot overwrite a multiply-linked snapshot',()=>fixture(async(path,dir)=>{
  const victim=join(dir,'victim.txt')
  await writeFile(victim,'DO NOT CHANGE')
  await link(victim,path)
  await assert.rejects(runNqd(path,schema),/NOE-SEC|hard-link/)
  assert.equal(await readFile(victim,'utf8'),'DO NOT CHANGE')
}))
test('checkpoint does not replace a substituted symlink',()=>fixture(async(path,dir)=>{
  await runNqd(path,schema)
  const victim=join(dir,'victim.txt')
  await writeFile(victim,'DO NOT CHANGE')
  await assert.rejects(runNqd(path,'users add {name:"Ada"}',{},{
    inject:async phase=>{if(phase==='afterSnapshotSync'){
      await rm(path)
      await symlink(victim,path)
    }}
  }),/NOE-SEC|ELOOP|invalid file/)
  assert.equal(await readFile(victim,'utf8'),'DO NOT CHANGE')
  await assert.rejects(readNqd(path),/ELOOP|invalid file/)
}))

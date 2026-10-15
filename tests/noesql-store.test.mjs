import {test} from 'node:test'
import {strict as assert} from 'node:assert'
import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {runNqd,readNqd,namedNqd} from '../Runtime/noesql-store.mjs'
async function fixture(fn){const folder=await mkdtemp(join(tmpdir(),'noesql-'));try{await fn(join(folder,'test.nqd'))}finally{await rm(folder,{recursive:true,force:true})}}
test('persist .nqd with checksum and query after restart',()=>fixture(async path=>{
  await runNqd(path,'table users { id: int key auto\n name: text required\n}\nquery all() { users get id, name }')
  await runNqd(path,'users add {name:"Ada"}')
  const rows=(await namedNqd(path,'all')).rows
  assert.deepEqual(rows,[{id:1,name:'Ada'}]);assert.equal((await readNqd(path)).tables.users.rows.length,1)
  const bytes=await readFile(path);assert.match(bytes.toString(),/NOESQL-NQD-EXPERIMENTAL/)
}))
test('reject corrupted storage without mutation',()=>fixture(async path=>{
  await runNqd(path,'table users {id:int key}')
  const content=await readFile(path,'utf8')
  await writeFile(path,content.replace('noesql-reference-1','noesql-reference-2'))
  await assert.rejects(readNqd(path),/checksum/)
}))
test('failed write does not persist state',()=>fixture(async path=>{
  await runNqd(path,'table users {id:int key\n name:text required}')
  await assert.rejects(runNqd(path,'users add {id:1,name:23}'),/expected text/)
  assert.equal((await readNqd(path)).tables.users.rows.length,0)
}))
test('cannot write when lock exists',()=>fixture(async path=>{
  await writeFile(path+'.lock','taken')
  await assert.rejects(runNqd(path,'db test'),/locked/)
}))

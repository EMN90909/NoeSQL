import {test} from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp,readFile,rm,writeFile} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {migrateLegacyNoedb,readNqd,runNqd,namedNqd,verifyNqd} from '../Runtime/noesql-store.mjs'
const schema='table users {id:int key auto\n name:text required}\nquery names() {users get name sort name}'
async function fixture(callback){const dir=await mkdtemp(join(tmpdir(),'noe-unified-migrate-'));try{await callback(dir)}finally{await rm(dir,{recursive:true,force:true})}}
test('legacy .nqdb -> NQDB0002 .noedb migration preserves named queries and row state',()=>fixture(async dir=>{
 const source=join(dir,'legacy.nqdb'),target=join(dir,'native.noedb')
 await runNqd(source,schema+'\nusers add {name:"Ada"}')
 const oldBytes=await readFile(source)
 await migrateLegacyNoedb(source,target)
 assert.equal((await readFile(target)).toString('ascii',0,8),'NQDB0002')
 assert.deepEqual((await namedNqd(target,'names')).rows,[{name:'Ada'}])
 assert.equal((await verifyNqd(target)).storageSequence,1)
 assert.deepEqual(await readFile(source),oldBytes)
 await runNqd(target,'users add {name:"Bea"}')
 assert.deepEqual((await namedNqd(target,'names')).rows,[{name:'Ada'},{name:'Bea'}])
 await assert.rejects(migrateLegacyNoedb(source,target),/already exists/)
}))
test('rejects legacy JSON under the .noedb extension rather than silently overwriting',()=>fixture(async dir=>{
 const path=join(dir,'unsafe.noedb')
 await writeFile(path,'{"magic":"NOESQL-NQD-EXPERIMENTAL"}')
 await assert.rejects(readNqd(path),/incompatible database format/)
 await assert.rejects(runNqd(path,schema),/incompatible database format/)
}))

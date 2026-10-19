import test from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {emptyDatabase} from '../Runtime/noesql-reference.mjs'
import {writeZigNoedb,readZigNoedb,verifyZigNoedb,MAX_TX_PAGES} from '../Runtime/noesql-zig-pages.mjs'

test('Zig and Node page framing supports state images above the old 15-page ceiling', {skip:process.platform!=='linux'},async()=>{
 const dir=await mkdtemp(join(tmpdir(),'noesql-big-page-tx-')),file=join(dir,'big.noedb')
 try{
  assert.equal(MAX_TX_PAGES,256)
  const payload='D'.repeat(180000)
  const state={...emptyDatabase(),tables:{records:{fields:[],rows:[{payload}],nextId:2}}}
  await writeZigNoedb(file,()=>({database:state,results:[]}),emptyDatabase)
  assert.equal((await readZigNoedb(file,emptyDatabase)).tables.records.rows[0].payload,payload)
  const verified=await verifyZigNoedb(file)
  assert.equal(verified.format,'NQDB0002')
  assert.equal(verified.storageSequence,1)
  const tooLarge='X'.repeat(256*4096)
  await assert.rejects(writeZigNoedb(file,()=>({database:{...state,tables:{records:{fields:[],rows:[{payload:tooLarge}],nextId:2}}},results:[]}),emptyDatabase),/exceeds|limit|too large|length/)
  assert.equal((await readZigNoedb(file,emptyDatabase)).tables.records.rows[0].payload,payload)
  assert.equal((await verifyZigNoedb(file)).storageSequence,1)
 }finally{await rm(dir,{recursive:true,force:true})}
})
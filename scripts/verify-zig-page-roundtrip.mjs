#!/usr/bin/env node
// Actual cross-engine ABI fixture. No native compiler / SQL claims are made here.
import {spawnSync} from 'node:child_process'
import {mkdtemp,writeFile,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join,resolve} from 'node:path'
import {emptyDatabase} from '../Runtime/noesql-reference.mjs'
import {writeZigNoedb,readZigNoedb,encodeReferencePages} from '../Runtime/noesql-zig-pages.mjs'
const driver=process.argv[2]
if(!driver||!resolve(driver).startsWith('/'))throw Error('supply compiled Zig-linked C driver path')
const work=await mkdtemp(join(tmpdir(),'noesql-zig-interop-'))
function run(...args){const r=spawnSync(driver,args,{encoding:'utf8',timeout:30000});if(r.status!==0)throw Error('Zig pager roundtrip: '+r.stderr)}
try{
  const first=join(work,'node.noedb'),second=join(work,'zig.noedb'),prefix=join(work,'pages')
  const input={...emptyDatabase(),storageSequence:1}
  await writeZigNoedb(first,async()=>({database:{...input,tables:{t:{fields:[],rows:[{id:1,name:'Ada'}],nextId:2}}},results:[]}),emptyDatabase)
  run('verify',first)
  const pages=encodeReferencePages((await readZigNoedb(first,emptyDatabase)))
  for(const [id,data] of pages)await writeFile(prefix+'.'+id,data)
  run('write',second,prefix)
  const state=await readZigNoedb(second,emptyDatabase)
  if(state.tables.t.rows[0].name!=='Ada'||state.storageSequence!==1)throw Error('native Zig checkpoint not decodable by reference')
  console.log('Node -> Zig ABI -> Node: NQDB0002 page/transaction roundtrip passed')
}finally{await rm(work,{recursive:true,force:true})}

import {test} from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp,readFile,writeFile,rm,stat} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {spawnSync} from 'node:child_process'
import {encodeZigCheckpoint,decodeZigCheckpoint,encodeZigTransaction,decodeZigWal,readZigNoedb,verifyZigNoedb,writeZigNoedb,PAGE_BYTES} from '../Runtime/noesql-zig-pages.mjs'
const empty=()=>({format:'noesql-reference-1',name:null,version:1,tables:{},indexes:{},queries:{}})
const insert=name=>async before=>({database:{...before,tables:{people:{rows:[...(before.tables.people?.rows||[]),{name}],fields:[],nextId:1}}},results:[name]})
async function fixture(callback){const dir=await mkdtemp(join(tmpdir(),'zig-pages-test-'));try{await callback(join(dir,'db.noedb'))}finally{await rm(dir,{recursive:true,force:true})}}
test('Zig checkpoint bytes and WAL record conform to Zig layout',()=>{
 const pages=new Map([[0,Buffer.alloc(PAGE_BYTES,0x30)],[3,Buffer.alloc(PAGE_BYTES,0x75)]])
 const ckpt=encodeZigCheckpoint(8,pages)
 assert.equal(ckpt.toString('ascii',0,8),'NQDB0002')
 assert.equal(ckpt.readUInt32LE(8),4096)
 assert.equal(ckpt.readUInt32LE(12),256)
 assert.equal(ckpt.readBigUInt64LE(16),8n)
 assert.deepEqual(decodeZigCheckpoint(ckpt).pages,pages)
 const wal=encodeZigTransaction(9,pages)
 assert.equal(wal.toString('ascii',0,8),'NQTX0002')
 assert.equal(wal.toString('ascii',wal.length-40,wal.length-32),'NQCM0002')
 assert.deepEqual(decodeZigWal(wal).commits[0].pages,pages)
 const corrupt=Buffer.from(wal);corrupt[99]^=0x40
 assert.throws(()=>decodeZigWal(corrupt),/SHA-256 mismatch/)
 const truncated=wal.subarray(0,wal.length-1)
 assert.equal(decodeZigWal(truncated).commits.length,0)
 const badHead=Buffer.from(ckpt);badHead[60]^=0x11
 assert.throws(()=>decodeZigCheckpoint(badHead),/SHA-256 mismatch/)
})
test('write, read, checkpoint and reopen native NQDB0002 .noedb',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 assert.equal((await readFile(db)).toString('ascii',0,8),'NQDB0002')
 assert.equal((await stat(db+'.wal')).size,0)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada'])
 assert.equal((await verifyZigNoedb(db)).storageSequence,1)
 await writeZigNoedb(db,insert('Bea'),empty)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada','Bea'])
 assert.equal((await verifyZigNoedb(db)).storageSequence,2)
}))
test('reference read only callback does not rewrite the database',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 const bytes=await readFile(db)
 await writeZigNoedb(db,async previous=>({database:structuredClone(previous),results:[]}),empty)
 assert.deepEqual(await readFile(db),bytes)
}))
test('fault after WAL fsync recovers committed state before checkpoint',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 await assert.rejects(writeZigNoedb(db,insert('Bea'),empty,phase=>{if(phase==='afterWalDirectorySync')throw Error('fault')}),/fault/)
 assert.equal((await verifyZigNoedb(db)).storageSequence,2)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada','Bea'])
 await writeZigNoedb(db,insert('Cam'),empty)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada','Bea','Cam'])
}))
test('incomplete WAL transaction after crash is truncated before next transaction',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 await assert.rejects(writeZigNoedb(db,insert('Bea'),empty,phase=>{if(phase==='afterImageWrite')throw Error('fault')}),/fault/)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada'])
 await writeZigNoedb(db,insert('Cam'),empty)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada','Cam'])
}))
test('checksum corruption fails closed without replacing database',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 const buf=await readFile(db);buf[96+12]^=1;await writeFile(db,buf)
 await assert.rejects(readZigNoedb(db,empty),/SHA-256 mismatch/)
 await assert.rejects(writeZigNoedb(db,insert('Bea'),empty),/SHA-256 mismatch/)
}))
test('flock excludes another simultaneous writer through the same stable inode',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 let release,reach
 const blocker=new Promise(resolve=>release=resolve)
 const entered=new Promise(resolve=>reach=resolve)
 const first=writeZigNoedb(db,insert('Bea'),empty,async phase=>{if(phase==='afterBeginWrite'){reach();await blocker}})
 await entered
 await assert.rejects(writeZigNoedb(db,insert('Cam'),empty),/locked/)
 release();await first
 await writeZigNoedb(db,insert('Cam'),empty)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada','Bea','Cam'])
}))
test('older committed WAL cannot replace a newer checkpoint',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('A'),empty)
 await assert.rejects(writeZigNoedb(db,insert('B'),empty,p=>{if(p==='afterWalSync')throw Error('fault')}),/fault/)
 const old=await readFile(db+'.wal')
 await writeZigNoedb(db,insert('C'),empty)
 await writeFile(db+'.wal',old)
 await assert.rejects(readZigNoedb(db,empty),/stale WAL generation/)
}))
test('migration refuses overwrite of an existing file',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('A'),empty)
 await assert.rejects(writeZigNoedb(db,insert('B'),empty,async()=>{},{newOnly:true}),/already exists/)
}))
test('accepts a zero-byte Zig-initialized .noedb and empty WAL, then writes first commit',()=>fixture(async db=>{
 await writeFile(db,Buffer.alloc(0));await writeFile(db+'.wal',Buffer.alloc(0))
 await writeZigNoedb(db,insert('A'),empty)
 assert.equal((await verifyZigNoedb(db)).storageSequence,1)
}))
test('removing large reference rows scrubs obsolete active Zig pages in the same commit',()=>fixture(async db=>{
 const secret='SUPERSECRET'.repeat(900)
 await writeZigNoedb(db,async before=>({database:{...before,tables:{t:{fields:[],rows:[{data:secret}],nextId:2}}},results:[]}),empty)
 assert.ok((await readFile(db)).includes(Buffer.from('SUPERSECRET')))
 await writeZigNoedb(db,async before=>({database:{...before,tables:{t:{fields:[],rows:[],nextId:2}}},results:[]}),empty)
 assert.equal((await readFile(db)).includes(Buffer.from('SUPERSECRET')),false)
}))
test('subprocess SIGKILL after WAL directory fsync recovers durable new state',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 const url=new URL('../Runtime/noesql-zig-pages.mjs',import.meta.url).href
 const script=`import {writeZigNoedb} from ${JSON.stringify(url)};\nconst make=()=>({format:'noesql-reference-1',name:null,version:1,tables:{},indexes:{},queries:{}});\nawait writeZigNoedb(${JSON.stringify(db)},async before=>({database:{...before,tables:{people:{...before.tables.people,rows:[...before.tables.people.rows,{name:'Bea'}]}}},results:[]}),make,p=>{if(p==='afterWalDirectorySync')process.kill(process.pid,'SIGKILL')});`
 const child=spawnSync(process.execPath,['--input-type=module','-e',script],{encoding:'utf8',timeout:30000})
 assert.equal(child.signal,'SIGKILL',child.stderr)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada','Bea'])
 await writeZigNoedb(db,insert('Cam'),empty)
 assert.deepEqual((await readZigNoedb(db,empty)).tables.people.rows.map(r=>r.name),['Ada','Bea','Cam'])
}))
test('rejects WAL at checkpoint generation with conflicting page payload',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 const content=await readFile(db)
 const pages=decodeZigCheckpoint(content).pages
 const malicious=new Map([[0,Buffer.from(pages.get(0))]])
 malicious.get(0)[100]=1
 await writeFile(db+'.wal',encodeZigTransaction(1,malicious))
 await assert.rejects(readZigNoedb(db,empty),/generation conflict/)
}))
test('reference refuses to overwrite native pages outside its bounded application area',()=>fixture(async db=>{
 await writeZigNoedb(db,insert('Ada'),empty)
 const contents=await readFile(db)
 const snapshot=decodeZigCheckpoint(contents)
 snapshot.pages.set(17,Buffer.alloc(PAGE_BYTES,42))
 await writeFile(db,encodeZigCheckpoint(snapshot.generation,snapshot.pages))
 await assert.rejects(writeZigNoedb(db,insert('Bea'),empty),/native pages beyond reference payload limit/)
}))
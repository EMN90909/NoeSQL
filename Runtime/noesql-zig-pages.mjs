// Byte-for-byte NQDB0002/NQTX0002 compatibility with Native/zig/src/pager_v2.zig.
// This reference codec is NOT a replacement for the Noqeri-compiled native SQL executor.
import {createHash,randomUUID} from 'node:crypto'
import {open,rename,lstat,unlink} from 'node:fs/promises'
import {constants} from 'node:fs'
import {spawn} from 'node:child_process'
import {dirname,resolve} from 'node:path'

export const PAGE_BYTES=4096,MAX_PAGES=256,MAX_TX_PAGES=16
const DB_HEADER=96, DB_FRAME=PAGE_BYTES+32, TX_HEADER=24,TX_FRAME=PAGE_BYTES+4,TX_TRAILER=40
const MAX_WAL_BYTES=64*1024*1024,MAX_DB_BYTES=DB_HEADER+MAX_PAGES*DB_FRAME
const DB_MAGIC='NQDB0002',TX_MAGIC='NQTX0002',COMMIT_MAGIC='NQCM0002',STATE_MAGIC='NQRS0001'
const sum=b=>createHash('sha256').update(b).digest()
const eq=(a,b)=>a.equals(b)
const fail=m=>{throw Error('NOE-ZIG: '+m)}
const int=v=>Number.isSafeInteger(v)&&v>=0
const safeSequence=v=>int(v)&&v<=Number.MAX_SAFE_INTEGER
const bounded=(v,max)=>int(v)&&v<=max
function u64(b,off){const n=b.readBigUInt64LE(off);if(n>BigInt(Number.MAX_SAFE_INTEGER))fail('generation exceeds safe integer range');return Number(n)}
function put64(b,off,v){if(!safeSequence(v))fail('invalid sequence');b.writeBigUInt64LE(BigInt(v),off)}
function shaCheck(b,d,what){if(!eq(sum(b),d))fail(what+' SHA-256 mismatch')}
async function readFileSafe(file,max){
  let handle
  try{handle=await open(file,constants.O_RDONLY|(constants.O_NOFOLLOW||0))}
  catch(e){if(e.code==='ENOENT')return null;throw e}
  try{
    const st=await handle.stat()
    if(!st.isFile()||st.nlink!==1||st.size<0||st.size>max)fail('unsafe or oversized file '+file)
    const buf=Buffer.alloc(st.size)
    let off=0
    while(off<buf.length){const {bytesRead}=await handle.read(buf,off,buf.length-off,off);if(!bytesRead)fail('truncated read');off+=bytesRead}
    const last=await handle.stat()
    if(last.ino!==st.ino||last.size!==st.size||last.mtimeMs!==st.mtimeMs||last.ctimeMs!==st.ctimeMs)fail('file changed during read')
    return buf
  }finally{await handle.close()}
}
export function encodeReferencePages(state){
  const content=Buffer.from(JSON.stringify(state),'utf8')
  const count=Math.ceil(content.length/PAGE_BYTES)
  if(!count||count+1>MAX_TX_PAGES)fail('reference image exceeds atomic 15-page payload limit')
  const header=Buffer.alloc(PAGE_BYTES)
  header.write(STATE_MAGIC,0,'ascii')
  header.writeUInt32LE(1,8)
  header.writeUInt32LE(count,12)
  header.writeUInt32LE(content.length,16)
  sum(content).copy(header,20)
  const pages=new Map([[0,header]])
  for(let i=0;i<count;i++){
    const page=Buffer.alloc(PAGE_BYTES)
    content.copy(page,0,i*PAGE_BYTES,(i+1)*PAGE_BYTES)
    pages.set(i+1,page)
  }
  return pages
}
function decodeStatePages(pages,generation){
  const header=pages.get(0)
  if(!header)fail('missing reference-state manifest page 0')
  if(header.toString('ascii',0,8)!==STATE_MAGIC||header.readUInt32LE(8)!==1)fail('unsupported reference-state page schema')
  const count=header.readUInt32LE(12),length=header.readUInt32LE(16)
  if(!count||count+1>MAX_TX_PAGES||length===0||length>(count*PAGE_BYTES)||Math.ceil(length/PAGE_BYTES)!==count)fail('invalid reference-state byte length')
  if(!header.subarray(52).equals(Buffer.alloc(PAGE_BYTES-52)))fail('noncanonical reference-state header')
  const data=Buffer.alloc(count*PAGE_BYTES)
  for(let i=0;i<count;i++){
    const page=pages.get(i+1)
    if(!page)fail('missing reference-state page '+(i+1))
    page.copy(data,i*PAGE_BYTES)
  }
  const payload=data.subarray(0,length)
  shaCheck(payload,header.subarray(20,52),'reference-state payload')
  if(!data.subarray(length).equals(Buffer.alloc(data.length-length)))fail('nonzero reference payload padding')
  let result
  try{result=JSON.parse(payload.toString('utf8'))}catch{fail('invalid reference-state JSON payload')}
  if(result?.format!=='noesql-reference-1'||!result.tables||typeof result.tables!=='object'||Array.isArray(result.tables))fail('unexpected reference database schema')
  if(result.storageSequence!==generation)fail('reference-state and Zig generation mismatch')
  return result
}
export function encodeZigCheckpoint(generation,pages){
  if(!safeSequence(generation)||!pages||!pages.size||pages.size>MAX_PAGES)fail('invalid checkpoint')
  const top=Math.max(...pages.keys()),image=Buffer.alloc(DB_HEADER+(top+1)*DB_FRAME)
  image.write(DB_MAGIC,0,'ascii');image.writeUInt32LE(PAGE_BYTES,8);image.writeUInt32LE(MAX_PAGES,12);put64(image,16,generation)
  for(const [id,page] of pages){
    if(!bounded(id,MAX_PAGES-1)||!Buffer.isBuffer(page)||page.length!==PAGE_BYTES)fail('invalid checkpoint page')
    image[24+(id>>3)]|=1<<(id&7)
    const at=DB_HEADER+id*DB_FRAME
    page.copy(image,at);sum(page).copy(image,at+PAGE_BYTES)
  }
  sum(image.subarray(0,56)).copy(image,56)
  return image
}
export function decodeZigCheckpoint(image){
  if(!Buffer.isBuffer(image)||image.length<DB_HEADER||image.length>MAX_DB_BYTES)fail('invalid checkpoint length')
  if(image.toString('ascii',0,8)!==DB_MAGIC)fail('incompatible database format (expected NQDB0002)')
  if(image.readUInt32LE(8)!==PAGE_BYTES||image.readUInt32LE(12)!==MAX_PAGES)fail('unsupported Zig page geometry')
  shaCheck(image.subarray(0,56),image.subarray(56,88),'checkpoint header')
  const generation=u64(image,16),pages=new Map()
  for(let id=0;id<MAX_PAGES;id++){
    if(!(image[24+(id>>3)]&(1<<(id&7))))continue
    const off=DB_HEADER+id*DB_FRAME
    if(off+DB_FRAME>image.length)fail('truncated checkpoint page')
    shaCheck(image.subarray(off,off+PAGE_BYTES),image.subarray(off+PAGE_BYTES,off+DB_FRAME),'checkpoint page '+id)
    pages.set(id,Buffer.from(image.subarray(off,off+PAGE_BYTES)))
  }
  return {generation,pages}
}
export function encodeZigTransaction(generation,pages){
  if(!safeSequence(generation)||generation<1||!pages?.size||pages.size>MAX_TX_PAGES)fail('invalid WAL generation/count')
  const size=TX_HEADER+pages.size*TX_FRAME+TX_TRAILER,out=Buffer.alloc(size)
  out.write(TX_MAGIC,0,'ascii');put64(out,8,generation);out.writeUInt32LE(pages.size,16)
  let i=0
  for(const [id,page] of pages){
    if(!bounded(id,MAX_PAGES-1)||!Buffer.isBuffer(page)||page.length!==PAGE_BYTES)fail('invalid transaction page')
    const at=TX_HEADER+i++*TX_FRAME
    out.writeUInt32LE(id,at);page.copy(out,at+4)
  }
  out.write(COMMIT_MAGIC,size-40,'ascii')
  sum(out.subarray(0,size-32)).copy(out,size-32)
  return out
}
export function decodeZigWal(bytes){
  if(!Buffer.isBuffer(bytes)||bytes.length>MAX_WAL_BYTES)fail('WAL too large')
  let off=0,previous=null
  const commits=[]
  while(off<bytes.length){
    const tail=bytes.subarray(off)
    const prefix=Buffer.from(TX_MAGIC,'ascii')
    if(tail.length<TX_HEADER){
      if(!tail.subarray(0,Math.min(tail.length,8)).equals(prefix.subarray(0,Math.min(tail.length,8))))fail('invalid incomplete WAL header')
      break
    }
    if(!tail.subarray(0,8).equals(prefix))fail('invalid WAL magic')
    const generation=u64(tail,8),count=tail.readUInt32LE(16)
    if(!generation||!count||count>MAX_TX_PAGES||tail.readUInt32LE(20)!==0)fail('invalid WAL transaction metadata')
    const size=TX_HEADER+count*TX_FRAME+TX_TRAILER
    if(tail.length<size)break
    const record=tail.subarray(0,size)
    if(record.toString('ascii',size-40,size-32)!==COMMIT_MAGIC)fail('invalid WAL COMMIT marker')
    shaCheck(record.subarray(0,size-32),record.subarray(size-32),'WAL transaction')
    if(previous!==null&&generation!==previous+1)fail('nonsequential WAL generation')
    const pages=new Map()
    for(let i=0;i<count;i++){
      const at=TX_HEADER+i*TX_FRAME,id=record.readUInt32LE(at)
      if(id>=MAX_PAGES||pages.has(id))fail('invalid or duplicate WAL page id')
      pages.set(id,Buffer.from(record.subarray(at+4,at+TX_FRAME)))
    }
    commits.push({generation,pages});previous=generation;off+=size
  }
  return {commits,validBytes:off,incomplete:off!==bytes.length}
}
function recover(checkpoint,wal){
  const base=checkpoint?.length?decodeZigCheckpoint(checkpoint):{generation:0,pages:new Map()}
  const scanned=decodeZigWal(wal||Buffer.alloc(0))
  if(base.generation&&scanned.commits.length&&scanned.commits.at(-1).generation<base.generation)fail('stale WAL generation')
  // A WAL record at the checkpoint generation must agree with that durable
  // checkpoint; otherwise two conflicting histories would be silently merged.
  const repeated=scanned.commits.find(c=>c.generation===base.generation)
  if(repeated){
    for(const [id,page] of repeated.pages){
      if(!base.pages.get(id)?.equals(page))fail('WAL/checkpoint generation conflict')
    }
  }
  const pages=new Map(base.pages)
  let generation=base.generation
  for(const c of scanned.commits){
    if(c.generation<=generation)continue
    if(c.generation!==generation+1)fail('WAL generation gap')
    for(const [id,page] of c.pages)pages.set(id,page)
    generation=c.generation
  }
  if(!generation){if(checkpoint?.length||wal?.length)fail('no durable reference-state commit');return {state:null,pages,generation,scanned}}
  const state=decodeStatePages(pages,generation)
  return {state,pages,generation,scanned}
}
export async function readZigNoedb(path,emptyDatabase){
  return withZigLock(path,async()=>{
    const checkpoint=await readFileSafe(path,MAX_DB_BYTES),wal=await readFileSafe(path+'.wal',MAX_WAL_BYTES)
    const recovered=recover(checkpoint,wal)
    return recovered.state??emptyDatabase()
  })
}
export async function verifyZigNoedb(path){
  return withZigLock(path,async()=>{
    const checkpoint=await readFileSafe(path,MAX_DB_BYTES),wal=await readFileSafe(path+'.wal',MAX_WAL_BYTES)
    if(!checkpoint?.length&&!wal?.length)fail('database does not exist or has no committed state')
    const recovered=recover(checkpoint,wal)
    return {ok:true,tables:Object.keys(recovered.state.tables).length,walPresent:!!wal?.length,storageSequence:recovered.generation,format:DB_MAGIC}
  })
}
async function syncDirectory(path){
  const handle=await open(dirname(resolve(path)),'r')
  try{await handle.sync()}finally{await handle.close()}
}
async function regularOrAbsent(path){
  try{const st=await lstat(path);if(!st.isFile()||st.isSymbolicLink()||st.nlink!==1)fail('unsafe linked storage path')}
  catch(e){if(e.code!=='ENOENT')throw e}
}
async function writeAllFile(path,bytes){
  const f=await open(path,'wx',0o600)
  try{await f.writeFile(bytes);await f.sync()}finally{await f.close()}
}
// Hold the *same* stable .lck inode as Zig's flock(LOCK_EX|LOCK_NB).
// Never unlink .lck: unlinking allows a second lock on a different inode.
async function withZigLock(path,fn){
  if(process.platform!=='linux')fail('reference/native shared locking is currently Linux-only')
  await regularOrAbsent(path+'.lck')
  const child=spawn('flock',['-n',path+'.lck','sh','-c','printf "READY\\n"; IFS= read -r done'],{stdio:['pipe','pipe','pipe'],shell:false})
  let ready=false
  try{
    await new Promise((resolve,reject)=>{
      let line=''
      child.once('error',reject)
      child.stdout.on('data',bytes=>{
        line+=bytes.toString('utf8')
        if(line.startsWith('READY\n')){ready=true;resolve()}
      })
      child.once('close',()=>{if(!ready)reject(Error('NOE-ZIG: database locked or flock unavailable'))})
    })
    await regularOrAbsent(path+'.lck')
    return await fn()
  }finally{
    child.stdin.end('\n')
    if(child.exitCode===null)await new Promise(resolve=>child.once('close',resolve))
  }
}
export async function writeZigNoedb(path,mutate,makeEmpty,inject=async()=>{},{newOnly=false}={}){
  if(!path.endsWith('.noedb'))fail('native reference storage requires .noedb suffix')
  return withZigLock(path,async()=>{
    await regularOrAbsent(path);await regularOrAbsent(path+'.wal')
    const dbBytes=await readFileSafe(path,MAX_DB_BYTES),walBytes=await readFileSafe(path+'.wal',MAX_WAL_BYTES)
    if(newOnly&&(dbBytes!==null||walBytes!==null))fail('destination database already exists')
    const before=recover(dbBytes,walBytes)
    if([...before.pages.keys()].some(id=>id>=MAX_TX_PAGES))
      fail('native pages beyond reference payload limit; reference mutation refused')
    // Never append after a torn WAL suffix; remove only the proven incomplete tail.
    if(before.scanned.incomplete){
      const f=await open(path+'.wal','r+')
      try{await f.truncate(before.scanned.validBytes);await f.sync()}finally{await f.close()}
    }
    // Replay a durable WAL before executing another mutation.
    if(walBytes?.length&&before.generation){await checkpoint(path,before,async()=>{})}
    const prior=before.state??makeEmpty()
    const outcome=await mutate(prior)
    if(!outcome||!outcome.database)fail('mutator must return database and results')
    if(JSON.stringify(outcome.database)===JSON.stringify(prior))return outcome.results
    if(before.generation>=Number.MAX_SAFE_INTEGER)fail('generation exhausted')
    outcome.database.storageSequence=before.generation+1
    const pages=encodeReferencePages(outcome.database)
    // Overwrite obsolete pages in the same transaction so deleted data does not
    // remain in the old active-page slots when the full-state image shrinks.
    for(let id=pages.size;id<MAX_TX_PAGES;id++)if(before.pages.has(id))pages.set(id,Buffer.alloc(PAGE_BYTES))
    const record=encodeZigTransaction(before.generation+1,pages)
    if(record.length>MAX_WAL_BYTES)fail('WAL exceeds resource budget')
    const f=await open(path+'.wal','a',0o600)
    try{
      const head=TX_HEADER,payload=record.length-TX_TRAILER
      await f.writeFile(record.subarray(0,head));await inject('afterBeginWrite')
      await f.writeFile(record.subarray(head,payload));await inject('afterImageWrite')
      await f.writeFile(record.subarray(payload));await inject('afterCommitWrite')
      await f.sync();await inject('afterWalSync')
    }finally{await f.close()}
    await syncDirectory(path);await inject('afterWalDirectorySync')
    const updated=new Map(before.pages)
    for(const [id,page] of pages)updated.set(id,page)
    await checkpoint(path,{pages:updated,generation:before.generation+1},inject)
    return outcome.results
  })
}
async function checkpoint(path,recovered,inject){
  const data=encodeZigCheckpoint(recovered.generation,recovered.pages)
  const temp=resolve(dirname(path),'.noesql-'+randomUUID()+'.tmp')
  let renamed=false
  try{
    await writeAllFile(temp,data)
    await inject('afterSnapshotSync')
    await regularOrAbsent(path)
    await rename(temp,path);renamed=true
    await inject('afterSnapshotRename')
    await syncDirectory(path);await inject('afterDirectorySync')
    // Truncate the WAL *after* the checkpoint is durable. Zig requires its WAL
    // path to remain accessible, so it must not be unlinked.
    const journal=await open(path+'.wal','a+',0o600)
    try{await journal.truncate(0);await journal.sync()}finally{await journal.close()}
    await inject('afterCheckpoint')
  }catch(e){if(!renamed)try{await unlink(temp)}catch{};throw e}
}
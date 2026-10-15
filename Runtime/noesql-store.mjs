// Experimental NoeSQL reference store: append+sync WAL before checkpointing.
// Native Noqeri engine and cross-platform power-loss semantics require further proof.
import { open, rename, unlink, lstat } from 'node:fs/promises'
import { constants as fsConstants } from 'node:fs'
import { dirname, extname, resolve } from 'node:path'
import { randomUUID, createHash } from 'node:crypto'
import { execute, emptyDatabase, runNamed } from './noesql-reference.mjs'
import { newWalTransaction, replayWal } from './noesql-wal.mjs'
const MAX_FILE = 16 * 1024 * 1024
const MAX_WAL = MAX_FILE + 3 * 56 + 64
const checksum = payload => createHash('sha256').update(payload).digest('hex')
const pathCheck = path => { if(extname(path)!=='.nqd') throw Error('NoeSQL reference store path must end in .nqd') }
// Read through one validated descriptor, avoiding a pathname re-open race.
// Bound allocation before reading and reject non-regular and multiply-linked files.
async function boundedRead(file, maximum) {
  const flags = fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW || 0)
  let handle
  try { handle = await open(file, flags) }
  catch (error) { if (error.code === 'ENOENT') return null; throw error }
  try {
    const st = await handle.stat()
    if (!st.isFile() || st.nlink !== 1 || st.size > maximum || st.size < 0)
      throw Error('NOE-SEC: invalid file kind, hard-link count or file size')
    const bytes = Buffer.alloc(st.size)
    let at = 0
    while (at < bytes.length) {
      const { bytesRead } = await handle.read(bytes, at, bytes.length - at, at)
      if (bytesRead === 0) throw Error('NOE-SEC: file truncated during read')
      at += bytesRead
    }
    const after = await handle.stat()
    if (after.size !== st.size || after.ino !== st.ino)
      throw Error('NOE-SEC: file modified during read')
    return bytes
  } finally { await handle.close() }
}
async function regularOrAbsent(file) {
  try { const s=await lstat(file);if(!s.isFile()||s.isSymbolicLink())throw Error('NoeSQL path must be a regular file, never a symlink') }
  catch(e){if(e.code!=='ENOENT')throw e}
}
function serialize(state) {
  const payload=JSON.stringify(state)
  const content=JSON.stringify({magic:'NOESQL-NQD-EXPERIMENTAL',version:1,checksum:checksum(payload),payload})+'\n'
  if(Buffer.byteLength(content)>MAX_FILE)throw Error('NoeSQL snapshot too large')
  return content
}
function decode(content) {
  if(content.length>MAX_FILE)throw Error('NoeSQL snapshot too large')
  let file,state
  try {file=JSON.parse(content.toString('utf8'))}catch{throw Error('NoeSQL snapshot invalid JSON')}
  if(file?.magic!=='NOESQL-NQD-EXPERIMENTAL'||file.version!==1||typeof file.payload!=='string'||file.checksum!==checksum(file.payload))throw Error('NoeSQL snapshot checksum/version mismatch')
  try {state=JSON.parse(file.payload)}catch{throw Error('NoeSQL snapshot payload invalid')}
  if(state?.format!=='noesql-reference-1')throw Error('NoeSQL snapshot state version mismatch')
  return state
}
async function baseState(path) {
  const bytes = await boundedRead(path, MAX_FILE)
  return bytes === null ? emptyDatabase() : decode(bytes)
}
async function walState(path) {
  const file=path+'.wal'
  const bytes = await boundedRead(file, MAX_WAL)
  if (bytes === null) return null
  const parsed=replayWal(bytes)
  return parsed.committed ? decode(parsed.committed) : null
}
export async function readNqd(path) {
  pathCheck(path)
  // If WAL is committed, use its image even when the older base is absent or corrupt.
  const wal=await walState(path)
  return wal || await baseState(path)
}
async function syncDirectory(path) {
  if(process.platform==='win32')return // Windows needs distinct native durability validation.
  const directory=await open(dirname(path),'r')
  try{await directory.sync()}finally{await directory.close()}
}
async function persistNqd(path, state, inject=async()=>{}) {
  const content=serialize(state)
  const temp=resolve(dirname(path),`.noesql-${randomUUID()}.tmp`)
  let file
  try {
    file=await open(temp,'wx',0o600)
    await file.writeFile(content)
    await file.sync()
    await inject('afterSnapshotSync')
  } finally {await file?.close()}
  try {await rename(temp,path);await inject('afterSnapshotRename');await syncDirectory(path);await inject('afterDirectorySync')}
  catch(e){try{await unlink(temp)}catch{}throw e}
}
async function walAppend(path, state, inject) {
  const tx=newWalTransaction(Buffer.from(serialize(state)))
  let file
  try {
    file=await open(path+'.wal','wx',0o600)
    await file.writeFile(tx.frames[0]);await inject('afterBeginWrite')
    await file.writeFile(tx.frames[1]);await inject('afterImageWrite')
    await file.writeFile(tx.frames[2]);await inject('afterCommitWrite')
    await file.sync();await inject('afterWalSync')
    // Directory fsync makes the new WAL name persistent on POSIX.
    await syncDirectory(path);await inject('afterWalDirectorySync')
  } finally {await file?.close()}
}
async function checkpoint(path,state,inject) {
  await persistNqd(path,state,inject)
  // A failed unlink leaves a harmless redo WAL; replay is idempotent.
  try{await unlink(path+'.wal')}catch(e){if(e.code!=='ENOENT')throw e}
  await syncDirectory(path)
  await inject('afterCheckpoint')
}
export async function verifyNqd(path) {
  const state=await readNqd(path)
  return {ok:true,tables:Object.keys(state.tables).length,walPresent:await (async()=>{try{await lstat(path+'.wal');return true}catch(e){if(e.code==='ENOENT')return false;throw e}})()}
}
export async function runNqd(path, source, params={}, options={}) {
  pathCheck(path)
  if (typeof source === 'string' && Buffer.byteLength(source, 'utf8') > 1024 * 1024)
    throw Error('NOE-SEC: query source exceeds 1 MiB')
  const inject=options.inject || (async()=>{})
  let holder
  try{holder=await open(path+'.lock','wx',0o600)}catch(e){if(e.code==='EEXIST')throw Error('NoeSQL locked; inspect stale lock manually');throw e}
  try {
    const before=await readNqd(path)
    // Finish outstanding recovery before accepting a new transaction.
    try{await lstat(path+'.wal');await checkpoint(path,before,async()=>{})}catch(e){if(e.code!=='ENOENT')throw e}
    const outcome=execute(source,before,params)
    await walAppend(path,outcome.database,inject)
    await checkpoint(path,outcome.database,inject)
    return outcome.results
  } finally {await holder.close();await unlink(path+'.lock')}
}
export async function namedNqd(path,name,params={}) {return runNamed(await readNqd(path),name,params)}

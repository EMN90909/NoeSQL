// Experimental reference write-ahead log; owned engine will implement the same
// documented wire format in .nqr. This is not a replacement for native WAL.
import { createHash, randomBytes } from 'node:crypto'
const MAGIC = Buffer.from('NWAL')
const VERSION = 1, HEADER = 56, MAX_IMAGE = 16 * 1024 * 1024
const MAX_TXID = (1n << 64n) - 1n
const digest = bytes => createHash('sha256').update(bytes).digest()
export const WAL_TYPES = Object.freeze({ BEGIN: 1, IMAGE: 2, COMMIT: 3 })
export function walFrame(type, txid, seq, payload = Buffer.alloc(0)) {
  if (![1,2,3].includes(type) || typeof txid !== 'bigint' || txid <= 0n || txid > MAX_TXID || !Number.isInteger(seq) || seq < 0 || seq > 0xffffffff || !Buffer.isBuffer(payload) || payload.length > MAX_IMAGE) throw Error('NOE-WAL: invalid frame')
  const header = Buffer.alloc(HEADER)
  MAGIC.copy(header)
  header.writeUInt16LE(VERSION, 4)
  header.writeUInt8(type, 6)
  header.writeUInt8(0, 7)
  header.writeBigUInt64LE(txid, 8)
  header.writeUInt32LE(seq, 16)
  header.writeUInt32LE(payload.length, 20)
  const hash = digest(Buffer.concat([header.subarray(0,24), payload]))
  hash.copy(header,24)
  return Buffer.concat([header,payload])
}
export function newWalTransaction(image, txid = randomBytes(8).readBigUInt64LE() || 1n) {
  if (!Buffer.isBuffer(image) && !(image instanceof Uint8Array)) throw Error('NOE-WAL: image must be bytes')
  const bytes = Buffer.isBuffer(image) ? image : Buffer.from(image)
  if (bytes.length < 1 || bytes.length > MAX_IMAGE) throw Error('NOE-WAL: image out of bounds')
  const frames = [
    walFrame(WAL_TYPES.BEGIN,txid,0),
    walFrame(WAL_TYPES.IMAGE,txid,1,bytes),
    walFrame(WAL_TYPES.COMMIT,txid,2,digest(bytes))
  ]
  return {txid, frames, bytes:Buffer.concat(frames)}
}
// Strict scanner: complete corrupt frames always fail; an incomplete suffix can
// be uncommitted and is not considered durable. Committed records are never skipped.
export function replayWal(buffer) {
  if (!Buffer.isBuffer(buffer) && !(buffer instanceof Uint8Array)) throw Error('NOE-WAL: input must be bytes')
  if (!Buffer.isBuffer(buffer)) buffer = Buffer.from(buffer)
  if (buffer.length > MAX_IMAGE + HEADER * 3 + 64) throw Error('NOE-WAL: file too large')
  let at=0, txid=0n, image=null, expectedSeq=0, stage=0, committed=null, incomplete=false
  while(at < buffer.length) {
    if (buffer.length - at < HEADER) { incomplete=true;break }
    const head=buffer.subarray(at,at+HEADER)
    if(!head.subarray(0,4).equals(MAGIC) || head.readUInt16LE(4)!==VERSION || head[7]!==0) throw Error('NOE-WAL: frame magic/version/flags mismatch')
    const type=head[6], tx=head.readBigUInt64LE(8), seq=head.readUInt32LE(16), size=head.readUInt32LE(20)
    if(size>MAX_IMAGE) throw Error('NOE-WAL: excessive payload')
    if(buffer.length - at - HEADER < size) { incomplete=true;break }
    const payload=buffer.subarray(at+HEADER,at+HEADER+size)
    if(!head.subarray(24,56).equals(digest(Buffer.concat([head.subarray(0,24),payload])))) throw Error('NOE-WAL: checksum mismatch')
    if(tx<=0n || seq!==expectedSeq || (stage===0&&type!==1) || (stage===1&&type!==2) || (stage===2&&type!==3) || stage===3 || (stage!==0&&tx!==txid)) throw Error('NOE-WAL: invalid transaction sequence')
    if(stage===0) { if(size!==0)throw Error('NOE-WAL: BEGIN payload'); txid=tx;stage=1 }
    else if(stage===1) { if(size===0)throw Error('NOE-WAL: empty image');image=payload;stage=2 }
    else { if(size!==32 || !payload.equals(digest(image)))throw Error('NOE-WAL: invalid COMMIT digest'); committed=Buffer.from(image);stage=3 }
    expectedSeq++;at+=HEADER+size
  }
  if (committed && (incomplete || at!==buffer.length)) throw Error('NOE-WAL: trailing bytes after COMMIT')
  return { committed, incomplete, frameCount:expectedSeq, txid }
}

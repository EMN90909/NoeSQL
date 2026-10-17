import test from 'node:test'
import assert from 'node:assert/strict'
import { walFrame, newWalTransaction, replayWal, WAL_TYPES } from '../Runtime/noesql-wal.mjs'

test('WAL rejects out-of-range IDs and sequence values', () => {
  for (const id of [0n, -1n, 1n << 64n]) {
    assert.throws(() => walFrame(WAL_TYPES.BEGIN, id, 0), /invalid frame/)
  }
  for (const seq of [-1, 2 ** 32, 1.5, Number.MAX_SAFE_INTEGER]) {
    assert.throws(() => walFrame(WAL_TYPES.BEGIN, 1n, seq), /invalid frame/)
  }
})
test('WAL API rejects non-byte content instead of coercing strings', () => {
  assert.throws(() => walFrame(WAL_TYPES.IMAGE, 1n, 1, 'payload'), /invalid frame/)
  assert.throws(() => newWalTransaction('payload'), /image must be bytes/)
  assert.throws(() => replayWal('NWAL'), /input must be bytes/)
})
test('WAL accepts valid Uint8Array and committed binary roundtrip', () => {
  const transaction = newWalTransaction(new Uint8Array([1, 2, 3]), 23n)
  const result = replayWal(transaction.bytes)
  assert.deepEqual([...result.committed], [1, 2, 3])
  assert.equal(result.txid, 23n)
})

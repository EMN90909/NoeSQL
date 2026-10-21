// Reference-only application boundary. Native Stage-2 integration is not qualified.
// This module always delegates storage to NoeSQL's canonical .noedb pager.
import { extname } from 'node:path'
import { readNqd, runNqd, namedNqd, verifyNqd } from './noesql-store.mjs'
const valid = file => {
  if (typeof file !== 'string' || extname(file) !== '.noedb') throw new TypeError('NoeSQL database() requires a .noedb file; migrate legacy .nqd/.nqdb first')
  return file
}
export function database(file) {
  const path = valid(file)
  // No long-lived connection is simulated here: every operation uses the engine's
  // atomic write/WAL path, not a competing JS persistence implementation.
  return Object.freeze({
    path,
    execute: source => runNqd(path, source),
    query: (name, parameters = {}) => namedNqd(path, name, parameters),
    snapshot: () => readNqd(path),
    verify: () => verifyNqd(path),
    atomic: source => runNqd(path, 'atomic {\n' + source + '\n}'),
    // An interactive transaction is not provided until a true native session ABI
    // preserves isolation across prepare, query and commit.
    begin: () => { throw new Error('NOE-SESSION: interactive transactions require the unqualified native session driver; use atomic()') }
  })
}

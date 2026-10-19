# NoeSQL unified .noedb format (19 October 2026)

`.noedb` now uses the same `NQDB0002` page and `NQTX0002` WAL wire format as Zig. See [unified format details](NOESQL_UNIFIED_NOEDB_FORMAT.md). Legacy reference `.nqd`/`.nqdb` files remain JSON/NWAL and are migrated explicitly, never silently.

# NoeSQL .nqds source and .noedb storage (19 October 2026)

## Naming contract

- `.nqds` is a NoeSQL query, schema, migration, and named-query source script. The reference CLI accepts it; legacy `.nqd` and `.noe` scripts still work.
- `.noedb` is the preferred database filename in the reference runtime and Zig pager. Legacy `.nqdb` (both) and `.nqd` (reference only) remain accepted.
- **The shared extension now means shared page/WAL bytes** for `.noedb`. The Zig pager can open reference checkpoints and read reference payload pages. The native Noqeri SQL executor is not yet connected. Legacy `.nqd`/`.nqdb` files still use their original JSON/NWAL; embedded SQLite continues to use `.sqlite`.

## Reference durability and isolation

On Linux, the `.noedb` bridge holds an exclusive `flock` on the **same persistent `.lck` inode as Zig** for reads and writes, clones state to guarantee atomic reference program execution, appends an NQTX0002 WAL transaction, fsyncs the journal and directory, atomically replaces an NQDB0002 checkpoint, syncs the directory, and truncates the WAL. Incomplete WAL transactions are ignored or truncated before a new write. A crash releases the OS lock without unlinking `.lck`. Read-only programs do not write a new transaction. Legacy JSON files continue using `.lock` and NWAL.

These are **bounded guarantees**, not production-grade ACID certification. The bridge serializes readers and writers rather than using MVCC; its full-state payload is limited to 15 data pages per atomic mutation; and cross-platform, filesystem power-loss, large-database, and native Noqeri SQL release qualification remain blocked. An error before WAL fsync may leave an uncertain commit outcome. This pager fails closed on a corrupt checkpoint even if a separate WAL image survives; do not promise automatic recovery from arbitrary checkpoint corruption.

## Native qualification

`npm run test:zig` verifies the isolated Zig pager. `npm run build:native-link` still requires pinned self-hosted Noqeri Stage-2 compiler provenance and actual machine-code generation and linking. `npm run qualify:native-e2e` demands a real native CLI that accepts `.nqds` and `.noedb`, executes the relational workflow, recovers storage, and produces genuine `NQDB0002` bytes. Neither script may fall back to the JS reference or an untrusted compiler. Native SQL and public npm publication remain blocked until these gates pass on verified artifacts.

## Grouped reference execution optimization (19 October 2026)

The bounded reference relational selector now computes grouped count, sum, min and max in streaming per-group state, preserving NULL semantics, checked integer overflow, and the 100,000 matching-row safety limit. A pure predicate on base-table columns is evaluated before an inner join only when the expression is a safe combination of comparisons and boolean operators. Query-local hash join remains the equality-join path. These improvements avoid retaining every matched row solely to aggregate it; they are not evidence of native SQL parity or a cost-based optimizer comparable to established databases.

## Monotonic reference commit sequence

New reference checkpoints include a safe-integer `storageSequence` that increases once per committed write (legacy checkpoints without the field are treated as sequence zero). When both a valid checkpoint and a committed WAL exist, the reader refuses a lower WAL sequence and refuses conflicting same-sequence images. For `.noedb` the WAL/checkpoint generation is the Zig v2 generation and the reference state must match it; stale WAL generations are rejected. A corrupt checkpoint is rejected before replay, consistent with the Zig pager. Checksums do not prevent malicious rollback or authenticated tampering. A scalable native row/index layout and compiled Noqeri SQL executor are still required.

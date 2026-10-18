# Stage 2: Zig WAL pager and Noqeri integration status (2026-10-18)

**State: experimental. Version remains 0.7.5-rc.2. Never label v1.0 or claim native SQL is fully integrated.**

## Implemented source

- `Native/zig/src/pager_v2.zig`: Zig-owned bounded **Linux-only** pager (256 pages, 4096 bytes each). Appends only changed pages inside a checksummed transaction record to `.nqdb.wal`; `fsync` occurs before reporting a successful commit and before changing visible in-memory state.
- `Pager.recover()`: validates complete WAL records and strictly increasing generations; replays committed changes; removes incomplete tails under a nonblocking OS writer lock; fails closed on invalid complete records. Checkpoint creates a new complete checksummed snapshot, fsyncs, atomically renames, fsyncs the parent directory, then truncates/fsyncs the WAL.
- `Native/zig/src/index_v2.zig`: persisted single-page sorted numeric unique index, checksum verified, binary search and bounded range scan (max `CAPACITY` entries; **not** a B+tree or relational planner). Index pages use exactly the same WAL transaction path.
- `Native/zig/src/storage_abi.zig`: Zig-owned native page ABI v2 for trusted Noqeri callers, distinct from the v1 query dispatch bridge. The API deliberately has no SQL parser or public authentication layer.
- `Database/noesql_parser.nqr`: the expression precedence parser synchronized from **EMN90909/Noqeri**, not invented as a compiled SQL statement frontend.
- `Database/noesql_pager_abi.nqr`: Noqeri-owned host ABI declarations. The native Noqeri compiler has **not** produced or linked its executor against the Zig pager.
- `scripts/verify-noqeri-bootstrap.mjs`: checks the pinned Git commit, compiler source closure, completed stages/suites, binary hashes and proof artifact paths from Noqeri's `scripts/bootstrap-stage2.mjs`. It is fail-closed by default. **A self-reported JSON file alone is not a signed trusted compiler attestation.** A real verified compiler and independent build provenance are still required.

## Memory, durability and concurrency limits

- Separate `NQDB0002` / `NQTX0002` disk magic, **not compatible** with C++ `NQPG0001`, JS reference JSON, or SQLite. **Never write existing user files in-place**; create a fresh v2 database and retain backed-up legacy source/data.
- 256 fixed data pages, 16 pages per transaction and 64 MiB bounded active WAL. `checkpoint()` re-encodes a complete snapshot. Not a large-scale incremental checkpoint.
- Linux POSIX libc filesystem model with trusted private parent directory; nonblocking exclusive process writer lock; concurrent immutable in-process page snapshots, **not** MVCC or simultaneous cross-process readers.
- Checksums detect incidental corruption; no authenticated encryption, tenant identity, multi-writer concurrency or secure cross-process SQL transaction API yet.
- No automatic migration or restore of legacy formats; backup/restore and migration remain release blockers.
- Native source is **not compiler-tested** unless a Zig 0.15.2 run explicitly succeeds. Fault-injection tests cover malformed records, checksums and torn tails, but not physical power cuts, ENOSPC, every fsync/rename failure or full host compromise.

## Run and qualify

```sh
node --test tests/noqeri-bootstrap.test.mjs
(cd Native/zig && zig build test)
(cd Native/zig && zig build -Doptimize=ReleaseSafe)
bash scripts/benchmark-zig-v2.sh 250
npm run qualify:zig
```

The C narrow durable-write benchmark compares 4KiB fsynced commits to SQLite WAL/FULL and reports per-operation p50/p95/p99. It is a *methodology harness*, not a benchmark result and not relational-query performance; run only on a comparable fresh device and report raw values.

Noqeri compiler evidence needs an accessible local Noqeri checkout and pins:
```sh
export NOQERI_ROOT=/path/to/Noqeri
export NOQERI_COMMIT_SHA=<reviewed Git commit hash>
export NOQERI_SOURCE_CLOSURE_SHA256=<reviewed compiler source closure SHA256>
npm run verify:noqeri
```
Expected without authentic actual bootstrap proof: nonzero exit.

## Explicit remaining Stage 2 production blockers

1. A genuinely self-hosting, independently authenticated executable Noqeri compiler with full statement frontend.
2. End-to-end compiled Noqeri typed executor → Zig page ABI → durable query, with constraints, joins and prepared params.
3. Variable-sized page allocator/free list and B+tree/fanout index splits, persisted multi-column indexes, cost-based query planner.
4. Snapshot isolation and concurrent readers/writer, cancellation, row-level authorization and encryption/key rotation.
5. Deterministic IO syscall fault matrix, crash/restart and real power-loss verification, backup/restore and backwards migration.
6. Linux/macOS/Windows verified tests, CI that actually executes, benchmark replicates versus SQLite and PostgreSQL, signed release artifacts.

**Do not move release status beyond experimental until all six are observed, tested and independently reviewed.**

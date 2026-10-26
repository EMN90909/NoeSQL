# NQDB0003 as default CLI engine — performance and safety gate

## Scope of this branch

The native CLI's standard `run`, `query`, `explain`, `schema-check`,
`export-sql`, `export-sqlite`, `backup` and `restore` paths now target
**NQDB0003**. The first successful mutation on a nonexistent `.noedb`
path creates a new v3 database atomically. Reads on missing databases fail.
Existing NQDB0002 files are NEVER silently upgraded or overwritten.

The original Zig v2 engine is still present. Operators of existing
NQDB0002 files should use explicit `v2-run`, `v2-query`, `v2-explain`,
`v2-schema-check`, `v2-export-sql`, `v2-export-sqlite`,
`v2-backup` and `v2-restore`. To migrate, first run
`noesql migrate-v3 /absolute/legacy.noedb /absolute/new.noedb`, verify
the generated v3 with `noesql v3-check` and compare query results before
switching your application's configured database path. Migration preserves
the old source and requires a new target. No transparent online cutover or
cross-process replication between v2 and v3 is implied.

## Targeted speed improvements

1. Private v3 commit execution no longer clones the complete relational
   Database a second time or encodes a redundant before/after snapshot.
   Failed scripts are discarded without publication. Idempotent zero-row
   UPDATE and DELETE statements do not create another immutable generation.
2. Zig-backed private writes similarly avoid an extra transaction-wide copy.
3. Explicit single-column integer indexes, unique integer columns and
   integer primary keys can use a **lazily built, immutable, bounded-fanout
   in-memory B+ lookup tree** when the WHERE predicate is AND-conjoined
   equality to an integer literal or typed parameter. Index candidates are
   rechecked through the full query predicate before projection. The query
   result preserves original row insertion order unless explicitly sorted.
4. The B+ tree's leaf and branch capacities are 240 entries, sized for a
   prospective 4 KiB disk encoding. This is **not** an implemented or persisted
   4 KiB B+ disk-page format. Cold snapshot loads still deserialize all
   relational tables; writes still rewrite the complete generation.
   Index caches live only as long as an in-memory database/snapshot and
   are invalidated on writes through the database APIs. Do not directly
   mutate exposed trusted `Table.rows` after the cache has been populated.
5. `EXPLAIN` emits `access=memory_bplus(field)` only for a supported
   path and `access=full_scan` otherwise.

## Required test matrix before a main-branch rollout

```sh
make test
make stress
make test-sqlite
cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets
NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" \
  cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets --features zig-pager,sqlite-adapter
make benchmark-bplus
```

Run the v3 multi-process, failpoint, corrupted-head, interrupted-migration
and >256-page tests on a real Linux runner. Benchmark the same datasets and
hardware against the previous v2 and opt-in v3 build; report p50/p95 latency,
throughput, cold/warm indexed reads, WAL/HEAD fsync behavior, memory footprint,
CPU and disk amplification. Never infer a performance speedup merely because
the index exists or the new path avoids two encodings.

## Important blockers

This source branch has **not** passed compiler or durability tests. The hosted
GitHub Actions jobs are failing before any job steps are returned, while the
editing environment contains no Rust/Zig compiler and cannot reach public
toolchain downloads. Do not mark the v3 default deployment production-ready,
merge the storage changes into `main`, publish release artifacts, or drop v2
backups before these checks run.

v3 still has fully materialized generation writes, no incremental native B+
page WAL, no persisted secondary index structures, no safe generation garbage
collector, no live atomic v2-to-v3 application cutover, and no verified
production security/linearizability gates. The v3 CLI default on this **draft
branch** does not change the main-branch production release.

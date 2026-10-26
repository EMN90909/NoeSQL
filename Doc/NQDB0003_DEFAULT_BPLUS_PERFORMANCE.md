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

1. Private v3 commit execution avoids the second relational Database clone
   and redundant before/after snapshot serializations. Failed scripts remain
   private; no-op zero-row UPDATE/DELETE does not publish another generation.
2. Explicit single-column integer indexes, key/unique int columns and
   declared secondary indexes use bounded-fanout B+ equality lookup. Full
   query predicates are rechecked, and unsupported expressions scan safely.
3. **NEW: persistent 4 KiB B+ node pages.** Index leaves and branches are
   canonical fixed-size pages with SHA-256 content-addressed filenames in
   `<db>.versions/index-pages/`. A checksummed index manifest is embedded
   as trailing page frames in each NQDB0003 generation snapshot; the
   generation HEAD digest covers those frames. Per-page SHA-256 and row-value
   validation are performed on cold reopen. Corrupt committed indexes fail
   closed rather than silently rebuilding.
4. **Logarithmic on-disk B+ insertion/deletion.** A new stable-primary-key
   page format (`NQBI0002`, `NQIM0002`) represents each leaf as a sorted
   (indexed value, primary key) pair. Changing a key descends to one leaf
   and copy-on-write updates its ancestors; splits, sibling borrowing,
   merges and root contraction maintain occupancy in O(log n) node reads
   and physical writes per changed entry. Row-vector deletion no longer
   changes the identity of surviving B+ entries.
5. **Transaction delta application.** Before mutation, the engine captures
   indexed pairs from affected tables. It compares those pairs with the
   committed result, then applies exactly the removed and added pairs
   through logarithmic tree operations. Unchanged subtrees retain their
   content hashes. Unaffected column indexes retain their original roots.
   New node files are first written to private temporary files, fsynced,
   atomically hard-linked under their SHA-256 names and directory-synced
   **before** generation HEAD publication.
6. **One-time index-format upgrade.** The previous `NQIM0001` index
   manifest remains readable. On its next write, the index is bulk-built
   once into stable-primary-key `NQIM0002`; subsequent changes use
   logarithmic mutation instead of full tree reconstruction.
7. Indexes are loaded and checked during NQDB0003 cold reopening and
   installed in the relational query cache. This avoids rebuilding trees
   solely by scanning all rows, although verifying relational agreement
   still visits indexed table rows.
8. The original v3 page-header extension byte range (28–32) was reserved
   and zero. Zero continues to mean a pre-index generation with no
   persistent index manifest. Nonzero specifies the number of extra
   index-manifest frames appended after relational data. Older v3
   generations remain readable.

**Remaining limits:** A single B+ key mutation has O(log n) node
read/write complexity, but extracting the transaction's changed index pairs
still scans affected table rows (O(n)). The relational snapshot remains
fully serialized into new generation data pages on each committed write.
This is not yet an end-to-end O(log n) database transaction.
Secondary indexes support integer equality, not persisted text, composite,
range or covering-index scans. There is no automatic garbage collection
for historical page blobs or standalone page-level WAL.
The implementation remains unqualified until native tests execute.

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

v3 still has fully materialized relational generation writes, no
standalone incremental B+ page WAL, no persisted text/composite/range index
access path, no safe generation garbage collector, no live atomic v2-to-v3
application cutover, and no verified production security/linearizability
gates. Integer equality B+ trees now use path-copy updates rather than
reconstruction, but discovery of changed entries and full relational
snapshot persistence are still linear in table/database size.
The v3 CLI default on this **draft branch** does not change the main-branch
production release.

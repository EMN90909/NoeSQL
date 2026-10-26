# NoeSQL hardening and v3 acceptance gates

Updated: 2026-10-26. **Do not ship this branch as production-ready.** It is a staged patch atop the prior concurrency fixes. Changes marked *prototype* are not wired to the live ABI.

## Current scope and demonstrated code changes

| Priority | Requirement | Status in this branch | Hard release gate |
| --- | --- | --- | --- |
| P0 | CI / release protection | Reusable Rust/Zig/Noqeri + session jobs, fail-closed release attestation; GitHub-hosted jobs currently fail before recording test steps | Executing green CI and required branch checks configured by administrator |
| P0 | Incremental page durability | Live v2 snapshots stage only changed data pages plus changed checksum manifest, within one WAL transaction | Dirty-page/no-op/replay and crash-boundary stress suites |
| P0 | Larger database capacity | **Prototype:** bounded 65,536-page allocation bitmap and v3 checksummed manifest; live v2 remains 256 pages | Full v3 pager, WAL, indexes, migration, rollback and fuzz validation |
| P0 | Reader concurrency / isolation | Existing exclusive lock serializes readers and writers; previous PR fixes lost CLI updates | Multi-process snapshot-isolation, consistency, fairness, crash tests; not satisfied |
| P0 | Native relational Noqeri | Existing scalar Noqeri bridge; Rust implements full relational semantics | Verified native relational CRUD/joins/aggregates via Noqeri ABI and Zig WAL; not satisfied |
| P1 | Index/query planning | CLI `explain` reports actual full-scan/hash/nested-loop choice without claiming fake index usage | On-disk B+tree, composite keys, stats, costed plans and index/WAL tests |
| P1 | Security | Existing bounded parser/ABI; no multi-tenant authorization boundary | Auth, row policy, resource quotas, secret handling, audit, tenant escape tests |
| P1 | Schema migrations | Existing alter-add and version metadata | Transactional migration ledger, hashes, repeat application, rollback/restore, compatibility |
| P1 | Backup/restore | **Initial:** source-locked, verified, checkpointed single-file no-clobber backup & fresh-target restore | Live multi-reader backups, fault injection/retention/restore on external machines |
| P1 | Rich data types | Existing int/bool/text/ref only | Decimal rounding, timezone, JSON/binary validation, sort/index/encoding interop |
| P2 | Developer experience | CLI `explain`, `backup`, `restore`; Rust parser tests/benchmarks | REPL, error spans, stable SDK, formatter, editor support |
| P2 | Ecosystem integration | Existing Rust session ABI with experimental native scalar Noqeri bridge | Stable ABI, versioned bindings, optional SQLite/Postgres adapters and compatibility tests |

### Important distinction: dirty-page WAL versus incremental relational storage

The live `NQRS0001` snapshot remains a single serialized Rust payload. Rust still clones the mutable database for statement-level atomicity and encodes the whole payload after writes. Zig now compares each 4 KiB page with the committed page and WAL-commits only the changed pages and checksum manifest. This reduces WAL bytes, but it does **not** add per-row storage, sparse logical allocation, MVCC, or remove the 256-page v2 ceiling.

### Proposed v3 on-disk format (not active)

- Magic `NQDB0003`, explicit format and feature bits, page size, generation, allocation bitmap roots, free-space roots, double superblock; never reinterpret `NQDB0002`.
- Page IDs at least 32-bit, with typed page headers, checksums, length bounds and page-generation tagging; reserve page zero for metadata.
- WAL records include transaction ID, generation, checksum, previous LSN and an explicit durable commit marker; replay is idempotent, detects torn tails, and never applies uncommitted pages.
- Page allocator can track 65,536 pages in the standalone Zig prototype. Live version must durably WAL-log alloc/free operations in the same transaction as data/index updates; no page may be reused until safe relative to old readers.
- A reader snapshot pins a visible commit generation. Multiple readers use immutable snapshot pages while a single writer commits, with explicit busy policy, lock ordering, deadlock/timeout handling and fairness metrics.
- The existing Rust snapshot must become a versioned relational catalog and table/page layout; migrate indexes and constraints without losing native Noqeri semantics.

### Migration protocol for production v3

1. Validate source v2 snapshot and complete WAL, reject unknown formats and active mutations; obtain exclusive migration lock.
2. Make a verified read-only backup and record its checksum, byte size, format, generation and original path.
3. Build **shadow v3 files**, copy catalog/schema/rows and indexes and verify referential constraints and row counts.
4. fsync shadow files and parent directory; crash injection at every publication boundary.
5. Atomically publish a version/manifest pointer or use a journaled rename protocol preserving the verified v2 backup; fsync parent again.
6. On restart: deterministically select last fully committed format, never silently downgrade/upgrade, and never delete the original before verification.
7. Compare old/new query results, index scans, permissions, and join/aggregate outputs. If verification fails, restore only from the verified backup via a documented rollback procedure.

### CI and release policy

Run `make test`, `make stress`, native Noqeri AOT + Zig tests, Stage-2 NQRI chain, cross-mode tests and backup/restore regression. Release qualification calls the native and session workflows; **no attestation is emitted if either fails or is skipped**. An attestation is only an execution receipt, not a proof of security.

**Known external blocker:** GitHub Actions workflow runs currently fail before exposing job steps. Investigate Settings -> Actions (runner access, permitted actions, workflow permissions, enabled services) and billing/minutes if relevant. The repository's branch-protection endpoint returned 403 and its ruleset endpoint indicates the current private-repository plan does not support rulesets. These settings cannot be fixed by editing this repository. A repository administrator must enable a plan/visibility supporting protected branches and mark `Noqeri + Rust + Zig` and `Rust session + Zig durability` as required checks before a merge can be reliably blocked. If Noqeri is private, provide a minimal **read-only** `NOQERI_READ_TOKEN` secret for the pinned compiler checkout.

### Required performance and fault benchmarks

Run on Linux x86-64 with Zig 0.15.2 and a pinned Rust toolchain; save toolchain versions, machine CPU, storage/fs, commit SHA, database size and warm/cold cache conditions. Measure 1/8/32/128 clients, 90/10 and 50/50 read/write mixes, transactions per second, p50/p95/p99 latency, fsync/commit durability, memory, WAL bytes per update, retry/timeout rate and recovery duration. Replay under process crashes and injected short writes and low disk space. Only publish a speedup after comparing identical workloads against the prior baseline on the same hardware.

### Local commands

```sh
make test
make stress
NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets --features zig-pager
make benchmark-parser-reads
```

The v3 allocator tests are wired into `Native/zig/build.zig test`, but remain an isolated prototype until the transaction/logging format is implemented and verified.

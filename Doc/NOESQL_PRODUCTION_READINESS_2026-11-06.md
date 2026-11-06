# NoeSQL production-readiness audit and hardening gate — 2026-11-06

**Decision: NOT PRODUCTION READY. Do not tag v1.0, claim an ACID-certified engine, or expose the CLI/native administrative ABI to untrusted users.** This audit is based on source inspection, existing tests, and the repository's explicit release-gate documents. The Rust/Zig toolchains are not present in the authoring environment; neither compile nor durability execution is verified here.

## Code paths inspected

- `Native/rust/src/reference.rs` — relational AST, CRUD, constraints, joins and aggregations.
- `Native/rust/src/authorization.rs`, `mvcc.rs`, `relational_abi.rs` — optional in-process grants, row filters, optimistic transactions and FFI boundary.
- `Native/rust/src/v3_store.rs`, `v3_bplus_disk.rs`, `v3_bplus_cow.rs`, `codec.rs` — immutable v3 HEAD/generations, persistent B+ pages, checksums and serialization.
- `Native/zig/src/pager_v2.zig`, `storage_abi.zig` — legacy v2 WAL, locking, atomic checkpoints and snapshots.
- `Database/*.noqeri`, `Native/rust/src/noqeri_native.rs`, `stage2_nqri.rs` — Noqeri integration boundaries.
- `Native/rust/tests/*`, `Native/zig/build.zig`, `Makefile`, existing release gates.

## This branch's concrete changes

| Component | Implementation | Status |
| --- | --- | --- |
| v3 new metadata, generation and index files | Create with owner-only mode (0600); newly created versions/index directories with 0700 on Unix | Source changed, tests added; **not executed** |
| v3 final-component symlink defense | Linux `O_NOFOLLOW` when opening lock, HEAD, generation and index-blob files; require regular files | Source changed, tests added; **not executed** |
| Existing v3 permission exposure | Read-only `noesql v3-audit-permissions <absolute-db.noedb>` audits HEAD, lock, all retained generation files and index blobs | Source changed, tests added; **not executed** |
| v3 restricted embedded execution | `v3_store::execute_authorized` uses `AccessControl::prepare` for grants and row scoping and validates result budget **before** publishing HEAD | Source changed, tests added; **not executed** |
| Host script reader | `read_source` reads at most `MAX_SOURCE_BYTES + 1` bytes before rejecting, instead of allocating the entire script | Source changed, test added; **not executed** |
| v3 lock acquisition | Fail immediately on non-contention errors instead of retrying every OS error for 10 seconds | Source changed; **not executed** |
| Local evidence gate | `bash scripts/qualify-production-local.sh` demands pinned Linux/Rust/Zig, Rust/Zig/native Noqeri/Stage-2 suites and stress runs; no GitHub Actions required | Script added; **not executed** |

**These changes are hardening, not engine-wide isolation or complete production qualification.** The unrestricted `execute`, `execute_if_generation`, raw `Database`, CLI, and FFI/admin entry points intentionally remain trusted administrative APIs. A caller must supply a separately authenticated principal and carefully controlled `AccessControl`; permissions are NOT stored durably, and the database provides no public network authentication service.

New file permissions apply only to newly created files. Previously created v3 files are **not automatically chmod'd**. The permission auditor is non-mutating. `O_NOFOLLOW` protects the final component only; it does NOT prevent replacement of a writable ancestor directory, malicious hardlinks, or compromised administrators. Deploy only under a privately owned, non-writable-by-others parent directory. Encryption at rest is not provided.

## Remaining release blockers (prioritized)

### P0 — data integrity, crash safety and production limits

1. **Prove durability, not only source-level atomicity.** Execute Linux power-loss/process-kill, short-write, ENOSPC, checkpoint, fsync/rename, truncated-page, corrupt HEAD/manifest and restart tests across both v2 and v3; record independent evidence and non-ambiguous/ambiguous commit behavior.
2. **Bound v3 retention and write amplification.** Every mutating v3 transaction writes a full serialized database generation; historical generations and content-addressed B+ blobs are not reclaimed. Implement safe reachability-aware, crash-resilient reclamation before a write-heavy production rollout. Never delete page files merely because their generation is not current.
3. **Qualify relational concurrency.** Demonstrate linearizable single-writer commits, pinned-reader snapshots, fairness, restart behavior, stale generation rejection, and bounded lock wait at 1/8/32/128 clients and during independent process termination. The in-memory MVCC module is not persistent database MVCC.
4. **Canonical on-disk format and migrations.** Golden-byte fixtures, backwards reading, typed field compatibility, verifiable v2-to-v3 copy migration, backup/restore, and application cutover rollback. Do not silently reinterpret NQDB0002 as NQDB0003.
5. **Native Noqeri parity.** Full Noqeri-compiled relational plan/execute path, not only the narrow native scalar bridge, must pass CRUD/constraints/join/group/decimal/native binding differential fixtures against the Rust semantics and Zig storage.
6. **Executable, reproducible qualification.** Pin compiler/toolchain inputs and execute Rust/Zig/native Noqeri/Stage-2 suites on a trusted Linux x86-64 machine. A repository's tests existing is not proof that they pass.

### P1 — defense-in-depth and operational maturity

1. **AuthN/AuthZ at every externally exposed boundary.** Persist grants/tenant policies only after a security design; add credential/key lifecycle, authorization-context binding to sessions, audit events, revocation, cancellation, explicit memory/CPU/IO budgets, and adversarial tenant-escape tests. Deny-by-default embedded APIs are a partial primitive.
2. **Path hardening.** Replace path-based trusted-parent assumptions with Linux dirfd/openat2-style containment where needed; test symlink/hardlink/path races. Audit already-created storage permissions and protect backups and credentials.
3. **Capacity and performance.** Implement incremental relational records/pages, efficient reclaim, text/composite/range indexes, planner statistics, bounded result/aggregate materialization and resource pressure behavior; compare verified p50/p95/p99 metrics to SQLite/PostgreSQL on identical workloads.
4. **Backup/restore drills.** Test cross-filesystem copy, permission preservation, unknown-format rejection, no-clobber creation, cold restore and row-by-row equality with independent tools.
5. **Operational controls.** Stable ABI/API compatibility, observability, predictable errors for post-publish fsync failures (ambiguous commit), reproducible releases, platform support labels, SBOM and independent security review.

### P2 — adoption and ecosystem

Documentation with verified examples, client SDKs, migration tooling, REPL, language services, and optional live SQLite/PostgreSQL adapters, with semantic/round-trip tests.

## Local verification commands (no GitHub Actions)

Requires Linux x86-64, pinned Zig **0.15.2**, Rust + Cargo, GNU Make, SQLite dev headers for adapter tests, a trusted executable Noqeri compiler, a pinned local Noqeri checkout, and a trusted NQRI VM:

```bash
export NOQERI_COMPILER=/absolute/path/to/noqeri
export NOQERI_REPO=/absolute/path/to/Noqeri
export NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri
bash scripts/qualify-production-local.sh
```

The script exits with failure when a prerequisite or a test fails. It writes a log to `.qualification/local-engineering.log`. It is a **necessary engineering gate, never a production attestation**. Independent fault-injection and long-soak/benchmark qualification still need execution and retained evidence.

After compiling the native CLI, test a private existing database:

```bash
noesql v3-check /private/data/example.noedb
noesql v3-audit-permissions /private/data/example.noedb
```

If the audit reports existing files with group/other access, preserve the original and take a **verified logical backup to a new database path in a trusted private directory**. Do not mutate or remove database generations to silence warnings. Validate backup and restore before any application cutover.

## Release decision

**Block stable/production deployment until every applicable P0 item is proven with independent executable evidence.** The changes in this branch are safe-default improvements for code review, not a claim that the complete database has reached PostgreSQL/SQLite parity or that crash consistency and security have been certified.

Tracking issue: [#21 — v3 production readiness](https://github.com/EMN90909/NoeSQL/issues/21).

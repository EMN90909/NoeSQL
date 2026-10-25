> **Historical document.** This file describes an earlier C++/legacy migration stage and is retained only for engineering history. The C/C++ implementation it references has been removed from NoeSQL. The current source architecture is **Noqeri + Rust + Zig**. See `README.md`, `Doc/NOQERI_RUST_ZIG_INTEGRATION.md`, and `Doc/NOESQL_NATIVE_MIGRATION_PARITY.md` for current build and qualification guidance.

# NoeSQL v1.0 engineering implementation plan and review (2026-10-18)

Status: **staged remediation in progress; v1.0 and the independent Zig/Noqeri engine are NOT qualified**.
Basis: the 2026-10-18 NoeSQL engineering audit F01–F18, reviewed with ThoughtfulBits product-plan methodology and Vibe Code Security Reviewer threat/checklist methodology.

## Strategy and acceptance metric
A simple, verifiable goal: a readable Noqeri-owned database language backed by a durable, crash-tested Zig storage engine, with honest SQLite/Postgres comparison rather than unmeasured speed claims.

Measure weekly: (1) zero P0/P1 data loss, recovery or authorization regressions; (2) native conformance fixture pass rate, including negative and cross-backend differential cases; (3) fsynced transaction p50/p95/p99 and write amplification at 10k/100k/1m rows.

## Review scorecard (product strategy / roadmap / metrics)
- Strategy: **present but weak**. Ownership is clear, but no trusted bootstrap or demonstrated native end-to-end transaction.
- Product & roadmap: **present but weak**. Stage order exists; exit criteria now explicit below. Native vs reference scope must never be conflated.
- Metrics: **present but weak**. The old benchmark has methodology errors; realistic data-scale, disk-write and crash measurements remain missing.
- GTM, pricing and customer-feedback workflows: **out of scope** for this technical remediation plan.
- Decision: execute bounded patches; do not approve v1.0 or remove compatibility implementations.

## Migration safety policy
- Future language source: `.nqd`. Legacy `.noe` accepted by experimental CLI during transition, with no silent recompilation.
- Future native database data: `.nqdb`. Legacy JSON reference and C++ page-image `.nqd` remain explicitly supported by their own existing readers while migration is unfinished.
- The extensions overlap for legacy `.nqd` data and new `.nqd` source. Determine type using the **explicit command position/API**, and verify magic when opening data. Never auto-open/overwrite a source file as a database, never auto-rewrite legacy formats.
- Preserve backups; v1 needs an explicit read-only conversion verifier, rollback, golden format fixtures, and checksum/durability tests.

## Milestone 1: correctness and prevention
F01: empty-orphan native WAL recovery, real crash regression.
F02: reject reserved system tables in public embedded CRUD/index access, test restart.
F03: hash-pin seed as minimum hygiene; prevent false qualified status until actual end-to-end compiled Zig/Noqeri engine and provenance exist.
F04: fix Zig parser syntax and add pinned Zig CI.
F05: benchmark SQLite cursor and independent commits correctly.
F15/F17: broaden CI to native C++ and Zig and verify actual SQLite journal settings.
Exit: Node suite, native C++ fault tests, Zig 0.15.2 tests, benchmark smoke **all observed green**. An unavailable suite is an explicit blocker.

## Milestone 2: native Zig storage (F06/F07/F09/F10)
- Port legacy page and WAL format validators to Zig read-only, with binary golden samples.
- Implement a bounded Zig pager with incremental copy-on-write updates, page cache, free-page reclamation, transactional WAL frame sequences and stable commit protocol.
- Keep old C++ read path for migration until byte-for-byte compatibility and power-loss tests establish safety.
- Add POSIX fsync/rename/ENOSPC tests; then platform-specific Windows and macOS IO implementations or explicitly narrow supported v1 targets.
Exit: cold-restart recovery and fault injection across every syscall boundary, verified format migration and backup/restore without data loss.

## Milestone 3: compiled Noqeri relational engine (F06/F11)
- Use compiler source and stage proof from EMN90909/Noqeri, NOT the JavaScript reference as bootstrap proof.
- Compile parser, typed IR, planner and executor into a real ABI callback and link the Zig pager.
- Execute a common corpus for type errors, prepared params, joins, grouping, FK/unique constraints, migrations, null semantics and atomic transactions.
Exit: actual native process runs all fixtures, independent differential parity, known ABI hash and byte-identical/semantically-proven bootstrap.

## Milestone 4: security and concurrency (F08/F12/F13/F14/F18)
- Define transaction handles and isolation guarantees. Add snapshot readers and writer fairness while preserving WAL retention.
- Enforce authentication, role checks, table/row scoping, system metadata isolation, bounded memory/CPU, audit logging and cancellation at every entry point.
- Harden paths using directory descriptors, avoid process-global unsafe lock-file removal, and add malformed-page, API fuzz, sanitizer, and adversarial concurrency tests.
Exit: no exploitable P0/P1s in independent review, resource exhaustion tests with bounded outcomes, multi-process/host threat model documented.

## Milestone 5: release qualification (F05/F15/F16)
- Run reproducible hardware-isolated repeated benchmarks against SQLite WAL FULL and PostgreSQL on identical durable workloads.
- Enforce protected main, signed pinned source artifacts, SBOM, clean install, versioned packages, restoration drills and third-party storage review.
Exit: green CI, provenance, signed install tested distribution; only then update stable version.

## Known blockers as of this change
No trusted self-hosted Noqeri stage proof available in this repository. Zig bridge and read-only validators do not execute relational scripts or persist data. C++ remains experimental storage. Native WAL remains full-image and POSIX; SQLite compatibility is optional. GitHub Actions failure logs have been unavailable. There is no verified PostgreSQL comparison or production security certification.

# NoeSQL — implementation status and release gates

## Decision for the Noqeri 1.0 request

- **Compiler:** preserve the normal Noqeri-authored `Compiler/selfhost/*.nqr` path; do not reintroduce C++ into the normal build. Retain the archival C++ stage-0 seed and do NOT claim self-hosting until `Doc/SELF_HOSTING.md` Stage-2/Stage-3 proof succeeds on actual compiled artifacts. `project.nqr` currently says 1.0.0, but a manifest version is not a release-qualification certificate.
- **Naming:** NoeSQL is the intended native Noqeri database language/engine. The existing `Database/noqeridb.nqr` implementation has primitive integer-key map behavior and does not yet implement relational storage or transactions. Renaming it today without a migration would hide incompatibilities; compatibility and migration must be explicit.
- **Extensions:** `.nqr` Noqeri source, `.noe` NoeSQL scripts, `.nqo` compiled/web output, `.nqd` database data/embedded storage, `.nqdb` legacy database naming pending a versioned migration. Experimental reference `.nqd` data here is NOT compatible with a future native page/WAL format.
- **Adapters:** SQLite and PostgreSQL will be optional backends via parameterized, type-checked adapters, not runtime requirements for NoeSQL. No drivers are shipped in this increment; no compatibility is claimed.

## Tested experimental increment

`Runtime/noesql-reference.mjs` is a standalone, dependency-free JavaScript *reference oracle* for exercising early language semantics while the Noqeri compiler is being bootstrapped. It is NOT a substitute for the required Noqeri-authored parser/executor. `Database/noesql_lexer.nqr` begins the Noqeri-native frontend with real source-spanned lexical tokenization, including `$`, comments, delimiters and operators; native binary compilation remains unverified without stage-0.

Supported by the reference oracle: token spans, recursive-descent declarations and precedence parsing, schema catalog, identifier/type validation, single `int key` and `auto`, `required`/`unique` constraints, integer foreign keys and restricted deletes, index declaration validation, typed named-query parameters/defaults, `get`/`where`/`sort`/`skip`/`take`, `add`/`set`/`delete` with one executor, checked integer arithmetic, boolean short circuiting, atomic rollback on errors, and source diagnostics. Indexes are metadata; they do not yet accelerate scans.

`Runtime/noesql-store.mjs` and `noesql-cli.mjs` provide **experimental** `.nqd` JSON snapshots with SHA-256 checksums, no-overwrite lock files, temporary-file writes and atomic rename. This is *not* the future NoeSQL physical page/WAL storage engine: there is no measured power-loss durability, cross-platform isolation proof, recovery journal or compatible binary format. Use temporary/test data only, not user production databases.

## Reproduce local tests (no GitHub Actions)

From the repository root with Node.js 20+:

```sh
node --test tests/noesql-reference.test.mjs tests/noesql-store.test.mjs
node Runtime/noesql-cli.mjs run /tmp/school.nqd tests/school.noe
node Runtime/noesql-cli.mjs query /tmp/school.nqd adults '{"minAge":20}'
```

The native Noqeri lexer test is `tests/noesql_lexer.nqr` and must be run using an authenticated working Noqeri compiler (`noqeri check`, `noqeri run` where available). Never report it as passing before execution.

## Hard blockers before stable 1.0

1. Native compiler and parser: complete `.nqr` lexer/AST/recursive-descent expression and statement parser, schema-aware semantic analysis, stable diagnostics and grammar conformance. Prove bootstrap Stage-2/Stage-3 and test actual native/web emission.
2. Native typed NoeIR, row encoding/decoding, indexes, relational planner and shared scan/filter/project/CRUD executor, foreign-key enforcement, schema versions/migrations, grouping/joins and parameterized `.nqr` bindings.
3. Versioned native `.nqd` page format and WAL with a measured durable commit point, exclusive-writer lock, read-only verify, replay, recovery, corruption and power-failure injection on all supported platforms.
4. Optional SQLite and PostgreSQL interoperability adapters with prepared binds, type mapping, error semantics, transactions and integration fixtures; no silent fallback to either engine.
5. Windows/Linux/macOS build/rebuild, language and database positive/negative conformance, reproducible binaries, fuzz tests, overflow/borrow/safety tests, performance evidence, documentation and signed release artifacts.

Until the listed gates pass, **do not tag v1.0 or delete the stage-0 compiler seed**. This increment is a testable reference implementation and a native lexer start, not a production database or complete compiler migration.

## 15 October 2026: native IR and reference WAL increment

The follow-up adds native **source** modules `Database/noesql_ir.nqr`,
`Database/noesql_executor.nqr`, `Database/noesql_row.nqr` and
`Database/noesql_wal.nqr`. The IR uses catalog column IDs, typed scalar values,
source-independent plans, version checks and explicit predicate operators. The
native executor is a bounded, supplied-buffer kernel: primitive int/bool/text
values, in-memory insert/scans/projections/updates/deletes, staging of mutations,
required and unique-field validation. The native row codec defines `NROW` ROW1
little-endian records. It does **not** yet execute parsed `.noe` scripts, resolve
relationships, implement composite indexes, sort/group/join, or persist records
through the native runtime. The native WAL module encodes/validates NoeSQL
NWAL v1 frames and committed full-image recovery using a required external
SHA-256 provider `noeHostSha256`. It does **not** itself implement host fsync,
locks or checkpoint IO.

The **JavaScript reference** now uses a binary NWAL journal before snapshots:
BEGIN/IMAGE/COMMIT frames with SHA-256 frame and commit checks, WAL fsync,
POSIX directory sync, snapshot tempfile fsync/rename and WAL removal. Recovery
prefers a fully committed WAL image; it rejects complete corrupt frames, ignores
uncommitted tails and preserves committed data if checkpoint fails. The
experimental `.nqd` snapshot payload is still JSON, and the host implementation
is **not** Noqeri's native database engine. Windows durable directory sync,
real process termination/disk-error injection, isolation and storage-version
migration need separate evidence.

Locally runnable with Node.js 20+ (no Actions):

```sh
node --test tests/noesql-reference.test.mjs tests/noesql-store.test.mjs tests/noesql-wal.test.mjs tests/noesql-row.test.mjs
```

`tests/noesql_native_ir.nqr` is an unverified native fixture pending a trusted
Noqeri compiler. The Stage-2/Stage-3 gate in `Doc/SELF_HOSTING.md` remains
unproven. This is not sufficient for a stable 1.0 release or merging to main.

The WAL suite also includes a real POSIX subprocess SIGKILL at the
post-WAL-directory-fsync point. The tested reference store recovers the committed
image after a process kill; the stale writer lock is left in place and writers
fail closed. **This does not prove power-loss durability on all operating systems.**

## 2026-10-15 interoperability increment (feature branch only)

Added `Runtime/noesql-adapters.mjs`, supporting **experimental** SQLite execution via prepared statements and PostgreSQL SQL translation plus optional `pg`-based execution. Validation covers typed parameters, SQL identifier safety, schema and projection diagnostics, basic foreign keys, atomic SQL rollback and native SQLite boolean decoding. See `Doc/NOESQL_SQL_ADAPTERS.md`.

Local Node.js 22 test run: **35 passed, 0 failed**, covering reference CRUD/WAL/row serialization and optional SQLite compatibility. This does not validate native `.nqr` modules or the complete PostgreSQL runtime (requires a live server).

**Still release-blocked:** the trusted stage-0 seed is not installed in this execution environment. Native `.nqr` compilation, compiled NoeSQL AST/semantic binding to NoeIR, true native storage/WAL provider and recovery, migrations, full SQL feature coverage, full Noqeri self-hosting and broad cross-platform tests have not passed. Do not merge/tag v1.0 solely on the basis of Node reference tests.

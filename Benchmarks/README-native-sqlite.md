# Native Noqeri + Rust + Zig vs SQLite WAL benchmark

**Status: test harness implemented; no results are certified until it executes on an actual runner.** The historical `benchmark_compare_sqlite` placeholder is removed on this branch.

## What is compared

The benchmark measures a narrow, verified workload:

- NoeSQL: Noqeri AOT `noesqlStage2BoolRow` scalar semantics called through Rust, with each update committed to the Zig WAL pager.
- SQLite: the same single primary-key boolean row, SQLite WAL mode, `PRAGMA synchronous=FULL`.
- Exactly **96 update/checkpoint/reopen/read-back cycles** per pass, **3 passes**.
- Both stores begin with one row (`id=44`), alternate boolean state, checkpoint every seventh iteration, and cold-reopen for read-back.
- The runtime verifies every read value. SQLite additionally runs `PRAGMA integrity_check`. No timing is reported for a pass if any correctness check fails.
- Runs alternate engine order to reduce cache/order bias. Output reports per-pass milliseconds and the median NoeSQL/SQLite latency ratio.

This is **not** a full SQL parser or relational query benchmark. The NoeSQL reference runtime, Noqeri NQRI interpreter, and SQLite B-tree planner have significantly different query features and transaction APIs. This fixture qualifies only the **native scalar Noqeri→Rust→Zig WAL path**. Its success must not be extrapolated into “NoeSQL is faster than SQLite” for general database workloads.

## Prerequisites

- Linux x86-64.
- Zig **0.15.2**, Rust/Cargo, a C compiler/archiver, and SQLite development libraries, e.g. `libsqlite3-dev` on Ubuntu.
- A locally compiled Noqeri Stage-0 CLI from the separate Noqeri repository. Do not pretend the NQRI VM is a native compiler.

## Commands

```bash
# Existing integrity tests, plus Rust ↔ Zig integration.
make test

# Re-run Zig/Rust suites three separate times. The Zig ABI test already
# includes 96 WAL/checkpoint/cold-reopen cycles and corruption cases.
make stress

# Compiles the native Noqeri scalar bridge and then executes both engines.
make benchmark-native-sqlite NOQERI_COMPILER=/absolute/path/to/noqeri
```

The GitHub Actions native workflow also runs the benchmark after compiling the Noqeri archive and uploads the `native-sqlite-benchmark` output artifact.

## Interpretation and cautions

- Measurements are meaningful only when the Noqeri, Rust and Zig binaries **actually compile and run** on the same machine as SQLite.
- Database page representation is different: NoeSQL uses 4096-byte physical pages for each native row; SQLite packs one integer/boolean row in its B-tree. The test compares user-visible operations, not identical bytes written.
- Application startup, source parsing, concurrent writes, joins and index scans are not measured.
- Sync/fsync behavior and performance vary with filesystem, virtualization, SSD cache and power-loss semantics. Repeat on multiple machines.
- A simulated fault is not a real power-loss test. The production release still needs independent crash/fuzz/concurrency tests, compatibility migration, and real filesystem durability qualification.
- **Do not compare any SQLite-only sandbox baseline with an imagined NoeSQL result.**

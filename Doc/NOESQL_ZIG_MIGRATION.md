# NoeSQL: Noqeri + Zig migration plan and qualification

**Status (18 October 2026): IN PROGRESS. Not a native Zig engine release.**

## Current ownership and migration boundary

- **Noqeri**: `Database/*.nqr` owns parsing, typed IR, authorization policy, transaction semantics and relational execution. The trusted Noqeri stage-0/stage-2 compiler and native ABI export are still not verified.
- **Zig 0.15.2**: `Native/zig/` now supplies a bounded, versioned C ABI dispatch bridge. It validates the function table, input/output pointers, limits, callback status and output lengths. **It does not perform SQL execution or durable storage.**
- **Legacy C++17**: `Native/cpp/noesql_page_store.*` still owns the experimental page storage, authentication, locking and WAL. It is retained to avoid making existing `.nqd` files inaccessible or weakening crash recovery.
- **JavaScript**: `Runtime/*.mjs` remains a reference oracle and optional SQLite-backed embedded/web adapter. It is not the compiled Noqeri + Zig engine.

## Build the new Zig bridge

Use Zig **0.15.2** with C ABI compatibility:

```sh
cd Native/zig
zig build test
zig build -Doptimize=ReleaseSafe
```

The static library `libnoesql_zig_bridge.a` and `noesql_zig_bridge.h` are produced by the build. Callback ABI v1 uses the same field order and bounds as the earlier C++ dispatcher but exports `noesql_zig_validate_engine` and `noesql_zig_execute`. Callers must not link the old and new bridges under identical application assumptions without an ABI harness.

The Zig bridge does **not** yet link a Noqeri-compiled callback. These tests exercise native boundary behavior only, not parsed `.noe` queries, WAL or storage durability. Tests were not executed in the current editing environment, where Zig and the trusted Noqeri compiler were absent.

## Required gates before removing C++ or switching the default CLI

1. Obtain a reproducible, trusted Noqeri compiler bootstrap; compile the NoeSQL `.nqr` parser, typed IR and executor, and export the ABI v1 callback. Do not substitute the JavaScript reference for this proof.
2. Run shared `.noe` fixtures against compiled Noqeri/Zig and reference implementations; compare success/error values, parameter checking, grouping, joins, NULL behavior, schema constraints and query limits.
3. Port `Native/cpp/noesql_page_store.cpp` to Zig with an explicitly versioned format. Preserve lock semantics, authentication, revocation, quotas, checksums, WAL commit ordering, fsync and recovery invariants. Do not claim `.nqd` compatibility without byte-for-byte format fixtures and read/write/recovery tests. Provide a backed-up migration tool for incompatible formats.
4. Fault-inject at every durable IO boundary; test crash recovery and data integrity over concurrent clients, malformed pages, disk-full errors, interrupted checkpoints, power-loss simulations, and cross-platform filesystem semantics.
5. Test Linux/macOS/Windows builds, thread safety, sanitizers/fuzzing and reproducible binaries. Collect p50/p95/p99 latency, throughput and memory on public, versioned workloads. Zig alone does not prove faster query execution.
6. Build a **separate** package distribution gate with explicit release authorization. NoeSQL is currently a private repository, and it is not yet published as a public package in `noqeri-registry`. Do not expose private sources or advertise an unverified public download.

## Commands and release truth

```sh
npm test
npm run test:native              # legacy C++ prototype compatibility
npm run test:zig                 # Zig bridge tests; Zig 0.15.2 required
npm run qualify:zig              # deliberately FAILS unless Zig + Noqeri native gates are present
```

The current `0.7.5-rc.2` is **experimental**, and these migration steps do not turn it into a stable database. Preserve backups and keep existing C++ native file format and JS/SQLite reference behavior explicit.

See [Noqeri language source](https://github.com/EMN90909/Noqeri), [existing native page engine](NOESQL_NATIVE_PAGE_ENGINE.md), and [0.7.5 release gates](NOESQL_075_RELEASE_GATES.md).

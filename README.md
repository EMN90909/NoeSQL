# NoeSQL

NoeSQL is Noethric's native database language and storage engine project for Noqeri.

> Development status: the NoeSQL source stack is now **Noqeri + Rust + Zig**. C and C++ source have been removed from this repository. Noqeri owns database semantics and native execution modules, Rust owns the safe host/runtime and ABI orchestration, and Zig owns durable page/WAL/index storage.

## Native stack

- **Noqeri**: typed IR, policy/auth/limits, row semantics, relational executor modules, Stage-2 qualification code, and the native scalar decision bridge in `Database/*.noqeri`.
- **Rust**: lexer/parser, general relational runtime, CLI, persistence codec, session host, native build tooling, direct native-Noqeri calls, and safe Zig FFI wrappers in `Native/rust/`.
- **Zig 0.15.2**: `NQDB0002` pages, `NQTX0002` WAL, indexes, snapshot ABI, recovery, locking, and low-level storage tests in `Native/zig/`.

There is **no JavaScript, C, or C++ source in NoeSQL**. `make no-js` and `make no-c-family` fail closed if those languages return to the repository.

The current Noqeri Stage-0 compiler is still built from the separate Noqeri repository during CI until Noqeri completes native self-hosting. That bootstrap implementation is a compiler toolchain dependency outside NoeSQL, not part of NoeSQL's source stack.

## Runtime ownership

The Rust runtime implements:

- lexer/parser validation and source limits
- typed values and schemas
- required, unique, key, auto, default, and foreign-key constraints
- referenced-row delete protection and primary-key update protection
- create/insert/read/update/delete behavior
- whole-program and explicit atomic rollback
- bound parameters
- catalog indexes and named queries
- schema-level query/type validation even on empty tables
- bounded relational planning with hash-join and nested-loop paths
- inner and left joins
- grouping and HAVING
- `count`, `sum`, `min`, and `max`
- sorting, skip, and take
- alter-add/default backfill
- JSON-safe row output
- bounded binary snapshot encoding/decoding
- CLI execution and query commands

Durability is not implemented by serializing files directly from Rust. Rust sends bounded snapshots and raw page operations through Zig's exported ABI. Zig owns WAL ordering, checksummed pages, recovery, checkpointing, and file locking.

## Rust -> Noqeri -> Rust -> Zig native chain

Noqeri's current Stage-0 compiler can emit Linux x86-64 machine code. NoeSQL uses that backend only for a deliberately narrow scalar boundary that is already qualified by the existing Noqeri backend.

The native chain is:

```text
Rust host/runtime
      |
      | scalar native call
      v
Noqeri AOT semantic bridge
      |
      | CRUD decision/result code
      v
Rust pager orchestration
      |
      | Zig exported ABI
      v
Zig NQDB0002 + NQTX0002 storage
```

This removes the old C++ compatibility shim. Noqeri decides database semantics, Rust owns pointer/handle validation and orchestration, and Zig owns durable bytes.

Build and execute the direct native chain:

```sh
make native-chain NOQERI_COMPILER=/absolute/path/to/noqeri
```

The build compiles `Database/noesql_native_bridge.noqeri` into `libnoesql_noqeri.a`, links it with the Rust runtime and Zig pager, then executes native CRUD through the complete chain.

### Verified Stage-2 NQRI chain

NoeSQL also consumes Noqeri's self-hosted Stage-2 NQRI path without pretending it is already a native self-hosted compiler.

```text
trusted Stage-1 NQRI
        |
        v
Stage-2 compiler NQRI
        |
        v
Stage-3 compiler NQRI
        |
        | byte-for-byte equality gate
        v
Stage-2 compiles NoeSQL executor bridge
        |
        v
Rust host executes NQRI semantics
        |
        v
Zig pager / WAL commit
```

Run it locally with:

```sh
make stage2-chain \
  NOQERI_REPO=/absolute/path/to/Noqeri \
  NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri
```

## Canonical formats

- Noqeri source: `.noqeri`
- NoeSQL script source: `.noe`
- canonical database storage: `.noedb`
- Zig page format: `NQDB0002`
- Zig WAL framing: `NQTX0002`
- Rust snapshot payload inside Zig pages: `NQRP0001`
- Zig snapshot manifest: `NQRS0001`

Legacy `.nqd` and `.nqdb` are migration inputs only and must never be silently reinterpreted as `.noedb`.

## Build and test

Requirements for NoeSQL itself:

- Zig 0.15.2
- stable Rust + Cargo
- GNU Make

For the direct native Noqeri chain, also provide a built Noqeri compiler plus the system assembler/archive tools used to turn emitted assembly into a static archive. CI currently builds that compiler from the separate Noqeri repository with its bootstrap dependencies.

Core tests:

```sh
make test
```

Equivalent suites:

```sh
cargo test --manifest-path Native/rust/Cargo.toml --all-targets
zig build --build-file Native/zig/build.zig test
make no-js
make no-c-family
```

Build the Rust + Zig runtime:

```sh
zig build native
```

Build the direct AOT Noqeri integration:

```sh
make native-chain NOQERI_COMPILER=/absolute/path/to/noqeri
```

## CLI

Build with Zig persistence enabled:

```sh
NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" \
cargo build --manifest-path Native/rust/Cargo.toml --features zig-pager
```

Then:

```sh
noesql check tests/school.noe
noesql check-db scratch.noedb
noesql run scratch.noedb tests/school.noe
noesql query scratch.noedb 'users get id,name sort id'
```

## Language example

```noe
db school
table users {
  id: int key auto
  name: text required unique
  age: int
}

users add {name:"Amina", age:21}
users get name, age where age >= 18 sort name take 10
```

## Repository layout

```text
Database/        Noqeri semantic/executor/native bridge modules
Native/rust/     Rust runtime, parser, codec, CLI, FFI/build tooling
Native/zig/      Zig pager, WAL, index, snapshot ABI and tests
Tools/           Zig repository contract tests
Grammar/         NoeSQL grammar
tests/           Noqeri native fixtures
Benchmarks/      native benchmark methodology and Noqeri fixtures
Doc/             architecture, security and qualification notes
build.zig        top-level native build/test orchestration
Makefile         native developer entrypoints and stack guards
```

## Concurrency, stress tests, and measured performance

The native Zig pager currently **serializes access through one exclusive file
lock**, including reads. The Rust CLI holds this lock across the complete
load / execution / durable commit cycle, preventing lost updates between
independent CLI writer processes. Busy file locks are retried for a bounded
period; a busy timeout is a failure, not evidence that a transaction committed.

The Rust runtime avoids cloning an entire database for read-only programs
and the CLI skips redundant snapshot encoding when no statement can write.
The NoeSQL parser moves tokens rather than cloning each token string.
These changes reduce allocation overhead but **do not** replace the existing
full-snapshot copy-on-write model for mutating queries, nor do they implement
reader/writer lock separation or MVCC.

The WAL now checkpoints automatically before a commit that would exceed
a **16 MiB replay threshold**, retaining the 64 MiB hard maximum. This
prevents sustained ordinary writes from exhausting the journal solely from
accumulated committed transactions. The pager also avoids syncing the
**parent directory on routine reopen** when no file entry was created.
Existing WAL and checkpoint durability fsyncs are unchanged.
Crash recovery still validates the versioned checkpoint and WAL;
auto-checkpoint failure aborts the pending transaction before its WAL append.

Reproduce the durability and concurrency regression suite:

```sh
make stress
# Single Rust parser / read-path microbenchmark, not a SQL-server benchmark:
make benchmark-parser-reads
```

The integration stress test exercises 8 independent writer processes,
96 committed rows, WAL recovery and cold reopening. The Zig tests cover
automatic checkpoint rollover, 190 successive multi-page transactions,
and checkpoint fault injection before a newly staged WAL commit.
The microbenchmark reports parser throughput and read execution time on the
**machine that executes it**; no unexecuted speedups or comparisons to SQLite
or PostgreSQL should be inferred from code inspection alone.

Production-scale concurrency remains out of scope until the storage format
supports independent read snapshots and write-ahead transactions without
global exclusive read locking. Run longer multi-process soak, kill/restart,
filesystem fault-injection, memory-limit, and comparative latency suites
before any production readiness claim.

## Store format and safety boundaries

The current `NQDB0002` header has a 32-byte active-page bitmap: only **256 physical pages (0..255)** are representable. This branch enforces the actual format limit rather than permitting writes that overflow the header. The bounded snapshot interface supports at most 255 data pages plus the page-zero manifest.

The native Noqeri scalar-row fixture and the Stage-2 NQRI fixture use the same `NQNR0001` row encoding and `NQRO0001` page-zero store marker. The Rust relational snapshot path uses its distinct `NQRS0001` page-zero manifest. Both sides must **reject rather than overwrite** an incompatible store type. Earlier experimental Stage-2 `NQS20001` pages require an explicit migration and are not silently reinterpreted.

This repository contains source-level fixes and executable tests for cold reopening, WAL corruption rejection, index mutations, format bounds, and native/Stage-2 cross-mode behavior, but passing source review is **not** a replacement for a successful test run. The full Noqeri + Rust + Zig stack must execute on a trusted Linux x86-64 runner before merge or production deployment.

## Native Noqeri web framework session-v2 integration (unverified)

NoeSQL now includes a Linux Rust shared-library provider for the exact
Noqeri native web session-v2 C ABI. It exports noesql_session_v2_get and
backs prepared queries, strict JSON/typed bindings, transaction begin/commit/
rollback, and generation-checked autocommit with the real NQDB0003 engine.

This is an **opt-in trusted embedded provider**, not a public database
server or proof of full Noqeri Stage-2 source-language integration.
Noqeri's language-side noesqlHost* extern implementations, durable tenant
security, and full Noqeri-compiled relational NIR are still incomplete.

Build locally with Rust/Cargo (Linux):

    cargo build --locked --release --manifest-path Native/rust/Cargo.toml --lib --bin noesql

Then set NOESQL_NATIVE_PROVIDER to the absolute path of
Native/rust/target/release/libnoesql_runtime.so in the Noqeri web process.
The database must already exist in canonical NQDB0003 format, under a
trusted private directory; no storage migration happens implicitly.

Run the real framework integration locally, WITHOUT GitHub Actions:

    make test-noqeri-framework-local NOQERI_WEB=/absolute/path/to/noqeri-web

See Doc/NOQERI_NATIVE_WEB_SESSION_V2_INTEGRATION.md for the exact upstream
capability scan, v2 binding contract and outstanding production release
gates. Neither compilation nor the end-to-end test has been run in the
authoring environment.

## v3 security and local qualification (unverified hardening)

Newly created NQDB0003 HEAD/generation/index files use private Unix permissions
(0600 files, 0700 generation directories); on Linux, final-component file
symlinks are rejected for critical v3 reads and the lock file. **Existing v3
stores are not silently modified.** Audit current and retained generations with:

```sh
noesql v3-audit-permissions /private/data/example.noedb
```

The audit also flags an insecure immediate parent directory. Use a privately
owned, trusted parent directory: these precautions do not
provide dirfd containment, public-service authentication, encryption at rest,
or verified cross-platform isolation. The library's
`v3_store::execute_authorized` applies explicit embedded table grants,
row filters and returned-row budgets before durable publication; the ordinary
CLI and low-level APIs remain trusted **administrative** interfaces.

For local engineering tests without GitHub Actions, see
`scripts/qualify-production-local.sh`. This requires pinned Rust, Zig,
Noqeri AOT and Stage-2 toolchains and **fails closed** if any suite fails.
A successful local suite is not a production certificate. See
[`Doc/NOESQL_PRODUCTION_READINESS_2026-11-06.md`](Doc/NOESQL_PRODUCTION_READINESS_2026-11-06.md)
for audit scope, unresolved P0/P1 blockers and release acceptance criteria.

## Release qualification

The source migration is complete, but production qualification is not. Remaining gates include:

1. record a green executing Noqeri/Rust/Zig CI run;
2. expand the bounded native Noqeri semantic bridge toward the full general-schema relational path;
3. complete concurrency, crash-boundary, migration, fuzz, and differential qualification;
4. qualify additional platforms before cross-platform claims;
5. replace Stage-0 AOT provenance with native self-hosted Stage-2 provenance only after Noqeri itself reaches that milestone.

See `Doc/NOESQL_NATIVE_MIGRATION_PARITY.md` for the current matrix.

## License

GPL-3.0-only.

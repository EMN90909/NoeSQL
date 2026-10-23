# NoeSQL

NoeSQL is Noethric's native database language and storage engine project for Noqeri.

> Development status: native migration in progress. The NoeSQL repository is JavaScript-free. Rust now owns the application/reference runtime, Zig owns durable page/WAL/index storage, Noqeri owns typed database semantics and native execution modules, and C++ remains a compatibility kernel while migration qualification is completed.

## Native stack

- **Noqeri**: typed IR, policy/auth/limits, row encoding, relational executor, and native Zig-facing database modules in `Database/*.noqeri`.
- **Rust**: lexer/parser, general relational runtime, CLI, persistence codec, session host, native-build tooling, and safe FFI wrappers in `Native/rust/`.
- **Zig 0.15.2**: `NQDB0002` pages, `NQTX0002` WAL, index implementation, atomic snapshot ABI, and low-level storage tests in `Native/zig/`.
- **C++17**: compatibility page-store/ABI and durability tests in `Native/cpp/`.

There is **no JavaScript runtime, Node test suite, npm package, or JavaScript source file in NoeSQL**. `make no-js` fails if `.js`, `.mjs`, `.cjs`, or `.jsx` returns.

## What replaced the old JavaScript layer

The Rust runtime now implements:

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

Durability is not implemented by serializing files directly from Rust. Rust sends its bounded snapshot through the Zig pager ABI, which owns WAL ordering, checksummed pages, recovery, checkpointing, and file locking.

## Rust -> Noqeri -> Zig native chain

Noqeri's current Stage-0 C++ compiler has a real Linux x86-64 AOT backend. PR #12 uses that backend rather than pretending the self-hosted NQRI Stage-2 compiler is already native.

The native chain is:

```text
Rust host
   |
   | C ABI
   v
Noqeri AOT bridge
   |
   | typed NoeQueryContext / NoeIrPlan
   v
Noqeri executor
   |
   | pager ABI
   v
Zig NQDB0002 + NQTX0002 storage
```

The Noqeri executor and relational bridge use request-record pointers so they remain compatible with the current x86-64 backend's six-register-argument limit.

To build and execute this chain locally:

```sh
make native-chain NOQERI_COMPILER=/absolute/path/to/noqeri
```

That builds Zig storage, uses the Rust `build_native` helper to compile `Database/noesql_native_bridge.noqeri` into `libnoesql_noqeri.a`, links it into Rust, and executes native CRUD through Rust -> Noqeri -> Zig.

This proves a native machine-code integration route without claiming that Noqeri's self-hosted Stage-2 compiler is native.

### Verified Stage-2 NQRI chain

PR #12 also consumes Noqeri's actual self-hosted Stage-2 compiler path. It does not rename Stage-0 as Stage-2.

The qualification chain is:

```text
trusted Stage-1 NQRI
        |
        v
Stage-2 compiler NQRI
        |
        | rebuild same source closure
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

Run it locally after building Noqeri's native NQRI VM:

```sh
make stage2-chain \
  NOQERI_REPO=/absolute/path/to/Noqeri \
  NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri
```

This gives NoeSQL a real Rust -> Stage-2 NQRI -> Zig execution path. It still does **not** claim native machine-code self-hosting for the Noqeri compiler, because Noqeri's own bootstrap contract explicitly says Stage-2 currently executes as NQRI.

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

Requirements:

- Zig 0.15.2
- stable Rust + Cargo
- C++17 compiler
- CMake
- GNU Make
- OpenSSL development headers
- SQLite development headers for the compatibility benchmark

Core native tests:

```sh
make test
```

Equivalent individual suites:

```sh
cargo test --manifest-path Native/rust/Cargo.toml --all-targets
zig build --build-file Native/zig/build.zig test
make -C Native/cpp test
make no-js
```

Build the Rust + Zig runtime:

```sh
zig build native
```

Build the direct AOT Rust -> Noqeri -> Zig integration:

```sh
make native-chain NOQERI_COMPILER=/absolute/path/to/noqeri
```

Build the verified self-hosted Stage-2 NQRI -> Rust -> Zig integration:

```sh
make stage2-chain \
  NOQERI_REPO=/absolute/path/to/Noqeri \
  NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri
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
Native/rust/     Rust runtime, parser, codec, CLI, tests, FFI/build tooling
Native/zig/      Zig pager, WAL, index, snapshot ABI and tests
Native/cpp/      C++ compatibility kernel and ABI/durability tests
Tools/           Zig repository contract tests
Grammar/         NoeSQL grammar
tests/           Noqeri native fixtures
Benchmarks/      native benchmark sources and methodology
Doc/             architecture, security and qualification notes
build.zig        top-level native build/test orchestration
Makefile         native developer entrypoints
```

## Release qualification

The JavaScript migration is no longer the blocker. Remaining gates are native qualification work:

1. obtain an executing CI runner and record a green Rust/Noqeri/Zig/C++ run;
2. expand the current bounded native Noqeri row/page bridge to full general-schema relational execution;
3. complete concurrency, crash-boundary, migration, fuzz, and differential qualification;
4. qualify additional platforms before cross-platform claims;
5. keep the verified Stage-2 NQRI path distinct from the Stage-0 AOT machine-code path, and claim native self-hosted Stage-2 provenance only after Noqeri itself supports native self-hosting.

See `Doc/NOESQL_NATIVE_MIGRATION_PARITY.md` for the exact matrix.

## License

GPL-3.0-only.

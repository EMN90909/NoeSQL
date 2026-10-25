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

## Store format and safety boundaries

The current `NQDB0002` header has a 32-byte active-page bitmap: only **256 physical pages (0..255)** are representable. This branch enforces the actual format limit rather than permitting writes that overflow the header. The bounded snapshot interface supports at most 255 data pages plus the page-zero manifest.

The native Noqeri scalar-row fixture and the Stage-2 NQRI fixture use the same `NQNR0001` row encoding and `NQRO0001` page-zero store marker. The Rust relational snapshot path uses its distinct `NQRS0001` page-zero manifest. Both sides must **reject rather than overwrite** an incompatible store type. Earlier experimental Stage-2 `NQS20001` pages require an explicit migration and are not silently reinterpreted.

This repository contains source-level fixes and executable tests for cold reopening, WAL corruption rejection, index mutations, format bounds, and native/Stage-2 cross-mode behavior, but passing source review is **not** a replacement for a successful test run. The full Noqeri + Rust + Zig stack must execute on a trusted Linux x86-64 runner before merge or production deployment.

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

# NoeSQL

NoeSQL is Noethric's native database language and storage engine project for Noqeri.

> Development status: native migration in progress. The NoeSQL implementation stack is now Noqeri + Rust + Zig. C++ implementation sources and the old Native/cpp compatibility layer are removed.

## Native stack

- **Noqeri** owns typed database semantics and native semantic kernels in Database/*.noqeri.
- **Rust** owns the host/runtime, parser/reference execution, safe native boundary, snapshot codec, CLI, and build tooling in Native/rust/.
- **Zig 0.15.2** owns NQDB0002 pages, NQTX0002 WAL, indexes, snapshot storage, recovery, and low-level durability tests in Native/zig/.

NoeSQL contains no JavaScript runtime and no C/C++ implementation source. The Zig ABI declaration headers remain because Rust and native Noqeri integration link against the Zig storage ABI.

## Why Rust sits between Noqeri and Zig

Noqeri's current native backend can emit scalar export symbols, but its own FFI documentation does not yet promise a portable arbitrary C ABI for pointer-heavy records and callbacks. NoeSQL therefore keeps the native Noqeri boundary deliberately scalar on Linux x86-64 and lets Rust own pointer/string/handle safety.

The direct native path is:

    Rust host/runtime
        |
        | scalar native call
        v
    Noqeri AOT semantic kernel
        |
        | result/status
        v
    Rust storage boundary
        |
        | Zig C ABI
        v
    Zig pager + WAL + indexes

This removes the old C++ shim without pretending Noqeri's experimental native ABI is more portable than it currently is.

## Native chains

Build and execute the direct AOT chain:

    make native-chain NOQERI_COMPILER=/absolute/path/to/noqeri

The command builds Zig storage, compiles Database/noesql_native_bridge.noqeri into libnoesql_noqeri.a, links Rust directly to the Noqeri scalar kernel and Zig pager, then executes CRUD smoke coverage.

Build the verified self-hosted Stage-2 NQRI chain:

    make stage2-chain \
      NOQERI_REPO=/absolute/path/to/Noqeri \
      NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri

The Stage-2 path remains NQRI-hosted. It is kept distinct from the Stage-0 AOT machine-code path until Noqeri itself reaches native self-hosting.

## Canonical formats

- Noqeri source: .noqeri
- NoeSQL script source: .noe
- canonical database storage: .noedb
- Zig page format: NQDB0002
- Zig WAL framing: NQTX0002
- Rust snapshot payload inside Zig pages: NQRP0001
- Zig snapshot manifest: NQRS0001

Legacy .nqd and .nqdb are migration inputs only and must never be silently reinterpreted as .noedb.

## Build and test

Core NoeSQL requirements:

- Zig 0.15.2
- stable Rust + Cargo
- GNU Make
- a C compiler and archiver for assembling the current Noqeri AOT output archive

Building the current Noqeri Stage-0 compiler from its own repository still requires that compiler repository's CMake/C++ bootstrap toolchain. That is compiler provenance outside the NoeSQL implementation stack, not a C++ runtime layer in NoeSQL.

Run the native suites:

    make test

Equivalent suites:

    cargo test --manifest-path Native/rust/Cargo.toml --all-targets
    zig build --build-file Native/zig/build.zig test
    make no-js no-native-c-family

Build the Rust + Zig runtime:

    zig build native

## CLI

Build with Zig persistence enabled:

    NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" \
    cargo build --manifest-path Native/rust/Cargo.toml --features zig-pager

Then:

    noesql check tests/school.noe
    noesql check-db scratch.noedb
    noesql run scratch.noedb tests/school.noe
    noesql query scratch.noedb 'users get id,name sort id'

## Language example

    db school
    table users {
      id: int key auto
      name: text required unique
      age: int
    }

    users add {name:"Amina", age:21}
    users get name, age where age >= 18 sort name take 10

## Repository layout

    Database/        Noqeri semantic/executor/native bridge modules
    Native/rust/     Rust runtime, parser, codec, CLI, tests, FFI/build tooling
    Native/zig/      Zig pager, WAL, index, snapshot ABI and tests
    Tools/           Zig repository contract tests
    Grammar/         NoeSQL grammar
    tests/           Noqeri native fixtures
    Benchmarks/      native benchmark sources and methodology
    Doc/             architecture, security and qualification notes

## Release qualification

Remaining gates are qualification work, not C++ migration work:

1. record a green executing Noqeri/Rust/Zig CI run;
2. expand the bounded native Noqeri row/page semantic kernel into the full general-schema relational path;
3. complete concurrency, crash-boundary, migration, fuzz and differential qualification;
4. qualify additional platforms before cross-platform native-export claims;
5. replace Stage-0 AOT provenance with native self-hosted Stage-2 provenance only after Noqeri itself supports it.

See Doc/NOESQL_NATIVE_MIGRATION_PARITY.md and Doc/NOESQL_NOQERI_NATIVE_INTEGRATION.md.

## License

GPL-3.0-only.

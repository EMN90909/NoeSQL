# NoeSQL

NoeSQL is Noethric's native database language and storage engine project for Noqeri.

> Development status: experimental. The native stack is now JavaScript-free, but the complete Noqeri Stage-2 relational executor is still being qualified. Do not use this branch for irreplaceable production data yet.

## Native language stack

NoeSQL now has four explicit responsibilities:

- **Noqeri** — language grammar, lexer, parser, IR validation, authorization policy, limits, relational execution semantics and WAL-facing contracts in `Database/*.noqeri`.
- **Rust** — host/runtime/session layer and native CLI in `Native/rust/`. It owns safe application-facing lifetime management around the stable C-compatible session ABI.
- **Zig 0.15.2** — low-level page/WAL/index implementation plus repository build/test orchestration in `Native/zig/`, `Tools/`, and root `build.zig`.
- **C++17** — compatibility page-store, recovery implementation and ABI bridge in `Native/cpp/` while the Zig storage path is qualified against it.

There is **no JavaScript runtime, Node test suite, npm package, or `.mjs` source** in the native migration branch. CI has a hard regression gate that fails if JavaScript is reintroduced.

## Canonical formats

- Noqeri implementation source: `.noqeri`
- NoeSQL script/example source: `.noe` while grammar migration is finalized
- Canonical database storage: `.noedb`
- Zig page format: `NQDB0002`
- Zig WAL framing: `NQTX0002`

Legacy `.nqd` and `.nqdb` data are migration inputs only. Never silently reinterpret them as `.noedb`.

## Build and test

Requirements:

- Zig **0.15.2**
- stable Rust toolchain with Cargo
- C++17 compiler
- GNU Make
- OpenSSL development headers
- SQLite development headers for the comparison benchmark

Run the complete native suite:

```sh
zig build test
```

Or through Make:

```sh
make test
```

Build release-safe native components:

```sh
zig build native
```

Individual suites:

```sh
zig build --build-file Native/zig/build.zig test
cargo test --manifest-path Native/rust/Cargo.toml --all-targets
make -C Native/cpp test
```

The repository also enforces:

```sh
make no-js
```

which fails if a `.js`, `.mjs`, `.cjs`, or `.jsx` file appears.

## Rust runtime

The Rust runtime implements the application-facing wrapper around `noesql_session_v2`.

It validates:

- ABI version
- canonical `.noedb` paths
- UTF-8 source
- 1 MiB source limit
- 16 MiB output limit
- callback presence
- connection and prepared-statement lifetimes
- begin / commit / rollback forwarding
- provider error propagation

CLI diagnostics:

```sh
cargo run --manifest-path Native/rust/Cargo.toml -- version
cargo run --manifest-path Native/rust/Cargo.toml -- check tests/school.noe
cargo run --manifest-path Native/rust/Cargo.toml -- check-db scratch.noedb
```

The CLI deliberately does not pretend the unfinished Stage-2 provider is production-qualified. Database execution becomes available only through a validated provider implementing the stable session ABI.

## Language example

```noe
db school
table users {
  id: int key auto
  name: text required
  age: int
}

users add {name:"Amina", age:21}
users get name, age where age >= 18 sort name take 10
```

The native Noqeri modules currently cover lexer/parser foundations, typed IR, row validation, authorization, limits, isolation contracts, secure execution and WAL/storage ABI integration. More complex relational features must be considered qualified only after they pass compiled Noqeri-to-native end-to-end tests.

## Repository layout

```text
Database/        Noqeri language and execution modules
Native/rust/     Rust host/runtime/session layer and CLI
Native/zig/      Zig pager, WAL, index and C ABI
Native/cpp/      C++ compatibility kernel and ABI tests
Tools/           Zig-native repository contract tests
Grammar/         NoeSQL grammar
tests/           Noqeri/native fixtures
Benchmarks/      Native benchmark sources and methodology
Doc/             architecture, storage, security and qualification notes
build.zig        top-level native build/test orchestrator
Makefile         developer convenience entrypoints
```

## Current release gates

NoeSQL should not be described as production-ready until all of these are demonstrated on the native stack:

1. A pinned Noqeri Stage-2 compiler produces the relational executor used by the runtime.
2. The executor links cleanly through the stable ABI to the Rust host and Zig/C++ storage components.
3. CRUD, joins, aggregation, typed parameters and rollback pass native end-to-end tests.
4. Cold restart, WAL replay, checkpoint, conflicting-writer and crash-boundary tests pass.
5. Authentication, authorization, tenant isolation and resource limits pass adversarial tests.
6. File-format compatibility and migration tests prove no silent legacy-format reinterpretation.
7. Linux qualification is complete before cross-platform claims are made.

Historical documents under `Doc/` may describe the former JavaScript reference implementation. Those documents are retained as migration history, not as the current runtime architecture.

## License

GPL-3.0-only.

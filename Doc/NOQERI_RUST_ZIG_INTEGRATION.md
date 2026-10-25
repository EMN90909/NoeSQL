# NoeSQL integration: Noqeri + Rust + Zig

## Ownership

NoeSQL uses three source-language layers:

1. **Noqeri** owns database semantics, typed execution modules, policy/auth/limits, and the scalar native decision bridge.
2. **Rust** owns the safe host/runtime boundary: source parsing/runtime services, session APIs, native symbol calls, handle validation, page encoding/decoding, and orchestration.
3. **Zig** owns durable storage: pages, WAL, indexes, locking, recovery, snapshots, and checkpoints.

There is no C or C++ compatibility layer in NoeSQL.

## Native bridge contract

`Database/noesql_native_bridge.noqeri` imports the Stage-2 semantic bridge and exports native bridge metadata.

The direct AOT path deliberately keeps the Noqeri/Rust call surface scalar-only because Noqeri's current backend documents arbitrary aggregate C-ABI interop as experimental. Rust calls:

- `noesqlNativeBridgeVersion()`
- `noesqlStage2BoolRow(operation, present, current_id, current_enabled, requested_id, requested_enabled)`

Rust then performs the trusted storage orchestration through Zig.

This gives Noqeri semantic ownership without pretending the current backend can safely pass arbitrary strings, records, callbacks, or ownership-bearing aggregates across a portable ABI.

## Zig boundary

The canonical pager implementation is `Native/zig/src/*.zig`.

Rust declares the required Zig symbols directly in `Native/rust/src/zig_pager.rs`. No generated C header is required. Noqeri's experimental declarations live in `Database/noesql_pager_abi.noqeri`.

The current native CRUD qualification format uses a 4096-byte page:

- bytes 0..8: `NQNR0001`
- bytes 8..16: little-endian i64 row id
- byte 16: boolean 0/1
- remaining bytes: zero

All-zero pages represent absence.

## Build

```sh
make native-chain NOQERI_COMPILER=/absolute/path/to/noqeri
```

That command:

1. builds Zig storage;
2. compiles `Database/noesql_native_bridge.noqeri` to native assembly;
3. archives the emitted object as `libnoesql_noqeri.a`;
4. links Rust against the Noqeri archive and Zig static library;
5. runs the native bridge tests and CRUD smoke chain.

## Source-stack gate

```sh
make no-c-family
```

fails if C or C++ source/header files are reintroduced into NoeSQL.

The separate Noqeri repository may still use its bootstrap implementation to build the compiler until Noqeri completes native self-hosting. That does not change NoeSQL's repository source stack.

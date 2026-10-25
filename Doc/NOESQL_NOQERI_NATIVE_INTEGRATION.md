# Noqeri native integration

NoeSQL no longer uses a C++ compatibility shim.

## Boundary design

The direct native path uses Database/noesql_native_bridge.noqeri for scalar semantic decisions. Native/rust/src/noqeri_native.rs reads and writes the row page through Zig, calls the Noqeri scalar kernel, validates the returned status, then commits through Zig.

This split is deliberate. Noqeri's current native export ABI is suitable for the qualified scalar Linux x86-64 path used here, while its own FFI documentation still marks arbitrary portable native exports as experimental.

## Ownership

- Noqeri decides database operation semantics.
- Rust validates paths, owns native handles and converts page bytes to bounded Rust values.
- Zig owns durable page/WAL/index behavior.

## Qualification command

    make native-chain NOQERI_COMPILER=/absolute/path/to/noqeri

The command must compile Database/noesql_native_bridge.noqeri to a static archive, link it with the Rust runtime and Zig storage library, and pass native_chain_smoke.

## Portability

Do not claim this scalar export is a portable cross-platform C ABI. Cross-platform native export qualification belongs in Noqeri. NoeSQL should widen the direct ABI only after Noqeri has calling-convention and layout conformance tests for the target platform.

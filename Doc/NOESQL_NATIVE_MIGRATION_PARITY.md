# Native migration parity gates

## 25 October 2026: Noqeri + Rust + Zig only

The old Native/cpp compatibility layer has been removed. NoeSQL implementation ownership is now:

- Noqeri: typed database semantics and scalar native semantic kernels.
- Rust: host/runtime, parser/reference behavior, safe native boundary, CLI and snapshot codec.
- Zig: pages, WAL, indexes, recovery and durable storage.

Completed by this migration:

- [x] Delete Native/cpp and its C++ page-store, ABI shims, tests and benchmarks.
- [x] Delete orphaned Native/link-tests C fixtures.
- [x] Add Database/noesql_native_bridge.noqeri as the direct native semantic kernel.
- [x] Replace Rust calls to noesql_cpp_* with direct scalar Noqeri exports.
- [x] Keep pointer-heavy pager/handle operations in Rust and call Zig through its versioned C ABI.
- [x] Preserve the existing NQNR0001 bool-row page encoding used by the native smoke path.
- [x] Add source gates that reject JavaScript and C/C++ implementation files from NoeSQL.
- [x] Update CI to compile the Noqeri kernel, verify its exported symbols, link Rust directly to it and execute the Rust -> Noqeri -> Zig smoke chain.

## ABI rule

Noqeri's current native backend does not yet claim a portable arbitrary C ABI for records, callbacks and pointer-heavy exports. The direct AOT boundary in NoeSQL is therefore intentionally scalar and currently qualified only for Linux x86-64. Rust owns the unsafe pointer/string/handle boundary to Zig.

This is a stronger architecture than recreating the deleted C++ shim in Noqeri with unqualified pointer layouts.

## Existing completed migration work

- [x] JavaScript/Node runtime removed from NoeSQL.
- [x] Reference lexer/parser and general relational runtime ported to Rust.
- [x] Rust snapshot codec connected to Zig NQDB0002/NQTX0002 storage.
- [x] Zig cold-reopen, WAL, checkpoint and index coverage retained.
- [x] Verified Stage-1 -> Stage-2 -> Stage-3 NQRI bootstrap path retained.
- [x] Executable Rust -> Stage-2 NQRI semantics -> Zig storage chain retained.

## Remaining qualification gates

- [ ] Record a green executing Noqeri/Rust/Zig GitHub Actions run.
- [ ] Expand the bounded native Noqeri row semantic kernel into the full general-schema relational path.
- [ ] Add native differential tests against an independently trusted SQL oracle.
- [ ] Complete concurrent-writer, crash-boundary, power-loss and platform qualification.
- [ ] Complete explicit legacy .nqd/.nqdb -> .noedb migration qualification.
- [ ] Replace Stage-0 AOT provenance with native self-hosted Stage-2 provenance only after Noqeri reaches that milestone.

A green source build is necessary but is not by itself a production database durability certificate.

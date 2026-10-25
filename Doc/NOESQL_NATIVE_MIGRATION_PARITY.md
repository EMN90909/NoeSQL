# Native migration parity gates

NoeSQL's repository-native source stack is **Noqeri + Rust + Zig**.

The previous C++ compatibility kernel has been removed. Its useful orchestration responsibility now lives in Rust, while Noqeri remains the semantic decision layer and Zig remains the durable storage layer.

## Completed

- [x] Removed JavaScript, Node.js, npm packaging, and JavaScript source from NoeSQL.
- [x] Added a hard `make no-js` regression gate.
- [x] Removed `Native/cpp/`, C/C++ headers, C link fixtures, and the legacy C benchmark source.
- [x] Added `make no-c-family` to reject `.c`, `.cc`, `.cpp`, `.cxx`, `.h`, `.hh`, and `.hpp` files in NoeSQL.
- [x] Ported the reference lexer/parser and general relational runtime to Rust.
- [x] Kept typed database semantics and native execution modules in Noqeri.
- [x] Kept durable page/WAL/index storage in Zig.
- [x] Moved page read/decode/stage/commit orchestration from the former C++ Noqeri/Zig shim into Rust.
- [x] Rust now calls the native Noqeri scalar decision function directly and then commits through the Zig pager.
- [x] Added `Database/noesql_native_bridge.noqeri` as the native AOT entry module.
- [x] Preserved the verified Stage-1 -> Stage-2 -> Stage-3 NQRI bootstrap equality gate.
- [x] Preserved the executable Rust -> Stage-2 NQRI -> Zig qualification chain.
- [x] Preserved Rust snapshot codec, relational, constraint, rollback, and Zig cold-reopen coverage.
- [x] CI now labels and validates the repository as Noqeri + Rust + Zig.

## Native compiler status

Noqeri's current Stage-0 compiler can emit native Linux x86-64 assembly. NoeSQL uses it to compile the narrow scalar native bridge. That compiler currently comes from the separate Noqeri repository and is a bootstrap toolchain dependency, not NoeSQL source.

Noqeri's self-hosted Stage-2 compiler remains NQRI-hosted. NoeSQL therefore keeps two claims separate:

1. **native Noqeri program execution** through the current AOT backend;
2. **self-hosted Stage-2 qualification** through Stage-2/Stage-3 NQRI equality.

NoeSQL must not call Stage-2 natively self-hosted until Noqeri itself reaches that milestone.

## Remaining qualification gates

- [ ] Record a green executing CI run for the Noqeri/Rust/Zig matrix.
- [ ] Expand the bounded native Noqeri scalar/page bridge into the full general-schema relational path.
- [ ] Add native differential tests against an independently trusted SQL oracle.
- [ ] Complete concurrent-writer, crash-boundary, power-loss methodology, and platform qualification.
- [ ] Complete explicit legacy `.nqd`/`.nqdb` -> `.noedb` migration qualification.
- [ ] Replace Stage-0 AOT provenance with native self-hosted Stage-2 provenance only after Noqeri reaches that compiler milestone.

A workflow with zero executed steps is runner failure, not source-test evidence. A green executing run remains required before release qualification.

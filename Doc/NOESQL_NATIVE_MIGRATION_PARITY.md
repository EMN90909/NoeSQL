# Native migration parity gates

This file prevents the JavaScript removal from being mistaken for complete semantic parity.

## Completed in this migration

- [x] JavaScript, Node.js and npm removed from the repository runtime surface.
- [x] Rust native runtime/session ABI wrapper added.
- [x] Rust native CLI diagnostics added.
- [x] Existing C++ compatibility storage/ABI tests retained.
- [x] Existing Zig pager/WAL/index tests retained.
- [x] Root Zig build orchestrates Rust, Zig and C++ validation.
- [x] CI rejects reintroduction of JavaScript source files.
- [x] Canonical .noedb path checks and source/output limits are enforced in Rust.

## Required before claiming full native parity

- [ ] Compile the current Noqeri lexer, parser, IR and executor with the pinned Stage-2 compiler.
- [ ] Link the compiled Noqeri executor through the stable C ABI into the Rust session host.
- [ ] Route Noqeri execution to the Zig pager/WAL/index implementation.
- [ ] Preserve C++ compatibility tests until byte-level storage parity is demonstrated.
- [ ] Recreate native CRUD acceptance coverage.
- [ ] Recreate join, left join, aggregation, HAVING and grouping coverage.
- [ ] Recreate typed parameter and named query coverage.
- [ ] Recreate transaction rollback and atomicity coverage.
- [ ] Recreate WAL sequence, corruption rejection and restart recovery coverage.
- [ ] Recreate path-hardening, catalog-protection and resource-limit coverage.
- [ ] Recreate concurrent-writer and read-only snapshot coverage.
- [ ] Recreate large database and migration coverage.
- [ ] Recreate fuzz/property tests against the Noqeri parser/executor.
- [ ] Add differential tests against an independently trusted SQL oracle.
- [ ] Produce release artifacts only after the above pass on an executing CI runner.

A red GitHub Actions run with zero executed steps is a runner-provisioning failure, not evidence that source tests failed. A green run is required before merging this migration.

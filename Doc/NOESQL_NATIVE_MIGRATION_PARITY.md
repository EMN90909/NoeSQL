# Native migration parity gates

PR #12 replaces the former JavaScript/Node reference surface with a native Rust host/runtime and uses Zig for durable page/WAL/index storage. C++ remains a compatibility kernel while the Noqeri compiler itself completes native self-hosting.

## Completed in PR #12

- [x] Removed JavaScript, Node.js, npm packaging, npm release workflows, and every `.js`/`.mjs`/`.cjs`/`.jsx` file from NoeSQL.
- [x] Added a hard `make no-js` regression gate.
- [x] Ported the reference lexer/parser to Rust with source, token, nesting, UTF-8, escape, and reserved-name validation.
- [x] Ported typed values, schemas, required/key/unique/auto constraints, foreign keys, referenced-row delete protection, CRUD, predicates, bound parameters, whole-program atomic rollback, alter-add/default backfill, indexes, named queries, inner/left joins, grouping, HAVING, count/sum/min/max, sorting, skip/take, and JSON result encoding to Rust.
- [x] Added a bounded Rust binary snapshot codec with corruption, version, length, UTF-8, and count checks.
- [x] Added Rust CLI execution: `check`, `check-db`, `run`, and `query`.
- [x] Connected Rust persistence to the Zig `NQDB0002` pager and `NQTX0002` WAL through a C ABI.
- [x] Added atomic Zig snapshot read/write with SHA-256 manifest validation.
- [x] Added Zig cold-reopen snapshot tests and Rust cold-reopen/checkpoint persistence tests.
- [x] Refactored Noqeri executor calls behind request records so the current x86-64 AOT backend's six-argument calling-convention limit no longer blocks compilation.
- [x] Added a C-compatible Noqeri native bridge and a Rust wrapper for it.
- [x] Added a Rust native build helper that compiles the Noqeri bridge with the current Stage-0 AOT compiler, assembles it, and creates `libnoesql_noqeri.a`.
- [x] Added an executable Rust -> Noqeri AOT -> Zig CRUD smoke chain.
- [x] Added a verified Stage-1 -> Stage-2 -> Stage-3 NQRI bootstrap gate with Stage-2/Stage-3 byte equality.
- [x] Added a Stage-2-compiled NoeSQL executor bridge and executable Rust -> Stage-2 NQRI -> Zig CRUD smoke chain.
- [x] Ported the bounded relational query planner to Rust, including the hash-join fast path with full predicate rechecks.
- [x] Restored Rust acceptance coverage for constraints, rollback, joins, grouping/HAVING, parameters, alter migration, large row workloads, and codec corruption.
- [x] Preserved existing Zig pager/WAL/index and C++ compatibility/durability tests.
- [x] CI now contains the full Stage-0 AOT build/link chain instead of Node-based reference tests.

## Native compiler status

Noqeri's **current Stage-0 C++ compiler can emit native Linux x86-64 assembly**, so PR #12 uses it to build the real NoeSQL Noqeri bridge. This is genuine native Noqeri program execution.

Noqeri's self-hosted Stage-2 compiler is an NQRI bootstrap and is **not a native self-hosted compiler**. PR #12 now consumes that Stage-2 path honestly: CI rebuilds Stage-2, rebuilds Stage-3, requires byte equality, compiles a NoeSQL executor with Stage-2, and lets Rust host the NQRI execution while Zig owns durable storage. Native compiler self-hosting remains a Noqeri compiler milestone, not a reason to keep JavaScript in NoeSQL.

Accordingly, PR #12 must not claim that Noqeri Stage-2 is natively self-hosted. The runtime migration itself no longer depends on JavaScript.

## Remaining qualification gates

- [ ] Obtain an executing CI runner and record a green run of the new Rust/Noqeri/Zig/C++ matrix.
- [ ] Expand the bounded native Noqeri page-row bridge into the full general-schema relational path.
- [ ] Add native differential tests against an independently trusted SQL oracle.
- [ ] Complete concurrent-writer, crash-boundary, power-loss methodology, and platform qualification.
- [ ] Complete explicit legacy `.nqd`/`.nqdb` -> `.noedb` migration qualification.
- [ ] Replace Stage-0 AOT provenance with native self-hosted Stage-2 provenance only after Noqeri itself reaches that compiler milestone.

A GitHub Actions run with zero executed steps is a runner-provisioning failure, not source-test evidence. A green executing run is required before release qualification.

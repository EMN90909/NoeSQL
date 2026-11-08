# Noqeri ↔ NoeSQL compiler integration contract

## Verified source ownership

Noqeri owns compiler semantics in `Compiler/selfhost/` and exposes `stage2CompileToNir` from `Compiler/selfhost/main.nqr`. Existing `Database/storage.nqr` declares operating-system functions for handles, locks, WAL and fsync. NoeSQL owns its `.noeqr` frontend and typed `NoeIrPlan` schema.

## Proposed linking sequence (NOT integrated today)

1. Parse a `.noeqr` module into a validated AST, resolve table/column identifiers and named parameters.
2. Type check against a catalog version; lower to `NoeIrPlan` and exact projection indices.
3. Generate a typed Noqeri `.nqr` adapter that imports `noesql_api.nqr`; never splice user values into text-based queries.
4. Resolve `NoeQueryContext` with authenticated tenant principal, resource budget, and schema version.
5. Call `noeApiValidate` before all executor operations. Route execution only through a mandatory authorized entry point.
6. Host provider supplies atomic locks, durable journal, fsync, encryption and validated identity. A generation counter alone is not thread safe.

## Stage 2 additions (not compiled into a complete relational engine)

The Noqeri-owned expression parser has been synchronized to `Database/noesql_parser.nqr`, and a separate native Zig pager ABI v2 is declared in `Database/noesql_pager_abi.nqr`. `scripts/verify-noqeri-bootstrap.mjs` enforces pinned Noqeri revision/source closure and compiler artifact verification before any future integration qualifies. **There is still no Noqeri-compiled statement frontend/executor linked to the Zig page ABI.**

## Current implementation

`Database/noesql_api.nqr` provides a pure, Noqeri-owned typed ABI/policy validation function. `tests/noesql_native_api.nqr` is its native conformance fixture. **Native compilation and execution are blocked without a trusted Noqeri stage-0 binary.**

The native NoeSQL AST parser/lowerer, build target integration, host storage provider, authentication and production concurrency are still incomplete. JS reference tests do not prove these pieces.

## Release gates

Build stage1→stage2→stage3 compiler; compile all native .nqr modules and their native tests; compile a real `.noeqr` CRUD application into executable output; run correct isolated/durable transactions and live concurrent stress tests; compare workloads against SQLite on identical hardware. No performance or >8/10 production rating is warranted until then.

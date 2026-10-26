# Experimental MVCC, authorization, typed fields and SQL export

Date: 2026-10-26

This branch extends the code in PR #20. It is **not** a production-ready database/server implementation. The test suites require a Rust/Zig-enabled Linux runner. Nothing in this document substitutes for executed CI, release qualification, crash simulation or independent security testing.

## In-process MVCC

Rust module: `Native/rust/src/mvcc.rs`. `MvccStore::snapshot()` pins an immutable generation of the relational `Database` in an `Arc`; readers run pre-parsed SELECTs without modifying or cloning tables. `begin()` returns a transaction that obtains copy-on-write state only when it modifies the database. `commit()` validates the base generation while holding the in-process write lock. A conflicting writer returns `MvccError::Conflict`, never silently overwrites a committed transaction. Rollback does not publish writes.

**Limitations:** This has no persistent generation compare-and-swap, no cross-process MVCC, no predicate locking, no serializable isolation and no durable WAL integration. The existing CLI still uses its exclusive Zig pager and is NOT changed to read concurrently.

## Embedded authorization

Rust module: `Native/rust/src/authorization.rs`. `AccessControl::new()` denies all reads, writes and schema changes by default. A caller explicitly grants table read/write rights; schema-admin rights are separate. The `AuthorizedSession` façade keeps its mutable database handle private. The authorizer checks both tables of a join and recursively checks every statement within an atomic block. It enforces source-byte and returned-row budgets, and rejects a failing authorization without publishing database changes.

An optional `restrict_rows(table, column, Value)` policy adds a mandatory equality predicate to reads, updates and deletes, rejects inserted rows carrying a different tenant value, and disallows modifying the tenant column. JOINs involving any row-policy-protected table fail closed rather than risk an incomplete policy rewrite. Tests cover tenant escape attempts and nested mutations.

**Limitations:** This is only an opt-in **in-process** authorization boundary. Raw `Database::execute` remains administrative, policies are not persisted, untrusted callers cannot safely receive raw database or process access, and the CLI/native Noqeri paths do not enforce these policies. No transport-layer authentication, encrypted secrets, robust tenant isolation across processes, complete SQL row-level security or tamper-resistant audit records have been added.

## Validated text-backed types

New field names: `date`, `datetime`, `uuid`, `json`, `binary`.

- `date`: `YYYY-MM-DD`, Gregorian calendar, including leap years.
- `datetime`: canonical UTC `YYYY-MM-DDTHH:MM:SS.mmmZ`. Other offsets must be explicitly normalized by clients.
- `uuid`: lowercase canonical hexadecimal `8-4-4-4-12`.
- `json`: strictly validated JSON under 1 MiB, maximum nesting depth 32, valid Unicode escapes including surrogate pairs.
- `binary`: lowercase hex string prefixed with `0x`, size-limited to 1 MiB of binary content.

These values are still encoded as Rust `Value::Text` and stored in the existing v2 Rust payload encoding. The new field kind tags are append-only. Logical snapshot decode now verifies declared types, required columns and unknown column names in addition to storage checksums.

**Limitations:** There is no decimal type yet. These are validated text-backed types, not native binary/date/JSON indexes, JSON-path functions, timezone conversion or vector representations. Old NoeSQL binaries may reject snapshots containing new type tags. New types require explicit upgrade compatibility tests before deployment.

## SQLite and PostgreSQL interoperability

The `interop::export_sql` API and `noesql export-sql <db.noedb> <sqlite|postgres> <new.sql>` CLI generate a bounded (16 MiB) logical SQL export for an **empty target database**. Properly quote identifiers and string literals, preserve scalar rows, integer/boolean/NULL values, date/UUID types where supported, and PostgreSQL binary hex as `decode(..., 'hex')`. The exporter rejects stored named queries, indexes, default expressions and reference constraints rather than dropping their semantics.

**Limitations:** The tool does not connect to SQLite/PostgreSQL, import SQL, stream large datasets, migrate sequences or prove result parity. Do not call this a drop-in adapter or complete interoperability.

## v3 storage and native Noqeri relational work

The v3 allocator from PR #20 is an isolated, checksummed bitmap prototype supporting up to 65,536 4 KiB logical page IDs; it is NOT yet a v3 WAL pager. The live v2 format still supports 256 physical pages. There is no crash-safe v2-to-v3 migration, true per-row storage, online compaction or free-space reclamation. The Noqeri native bridge still owns only the bounded scalar bool-row decision function. Full relational join/aggregation/constraint semantics run in the Rust reference engine, not the compiled Noqeri bridge.

A full release requires versioned shadow-file migration and rollback, WAL-logged page allocation/free, on-disk MVCC with independent readers, native relational ABI parity, full authentication/RLS, a proper decimal implementation, native SQLite/PostgreSQL drivers and demonstrably green CI.

## Validate on the pinned Linux/Rust/Zig toolchain

```sh
make test
make stress
make benchmark-parser-reads
NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" \
  cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets --features zig-pager
```

No version number or production-readiness label should be advanced until those commands and all cross-runtime qualification tests execute successfully.

## Additional hardening (2026-10-26)

- Authorized in-process MVCC façade (`AuthorizedMvccStore`) applies grants and tenant row filters to both immutable snapshots and optimistic transactions. Result-budget errors discard staged writes. This is *not* durable or cross-process security; the underlying unrestricted MVCC store remains an administrative API.
- Embedded authorized read-only queries now use the immutable reference execution path without copying the full database. No limit on intermediate JOIN/group materialization is claimed.
- `AuthorizedSession::into_database` is now crate-private to avoid handing unrestricted state back to callers holding only a restricted façade. The lower-level `Database` API remains intentionally unrestricted for trusted administrators.
- Relational GROUP BY now keys on typed tuples instead of formatted strings. SQLite logical export adds boolean-domain checks, and PostgreSQL export pins standard-conforming string behavior inside the import transaction.
- Added focused regression tests for tenant-scoped MVCC reads/writes, write rejection after result-budget errors, typed composite GROUP BY keys, and SQL export safeguards. **None of these have been executed in this environment** (Rust/Zig toolchains unavailable); they require CI execution before merging.

**Still not complete:** persistent MVCC, live v3 pager/migration, native Noqeri relational execution, full authN/durable RLS, exact decimals, and online SQLite/Postgres interoperability remain open acceptance gates. Do not remove the draft status or declare this release production-ready.

# Noqeri native framework integration (2026-11-06)

**Status: implementation committed; not compiled or qualified.**

Upstream source reviewed: EMN90909/Noqeri main commit
6fea0307b84e5213e687d484c36f92a1f89e25a4.

## Framework compatibility scan

| Noqeri component | Integration status |
| --- | --- |
| Compiler/bootstrap/web_native_noesql_v2.hpp | ABI-v2 Rust callback table and native exported getter implemented |
| Compiler/bootstrap/web_native_db.hpp | Real provider accepts the Noqeri native-web JSON binding profile through NOESQL_NATIVE_PROVIDER; no subprocess or alternate SQL engine |
| tests/native_web_noesql_session.py | Existing Noqeri test is mock-only; NoeSQL now includes an optional real-engine E2E harness |
| Lib/std/noesql.noqeri | Source-level noesqlHost* externs **remain unimplemented** in the Noqeri compiler/runtime host; C session ABI does not automatically resolve Noqeri string ABI |
| Compiler/selfhost/main.noqeri | New Stage-2 NIR capabilities exist; full NoeIR-to-native compiled relational execution remains incomplete |
| Database/noesql_api.noqeri and noesql_secure_executor.noqeri | Experimental typed policy helpers, not the full persistent relational engine |
| Native web TLS, auth and sessions | Existing loopback, reverse-proxy-origin, CSRF and expiry support are not full database authentication, tenant isolation or RLS |
| New Noqeri AI, graphics and generic frameworks | Not relevant direct dependencies of the database engine; no implicit integration or invented compatibility claims |

The Noqeri repository still retains reference and legacy database modules for
compatibility. They are not silently imported as the canonical NoeSQL v3 engine.

## Implemented in NoeSQL

1. Native Rust cdylib exports noesql_session_v2_get with ABI version 2.
   Exposes the 11 callbacks matching the Noqeri web C++ layout, including
   open/close, prepare/release, execute/query, begin/commit/rollback, last_error.
2. Opens existing, format-verified NQDB0003 stores only. NQDB0002 and unknown
   formats fail closed. No implicit schema creation, migration or SQLite fallback.
3. Strict native JSON binding parser: 32 KiB request cap, 32 fields, unique
   64-byte ASCII identifier keys, 4096-byte string values and validated UTF-8.
   JSON strings stay TEXT, integer/bool/null literals retain types; explicitly
   tagged records can pass exact DECIMAL or validated UUID/date/datetime/JSON/
   binary values. No text interpolation or permissive coercion.
4. Both autocommit and explicit transactions: BEGIN pins a verified snapshot;
   reads see staged writes; writes remain private until COMMIT. COMMIT checks
   the durable generation under the exclusive writer lock and publishes one
   checksummed snapshot. ROLLBACK drops all pending writes.
5. Response bounds checked before any transaction is staged/published. A stale
   writer conflicts, and a commit failure after HEAD publication is marked
   CommitUncertain instead of incorrectly reporting a definitely aborted write.
6. Per-connection Mutex protects transactions and error state. Host must not
   close a handle while another callback uses it; opaque FFI handles remain
   trusted capabilities, not untrusted network session tokens.
7. Rust tests, new live Noqeri web/NoeSQL integration regression and a Makefile
   target are added. No GitHub Actions tests are started.

## Local tests and building (Linux only)

Requirements: stable Rust/Cargo, Python 3, a separately built Noqeri native-web
binary and an existing NoeSQL v3 file in a privately owned directory.

Run from the NoeSQL repository root:

    cargo test --locked --manifest-path Native/rust/Cargo.toml --lib session_bindings::tests
    cargo test --locked --manifest-path Native/rust/Cargo.toml --lib session_provider::tests
    cargo build --locked --release --manifest-path Native/rust/Cargo.toml --lib --bin noesql

The shared library is Native/rust/target/release/libnoesql_runtime.so.
Configure NOESQL_NATIVE_PROVIDER to its absolute path; Noqeri's native web
adapter uses that library instead of launching the CLI when set.

Run the real cross-repository verification without GitHub Actions:

    make test-noqeri-framework-local NOQERI_WEB=/absolute/path/to/noqeri-web NOQERI_REPO=/absolute/path/to/Noqeri

The integration test boots Noqeri's actual web server against a temporary
NQDB0003 store, performs GET, a parameterized POST, an unsafe-looking string
insertion, and a failing duplicate; then checks durable generation and rows.

**None of the commands have been executed here:** this authoring environment
does not have Rust, Cargo or Zig compilers. Newly added tests are source-level
fixtures, not independent performance, correctness or crash evidence.

## Remaining release blockers

- Connect Noqeri source-level noesqlHost* externs to the canonical native
  session ABI with safe, tested Noqeri string/handle/ownership representations.
- Full typed Noqeri NoeIR lowering for joins, aggregates, indexes, constraints
  and NoeSQL scripts; cross-compiler differential conformance and Stage-2.
- Complete production authentication, tenant policy persistence, authorization
  at every external boundary, credential lifecycle, cancellation and audits.
- Safe v3 generation and B+ page reclamation, incremental relational pages,
  bounded storage growth, portable recovery and v2-to-v3 cutover.
- Crash/power/ENOSPC/short-write fault testing, concurrent stress and real
  benchmarks on a pinned operating system/filesystem and toolchain.
- Independent security review, stable ABI governance and release provenance.

See Doc/NOESQL_PRODUCTION_READINESS_2026-11-06.md and issue #21.
**No production release or automatic merge is authorized by this increment.**

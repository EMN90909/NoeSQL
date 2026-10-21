# NoeSQL ownership and v2 host/session ABI

NoeSQL owns the `.noedb` durable database format and associated Zig pager/WAL. Noqeri imports its interface; the Noqeri compiler should not vendor its own independent database engine. SQLite and PostgreSQL are optional **external** adapters, not replacements.

`Native/cpp/noesql_session_abi.h` defines the **versioned** `noesql_session_v2` callback table with open/close, prepare/release, execute/query, begin/commit/rollback and last-error reporting. `noesql_session_abi.cpp` provides pointer, buffer and ABI validation around provider callbacks; it does not implement SQL execution or the native engine. It does not change the legacy v1 one-shot `noesql_engine_abi.h` contract. The C++ test uses a mock provider and therefore only establishes ABI-wrapper validation, **not** actual Noqeri-to-NoeSQL query execution.

The separate `Runtime/noesql-session.mjs` offers `database('app.noedb')` as an **experimental Node reference adapter**, delegating all reads/writes to `Runtime/noesql-store.mjs` and the existing Zig-compatible `NQDB0002` pager layout. `db.execute(source)`, `db.query(name,parameters)`, `db.atomic(source)`, `db.verify()`, and `db.snapshot()` are supported by the reference engine, subject to its documented limits. Interactive `db.begin()` currently throws rather than faking transactional isolation. It is not proof of full ACID certification or production-native performance.

Migrate legacy `.nqd/.nqdb` explicitly:
`node Runtime/noesql-cli.mjs migrate ./legacy.nqdb ./new.noedb`.
Do not simply rename incompatible bytes. Preserve historical fixtures.

**Remaining required:** implement a real NoeSQL v2 provider using the Noqeri parser/planner/executor and Zig storage, link it into the Noqeri compiler/runtime host, and run compiler-created `.nqri` CRUD/transaction/crash/concurrency tests against native `.noedb` on CI. No v2 provider has been qualified in this branch. Do not mark the ecosystem complete before this gate.

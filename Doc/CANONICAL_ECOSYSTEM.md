# NoeSQL ownership and integration contract

NoeSQL is the unique database engine for new Noqeri applications. Noqeri must call NoeSQL through `Native/cpp/noesql_runtime_abi.h`, never maintain an independent relational engine in its compiler repository.

The versioned C ABI defines opaque database and statement handles and bounded `open`, `close`, `prepare`, `execute`, `query`, `finalize`, `begin`, `commit`, `rollback`, and `last_error`. `noesql_runtime_abi.cpp` validates and dispatches provider callbacks; these callbacks are **not yet** proof that a compiler-generated Noqeri engine is linked.

New data files use `.noedb` and Zig-compatible NQDB0002 pages. Old reference `.nqd/.nqdb` are accepted only on explicit legacy paths. `noesql migrate <legacy.nqd|legacy.nqdb> <output.noedb>` verifies the legacy checksum/format before conversion and does not overwrite an existing destination. The independent NoqeriDB NQDB1 format is not compatible and is not migrated by this command.

The `Database/*.noqeri` modules are the canonical NoeSQL language sources. `Database/*.nqr` and historic tests remain for compatibility and regression evidence. Database runtime reference code written in JavaScript and the SQLite adapter are transitional, **not** the claimed compiled Noqeri-native engine. Keep all Zig storage, WAL, planner, index, isolation and native security tests in place. Performance and native bootstrap claims require independent qualification.

Registry persistence uses `registry.noedb` for package metadata only, with immutable `.nqpkg` files in separate object storage. The Registry client and website must access its versioned HTTP interface, not embed engine copies.

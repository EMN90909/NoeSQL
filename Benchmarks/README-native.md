# NoeSQL native benchmark status

The former C++ page-store benchmark was retired with the NoeSQL C++ compatibility layer.

The active NoeSQL source stack is:

- Noqeri for database semantics and native execution modules
- Rust for the host/runtime, relational reference runtime, codecs, and ABI orchestration
- Zig for durable pages, WAL, indexes, recovery, snapshots, and locking

The historical C++ benchmark numbers that previously lived in this document are **not current NoeSQL performance results** and must not be used for performance claims.

## Current benchmark policy

Before claiming NoeSQL is faster than SQLite, PostgreSQL, or another database, the benchmark must exercise the current Noqeri + Rust + Zig path and must record:

- exact NoeSQL commit and Noqeri compiler commit
- hardware, OS, filesystem, storage device, and durability settings
- dataset size and row/value widths
- warm-up policy and cache state
- transaction batch size
- p50/p95/p99 latency and throughput
- WAL/checkpoint write amplification
- repeated runs with variance
- equivalent durability settings for every compared database

`Native/rust/src/bin/benchmark_compare_sqlite.rs` is currently only a placeholder. It is not evidence of comparative performance.

The supported validation commands are:

```sh
make test
make native-chain NOQERI_COMPILER=/absolute/path/to/noqeri
```

Do not revive the removed C++ benchmark harness as a compatibility shortcut. New benchmark code should use the current Noqeri + Rust + Zig architecture.

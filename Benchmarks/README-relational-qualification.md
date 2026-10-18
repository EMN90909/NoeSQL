# Reproducible relational benchmark qualification

`npm run benchmark:relational-baseline` measures a **SQLite-only** baseline on a new on-disk database, WAL journal, `synchronous=FULL`, explicit `BEGIN IMMEDIATE` / `COMMIT` for each transaction, 4KiB BLOB payloads, cold-restart digest/quick-check, p50/p95/p99 latencies and multiple independent samples. It is **not** a NoeSQL measurement.

`npm run benchmark:relational-compare` is deliberately blocked until the real compiled Noqeri→Zig binary passes the full pinned native E2E qualifier AND exposes a comparable timed transaction API. Noqeri's typed single-row Zig source does **not** yet constitute a complete relational engine or satisfy the dialect used by the JS reference.

## Commands

```sh
node --test tests/noesql-relational-benchmark.test.mjs
npm run benchmark:relational-baseline
npm run benchmark:relational-compare  # fails safely without verified native executable
```

For production comparison, require exactly matched logical data types, durability/sync behavior, transactions per batch, indexes, cold restart, hardware/device, and repeat counts. Publish raw run times, device details, environmental variability, compiler provenance, and all conformance results. Do not equate a tiny key-value page write test with general-purpose SQL query performance. PostgreSQL has not been measured in this project.

## 19 October 2026 isolated worker observation

On this conversation's temporary Linux container, with Node.js 22.16.0 `node:sqlite`, seven SQLite-only trials of 200 distinct durable commits writing 4096-byte BLOBs each yielded a median approximately 3,405 transactions/s (observed 2,226–3,561). All seven passed an integrity check and cold-restart row checksum. The filesystem's virtualization and storage cache are unknown; these are illustrative baseline observations, **not** performance claims for NoeSQL.

NoeSQL native executable cannot be benchmarked: Zig 0.15.2 and the trusted Noqeri compiler seed are unavailable, the container has no external DNS, and previous GitHub Actions runners returned failure without captured job steps.

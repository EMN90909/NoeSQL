# NoeSQL native physical-store microbenchmark

Run `make -C Native/cpp benchmark` to compile and run the C++ benchmark locally. The printed JSON reports:

- Batched rows committed per second in one durable transaction.
- 20 independent durable single-row commits per second.
- Authenticated session B+ tree point-read P50, P95, and P99 latencies.
- Database checkpoint size after the workload.

By default this uses 500 rows. To change the row count compile `noesql_page_store.cpp` with `benchmark_page_store.cpp` and run the binary with a value from 1 to 5000. Each run uses a fresh temporary database and removes it afterward. No SQLite/PostgreSQL numbers are produced, and results must not be presented as competitive claims. The current v1 kernel does full-image WAL and checkpoint rewrites for each transaction.

## SQLite side-by-side baseline (requires libsqlite3-dev)

Run `make -C Native/cpp benchmark-compare`. The C++ harness
`benchmark_compare_sqlite.cpp` executes 500 (configurable 1..5000) keyed
120-byte values using one durable batch transaction, 500 point lookups,
and 20 independent durable commits for each engine. SQLite uses
`PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL` and `WITHOUT ROWID` primary
keys. NoeSQL uses `Store::login` once and passes a session to operations.

These are **not exactly equivalent**: SQLite has no embedded password system,
NoeSQL maintains a credential record and checks session revocation on every
operation, and NoeSQL's WAL/checkpoint implementation differs architecturally.
Both tests use fresh databases on the same machine, in one process, and do not
include server/network latency. Small in-memory cache-friendly datasets cannot
predict real workloads. Run repeated trials on target production filesystems,
record hardware, OS, filesystem, caching, process counts and real data sizes.

Example two local Debian/GCC 14.2 trials (500 rows; 18 Oct 2026):

| Engine | Batch rows/s | Independent commits/s | Lookup P99 ms |
| --- | ---: | ---: | ---: |
| NoeSQL native run 1 | 185056 | 566 | 0.0361 |
| SQLite WAL FULL run 1 | 661459 | 4182 | 0.0019 |
| NoeSQL native run 2 | 148699 | 399 | 0.0435 |
| SQLite WAL FULL run 2 | 651745 | 4366 | 0.0014 |

SQLite wins these three sampled workloads. No PostgreSQL server benchmark was
conducted. Do **not** claim NoeSQL exceeds or matches SQLite or PostgreSQL.

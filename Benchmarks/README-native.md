# NoeSQL native physical-store microbenchmark

Run `make -C Native/cpp benchmark` to compile and run the C++ benchmark locally. The printed JSON reports:

- Batched rows committed per second in one durable transaction.
- 20 independent durable single-row commits per second.
- Authenticated session B+ tree point-read P50, P95, and P99 latencies.
- Database checkpoint size after the workload.

By default this uses 500 rows. To change the row count compile `noesql_page_store.cpp` with `benchmark_page_store.cpp` and run the binary with a value from 1 to 5000. Each run uses a fresh temporary database and removes it afterward. No SQLite/PostgreSQL numbers are produced, and results must not be presented as competitive claims. The current v1 kernel does full-image WAL and checkpoint rewrites for each transaction.

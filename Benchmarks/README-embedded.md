# Embedded NoeSQL overhead benchmark

Run `node Benchmarks/embedded-vs-sqlite.mjs [rows] [lookups]` on Node.js 22.13+.

Both workloads operate on SQLite 3.x through `node:sqlite`. The direct side uses reusable prepared SQL; the NoeSQL side runs a readable `from...select...where...take` plan through the validated cached compiler and prepared query. An additional 5,000-row batch uses SQLite's direct prepared insert versus the NoeSQL record API. This measures the language and binding overhead only; it **does not** measure the independent C++ `.nqd` kernel and does **not** compare PostgreSQL.

In-memory backend and small deterministic workload mean results primarily reflect CPU and runtime costs, not fsync, WAL throughput, checkpoint starvation, SSD wear, 99.9-percentile production latency or distributed availability. Re-run under identical OS/hardware and assess correctness before interpreting ratios. The SQLite-backed option derives its durability and concurrency from SQLite, not a new NoeSQL implementation.

# NoeSQL native security policy: scope and limitations

Three Noqeri-authored modules were added: `Database/noesql_isolation.nqr` (single-writer generation and reader exclusion state), `Database/noesql_auth.nqr` (scoped authorization after host-validated identity) and `Database/noesql_limits.nqr` (request operation/row budget accounting).

These are **policy primitives, not an integrated transactional engine**. The transaction state is not thread-safe by itself; call sites require an OS writer lock and atomic synchronization. Reader exclusion is not MVCC and does not permit concurrent writers. Commit policy requires a caller-provided durable-WAL confirmation but cannot prove an fsync happened. The authorization module **does not authenticate credentials**, and must receive a trusted, authenticated principal; it has no independently implemented identity provider, token verification, revocation, roles catalog or encrypted channel. Budgets must be checked at every executor step and complemented with host-enforced wall-time, memory and cancellation rules.

Run the native fixture and microbenchmark ONLY using a verified Noqeri compiler/runtime:

```sh
noqeri check tests/noesql_native_security.nqr
noqeri run tests/noesql_native_security.nqr
noqeri run Benchmarks/noesql_policy.nqr
```

No GitHub Actions. No performance numbers are asserted without measured execution. Before production: compile and execute the fixtures, connect authorization to every executor entry point, build the physical transaction manager, test cross-process isolation, verify fsync/recovery, fuzz corrupt inputs, and benchmark concurrent workloads across Linux/Windows/macOS. This increment does not establish parity with PostgreSQL or SQLite.

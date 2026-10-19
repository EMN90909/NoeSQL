# NoeSQL native index milestone

The native Zig WAL pager now exposes a **bounded persisted integer index**
through the established ABI v2, and Noqeri source declarations/wrappers are in
`Database/noesql_pager_abi.nqr` and `Database/noesql_native_index.nqr`.

**Implemented in source:** sorted signed-int64 keys, binary-search point and
range probes, unique insert, delete, integrity-checking SHA-256 page encoding,
transaction-local read-your-writes, transaction staging, WAL commit and recovery
through the existing Zig pager. `tests/noesql_native_index.nqr` is a cold
reopen smoke input for a *verified* native Noqeri compiler.

**Strict constraints:** one reserved index page holds at most 337 distinct keys;
pages must be allocated separately by the trusted caller; the caller must
update index and rows atomically; there is no B+tree splitting, composite/text
keys, NULL handling, SQL planner integration, automatic index maintenance, or
concurrent MVCC readers. Using an index page as a data/catalog page can corrupt
the database. The Zig pager is Linux-only.

The Node.js reference engine remains available for compatibility and tests.
Pure `get` programs now avoid full database clones and hash joins no longer
stringify scalar keys; this is NOT a replacement of the SQL runtime by a
compiled Noqeri executable. Noqeri's Stage-2 native compiler and Zig linkage
must pass the repository's strict proof and native E2E gates before release.
Native Zig/Noqeri binaries were not built in an environment without Zig and a
trusted Stage-2 compiler seed; the ABI needs compilation and integration tests.
## Reference profiling (not the native engine)

After the pure-SELECT no-copy changes, profile realistic bounded reads with:

```bash
npm run benchmark:read-path
node Benchmarks/read-path-profile.mjs 5000 7
node --cpu-prof Benchmarks/read-path-profile.mjs 1000 15
```

The profiler builds valid synthetic rows directly so SQL insertion time is not
mixed into SELECT timings. It measures a selective equality scan, a filter
scan and a hash-join/group query, and separately measures the deep database
clone removed from pure reads. It verifies result sizes on every trial.
The indexed physical Zig single-page API is not yet used by the reference SQL
optimizer. This profiler **does not measure a native Noqeri executable or
SQLite** and cannot justify cross-engine speed claims.

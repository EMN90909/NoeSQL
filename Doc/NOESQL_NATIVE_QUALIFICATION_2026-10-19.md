# NoeSQL native SQL qualification progress (2026-10-19)

## Implemented and evidence-backed

- `.noedb` Node reference and Zig pager have byte-compatible NQDB0002 and NQTX0002 storage envelopes. The Node reference state remains a **checksummed JSON application payload** (`NQRS0001`) inside these binary pages, not Zig-native Noqeri relational storage.
- The NoeSQL reference relational executor selects hash joins for moderate/large safe equality joins, including equality terms inside pure conjunctive ON predicates, and nested loops for tiny tables. Full ON predicates remain rechecked. The planner does not execute arbitrary arithmetic during optimization, and base-only safe WHERE filters can precede joins. This **is not a cost-based multi-join SQL optimizer** and is not a persisted native physical index.
- `scripts/build-noqeri-zig-link.mjs` now retains the compiled Noqeri machine-code assembly, object, linked Zig test executable and hashed provenance after a successful *proof-gated* native smoke test. Output is `build/noesql-native-smoke/` by default, or an absolute `NOESQL_NATIVE_OUT_DIR`. An existing artifact directory is never overwritten. The script checks that the compiled object really references Zig pager APIs and that the committed checkpoint begins with NQDB0002.
- New tests exercise planner decisions and relational query equivalence, multi-process write serialization, single-program rollback, and SIGKILL at WAL and checkpoint boundaries.

## Independent gates still blocking a complete native SQL engine

1. **Trusted Noqeri native compiler:** The public Noqeri Stage-2 portable host currently exposes syntax, semantic checks and NIR lowering, **not** the verified `build source.nqr output.s` machine-code command. Its Stage-2 bootstrap artifact/provenance chain is not qualified. A C++ bootstrap executable must never be relabeled as the Noqeri self-hosted compiler.
2. **Genuine native SQL CLI:** The existing Noqeri-compiled smoke only tests a bounded typed single-row integer/boolean query and pager ABI, **not** `.nqds` parsing, multi-table schema/catalog, joins, aggregates, transactions, roles, indexes and `identity/run/query/verify/checkpoint` semantics in native machine code. `npm run qualify:native-e2e` must remain fail-closed.
3. **Transaction proof:** The Linux reference code serializes readers and writers with `flock`, and Zig implements WAL and snapshots. Neither has independently established power-loss durability, cross-platform locking, MVCC, read/write conflict resolution, filesystem crash consistency or formal ACID qualification.
4. **Optimizer proof:** Persistent indexes, composite statistics, join-order search, buffer/memory budgeting, query-plan cache, spill-to-disk and comparable SQLite/PostgreSQL benchmark results are still missing in the verified native Noqeri executor.
5. **CI:** GitHub Actions jobs have been marked failed while their steps and logs are unavailable through the connected API. The repository cannot be called green until the provider supplies usable runner logs and the actual matrix passes.

## Commands on a trusted Linux x64 builder

```sh
npm test
npm run test:noedb
npm run test:native-link-contract
npm run test:zig
npm run build:zig
# Requires independent Stage-2 compiler SHA-256 proof, Noqeri checkout and Zig 0.15.2:
npm run build:native-link
# Separately, a complete native SQL executable and identity proof are required:
npm run qualify:native-e2e
npm run benchmark:relational-compare
```

Passing a reference query oracle, a mocked CLI, a smoke executable, or a file-header check **does not** qualify the complete engine. Keep native release and npm publication blocked until those actual checks pass on pinned binaries and the full durability/performance matrix is documented.
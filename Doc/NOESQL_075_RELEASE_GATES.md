# NoeSQL 0.7.5 release readiness

Status: **0.7.5-rc.1, NOT STABLE and NOT production ready** (18 October 2026).

## Verified locally in this increment

- Existing native POSIX test suite passes after restricting test crash hooks to
  `NOESQL_TEST_FAULT_INJECTION` builds.
- A separate ordinary production-style build ignores the fault injection
  environment variables, eliminating a possible environment-controlled process exit.
- The index consistency decoder refuses physical tree depth above 16 levels.
- The optional SQLite WAL/FULL comparison harness builds and reports results.

## Blocking gates for 0.7.5 stable

1. Compile and integrate Noqeri syntax, typed execution, schema constraints and
   migrations into the native page kernel. The current kernel is a KV store.
2. Replace full-image WAL/checkpoint on every write with incremental crash-safe
   page updates and a documented durable commit protocol.
3. Prove multi-operation transaction isolation, concurrent readers/writers,
   compatibility across filesystem and OS failures, and crash recovery at all IO stages.
4. Harden access control for untrusted clients: authorization boundary, secure
   credential transport, brute-force protections, audit history, key rotation,
   file/directory threat model, backups/restore, and quota enforcement under IO stalls.
5. Stress/fuzz the on-disk format and parsers; prove ASan/UBSan and realistic
   performance on Linux/macOS/Windows with a stable migration policy.
6. Run reproducible SQLite and PostgreSQL comparisons on multiple dataset sizes
   and publish methodology; complete an independent storage/security audit.

No stable tag should be created merely by changing the package version. A
failing or unverified gate remains a blocker, even when a local smoke test passes.

## Build/test

```sh
make -C Native/cpp test
make -C Native/cpp benchmark-compare # optional sqlite3 development library
npm test                              # JavaScript reference suite
```

The native compiled kernel is POSIX-only and must not be used to store
irreplaceable or customer production data. Back up any existing `.nqd` files;
JSON reference `.nqd` and native page `.nqd` are mutually incompatible.

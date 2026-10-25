> **Historical document.** This file describes an earlier C++/legacy migration stage and is retained only for engineering history. The C/C++ implementation it references has been removed from NoeSQL. The current source architecture is **Noqeri + Rust + Zig**. See `README.md`, `Doc/NOQERI_RUST_ZIG_INTEGRATION.md`, and `Doc/NOESQL_NATIVE_MIGRATION_PARITY.md` for current build and qualification guidance.

# Native page-store v1: implemented prototype, not a production NoeSQL release

Date: 2026-10-18. This is a **native C++17 storage-kernel prototype**, not yet a native Noqeri-compiled SQL engine, not an approved migration from the JavaScript `.nqd` reference format, and not production-ready. It deliberately does **not** replace `Database/*.nqr`, `Runtime/noesql-reference.mjs`, or the established `noesql_engine_v1` callback ABI.

## What this increment actually implements

- **Binary page format:** fixed 4096-byte pages with a versioned file header, generation number, SHA-256 page checksums, row-count sanity checks, and explicit endian encoding. Binary files begin with `NQPG0001`. Neither JSON `.nqd` nor a future native Noqeri format is silently accepted. Existing experimental data must be backed up and migrated using a separately designed and tested tool. No compatible migration tool is provided here.
- **Physical index:** an on-disk immutable B+ tree built from sorted key/value pairs; leaf pages contain encoded rows and internal pages contain child pointers plus first-key separators. Index lookup on a clean checkpoint reads header/credentials/target path pages with `pread`; it does not decode the entire file. The slow path validates and replays a committed WAL image. Full consistency verification traverses all pages and rejects cycles, duplicate keys, invalid separators, checksum errors, invalid page references, and unreachable pages.
- **Serializable single-statement/batch writes:** a persistent `.lck` file with POSIX `flock(LOCK_EX)` for writer transactions and `LOCK_SH` for individual read operations. The lock is intentionally never unlinked because doing so would allow separate processes to lock different inodes. Each `transact()` call atomically applies up to `max_mutations` writes. Reads see only complete committed states. This is **not MVCC**, not a snapshot held across multiple `get()` calls, and **not simultaneous read/write throughput**: writers block readers, and vice versa. Local POSIX filesystem locking is a requirement; no multi-host or network-filesystem safety is claimed.
- **WAL and recovery:** each transaction emits a single full-snapshot redo WAL record (`NQWL0001`) containing generation, length, SHA-256 image digest, and authenticated-by-checksum commit footer (not cryptographic MAC). The commit path is WAL write, WAL `fsync`, directory `fsync`, checkpoint temp write and `fsync`, atomic rename and directory `fsync`, then WAL deletion and directory `fsync`. Recovery selects a valid committed WAL image even if the main checkpoint is corrupted, and checkpoints it before the next transaction. Incomplete WAL suffixes are ignored, complete malformed frames fail closed. The WAL format is not an append-many-frames log yet, and every commit rewrites the **whole database**. POSIX subprocess forced-exit tests cover two commit boundaries; **power failure and cross-platform hardware guarantees have NOT been established**.
- **Credential checking and permissions:** an explicit bootstrap admin password, per-user random salts, OpenSSL `EVP_PBE_scrypt` (`N=32768,r=8,p=1`), constant-time verifier comparisons, read-only/writer roles, an admin-only add/remove-user operation, and a reserved internal key namespace. There is no hardcoded production credential. `Session` caches proof after login but the stored verifier is rechecked for revocation on each operation. Passwords are **not encrypted at rest**. Credentials are stored in a checksum-protected but otherwise unencrypted database, so file readers can perform offline attacks. There is **no network authentication service, TLS, audit log, brute-force throttling, key rotation, secure password reset, or full per-table authorization**. Do not expose this kernel to untrusted clients as a server.
- **Resource limits:** file quota (64 MiB default), max key (128 bytes), max value (1024 bytes), max mutations per call (10,000), scan row cap (100,000), password length bounds, and a cooperative transaction deadline (30 s default, checked before commit). These are limits, not a query scheduler, OS memory sandbox, cancellation facility, or deadline interrupt for blocked IO. A database may remain too large for a constrained machine.
- **Native integration:** a versioned `noesql_storage_abi.h` C interface with opaque handles and authentication sessions for open/init/login/get/put/delete plus the pre-existing separate `noesql_engine_v1` language ABI. A Noqeri-compiled executor has **not yet been linked to it**. The `.noe` syntax is unchanged and still executed by the separate reference runtime.

## Build and test

Requires a POSIX platform, C++17 compiler, OpenSSL headers and `libcrypto` (tested locally with Debian Linux, GCC 14.2 and OpenSSL 3.5). Avoid GitHub Actions if working locally:

```sh
make -C Native/cpp test       # native page, C ABI, and existing bridge contract tests
make -C Native/cpp native-cli # binary: /tmp/noesql-native by default
make -C Native/cpp benchmark  # prints machine-local JSON, no competitor comparison
npm test                      # separate JavaScript reference regression suite
```

Example (development data only; avoid using real secrets in shell history):

```sh
export NOESQL_PASSWORD='a-long-development-password-from-your-secret-manager'
/tmp/noesql-native init ./scratch.nqd
/tmp/noesql-native put ./scratch.nqd admin hello world
/tmp/noesql-native get ./scratch.nqd admin hello
/tmp/noesql-native verify ./scratch.nqd
unset NOESQL_PASSWORD
```

`useradd` takes the new password from `NOESQL_NEW_PASSWORD`, not a command-line argument. Environment variables are a CLI demonstration transport, not an application secrets solution. Do not run the example against an existing JavaScript-reference `.nqd`: it is intentionally rejected.

## Benchmarks: how to reproduce and interpret

`benchmark_page_store.cpp` reports JSON covering creation cost, a 500-row single-transaction batch, 20 independently durable single-row commits, and 500 authenticated-session point reads (P50/P95/P99). Files are temporary and wiped after each run; the workload includes the cost of file IO, WAL, sync, and checkpoint. These microbenchmarks are **not comparable** to SQLite or PostgreSQL until identical schema, hardware, durability mode, warm-up, fault budget, and client behaviors are tested.

One sample of the local environment (Debian container, GCC 14.2/OpenSSL 3.5, 500 rows) recorded approximately 5,395 rows/s batched, 525 independent commits/s, and point-read P50/P95/P99 0.021/0.028/0.045 ms. Results vary. Per-row commit throughput **degrades with database size** because every commit rebuilds the B+ tree and WAL full snapshot. This version is neither designed nor proven faster than mature databases.

## Exact outstanding production release gates

1. Integrate the real Noqeri parser, typed NoeIR, query executor, schema, SQL constraints and secondary/composite indexes with this physical store via the C ABI; compile and run using a trusted Noqeri toolchain.
2. Incremental copy-on-write page updates, free-list/garbage collection, and small frame-based WAL rather than rewriting the full file. Add ordered multi-column and secondary indexes and bounded range cursors.
3. Add multi-statement transaction handles, documented isolation across client calls, MVCC or proven snapshot reader concurrency, retry/cancellation semantics, and multi-process tests with hundreds of thousands of interleavings. Decide on durability guarantees for unsupported filesystems and platform sync behavior.
4. Complete input/path threat modeling, secure directory-descriptor traversal, password change/rotation, a trusted server authorization boundary, rate limiting, robust audit logging, encryption policy and tenant isolation.
5. Implement schema migration tooling, rollback/roll-forward version compatibility tests, online verification/repair tools, backups/restores, stress/fuzz/property tests, fault-injected fsync/rename/ENOSPC failures, power-loss on actual hardware, ASan/UBSan builds, and Windows/macOS qualification.
6. Compare against SQLite and PostgreSQL with the same conditions (including durable single-row commits), and run long-duration randomized crash/concurrency testing. Subject the code to external security and storage-engine review before any production label.

## Security and operational boundaries

Do not keep live data in this prototype. Use a private trusted filesystem directory, not a directory writable by adversaries, and do not use a network-mounted filesystem. Checksums are *not* signatures or MACs and cannot stop a malicious writer with access to the file. There remains a check-before-rename race against hostile directories. Passwords are not protected against privileged host users, swap/memory dumps, or anyone who can read the database file. Never interpret a successful build or a handful of crash tests as an ACID certification.

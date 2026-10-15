# NoeSQL security and concurrency hardening (experimental reference path)

## Implemented in PR #12

The reference .nqd store enforces a 16 MiB snapshot ceiling and a bounded NWAL size **before allocating input buffers**. Reads use the same validated descriptor for metadata and data, request O_NOFOLLOW where supported, and reject non-regular or multiply-linked files. The 1 MiB source-code size cap is checked before acquiring the exclusive writer lock.

Write transactions use wx-created lock files and fail closed when a second writer attempts to begin. Committed reference WAL records are authenticated for corruption detection by SHA-256 (an unkeyed checksum, **not** adversarial cryptographic authentication). POSIX fsync/checkpoint and SIGKILL tests exercise a subset of crash cases.

Reproduce locally (without GitHub Actions):

```sh
node --test tests/noesql-*.test.mjs
```

46 local reference tests passed in the development environment (Linux, Node 22), including five new security/concurrency regression tests. These are **not compiled native .nqr tests**, security certifications, or production performance benchmarks.

## Known risks and release blockers

1. The on-disk .nqd reference format is still a JSON full-snapshot rewrite, not indexed native page storage. Write cost grows with database size, and lock serialization allows only one writer.
2. Reader snapshots are not backed by an isolation-proof MVCC or read/write lock design. Readers may observe an unfsynced but fully written WAL commit frame; this is not a verified read-committed guarantee. Lock files left by process crashes must be handled manually; recovery cannot guess a safe lock owner.
3. O_NOFOLLOW may be unavailable on some platforms, and the parent directories, checkpoint rename path, and writer lock acquisition still need a complete symlink/path-traversal and hostile-filesystem threat model. Never use the experimental store across untrusted directories.
4. The SHA-256 checksum is unkeyed. It detects accidental corruption, not deliberate rewriting by an attacker with disk write access. Authentication, authorization, encryption, audit records and key rotation are unimplemented.
5. No cross-process stress test, multi-host lease fencing, cancellation/deadlines, query memory/time quota enforcement beyond source/read bounds, fault-injected native recovery, fuzzing or security audit has been performed.
6. No native Noqeri Stage-2 bootstrap/compiler proof or integrated .nqr storage engine exists. SQLite/PostgreSQL adapters have separate operational/security models.

## Next mandatory gates

- Establish a trusted Noqeri compiler seed, compile native .nqr modules, then integrate AST → typed NoeIR → executor → page store/WAL.
- Document and prove transactional durability, serializability/isolation, deadlock/timeout behavior, bounded query memory and query cancellation.
- Add adversarial parser and corrupted-file fuzzers, sanitizer/memory safety tests, independent restart/crash interleavings and security audit.
- Benchmark indexed lookups, scan throughput, write throughput, P95/P99 latency, memory growth and concurrent readers/writers against SQLite/PostgreSQL on the same machines and datasets.

NoeSQL cannot yet be described as more secure or faster than established databases. Do not merge or tag v1.0 on this evidence alone.

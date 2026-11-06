# NoeSQL engine release decision: hard production gates

Date: 2026-11-07. Target: **Linux x86-64, trusted embedded NoeSQL v3**.
Decision: **BLOCKED, NOT PRODUCTION READY.** A source merge, passing unit tests,
or one backup/restore demo is not equivalent to a production guarantee.

Noqeri native-web and the Rust provider execute database operations as trusted
in-process code; they do **not** authenticate end users. Do not expose the
administrative Rust FFI callbacks or the NoeSQL CLI to public clients.

## Hard P0 gates — ALL must pass with machine-verifiable artifacts

| Gate | Acceptance criteria | Status |
| --- | --- | --- |
| Native reproducible build | Pin Rust/Cargo, Zig 0.15.2, the exact Noqeri source commit, NQRI compiler seed; build twice and compare executable behavior and source digests | NOT VERIFIED |
| Engine semantics | Rust + Zig tests, ACID visibility, uniqueness/foreign keys, type constraints, joins, aggregates, transaction abort-on-error, parameter injection, stale writers; deterministic and differential against SQLite/PostgreSQL as applicable | NOT VERIFIED |
| Crash recovery | Automated fresh-process kill/ENOSPC/short-write/sync/rename interruption suite before and after HEAD publication; verify cold-reopen and no acknowledged writes are lost; demonstrate ambiguous-commit handling | NOT VERIFIED |
| Multiple clients | 1/8/32/128 independent readers/writers with serializability-or-snapshot contract stated and checked, deadlock and starvation bounds, multihour soak and process termination | NOT VERIFIED |
| Capacity/retention | Current full-snapshot rewrite growth characterized; v3 offline vacuum verified after crash; disk-full behavior, current index reachability, backups, and storage usage alerts tested | PARTIAL SOURCE ONLY |
| Restore and upgrade | Golden on-disk fixtures, cold restore on a different machine/filesystem, v2->v3 checksum/row parity, idempotent cutover and rollback | NOT VERIFIED |
| Native Noqeri API | Stage-2 NIR real Noqeri frontend, all noesqlHost* externs, compiler/linker ABI/ownership, real native web E2E; no mock-provider-only proof | NOT VERIFIED |
| Security | No public unauthenticated access; trusted parent path policy, recoverable secrets, persistent tenant roles/grants, authorization on every entry point, third-party threat analysis, fuzzer/sanitizer runs | NOT VERIFIED |
| Performance/SLO | Predeclared benchmark workloads, reproducible p50/p95/p99 read/write latency and throughput, fsync costs, read amplification, storage write amplification, low-memory/low-disk and fairness under contention | NOT VERIFIED |

A release gate must **fail closed** when the needed compiler, runtime, local
Noqeri tree, fixture or evidence is missing. Existing scripts intentionally
never manufacture a "production certified" flag.

## This increment: offline v3 maintenance

The v3 code now has an explicit operator-only maintenance path:

    noesql v3-check /private/data/app.noedb
    noesql v3-audit-permissions /private/data/app.noedb
    noesql backup /private/data/app.noedb /private/backup/app-verified.noedb
    noesql v3-check /private/backup/app-verified.noedb
    noesql v3-vacuum /private/data/app.noedb
    # Read the DRY-RUN result; stop all uncoordinated external backup readers.
    noesql v3-vacuum /private/data/app.noedb --apply --discard-history
    noesql v3-check /private/data/app.noedb

The vacuum takes the v3 **exclusive cross-process flock** and re-verifies
the committed HEAD/current generation and both legacy/stable-key index trees.
It fsyncs the HEAD parent before removal, verifies directory contents, refuses
unknown files/symlinks, removes obsolete generation files then fsyncs the
versions directory, and removes only unreachable index blobs/temporary files
then fsyncs the index directory. This is **explicit offline history
destruction**, not automatic compaction or incremental row storage.

**Limitations:** v3 still rewrites the full relational snapshot every write.
Vacuum frees space only when invoked; this does not bound transient peak disk
usage or solve poor write amplification. A concurrent file-copy agent not
obeying NoeSQL's lock can be disrupted. An attacker able to replace ancestor
directories may still exploit pathname races; private trusted ancestry is
mandatory. The committed data is in NQDB0003, not interoperable with SQLite.
Power-loss and filesystem behavior remain **unverified**.

## Run all local engineering qualification

This work requires a machine with the actual compiler/runtime toolchains and
a separately compiled Noqeri-native web server:

    export NOQERI_COMPILER=/absolute/path/to/noqeri
    export NOQERI_REPO=/absolute/path/to/Noqeri
    export NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri
    export NOQERI_WEB=/absolute/path/to/noqeri-web
    bash scripts/qualify-production-local.sh

The script invokes Makefile native Rust/Zig and Stage-2 chains, the real
Noqeri-web session-v2 provider test and tests/v3_vacuum_cli.py. It writes
a local evidence log and fails on missing prerequisites/test failures.
It does not invoke GitHub Actions.

Separate release evidence remains mandatory: independent crash/power and
disk-full tests, multi-client soak, security review, performance baselines,
restore drill and a human approval tied to specific exact binaries/source SHAs.
A passing source diff or clean branch is **not** such evidence.

## Merge policy

It is safe to merge **non-destructive** feature and test scaffolding for review
only after code review, but new deletion/GC code should NOT be promoted into
the production branch until built and exercised on a real filesystem.
Never tag v1.0 or represent the engine as production ready based on source
changes alone. Track outstanding gates in issue #21.

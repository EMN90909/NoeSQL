# NQDB0003 opt-in immutable generation store (UNQUALIFIED)

This is a Rust-hosted opt-in v3 page store, **not** a release-ready replacement
for the Zig NQDB0002 pager. No default behavior changes for existing databases.
The existing Zig path retains its 256-page limit. The versioned v3 store offers
immutable-generation MVCC between independent processes, but rewrites a full
relational snapshot each commit, retains old snapshots indefinitely, and has
not passed build, crash, concurrency or power-failure qualification.

## CLI (native Rust binary)

```sh
noesql v3-create /private/data/fresh.noedb schema.noe
noesql v3-check /private/data/fresh.noedb\n# Only for a crash during initial creation BEFORE a published HEAD exists:\nnoesql v3-discard-unpublished /private/data/unfinished.noedb
noesql v3-exec /private/data/fresh.noedb 'users add {name:"Amina"}'
noesql v3-query /private/data/fresh.noedb 'users get id,name sort id'

# Copy-only v2 -> v3 migration. The source v2 database remains unchanged.
noesql migrate-v3 /private/data/legacy.noedb /private/data/migrated.noedb
```

The v3 target must NOT already exist. Parent directories must be privately
owned; Linux advisory flock is used. A migrated v3 target is independent of
the source. A cutover of application traffic must be coordinated externally;
writes to the v2 source after the migration are not propagated to v3.

## Physical on-disk format

- \`/data/name.noedb\` is a 168-byte **HEAD** with \`NQDB0003\` magic, format
  version 3, page size 4096, monotonic generation, logical codec byte count,
  page count (up to 65,536), basename of an immutable generation file, 32-byte
  SHA-256 of that file and SHA-256 covering the head metadata.
- \`/data/name.noedb.versions/<generation>-<nonce>.pages\` is immutable once
  published. Its 64-byte \`NQPG0003\` header holds generation, logical length,
  page count and SHA-256 of the canonical Rust relational codec payload.
  Each frame is 4,096 bytes followed by a 32-byte SHA-256 checksum.
  The last page has zero padding, verified on load.
- \`/data/name.noedb.lck\` is the advisory lock: multiple shared snapshot
  readers, one exclusive writer. The lock file is never removed during use.
  Reader queries evaluate outside the lock after loading their immutable
  generation into memory. Serial writer commits publish a new generation.
- The \`NQDB0003\` header and \`NQPG0003\` data are never reinterpreted as
  \`NQDB0002\` or the separate Zig v3 allocator's bitmap prototype.

The v3 capacity limit is 65,536 * 4,096 logical bytes. This is an upper
bound for one serialized relational snapshot (not 65,536 independently
allocated B+tree/record pages). It is not a per-row incremental engine.

## Crash consistency and migration guarantees in the code

1. Serialize an isolated new generation and checksum every page.
2. \`fsync\` the generation file, then \`fsync\` its directory. Until then,
   the previous HEAD still points to the last committed generation.
3. Write and \`fsync\` a HEAD temporary file. Atomically \`rename\` it over
   the current HEAD (or \`hard_link\` with no-overwrite for a newly created DB).
4. \`fsync\` the parent directory. A crash between rename and parent fsync
   may show the old or new HEAD; either must point only to an already synced
   complete generation. If a synchronization call fails *after* rename, the
   caller may receive an error despite the new generation being visible;
   do not blindly retry a non-idempotent write.
5. Migration obtains the legacy Zig exclusive lock, reads and validates v2
   under that lock, creates the fresh v3 target and compares canonical
   relational payload bytes with source while the v2 lock remains held.
   The original v2 source stays in place as the rollback copy.\n   A retry with an existing v3 target succeeds only when the target is\n   fully valid and its relational bytes match the still-locked v2 source.
6. A crash before HEAD publication may leave an unreferenced generation
   file/directory. Do not automatically adopt, prune or reuse unreferenced
   generations. The target is only committed after a valid HEAD exists.
   An explicit `v3-discard-unpublished` recovery command removes only\n   unpublished orphan generation files after refusing unexpected entries;\n   it never removes data from a valid published HEAD. Version pruning of\n   committed historical generations remains unimplemented.

SHA-256 is a corruption checksum, not cryptographic authentication against
malicious file modification. There is no stored credential, tenant policy, or
encrypted page mechanism. Do not expose the unrestricted v3 CLI to untrusted
principals.

## Tests and validation

The added Rust unit/integration tests include SHA-256 known vectors, v3
create/cold reopen, strict HEAD/page corruption rejection, deterministic
failure between generation \`fsync\` and HEAD publication, concurrent in-process
reader snapshots, >256-page payload cold reopen, v2/v3 migration parity,
and multiple independent CLI reader/writer processes. Run:

```sh
make test
make stress
NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" \
  cargo test --locked --manifest-path Native/rust/Cargo.toml \
  --all-targets --features zig-pager
NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" \
  cargo test --locked --manifest-path Native/rust/Cargo.toml \
  --features zig-pager --test v3_process
```

**Actual execution status:** there is no Rust/Zig compiler in the authoring
environment, and recent GitHub Actions jobs have failed before recording
steps. These tests were added but have not executed. Do not treat the
source-level test cases as successful durability or linearizability evidence.

## Before enabling as the default engine

- Complete independent crash/power-loss tests at all file/directory fsync and
  rename boundaries (including power cuts, disk full, short writes and torn
  checksums), then repair/restart tests on supported filesystems.
- Implement bounded historical retention with pinned-reader safe garbage
  collection and durable free-page allocation; current files grow without
  bound, and commit encodes the full snapshot.
- Replace whole-database relational re-encoding with incremental record/page
  allocation and WAL-aware indexes (or formally redesign the format).
- Implement an online/single-path v2→v3 migration or explicit maintenance
  cutover protocol and crash-recovery mechanism for interrupted migrations.
  The current copy migration is safe for the source but **not** atomic
  application cutover.
- Integrate the pinned Noqeri relational compiler, durable authorization,
  audit/quotas and the SDK with the v3 path.
- Require working CI, Rust and Zig compilers, all native tests, multi-process
  soak, release attestation and benchmarks. Do not merge production tags
  based only on static review.

See \`Doc/ENGINE_V3_RELEASE_GATES.md\` and issue #21.

## Persistent B+ index-page format extension (experimental)

NQDB0003 continues to publish immutable checksum-bound generation files.
Newly written generations use reserved generation-header bytes 28..32 for
the count of **additional 4 KiB manifest frames** appended after the
relational data frames. Each manifest is bound to the generation's SHA-256
HEAD digest and describes the integer index roots, indexed-row counts,
table and column names. Zero means an older v3 generation with no index
manifest; it remains readable.

Each node is exactly 4,096 bytes and has magic `NQBI0001`, an internal
or leaf type, count and canonical zero padding. Leaf entries store signed
64-bit keys with row ordinal positions; branch entries store key bounds
and 256-bit child content hashes. Content-addressed immutable node files
live under `<database>.versions/index-pages/`. The same page blob is
reused across generations if its contents do not change. Before HEAD is
atomically published, all new node files and their containing directory
are fsynced, followed by the generation file and directory.

On cold reopen all manifest frames, node hashes, separators, row positions
and indexed data are checked. Corrupt or missing nodes fail the read.
Incremental write analysis retains prior authenticated roots for indexes
that a transaction's AST could not have changed; otherwise it rebuilds
that index in memory but writes only changed physical nodes. Existing
uncommitted node blobs after failed HEAD publication are harmless orphans
until an administrator performs a safe cleanup. Old committed pages must
not be removed without version reachability accounting.

**Unresolved:** page reclamation, allocator transaction logging,
incremental in-memory tree insert/split/delete algorithms, persistent
range/text/composite indexes, standalone disk index WAL, crash/power-fail
qualification and benchmarking. Keep the PR draft and the production
release gate closed until native CI actually runs and succeeds.

# NoeSQL .nqds source and .noedb storage (19 October 2026)

## Naming contract

- `.nqds` is a NoeSQL query, schema, migration, and named-query source script. The reference CLI accepts it; legacy `.nqd` and `.noe` scripts still work.
- `.noedb` is the preferred database filename in the reference runtime and Zig pager. Legacy `.nqdb` (both) and `.nqd` (reference only) remain accepted.
- **A shared extension does not mean shared bytes.** The reference runner uses checksummed `NOESQL-NQD-EXPERIMENTAL` JSON snapshots and `NWAL` records. The Zig pager uses `NQDB0002` 4-KiB pages and a different WAL. They are currently **not interoperable**. Never open a reference `.noedb` in the native pager or vice versa. The embedded SQLite backend still requires a separate `.sqlite` file.

## Reference durability and isolation

The reference writer takes an exclusive `.lock`, clones database state for program-level atomicity, appends a checksummed full-image WAL, syncs the WAL file and directory, atomically renames a checksummed snapshot, syncs the directory, and then removes the WAL. A complete committed WAL recovers a pre-checkpoint write; an incomplete transaction is not committed; invalid frames fail closed. A SIGKILL intentionally leaves a lock for manual review. Read-only operations do not write an unnecessary new WAL.

These are **bounded guarantees**, not a declaration of production-grade ACID: database-wide multiple-reader isolation, robust stale-lock reclamation, filesystem-specific power-loss durability, rollback/anti-replay protection, and interprocess crash orchestration are not completely qualified. A crash or exception between writing COMMIT and syncing it may leave an uncertain outcome. Applications must reconcile after errors.

## Native qualification

`npm run test:zig` verifies the isolated Zig pager. `npm run build:native-link` still requires pinned self-hosted Noqeri Stage-2 compiler provenance and actual machine-code generation and linking. `npm run qualify:native-e2e` demands a real native CLI that accepts `.nqds` and `.noedb`, executes the relational workflow, recovers storage, and produces genuine `NQDB0002` bytes. Neither script may fall back to the JS reference or an untrusted compiler. Native SQL and public npm publication remain blocked until these gates pass on verified artifacts.

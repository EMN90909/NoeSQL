# Zig native WAL history validation (19 October 2026)

The NQTX0002 WAL can legitimately contain a previously checkpointed transaction when a process crashes after the checkpoint rename and before WAL truncation. The recovered page bytes for that **same generation** must match the checkpoint; otherwise, two different versions of one committed transaction could be accepted on different runtimes.

`Native/zig/src/pager_v2.zig` now rejects two unsafe replay histories: (1) a complete WAL whose final generation predates the checkpoint, and (2) a record exactly at the checkpoint generation whose modified page bytes disagree with that checkpoint. This matches the `.noedb` reference reader's earlier protection and preserves valid replay of a checkpointed WAL followed by newer committed frames.

Native Zig regression tests reconstruct a retained WAL record, replay it after a later checkpoint, and construct a checksummed conflicting same-generation record. Both must fail closed with `error.CorruptWal` before exposing mutable page state.

These are accidental rollback/corruption safety checks, not authenticated anti-rollback guarantees: an attacker able to rewrite both files can forge new hashes, and a successful crash test does not prove filesystem power-loss durability.
# Unified NoeSQL `.noedb` page layout

Date: 2026-10-19. Status: **prototype with shared Zig-compatible binary page and WAL format**. This is not a self-hosted native Noqeri SQL engine, a full ACID certification, or a benchmark result.

## Canonical bytes

`.noedb` now uses Zig's existing NQDB0002 pager layout, not the earlier reference JSON checkpoint. `Runtime/noesql-zig-pages.mjs` encodes and decodes exactly the `Native/zig/src/pager_v2.zig` format:

- Header: 96 bytes; ASCII `NQDB0002`, little-endian 32-bit page size `4096`, maximum pages `256`, little-endian 64-bit generation, 32-byte active-page bitmap, and SHA-256 over header bytes 0..55 stored at bytes 56..87.
- Page slots: slot `id` at offset `96 + id * (4096 + 32)`, 4096 bytes of page data followed by SHA-256 of those bytes. Unused slots are sparse/padded zeros.
- WAL: `NQTX0002`, uint64 generation, uint32 mutation count, zero reserved word; each mutation is uint32 page id and 4096 bytes; `NQCM0002` marker plus SHA-256 over the entire record except its trailing digest. An incomplete transaction is uncommitted. Replay rejects corrupt complete records, repeated page ids, and generation gaps. The journal remains as an empty `.wal` file after successful checkpoint, matching the Zig pager.
- Application state: page 0 holds `NQRS0001`, version `1`, number of payload pages, exact byte length and SHA-256 over the serialized reference database state. Pages 1..N hold UTF-8 bytes, zero-padded. The `storageSequence` inside the state must equal the Zig checkpoint/WAL generation.

The Zig pager's `page_get` can retrieve this same page 0 and the payload pages; `noesql_zig_page_validate_reference_state` checks the page manifest and payload checksum. The ABI fixture `Native/link-tests/zig_page_roundtrip.c` validates Node-generated images using Zig and writes a Zig checkpoint containing Node-supplied pages, which Node reads back. Its invocation is `node scripts/verify-zig-page-roundtrip.mjs /path/to/compiled-Zig-linked-driver`.

## Locking and atomicity

On Linux the reference writer uses system `flock` to hold the **same persistent** `.noedb.lck` inode and exclusive lock as Zig. Never unlink that lockfile. Writers stage a checksummed full-state page mutation transaction, sync WAL and directory, fsync a temporary checkpoint, rename, sync parent, then truncate and fsync the WAL. Replay accepts complete checksummed committed transactions, rejects stale WAL generations, and discards partial tails before appending. Failure injection and checksum/locking regression tests are included.

This reference bridge intentionally uses **at most 256 mutated pages per transaction**, with one manifest plus at most 255 UTF-8 data pages (~1 MiB JSON). Larger database states reject before any new transaction begins. Shrinking state zeroes previously occupied reference payload pages atomically, and a reference writer fails closed if the native database contains page ids outside the reserved reference application area. Matching-generation WAL records must match the checkpoint's already committed page bytes. An eventual native incremental row/index encoding and transaction machinery must remove this limit and improve write amplification.

## Migration

Do **not** rename old JSON `.nqdb` / `.nqd` files to `.noedb`; they are incompatible. To migrate an existing legacy reference file, execute `node Runtime/noesql-cli.mjs migrate ./existing.nqdb ./new.noedb`. The source remains untouched and existing destinations are refused. Keep backups before migration. Existing `.nqdb` and `.nqd` filenames remain readable using the legacy reference codec, and the SQLite backend still requires its own `.sqlite` filename.

## Qualification boundary

The codec has local Node tests for page/WAL layout, WAL truncation, WAL checksum failures, replay, 2-writer exclusion and safe generation handling. The C driver and Zig ABI must actually be compiled and run under Zig 0.15.2 before claiming cross-language executable verification. Independent Noqeri Stage-2 compiler provenance, full SQL integration, production-grade multi-reader isolation, platforms beyond Linux, real power-loss validation, query optimizer parity, external security review, and green GitHub CI remain separate gates.

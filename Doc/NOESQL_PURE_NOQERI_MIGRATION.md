# Phase 1: NoeSQL Pure Noqeri Migration Inventory & Architecture

**Date:** 2026-11-10  
**Target:** 100% Pure Noqeri Engine (Eliminating Rust and Zig runtime dependencies)

---

## 1. Inventory of Current Modules & Migration Mapping

| Rust / Zig Module | Responsibility | Pure Noqeri Target Module | Status / Strategy |
| :--- | :--- | :--- | :--- |
| `Native/rust/src/reference.rs` | Lexer, AST, Parser, Relational Engine, CRUD, Joins, Aggregates, FK validation | `Database/noesql_engine.noqeri`, `Database/noesql_parser.noqeri`, `Database/noesql_lexer.noqeri` | Expand `Database/noesql_parser.noqeri` and `noesql_executor.noqeri` to support complete `.noeqr` DDL & DML statements. |
| `Native/rust/src/v3_store.rs` | NQDB0003 Generations, Head Management, Atomic Commit, Locking | `Database/noesql_v3_store.noqeri` | Implement file-backed generation tracking, checksum validation, and atomic commits in Noqeri. |
| `Native/rust/src/v3_bplus_cow.rs` | Copy-on-Write B+ Tree Indexing | `Database/noesql_bplus.noqeri` | Persistent B+ Tree with ordered key splitting, search, and page persistence. |
| `Native/rust/src/v3_data_pages.rs` | Content-addressed Data Blobs (`.bpd`) | `Database/noesql_data_pages.noqeri` | Content-addressed CoW page storage with deduplication. |
| `Native/rust/src/codec.rs` | Snapshot Serialization (`NQRP0001`) | `Database/noesql_codec.noqeri` | Little-endian binary codec for schemas, tables, and rows. |
| `Native/rust/src/authorization.rs` | Principal RBAC, Tenant Scopes, Path Security | `Database/noesql_auth.noqeri` | Already implemented in Noqeri; add path validation. |
| `Native/rust/src/mvcc.rs` | Multi-version Concurrency & Snapshot Isolation | `Database/noesql_isolation.noqeri` | Snapshot isolation with generation pinning and monotonic counter. |
| `Native/rust/src/rest_server.rs` | Loopback HTTP/1.1 API Daemon | `Database/noesql_server.noqeri` | Native HTTP server via `Lib/std/http/server.noqeri` and TCP socket abstractions. |
| `Native/rust/src/main.rs` | CLI Command Dispatcher | `Programs/noesql.noqeri` | Native executable CLI compiled via `noqeri build-exe`. |
| `Native/zig/src/storage_abi.zig` | Low-level Page/WAL C-ABI | Replaced by `noesql_wal.noqeri` and `noesql_v3_store.noqeri` | Obsoleted: pure Noqeri replaces Zig storage layer. |

---

## 2. Ranked Implementation Plan

1. **Phase 1: Architecture & Inventory** (Complete).
2. **Phase 2 & 4: Storage Foundations & Binary Codec** (`noesql_codec.noqeri`, `noesql_v3_store.noqeri`):
   - Checksummed binary serialization for tables, columns, rows, and indexes.
   - Atomic generation publication (`HEAD` swap) and fsync durability.
3. **Phase 3 & 6: Data Structures & B+ Tree Indexing** (`noesql_bplus.noqeri`):
   - Fast ordered B+ tree node splits, binary search, and range queries.
4. **Phase 5 & Concurrency**:
   - Single-writer cross-process advisory lock (`flock`) and multi-reader generation snapshots.
5. **Phase 8: Complete Engine Migration & CLI**:
   - Pure Noqeri `noesql` binary with all commands: `check`, `run`, `query`, `v3-create`, `v3-check`, `v3-vacuum`, `backup`, `restore`.
6. **Hardened Adversarial Quality Gates**:
   - Foreign-Key adversarial enforcement (self-referencing FKs, cascade/restrict, circular FKs).
   - Power-loss & fsync verification (atomic rename & crash recovery).
   - Query injection resistance (bound parameters, zero concatenation).
   - Fuzz testing of parser and corrupted storage pages.
   - Security: RBAC, tenant isolation, loopback restrictions.
   - Long-running multi-process concurrent soak test.

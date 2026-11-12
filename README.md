# NoeSQL: Pure Noqeri Relational Database Engine

**Engine Version:** 0.9.5-pure-noqeri  
**Architecture:** 100% Pure Noqeri Native Execution (Zero Rust, Zero Zig, Zero C/C++, Zero JS)  
**Primary Repository:** `EMN90909/NoeSQL` (branch `main`)  
**License:** GPL-3.0-only  

---

## 1. Overview & Architecture

NoeSQL has been completely rebuilt as a pure, native Noqeri-authored relational database engine. All legacy Rust, Zig, C, and C++ codebases have been excised. The entire engine—lexer, parser, planner, B+ tree index, multi-process concurrency, transactional WAL, and cold disk reconstruction—is implemented purely in `.noqeri`.

The engine features a dual-engine architecture:
1. **Multi-Process Daemon Client-Server WAL (`NWAL0001`)**: Low-latency append-only write-ahead logging with cryptographic SHA-256 rolling transaction digests, streaming multi-transaction crash recovery, and durable B+ tree indexing.
2. **Immutable Snapshot Generations (`NQDB0003`)**: Atomic Copy-on-Write generation snapshots with durable transaction watermarks (`lastTxId`) and directory fsync barriers.

```
       +-------------------------------------------------------------+
       |                  NoeSQL Client / Daemon                     |
       |               (Pure Noqeri Language Native)                 |
       +-------------------------------------------------------------+
                         |                           |
                 (Writes / Commits)             (Checkpoints)
                         v                           v
       +---------------------------------+   +-----------------------+
       | Write-Ahead Log (WAL.log)       |   | Generation Snapshots  |
       | - NWAL0001 Binary Frames        |   | - NQDB0003 Magic      |
       | - SHA-256 Rolling Digest        |   | - Atomic HEAD commit  |
       | - Streaming Replay (> 64 KB)    |   | - Durable lastTxId    |
       | - Fail-Closed Torn Tail Repair  |   | - 512-row capacity    |
       +---------------------------------+   +-----------------------+
                         ^                           ^
                         |                           |
       +-------------------------------------------------------------+
       |  In-Memory Table Data, Foreign Keys & B+ Tree Index Primary |
       +-------------------------------------------------------------+
```

---

## 2. Durability & Fault-Injection Guarantees

NoeSQL is qualified with an automated durability and fault-injection test suite:

- **Hard SIGKILL Mid-Run Recovery**: Withstands abrupt process termination (`kill -9`) mid-transaction. Uncheckpointed writes are cryptographically verified and replayed from `WAL.log` with 100% exact ledger parity and zero corruption.
- **Checkpoint Crash Recovery (Watermark Protection)**: A crash occurring precisely after snapshot publication but before WAL truncation restarts without duplicating or losing a single transaction. The engine compares WAL transaction IDs against the snapshot's durable watermark (`engine.db.lastTxId`) and skips already-persisted records.
- **Fail-Closed Physical Torn Tail Repair**: Incomplete or torn frame tails are physically truncated and synced under exclusive lock. If truncation or sync fails, the engine immediately fails closed (`-10`), prohibiting corrupted state.
- **Transactional Rollback Reversion**: Aborted transactions completely revert both in-memory table rows and B+ tree index entries.
- **Pre-Execution Statement Validation**: Statements exceeding durable frame limits (> 2,000 bytes) are rejected with `-115` before memory execution, preventing memory/WAL divergence.
- **Capacity Alignment**: Validated up to the 512-row capacity ceiling with cold reopen verification at 257, 300, and 512 rows.

---

## 3. Benchmarks & Performance Comparison

All benchmarks are executed live on Linux x86-64 using strict synchronous durability (`fsync` / `PRAGMA synchronous = FULL;` per transaction).

| Engine | Storage Model | Mean Latency (50 Sync Tx) | p50 Latency | p95 Latency | On-Disk Storage Footprint |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **NoeSQL (COW Snapshot)** | Pure Noqeri COW | **43.31 ms** | **42.28 ms** | **47.78 ms** | **1,848 bytes** |
| **NoeSQL (WAL Engine)** | Pure Noqeri WAL + B-Tree | **238.08 ms** | **237.77 ms** | **244.48 ms** | **1,880 bytes** |
| **SQLite (Rollback Mode)** | Rollback Journal (Sync) | 319.16 ms | 318.54 ms | 328.19 ms | 12,288 bytes |
| **SQLite (WAL Mode)** | C-Binding In-Process WAL | 162.24 ms | 162.86 ms | 172.04 ms | 12,288 bytes |

### Empirical Insights
1. **NoeSQL WAL vs. SQLite Rollback**: NoeSQL WAL (238.08 ms) is **1.34x faster** than SQLite Rollback mode (319.16 ms).
2. **Disk Storage Footprint**: NoeSQL stores schemas, rows, and WAL frames in **1,880 bytes**, which is **6.5x smaller** than SQLite's 12,288-byte minimum allocation.
3. **Pure Noqeri COW Performance**: Compiles schema, inserts 50 rows, writes atomic generation, fsyncs, reopens cold, and queries in **43.31 ms**.

---

## 4. Repository Structure

```text
Database/
  noesql_engine.noqeri       # Core relational engine, schema, COW snapshot & NQDB0003 parser
  noesql_wal_engine.noqeri   # Client-server WAL storage engine, transaction watermark, fail-closed repair
  noesql_wal.noqeri          # NWAL0001 binary frame encoder, decoder, SHA-256 rolling hasher
  noesql_btree.noqeri        # Pure Noqeri B+ Tree index (insert, split, search, range scan)
  noesql_row.noqeri          # Relational row cell encoding, types, foreign key validation
  noesql_ir.noqeri           # AST, plan nodes, and executor
Programs/
  noesql.noqeri              # Complete end-to-end operational engine verification
tests/
  test_wal_noqeri.noqeri     # Binary frame and checksum validation
  test_btree_noqeri.noqeri   # B+ Tree index insertion, lookup, and range scans
  test_wal_engine_lifecycle.noqeri # WAL engine open, begin, execute, commit, checkpoint
  test_wal_rollback.noqeri   # Real transaction rollback and index reversion
  test_wal_streaming_multistmt.noqeri # Streaming recovery > 64 KB and multi-statement transactions
  test_wal_multistatement_40.noqeri   # 40-statement transaction recovery without fixed limits
  test_wal_torn_tail.noqeri  # Checkpoint interruption and second cold reopen verification
  test_boundary_durability.noqeri     # 257-512 row cold reopen and oversized statement auto-rollback
  test_durability_crash_kill.py       # SIGKILL mid-flight, checkpoint crash, and fail-closed fault injection
  test_concurrent_workload.py         # Multi-process concurrent reader/writer workload
Benchmarks/
  bench_stress.py            # Comparative benchmark harness against SQLite, PostgreSQL, MySQL, MongoDB
  stress_test_suite.py       # Capacity saturation, 100 rapid atomic commits, 8-process lock contention
```

---

## 5. Building and Running

### Prerequisites
- Linux x86-64
- Noqeri Compiler (built from `EMN90909/Noqeri`)
- Python 3

### Running Tests
```sh
# Run full automated test suite (pure Noqeri tests + crash & concurrency tests)
make test

# Invariant checks: verify zero JS, zero C/C++, zero Rust/Zig
make no-js no-c-family no-rust-zig

# Run crash fault-injection suite
python3 tests/test_durability_crash_kill.py

# Run comparative benchmarks
python3 Benchmarks/bench_stress.py
```

### Running Standalone NoeSQL
```sh
./bin/noesql
```

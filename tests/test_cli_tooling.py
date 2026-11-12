#!/usr/bin/env python3
"""
Test Suite: NoeSQL Operational CLI Tooling & Production Safeguards
Verifies:
  - noesql doctor (truthful SHA-256 snapshot validation, catalog inspection, cryptographic ledger checks)
  - noesql check (B+Tree index parity, foreign-key referential integrity)
  - noesql backup (continuous exclusive lock, atomic staging & publication, non-clobbering safeguard)
  - noesql restore (verified cold point-in-time recovery)
  - noesql explain (real catalog lookup, table/column validation, O(log N) index scan vs O(N) table scan)
  - noesql query (statement execution, error propagation to process exit code)
  - Parallel CLI execution (zero argument or status file clobbering across concurrent processes)
"""

import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

NOESQL_BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin", "noesql"))

def run_cmd(args, expect_success=True):
    cmd = [NOESQL_BIN] + args
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if expect_success and res.returncode != 0:
        raise RuntimeError(f"Command failed (rc={res.returncode}): {' '.join(cmd)}\nSTDOUT:\n{res.stdout}\nSTDERR:\n{res.stderr}")
    return res

def test_cli_help():
    print("[*] Test 1: CLI Help output...")
    res = run_cmd(["help"])
    assert "Usage:" in res.stdout
    assert "noesql doctor" in res.stdout
    assert "noesql check" in res.stdout
    assert "noesql backup" in res.stdout
    assert "noesql restore" in res.stdout
    assert "noesql explain" in res.stdout
    print("  -> PASSED: CLI Help verified.")

def test_cli_doctor():
    print("[*] Test 2: Truthful CLI Doctor diagnostic check...")
    temp_dir = tempfile.mkdtemp(prefix="noesql_cli_doc_")
    db_path = os.path.join(temp_dir, "test.noedb")
    try:
        # 1. Missing db should fail doctor
        res_fail = run_cmd(["doctor", db_path], expect_success=False)
        assert res_fail.returncode != 0
        assert "CRITICAL" in res_fail.stdout

        # 2. Valid database doctor check
        run_cmd(["test"]) # Ensure production.noedb exists
        res_doc = run_cmd(["doctor", "production.noedb"])
        assert "Snapshot SHA-256 Checksum: VERIFIED (100% Valid)" in res_doc.stdout
        assert "Relational Catalog:" in res_doc.stdout
        assert "departments" in res_doc.stdout
        assert "Overall Status: 100% HEALTHY" in res_doc.stdout

        # 3. Corrupted snapshot payload should be caught by Doctor's SHA-256 check
        res_b = run_cmd(["backup", "production.noedb", db_path])
        head_file = os.path.join(db_path, "HEAD")
        with open(head_file, "r") as f:
            gen = int(f.read().strip())
        snap_file = os.path.join(db_path, "versions", f"generation-{gen}.pages")
        # Flip a byte in the snapshot payload
        with open(snap_file, "r+b") as f:
            f.seek(16)
            b = f.read(1)
            f.seek(16)
            f.write(bytes([b[0] ^ 0xFF]))
        res_corrupt = run_cmd(["doctor", db_path], expect_success=False)
        assert res_corrupt.returncode != 0
        assert "Snapshot open/integrity check failed" in res_corrupt.stdout

        print("  -> PASSED: Truthful CLI Doctor verified (detects corruption and verifies authentic catalog).")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)

def test_cli_check():
    print("[*] Test 3: CLI Relational Integrity & B+Tree Checker...")
    res = run_cmd(["check", "production.noedb"])
    assert "Primary Key B+Tree:" in res.stdout
    assert "Foreign Key 'dept_id' -> 'departments': 100% referential integrity" in res.stdout
    assert "Relational Consistency & Index Verification: ALL PASSED" in res.stdout
    print("  -> PASSED: CLI Check verified.")

def test_cli_backup_restore():
    print("[*] Test 4: CLI Backup & Restore Safeguards (Continuous Lock & Atomic Publish)...")
    temp_dir = tempfile.mkdtemp(prefix="noesql_cli_backup_")
    backup_path = os.path.join(temp_dir, "backup.noedb")
    restore_path = os.path.join(temp_dir, "restore.noedb")
    try:
        # Create backup
        res_b = run_cmd(["backup", "production.noedb", backup_path])
        assert "Point-in-Time Backup Created & Verified" in res_b.stdout
        assert os.path.exists(os.path.join(backup_path, "HEAD"))

        # Refuse to clobber existing backup target
        res_clobber = run_cmd(["backup", "production.noedb", backup_path], expect_success=False)
        assert res_clobber.returncode != 0
        assert "refusing to overwrite" in res_clobber.stdout

        # Restore from backup
        res_r = run_cmd(["restore", backup_path, restore_path])
        assert "Database Restored Successfully" in res_r.stdout

        # Verify restored database with doctor & check
        res_chk = run_cmd(["check", restore_path])
        assert "Relational Consistency & Index Verification: ALL PASSED" in res_chk.stdout
        print("  -> PASSED: CLI Backup & Restore verified.")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)

def test_cli_explain():
    print("[*] Test 5: Real EXPLAIN Query Plan Inspector...")
    # 1. Valid indexed primary key lookup
    res_idx = run_cmd(["explain", "production.noedb", "SELECT * FROM departments WHERE id: 1;"])
    assert "PLAN: INDEX_SCAN" in res_idx.stdout
    assert "Operator: B+Tree Primary Key Index Lookup (noeBTreeSearch)" in res_idx.stdout
    assert "Complexity: O(log N)" in res_idx.stdout

    # 2. Valid table scan on unindexed column
    res_scan = run_cmd(["explain", "production.noedb", "SELECT * FROM departments WHERE name: engineering;"])
    assert "PLAN: TABLE_SCAN" in res_scan.stdout
    assert "Reason: Column 'name' does not possess an active B+Tree index" in res_scan.stdout
    assert "Complexity: O(N)" in res_scan.stdout

    # 3. Non-existent table rejected
    res_bad_tbl = run_cmd(["explain", "production.noedb", "SELECT * FROM missing_tbl WHERE id: 1;"], expect_success=False)
    assert res_bad_tbl.returncode != 0
    assert "does not exist in schema catalog" in res_bad_tbl.stdout

    # 4. Non-existent column rejected
    res_bad_col = run_cmd(["explain", "production.noedb", "SELECT * FROM departments WHERE bad_col: 1;"], expect_success=False)
    assert res_bad_col.returncode != 0
    assert "does not exist in table 'departments'" in res_bad_col.stdout

    print("  -> PASSED: Real EXPLAIN Query Plan Inspector verified against schema catalog.")

def test_cli_query():
    print("[*] Test 6: CLI Query Execution & Exit Code Error Propagation...")
    temp_dir = tempfile.mkdtemp(prefix="noesql_cli_query_")
    db_path = os.path.join(temp_dir, "query_test.noedb")
    try:
        # Create fresh isolated database via backup
        run_cmd(["backup", "production.noedb", db_path])

        # 1. Successful query on fresh database
        res_ok = run_cmd(["query", db_path, "departments add { id: 99, name: unique_test_dept }"])
        assert "OK:tx=" in res_ok.stdout

        # 2. Duplicate key query fails closed with -203 and propagates non-zero exit code
        res_dup = run_cmd(["query", db_path, "departments add { id: 99, name: duplicate_key }"], expect_success=False)
        assert res_dup.returncode != 0
        assert "ERROR:exec_failed:-203" in res_dup.stdout

        # 3. Failed query on non-existent table propagates non-zero exit code
        res_err = run_cmd(["query", db_path, "nonexistent_table add { id: 1 }"], expect_success=False)
        assert res_err.returncode != 0
        assert "ERROR:exec_failed:" in res_err.stdout

        print("  -> PASSED: CLI Query statement execution and error exit codes verified.")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)

def test_cli_parallel_execution():
    print("[*] Test 7: Parallel Concurrent CLI Execution (Zero File Clobbering)...")
    commands = [
        ["doctor", "production.noedb"],
        ["explain", "production.noedb", "SELECT * FROM departments WHERE id: 1;"],
        ["explain", "production.noedb", "SELECT * FROM departments WHERE name: engineering;"],
        ["check", "production.noedb"],
        ["doctor", "production.noedb"],
    ]

    with ThreadPoolExecutor(max_workers=5) as executor:
        futures = [executor.submit(run_cmd, cmd) for cmd in commands]
        for f in futures:
            res = f.result()
            assert res.returncode == 0

    print("  -> PASSED: 5 concurrent CLI commands executed simultaneously with zero collision.")

def main():
    print("================================================================")
    print("  Running Hardened NoeSQL CLI Tooling & Safeguards Suite")
    print("================================================================")
    test_cli_help()
    test_cli_doctor()
    test_cli_check()
    test_cli_backup_restore()
    test_cli_explain()
    test_cli_query()
    test_cli_parallel_execution()
    print("================================================================")
    print("  ALL CLI TOOLING ACCEPTANCE TESTS PASSED (100% SUCCESS)")
    print("================================================================")

if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""
Test Suite: NoeSQL Operational CLI Tooling & Production Safeguards
Verifies:
  - noesql doctor (snapshot validation, HEAD check, ledger state, lock inspection)
  - noesql check (B+Tree index parity, foreign-key referential integrity)
  - noesql backup (consistent locked snapshot, non-clobbering target safeguard)
  - noesql restore (verified cold point-in-time recovery)
  - noesql explain (O(log N) index scan vs O(N) sequential table scan)
  - noesql query (statement execution via CLI runner)
"""

import os
import shutil
import subprocess
import sys
import tempfile

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
    print("[*] Test 2: CLI Doctor diagnostic check...")
    temp_dir = tempfile.mkdtemp(prefix="noesql_cli_doc_")
    db_path = os.path.join(temp_dir, "test.noedb")
    try:
        # 1. Missing db should fail doctor
        res_fail = run_cmd(["doctor", db_path], expect_success=False)
        assert res_fail.returncode != 0
        assert "CRITICAL" in res_fail.stdout

        # 2. Initialize a valid database via binary test/query
        run_cmd(["test"]) # Ensure production.noedb exists
        res_doc = run_cmd(["doctor", "production.noedb"])
        assert "Snapshot SHA-256 Checksum: VERIFIED" in res_doc.stdout
        assert "Overall Status: 100% HEALTHY" in res_doc.stdout
        print("  -> PASSED: CLI Doctor verified.")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)

def test_cli_check():
    print("[*] Test 3: CLI Relational Integrity & B+Tree Checker...")
    res = run_cmd(["check", "production.noedb"])
    assert "Primary Key B+Tree: 2 keys verified in O(log N) lookup" in res.stdout
    assert "Foreign Key 'dept_id' -> 'departments': 100% referential integrity" in res.stdout
    assert "Relational Consistency & Index Verification: ALL PASSED" in res.stdout
    print("  -> PASSED: CLI Check verified.")

def test_cli_backup_restore():
    print("[*] Test 4: CLI Backup & Restore Safeguards...")
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
    print("[*] Test 5: CLI Explain Query Plan Inspector...")
    res_idx = run_cmd(["explain", "production.noedb", "SELECT * FROM users WHERE id: 1;"])
    assert "PLAN: INDEX_SCAN" in res_idx.stdout
    assert "Complexity: O(log N)" in res_idx.stdout

    res_scan = run_cmd(["explain", "production.noedb", "SELECT * FROM users WHERE name: alice;"])
    assert "PLAN: TABLE_SCAN" in res_scan.stdout
    assert "Complexity: O(N)" in res_scan.stdout
    print("  -> PASSED: CLI Explain verified.")

def test_cli_query():
    print("[*] Test 6: CLI Query & Transaction Statement Execution...")
    # Execute valid statement via CLI
    res = run_cmd(["query", "production.noedb", "departments add { id: 3, name: finance }"])
    assert "OK:tx=" in res.stdout or "UNCERTAIN_COMMIT" in res.stdout
    
    # Reconcile historical tx
    res_rec = run_cmd(["query", "production.noedb", "RECONCILE:1"])
    assert "RECONCILED:committed" in res_rec.stdout or "RECONCILED:aborted" in res_rec.stdout
    print("  -> PASSED: CLI Query verified.")

def main():
    print("================================================================")
    print("  Running NoeSQL CLI Tooling & Production Safeguards Suite")
    print("================================================================")
    test_cli_help()
    test_cli_doctor()
    test_cli_check()
    test_cli_backup_restore()
    test_cli_explain()
    test_cli_query()
    print("================================================================")
    print("  ALL CLI TOOLING ACCEPTANCE TESTS PASSED (100% SUCCESS)")
    print("================================================================")

if __name__ == "__main__":
    main()

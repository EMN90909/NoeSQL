import os
import sys
import time
import signal
import shutil
import subprocess

NOESQL_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
NOQERI_BIN = os.environ.get("NOQERI", os.path.abspath(os.path.join(NOESQL_DIR, "..", "Noqeri", "build", "noqeri")))

print("================================================================================")
print("  NOESQL REAL CRASH & FAULT INJECTION DURABILITY SUITE")
print("  Testing SIGKILL during writes, post-commit kills, and ledger recovery")
print("================================================================================")

DB_PATH = os.path.join(NOESQL_DIR, "tmp_crash_test.noedb")
LEDGER_PATH = os.path.join(NOESQL_DIR, "tmp_crash_ledger.txt")

def cleanup():
    if os.path.exists(DB_PATH):
        shutil.rmtree(DB_PATH, ignore_errors=True)
    if os.path.exists(DB_PATH + ".lck"):
        try: os.remove(DB_PATH + ".lck")
        except OSError: pass
    if os.path.exists(LEDGER_PATH):
        try: os.remove(LEDGER_PATH)
        except OSError: pass

cleanup()

# -----------------------------------------------------------------------------
# Test 1: SIGKILL Immediately Mid-Run (Abrupt Process Termination)
# -----------------------------------------------------------------------------
print("\n[CRASH TEST 1] SIGKILL Mid-Run Fault Injection & Cold Ledger Verification")

worker_code = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"

function main(): void {
    let dbPath: string = "tmp_crash_test.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("OPEN_FAILED") return }

    let txSchema: NoeWalTx
    noeWalEngineBegin(&engine, &txSchema)
    noeWalEngineExecute(&engine, &txSchema, "table accounts { id: int key auto, balance: int }")
    noeWalEngineCommit(&engine, &txSchema)

    let ledgerFd: isize = file_open_append("tmp_crash_ledger.txt")

    let i: usize = 0 as usize
    while i < 200 as usize {
        let tx: NoeWalTx
        noeWalEngineBegin(&engine, &tx)
        let val: i64 = ((i + 1 as usize) * 50 as usize) as i64
        let stmt: string = "accounts add { balance: " + intToString(val) + " }"
        noeWalEngineExecute(&engine, &tx, stmt)
        noeWalEngineCommit(&engine, &tx)

        // Write directly to verified disk ledger
        let logLine: string = intToString(val) + "\\n"
        let buf: [u8; 64]
        let bLen: usize = stringToBytes(logLine, &buf[0], 64 as usize)
        file_write_all(ledgerFd as usize, slice(&buf[0], bLen))
        file_sync(ledgerFd as usize)

        thread_sleep_millis(15 as i64)
        i = i + 1 as usize
    }
    file_close(ledgerFd as usize)
}
main()
"""
script_path = os.path.join(NOESQL_DIR, "tests", "tmp_crash_worker.noqeri")
with open(script_path, "w") as f:
    f.write(worker_code)

# Launch worker
p = subprocess.Popen([NOQERI_BIN, "run", script_path], cwd=NOESQL_DIR)

# Wait until at least 15 commits have been recorded and synced to disk ledger
while True:
    time.sleep(0.05)
    if os.path.exists(LEDGER_PATH):
        with open(LEDGER_PATH, "r") as f:
            lines = [line.strip() for line in f if line.strip()]
        if len(lines) >= 15:
            # Inject hard SIGKILL mid-flight!
            os.kill(p.pid, signal.SIGKILL)
            break
    if p.poll() is not None:
        break

p.communicate()

# Read the exact committed ledger
with open(LEDGER_PATH, "r") as f:
    acknowledged_commits = [int(line.strip()) for line in f if line.strip()]

print(f"Injected SIGKILL after acknowledging {len(acknowledged_commits)} transactions.")
assert len(acknowledged_commits) >= 15, "Did not reach target commits before kill"

# Now cold-reopen in a fresh reader process
reader_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_crash_test.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("REOPEN_FAILED:" + intToString(openRes as i64)) return }

    let tIdx: isize = noeFindTable(&engine.db, "accounts")
    if tIdx < 0 as isize { print("TABLE_NOT_FOUND") return }

    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("TOTAL_RECOVERED:" + intToString(t.rowCount as i64))

    let r: usize = 0 as usize
    while r < t.rowCount {
        let cell: *NoeRowCell = &t.cells[r * 16 as usize + 1 as usize]
        if !cell.isNull {
            print("RECOVERED_VAL:" + intToString(cell.intVal))
        }
        r = r + 1 as usize
    }
    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""
reader_path = os.path.join(NOESQL_DIR, "tests", "tmp_crash_reader.noqeri")
with open(reader_path, "w") as f:
    f.write(reader_code)

res = subprocess.run([NOQERI_BIN, "run", reader_path], cwd=NOESQL_DIR, capture_output=True, text=True)
if res.returncode != 0:
    print("Cold recovery failed:\n", res.stdout, res.stderr)
    sys.exit(1)

recovered_vals = []
for line in res.stdout.splitlines():
    if line.startswith("RECOVERED_VAL:"):
        recovered_vals.append(int(line.split(":")[1]))

print(f"Recovered {len(recovered_vals)} rows after hard SIGKILL.")
assert len(recovered_vals) >= len(acknowledged_commits) and recovered_vals[:len(acknowledged_commits)] == acknowledged_commits and len(recovered_vals) <= len(acknowledged_commits) + 1, f"Mismatch!\nAck: {acknowledged_commits}\nRec: {recovered_vals}"
print("=> PASS: 100% exact match between acknowledged transactions and cold recovered rows after SIGKILL!")

# -----------------------------------------------------------------------------
# Test 2: Resume New Transactions Immediately After Crash Recovery
# -----------------------------------------------------------------------------
print("\n[CRASH TEST 2] Verify Database Can Immediately Accept New Commits After Crash Recovery")
resume_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_crash_test.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("REOPEN_FAILED") return }

    let tx: NoeWalTx
    noeWalEngineBegin(&engine, &tx)
    noeWalEngineExecute(&engine, &tx, "accounts add { balance: 999999 }")
    noeWalEngineCommit(&engine, &tx)

    let tIdx: isize = noeFindTable(&engine.db, "accounts")
    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("RESUME_ROW_COUNT:" + intToString(t.rowCount as i64))

    noeWalEngineClose(&engine)
}
main()
"""
resume_path = os.path.join(NOESQL_DIR, "tests", "tmp_crash_resume.noqeri")
with open(resume_path, "w") as f:
    f.write(resume_code)

res_resume = subprocess.run([NOQERI_BIN, "run", resume_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Output:", res_resume.stdout.strip())
expected_count = len(recovered_vals) + 1
assert f"RESUME_ROW_COUNT:{expected_count}" in res_resume.stdout, "Failed to commit new transaction after crash recovery"
print("=> PASS: Successfully resumed writing and committed new transaction after crash recovery.")

# Clean up
cleanup()
if os.path.exists(script_path): os.remove(script_path)
if os.path.exists(reader_path): os.remove(reader_path)
if os.path.exists(resume_path): os.remove(resume_path)

# -----------------------------------------------------------------------------
# Test 3: SIGKILL Inside Real noeWalEngineCheckpoint Function
# -----------------------------------------------------------------------------
print("\n[CRASH TEST 3] SIGKILL Inside Real noeWalEngineCheckpoint Function (After Snapshot Publication)")
CKPT_DB = os.path.join(NOESQL_DIR, "tmp_ckpt_crash.noedb")
if os.path.exists(CKPT_DB):
    shutil.rmtree(CKPT_DB, ignore_errors=True)
if os.path.exists(CKPT_DB + ".lck"):
    try: os.remove(CKPT_DB + ".lck")
    except OSError: pass

ckpt_worker_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_ckpt_crash.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("OPEN_FAILED") return }

    let txSchema: NoeWalTx
    noeWalEngineBegin(&engine, &txSchema)
    noeWalEngineExecute(&engine, &txSchema, "table records { id: int key auto, val: int }")
    noeWalEngineCommit(&engine, &txSchema)

    // Commit 10 sequential transactions
    let i: usize = 0 as usize
    while i < 10 as usize {
        let tx: NoeWalTx
        noeWalEngineBegin(&engine, &tx)
        let val: i64 = ((i + 1 as usize) * 100 as usize) as i64
        let stmt: string = "records add { val: " + intToString(val) + " }"
        noeWalEngineExecute(&engine, &tx, stmt)
        noeWalEngineCommit(&engine, &tx)
        i = i + 1 as usize
    }

    // Call REAL noeWalEngineCheckpoint function with fault-injection environment active!
    // Inside noeWalEngineCheckpoint, it will publish snapshot, update HEAD, and trigger fault termination before WAL truncation.
    noeWalEngineCheckpoint(&engine)
}
main()
"""
ckpt_worker_path = os.path.join(NOESQL_DIR, "tests", "tmp_ckpt_crash_worker.noqeri")
with open(ckpt_worker_path, "w") as f:
    f.write(ckpt_worker_code)

os.makedirs(CKPT_DB, exist_ok=True)
ckpt_test_flag = os.path.join(CKPT_DB, ".noesql_testing_faults_enabled")
open(ckpt_test_flag, "w").close()
ckpt_kill_flag = os.path.join(CKPT_DB, "FAULT_CHECKPOINT_KILL.flag")
open(ckpt_kill_flag, "w").close()

p_ckpt = subprocess.Popen([NOQERI_BIN, "run", ckpt_worker_path], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

# Wait for engine to finish snapshot publication and signal test controller
marker_file = os.path.join(CKPT_DB, "SNAPSHOT_PUBLISHED.marker")
t_start = time.time()
while not os.path.exists(marker_file) and time.time() - t_start < 10.0:
    time.sleep(0.01)

assert os.path.exists(marker_file), "Engine did not create SNAPSHOT_PUBLISHED.marker within timeout"

# Inject genuine OS SIGKILL from external test controller!
os.kill(p_ckpt.pid, signal.SIGKILL)
stdout_ckpt, stderr_ckpt = p_ckpt.communicate()

# Assert OS process termination by SIGKILL (-9 or 137)
print(f"Checkpoint process returncode: {p_ckpt.returncode}")
assert p_ckpt.returncode == -signal.SIGKILL or p_ckpt.returncode == 128 + signal.SIGKILL, (
    f"Expected process termination by OS SIGKILL, got returncode: {p_ckpt.returncode}"
)
print("=> VERIFIED: Process terminated by genuine OS SIGKILL (signal 9) after snapshot publication!")

if os.path.exists(ckpt_kill_flag): os.remove(ckpt_kill_flag)
if os.path.exists(marker_file): os.remove(marker_file)
if os.path.exists(ckpt_test_flag): os.remove(ckpt_test_flag)

# Verify WAL.log still exists and has not been truncated
wal_file = os.path.join(CKPT_DB, "WAL.log")
assert os.path.exists(wal_file), "WAL.log must exist on disk"
wal_sz = os.path.getsize(wal_file)
assert wal_sz > 0, f"WAL.log must be non-empty (actual: {wal_sz} bytes), confirming crash occurred before truncation"

# Now cold-reopen in a fresh process:
# Checkpoint watermark MUST prevent replaying already-checkpointed transactions!
ckpt_reader_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_ckpt_crash.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("REOPEN_FAILED:" + intToString(openRes as i64)) return }

    let tIdx: isize = noeFindTable(&engine.db, "records")
    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("REOPEN_ROW_COUNT:" + intToString(t.rowCount as i64))

    let r: usize = 0 as usize
    while r < t.rowCount {
        let cell: *NoeRowCell = &t.cells[r * 16 as usize + 1 as usize]
        if !cell.isNull {
            print("REOPEN_VAL:" + intToString(cell.intVal))
        }
        r = r + 1 as usize
    }

    // Insert 11th record to verify write operations continue properly
    let tx11: NoeWalTx
    noeWalEngineBegin(&engine, &tx11)
    noeWalEngineExecute(&engine, &tx11, "records add { val: 1100 }")
    noeWalEngineCommit(&engine, &tx11)

    print("AFTER_ADD_ROW_COUNT:" + intToString(t.rowCount as i64))
    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""
ckpt_reader_path = os.path.join(NOESQL_DIR, "tests", "tmp_ckpt_crash_reader.noqeri")
with open(ckpt_reader_path, "w") as f:
    f.write(ckpt_reader_code)

res_ckpt = subprocess.run([NOQERI_BIN, "run", ckpt_reader_path], cwd=NOESQL_DIR, capture_output=True, text=True)
if res_ckpt.returncode != 0:
    print("Checkpoint crash reader failed:\n", res_ckpt.stdout, res_ckpt.stderr)
    sys.exit(1)

recovered_ckpt_vals = []
for line in res_ckpt.stdout.splitlines():
    if line.startswith("REOPEN_VAL:"):
        recovered_ckpt_vals.append(int(line.split(":")[1]))

expected_ckpt_vals = [(i + 1) * 100 for i in range(10)]
print(f"Recovered rows: {len(recovered_ckpt_vals)} (Expected: 10)")
assert "REOPEN_ROW_COUNT:10" in res_ckpt.stdout, f"Expected exactly 10 rows without duplicates, got:\n{res_ckpt.stdout}"
assert recovered_ckpt_vals == expected_ckpt_vals, f"Value mismatch!\nExpected: {expected_ckpt_vals}\nGot: {recovered_ckpt_vals}"
assert "AFTER_ADD_ROW_COUNT:11" in res_ckpt.stdout, "Failed to resume writes and append 11th record"

# Second cold reopen to verify that 11th row also persists cleanly!
res_ckpt2 = subprocess.run([NOQERI_BIN, "run", ckpt_reader_path], cwd=NOESQL_DIR, capture_output=True, text=True)
assert "REOPEN_ROW_COUNT:11" in res_ckpt2.stdout, "Failed second cold reopen after resume"
print("=> PASS: 100% duplicate-free recovery after genuine OS SIGKILL crash inside real noeWalEngineCheckpoint!")

# -----------------------------------------------------------------------------
# Test 4: Physical Torn-Tail Repair & Checkpoint Fail-Closed Fault Injections
# -----------------------------------------------------------------------------
print("\n[FAULT INJECTION TEST 4] Physical Torn-Tail Repair & Checkpoint Fail-Closed Fault Injections")
FAIL_DB = os.path.join(NOESQL_DIR, "tmp_fail_repair.noedb")
if os.path.exists(FAIL_DB):
    shutil.rmtree(FAIL_DB, ignore_errors=True)
if os.path.exists(FAIL_DB + ".lck"):
    try: os.remove(FAIL_DB + ".lck")
    except OSError: pass

os.makedirs(FAIL_DB, exist_ok=True)
fail_test_flag = os.path.join(FAIL_DB, ".noesql_testing_faults_enabled")
open(fail_test_flag, "w").close()

init_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_fail_repair.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)
    let tx: NoeWalTx
    noeWalEngineBegin(&engine, &tx)
    noeWalEngineExecute(&engine, &tx, "table t { id: int key auto, val: int }")
    noeWalEngineCommit(&engine, &tx)
    noeWalEngineClose(&engine)
}
main()
"""
init_path = os.path.join(NOESQL_DIR, "tests", "tmp_fail_repair_init.noqeri")
with open(init_path, "w") as f:
    f.write(init_code)

subprocess.run([NOQERI_BIN, "run", init_path], cwd=NOESQL_DIR, check=True)

# Corrupt WAL with 25 bytes of torn tail
fail_wal = os.path.join(FAIL_DB, "WAL.log")
with open(fail_wal, "ab") as f:
    f.write(b"CORRUPTED_INCOMPLETE_TAIL")

test_fail_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_fail_repair.noedb"
    let engine: NoeWalEngine
    let res: isize = noeWalEngineOpen(&engine, dbPath)
    print("FAIL_CLOSED_RES:" + intToString(res as i64))
}
main()
"""
test_fail_path = os.path.join(NOESQL_DIR, "tests", "tmp_fail_repair_test.noqeri")
with open(test_fail_path, "w") as f:
    f.write(test_fail_code)

# 4A. Injected file_truncate failure -> returns -11
trunc_flag = os.path.join(FAIL_DB, "FAULT_TRUNCATE_FAIL.flag")
open(trunc_flag, "w").close()
res_trunc = subprocess.run([NOQERI_BIN, "run", test_fail_path], cwd=NOESQL_DIR, capture_output=True, text=True)
if os.path.exists(trunc_flag): os.remove(trunc_flag)
print("Truncate fault output:", res_trunc.stdout.strip())
assert "FAIL_CLOSED_RES:-11" in res_trunc.stdout, f"Expected -11 truncate failure, got: {res_trunc.stdout}"
print("=> PASS 4A: Injected file_truncate failure fails closed with -11.")

# 4B. Injected file_sync failure during torn-tail repair -> returns -12
sync_flag = os.path.join(FAIL_DB, "FAULT_SYNC_FAIL.flag")
open(sync_flag, "w").close()
res_sync = subprocess.run([NOQERI_BIN, "run", test_fail_path], cwd=NOESQL_DIR, capture_output=True, text=True)
if os.path.exists(sync_flag): os.remove(sync_flag)
print("Sync fault output:", res_sync.stdout.strip())
assert "FAIL_CLOSED_RES:-12" in res_sync.stdout, f"Expected -12 sync failure, got: {res_sync.stdout}"
print("=> PASS 4B: Injected file_sync repair failure fails closed with -12.")

# 4C. Injected checkpoint WAL sync failure -> returns -13
ckpt_fail_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_fail_repair.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("REOPEN_FOR_CKPT_FAILED:" + intToString(openRes as i64)) return }
    let cpRes: isize = noeWalEngineCheckpoint(&engine)
    print("CKPT_SYNC_FAIL_RES:" + intToString(cpRes as i64))
    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""
ckpt_fail_path = os.path.join(NOESQL_DIR, "tests", "tmp_fail_ckpt_test.noqeri")
with open(ckpt_fail_path, "w") as f:
    f.write(ckpt_fail_code)

ckpt_sync_flag = os.path.join(FAIL_DB, "FAULT_CHECKPOINT_SYNC_FAIL.flag")
open(ckpt_sync_flag, "w").close()
res_ckpt_sync = subprocess.run([NOQERI_BIN, "run", ckpt_fail_path], cwd=NOESQL_DIR, capture_output=True, text=True)
if os.path.exists(ckpt_sync_flag): os.remove(ckpt_sync_flag)
print("Checkpoint sync fault output:", res_ckpt_sync.stdout.strip())
assert "CKPT_SYNC_FAIL_RES:-13" in res_ckpt_sync.stdout, f"Expected -13 checkpoint sync failure, got: {res_ckpt_sync.stdout}"
print("=> PASS 4C: Injected checkpoint file_sync failure fails closed with -13.")

# 4D. Blocked file write permission (chmod 0444) -> returns -10
with open(fail_wal, "ab") as f:
    f.write(b"ANOTHER_TORN_TAIL_FOR_PERM")
os.chmod(fail_wal, 0o444)
res_perm = subprocess.run([NOQERI_BIN, "run", test_fail_path], cwd=NOESQL_DIR, capture_output=True, text=True)
os.chmod(fail_wal, 0o644)  # Restore permissions
print("Permission fault output:", res_perm.stdout.strip())
assert "FAIL_CLOSED_RES:-10" in res_perm.stdout, f"Expected -10 permission failure, got: {res_perm.stdout}"
print("=> PASS 4D: Blocked WAL write permissions fails closed with -10.")

# 4E. Unreadable WAL file (chmod 0000) -> returns -14
os.chmod(fail_wal, 0o000)
res_unreadable = subprocess.run([NOQERI_BIN, "run", test_fail_path], cwd=NOESQL_DIR, capture_output=True, text=True)
os.chmod(fail_wal, 0o644)  # Restore permissions
print("Unreadable WAL output:", res_unreadable.stdout.strip())
assert "FAIL_CLOSED_RES:-14" in res_unreadable.stdout, f"Expected -14 unreadable WAL failure, got: {res_unreadable.stdout}"
print("=> PASS 4E: Unreadable existing WAL.log fails closed with -14.")

# 4F. Injected checkpoint cleanup sync failure -> returns -16
# Reopen without faults to clean torn tail, then insert and checkpoint to advance base snapshot lastTxId
res_clean = subprocess.run([NOQERI_BIN, "run", test_fail_path], cwd=NOESQL_DIR, capture_output=True, text=True)
assert "FAIL_CLOSED_RES:0" in res_clean.stdout, f"Expected 0 for clean repair, got: {res_clean.stdout}"

# Setup interrupted checkpoint scenario: snapshot has lastTxId=1, and WAL has lastTxId=1
interrupted_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_fail_repair.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("OPEN_FAILED:" + intToString(openRes as i64)) return }
    let tx: NoeWalTx
    noeWalEngineBegin(&engine, &tx)
    noeWalEngineExecute(&engine, &tx, "t add { val: 42 }")
    noeWalEngineCommit(&engine, &tx)
    // Save snapshot with lastTxId=1
    engine.db.lastTxId = engine.lastTxId
    noeDbSaveV3(&engine.db, dbPath)
    // Don't truncate WAL, just close handles to simulate crash right before WAL truncation!
    file_close(engine.walFd)
    file_close(engine.lockFd)
    print("INTERRUPTED_SETUP_DONE")
}
main()
"""
interrupted_path = os.path.join(NOESQL_DIR, "tests", "tmp_interrupted_setup.noqeri")
with open(interrupted_path, "w") as f:
    f.write(interrupted_code)

res_setup = subprocess.run([NOQERI_BIN, "run", interrupted_path], cwd=NOESQL_DIR, capture_output=True, text=True)
assert "INTERRUPTED_SETUP_DONE" in res_setup.stdout, f"Setup failed: {res_setup.stdout}"

# Now opening with FAULT_CLEANUP_SYNC_FAIL.flag must fail closed with -16
cleanup_sync_flag = os.path.join(FAIL_DB, "FAULT_CLEANUP_SYNC_FAIL.flag")
open(cleanup_sync_flag, "w").close()
res_cleanup_fail = subprocess.run([NOQERI_BIN, "run", test_fail_path], cwd=NOESQL_DIR, capture_output=True, text=True)
if os.path.exists(cleanup_sync_flag): os.remove(cleanup_sync_flag)
print("Cleanup sync fault output:", res_cleanup_fail.stdout.strip())
assert "FAIL_CLOSED_RES:-16" in res_cleanup_fail.stdout, f"Expected -16 cleanup sync failure, got: {res_cleanup_fail.stdout}"
print("=> PASS 4F: Injected checkpoint cleanup file_sync failure fails closed with -16.")

# Clean up temporary test files
if os.path.exists(ckpt_worker_path): os.remove(ckpt_worker_path)
if os.path.exists(ckpt_reader_path): os.remove(ckpt_reader_path)
if os.path.exists(init_path): os.remove(init_path)
if os.path.exists(test_fail_path): os.remove(test_fail_path)
if os.path.exists(ckpt_fail_path): os.remove(ckpt_fail_path)
if os.path.exists(interrupted_path): os.remove(interrupted_path)
if os.path.exists(CKPT_DB): shutil.rmtree(CKPT_DB, ignore_errors=True)
if os.path.exists(FAIL_DB): shutil.rmtree(FAIL_DB, ignore_errors=True)
if os.path.exists(FAIL_DB + ".lck"):
    try: os.remove(FAIL_DB + ".lck")
    except OSError: pass

# -----------------------------------------------------------------------------
# Test 5: Decoupled Commit Durability from Maintenance Status (P0 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 5] Commit Durability Decoupled from Automatic Checkpoint Maintenance Failure")
MAINT_DB = os.path.join(NOESQL_DIR, "tmp_maint_fail.noedb")
if os.path.exists(MAINT_DB):
    shutil.rmtree(MAINT_DB, ignore_errors=True)
if os.path.exists(MAINT_DB + ".lck"):
    try: os.remove(MAINT_DB + ".lck")
    except OSError: pass

maint_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_maint_fail.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("OPEN_FAILED") return }

    let txSchema: NoeWalTx
    noeWalEngineBegin(&engine, &txSchema)
    noeWalEngineExecute(&engine, &txSchema, "table items { id: int key auto, val: int }")
    noeWalEngineCommit(&engine, &txSchema)

    // Force checkpoint threshold low so next commit triggers auto-checkpoint
    engine.checkpointThreshold = 10 as usize

    // Inject fault into checkpoint sync truncation
    let testFlag: isize = file_open_create(dbPath + "/.noesql_testing_faults_enabled")
    if file_handle_valid(testFlag) { file_close(testFlag as usize) }
    let syncFlag: isize = file_open_create(dbPath + "/FAULT_CHECKPOINT_SYNC_FAIL.flag")
    if file_handle_valid(syncFlag) { file_close(syncFlag as usize) }

    let tx: NoeWalTx
    noeWalEngineBegin(&engine, &tx)
    noeWalEngineExecute(&engine, &tx, "items add { val: 7777 }")
    let cRes: isize = noeWalEngineCommit(&engine, &tx)
    print("COMMIT_RES:" + intToString(cRes as i64))
    print("MAINT_STATUS:" + intToString(engine.lastMaintenanceStatus as i64))
    if !engine.walOpen {
        print("ENGINE_CLOSED_ON_MAINT_FAIL")
    }

    // Attempting another write on the closed engine must fail with -1
    let tx2: NoeWalTx
    let bRes2: isize = noeWalEngineBegin(&engine, &tx2)
    print("BEGIN2_RES:" + intToString(bRes2 as i64))
}
main()
"""
maint_path = os.path.join(NOESQL_DIR, "tests", "tmp_maint_test.noqeri")
with open(maint_path, "w") as f:
    f.write(maint_code)

res_maint = subprocess.run([NOQERI_BIN, "run", maint_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Maintenance test output:\n", res_maint.stdout.strip())
assert "COMMIT_RES:0" in res_maint.stdout, "Durable commit was reported as an error due to maintenance failure!"
assert "MAINT_STATUS:-13" in res_maint.stdout, "Maintenance status was not tracked!"
assert "ENGINE_CLOSED_ON_MAINT_FAIL" in res_maint.stdout, "Engine remained open after failed checkpoint truncation!"
assert "BEGIN2_RES:-1" in res_maint.stdout, "Subsequent begin on degraded engine was not rejected!"

# Now cold-reopen in a fresh process: verify the committed record val: 7777 IS recovered from WAL!
maint_reader_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_maint_fail.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("REOPEN_FAILED:" + intToString(openRes as i64)) return }

    let tIdx: isize = noeFindTable(&engine.db, "items")
    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("TOTAL_RECOVERED:" + intToString(t.rowCount as i64))

    let r: usize = 0 as usize
    while r < t.rowCount {
        let cell: *NoeRowCell = &t.cells[r * 16 as usize + 1 as usize]
        if !cell.isNull {
            print("RECOVERED_VAL:" + intToString(cell.intVal))
        }
        r = r + 1 as usize
    }
    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""
maint_reader_path = os.path.join(NOESQL_DIR, "tests", "tmp_maint_reader.noqeri")
with open(maint_reader_path, "w") as f:
    f.write(maint_reader_code)

# Remove the fault flag before cold reopening
if os.path.exists(os.path.join(MAINT_DB, "FAULT_CHECKPOINT_SYNC_FAIL.flag")):
    os.remove(os.path.join(MAINT_DB, "FAULT_CHECKPOINT_SYNC_FAIL.flag"))
if os.path.exists(os.path.join(MAINT_DB, ".noesql_testing_faults_enabled")):
    os.remove(os.path.join(MAINT_DB, ".noesql_testing_faults_enabled"))

res_maint_reader = subprocess.run([NOQERI_BIN, "run", maint_reader_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Maintenance reader output:\n", res_maint_reader.stdout.strip())
assert "TOTAL_RECOVERED:1" in res_maint_reader.stdout, f"Expected 1 recovered item, got:\n{res_maint_reader.stdout}"
assert "RECOVERED_VAL:7777" in res_maint_reader.stdout, f"Expected val 7777, got:\n{res_maint_reader.stdout}"
print("=> PASS: Transaction commit durability strictly decoupled from maintenance failure; zero false commit errors!")

# Clean up
if os.path.exists(maint_path): os.remove(maint_path)
if os.path.exists(maint_reader_path): os.remove(maint_reader_path)
if os.path.exists(MAINT_DB): shutil.rmtree(MAINT_DB, ignore_errors=True)
if os.path.exists(MAINT_DB + ".lck"):
    try: os.remove(MAINT_DB + ".lck")
    except OSError: pass

# -----------------------------------------------------------------------------
# Test 6: Failed Commit Auto-Rollback & In-Memory Isolation (P0 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 6] Failed Commit Automatically Reverts In-Memory State")
FAIL_COMMIT_DB = os.path.join(NOESQL_DIR, "tmp_fail_commit.noedb")
if os.path.exists(FAIL_COMMIT_DB):
    shutil.rmtree(FAIL_COMMIT_DB, ignore_errors=True)

commit_fail_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_fail_commit.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("OPEN_FAILED") return }

    let tx1: NoeWalTx
    noeWalEngineBegin(&engine, &tx1)
    noeWalEngineExecute(&engine, &tx1, "table balances { id: int key auto, amount: int }")
    noeWalEngineExecute(&engine, &tx1, "balances add { amount: 100 }")
    let c1: isize = noeWalEngineCommit(&engine, &tx1)
    if c1 != 0 as isize { print("COMMIT1_FAILED") return }

    // Begin tx2, execute an insert that modifies in-memory state
    let tx2: NoeWalTx
    noeWalEngineBegin(&engine, &tx2)
    noeWalEngineExecute(&engine, &tx2, "balances add { amount: 9999 }")

    // In-memory table now has 2 rows before commit
    let tIdx: isize = noeFindTable(&engine.db, "balances")
    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("PRE_COMMIT_FAIL_ROWS:" + intToString(t.rowCount as i64))

    // Close WAL fd to force file_write_all or sync failure during commit
    file_close(engine.walFd)
    engine.walFd = 99999 as usize // Invalid fd

    // Attempt commit: must fail and automatically rollback memory!
    let c2: isize = noeWalEngineCommit(&engine, &tx2)
    print("FAILED_COMMIT_RES:" + intToString(c2 as i64))

    // In-memory table row count must have been reverted to 1!
    print("POST_REVERT_ROWS:" + intToString(t.rowCount as i64))
    let cell: *NoeRowCell = &t.cells[0 * 16 as usize + 1 as usize]
    print("SURVIVING_VAL:" + intToString(cell.intVal))

    noeWalEngineClose(&engine)
}
main()
"""
commit_fail_path = os.path.join(NOESQL_DIR, "tests", "tmp_fail_commit.noqeri")
with open(commit_fail_path, "w") as f:
    f.write(commit_fail_code)

res_cfail = subprocess.run([NOQERI_BIN, "run", commit_fail_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Commit fail test output:\n", res_cfail.stdout.strip())
assert "PRE_COMMIT_FAIL_ROWS:2" in res_cfail.stdout, "Pre-commit rows unexpected"
assert "POST_REVERT_ROWS:1" in res_cfail.stdout, "In-memory state was NOT reverted after failed commit!"
assert "SURVIVING_VAL:100" in res_cfail.stdout, "Surviving value corrupted"
print("=> PASS: Failed commit automatically reverted in-memory state to clean committed snapshot!")

# -----------------------------------------------------------------------------
# Test 7: Missing WAL Fail-Closed Detection (P1 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 7] Missing WAL Log Fails Closed When Committed Offset > 0")
MISSING_WAL_DB = os.path.join(NOESQL_DIR, "tmp_missing_wal.noedb")
if os.path.exists(MISSING_WAL_DB):
    shutil.rmtree(MISSING_WAL_DB, ignore_errors=True)

missing_init_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_missing_wal.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)
    let tx: NoeWalTx
    noeWalEngineBegin(&engine, &tx)
    noeWalEngineExecute(&engine, &tx, "table test_tbl { id: int key auto, val: int }")
    noeWalEngineExecute(&engine, &tx, "test_tbl add { val: 555 }")
    noeWalEngineCommit(&engine, &tx)
    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""
missing_init_path = os.path.join(NOESQL_DIR, "tests", "tmp_missing_init.noqeri")
with open(missing_init_path, "w") as f:
    f.write(missing_init_code)

subprocess.run([NOQERI_BIN, "run", missing_init_path], cwd=NOESQL_DIR, check=True)

# Delete WAL.log while committed transactions exist
wal_to_delete = os.path.join(MISSING_WAL_DB, "WAL.log")
assert os.path.exists(wal_to_delete), "WAL.log must exist before deletion"
os.remove(wal_to_delete)

missing_test_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_missing_wal.noedb"
    let engine: NoeWalEngine
    noeWalEngineInit(&engine)
    engine.dbPath = dbPath
    engine.walCommittedOffset = 500 as usize // Engine expects committed transactions in WAL
    engine.baseGeneration = 1 as u64

    let refRes: isize = noeWalEngineRefresh(&engine)
    print("REFRESH_MISSING_WAL:" + intToString(refRes as i64))

    let revRes: isize = noeWalEngineRevert(&engine)
    print("REVERT_MISSING_WAL:" + intToString(revRes as i64))
}
main()
"""
missing_test_path = os.path.join(NOESQL_DIR, "tests", "tmp_missing_test.noqeri")
with open(missing_test_path, "w") as f:
    f.write(missing_test_code)

res_missing = subprocess.run([NOQERI_BIN, "run", missing_test_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Missing WAL test output:\n", res_missing.stdout.strip())
assert "REFRESH_MISSING_WAL:-17" in res_missing.stdout, f"Expected -17 missing WAL in refresh, got: {res_missing.stdout}"
assert "REVERT_MISSING_WAL:-17" in res_missing.stdout, f"Expected -17 missing WAL in revert, got: {res_missing.stdout}"
print("=> PASS: Unexpectedly missing WAL fails closed with -17 in refresh and revert!")

# -----------------------------------------------------------------------------
# Test 8: Degraded Engine Resource Cleanup (P1 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 8] Degraded Engine Releases All File and Lock Descriptors on Close")
degraded_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_fail_commit.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)

    // Mark engine degraded
    engine.walOpen = false

    // Close must clean up engine.lockFd and engine.walFd even when walOpen == false!
    let cRes: isize = noeWalEngineClose(&engine)
    print("CLOSE_RES:" + intToString(cRes as i64))
    print("LOCK_FD_CLEARED:" + intToString(engine.lockFd as i64))
    print("WAL_FD_CLEARED:" + intToString(engine.walFd as i64))

    // Verify another engine instance can immediately open and lock the database
    let engine2: NoeWalEngine
    let open2: isize = noeWalEngineOpen(&engine2, dbPath)
    print("REOPEN_AFTER_DEGRADED_CLOSE:" + intToString(open2 as i64))
    noeWalEngineClose(&engine2)
}
main()
"""
degraded_path = os.path.join(NOESQL_DIR, "tests", "tmp_degraded_test.noqeri")
with open(degraded_path, "w") as f:
    f.write(degraded_code)

res_degraded = subprocess.run([NOQERI_BIN, "run", degraded_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Degraded cleanup output:\n", res_degraded.stdout.strip())
assert "CLOSE_RES:0" in res_degraded.stdout
assert "LOCK_FD_CLEARED:0" in res_degraded.stdout
assert "WAL_FD_CLEARED:0" in res_degraded.stdout
assert "REOPEN_AFTER_DEGRADED_CLOSE:0" in res_degraded.stdout
print("=> PASS: Degraded engine completely releases locks and descriptors on close without hanging!")

# -----------------------------------------------------------------------------
# Test 9: Active Unlinked WAL Detected on Open Descriptor (P0 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 9] Active Unlinked WAL Fails Closed with -17 and Quarantines Engine")
ACTIVE_UNLINK_DB = os.path.join(NOESQL_DIR, "tmp_active_unlink.noedb")
if os.path.exists(ACTIVE_UNLINK_DB): shutil.rmtree(ACTIVE_UNLINK_DB, ignore_errors=True)
if os.path.exists(ACTIVE_UNLINK_DB + ".lck"):
    try: os.remove(ACTIVE_UNLINK_DB + ".lck")
    except OSError: pass

active_unlink_code = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"

function main(): void {
    let dbPath: string = "tmp_active_unlink.noedb"
    let engine: NoeWalEngine
    let oRes: isize = noeWalEngineOpen(&engine, dbPath)
    if oRes != 0 as isize { print("OPEN_FAILED:" + intToString(oRes as i64)) return }

    let tx1: NoeWalTx
    noeWalEngineBegin(&engine, &tx1)
    noeWalEngineExecute(&engine, &tx1, "table items { id: int key auto, val: int }")
    noeWalEngineExecute(&engine, &tx1, "items add { val: 100 }")
    noeWalEngineCommit(&engine, &tx1)

    // Signal orchestrator that engine is open and holding active walFd
    let sig: isize = file_open_create(dbPath + "/READY_FOR_UNLINK.flag")
    if file_handle_valid(sig) { file_close(sig as usize) }

    // Wait until orchestrator unlinks WAL.log
    let waitIter: usize = 0 as usize
    while waitIter < 500 as usize {
        if fs_exists(dbPath + "/UNLINK_DONE.flag") { break }
        thread_sleep_millis(10 as i64)
        waitIter = waitIter + 1 as usize
    }

    // Attempt next transaction with live open descriptor on unlinked file
    let tx2: NoeWalTx
    let beginRes: isize = noeWalEngineBegin(&engine, &tx2)
    print("BEGIN_ON_UNLINKED_WAL:" + intToString(beginRes as i64))
    if engine.walOpen {
        print("ENGINE_QUARANTINED:false")
    } else {
        print("ENGINE_QUARANTINED:true")
    }

    noeWalEngineClose(&engine)
}
main()
"""
unlink_script_path = os.path.join(NOESQL_DIR, "tests", "tmp_active_unlink.noqeri")
with open(unlink_script_path, "w") as f:
    f.write(active_unlink_code)

p_unlink = subprocess.Popen([NOQERI_BIN, "run", unlink_script_path], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

# Wait for READY_FOR_UNLINK.flag
ready_flag = os.path.join(ACTIVE_UNLINK_DB, "READY_FOR_UNLINK.flag")
while not os.path.exists(ready_flag):
    time.sleep(0.01)
    if p_unlink.poll() is not None:
        break

# Unlink WAL.log while the process holds the open descriptor
wal_file = os.path.join(ACTIVE_UNLINK_DB, "WAL.log")
assert os.path.exists(wal_file), "WAL.log must exist before unlink"
os.remove(wal_file)

# Signal worker that unlink is done
with open(os.path.join(ACTIVE_UNLINK_DB, "UNLINK_DONE.flag"), "w") as f:
    f.write("done")

stdout_u, stderr_u = p_unlink.communicate(timeout=10)
print("Active unlink test output:\n", stdout_u.strip())
assert "BEGIN_ON_UNLINKED_WAL:-17" in stdout_u, f"Expected -17 on unlinked WAL begin, got: {stdout_u}"
assert "ENGINE_QUARANTINED:true" in stdout_u, f"Expected engine to be quarantined, got: {stdout_u}"
print("=> PASS: Live unlinked WAL fails closed with -17 and quarantines engine!")

# -----------------------------------------------------------------------------
# Test 10: Successful COMMIT Write Followed by Failed Fsync (P0 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 10] Ambiguous Outcome: Commit Frame Written But Fsync Fails (-18 Quarantine)")
COMMIT_SYNC_DB = os.path.join(NOESQL_DIR, "tmp_commit_sync.noedb")
if os.path.exists(COMMIT_SYNC_DB): shutil.rmtree(COMMIT_SYNC_DB, ignore_errors=True)
if os.path.exists(COMMIT_SYNC_DB + ".lck"):
    try: os.remove(COMMIT_SYNC_DB + ".lck")
    except OSError: pass

commit_sync_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_commit_sync.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)

    let tx1: NoeWalTx
    noeWalEngineBegin(&engine, &tx1)
    noeWalEngineExecute(&engine, &tx1, "table accounts { id: int key auto, balance: int }")
    noeWalEngineExecute(&engine, &tx1, "accounts add { balance: 100 }")
    noeWalEngineCommit(&engine, &tx1)

    // Enable testing fault injection for failed commit fsync
    let flag1: isize = file_open_create(dbPath + "/.noesql_testing_faults_enabled")
    if file_handle_valid(flag1) { file_close(flag1 as usize) }
    let flag2: isize = file_open_create(dbPath + "/FAULT_COMMIT_SYNC_FAIL.flag")
    if file_handle_valid(flag2) { file_close(flag2 as usize) }

    let tx2: NoeWalTx
    noeWalEngineBegin(&engine, &tx2)
    noeWalEngineExecute(&engine, &tx2, "accounts add { balance: 200 }")
    let cRes: isize = noeWalEngineCommit(&engine, &tx2)
    print("COMMIT_SYNC_FAIL_RES:" + intToString(cRes as i64))
    if engine.walOpen {
        print("WRITER_QUARANTINED:false")
    } else {
        print("WRITER_QUARANTINED:true")
    }

    // Subsequent operation on quarantined engine must fail
    let tx3: NoeWalTx
    let b3: isize = noeWalEngineBegin(&engine, &tx3)
    print("SUBSEQUENT_BEGIN_REJECTED:" + intToString(b3 as i64))

    noeWalEngineClose(&engine)
}
main()
"""
commit_sync_path = os.path.join(NOESQL_DIR, "tests", "tmp_commit_sync.noqeri")
with open(commit_sync_path, "w") as f:
    f.write(commit_sync_code)

res_csync = subprocess.run([NOQERI_BIN, "run", commit_sync_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Commit fsync fail output:\n", res_csync.stdout.strip())
assert "COMMIT_SYNC_FAIL_RES:-18" in res_csync.stdout, f"Expected -18, got: {res_csync.stdout}"
assert "WRITER_QUARANTINED:true" in res_csync.stdout, f"Expected writer quarantined, got: {res_csync.stdout}"
assert "SUBSEQUENT_BEGIN_REJECTED:-1" in res_csync.stdout, f"Expected -1 on subsequent begin, got: {res_csync.stdout}"

# Now verify cold recovery: remove fault flags and reopen database
os.remove(os.path.join(COMMIT_SYNC_DB, "FAULT_COMMIT_SYNC_FAIL.flag"))

verify_sync_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_commit_sync.noedb"
    let engine: NoeWalEngine
    let oRes: isize = noeWalEngineOpen(&engine, dbPath)
    print("COLD_REOPEN_RES:" + intToString(oRes as i64))
    if oRes == 0 as isize {
        let t: *NoeTableData = &engine.db.tables[0]
        print("VERIFIED_ROWS:" + intToString(t.rowCount as i64))
    }
    noeWalEngineClose(&engine)
}
main()
"""
verify_sync_path = os.path.join(NOESQL_DIR, "tests", "tmp_verify_sync.noqeri")
with open(verify_sync_path, "w") as f:
    f.write(verify_sync_code)

res_vsync = subprocess.run([NOQERI_BIN, "run", verify_sync_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Cold reopen output:\n", res_vsync.stdout.strip())
assert "COLD_REOPEN_RES:0" in res_vsync.stdout
# Because the COMMIT frame was physically written to disk, cold recovery verified and recovered it cleanly
assert "VERIFIED_ROWS:2" in res_vsync.stdout
print("=> PASS: Failed commit fsync returns -18, writer quarantined, cold recovery succeeds cleanly!")

# -----------------------------------------------------------------------------
# Test 11: Snapshot Persistence Failures in Checkpoint (P0 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 11] Checkpoint Fails Closed on Snapshot/HEAD Failure and Preserves WAL")
CP_FAULT_DB = os.path.join(NOESQL_DIR, "tmp_cp_fault.noedb")
if os.path.exists(CP_FAULT_DB): shutil.rmtree(CP_FAULT_DB, ignore_errors=True)
if os.path.exists(CP_FAULT_DB + ".lck"):
    try: os.remove(CP_FAULT_DB + ".lck")
    except OSError: pass

cp_init_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_cp_fault.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)

    let tx1: NoeWalTx
    noeWalEngineBegin(&engine, &tx1)
    noeWalEngineExecute(&engine, &tx1, "table users { id: int key auto, name: string }")
    noeWalEngineExecute(&engine, &tx1, "users add { name: 'alice' }")
    noeWalEngineExecute(&engine, &tx1, "users add { name: 'bob' }")
    noeWalEngineCommit(&engine, &tx1)

    file_close(engine.walFd)
    file_close(engine.lockFd)
    engine.walOpen = false
}
main()
"""
cp_init_path = os.path.join(NOESQL_DIR, "tests", "tmp_cp_init.noqeri")
with open(cp_init_path, "w") as f:
    f.write(cp_init_code)
subprocess.run([NOQERI_BIN, "run", cp_init_path], cwd=NOESQL_DIR, check=True)

# Test 11a: Snapshot write failure injection
with open(os.path.join(CP_FAULT_DB, ".noesql_testing_faults_enabled"), "w") as f: f.write("1")
with open(os.path.join(CP_FAULT_DB, "FAULT_SNAPSHOT_SAVE_FAIL.flag"), "w") as f: f.write("1")

cp_run_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_cp_fault.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)
    let cp: isize = noeWalEngineCheckpoint(&engine)
    print("CP_RES:" + intToString(cp as i64))
    file_close(engine.walFd)
    file_close(engine.lockFd)
    engine.walOpen = false
}
main()
"""
cp_run_path = os.path.join(NOESQL_DIR, "tests", "tmp_cp_run.noqeri")
with open(cp_run_path, "w") as f:
    f.write(cp_run_code)

res_cp1 = subprocess.run([NOQERI_BIN, "run", cp_run_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Checkpoint snapshot fault output:\n", res_cp1.stdout.strip())
assert "CP_RES:-25" in res_cp1.stdout, f"Expected -25, got: {res_cp1.stdout}"

# Test 11b: HEAD sync failure injection
os.remove(os.path.join(CP_FAULT_DB, "FAULT_SNAPSHOT_SAVE_FAIL.flag"))
with open(os.path.join(CP_FAULT_DB, "FAULT_HEAD_SYNC_FAIL.flag"), "w") as f: f.write("1")

res_cp2 = subprocess.run([NOQERI_BIN, "run", cp_run_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Checkpoint HEAD sync fault output:\n", res_cp2.stdout.strip())
assert "CP_RES:-26" in res_cp2.stdout, f"Expected -26, got: {res_cp2.stdout}"

os.remove(os.path.join(CP_FAULT_DB, "FAULT_HEAD_SYNC_FAIL.flag"))
os.remove(os.path.join(CP_FAULT_DB, ".noesql_testing_faults_enabled"))

# Verify WAL.log was NOT truncated and still exists
wal_cp = os.path.join(CP_FAULT_DB, "WAL.log")
assert os.path.exists(wal_cp), "WAL.log must be preserved on checkpoint failure"
assert os.path.getsize(wal_cp) > 0, "WAL.log must not be truncated when checkpoint fails"

# Verify cold recovery recovers all rows from preserved WAL
cp_verify_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_cp_fault.noedb"
    let engine: NoeWalEngine
    let oRes: isize = noeWalEngineOpen(&engine, dbPath)
    print("RECOVER_AFTER_CP_FAIL_OPEN:" + intToString(oRes as i64))
    if oRes == 0 as isize {
        let t: *NoeTableData = &engine.db.tables[0]
        print("RECOVERED_ROWS:" + intToString(t.rowCount as i64))
    }
    noeWalEngineClose(&engine)
}
main()
"""
cp_verify_path = os.path.join(NOESQL_DIR, "tests", "tmp_cp_verify.noqeri")
with open(cp_verify_path, "w") as f:
    f.write(cp_verify_code)

res_cpv = subprocess.run([NOQERI_BIN, "run", cp_verify_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Recovery after failed checkpoint output:\n", res_cpv.stdout.strip())
assert "RECOVER_AFTER_CP_FAIL_OPEN:0" in res_cpv.stdout
assert "RECOVERED_ROWS:2" in res_cpv.stdout
print("=> PASS: Checkpoint aborts on snapshot/HEAD failure, preserving WAL with complete recovery!")

# -----------------------------------------------------------------------------
# Test 12: Equal-Size WAL Replacement Detection via File Identity (P0 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 12] Equal-Size WAL Replacement Inode Detection & Recovery")
REPLACE_WAL_DB = os.path.join(NOESQL_DIR, "tmp_replace_wal.noedb")
if os.path.exists(REPLACE_WAL_DB): shutil.rmtree(REPLACE_WAL_DB, ignore_errors=True)
if os.path.exists(REPLACE_WAL_DB + ".lck"):
    try: os.remove(REPLACE_WAL_DB + ".lck")
    except OSError: pass

replace_init_code = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"

function main(): void {
    let dbPath: string = "tmp_replace_wal.noedb"
    let engine: NoeWalEngine
    let oRes: isize = noeWalEngineOpen(&engine, dbPath)
    if oRes != 0 as isize { print("OPEN_FAILED") return }

    let tx1: NoeWalTx
    noeWalEngineBegin(&engine, &tx1)
    noeWalEngineExecute(&engine, &tx1, "table items { id: int key auto, val: int }")
    noeWalEngineExecute(&engine, &tx1, "items add { val: 100 }")
    noeWalEngineCommit(&engine, &tx1)

    // Signal test harness that transaction is committed and walFd is held open
    let sig: isize = file_open_create(dbPath + "/READY_FOR_REPLACE.flag")
    if file_handle_valid(sig) { file_close(sig as usize) }

    // Wait until test harness replaces WAL.log with another file of the EXACT same size
    let waitIter: usize = 0 as usize
    while waitIter < 500 as usize {
        if fs_exists(dbPath + "/REPLACE_DONE.flag") { break }
        thread_sleep_millis(10 as i64)
        waitIter = waitIter + 1 as usize
    }

    // Refresh under lock to verify equal-size WAL replacement detection
    let refRes: isize = noeWalEngineRefresh(&engine)
    print("REFRESH_AFTER_SAME_SIZE_REPLACE:" + noeIntToString(refRes as i64))

    // Recheck open descriptor inode vs disk inode
    let diskInode: i64 = fs_inode(dbPath + "/WAL.log")
    let curInode: i64 = file_inode(engine.walFd)
    if curInode == diskInode && diskInode > 0 as i64 {
        print("INODES_REBOUND_MATCH:true")
    } else {
        print("INODES_REBOUND_MATCH:false")
    }
    print("POST_REPLACE_ROW_COUNT:" + noeIntToString(engine.db.tables[0].rowCount as i64))

    noeWalEngineClose(&engine)
}
main()
"""
replace_script_path = os.path.join(NOESQL_DIR, "tests", "tmp_replace_script.noqeri")
with open(replace_script_path, "w") as f:
    f.write(replace_init_code)

p_rep = subprocess.Popen([NOQERI_BIN, "run", replace_script_path], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

# Wait for READY_FOR_REPLACE.flag
ready_replace = os.path.join(REPLACE_WAL_DB, "READY_FOR_REPLACE.flag")
while not os.path.exists(ready_replace):
    time.sleep(0.01)
    if p_rep.poll() is not None:
        break

wal_to_replace = os.path.join(REPLACE_WAL_DB, "WAL.log")
wal_size = os.path.getsize(wal_to_replace)
old_ino = os.stat(wal_to_replace).st_ino

# Read exact bytes, modify header slightly or copy, and write to a new file of identical size
with open(wal_to_replace, "rb") as f:
    wal_bytes = f.read()

# Replace WAL.log with a new inode having the exact same byte size
tmp_wal = os.path.join(REPLACE_WAL_DB, "WAL.log.tmp")
with open(tmp_wal, "wb") as f:
    f.write(wal_bytes)
os.replace(tmp_wal, wal_to_replace)

new_ino = os.stat(wal_to_replace).st_ino
assert new_ino != old_ino, f"New inode ({new_ino}) must differ from old ({old_ino})"
assert os.path.getsize(wal_to_replace) == wal_size, "Replacement file must have identical size"

# Signal worker that replace is done
with open(os.path.join(REPLACE_WAL_DB, "REPLACE_DONE.flag"), "w") as f:
    f.write("done")

stdout_r, stderr_r = p_rep.communicate(timeout=10)
print("Same-size WAL replacement output:\n", stdout_r.strip())
assert "REFRESH_AFTER_SAME_SIZE_REPLACE:0" in stdout_r
assert "INODES_REBOUND_MATCH:true" in stdout_r
assert "POST_REPLACE_ROW_COUNT:1" in stdout_r, "Expected exactly 1 row, duplicate replay prevented"

# Cold restart verification after replacement
cold_restart_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_replace_wal.noedb"
    let engine: NoeWalEngine
    let oRes: isize = noeWalEngineOpen(&engine, dbPath)
    print("COLD_RESTART_OPEN:" + noeIntToString(oRes as i64))
    print("COLD_RESTART_ROW_COUNT:" + noeIntToString(engine.db.tables[0].rowCount as i64))
    noeWalEngineClose(&engine)
}
main()
"""
cold_script_path = os.path.join(NOESQL_DIR, "tests", "tmp_cold_replace.noqeri")
with open(cold_script_path, "w") as f:
    f.write(cold_restart_code)
res_cold = subprocess.run([NOQERI_BIN, "run", cold_script_path], cwd=NOESQL_DIR, capture_output=True, text=True, check=True)
print("Cold restart output:\n", res_cold.stdout.strip())
assert "COLD_RESTART_ROW_COUNT:1" in res_cold.stdout, "Cold restart must show exactly 1 row without duplicates"
print("=> PASS: Equal-size WAL replacement re-bound with zero duplicate rows in memory and cold restart!")

# -----------------------------------------------------------------------------
# Test 13: Strict Read Failure on Damaged/Truncated Snapshot Pages (P0 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 13] Strict Deserialization Read Checks Fail Closed on Truncated Snapshot")
CORRUPT_SNAP_DB = os.path.join(NOESQL_DIR, "tmp_corrupt_snap.noedb")
if os.path.exists(CORRUPT_SNAP_DB): shutil.rmtree(CORRUPT_SNAP_DB, ignore_errors=True)
if os.path.exists(CORRUPT_SNAP_DB + ".lck"):
    try: os.remove(CORRUPT_SNAP_DB + ".lck")
    except OSError: pass

corrupt_init_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_corrupt_snap.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)

    let tx: NoeWalTx
    noeWalEngineBegin(&engine, &tx)
    noeWalEngineExecute(&engine, &tx, "table users { id: int key auto, name: string }")
    noeWalEngineExecute(&engine, &tx, "users add { name: 'alice_super_long_name_for_testing' }")
    noeWalEngineExecute(&engine, &tx, "users add { name: 'bob_super_long_name_for_testing' }")
    noeWalEngineCommit(&engine, &tx)

    // Run explicit checkpoint so snapshot pages are written
    noeWalEngineCheckpoint(&engine)
    noeWalEngineClose(&engine)
}
main()
"""
corrupt_init_path = os.path.join(NOESQL_DIR, "tests", "tmp_corrupt_init.noqeri")
with open(corrupt_init_path, "w") as f:
    f.write(corrupt_init_code)
subprocess.run([NOQERI_BIN, "run", corrupt_init_path], cwd=NOESQL_DIR, check=True)

# Truncate active generation pages file halfway through row data
with open(os.path.join(CORRUPT_SNAP_DB, "HEAD"), "r") as f:
    active_gen = f.read().strip()
gen_pages = os.path.join(CORRUPT_SNAP_DB, "versions", f"generation-{active_gen}.pages")
assert os.path.exists(gen_pages), f"{gen_pages} must exist"
orig_size = os.path.getsize(gen_pages)
# Truncate to half the file size
with open(gen_pages, "r+b") as f:
    f.truncate(orig_size // 2)


corrupt_read_code = """import "../Database/noesql_engine.noqeri"

function main(): void {
    let db: NoeDatabaseState
    noeEngineInit(&db, "corrupt_test")
    let openRes: isize = noeDbOpenV3(&db, "tmp_corrupt_snap.noedb")
    print("CORRUPTED_OPEN_RES:" + noeIntToString(openRes as i64))
}
main()
"""
corrupt_read_path = os.path.join(NOESQL_DIR, "tests", "tmp_corrupt_read.noqeri")
with open(corrupt_read_path, "w") as f:
    f.write(corrupt_read_code)

res_corrupt = subprocess.run([NOQERI_BIN, "run", corrupt_read_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Corrupted snapshot open output:\n", res_corrupt.stdout.strip())
assert "CORRUPTED_OPEN_RES:" in res_corrupt.stdout
status_val = int(res_corrupt.stdout.strip().split("CORRUPTED_OPEN_RES:")[1].split("\n")[0])
assert status_val < 0, f"Expected negative error on truncated snapshot, got {status_val}"
print("=> PASS: Truncated snapshot strictly fails closed with error status: " + str(status_val))

# Sub-test 13B: Altered Byte / Bit-Flip Snapshot Corruption (Checksum Invariant)
BITFLIP_SNAP_DB = os.path.join(NOESQL_DIR, "tmp_bitflip_snap.noedb")
if os.path.exists(BITFLIP_SNAP_DB): shutil.rmtree(BITFLIP_SNAP_DB, ignore_errors=True)
bitflip_init_code = corrupt_init_code.replace("tmp_corrupt_snap.noedb", "tmp_bitflip_snap.noedb")
bitflip_init_path = os.path.join(NOESQL_DIR, "tests", "tmp_bitflip_init.noqeri")
with open(bitflip_init_path, "w") as f:
    f.write(bitflip_init_code)
subprocess.run([NOQERI_BIN, "run", bitflip_init_path], cwd=NOESQL_DIR, check=True)

with open(os.path.join(BITFLIP_SNAP_DB, "HEAD"), "r") as f:
    bf_gen = f.read().strip()
bf_pages = os.path.join(BITFLIP_SNAP_DB, "versions", f"generation-{bf_gen}.pages")
with open(bf_pages, "r+b") as f:
    bf_data = bytearray(f.read())
    # Corrupt one byte inside table definition / row payload (without truncating)
    bf_data[40] ^= 0xFF
    f.seek(0)
    f.write(bf_data)

bitflip_read_code = corrupt_read_code.replace("tmp_corrupt_snap.noedb", "tmp_bitflip_snap.noedb")
bitflip_read_path = os.path.join(NOESQL_DIR, "tests", "tmp_bitflip_read.noqeri")
with open(bitflip_read_path, "w") as f:
    f.write(bitflip_read_code)
res_bf = subprocess.run([NOQERI_BIN, "run", bitflip_read_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Bitflip snapshot open output:\n", res_bf.stdout.strip())
assert "CORRUPTED_OPEN_RES:-83" in res_bf.stdout, f"Expected -83 (Snapshot checksum failure), got {res_bf.stdout}"
print("=> PASS: Bit-flipped / altered snapshot payload detected by SHA-256 checksum and rejected with -83!")

# Sub-test 13C: Snapshot Checksum Trailer Removed Entirely (Empty Database and Populated Database)
EMPTY_SNAP_DB = os.path.join(NOESQL_DIR, "tmp_empty_snap.noedb")
if os.path.exists(EMPTY_SNAP_DB): shutil.rmtree(EMPTY_SNAP_DB, ignore_errors=True)
empty_init_code = """import "../Database/noesql_engine.noqeri"

function main(): void {
    let db: NoeDatabaseState
    noeEngineInit(&db, "empty_target")
    let sRes: isize = noeDbSaveV3(&db, "tmp_empty_snap.noedb")
    print("EMPTY_INIT_SAVE:" + noeIntToString(sRes as i64))
}
main()
"""
empty_init_path = os.path.join(NOESQL_DIR, "tests", "tmp_empty_init.noqeri")
with open(empty_init_path, "w") as f:
    f.write(empty_init_code)
subprocess.run([NOQERI_BIN, "run", empty_init_path], cwd=NOESQL_DIR, check=True)

with open(os.path.join(EMPTY_SNAP_DB, "HEAD"), "r") as f:
    empty_gen = f.read().strip()
empty_pages = os.path.join(EMPTY_SNAP_DB, "versions", f"generation-{empty_gen}.pages")
# Empty snapshot is exactly 64 bytes (32-byte header + 32-byte checksum trailer).
# Strip the 32-byte trailer entirely by truncating to 32 bytes:
with open(empty_pages, "r+b") as f:
    f.truncate(32)

empty_read_code = corrupt_read_code.replace("tmp_corrupt_snap.noedb", "tmp_empty_snap.noedb")
empty_read_path = os.path.join(NOESQL_DIR, "tests", "tmp_empty_read.noqeri")
with open(empty_read_path, "w") as f:
    f.write(empty_read_code)
res_empty = subprocess.run([NOQERI_BIN, "run", empty_read_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Empty snapshot stripped trailer output:\n", res_empty.stdout.strip())
assert "CORRUPTED_OPEN_RES:-83" in res_empty.stdout, f"Expected -83 on missing trailer for empty snapshot, got {res_empty.stdout}"
print("=> PASS: Stripped checksum trailer on empty snapshot strictly rejected with -83!")

# -----------------------------------------------------------------------------
# Test 14: Daemon UNCERTAIN_COMMIT, Reconciliation API & Post-Checkpoint Cold Restart (P0 & P1 Fixes)
# -----------------------------------------------------------------------------
print("\n[TEST 14] Daemon Exposes UNCERTAIN_COMMIT and Reconciles Rollback Gaps Before & After Checkpointing")
DAEMON_RECON_DB = os.path.join(NOESQL_DIR, "tmp_daemon_recon.noedb")
if os.path.exists(DAEMON_RECON_DB): shutil.rmtree(DAEMON_RECON_DB, ignore_errors=True)
if os.path.exists(DAEMON_RECON_DB + ".lck"):
    try: os.remove(DAEMON_RECON_DB + ".lck")
    except OSError: pass

daemon_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_daemon_recon.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)

    // 1. TX 1: Table schema creation (Commits -> tx 1)
    let res1: string = noeDaemonHandleRequest(&engine, "table orders { id: int key auto, amount: int }")
    print("DAEMON_RES1:" + res1)

    // 2. TX 2: Aborted / Rolled-back transaction (creates gap in sequence)
    let tx2: NoeWalTx
    noeWalEngineBegin(&engine, &tx2)
    noeWalEngineExecute(&engine, &tx2, "orders add { amount: 100 }")
    noeWalEngineRollback(&engine, &tx2)
    print("DAEMON_TX2_ROLLED_BACK:tx=" + noeIntToString(tx2.txId as i64))

    // 3. TX 3: Subsequent transaction (Commits -> tx 3)
    let tx3: NoeWalTx
    noeWalEngineBegin(&engine, &tx3)
    noeWalEngineExecute(&engine, &tx3, "orders add { amount: 200 }")
    noeWalEngineCommit(&engine, &tx3)
    print("DAEMON_TX3_COMMITTED:tx=" + noeIntToString(tx3.txId as i64))

    // 4. Verify exact reconciliation across rollback gap BEFORE checkpoint
    let r1: string = noeDaemonReconcileTx(&engine, 1 as u64)
    print("RECON_TX1:" + r1)

    let r2: string = noeDaemonReconcileTx(&engine, 2 as u64)
    print("RECON_TX2_GAP:" + r2)

    let r3: string = noeDaemonReconcileTx(&engine, 3 as u64)
    print("RECON_TX3:" + r3)

    let r999: string = noeDaemonReconcileTx(&engine, 999 as u64)
    print("RECON_TX999:" + r999)

    // 5. Injected fsync failure on commit (TX 4) -> surfaces UNCERTAIN_COMMIT
    let f1: isize = file_open_create(dbPath + "/.noesql_testing_faults_enabled")
    if file_handle_valid(f1) { file_close(f1 as usize) }
    let f2: isize = file_open_create(dbPath + "/FAULT_COMMIT_SYNC_FAIL.flag")
    if file_handle_valid(f2) { file_close(f2 as usize) }

    let res4: string = noeDaemonHandleRequest(&engine, "orders add { amount: 500 }")
    print("DAEMON_RES4_UNCERTAIN:" + res4)

    // Close engine (writer quarantined)
    noeWalEngineClose(&engine)

    // Clear fault flag for recovery/reconciliation
    let fRem: isize = fs_remove(dbPath + "/FAULT_COMMIT_SYNC_FAIL.flag")

    // 6. New engine instance opens and reconciles uncertain TX 4
    let engine2: NoeWalEngine
    noeWalEngineOpen(&engine2, dbPath)

    let reconRes4: string = noeDaemonHandleRequest(&engine2, "RECONCILE:4")
    print("DAEMON_RECON_UNCERTAIN_TX4:" + reconRes4)

    // 7. Explicit checkpoint to truncate WAL and move state to persistent snapshot
    let cpRes: isize = noeWalEngineCheckpoint(&engine2)
    print("CHECKPOINT_RES:" + noeIntToString(cpRes as i64))
    noeWalEngineClose(&engine2)

    // 8. COLD RESTART: Verify rollback gap (TX 2) is STILL reported as aborted after checkpoint & WAL truncation!
    let engine3: NoeWalEngine
    noeWalEngineOpen(&engine3, dbPath)

    let postCpReconTx1: string = noeDaemonHandleRequest(&engine3, "RECONCILE:1")
    print("POST_CP_COLD_RECON_TX1:" + postCpReconTx1)

    let postCpReconTx2: string = noeDaemonHandleRequest(&engine3, "RECONCILE:2")
    print("POST_CP_COLD_RECON_TX2_GAP:" + postCpReconTx2)

    let postCpReconTx3: string = noeDaemonHandleRequest(&engine3, "RECONCILE:3")
    print("POST_CP_COLD_RECON_TX3:" + postCpReconTx3)

    let postCpReconTx4: string = noeDaemonHandleRequest(&engine3, "RECONCILE:4")
    print("POST_CP_COLD_RECON_TX4:" + postCpReconTx4)

    let postCpReconTx999: string = noeDaemonHandleRequest(&engine3, "RECONCILE:999")
    print("POST_CP_COLD_RECON_TX999:" + postCpReconTx999)

    noeWalEngineClose(&engine3)
}
main()
"""
daemon_path = os.path.join(NOESQL_DIR, "tests", "tmp_daemon_test.noqeri")
with open(daemon_path, "w") as f:
    f.write(daemon_code)

res_daemon = subprocess.run([NOQERI_BIN, "run", daemon_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Daemon uncertain commit & reconcile output:\n", res_daemon.stdout.strip())
assert "DAEMON_RES1:OK:tx=1" in res_daemon.stdout
assert "RECON_TX1:RECONCILED:committed:tx=1" in res_daemon.stdout
assert "RECON_TX2_GAP:RECONCILED:aborted:tx=2" in res_daemon.stdout, "Rollback gap must NEVER be reported as committed!"
assert "RECON_TX3:RECONCILED:committed:tx=3" in res_daemon.stdout
assert "RECON_TX999:RECONCILED:aborted:tx=999" in res_daemon.stdout
assert "DAEMON_RES4_UNCERTAIN:UNCERTAIN_COMMIT:tx=4" in res_daemon.stdout
assert "DAEMON_RECON_UNCERTAIN_TX4:RECONCILED:committed:tx=4" in res_daemon.stdout

# Assert post-checkpoint and cold restart reconciliation:
assert "POST_CP_COLD_RECON_TX1:RECONCILED:committed:tx=1" in res_daemon.stdout
assert "POST_CP_COLD_RECON_TX2_GAP:RECONCILED:aborted:tx=2" in res_daemon.stdout, "Rollback gap after checkpoint and cold restart must NEVER be reported as committed!"
assert "POST_CP_COLD_RECON_TX3:RECONCILED:committed:tx=3" in res_daemon.stdout
assert "POST_CP_COLD_RECON_TX4:RECONCILED:committed:tx=4" in res_daemon.stdout
assert "POST_CP_COLD_RECON_TX999:RECONCILED:aborted:tx=999" in res_daemon.stdout
print("=> PASS: Reconciliation accurately distinguishes rollback gaps (aborted) from valid commits across checkpoints and cold restarts!")

# -----------------------------------------------------------------------------
# Test 15: Forged / Corrupted WAL COMMIT-Like Frame Detection (P1 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 15] Forged / Tampered COMMIT Frame Rejection in Reconciliation")
FORGE_WAL_DB = os.path.join(NOESQL_DIR, "tmp_forge_wal.noedb")
if os.path.exists(FORGE_WAL_DB): shutil.rmtree(FORGE_WAL_DB, ignore_errors=True)
if os.path.exists(FORGE_WAL_DB + ".lck"):
    try: os.remove(FORGE_WAL_DB + ".lck")
    except OSError: pass

# Process 1: Starts engine, commits TX 1, signals ready, waits for forged frame, reconciles TX 2
forge_runner_code = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"

function main(): void {
    let dbPath: string = "tmp_forge_wal.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)

    // TX 1: Valid schema creation
    let res1: string = noeDaemonHandleRequest(&engine, "table accounts { id: int key auto, balance: int }")
    print("FORGE_INIT:" + res1)
    file_flush(1 as usize)

    // Signal TX 1 is durable
    let sig1: isize = file_open_create(dbPath + "/TX1_READY.marker")
    if file_handle_valid(sig1) { file_close(sig1 as usize) }

    // Wait for external harness to inject forged COMMIT frame into WAL.log
    let waitIter: usize = 0 as usize
    while waitIter < 500 as usize {
        if fs_exists(dbPath + "/FORGE_INJECTED.marker") { break }
        thread_sleep_millis(10 as i64)
        waitIter = waitIter + 1 as usize
    }

    // Attempt to reconcile forged TX 2 while engine is live
    let res2: string = noeDaemonHandleRequest(&engine, "RECONCILE:2")
    print("FORGED_RECON_RES:" + res2)
    file_flush(1 as usize)

    noeWalEngineClose(&engine)
}
main()
"""
forge_runner_path = os.path.join(NOESQL_DIR, "tests", "tmp_forge_runner.noqeri")
with open(forge_runner_path, "w") as f:
    f.write(forge_runner_code)

proc_forge = subprocess.Popen([NOQERI_BIN, "run", forge_runner_path], cwd=NOESQL_DIR, stdout=subprocess.PIPE, text=True)

# Wait for TX 1 to commit
tx1_marker = os.path.join(FORGE_WAL_DB, "TX1_READY.marker")
for _ in range(100):
    if os.path.exists(tx1_marker): break
    time.sleep(0.02)

# Inject forged transaction frame for TX 2 directly into WAL.log
wal_file = os.path.join(FORGE_WAL_DB, "WAL.log")
with open(wal_file, "a+b") as f:
    # BEGIN frame (56 bytes): magic NWAL(4), ver=1(2), kind=1(1), flags=0(1), tx=2(8), seq=1(4), psize=0(4), digest(32)
    begin_hdr = bytearray(56)
    begin_hdr[0:4] = b"NWAL"
    begin_hdr[4] = 1 # version
    begin_hdr[6] = 1 # kind = BEGIN
    begin_hdr[8:16] = (2).to_bytes(8, "little")
    begin_hdr[16:20] = (1).to_bytes(4, "little")
    begin_hdr[20:24] = (0).to_bytes(4, "little")
    f.write(begin_hdr)

    # IMAGE frame (56 bytes header + payload):
    stmt = b"accounts add { balance: 9999 }"
    img_hdr = bytearray(56)
    img_hdr[0:4] = b"NWAL"
    img_hdr[4] = 1
    img_hdr[6] = 2 # kind = IMAGE
    img_hdr[8:16] = (2).to_bytes(8, "little")
    img_hdr[16:20] = (2).to_bytes(4, "little")
    img_hdr[20:24] = len(stmt).to_bytes(4, "little")
    f.write(img_hdr)
    f.write(stmt)

    # Forged COMMIT frame (56 bytes header + 32 bytes garbage payload)
    commit_hdr = bytearray(56)
    commit_hdr[0:4] = b"NWAL"
    commit_hdr[4] = 1
    commit_hdr[6] = 3 # kind = COMMIT
    commit_hdr[8:16] = (2).to_bytes(8, "little")
    commit_hdr[16:20] = (3).to_bytes(4, "little")
    commit_hdr[20:24] = (32).to_bytes(4, "little")
    f.write(commit_hdr)
    f.write(b"\xee" * 32) # Forged payload (does NOT match SHA-256 rolling digest)

# Signal forged write is ready
with open(os.path.join(FORGE_WAL_DB, "FORGE_INJECTED.marker"), "w") as f:
    f.write("injected\n")

out_forge, _ = proc_forge.communicate()
print("Forged frame reconciliation output:\n", out_forge.strip())
assert proc_forge.returncode == 0, f"Forge runner failed with returncode {proc_forge.returncode}"
assert "committed" not in out_forge.lower() or "reconciled:committed" not in out_forge.lower(), f"Forged frame must NEVER be reported as committed! Got: {out_forge}"
assert ("FORGED_RECON_RES:RECONCILE_ERROR:-107" in out_forge) or ("FORGED_RECON_RES:RECONCILED:aborted:tx=2" in out_forge), f"Expected rejection of forged frame, got: {out_forge}"
print("=> PASS: Forged COMMIT frame detected and rejected by cryptographic digest verification!")

# Also verify that a cold restart fails closed (-107) on the forged frame
forge_cold_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_forge_wal.noedb"
    let engine: NoeWalEngine
    let oRes: isize = noeWalEngineOpen(&engine, dbPath)
    print("FORGED_COLD_OPEN:" + noeIntToString(oRes as i64))
    if oRes == 0 as isize { noeWalEngineClose(&engine) }
}
main()
"""
forge_cold_path = os.path.join(NOESQL_DIR, "tests", "tmp_forge_cold.noqeri")
with open(forge_cold_path, "w") as f:
    f.write(forge_cold_code)
res_forge_cold = subprocess.run([NOQERI_BIN, "run", forge_cold_path], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Forged cold open output:\n", res_forge_cold.stdout.strip())
assert "FORGED_COLD_OPEN:-107" in res_forge_cold.stdout, f"Expected -107 (WAL digest verification failed), got {res_forge_cold.stdout}"
print("=> PASS: Cold startup strictly rejects forged WAL frame with -107!")

# -----------------------------------------------------------------------------
# Test 16: Concurrency: Reconciliation During Active Checkpointing (P1 Fix)
# -----------------------------------------------------------------------------
print("\n[TEST 16] Reconciliation Synchronization During Active Checkpointing")
CONCUR_CKPT_DB = os.path.join(NOESQL_DIR, "tmp_concur_ckpt.noedb")
if os.path.exists(CONCUR_CKPT_DB): shutil.rmtree(CONCUR_CKPT_DB, ignore_errors=True)
if os.path.exists(CONCUR_CKPT_DB + ".lck"):
    try: os.remove(CONCUR_CKPT_DB + ".lck")
    except OSError: pass

concur_init_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_concur_ckpt.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)
    noeDaemonHandleRequest(&engine, "table ledger { id: int key auto, amount: int }")
    noeDaemonHandleRequest(&engine, "ledger add { amount: 100 }")
    print("CONCUR_INIT_OK")
    noeWalEngineClose(&engine)
}
main()
"""
concur_init_path = os.path.join(NOESQL_DIR, "tests", "tmp_concur_init.noqeri")
with open(concur_init_path, "w") as f:
    f.write(concur_init_code)
subprocess.run([NOQERI_BIN, "run", concur_init_path], cwd=NOESQL_DIR, check=True)

concur_worker_a = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"

function main(): void {
    let dbPath: string = "tmp_concur_ckpt.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)

    let round: usize = 0 as usize
    while round < 5 as usize {
        let req: string = "ledger add { amount: " + noeIntToString((round + 1 as usize) as i64 * 100 as i64) + " }"
        noeDaemonHandleRequest(&engine, req)
        thread_sleep_millis(30 as i64)
        noeWalEngineCheckpoint(&engine)
        thread_sleep_millis(20 as i64)
        round = round + 1 as usize
    }

    noeWalEngineClose(&engine)
    print("WORKER_A_FINISHED")
}
main()
"""
concur_worker_a_path = os.path.join(NOESQL_DIR, "tests", "tmp_concur_worker_a.noqeri")
with open(concur_worker_a_path, "w") as f:
    f.write(concur_worker_a)

concur_worker_b = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"

function main(): void {
    let dbPath: string = "tmp_concur_ckpt.noedb"
    let engine: NoeWalEngine
    noeWalEngineOpen(&engine, dbPath)

    let round: usize = 0 as usize
    while round < 5 as usize {
        let r1: string = noeDaemonReconcileTx(&engine, 1 as u64)
        print("CONCUR_RECON_TX1:" + r1)
        let r2: string = noeDaemonReconcileTx(&engine, 2 as u64)
        print("CONCUR_RECON_TX2:" + r2)
        thread_sleep_millis(25 as i64)
        round = round + 1 as usize
    }

    noeWalEngineClose(&engine)
    print("WORKER_B_FINISHED")
}
main()
"""
concur_worker_b_path = os.path.join(NOESQL_DIR, "tests", "tmp_concur_worker_b.noqeri")
with open(concur_worker_b_path, "w") as f:
    f.write(concur_worker_b)

proc_a = subprocess.Popen([NOQERI_BIN, "run", concur_worker_a_path], cwd=NOESQL_DIR, stdout=subprocess.PIPE, text=True)
proc_b = subprocess.Popen([NOQERI_BIN, "run", concur_worker_b_path], cwd=NOESQL_DIR, stdout=subprocess.PIPE, text=True)

out_a, _ = proc_a.communicate()
out_b, _ = proc_b.communicate()

print("Worker A output:\n", out_a.strip())
print("Worker B output:\n", out_b.strip())
assert proc_a.returncode == 0, f"Worker A failed with {proc_a.returncode}"
assert proc_b.returncode == 0, f"Worker B failed with {proc_b.returncode}"
assert "CONCUR_RECON_TX1:RECONCILED:committed:tx=1" in out_b
assert "CONCUR_RECON_TX2:RECONCILED:committed:tx=2" in out_b
print("=> PASS: Concurrent reconciliation cleanly synchronized with active checkpointing under lock!")

# Cleanup temporary files
for p in [commit_fail_path, missing_init_path, missing_test_path, degraded_path, unlink_script_path, commit_sync_path, verify_sync_path, cp_init_path, cp_run_path, cp_verify_path, replace_script_path, cold_script_path, corrupt_init_path, corrupt_read_path, bitflip_init_path, bitflip_read_path, empty_init_path, empty_read_path, daemon_path, forge_runner_path, forge_cold_path, concur_init_path, concur_worker_a_path, concur_worker_b_path]:
    if os.path.exists(p): os.remove(p)
for d in [FAIL_COMMIT_DB, MISSING_WAL_DB, ACTIVE_UNLINK_DB, COMMIT_SYNC_DB, CP_FAULT_DB, REPLACE_WAL_DB, CORRUPT_SNAP_DB, BITFLIP_SNAP_DB, EMPTY_SNAP_DB, DAEMON_RECON_DB, FORGE_WAL_DB, CONCUR_CKPT_DB]:
    if os.path.exists(d): shutil.rmtree(d, ignore_errors=True)
    if os.path.exists(d + ".lck"):
        try: os.remove(d + ".lck")
        except OSError: pass

print("\nReal Crash & Fault Injection Durability Suite: ALL PASSED")

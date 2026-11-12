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
assert recovered_vals == acknowledged_commits, f"Mismatch!\nAck: {acknowledged_commits}\nRec: {recovered_vals}"
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
expected_count = len(acknowledged_commits) + 1
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

# Cleanup temporary files
for p in [commit_fail_path, missing_init_path, missing_test_path, degraded_path]:
    if os.path.exists(p): os.remove(p)
for d in [FAIL_COMMIT_DB, MISSING_WAL_DB]:
    if os.path.exists(d): shutil.rmtree(d, ignore_errors=True)
    if os.path.exists(d + ".lck"):
        try: os.remove(d + ".lck")
        except OSError: pass

print("\nReal Crash & Fault Injection Durability Suite: ALL PASSED")

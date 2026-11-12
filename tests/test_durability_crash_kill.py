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

print("\nReal Crash & Fault Injection Durability Suite: ALL PASSED")

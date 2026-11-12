import os
import sys
import time
import shutil
import subprocess

NOESQL_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
NOQERI_BIN = os.environ.get("NOQERI", os.path.abspath(os.path.join(NOESQL_DIR, "..", "Noqeri", "build", "noqeri")))

print("================================================================================")
print("  TEST: Cross-Process State Refresh & Persistent Monotonic Transaction IDs")
print("================================================================================")

DB_PATH = os.path.join(NOESQL_DIR, "tmp_test_refresh.noedb")

def cleanup():
    if os.path.exists(DB_PATH):
        shutil.rmtree(DB_PATH, ignore_errors=True)
    if os.path.exists(DB_PATH + ".lck"):
        try: os.remove(DB_PATH + ".lck")
        except OSError: pass

cleanup()

# Process A script: creates schema, writes 100, waits for B, then writes 300
proc_a_code = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"
import "../../Noqeri/Lib/std/fs.noqeri"

function main(): void {
    let dbPath: string = "tmp_test_refresh.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("A_OPEN_FAILED") return }

    // Init schema
    let tx0: NoeWalTx
    noeWalEngineBegin(&engine, &tx0)
    noeWalEngineExecute(&engine, &tx0, "table records { id: int key auto, val: int }")
    noeWalEngineCommit(&engine, &tx0)

    // Tx 1: val 100
    let tx1: NoeWalTx
    noeWalEngineBegin(&engine, &tx1)
    noeWalEngineExecute(&engine, &tx1, "records add { val: 100 }")
    noeWalEngineCommit(&engine, &tx1)
    print("A_COMMITTED_TX:" + intToString(tx1.txId as i64))

    // Signal B to proceed by creating flag file
    let flagA: isize = file_open_create("tmp_flag_a.txt")
    file_close(flagA as usize)

    // Wait until B has committed its write
    while !fs_exists("tmp_flag_b.txt") {
        thread_sleep_millis(20 as i64)
    }

    // Now A begins Tx: must REFRESH and see B's row 200!
    let txA2: NoeWalTx
    noeWalEngineBegin(&engine, &txA2)
    print("A_NEXT_TX_ID:" + intToString(txA2.txId as i64))

    let tIdx: isize = noeFindTable(&engine.db, "records")
    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("A_SEES_ROW_COUNT:" + intToString(t.rowCount as i64))

    let r: usize = 0 as usize
    while r < t.rowCount {
        let cell: *NoeRowCell = &t.cells[r * 16 as usize + 1 as usize]
        if !cell.isNull {
            print("A_SEES_VAL:" + intToString(cell.intVal))
        }
        r = r + 1 as usize
    }

    noeWalEngineExecute(&engine, &txA2, "records add { val: 300 }")
    noeWalEngineCommit(&engine, &txA2)
    print("A_COMMITTED_FINAL")

    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""

# Process B script: opened concurrently, waits for A, then verifies it sees 100, then writes 200
proc_b_code = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"
import "../../Noqeri/Lib/std/fs.noqeri"

function main(): void {
    let dbPath: string = "tmp_test_refresh.noedb"

    // Wait until A has initialized schema and committed Tx 1
    while !fs_exists("tmp_flag_a.txt") {
        thread_sleep_millis(20 as i64)
    }

    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("B_OPEN_FAILED") return }

    // Begin transaction in B: should refresh and see Tx 1's write!
    let txB: NoeWalTx
    noeWalEngineBegin(&engine, &txB)
    print("B_ACQUIRED_TX_ID:" + intToString(txB.txId as i64))

    let tIdx: isize = noeFindTable(&engine.db, "records")
    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("B_SEES_ROW_COUNT:" + intToString(t.rowCount as i64))

    let r: usize = 0 as usize
    while r < t.rowCount {
        let cell: *NoeRowCell = &t.cells[r * 16 as usize + 1 as usize]
        if !cell.isNull {
            print("B_SEES_VAL:" + intToString(cell.intVal))
        }
        r = r + 1 as usize
    }

    noeWalEngineExecute(&engine, &txB, "records add { val: 200 }")
    noeWalEngineCommit(&engine, &txB)
    print("B_COMMITTED_TX:" + intToString(txB.txId as i64))

    // Signal A that B has committed
    let flagB: isize = file_open_create("tmp_flag_b.txt")
    file_close(flagB as usize)

    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""

script_a = os.path.join(NOESQL_DIR, "tests", "tmp_proc_a.noqeri")
script_b = os.path.join(NOESQL_DIR, "tests", "tmp_proc_b.noqeri")
with open(script_a, "w") as f: f.write(proc_a_code)
with open(script_b, "w") as f: f.write(proc_b_code)

try:
    if os.path.exists("tmp_flag_a.txt"): os.remove("tmp_flag_a.txt")
    if os.path.exists("tmp_flag_b.txt"): os.remove("tmp_flag_b.txt")
except OSError: pass

# Start process A and process B concurrently
p_a = subprocess.Popen([NOQERI_BIN, "run", script_a], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
p_b = subprocess.Popen([NOQERI_BIN, "run", script_b], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

out_a, err_a = p_a.communicate(timeout=30)
out_b, err_b = p_b.communicate(timeout=30)

print("--- Process A Output ---")
print(out_a.strip())
print("--- Process B Output ---")
print(out_b.strip())

assert p_a.returncode == 0, f"Process A failed: {err_a}"
assert p_b.returncode == 0, f"Process B failed: {err_b}"

# Verifications
assert "B_SEES_VAL:100" in out_b, "Process B failed to see Process A's committed write!"
assert "A_SEES_VAL:200" in out_a, "Process A failed to refresh and see Process B's committed write!"

# Monotonic tx IDs verification: tx0=1, tx1=2, txB=3, txA2=4
assert "A_COMMITTED_TX:2" in out_a, "Expected tx 2 for A's first commit"
assert "B_ACQUIRED_TX_ID:3" in out_b, "Expected tx 3 for B's commit"
assert "A_NEXT_TX_ID:4" in out_a, "Expected tx 4 for A's second commit"

print("\n=> PASS: Cross-process state refresh & persistent monotonic transaction IDs verified!")

# Cleanup
for f in ["tmp_flag_a.txt", "tmp_flag_b.txt", script_a, script_b]:
    if os.path.exists(f): os.remove(f)
cleanup()

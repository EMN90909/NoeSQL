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
    print("A_FINAL_OFFSET:" + intToString(engine.walCommittedOffset as i64))

    // Signal B to commit its second write
    let flagA2: isize = file_open_create("tmp_flag_a2.txt")
    file_close(flagA2 as usize)

    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""

# Process B script: opened concurrently, waits for A, then verifies it sees 100, writes 200,
# then waits for A's second commit, refreshes, sees 300, and writes 400!
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

    // Signal A that B has committed 200
    let flagB: isize = file_open_create("tmp_flag_b.txt")
    file_close(flagB as usize)

    // Wait until A has committed 300
    while !fs_exists("tmp_flag_a2.txt") {
        thread_sleep_millis(20 as i64)
    }

    // Now B begins second Tx (Interleaved A -> B -> A -> B): must refresh and see A's row 300!
    let txB2: NoeWalTx
    noeWalEngineBegin(&engine, &txB2)
    print("B_NEXT_TX_ID:" + intToString(txB2.txId as i64))

    let r2: usize = 0 as usize
    while r2 < t.rowCount {
        let cell2: *NoeRowCell = &t.cells[r2 * 16 as usize + 1 as usize]
        if !cell2.isNull {
            print("B_SEES_VAL_ROUND2:" + intToString(cell2.intVal))
        }
        r2 = r2 + 1 as usize
    }

    noeWalEngineExecute(&engine, &txB2, "records add { val: 400 }")
    noeWalEngineCommit(&engine, &txB2)
    print("B_COMMITTED_FINAL")
    print("B_FINAL_OFFSET:" + intToString(engine.walCommittedOffset as i64))

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
    if os.path.exists("tmp_flag_a2.txt"): os.remove("tmp_flag_a2.txt")
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
assert "B_SEES_VAL_ROUND2:300" in out_b, "Process B failed to refresh and see Process A's second committed write!"

# Monotonic tx IDs verification: tx0=1, tx1=2, txB=3, txA2=4, txB2=5
assert "A_COMMITTED_TX:2" in out_a, "Expected tx 2 for A's first commit"
assert "B_ACQUIRED_TX_ID:3" in out_b, "Expected tx 3 for B's commit"
assert "A_NEXT_TX_ID:4" in out_a, "Expected tx 4 for A's second commit"
assert "B_NEXT_TX_ID:5" in out_b, "Expected tx 5 for B's second commit"

# Verify physical disk WAL matches the tracked committed offset of B's final commit!
wal_disk_size = os.path.getsize(os.path.join(DB_PATH, "WAL.log"))
assert f"B_FINAL_OFFSET:{wal_disk_size}" in out_b, f"B's final offset does not match physical WAL size {wal_disk_size}!"
print(f"Verified physical WAL disk size ({wal_disk_size} bytes) matches B_FINAL_OFFSET exactly!")

# Cold Restart Verification in Process C:
# Open database cold, assert all 4 rows [100, 200, 300, 400] are recovered in exact sequence!
proc_c_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_test_refresh.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("C_OPEN_FAILED") return }

    let tIdx: isize = noeFindTable(&engine.db, "records")
    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("C_TOTAL_RECOVERED:" + intToString(t.rowCount as i64))

    let r: usize = 0 as usize
    while r < t.rowCount {
        let cell: *NoeRowCell = &t.cells[r * 16 as usize + 1 as usize]
        if !cell.isNull {
            print("C_RECOVERED_VAL:" + intToString(cell.intVal))
        }
        r = r + 1 as usize
    }

    print("C_FINAL_OFFSET:" + intToString(engine.walCommittedOffset as i64))
    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""
script_c = os.path.join(NOESQL_DIR, "tests", "tmp_proc_c.noqeri")
with open(script_c, "w") as f: f.write(proc_c_code)

res_c = subprocess.run([NOQERI_BIN, "run", script_c], cwd=NOESQL_DIR, capture_output=True, text=True)
print("--- Process C (Cold Restart) Output ---")
print(res_c.stdout.strip())
assert res_c.returncode == 0, f"Process C failed:\n{res_c.stderr}"
assert "C_TOTAL_RECOVERED:4" in res_c.stdout, f"Expected 4 rows recovered, got:\n{res_c.stdout}"
assert f"C_FINAL_OFFSET:{wal_disk_size}" in res_c.stdout, "Cold reopen offset does not match disk size!"

recovered_c_vals = []
for line in res_c.stdout.splitlines():
    if line.startswith("C_RECOVERED_VAL:"):
        recovered_c_vals.append(int(line.split(":")[1]))

assert recovered_c_vals == [100, 200, 300, 400], f"Recovered values mismatch: {recovered_c_vals}"
print("=> PASS: Interleaved A -> B -> A -> B writes verified with exact physical offsets and cold restart recovery!")

# Cleanup
for f in ["tmp_flag_a.txt", "tmp_flag_b.txt", "tmp_flag_a2.txt", script_a, script_b, script_c]:
    if os.path.exists(f): os.remove(f)
cleanup()

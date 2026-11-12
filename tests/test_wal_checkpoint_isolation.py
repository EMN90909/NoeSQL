import os
import sys
import time
import shutil
import subprocess

NOESQL_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
NOQERI_BIN = os.environ.get("NOQERI", os.path.abspath(os.path.join(NOESQL_DIR, "..", "Noqeri", "build", "noqeri")))

print("================================================================================")
print("  TEST: Coordinated Multi-Process Checkpoint Isolation")
print("  Verifying stale process cannot overwrite newer writes during checkpoint")
print("================================================================================")

DB_PATH = os.path.join(NOESQL_DIR, "tmp_test_ckpt_isolation.noedb")

def cleanup():
    if os.path.exists(DB_PATH):
        shutil.rmtree(DB_PATH, ignore_errors=True)
    if os.path.exists(DB_PATH + ".lck"):
        try: os.remove(DB_PATH + ".lck")
        except OSError: pass

cleanup()

# 1. Initialize DB with initial schema
init_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_test_ckpt_isolation.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("OPEN_FAILED") return }

    let tx: NoeWalTx
    noeWalEngineBegin(&engine, &tx)
    noeWalEngineExecute(&engine, &tx, "table ledger { id: int key auto, amount: int }")
    noeWalEngineExecute(&engine, &tx, "ledger add { amount: 1000 }")
    noeWalEngineCommit(&engine, &tx)

    noeWalEngineClose(&engine)
    print("INIT_OK")
}
main()
"""
init_script = os.path.join(NOESQL_DIR, "tests", "tmp_ckpt_init.noqeri")
with open(init_script, "w") as f: f.write(init_code)

res = subprocess.run([NOQERI_BIN, "run", init_script], cwd=NOESQL_DIR, capture_output=True, text=True)
assert "INIT_OK" in res.stdout, f"Init failed: {res.stdout}"

# 2. Process B opens the database early (holds snapshot with only amount: 1000)
# Process A then opens and commits amount: 2000, 3000, 4000
# Process B then calls noeWalEngineClose() (checkpointing).
# Process B must REFRESH under the lock before saving, so amounts 2000, 3000, 4000 are NOT lost!

proc_b_code = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"
import "../../Noqeri/Lib/std/fs.noqeri"

function main(): void {
    let dbPath: string = "tmp_test_ckpt_isolation.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("B_OPEN_FAILED") return }

    print("B_OPENED_EARLY")

    // Signal A to proceed and write amounts 2000, 3000, 4000
    let fA: isize = file_open_create("tmp_flag_b_ready.txt")
    file_close(fA as usize)

    // Wait until A has committed its transactions
    while !fs_exists("tmp_flag_a_done.txt") {
        thread_sleep_millis(20 as i64)
    }

    // Now B checkpoints and closes!
    // Coordinated checkpoint must refresh under the lock before snapshotting!
    let closeRes: isize = noeWalEngineClose(&engine)
    print("B_CHECKPOINTED:" + intToString(closeRes as i64))
}
main()
"""

proc_a_code = """import "../Database/noesql_wal_engine.noqeri"
import "../../Noqeri/Lib/std/thread.noqeri"
import "../../Noqeri/Lib/std/fs.noqeri"

function main(): void {
    let dbPath: string = "tmp_test_ckpt_isolation.noedb"

    // Wait for B to be open
    while !fs_exists("tmp_flag_b_ready.txt") {
        thread_sleep_millis(20 as i64)
    }

    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("A_OPEN_FAILED") return }

    let i: usize = 0 as usize
    while i < 3 as usize {
        let tx: NoeWalTx
        noeWalEngineBegin(&engine, &tx)
        let amt: i64 = ((i + 2 as usize) * 1000 as usize) as i64
        noeWalEngineExecute(&engine, &tx, "ledger add { amount: " + intToString(amt) + " }")
        noeWalEngineCommit(&engine, &tx)
        i = i + 1 as usize
    }

    file_close(engine.walFd)
    file_close(engine.lockFd)
    print("A_COMMITTED_ALL")

    let fDone: isize = file_open_create("tmp_flag_a_done.txt")
    file_close(fDone as usize)
}
main()
"""

script_b = os.path.join(NOESQL_DIR, "tests", "tmp_ckpt_proc_b.noqeri")
script_a = os.path.join(NOESQL_DIR, "tests", "tmp_ckpt_proc_a.noqeri")
with open(script_b, "w") as f: f.write(proc_b_code)
with open(script_a, "w") as f: f.write(proc_a_code)

for f in ["tmp_flag_b_ready.txt", "tmp_flag_a_done.txt"]:
    if os.path.exists(f): os.remove(f)

# Launch B first (opened early)
p_b = subprocess.Popen([NOQERI_BIN, "run", script_b], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
p_a = subprocess.Popen([NOQERI_BIN, "run", script_a], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

out_b, err_b = p_b.communicate(timeout=30)
out_a, err_a = p_a.communicate(timeout=30)

print("Process A:", out_a.strip())
print("Process B:", out_b.strip())

assert p_a.returncode == 0, f"A failed: {err_a}"
assert p_b.returncode == 0, f"B failed: {err_b}"

# 3. Cold reopen in Process C to verify all records (1000, 2000, 3000, 4000) survived the checkpoint!
verify_code = """import "../Database/noesql_wal_engine.noqeri"

function main(): void {
    let dbPath: string = "tmp_test_ckpt_isolation.noedb"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize { print("OPEN_FAILED") return }

    let tIdx: isize = noeFindTable(&engine.db, "ledger")
    let t: *NoeTableData = &engine.db.tables[tIdx as usize]
    print("TOTAL_RECOVERED:" + intToString(t.rowCount as i64))

    let r: usize = 0 as usize
    while r < t.rowCount {
        let cell: *NoeRowCell = &t.cells[r * 16 as usize + 1 as usize]
        if !cell.isNull {
            print("RECOVERED_AMOUNT:" + intToString(cell.intVal))
        }
        r = r + 1 as usize
    }

    file_close(engine.walFd)
    file_close(engine.lockFd)
}
main()
"""
script_c = os.path.join(NOESQL_DIR, "tests", "tmp_ckpt_proc_c.noqeri")
with open(script_c, "w") as f: f.write(verify_code)

res_c = subprocess.run([NOQERI_BIN, "run", script_c], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Process C (Verification):\n", res_c.stdout.strip())
assert res_c.returncode == 0, f"Verification failed: {res_c.stderr}"

expected = [1000, 2000, 3000, 4000]
recovered = []
for line in res_c.stdout.splitlines():
    if line.startswith("RECOVERED_AMOUNT:"):
        recovered.append(int(line.split(":")[1]))

print(f"Expected: {expected}")
print(f"Recovered: {recovered}")
assert recovered == expected, f"Data lost during checkpoint! Expected {expected}, got {recovered}"
print("=> PASS: Coordinated checkpoint refreshed under lock; zero data overwritten!")

# Cleanup
for f in ["tmp_flag_b_ready.txt", "tmp_flag_a_done.txt", init_script, script_a, script_b, script_c]:
    if os.path.exists(f): os.remove(f)
cleanup()

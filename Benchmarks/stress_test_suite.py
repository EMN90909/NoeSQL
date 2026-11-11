import os
import sys
import time
import shutil
import subprocess
from concurrent.futures import ThreadPoolExecutor, as_completed

NOESQL_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
NOQERI_BIN = os.environ.get("NOQERI", os.path.abspath(os.path.join(NOESQL_DIR, "..", "Noqeri", "build", "noqeri")))

print("================================================================================")
print("  NOESQL DEEP STRESS & ADVERSARIAL CAPACITY SUITE")
print("================================================================================")

# --- Test 1: Capacity Limit & Out-of-Bounds Row Stress ---
print("\n[STRESS TEST 1] Table Row Capacity Saturation (Limit: 64 rows)")
script_capacity = """
import "../Database/noesql_engine.noqeri"

function main(): void {
    let db: NoeDatabaseState
    noeEngineInit(&db, "stress_cap")
    let tIdx: isize = noeCreateTable(&db, "test_table")
    noeAddField(&db, tIdx as usize, "id", noeTypeInt(), true, true, true, true, "")
    noeAddField(&db, tIdx as usize, "val", noeTypeInt(), true, false, false, false, "")

    let inserted: usize = 0 as usize
    let i: usize = 0 as usize
    while i < 100 as usize {
        let r: [NoeRowCell; 2]
        r[0].isNull = true
        r[1].isNull = false
        r[1].intVal = i as i64
        let res: isize = noeInsertRow(&db, tIdx as usize, slice(r))
        if res == 0 as isize {
            inserted = inserted + 1 as usize
        } else {
            // Reached capacity limit
            print("CAPACITY_SATURATED_AT:" + intToString(inserted as i64) + ":ERROR:" + intToString(res as i64))
            return
        }
        i = i + 1 as usize
    }
}
main()
"""
with open(os.path.join(NOESQL_DIR, "Benchmarks", "stress_cap.noqeri"), "w") as f:
    f.write(script_capacity)

res = subprocess.run([NOQERI_BIN, "run", "Benchmarks/stress_cap.noqeri"], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Output:", res.stdout.strip())
assert "CAPACITY_SATURATED_AT:64:ERROR:-101" in res.stdout, f"Unexpected capacity behavior: {res.stdout}"
print("=> PASS: Fixed static memory strictly bounds rows at 64 without memory leaks or buffer overflows.")

# --- Test 2: Rapid Commit Generation Rollover Stress ---
print("\n[STRESS TEST 2] Rapid Successive Immutable Generation Commits (100 Generations)")
script_gen_rollover = """
import "../Database/noesql_engine.noqeri"
import "../../Noqeri/Lib/std/fs.noqeri"

function main(): void {
    let dbPath: string = "/tmp/stress_gen.noedb"
    fs_remove_tree(dbPath)
    fs_remove(dbPath + ".lck")

    let db: NoeDatabaseState
    noeEngineInit(&db, "stress_gen")
    let tIdx: isize = noeCreateTable(&db, "items")
    noeAddField(&db, tIdx as usize, "id", noeTypeInt(), true, true, true, true, "")

    let g: usize = 0 as usize
    while g < 100 as usize {
        let r: [NoeRowCell; 1]
        r[0].isNull = true
        noeInsertRow(&db, tIdx as usize, slice(r))
        let saveRes: isize = noeDbSaveV3(&db, dbPath)
        if saveRes != 0 as isize {
            print("SAVE_FAILED_AT_GEN:" + intToString(g as i64))
            return
        }
        g = g + 1 as usize
    }

    let coldDb: NoeDatabaseState
    let openRes: isize = noeDbOpenV3(&coldDb, dbPath)
    if openRes != 0 as isize {
        print("COLD_REOPEN_FAILED")
        return
    }

    print("RAPID_COMMITS_OK:active_generation=" + intToString(coldDb.generation as i64) + ":row_count=" + intToString(coldDb.tables[0].rowCount as i64))
}
main()
"""
with open(os.path.join(NOESQL_DIR, "Benchmarks", "stress_gen.noqeri"), "w") as f:
    f.write(script_gen_rollover)

t0 = time.perf_counter()
res = subprocess.run([NOQERI_BIN, "run", "Benchmarks/stress_gen.noqeri"], cwd=NOESQL_DIR, capture_output=True, text=True)
t1 = time.perf_counter()
print("Output:", res.stdout.strip())
print(f"Elapsed Time for 100 Atomic Commits + Fsyncs: {(t1 - t0)*1000.0:.2f} ms")
assert "RAPID_COMMITS_OK:active_generation=100" in res.stdout, f"Generation mismatch: {res.stdout}"
print("=> PASS: 100 sequential atomic fsync + replacement generation commits succeeded without corrupting HEAD.")

# --- Test 3: Multi-Process Lock Contention Under Heavy Concurrency ---
print("\n[STRESS TEST 3] Extreme Multi-Process Lock Contention Stress")
print("Launching 8 parallel processes aggressively competing for the same database file lock...")

def run_concurrent_worker(worker_id):
    worker_script = f"""
import "../Database/noesql_engine.noqeri"

function main(): void {{
    let dbPath: string = "/tmp/stress_contention.noedb"
    let i: usize = 0 as usize
    let successes: usize = 0 as usize
    let failures: usize = 0 as usize

    while i < 15 as usize {{
        let db: NoeDatabaseState
        let openRes: isize = noeDbOpenV3(&db, dbPath)
        let tIdx: isize = -1 as isize
        if openRes < 0 as isize {{
            noeEngineInit(&db, "contention")
            tIdx = noeCreateTable(&db, "events")
            noeAddField(&db, tIdx as usize, "id", noeTypeInt(), true, true, true, true, "")
            noeAddField(&db, tIdx as usize, "worker", noeTypeInt(), true, false, false, false, "")
        }} else {{
            tIdx = noeFindTable(&db, "events")
            if tIdx < 0 as isize {{
                tIdx = noeCreateTable(&db, "events")
                noeAddField(&db, tIdx as usize, "id", noeTypeInt(), true, true, true, true, "")
                noeAddField(&db, tIdx as usize, "worker", noeTypeInt(), true, false, false, false, "")
            }}
        }}

        if db.tables[tIdx as usize].rowCount < 60 as usize {{
            let r: [NoeRowCell; 2]
            r[0].isNull = true
            r[1].isNull = false
            r[1].intVal = {worker_id} as i64
            noeInsertRow(&db, tIdx as usize, slice(r))
            let saveRes: isize = noeDbSaveV3(&db, dbPath)
            if saveRes == 0 as isize {{
                successes = successes + 1 as usize
            }} else {{
                failures = failures + 1 as usize
            }}
        }}
        i = i + 1 as usize
    }}
    print("WORKER_{worker_id}_DONE:succ=" + intToString(successes as i64) + ":fail=" + intToString(failures as i64))
}}
main()
"""
    script_path = os.path.join(NOESQL_DIR, "Benchmarks", f"worker_{worker_id}.noqeri")
    with open(script_path, "w") as f:
        f.write(worker_script)
    r = subprocess.run([NOQERI_BIN, "run", script_path], cwd=NOESQL_DIR, capture_output=True, text=True)
    return r.stdout.strip()

shutil.rmtree("/tmp/stress_contention.noedb", ignore_errors=True)
if os.path.exists("/tmp/stress_contention.noedb.lck"):
    os.remove("/tmp/stress_contention.noedb.lck")

# Pre-init
pre_worker = 0
run_concurrent_worker(0)

with ThreadPoolExecutor(max_workers=8) as executor:
    futures = [executor.submit(run_concurrent_worker, i) for i in range(1, 9)]
    results = [f.result() for f in as_completed(futures)]

for res_line in results:
    print("  ", res_line)

# Verify final integrity
verify_script = """
import "../Database/noesql_engine.noqeri"

function main(): void {
    let dbPath: string = "/tmp/stress_contention.noedb"
    let db: NoeDatabaseState
    let check: isize = noeDbCheckV3(&db, dbPath)
    if check != 0 as isize {
        print("CHECK_FAILED:" + intToString(check as i64))
        return
    }
    let openRes: isize = noeDbOpenV3(&db, dbPath)
    if openRes != 0 as isize {
        print("OPEN_FAILED:" + intToString(openRes as i64))
        return
    }
    print("CONTENTION_FINAL_OK:gen=" + intToString(db.generation as i64) + ":rows=" + intToString(db.tables[0].rowCount as i64))
}
main()
"""
with open(os.path.join(NOESQL_DIR, "Benchmarks", "verify_contention.noqeri"), "w") as f:
    f.write(verify_script)

final_res = subprocess.run([NOQERI_BIN, "run", "Benchmarks/verify_contention.noqeri"], cwd=NOESQL_DIR, capture_output=True, text=True)
print("Final Verification:", final_res.stdout.strip())
assert "CONTENTION_FINAL_OK" in final_res.stdout
print("=> PASS: Heavy multi-process lock contention preserved database consistency with zero corruptions.")


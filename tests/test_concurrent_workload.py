import os
import shutil
import subprocess
import sys
import time

NOESQL_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
NOQERI_BIN = os.environ.get("NOQERI", os.path.abspath(os.path.join(NOESQL_DIR, "..", "Noqeri", "build", "noqeri")))
DB_PATH = os.path.join(NOESQL_DIR, "tmp_concurrent_db")
LOCK_PATH = DB_PATH + ".lck"

def cleanup():
    if os.path.exists(DB_PATH):
        shutil.rmtree(DB_PATH, ignore_errors=True)
    if os.path.exists(LOCK_PATH):
        try:
            os.remove(LOCK_PATH)
        except OSError:
            pass

def main():
    print("=== Testing Long-Running Concurrent Workload (Multi-Process Readers & Writers) ===")
    cleanup()

    # Step 1: Pre-initialize database so readers don't fail before first write
    init_cmd = [NOQERI_BIN, "run", "tests/worker_writer.noqeri"]
    res = subprocess.run(init_cmd, cwd=NOESQL_DIR, capture_output=True, text=True)
    if res.returncode != 0 or "WRITER_COMPLETE" not in res.stdout:
        print(f"Failed to pre-initialize database:\nstdout: {res.stdout}\nstderr: {res.stderr}")
        sys.exit(1)

    print("Initial database created. Launching 4 concurrent writers and 4 concurrent readers...")

    # Step 2: Spawn 4 writers and 4 readers concurrently
    processes = []
    num_writers = 4
    num_readers = 4

    for i in range(num_writers):
        p = subprocess.Popen(
            [NOQERI_BIN, "run", "tests/worker_writer.noqeri"],
            cwd=NOESQL_DIR,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )
        processes.append(("writer", i, p))

    for j in range(num_readers):
        p = subprocess.Popen(
            [NOQERI_BIN, "run", "tests/worker_reader.noqeri"],
            cwd=NOESQL_DIR,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )
        processes.append(("reader", j, p))

    # Wait for all processes
    all_ok = True
    for role, idx, p in processes:
        stdout, stderr = p.communicate(timeout=60)
        if p.returncode != 0:
            print(f"Process {role} #{idx} FAILED with exit code {p.returncode}")
            print(f"Stdout: {stdout}\nStderr: {stderr}")
            all_ok = False
        elif "CORRUPTION" in stdout:
            print(f"Process {role} #{idx} detected corruption!\nStdout: {stdout}")
            all_ok = False

    if not all_ok:
        cleanup()
        sys.exit(1)

    # Step 3: Run cold verification
    verify_res = subprocess.run([NOQERI_BIN, "run", "tests/worker_reader.noqeri"], cwd=NOESQL_DIR, capture_output=True, text=True)
    if verify_res.returncode != 0 or "READER_COMPLETE" not in verify_res.stdout:
        print(f"Final cold verification failed:\nstdout: {verify_res.stdout}\nstderr: {verify_res.stderr}")
        cleanup()
        sys.exit(1)

    cleanup()

    # Step 4: WAL Multi-Process Concurrency Test with Independent Transaction Ledger
    print("Testing Concurrent WAL Workload (Multi-Process WAL Writers & Independent Ledger Verification)...")
    wal_db = os.path.join(NOESQL_DIR, "tmp_concurrent_wal_db")
    if os.path.exists(wal_db):
        shutil.rmtree(wal_db, ignore_errors=True)
    if os.path.exists(wal_db + ".lck"):
        try: os.remove(wal_db + ".lck")
        except OSError: pass

    def write_worker_script(w_id):
        script_file = os.path.join(NOESQL_DIR, "tests", f"tmp_worker_wal_{w_id}.noqeri")
        code = f"""import "../Database/noesql_wal_engine.noqeri"

function main(): void {{
    let dbPath: string = "tmp_concurrent_wal_db"
    let engine: NoeWalEngine
    let openRes: isize = noeWalEngineOpen(&engine, dbPath)
    if openRes != 0 as isize {{ print("OPEN_FAILED") return }}

    let tIdx: isize = noeFindTable(&engine.db, "events")
    if tIdx < 0 as isize {{
        let txSchema: NoeWalTx
        noeWalEngineBegin(&engine, &txSchema)
        noeWalEngineExecute(&engine, &txSchema, "table events {{ id: int key auto, data: int }}")
        noeWalEngineCommit(&engine, &txSchema)
    }}

    let wId: i64 = {w_id} as i64
    let i: usize = 0 as usize
    while i < 5 as usize {{
        let tx: NoeWalTx
        let bRes: isize = noeWalEngineBegin(&engine, &tx)
        if bRes != 0 as isize {{ print("BEGIN_FAILED") return }}

        let val: i64 = wId * 100 as i64 + (i as i64)
        let stmt: string = "events add {{ data: " + intToString(val) + " }}"
        let eRes: isize = noeWalEngineExecute(&engine, &tx, stmt)
        if eRes != 0 as isize {{ noeWalEngineRollback(&engine, &tx) print("EXEC_FAILED") return }}

        let cRes: isize = noeWalEngineCommit(&engine, &tx)
        if cRes != 0 as isize {{ print("COMMIT_FAILED") return }}

        print("ACK_WRITE:worker=" + intToString(wId) + ":val=" + intToString(val) + ":tx=" + intToString(tx.txId as i64))
        i = i + 1 as usize
    }}

    file_close(engine.walFd)
    file_close(engine.lockFd)
    print("WAL_WRITER_COMPLETE:worker=" + intToString(wId))
}}
main()
"""
        with open(script_file, "w") as f:
            f.write(code)
        return script_file

    # Pre-init table with worker 0
    init_script = write_worker_script(0)
    init_wal = subprocess.run([NOQERI_BIN, "run", init_script], cwd=NOESQL_DIR, capture_output=True, text=True)
    if init_wal.returncode != 0 or "WAL_WRITER_COMPLETE" not in init_wal.stdout:
        print(f"Failed to pre-init WAL database:\n{init_wal.stdout}\n{init_wal.stderr}")
        sys.exit(1)

    # Collect initial writes
    acknowledged_ledger = set()
    for line in init_wal.stdout.splitlines():
        if line.startswith("ACK_WRITE:"):
            parts = dict(p.split("=") for p in line[len("ACK_WRITE:"):].split(":"))
            acknowledged_ledger.add(int(parts["val"]))

    # Spawn 4 concurrent writers with unique worker IDs and 4 concurrent readers
    wal_procs = []
    num_writers = 4
    for i in range(1, num_writers + 1):
        s_file = write_worker_script(i)
        p = subprocess.Popen([NOQERI_BIN, "run", s_file], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        wal_procs.append(("wal_writer", i, p))

    for j in range(4):
        p = subprocess.Popen([NOQERI_BIN, "run", "tests/worker_wal_reader.noqeri"], cwd=NOESQL_DIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        wal_procs.append(("wal_reader", j, p))

    for role, idx, p in wal_procs:
        stdout, stderr = p.communicate(timeout=60)
        if p.returncode != 0:
            print(f"WAL process {role} #{idx} FAILED with code {p.returncode}: {stderr}")
            all_ok = False
        if role == "wal_writer":
            for line in stdout.splitlines():
                if line.startswith("ACK_WRITE:"):
                    parts = dict(p.split("=") for p in line[len("ACK_WRITE:"):].split(":"))
                    acknowledged_ledger.add(int(parts["val"]))
        elif "CORRUPTION" in stdout:
            print(f"WAL process {role} #{idx} reported corruption: {stdout}")
            all_ok = False

    if not all_ok:
        sys.exit(1)

    print(f"Total Acknowledged Transactions Across Concurrent Writers: {len(acknowledged_ledger)}")

    # Verify cold reopen and exact row match against independent ledger
    verify_reader = subprocess.run([NOQERI_BIN, "run", "tests/worker_wal_reader.noqeri"], cwd=NOESQL_DIR, capture_output=True, text=True)
    if verify_reader.returncode != 0:
        print(f"Cold verification reader failed:\n{verify_reader.stdout}\n{verify_reader.stderr}")
        sys.exit(1)

    recovered_rows = set()
    for line in verify_reader.stdout.splitlines():
        if line.startswith("RECOVERED_ROW_VAL:"):
            val = int(line[len("RECOVERED_ROW_VAL:"):])
            recovered_rows.add(val)

    print(f"Total Recovered Rows from Uncheckpointed Cold WAL Replay: {len(recovered_rows)}")

    missing = acknowledged_ledger - recovered_rows
    extra = recovered_rows - acknowledged_ledger
    if missing or extra or len(recovered_rows) != len(acknowledged_ledger):
        print(f"FATAL: Ledger mismatch! Missing: {missing}, Extra: {extra}")
        sys.exit(1)

    print("=> 100% LEDGER MATCH: Every acknowledged write was recovered with exact identity and zero loss.")

    if os.path.exists(wal_db):
        shutil.rmtree(wal_db, ignore_errors=True)
    if os.path.exists(wal_db + ".lck"):
        try: os.remove(wal_db + ".lck")
        except OSError: pass

    print("Long-Running Concurrent Workload (Multi-Process Readers & Writers, COW + WAL): ALL PASSED (Zero Corruption, 100% Consistency)")

if __name__ == "__main__":
    main()

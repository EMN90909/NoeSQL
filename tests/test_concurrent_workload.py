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
    print("Long-Running Concurrent Workload (Multi-Process Readers & Writers): ALL PASSED (Zero Corruption, 100% Consistency)")

if __name__ == "__main__":
    main()

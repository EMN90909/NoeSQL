import os
import sys
import time
import shutil
import sqlite3
import psycopg2
import pymongo
import mysql.connector
import subprocess
from concurrent.futures import ThreadPoolExecutor, as_completed

NOQERI_BIN = "/home/antigravity/EMN90909/Noqeri/build/noqeri"
NOESQL_DIR = "/home/antigravity/EMN90909/NoeSQL"

print("================================================================================")
print("  MULTI-DATABASE BENCHMARK & COMPARISON HARNESS")
print("  Engines: NoeSQL (Pure Noqeri) vs SQLite vs PostgreSQL vs MongoDB vs MySQL")
print("================================================================================")

# --- 1. SQLite Benchmark ---
def run_sqlite_bench(num_rows=50, num_runs=10):
    latencies_insert = []
    latencies_query = []
    db_file = "/tmp/bench_sqlite.db"
    
    for _ in range(num_runs):
        if os.path.exists(db_file):
            os.remove(db_file)
        
        t0 = time.perf_counter()
        conn = sqlite3.connect(db_file)
        cur = conn.cursor()
        cur.execute("PRAGMA synchronous = FULL;")
        cur.execute("CREATE TABLE records (id INTEGER PRIMARY KEY AUTOINCREMENT, val INTEGER, name TEXT);")
        for i in range(num_rows):
            cur.execute("INSERT INTO records (val, name) VALUES (?, ?);", (i * 10, "test_record"))
        conn.commit()
        t1 = time.perf_counter()
        latencies_insert.append((t1 - t0) * 1000.0) # ms
        
        t2 = time.perf_counter()
        cur.execute("SELECT count(*) FROM records WHERE val >= 250;")
        res = cur.fetchone()[0]
        conn.close()
        t3 = time.perf_counter()
        latencies_query.append((t3 - t2) * 1000.0) # ms
        
    avg_ins = sum(latencies_insert) / len(latencies_insert)
    avg_qry = sum(latencies_query) / len(latencies_query)
    return avg_ins, avg_qry

# --- 2. PostgreSQL Benchmark ---
def run_postgres_bench(num_rows=50, num_runs=10):
    latencies_insert = []
    latencies_query = []
    
    for _ in range(num_runs):
        conn = psycopg2.connect(dbname="postgres", port=5433, host="/tmp")
        cur = conn.cursor()
        cur.execute("DROP TABLE IF EXISTS records;")
        cur.execute("CREATE TABLE records (id SERIAL PRIMARY KEY, val INTEGER, name TEXT);")
        conn.commit()
        
        t0 = time.perf_counter()
        for i in range(num_rows):
            cur.execute("INSERT INTO records (val, name) VALUES (%s, %s);", (i * 10, "test_record"))
        conn.commit()
        t1 = time.perf_counter()
        latencies_insert.append((t1 - t0) * 1000.0)
        
        t2 = time.perf_counter()
        cur.execute("SELECT count(*) FROM records WHERE val >= 250;")
        res = cur.fetchone()[0]
        t3 = time.perf_counter()
        latencies_query.append((t3 - t2) * 1000.0)
        conn.close()
        
    avg_ins = sum(latencies_insert) / len(latencies_insert)
    avg_qry = sum(latencies_query) / len(latencies_query)
    return avg_ins, avg_qry

# --- 3. MongoDB Benchmark ---
def run_mongo_bench(num_rows=50, num_runs=10):
    latencies_insert = []
    latencies_query = []
    client = pymongo.MongoClient("mongodb://localhost:27018")
    db = client["bench_db"]
    
    for _ in range(num_runs):
        col = db["records"]
        col.drop()
        
        t0 = time.perf_counter()
        docs = [{"id": i + 1, "val": i * 10, "name": "test_record"} for i in range(num_rows)]
        col.insert_many(docs)
        t1 = time.perf_counter()
        latencies_insert.append((t1 - t0) * 1000.0)
        
        t2 = time.perf_counter()
        count = col.count_documents({"val": {"$gte": 250}})
        t3 = time.perf_counter()
        latencies_query.append((t3 - t2) * 1000.0)
        
    avg_ins = sum(latencies_insert) / len(latencies_insert)
    avg_qry = sum(latencies_query) / len(latencies_query)
    return avg_ins, avg_qry

# --- 4. MySQL Benchmark ---
def run_mysql_bench(num_rows=50, num_runs=10):
    latencies_insert = []
    latencies_query = []
    
    for _ in range(num_runs):
        conn = mysql.connector.connect(user="root", unix_socket="/tmp/mysql.sock")
        cur = conn.cursor()
        cur.execute("CREATE DATABASE IF NOT EXISTS bench_db;")
        cur.execute("USE bench_db;")
        cur.execute("DROP TABLE IF EXISTS records;")
        cur.execute("CREATE TABLE records (id INT AUTO_INCREMENT PRIMARY KEY, val INT, name VARCHAR(50));")
        conn.commit()
        
        t0 = time.perf_counter()
        for i in range(num_rows):
            cur.execute("INSERT INTO records (val, name) VALUES (%s, %s);", (i * 10, "test_record"))
        conn.commit()
        t1 = time.perf_counter()
        latencies_insert.append((t1 - t0) * 1000.0)
        
        t2 = time.perf_counter()
        cur.execute("SELECT count(*) FROM records WHERE val >= 250;")
        res = cur.fetchone()[0]
        t3 = time.perf_counter()
        latencies_query.append((t3 - t2) * 1000.0)
        conn.close()
        
    avg_ins = sum(latencies_insert) / len(latencies_insert)
    avg_qry = sum(latencies_query) / len(latencies_query)
    return avg_ins, avg_qry

# --- 5. NoeSQL (Pure Noqeri) Benchmark ---
def run_noesql_bench(num_rows=50, num_runs=10):
    latencies_total = []
    for _ in range(num_runs):
        t0 = time.perf_counter()
        res = subprocess.run([NOQERI_BIN, "run", "Benchmarks/bench_noesql_worker.noqeri"], cwd=NOESQL_DIR, capture_output=True, text=True)
        t1 = time.perf_counter()
        if res.returncode != 0 or "NOESQL_BENCH_OK" not in res.stdout:
            print("NoeSQL bench run failed:", res.stdout, res.stderr)
            sys.exit(1)
        latencies_total.append((t1 - t0) * 1000.0)
    avg_total = sum(latencies_total) / len(latencies_total)
    return avg_total

print("Executing benchmarks across engines...")
sq_ins, sq_qry = run_sqlite_bench()
print(f"[*] SQLite:       Insert 50 rows + Commit: {sq_ins:.2f} ms | Query/Scan: {sq_qry:.2f} ms")

pg_ins, pg_qry = run_postgres_bench()
print(f"[*] PostgreSQL:   Insert 50 rows + Commit: {pg_ins:.2f} ms | Query/Scan: {pg_qry:.2f} ms")

mg_ins, mg_qry = run_mongo_bench()
print(f"[*] MongoDB:      Insert 50 docs:          {mg_ins:.2f} ms | Query/Count: {mg_qry:.2f} ms")

my_ins, my_qry = run_mysql_bench()
print(f"[*] MySQL:        Insert 50 rows + Commit: {my_ins:.2f} ms | Query/Scan: {my_qry:.2f} ms")

noe_tot = run_noesql_bench()
print(f"[*] NoeSQL:       Total (Compiler + Init + 50 Inserts + Disk Commit + Cold Reopen + Query): {noe_tot:.2f} ms")


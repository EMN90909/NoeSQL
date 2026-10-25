//! Reproducible *native AOT* NoeSQL vs SQLite WAL durability microbenchmark.
//! Both engines update and read back one boolean row on disk in each cycle,
//! closing and reopening between write and read, and checkpointing every 7th
//! update. No benchmark result is printed unless correctness checks succeed.
//! This is an intentionally narrow single-row workload, NOT general SQL parity.
#[cfg(feature = "noqeri-native")]
mod bench {
    use noesql_runtime::{noqeri_native, zig_pager};
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::path::{Path, PathBuf};
    use std::ptr::{self, NonNull};
    use std::time::{Duration, Instant};

    const CYCLES: usize = 96;
    const REPEATS: usize = 3;
    const SQLITE_OK: c_int = 0;
    const SQLITE_ROW: c_int = 100;

    #[link(name = "sqlite3")]
    unsafe extern "C" {
        fn sqlite3_open(filename: *const c_char, handle: *mut *mut c_void) -> c_int;
        fn sqlite3_close(handle: *mut c_void) -> c_int;
        fn sqlite3_errmsg(handle: *mut c_void) -> *const c_char;
        fn sqlite3_exec(
            handle: *mut c_void, sql: *const c_char,
            callback: Option<unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int>,
            context: *mut c_void, error: *mut *mut c_char,
        ) -> c_int;
        fn sqlite3_prepare_v2(
            handle: *mut c_void, sql: *const c_char, nbytes: c_int,
            stmt: *mut *mut c_void, tail: *mut *const c_char,
        ) -> c_int;
        fn sqlite3_step(stmt: *mut c_void) -> c_int;
        fn sqlite3_column_int(stmt: *mut c_void, col: c_int) -> c_int;
        fn sqlite3_finalize(stmt: *mut c_void) -> c_int;
    }

    struct Sqlite(NonNull<c_void>);
    impl Sqlite {
        fn open(path: &Path) -> Result<Self, String> {
            let name = CString::new(path.to_str().ok_or("invalid path")?)
                .map_err(|e| e.to_string())?;
            let mut raw: *mut c_void = ptr::null_mut();
            let code = unsafe { sqlite3_open(name.as_ptr(), &mut raw) };
            if code != SQLITE_OK {
                let detail = if raw.is_null() {
                    format!("SQLite open failed ({code})")
                } else {
                    unsafe { CStr::from_ptr(sqlite3_errmsg(raw)) }
                        .to_string_lossy().into_owned()
                };
                if !raw.is_null() { unsafe { sqlite3_close(raw); } }
                return Err(detail);
            }
            NonNull::new(raw).map(Self).ok_or("SQLite returned null handle".to_string())
        }
        fn ptr(&self) -> *mut c_void { self.0.as_ptr() }
        fn error(&self) -> String {
            unsafe { CStr::from_ptr(sqlite3_errmsg(self.ptr())) }.to_string_lossy().into_owned()
        }
        fn exec(&self, sql: &str) -> Result<(), String> {
            let text = CString::new(sql).map_err(|e| e.to_string())?;
            let code = unsafe {
                sqlite3_exec(self.ptr(), text.as_ptr(), None, ptr::null_mut(), ptr::null_mut())
            };
            if code != SQLITE_OK { return Err(format!("SQLite SQL error {code}: {}", self.error())); }
            Ok(())
        }
        fn select_enabled(&self) -> Result<bool, String> {
            let query = CString::new("SELECT enabled FROM rows WHERE id=44")
                .map_err(|e| e.to_string())?;
            let mut raw: *mut c_void = ptr::null_mut();
            let code = unsafe {
                sqlite3_prepare_v2(self.ptr(), query.as_ptr(), -1, &mut raw, ptr::null_mut())
            };
            if code != SQLITE_OK {
                return Err(format!("SQLite prepare: {}", self.error()));
            }
            let stmt = NonNull::new(raw).ok_or("SQLite prepare returned null".to_string())?;
            let status = unsafe { sqlite3_step(stmt.as_ptr()) };
            let value = if status == SQLITE_ROW {
                let n = unsafe { sqlite3_column_int(stmt.as_ptr(), 0) };
                if !(0..=1).contains(&n) {
                    unsafe { sqlite3_finalize(stmt.as_ptr()) };
                    return Err(format!("SQLite invalid bool {n}"));
                }
                n == 1
            } else {
                unsafe { sqlite3_finalize(stmt.as_ptr()) };
                return Err(format!("SQLite select returned {status}: {}", self.error()));
            };
            let finalized = unsafe { sqlite3_finalize(stmt.as_ptr()) };
            if finalized != SQLITE_OK { return Err(format!("SQLite finalize: {}", self.error())); }
            Ok(value)
        }
        fn integrity_check(&self) -> Result<(), String> {
            let query = CString::new("PRAGMA integrity_check").unwrap();
            let mut raw = ptr::null_mut();
            let status = unsafe {
                sqlite3_prepare_v2(self.ptr(), query.as_ptr(), -1, &mut raw, ptr::null_mut())
            };
            if status != SQLITE_OK { return Err(self.error()); }
            let stmt = NonNull::new(raw).ok_or("null integrity statement")?;
            let row_status = unsafe { sqlite3_step(stmt.as_ptr()) };
            let ok = if row_status == SQLITE_ROW {
                unsafe {
                    let ptr = sqlite3_column_text(stmt.as_ptr(), 0);
                    !ptr.is_null() && CStr::from_ptr(ptr.cast::<c_char>()).to_bytes() == b"ok"
                }
            } else { false };
            unsafe { sqlite3_finalize(stmt.as_ptr()); }
            if ok { Ok(()) } else { Err("SQLite integrity_check failed".into()) }
        }
    }
    impl Drop for Sqlite {
        fn drop(&mut self) { let _ = unsafe { sqlite3_close(self.ptr()) }; }
    }
    unsafe extern "C" {
        fn sqlite3_column_text(stmt: *mut c_void, col: c_int) -> *const u8;
    }

    fn remove(path: &Path, sqlite: bool) {
        let _ = std::fs::remove_file(path);
        for suffix in if sqlite { &["-wal", "-shm"][..] } else { &[".wal", ".lck", ".checkpoint.tmp"][..] } {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }
    fn path(prefix: &str, pass: usize, sqlite: bool) -> PathBuf {
        std::env::temp_dir().join(format!(
            "noesql-bench-{prefix}-{}-{pass}.{}",
            std::process::id(), if sqlite { "sqlite3" } else { "noedb" }
        ))
    }

    fn bench_noesql(pass: usize) -> Result<Duration, String> {
        let path = path("native", pass, false);
        remove(&path, false);
        if noqeri_native::api_version() != 2 {
            return Err("Noqeri native bridge ABI version mismatch".into());
        }
        noqeri_native::insert_bool_row(&path, 3, 44, false).map_err(|e| e.to_string())?;
        let start = Instant::now();
        for i in 0..CYCLES {
            let enabled = i % 2 == 0;
            noqeri_native::set_bool_row(&path, 3, 44, enabled).map_err(|e| e.to_string())?;
            if i % 7 == 0 {
                zig_pager::checkpoint(&path).map_err(|e| e.to_string())?;
            }
            let readback = noqeri_native::get_bool_row(&path, 3, 44)
                .map_err(|e| e.to_string())?;
            if readback != Some(enabled) {
                return Err(format!("NoeSQL data mismatch on cycle {i}: {readback:?}"));
            }
        }
        let elapsed = start.elapsed();
        if noqeri_native::get_bool_row(&path, 3, 44).map_err(|e| e.to_string())? != Some(false) {
            return Err("NoeSQL final durable value mismatch".into());
        }
        remove(&path, false);
        Ok(elapsed)
    }

    fn bench_sqlite(pass: usize) -> Result<Duration, String> {
        let path = path("sqlite", pass, true);
        remove(&path, true);
        {
            let db = Sqlite::open(&path)?;
            db.exec("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
            db.exec("CREATE TABLE rows(id INTEGER PRIMARY KEY, enabled INTEGER NOT NULL CHECK(enabled IN (0,1)));")?;
            db.exec("INSERT INTO rows(id,enabled) VALUES(44,0);")?;
        }
        let start = Instant::now();
        for i in 0..CYCLES {
            let enabled = i % 2 == 0;
            {
                let db = Sqlite::open(&path)?;
                db.exec("PRAGMA synchronous=FULL;")?;
                db.exec(&format!("UPDATE rows SET enabled={} WHERE id=44;", u8::from(enabled)))?;
                if i % 7 == 0 {
                    db.exec("PRAGMA wal_checkpoint(TRUNCATE);")?;
                }
            }
            let db = Sqlite::open(&path)?;
            let got = db.select_enabled()?;
            if got != enabled {
                return Err(format!("SQLite data mismatch on cycle {i}: {got}"));
            }
        }
        let elapsed = start.elapsed();
        let db = Sqlite::open(&path)?;
        db.integrity_check()?;
        if db.select_enabled()? {
            return Err("SQLite final durable value mismatch".into());
        }
        drop(db);
        remove(&path, true);
        Ok(elapsed)
    }

    pub fn run() -> Result<(), String> {
        println!("Durable single-row benchmark: Noqeri AOT→Rust→Zig vs SQLite WAL/FULL");
        println!("cycles={CYCLES}, passes={REPEATS}; includes write, checkpoint every 7, cold reopen and verification");
        println!("pass,engine,elapsed_ms,verified_cycles,cycles_per_second");
        let mut noesql = Vec::with_capacity(REPEATS);
        let mut sqlite = Vec::with_capacity(REPEATS);
        for pass in 0..REPEATS {
            // Alternate order to reduce warm-cache/systematic scheduling bias.
            let pairs = if pass % 2 == 0 {
                [("NoeSQL-native", bench_noesql(pass)?), ("SQLite-WAL-FULL", bench_sqlite(pass)?)]
            } else {
                [("SQLite-WAL-FULL", bench_sqlite(pass)?), ("NoeSQL-native", bench_noesql(pass)?)]
            };
            for (name, duration) in pairs {
                let ms = duration.as_secs_f64() * 1_000.0;
                println!("{},{},{:.3},{},{}", pass + 1, name, ms, CYCLES,
                    (CYCLES as f64 / duration.as_secs_f64()).round());
                if name == "NoeSQL-native" { noesql.push(ms); } else { sqlite.push(ms); }
            }
        }
        noesql.sort_by(|a,b| a.total_cmp(b));
        sqlite.sort_by(|a,b| a.total_cmp(b));
        let mid = REPEATS / 2;
        println!("median_ms,NoeSQL-native,{:.3}", noesql[mid]);
        println!("median_ms,SQLite-WAL-FULL,{:.3}", sqlite[mid]);
        println!("NoeSQL/SQLite latency ratio: {:.3}x (lower is faster)", noesql[mid] / sqlite[mid]);
        println!("Scope: a single bool-row semantic and WAL persistence path; not full SQL or general throughput.");
        Ok(())
    }
}

#[cfg(feature = "noqeri-native")]
fn main() {
    if let Err(error) = bench::run() {
        eprintln!("benchmark failed correctness/IO checks: {error}");
        std::process::exit(1);
    }
}
#[cfg(not(feature = "noqeri-native"))]
fn main() {
    eprintln!("Build native Noqeri AOT and Zig libraries, then run with --features noqeri-native");
    std::process::exit(2);
}

use core::ffi::{c_char, c_int, c_void};
use std::env;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

const ZIG_ABI_VERSION: u32 = 2;
const PAGE_BYTES: usize = 4096;
const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;

#[repr(C)]
struct Sqlite3 {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Stmt {
    _private: [u8; 0],
}

#[link(name = "sqlite3")]
unsafe extern "C" {
    fn sqlite3_open(filename: *const c_char, db: *mut *mut Sqlite3) -> c_int;
    fn sqlite3_close(db: *mut Sqlite3) -> c_int;
    fn sqlite3_errmsg(db: *mut Sqlite3) -> *const c_char;
    fn sqlite3_exec(
        db: *mut Sqlite3,
        sql: *const c_char,
        callback: Option<
            unsafe extern "C" fn(
                *mut c_void,
                c_int,
                *mut *mut c_char,
                *mut *mut c_char,
            ) -> c_int,
        >,
        argument: *mut c_void,
        error_message: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_prepare_v2(
        db: *mut Sqlite3,
        sql: *const c_char,
        bytes: c_int,
        statement: *mut *mut Sqlite3Stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_step(statement: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_finalize(statement: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_reset(statement: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_clear_bindings(statement: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_bind_int64(statement: *mut Sqlite3Stmt, index: c_int, value: i64) -> c_int;
    fn sqlite3_bind_blob(
        statement: *mut Sqlite3Stmt,
        index: c_int,
        value: *const c_void,
        bytes: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_column_text(statement: *mut Sqlite3Stmt, column: c_int) -> *const u8;
    fn sqlite3_column_int(statement: *mut Sqlite3Stmt, column: c_int) -> c_int;
}

unsafe extern "C" {
    fn noesql_zig_page_open(
        version: u32,
        path: *const c_char,
        output: *mut *mut c_void,
    ) -> c_int;
    fn noesql_zig_page_close(handle: *mut c_void);
    fn noesql_zig_page_stage(
        handle: *mut c_void,
        page_id: u32,
        bytes: *const u8,
        count: usize,
    ) -> c_int;
    fn noesql_zig_page_commit(handle: *mut c_void) -> c_int;
}

fn cstring(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| format!("string contains NUL: {value:?}"))
}

fn sqlite_error(db: *mut Sqlite3, action: &str, code: c_int) -> String {
    let message = if db.is_null() {
        "database handle unavailable".to_string()
    } else {
        unsafe {
            let ptr = sqlite3_errmsg(db);
            if ptr.is_null() {
                "unknown SQLite error".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    };
    format!("{action} failed with SQLite code {code}: {message}")
}

fn sqlite_check(db: *mut Sqlite3, action: &str, code: c_int) -> Result<(), String> {
    if code == SQLITE_OK || code == SQLITE_DONE {
        Ok(())
    } else {
        Err(sqlite_error(db, action, code))
    }
}

fn sqlite_exec(db: *mut Sqlite3, sql: &str) -> Result<(), String> {
    let sql = cstring(sql)?;
    let code = unsafe {
        sqlite3_exec(
            db,
            sql.as_ptr(),
            None,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    sqlite_check(db, "sqlite3_exec", code)
}

struct SqliteDb(*mut Sqlite3);

impl SqliteDb {
    fn open(path: &Path) -> Result<Self, String> {
        let path = path
            .to_str()
            .ok_or_else(|| "SQLite benchmark path is not UTF-8".to_string())?;
        let path = cstring(path)?;
        let mut db = ptr::null_mut();
        let code = unsafe { sqlite3_open(path.as_ptr(), &mut db) };
        if code != SQLITE_OK || db.is_null() {
            return Err(sqlite_error(db, "sqlite3_open", code));
        }
        Ok(Self(db))
    }

    fn ptr(&self) -> *mut Sqlite3 {
        self.0
    }
}

impl Drop for SqliteDb {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = sqlite3_close(self.0);
            }
        }
    }
}

struct Statement {
    db: *mut Sqlite3,
    ptr: *mut Sqlite3Stmt,
}

impl Statement {
    fn prepare(db: *mut Sqlite3, sql: &str) -> Result<Self, String> {
        let sql = cstring(sql)?;
        let mut statement = ptr::null_mut();
        let code = unsafe {
            sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut statement, ptr::null_mut())
        };
        if code != SQLITE_OK || statement.is_null() {
            return Err(sqlite_error(db, "sqlite3_prepare_v2", code));
        }
        Ok(Self { db, ptr: statement })
    }

    fn step(&self) -> Result<c_int, String> {
        let code = unsafe { sqlite3_step(self.ptr) };
        if code == SQLITE_ROW || code == SQLITE_DONE {
            Ok(code)
        } else {
            Err(sqlite_error(self.db, "sqlite3_step", code))
        }
    }

    fn reset(&self) -> Result<(), String> {
        sqlite_check(self.db, "sqlite3_reset", unsafe { sqlite3_reset(self.ptr) })?;
        sqlite_check(
            self.db,
            "sqlite3_clear_bindings",
            unsafe { sqlite3_clear_bindings(self.ptr) },
        )
    }
}

impl Drop for Statement {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                let _ = sqlite3_finalize(self.ptr);
            }
        }
    }
}

struct ZigPager(*mut c_void);

impl ZigPager {
    fn open(path: &Path) -> Result<Self, String> {
        let path = path
            .to_str()
            .ok_or_else(|| "Zig benchmark path is not UTF-8".to_string())?;
        let path = cstring(path)?;
        let mut handle = ptr::null_mut();
        let code = unsafe { noesql_zig_page_open(ZIG_ABI_VERSION, path.as_ptr(), &mut handle) };
        if code != 0 || handle.is_null() {
            return Err(format!("Zig pager open failed with status {code}"));
        }
        Ok(Self(handle))
    }

    fn write_page(&self, page_id: u32, page: &[u8; PAGE_BYTES]) -> Result<(), String> {
        let staged = unsafe {
            noesql_zig_page_stage(self.0, page_id, page.as_ptr(), page.len())
        };
        if staged != 0 {
            return Err(format!("Zig pager stage failed with status {staged}"));
        }
        let committed = unsafe { noesql_zig_page_commit(self.0) };
        if committed != 0 {
            return Err(format!("Zig pager commit failed with status {committed}"));
        }
        Ok(())
    }
}

impl Drop for ZigPager {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { noesql_zig_page_close(self.0) };
        }
    }
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let last = sorted.len().saturating_sub(1);
    let index = ((last as f64) * fraction).floor() as usize;
    sorted[index.min(last)]
}

fn report(name: &str, samples: &mut [Duration]) {
    samples.sort_unstable();
    let total: f64 = samples.iter().map(Duration::as_secs_f64).sum();
    let rate = if total > 0.0 {
        samples.len() as f64 / total
    } else {
        f64::INFINITY
    };
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "{name},transactions={},txn_per_s={rate:.2},p50_ms={:.4},p95_ms={:.4},p99_ms={:.4}",
        samples.len(),
        ms(percentile(samples, 0.50)),
        ms(percentile(samples, 0.95)),
        ms(percentile(samples, 0.99)),
    );
}

fn parse_args() -> Result<(PathBuf, usize), String> {
    let mut args = env::args().skip(1);
    let workdir = PathBuf::from(
        args.next()
            .ok_or_else(|| "usage: benchmark_compare_sqlite <workdir> [transactions]".to_string())?,
    );
    let count = match args.next() {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "transactions must be an integer".to_string())?,
        None => 250,
    };
    if args.next().is_some() {
        return Err("usage: benchmark_compare_sqlite <workdir> [transactions]".to_string());
    }
    if !(10..=5000).contains(&count) {
        return Err("transactions must be between 10 and 5000".to_string());
    }
    fs::create_dir_all(&workdir)
        .map_err(|error| format!("cannot create {}: {error}", workdir.display()))?;
    Ok((workdir, count))
}

fn run() -> Result<(), String> {
    if env::consts::OS != "linux" {
        return Err("the Zig pager benchmark is currently qualified only on Linux".to_string());
    }

    let (workdir, count) = parse_args()?;
    let zig_path = workdir.join("native.noedb");
    let sqlite_path = workdir.join("sqlite.sqlite");
    let _ = fs::remove_file(&zig_path);
    let _ = fs::remove_file(format!("{}.wal", zig_path.display()));
    let _ = fs::remove_file(format!("{}.lck", zig_path.display()));
    let _ = fs::remove_file(&sqlite_path);
    let _ = fs::remove_file(format!("{}-wal", sqlite_path.display()));
    let _ = fs::remove_file(format!("{}-shm", sqlite_path.display()));

    let pager = ZigPager::open(&zig_path)?;
    let mut zig_samples = Vec::with_capacity(count);
    let mut page = [0u8; PAGE_BYTES];

    for index in 0..count {
        page[0..8].copy_from_slice(&(index as u64).to_le_bytes());
        let start = Instant::now();
        pager.write_page((index % 128 + 1) as u32, &page)?;
        zig_samples.push(start.elapsed());
    }
    drop(pager);

    let db = SqliteDb::open(&sqlite_path)?;
    sqlite_exec(db.ptr(), "PRAGMA journal_mode=WAL")?;
    sqlite_exec(db.ptr(), "PRAGMA synchronous=FULL")?;
    sqlite_exec(
        db.ptr(),
        "CREATE TABLE pages(id INTEGER PRIMARY KEY, body BLOB NOT NULL)",
    )?;

    let mode = Statement::prepare(db.ptr(), "PRAGMA journal_mode")?;
    if mode.step()? != SQLITE_ROW {
        return Err("SQLite did not return a journal mode".to_string());
    }
    let mode_ptr = unsafe { sqlite3_column_text(mode.ptr, 0) };
    if mode_ptr.is_null()
        || unsafe { CStr::from_ptr(mode_ptr.cast()) }
            .to_string_lossy()
            .to_ascii_lowercase()
            != "wal"
    {
        return Err("SQLite WAL mode is unavailable".to_string());
    }
    drop(mode);

    let sync = Statement::prepare(db.ptr(), "PRAGMA synchronous")?;
    if sync.step()? != SQLITE_ROW || unsafe { sqlite3_column_int(sync.ptr, 0) } != 2 {
        return Err("SQLite synchronous=FULL is unavailable".to_string());
    }
    drop(sync);

    let upsert = Statement::prepare(
        db.ptr(),
        "INSERT INTO pages(id,body) VALUES(?,?) \
         ON CONFLICT(id) DO UPDATE SET body=excluded.body",
    )?;
    let mut sqlite_samples = Vec::with_capacity(count);

    for index in 0..count {
        page[0..8].copy_from_slice(&(index as u64).to_le_bytes());
        let start = Instant::now();
        sqlite_exec(db.ptr(), "BEGIN IMMEDIATE")?;
        sqlite_check(
            db.ptr(),
            "sqlite3_bind_int64",
            unsafe { sqlite3_bind_int64(upsert.ptr, 1, (index % 128 + 1) as i64) },
        )?;
        sqlite_check(
            db.ptr(),
            "sqlite3_bind_blob",
            unsafe {
                sqlite3_bind_blob(
                    upsert.ptr,
                    2,
                    page.as_ptr().cast(),
                    PAGE_BYTES as c_int,
                    None,
                )
            },
        )?;
        if upsert.step()? != SQLITE_DONE {
            return Err("SQLite upsert did not finish".to_string());
        }
        upsert.reset()?;
        sqlite_exec(db.ptr(), "COMMIT")?;
        sqlite_samples.push(start.elapsed());
    }

    report("Zig-pager-v2", &mut zig_samples);
    report("SQLite-WAL-FULL", &mut sqlite_samples);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

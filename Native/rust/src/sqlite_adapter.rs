//! Optional live SQLite import from a NoeSQL logical snapshot.
//! Uses sqlite3 C ABI directly; never synthesizes individual SQL from user
//! parameters. Refuses to overwrite an existing target. It is an OFFLINE
//! one-way migration for the portable export subset, not change-data capture.
use crate::interop::{export_sql, Dialect, InteropError};
use crate::reference::Database;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::ptr::NonNull;

#[repr(C)]
struct RawSqlite { _private: [u8; 0] }

#[link(name = "sqlite3")]
unsafe extern "C" {
    fn sqlite3_open_v2(filename: *const c_char, db: *mut *mut RawSqlite,
        flags: c_int, vfs: *const c_char) -> c_int;
    fn sqlite3_close(db: *mut RawSqlite) -> c_int;
    fn sqlite3_exec(db: *mut RawSqlite, sql: *const c_char,
        callback: Option<unsafe extern "C" fn(*mut c_void, c_int,
            *mut *mut c_char, *mut *mut c_char) -> c_int>,
        data: *mut c_void, errmsg: *mut *mut c_char) -> c_int;
    fn sqlite3_free(ptr: *mut c_void);
    fn sqlite3_errmsg(db: *mut RawSqlite) -> *const c_char;
    fn sqlite3_prepare_v2(db: *mut RawSqlite, sql: *const c_char,
        length: c_int, stmt: *mut *mut c_void, tail: *mut *const c_char) -> c_int;
    fn sqlite3_step(stmt: *mut c_void) -> c_int;
    fn sqlite3_column_int64(stmt: *mut c_void, index: c_int) -> i64;
    fn sqlite3_column_text(stmt: *mut c_void, index: c_int) -> *const u8;
    fn sqlite3_finalize(stmt: *mut c_void) -> c_int;
}

const OK: c_int = 0;
const ROW: c_int = 100;
const DONE: c_int = 101;
const OPEN_READWRITE: c_int = 2;
const OPEN_CREATE: c_int = 4;
const OPEN_READONLY: c_int = 1;
const OPEN_NOMUTEX: c_int = 0x8000;

struct Sqlite(NonNull<RawSqlite>);
impl Sqlite {
    fn open(path: &Path, flags: c_int) -> Result<Self, InteropError> {
        let text = path.to_str().ok_or_else(|| InteropError("SQLite path is not valid UTF8".into()))?;
        let name = CString::new(text).map_err(|_| InteropError("SQLite path includes NUL".into()))?;
        let mut ptr = std::ptr::null_mut();
        let status = unsafe { sqlite3_open_v2(name.as_ptr(), &mut ptr, flags | OPEN_NOMUTEX, std::ptr::null()) };
        if status != OK {
            if !ptr.is_null() { unsafe { sqlite3_close(ptr) }; }
            return Err(InteropError(format!("sqlite3_open_v2 status {status}")));
        }
        Ok(Self(NonNull::new(ptr).ok_or_else(|| InteropError("SQLite connection is null".into()))?))
    }
    fn errmsg(&self) -> String {
        unsafe { CStr::from_ptr(sqlite3_errmsg(self.0.as_ptr())) }.to_string_lossy().into_owned()
    }
    fn exec(&self, sql: &str) -> Result<(), InteropError> {
        let sql = CString::new(sql).map_err(|_| InteropError("SQL contains NUL".into()))?;
        let mut message = std::ptr::null_mut();
        let status = unsafe { sqlite3_exec(self.0.as_ptr(), sql.as_ptr(), None, std::ptr::null_mut(), &mut message) };
        if status == OK { return Ok(()); }
        let reason = if message.is_null() {
            self.errmsg()
        } else {
            let text = unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned();
            unsafe { sqlite3_free(message.cast()) };
            text
        };
        Err(InteropError(format!("SQLite status {status}: {reason}")))
    }
    fn scalar(&self, query: &str) -> Result<Scalar, InteropError> {
        let query = CString::new(query).map_err(|_| InteropError("query contains NUL".into()))?;
        let mut statement: *mut c_void = std::ptr::null_mut();
        let code = unsafe { sqlite3_prepare_v2(self.0.as_ptr(), query.as_ptr(), -1,
            &mut statement, std::ptr::null_mut()) };
        if code != OK { return Err(InteropError(self.errmsg())); }
        let stmt = NonNull::new(statement).ok_or_else(|| InteropError("empty SQL scalar query".into()))?;
        let result = (|| {
            let step = unsafe { sqlite3_step(stmt.as_ptr()) };
            if step != ROW { return Err(InteropError(format!("SQLite expected row; status {step}"))); }
            let value = if query.as_bytes().starts_with(b"PRAGMA integrity_check") {
                let text = unsafe { sqlite3_column_text(stmt.as_ptr(), 0) };
                if text.is_null() { return Err(InteropError("SQLite integrity check returned NULL".into())); }
                Scalar::Text(unsafe { CStr::from_ptr(text.cast()) }.to_string_lossy().into_owned())
            } else {
                Scalar::Int(unsafe { sqlite3_column_int64(stmt.as_ptr(), 0) })
            };
            if unsafe { sqlite3_step(stmt.as_ptr()) } != DONE {
                return Err(InteropError("SQLite query returned multiple rows or failed".into()));
            }
            Ok(value)
        })();
        let close = unsafe { sqlite3_finalize(stmt.as_ptr()) };
        if close != OK { return Err(InteropError(format!("SQLite finalize status {close}"))); }
        result
    }
}
impl Drop for Sqlite {
    fn drop(&mut self) { let _ = unsafe { sqlite3_close(self.0.as_ptr()) }; }
}
#[derive(Debug, PartialEq, Eq)]
enum Scalar { Int(i64), Text(String) }

fn identifier(name: &str) -> String { format!("\"{}\"", name.replace('"', "\"\"")) }

/// Imports into a brand new SQLite file and checks SQLite integrity and every
/// table row count before returning. The caller's NoeSQL snapshot is unchanged.
/// Failure attempts removal after closing SQLite. Use a private trusted path
/// because symlink/path replacement by another user is outside this boundary.
pub fn export_into_sqlite(db: &Database, target: &Path) -> Result<(), InteropError> {
    if target.extension().and_then(|ext| ext.to_str()) != Some("sqlite3") {
        return Err(InteropError("SQLite target must use .sqlite3".into()));
    }
    let sql = export_sql(db, Dialect::Sqlite)?;
    let reserved = OpenOptions::new().write(true).create_new(true).open(target)
        .map_err(|error| InteropError(format!("SQLite destination must be new: {error}")))?;
    reserved.sync_all().map_err(|error| InteropError(error.to_string()))?;
    drop(reserved);
    let result = (|| -> Result<(), InteropError> {
        let conn = Sqlite::open(target, OPEN_READWRITE | OPEN_CREATE)?;
        conn.exec(&sql)?;
        if conn.scalar("PRAGMA integrity_check;")? != Scalar::Text("ok".into()) {
            return Err(InteropError("SQLite failed PRAGMA integrity_check".into()));
        }
        for (name, table) in &db.tables {
            let query = format!("SELECT COUNT(*) FROM {};", identifier(name));
            let expected = i64::try_from(table.rows.len())
                .map_err(|_| InteropError("SQLite row count overflow".into()))?;
            if conn.scalar(&query)? != Scalar::Int(expected) {
                return Err(InteropError(format!("SQLite row count mismatch for {name}")));
            }
        }
        drop(conn);
        File::open(target).and_then(|file| file.sync_all())
            .map_err(|error| InteropError(error.to_string()))?;
        let parent = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        File::open(parent).and_then(|dir| dir.sync_all())
            .map_err(|error| InteropError(error.to_string()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(target);
    }
    result
}

/// Independently validates the result of a prior transfer without requiring
/// modification of either the source or destination.
pub fn verify_sqlite(db: &Database, target: &Path) -> Result<(), InteropError> {
    let conn = Sqlite::open(target, OPEN_READONLY)?;
    if conn.scalar("PRAGMA integrity_check;")? != Scalar::Text("ok".into()) {
        return Err(InteropError("SQLite integrity check failed".into()));
    }
    for (name, table) in &db.tables {
        let query = format!("SELECT COUNT(*) FROM {};", identifier(name));
        if conn.scalar(&query)? != Scalar::Int(
            i64::try_from(table.rows.len()).map_err(|_| InteropError("row count overflow".into()))?)
        {
            return Err(InteropError(format!("SQLite row count mismatch for {name}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::Params;
    #[test]
    fn live_sqlite_transfer_checks_counts_and_integrity_without_overwrite() {
        let path = std::env::temp_dir().join(format!(
            "noesql-live-import-{}-{}.sqlite3", std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_file(&path);
        let mut db = Database::default();
        db.execute(r#"table ledger { id:int key auto
 category:text required
 amount:decimal required }
ledger add {category:"nairobi's",amount:0.1}
ledger add {category:"coast",amount:0.2}"#, &Params::new()).unwrap();
        export_into_sqlite(&db, &path).unwrap();
        verify_sqlite(&db, &path).unwrap();
        assert!(export_into_sqlite(&db, &path).is_err(), "must never overwrite target");
        let mut changed = db.clone();
        changed.execute("ledger add {category:\"new\",amount:0.3}", &Params::new()).unwrap();
        assert!(verify_sqlite(&changed, &path).is_err());
        let _ = std::fs::remove_file(path);
    }
}

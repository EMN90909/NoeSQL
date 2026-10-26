use crate::codec;
use crate::reference::{parse, Database, ExecutionReport, Params};
use crate::{validate_database_path, Error};
use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::path::Path;
use std::ptr::NonNull;

const ABI_VERSION: u32 = 2;
const STATUS_OK: i32 = 0;
const STATUS_NOT_FOUND: i32 = 1;
const STATUS_BUFFER_TOO_SMALL: i32 = -3;
const STATUS_LOCKED: i32 = -6;
const MAX_SNAPSHOT_BYTES: usize = 255 * PAGE_BYTES;
const SNAPSHOT_MAGIC: &[u8; 8] = b"NQRS0001";
pub(crate) const PAGE_BYTES: usize = 4096;

unsafe extern "C" {
    fn noesql_zig_page_open(version: u32, path: *const c_char, output: *mut *mut c_void) -> i32;
    fn noesql_zig_page_close(handle: *mut c_void);
    fn noesql_zig_page_stage(handle: *mut c_void, id: u32, data: *const u8, length: usize) -> i32;
    fn noesql_zig_page_commit(handle: *mut c_void) -> i32;
    fn noesql_zig_page_rollback(handle: *mut c_void) -> i32;
    fn noesql_zig_page_get(handle: *mut c_void, id: u32, output: *mut u8, capacity: usize) -> i32;
    fn noesql_zig_page_validate_reference_state(handle: *mut c_void) -> i32;
    fn noesql_zig_page_checkpoint(handle: *mut c_void) -> i32;
    fn noesql_zig_snapshot_write(handle: *mut c_void, input: *const u8, size: usize) -> i32;
    fn noesql_zig_snapshot_read(handle: *mut c_void, output: *mut u8, capacity: usize, written: *mut usize) -> i32;
}

pub(crate) struct Pager(NonNull<c_void>);

impl Pager {
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        validate_database_path(path)?;
        let text = path.to_str().ok_or(Error::InvalidDatabasePath)?;
        let c_path = CString::new(text).map_err(|_| Error::InvalidDatabasePath)?;
        let mut handle = std::ptr::null_mut();
        let mut retries = 0;
        loop {
            let status = unsafe { noesql_zig_page_open(ABI_VERSION, c_path.as_ptr(), &mut handle) };
            if status == STATUS_LOCKED && retries < 500 {
                retries += 1;
                // A bounded busy-wait lets independent CLI processes serialize
                // rather than failing instantly during ordinary write contention.
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            }
            if status != STATUS_OK {
                return Err(Error::Provider(status));
            }
            return NonNull::new(handle).map(Self).ok_or(Error::Provider(-1002));
        }
    }

    fn as_ptr(&self) -> *mut c_void {
        self.0.as_ptr()
    }

    pub(crate) fn read_page(&self, id: u32) -> Result<Option<[u8; PAGE_BYTES]>, Error> {
        let mut page = [0u8; PAGE_BYTES];
        let status = unsafe { noesql_zig_page_get(self.as_ptr(), id, page.as_mut_ptr(), page.len()) };
        match status {
            STATUS_OK => Ok(Some(page)),
            STATUS_NOT_FOUND => Ok(None),
            other => Err(Error::Provider(other)),
        }
    }

    pub(crate) fn stage_page(&self, id: u32, page: &[u8; PAGE_BYTES]) -> Result<(), Error> {
        let status = unsafe { noesql_zig_page_stage(self.as_ptr(), id, page.as_ptr(), page.len()) };
        if status == STATUS_OK {
            Ok(())
        } else {
            Err(Error::Provider(status))
        }
    }

    pub(crate) fn commit(&self) -> Result<(), Error> {
        let status = unsafe { noesql_zig_page_commit(self.as_ptr()) };
        if status == STATUS_OK {
            Ok(())
        } else {
            Err(Error::Provider(status))
        }
    }

    pub(crate) fn rollback(&self) -> Result<(), Error> {
        let status = unsafe { noesql_zig_page_rollback(self.as_ptr()) };
        if status == STATUS_OK {
            Ok(())
        } else {
            Err(Error::Provider(status))
        }
    }
}

impl Drop for Pager {
    fn drop(&mut self) {
        unsafe { noesql_zig_page_close(self.as_ptr()) }
    }
}

fn save_with_pager(pager: &Pager, database: &Database) -> Result<(), Error> {
    let payload = codec::encode(database).map_err(|error| Error::Codec(error.to_string()))?;
    if payload.is_empty() || payload.len() > MAX_SNAPSHOT_BYTES {
        return Err(Error::OutputTooLarge(payload.len()));
    }
    match pager.read_page(0)? {
        Some(page) if &page[..8] == SNAPSHOT_MAGIC.as_slice() => {
            // Never overwrite a corrupt logical snapshot.
            let valid = unsafe { noesql_zig_page_validate_reference_state(pager.as_ptr()) };
            if valid != STATUS_OK {
                return Err(Error::Provider(valid));
            }
        }
        Some(_) => return Err(Error::Codec("page zero is not a NoeSQL snapshot manifest".into())),
        None => {
            // No manifest is not synonymous with an empty physical store.
            for id in 1..=255u32 {
                if pager.read_page(id)?.is_some() {
                    return Err(Error::Codec("unmarked nonempty store cannot become a snapshot".into()));
                }
            }
        }
    }
    let status = unsafe { noesql_zig_snapshot_write(pager.as_ptr(), payload.as_ptr(), payload.len()) };
    if status != STATUS_OK {
        return Err(Error::Provider(status));
    }
    Ok(())
}

pub fn save(path: &Path, database: &Database) -> Result<(), Error> {
    let pager = Pager::open(path)?;
    save_with_pager(&pager, database)
}

fn load_with_pager(pager: &Pager) -> Result<Database, Error> {
    let mut required = 0usize;
    let status = unsafe { noesql_zig_snapshot_read(pager.as_ptr(), std::ptr::null_mut(), 0, &mut required) };
    if status != STATUS_BUFFER_TOO_SMALL && !(status == STATUS_OK && required == 0) {
        return Err(Error::Provider(status));
    }
    if required == 0 || required > MAX_SNAPSHOT_BYTES {
        return Err(Error::OutputTooLarge(required));
    }
    let mut payload = vec![0u8; required];
    let mut written = 0usize;
    let status = unsafe { noesql_zig_snapshot_read(pager.as_ptr(), payload.as_mut_ptr(), payload.len(), &mut written) };
    if status != STATUS_OK {
        return Err(Error::Provider(status));
    }
    if written != payload.len() {
        return Err(Error::Codec("snapshot length changed during read".into()));
    }
    codec::decode(&payload).map_err(|error| Error::Codec(error.to_string()))
}

pub fn load(path: &Path) -> Result<Database, Error> {
    let pager = Pager::open(path)?;
    load_with_pager(&pager)
}

fn load_or_default_with_pager(pager: &Pager) -> Result<Database, Error> {
    match load_with_pager(pager) {
        Ok(database) => Ok(database),
        Err(Error::Provider(STATUS_NOT_FOUND)) => {
            // A missing manifest is a fresh database ONLY if all physical
            // pages are absent; otherwise reject legacy/unmarked stores.
            for id in 1..=255u32 {
                if pager.read_page(id)?.is_some() {
                    return Err(Error::Codec("unmarked nonempty store is not a fresh database".into()));
                }
            }
            Ok(Database::default())
        }
        Err(error) => Err(error),
    }
}

// One Zig flock covers load -> query/mutation -> durable WAL commit.
// Without this scope two CLI writers can silently lose each other's changes.
pub fn execute_locked(path: &Path, source: &str, params: &Params) -> Result<ExecutionReport, String> {
    // Syntax errors cannot change database state, so parse before waiting
    // for the exclusive storage lock. This also prevents a second parse.
    let statements = parse(source).map_err(|error| error.to_string())?;
    let read_only = Database::statements_read_only(&statements);

    let pager = Pager::open(path).map_err(|error| error.to_string())?;
    let mut database = load_or_default_with_pager(&pager).map_err(|error| error.to_string())?;

    if read_only {
        // No copy of the database and no two full snapshot encodings.
        // An exclusive lock is still held during the query for consistency.
        return database.execute_parsed(&statements, params).map_err(|error| error.to_string());
    }

    // Keep the canonical before/after payload comparison for writes:
    // schema changes can affect no rows, and no-op writes must not grow WAL.
    let before = codec::encode(&database).map_err(|error| error.to_string())?;
    let report = database.execute_parsed(&statements, params).map_err(|error| error.to_string())?;
    let after = codec::encode(&database).map_err(|error| error.to_string())?;
    if before != after {
        save_with_pager(&pager, &database).map_err(|error| error.to_string())?;
    }
    Ok(report)
}

pub fn checkpoint(path: &Path) -> Result<(), Error> {
    let pager = Pager::open(path)?;
    let status = unsafe { noesql_zig_page_checkpoint(pager.as_ptr()) };
    if status == STATUS_OK {
        Ok(())
    } else {
        Err(Error::Provider(status))
    }
}

pub fn load_or_default(path: &Path) -> Result<Database, Error> {
    let pager = Pager::open(path)?;
    load_or_default_with_pager(&pager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::Params;
    use crate::value::Value;

    fn temp_path(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "noesql-rust-zig-{}-{}-{}.noedb",
            label,
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        cleanup(&path);
        path
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}.wal", path.display()));
        let _ = std::fs::remove_file(format!("{}.lck", path.display()));
    }

    #[test]
    fn rust_database_survives_zig_wal_cold_reopen() {
        let path = temp_path("roundtrip");
        let mut database = Database::default();
        database.execute(
            r#"db durable
table users {
 id:int key auto
 name:text required unique
 active:bool required = true
}
users add {name:"Amina"}
users add {name:"Brian",active:false}"#,
            &Params::new(),
        ).unwrap();

        save(&path, &database).unwrap();
        drop(database);

        let mut loaded = load(&path).unwrap();
        let rows = loaded.execute("users get id,name,active sort id", &Params::new()).unwrap().rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("name"), Some(&Value::Text("Amina".into())));
        assert_eq!(rows[1].get("active"), Some(&Value::Bool(false)));

        loaded.execute("users where name == \"Brian\" set active = true", &Params::new()).unwrap();
        save(&path, &loaded).unwrap();
        checkpoint(&path).unwrap();

        let mut reopened = load(&path).unwrap();
        let rows = reopened.execute("users get name,active sort name", &Params::new()).unwrap().rows;
        assert_eq!(rows[1].get("active"), Some(&Value::Bool(true)));

        cleanup(&path);
    }

    #[test]
    fn missing_manifest_payload_page_never_falls_back_to_new_database() {
        let path = temp_path("missing-manifest-page");
        let pager = Pager::open(&path).unwrap();
        let mut manifest = [0u8; PAGE_BYTES];
        manifest[..8].copy_from_slice(b"NQRS0001");
        manifest[8..12].copy_from_slice(&1u32.to_le_bytes());
        manifest[12..16].copy_from_slice(&2u32.to_le_bytes());
        manifest[16..20].copy_from_slice(&((PAGE_BYTES + 1) as u32).to_le_bytes());
        pager.stage_page(0, &manifest).unwrap();
        pager.stage_page(1, &[0u8; PAGE_BYTES]).unwrap();
        pager.commit().unwrap();
        drop(pager);

        assert!(
            matches!(load_or_default(&path), Err(Error::Provider(-2))),
            "a declared but missing snapshot data page is corruption, not an empty DB",
        );
        cleanup(&path);
    }

    #[test]
    fn raw_pages_without_snapshot_manifest_are_not_a_fresh_database() {
        let path = temp_path("unmarked-raw-page");
        let pager = Pager::open(&path).unwrap();
        let mut raw_page = [0u8; PAGE_BYTES];
        raw_page[0] = 0x7f;
        pager.stage_page(7, &raw_page).unwrap();
        pager.commit().unwrap();
        drop(pager);

        assert!(
            matches!(load_or_default(&path), Err(Error::Codec(_))),
            "unmarked physical pages must not be misread as an empty DB"
        );
        let empty = Database::default();
        assert!(
            save(&path, &empty).is_err(),
            "the same unmarked pages must never be overwritten"
        );
        cleanup(&path);
    }

    #[test]
    fn empty_zig_store_loads_as_new_database_only_via_load_or_default() {
        let path = temp_path("empty");
        assert!(load(&path).is_err());
        let database = load_or_default(&path).unwrap();
        assert!(database.tables.is_empty());
        cleanup(&path);
    }
}

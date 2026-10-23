use crate::codec;
use crate::reference::Database;
use crate::{validate_database_path, Error};
use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::path::Path;
use std::ptr::NonNull;

const ABI_VERSION: u32 = 2;
const STATUS_OK: i32 = 0;
const STATUS_BUFFER_TOO_SMALL: i32 = -3;
const MAX_SNAPSHOT_BYTES: usize = 255 * 4096;

unsafe extern "C" {
    fn noesql_zig_page_open(version: u32, path: *const c_char, output: *mut *mut c_void) -> i32;
    fn noesql_zig_page_close(handle: *mut c_void);
    fn noesql_zig_page_checkpoint(handle: *mut c_void) -> i32;
    fn noesql_zig_snapshot_write(handle: *mut c_void, input: *const u8, size: usize) -> i32;
    fn noesql_zig_snapshot_read(handle: *mut c_void, output: *mut u8, capacity: usize, written: *mut usize) -> i32;
}

struct Pager(NonNull<c_void>);
impl Pager {
    fn open(path: &Path) -> Result<Self, Error> {
        validate_database_path(path)?;
        let text = path.to_str().ok_or(Error::InvalidDatabasePath)?;
        let c_path = CString::new(text).map_err(|_| Error::InvalidDatabasePath)?;
        let mut handle = std::ptr::null_mut();
        let status = unsafe { noesql_zig_page_open(ABI_VERSION, c_path.as_ptr(), &mut handle) };
        if status != STATUS_OK { return Err(Error::Provider(status)); }
        NonNull::new(handle).map(Self).ok_or(Error::Provider(-1002))
    }
    fn as_ptr(&self) -> *mut c_void { self.0.as_ptr() }
}
impl Drop for Pager {
    fn drop(&mut self) { unsafe { noesql_zig_page_close(self.as_ptr()) } }
}

pub fn save(path: &Path, database: &Database) -> Result<(), Error> {
    let payload = codec::encode(database).map_err(|error| Error::Codec(error.to_string()))?;
    if payload.is_empty() || payload.len() > MAX_SNAPSHOT_BYTES { return Err(Error::OutputTooLarge(payload.len())); }
    let pager = Pager::open(path)?;
    let status = unsafe { noesql_zig_snapshot_write(pager.as_ptr(), payload.as_ptr(), payload.len()) };
    if status != STATUS_OK { return Err(Error::Provider(status)); }
    Ok(())
}

pub fn load(path: &Path) -> Result<Database, Error> {
    let pager = Pager::open(path)?;
    let mut required = 0usize;
    let status = unsafe { noesql_zig_snapshot_read(pager.as_ptr(), std::ptr::null_mut(), 0, &mut required) };
    if status != STATUS_BUFFER_TOO_SMALL && !(status == STATUS_OK && required == 0) {
        return Err(Error::Provider(status));
    }
    if required == 0 || required > MAX_SNAPSHOT_BYTES { return Err(Error::OutputTooLarge(required)); }
    let mut payload = vec![0u8; required];
    let mut written = 0usize;
    let status = unsafe { noesql_zig_snapshot_read(pager.as_ptr(), payload.as_mut_ptr(), payload.len(), &mut written) };
    if status != STATUS_OK { return Err(Error::Provider(status)); }
    if written != payload.len() { return Err(Error::Codec("snapshot length changed during read".into())); }
    codec::decode(&payload).map_err(|error| Error::Codec(error.to_string()))
}

pub fn checkpoint(path: &Path) -> Result<(), Error> {
    let pager = Pager::open(path)?;
    let status = unsafe { noesql_zig_page_checkpoint(pager.as_ptr()) };
    if status == STATUS_OK { Ok(()) } else { Err(Error::Provider(status)) }
}

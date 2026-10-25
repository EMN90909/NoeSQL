use crate::{validate_database_path, Error};
use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::path::Path;
use std::ptr::NonNull;

const ABI_VERSION: u32 = 2;
const PAGE_BYTES: usize = 4096;
const STATUS_OK: i32 = 0;
const STATUS_NOT_FOUND: i32 = 1;
const MAGIC: &[u8; 8] = b"NQNR0001";

unsafe extern "C" {
    fn noesqlNativeApiVersion() -> u32;
    fn noesqlNativeBoolRow(
        operation: i64,
        present: i64,
        current_id: i64,
        current_enabled: i64,
        requested_id: i64,
        requested_enabled: i64,
    ) -> i64;

    fn noesql_zig_page_open(version: u32, path: *const c_char, output: *mut *mut c_void) -> i32;
    fn noesql_zig_page_close(handle: *mut c_void);
    fn noesql_zig_page_get(handle: *mut c_void, id: u32, output: *mut u8, capacity: usize) -> i32;
    fn noesql_zig_page_stage(handle: *mut c_void, id: u32, data: *const u8, length: usize) -> i32;
    fn noesql_zig_page_commit(handle: *mut c_void) -> i32;
    fn noesql_zig_page_rollback(handle: *mut c_void) -> i32;
}

pub fn api_version() -> u32 {
    unsafe { noesqlNativeApiVersion() }
}

struct Pager(NonNull<c_void>);

impl Pager {
    fn open(path: &Path) -> Result<Self, Error> {
        validate_database_path(path)?;
        let text = path.to_str().ok_or(Error::InvalidDatabasePath)?;
        let path = CString::new(text).map_err(|_| Error::InvalidDatabasePath)?;
        let mut handle = std::ptr::null_mut();
        let status = unsafe { noesql_zig_page_open(ABI_VERSION, path.as_ptr(), &mut handle) };
        if status != STATUS_OK {
            return Err(Error::Provider(status));
        }
        NonNull::new(handle).map(Self).ok_or(Error::Provider(-1300))
    }

    fn ptr(&self) -> *mut c_void {
        self.0.as_ptr()
    }
}

impl Drop for Pager {
    fn drop(&mut self) {
        unsafe { noesql_zig_page_close(self.ptr()) };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoolRow {
    id: i64,
    enabled: bool,
}

fn decode_page(page: &[u8; PAGE_BYTES]) -> Result<Option<BoolRow>, Error> {
    if page.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if &page[0..8] != MAGIC {
        return Err(Error::Codec("invalid native Noqeri row page magic".into()));
    }
    let id = i64::from_le_bytes(
        page[8..16]
            .try_into()
            .map_err(|_| Error::Codec("invalid native Noqeri row id".into()))?,
    );
    let enabled = match page[16] {
        0 => false,
        1 => true,
        _ => return Err(Error::Codec("invalid native Noqeri bool row".into())),
    };
    Ok(Some(BoolRow { id, enabled }))
}

fn encode_page(row: Option<BoolRow>) -> [u8; PAGE_BYTES] {
    let mut page = [0u8; PAGE_BYTES];
    if let Some(row) = row {
        page[0..8].copy_from_slice(MAGIC);
        page[8..16].copy_from_slice(&row.id.to_le_bytes());
        page[16] = u8::from(row.enabled);
    }
    page
}

fn read_row(pager: &Pager, page_id: u32) -> Result<Option<BoolRow>, Error> {
    let mut page = [0u8; PAGE_BYTES];
    let status = unsafe { noesql_zig_page_get(pager.ptr(), page_id, page.as_mut_ptr(), page.len()) };
    match status {
        STATUS_OK => decode_page(&page),
        STATUS_NOT_FOUND => Ok(None),
        other => Err(Error::Provider(other)),
    }
}

fn commit_row(pager: &Pager, page_id: u32, row: Option<BoolRow>) -> Result<(), Error> {
    let page = encode_page(row);
    let staged = unsafe { noesql_zig_page_stage(pager.ptr(), page_id, page.as_ptr(), page.len()) };
    if staged != STATUS_OK {
        unsafe { noesql_zig_page_rollback(pager.ptr()) };
        return Err(Error::Provider(staged));
    }
    let committed = unsafe { noesql_zig_page_commit(pager.ptr()) };
    if committed != STATUS_OK {
        unsafe { noesql_zig_page_rollback(pager.ptr()) };
        return Err(Error::Provider(committed));
    }
    Ok(())
}

fn execute(
    path: &Path,
    page_id: u32,
    row_id: i64,
    enabled: bool,
    operation: i64,
) -> Result<Option<bool>, Error> {
    validate_database_path(path)?;
    if page_id == 0 || page_id >= 256 {
        return Err(Error::Provider(-211));
    }

    let pager = Pager::open(path)?;
    let current = read_row(&pager, page_id)?;
    let result = unsafe {
        noesqlNativeBoolRow(
            operation,
            i64::from(current.is_some()),
            current.map(|row| row.id).unwrap_or(0),
            i64::from(current.map(|row| row.enabled).unwrap_or(false)),
            row_id,
            i64::from(enabled),
        )
    };

    match operation {
        1 => match result {
            100 => Ok(Some(false)),
            101 => Ok(Some(true)),
            110 => Ok(None),
            other => Err(Error::Provider(other as i32)),
        },
        2 => match result {
            200 | 201 => {
                commit_row(&pager, page_id, Some(BoolRow { id: row_id, enabled: result == 201 }))?;
                Ok(None)
            }
            other => Err(Error::Provider(other as i32)),
        },
        3 => match result {
            300 | 301 => {
                commit_row(&pager, page_id, Some(BoolRow { id: row_id, enabled: result == 301 }))?;
                Ok(None)
            }
            other => Err(Error::Provider(other as i32)),
        },
        4 => match result {
            400 => {
                commit_row(&pager, page_id, None)?;
                Ok(None)
            }
            other => Err(Error::Provider(other as i32)),
        },
        _ => Err(Error::Provider(-1)),
    }
}

pub fn insert_bool_row(path: &Path, page_id: u32, row_id: i64, enabled: bool) -> Result<(), Error> {
    execute(path, page_id, row_id, enabled, 2).map(|_| ())
}

pub fn get_bool_row(path: &Path, page_id: u32, row_id: i64) -> Result<Option<bool>, Error> {
    execute(path, page_id, row_id, false, 1)
}

pub fn set_bool_row(path: &Path, page_id: u32, row_id: i64, enabled: bool) -> Result<(), Error> {
    execute(path, page_id, row_id, enabled, 3).map(|_| ())
}

pub fn delete_bool_row(path: &Path, page_id: u32, row_id: i64) -> Result<(), Error> {
    execute(path, page_id, row_id, false, 4).map(|_| ())
}

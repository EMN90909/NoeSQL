use crate::{validate_database_path, Error};
use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::NonNull;

const ABI_VERSION: u32 = 2;
const PAGE_BYTES: usize = 4096;
const STATUS_OK: i32 = 0;
const STATUS_NOT_FOUND: i32 = 1;
const MAGIC: &[u8; 8] = b"NQS20001";

unsafe extern "C" {
    fn noesql_zig_page_open(version: u32, path: *const c_char, output: *mut *mut c_void) -> i32;
    fn noesql_zig_page_close(handle: *mut c_void);
    fn noesql_zig_page_get(handle: *mut c_void, id: u32, output: *mut u8, capacity: usize) -> i32;
    fn noesql_zig_page_stage(handle: *mut c_void, id: u32, data: *const u8, length: usize) -> i32;
    fn noesql_zig_page_commit(handle: *mut c_void) -> i32;
    fn noesql_zig_page_rollback(handle: *mut c_void) -> i32;
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
        NonNull::new(handle).map(Self).ok_or(Error::Provider(-1200))
    }
    fn ptr(&self) -> *mut c_void { self.0.as_ptr() }
}
impl Drop for Pager {
    fn drop(&mut self) { unsafe { noesql_zig_page_close(self.ptr()) } }
}

#[derive(Clone, Debug)]
pub struct Stage2Runtime {
    vm: PathBuf,
    module: PathBuf,
}
impl Stage2Runtime {
    pub fn new(vm: impl Into<PathBuf>, module: impl Into<PathBuf>) -> Self {
        Self { vm: vm.into(), module: module.into() }
    }

    fn invoke(&self, args: [i64; 6]) -> Result<i64, Error> {
        let output = Command::new(&self.vm)
            .arg(&self.module)
            .arg("noesqlStage2BoolRow")
            .args(args.map(|value| value.to_string()))
            .output()
            .map_err(|error| Error::Io(format!("failed to launch Stage-2 NQRI VM: {error}")))?;
        if !output.status.success() {
            return Err(Error::Io(format!(
                "Stage-2 NQRI execution failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let stdout = String::from_utf8(output.stdout).map_err(|_| Error::Utf8)?;
        stdout.trim().parse::<i64>()
            .map_err(|_| Error::Codec(format!("invalid Stage-2 result: {}", stdout.trim())))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoolRow {
    id: i64,
    enabled: bool,
}

fn decode_page(page: &[u8; PAGE_BYTES]) -> Result<Option<BoolRow>, Error> {
    if page.iter().all(|byte| *byte == 0) { return Ok(None); }
    if &page[0..8] != MAGIC { return Err(Error::Codec("invalid Stage-2 row page magic".into())); }
    let id = i64::from_le_bytes(page[8..16].try_into().map_err(|_| Error::Codec("bad row id".into()))?);
    match page[16] {
        0 => Ok(Some(BoolRow { id, enabled: false })),
        1 => Ok(Some(BoolRow { id, enabled: true })),
        _ => Err(Error::Codec("invalid Stage-2 bool row".into())),
    }
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
    runtime: &Stage2Runtime,
    path: &Path,
    page_id: u32,
    row_id: i64,
    enabled: bool,
    operation: i64,
) -> Result<Option<bool>, Error> {
    validate_database_path(path)?;
    if page_id == 0 || page_id >= 256 { return Err(Error::Provider(-211)); }
    let pager = Pager::open(path)?;
    let current = read_row(&pager, page_id)?;
    let result = runtime.invoke([
        operation,
        i64::from(current.is_some()),
        current.map(|row| row.id).unwrap_or(0),
        i64::from(current.map(|row| row.enabled).unwrap_or(false)),
        row_id,
        i64::from(enabled),
    ])?;

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
            210 => Err(Error::Provider(210)),
            other => Err(Error::Provider(other as i32)),
        },
        3 => match result {
            300 | 301 => {
                commit_row(&pager, page_id, Some(BoolRow { id: row_id, enabled: result == 301 }))?;
                Ok(None)
            }
            310 => Err(Error::Provider(310)),
            other => Err(Error::Provider(other as i32)),
        },
        4 => match result {
            400 => {
                commit_row(&pager, page_id, None)?;
                Ok(None)
            }
            410 => Err(Error::Provider(410)),
            other => Err(Error::Provider(other as i32)),
        },
        _ => Err(Error::Provider(-1)),
    }
}

pub fn insert_bool_row(runtime: &Stage2Runtime, path: &Path, page_id: u32, row_id: i64, enabled: bool) -> Result<(), Error> {
    execute(runtime, path, page_id, row_id, enabled, 2).map(|_| ())
}
pub fn get_bool_row(runtime: &Stage2Runtime, path: &Path, page_id: u32, row_id: i64) -> Result<Option<bool>, Error> {
    execute(runtime, path, page_id, row_id, false, 1)
}
pub fn set_bool_row(runtime: &Stage2Runtime, path: &Path, page_id: u32, row_id: i64, enabled: bool) -> Result<(), Error> {
    execute(runtime, path, page_id, row_id, enabled, 3).map(|_| ())
}
pub fn delete_bool_row(runtime: &Stage2Runtime, path: &Path, page_id: u32, row_id: i64) -> Result<(), Error> {
    execute(runtime, path, page_id, row_id, false, 4).map(|_| ())
}

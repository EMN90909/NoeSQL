use crate::{validate_database_path, Error};
use core::ffi::c_char;
use std::ffi::CString;
use std::path::Path;

unsafe extern "C" {
    fn noesqlNativeApiVersion() -> u32;
    fn noesqlNativeBoolRow(
        path: *const c_char,
        page_id: u32,
        row_id: i64,
        enabled: bool,
        operation: i32,
        out_enabled: *mut u8,
    ) -> isize;
}

pub fn api_version() -> u32 {
    unsafe { noesqlNativeApiVersion() }
}

fn execute(path: &Path, page_id: u32, row_id: i64, enabled: bool, operation: i32) -> Result<Option<bool>, Error> {
    validate_database_path(path)?;
    if page_id == 0 || page_id >= 256 { return Err(Error::Provider(-211)); }
    let path = path.to_str().ok_or(Error::InvalidDatabasePath)?;
    let path = CString::new(path).map_err(|_| Error::InvalidDatabasePath)?;
    let mut output = 0u8;
    let status = unsafe { noesqlNativeBoolRow(path.as_ptr(), page_id, row_id, enabled, operation, &mut output) };
    if status < 0 { return Err(Error::Provider(status as i32)); }
    if operation == 1 {
        if status == 1 { Ok(None) } else { Ok(Some(output != 0)) }
    } else {
        Ok(None)
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

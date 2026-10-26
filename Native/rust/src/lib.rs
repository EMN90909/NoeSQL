pub mod codec;
pub mod decimal;
pub mod planner;
pub mod mvcc;
pub mod extended_types;
pub mod authorization;
pub mod interop;
pub mod reference;
pub mod value;
#[cfg(feature = "zig-pager")]
pub mod zig_pager;
#[cfg(feature = "noqeri-native")]
pub mod noqeri_native;
#[cfg(feature = "stage2-nqri")]
pub mod stage2_nqri;

use core::ffi::c_void;
use core::fmt;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub const SESSION_ABI_VERSION: u32 = 2;
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

pub type ConnectionHandle = *mut c_void;
pub type StatementHandle = *mut c_void;

#[repr(C)]
pub struct NoeSqlSessionV2 {
    pub abi_version: u32,
    pub context: *mut c_void,
    pub open: Option<unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut ConnectionHandle) -> i32>,
    pub close: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle) -> i32>,
    pub prepare: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle, *const u8, usize, *mut StatementHandle) -> i32>,
    pub release: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle, StatementHandle) -> i32>,
    pub execute: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle, StatementHandle, *const u8, usize, *mut u8, usize, *mut usize) -> i32>,
    pub query: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle, StatementHandle, *const u8, usize, *mut u8, usize, *mut usize) -> i32>,
    pub begin: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle) -> i32>,
    pub commit: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle) -> i32>,
    pub rollback: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle) -> i32>,
    pub last_error: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle, *mut u8, usize, *mut usize) -> i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidAbi(u32),
    MissingCallback(&'static str),
    SourceTooLarge(usize),
    OutputTooLarge(usize),
    InvalidDatabasePath,
    Provider(i32),
    Io(String),
    Codec(String),
    Utf8,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAbi(v) => write!(f, "unsupported NoeSQL session ABI {v}"),
            Self::MissingCallback(name) => write!(f, "provider is missing required callback {name}"),
            Self::SourceTooLarge(n) => write!(f, "NoeSQL source is {n} bytes; limit is {MAX_SOURCE_BYTES}"),
            Self::OutputTooLarge(n) => write!(f, "provider output exceeded {n} bytes"),
            Self::InvalidDatabasePath => write!(f, "database path must be a non-empty .noedb path without NUL bytes"),
            Self::Provider(code) => write!(f, "NoeSQL provider returned status {code}"),
            Self::Io(message) => write!(f, "I/O error: {message}"),
            Self::Codec(message) => write!(f, "snapshot codec error: {message}"),
            Self::Utf8 => write!(f, "provider returned non-UTF-8 output"),
        }
    }
}

impl std::error::Error for Error {}

pub fn validate_database_path(path: &Path) -> Result<(), Error> {
    let text = path.to_str().ok_or(Error::InvalidDatabasePath)?;
    if text.is_empty() || text.as_bytes().contains(&0) || path.extension() != Some(OsStr::new("noedb")) {
        return Err(Error::InvalidDatabasePath);
    }
    Ok(())
}

pub fn validate_source(source: &[u8]) -> Result<(), Error> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(Error::SourceTooLarge(source.len()));
    }
    std::str::from_utf8(source).map_err(|_| Error::Utf8)?;
    Ok(())
}

pub struct Session<'a> {
    api: &'a NoeSqlSessionV2,
    connection: ConnectionHandle,
}

impl<'a> Session<'a> {
    pub fn open(api: &'a NoeSqlSessionV2, path: &Path) -> Result<Self, Error> {
        validate_database_path(path)?;
        if api.abi_version != SESSION_ABI_VERSION {
            return Err(Error::InvalidAbi(api.abi_version));
        }
        let open = api.open.ok_or(Error::MissingCallback("open"))?;
        let bytes = path.to_str().ok_or(Error::InvalidDatabasePath)?.as_bytes();
        let mut connection: ConnectionHandle = core::ptr::null_mut();
        let code = unsafe { open(api.context, bytes.as_ptr(), bytes.len(), &mut connection) };
        if code != 0 { return Err(Error::Provider(code)); }
        if connection.is_null() { return Err(Error::Provider(-1000)); }
        Ok(Self { api, connection })
    }

    pub fn prepare(&self, source: &str) -> Result<Statement<'_, 'a>, Error> {
        validate_source(source.as_bytes())?;
        let prepare = self.api.prepare.ok_or(Error::MissingCallback("prepare"))?;
        let mut handle: StatementHandle = core::ptr::null_mut();
        let code = unsafe {
            prepare(self.api.context, self.connection, source.as_ptr(), source.len(), &mut handle)
        };
        if code != 0 { return Err(Error::Provider(code)); }
        if handle.is_null() { return Err(Error::Provider(-1001)); }
        Ok(Statement { session: self, handle })
    }

    pub fn begin(&self) -> Result<(), Error> { self.call_tx(self.api.begin, "begin") }
    pub fn commit(&self) -> Result<(), Error> { self.call_tx(self.api.commit, "commit") }
    pub fn rollback(&self) -> Result<(), Error> { self.call_tx(self.api.rollback, "rollback") }

    fn call_tx(
        &self,
        callback: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle) -> i32>,
        name: &'static str,
    ) -> Result<(), Error> {
        let callback = callback.ok_or(Error::MissingCallback(name))?;
        let code = unsafe { callback(self.api.context, self.connection) };
        if code == 0 { Ok(()) } else { Err(Error::Provider(code)) }
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if let Some(close) = self.api.close {
            let _ = unsafe { close(self.api.context, self.connection) };
        }
    }
}

pub struct Statement<'s, 'a> {
    session: &'s Session<'a>,
    handle: StatementHandle,
}

impl Statement<'_, '_> {
    pub fn execute(&self, params: &[u8]) -> Result<Vec<u8>, Error> {
        self.call(self.session.api.execute, "execute", params)
    }

    pub fn query(&self, params: &[u8]) -> Result<Vec<u8>, Error> {
        self.call(self.session.api.query, "query", params)
    }

    fn call(
        &self,
        callback: Option<unsafe extern "C" fn(*mut c_void, ConnectionHandle, StatementHandle, *const u8, usize, *mut u8, usize, *mut usize) -> i32>,
        name: &'static str,
        params: &[u8],
    ) -> Result<Vec<u8>, Error> {
        let callback = callback.ok_or(Error::MissingCallback(name))?;
        let mut output = vec![0u8; MAX_OUTPUT_BYTES];
        let mut written = 0usize;
        let code = unsafe {
            callback(
                self.session.api.context,
                self.session.connection,
                self.handle,
                params.as_ptr(),
                params.len(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        };
        if code != 0 { return Err(Error::Provider(code)); }
        if written > output.len() { return Err(Error::OutputTooLarge(written)); }
        output.truncate(written);
        Ok(output)
    }
}

impl Drop for Statement<'_, '_> {
    fn drop(&mut self) {
        if let Some(release) = self.session.api.release {
            let _ = unsafe { release(self.session.api.context, self.session.connection, self.handle) };
        }
    }
}

pub fn read_source(path: impl AsRef<Path>) -> Result<String, Error> {
    let bytes = fs::read(path.as_ref()).map_err(|e| Error::Io(e.to_string()))?;
    validate_source(&bytes)?;
    String::from_utf8(bytes).map_err(|_| Error::Utf8)
}

pub fn canonical_db_path(path: impl AsRef<Path>) -> Result<PathBuf, Error> {
    let path = path.as_ref();
    validate_database_path(path)?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::NonNull;

    unsafe extern "C" fn open(_: *mut c_void, _: *const u8, _: usize, out: *mut ConnectionHandle) -> i32 {
        unsafe { *out = NonNull::<u8>::dangling().as_ptr().cast(); }
        0
    }
    unsafe extern "C" fn close(_: *mut c_void, _: ConnectionHandle) -> i32 { 0 }
    unsafe extern "C" fn prepare(_: *mut c_void, _: ConnectionHandle, _: *const u8, _: usize, out: *mut StatementHandle) -> i32 {
        unsafe { *out = NonNull::<u8>::dangling().as_ptr().cast(); }
        0
    }
    unsafe extern "C" fn release(_: *mut c_void, _: ConnectionHandle, _: StatementHandle) -> i32 { 0 }
    unsafe extern "C" fn call(_: *mut c_void, _: ConnectionHandle, _: StatementHandle, _: *const u8, _: usize, output: *mut u8, cap: usize, written: *mut usize) -> i32 {
        let payload = br#"{"ok":true}"#;
        if cap < payload.len() { return -3; }
        unsafe {
            std::ptr::copy_nonoverlapping(payload.as_ptr(), output, payload.len());
            *written = payload.len();
        }
        0
    }
    unsafe extern "C" fn tx(_: *mut c_void, _: ConnectionHandle) -> i32 { 0 }
    unsafe extern "C" fn last(_: *mut c_void, _: ConnectionHandle, _: *mut u8, _: usize, written: *mut usize) -> i32 {
        unsafe { *written = 0; }
        0
    }

    fn api() -> NoeSqlSessionV2 {
        NoeSqlSessionV2 {
            abi_version: SESSION_ABI_VERSION,
            context: core::ptr::null_mut(),
            open: Some(open), close: Some(close), prepare: Some(prepare), release: Some(release),
            execute: Some(call), query: Some(call), begin: Some(tx), commit: Some(tx), rollback: Some(tx),
            last_error: Some(last),
        }
    }

    #[test]
    fn rejects_non_noedb_path() {
        assert_eq!(validate_database_path(Path::new("db.nqd")), Err(Error::InvalidDatabasePath));
    }

    #[test]
    fn wraps_native_session_contract() {
        let api = api();
        let session = Session::open(&api, Path::new("test.noedb")).unwrap();
        session.begin().unwrap();
        let statement = session.prepare("users get *").unwrap();
        assert_eq!(statement.query(b"{}").unwrap(), br#"{"ok":true}"#);
        session.commit().unwrap();
    }

    #[test]
    fn rejects_oversized_source() {
        let source = vec![b'a'; MAX_SOURCE_BYTES + 1];
        assert!(matches!(validate_source(&source), Err(Error::SourceTooLarge(_))));
    }
}

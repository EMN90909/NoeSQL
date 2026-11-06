//! Native session-v2 provider for Noqeri's web framework and stdlib hosts.
//!
//! ABI: Compiler/bootstrap/web_native_noesql_v2.hpp in Noqeri. No C++
//! bridge or subprocess is used here. v3 is the ONLY supported storage format;
//! legacy NQDB0002 is never silently reinterpreted or upgraded.
//!
//! This is an administrative in-process provider, not a public multi-tenant
//! database server. The host must authenticate users and enforce request/tenant
//! policies before invoking this ABI. Release qualification is still pending.
//!
//! SAFETY CONTRACT: Hosts pass live handles created by these callbacks, valid
//! non-overlapping buffers, and release all statements before closing their
//! connection. A close may not race with any callback on that handle.
use crate::reference::{parse, Database, ExecutionReport, Params, Statement};
use crate::value::Value;
use crate::session_bindings::parse_bindings;
use crate::v3_store::{self, V3Error};
use crate::{ConnectionHandle, NoeSqlSessionV2, StatementHandle, SESSION_ABI_VERSION};
use core::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const OK: i32 = 0;
const INVALID: i32 = -1;
const BAD_INPUT: i32 = -2;
const TOO_SMALL: i32 = -3;
const ENGINE_ERROR: i32 = -4;
const CONFLICT: i32 = -5;
const TX_ERROR: i32 = -6;
const PANIC: i32 = -7;
const MAX_SOURCE: usize = 65_536;
const MAX_PATH: usize = 4096;
const MAX_BINDINGS: usize = 32 * 1024;
const MAX_RESPONSE: usize = 16 * 1024 * 1024;
const MAX_TRANSACTION_STATEMENTS: usize = 256;
const MAX_TRANSACTION_SOURCE: usize = 1024 * 1024;

struct Transaction {
    generation: u64,
    staged: Database,
    statements: Vec<Statement>,
    source_bytes: usize,
    // Once any request fails, no earlier staged write can be committed.
    // COMMIT consumes and rejects this transaction until the caller retries
    // in a NEW transaction after an explicit ROLLBACK.
    failed: bool,
}
struct State {
    transaction: Option<Transaction>,
    statements_open: usize,
    last_error: String,
}
struct Connection {
    path: PathBuf,
    state: Mutex<State>,
}
struct Prepared {
    owner: usize,
    source: String,
    parsed: Vec<Statement>,
}

fn code_for_v3(e: &V3Error) -> i32 {
    match e {
        V3Error::Conflict(_) => CONFLICT,
        V3Error::CommitUncertain(_) => ENGINE_ERROR,
        _ => ENGINE_ERROR,
    }
}
fn ffi_guard(f: impl FnOnce() -> i32) -> i32 {
    // panic=abort release builds abort instead of unwinding; no Rust panic
    // is ever permitted to unwind into the C++ framework.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(PANIC)
}
unsafe fn input<'a>(ptr: *const u8, len: usize, maximum: usize) -> Result<&'a [u8], String> {
    if len > maximum || (len != 0 && ptr.is_null()) {
        return Err("invalid or oversized session-v2 byte buffer".into());
    }
    if len == 0 { return Ok(&[]); }
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}
unsafe fn utf8<'a>(ptr: *const u8, len: usize, maximum: usize) -> Result<&'a str, String> {
    let bytes = unsafe { input(ptr, len, maximum) }?;
    let text = std::str::from_utf8(bytes).map_err(|_| "session-v2 requires UTF-8")?;
    if text.as_bytes().contains(&0) { return Err("NUL byte in NoeSQL input".into()); }
    Ok(text)
}
unsafe fn connection<'a>(handle: ConnectionHandle) -> Option<&'a Connection> {
    if handle.is_null() { None } else { Some(unsafe { &*(handle as *const Connection) }) }
}
unsafe fn statement<'a>(handle: StatementHandle) -> Option<&'a Prepared> {
    if handle.is_null() { None } else { Some(unsafe { &*(handle as *const Prepared) }) }
}
fn set_error(state: &mut State, code: i32, error: impl ToString) -> i32 {
    state.last_error = error.to_string();
    code
}
fn append_bounded(out: &mut String, part: &str, capacity: usize) -> Result<(), (i32, String)> {
    let maximum = capacity.min(MAX_RESPONSE);
    if out.len().checked_add(part.len()).is_none_or(|size| size > maximum) {
        return Err((TOO_SMALL, format!(
            "response exceeds {maximum}-byte budget; nothing committed"
        )));
    }
    out.push_str(part);
    Ok(())
}
fn json_string_bounded(
    out: &mut String, text: &str, capacity: usize,
) -> Result<(), (i32, String)> {
    append_bounded(out, "\"", capacity)?;
    for ch in text.chars() {
        match ch {
            '"' => append_bounded(out, "\\\"", capacity)?,
            '\\' => append_bounded(out, "\\\\", capacity)?,
            '\n' => append_bounded(out, "\\n", capacity)?,
            '\r' => append_bounded(out, "\\r", capacity)?,
            '\t' => append_bounded(out, "\\t", capacity)?,
            control if control.is_control() => {
                append_bounded(out, &format!("\\u{:04x}", control as u32), capacity)?;
            },
            c => {
                let mut tmp = [0u8; 4];
                append_bounded(out, c.encode_utf8(&mut tmp), capacity)?;
            },
        }
    }
    append_bounded(out, "\"", capacity)
}
fn report_bytes(
    report: &ExecutionReport, query: bool, capacity: usize,
) -> Result<Vec<u8>, (i32, String)> {
    if !query {
        let payload = format!("affected={}", report.affected);
        require_capacity(payload.as_bytes(), capacity)?;
        return Ok(payload.into_bytes());
    }
    // Don't materialize unbounded rows_json before checking the response cap:
    // it can multiply a small output buffer into a very large allocation.
    let mut out = String::new();
    append_bounded(&mut out, "[", capacity)?;
    for (index, row) in report.rows.iter().enumerate() {
        if index != 0 { append_bounded(&mut out, ",", capacity)?; }
        append_bounded(&mut out, "{", capacity)?;
        for (i, (key, value)) in row.iter().enumerate() {
            if i != 0 { append_bounded(&mut out, ",", capacity)?; }
            json_string_bounded(&mut out, key, capacity)?;
            append_bounded(&mut out, ":", capacity)?;
            match value {
                Value::Null => append_bounded(&mut out, "null", capacity)?,
                Value::Bool(v) => append_bounded(
                    &mut out, if *v { "true" } else { "false" }, capacity
                )?,
                Value::Int(v) => append_bounded(&mut out, &v.to_string(), capacity)?,
                Value::Decimal(v) => json_string_bounded(
                    &mut out, &v.to_string(), capacity
                )?,
                Value::Text(v) => json_string_bounded(&mut out, v, capacity)?,
            }
        }
        append_bounded(&mut out, "}", capacity)?;
    }
    append_bounded(&mut out, "]", capacity)?;
    Ok(out.into_bytes())
}
unsafe fn write_response(
    payload: &[u8], output: *mut u8, capacity: usize, written: *mut usize,
) -> Result<(), (i32, String)> {
    if written.is_null() || capacity > MAX_RESPONSE ||
       (capacity != 0 && output.is_null()) {
        return Err((INVALID, "invalid response buffer".into()));
    }
    unsafe { *written = payload.len(); }
    if payload.len() > capacity {
        return Err((TOO_SMALL, format!(
            "NoeSQL response needs {} bytes, but caller supplied {capacity}; nothing committed",
            payload.len()
        )));
    }
    if !payload.is_empty() {
        unsafe { std::ptr::copy_nonoverlapping(payload.as_ptr(), output, payload.len()); }
    }
    Ok(())
}
fn require_capacity(bytes: &[u8], capacity: usize) -> Result<(), (i32, String)> {
    if bytes.len() > capacity || bytes.len() > MAX_RESPONSE {
        return Err((TOO_SMALL, format!(
            "NoeSQL response {} bytes exceeds caller capacity {capacity}; not committed",
            bytes.len()
        )));
    }
    Ok(())
}
fn execute_inner(
    path: &Path,
    state: &mut State,
    prepared: &Prepared,
    params: &Params,
    read: bool,
    capacity: usize,
) -> Result<Vec<u8>, (i32, String)> {
    let is_read_only = Database::statements_read_only(&prepared.parsed);
    if read && !is_read_only {
        return Err((BAD_INPUT, "query callback refuses any mutating statement".into()));
    }
    if let Some(transaction) = &mut state.transaction {
        if transaction.failed {
            return Err((TX_ERROR, "transaction aborted by prior failure; rollback required".into()));
        }
        if read || is_read_only {
            let report = transaction.staged.query_parsed(&prepared.parsed, params)
                .map_err(|e| (ENGINE_ERROR, e.to_string()))?;
            let data = report_bytes(&report, read || is_read_only, capacity)?;
            return Ok(data);
        }
        if transaction.statements.len().saturating_add(prepared.parsed.len()) > MAX_TRANSACTION_STATEMENTS ||
           transaction.source_bytes.saturating_add(prepared.source.len()) > MAX_TRANSACTION_SOURCE {
            return Err((TX_ERROR, "transaction statement/source budget exceeded".into()));
        }
        // The current staged state is not modified unless both the executor
        // succeeds and the entire result fits the caller-owned output buffer.
        let mut candidate = transaction.staged.clone();
        let report = candidate.execute_isolated_parsed(&prepared.parsed, params)
            .map_err(|e| (ENGINE_ERROR, e.to_string()))?;
        let data = report_bytes(&report, false, capacity)?;
        transaction.staged = candidate;
        transaction.statements.extend(prepared.parsed.iter().cloned());
        transaction.source_bytes += prepared.source.len();
        return Ok(data);
    }

    let pinned = v3_store::snapshot(path)
        .map_err(|e| (code_for_v3(&e), e.to_string()))?;
    if read || is_read_only {
        let report = pinned.database().query_parsed(&prepared.parsed, params)
            .map_err(|e| (ENGINE_ERROR, e.to_string()))?;
        let data = report_bytes(&report, true, capacity)?;
        return Ok(data);
    }

    // No explicit BEGIN: standalone stdlib/database_execute is autocommit,
    // but only AFTER the bounded response has been fully validated.
    let mut staged = pinned.database().clone();
    let report = staged.execute_isolated_parsed(&prepared.parsed, params)
        .map_err(|e| (ENGINE_ERROR, e.to_string()))?;
    let data = report_bytes(&report, false, capacity)?;
    v3_store::commit_staged_if_generation(
        path, pinned.generation, &staged, &prepared.parsed
    ).map_err(|e| (code_for_v3(&e), e.to_string()))?;
    Ok(data)
}

unsafe extern "C" fn open(
    _: *mut c_void, ptr: *const u8, len: usize, out: *mut ConnectionHandle,
) -> i32 {
    ffi_guard(|| {
        if out.is_null() { return INVALID; }
        unsafe { *out = std::ptr::null_mut(); }
        let path = match unsafe { utf8(ptr, len, MAX_PATH) } {
            Ok(path) if !path.is_empty() => path,
            _ => return BAD_INPUT,
        };
        let path = Path::new(path);
        if crate::validate_database_path(path).is_err() { return BAD_INPUT; }
        let owned = if path.is_absolute() {
            path.to_path_buf()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(path),
                Err(_) => return ENGINE_ERROR,
            }
        };
        // Fail closed on missing, legacy, corrupt, or incompatible stores,
        // and on a publicly accessible database parent or metadata entry.
        // For a large retained history use the separate offline full audit.
        if v3_store::snapshot(&owned).is_err() ||
           v3_store::verify_embedded_private_store(&owned).is_err() {
            return ENGINE_ERROR;
        }
        let holder = Box::new(Connection {
            path: owned,
            state: Mutex::new(State {
                transaction: None, statements_open: 0, last_error: String::new()
            }),
        });
        unsafe { *out = Box::into_raw(holder).cast(); }
        OK
    })
}
unsafe extern "C" fn close(_: *mut c_void, handle: ConnectionHandle) -> i32 {
    ffi_guard(|| {
        let Some(conn) = (unsafe { connection(handle) }) else { return INVALID; };
        {
            let state = match conn.state.lock() { Ok(v) => v, Err(_) => return ENGINE_ERROR };
            if state.statements_open != 0 { return TX_ERROR; }
        }
        // Dropping an uncommitted transaction is an implicit rollback.
        unsafe { drop(Box::from_raw(handle as *mut Connection)); }
        OK
    })
}
unsafe extern "C" fn prepare(
    _: *mut c_void, handle: ConnectionHandle,
    ptr: *const u8, len: usize, out: *mut StatementHandle,
) -> i32 {
    ffi_guard(|| {
        if out.is_null() { return INVALID; }
        unsafe { *out = std::ptr::null_mut(); }
        let Some(conn) = (unsafe { connection(handle) }) else { return INVALID; };
        let mut state = match conn.state.lock() { Ok(v) => v, Err(_) => return ENGINE_ERROR };
        let source = match unsafe { utf8(ptr, len, MAX_SOURCE) } {
            Ok(source) if !source.is_empty() => source,
            Ok(_) => return set_error(&mut state, BAD_INPUT, "empty NoeSQL statement"),
            Err(e) => return set_error(&mut state, BAD_INPUT, e),
        };
        let parsed = match parse(source) {
            Ok(statements) if !statements.is_empty() => statements,
            Ok(_) => return set_error(&mut state, BAD_INPUT, "empty parsed program"),
            Err(e) => return set_error(&mut state, BAD_INPUT, e),
        };
        if state.statements_open >= 256 {
            return set_error(&mut state, TX_ERROR, "more than 256 prepared statements");
        }
        let statement = Box::new(Prepared {
            owner: handle as usize, source: source.into(), parsed
        });
        state.statements_open += 1;
        state.last_error.clear();
        unsafe { *out = Box::into_raw(statement).cast(); }
        OK
    })
}
unsafe extern "C" fn release(
    _: *mut c_void, handle: ConnectionHandle, stmt: StatementHandle,
) -> i32 {
    ffi_guard(|| {
        let Some(conn) = (unsafe { connection(handle) }) else { return INVALID; };
        let Some(prepared) = (unsafe { statement(stmt) }) else { return INVALID; };
        if prepared.owner != handle as usize { return BAD_INPUT; }
        let mut state = match conn.state.lock() { Ok(v) => v, Err(_) => return ENGINE_ERROR };
        if state.statements_open == 0 {
            return set_error(&mut state, BAD_INPUT, "statement release underflow");
        }
        state.statements_open -= 1;
        unsafe { drop(Box::from_raw(stmt as *mut Prepared)); }
        OK
    })
}
unsafe fn invoke(
    handle: ConnectionHandle,
    stmt: StatementHandle, ptr: *const u8, len: usize,
    output: *mut u8, capacity: usize, written: *mut usize, read: bool,
) -> i32 {
    ffi_guard(|| {
        if written.is_null() || output.is_null() || capacity == 0 || capacity > MAX_RESPONSE {
            return INVALID;
        }
        unsafe { *written = 0; }
        let Some(conn) = (unsafe { connection(handle) }) else { return INVALID; };
        let Some(prepared) = (unsafe { statement(stmt) }) else { return INVALID; };
        if prepared.owner != handle as usize { return BAD_INPUT; }
        let mut state = match conn.state.lock() { Ok(v) => v, Err(_) => return ENGINE_ERROR };
        let bytes = match unsafe { input(ptr, len, MAX_BINDINGS) } {
            Ok(v) => v,
            Err(e) => {
                if let Some(tx) = &mut state.transaction { tx.failed = true; }
                return set_error(&mut state, BAD_INPUT, e);
            },
        };
        let params = match parse_bindings(bytes) {
            Ok(params) => params,
            Err(e) => {
                if let Some(tx) = &mut state.transaction { tx.failed = true; }
                return set_error(&mut state, BAD_INPUT, e);
            },
        };
        let data = match execute_inner(&conn.path, &mut state, prepared, &params, read, capacity) {
            Ok(data) => data,
            Err((status, error)) => {
                if let Some(tx) = &mut state.transaction { tx.failed = true; }
                return set_error(&mut state, status, error);
            },
        };
        match unsafe { write_response(&data, output, capacity, written) } {
            Ok(()) => { state.last_error.clear(); OK },
            Err((status, error)) => set_error(&mut state, status, error),
        }
    })
}
unsafe extern "C" fn execute(
    _: *mut c_void, conn: ConnectionHandle, stmt: StatementHandle,
    ptr: *const u8, len: usize, output: *mut u8, capacity: usize,
    written: *mut usize,
) -> i32 {
    unsafe { invoke(conn, stmt, ptr, len, output, capacity, written, false) }
}
unsafe extern "C" fn query(
    _: *mut c_void, conn: ConnectionHandle, stmt: StatementHandle,
    ptr: *const u8, len: usize, output: *mut u8, capacity: usize,
    written: *mut usize,
) -> i32 {
    unsafe { invoke(conn, stmt, ptr, len, output, capacity, written, true) }
}
unsafe extern "C" fn begin(_: *mut c_void, handle: ConnectionHandle) -> i32 {
    ffi_guard(|| {
        let Some(conn) = (unsafe { connection(handle) }) else { return INVALID; };
        let mut state = match conn.state.lock() { Ok(v) => v, Err(_) => return ENGINE_ERROR };
        if state.transaction.is_some() {
            return set_error(&mut state, TX_ERROR, "nested session-v2 transactions unsupported");
        }
        let pinned = match v3_store::snapshot(&conn.path) {
            Ok(value) => value,
            Err(e) => return set_error(&mut state, code_for_v3(&e), e.to_string()),
        };
        state.transaction = Some(Transaction {
            generation: pinned.generation,
            staged: pinned.database().clone(),
            statements: Vec::new(),
            source_bytes: 0,
            failed: false,
        });
        state.last_error.clear();
        OK
    })
}
unsafe extern "C" fn commit(_: *mut c_void, handle: ConnectionHandle) -> i32 {
    ffi_guard(|| {
        let Some(conn) = (unsafe { connection(handle) }) else { return INVALID; };
        let mut state = match conn.state.lock() { Ok(v) => v, Err(_) => return ENGINE_ERROR };
        let Some(tx) = state.transaction.take() else {
            return set_error(&mut state, TX_ERROR, "no active session-v2 transaction");
        };
        if tx.failed {
            return set_error(&mut state, TX_ERROR,
                "transaction had a failed statement; all staged changes discarded");
        }
        if tx.statements.is_empty() {
            state.last_error.clear();
            return OK;
        }
        // A failed/uncertain commit cannot be blindly retried: transaction
        // is cleared, and last_error differentiates stale versus uncertain.
        match v3_store::commit_staged_if_generation(
            &conn.path, tx.generation, &tx.staged, &tx.statements
        ) {
            Ok(_) => { state.last_error.clear(); OK },
            Err(e) => set_error(&mut state, code_for_v3(&e), e.to_string()),
        }
    })
}
unsafe extern "C" fn rollback(_: *mut c_void, handle: ConnectionHandle) -> i32 {
    ffi_guard(|| {
        let Some(conn) = (unsafe { connection(handle) }) else { return INVALID; };
        let mut state = match conn.state.lock() { Ok(v) => v, Err(_) => return ENGINE_ERROR };
        // Last-error is deliberately retained if the previous commit outcome
        // was uncertain; rollback cannot undo an already published HEAD.
        if state.transaction.take().is_none() {
            // Noqeri's native web adapter calls rollback when COMMIT failed.
            // Preserve the precise conflict / COMMIT OUTCOME UNCERTAIN error:
            // rollback after a failed commit cannot reverse a published HEAD.
            if state.last_error.is_empty() {
                state.last_error = "no active transaction to roll back".into();
            }
            return TX_ERROR;
        }
        state.last_error.clear();
        OK
    })
}
unsafe extern "C" fn last_error(
    _: *mut c_void, handle: ConnectionHandle,
    output: *mut u8, capacity: usize, written: *mut usize,
) -> i32 {
    ffi_guard(|| {
        let Some(conn) = (unsafe { connection(handle) }) else { return INVALID; };
        let state = match conn.state.lock() { Ok(v) => v, Err(_) => return ENGINE_ERROR };
        match unsafe { write_response(state.last_error.as_bytes(), output, capacity, written) } {
            Ok(()) => OK,
            Err((code, _)) => code,
        }
    })
}

// Only this immutable, null-context callback table is declared Sync. The
// session handles themselves are protected by Mutex and are not global.
struct SharedTable(NoeSqlSessionV2);
// SAFETY: the callback table stores only immutable pointers to functions and
// a null context; it has no interior mutable state.
unsafe impl Sync for SharedTable {}
static PROVIDER: SharedTable = SharedTable(NoeSqlSessionV2 {
    abi_version: SESSION_ABI_VERSION,
    context: std::ptr::null_mut(),
    open: Some(open),
    close: Some(close),
    prepare: Some(prepare),
    release: Some(release),
    execute: Some(execute),
    query: Some(query),
    begin: Some(begin),
    commit: Some(commit),
    rollback: Some(rollback),
    last_error: Some(last_error),
});

#[no_mangle]
pub extern "C" fn noesql_session_v2_get() -> *const NoeSqlSessionV2 {
    &PROVIDER.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::Database;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn database_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "noesql-native-v2-{label}-{}-{nonce}", std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        directory.join("database.noedb")
    }
    fn create(path: &Path) {
        let mut db = Database::default();
        db.execute("table users { id:int key auto\n name:text required unique }", &Params::new()).unwrap();
        v3_store::create_new(path, &db).unwrap();
    }
    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(format!("{}.lck", path.display()));
        let _ = fs::remove_dir_all(format!("{}.versions", path.display()));
        let _ = fs::remove_dir(path.parent().unwrap());
    }
    struct TestConnection {
        api: &'static NoeSqlSessionV2,
        handle: ConnectionHandle,
    }
    impl TestConnection {
        fn new(path: &Path) -> Self {
            let api = unsafe { &*noesql_session_v2_get() };
            let mut handle = std::ptr::null_mut();
            let p = path.to_str().unwrap().as_bytes();
            assert_eq!(unsafe { api.open.unwrap()(
                api.context,p.as_ptr(),p.len(),&mut handle
            ) },0);
            Self { api, handle }
        }
        fn prepare(&self, source: &str) -> StatementHandle {
            let mut stmt = std::ptr::null_mut();
            assert_eq!(unsafe { self.api.prepare.unwrap()(
                self.api.context,self.handle,source.as_ptr(),source.len(),&mut stmt
            ) },0);
            stmt
        }
        fn exec(&self, stmt: StatementHandle, params: &str, read: bool, cap: usize) -> (i32,String) {
            let mut output = vec![0; cap];
            let mut written = 0usize;
            let callback = if read { self.api.query.unwrap() } else { self.api.execute.unwrap() };
            let code = unsafe { callback(
                self.api.context, self.handle, stmt,
                params.as_ptr(), params.len(),
                output.as_mut_ptr(), output.len(), &mut written
            ) };
            (code, String::from_utf8_lossy(&output[..written.min(cap)]).into_owned())
        }
        fn begin(&self) -> i32 { unsafe { self.api.begin.unwrap()(self.api.context,self.handle) } }
        fn commit(&self) -> i32 { unsafe { self.api.commit.unwrap()(self.api.context,self.handle) } }
        fn rollback(&self) -> i32 { unsafe { self.api.rollback.unwrap()(self.api.context,self.handle) } }
        fn release(&self, s: StatementHandle) {
            assert_eq!(unsafe { self.api.release.unwrap()(self.api.context,self.handle,s) },0);
        }
    }
    impl Drop for TestConnection {
        fn drop(&mut self) {
            assert_eq!(unsafe { self.api.close.unwrap()(self.api.context,self.handle) },0);
        }
    }


    #[test]
    fn bounded_row_encoding_matches_reference_json_and_prevents_output_growth() {
        use crate::reference::rows_json;
        use crate::value::Value;
        let mut row = std::collections::BTreeMap::new();
        row.insert("name".into(), Value::Text("Quote \" <>&\n🚀".into()));
        row.insert("count".into(), Value::Int(2));
        let report = ExecutionReport { affected: 0, rows: vec![row] };
        let reference = rows_json(&report.rows);
        assert_eq!(report_bytes(&report, true, reference.len()).unwrap(), reference.as_bytes());
        assert!(matches!(
            report_bytes(&report, true, reference.len()-1),
            Err((TOO_SMALL, _))
        ));
        assert_eq!(report_bytes(&report, false, 10).unwrap(), b"affected=0");
    }

    #[test]
    fn native_session_v2_matches_noqeri_framework_transaction_and_json_contract() {
        let path = database_path("callback");
        create(&path);
        let client = TestConnection::new(&path);
        let add = client.prepare("users add {name:$name}");
        let read = client.prepare("users get name sort name");
        assert_eq!(client.begin(),0);
        assert_eq!(client.exec(add, r#"{"name":"A & B <script>"}"#,false,1024),
            (0,"affected=1".into()));
        assert!(client.exec(read,"{}",true,1024).1.contains("A & B <script>"));
        assert_eq!(v3_store::snapshot(&path).unwrap()
            .query("users get name",&Params::new()).unwrap().rows.len(),0,
            "uncommitted session rows must not reach durable HEAD");
        assert_eq!(client.commit(),0);
        assert_eq!(v3_store::snapshot(&path).unwrap().generation,2);
        assert!(client.exec(read,"{}",true,1024).1.contains("A & B <script>"));
        assert_eq!(client.begin(),0);
        assert_eq!(client.exec(add, r#"{"name":"discard"}"#,false,1024).0,0);
        assert_eq!(client.rollback(),0);
        assert_eq!(v3_store::snapshot(&path).unwrap()
            .query("users get name",&Params::new()).unwrap().rows.len(),1);
        client.release(add); client.release(read);
        drop(client);
        cleanup(&path);
    }

    #[test]
    fn noesql_session_v2_overflow_failed_queries_and_stale_writer_never_commit() {
        let path = database_path("guards");
        create(&path);
        let first = TestConnection::new(&path);
        let second = TestConnection::new(&path);
        let add = first.prepare("users add {name:$name}");
        let query = first.prepare("users get name");
        assert_eq!(first.begin(),0);
        assert_eq!(first.exec(add,r#"{"name":"not committed"}"#,false,1).0,TOO_SMALL);
        assert_eq!(first.commit(),TX_ERROR);
        assert_eq!(v3_store::snapshot(&path).unwrap().generation,1);
        assert_eq!(first.begin(),0);
        assert_eq!(first.exec(add,r#"{"name":"stale"}"#,false,1024).0,0);
        let other_add = second.prepare("users add {name:$name}");
        assert_eq!(second.exec(other_add,r#"{"name":"winner"}"#,false,1024).0,0);
        assert_eq!(first.commit(),CONFLICT);
        let response = first.exec(query,"{}",true,1024);
        assert_eq!(response.0,0);
        assert!(response.1.contains("winner"));
        assert!(!response.1.contains("stale"));
        assert_eq!(v3_store::snapshot(&path).unwrap().generation,2);
        assert_eq!(first.exec(add,r#"{"name":"without interpolation' OR 1=1"}"#,false,1024).0,0);
        assert_eq!(v3_store::snapshot(&path).unwrap().generation,3);
        first.release(add);first.release(query);second.release(other_add);
        drop(first);drop(second);
        cleanup(&path);
    }

    #[test]
    fn v2_sessions_commit_validated_integer_bplus_rows_atomically() {
        let path = database_path("index");
        let mut db = Database::default();
        db.execute(
            "table accounts { id:int key auto\n tenant:int required }\nindex by_tenant on accounts(tenant)",
            &Params::new()
        ).unwrap();
        v3_store::create_new(&path, &db).unwrap();
        let client = TestConnection::new(&path);
        let insert = client.prepare("accounts add {tenant:$tenant}");
        assert_eq!(client.begin(),0);
        assert_eq!(client.exec(insert,r#"{"tenant":11}"#,false,1024).0,0);
        assert_eq!(client.exec(insert,r#"{"tenant":{"kind":"int","value":"22"}}"#,false,1024).0,0);
        assert_eq!(client.commit(),0);
        let reopened = v3_store::snapshot(&path).unwrap();
        assert_eq!(reopened.generation,2);
        assert_eq!(reopened.query(
            "accounts get id where tenant == 11", &Params::new()
        ).unwrap().rows.len(),1);
        assert_eq!(reopened.query(
            "accounts get id where tenant == 22", &Params::new()
        ).unwrap().rows.len(),1);

        let duplicate = client.prepare("accounts add {id:1,tenant:44}");
        assert_eq!(client.begin(),0);
        assert_eq!(client.exec(insert,r#"{"tenant":33}"#,false,1024).0,0);
        assert_eq!(client.exec(duplicate,"{}",false,1024).0,ENGINE_ERROR);
        assert_eq!(client.rollback(),0);
        assert_eq!(v3_store::snapshot(&path).unwrap().generation,2);
        assert_eq!(v3_store::snapshot(&path).unwrap()
            .query("accounts get id", &Params::new()).unwrap().rows.len(),2);
        client.release(insert);client.release(duplicate);
        drop(client);
        cleanup(&path);
    }

    #[test]
    fn a_failed_statement_poisoned_transaction_cannot_commit_prior_writes() {
        let path = database_path("abort-on-error");
        create(&path);
        let client = TestConnection::new(&path);
        let add = client.prepare("users add {name:$name}");
        assert_eq!(client.begin(), OK);
        assert_eq!(client.exec(add, r#"{"name":"first"}"#, false, 1024).0, OK);
        assert_eq!(client.exec(add, r#"{"name":"first"}"#, false, 1024).0, ENGINE_ERROR);
        assert_eq!(client.exec(add, r#"{"name":"second"}"#, false, 1024).0, TX_ERROR);
        assert_eq!(client.commit(), TX_ERROR);
        assert_eq!(v3_store::snapshot(&path).unwrap().generation, 1);
        assert_eq!(v3_store::snapshot(&path).unwrap()
            .query("users get name", &Params::new()).unwrap().rows.len(), 0);
        assert_eq!(client.begin(), OK);
        assert_eq!(client.exec(add, r#"{"name":"recovered"}"#, false, 1024).0, OK);
        assert_eq!(client.commit(), OK);
        assert_eq!(v3_store::snapshot(&path).unwrap().generation, 2);
        client.release(add);
        drop(client);
        cleanup(&path);
    }

    #[test]
    fn rust_session_v2_wrapper_uses_real_provider_contract() {
        let path = database_path("rust-wrapper");
        create(&path);
        let provider = unsafe { &*noesql_session_v2_get() };
        let session = crate::Session::open(provider, &path).unwrap();
        session.begin().unwrap();
        {
            let statement = session.prepare("users add {name:$name}").unwrap();
            assert_eq!(statement.execute(br#"{"name":"wrapper"}"#).unwrap(), b"affected=1");
        }
        session.commit().unwrap();
        {
            let read = session.prepare("users get name").unwrap();
            let json = read.query(b"{}").unwrap();
            assert!(String::from_utf8(json).unwrap().contains("wrapper"));
        }
        drop(session);
        cleanup(&path);
    }

    #[test]
    fn malformed_binding_and_restricted_read_do_not_publish_writes() {
        let path = database_path("rejections");
        create(&path);
        let client = TestConnection::new(&path);
        let add = client.prepare("users add {name:$name}");
        assert_eq!(client.begin(),0);
        assert_eq!(client.exec(add,r#"{"name":true,"name":1}"#,false,1024).0,BAD_INPUT);
        assert_eq!(client.exec(add,r#"{"name":"x"}"#,true,1024).0,BAD_INPUT);
        assert_eq!(client.rollback(),0);
        assert_eq!(v3_store::snapshot(&path).unwrap().generation,1);
        client.release(add);
        drop(client);
        cleanup(&path);
    }
}

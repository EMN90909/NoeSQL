//! Native in-process relational execution boundary for trusted Noqeri AOT
//! hosts. No subprocess and no string interpolation of query parameters.
//! Rust executes the complete reference relational AST and Zig WAL persists
//! commits; this is not a native Noqeri NIR relational code generator.
//!
//! WARNING: this low-level C ABI does not authenticate a caller. Never expose
//! it as a public SQL service or tenant boundary. Only trusted embedded hosts
//! with private database files can use it until durable authorization exists.
use crate::decimal::Decimal;
use crate::reference::Params;
use crate::value::Value;
use crate::{validate_database_path, MAX_OUTPUT_BYTES, MAX_SOURCE_BYTES};
use std::collections::btree_map::Entry;
use std::path::Path;

// All pointers below are borrowed for this call only.
// Kind: 0=NULL, 1=bool, 2=int64, 3=text, 4=decimal,
// 5=date, 6=datetime, 7=uuid, 8=json, 9=binary.
// Extended text-backed types are checked by the relational schema executor.
#[repr(C)]
pub struct RelationalBindingV1 {
    pub name: *const u8,
    pub name_len: usize,
    pub value: *const u8,
    pub value_len: usize,
    pub kind: u32,
}

pub const ABI_VERSION: u32 = 1;
pub const OK: i32 = 0;
pub const INVALID_ARGUMENT: i32 = -1;
pub const INVALID_UTF8: i32 = -2;
pub const BUFFER_TOO_SMALL: i32 = -3;
pub const INVALID_BINDING: i32 = -4;
pub const EXECUTION_FAILURE: i32 = -5;

#[no_mangle]
pub extern "C" fn noesql_relational_abi_version() -> u32 { ABI_VERSION }

fn parse_binding(kind: u32, bytes: &[u8]) -> Option<Value> {
    match kind {
        0 if bytes.is_empty() => Some(Value::Null),
        1 if bytes == b"true" => Some(Value::Bool(true)),
        1 if bytes == b"false" => Some(Value::Bool(false)),
        2 => {
            let s = std::str::from_utf8(bytes).ok()?;
            s.parse::<i64>().ok().map(Value::Int)
        },
        3 | 5 | 6 | 7 | 8 | 9 => std::str::from_utf8(bytes).ok().map(|s| Value::Text(s.into())),
        4 => Decimal::parse(std::str::from_utf8(bytes).ok()?).map(Value::Decimal),
        _ => None,
    }
}

// SAFETY: A C ABI caller promises each nonnull pointer is valid for its
// supplied length, with no concurrent changes during this function call.
unsafe fn read_bytes<'a>(ptr: *const u8, length: usize, limit: usize) -> Result<&'a [u8], i32> {
    if length > limit { return Err(INVALID_ARGUMENT); }
    if length == 0 { return Ok(&[]); }
    if ptr.is_null() { return Err(INVALID_ARGUMENT); }
    Ok(unsafe { std::slice::from_raw_parts(ptr, length) })
}

/// Execute one NoeSQL program and return JSON {affected, rows}.
/// A write NEVER commits when the provided response buffer is undersized.
/// There is deliberately no output-size discovery call because calling it
/// twice with a mutating query could duplicate writes.
///
/// SAFETY: all buffer pointers must point to valid, non-overlapping readable
/// or writable memory of the specified lengths for the duration of the call.
/// `written` must point to one writable usize. Callers must allocate enough
/// response space before executing; 16 MiB is the maximum permitted capacity.
#[no_mangle]
pub unsafe extern "C" fn noesql_relational_exec_v1(
    path: *const u8, path_len: usize,
    source: *const u8, source_len: usize,
    bindings: *const RelationalBindingV1, binding_count: usize,
    output: *mut u8, output_capacity: usize, written: *mut usize,
) -> i32 {
    if written.is_null() || output.is_null() || output_capacity == 0
        || output_capacity > MAX_OUTPUT_BYTES || binding_count > 256 {
        return INVALID_ARGUMENT;
    }
    unsafe { *written = 0; }
    let path = match unsafe { read_bytes(path, path_len, 4096) } {
        Ok(bytes) => bytes, Err(code) => return code,
    };
    let source = match unsafe { read_bytes(source, source_len, MAX_SOURCE_BYTES) } {
        Ok(bytes) => bytes, Err(code) => return code,
    };
    let path = match std::str::from_utf8(path) { Ok(s) => s, Err(_) => return INVALID_UTF8 };
    let source = match std::str::from_utf8(source) { Ok(s) => s, Err(_) => return INVALID_UTF8 };
    if validate_database_path(Path::new(path)).is_err() { return INVALID_ARGUMENT; }
    if binding_count > 0 && bindings.is_null() { return INVALID_ARGUMENT; }
    let ffi_bindings = if binding_count == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(bindings, binding_count) }
    };
    let mut params = Params::new();
    for binding in ffi_bindings {
        let name = match unsafe { read_bytes(binding.name, binding.name_len, 64) } {
            Ok(bytes) => bytes, Err(_) => return INVALID_BINDING,
        };
        if name.is_empty() || !name[0].is_ascii_alphabetic() && name[0] != b'_'
            || !name.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_') {
            return INVALID_BINDING;
        }
        let name = match std::str::from_utf8(name) {
            Ok(name) => name, Err(_) => return INVALID_BINDING,
        };
        let value = match unsafe { read_bytes(binding.value, binding.value_len, 1024 * 1024) } {
            Ok(bytes) => bytes, Err(_) => return INVALID_BINDING,
        };
        let value = match parse_binding(binding.kind, value) {
            Some(v) => v, None => return INVALID_BINDING,
        };
        match params.entry(name.to_string()) {
            Entry::Vacant(e) => { e.insert(value); },
            Entry::Occupied(_) => return INVALID_BINDING,
        }
    }
    // Locked execution checks the size BEFORE writing the WAL commit record.
    match crate::zig_pager::execute_locked_bounded(Path::new(path), source, &params, output_capacity) {
        Ok((_, json)) => {
            if json.len() > output_capacity { return BUFFER_TOO_SMALL; }
            unsafe {
                std::ptr::copy_nonoverlapping(json.as_ptr(), output, json.len());
                *written = json.len();
            }
            OK
        },
        Err(error) if error.starts_with("response of ") => BUFFER_TOO_SMALL,
        Err(_) => EXECUTION_FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_binding_types_do_not_coerce_or_interpolate() {
        assert_eq!(parse_binding(0, b""), Some(Value::Null));
        assert_eq!(parse_binding(1, b"true"), Some(Value::Bool(true)));
        assert_eq!(parse_binding(1, b"True"), None);
        assert_eq!(parse_binding(2, b"-9223372036854775808"), Some(Value::Int(i64::MIN)));
        assert_eq!(parse_binding(2, b"999999999999999999999"), None);
        assert_eq!(parse_binding(3, b"' OR 1=1"), Some(Value::Text("' OR 1=1".into())));
        assert_eq!(parse_binding(4, b"1.250000"), Some(Value::Decimal(Decimal::parse("1.25").unwrap())));
        assert_eq!(parse_binding(4, b"1.2500001"), None);
        assert_eq!(parse_binding(9, b"0x123abc"), Some(Value::Text("0x123abc".into())));
    }
    #[test]
    fn small_response_buffer_does_not_commit_mutation() {
        let path = std::env::temp_dir().join(format!(
            "noesql-relational-abi-{}-{}.noedb", std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        for suffix in ["", ".wal", ".lck"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
        let source = b"table clients { id:int key auto }\nclients add {}";
        let path_str = path.to_str().unwrap().as_bytes();
        let mut buffer = [0u8; 2];
        let mut written = 100usize;
        let code = unsafe {
            noesql_relational_exec_v1(path_str.as_ptr(), path_str.len(),
                source.as_ptr(), source.len(), std::ptr::null(), 0,
                buffer.as_mut_ptr(), buffer.len(), &mut written)
        };
        assert_eq!(code, BUFFER_TOO_SMALL);
        assert_eq!(written, 0);
        let db = crate::zig_pager::load_or_default(&path).unwrap();
        assert!(db.tables.is_empty(), "insufficient response space committed DDL");
        let mut good = [0u8; 1024];
        let code = unsafe {
            noesql_relational_exec_v1(path_str.as_ptr(), path_str.len(),
                source.as_ptr(), source.len(), std::ptr::null(), 0,
                good.as_mut_ptr(), good.len(), &mut written)
        };
        assert_eq!(code, OK);
        assert!(String::from_utf8_lossy(&good[..written]).contains("affected"));
        let db = crate::zig_pager::load_or_default(&path).unwrap();
        assert!(db.tables.contains_key("clients"));
        for suffix in ["", ".wal", ".lck"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }
}

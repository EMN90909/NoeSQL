//! Loopback-only NoeSQL v3 HTTP/1.1 bridge for Noqeri std.http.client.
//! Not a public TLS/multi-tenant service. No CORS, no remotely supplied paths.
use crate::reference::{self, Database, Params, Statement};
use crate::session_bindings::{parse_rest_request, RestRequest};
use crate::v3_store::{self, V3Error};
use crate::value::Value;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 128 * 1024;
const MAX_SOURCE: usize = 65_536;
const MAX_RESPONSE: usize = 1024 * 1024;
const MAX_ROWS: usize = 1024;
const MAX_CLIENTS: usize = 32;

#[derive(Clone)]
struct Config { db: PathBuf, scripts: Option<PathBuf>, token: Vec<u8> }
struct Request { method: String, path: String, authorization: Option<String>,
    content_type: Option<String>, origin: bool, body: Vec<u8> }
struct Reply { status: u16, body: Vec<u8> }
fn quote(s: &str) -> String {
    let mut buf = String::new();
    Value::Text(s.to_owned()).encode_json(&mut buf);
    buf
}
impl Reply {
    fn json(status: u16, body: String) -> Self { Self { status, body: body.into_bytes() } }
    fn error(status: u16, code: &str, detail: &str) -> Self {
        Self::json(status, format!("{{\"error\":{},\"message\":{}}}", quote(code), quote(detail)))
    }
}
fn is_dml(statement: &Statement) -> bool {
    match statement {
        Statement::Add(_, _) | Statement::Set(_, _, _) | Statement::Delete(_, _) => true,
        Statement::Atomic(nested) => !nested.is_empty() && nested.iter().all(is_dml),
        _ => false,
    }
}
fn private_dir(root: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if !root.is_absolute() { return Err("scripts root must be absolute".into()); }
    let m = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if !m.file_type().is_dir() || m.permissions().mode() & 0o077 != 0 {
        return Err("scripts root must be an owner-private non-symlink directory".into());
    }
    Ok(())
}
fn approved_script(root: &Path, name: &str) -> Result<String, Reply> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    if name.len() < 7 || name.len() > 96 || !name.ends_with(".noeqr") ||
       name.starts_with('.') || name.contains("..") ||
       !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.') {
        return Err(Reply::error(400, "bad_script", "only a .noeqr basename is allowed"));
    }
    let file = root.join(name);
    let before = fs::symlink_metadata(&file)
        .map_err(|_| Reply::error(404, "script_missing", "script not found"))?;
    if !before.file_type().is_file() || before.permissions().mode() & 0o077 != 0 {
        return Err(Reply::error(403, "script_denied", "script is not a private regular file"));
    }
    let reader = OpenOptions::new().read(true).custom_flags(0x20000) // Linux O_NOFOLLOW
        .open(file).map_err(|_| Reply::error(403, "script_denied", "script open denied"))?;
    let after = reader.metadata()
        .map_err(|_| Reply::error(403, "script_denied", "script metadata unavailable"))?;
    if !after.is_file() || before.ino() != after.ino() || before.dev() != after.dev() {
        return Err(Reply::error(403, "script_denied", "script changed during open"));
    }
    let mut bytes = Vec::new();
    reader.take((MAX_SOURCE + 1) as u64).read_to_end(&mut bytes)
        .map_err(|_| Reply::error(503, "script_error", "script read failure"))?;
    if bytes.is_empty() || bytes.len() > MAX_SOURCE {
        return Err(Reply::error(413, "script_size", "script must be 1..65536 bytes"));
    }
    String::from_utf8(bytes).map_err(|_| Reply::error(400, "script_utf8", "script must be UTF-8"))
}
fn source(config: &Config, req: RestRequest) -> Result<(String, Params), Reply> {
    let text = if let Some(text) = req.source { text } else if let Some(script) = req.script {
        let root = config.scripts.as_ref().ok_or_else(|| Reply::error(
            403, "scripts_disabled", "server has no approved scripts directory"))?;
        approved_script(root, &script)?
    } else { return Err(Reply::error(400, "bad_request", "source or script required")); };
    Ok((text, req.params))
}
fn storage_error(error: V3Error) -> Reply {
    match error {
        V3Error::Conflict(_) => Reply::error(409, "conflict", "writer conflict; retry after reading"),
        V3Error::Access(_) => Reply::error(403, "denied", "access policy denied request"),
        V3Error::CommitUncertain(_) => Reply::error(503, "commit_uncertain",
            "commit may have published; check generation before retrying"),
        other => {
            eprintln!("NoeSQL REST database failure: {other}");
            Reply::error(503, "database_unavailable", "database unavailable")
        },
    }
}
fn token_eq(actual: &[u8], expected: &[u8]) -> bool {
    if actual.len() > 256 { return false; }
    let mut diff = actual.len() ^ expected.len();
    for (i, byte) in expected.iter().enumerate() {
        diff |= (actual.get(i).copied().unwrap_or_default() ^ byte) as usize;
    }
    diff == 0
}
fn route(config: &Config, request: Request) -> Reply {
    if request.method == "GET" && request.path == "/health" {
        return Reply::json(200, "{\"status\":\"ok\"}".into());
    }
    if request.origin { return Reply::error(403, "origin_denied", "browser origins not permitted"); }
    let authorized = request.authorization.as_deref().and_then(|s| s.strip_prefix("Bearer "))
        .is_some_and(|secret| token_eq(secret.as_bytes(), &config.token));
    if !authorized { return Reply::error(401, "unauthorized", "bearer token required"); }
    if request.method == "GET" && request.path == "/v1/status" {
        return match v3_store::inspect(&config.db) {
            Ok((generation, bytes, pages)) => Reply::json(200, format!(
                "{{\"format\":\"NQDB0003\",\"generation\":{generation},\"bytes\":{bytes},\"pages\":{pages}}}"
            )),
            Err(error) => storage_error(error),
        };
    }
    if request.method != "POST" ||
       !matches!(request.path.as_str(), "/v1/query" | "/v1/execute") {
        return Reply::error(404, "not_found", "unknown endpoint");
    }
    if request.content_type.as_deref() != Some("application/json") {
        return Reply::error(415, "content_type", "Content-Type must be application/json");
    }
    let req = match parse_rest_request(&request.body) {
        Ok(req) => req, Err(err) => return Reply::error(400, "bad_json", &err),
    };
    let (source, params) = match source(config, req) {
        Ok(value) => value, Err(reply) => return reply,
    };
    let statements = match reference::parse(&source) {
        Ok(list) if !list.is_empty() && list.len() <= 32 => list,
        Ok(_) => return Reply::error(400, "statements", "1..32 statements allowed"),
        Err(err) => return Reply::error(422, "invalid_noeqr", &err.to_string()),
    };
    if request.path == "/v1/query" {
        if !Database::statements_read_only(&statements) {
            return Reply::error(403, "read_only", "query endpoint rejects mutations");
        }
        let snapshot = match v3_store::snapshot(&config.db) {
            Ok(value) => value, Err(err) => return storage_error(err),
        };
        let report = match snapshot.database().query_parsed(&statements, &params) {
            Ok(value) => value, Err(err) => return Reply::error(422, "query_failed", &err.to_string()),
        };
        if report.rows.len() > MAX_ROWS {
            return Reply::error(413, "rows_limit", "query exceeded 1024 rows");
        }
        // Source engine still materializes query rows; this is not a CPU/
        // memory budget or a security boundary for untrusted multi-tenancy.
        let rows = reference::rows_json(&report.rows);
        if rows.len() > MAX_RESPONSE {
            return Reply::error(413, "response_limit", "query exceeded 1 MiB response");
        }
        Reply::json(200, format!("{{\"rows\":{},\"count\":{}}}", rows, report.rows.len()))
    } else {
        // DDL and schema migrations remain local-CLI-only.
        if !statements.iter().all(is_dml) {
            return Reply::error(403, "dml_only", "HTTP execute only accepts add/set/delete");
        }
        match v3_store::execute(&config.db, &source, &params) {
            Ok(report) => Reply::json(200, format!("{{\"affected\":{}}}", report.affected)),
            Err(err) => storage_error(err),
        }
    }
}
fn read_request(stream: &mut TcpStream) -> Result<Request, Reply> {
    let mut data = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") { break at + 4; }
        if data.len() > MAX_HEADER {
            return Err(Reply::error(431, "headers_limit", "headers exceed 16 KiB"));
        }
        let n = stream.read(&mut chunk)
            .map_err(|_| Reply::error(408, "timeout", "request read timeout"))?;
        if n == 0 { return Err(Reply::error(400, "incomplete", "request headers incomplete")); }
        data.extend_from_slice(&chunk[..n]);
    };
    if header_end > MAX_HEADER { return Err(Reply::error(431, "headers_limit", "headers too large")); }
    let head = std::str::from_utf8(&data[..header_end-4])
        .map_err(|_| Reply::error(400, "bad_headers", "headers must be UTF-8"))?;
    let mut lines = head.split("\r\n");
    let mut start = lines.next().unwrap_or("").split(' ');
    let (Some(method), Some(path), Some(version), None) =
        (start.next(), start.next(), start.next(), start.next()) else {
            return Err(Reply::error(400, "bad_request", "invalid request line"));
        };
    if version != "HTTP/1.1" || !matches!(method, "GET" | "POST") ||
       !path.starts_with('/') || path.contains('?') || path.contains('#') {
        return Err(Reply::error(400, "bad_request", "HTTP/1.1 GET/POST absolute path required"));
    }
    let method = method.to_owned();
    let path = path.to_owned();
    let mut length = None;
    let mut authorization = None;
    let mut content_type = None;
    let mut origin = false;
    let mut host = false;
    for (i, line) in lines.enumerate() {
        if i >= 48 || line.starts_with(' ') || line.starts_with('\t') {
            return Err(Reply::error(400, "bad_headers", "too many or folded headers"));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(Reply::error(400, "bad_headers", "malformed header"));
        };
        let value = value.trim();
        if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') ||
           value.bytes().any(|b| b < 32 && b != b'\t') {
            return Err(Reply::error(400, "bad_headers", "invalid header field"));
        }
        match name.to_ascii_lowercase().as_str() {
            "host" if !host => host = true,
            "host" => return Err(Reply::error(400, "bad_headers", "duplicate Host")),
            "content-length" => {
                if length.is_some() || value.is_empty() ||
                   !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Reply::error(400, "bad_length", "invalid Content-Length"));
                }
                let n = value.parse::<usize>()
                    .map_err(|_| Reply::error(413, "body_limit", "Content-Length overflow"))?;
                if n > MAX_BODY { return Err(Reply::error(413, "body_limit", "body exceeds 128KiB")); }
                length = Some(n);
            },
            "authorization" => if authorization.replace(value.to_owned()).is_some() {
                return Err(Reply::error(400, "bad_headers", "duplicate Authorization"));
            },
            "content-type" => if content_type.replace(value.to_ascii_lowercase()).is_some() {
                return Err(Reply::error(400, "bad_headers", "duplicate Content-Type"));
            },
            "origin" => origin = true,
            "transfer-encoding" | "content-encoding" | "expect" => {
                return Err(Reply::error(400, "unsupported_encoding",
                    "chunked, compressed and 100-continue requests forbidden"));
            },
            _ => {},
        }
    }
    if !host { return Err(Reply::error(400, "bad_headers", "Host required")); }
    let body_length = if method == "POST" {
        length.ok_or_else(|| Reply::error(411, "length_required", "POST requires Content-Length"))?
    } else {
        if length.unwrap_or_default() != 0 {
            return Err(Reply::error(400, "bad_request", "GET body forbidden"));
        }
        0
    };
    let total = header_end + body_length;
    if data.len() > total {
        return Err(Reply::error(400, "pipelining", "one HTTP request per connection"));
    }
    while data.len() < total {
        let want = (total - data.len()).min(chunk.len());
        let n = stream.read(&mut chunk[..want])
            .map_err(|_| Reply::error(408, "timeout", "body read timeout"))?;
        if n == 0 { return Err(Reply::error(400, "incomplete", "truncated body")); }
        data.extend_from_slice(&chunk[..n]);
    }
    Ok(Request { method, path, authorization, content_type, origin,
        body: data[header_end..].to_vec() })
}
fn send(stream: &mut TcpStream, reply: Reply) -> std::io::Result<()> {
    let reason = match reply.status {
        200 => "OK", 400 => "Bad Request", 401 => "Unauthorized",
        403 => "Forbidden", 404 => "Not Found", 408 => "Request Timeout",
        409 => "Conflict", 411 => "Length Required", 413 => "Content Too Large",
        415 => "Unsupported Media Type", 422 => "Unprocessable Content",
        431 => "Headers Too Large", 503 => "Service Unavailable", _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        reply.status, reason, reply.body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(&reply.body)?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}
fn client(mut stream: TcpStream, config: &Config) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let reply = match read_request(&mut stream) {
        Ok(request) => route(config, request), Err(reply) => reply,
    };
    let _ = send(&mut stream, reply);
}
struct ClientSlot(Arc<AtomicUsize>);
impl Drop for ClientSlot { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
pub fn serve(path: &Path, port: u16, scripts: Option<PathBuf>) -> Result<(), String> {
    if port == 0 { return Err("REST port must be 1..65535".into()); }
    crate::validate_database_path(path).map_err(|e| e.to_string())?;
    if !path.is_absolute() { return Err("REST database path must be absolute".into()); }
    v3_store::verify_embedded_private_store(path).map_err(|e| e.to_string())?;
    v3_store::snapshot(path).map_err(|e| e.to_string())?;
    if let Some(root) = scripts.as_ref() { private_dir(root)?; }
    let token = std::env::var("NOESQL_API_TOKEN")
        .map_err(|_| "NOESQL_API_TOKEN must be set".to_string())?;
    if token.len() < 32 || token.len() > 256 ||
       !token.bytes().all(|c| (33..=126).contains(&c)) {
        return Err("NOESQL_API_TOKEN must be 32..256 printable ASCII bytes".into());
    }
    let config = Arc::new(Config {
        db: path.to_owned(), scripts, token: token.into_bytes(),
    });
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("cannot bind IPv4 loopback port: {e}"))?;
    let active = Arc::new(AtomicUsize::new(0));
    eprintln!("NoeSQL REST listening on 127.0.0.1:{port} (token required)");
    for incoming in listener.incoming() {
        match incoming {
            Ok(mut stream) => {
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CLIENTS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                    let _ = send(&mut stream, Reply::error(
                        503, "busy", "server concurrency budget exceeded"));
                    continue;
                }
                let slot = ClientSlot(Arc::clone(&active));
                let cfg = Arc::clone(&config);
                thread::spawn(move || { let _slot = slot; client(stream, &cfg); });
            },
            Err(e) => eprintln!("NoeSQL REST accept error: {e}"),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_denied_before_database_access() {
        let config = Config { db: PathBuf::from("/missing.noedb"), scripts: None,
            token: b"01234567890123456789012345678901".to_vec() };
        let req = |auth: Option<&str>, origin: bool| Request {
            method: "POST".into(), path: "/v1/execute".into(),
            authorization: auth.map(str::to_owned), content_type: Some("application/json".into()),
            origin, body: br#"{"source":"users add {id:1}"}"#.to_vec(),
        };
        assert_eq!(route(&config, req(None, false)).status, 401);
        assert_eq!(route(&config, req(Some("Bearer wrong"), false)).status, 401);
        assert_eq!(route(&config, req(Some(
            "Bearer 01234567890123456789012345678901"), true)).status, 403);
    }
    #[test]
    fn schema_and_ddl_rejected_by_http_dml_gate() {
        assert!(!is_dml(&reference::parse("table entries { id:int key auto }").unwrap()[0]));
        assert!(is_dml(&reference::parse("entries add {id:1}").unwrap()[0]));
    }
}

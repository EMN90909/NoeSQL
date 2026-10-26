use noesql_runtime::reference::{parse, rows_json, Params};
use noesql_runtime::value::Value;
use noesql_runtime::{read_source, validate_database_path};
use std::path::Path;

fn usage() -> ! {
    eprintln!(
        "NoeSQL native runtime\n\nusage:\n  noesql version\n  noesql check <file.noe>\n  noesql check-db <file.noedb>\n  noesql run <file.noedb> <script.noe>\n  noesql query <file.noedb> <query>\n  noesql exec <file.noedb> <NoeSQL-source> [name:type:value ...]"
    );
    std::process::exit(2);
}

fn fail(error: impl std::fmt::Display) -> ! {
    eprintln!("error: {error}");
    std::process::exit(1);
}

#[cfg(feature = "zig-pager")]
fn run_script(database_path: &str, source: &str, params: &Params) {
    let path = Path::new(database_path);
    let mut database = noesql_runtime::zig_pager::load_or_default(path).unwrap_or_else(|error| fail(error));
    // Reads must not add WAL transactions or rewrite snapshots. This also
    // avoids consuming the finite WAL budget merely by querying the DB.
    // Compare the canonical payload, not just the affected-row count: DDL
    // can change schema without affecting any rows.
    let original = noesql_runtime::codec::encode(&database).unwrap_or_else(|error| fail(error));
    let report = database.execute(source, params).unwrap_or_else(|error| fail(error));
    let updated = noesql_runtime::codec::encode(&database).unwrap_or_else(|error| fail(error));
    if updated != original {
        noesql_runtime::zig_pager::save(path, &database).unwrap_or_else(|error| fail(error));
    }
    if !report.rows.is_empty() {
        println!("{}", rows_json(&report.rows));
    } else {
        println!("affected={}", report.affected);
    }
}

#[cfg(not(feature = "zig-pager"))]
fn run_script(_: &str, _: &str, _: &Params) {
    fail("this binary was built without the zig-pager feature");
}

fn parse_binding(raw: &str) -> (String, Value) {
    let mut parts = raw.splitn(3, ':');
    let name = parts.next().unwrap_or("");
    let kind = parts.next().unwrap_or("");
    let raw_value = parts.next().unwrap_or("");
    if name.is_empty() || name.len() > 64 || !name.as_bytes()[0].is_ascii_alphabetic() && name.as_bytes()[0] != b'_' ||
       !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') || raw_value.len() > 4096 {
        fail("invalid NoeSQL bind parameter");
    }
    let value = match kind {
        "text" => Value::Text(raw_value.to_owned()),
        "int" => Value::Int(raw_value.parse::<i64>().unwrap_or_else(|_| fail("invalid int bind value"))),
        "bool" => match raw_value { "true" => Value::Bool(true), "false" => Value::Bool(false),
            _ => fail("invalid bool bind value") },
        "null" if raw_value.is_empty() => Value::Null,
        _ => fail("unsupported bind type (use text, int, bool or null)"),
    };
    (name.to_owned(), value)
}
fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("version") => println!("NoeSQL native runtime 0.8.0"),
        Some("check") => {
            let path = args.next().unwrap_or_else(|| usage());
            let source = read_source(&path).unwrap_or_else(|error| fail(error));
            let statements = parse(&source).unwrap_or_else(|error| fail(error));
            println!("ok: {} statement(s), {} bytes", statements.len(), source.len());
        }
        Some("check-db") => {
            let path = args.next().unwrap_or_else(|| usage());
            validate_database_path(Path::new(&path)).unwrap_or_else(|error| fail(error));
            println!("ok: canonical .noedb path");
        }
        Some("run") => {
            let database = args.next().unwrap_or_else(|| usage());
            let script = args.next().unwrap_or_else(|| usage());
            let source = read_source(script).unwrap_or_else(|error| fail(error));
            run_script(&database, &source, &Params::new());
        }
        Some("query") => {
            let database = args.next().unwrap_or_else(|| usage());
            let query: Vec<String> = args.collect();
            if query.is_empty() { usage(); }
            run_script(&database, &query.join(" "), &Params::new());
        }
        Some("exec") => {
            let database = args.next().unwrap_or_else(|| usage());
            let source = args.next().unwrap_or_else(|| usage());
            if source.len() > 65536 { fail("NoeSQL source exceeds web bridge limit"); }
            let mut params = Params::new();
            for raw in args {
                if params.len() >= 32 { fail("too many NoeSQL parameters"); }
                let (name,value) = parse_binding(&raw);
                if params.insert(name,value).is_some() { fail("duplicate NoeSQL bind parameter"); }
            }
            run_script(&database, &source, &params);
        }
        _ => usage(),
    }
}

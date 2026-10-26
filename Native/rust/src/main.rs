use noesql_runtime::reference::{parse, rows_json, Params};
use noesql_runtime::{read_source, validate_database_path};
use std::path::Path;

fn usage() -> ! {
    eprintln!(
        "NoeSQL native runtime\n\nusage:\n  noesql version\n  noesql check <file.noe>\n  noesql check-db <file.noedb>\n  noesql run <file.noedb> <script.noe>\n  noesql query <file.noedb> <query>"
    );
    std::process::exit(2);
}

fn fail(error: impl std::fmt::Display) -> ! {
    eprintln!("error: {error}");
    std::process::exit(1);
}

#[cfg(feature = "zig-pager")]
fn run_script(database_path: &str, source: &str) {
    let path = Path::new(database_path);
    // The Zig pager handle must remain open while the entire statement
    // executes and commits, otherwise multiple CLI writers can lose updates.
    let report = noesql_runtime::zig_pager::execute_locked(path, source, &Params::new())
        .unwrap_or_else(|error| fail(error));
    if !report.rows.is_empty() {
        println!("{}", rows_json(&report.rows));
    } else {
        println!("affected={}", report.affected);
    }
}

#[cfg(not(feature = "zig-pager"))]
fn run_script(_: &str, _: &str) {
    fail("this binary was built without the zig-pager feature");
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
            run_script(&database, &source);
        }
        Some("query") => {
            let database = args.next().unwrap_or_else(|| usage());
            let query: Vec<String> = args.collect();
            if query.is_empty() { usage(); }
            run_script(&database, &query.join(" "));
        }
        _ => usage(),
    }
}

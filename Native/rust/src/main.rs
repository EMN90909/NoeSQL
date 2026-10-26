use noesql_runtime::reference::{parse, rows_json, Params};
use noesql_runtime::{read_source, validate_database_path};
use std::path::Path;

fn usage() -> ! {
    eprintln!(
        "NoeSQL native runtime\n\nusage:\n  noesql version\n  noesql check <file.noe>\n  noesql check-db <file.noedb>\n  noesql run <file.noedb> <script.noe>\n  noesql query <file.noedb> <query>\n  noesql v2-run <legacy.noedb> <script.noe>\n  noesql v2-query <legacy.noedb> <query>\n  noesql explain <file.noedb> <query>\n  noesql schema-check <file.noedb> <migration.noe>\n  noesql export-sql <file.noedb> <sqlite|postgres> <new-file.sql>\n  noesql export-sqlite <source.noedb> <new-file.sqlite3>\n  noesql migrate-v3 <existing-v2.noedb> <new-v3.noedb>\n  noesql v3-create <new-v3.noedb> <schema.noe>\n  noesql v3-discard-unpublished <uncommitted-v3.noedb>\n  noesql v3-check <file.noedb>\n  noesql v3-query <file.noedb> <query>\n  noesql v3-run <file.noedb> <script.noe>\n  noesql v3-exec <file.noedb> <NoeSQL-source>\n  noesql backup <source.noedb> <new-backup.noedb>\n  noesql restore <verified-backup.noedb> <new-database.noedb>"
    );
    std::process::exit(2);
}

fn fail(error: impl std::fmt::Display) -> ! {
    eprintln!("error: {error}");
    std::process::exit(1);
}

fn default_path(raw: &str) -> std::path::PathBuf {
    let path = Path::new(raw);
    validate_database_path(path).unwrap_or_else(|error| fail(error));
    if path.is_absolute() { path.to_path_buf() } else {
        std::env::current_dir().unwrap_or_else(|error| fail(error)).join(path)
    }
}

fn display_report(report: noesql_runtime::reference::ExecutionReport) {
    if !report.rows.is_empty() {
        println!("{}", rows_json(&report.rows));
    } else {
        println!("affected={}", report.affected);
    }
}

/// NQDB0003 is the default for new databases; existing v2 files are never
/// changed implicitly. The Rust v3 store intentionally requires an owned,
/// private directory and an absolute filesystem path.
fn run_script(database_path: &str, source: &str) {
    let path = default_path(database_path);
    let report = noesql_runtime::v3_store::execute_or_create(
        &path, source, &Params::new()
    ).unwrap_or_else(|error| fail(error));
    display_report(report);
}

#[cfg(feature = "zig-pager")]
fn run_v2(database_path: &str, source: &str) {
    let path = Path::new(database_path);
    display_report(noesql_runtime::zig_pager::execute_locked(path, source, &Params::new())
        .unwrap_or_else(|error| fail(error)));
}
#[cfg(not(feature = "zig-pager"))]
fn run_v2(_: &str, _: &str) {
    fail("legacy v2 execution requires the zig-pager feature");
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
        Some(operation @ ("run" | "v2-run")) => {
            let database = args.next().unwrap_or_else(|| usage());
            let script = args.next().unwrap_or_else(|| usage());
            let source = read_source(script).unwrap_or_else(|error| fail(error));
            if operation == "v2-run" { run_v2(&database, &source); }
            else { run_script(&database, &source); }
        }
        Some(operation @ ("query" | "v2-query")) => {
            let database = args.next().unwrap_or_else(|| usage());
            let query: Vec<String> = args.collect();
            if query.is_empty() { usage(); }
            if operation == "v2-query" { run_v2(&database, &query.join(" ")); }
            else { run_script(&database, &query.join(" ")); }
        }
        Some("explain") => {
            let database = args.next().unwrap_or_else(|| usage());
            let query: Vec<String> = args.collect();
            if query.is_empty() { usage(); }
            let snapshot = noesql_runtime::v3_store::snapshot(&default_path(&database))
                .unwrap_or_else(|error| fail(error));
            println!("{}", snapshot.database().explain(&query.join(" "), &Params::new())
                .unwrap_or_else(|error| fail(error)));
        }
        Some("export-sql") => {
            let database = args.next().unwrap_or_else(|| usage());
            let flavor = args.next().unwrap_or_else(|| usage());
            let destination = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            #[cfg(feature = "zig-pager")]
            {
                use noesql_runtime::interop::{export_sql, Dialect};
                use std::io::Write;
                let dialect = match flavor.as_str() {
                    "sqlite" => Dialect::Sqlite,
                    "postgres" => Dialect::Postgres,
                    _ => usage(),
                };
                let db = noesql_runtime::zig_pager::load(Path::new(&database))
                    .unwrap_or_else(|error| fail(error));
                let sql = export_sql(&db, dialect).unwrap_or_else(|error| fail(error));
                let mut output = std::fs::OpenOptions::new()
                    .write(true).create_new(true).open(&destination)
                    .unwrap_or_else(|error| fail(error));
                if let Err(error) = output.write_all(sql.as_bytes()).and_then(|_| output.sync_all()) {
                    let _ = std::fs::remove_file(&destination);
                    fail(error);
                }
                println!("SQL export written: {destination}");
            }
            #[cfg(not(feature = "zig-pager"))]
            fail("SQL export requires the zig-pager feature");
        }
        Some("export-sqlite") => {
            let source = args.next().unwrap_or_else(|| usage());
            let target = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            #[cfg(all(feature = "zig-pager", feature = "sqlite-adapter"))]
            {
                let db = noesql_runtime::zig_pager::load(Path::new(&source))
                    .unwrap_or_else(|error| fail(error));
                noesql_runtime::sqlite_adapter::export_into_sqlite(&db, Path::new(&target))
                    .unwrap_or_else(|error| fail(error));
                println!("verified SQLite import: {target}");
            }
            #[cfg(not(all(feature = "zig-pager", feature = "sqlite-adapter")))]
            fail("export-sqlite requires zig-pager and sqlite-adapter features");
        }
        Some("migrate-v3") => {
            let source = args.next().unwrap_or_else(|| usage());
            let target = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            #[cfg(feature = "zig-pager")]
            {
                noesql_runtime::v3_store::migrate_copy_v2(Path::new(&source), Path::new(&target))
                    .unwrap_or_else(|error| fail(error));
                println!("NQDB0003 migration verified: {source} -> {target} (v2 source preserved)");
            }
            #[cfg(not(feature = "zig-pager"))]
            fail("migrate-v3 requires zig-pager feature for source format");
        }
        Some("v3-discard-unpublished") => {
            let target = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            noesql_runtime::v3_store::discard_unpublished(Path::new(&target))
                .unwrap_or_else(|error| fail(error));
            println!("discarded unpublished v3 generation files: {target}");
        }
        Some("v3-create") => {
            let target = args.next().unwrap_or_else(|| usage());
            let schema = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            let source = read_source(&schema).unwrap_or_else(|e| fail(e));
            let mut db = noesql_runtime::reference::Database::default();
            db.execute(&source, &Params::new()).unwrap_or_else(|e| fail(e));
            noesql_runtime::v3_store::create_new(Path::new(&target), &db)
                .unwrap_or_else(|e| fail(e));
            println!("NQDB0003 created: {target}");
        }
        Some("v3-exec") => {
            let path = args.next().unwrap_or_else(|| usage());
            let text: Vec<_> = args.collect();
            if text.is_empty() { usage(); }
            let report = noesql_runtime::v3_store::execute(
                Path::new(&path), &text.join(" "), &Params::new()
            ).unwrap_or_else(|e| fail(e));
            if !report.rows.is_empty() { println!("{}", rows_json(&report.rows)); }
            else { println!("affected={}", report.affected); }
        }
        Some("v3-check") => {
            let source = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            let (generation, size, pages) =
                noesql_runtime::v3_store::inspect(Path::new(&source))
                    .unwrap_or_else(|error| fail(error));
            println!("NQDB0003 verified: generation={generation} bytes={size} pages={pages}");
        }
        Some("v3-query") => {
            let database = args.next().unwrap_or_else(|| usage());
            let query: Vec<_> = args.collect();
            if query.is_empty() { usage(); }
            let report = noesql_runtime::v3_store::snapshot(Path::new(&database))
                .unwrap_or_else(|error| fail(error))
                .query(&query.join(" "), &Params::new())
                .unwrap_or_else(|error| fail(error));
            if !report.rows.is_empty() { println!("{}", rows_json(&report.rows)); }
            else { println!("affected={}", report.affected); }
        }
        Some("v3-run") => {
            let database = args.next().unwrap_or_else(|| usage());
            let script = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            let source = read_source(&script).unwrap_or_else(|error| fail(error));
            let report = noesql_runtime::v3_store::execute(Path::new(&database), &source, &Params::new())
                .unwrap_or_else(|error| fail(error));
            if !report.rows.is_empty() { println!("{}", rows_json(&report.rows)); }
            else { println!("affected={}", report.affected); }
        }
        Some("schema-check") => {
            let database = args.next().unwrap_or_else(|| usage());
            let script = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            let source = read_source(&script).unwrap_or_else(|error| fail(error));
            #[cfg(feature = "zig-pager")]
            {
                let mut simulated = noesql_runtime::zig_pager::load(Path::new(&database))
                    .unwrap_or_else(|error| fail(error));
                let original_version = simulated.version;
                let report = simulated.execute(&source, &Params::new())
                    .unwrap_or_else(|error| fail(error));
                println!(
                    "schema-check valid: version {} -> {}; affected={} (not applied)",
                    original_version, simulated.version, report.affected
                );
            }
            #[cfg(not(feature = "zig-pager"))]
            fail("schema-check requires the zig-pager feature");
        }
        Some(operation @ ("backup" | "restore")) => {
            let from = args.next().unwrap_or_else(|| usage());
            let to = args.next().unwrap_or_else(|| usage());
            if args.next().is_some() { usage(); }
            #[cfg(feature = "zig-pager")]
            {
                noesql_runtime::zig_pager::backup(Path::new(&from), Path::new(&to))
                    .unwrap_or_else(|error| fail(error));
                println!("{}: {to}", if operation == "restore" { "verified database restored" } else { "verified backup created" });
            }
            #[cfg(not(feature = "zig-pager"))]
            fail("backup and restore require the zig-pager feature");
        }
        _ => usage(),
    }
}

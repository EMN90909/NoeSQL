// Real process-level contention test: every writer uses the compiled CLI,
// a separate Zig pager handle, WAL persistence, and a cold reopened database.
#![cfg(feature = "zig-pager")]

use noesql_runtime::reference::Params;
use noesql_runtime::zig_pager;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_path() -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!(
        "noesql-cli-contention-{}-{nonce}.noedb",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    for target in [
        path.to_path_buf(),
        PathBuf::from(format!("{}.wal", path.display())),
        PathBuf::from(format!("{}.lck", path.display())),
        PathBuf::from(format!("{}.checkpoint.tmp", path.display())),
    ] {
        let _ = fs::remove_file(target);
    }
}

fn call(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_noesql"))
        .args(args)
        .output()
        .expect("cannot launch NoeSQL CLI");
    assert!(
        output.status.success(),
        "NoeSQL {:?} failed (status {:?}): {}",
        args,
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("CLI output must be UTF-8")
}

#[test]
fn multiple_independent_writers_do_not_lose_committed_rows() {
    const WRITERS: usize = 8;
    const RECORDS_PER_WRITER: usize = 12;
    const EXPECTED: usize = WRITERS * RECORDS_PER_WRITER;

    let path = fixture_path();
    let schema = path.with_extension("schema.noe");
    cleanup(&path);
    fs::write(
        &schema,
        "table events {\n id:int key auto\n token:text required unique\n}",
    ).unwrap();
    let path_str = path.to_str().unwrap().to_owned();
    let schema_str = schema.to_str().unwrap().to_owned();
    call(&["v2-run", &path_str, &schema_str]);

    let start = Arc::new(Barrier::new(WRITERS));
    let mut workers = Vec::new();
    for writer in 0..WRITERS {
        let start = Arc::clone(&start);
        let path_str = path_str.clone();
        workers.push(thread::spawn(move || {
            start.wait();
            for sequence in 0..RECORDS_PER_WRITER {
                let command = format!(
                    "events add {{token:\"writer-{writer}-entry-{sequence}\"}}"
                );
                let output = call(&["v2-query", &path_str, &command]);
                assert!(
                    output.contains("affected=1"),
                    "successful command must insert exactly one row: {output}"
                );
            }
        }));
    }
    for worker in workers {
        worker.join().expect("concurrent writer panicked");
    }

    // Opening the data after ALL writing handles are closed must replay
    // a durable WAL, not read process-local cache state.
    let output = call(&["v2-query", &path_str, "events get id,token sort id"]);
    assert_eq!(
        output.matches("\"token\":").count(),
        EXPECTED,
        "lost update or duplicate transaction detected: {output}"
    );
    for writer in 0..WRITERS {
        for sequence in 0..RECORDS_PER_WRITER {
            let expected = format!("\"writer-{writer}-entry-{sequence}\"");
            assert!(output.contains(&expected), "missing durable record {expected}");
        }
    }

    zig_pager::checkpoint(&path).expect("checkpoint under Zig lock must succeed");
    let mut recovered = zig_pager::load(&path).expect("checkpoint must cold-reopen");
    let rows = recovered.execute("events get id,token sort id", &Params::new())
        .expect("recovered query failed")
        .rows;
    assert_eq!(rows.len(), EXPECTED);

    cleanup(&path);
    let _ = fs::remove_file(schema);
}

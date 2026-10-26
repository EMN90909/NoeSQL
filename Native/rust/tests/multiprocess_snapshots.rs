//! OS-process reader/writer stress for WAL-recovered snapshot acquisitions.
//! Demonstrates no partially visible writes or lost committed rows; does NOT
//! establish full on-disk MVCC or an optimizer performance guarantee.
#![cfg(feature = "zig-pager")]

use noesql_runtime::reference::Params;
use noesql_runtime::zig_pager;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture() -> PathBuf {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("noesql-multiprocess-snapshots-{}-{n}.noedb", std::process::id()))
}
fn cleanup(path: &Path) {
    for ext in ["", ".wal", ".lck", ".checkpoint.tmp"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), ext));
    }
}
fn command(path: &Path, query: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_noesql"))
        .args(["v2-query", path.to_str().unwrap(), query])
        .output().expect("cannot run native NoeSQL CLI");
    assert!(out.status.success(), "NoeSQL process error: {}",
        String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}
#[test]
fn concurrent_process_readers_observe_complete_snapshots_while_writers_commit() {
    const WRITERS: usize = 4;
    const PER_WRITER: usize = 6;
    const READERS: usize = 5;
    const READS: usize = 12;
    let path = fixture();
    cleanup(&path);
    command(&path, "table records { id:int key auto\n token:text required unique }");
    let barrier = Arc::new(Barrier::new(WRITERS + READERS));
    let mut workers = Vec::new();

    for writer in 0..WRITERS {
        let start = barrier.clone();
        let path = path.clone();
        workers.push(thread::spawn(move || {
            start.wait();
            for n in 0..PER_WRITER {
                let cmd = format!("records add {{token:\"writer-{writer}-row-{n}\"}}");
                assert_eq!(command(&path, &cmd).trim(), "affected=1");
            }
        }));
    }
    for _ in 0..READERS {
        let start = barrier.clone();
        let path = path.clone();
        workers.push(thread::spawn(move || {
            start.wait();
            for _ in 0..READS {
                let output = command(&path, "records get id,token sort id");
                let count = output.matches("\"id\":").count();
                assert!(count <= WRITERS * PER_WRITER);
                // Every visible row has a complete ID and token; a partially
                // persisted WAL frame must never appear in a reader snapshot.
                assert_eq!(count, output.matches("\"token\":").count());
                // A single query's IDs are unique and monotonically ordered.
                let mut previous = 0i64;
                for field in output.split("\"id\":").skip(1) {
                    let digits: String = field.chars().take_while(|c| c.is_ascii_digit()).collect();
                    let id = digits.parse::<i64>().expect("truncated row id");
                    assert!(id > previous, "snapshot saw duplicate or out-of-order keys");
                    previous = id;
                }
            }
        }));
    }
    for worker in workers { worker.join().expect("child process stress failed"); }
    zig_pager::checkpoint(&path).unwrap();
    let mut recovered = zig_pager::load(&path).unwrap();
    let rows = recovered.execute("records get id,token sort id", &Params::new()).unwrap().rows;
    assert_eq!(rows.len(), WRITERS * PER_WRITER);
    cleanup(&path);
}

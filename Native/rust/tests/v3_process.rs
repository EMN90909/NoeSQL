//! Independent Linux CLI processes exercise NQDB0003 generations.
//! These are executable stress regressions, not proof of production
//! linearizability or crash-safe behavior across real power loss.
#![cfg(all(feature = "zig-pager", target_os = "linux"))]
use noesql_runtime::{codec, reference::Params, v3_store, zig_pager};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp(label: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("noesql-v3-{label}-{}-{nanos}.noedb", std::process::id()))
}
fn cleanup(path: &Path) {
    for suffix in ["", ".lck", ".wal", ".checkpoint.tmp"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
    let _ = fs::remove_dir_all(format!("{}.versions", path.display()));
}
fn cli(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_noesql")).args(args).output()
        .expect("native NoeSQL CLI process must start");
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn opt_in_v3_process_readers_and_writers_never_lose_committed_rows() {
    const WRITERS: usize = 4;
    const PER_WRITER: usize = 8;
    const READERS: usize = 4;
    let path = temp("multi");
    let schema = path.with_extension("schema.noeqr");
    cleanup(&path);
    fs::write(&schema, "table items { id:int key auto\n value:text required unique }").unwrap();
    cli(&["v3-create", path.to_str().unwrap(), schema.to_str().unwrap()]);
    let gate = Arc::new(Barrier::new(WRITERS + READERS));
    let mut tasks = vec![];
    for writer in 0..WRITERS {
        let path = path.clone();
        let gate = gate.clone();
        tasks.push(thread::spawn(move || {
            gate.wait();
            for n in 0..PER_WRITER {
                let command = format!("items add {{value:\"writer-{writer}-{n}\"}}");
                let result = cli(&["query", path.to_str().unwrap(), &command]);
                assert!(result.contains("affected=1"), "{result}");
            }
        }));
    }
    for _ in 0..READERS {
        let path = path.clone();
        let gate = gate.clone();
        tasks.push(thread::spawn(move || {
            gate.wait();
            for _ in 0..12 {
                let result = cli(&["query", path.to_str().unwrap(), "items get id,value sort id"]);
                let ids = result.matches("\"id\":").count();
                assert!(ids <= WRITERS * PER_WRITER);
                assert_eq!(ids, result.matches("\"value\":").count());
            }
        }));
    }
    for task in tasks { task.join().expect("reader/writer process failed"); }
    let (generation, _, _) = v3_store::inspect(&path).unwrap();
    assert_eq!(generation, 1 + (WRITERS * PER_WRITER) as u64);
    let data = v3_store::snapshot(&path).unwrap().query("items get id,value sort id", &Params::new()).unwrap().rows;
    assert_eq!(data.len(), WRITERS * PER_WRITER);
    for writer in 0..WRITERS {
        for n in 0..PER_WRITER {
            let name = format!("writer-{writer}-{n}");
            assert!(data.iter().any(|row| row.get("value") == Some(&noesql_runtime::value::Value::Text(name.clone()))));
        }
    }
    cleanup(&path);
    let _ = fs::remove_file(schema);
}
#[test]
fn copy_migration_preserves_v2_source_and_matches_v3_snapshot() {
    let v2 = temp("original-v2");
    let v3 = temp("migrated");
    cleanup(&v2);
    cleanup(&v3);
    let script = "table clients { id:int key auto\n name:text required unique }\nclients add {name:\"Amina\"}\nclients add {name:\"Brian\"}";
    zig_pager::execute_locked(&v2, script, &Params::new()).unwrap();
    let source = zig_pager::load(&v2).unwrap();
    cli(&["migrate-v3", v2.to_str().unwrap(), v3.to_str().unwrap()]);
    assert_eq!(codec::encode(&source).unwrap(),
        codec::encode(&zig_pager::load(&v2).unwrap()).unwrap(),
        "migration changed the legacy v2 source");
    assert_eq!(v3_store::snapshot(&v3).unwrap()
        .query("clients get id,name sort id", &Params::new()).unwrap().rows.len(), 2);
    // If the first migration committed but its response was lost, an
    // identical destination verifies successfully without overwriting it.
    cli(&["migrate-v3", v2.to_str().unwrap(), v3.to_str().unwrap()]);
    v3_store::execute(&v3, "clients add {name:\"Cara\"}", &Params::new()).unwrap();
    // Existing, divergent v3 data must never be overwritten on retry.
    assert!(!Command::new(env!("CARGO_BIN_EXE_noesql"))
        .args(["migrate-v3", v2.to_str().unwrap(), v3.to_str().unwrap()])
        .output().unwrap().status.success());
    assert_eq!(zig_pager::load(&v2).unwrap().tables["clients"].rows.len(), 2);
    assert_eq!(v3_store::snapshot(&v3).unwrap()
        .query("clients get id,name", &Params::new()).unwrap().rows.len(), 3);
    cleanup(&v2);
    cleanup(&v3);
}

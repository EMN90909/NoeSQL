#![cfg(feature = "zig-pager")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn path(label: &str) -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("noesql-backup-{label}-{}-{nonce}.noedb", std::process::id()))
}

fn clean(file: &Path) {
    for p in [
        file.to_path_buf(),
        PathBuf::from(format!("{}.wal", file.display())),
        PathBuf::from(format!("{}.lck", file.display())),
        PathBuf::from(format!("{}.checkpoint.tmp", file.display())),
    ] {
        let _ = fs::remove_file(p);
    }
}

fn cli(args: &[&str], should_succeed: bool) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_noesql")).args(args).output().unwrap();
    assert_eq!(
        output.status.success(), should_succeed,
        "command {:?} failed: {}", args, String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn verified_backup_is_standalone_is_consistent_and_never_overwrites() {
    let source = path("source");
    let backup = path("backup");
    let restored = path("restored");
    let script = source.with_extension("noe");
    for file in [&source, &backup, &restored] { clean(file); }
    fs::write(&script, "table users {\n id:int key auto\n name:text required unique\n}\nusers add {name:\"Amina\"}").unwrap();

    let source_name = source.to_str().unwrap();
    let backup_name = backup.to_str().unwrap();
    let restored_name = restored.to_str().unwrap();
    cli(&["run", source_name, script.to_str().unwrap()], true);
    cli(&["backup", source_name, backup_name], true);
    assert!(backup.exists());
    // A portable backup is checkpointed: reading the standalone .noedb
    // must not need a copied live WAL file.
    let journal = PathBuf::from(format!("{}.wal", backup.display()));
    assert!(!journal.exists(), "backups must not retain a live WAL");
    assert!(cli(&["query", backup_name, "users get name"], true).contains("Amina"));

    cli(&["query", source_name, "users add {name:\"Brian\"}"], true);
    let snapshot = cli(&["query", backup_name, "users get name sort name"], true);
    assert!(snapshot.contains("Amina"));
    assert!(!snapshot.contains("Brian"), "backup changed after later live write");

    cli(&["backup", source_name, backup_name], false);
    assert_eq!(snapshot, cli(&["query", backup_name, "users get name sort name"], true));
    cli(&["restore", backup_name, restored_name], true);
    assert!(cli(&["query", restored_name, "users get name"], true).contains("Amina"));
    assert!(!cli(&["query", restored_name, "users get name"], true).contains("Brian"));

    for file in [&source, &backup, &restored] { clean(file); }
    let _ = fs::remove_file(script);
}

#[test]
fn explain_reports_real_plan_and_rejects_mutations() {
    let source = path("explain");
    let script = source.with_extension("noe");
    clean(&source);
    fs::write(&script, "table items {\n id:int key auto\n value:int required\n}\nitems add {value:1}").unwrap();
    let db = source.to_str().unwrap();
    cli(&["run", db, script.to_str().unwrap()], true);

    let plan = cli(&["explain", db, "items get id,value where value >= 1 sort id"], true);
    assert!(plan.contains("access=full_scan"), "{plan}");
    assert!(plan.contains("join=none"), "{plan}");
    assert!(plan.contains("sort=id"), "{plan}");
    cli(&["explain", db, "items add {value:2}"], false);
    assert_eq!(
        cli(&["query", db, "items get id"], true).matches("\"id\":").count(), 1,
        "EXPLAIN must never mutate tables"
    );

    clean(&source);
    let _ = fs::remove_file(script);
}

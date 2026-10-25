// The CLI must not commit a new WAL transaction for a pure SELECT-like read.
// This test executes the actual binary rather than duplicating its logic.
#![cfg(feature = "zig-pager")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_path() -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!(
        "noesql-cli-no-read-write-{}-{nonce}.noedb", std::process::id()
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

fn command(parts: &[&str]) -> std::process::Output {
    let output = Command::new(env!("CARGO_BIN_EXE_noesql")).args(parts).output()
        .expect("failed to launch the compiled NoeSQL CLI");
    assert!(
        output.status.success(),
        "NoeSQL CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn repeated_reads_do_not_increase_wal_or_change_persisted_data() {
    let path = fixture_path();
    let schema = path.with_extension("schema.noe");
    cleanup(&path);
    fs::write(
        &schema,
        r#"table users {
 id:int key auto
 name:text required unique
}
users add {name:"Amina"}"#,
    ).unwrap();
    let db = path.to_str().expect("UTF-8 temp directory");
    let script = schema.to_str().expect("UTF-8 schema file");

    command(&["run", db, script]);
    let wal_path = PathBuf::from(format!("{}.wal", path.display()));
    let before = fs::metadata(&wal_path).unwrap().len();
    assert!(before > 0, "creating a row must write the WAL");

    for _ in 0..32 {
        let output = command(&["query", db, "users get id,name sort id"]);
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Amina"), "read returned the wrong row: {text}");
    }

    let after = fs::metadata(&wal_path).unwrap().len();
    assert_eq!(
        before, after,
        "32 read-only queries must not consume WAL space or mutate persistent data"
    );
    cleanup(&path);
    let _ = fs::remove_file(schema);
}

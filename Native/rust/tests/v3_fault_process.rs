//! Real Linux process-level v3 interruption and size-limit regressions.
//!
//! These exercise independently launched NoeSQL CLI processes. Abrupt process
//! termination and RLIMIT_FSIZE are NOT power-cut testing or genuine ENOSPC,
//! and do not qualify the database for production by themselves.
#![cfg(target_os = "linux")]

use noesql_runtime::reference::{Database, Params};
use noesql_runtime::v3_store;
use noesql_runtime::value::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::os::unix::fs::PermissionsExt;

fn private_database(label: &str) -> (PathBuf, PathBuf) {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let home = std::env::temp_dir().join(format!(
        "noesql-v3-fault-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    let path = home.join("main.noedb");
    let mut initial = Database::default();
    initial.execute(
        "table archive { id:int key auto\n body:text required }\n\
         table entries { id:int key auto\n owner:int required }\n\
         entries add {owner:1}",
        &Params::new(),
    ).unwrap();
    // A multi-page immutable relational snapshot makes interruption possible
    // during actual I/O, not just immediately after process startup.
    let mut row = BTreeMap::new();
    row.insert("id".into(), Value::Int(1));
    let mut seed = 0xcafe1234u32;
    let mut text = String::with_capacity(4096 * 24);
    for _ in 0..4096 * 24 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        text.push((b'a' + (seed % 26) as u8) as char);
    }
    row.insert("body".into(), Value::Text(text));
    let archive = initial.tables.get_mut("archive").unwrap();
    archive.rows.push(row);
    archive.next_auto = 2;
    v3_store::create_new(&path, &initial).unwrap();
    (home, path)
}
fn cleanup(home: &Path) { fs::remove_dir_all(home).unwrap(); }
fn rows(path: &Path) -> usize {
    v3_store::snapshot(path).unwrap()
        .query("entries get id,owner", &Params::new()).unwrap().rows.len()
}
#[test]
fn killed_writer_never_exposes_a_partial_or_corrupt_generation() {
    let (home, path) = private_database("kill");
    let binary = env!("CARGO_BIN_EXE_noesql");
    for attempt in 0..8usize {
        let before = v3_store::inspect(&path).unwrap().0;
        let count = rows(&path);
        let command = format!("entries add {{owner:{}}}", 1000 + attempt);
        let mut child = Command::new(binary).args([
            "v3-exec", path.to_str().unwrap(), &command,
        ]).spawn().unwrap();
        thread::sleep(Duration::from_millis(3));
        // SIGKILL may interrupt any phase. A fast commit before the kill is
        // also legitimate. Every reopened result must be exactly old or new.
        let _ = child.kill();
        let _ = child.wait().unwrap();
        let after = v3_store::inspect(&path).unwrap().0;
        let observed = rows(&path);
        assert!(after == before || after == before + 1,
            "writer exposed unexpected generation transition");
        assert_eq!(observed, count + (after - before) as usize,
            "writer exposed partial relational changes");
        assert_eq!(v3_store::snapshot(&path).unwrap()
            .query("archive get id", &Params::new()).unwrap().rows.len(), 1);
        // Follow each uncertain write with a fresh independent successful
        // commit. Residual orphans may exist but cannot block live progress.
        v3_store::execute(
            &path, &format!("entries add {{owner:{}}}", 2000 + attempt),
            &Params::new(),
        ).unwrap();
        assert_eq!(rows(&path), observed + 1);
    }
    cleanup(&home);
}

#[test]
fn file_size_exhaustion_in_child_does_not_change_committed_head() {
    let (home, path) = private_database("limit");
    let before = v3_store::inspect(&path).unwrap().0;
    let initial_rows = rows(&path);

    // POSIX RLIMIT_FSIZE limits any *single* newly written file, unlike
    // ENOSPC (exhausting the entire filesystem). The child cannot create a
    // complete immutable 4096-byte data blob under this 1024-byte limit.
    // No root privileges, mounts or shared production paths are involved.
    let result = Command::new("/bin/sh").args([
        "-c", "ulimit -f 1; exec \"$@\"",
        "noesql-limit",
        env!("CARGO_BIN_EXE_noesql"),
        "v3-exec",
        path.to_str().unwrap(),
        "entries add {owner:2222}",
    ]).output().unwrap();
    assert!(!result.status.success(),
        "size-constrained child unexpectedly committed transaction");
    assert_eq!(v3_store::inspect(&path).unwrap().0, before);
    assert_eq!(rows(&path), initial_rows);
    v3_store::execute(&path, "entries add {owner:3333}", &Params::new()).unwrap();
    assert_eq!(rows(&path), initial_rows + 1);
    cleanup(&home);
}

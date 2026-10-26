//! Default engine contract: new files are NQDB0003. Legacy files are never
//! migrated or overwritten by run/query; old v2 operations stay explicit.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(label: &str) -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("noesql-default-{label}-{}-{nonce}.noedb", std::process::id()))
}
fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(format!("{}.lck", path.display()));
    let _ = fs::remove_dir_all(format!("{}.versions", path.display()));
}
fn command(args: &[&str], ok: bool) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_noesql")).args(args)
        .output().expect("NoeSQL executable missing");
    assert_eq!(result.status.success(), ok, "args={args:?}, stderr={}",
        String::from_utf8_lossy(&result.stderr));
    String::from_utf8_lossy(&result.stdout).into_owned()
}
#[test]
fn normal_commands_create_v3_and_keep_v2_explicit() {
    let db = fixture("v3");
    let backup = fixture("backup");
    let old = fixture("v2");
    cleanup(&db); cleanup(&backup); cleanup(&old);
    let file = db.with_extension("noe");
    fs::write(&file, "table things { id:int key auto\n value:int required }\nthings add {value:4}").unwrap();
    command(&["run", db.to_str().unwrap(), file.to_str().unwrap()], true);
    assert_eq!(&fs::read(&db).unwrap()[..8], b"NQDB0003");
    command(&["query", db.to_str().unwrap(), "things add {value:8}"], true);
    let result = command(&["query", db.to_str().unwrap(), "things get id,value sort id"], true);
    assert!(result.contains("\"value\":4") && result.contains("\"value\":8"));
    let explain = command(&["explain", db.to_str().unwrap(), "things get id where id == 2"], true);
    assert!(explain.contains("access=memory_bplus(id)"), "{explain}");
    command(&["backup", db.to_str().unwrap(), backup.to_str().unwrap()], true);
    assert_eq!(&fs::read(&backup).unwrap()[..8], b"NQDB0003");
    let restored = command(&["query", backup.to_str().unwrap(), "things get id,value sort id"], true);
    assert_eq!(result, restored);
    command(&["backup", db.to_str().unwrap(), backup.to_str().unwrap()], false);
    command(&["v2-query", db.to_str().unwrap(), "things get id"], false);
    fs::write(&old, b"NQDB0002").unwrap();
    command(&["query", old.to_str().unwrap(), "things add {value:7}"], false);
    assert_eq!(fs::read(&old).unwrap(), b"NQDB0002");
    cleanup(&db); cleanup(&backup); cleanup(&old);
    let _ = fs::remove_file(file);
}

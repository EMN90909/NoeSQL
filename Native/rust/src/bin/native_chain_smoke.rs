#[cfg(feature = "noqeri-native")]
fn main() {
    use noesql_runtime::noqeri_native;
    use std::path::PathBuf;

    if noqeri_native::api_version() != 2 {
        eprintln!("unexpected native NoeSQL API version");
        std::process::exit(1);
    }

    let mut path = std::env::temp_dir();
    path.push(format!("noesql-rust-noqeri-zig-{}.noedb", std::process::id()));
    cleanup(&path);

    noqeri_native::insert_bool_row(&path, 3, 44, true).unwrap();
    assert_eq!(noqeri_native::get_bool_row(&path, 3, 44).unwrap(), Some(true));
    noqeri_native::set_bool_row(&path, 3, 44, false).unwrap();
    assert_eq!(noqeri_native::get_bool_row(&path, 3, 44).unwrap(), Some(false));
    noqeri_native::delete_bool_row(&path, 3, 44).unwrap();
    assert_eq!(noqeri_native::get_bool_row(&path, 3, 44).unwrap(), None);

    cleanup(&path);
    println!("Rust -> Noqeri AOT -> Zig pager smoke: OK");
}

#[cfg(feature = "noqeri-native")]
fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}.wal", path.display()));
    let _ = std::fs::remove_file(format!("{}.lck", path.display()));
}

#[cfg(not(feature = "noqeri-native"))]
fn main() {
    eprintln!("native_chain_smoke requires --features noqeri-native");
    std::process::exit(2);
}

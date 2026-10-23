#[cfg(feature = "stage2-nqri")]
fn main() {
    use noesql_runtime::stage2_nqri::{self, Stage2Runtime};

    let vm = std::env::var("NOESQL_NQRI_VM").expect("NOESQL_NQRI_VM is required");
    let module = std::env::var("NOESQL_STAGE2_MODULE").expect("NOESQL_STAGE2_MODULE is required");
    let runtime = Stage2Runtime::new(vm, module);

    let mut path = std::env::temp_dir();
    path.push(format!("noesql-rust-stage2-zig-{}.noedb", std::process::id()));
    cleanup(&path);

    stage2_nqri::insert_bool_row(&runtime, &path, 7, 91, true).unwrap();
    assert_eq!(stage2_nqri::get_bool_row(&runtime, &path, 7, 91).unwrap(), Some(true));
    stage2_nqri::set_bool_row(&runtime, &path, 7, 91, false).unwrap();
    assert_eq!(stage2_nqri::get_bool_row(&runtime, &path, 7, 91).unwrap(), Some(false));
    stage2_nqri::delete_bool_row(&runtime, &path, 7, 91).unwrap();
    assert_eq!(stage2_nqri::get_bool_row(&runtime, &path, 7, 91).unwrap(), None);

    cleanup(&path);
    println!("Rust -> Stage-2 NQRI -> Zig pager smoke: OK");
}

#[cfg(feature = "stage2-nqri")]
fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}.wal", path.display()));
    let _ = std::fs::remove_file(format!("{}.lck", path.display()));
}

#[cfg(not(feature = "stage2-nqri"))]
fn main() {
    eprintln!("stage2_chain_smoke requires --features stage2-nqri");
    std::process::exit(2);
}

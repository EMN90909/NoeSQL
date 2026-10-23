use std::env;
use std::path::PathBuf;

fn main() {
    if env::var_os("CARGO_FEATURE_ZIG_PAGER").is_none() {
        return;
    }
    let lib_dir = env::var_os("NOESQL_ZIG_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../zig/zig-out/lib"));
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=noesql_zig_pages");
    if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=dl");
    }
    println!("cargo:rerun-if-env-changed=NOESQL_ZIG_LIB_DIR");
}

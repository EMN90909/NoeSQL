use std::env;
use std::path::PathBuf;

fn main() {
    let zig_enabled = env::var_os("CARGO_FEATURE_ZIG_PAGER").is_some();
    let noqeri_enabled = env::var_os("CARGO_FEATURE_NOQERI_NATIVE").is_some();

    if noqeri_enabled {
        let lib_dir = env::var_os("NOESQL_NOQERI_LIB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("native-lib"));
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        // Link order matters for static archives:
        // Rust -> Noqeri scalar kernel -> Zig pager.
        println!("cargo:rustc-link-lib=static=noesql_noqeri");
        println!("cargo:rerun-if-env-changed=NOESQL_NOQERI_LIB_DIR");
    }

    if zig_enabled {
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
}

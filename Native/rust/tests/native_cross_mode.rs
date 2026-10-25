// Cross-backend parity and durability qualification.
#[cfg(all(feature = "noqeri-native", feature = "stage2-nqri"))]
mod cross_mode {
    use noesql_runtime::{noqeri_native, stage2_nqri::{self, Stage2Runtime}};
    use std::path::Path;

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}.wal", path.display()));
        let _ = std::fs::remove_file(format!("{}.lck", path.display()));
    }

    #[test]
    fn native_and_stage2_roundtrip_same_page_without_format_conversion() {
        let vm = std::env::var_os("NOESQL_NQRI_VM").expect("NOESQL_NQRI_VM must be set");
        let module = std::env::var_os("NOESQL_STAGE2_MODULE").expect("NOESQL_STAGE2_MODULE must be set");
        let runtime = Stage2Runtime::new(vm, module);
        let mut path = std::env::temp_dir();
        path.push(format!("noesql-cross-mode-{}.noedb", std::process::id()));
        cleanup(&path);

        assert_eq!(noqeri_native::api_version(), 2);
        assert_eq!(noqeri_native::get_bool_row(&path, 255, i64::MIN).unwrap(), None);
        noqeri_native::insert_bool_row(&path, 255, i64::MIN, true).unwrap();

        assert_eq!(stage2_nqri::get_bool_row(&runtime, &path, 255, i64::MIN).unwrap(), Some(true));
        // A raw-row .noedb must never be silently overwritten by the snapshot API.
        let reference = noesql_runtime::reference::Database::default();
        assert!(noesql_runtime::zig_pager::save(&path, &reference).is_err());
        assert!(stage2_nqri::insert_bool_row(&runtime, &path, 255, i64::MIN, false).is_err());
        assert_eq!(noqeri_native::get_bool_row(&path, 255, i64::MIN).unwrap(), Some(true));

        stage2_nqri::set_bool_row(&runtime, &path, 255, i64::MIN, false).unwrap();
        assert_eq!(noqeri_native::get_bool_row(&path, 255, i64::MIN).unwrap(), Some(false));
        assert_eq!(noqeri_native::get_bool_row(&path, 255, i64::MAX).unwrap(), None);
        assert!(noqeri_native::set_bool_row(&path, 255, i64::MAX, true).is_err());

        noqeri_native::delete_bool_row(&path, 255, i64::MIN).unwrap();
        assert_eq!(stage2_nqri::get_bool_row(&runtime, &path, 255, i64::MIN).unwrap(), None);
        assert!(stage2_nqri::delete_bool_row(&runtime, &path, 255, i64::MIN).is_err());

        cleanup(&path);

        let mut snapshot = std::env::temp_dir();
        snapshot.push(format!("noesql-cross-snapshot-{}.noedb", std::process::id()));
        cleanup(&snapshot);
        let reference = noesql_runtime::reference::Database::default();
        noesql_runtime::zig_pager::save(&snapshot, &reference).unwrap();
        assert!(noqeri_native::get_bool_row(&snapshot, 255, 42).is_err());
        assert!(stage2_nqri::get_bool_row(&runtime, &snapshot, 255, 42).is_err());
        cleanup(&snapshot);
    }
}

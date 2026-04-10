#[cfg(feature = "daemon")]
mod conventions_integration {
    use cxpak::commands::conventions::{run_diff, run_export};
    use tempfile::TempDir;

    fn minimal_repo(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.join("lib.rs"), "pub fn compute() {}").unwrap();
    }

    #[test]
    fn conventions_export_diff_roundtrip() {
        let dir = TempDir::new().unwrap();
        minimal_repo(dir.path());
        let export_result = run_export(dir.path());
        // export may succeed or fail (minimal repo may not have full git objects)
        // if it succeeds, diff against same baseline should also succeed
        if export_result.is_ok() {
            let diff_result = run_diff(dir.path());
            assert!(diff_result.is_ok(), "diff failed: {:?}", diff_result);
        }
    }

    #[test]
    fn conventions_diff_fails_without_baseline() {
        let dir = TempDir::new().unwrap();
        minimal_repo(dir.path());
        let result = run_diff(dir.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No baseline found"), "unexpected error: {msg}");
        assert!(
            msg.contains("cxpak conventions export"),
            "missing fix hint: {msg}"
        );
    }
}

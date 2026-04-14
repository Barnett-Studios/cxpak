//! Integration tests for the `cxpak plugin list|add` CLI commands.
//!
//! These tests exercise the binary end-to-end by invoking the compiled
//! `cxpak` binary through `assert_cmd`.  All tests require the `plugins`
//! feature flag.

#[cfg(feature = "plugins")]
mod plugin_cli_tests {
    use assert_cmd::Command;
    use tempfile::TempDir;

    fn cxpak() -> Command {
        Command::new(assert_cmd::cargo_bin!("cxpak"))
    }

    /// Write minimal valid WASM magic bytes to `path`.
    /// The actual WASM module bytes are: magic number (4 bytes) + version (4 bytes).
    fn write_fake_wasm(path: &std::path::Path) {
        let bytes: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        std::fs::write(path, bytes).unwrap();
    }

    // -------------------------------------------------------------------------
    // plugin list — empty manifest
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_list_empty_on_fresh_directory() {
        let dir = TempDir::new().unwrap();
        let output = cxpak()
            .args(["plugin", "list"])
            .arg(dir.path())
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "plugin list must succeed on empty directory"
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("No plugins registered"),
            "stdout must say 'No plugins registered', got: {stdout}"
        );
    }

    // -------------------------------------------------------------------------
    // plugin add — happy path, then list shows entry with checksum
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_add_then_list_shows_name_patterns_and_checksum() {
        let dir = TempDir::new().unwrap();
        let plugins_dir = dir.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let wasm = plugins_dir.join("test.wasm");
        write_fake_wasm(&wasm);

        // add
        let add_out = cxpak()
            .args(["plugin", "add"])
            .arg(&wasm)
            .args(["--name", "test-plugin", "--patterns", "**/*.py"])
            .arg(dir.path())
            .output()
            .unwrap();

        assert!(
            add_out.status.success(),
            "plugin add must succeed: stderr={}",
            String::from_utf8_lossy(&add_out.stderr)
        );
        let add_stdout = String::from_utf8(add_out.stdout).unwrap();
        assert!(
            add_stdout.contains("Added plugin 'test-plugin'"),
            "add must confirm plugin name: {add_stdout}"
        );

        // list
        let list_out = cxpak()
            .args(["plugin", "list"])
            .arg(dir.path())
            .output()
            .unwrap();

        assert!(
            list_out.status.success(),
            "plugin list must succeed after add"
        );
        let list_stdout = String::from_utf8(list_out.stdout).unwrap();

        assert!(
            list_stdout.contains("test-plugin"),
            "list must show plugin name: {list_stdout}"
        );
        assert!(
            list_stdout.contains("**/*.py"),
            "list must show patterns: {list_stdout}"
        );
        // checksum must be exactly 64 hex characters on the 'checksum:' line
        let checksum_line = list_stdout
            .lines()
            .find(|l| l.contains("checksum:"))
            .expect("list must show a checksum line");
        let checksum_value = checksum_line
            .split("checksum:")
            .nth(1)
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(
            checksum_value.len(),
            64,
            "checksum must be 64 hex characters, got: '{checksum_value}'"
        );
        assert!(
            checksum_value.chars().all(|c| c.is_ascii_hexdigit()),
            "checksum must contain only hex digits: '{checksum_value}'"
        );

        // Total count
        assert!(
            list_stdout.contains("Total: 1 plugin"),
            "list must show 'Total: 1 plugin', got: {list_stdout}"
        );
    }

    #[test]
    fn plugin_add_multiple_then_list_shows_correct_total() {
        let dir = TempDir::new().unwrap();
        let wasm_a = dir.path().join("a.wasm");
        let wasm_b = dir.path().join("b.wasm");
        write_fake_wasm(&wasm_a);
        write_fake_wasm(&wasm_b);

        cxpak()
            .args(["plugin", "add"])
            .arg(&wasm_a)
            .args(["--name", "plugin-a", "--patterns", "**/*.py"])
            .arg(dir.path())
            .assert()
            .success();

        cxpak()
            .args(["plugin", "add"])
            .arg(&wasm_b)
            .args(["--name", "plugin-b", "--patterns", "**/*.ts"])
            .arg(dir.path())
            .assert()
            .success();

        let list_out = cxpak()
            .args(["plugin", "list"])
            .arg(dir.path())
            .output()
            .unwrap();

        assert!(list_out.status.success());
        let stdout = String::from_utf8(list_out.stdout).unwrap();
        assert!(
            stdout.contains("Total: 2 plugins"),
            "list must show 'Total: 2 plugins', got: {stdout}"
        );
        assert!(stdout.contains("plugin-a"), "list must mention plugin-a");
        assert!(stdout.contains("plugin-b"), "list must mention plugin-b");
    }

    // -------------------------------------------------------------------------
    // plugin add — duplicate name is rejected
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_add_duplicate_name_exits_nonzero_with_error_message() {
        let dir = TempDir::new().unwrap();
        let wasm = dir.path().join("dup.wasm");
        write_fake_wasm(&wasm);

        // First add — must succeed
        cxpak()
            .args(["plugin", "add"])
            .arg(&wasm)
            .args(["--name", "dup", "--patterns", "*"])
            .arg(dir.path())
            .assert()
            .success();

        // Second add with same name — must fail
        let dup = cxpak()
            .args(["plugin", "add"])
            .arg(&wasm)
            .args(["--name", "dup", "--patterns", "*"])
            .arg(dir.path())
            .output()
            .unwrap();

        assert!(
            !dup.status.success(),
            "duplicate plugin add must exit non-zero"
        );
        let stderr = String::from_utf8(dup.stderr).unwrap();
        assert!(
            stderr.contains("already registered"),
            "error must mention 'already registered': {stderr}"
        );
    }

    // -------------------------------------------------------------------------
    // plugin add — missing wasm file is rejected
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_add_missing_file_exits_nonzero() {
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("nonexistent.wasm");

        let result = cxpak()
            .args(["plugin", "add"])
            .arg(&nonexistent)
            .args(["--patterns", "*"])
            .arg(dir.path())
            .output()
            .unwrap();

        assert!(
            !result.status.success(),
            "plugin add of nonexistent file must fail"
        );
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(
            stderr.contains("not found") || stderr.contains("nonexistent"),
            "error must mention the missing file: {stderr}"
        );
    }

    // -------------------------------------------------------------------------
    // plugin add -- needs-content warns on stderr
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_add_needs_content_emits_warning_on_stderr() {
        let dir = TempDir::new().unwrap();
        let wasm = dir.path().join("invasive.wasm");
        write_fake_wasm(&wasm);

        let out = cxpak()
            .args(["plugin", "add"])
            .arg(&wasm)
            .args(["--name", "invasive", "--patterns", "*", "--needs-content"])
            .arg(dir.path())
            .output()
            .unwrap();

        assert!(
            out.status.success(),
            "plugin add --needs-content must succeed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stderr.contains("WARNING"),
            "--needs-content must emit a WARNING on stderr, got: {stderr}"
        );
        assert!(
            stderr.to_lowercase().contains("raw file content")
                || stderr.to_lowercase().contains("raw file contents"),
            "warning must mention 'raw file content(s)': {stderr}"
        );
    }

    // -------------------------------------------------------------------------
    // plugin add -- needs-content=false produces no warning
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_add_without_needs_content_emits_no_warning() {
        let dir = TempDir::new().unwrap();
        let wasm = dir.path().join("safe.wasm");
        write_fake_wasm(&wasm);

        let out = cxpak()
            .args(["plugin", "add"])
            .arg(&wasm)
            .args(["--name", "safe-plugin", "--patterns", "**/*.rs"])
            .arg(dir.path())
            .output()
            .unwrap();

        assert!(
            out.status.success(),
            "add without --needs-content must succeed"
        );
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            !stderr.contains("WARNING"),
            "no WARNING expected when --needs-content is not set, got: {stderr}"
        );
    }

    // -------------------------------------------------------------------------
    // plugin add -- wasm file outside repo root is rejected
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_add_wasm_outside_repo_root_is_rejected() {
        let repo_dir = TempDir::new().unwrap();
        let outside_dir = TempDir::new().unwrap();
        let external_wasm = outside_dir.path().join("external.wasm");
        write_fake_wasm(&external_wasm);

        let result = cxpak()
            .args(["plugin", "add"])
            .arg(&external_wasm)
            .args(["--name", "external", "--patterns", "**/*.py"])
            .arg(repo_dir.path())
            .output()
            .unwrap();

        assert!(
            !result.status.success(),
            "wasm file outside repo root must be rejected"
        );
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(
            stderr.contains("inside the repo root"),
            "error must mention 'inside the repo root': {stderr}"
        );
    }

    // -------------------------------------------------------------------------
    // plugin add -- stored checksum matches content
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_add_stored_checksum_matches_sha256_of_content() {
        use sha2::{Digest, Sha256};

        let dir = TempDir::new().unwrap();
        let wasm = dir.path().join("verify.wasm");
        let content: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        std::fs::write(&wasm, content).unwrap();

        cxpak()
            .args(["plugin", "add"])
            .arg(&wasm)
            .args(["--name", "verify-plugin", "--patterns", "**/*.go"])
            .arg(dir.path())
            .assert()
            .success();

        // Read the manifest and verify the checksum matches the actual sha256
        let manifest_path = dir.path().join(".cxpak").join("plugins.json");
        assert!(manifest_path.exists(), "manifest must be created");

        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap())
                .expect("manifest must be valid JSON");

        let stored_checksum = manifest["plugins"][0]["checksum"]
            .as_str()
            .expect("checksum must be a string");

        let expected = format!("{:x}", Sha256::digest(content));
        assert_eq!(
            stored_checksum, expected,
            "stored checksum must equal sha256 of actual wasm bytes"
        );
    }

    // -------------------------------------------------------------------------
    // plugin help — help text is accessible
    // -------------------------------------------------------------------------

    #[test]
    fn plugin_help_is_accessible() {
        cxpak().args(["plugin", "--help"]).assert().success();
    }

    #[test]
    fn plugin_list_help_is_accessible() {
        cxpak()
            .args(["plugin", "list", "--help"])
            .assert()
            .success();
    }

    #[test]
    fn plugin_add_help_is_accessible() {
        cxpak().args(["plugin", "add", "--help"]).assert().success();
    }
}

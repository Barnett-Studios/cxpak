#[cfg(feature = "lsp")]
mod lsp_integration {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use tempfile::TempDir;

    fn minimal_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .unwrap();
        dir
    }

    fn make_request(id: u64, method: &str, params: &str) -> String {
        let body =
            format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#);
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
    }

    #[test]
    #[ignore]
    fn lsp_initialize_shutdown_roundtrip() {
        let repo = minimal_repo();
        let mut child = Command::new(env!("CARGO_BIN_EXE_cxpak"))
            .args(["lsp", repo.path().to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn cxpak lsp");

        let stdin = child.stdin.as_mut().unwrap();
        let init = make_request(
            1,
            "initialize",
            r#"{"processId":null,"rootUri":null,"capabilities":{}}"#,
        );
        stdin.write_all(init.as_bytes()).unwrap();

        let shutdown = make_request(2, "shutdown", "null");
        stdin.write_all(shutdown.as_bytes()).unwrap();
        drop(child.stdin.take());

        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("capabilities"),
            "missing capabilities in response: {stdout}"
        );
    }

    #[test]
    #[ignore]
    fn lsp_custom_method_health() {
        let repo = minimal_repo();
        let mut child = Command::new(env!("CARGO_BIN_EXE_cxpak"))
            .args(["lsp", repo.path().to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn cxpak lsp");

        let stdin = child.stdin.as_mut().unwrap();
        stdin
            .write_all(
                make_request(
                    1,
                    "initialize",
                    r#"{"processId":null,"rootUri":null,"capabilities":{}}"#,
                )
                .as_bytes(),
            )
            .unwrap();
        stdin
            .write_all(make_request(2, "cxpak/health", "{}").as_bytes())
            .unwrap();
        stdin
            .write_all(make_request(3, "shutdown", "null").as_bytes())
            .unwrap();
        drop(child.stdin.take());

        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("total_files"),
            "missing total_files in response: {stdout}"
        );
    }
}

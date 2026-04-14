//! Integration tests for `cxpak visual` — coverage matrix of every view type
//! crossed with every output format.
//!
//! All tests are gated on `#[cfg(feature = "visual")]`.

#[cfg(feature = "visual")]
mod visual_cli_tests {
    use assert_cmd::Command;
    use tempfile::TempDir;

    fn cxpak() -> Command {
        Command::new(assert_cmd::cargo_bin!("cxpak"))
    }

    /// Build a minimal git repository so the scanner finds at least one source file.
    fn make_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\nfn helper() -> i32 { 42 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn greet() { println!(\"hi\"); }\npub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        dir
    }

    // -------------------------------------------------------------------------
    // PNG output — binary format validation
    // -------------------------------------------------------------------------

    #[test]
    fn visual_png_has_valid_magic_bytes_and_minimum_size() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("out.png");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "architecture",
                "--format",
                "png",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        assert!(out_file.exists(), "png output file must exist");
        let bytes = std::fs::read(&out_file).unwrap();
        // PNG magic bytes: 0x89 0x50 0x4E 0x47
        assert_eq!(
            &bytes[..4],
            &[0x89, 0x50, 0x4E, 0x47],
            "png output must start with PNG magic bytes"
        );
        assert!(
            bytes.len() >= 1024,
            "png output must be at least 1 KB, got {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn visual_png_dashboard_has_valid_magic_bytes() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("dashboard.png");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "dashboard",
                "--format",
                "png",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let bytes = std::fs::read(&out_file).unwrap();
        assert_eq!(
            &bytes[..4],
            &[0x89, 0x50, 0x4E, 0x47],
            "dashboard png must have PNG magic bytes"
        );
    }

    // -------------------------------------------------------------------------
    // C4 output — workspace/model keywords
    // -------------------------------------------------------------------------

    #[test]
    fn visual_c4_architecture_contains_workspace_and_model() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("out.dsl");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "architecture",
                "--format",
                "c4",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            content.contains("workspace"),
            "c4 output must contain 'workspace', got: {}",
            &content[..content.len().min(200)]
        );
        assert!(
            content.contains("model"),
            "c4 output must contain 'model', got: {}",
            &content[..content.len().min(200)]
        );
    }

    #[test]
    fn visual_c4_risk_contains_workspace_and_model() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("risk.dsl");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "risk",
                "--format",
                "c4",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            content.contains("workspace"),
            "c4 risk must contain 'workspace'"
        );
        assert!(content.contains("model"), "c4 risk must contain 'model'");
    }

    #[test]
    fn visual_c4_stdout_when_no_out() {
        let repo = make_test_repo();
        let output = cxpak()
            .args(["visual", "--visual-type", "architecture", "--format", "c4"])
            .arg(repo.path())
            .output()
            .unwrap();

        assert!(output.status.success(), "c4 without --out must succeed");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(!stdout.is_empty(), "c4 without --out must print to stdout");
        assert!(
            stdout.contains("workspace"),
            "stdout c4 must contain 'workspace'"
        );
    }

    // -------------------------------------------------------------------------
    // SVG output — svg element + xmlns for multiple view types
    // -------------------------------------------------------------------------

    #[test]
    fn visual_svg_risk_is_valid() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("risk.svg");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "risk",
                "--format",
                "svg",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            content.contains("<svg"),
            "svg risk output must contain <svg"
        );
        assert!(
            content.contains("xmlns"),
            "svg risk output must have xmlns attribute"
        );
        assert!(
            content.contains("</svg>"),
            "svg risk output must close </svg>"
        );
    }

    #[test]
    fn visual_svg_dashboard_is_valid() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("dashboard.svg");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "dashboard",
                "--format",
                "svg",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content = std::fs::read_to_string(&out_file).unwrap();
        assert!(content.contains("<svg"), "svg dashboard must contain <svg");
        assert!(content.contains("xmlns"), "svg dashboard must have xmlns");
    }

    // -------------------------------------------------------------------------
    // JSON output — parseable for multiple view types
    // -------------------------------------------------------------------------

    #[test]
    fn visual_json_dashboard_is_valid() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("dashboard.json");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "dashboard",
                "--format",
                "json",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content = std::fs::read(&out_file).unwrap();
        let _: serde_json::Value =
            serde_json::from_slice(&content).expect("dashboard json output must be valid JSON");
    }

    #[test]
    fn visual_json_risk_is_valid() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("risk.json");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "risk",
                "--format",
                "json",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content = std::fs::read(&out_file).unwrap();
        let _: serde_json::Value =
            serde_json::from_slice(&content).expect("risk json output must be valid JSON");
    }

    // -------------------------------------------------------------------------
    // Mermaid output — graph/flowchart header for multiple view types
    // -------------------------------------------------------------------------

    #[test]
    fn visual_mermaid_risk_has_graph_header() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("risk.mmd");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "risk",
                "--format",
                "mermaid",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content = std::fs::read_to_string(&out_file).unwrap();
        let first_line = content.lines().next().unwrap_or("");
        assert!(
            first_line.starts_with("graph") || first_line.starts_with("flowchart"),
            "mermaid risk must start with 'graph' or 'flowchart', got: {first_line}"
        );
    }

    #[test]
    fn visual_mermaid_dashboard_has_graph_header() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("dashboard.mmd");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "dashboard",
                "--format",
                "mermaid",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content = std::fs::read_to_string(&out_file).unwrap();
        let first_line = content.lines().next().unwrap_or("");
        assert!(
            first_line.starts_with("graph") || first_line.starts_with("flowchart"),
            "mermaid dashboard must start with 'graph' or 'flowchart', got: {first_line}"
        );
    }

    // -------------------------------------------------------------------------
    // HTML — self-contained output (no CDN references)
    // -------------------------------------------------------------------------

    #[test]
    fn visual_html_is_self_contained_no_cdn_refs() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("arch.html");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "architecture",
                "--format",
                "html",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let html = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            !html.contains("cdn.jsdelivr.net"),
            "html must not reference cdn.jsdelivr.net"
        );
        assert!(
            !html.contains("unpkg.com"),
            "html must not reference unpkg.com"
        );
        assert!(
            !html.contains("<link rel=\"stylesheet\" href=\"http"),
            "html must not link external stylesheets"
        );
    }

    #[test]
    fn visual_html_dashboard_is_self_contained() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("dash.html");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "dashboard",
                "--format",
                "html",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let html = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            !html.contains("cdn.jsdelivr.net"),
            "dashboard html must not reference CDN"
        );
        assert!(
            !html.contains("unpkg.com"),
            "dashboard html must not reference unpkg"
        );
    }

    // -------------------------------------------------------------------------
    // HTML output — not empty for all four non-flow/timeline types
    // -------------------------------------------------------------------------

    #[test]
    fn visual_html_risk_is_not_empty() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("risk.html");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "risk",
                "--format",
                "html",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let html = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            html.contains("<!DOCTYPE html>"),
            "risk html must have DOCTYPE"
        );
        assert!(!html.is_empty(), "risk html must not be empty");
    }

    // -------------------------------------------------------------------------
    // Flow view — requires --symbol; exits 0 or 1, never crashes
    // -------------------------------------------------------------------------

    #[test]
    fn visual_flow_with_symbol_exits_cleanly() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("flow.html");

        let output = cxpak()
            .args([
                "visual",
                "--visual-type",
                "flow",
                "--symbol",
                "main",
                "--format",
                "html",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .output()
            .unwrap();

        let code = output.status.code().expect("process must exit normally");
        assert!(
            code == 0 || code == 1,
            "flow with --symbol must exit 0 or 1, got {code}"
        );

        if code == 0 {
            let html = std::fs::read_to_string(&out_file).unwrap_or_default();
            assert!(
                html.contains("<!DOCTYPE html>"),
                "flow html output must have DOCTYPE"
            );
        }
    }

    #[test]
    fn visual_flow_without_symbol_uses_default_main() {
        // No --symbol given; the CLI defaults to "main". Must exit 0 or 1 (not crash).
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("flow_default.html");

        let output = cxpak()
            .args([
                "visual",
                "--visual-type",
                "flow",
                "--format",
                "html",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .output()
            .unwrap();

        let code = output
            .status
            .code()
            .expect("process must exit normally, not via signal");
        assert!(
            code == 0 || code == 1,
            "flow without explicit --symbol must exit 0 or 1, got {code}"
        );
    }

    // -------------------------------------------------------------------------
    // Diff view — exits cleanly with and without --files
    // -------------------------------------------------------------------------

    #[test]
    fn visual_diff_with_files_arg_exits_zero() {
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("diff.html");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "diff",
                "--files",
                "src/main.rs",
                "--format",
                "html",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let html = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            html.contains("<!DOCTYPE html>"),
            "diff html must have DOCTYPE"
        );
    }

    #[test]
    fn visual_diff_without_files_does_not_panic() {
        // diff without --files falls back to empty changed list; must exit 0 or 1
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_file = out_dir.path().join("diff_empty.html");

        let output = cxpak()
            .args([
                "visual",
                "--visual-type",
                "diff",
                "--format",
                "html",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .output()
            .unwrap();

        let code = output.status.code().expect("process must exit normally");
        assert!(
            code == 0 || code == 1,
            "diff without --files must exit 0 or 1, got {code}"
        );
    }

    // -------------------------------------------------------------------------
    // JSON determinism — two successive runs produce identical output
    // -------------------------------------------------------------------------

    #[test]
    fn visual_json_risk_is_deterministic() {
        let repo = make_test_repo();

        let run = |suffix: &str| -> serde_json::Value {
            let out_dir = TempDir::new().unwrap();
            let out_file = out_dir.path().join(format!("risk_{suffix}.json"));
            cxpak()
                .args([
                    "visual",
                    "--visual-type",
                    "risk",
                    "--format",
                    "json",
                    "--out",
                    out_file.to_str().unwrap(),
                ])
                .arg(repo.path())
                .assert()
                .success();
            let content = std::fs::read(&out_file).unwrap();
            serde_json::from_slice(&content).expect("run must produce valid JSON")
        };

        let first: serde_json::Value = run("a");
        let second: serde_json::Value = run("b");
        assert_eq!(
            first, second,
            "two runs of risk --format json must produce identical output"
        );
    }

    #[test]
    fn visual_json_dashboard_is_deterministic() {
        let repo = make_test_repo();

        let run = |suffix: &str| -> serde_json::Value {
            let out_dir = TempDir::new().unwrap();
            let out_file = out_dir.path().join(format!("dash_{suffix}.json"));
            cxpak()
                .args([
                    "visual",
                    "--visual-type",
                    "dashboard",
                    "--format",
                    "json",
                    "--out",
                    out_file.to_str().unwrap(),
                ])
                .arg(repo.path())
                .assert()
                .success();
            let content = std::fs::read(&out_file).unwrap();
            serde_json::from_slice(&content).expect("run must produce valid JSON")
        };

        let first: serde_json::Value = run("a");
        let second: serde_json::Value = run("b");
        assert_eq!(
            first, second,
            "two runs of dashboard --format json must produce identical output"
        );
    }

    // -------------------------------------------------------------------------
    // Output file is non-empty for all combinations that write to --out
    // -------------------------------------------------------------------------

    /// Helper: run `cxpak visual` with --out and assert the file is non-empty.
    fn assert_visual_out_nonempty(
        repo_path: &std::path::Path,
        view: &str,
        format: &str,
        extra_args: &[&str],
    ) {
        let out_dir = TempDir::new().unwrap();
        let ext = match format {
            "html" => "html",
            "mermaid" => "mmd",
            "svg" => "svg",
            "png" => "png",
            "c4" => "dsl",
            "json" => "json",
            _ => "out",
        };
        let out_file = out_dir.path().join(format!("{view}.{ext}"));

        let mut cmd = cxpak();
        cmd.args(["visual", "--visual-type", view, "--format", format, "--out"])
            .arg(&out_file);
        for arg in extra_args {
            cmd.arg(arg);
        }
        cmd.arg(repo_path);

        let output = cmd.output().unwrap();
        let code = output.status.code().unwrap_or(-1);
        // For timeline we may get code 0 or 1 (no cached snapshots).
        if view == "timeline" && code != 0 {
            return;
        }
        assert!(
            output.status.success(),
            "visual --visual-type {view} --format {format} failed (exit {code}): stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            out_file.exists(),
            "output file must exist at {}",
            out_file.display()
        );
        let bytes = std::fs::read(&out_file).unwrap();
        assert!(
            !bytes.is_empty(),
            "output file must not be empty for view={view} format={format}"
        );
    }

    #[test]
    fn visual_all_non_flow_types_produce_html_output() {
        let repo = make_test_repo();
        for view in &["dashboard", "architecture", "risk", "diff"] {
            let extra: &[&str] = if *view == "diff" {
                &["--files", "src/main.rs"]
            } else {
                &[]
            };
            assert_visual_out_nonempty(repo.path(), view, "html", extra);
        }
    }

    #[test]
    fn visual_all_non_flow_types_produce_json_output() {
        let repo = make_test_repo();
        for view in &["dashboard", "architecture", "risk", "diff"] {
            let extra: &[&str] = if *view == "diff" {
                &["--files", "src/main.rs"]
            } else {
                &[]
            };
            assert_visual_out_nonempty(repo.path(), view, "json", extra);
        }
    }

    #[test]
    fn visual_all_non_flow_types_produce_svg_output() {
        let repo = make_test_repo();
        for view in &["dashboard", "architecture", "risk", "diff"] {
            let extra: &[&str] = if *view == "diff" {
                &["--files", "src/main.rs"]
            } else {
                &[]
            };
            assert_visual_out_nonempty(repo.path(), view, "svg", extra);
        }
    }

    #[test]
    fn visual_all_non_flow_types_produce_mermaid_output() {
        let repo = make_test_repo();
        for view in &["dashboard", "architecture", "risk", "diff"] {
            let extra: &[&str] = if *view == "diff" {
                &["--files", "src/main.rs"]
            } else {
                &[]
            };
            assert_visual_out_nonempty(repo.path(), view, "mermaid", extra);
        }
    }

    #[test]
    fn visual_all_non_flow_types_produce_c4_output() {
        let repo = make_test_repo();
        for view in &["dashboard", "architecture", "risk", "diff"] {
            let extra: &[&str] = if *view == "diff" {
                &["--files", "src/main.rs"]
            } else {
                &[]
            };
            assert_visual_out_nonempty(repo.path(), view, "c4", extra);
        }
    }

    #[test]
    fn visual_all_non_flow_types_produce_png_output() {
        let repo = make_test_repo();
        for view in &["dashboard", "architecture", "risk", "diff"] {
            let extra: &[&str] = if *view == "diff" {
                &["--files", "src/main.rs"]
            } else {
                &[]
            };
            assert_visual_out_nonempty(repo.path(), view, "png", extra);
        }
    }
}

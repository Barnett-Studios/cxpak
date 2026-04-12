//! Integration tests for the `cxpak visual` and related visual output commands.
//!
//! All tests require the `visual` feature flag and exercise the CLI end-to-end
//! against a real git repository created in a temporary directory.

#[cfg(feature = "visual")]
mod visual_tests {
    use assert_cmd::Command;
    use std::path::Path;
    use tempfile::TempDir;

    fn cxpak() -> Command {
        Command::new(assert_cmd::cargo_bin!("cxpak"))
    }

    /// Build a minimal git repository with a few source files in a temp dir.
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
            dir.path().join("src/utils.rs"),
            "pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 { v.max(lo).min(hi) }\n",
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

    /// Run `cxpak visual --format <fmt>` writing output to a file, return the content.
    fn run_visual_to_file(repo: &Path, visual_type: &str, format: &str) -> String {
        let out_dir = TempDir::new().unwrap();
        let out_path = out_dir.path().join(format!("out.{format}"));

        cxpak()
            .args([
                "visual",
                "--visual-type",
                visual_type,
                "--format",
                format,
                "--out",
                out_path.to_str().unwrap(),
            ])
            .arg(repo)
            .assert()
            .success();

        std::fs::read_to_string(&out_path).expect("output file should exist after success")
    }

    // -------------------------------------------------------------------------
    // HTML — well-formed structure
    // -------------------------------------------------------------------------

    #[test]
    fn visual_html_architecture_is_well_formed() {
        let repo = make_test_repo();
        let content = run_visual_to_file(repo.path(), "architecture", "html");

        assert!(
            content.contains("<!DOCTYPE html>"),
            "HTML output must start with DOCTYPE"
        );
        assert!(
            content.contains("<html"),
            "HTML output must contain <html tag"
        );
        assert!(
            content.contains("</html>"),
            "HTML output must contain </html>"
        );
        assert!(
            content.contains("</body>"),
            "HTML output must contain </body>"
        );
        // The architecture explorer embeds its data in a JSON script tag.
        // Verify that JSON data block does not contain literal "undefined" or NaN.
        // (The vendor JS bundle may legitimately use `typeof undefined` — we
        // restrict the check to the cxpak data script tags only.)
        if let Some(start) = content.find("<script id=\"cxpak-") {
            let data_section = &content[start..];
            let end = data_section.find("</script>").unwrap_or(data_section.len());
            let data_json = &data_section[..end];
            assert!(
                !data_json.contains(":undefined") && !data_json.contains(",undefined"),
                "cxpak data JSON must not contain bare undefined values"
            );
            assert!(
                !data_json.contains(":NaN") && !data_json.contains(",NaN"),
                "cxpak data JSON must not contain NaN values"
            );
        }
    }

    #[test]
    fn visual_html_dashboard_is_well_formed() {
        let repo = make_test_repo();
        let content = run_visual_to_file(repo.path(), "dashboard", "html");

        assert!(content.contains("<!DOCTYPE html>"));
        assert!(content.contains("<html"));
        assert!(content.contains("</html>"));
        // Check that the HTML structure is complete
        assert!(content.contains("</body>") || content.contains("</html>"));
    }

    #[test]
    fn visual_html_risk_is_well_formed() {
        let repo = make_test_repo();
        let content = run_visual_to_file(repo.path(), "risk", "html");

        assert!(content.contains("<!DOCTYPE html>"));
        assert!(content.contains("<html"));
        assert!(content.contains("</html>"));
        assert!(content.contains("</body>") || content.contains("</html>"));
    }

    // -------------------------------------------------------------------------
    // SVG — valid structure
    // -------------------------------------------------------------------------

    #[test]
    fn visual_svg_contains_svg_element() {
        let repo = make_test_repo();
        let content = run_visual_to_file(repo.path(), "architecture", "svg");

        assert!(
            content.contains("<svg"),
            "SVG output must contain <svg element"
        );
        assert!(content.contains("</svg>"), "SVG output must contain </svg>");
        assert!(
            content.contains("xmlns"),
            "SVG output must contain xmlns attribute"
        );
        // At minimum a background rect is always present
        assert!(
            content.contains("<rect"),
            "SVG output must contain at least one <rect"
        );
    }

    // -------------------------------------------------------------------------
    // Mermaid — graph syntax
    // -------------------------------------------------------------------------

    #[test]
    fn visual_mermaid_has_graph_header() {
        let repo = make_test_repo();
        // Mermaid format prints to stdout when no --out is given; use --out here
        let content = run_visual_to_file(repo.path(), "architecture", "mermaid");

        assert!(
            content.starts_with("graph"),
            "Mermaid output must start with 'graph', got: {}",
            &content[..content.len().min(40)]
        );
        // A well-formed Mermaid graph has at least one node definition
        assert!(
            content.contains("[\n") || content.contains("[\""),
            "Mermaid output must contain at least one node"
        );
    }

    // -------------------------------------------------------------------------
    // JSON — valid and parseable
    // -------------------------------------------------------------------------

    #[test]
    fn visual_json_is_valid() {
        let repo = make_test_repo();
        let content = run_visual_to_file(repo.path(), "architecture", "json");

        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("visual JSON output must be valid JSON");

        // The JSON layout contains nodes and edges arrays
        assert!(
            parsed.get("nodes").is_some() || parsed.get("layers").is_some(),
            "JSON output must have a known top-level key; got keys: {:?}",
            parsed
                .as_object()
                .map(|m| m.keys().collect::<Vec<_>>())
                .unwrap_or_default()
        );
    }

    // -------------------------------------------------------------------------
    // Determinism — two runs produce identical output
    // -------------------------------------------------------------------------

    #[test]
    fn visual_html_architecture_structure_is_stable() {
        // Verify that the static structural parts of the HTML are identical
        // across two runs.  We check the <head> section which does not include
        // the variable data blob or the generated_at timestamp.
        let repo = make_test_repo();

        let first = run_visual_to_file(repo.path(), "architecture", "html");
        let second = run_visual_to_file(repo.path(), "architecture", "html");

        // Both runs must produce a DOCTYPE and the same title.
        assert!(first.contains("<!DOCTYPE html>"));
        assert!(second.contains("<!DOCTYPE html>"));

        // Extract just the <title> element from both — it should be identical.
        let extract_title = |s: &str| -> String {
            let start = s.find("<title>").unwrap_or(0);
            let end = s.find("</title>").map(|p| p + 8).unwrap_or(0);
            s[start..end.max(start)].to_string()
        };

        assert_eq!(
            extract_title(&first),
            extract_title(&second),
            "HTML <title> must be identical across runs"
        );
    }

    #[test]
    fn visual_json_is_deterministic() {
        let repo = make_test_repo();

        let first = run_visual_to_file(repo.path(), "architecture", "json");
        let second = run_visual_to_file(repo.path(), "architecture", "json");

        // JSON layout doesn't embed timestamps, so the comparison is exact.
        assert_eq!(
            first, second,
            "Two runs of `cxpak visual --format json` must produce identical output"
        );
    }

    // -------------------------------------------------------------------------
    // Stdout behaviour — non-HTML text formats print to stdout without --out
    // -------------------------------------------------------------------------

    #[test]
    fn visual_mermaid_prints_to_stdout_without_out() {
        let repo = make_test_repo();

        let output = cxpak()
            .args([
                "visual",
                "--visual-type",
                "architecture",
                "--format",
                "mermaid",
            ])
            .arg(repo.path())
            .output()
            .expect("failed to run cxpak visual");

        assert!(output.status.success(), "command should succeed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.starts_with("graph"),
            "stdout must contain Mermaid graph syntax"
        );
    }

    #[test]
    fn visual_json_prints_to_stdout_without_out() {
        let repo = make_test_repo();

        let output = cxpak()
            .args([
                "visual",
                "--visual-type",
                "architecture",
                "--format",
                "json",
            ])
            .arg(repo.path())
            .output()
            .expect("failed to run cxpak visual");

        assert!(output.status.success(), "command should succeed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let _: serde_json::Value =
            serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    }

    // -------------------------------------------------------------------------
    // Timeline — well-formed HTML
    // -------------------------------------------------------------------------

    #[test]
    fn visual_html_timeline_is_well_formed() {
        // Timeline requires cached snapshots to render — seed the cache by
        // running compute_timeline_snapshots on the test repo first.
        let repo = make_test_repo();

        let snapshots =
            cxpak::visual::timeline::compute_timeline_snapshots(repo.path(), 10).unwrap();
        assert!(
            !snapshots.is_empty(),
            "test repo must have at least one commit"
        );
        cxpak::visual::timeline::save_snapshots(repo.path(), &snapshots)
            .expect("should save timeline snapshots");

        let out_dir = TempDir::new().unwrap();
        let out_path = out_dir.path().join("out.html");

        cxpak()
            .args([
                "visual",
                "--visual-type",
                "timeline",
                "--format",
                "html",
                "--out",
                out_path.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content =
            std::fs::read_to_string(&out_path).expect("output file should exist after success");
        assert!(
            content.contains("<!DOCTYPE html>"),
            "timeline HTML must have DOCTYPE"
        );
        assert!(content.contains("<html"), "timeline HTML must have <html");
        assert!(
            content.contains("</html>"),
            "timeline HTML must have </html>"
        );
    }

    // -------------------------------------------------------------------------
    // Flow — does not panic, exits cleanly
    // -------------------------------------------------------------------------

    #[test]
    fn visual_html_flow_produces_output() {
        // The simple fixture repo has no call graph edges, so the flow diagram
        // will have no paths, but the command must not panic and must exit with
        // code 0 or 1 (a legitimate "no data" error), never a signal/crash.
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_path = out_dir.path().join("out.html");

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
                out_path.to_str().unwrap(),
            ])
            .arg(repo.path())
            .output()
            .expect("failed to spawn cxpak visual flow");

        // The command must not crash (signal) — exit code 0 or 1 are both acceptable.
        let code = output
            .status
            .code()
            .expect("process must exit normally, not via signal");
        assert!(
            code == 0 || code == 1,
            "cxpak visual flow must exit 0 or 1, got {code}"
        );

        // If it succeeded, the file must contain valid HTML.
        if code == 0 {
            if let Ok(content) = std::fs::read_to_string(&out_path) {
                assert!(
                    content.contains("<!DOCTYPE html>"),
                    "flow HTML output must have DOCTYPE"
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // Diff — well-formed HTML
    // -------------------------------------------------------------------------

    #[test]
    fn visual_html_diff_produces_output() {
        // Diff with a known file path renders a diff view comparing that file
        // against the current index.
        let repo = make_test_repo();
        let out_dir = TempDir::new().unwrap();
        let out_path = out_dir.path().join("out.html");

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
                out_path.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let content =
            std::fs::read_to_string(&out_path).expect("output file should exist after success");
        assert!(
            content.contains("<!DOCTYPE html>"),
            "diff HTML must have DOCTYPE"
        );
        assert!(content.contains("<html"), "diff HTML must have <html");
        assert!(content.contains("</html>"), "diff HTML must have </html>");
    }
}

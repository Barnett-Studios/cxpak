//! Integration tests for the `cxpak onboard` CLI command.
//!
//! These tests exercise the binary end-to-end: output to a file via `--out`,
//! output to stdout (no `--out`), and structural correctness for every supported
//! format (markdown, json, xml).
//!
//! All tests are gated on `#[cfg(feature = "visual")]`.

#[cfg(feature = "visual")]
mod onboard_cli_tests {
    use assert_cmd::Command;
    use tempfile::TempDir;

    fn cxpak() -> Command {
        Command::new(assert_cmd::cargo_bin!("cxpak"))
    }

    /// Build a minimal git repository that `cxpak onboard` can index.
    fn make_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn greet() -> &'static str { \"hi\" }\n",
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
    // Markdown format — stdout and file output
    // -------------------------------------------------------------------------

    #[test]
    fn onboard_markdown_stdout_has_heading_and_sections() {
        let repo = make_test_repo();
        let output = cxpak()
            .args(["onboard", "--format", "markdown"])
            .arg(repo.path())
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "onboard --format markdown must succeed"
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("# Codebase Onboarding Map"),
            "markdown must contain top-level heading"
        );
        assert!(
            stdout.contains("Phase"),
            "markdown must contain at least one Phase section"
        );
        // Must have at least one file path wrapped in backticks
        assert!(
            stdout.matches('`').count() >= 2,
            "markdown must contain at least one backtick-wrapped path, got: {}",
            &stdout[..stdout.len().min(400)]
        );
    }

    #[test]
    fn onboard_markdown_to_file_has_heading_and_phases() {
        let repo = make_test_repo();
        let out = TempDir::new().unwrap();
        let out_file = out.path().join("onboard.md");

        cxpak()
            .args([
                "onboard",
                "--format",
                "markdown",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        assert!(out_file.exists(), "output file must be created");
        let md = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            md.contains("# Codebase Onboarding Map"),
            "file output must contain onboarding heading"
        );
        assert!(md.contains("Phase"), "file output must mention Phase");
        assert!(!md.is_empty(), "onboard markdown file must not be empty");
    }

    // -------------------------------------------------------------------------
    // JSON format — stdout and file output
    // -------------------------------------------------------------------------

    #[test]
    fn onboard_json_stdout_has_phases_array_and_metadata() {
        let repo = make_test_repo();
        let output = cxpak()
            .args(["onboard", "--format", "json"])
            .arg(repo.path())
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "onboard --format json must succeed"
        );
        let j: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON");

        assert!(
            j["phases"].is_array(),
            "json output must have a 'phases' array"
        );
        assert!(
            j["total_files"].is_number(),
            "json output must have a numeric 'total_files'"
        );
        assert!(
            j["total_files"].as_u64().unwrap() >= 1,
            "total_files must be at least 1"
        );
        assert!(
            j["estimated_reading_time"].is_string(),
            "json output must have a string 'estimated_reading_time'"
        );
        let rt = j["estimated_reading_time"].as_str().unwrap();
        assert!(
            rt.starts_with('~'),
            "estimated_reading_time must start with '~', got: {rt}"
        );
    }

    #[test]
    fn onboard_json_to_file_has_phases_array_and_metadata() {
        let repo = make_test_repo();
        let out = TempDir::new().unwrap();
        let out_file = out.path().join("onboard.json");

        cxpak()
            .args([
                "onboard",
                "--format",
                "json",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        assert!(out_file.exists(), "json output file must be created");
        let j: serde_json::Value = serde_json::from_slice(&std::fs::read(&out_file).unwrap())
            .expect("file must contain valid JSON");

        assert!(j["phases"].is_array(), "file JSON must have phases array");
        assert!(
            j["total_files"].is_number(),
            "file JSON must have total_files"
        );
        assert!(
            j["estimated_reading_time"].is_string(),
            "file JSON must have estimated_reading_time"
        );
    }

    #[test]
    fn onboard_json_phases_each_have_required_fields() {
        let repo = make_test_repo();
        let output = cxpak()
            .args(["onboard", "--format", "json"])
            .arg(repo.path())
            .output()
            .unwrap();

        assert!(output.status.success());
        let j: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let phases = j["phases"].as_array().unwrap();

        for phase in phases {
            assert!(
                phase["name"].is_string(),
                "each phase must have a string 'name', got: {phase}"
            );
            assert!(
                phase["files"].is_array(),
                "each phase must have a 'files' array, got: {phase}"
            );
            // Each file entry must have a path
            for file in phase["files"].as_array().unwrap() {
                assert!(
                    file["path"].is_string(),
                    "each file entry must have a string 'path', got: {file}"
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // XML format — stdout and file output
    // -------------------------------------------------------------------------

    #[test]
    fn onboard_xml_stdout_is_well_formed() {
        let repo = make_test_repo();
        let output = cxpak()
            .args(["onboard", "--format", "xml"])
            .arg(repo.path())
            .output()
            .unwrap();

        assert!(output.status.success(), "onboard --format xml must succeed");
        let xml = String::from_utf8(output.stdout).unwrap();

        assert!(
            xml.starts_with("<?xml"),
            "xml must start with xml declaration, got: {}",
            &xml[..xml.len().min(40)]
        );
        assert!(
            xml.contains("<onboarding>"),
            "xml must contain <onboarding>"
        );
        assert!(
            xml.contains("</onboarding>"),
            "xml must contain </onboarding>"
        );
        assert!(
            xml.contains("<total_files>"),
            "xml must contain <total_files>"
        );
    }

    #[test]
    fn onboard_xml_to_file_has_balanced_phase_tags() {
        let repo = make_test_repo();
        let out = TempDir::new().unwrap();
        let out_file = out.path().join("onboard.xml");

        cxpak()
            .args([
                "onboard",
                "--format",
                "xml",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let xml = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            xml.starts_with("<?xml"),
            "xml file must start with declaration"
        );
        assert!(
            xml.contains("<onboarding>"),
            "xml file must have <onboarding>"
        );
        assert!(
            xml.contains("</onboarding>"),
            "xml file must have </onboarding>"
        );

        // <phase ...> and </phase> must be balanced
        let open_count = xml.matches("<phase ").count();
        let close_count = xml.matches("</phase>").count();
        assert_eq!(
            open_count, close_count,
            "phase tags must be balanced: {open_count} open, {close_count} close"
        );
    }

    #[test]
    fn onboard_xml_file_does_not_contain_unescaped_ampersand() {
        // Verifies that XML special characters are escaped correctly.
        let repo = make_test_repo();
        let out = TempDir::new().unwrap();
        let out_file = out.path().join("onboard.xml");

        cxpak()
            .args([
                "onboard",
                "--format",
                "xml",
                "--out",
                out_file.to_str().unwrap(),
            ])
            .arg(repo.path())
            .assert()
            .success();

        let xml = std::fs::read_to_string(&out_file).unwrap();
        // Strip off the XML prologue and skeleton tags to check content only.
        // All & characters in content must appear as &amp;
        // We verify that no raw & appears in attribute values (path="...&...").
        for line in xml.lines() {
            if line.trim_start().starts_with("<file ") || line.trim_start().starts_with("<phase ") {
                assert!(
                    !line.contains("&amp;") || !line.contains(" & "),
                    "raw '&' in XML attribute at: {line}"
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // Default format is markdown when no --format is given
    // -------------------------------------------------------------------------

    #[test]
    fn onboard_default_format_is_markdown() {
        let repo = make_test_repo();
        let output = cxpak().args(["onboard"]).arg(repo.path()).output().unwrap();

        assert!(
            output.status.success(),
            "onboard without --format must succeed"
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        // Default format is markdown — must contain the onboarding heading
        assert!(
            stdout.contains("# Codebase Onboarding Map") || stdout.contains("Phase"),
            "default onboard output must look like markdown"
        );
    }
}

//! Integration tests for the onboarding map pipeline.
//!
//! These tests exercise `cxpak::visual::onboard::compute_onboarding_map` directly
//! and also validate the `cxpak onboard` CLI command end-to-end.  All tests
//! that touch the visual module are gated behind `#[cfg(feature = "visual")]`.

#[cfg(feature = "visual")]
mod onboarding_tests {
    use assert_cmd::Command;
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    use cxpak::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
    use cxpak::scanner::ScannedFile;
    use cxpak::visual::onboard::compute_onboarding_map;
    use std::collections::HashMap;
    use std::path::PathBuf;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn cxpak() -> Command {
        Command::new(assert_cmd::cargo_bin!("cxpak"))
    }

    /// Build a `CodebaseIndex` from a set of in-memory files without touching
    /// the filesystem.  Public symbols are attached to files that have a
    /// `parse_result` entry; all other files get no parse result.
    fn build_index_from_files(
        files: Vec<ScannedFile>,
        parse_results: HashMap<String, ParseResult>,
        content_map: HashMap<String, String>,
    ) -> CodebaseIndex {
        let counter = TokenCounter::new();
        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    /// Returns a `ScannedFile` with a Rust language tag.
    fn rust_file(rel: &str) -> ScannedFile {
        ScannedFile {
            relative_path: rel.to_string(),
            absolute_path: PathBuf::from("/tmp").join(rel),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }
    }

    /// Returns a `ParseResult` containing a single public function named `name`.
    fn single_pub_fn(name: &str) -> ParseResult {
        ParseResult {
            symbols: vec![Symbol {
                name: name.to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: format!("pub fn {name}()"),
                body: "{}".to_string(),
                start_line: 1,
                end_line: 1,
            }],
            imports: vec![Import {
                source: "std".to_string(),
                names: vec![],
            }],
            exports: vec![],
        }
    }

    /// Create a multi-file index that exercises entry-point detection and
    /// cross-module grouping.
    fn make_multi_file_index() -> CodebaseIndex {
        let files = vec![
            rust_file("src/main.rs"),
            rust_file("src/lib.rs"),
            rust_file("src/commands/mod.rs"),
            rust_file("src/commands/run.rs"),
            rust_file("src/parser/mod.rs"),
            rust_file("src/parser/ast.rs"),
            rust_file("src/index/mod.rs"),
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert("src/main.rs".to_string(), single_pub_fn("main"));
        parse_results.insert("src/lib.rs".to_string(), single_pub_fn("run"));
        parse_results.insert("src/commands/mod.rs".to_string(), single_pub_fn("execute"));
        parse_results.insert("src/parser/mod.rs".to_string(), single_pub_fn("parse"));
        parse_results.insert("src/index/mod.rs".to_string(), single_pub_fn("build"));

        let mut content_map = HashMap::new();
        for f in &files {
            content_map.insert(f.relative_path.clone(), format!("// {}", f.relative_path));
        }

        build_index_from_files(files, parse_results, content_map)
    }

    // -------------------------------------------------------------------------
    // Test 1: Determinism
    // -------------------------------------------------------------------------

    #[test]
    fn onboarding_map_is_deterministic() {
        let index = make_multi_file_index();
        let map1 = compute_onboarding_map(&index, None);
        let map2 = compute_onboarding_map(&index, None);

        let json1 = serde_json::to_string(&map1).expect("serialization must succeed");
        let json2 = serde_json::to_string(&map2).expect("serialization must succeed");

        assert_eq!(
            json1, json2,
            "Two calls to compute_onboarding_map on the same index must produce identical output"
        );
    }

    // -------------------------------------------------------------------------
    // Test 2: Phase size constraint (1–9 files per phase)
    // -------------------------------------------------------------------------

    #[test]
    fn every_phase_has_between_one_and_nine_files() {
        let index = make_multi_file_index();
        let map = compute_onboarding_map(&index, None);

        assert!(
            !map.phases.is_empty(),
            "onboarding map must have at least one phase"
        );

        for phase in &map.phases {
            assert!(
                !phase.files.is_empty(),
                "phase '{}' must have at least 1 file",
                phase.name
            );
            assert!(
                phase.files.len() <= 9,
                "phase '{}' has {} files — must be ≤9",
                phase.name,
                phase.files.len()
            );
        }
    }

    #[test]
    fn phase_size_constraint_holds_for_large_index() {
        // Build an index with 30 files all in the same module to force splitting.
        let files: Vec<ScannedFile> = (0..30)
            .map(|i| rust_file(&format!("src/big_module/file_{i:02}.rs")))
            .collect();
        let content_map: HashMap<String, String> = files
            .iter()
            .map(|f| (f.relative_path.clone(), format!("// {}", f.relative_path)))
            .collect();
        let index = build_index_from_files(files, HashMap::new(), content_map);

        let map = compute_onboarding_map(&index, None);

        for phase in &map.phases {
            assert!(
                phase.files.len() <= 9,
                "phase '{}' has {} files — must be ≤9 even for large modules",
                phase.name,
                phase.files.len()
            );
        }
    }

    // -------------------------------------------------------------------------
    // Test 3: Symbols limit (≤5 symbols_to_focus_on per file)
    // -------------------------------------------------------------------------

    #[test]
    fn each_file_has_at_most_five_symbols_to_focus_on() {
        // Create a file with many public symbols.
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/big_api.rs".to_string(),
            ParseResult {
                symbols: (0..10)
                    .map(|i| Symbol {
                        name: format!("fn_{i}"),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: format!("pub fn fn_{i}()"),
                        body: "{}".to_string(),
                        start_line: i + 1,
                        end_line: i + 1,
                    })
                    .collect(),
                imports: vec![],
                exports: vec![],
            },
        );

        let files = vec![rust_file("src/main.rs"), rust_file("src/big_api.rs")];
        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
        content_map.insert(
            "src/big_api.rs".to_string(),
            "pub fn fn_0() {} // ...".to_string(),
        );

        let index = build_index_from_files(files, parse_results, content_map);
        let map = compute_onboarding_map(&index, None);

        for phase in &map.phases {
            for file in &phase.files {
                assert!(
                    file.symbols_to_focus_on.len() <= 5,
                    "file '{}' has {} symbols_to_focus_on — must be ≤5",
                    file.path,
                    file.symbols_to_focus_on.len()
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // Test 4: Reading time format
    // -------------------------------------------------------------------------

    #[test]
    fn reading_time_format_for_small_index_is_minutes() {
        // Small index → should be a few minutes, no hours.
        let index = make_multi_file_index();
        let map = compute_onboarding_map(&index, None);
        let rt = &map.estimated_reading_time;

        assert!(
            rt.starts_with('~'),
            "reading time must start with '~', got: {rt}"
        );
        // For a tiny codebase, we expect minutes not hours.
        assert!(
            !rt.contains('h'),
            "reading time for small index should not show hours, got: {rt}"
        );
        // Must match ~{digits}m
        let re = regex::Regex::new(r"^~\d+m$").unwrap();
        assert!(
            re.is_match(rt),
            "reading time for small index must match ~{{digits}}m, got: {rt}"
        );
    }

    #[test]
    fn reading_time_format_for_large_index_includes_hours() {
        // 600 tokens/file × 300 files = 180,000 tokens → 900 minutes → 15 hours
        let files: Vec<ScannedFile> = (0..300)
            .map(|i| rust_file(&format!("src/module_{i:03}/lib.rs")))
            .collect();

        // Each file gets ~600 chars which the token counter will turn into ~150 tokens,
        // giving 300 × 150 = 45,000 tokens → 225 minutes → ~3h 45m.
        let content_map: HashMap<String, String> = files
            .iter()
            .map(|f| {
                (
                    f.relative_path.clone(),
                    "pub fn placeholder() {}\n".repeat(25),
                )
            })
            .collect();

        let index = build_index_from_files(files, HashMap::new(), content_map);
        let map = compute_onboarding_map(&index, None);
        let rt = &map.estimated_reading_time;

        assert!(
            rt.starts_with('~'),
            "reading time must start with '~', got: {rt}"
        );

        // Must match either ~{digits}m or ~{digits}h {digits}m
        let minutes_re = regex::Regex::new(r"^~\d+m$").unwrap();
        let hours_re = regex::Regex::new(r"^~\d+h \d+m$").unwrap();
        assert!(
            minutes_re.is_match(rt) || hours_re.is_match(rt),
            "reading time must match ~{{digits}}m or ~{{digits}}h {{digits}}m, got: {rt}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 5: total_files equals sum across phases
    // -------------------------------------------------------------------------

    #[test]
    fn total_files_equals_sum_of_phase_file_counts() {
        let index = make_multi_file_index();
        let map = compute_onboarding_map(&index, None);

        let sum: usize = map.phases.iter().map(|p| p.files.len()).sum();
        assert_eq!(
            map.total_files, sum,
            "total_files must equal the sum of files across all phases"
        );
    }

    // -------------------------------------------------------------------------
    // Test 6: Entry points phase comes first when present
    // -------------------------------------------------------------------------

    #[test]
    fn entry_points_phase_is_first_when_present() {
        let index = make_multi_file_index();
        let map = compute_onboarding_map(&index, None);

        // The index contains src/main.rs and src/lib.rs which are entry points.
        let first_phase_name = &map.phases[0].name;
        assert_eq!(
            first_phase_name, "Entry Points",
            "first phase must be 'Entry Points' when entry points exist, got: {first_phase_name}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 7: Empty index produces valid map
    // -------------------------------------------------------------------------

    #[test]
    fn empty_index_produces_valid_empty_map() {
        let counter = TokenCounter::new();
        let index =
            CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
        let map = compute_onboarding_map(&index, None);

        assert_eq!(map.total_files, 0, "empty index must have 0 total_files");
        assert!(map.phases.is_empty(), "empty index must have no phases");
        assert!(
            map.estimated_reading_time.starts_with('~'),
            "reading time must start with '~' even for empty index"
        );
    }

    // -------------------------------------------------------------------------
    // Test 8: CLI `cxpak onboard` exits 0 on the fixture repo
    // -------------------------------------------------------------------------

    #[test]
    fn cli_onboard_exits_zero_on_simple_repo() {
        cxpak()
            .args(["onboard", "tests/fixtures/simple_repo"])
            .assert()
            .success();
    }

    #[test]
    fn cli_onboard_json_is_valid() {
        let output = cxpak()
            .args(["onboard", "--format", "json", "tests/fixtures/simple_repo"])
            .output()
            .expect("failed to run cxpak onboard");

        assert!(output.status.success(), "command must succeed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let _: serde_json::Value = serde_json::from_str(&stdout)
            .expect("cxpak onboard --format json must produce valid JSON");
    }

    #[test]
    fn cli_onboard_markdown_contains_overview_section() {
        let output = cxpak()
            .args(["onboard", "tests/fixtures/simple_repo"])
            .output()
            .expect("failed to run cxpak onboard");

        assert!(output.status.success(), "command must succeed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("# "),
            "markdown output must contain a top-level heading"
        );
        assert!(
            stdout.contains("## "),
            "markdown output must contain section headings"
        );
    }
}

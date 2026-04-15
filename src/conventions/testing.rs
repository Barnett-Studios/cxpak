use crate::conventions::PatternObservation;
use crate::index::CodebaseIndex;
use crate::parser::language::SymbolKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestingConventions {
    pub coverage_by_dir: HashMap<String, f64>,
    pub mock_usage: Option<PatternObservation>,
    pub test_naming: Option<PatternObservation>,
    pub density: Option<PatternObservation>,
    /// Whether the codebase uses inline tests (e.g. `#[cfg(test)]` in Rust,
    /// `def test_` in Python, `describe(`/`it(` in JS/TS, `func Test` in Go).
    pub has_inline_tests: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
}

/// Returns true when content contains inline test patterns for the given language.
fn file_has_inline_tests(content: &str, language: Option<&str>) -> bool {
    match language {
        Some("rust") => content.contains("#[test]") || content.contains("#[cfg(test)]"),
        Some("python") => content.contains("def test_") || content.contains("class Test"),
        Some("javascript" | "typescript") => {
            content.contains("describe(") || content.contains("it(") || content.contains("test(")
        }
        Some("go") => content.contains("func Test"),
        _ => false,
    }
}

/// Extract testing conventions from the codebase index.
pub fn extract_testing(index: &CodebaseIndex) -> TestingConventions {
    // Coverage per directory from test_map
    let mut dir_sources: HashMap<String, usize> = HashMap::new();
    let mut dir_tested: HashMap<String, usize> = HashMap::new();

    for file in &index.files {
        if crate::conventions::errors::is_test_file(&file.relative_path) {
            continue;
        }
        let dir = file
            .relative_path
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        *dir_sources.entry(dir.clone()).or_insert(0) += 1;

        if index.test_map.contains_key(&file.relative_path) {
            *dir_tested.entry(dir).or_insert(0) += 1;
        }
    }

    let mut coverage_by_dir: HashMap<String, f64> = HashMap::new();
    for (dir, total) in &dir_sources {
        let tested = dir_tested.get(dir).copied().unwrap_or(0);
        if *total > 0 {
            coverage_by_dir.insert(dir.clone(), (tested as f64 / *total as f64) * 100.0);
        }
    }

    // Mock detection
    let mock_patterns = [
        "jest.mock",
        "vi.mock",
        "unittest.mock",
        "sinon.stub",
        "@mock",
        "mock!(",
    ];
    let mut mock_files = 0usize;
    let mut test_files = 0usize;

    for file in &index.files {
        if crate::conventions::errors::is_test_file(&file.relative_path) {
            test_files += 1;
            if mock_patterns.iter().any(|p| file.content.contains(p)) {
                mock_files += 1;
            }
        }
    }

    let mock_usage = if test_files > 0 {
        let no_mock = test_files - mock_files;
        PatternObservation::new("mock_usage", "no mocks", no_mock, test_files)
    } else {
        None
    };

    // Test naming pattern detection
    let mut test_name_patterns: HashMap<String, usize> = HashMap::new();
    let mut total_test_fns = 0usize;

    for file in &index.files {
        if !crate::conventions::errors::is_test_file(&file.relative_path) {
            continue;
        }
        if let Some(pr) = &file.parse_result {
            for symbol in &pr.symbols {
                if symbol.kind == SymbolKind::Function && symbol.name.starts_with("test_") {
                    total_test_fns += 1;
                    // Detect pattern: test_{fn}_{scenario}_{expected}
                    let parts: Vec<&str> = symbol.name.splitn(4, '_').collect();
                    let pattern = if parts.len() >= 4 {
                        "test_{fn}_{scenario}_{expected}"
                    } else if parts.len() >= 3 {
                        "test_{fn}_{scenario}"
                    } else {
                        "test_{name}"
                    };
                    *test_name_patterns.entry(pattern.to_string()).or_insert(0) += 1;
                }
            }
        }
    }

    let test_naming = if total_test_fns > 0 {
        // Stable tie detection: if two patterns share the max count,
        // return None rather than picking one non-deterministically.
        let max_count = test_name_patterns.values().copied().max().unwrap_or(0);
        let winners: Vec<_> = test_name_patterns
            .iter()
            .filter(|(_, &v)| v == max_count)
            .collect();
        if winners.len() == 1 {
            let (dominant_pattern, &dominant_count) = winners[0];
            PatternObservation::new(
                "test_naming",
                dominant_pattern,
                dominant_count,
                total_test_fns,
            )
        } else {
            None
        }
    } else {
        None
    };

    // Test density (tests per public function)
    let total_public_fns = index
        .files
        .iter()
        .filter(|f| !crate::conventions::errors::is_test_file(&f.relative_path))
        .filter_map(|f| f.parse_result.as_ref())
        .flat_map(|pr| &pr.symbols)
        .filter(|s| {
            matches!(s.kind, SymbolKind::Function | SymbolKind::Method)
                && s.visibility == crate::parser::language::Visibility::Public
        })
        .count();

    let density = if total_public_fns > 0 && total_test_fns > 0 {
        let ratio = total_test_fns as f64 / total_public_fns as f64;
        let ratio_str = format!("{ratio:.1} tests/public fn");
        // Report as observation if ratio > 1.0
        Some(PatternObservation {
            name: "test_density".into(),
            dominant: ratio_str,
            count: total_test_fns,
            total: total_public_fns,
            percentage: ratio * 100.0,
            strength: if ratio >= 3.0 {
                crate::conventions::PatternStrength::Convention
            } else if ratio >= 1.5 {
                crate::conventions::PatternStrength::Trend
            } else {
                crate::conventions::PatternStrength::Mixed
            },
            exceptions: Vec::new(),
        })
    } else {
        None
    };

    // Inline test detection
    let mut files_with_inline = 0usize;
    let mut total_src_files = 0usize;
    for file in &index.files {
        // Check all files — inline tests can appear in source files (e.g. Rust #[cfg(test)])
        // as well as dedicated test files.
        total_src_files += 1;
        if file_has_inline_tests(&file.content, file.language.as_deref()) {
            files_with_inline += 1;
        }
    }

    let has_inline_tests = if total_src_files > 0 {
        PatternObservation::new(
            "inline_tests",
            "inline tests present",
            files_with_inline,
            total_src_files,
        )
    } else {
        None
    };

    TestingConventions {
        coverage_by_dir,
        mock_usage,
        test_naming,
        density,
        has_inline_tests,
        additional: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::intelligence::test_map::TestFileRef;
    use crate::parser::language::{ParseResult, Symbol, Visibility};
    use crate::scanner::ScannedFile;

    #[test]
    fn test_coverage_per_dir() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let fp1 = dir.path().join("src_api.rs");
        let fp2 = dir.path().join("src_api2.rs");
        std::fs::write(&fp1, "x").unwrap();
        std::fs::write(&fp2, "x").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/api/handler.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 1,
            },
            ScannedFile {
                relative_path: "src/api/auth.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 1,
            },
        ];

        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        // Only one file has a test mapping
        index.test_map.insert(
            "src/api/handler.rs".into(),
            vec![TestFileRef {
                path: "tests/api_test.rs".into(),
                confidence: crate::intelligence::test_map::TestConfidence::NameMatch,
            }],
        );

        let testing = extract_testing(&index);
        let coverage = testing.coverage_by_dir.get("src/api").unwrap();
        assert_eq!(*coverage, 50.0);
    }

    #[test]
    fn test_mock_detection() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let fp1 = dir.path().join("test1.rs");
        let fp2 = dir.path().join("test2.rs");
        std::fs::write(&fp1, "jest.mock('module')").unwrap();
        std::fs::write(&fp2, "fn test_something() {}").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "tests/test_api.js".into(),
                absolute_path: fp1,
                language: Some("javascript".into()),
                size_bytes: 20,
            },
            ScannedFile {
                relative_path: "tests/test_service.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 22,
            },
        ];

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let testing = extract_testing(&index);

        // 1 mock file out of 2 test files = 50% no-mock
        let mock = testing.mock_usage.unwrap();
        assert_eq!(mock.dominant, "no mocks");
        assert_eq!(mock.count, 1); // 1 file without mocks
        assert_eq!(mock.total, 2);
    }

    #[test]
    fn test_test_naming_pattern() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "tests/test_api.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "tests/test_api.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "test_handle_request_returns_ok".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_handle_request_returns_ok()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "test_parse_input_invalid_json".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_parse_input_invalid_json()".into(),
                        body: "{}".into(),
                        start_line: 3,
                        end_line: 3,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let testing = extract_testing(&index);

        assert!(testing.test_naming.is_some());
    }

    #[test]
    fn test_test_naming_two_part_pattern() {
        // test_{fn}_{scenario} — 3 parts total
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "tests/test_api.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "tests/test_api.rs".into(),
            ParseResult {
                symbols: vec![
                    // splitn(4, '_') on "test_parse_ok" gives ["test", "parse", "ok"] → 3 parts
                    Symbol {
                        name: "test_parse_ok".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_parse_ok()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "test_build_err".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_build_err()".into(),
                        body: "{}".into(),
                        start_line: 3,
                        end_line: 3,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let testing = extract_testing(&index);

        let naming = testing.test_naming.unwrap();
        assert_eq!(naming.dominant, "test_{fn}_{scenario}");
        assert_eq!(naming.count, 2);
    }

    #[test]
    fn test_density_calculation_with_public_fns_and_tests() {
        // Populate both source files (with public fns) and test files (with test_ fns)
        // so that the density branch fires.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let src_fp = dir.path().join("src.rs");
        let test_fp = dir.path().join("test.rs");
        std::fs::write(&src_fp, "x").unwrap();
        std::fs::write(&test_fp, "x").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/lib.rs".into(),
                absolute_path: src_fp,
                language: Some("rust".into()),
                size_bytes: 1,
            },
            ScannedFile {
                relative_path: "tests/lib_test.rs".into(),
                absolute_path: test_fp,
                language: Some("rust".into()),
                size_bytes: 1,
            },
        ];

        let mut parse_results = HashMap::new();
        // 2 public functions in source
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "process_a".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn process_a()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "process_b".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn process_b()".into(),
                        body: "{}".into(),
                        start_line: 3,
                        end_line: 3,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );
        // 6 test functions → ratio = 6/2 = 3.0 → Convention strength
        parse_results.insert(
            "tests/lib_test.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "test_a_ok".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_a_ok()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "test_a_err".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_a_err()".into(),
                        body: "{}".into(),
                        start_line: 3,
                        end_line: 3,
                    },
                    Symbol {
                        name: "test_b_ok".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_b_ok()".into(),
                        body: "{}".into(),
                        start_line: 5,
                        end_line: 5,
                    },
                    Symbol {
                        name: "test_b_err".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_b_err()".into(),
                        body: "{}".into(),
                        start_line: 7,
                        end_line: 7,
                    },
                    Symbol {
                        name: "test_b_edge".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_b_edge()".into(),
                        body: "{}".into(),
                        start_line: 9,
                        end_line: 9,
                    },
                    Symbol {
                        name: "test_a_edge".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_a_edge()".into(),
                        body: "{}".into(),
                        start_line: 11,
                        end_line: 11,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let testing = extract_testing(&index);

        let density = testing.density.unwrap();
        assert_eq!(density.count, 6); // 6 test functions
        assert_eq!(density.total, 2); // 2 public functions
        assert_eq!(density.percentage, 300.0); // 6/2 * 100
        assert!(matches!(
            density.strength,
            crate::conventions::PatternStrength::Convention
        ));
    }

    #[test]
    fn test_density_trend_strength() {
        // ratio between 1.5 and 3.0 → Trend
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let src_fp = dir.path().join("src.rs");
        let test_fp = dir.path().join("test.rs");
        std::fs::write(&src_fp, "x").unwrap();
        std::fs::write(&test_fp, "x").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/lib.rs".into(),
                absolute_path: src_fp,
                language: Some("rust".into()),
                size_bytes: 1,
            },
            ScannedFile {
                relative_path: "tests/lib_test.rs".into(),
                absolute_path: test_fp,
                language: Some("rust".into()),
                size_bytes: 1,
            },
        ];

        let mut parse_results = HashMap::new();
        // 2 public functions
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "fn_one".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn fn_one()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "fn_two".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn fn_two()".into(),
                        body: "{}".into(),
                        start_line: 3,
                        end_line: 3,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );
        // 4 test functions → ratio = 4/2 = 2.0 → Trend (1.5 ≤ 2.0 < 3.0)
        parse_results.insert(
            "tests/lib_test.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "test_one_a".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_one_a()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "test_one_b".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_one_b()".into(),
                        body: "{}".into(),
                        start_line: 3,
                        end_line: 3,
                    },
                    Symbol {
                        name: "test_two_a".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_two_a()".into(),
                        body: "{}".into(),
                        start_line: 5,
                        end_line: 5,
                    },
                    Symbol {
                        name: "test_two_b".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_two_b()".into(),
                        body: "{}".into(),
                        start_line: 7,
                        end_line: 7,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let testing = extract_testing(&index);

        let density = testing.density.unwrap();
        assert!(matches!(
            density.strength,
            crate::conventions::PatternStrength::Trend
        ));
    }

    // Bug 4 regression: `has_inline_tests` was not populated before the fix.
    // After the fix, files containing language-appropriate inline test markers
    // (e.g. `#[test]` / `#[cfg(test)]` for Rust, `def test_` for Python,
    // `describe(` for JS/TS, `func Test` for Go) must produce a Some observation.
    #[test]
    fn test_has_inline_tests_populated_for_rust_content() {
        use crate::scanner::ScannedFile;

        let counter = crate::budget::counter::TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        // File with inline tests.
        let fp_with = dir.path().join("lib.rs");
        std::fs::write(
            &fp_with,
            "#[cfg(test)] mod tests { #[test] fn it_works() {} }",
        )
        .unwrap();
        // File without inline tests.
        let fp_without = dir.path().join("main.rs");
        std::fs::write(&fp_without, "fn main() {}").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/lib.rs".into(),
                absolute_path: fp_with,
                language: Some("rust".into()),
                size_bytes: 50,
            },
            ScannedFile {
                relative_path: "src/main.rs".into(),
                absolute_path: fp_without,
                language: Some("rust".into()),
                size_bytes: 12,
            },
        ];

        let mut content_map = std::collections::HashMap::new();
        content_map.insert(
            "src/lib.rs".into(),
            "#[cfg(test)] mod tests { #[test] fn it_works() {} }".into(),
        );
        content_map.insert("src/main.rs".into(), "fn main() {}".into());

        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let testing = extract_testing(&index);

        let obs = testing
            .has_inline_tests
            .expect("has_inline_tests must be Some when inline tests are present");
        // 1 out of 2 files has inline tests.
        assert_eq!(obs.count, 1, "count must be 1 (the file with #[test])");
        assert_eq!(obs.total, 2, "total must be 2 (all files)");
        assert_eq!(obs.percentage, 50.0);
    }
}

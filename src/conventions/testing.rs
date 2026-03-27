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
    pub additional: Vec<PatternObservation>,
}

/// Extract testing conventions from the codebase index.
pub fn extract_testing(index: &CodebaseIndex) -> TestingConventions {
    // Coverage per directory from test_map
    let mut dir_sources: HashMap<String, usize> = HashMap::new();
    let mut dir_tested: HashMap<String, usize> = HashMap::new();

    for file in &index.files {
        if file.relative_path.contains("test") || file.relative_path.starts_with("tests/") {
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
        if file.relative_path.contains("test") || file.relative_path.starts_with("tests/") {
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
        if !(file.relative_path.contains("test") || file.relative_path.starts_with("tests/")) {
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
        let (dominant_pattern, &dominant_count) =
            test_name_patterns.iter().max_by_key(|(_, &v)| v).unwrap();
        PatternObservation::new(
            "test_naming",
            dominant_pattern,
            dominant_count,
            total_test_fns,
        )
    } else {
        None
    };

    // Test density (tests per public function)
    let total_public_fns = index
        .files
        .iter()
        .filter(|f| !f.relative_path.contains("test") && !f.relative_path.starts_with("tests/"))
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

    TestingConventions {
        coverage_by_dir,
        mock_usage,
        test_naming,
        density,
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
}

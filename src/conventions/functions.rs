use crate::conventions::{FileContribution, PatternObservation};
use crate::index::{CodebaseIndex, IndexedFile};
use crate::parser::language::SymbolKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionConventions {
    pub avg_length: Option<f64>,
    pub median_length: Option<f64>,
    pub by_directory: HashMap<String, DirectoryFunctionStats>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryFunctionStats {
    pub avg_length: f64,
    pub median_length: f64,
    pub count: usize,
}

/// Extract function length conventions from the codebase index.
pub fn extract_functions(index: &CodebaseIndex) -> FunctionConventions {
    let mut all_lengths: Vec<usize> = Vec::new();
    let mut dir_lengths: HashMap<String, Vec<usize>> = HashMap::new();
    let mut file_contributions: HashMap<String, FileContribution> = HashMap::new();

    for file in &index.files {
        if file.relative_path.contains("test") || file.relative_path.starts_with("tests/") {
            continue;
        }

        let mut contribution = FileContribution::default();
        let dir = file
            .relative_path
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();

        if let Some(pr) = &file.parse_result {
            for symbol in &pr.symbols {
                if matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
                    let length = symbol.end_line.saturating_sub(symbol.start_line) + 1;
                    all_lengths.push(length);
                    dir_lengths.entry(dir.clone()).or_default().push(length);
                    *contribution.counts.entry("total_lines".into()).or_insert(0) += length;
                    *contribution.counts.entry("fn_count".into()).or_insert(0) += 1;
                }
            }
        }

        file_contributions.insert(file.relative_path.clone(), contribution);
    }

    let avg_length = if all_lengths.is_empty() {
        None
    } else {
        Some(all_lengths.iter().sum::<usize>() as f64 / all_lengths.len() as f64)
    };

    let median_length = if all_lengths.is_empty() {
        None
    } else {
        Some(median(&mut all_lengths))
    };

    let mut by_directory: HashMap<String, DirectoryFunctionStats> = HashMap::new();
    for (dir, mut lengths) in dir_lengths {
        if lengths.is_empty() {
            continue;
        }
        let avg = lengths.iter().sum::<usize>() as f64 / lengths.len() as f64;
        let med = median(&mut lengths);
        by_directory.insert(
            dir,
            DirectoryFunctionStats {
                avg_length: avg,
                median_length: med,
                count: lengths.len(),
            },
        );
    }

    FunctionConventions {
        avg_length,
        median_length,
        by_directory,
        additional: Vec::new(),
        file_contributions,
    }
}

fn median(values: &mut [usize]) -> f64 {
    values.sort_unstable();
    let len = values.len();
    if len == 0 {
        return 0.0;
    }
    if len.is_multiple_of(2) {
        (values[len / 2 - 1] + values[len / 2]) as f64 / 2.0
    } else {
        values[len / 2] as f64
    }
}

pub fn remove_file_contribution(conventions: &mut FunctionConventions, path: &str) {
    conventions.file_contributions.remove(path);
}

pub fn update_file_contribution(_conventions: &mut FunctionConventions, _file: &IndexedFile) {
    // Deferred to orchestrator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, Visibility};
    use crate::scanner::ScannedFile;

    #[test]
    fn test_extract_functions_avg_and_median() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "short_fn".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn short_fn()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 5, // 5 lines
                    },
                    Symbol {
                        name: "long_fn".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn long_fn()".into(),
                        body: "{}".into(),
                        start_line: 10,
                        end_line: 39, // 30 lines
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let fns = extract_functions(&index);

        assert!(fns.avg_length.is_some());
        let avg = fns.avg_length.unwrap();
        assert!((avg - 17.5).abs() < 0.1); // (5 + 30) / 2

        assert!(fns.median_length.is_some());
    }

    #[test]
    fn test_extract_functions_by_directory() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("a.rs");
        let fp2 = dir.path().join("b.rs");
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
                relative_path: "src/services/user.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 1,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/api/handler.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "handle".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn handle()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 10,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/services/user.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "process".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn process()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 30,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let fns = extract_functions(&index);

        assert!(fns.by_directory.contains_key("src/api"));
        assert!(fns.by_directory.contains_key("src/services"));
        assert_eq!(fns.by_directory["src/api"].avg_length, 10.0);
        assert_eq!(fns.by_directory["src/services"].avg_length, 30.0);
    }

    #[test]
    fn test_median_calculation() {
        assert_eq!(median(&mut [1, 3, 5]), 3.0);
        assert_eq!(median(&mut [1, 2, 3, 4]), 2.5);
        assert_eq!(median(&mut [10]), 10.0);
    }

    #[test]
    fn test_median_empty_returns_zero() {
        assert_eq!(median(&mut []), 0.0);
    }

    #[test]
    fn test_median_even_count() {
        // [2, 4] → (2 + 4) / 2 = 3.0
        assert_eq!(median(&mut [4, 2]), 3.0);
    }

    #[test]
    fn test_extract_functions_no_functions_returns_none() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        // No parse results → no functions
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let fns = extract_functions(&index);

        assert!(fns.avg_length.is_none());
        assert!(fns.median_length.is_none());
        assert!(fns.by_directory.is_empty());
    }

    #[test]
    fn test_remove_file_contribution_removes_entry() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "my_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn my_fn()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 5,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let mut fns = extract_functions(&index);

        assert!(fns.file_contributions.contains_key("src/lib.rs"));
        remove_file_contribution(&mut fns, "src/lib.rs");
        assert!(!fns.file_contributions.contains_key("src/lib.rs"));
    }

    #[test]
    fn test_update_file_contribution_is_noop() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let mut fns = extract_functions(&index);
        let before_count = fns.file_contributions.len();

        // update_file_contribution is a noop — state must be unchanged
        let file = &index.files[0];
        update_file_contribution(&mut fns, file);
        assert_eq!(fns.file_contributions.len(), before_count);
    }
}

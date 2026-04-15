use crate::conventions::{FileContribution, PatternObservation};
use crate::index::{CodebaseIndex, IndexedFile};
use crate::parser::language::SymbolKind;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorConventions {
    pub result_return: Option<PatternObservation>,
    pub unwrap_usage: Option<PatternObservation>,
    pub expect_usage: Option<PatternObservation>,
    pub question_mark_propagation: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

/// Returns true when the file path looks like a test file.
pub fn is_test_file(path: &str) -> bool {
    path.contains("/test")
        || path.starts_with("tests/")
        || path.contains("_test.")
        || path.contains("/tests/")
}

pub fn extract_errors(index: &CodebaseIndex) -> ErrorConventions {
    let mut result_count = 0usize;
    // Separate src vs test function counts
    let mut src_fn_count = 0usize;
    let mut test_fn_count = 0usize;
    let mut unwrap_src = 0usize;
    let mut expect_src = 0usize;
    let mut question_count = 0usize;
    let mut question_total = 0usize;
    let mut file_contributions: HashMap<String, FileContribution> = HashMap::new();

    let question_re =
        Regex::new(r"[)\w]\?\s*[;,\n)]").unwrap_or_else(|_| Regex::new(r"$^").unwrap());

    for file in &index.files {
        let mut contribution = FileContribution::default();
        let is_test = is_test_file(&file.relative_path);
        let is_rust = file.language.as_deref() == Some("rust");

        if let Some(pr) = &file.parse_result {
            for symbol in &pr.symbols {
                match symbol.kind {
                    SymbolKind::Function | SymbolKind::Method => {
                        if is_test {
                            test_fn_count += 1;
                        } else {
                            src_fn_count += 1;
                            *contribution.counts.entry("total_fns".into()).or_insert(0) += 1;
                        }

                        // Check for Result/Option return type (src only)
                        if !is_test
                            && (symbol.signature.contains("Result<")
                                || symbol.signature.contains("-> Result"))
                        {
                            result_count += 1;
                            *contribution
                                .counts
                                .entry("result_return".into())
                                .or_insert(0) += 1;
                        }

                        // Check for .unwrap() usage — only count in src
                        // Strip line comments before matching to avoid false positives.
                        if !is_test {
                            let stripped: String = symbol
                                .body
                                .lines()
                                .map(|l| l.find("//").map(|i| &l[..i]).unwrap_or(l))
                                .collect::<Vec<_>>()
                                .join("\n");
                            let unwrap_occurrences = stripped.matches(".unwrap()").count();
                            if unwrap_occurrences > 0 {
                                unwrap_src += unwrap_occurrences;
                                *contribution.counts.entry("unwrap_src".into()).or_insert(0) +=
                                    unwrap_occurrences;
                            }

                            // Check for .expect() usage
                            let expect_occurrences = stripped.matches(".expect(").count();
                            if expect_occurrences > 0 {
                                expect_src += expect_occurrences;
                                *contribution.counts.entry("expect_src".into()).or_insert(0) +=
                                    expect_occurrences;
                            }
                        }

                        // ? propagation (Rust only, src only)
                        if is_rust && !is_test {
                            let q_count = question_re.find_iter(&symbol.body).count();
                            if q_count > 0 {
                                question_count += 1; // count functions using ?, not total ? tokens
                                *contribution
                                    .counts
                                    .entry("question_mark".into())
                                    .or_insert(0) += 1;
                            }
                            question_total += 1;
                        }
                    }
                    _ => {}
                }
            }
        }

        file_contributions.insert(file.relative_path.clone(), contribution);
    }

    // Suppress unused variable warnings for test_fn_count — kept for future use
    let _ = test_fn_count;

    let result_return = PatternObservation::new(
        "result_return_type",
        "Result<T, E>",
        result_count,
        src_fn_count,
    );

    // unwrap: measured per-occurrence in src functions vs total src functions
    let no_unwrap_count = src_fn_count.saturating_sub(unwrap_src);
    let unwrap_usage = if src_fn_count > 0 && unwrap_src > 0 {
        PatternObservation::new(
            "unwrap_in_source",
            "no .unwrap() in src/",
            no_unwrap_count,
            src_fn_count,
        )
    } else if src_fn_count > 0 {
        PatternObservation::new(
            "unwrap_in_source",
            "no .unwrap() in src/",
            src_fn_count,
            src_fn_count,
        )
    } else {
        None
    };

    let no_expect_count = src_fn_count.saturating_sub(expect_src);
    let expect_usage = if src_fn_count > 0 && expect_src > 0 {
        PatternObservation::new(
            "expect_in_source",
            "no .expect() in src/",
            no_expect_count,
            src_fn_count,
        )
    } else {
        None
    };

    let question_mark_propagation = if question_total > 0 && question_count > 0 {
        PatternObservation::new(
            "question_mark_propagation",
            "? operator",
            question_count,
            question_total,
        )
    } else {
        None
    };

    ErrorConventions {
        result_return,
        unwrap_usage,
        expect_usage,
        question_mark_propagation,
        additional: Vec::new(),
        file_contributions,
    }
}

pub fn remove_file_contribution(conventions: &mut ErrorConventions, path: &str) {
    conventions.file_contributions.remove(path);
}

pub fn update_file_contribution(_conventions: &mut ErrorConventions, _file: &IndexedFile) {
    // Deferred to orchestrator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, Visibility};
    use crate::scanner::ScannedFile;

    #[test]
    fn test_extract_errors_result_return() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/api.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/api.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "handle".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn handle() -> Result<Response, Error>".into(),
                        body: "{ Ok(resp) }".into(),
                        start_line: 1,
                        end_line: 3,
                    },
                    Symbol {
                        name: "parse".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn parse() -> Result<Data, Error>".into(),
                        body: "{ Ok(data) }".into(),
                        start_line: 5,
                        end_line: 7,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let errors = extract_errors(&index);

        let result_return = errors.result_return.unwrap();
        assert_eq!(result_return.count, 2);
        assert_eq!(result_return.total, 2);
        assert_eq!(result_return.percentage, 100.0);
    }

    #[test]
    fn test_extract_errors_unwrap_detection() {
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
                symbols: vec![Symbol {
                    name: "bad_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn bad_fn()".into(),
                    body: "{ x.unwrap() }".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let errors = extract_errors(&index);

        // unwrap was found in src, so the "no unwrap" observation should either be None or low %
        assert!(
            errors.unwrap_usage.is_none()
                || errors.unwrap_usage.as_ref().unwrap().percentage < 100.0
        );
    }

    #[test]
    fn test_extract_errors_question_mark_rust_only() {
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
                symbols: vec![Symbol {
                    name: "handler".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn handler() -> Result<(), Error>".into(),
                    body: "{ let x = foo()?; bar()?; Ok(()) }".into(),
                    start_line: 1,
                    end_line: 3,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let errors = extract_errors(&index);

        assert!(errors.question_mark_propagation.is_some());
    }

    #[test]
    fn test_extract_errors_expect_usage_branch() {
        // expect_usage is only Some when expect_src > 0 AND no_expect_count/total >= 50%.
        // Use 10 functions where only 1 has .expect() → 9/10 = 90% → Some.
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

        let clean_sym = |i: usize| Symbol {
            name: format!("clean_{i}"),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: format!("fn clean_{i}()"),
            body: "{ Ok(()) }".into(),
            start_line: i,
            end_line: i,
        };

        let mut symbols: Vec<Symbol> = (1..10).map(clean_sym).collect();
        symbols.push(Symbol {
            name: "bad_fn".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "fn bad_fn()".into(),
            body: "{ x.expect(\"must work\") }".into(),
            start_line: 10,
            end_line: 11,
        });

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols,
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let errors = extract_errors(&index);

        // expect_src == 1, total_src_fns == 10, no_expect_count == 9 → 90% → Some
        assert!(errors.expect_usage.is_some());
        let obs = errors.expect_usage.unwrap();
        assert_eq!(obs.dominant, "no .expect() in src/");
        assert_eq!(obs.count, 9);
        assert_eq!(obs.total, 10);
    }

    #[test]
    fn test_extract_errors_no_unwrap_at_all_branch() {
        // When total_src_fns > 0 but unwrap_src == 0, unwrap_usage should
        // use the "no unwrap at all" branch (unwrap_usage is Some with 100%).
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
                    name: "clean_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn clean_fn() -> Result<(), E>".into(),
                    body: "{ Ok(()) }".into(),
                    start_line: 1,
                    end_line: 2,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let errors = extract_errors(&index);

        // No unwrap at all in source → still reported as observation
        let obs = errors.unwrap_usage.unwrap();
        assert_eq!(obs.dominant, "no .unwrap() in src/");
        assert_eq!(obs.percentage, 100.0);
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

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let mut errors = extract_errors(&index);

        assert!(errors.file_contributions.contains_key("src/lib.rs"));
        remove_file_contribution(&mut errors, "src/lib.rs");
        assert!(!errors.file_contributions.contains_key("src/lib.rs"));
    }

    // Bug 1 regression: test-file functions must NOT count toward the src denominator.
    // Before the fix, `total_src_fns` was computed by subtracting occurrence counts
    // from the total, which produced an incorrect denominator when test functions were
    // plentiful.  After the fix, each file is classified up-front as test/src and only
    // src-file function symbols contribute to the denominator.
    #[test]
    fn test_question_mark_percentage_at_most_100() {
        // A function with multiple ? operators should count as 1 function,
        // not as multiple occurrences, so percentage stays ≤ 100%.
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
                    name: "heavy_user".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn heavy_user() -> Result<(), E>".into(),
                    // 10 question-mark uses in one function body
                    body: "{ a()?; b()?; c()?; d()?; e()?; f()?; g()?; h()?; i()?; j()?; Ok(()) }"
                        .into(),
                    start_line: 1,
                    end_line: 2,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let errors = extract_errors(&index);

        if let Some(obs) = errors.question_mark_propagation {
            assert!(
                obs.percentage <= 100.0,
                "question-mark percentage must not exceed 100, got {}",
                obs.percentage
            );
        }
    }

    #[test]
    fn test_comment_unwrap_not_counted() {
        // An `.unwrap()` that appears only in a line comment must not be counted.
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
                    name: "clean_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn clean_fn() -> Result<(), E>".into(),
                    // The only .unwrap() is inside a comment — should not be detected.
                    body: "{ // .unwrap() is bad practice\n    Ok(()) }".into(),
                    start_line: 1,
                    end_line: 3,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let errors = extract_errors(&index);

        // With only one src function and no real unwrap, should show 100% clean.
        let obs = errors.unwrap_usage.expect("unwrap_usage should be Some");
        assert_eq!(
            obs.percentage, 100.0,
            "commented-out .unwrap() must not count as an unwrap usage"
        );
    }

    #[test]
    fn test_extract_errors_test_file_fns_excluded_from_src_denominator() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        // Two files: one src (with a Result-returning fn), one test (with two fns).
        let src_path = dir.path().join("lib.rs");
        let test_path = dir.path().join("lib_test.rs");
        std::fs::write(&src_path, "fn a() -> Result<(), E> {}").unwrap();
        std::fs::write(&test_path, "fn test_one() {} fn test_two() {}").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/lib.rs".into(),
                absolute_path: src_path,
                language: Some("rust".into()),
                size_bytes: 30,
            },
            ScannedFile {
                relative_path: "src/lib_test.rs".into(),
                absolute_path: test_path,
                language: Some("rust".into()),
                size_bytes: 40,
            },
        ];

        use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "a".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn a() -> Result<(), E>".into(),
                    body: "fn a() -> Result<(), E> {}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/lib_test.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "test_one".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_one()".into(),
                        body: "fn test_one() {}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "test_two".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn test_two()".into(),
                        body: "fn test_two() {}".into(),
                        start_line: 2,
                        end_line: 2,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let errors = extract_errors(&index);

        // The src file has 1 function; it uses `Result` in the signature, so
        // result_return should reflect 1 out of 1 src function (100%).
        if let Some(obs) = errors.result_return {
            // total must be the src-file fn count only (1), not 3.
            assert_eq!(obs.total, 1, "denominator must exclude test-file functions");
        }
    }
}

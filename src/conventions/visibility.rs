use crate::conventions::{FileContribution, PatternObservation};
use crate::index::{CodebaseIndex, IndexedFile};
use crate::parser::language::Visibility;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisibilityConventions {
    pub public_ratio: Option<PatternObservation>,
    pub doc_comment_coverage: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

/// Extract visibility conventions from the codebase index.
pub fn extract_visibility(index: &CodebaseIndex) -> VisibilityConventions {
    let mut public_count = 0usize;
    let mut private_count = 0usize;
    let mut doc_comment_count = 0usize;
    let mut public_with_body = 0usize;
    let mut file_contributions: HashMap<String, FileContribution> = HashMap::new();

    for file in &index.files {
        if file.relative_path.contains("test") || file.relative_path.starts_with("tests/") {
            continue;
        }

        let mut contribution = FileContribution::default();

        if let Some(pr) = &file.parse_result {
            for symbol in &pr.symbols {
                match symbol.visibility {
                    Visibility::Public => {
                        public_count += 1;
                        *contribution.counts.entry("public".into()).or_insert(0) += 1;

                        // Check for doc comments
                        if has_doc_comment(&symbol.body, file.language.as_deref()) {
                            doc_comment_count += 1;
                            *contribution.counts.entry("doc_comment".into()).or_insert(0) += 1;
                        }
                        public_with_body += 1;
                    }
                    Visibility::Private => {
                        private_count += 1;
                        *contribution.counts.entry("private".into()).or_insert(0) += 1;
                    }
                }
            }
        }

        file_contributions.insert(file.relative_path.clone(), contribution);
    }

    let total = public_count + private_count;

    // Report as "default to private" if private > public
    let public_ratio = if private_count > public_count {
        PatternObservation::new("visibility_default", "private", private_count, total)
    } else if public_count > 0 {
        PatternObservation::new("visibility_default", "public", public_count, total)
    } else {
        None
    };

    let doc_comment_coverage = if public_with_body > 0 {
        PatternObservation::new(
            "doc_comment_coverage",
            "documented public APIs",
            doc_comment_count,
            public_with_body,
        )
    } else {
        None
    };

    VisibilityConventions {
        public_ratio,
        doc_comment_coverage,
        additional: Vec::new(),
        file_contributions,
    }
}

fn has_doc_comment(body: &str, language: Option<&str>) -> bool {
    let trimmed = body.trim();
    match language {
        Some("rust") => trimmed.starts_with("///") || trimmed.starts_with("//!"),
        Some("python") => trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''"),
        Some("javascript" | "typescript" | "java" | "kotlin" | "c" | "cpp" | "csharp") => {
            trimmed.starts_with("/**") || trimmed.starts_with("///")
        }
        Some("ruby") => trimmed.starts_with('#'),
        _ => {
            trimmed.starts_with("///")
                || trimmed.starts_with("/**")
                || trimmed.starts_with("\"\"\"")
        }
    }
}

pub fn remove_file_contribution(conventions: &mut VisibilityConventions, path: &str) {
    conventions.file_contributions.remove(path);
}

pub fn update_file_contribution(_conventions: &mut VisibilityConventions, _file: &IndexedFile) {
    // Deferred to orchestrator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind};
    use crate::scanner::ScannedFile;

    #[test]
    fn test_extract_visibility_mostly_private() {
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
                        name: "public_fn".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn public_fn()".into(),
                        body: "/// Documented\n{}".into(),
                        start_line: 1,
                        end_line: 2,
                    },
                    Symbol {
                        name: "private_fn".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn private_fn()".into(),
                        body: "{}".into(),
                        start_line: 3,
                        end_line: 3,
                    },
                    Symbol {
                        name: "another_private".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn another_private()".into(),
                        body: "{}".into(),
                        start_line: 4,
                        end_line: 4,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let vis = extract_visibility(&index);

        let ratio = vis.public_ratio.unwrap();
        assert_eq!(ratio.dominant, "private");
        assert!(matches!(
            ratio.strength,
            crate::conventions::PatternStrength::Mixed
        ));
    }

    #[test]
    fn test_doc_comment_detection() {
        assert!(has_doc_comment("/// A doc comment\n{}", Some("rust")));
        assert!(!has_doc_comment("{}", Some("rust")));
        assert!(has_doc_comment("/** JSDoc */\n{}", Some("javascript")));
    }

    #[test]
    fn test_extract_visibility_mostly_public() {
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
                symbols: vec![
                    Symbol {
                        name: "pub_a".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn pub_a()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "pub_b".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn pub_b()".into(),
                        body: "{}".into(),
                        start_line: 2,
                        end_line: 2,
                    },
                    Symbol {
                        name: "priv_a".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn priv_a()".into(),
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
        let vis = extract_visibility(&index);

        let ratio = vis.public_ratio.unwrap();
        assert_eq!(ratio.dominant, "public");
        assert_eq!(ratio.count, 2);
        assert_eq!(ratio.total, 3);
    }

    #[test]
    fn test_extract_visibility_doc_comment_coverage() {
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
                symbols: vec![
                    Symbol {
                        name: "documented".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn documented()".into(),
                        // body starts with doc comment → detected
                        body: "/// Does the thing.\n{ }".into(),
                        start_line: 1,
                        end_line: 3,
                    },
                    Symbol {
                        name: "undocumented".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn undocumented()".into(),
                        body: "{ }".into(),
                        start_line: 5,
                        end_line: 6,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let vis = extract_visibility(&index);

        let doc = vis.doc_comment_coverage.unwrap();
        assert_eq!(doc.count, 1);
        assert_eq!(doc.total, 2);
        assert_eq!(doc.percentage, 50.0);
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
        let mut vis = extract_visibility(&index);

        assert!(vis.file_contributions.contains_key("src/lib.rs"));
        remove_file_contribution(&mut vis, "src/lib.rs");
        assert!(!vis.file_contributions.contains_key("src/lib.rs"));
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
        let mut vis = extract_visibility(&index);
        let before_len = vis.file_contributions.len();

        let file = &index.files[0];
        update_file_contribution(&mut vis, file);

        assert_eq!(vis.file_contributions.len(), before_len);
    }
}

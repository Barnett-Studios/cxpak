use crate::conventions::{FileContribution, PatternObservation};
use crate::index::{CodebaseIndex, IndexedFile};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportConventions {
    pub style: Option<PatternObservation>,
    pub grouping: Option<PatternObservation>,
    pub re_exports: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

pub fn extract_imports(index: &CodebaseIndex) -> ImportConventions {
    let mut absolute_count = 0usize;
    let mut relative_count = 0usize;
    // Grouping: count files using block-style (`use a::{b, c}`) vs single-line
    let mut block_files = 0usize;
    let mut single_files = 0usize;
    // Re-exports: count `pub use` occurrences vs all `use` occurrences
    let mut pub_use_count = 0usize;
    let mut all_use_count = 0usize;
    let mut file_contributions: HashMap<String, FileContribution> = HashMap::new();

    for file in &index.files {
        let mut contribution = FileContribution::default();

        if let Some(pr) = &file.parse_result {
            for import in &pr.imports {
                if import.source.starts_with("./") || import.source.starts_with("../") {
                    relative_count += 1;
                    *contribution.counts.entry("relative".into()).or_insert(0) += 1;
                } else {
                    absolute_count += 1;
                    *contribution.counts.entry("absolute".into()).or_insert(0) += 1;
                }
            }
        }

        // Grouping detection: scan raw file content for block vs single use statements.
        // Block: `use crate::{a, b}` or `use std::io::{Read, Write}` — multi-name groups.
        // Single: plain `use module::name;` on separate lines.
        if !file.content.is_empty() {
            let has_block = file.content.contains("use ") && file.content.contains(":{");
            let use_line_count = file
                .content
                .lines()
                .filter(|l| {
                    let trimmed = l.trim_start();
                    trimmed.starts_with("use ") || trimmed.starts_with("pub use ")
                })
                .count();

            if use_line_count > 0 {
                if has_block {
                    block_files += 1;
                    *contribution
                        .counts
                        .entry("block_imports".into())
                        .or_insert(0) += 1;
                } else {
                    single_files += 1;
                    *contribution
                        .counts
                        .entry("single_imports".into())
                        .or_insert(0) += 1;
                }

                // Re-exports: count `pub use` lines in this file
                let pub_use_lines = file
                    .content
                    .lines()
                    .filter(|l| {
                        let trimmed = l.trim_start();
                        trimmed.starts_with("pub use ")
                    })
                    .count();
                pub_use_count += pub_use_lines;
                all_use_count += use_line_count;
            }
        }

        file_contributions.insert(file.relative_path.clone(), contribution);
    }

    let total = absolute_count + relative_count;
    let style = if absolute_count >= relative_count {
        PatternObservation::new("import_style", "absolute", absolute_count, total)
    } else {
        PatternObservation::new("import_style", "relative", relative_count, total)
    };

    // Grouping observation: dominant style between "block" and "single"
    let grouping_total = block_files + single_files;
    let grouping = if block_files >= single_files {
        PatternObservation::new("import_grouping", "block", block_files, grouping_total)
    } else {
        PatternObservation::new("import_grouping", "single", single_files, grouping_total)
    };

    // Re-exports observation: ratio of `pub use` to all `use` statements
    let re_exports = if all_use_count > 0 {
        PatternObservation::new("re_exports", "pub use", pub_use_count, all_use_count)
    } else {
        None
    };

    ImportConventions {
        style,
        grouping,
        re_exports,
        additional: Vec::new(),
        file_contributions,
    }
}

pub fn remove_file_contribution(conventions: &mut ImportConventions, path: &str) {
    conventions.file_contributions.remove(path);
}

pub fn update_file_contribution(_conventions: &mut ImportConventions, _file: &IndexedFile) {
    // Deferred to orchestrator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{Import, ParseResult};
    use crate::scanner::ScannedFile;

    #[test]
    fn test_extract_imports_all_absolute() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "use std::io;").unwrap();

        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 12,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "test.rs".into(),
            ParseResult {
                symbols: vec![],
                imports: vec![
                    Import {
                        source: "std::io".into(),
                        names: vec!["io".into()],
                    },
                    Import {
                        source: "crate::index".into(),
                        names: vec!["CodebaseIndex".into()],
                    },
                ],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let imports = extract_imports(&index);

        let style = imports.style.unwrap();
        assert_eq!(style.dominant, "absolute");
        assert_eq!(style.percentage, 100.0);
    }

    #[test]
    fn test_extract_imports_mixed() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.js");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "test.js".into(),
            absolute_path: fp,
            language: Some("javascript".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "test.js".into(),
            ParseResult {
                symbols: vec![],
                imports: vec![
                    Import {
                        source: "react".into(),
                        names: vec!["React".into()],
                    },
                    Import {
                        source: "./utils".into(),
                        names: vec!["helper".into()],
                    },
                    Import {
                        source: "../common".into(),
                        names: vec!["shared".into()],
                    },
                ],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let imports = extract_imports(&index);

        let style = imports.style.unwrap();
        assert_eq!(style.dominant, "relative");
        assert!(matches!(
            style.strength,
            crate::conventions::PatternStrength::Mixed
        ));
    }

    #[test]
    fn test_extract_imports_none() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "fn main() {}").unwrap();

        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 12,
        }];

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let imports = extract_imports(&index);
        assert!(imports.style.is_none());
    }

    #[test]
    fn test_remove_file_contribution_removes_entry() {
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

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let mut imports = extract_imports(&index);

        assert!(imports.file_contributions.contains_key("src/lib.rs"));
        remove_file_contribution(&mut imports, "src/lib.rs");
        assert!(!imports.file_contributions.contains_key("src/lib.rs"));
    }

    #[test]
    fn test_update_file_contribution_is_noop() {
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

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let mut imports = extract_imports(&index);
        let before_len = imports.file_contributions.len();

        let file = &index.files[0];
        update_file_contribution(&mut imports, file);

        assert_eq!(imports.file_contributions.len(), before_len);
    }

    // Bug 5 regression: `grouping` and `re_exports` were always None before the fix.
    // After the fix, files containing block-style imports (`use foo::{A, B}`) must
    // produce a Some grouping observation with dominant "block", and files containing
    // `pub use` lines must produce a Some re_exports observation.
    #[test]
    fn test_extract_imports_grouping_block_style_detected() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let content = "use std::{io, fs};";
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }];

        let mut content_map = HashMap::new();
        content_map.insert("src/lib.rs".into(), content.into());
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let imports = extract_imports(&index);

        let grouping = imports
            .grouping
            .expect("grouping must be Some when block imports are present");
        assert_eq!(
            grouping.dominant, "block",
            "block-style import must be detected as dominant"
        );
    }

    #[test]
    fn test_extract_imports_re_exports_detected() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let content = "pub use std::io;\nuse std::fs;";
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }];

        let mut content_map = HashMap::new();
        content_map.insert("src/lib.rs".into(), content.into());
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let imports = extract_imports(&index);

        let re_exports = imports
            .re_exports
            .expect("re_exports must be Some when pub use lines are present");
        // 1 out of 2 use-lines is `pub use`
        assert_eq!(re_exports.count, 1);
        assert_eq!(re_exports.total, 2);
        assert_eq!(re_exports.percentage, 50.0);
    }

    #[test]
    fn test_extract_imports_grouping_single_style_detected() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let content = "use std::io;\nuse std::fs;";
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }];

        let mut content_map = HashMap::new();
        content_map.insert("src/lib.rs".into(), content.into());
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let imports = extract_imports(&index);

        let grouping = imports
            .grouping
            .expect("grouping must be Some for files with use statements");
        assert_eq!(
            grouping.dominant, "single",
            "non-block imports must produce 'single' grouping"
        );
    }

    #[test]
    fn test_extract_imports_grouping_none_when_no_use_statements() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let content = "fn main() {}";
        let fp = dir.path().join("main.rs");
        std::fs::write(&fp, content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/main.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }];

        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".into(), content.into());
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let imports = extract_imports(&index);

        // No use statements at all → grouping and re_exports must be None.
        assert!(
            imports.grouping.is_none() || imports.grouping.as_ref().map(|g| g.total) == Some(0),
            "grouping must be None or zero-total when no use statements exist"
        );
        assert!(
            imports.re_exports.is_none(),
            "re_exports must be None when no use statements exist"
        );
    }

    #[test]
    fn test_extract_imports_grouping_none_with_zero_total() {
        // When there are no `use` lines (grouping_total == 0), grouping must be None.
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let imports = extract_imports(&index);
        assert!(
            imports.grouping.is_none(),
            "grouping must be None when no files exist"
        );
        assert!(
            imports.re_exports.is_none(),
            "re_exports must be None when no files exist"
        );
    }

    #[test]
    fn test_extract_imports_grouping_none_with_zero_total_check() {
        // PatternObservation::new with count=0 and total=0 should return None.
        // If grouping_total == 0 (no files with use statements), grouping must be None.
        assert!(
            PatternObservation::new("grouping", "block", 0usize, 0usize).is_none(),
            "PatternObservation with 0/0 must be None"
        );
    }

    #[test]
    fn test_extract_imports_file_contributions_track_absolute_imports() {
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
                symbols: vec![],
                imports: vec![
                    Import {
                        source: "std::io".into(),
                        names: vec!["io".into()],
                    },
                    Import {
                        source: "crate::utils".into(),
                        names: vec!["helper".into()],
                    },
                ],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let imports = extract_imports(&index);

        let contrib = &imports.file_contributions["src/lib.rs"];
        assert_eq!(contrib.counts.get("absolute").copied().unwrap_or(0), 2);
    }
}

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

        file_contributions.insert(file.relative_path.clone(), contribution);
    }

    let total = absolute_count + relative_count;
    let style = if absolute_count >= relative_count {
        PatternObservation::new("import_style", "absolute", absolute_count, total)
    } else {
        PatternObservation::new("import_style", "relative", relative_count, total)
    };

    ImportConventions {
        style,
        grouping: None,
        re_exports: None,
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
}

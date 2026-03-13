pub mod graph;
pub mod ranking;
pub mod symbols;

use crate::budget::counter::TokenCounter;
use crate::parser::language::{Import, ParseResult, Symbol, Visibility};
use crate::scanner::ScannedFile;
use std::collections::HashMap;

#[derive(Debug)]
pub struct CodebaseIndex {
    pub files: Vec<IndexedFile>,
    pub language_stats: HashMap<String, LanguageStats>,
    pub total_files: usize,
    pub total_bytes: u64,
    pub total_tokens: usize,
}

#[derive(Debug)]
pub struct IndexedFile {
    pub relative_path: String,
    pub language: Option<String>,
    pub size_bytes: u64,
    pub token_count: usize,
    pub parse_result: Option<ParseResult>,
    pub content: String,
}

#[derive(Debug)]
pub struct LanguageStats {
    pub file_count: usize,
    pub total_bytes: u64,
    pub total_tokens: usize,
}

impl CodebaseIndex {
    pub fn build(
        files: Vec<ScannedFile>,
        parse_results: HashMap<String, ParseResult>,
        counter: &TokenCounter,
    ) -> Self {
        let mut language_stats: HashMap<String, LanguageStats> = HashMap::new();
        let mut indexed_files = Vec::new();
        let mut total_tokens = 0usize;
        let mut total_bytes = 0u64;

        for file in &files {
            let content = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
            let token_count = counter.count_or_zero(&content);
            total_tokens += token_count;
            total_bytes += file.size_bytes;

            if let Some(lang) = &file.language {
                let stats = language_stats.entry(lang.clone()).or_insert(LanguageStats {
                    file_count: 0,
                    total_bytes: 0,
                    total_tokens: 0,
                });
                stats.file_count += 1;
                stats.total_bytes += file.size_bytes;
                stats.total_tokens += token_count;
            }

            let parse_result = parse_results.get(&file.relative_path).cloned();
            indexed_files.push(IndexedFile {
                relative_path: file.relative_path.clone(),
                language: file.language.clone(),
                size_bytes: file.size_bytes,
                token_count,
                parse_result,
                content,
            });
        }

        Self {
            total_files: indexed_files.len(),
            total_bytes,
            total_tokens,
            files: indexed_files,
            language_stats,
        }
    }

    pub fn all_public_symbols(&self) -> Vec<(&str, &Symbol)> {
        self.files
            .iter()
            .filter_map(|f| {
                f.parse_result.as_ref().map(|pr| {
                    pr.symbols
                        .iter()
                        .filter(|s| s.visibility == Visibility::Public)
                        .map(move |s| (f.relative_path.as_str(), s))
                })
            })
            .flatten()
            .collect()
    }

    pub fn all_imports(&self) -> Vec<(&str, &Import)> {
        self.files
            .iter()
            .filter_map(|f| {
                f.parse_result.as_ref().map(|pr| {
                    pr.imports
                        .iter()
                        .map(move |i| (f.relative_path.as_str(), i))
                })
            })
            .flatten()
            .collect()
    }

    /// Find all symbols whose name matches `target` (case-insensitive).
    ///
    /// Returns `(relative_path, symbol)` pairs across all indexed files.
    pub fn find_symbol<'a>(&'a self, target: &str) -> Vec<(&'a str, &'a Symbol)> {
        let target_lower = target.to_lowercase();
        self.files
            .iter()
            .filter_map(|f| {
                let tl = &target_lower;
                f.parse_result.as_ref().map(|pr| {
                    pr.symbols
                        .iter()
                        .filter(move |s| s.name.to_lowercase() == *tl)
                        .map(move |s| (f.relative_path.as_str(), s))
                })
            })
            .flatten()
            .collect()
    }

    /// Find all files whose content contains `target` as a substring (case-insensitive).
    ///
    /// Returns the relative paths of matching files.
    pub fn find_content_matches<'a>(&'a self, target: &str) -> Vec<&'a str> {
        let target_lower = target.to_lowercase();
        self.files
            .iter()
            .filter(|f| f.content.to_lowercase().contains(&target_lower))
            .map(|f| f.relative_path.as_str())
            .collect()
    }

    /// Build a `CodebaseIndex` using pre-read file content instead of reading from disk.
    ///
    /// `content` maps `relative_path` -> file contents.  When an entry is present the
    /// provided string is used directly; missing entries fall back to a disk read so
    /// that callers are not required to pre-read every file.
    pub fn build_with_content(
        files: Vec<ScannedFile>,
        parse_results: HashMap<String, ParseResult>,
        counter: &TokenCounter,
        content: HashMap<String, String>,
    ) -> Self {
        let mut language_stats: HashMap<String, LanguageStats> = HashMap::new();
        let mut indexed_files = Vec::new();
        let mut total_tokens = 0usize;
        let mut total_bytes = 0u64;

        for file in &files {
            let file_content = content
                .get(&file.relative_path)
                .cloned()
                .unwrap_or_else(|| {
                    std::fs::read_to_string(&file.absolute_path).unwrap_or_default()
                });
            let token_count = counter.count_or_zero(&file_content);
            total_tokens += token_count;
            total_bytes += file.size_bytes;

            if let Some(lang) = &file.language {
                let stats = language_stats.entry(lang.clone()).or_insert(LanguageStats {
                    file_count: 0,
                    total_bytes: 0,
                    total_tokens: 0,
                });
                stats.file_count += 1;
                stats.total_bytes += file.size_bytes;
                stats.total_tokens += token_count;
            }

            let parse_result = parse_results.get(&file.relative_path).cloned();
            indexed_files.push(IndexedFile {
                relative_path: file.relative_path.clone(),
                language: file.language.clone(),
                size_bytes: file.size_bytes,
                token_count,
                parse_result,
                content: file_content,
            });
        }

        Self {
            total_files: indexed_files.len(),
            total_bytes,
            total_tokens,
            files: indexed_files,
            language_stats,
        }
    }

    /// Insert or update a single file in the index.
    ///
    /// If a file with the same `relative_path` already exists, it is replaced.
    /// Language stats and totals are recomputed.
    pub fn upsert_file(
        &mut self,
        relative_path: &str,
        language: Option<&str>,
        content: &str,
        parse_result: Option<ParseResult>,
        counter: &TokenCounter,
    ) {
        // Remove old entry if it exists (adjusts stats)
        self.remove_file(relative_path);

        let token_count = counter.count_or_zero(content);
        let size_bytes = content.len() as u64;

        if let Some(lang) = language {
            let stats = self
                .language_stats
                .entry(lang.to_string())
                .or_insert(LanguageStats {
                    file_count: 0,
                    total_bytes: 0,
                    total_tokens: 0,
                });
            stats.file_count += 1;
            stats.total_bytes += size_bytes;
            stats.total_tokens += token_count;
        }

        self.total_tokens += token_count;
        self.total_bytes += size_bytes;

        self.files.push(IndexedFile {
            relative_path: relative_path.to_string(),
            language: language.map(|s| s.to_string()),
            size_bytes,
            token_count,
            parse_result,
            content: content.to_string(),
        });

        self.total_files = self.files.len();
    }

    /// Remove a file from the index by relative path.
    ///
    /// Adjusts language stats and totals. No-op if the file is not present.
    pub fn remove_file(&mut self, relative_path: &str) {
        if let Some(pos) = self
            .files
            .iter()
            .position(|f| f.relative_path == relative_path)
        {
            let removed = self.files.swap_remove(pos);
            self.total_tokens = self.total_tokens.saturating_sub(removed.token_count);
            self.total_bytes = self.total_bytes.saturating_sub(removed.size_bytes);

            if let Some(lang) = &removed.language {
                if let Some(stats) = self.language_stats.get_mut(lang) {
                    stats.file_count = stats.file_count.saturating_sub(1);
                    stats.total_bytes = stats.total_bytes.saturating_sub(removed.size_bytes);
                    stats.total_tokens = stats.total_tokens.saturating_sub(removed.token_count);
                    if stats.file_count == 0 {
                        self.language_stats.remove(lang);
                    }
                }
            }

            self.total_files = self.files.len();
        }
    }

    pub fn is_key_file(path: &str) -> bool {
        let lower = path.to_lowercase();
        let filename = lower.rsplit('/').next().unwrap_or(&lower);
        matches!(
            filename,
            "readme.md"
                | "readme"
                | "cargo.toml"
                | "package.json"
                | "pom.xml"
                | "build.gradle"
                | "build.gradle.kts"
                | "go.mod"
                | "pyproject.toml"
                | "setup.py"
                | "setup.cfg"
                | "makefile"
                | "dockerfile"
                | "docker-compose.yml"
                | "docker-compose.yaml"
                | ".env.example"
        ) || lower.ends_with("main.rs")
            || lower.ends_with("main.go")
            || lower.ends_with("main.py")
            || lower.ends_with("main.java")
            || lower.ends_with("app.py")
            || lower.ends_with("index.ts")
            || lower.ends_with("index.js")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_key_file() {
        assert!(CodebaseIndex::is_key_file("README.md"));
        assert!(CodebaseIndex::is_key_file("Cargo.toml"));
        assert!(CodebaseIndex::is_key_file("src/main.rs"));
        assert!(CodebaseIndex::is_key_file("cmd/server/main.go"));
        assert!(CodebaseIndex::is_key_file("Dockerfile"));
        assert!(!CodebaseIndex::is_key_file("src/utils/helper.rs"));
        assert!(!CodebaseIndex::is_key_file("tests/test_foo.py"));
    }

    #[test]
    fn test_find_symbol_case_insensitive() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "pub fn MyFunc() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: file_path,
            language: Some("rust".into()),
            size_bytes: 18,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "test.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "MyFunc".into(),
                    kind: crate::parser::language::SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn MyFunc()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, parse_results, &counter);

        let matches = index.find_symbol("myfunc");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].1.name, "MyFunc");

        let no_match = index.find_symbol("nonexistent");
        assert!(no_match.is_empty());
    }

    #[test]
    fn test_find_content_matches() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "fn hello_world() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: file_path,
            language: Some("rust".into()),
            size_bytes: 20,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);

        let matches = index.find_content_matches("hello_world");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], "test.rs");

        let no_match = index.find_content_matches("xyz_not_found");
        assert!(no_match.is_empty());
    }

    #[test]
    fn test_all_public_symbols() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "pub fn foo() {} fn bar() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: file_path,
            language: Some("rust".into()),
            size_bytes: 27,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "test.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "foo".into(),
                        kind: crate::parser::language::SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn foo()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "bar".into(),
                        kind: crate::parser::language::SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn bar()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, parse_results, &counter);
        let public = index.all_public_symbols();
        assert_eq!(public.len(), 1);
        assert_eq!(public[0].1.name, "foo");
    }

    #[test]
    fn test_all_imports() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "use std::io;").unwrap();
        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: file_path,
            language: Some("rust".into()),
            size_bytes: 12,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "test.rs".into(),
            ParseResult {
                symbols: vec![],
                imports: vec![Import {
                    source: "std::io".into(),
                    names: vec!["io".into()],
                }],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, parse_results, &counter);
        let imports = index.all_imports();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].1.source, "std::io");
    }

    #[test]
    fn test_language_stats() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("a.rs");
        let fp2 = dir.path().join("b.rs");
        let fp3 = dir.path().join("c.py");
        std::fs::write(&fp1, "fn a() {}").unwrap();
        std::fs::write(&fp2, "fn b() {}").unwrap();
        std::fs::write(&fp3, "def c(): pass").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 9,
            },
            ScannedFile {
                relative_path: "b.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 9,
            },
            ScannedFile {
                relative_path: "c.py".into(),
                absolute_path: fp3,
                language: Some("python".into()),
                size_bytes: 13,
            },
        ];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert_eq!(index.language_stats["rust"].file_count, 2);
        assert_eq!(index.language_stats["python"].file_count, 1);
        assert_eq!(index.total_files, 3);
    }

    #[test]
    fn test_upsert_file_adds_new() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn a() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert_eq!(index.files.len(), 1);

        index.upsert_file("b.rs", Some("rust"), "fn b() {}", None, &counter);
        assert_eq!(index.files.len(), 2);
        assert_eq!(index.total_files, 2);
        let b = index
            .files
            .iter()
            .find(|f| f.relative_path == "b.rs")
            .unwrap();
        assert!(b.content.contains("fn b()"));
    }

    #[test]
    fn test_upsert_file_updates_existing() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn a() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);

        index.upsert_file(
            "a.rs",
            Some("rust"),
            "fn a_v2() { /* updated */ }",
            None,
            &counter,
        );
        assert_eq!(index.files.len(), 1);
        assert!(index.files[0].content.contains("a_v2"));
    }

    #[test]
    fn test_remove_file() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("a.rs");
        let fp2 = dir.path().join("b.rs");
        std::fs::write(&fp1, "fn a() {}").unwrap();
        std::fs::write(&fp2, "fn b() {}").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 9,
            },
            ScannedFile {
                relative_path: "b.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 9,
            },
        ];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert_eq!(index.files.len(), 2);

        index.remove_file("a.rs");
        assert_eq!(index.files.len(), 1);
        assert_eq!(index.total_files, 1);
        assert_eq!(index.files[0].relative_path, "b.rs");
    }

    #[test]
    fn test_remove_file_adjusts_language_stats() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("a.rs");
        let fp2 = dir.path().join("b.py");
        std::fs::write(&fp1, "fn a() {}").unwrap();
        std::fs::write(&fp2, "def b(): pass").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 9,
            },
            ScannedFile {
                relative_path: "b.py".into(),
                absolute_path: fp2,
                language: Some("python".into()),
                size_bytes: 13,
            },
        ];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert!(index.language_stats.contains_key("rust"));
        assert!(index.language_stats.contains_key("python"));

        index.remove_file("a.rs");
        // rust stats should be removed entirely (0 files)
        assert!(!index.language_stats.contains_key("rust"));
        assert!(index.language_stats.contains_key("python"));
    }

    #[test]
    fn test_remove_nonexistent_is_noop() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn a() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let orig_tokens = index.total_tokens;
        let orig_bytes = index.total_bytes;

        index.remove_file("nonexistent.rs");
        assert_eq!(index.files.len(), 1);
        assert_eq!(index.total_tokens, orig_tokens);
        assert_eq!(index.total_bytes, orig_bytes);
    }

    /// This test is intentionally FAILING until Task 4 implements `build_with_content`
    /// properly.  The stub ignores the content map and falls back to `build()`, which
    /// reads the file from disk.  Once the real implementation is in place the content
    /// map is used directly and no disk read occurs, causing the test to pass.
    #[test]
    fn test_build_with_content_uses_provided_content() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");
        // Write one string to disk — the implementation must NOT return this.
        std::fs::write(&file_path, "fn disk_version() {}").unwrap();

        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: file_path,
            language: Some("rust".into()),
            size_bytes: 20,
        }];

        // Provide DIFFERENT content via the content map.  A correct implementation
        // must use this string rather than reading from disk.
        let mut content_map = HashMap::new();
        content_map.insert(
            "test.rs".to_string(),
            "fn memory_version() { /* extra content here */ }".to_string(),
        );

        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);

        assert_eq!(index.files.len(), 1);
        assert!(
            index.files[0].content.contains("memory_version"),
            "build_with_content should use provided content, not read from disk. Got: {}",
            index.files[0].content
        );
        assert!(
            !index.files[0].content.contains("disk_version"),
            "build_with_content should NOT read from disk when content is provided. Got: {}",
            index.files[0].content
        );
    }
}

pub mod graph;
pub mod ranking;
pub mod symbols;

use crate::budget::counter::TokenCounter;
use crate::context_quality::expansion::Domain;
use crate::conventions::ConventionProfile;
use crate::index::graph::DependencyGraph;
use crate::intelligence::test_map::TestFileRef;
use crate::parser::language::{Import, ParseResult, Symbol, Visibility};
use crate::scanner::ScannedFile;
use crate::schema::SchemaIndex;
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub struct CodebaseIndex {
    pub files: Vec<IndexedFile>,
    pub language_stats: HashMap<String, LanguageStats>,
    pub total_files: usize,
    pub total_bytes: u64,
    pub total_tokens: usize,
    pub term_frequencies: HashMap<String, HashMap<String, u32>>,
    pub domains: HashSet<Domain>,
    pub schema: Option<SchemaIndex>,
    pub graph: DependencyGraph,
    pub pagerank: HashMap<String, f64>,
    pub test_map: HashMap<String, Vec<TestFileRef>>,
    pub conventions: ConventionProfile,
    pub co_changes: Vec<crate::intelligence::co_change::CoChangeEdge>,
    #[cfg(feature = "embeddings")]
    pub embedding_index: Option<crate::embeddings::EmbeddingIndex>,
}

#[derive(Debug)]
pub struct IndexedFile {
    pub relative_path: String,
    pub language: Option<String>,
    pub size_bytes: u64,
    pub token_count: usize,
    pub parse_result: Option<ParseResult>,
    pub content: String,
    pub mtime_secs: Option<u64>, // Unix epoch seconds, None if unavailable
}

#[derive(Debug)]
pub struct LanguageStats {
    pub file_count: usize,
    pub total_bytes: u64,
    pub total_tokens: usize,
}

pub(crate) fn compute_term_frequencies(content: &str) -> HashMap<String, u32> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for word in content.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if word.len() < 2 {
            continue;
        }
        for part in split_identifier(word) {
            if part.len() >= 2 {
                *counts.entry(part).or_insert(0) += 1;
            }
        }
    }
    counts
}

pub(crate) fn split_identifier(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for segment in s.split('_') {
        if segment.is_empty() {
            continue;
        }
        let mut current = String::new();
        let chars: Vec<char> = segment.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if i > 0 && ch.is_uppercase() {
                if !current.is_empty() {
                    parts.push(current.to_lowercase());
                }
                current = String::new();
            }
            current.push(ch);
        }
        if !current.is_empty() {
            parts.push(current.to_lowercase());
        }
    }
    parts
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
        let mut term_frequencies = HashMap::new();

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

            term_frequencies.insert(
                file.relative_path.clone(),
                compute_term_frequencies(&content),
            );

            let mtime_secs = std::fs::metadata(&file.absolute_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());

            let parse_result = parse_results.get(&file.relative_path).cloned();
            indexed_files.push(IndexedFile {
                relative_path: file.relative_path.clone(),
                language: file.language.clone(),
                size_bytes: file.size_bytes,
                token_count,
                parse_result,
                content,
                mtime_secs,
            });
        }

        let domains = crate::context_quality::expansion::detect_domains(&indexed_files);

        let mut index = Self {
            total_files: indexed_files.len(),
            total_bytes,
            total_tokens,
            files: indexed_files,
            language_stats,
            term_frequencies,
            domains,
            schema: None,
            graph: DependencyGraph::new(),
            pagerank: HashMap::new(),
            test_map: HashMap::new(),
            conventions: ConventionProfile::default(),
            co_changes: Vec::new(),
            #[cfg(feature = "embeddings")]
            embedding_index: None,
        };
        index.schema = crate::schema::detect::build_schema_index(&index);
        index.graph =
            crate::index::graph::build_dependency_graph(&index.files, index.schema.as_ref());
        index.pagerank = crate::intelligence::pagerank::compute_pagerank(&index.graph, 0.85, 100);
        let all_paths: std::collections::HashSet<String> = index
            .files
            .iter()
            .map(|f| f.relative_path.clone())
            .collect();
        index.test_map = crate::intelligence::test_map::build_test_map(&index.files, &all_paths);
        // NOTE: embedding_index is NOT built here. It's built at server startup
        // via build_embedding_index() — model download should not block CLI commands.
        // NOTE: conventions is NOT built here. It's built after index construction
        // via build_convention_profile() — needs repo_path for git2 access.
        index
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
        let mut term_frequencies = HashMap::new();

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

            term_frequencies.insert(
                file.relative_path.clone(),
                compute_term_frequencies(&file_content),
            );

            let mtime_secs = std::fs::metadata(&file.absolute_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());

            let parse_result = parse_results.get(&file.relative_path).cloned();
            indexed_files.push(IndexedFile {
                relative_path: file.relative_path.clone(),
                language: file.language.clone(),
                size_bytes: file.size_bytes,
                token_count,
                parse_result,
                content: file_content,
                mtime_secs,
            });
        }

        let domains = crate::context_quality::expansion::detect_domains(&indexed_files);

        let mut index = Self {
            total_files: indexed_files.len(),
            total_bytes,
            total_tokens,
            files: indexed_files,
            language_stats,
            term_frequencies,
            domains,
            schema: None,
            graph: DependencyGraph::new(),
            pagerank: HashMap::new(),
            test_map: HashMap::new(),
            conventions: ConventionProfile::default(),
            co_changes: Vec::new(),
            #[cfg(feature = "embeddings")]
            embedding_index: None,
        };
        index.schema = crate::schema::detect::build_schema_index(&index);
        index.graph =
            crate::index::graph::build_dependency_graph(&index.files, index.schema.as_ref());
        index.pagerank = crate::intelligence::pagerank::compute_pagerank(&index.graph, 0.85, 100);
        let all_paths: std::collections::HashSet<String> = index
            .files
            .iter()
            .map(|f| f.relative_path.clone())
            .collect();
        index.test_map = crate::intelligence::test_map::build_test_map(&index.files, &all_paths);
        // NOTE: embedding_index is NOT built here. It's built at server startup
        // via build_embedding_index() — model download should not block CLI commands.
        // NOTE: conventions is NOT built here. It's built after index construction
        // via build_convention_profile() — needs repo_path for git2 access.
        index
    }

    /// Rebuild the cached dependency graph from current files and schema.
    ///
    /// Call this after mutating `schema` post-construction to ensure the graph
    /// includes schema-aware edges.
    pub fn rebuild_graph(&mut self) {
        self.graph = crate::index::graph::build_dependency_graph(&self.files, self.schema.as_ref());
    }

    /// Rebuild the index incrementally: re-parse only files whose mtime/size differs.
    ///
    /// Steps:
    /// 1. Scan current files, compare mtime against stored IndexedFile.mtime_secs.
    /// 2. Call upsert_file() for changed/new files.
    /// 3. Call remove_file() for deleted files.
    /// 4. Call rebuild_graph() to recompute the dependency graph.
    /// 5. Recompute PageRank and test_map.
    pub fn incremental_rebuild(
        &mut self,
        current_files: &[crate::scanner::ScannedFile],
        parse_results: &std::collections::HashMap<String, crate::parser::language::ParseResult>,
        counter: &TokenCounter,
    ) {
        let current_paths: std::collections::HashSet<String> = current_files
            .iter()
            .map(|f| f.relative_path.clone())
            .collect();

        // Remove files that no longer exist
        let to_remove: Vec<String> = self
            .files
            .iter()
            .filter(|f| !current_paths.contains(&f.relative_path))
            .map(|f| f.relative_path.clone())
            .collect();
        for path in &to_remove {
            self.remove_file(path);
        }

        // Upsert changed or new files
        for file in current_files {
            let mtime_secs = std::fs::metadata(&file.absolute_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());

            let needs_update = match self
                .files
                .iter()
                .find(|f| f.relative_path == file.relative_path)
            {
                None => true, // new file
                Some(existing) => match (existing.mtime_secs, mtime_secs) {
                    (Some(old), Some(new)) => new > old || file.size_bytes != existing.size_bytes,
                    _ => true, // no mtime available: always re-parse
                },
            };

            if needs_update {
                let content = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
                let parse_result = parse_results.get(&file.relative_path).cloned();
                self.upsert_file(
                    &file.relative_path,
                    file.language.as_deref(),
                    &content,
                    parse_result,
                    counter,
                    mtime_secs,
                );
            }
        }

        // Rebuild graph and recompute derived scores
        self.rebuild_graph();
        self.pagerank = crate::intelligence::pagerank::compute_pagerank(&self.graph, 0.85, 100);
        let all_paths: std::collections::HashSet<String> =
            self.files.iter().map(|f| f.relative_path.clone()).collect();
        self.test_map = crate::intelligence::test_map::build_test_map(&self.files, &all_paths);
        self.total_files = self.files.len();
    }

    /// Returns `true` if an embedding index was successfully built for this codebase.
    pub fn has_embedding_index(&self) -> bool {
        #[cfg(feature = "embeddings")]
        {
            self.embedding_index.is_some()
        }
        #[cfg(not(feature = "embeddings"))]
        {
            false
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
        mtime_secs: Option<u64>,
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
            mtime_secs,
        });

        self.total_files = self.files.len();
        self.term_frequencies
            .insert(relative_path.to_string(), compute_term_frequencies(content));
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
            self.term_frequencies.remove(relative_path);
        }
    }

    /// Create an empty index with no files. Used when the MCP server
    /// starts in a non-git directory (graceful degradation).
    pub fn empty() -> Self {
        Self {
            files: Vec::new(),
            language_stats: HashMap::new(),
            total_files: 0,
            total_bytes: 0,
            total_tokens: 0,
            term_frequencies: HashMap::new(),
            domains: HashSet::new(),
            schema: None,
            graph: DependencyGraph::new(),
            pagerank: HashMap::new(),
            test_map: HashMap::new(),
            conventions: ConventionProfile::default(),
            co_changes: Vec::new(),
            #[cfg(feature = "embeddings")]
            embedding_index: None,
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

// ---------------------------------------------------------------------------
// Embedding index construction (feature-gated)
// ---------------------------------------------------------------------------

/// Attempt to build an embedding index for all public symbols in the codebase.
///
/// Returns `None` on any error (missing API key, no network, etc.) so that
/// the rest of the index build always succeeds.
#[cfg(feature = "embeddings")]
pub fn build_embedding_index(index: &CodebaseIndex) -> Option<crate::embeddings::EmbeddingIndex> {
    use crate::embeddings::{create_provider, EmbeddingConfig, EmbeddingIndex};

    let config = EmbeddingConfig::local_default();
    let provider = match create_provider(config.clone()) {
        Ok(p) => p,
        Err(_) => return None,
    };

    let symbols: Vec<(String, String)> = index
        .files
        .iter()
        .filter_map(|f| {
            f.parse_result
                .as_ref()
                .map(|pr| (f.relative_path.clone(), pr))
        })
        .flat_map(|(path, pr)| {
            pr.symbols
                .iter()
                .filter(|s| s.visibility == crate::parser::language::Visibility::Public)
                .map(move |s| (path.clone(), s.signature.clone()))
        })
        .collect();

    if symbols.is_empty() {
        return None;
    }

    let mut emb_index = EmbeddingIndex::new(provider.dimensions());

    // Embed in batches.
    let batch_size = config.batch_size;
    for chunk in symbols.chunks(batch_size) {
        let texts: Vec<&str> = chunk.iter().map(|(_, sig)| sig.as_str()).collect();
        let vectors = match provider.embed_batch(&texts) {
            Ok(v) => v,
            Err(_) => return None,
        };
        for ((path, _sig), vector) in chunk.iter().zip(vectors) {
            emb_index.add(path.clone(), vector);
        }
    }

    if emb_index.is_empty() {
        None
    } else {
        Some(emb_index)
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

        index.upsert_file("b.rs", Some("rust"), "fn b() {}", None, &counter, None);
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
            None,
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

    #[test]
    fn test_term_frequencies_built_during_index() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("api.rs");
        std::fs::write(&fp, "fn handle_request() { let rate = get_rate_limit(); }").unwrap();
        let files = vec![ScannedFile {
            relative_path: "api.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 55,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let tf = index
            .term_frequencies
            .get("api.rs")
            .expect("should have tf for api.rs");
        assert!(tf.get("handle").unwrap_or(&0) > &0);
        assert!(tf.get("request").unwrap_or(&0) > &0);
        assert!(tf.get("rate").unwrap_or(&0) > &0);
    }

    #[test]
    fn test_term_frequencies_empty_file() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("empty.rs");
        std::fs::write(&fp, "").unwrap();
        let files = vec![ScannedFile {
            relative_path: "empty.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 0,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let tf = index
            .term_frequencies
            .get("empty.rs")
            .expect("should have tf entry");
        assert!(tf.is_empty());
    }

    #[test]
    fn test_term_frequencies_with_build_with_content() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "").unwrap();
        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 30,
        }];
        let mut content_map = HashMap::new();
        content_map.insert(
            "test.rs".to_string(),
            "fn hello_world() { hello(); world(); }".to_string(),
        );
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let tf = index.term_frequencies.get("test.rs").unwrap();
        assert_eq!(*tf.get("hello").unwrap_or(&0), 2);
        assert_eq!(*tf.get("world").unwrap_or(&0), 2);
    }

    #[test]
    fn test_term_frequencies_updated_on_upsert() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn old() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 11,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert!(index.term_frequencies["a.rs"].contains_key("old"));
        index.upsert_file(
            "a.rs",
            Some("rust"),
            "fn new_func() {}",
            None,
            &counter,
            None,
        );
        assert!(!index.term_frequencies["a.rs"].contains_key("old"));
        assert!(index.term_frequencies["a.rs"].contains_key("new"));
    }

    #[test]
    fn test_term_frequencies_cleaned_on_remove() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn test() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 12,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert!(index.term_frequencies.contains_key("a.rs"));
        index.remove_file("a.rs");
        assert!(!index.term_frequencies.contains_key("a.rs"));
    }

    #[test]
    fn test_term_frequencies_camel_case_splitting() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("api.rs");
        std::fs::write(&fp, "fn handleRequest() { getResponse(); }").unwrap();
        let files = vec![ScannedFile {
            relative_path: "api.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 40,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let tf = index.term_frequencies.get("api.rs").unwrap();
        assert!(
            tf.get("handle").unwrap_or(&0) > &0,
            "should split handleRequest into handle"
        );
        assert!(
            tf.get("request").unwrap_or(&0) > &0,
            "should split handleRequest into request"
        );
        assert!(
            tf.get("get").unwrap_or(&0) > &0,
            "should split getResponse into get"
        );
        assert!(
            tf.get("response").unwrap_or(&0) > &0,
            "should split getResponse into response"
        );
    }

    // --- split_identifier edge cases ---

    #[test]
    fn test_split_identifier_snake_case() {
        let parts = split_identifier("rate_limit_check");
        assert_eq!(parts, vec!["rate", "limit", "check"]);
    }

    #[test]
    fn test_split_identifier_single_char_segments() {
        // Single-char segments are kept by split_identifier itself;
        // callers (compute_term_frequencies, tokenize) filter len < 2.
        let parts = split_identifier("a_b_cd");
        assert_eq!(parts, vec!["a", "b", "cd"]);
    }

    #[test]
    fn test_split_identifier_all_caps() {
        let parts = split_identifier("API");
        // Each uppercase letter is a camelCase boundary, so A|P|I → ["a","p","i"]
        assert_eq!(parts, vec!["a", "p", "i"]);
    }

    #[test]
    fn test_split_identifier_mixed_caps_and_numbers() {
        let parts = split_identifier("handle2Request");
        // "handle2" stays together (no uppercase boundary), then "Request" splits
        assert_eq!(parts, vec!["handle2", "request"]);
    }

    #[test]
    fn test_split_identifier_empty_string() {
        let parts = split_identifier("");
        assert!(parts.is_empty());
    }

    #[test]
    fn test_split_identifier_leading_underscores() {
        let parts = split_identifier("__private_field");
        assert_eq!(parts, vec!["private", "field"]);
    }

    #[test]
    fn test_compute_term_frequencies_filters_short_parts() {
        // "a_b" splits into ["a", "b"], both len=1, so neither should appear
        let freqs = compute_term_frequencies("a_b x_y");
        assert!(
            freqs.is_empty(),
            "single-char parts should be filtered: {:?}",
            freqs
        );
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

    #[test]
    fn test_index_co_changes_empty_by_default() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        assert!(index.co_changes.is_empty());
    }

    #[test]
    fn test_indexed_file_has_mtime_from_disk() {
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
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert!(
            index.files[0].mtime_secs.is_some(),
            "mtime_secs should be populated from disk"
        );
    }

    #[test]
    fn test_incremental_rebuild_removes_deleted_file() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp_a = dir.path().join("a.rs");
        let fp_b = dir.path().join("b.rs");
        std::fs::write(&fp_a, "fn a() {}").unwrap();
        std::fs::write(&fp_b, "fn b() {}").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp_a.clone(),
                language: Some("rust".into()),
                size_bytes: 9,
            },
            ScannedFile {
                relative_path: "b.rs".into(),
                absolute_path: fp_b,
                language: Some("rust".into()),
                size_bytes: 9,
            },
        ];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert_eq!(index.files.len(), 2);

        // Simulate b.rs deleted: only pass a.rs in current_files
        let current = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp_a,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        index.incremental_rebuild(&current, &HashMap::new(), &counter);
        assert_eq!(index.files.len(), 1);
        assert_eq!(index.files[0].relative_path, "a.rs");
    }
}

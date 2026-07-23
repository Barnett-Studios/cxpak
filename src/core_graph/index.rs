//! The index data model: `CodebaseIndex`, `IndexedFile`, `LanguageStats`, and
//! the *pure* query methods over them.
//!
//! This is the core type every analysis layer (`schema`, `intelligence`,
//! `conventions`) consumes. By living in `core_graph` it is the foundation of
//! the acyclic boundary (ADR-0007, cxpak 3.0.0 Phase 0): those modules depend
//! on `core_graph`, never the reverse.
//!
//! Construction and orchestration — `build`, `build_with_content`,
//! `incremental_rebuild`, `rebuild_graph*`, `upsert_file`, `remove_file`,
//! `health_cached`, `dead_code_cached`, `build_embedding_index` — stay in
//! `src/index/mod.rs` as inherent `impl` blocks on this type. They call DOWN
//! into scanner/parser/schema/intelligence/conventions/embeddings and therefore
//! belong above this boundary.

use crate::core_graph::conventions::ConventionProfile;
use crate::core_graph::domain::Domain;
use crate::core_graph::graph::DependencyGraph;
use crate::core_graph::intel::{CallGraph, CoChangeEdge, CrossLangEdge, DeadSymbol, HealthScore};
use crate::core_graph::schema::SchemaIndex;
use crate::parser::language::{Import, ParseResult, Symbol, Visibility};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone)]
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
    pub test_map: HashMap<String, Vec<crate::core_graph::intel::TestFileRef>>,
    pub conventions: ConventionProfile,
    pub call_graph: CallGraph,
    pub co_changes: Vec<CoChangeEdge>,
    /// Cross-language boundary edges detected during index build (v1.5.0).
    ///
    /// Each edge is also injected into `graph` as an
    /// [`crate::core_graph::graph::EdgeType::CrossLanguage`] edge so existing
    /// blast-radius / PageRank / auto_context pipelines pick them up.
    pub cross_lang_edges: Vec<CrossLangEdge>,
    /// Similarity signal #7 (opt-in via `.cxpak.json`; ADR-0186). Wrapped in
    /// `Arc` (issue #47 / ADR-0205) so cloning a `CodebaseIndex` — which the MCP
    /// freshness watcher does per edit batch — bumps a refcount instead of
    /// deep-copying the flat `Vec<f32>` matrix (`EmbeddingIndex::vectors`), the
    /// dominant `MALLOC_LARGE` allocation class. Nulling to `None` on the
    /// 6-signal-fallback path (ADR-0200) then drops that single shared ref.
    #[cfg(feature = "embeddings")]
    pub embedding_index: Option<Arc<crate::embeddings::EmbeddingIndex>>,
    /// Memoized `detect_dead_code(self, None)` result. Populated lazily on
    /// first call to `CodebaseIndex::dead_code_cached` (the orchestration method
    /// defined in `crate`'s index module). Shared across clones via `Arc`, so any
    /// clone that triggers computation benefits all clones.
    ///
    /// Invalidation contract: callers that mutate the index in-place (e.g.,
    /// `commands::serve::process_watcher_changes` after
    /// `apply_incremental_update`) MUST reset this with
    /// `idx.dead_code_cache = Arc::new(OnceLock::new())` so the next read
    /// recomputes against the new state. Constructors (`build`,
    /// `build_with_content`, `empty`) initialise it fresh, so a full
    /// replace via `*shared.write() = new_index` is also correct.
    #[doc(hidden)]
    pub dead_code_cache: Arc<OnceLock<Vec<DeadSymbol>>>,
    /// Memoized full HealthScore.  Same lazy-fill / Arc-shared / reset-on-
    /// watcher-update contract as `dead_code_cache`.  Without this, every
    /// `GET /v1/health` poll redoes O(F) convention/coupling/cycles work.
    #[doc(hidden)]
    pub health_cache: Arc<OnceLock<HealthScore>>,
}

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub relative_path: String,
    pub language: Option<String>,
    pub size_bytes: u64,
    pub token_count: usize,
    pub parse_result: Option<ParseResult>,
    pub content: String,
    pub mtime_secs: Option<u64>, // Unix epoch seconds, None if unavailable
}

#[derive(Debug, Clone)]
pub struct LanguageStats {
    pub file_count: usize,
    pub total_bytes: u64,
    pub total_tokens: usize,
}

/// NFKC-normalize then lowercase an identifier string.
///
/// Applied at BOTH the term-production site (`split_identifier`) and the
/// lookup site (`symbol_importance`) so they can never produce divergent keys.
/// Order is fixed: NFKC first, then `to_lowercase()`.  On pure-ASCII input
/// NFKC is a no-op, so ASCII identifiers are byte-for-byte unchanged.
pub fn normalize_identifier(s: &str) -> String {
    s.nfkc().collect::<String>().to_lowercase()
}

pub fn compute_term_frequencies(content: &str) -> HashMap<String, u32> {
    // NFKC-normalize the full content before splitting so that combining
    // diacritics (e.g. NFD decompositions) are composed into single codepoints
    // and do not act as word-boundary delimiters.  split_identifier then
    // re-applies NFKC on each token (idempotent) before camel/snake splitting.
    let normalized_content: String = content.nfkc().collect();
    let mut counts: HashMap<String, u32> = HashMap::new();
    for word in normalized_content.split(|c: char| !c.is_alphanumeric() && c != '_') {
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

pub fn split_identifier(s: &str) -> Vec<String> {
    // NFKC-normalize the input so Unicode-equivalent forms produce identical
    // terms.  Lowercase is applied per-part below (as before), so camelCase
    // boundary detection on uppercase ASCII chars is unaffected.
    let nfkc: String = s.nfkc().collect();
    let mut parts = Vec::new();
    for segment in nfkc.split('_') {
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
            call_graph: CallGraph::default(),
            conventions: ConventionProfile::default(),
            co_changes: Vec::new(),
            cross_lang_edges: Vec::new(),
            #[cfg(feature = "embeddings")]
            embedding_index: None,
            dead_code_cache: Arc::new(OnceLock::new()),
            health_cache: Arc::new(OnceLock::new()),
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

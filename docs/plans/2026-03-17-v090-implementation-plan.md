# cxpak v0.9.0 Implementation Plan: MCP Integration + Task-Aware Context

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a relevance scoring module and two new MCP tools (`cxpak_context_for_task`, `cxpak_pack_context`) that enable task-aware, token-budgeted context bundling, plus wire the MCP server into the Claude Code plugin via `.mcp.json`.

**Architecture:** New `src/relevance/` module with `RelevanceScorer` trait, 5 scoring signals, and seed selection with dependency fan-out. Two new tool handlers added to the existing MCP stdio loop in `src/commands/serve.rs`. Plugin gains `.mcp.json` and `ensure-cxpak-serve` wrapper.

**Tech Stack:** Rust, serde_json, existing tree-sitter index, existing DependencyGraph, existing TokenCounter, bash (plugin scripts)

---

## Task 1: Index Extension — term_frequencies field

**Files:**
- Modify: `src/index/mod.rs`
- Test: `src/index/mod.rs` (inline tests)

**Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block in `src/index/mod.rs`:

```rust
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

    // term_frequencies should exist and contain counts for api.rs
    let tf = index.term_frequencies.get("api.rs").expect("should have tf for api.rs");
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
    let tf = index.term_frequencies.get("empty.rs").expect("should have tf entry");
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
    content_map.insert("test.rs".to_string(), "fn hello_world() { hello(); world(); }".to_string());
    let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
    let tf = index.term_frequencies.get("test.rs").unwrap();
    assert_eq!(*tf.get("hello").unwrap_or(&0), 2); // hello appears in fn name + call
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

    index.upsert_file("a.rs", Some("rust"), "fn new_func() {}", None, &counter);
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
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib index::tests::test_term_frequencies -- --nocapture 2>&1 | head -30`
Expected: FAIL — `term_frequencies` field doesn't exist on `CodebaseIndex`

**Step 3: Implement term_frequencies**

In `src/index/mod.rs`:

1. Add the field to `CodebaseIndex`:
```rust
#[derive(Debug)]
pub struct CodebaseIndex {
    pub files: Vec<IndexedFile>,
    pub language_stats: HashMap<String, LanguageStats>,
    pub total_files: usize,
    pub total_bytes: u64,
    pub total_tokens: usize,
    pub term_frequencies: HashMap<String, HashMap<String, u32>>,
}
```

2. Add a helper function to compute term frequencies from content:
```rust
/// Split content on word boundaries and count occurrences of each term.
/// Terms are lowercased. Short terms (< 2 chars) and common syntax tokens are skipped.
fn compute_term_frequencies(content: &str) -> HashMap<String, u32> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for word in content.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let lower = word.to_lowercase();
        if lower.len() < 2 {
            continue;
        }
        // Split snake_case and camelCase
        for part in split_identifier(&lower) {
            if part.len() >= 2 {
                *counts.entry(part).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// Split an identifier into parts on underscores and camelCase boundaries.
fn split_identifier(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    // First split on underscores
    for segment in s.split('_') {
        if segment.is_empty() {
            continue;
        }
        // Then split on camelCase boundaries
        let mut current = String::new();
        let chars: Vec<char> = segment.chars().collect();
        for i in 0..chars.len() {
            if i > 0 && chars[i].is_uppercase() {
                if !current.is_empty() {
                    parts.push(current.to_lowercase());
                }
                current = String::new();
            }
            current.push(chars[i]);
        }
        if !current.is_empty() {
            parts.push(current.to_lowercase());
        }
    }
    parts
}
```

3. In `build()`, compute TF for each file and store in the struct:
```rust
// Inside the for loop, after building IndexedFile:
let tf = compute_term_frequencies(&content);
term_frequencies.insert(file.relative_path.clone(), tf);
```

4. In `build_with_content()`, same pattern.

5. In `upsert_file()`, recompute TF:
```rust
let tf = compute_term_frequencies(content);
self.term_frequencies.insert(relative_path.to_string(), tf);
```

6. In `remove_file()`, clean up TF:
```rust
self.term_frequencies.remove(relative_path);
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib index::tests -- --nocapture`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add src/index/mod.rs
git commit -m "feat: add term_frequencies to CodebaseIndex for relevance scoring"
```

---

## Task 2: Relevance Module — RelevanceScorer trait + MultiSignalScorer

**Files:**
- Create: `src/relevance/mod.rs`
- Modify: `src/lib.rs` (add `pub mod relevance;`)
- Test: `src/relevance/mod.rs` (inline tests)

**Step 1: Write the failing test**

Create `src/relevance/mod.rs` with tests at the bottom:

```rust
pub mod seed;
pub mod signals;

use crate::index::CodebaseIndex;

/// Result of scoring a single file against a query.
#[derive(Debug, Clone)]
pub struct ScoredFile {
    pub path: String,
    pub score: f64,
    pub signals: Vec<SignalResult>,
    pub token_count: usize,
}

/// Breakdown of a single signal's contribution.
#[derive(Debug, Clone)]
pub struct SignalResult {
    pub name: &'static str,
    pub score: f64,
    pub detail: String,
}

/// Trait for scoring file relevance against a query.
pub trait RelevanceScorer: Send + Sync {
    fn score(&self, query: &str, file_path: &str, index: &CodebaseIndex) -> ScoredFile;
}

/// Combines multiple weighted signals into a single score.
pub struct MultiSignalScorer {
    pub weights: SignalWeights,
}

#[derive(Debug, Clone)]
pub struct SignalWeights {
    pub path_similarity: f64,
    pub symbol_match: f64,
    pub import_proximity: f64,
    pub term_frequency: f64,
    pub recency_boost: f64,
}

impl Default for SignalWeights {
    fn default() -> Self {
        Self {
            path_similarity: 0.20,
            symbol_match: 0.35,
            import_proximity: 0.15,
            term_frequency: 0.20,
            recency_boost: 0.10,
        }
    }
}

impl MultiSignalScorer {
    pub fn new() -> Self {
        Self {
            weights: SignalWeights::default(),
        }
    }

    pub fn with_weights(weights: SignalWeights) -> Self {
        Self { weights }
    }

    /// Score all files in the index against the query.
    pub fn score_all(&self, query: &str, index: &CodebaseIndex) -> Vec<ScoredFile> {
        index
            .files
            .iter()
            .map(|f| self.score(query, &f.relative_path, index))
            .collect()
    }
}

impl RelevanceScorer for MultiSignalScorer {
    fn score(&self, query: &str, file_path: &str, index: &CodebaseIndex) -> ScoredFile {
        let w = &self.weights;

        let path_sig = signals::path_similarity(query, file_path);
        let symbol_sig = signals::symbol_match(query, file_path, index);
        let import_sig = signals::import_proximity(file_path, index);
        let tf_sig = signals::term_frequency(query, file_path, index);
        let recency_sig = SignalResult {
            name: "recency_boost",
            score: 0.5, // neutral — no git history in index
            detail: "no git history available".to_string(),
        };

        let combined = w.path_similarity * path_sig.score
            + w.symbol_match * symbol_sig.score
            + w.import_proximity * import_sig.score
            + w.term_frequency * tf_sig.score
            + w.recency_boost * recency_sig.score;

        // Clamp to 0.0–1.0
        let score = combined.clamp(0.0, 1.0);

        let token_count = index
            .files
            .iter()
            .find(|f| f.relative_path == file_path)
            .map(|f| f.token_count)
            .unwrap_or(0);

        ScoredFile {
            path: file_path.to_string(),
            score,
            signals: vec![path_sig, symbol_sig, import_sig, tf_sig, recency_sig],
            token_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("src/api/mod.rs");
        let fp2 = dir.path().join("src/api/middleware.rs");
        let fp3 = dir.path().join("src/config.rs");
        std::fs::create_dir_all(dir.path().join("src/api")).unwrap();
        std::fs::write(&fp1, "pub fn handle_request() { rate_limit(); }").unwrap();
        std::fs::write(&fp2, "pub fn rate_limit() {}").unwrap();
        std::fs::write(&fp3, "pub struct Config {}").unwrap();

        let files = vec![
            ScannedFile { relative_path: "src/api/mod.rs".into(), absolute_path: fp1, language: Some("rust".into()), size_bytes: 42 },
            ScannedFile { relative_path: "src/api/middleware.rs".into(), absolute_path: fp2, language: Some("rust".into()), size_bytes: 22 },
            ScannedFile { relative_path: "src/config.rs".into(), absolute_path: fp3, language: Some("rust".into()), size_bytes: 22 },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert("src/api/mod.rs".to_string(), ParseResult {
            symbols: vec![Symbol {
                name: "handle_request".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "pub fn handle_request()".into(),
                body: "{ rate_limit(); }".into(),
                start_line: 1, end_line: 1,
            }],
            imports: vec![],
            exports: vec![],
        });
        parse_results.insert("src/api/middleware.rs".to_string(), ParseResult {
            symbols: vec![Symbol {
                name: "rate_limit".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "pub fn rate_limit()".into(),
                body: "{}".into(),
                start_line: 1, end_line: 1,
            }],
            imports: vec![],
            exports: vec![],
        });

        CodebaseIndex::build(files, parse_results, &counter)
    }

    #[test]
    fn test_multi_signal_scorer_returns_scores() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::new();
        let result = scorer.score("api request handler", "src/api/mod.rs", &index);
        assert!(result.score >= 0.0 && result.score <= 1.0);
        assert_eq!(result.signals.len(), 5);
        assert_eq!(result.path, "src/api/mod.rs");
    }

    #[test]
    fn test_score_all_returns_all_files() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::new();
        let results = scorer.score_all("rate limit", &index);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_relevant_file_scores_higher() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::new();
        let api_score = scorer.score("api request", "src/api/mod.rs", &index);
        let config_score = scorer.score("api request", "src/config.rs", &index);
        assert!(
            api_score.score > config_score.score,
            "api/mod.rs ({}) should score higher than config.rs ({}) for 'api request'",
            api_score.score, config_score.score
        );
    }

    #[test]
    fn test_weights_sum_to_one() {
        let w = SignalWeights::default();
        let sum = w.path_similarity + w.symbol_match + w.import_proximity + w.term_frequency + w.recency_boost;
        assert!((sum - 1.0).abs() < 0.001, "Weights should sum to 1.0, got {sum}");
    }

    #[test]
    fn test_custom_weights() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::with_weights(SignalWeights {
            path_similarity: 1.0,
            symbol_match: 0.0,
            import_proximity: 0.0,
            term_frequency: 0.0,
            recency_boost: 0.0,
        });
        let result = scorer.score("api", "src/api/mod.rs", &index);
        // Only path_similarity contributes
        assert!(result.score > 0.0);
    }

    #[test]
    fn test_score_nonexistent_file() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::new();
        let result = scorer.score("test", "nonexistent.rs", &index);
        assert_eq!(result.token_count, 0);
        // Should still return a valid score (likely low)
        assert!(result.score >= 0.0 && result.score <= 1.0);
    }

    #[test]
    fn test_all_zero_query() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::new();
        let result = scorer.score("xyznonexistent", "src/config.rs", &index);
        // Should be low but valid
        assert!(result.score >= 0.0 && result.score <= 1.0);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib relevance::tests -- --nocapture 2>&1 | head -20`
Expected: FAIL — `signals` module doesn't exist yet

**Step 3: Add module declaration to lib.rs**

In `src/lib.rs`, add:
```rust
pub mod relevance;
```

**Step 4: Create stub signals module**

Create `src/relevance/signals.rs` with just enough stubs for the scorer to compile (empty functions returning `SignalResult` with score 0.0). This will be fully implemented in Task 3.

Create `src/relevance/seed.rs` as empty file with just a comment.

**Step 5: Run tests to verify they pass**

Run: `cargo test --lib relevance::tests -- --nocapture`
Expected: Most pass, but `test_relevant_file_scores_higher` may fail until signals are real (Task 3)

**Step 6: Commit**

```bash
git add src/lib.rs src/relevance/mod.rs src/relevance/signals.rs src/relevance/seed.rs
git commit -m "feat: add relevance module with RelevanceScorer trait and MultiSignalScorer"
```

---

## Task 3: Relevance Module — Five Signal Implementations

**Files:**
- Modify: `src/relevance/signals.rs`
- Test: `src/relevance/signals.rs` (inline tests)

**Step 1: Write the failing tests**

In `src/relevance/signals.rs`, write comprehensive tests for all 5 signals:

```rust
use super::{SignalResult};
use crate::index::CodebaseIndex;

/// PathSimilarity: Tokenize query + file path segments, compute Jaccard similarity.
pub fn path_similarity(query: &str, file_path: &str) -> SignalResult {
    todo!()
}

/// SymbolMatch: Fuzzy match query terms against function/struct/class names in the file.
pub fn symbol_match(query: &str, file_path: &str, index: &CodebaseIndex) -> SignalResult {
    todo!()
}

/// ImportProximity: Boost if file imports or is imported by other scored files.
/// Returns a base score of 0.5 (neutral) — adjusted by seed selection later.
pub fn import_proximity(file_path: &str, index: &CodebaseIndex) -> SignalResult {
    todo!()
}

/// TermFrequency: Lightweight TF of query terms in file content.
pub fn term_frequency(query: &str, file_path: &str, index: &CodebaseIndex) -> SignalResult {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    // --- PathSimilarity tests ---

    #[test]
    fn test_path_similarity_exact_match() {
        let result = path_similarity("api mod", "src/api/mod.rs");
        assert!(result.score > 0.8, "exact path segments should score high: {}", result.score);
    }

    #[test]
    fn test_path_similarity_partial_match() {
        let result = path_similarity("api", "src/api/middleware.rs");
        assert!(result.score > 0.0 && result.score < 1.0);
    }

    #[test]
    fn test_path_similarity_no_overlap() {
        let result = path_similarity("database", "src/api/mod.rs");
        assert!(result.score < 0.2, "no overlap should score near zero: {}", result.score);
    }

    #[test]
    fn test_path_similarity_case_insensitive() {
        let r1 = path_similarity("API", "src/api/mod.rs");
        let r2 = path_similarity("api", "src/api/mod.rs");
        assert!((r1.score - r2.score).abs() < 0.01);
    }

    #[test]
    fn test_path_similarity_nested_paths() {
        let result = path_similarity("middleware", "src/api/middleware/rate_limiter.rs");
        assert!(result.score > 0.3);
    }

    // --- SymbolMatch tests ---

    fn make_symbol_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("handler.rs");
        std::fs::write(&fp, "pub fn handle_api_request() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "handler.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 30,
        }];
        let mut pr = HashMap::new();
        pr.insert("handler.rs".to_string(), ParseResult {
            symbols: vec![Symbol {
                name: "handle_api_request".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "pub fn handle_api_request()".into(),
                body: "{}".into(),
                start_line: 1, end_line: 1,
            }],
            imports: vec![],
            exports: vec![],
        });
        CodebaseIndex::build(files, pr, &counter)
    }

    #[test]
    fn test_symbol_match_exact_hit() {
        let index = make_symbol_index();
        let result = symbol_match("handle_api_request", "handler.rs", &index);
        assert!(result.score > 0.8, "exact symbol match should be high: {}", result.score);
    }

    #[test]
    fn test_symbol_match_fuzzy() {
        let index = make_symbol_index();
        let result = symbol_match("api request", "handler.rs", &index);
        assert!(result.score > 0.3, "fuzzy match should score mid-range: {}", result.score);
    }

    #[test]
    fn test_symbol_match_no_match() {
        let index = make_symbol_index();
        let result = symbol_match("database migration", "handler.rs", &index);
        assert!(result.score < 0.2, "no match should be low: {}", result.score);
    }

    #[test]
    fn test_symbol_match_case_insensitive() {
        let index = make_symbol_index();
        let result = symbol_match("Handle_API_Request", "handler.rs", &index);
        assert!(result.score > 0.5);
    }

    #[test]
    fn test_symbol_match_no_symbols() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("empty.rs");
        std::fs::write(&fp, "// no symbols").unwrap();
        let files = vec![ScannedFile {
            relative_path: "empty.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 13,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let result = symbol_match("anything", "empty.rs", &index);
        assert_eq!(result.score, 0.0);
    }

    // --- ImportProximity tests ---

    #[test]
    fn test_import_proximity_with_imports() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("a.rs");
        let fp2 = dir.path().join("b.rs");
        std::fs::write(&fp1, "use b;").unwrap();
        std::fs::write(&fp2, "pub fn b() {}").unwrap();
        let files = vec![
            ScannedFile { relative_path: "a.rs".into(), absolute_path: fp1, language: Some("rust".into()), size_bytes: 6 },
            ScannedFile { relative_path: "b.rs".into(), absolute_path: fp2, language: Some("rust".into()), size_bytes: 14 },
        ];
        let mut pr = HashMap::new();
        pr.insert("a.rs".to_string(), ParseResult {
            symbols: vec![],
            imports: vec![crate::parser::language::Import { source: "b".into(), names: vec!["b".into()] }],
            exports: vec![],
        });
        let index = CodebaseIndex::build(files, pr, &counter);
        // a.rs has imports, so import_proximity should be > 0.5 (neutral)
        let result = import_proximity("a.rs", &index);
        assert!(result.score >= 0.5);
    }

    #[test]
    fn test_import_proximity_no_imports() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("standalone.rs");
        std::fs::write(&fp, "fn standalone() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "standalone.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 18,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let result = import_proximity("standalone.rs", &index);
        assert!((result.score - 0.5).abs() < 0.01, "no imports should be neutral (0.5): {}", result.score);
    }

    // --- TermFrequency tests ---

    #[test]
    fn test_term_frequency_high_frequency() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("rate.rs");
        std::fs::write(&fp, "fn rate_limit() { check_rate(); apply_rate(); rate_exceeded(); }").unwrap();
        let files = vec![ScannedFile {
            relative_path: "rate.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 62,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let result = term_frequency("rate limit", "rate.rs", &index);
        assert!(result.score > 0.5, "high term frequency should score high: {}", result.score);
    }

    #[test]
    fn test_term_frequency_missing_terms() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("unrelated.rs");
        std::fs::write(&fp, "fn hello_world() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "unrelated.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 20,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let result = term_frequency("database migration", "unrelated.rs", &index);
        assert_eq!(result.score, 0.0);
    }

    #[test]
    fn test_term_frequency_nonexistent_file() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let result = term_frequency("test", "nonexistent.rs", &index);
        assert_eq!(result.score, 0.0);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib relevance::signals::tests -- --nocapture 2>&1 | head -20`
Expected: FAIL — `todo!()` panics

**Step 3: Implement all signal functions**

Replace the `todo!()` stubs with real implementations:

- **`path_similarity`**: Tokenize query by splitting on whitespace/underscores/dots. Tokenize file path by splitting on `/`, `.`, `_`. Compute Jaccard similarity (intersection/union).
- **`symbol_match`**: Find the file in `index.files`, get its `parse_result.symbols`. For each symbol, split its name into parts (snake_case/camelCase). Check how many query terms appear in any symbol name. Best match wins.
- **`import_proximity`**: Count how many imports and reverse-imports the file has (via `parse_result.imports`). More connections = higher score. Normalize to 0.0–1.0 with a cap at 10 connections.
- **`term_frequency`**: Look up `index.term_frequencies[file_path]`. For each query term, sum the counts. Normalize by total terms in the file.

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib relevance::signals::tests -- --nocapture`
Expected: ALL PASS

**Step 5: Also verify MultiSignalScorer tests pass now**

Run: `cargo test --lib relevance::tests -- --nocapture`
Expected: ALL PASS (including `test_relevant_file_scores_higher`)

**Step 6: Commit**

```bash
git add src/relevance/signals.rs
git commit -m "feat: implement 5 relevance scoring signals (path, symbol, import, TF, recency)"
```

---

## Task 4: Relevance Module — Seed Selection + Dependency Fan-out

**Files:**
- Modify: `src/relevance/seed.rs`
- Test: `src/relevance/seed.rs` (inline tests)

**Step 1: Write the failing tests**

```rust
use super::{MultiSignalScorer, RelevanceScorer, ScoredFile};
use crate::commands::trace::build_dependency_graph;
use crate::index::CodebaseIndex;

/// Default score threshold for seed selection.
pub const SEED_THRESHOLD: f64 = 0.3;

/// Discount factor for dependency fan-out scores.
pub const FANOUT_DISCOUNT: f64 = 0.7;

/// Select seed files above threshold, then fan out to 1-hop dependency neighbors.
pub fn select_seeds(
    scored: &[ScoredFile],
    index: &CodebaseIndex,
    threshold: f64,
    limit: usize,
) -> Vec<ScoredFile> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_seed_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let fp1 = dir.path().join("src/api.rs");
        let fp2 = dir.path().join("src/middleware.rs");
        let fp3 = dir.path().join("src/config.rs");
        let fp4 = dir.path().join("src/utils.rs");
        std::fs::write(&fp1, "use crate::middleware; fn api() {}").unwrap();
        std::fs::write(&fp2, "use crate::config; fn middleware() {}").unwrap();
        std::fs::write(&fp3, "fn config() {}").unwrap();
        std::fs::write(&fp4, "fn utils() {}").unwrap();

        let files = vec![
            ScannedFile { relative_path: "src/api.rs".into(), absolute_path: fp1, language: Some("rust".into()), size_bytes: 35 },
            ScannedFile { relative_path: "src/middleware.rs".into(), absolute_path: fp2, language: Some("rust".into()), size_bytes: 40 },
            ScannedFile { relative_path: "src/config.rs".into(), absolute_path: fp3, language: Some("rust".into()), size_bytes: 14 },
            ScannedFile { relative_path: "src/utils.rs".into(), absolute_path: fp4, language: Some("rust".into()), size_bytes: 14 },
        ];

        let mut pr = HashMap::new();
        pr.insert("src/api.rs".to_string(), ParseResult {
            symbols: vec![Symbol { name: "api".into(), kind: SymbolKind::Function, visibility: Visibility::Public, signature: "fn api()".into(), body: "{}".into(), start_line: 1, end_line: 1 }],
            imports: vec![Import { source: "crate::middleware".into(), names: vec!["middleware".into()] }],
            exports: vec![],
        });
        pr.insert("src/middleware.rs".to_string(), ParseResult {
            symbols: vec![Symbol { name: "middleware".into(), kind: SymbolKind::Function, visibility: Visibility::Public, signature: "fn middleware()".into(), body: "{}".into(), start_line: 1, end_line: 1 }],
            imports: vec![Import { source: "crate::config".into(), names: vec!["config".into()] }],
            exports: vec![],
        });

        CodebaseIndex::build(files, pr, &counter)
    }

    #[test]
    fn test_select_seeds_threshold_filtering() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile { path: "src/api.rs".into(), score: 0.8, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/middleware.rs".into(), score: 0.5, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/config.rs".into(), score: 0.2, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/utils.rs".into(), score: 0.1, signals: vec![], token_count: 10 },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        let paths: Vec<&str> = seeds.iter().map(|s| s.path.as_str()).collect();
        assert!(paths.contains(&"src/api.rs"));
        assert!(paths.contains(&"src/middleware.rs"));
        // config.rs below threshold (0.2 < 0.3), but may appear as dependency fan-out
        assert!(!paths.contains(&"src/utils.rs")); // too low, not a dependency
    }

    #[test]
    fn test_select_seeds_fanout_discount() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile { path: "src/api.rs".into(), score: 0.8, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/middleware.rs".into(), score: 0.1, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/config.rs".into(), score: 0.1, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/utils.rs".into(), score: 0.1, signals: vec![], token_count: 10 },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        // middleware.rs should be added via fan-out from api.rs (0.8 * 0.7 = 0.56)
        let middleware = seeds.iter().find(|s| s.path == "src/middleware.rs");
        assert!(middleware.is_some(), "middleware should be added via fan-out");
        assert!((middleware.unwrap().score - 0.56).abs() < 0.01,
            "fan-out score should be seed_score * 0.7 = 0.56, got {}",
            middleware.unwrap().score
        );
    }

    #[test]
    fn test_select_seeds_limit() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile { path: "src/api.rs".into(), score: 0.8, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/middleware.rs".into(), score: 0.7, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/config.rs".into(), score: 0.6, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/utils.rs".into(), score: 0.5, signals: vec![], token_count: 10 },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 2);
        assert!(seeds.len() <= 2);
    }

    #[test]
    fn test_select_seeds_empty_results() {
        let index = make_seed_index();
        let scored: Vec<ScoredFile> = vec![];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        assert!(seeds.is_empty());
    }

    #[test]
    fn test_select_seeds_all_below_threshold() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile { path: "src/api.rs".into(), score: 0.1, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/utils.rs".into(), score: 0.05, signals: vec![], token_count: 10 },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        assert!(seeds.is_empty());
    }

    #[test]
    fn test_select_seeds_sorted_by_score() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile { path: "src/api.rs".into(), score: 0.5, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/middleware.rs".into(), score: 0.8, signals: vec![], token_count: 10 },
            ScoredFile { path: "src/config.rs".into(), score: 0.6, signals: vec![], token_count: 10 },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        for i in 1..seeds.len() {
            assert!(seeds[i - 1].score >= seeds[i].score, "results should be sorted descending");
        }
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib relevance::seed::tests -- --nocapture 2>&1 | head -20`
Expected: FAIL — `todo!()` panics

**Step 3: Implement select_seeds**

Algorithm:
1. Filter scored files above threshold → seeds
2. Build dependency graph via `build_dependency_graph(index)`
3. For each seed, look up 1-hop neighbors in the graph (both directions)
4. For each neighbor not already a seed, add with score = seed_score * FANOUT_DISCOUNT
5. If a neighbor was added by multiple seeds, keep the highest score
6. Sort all by score descending
7. Truncate to limit

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib relevance::seed::tests -- --nocapture`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add src/relevance/seed.rs
git commit -m "feat: implement seed selection with dependency fan-out for relevance scoring"
```

---

## Task 5: MCP Tool — cxpak_context_for_task

**Files:**
- Modify: `src/commands/serve.rs`
- Test: `src/commands/serve.rs` (inline tests)

**Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `src/commands/serve.rs`:

```rust
#[test]
fn test_mcp_tools_list_includes_new_tools() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let tools = response["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6, "should have 6 tools (4 existing + 2 new)");
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tool_names.contains(&"cxpak_context_for_task"));
    assert!(tool_names.contains(&"cxpak_pack_context"));
}

#[test]
fn test_mcp_context_for_task_happy_path() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"main function","limit":5}}}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    let result: Value = serde_json::from_str(content).unwrap();
    assert_eq!(result["task"], "main function");
    assert!(result["candidates"].as_array().unwrap().len() > 0);
    assert!(result["total_files_scored"].as_u64().unwrap() > 0);
}

#[test]
fn test_mcp_context_for_task_empty_query() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":""}}}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("Error") || content.contains("error"));
}

#[test]
fn test_mcp_context_for_task_default_limit() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"hello"}}}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    let result: Value = serde_json::from_str(content).unwrap();
    assert!(result["candidates"].as_array().unwrap().len() <= 15); // default limit
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib commands::serve::tests::test_mcp_context_for_task -- --nocapture 2>&1 | head -20`
Expected: FAIL — tool not implemented

**Step 3: Implement cxpak_context_for_task handler**

In `serve.rs`:

1. Add to the `tools/list` response: new tool definition for `cxpak_context_for_task` with inputSchema `{ task: string (required), limit: number (optional, default 15) }`

2. Add to `handle_tool_call` match:
```rust
"cxpak_context_for_task" => {
    let task = args.get("task").and_then(|t| t.as_str()).unwrap_or("");
    if task.is_empty() {
        return mcp_tool_result(id, "Error: 'task' argument is required and must not be empty");
    }
    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(15) as usize;

    let scorer = crate::relevance::MultiSignalScorer::new();
    let all_scored = scorer.score_all(task, index);
    let seeds = crate::relevance::seed::select_seeds(
        &all_scored, index, crate::relevance::seed::SEED_THRESHOLD, limit,
    );

    let graph = crate::commands::trace::build_dependency_graph(index);
    let candidates: Vec<Value> = seeds.iter().map(|s| {
        let deps: Vec<&str> = graph.dependencies(&s.path)
            .map(|d| d.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let signals: Vec<Value> = s.signals.iter().map(|sig| {
            json!({"name": sig.name, "score": sig.score, "detail": &sig.detail})
        }).collect();
        json!({
            "path": s.path,
            "score": (s.score * 100.0).round() / 100.0,
            "signals": signals,
            "tokens": s.token_count,
            "dependencies": deps,
        })
    }).collect();

    mcp_tool_result(id, &serde_json::to_string_pretty(&json!({
        "task": task,
        "candidates": candidates,
        "total_files_scored": all_scored.len(),
        "hint": "Review candidates and call cxpak_pack_context with selected paths, or use these as-is."
    })).unwrap_or_default())
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib commands::serve::tests::test_mcp_context_for_task -- --nocapture`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add cxpak_context_for_task MCP tool for task-aware file ranking"
```

---

## Task 6: MCP Tool — cxpak_pack_context

**Files:**
- Modify: `src/commands/serve.rs`
- Test: `src/commands/serve.rs` (inline tests)

**Step 1: Write the failing tests**

```rust
#[test]
fn test_mcp_pack_context_happy_path() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs","src/lib.rs"],"tokens":"50k"}}}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    let result: Value = serde_json::from_str(content).unwrap();
    assert!(result["packed_files"].as_u64().unwrap() > 0);
    assert!(result["total_tokens"].as_u64().unwrap() > 0);
    let files = result["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "src/main.rs"));
}

#[test]
fn test_mcp_pack_context_with_dependencies() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs"],"tokens":"50k","include_dependencies":true}}}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    let result: Value = serde_json::from_str(content).unwrap();
    assert!(result["packed_files"].as_u64().unwrap() >= 1);
}

#[test]
fn test_mcp_pack_context_budget_overflow() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    // Very small budget — should omit some files
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs","src/lib.rs"],"tokens":"1"}}}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    let result: Value = serde_json::from_str(content).unwrap();
    // With budget of 1 token, should have omitted files
    assert!(result["omitted"].as_array().unwrap().len() > 0 || result["packed_files"].as_u64().unwrap() == 0);
}

#[test]
fn test_mcp_pack_context_missing_files() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["nonexistent.rs"],"tokens":"50k"}}}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    let result: Value = serde_json::from_str(content).unwrap();
    assert_eq!(result["packed_files"].as_u64().unwrap(), 0);
}

#[test]
fn test_mcp_pack_context_empty_files_list() {
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":[],"tokens":"50k"}}}"#;
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("Error") || content.contains("error") || {
        let result: Value = serde_json::from_str(content).unwrap();
        result["packed_files"].as_u64().unwrap() == 0
    });
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib commands::serve::tests::test_mcp_pack_context -- --nocapture 2>&1 | head -20`
Expected: FAIL — tool not implemented

**Step 3: Implement cxpak_pack_context handler**

1. Add tool definition to `tools/list` response:
```json
{
    "name": "cxpak_pack_context",
    "description": "Pack selected files into a token-budgeted context bundle with dependency context",
    "inputSchema": {
        "type": "object",
        "properties": {
            "files": { "type": "array", "items": { "type": "string" }, "description": "File paths to include" },
            "tokens": { "type": "string", "description": "Token budget (e.g. '30k', '50k')", "default": "50k" },
            "include_dependencies": { "type": "boolean", "description": "Include 1-hop dependencies", "default": false }
        },
        "required": ["files"]
    }
}
```

2. Add handler in `handle_tool_call`:
```rust
"cxpak_pack_context" => {
    let files: Vec<String> = args.get("files")
        .and_then(|f| f.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let token_budget = args.get("tokens")
        .and_then(|t| t.as_str())
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);
    let include_deps = args.get("include_dependencies")
        .and_then(|d| d.as_bool())
        .unwrap_or(false);

    // Resolve files + optional dependencies
    let mut target_files: Vec<(String, &str)> = vec![]; // (path, included_as)
    let graph = crate::commands::trace::build_dependency_graph(index);

    for path in &files {
        target_files.push((path.clone(), "selected"));
        if include_deps {
            if let Some(deps) = graph.dependencies(path) {
                for dep in deps {
                    if !target_files.iter().any(|(p, _)| p == dep) {
                        target_files.push((dep.clone(), "dependency"));
                    }
                }
            }
        }
    }

    // Pack within budget
    let mut packed = vec![];
    let mut omitted = vec![];
    let mut total_tokens = 0usize;

    for (path, included_as) in &target_files {
        if let Some(file) = index.files.iter().find(|f| f.relative_path == *path) {
            if total_tokens + file.token_count <= token_budget {
                packed.push(json!({
                    "path": path,
                    "tokens": file.token_count,
                    "content": file.content,
                    "included_as": included_as,
                }));
                total_tokens += file.token_count;
            } else {
                omitted.push(json!({
                    "path": path,
                    "tokens": file.token_count,
                    "reason": "budget exceeded",
                }));
            }
        }
    }

    mcp_tool_result(id, &serde_json::to_string_pretty(&json!({
        "packed_files": packed.len(),
        "total_tokens": total_tokens,
        "budget": token_budget,
        "files": packed,
        "omitted": omitted,
    })).unwrap_or_default())
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib commands::serve::tests::test_mcp_pack_context -- --nocapture`
Expected: ALL PASS

**Step 5: Run full test suite**

Run: `cargo test --verbose`
Expected: ALL PASS

**Step 6: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add cxpak_pack_context MCP tool for token-budgeted context bundling"
```

---

## Task 7: Plugin Wiring — .mcp.json + ensure-cxpak-serve

**Files:**
- Create: `plugin/.mcp.json`
- Create: `plugin/lib/ensure-cxpak-serve`
- Test: `plugin/tests/mcp-wiring.bats` (new)

**Step 1: Create .mcp.json**

Create `plugin/.mcp.json`:
```json
{
  "cxpak": {
    "command": "${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak-serve",
    "args": [],
    "env": {}
  }
}
```

**Step 2: Create ensure-cxpak-serve wrapper**

Create `plugin/lib/ensure-cxpak-serve`:
```bash
#!/usr/bin/env bash
set -euo pipefail

# Resolve the cxpak binary using the existing ensure-cxpak script
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CXPAK="$("${SCRIPT_DIR}/ensure-cxpak")"

# exec replaces this shell process so stdio flows directly
# between Claude Code and cxpak's MCP server
exec "$CXPAK" serve --mcp
```

**Step 3: Make executable**

```bash
chmod +x plugin/lib/ensure-cxpak-serve
```

**Step 4: Write tests**

Create `plugin/tests/mcp-wiring.bats`:
```bash
#!/usr/bin/env bats

@test ".mcp.json exists and is valid JSON" {
    run cat plugin/.mcp.json
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "import sys, json; json.load(sys.stdin)"
}

@test ".mcp.json references ensure-cxpak-serve" {
    run cat plugin/.mcp.json
    [[ "$output" == *"ensure-cxpak-serve"* ]]
}

@test ".mcp.json uses CLAUDE_PLUGIN_ROOT variable" {
    run cat plugin/.mcp.json
    [[ "$output" == *'${CLAUDE_PLUGIN_ROOT}'* ]]
}

@test "ensure-cxpak-serve is executable" {
    [ -x plugin/lib/ensure-cxpak-serve ]
}

@test "ensure-cxpak-serve references ensure-cxpak" {
    run cat plugin/lib/ensure-cxpak-serve
    [[ "$output" == *"ensure-cxpak"* ]]
}

@test "ensure-cxpak-serve uses exec for direct stdio" {
    run cat plugin/lib/ensure-cxpak-serve
    [[ "$output" == *"exec"* ]]
}

@test "ensure-cxpak-serve passes serve --mcp" {
    run cat plugin/lib/ensure-cxpak-serve
    [[ "$output" == *"serve --mcp"* ]]
}
```

**Step 5: Run tests**

Run: `bats plugin/tests/mcp-wiring.bats`
Expected: ALL PASS

**Step 6: Commit**

```bash
git add plugin/.mcp.json plugin/lib/ensure-cxpak-serve plugin/tests/mcp-wiring.bats
git commit -m "feat: wire MCP server into Claude Code plugin via .mcp.json"
```

---

## Task 8: Integration Test — Two-Phase Handshake

**Files:**
- Modify: `src/commands/serve.rs` (add integration test)

**Step 1: Write the integration test**

Add to `src/commands/serve.rs` tests:

```rust
#[test]
fn test_mcp_two_phase_handshake() {
    // Simulates: context_for_task → review candidates → pack_context
    let index = make_test_index();
    let repo_path = std::path::Path::new("/tmp");

    // Phase 1: Get candidates
    let request1 = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"main function"}}}"#;
    let mut output1 = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request1.as_bytes(), &mut output1).unwrap();
    let response1: Value = serde_json::from_slice(&output1).unwrap();
    let content1 = response1["result"]["content"][0]["text"].as_str().unwrap();
    let result1: Value = serde_json::from_str(content1).unwrap();

    // Extract candidate paths (simulating Claude reviewing and selecting)
    let candidates = result1["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty(), "should have candidates");
    let selected_paths: Vec<String> = candidates
        .iter()
        .take(2)
        .map(|c| c["path"].as_str().unwrap().to_string())
        .collect();

    // Phase 2: Pack selected files
    let request2 = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"cxpak_pack_context","arguments":{{"files":{},"tokens":"50k","include_dependencies":true}}}}}}"#,
        serde_json::to_string(&selected_paths).unwrap()
    );
    let mut output2 = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, request2.as_bytes(), &mut output2).unwrap();
    let response2: Value = serde_json::from_slice(&output2).unwrap();
    let content2 = response2["result"]["content"][0]["text"].as_str().unwrap();
    let result2: Value = serde_json::from_str(content2).unwrap();

    assert!(result2["packed_files"].as_u64().unwrap() > 0);
    let packed_files = result2["files"].as_array().unwrap();
    // All selected files should be in the pack
    for path in &selected_paths {
        assert!(
            packed_files.iter().any(|f| f["path"].as_str().unwrap() == path),
            "selected file {} should be in pack",
            path
        );
    }
    // Content should be present
    for file in packed_files {
        assert!(
            !file["content"].as_str().unwrap().is_empty(),
            "packed file should have content"
        );
    }
}
```

**Step 2: Run test**

Run: `cargo test --lib commands::serve::tests::test_mcp_two_phase_handshake -- --nocapture`
Expected: PASS

**Step 3: Run full test suite + clippy + fmt**

```bash
cargo test --verbose
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

Expected: ALL PASS

**Step 4: Commit**

```bash
git add src/commands/serve.rs
git commit -m "test: add integration test for two-phase context handshake"
```

---

## Task 9: Version Bump + Final Verification

**Files:**
- Modify: `Cargo.toml` (version bump to 0.9.0)
- Modify: `plugin/.claude-plugin/plugin.json` (version bump)

**Step 1: Bump version**

In `Cargo.toml`, change `version = "0.8.1"` to `version = "0.9.0"`.
In `plugin/.claude-plugin/plugin.json`, change `"version": "0.8.1"` to `"version": "0.9.0"`.

**Step 2: Run full verification**

```bash
cargo test --verbose
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

Expected: ALL PASS, zero warnings

**Step 3: Commit**

```bash
git add Cargo.toml plugin/.claude-plugin/plugin.json
git commit -m "chore: bump version to 0.9.0"
```

---

## Summary

| Task | Component | New Files | Modified Files |
|------|-----------|-----------|----------------|
| 1 | Index extension | — | `src/index/mod.rs` |
| 2 | RelevanceScorer trait | `src/relevance/mod.rs`, `seed.rs`, `signals.rs` | `src/lib.rs` |
| 3 | Five signals | — | `src/relevance/signals.rs` |
| 4 | Seed selection | — | `src/relevance/seed.rs` |
| 5 | context_for_task tool | — | `src/commands/serve.rs` |
| 6 | pack_context tool | — | `src/commands/serve.rs` |
| 7 | Plugin wiring | `plugin/.mcp.json`, `plugin/lib/ensure-cxpak-serve`, `plugin/tests/mcp-wiring.bats` | — |
| 8 | Integration test | — | `src/commands/serve.rs` |
| 9 | Version bump | — | `Cargo.toml`, `plugin/.claude-plugin/plugin.json` |

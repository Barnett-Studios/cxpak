pub mod seed;
pub mod signals;

use crate::index::CodebaseIndex;
use std::collections::HashSet;

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
    pub expanded_tokens: Option<HashSet<String>>,
}

#[derive(Debug, Clone)]
pub struct SignalWeights {
    pub path_similarity: f64,
    pub symbol_match: f64,
    pub import_proximity: f64,
    pub term_frequency: f64,
    pub recency_boost: f64,
    pub pagerank: f64,
    /// Always present; value is 0.0 when embeddings are inactive.
    pub embedding_similarity: f64,
}

impl Default for SignalWeights {
    fn default() -> Self {
        Self::without_embeddings()
    }
}

impl SignalWeights {
    /// Weights used when an embedding index is available (7 active signals, sum = 1.0).
    pub fn with_embeddings() -> Self {
        Self {
            path_similarity: 0.15,
            symbol_match: 0.27,
            import_proximity: 0.12,
            term_frequency: 0.11,
            recency_boost: 0.05,
            pagerank: 0.15,
            embedding_similarity: 0.15,
        }
    }

    /// Weights used when no embedding index is present (sum = 1.0).
    pub fn without_embeddings() -> Self {
        Self {
            path_similarity: 0.18,
            symbol_match: 0.32,
            import_proximity: 0.14,
            term_frequency: 0.14,
            recency_boost: 0.05,
            pagerank: 0.17,
            embedding_similarity: 0.00,
        }
    }
}

impl Default for MultiSignalScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiSignalScorer {
    pub fn new() -> Self {
        Self {
            weights: SignalWeights::default(),
            expanded_tokens: None,
        }
    }

    /// Select weights based on whether the index has an embedding index.
    pub fn new_for_index(index: &CodebaseIndex) -> Self {
        let weights = if index.has_embedding_index() {
            SignalWeights::with_embeddings()
        } else {
            SignalWeights::without_embeddings()
        };
        Self {
            weights,
            expanded_tokens: None,
        }
    }

    pub fn with_weights(weights: SignalWeights) -> Self {
        Self {
            weights,
            expanded_tokens: None,
        }
    }

    pub fn with_expansion(mut self, tokens: HashSet<String>) -> Self {
        self.expanded_tokens = Some(tokens);
        self
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
        let expanded = self.expanded_tokens.as_ref();

        let path_sig = signals::path_similarity(query, file_path);
        let symbol_sig = signals::symbol_match(query, file_path, index, expanded);
        let import_sig = signals::import_proximity(file_path, index);
        let tf_sig = signals::term_frequency(query, file_path, index, expanded);
        let recency_sig = signals::recency_boost_signal(file_path, index);
        let pr_sig = signals::pagerank_signal(file_path, &index.pagerank);

        // Signal 7: embedding similarity (feature-gated, value 0.0 when inactive).
        #[cfg(feature = "embeddings")]
        let emb_sig = signals::embedding_similarity_signal(query, file_path, index);
        #[cfg(not(feature = "embeddings"))]
        let emb_sig = SignalResult {
            name: "embedding_similarity",
            score: 0.0,
            detail: "embeddings feature not enabled".to_string(),
        };

        let combined = w.path_similarity * path_sig.score
            + w.symbol_match * symbol_sig.score
            + w.import_proximity * import_sig.score
            + w.term_frequency * tf_sig.score
            + w.recency_boost * recency_sig.score
            + w.pagerank * pr_sig.score
            + w.embedding_similarity * emb_sig.score;

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
            signals: vec![
                path_sig,
                symbol_sig,
                import_sig,
                tf_sig,
                recency_sig,
                pr_sig,
                emb_sig,
            ],
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
            ScannedFile {
                relative_path: "src/api/mod.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 42,
            },
            ScannedFile {
                relative_path: "src/api/middleware.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 22,
            },
            ScannedFile {
                relative_path: "src/config.rs".into(),
                absolute_path: fp3,
                language: Some("rust".into()),
                size_bytes: 22,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/api/mod.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "handle_request".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn handle_request()".into(),
                    body: "{ rate_limit(); }".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/api/middleware.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "rate_limit".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn rate_limit()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        CodebaseIndex::build(files, parse_results, &counter)
    }

    #[test]
    fn test_multi_signal_scorer_returns_scores() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::new();
        let result = scorer.score("api request handler", "src/api/mod.rs", &index);
        assert!(result.score >= 0.0 && result.score <= 1.0);
        assert_eq!(result.signals.len(), 7);
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
            api_score.score,
            config_score.score
        );
    }

    #[test]
    fn test_weights_sum_to_one() {
        let w = SignalWeights::default();
        let sum = w.path_similarity
            + w.symbol_match
            + w.import_proximity
            + w.term_frequency
            + w.recency_boost
            + w.pagerank
            + w.embedding_similarity;
        assert!(
            (sum - 1.0).abs() < 0.001,
            "Weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn weights_without_embeddings_sum_to_one() {
        let w = SignalWeights::without_embeddings();
        let sum = w.path_similarity
            + w.symbol_match
            + w.import_proximity
            + w.term_frequency
            + w.recency_boost
            + w.pagerank
            + w.embedding_similarity;
        assert!(
            (sum - 1.0).abs() < 0.001,
            "without_embeddings weights should sum to 1.0, got {sum}"
        );
        assert_eq!(
            w.embedding_similarity, 0.0,
            "embedding_similarity must be 0.0 when inactive"
        );
    }

    #[test]
    fn weights_with_embeddings_sum_to_one() {
        let w = SignalWeights::with_embeddings();
        let sum = w.path_similarity
            + w.symbol_match
            + w.import_proximity
            + w.term_frequency
            + w.recency_boost
            + w.pagerank
            + w.embedding_similarity;
        assert!(
            (sum - 1.0).abs() < 0.001,
            "with_embeddings weights should sum to 1.0, got {sum}"
        );
        assert!(
            w.embedding_similarity > 0.0,
            "embedding_similarity must be positive when active"
        );
    }

    #[test]
    fn scorer_selects_correct_weights() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::new_for_index(&index);
        // Index was built without embeddings (local provider would fail in tests),
        // so we expect without_embeddings weights.
        assert_eq!(
            scorer.weights.embedding_similarity, 0.0,
            "no embedding index => embedding_similarity weight must be 0.0"
        );
        // Ensure the total still sums to 1.0.
        let w = &scorer.weights;
        let sum = w.path_similarity
            + w.symbol_match
            + w.import_proximity
            + w.term_frequency
            + w.recency_boost
            + w.pagerank
            + w.embedding_similarity;
        assert!(
            (sum - 1.0).abs() < 0.001,
            "scorer weights should sum to 1.0, got {sum}"
        );
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
            pagerank: 0.0,
            embedding_similarity: 0.0,
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

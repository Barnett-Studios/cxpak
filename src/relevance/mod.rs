pub mod identifier;
#[cfg(feature = "reranker")]
pub mod reranker;
pub mod rrf;
pub mod seed;
pub mod signals;

use crate::index::CodebaseIndex;
use std::collections::HashSet;

/// A/B control for the D1 semantic upgrade (ADR-0184).
///
/// The RRF fusion + contextual retrieval are switchable so that `Inert`
/// reproduces the pre-D1 weighted-sum ranking **byte-for-byte**, and `Active`
/// enables the upgrade. Both must be reachable from a SINGLE index build so the
/// D2 recall gate can be measured index-once (the C2 lesson: two-build harness
/// runs get reaped). A harness constructs one scorer per mode over the same
/// `CodebaseIndex` and scores both — no temporary measurement code required, the
/// control ships in the product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RelevanceMode {
    /// Weighted-sum combine (`Σ wᵢ·signalᵢ`) — the pre-D1 ranking, byte-identical.
    #[default]
    Inert,
    /// RRF fusion over the per-signal sub-rankings (ADR-0184). At index time this
    /// mode also enables the contextual-retrieval header on embedded text.
    Active,
}

impl RelevanceMode {
    /// Whether contextual-retrieval enrichment applies at index time in this
    /// mode. `Active` prepends the deterministic graph-context header to the
    /// embedded text; `Inert` embeds the bare signature (pre-D1, byte-identical).
    pub fn contextual(self) -> bool {
        matches!(self, RelevanceMode::Active)
    }
}

/// The relevance mode the shipped product uses.
///
/// Held at [`RelevanceMode::Active`] as of Phase R (ADR-0187): the full-corpus
/// recall A/B (31 runnable PRs — ripgrep/flask/express) measured Active (RRF
/// fusion) at +0.30 recall over Inert at both the 8k and 32k budgets (+164%
/// overall; 17 wins / 12 ties / 2 losses), and the controller+user gate approved
/// the flip. [`RelevanceMode::Inert`] remains available as a byte-stable control
/// via [`crate::relevance::MultiSignalScorer::with_mode`] and
/// [`crate::auto_context::auto_context_with_mode`].
pub const DEFAULT_RELEVANCE_MODE: RelevanceMode = RelevanceMode::Active;

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
    /// A/B control: `Inert` = weighted sum (pre-D1), `Active` = RRF fusion.
    pub mode: RelevanceMode,
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
            mode: DEFAULT_RELEVANCE_MODE,
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
            mode: DEFAULT_RELEVANCE_MODE,
        }
    }

    pub fn with_weights(weights: SignalWeights) -> Self {
        Self {
            weights,
            expanded_tokens: None,
            mode: DEFAULT_RELEVANCE_MODE,
        }
    }

    pub fn with_expansion(mut self, tokens: HashSet<String>) -> Self {
        self.expanded_tokens = Some(tokens);
        self
    }

    /// Set the A/B mode (`Inert` weighted sum vs `Active` RRF fusion).
    ///
    /// The D2 recall harness uses this to score both modes from a SINGLE index
    /// build: `MultiSignalScorer::new_for_index(idx).with_mode(Inert)` and
    /// `…with_mode(Active)` over the same `idx`.
    pub fn with_mode(mut self, mode: RelevanceMode) -> Self {
        self.mode = mode;
        self
    }

    /// Score all files in the index against the query.
    ///
    /// The query embedding is computed once here and reused for every file,
    /// avoiding per-file provider creation and embed calls.
    pub fn score_all(&self, query: &str, index: &CodebaseIndex) -> Vec<ScoredFile> {
        // Compute the query embedding a single time before the per-file loop.
        #[cfg(feature = "embeddings")]
        let query_embedding: Option<Vec<f32>> = {
            use crate::embeddings::{create_provider, EmbeddingConfig};
            create_provider(EmbeddingConfig::local_default())
                .ok()
                .and_then(|p| p.embed(query).ok())
        };
        #[cfg(not(feature = "embeddings"))]
        let query_embedding: Option<Vec<f32>> = None;

        // Identifier-level ranking (C2, ADR-0181): a single global pass over the
        // codebase's `(file, identifier)` units yields a boost-only per-file
        // factor fused with the conventions DNA. Built once here (it needs global
        // scope: ambiguity counts, personalized PageRank, cross-file
        // normalization) and shared across the per-file loop, mirroring the query
        // embedding. Uses the same expanded/normalized tokens the other signals
        // consume so identifier matching is consistent.
        let query_tokens: std::collections::HashSet<String> = self
            .expanded_tokens
            .clone()
            .unwrap_or_else(|| signals::tokenize(query));
        let ident_ranking = identifier::build_identifier_ranking(index, &query_tokens);

        // Pass 1: compute the 7 raw per-file signal sets. Shared by both modes;
        // RRF needs the WHOLE population to assign per-signal ranks, so signals
        // are materialized before combining (unlike the inert per-file map).
        let signal_sets: Vec<(String, [SignalResult; 7], usize)> = index
            .files
            .iter()
            .map(|f| {
                let (sigs, token_count) = self.compute_signals(
                    query,
                    &f.relative_path,
                    index,
                    query_embedding.as_deref(),
                );
                (f.relative_path.clone(), sigs, token_count)
            })
            .collect();

        // Pass 2: combine into a base score per mode.
        //   Inert  → weighted sum (byte-identical to pre-D1).
        //   Active → weighted, scale-normalized RRF over the per-signal ranks.
        let base_scores: Vec<f64> = match self.mode {
            RelevanceMode::Inert => signal_sets
                .iter()
                .map(|(_, sigs, _)| self.weighted_sum(sigs))
                .collect(),
            RelevanceMode::Active => {
                let files: Vec<(String, [f64; rrf::N_SIGNALS])> = signal_sets
                    .iter()
                    .map(|(path, sigs, _)| {
                        let mut arr = [0.0_f64; rrf::N_SIGNALS];
                        for (i, s) in sigs.iter().enumerate() {
                            arr[i] = s.score;
                        }
                        (path.clone(), arr)
                    })
                    .collect();
                rrf::fuse(&files, self.weight_array(), rrf::RRF_K)
            }
        };

        // Pass 3: finalize (apply the boost-only identifier factor + clamp).
        // `mut` is only consumed by the feature-gated reranker below.
        #[cfg_attr(not(feature = "reranker"), allow(unused_mut))]
        let mut scored: Vec<ScoredFile> = signal_sets
            .into_iter()
            .zip(base_scores)
            .map(|((path, sigs, token_count), base)| {
                self.finalize(path, sigs, token_count, base, Some(&ident_ranking))
            })
            .collect();

        // Optional local reranker (feature-gated, Active only): re-orders the
        // top-N fused candidates. Excluded from the determinism fixture (default
        // features + Inert mode never reach here). See ADR-0184.
        #[cfg(feature = "reranker")]
        if self.mode == RelevanceMode::Active {
            reranker::rerank(query, &mut scored, index, reranker::DEFAULT_TOP_N);
        }

        scored
    }

    /// Compute the 7 raw signal results for one file, in the fixed order
    /// `[path, symbol, import, tf, recency, pagerank, embedding]`, plus its token
    /// count. No combining — that is mode-specific.
    fn compute_signals(
        &self,
        query: &str,
        file_path: &str,
        index: &CodebaseIndex,
        query_embedding: Option<&[f32]>,
    ) -> ([SignalResult; 7], usize) {
        let expanded = self.expanded_tokens.as_ref();

        let path_sig = signals::path_similarity(query, file_path);
        let symbol_sig = signals::symbol_match(query, file_path, index, expanded);
        let import_sig = signals::import_proximity(file_path, index);
        let tf_sig = signals::term_frequency(query, file_path, index, expanded);
        let recency_sig = signals::recency_boost_signal(file_path, index);
        let pr_sig = signals::pagerank_signal(file_path, &index.pagerank);

        // Signal 7: embedding similarity (feature-gated, value 0.0 when inactive).
        #[cfg(feature = "embeddings")]
        let emb_sig = signals::embedding_similarity_signal(query_embedding, file_path, index);
        #[cfg(not(feature = "embeddings"))]
        let emb_sig = {
            let _ = query_embedding;
            SignalResult {
                name: "embedding_similarity",
                score: 0.0,
                detail: "embeddings feature not enabled".to_string(),
            }
        };

        let token_count = index
            .files
            .iter()
            .find(|f| f.relative_path == file_path)
            .map(|f| f.token_count)
            .unwrap_or(0);

        (
            [
                path_sig,
                symbol_sig,
                import_sig,
                tf_sig,
                recency_sig,
                pr_sig,
                emb_sig,
            ],
            token_count,
        )
    }

    /// The inert weighted sum `Σ wᵢ·signalᵢ` — byte-identical to the pre-D1
    /// combine (same operand order). The RRF path never calls this.
    fn weighted_sum(&self, sigs: &[SignalResult; 7]) -> f64 {
        let w = &self.weights;
        w.path_similarity * sigs[0].score
            + w.symbol_match * sigs[1].score
            + w.import_proximity * sigs[2].score
            + w.term_frequency * sigs[3].score
            + w.recency_boost * sigs[4].score
            + w.pagerank * sigs[5].score
            + w.embedding_similarity * sigs[6].score
    }

    /// The 7 signal weights as a fixed-order array, aligned with the signal order
    /// [`compute_signals`] produces — the weight vector RRF fuses with.
    fn weight_array(&self) -> [f64; rrf::N_SIGNALS] {
        let w = &self.weights;
        [
            w.path_similarity,
            w.symbol_match,
            w.import_proximity,
            w.term_frequency,
            w.recency_boost,
            w.pagerank,
            w.embedding_similarity,
        ]
    }

    /// Apply the boost-only identifier factor (C2, ADR-0181) to a base score,
    /// clamp to `[0, 1]`, and assemble the `ScoredFile` with the 8-signal
    /// breakdown (the 7 base signals + `identifier_rank`). Shared by both modes.
    fn finalize(
        &self,
        file_path: String,
        sigs: [SignalResult; 7],
        token_count: usize,
        base_score: f64,
        ident_ranking: Option<&identifier::IdentifierRanking>,
    ) -> ScoredFile {
        // Signal 8: identifier-level ranking (C2, ADR-0181). Boost-only multiplier
        // (factor >= 1.0) so a file whose identifiers match the query and conform
        // to the conventions DNA is lifted while no file can be demoted — keeping
        // the D2 recall gate monotone. Defaults to the neutral 1.0 on the
        // single-file scoring path (no global ranking available).
        let ident_factor = ident_ranking.map(|r| r.factor(&file_path)).unwrap_or(1.0);
        let ident_signal = ident_ranking.map(|r| r.signal(&file_path)).unwrap_or(0.0);
        let ident_sig = SignalResult {
            name: "identifier_rank",
            score: ident_signal,
            detail: format!("factor={:.3}, signal={:.3}", ident_factor, ident_signal),
        };

        let score = (base_score * ident_factor).clamp(0.0, 1.0);

        let [path_sig, symbol_sig, import_sig, tf_sig, recency_sig, pr_sig, emb_sig] = sigs;

        ScoredFile {
            path: file_path,
            score,
            signals: vec![
                path_sig,
                symbol_sig,
                import_sig,
                tf_sig,
                recency_sig,
                pr_sig,
                emb_sig,
                ident_sig,
            ],
            token_count,
        }
    }

    /// Single-file scoring (the [`RelevanceScorer`] trait path): always the inert
    /// weighted sum. RRF needs the whole population to assign ranks, so it is
    /// only available via [`score_all`]; single-file scoring stays inert
    /// (documented, mirroring C2's neutral identifier factor on this path).
    fn score_with_embedding(
        &self,
        query: &str,
        file_path: &str,
        index: &CodebaseIndex,
        query_embedding: Option<&[f32]>,
        ident_ranking: Option<&identifier::IdentifierRanking>,
    ) -> ScoredFile {
        let (sigs, token_count) = self.compute_signals(query, file_path, index, query_embedding);
        let base = self.weighted_sum(&sigs);
        self.finalize(
            file_path.to_string(),
            sigs,
            token_count,
            base,
            ident_ranking,
        )
    }
}

impl RelevanceScorer for MultiSignalScorer {
    fn score(&self, query: &str, file_path: &str, index: &CodebaseIndex) -> ScoredFile {
        // Single-file scoring path: no cached embedding and no global identifier
        // ranking available. Use score_all for bulk scoring to share the query
        // embedding and the identifier-ranking pass across files.
        self.score_with_embedding(query, file_path, index, None, None)
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
        // 7 weighted signals + the identifier_rank signal (C2).
        assert_eq!(result.signals.len(), 8);
        assert_eq!(result.signals.last().unwrap().name, "identifier_rank");
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

    // ── D1: A/B control (RelevanceMode) tests ──────────────────────────────

    #[test]
    fn default_mode_is_active() {
        // Post-flip (ADR-0187): the shipped default is Active (RRF fusion), which
        // the full-corpus recall A/B measured at +164% recall over Inert. The
        // freshly-constructed scorer must reflect that default.
        assert_eq!(DEFAULT_RELEVANCE_MODE, RelevanceMode::Active);
        let scorer = MultiSignalScorer::new();
        assert_eq!(scorer.mode, RelevanceMode::Active);
    }

    #[test]
    fn inert_mode_score_all_agrees_with_single_score() {
        // The Inert score_all path must produce the EXACT same f64 scores as the
        // single-file weighted-sum path (`score`) — i.e. the batch and per-file
        // entry points agree bit-for-bit. This proves path-agreement within the
        // Inert weighted sum. There is no separate frozen pre-D1 ranking baseline:
        // Inert IS the pre-D1 weighted-sum ranking by construction (the D1 A/B
        // control), and the spa golden covers the visual/PageRank path only — not
        // the relevance scorer — so nothing here asserts against a pre-D1 snapshot.
        let index = make_test_index();
        let scorer = MultiSignalScorer::new_for_index(&index).with_mode(RelevanceMode::Inert);
        let all = scorer.score_all("api request handler", &index);
        for sf in &all {
            let single = scorer.score("api request handler", &sf.path, &index);
            assert_eq!(
                sf.score.to_bits(),
                single.score.to_bits(),
                "inert score_all must byte-match the weighted sum for {}",
                sf.path
            );
        }
    }

    #[test]
    fn active_mode_enables_rrf_and_stays_in_unit_range() {
        // Active mode must produce a valid [0,1] ranking and generally reorder
        // vs inert (RRF fuses ranks, not magnitudes).
        let index = make_test_index();
        let inert = MultiSignalScorer::new_for_index(&index).with_mode(RelevanceMode::Inert);
        let active = MultiSignalScorer::new_for_index(&index).with_mode(RelevanceMode::Active);

        let inert_scores = inert.score_all("rate limit", &index);
        let active_scores = active.score_all("rate limit", &index);
        assert_eq!(inert_scores.len(), active_scores.len());
        for sf in &active_scores {
            assert!(
                sf.score >= 0.0 && sf.score <= 1.0,
                "active RRF score out of [0,1]: {} = {}",
                sf.path,
                sf.score
            );
            assert_eq!(sf.signals.len(), 8, "8-signal breakdown preserved");
        }

        // The relevant file should still outrank the irrelevant one under RRF.
        let get = |v: &[ScoredFile], p: &str| v.iter().find(|s| s.path == p).unwrap().score;
        assert!(
            get(&active_scores, "src/api/middleware.rs") >= get(&active_scores, "src/config.rs"),
            "RRF should keep the rate-limit file at or above config.rs"
        );
    }

    #[test]
    fn active_mode_is_deterministic() {
        let index = make_test_index();
        let scorer = MultiSignalScorer::new_for_index(&index).with_mode(RelevanceMode::Active);
        let a = scorer.score_all("api request handler", &index);
        let b = scorer.score_all("api request handler", &index);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.path, y.path);
            assert_eq!(
                x.score.to_bits(),
                y.score.to_bits(),
                "active mode must be byte-stable"
            );
        }
    }

    #[test]
    fn relevance_mode_contextual_flag() {
        assert!(!RelevanceMode::Inert.contextual());
        assert!(RelevanceMode::Active.contextual());
    }

    #[cfg(not(feature = "reranker"))]
    #[test]
    fn active_ranking_equals_pre_reranker_when_feature_off() {
        // With the reranker feature OFF (the default build), the active ranking
        // IS the pure fused ranking — nothing re-orders it. This locks the
        // "when off, ranking == pre-reranker" contract on the default surface.
        let index = make_test_index();
        let active = MultiSignalScorer::new_for_index(&index).with_mode(RelevanceMode::Active);
        let scored = active.score_all("rate limit", &index);
        // Recompute the fused base independently and confirm the finalize step
        // (ident factor 0.0 gain ⇒ factor 1.0) leaves scores == fused base.
        let signal_sets: Vec<(String, [f64; rrf::N_SIGNALS])> = index
            .files
            .iter()
            .map(|f| {
                let (sigs, _) =
                    active.compute_signals("rate limit", &f.relative_path, &index, None);
                let mut arr = [0.0; rrf::N_SIGNALS];
                for (i, s) in sigs.iter().enumerate() {
                    arr[i] = s.score;
                }
                (f.relative_path.clone(), arr)
            })
            .collect();
        let fused = rrf::fuse(&signal_sets, active.weight_array(), rrf::RRF_K);
        for (sf, (_, _arr)) in scored.iter().zip(signal_sets.iter()) {
            let f = fused[signal_sets.iter().position(|(p, _)| p == &sf.path).unwrap()];
            assert_eq!(
                sf.score.to_bits(),
                f.clamp(0.0, 1.0).to_bits(),
                "no reranker ⇒ score == clamped fused base for {}",
                sf.path
            );
        }
    }
}

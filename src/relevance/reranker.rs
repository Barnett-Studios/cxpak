//! Optional local reranker (cxpak 3.0.0, Phase D — ADR-0184).
//!
//! Behind the NON-default `reranker` Cargo feature. When the feature is off this
//! module does not compile and no code path calls it, so the core ranking (RRF
//! or the inert weighted sum) is exactly what ships and the determinism golden
//! is unaffected. The reranker is likewise **excluded from the determinism
//! fixture**: the fixture builds with default features, and even with the
//! feature on the reranker only fires in [`RelevanceMode::Active`].
//!
//! ## What it is (and is NOT)
//!
//! It is a *local, deterministic, no-LLM, no-new-dependency* cross-encoder: a
//! second pass that re-scores the top-N fused candidates by JOINTLY featurizing
//! the query against each file (token overlap between the query and the file's
//! symbol names + path, weighted by symbol visibility). A classic bi-encoder
//! (the embedding signal) scores query and document independently; a
//! cross-encoder looks at the (query, document) pair together — which is exactly
//! what the joint token-overlap features do here, without a neural model.
//!
//! It is NOT a model-backed transformer cross-encoder. A model-backed reranker
//! (e.g. a local ONNX/candle cross-encoder such as `bge-reranker`) is a future
//! extension that would live behind THIS SAME feature flag; it was deliberately
//! not added now because it needs a heavy model download / native runtime, which
//! the brief requires stopping to confirm before adding (ADR-0163: no OpenSSL,
//! keep the dependency tree lean). The lexical cross-encoder captures the
//! re-ranking *architecture* (top-N re-order after fusion) with zero new deps.
//!
//! ## Recall safety
//!
//! The reranker only re-orders the top-N candidates *among themselves*: it never
//! drops a candidate and never promotes anything from outside the top-N, so the
//! set of files above any prefix ≥ N is unchanged. It reassigns the reordered
//! candidates the SAME multiset of scores they already held (the top-N scores in
//! descending order), so they stay the top-N and remain above the seed
//! threshold — only their internal order changes.
//!
//! Scope of the guarantee: it cannot regress the ranking *set* at any prefix
//! ≥ N. It does NOT guarantee recall at a smaller *budget* cut inside the top-N —
//! a downstream token budget that admits only the first k < N files can drop a
//! file the reranker demoted within the top-N. The invariant is "no set change
//! above the top-N boundary", not "no file loss under an arbitrary budget cut".

use super::{signals, ScoredFile};
use crate::index::CodebaseIndex;
use crate::intelligence::pagerank::{build_symbol_cross_refs, symbol_importance};
use std::collections::HashSet;

/// Default number of top candidates the reranker re-orders.
pub const DEFAULT_TOP_N: usize = 20;

/// Re-order the top-N entries of `scored` (assumed already fused, but NOT yet
/// sorted) by a deterministic lexical cross-encoder score, in place.
///
/// `scored` is the full per-file fused list. We select the `top_n` highest by
/// fused score (score desc, path asc tiebreak), compute a joint query/document
/// re-score for each, and rewrite those entries so the re-ranked order carries
/// the original top-N scores in descending order. All other entries are
/// untouched. Deterministic: every ordering uses `f64::total_cmp` + a path
/// tiebreak; no `HashMap` iteration feeds an order.
pub fn rerank(query: &str, scored: &mut [ScoredFile], index: &CodebaseIndex, top_n: usize) {
    if scored.len() < 2 || top_n < 2 {
        return;
    }

    let query_tokens = signals::tokenize(query);
    if query_tokens.is_empty() {
        return;
    }

    // Identify the current top-N by fused score (desc), path asc tiebreak.
    let mut idx_order: Vec<usize> = (0..scored.len()).collect();
    idx_order.sort_by(|&a, &b| {
        scored[b]
            .score
            .total_cmp(&scored[a].score)
            .then_with(|| scored[a].path.cmp(&scored[b].path))
    });
    let n = top_n.min(idx_order.len());
    let top_indices: Vec<usize> = idx_order[..n].to_vec();

    // The pool of scores we will redistribute: the top-N fused scores, sorted
    // descending so the re-ranked winner keeps the highest existing score.
    let mut score_pool: Vec<f64> = top_indices.iter().map(|&i| scored[i].score).collect();
    score_pool.sort_by(|a, b| b.total_cmp(a));

    // Cross-encoder re-score each top-N candidate.
    let cross_refs = build_symbol_cross_refs(&index.term_frequencies);
    let mut reranked: Vec<(usize, f64)> = top_indices
        .iter()
        .map(|&i| {
            let ce = cross_encode(&query_tokens, &scored[i].path, index, &cross_refs);
            (i, ce)
        })
        .collect();
    // Order by cross-encoder score desc, path asc tiebreak (deterministic).
    reranked.sort_by(|&(ia, sa), &(ib, sb)| {
        sb.total_cmp(&sa)
            .then_with(|| scored[ia].path.cmp(&scored[ib].path))
    });

    // Reassign the descending score pool to the re-ranked candidates.
    for (rank, &(orig_idx, _ce)) in reranked.iter().enumerate() {
        scored[orig_idx].score = score_pool[rank];
    }
}

/// Joint query/document relevance: token overlap between the query and the
/// file's symbol names + path components, each symbol weighted by its
/// importance (visibility × reference count). This is the "cross" step — the
/// query and document are featurized together, not independently embedded.
fn cross_encode(
    query_tokens: &HashSet<String>,
    file_path: &str,
    index: &CodebaseIndex,
    cross_refs: &std::collections::HashMap<String, HashSet<String>>,
) -> f64 {
    let file = match index.files.iter().find(|f| f.relative_path == file_path) {
        Some(f) => f,
        None => return 0.0,
    };

    let mut score = 0.0_f64;

    // Path-token overlap (a modest, always-available joint feature).
    let path_tokens: HashSet<String> = file_path
        .split(['/', '.', '_', '-'])
        .flat_map(crate::index::split_identifier)
        .filter(|t| t.len() >= 2)
        .collect();
    let path_hits = path_tokens
        .iter()
        .filter(|t| query_tokens.contains(*t))
        .count();
    score += path_hits as f64;

    // Symbol-name overlap, weighted by symbol importance so a query term hitting
    // a prominent public symbol counts for more than one hitting an obscure one.
    if let Some(pr) = &file.parse_result {
        for sym in &pr.symbols {
            let sym_tokens = signals::tokenize(&sym.name);
            let overlap = sym_tokens
                .iter()
                .filter(|t| query_tokens.contains(*t))
                .count();
            if overlap > 0 {
                let w = symbol_importance(sym, 1.0, cross_refs, file_path);
                score += overlap as f64 * w;
            }
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn sym(name: &str) -> Symbol {
        Symbol {
            name: name.into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: format!("pub fn {name}()"),
            body: "{}".into(),
            start_line: 1,
            end_line: 1,
        }
    }

    fn build(files: &[(&str, &str, Vec<Symbol>)]) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let mut scanned = Vec::new();
        let mut parse_results = HashMap::new();
        for (path, src, symbols) in files {
            let abs = dir.path().join(path);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(&abs, src).unwrap();
            scanned.push(ScannedFile {
                relative_path: (*path).into(),
                absolute_path: abs,
                language: Some("rust".into()),
                size_bytes: src.len() as u64,
            });
            parse_results.insert(
                (*path).to_string(),
                ParseResult {
                    symbols: symbols.clone(),
                    imports: vec![],
                    exports: vec![],
                },
            );
        }
        CodebaseIndex::build(scanned, parse_results, &counter)
    }

    #[test]
    fn rerank_promotes_query_matching_file_within_top_n() {
        // Two files carry the same fused score, but only "src/rate.rs" has a
        // symbol matching the query "rate limit". The reranker must reorder so
        // the query-matching file takes the higher of the two pooled scores.
        let index = build(&[
            (
                "src/aaa.rs",
                "pub fn unrelated_thing() {}",
                vec![sym("unrelated_thing")],
            ),
            (
                "src/rate.rs",
                "pub fn rate_limit() {}",
                vec![sym("rate_limit")],
            ),
        ]);
        let mut scored = vec![
            // aaa sorts first by path and holds the top score pre-rerank.
            ScoredFile {
                path: "src/aaa.rs".into(),
                score: 0.5,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/rate.rs".into(),
                score: 0.4,
                signals: vec![],
                token_count: 10,
            },
        ];
        rerank("rate limit", &mut scored, &index, DEFAULT_TOP_N);
        let rate = scored.iter().find(|s| s.path == "src/rate.rs").unwrap();
        let aaa = scored.iter().find(|s| s.path == "src/aaa.rs").unwrap();
        assert!(
            rate.score > aaa.score,
            "query-matching file should take the higher pooled score: rate={} aaa={}",
            rate.score,
            aaa.score
        );
        // Score pool is conserved: the same two scores, just reassigned.
        let mut pool: Vec<f64> = scored.iter().map(|s| s.score).collect();
        pool.sort_by(|a, b| b.total_cmp(a));
        assert_eq!(pool, vec![0.5, 0.4]);
    }

    #[test]
    fn rerank_is_deterministic() {
        let index = build(&[
            ("src/a.rs", "pub fn alpha() {}", vec![sym("alpha")]),
            (
                "src/b.rs",
                "pub fn rate_limit() {}",
                vec![sym("rate_limit")],
            ),
        ]);
        let base = vec![
            ScoredFile {
                path: "src/a.rs".into(),
                score: 0.5,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/b.rs".into(),
                score: 0.4,
                signals: vec![],
                token_count: 10,
            },
        ];
        let mut r1 = base.clone();
        let mut r2 = base.clone();
        rerank("rate limit", &mut r1, &index, DEFAULT_TOP_N);
        rerank("rate limit", &mut r2, &index, DEFAULT_TOP_N);
        let s1: Vec<(String, f64)> = r1.iter().map(|s| (s.path.clone(), s.score)).collect();
        let s2: Vec<(String, f64)> = r2.iter().map(|s| (s.path.clone(), s.score)).collect();
        assert_eq!(s1, s2);
    }

    #[test]
    fn rerank_noop_on_empty_query() {
        let index = build(&[("src/a.rs", "pub fn alpha() {}", vec![sym("alpha")])]);
        let mut scored = vec![
            ScoredFile {
                path: "src/a.rs".into(),
                score: 0.5,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/b.rs".into(),
                score: 0.4,
                signals: vec![],
                token_count: 10,
            },
        ];
        let before: Vec<f64> = scored.iter().map(|s| s.score).collect();
        rerank("   ", &mut scored, &index, DEFAULT_TOP_N);
        let after: Vec<f64> = scored.iter().map(|s| s.score).collect();
        assert_eq!(before, after, "empty query must be a no-op");
    }
}

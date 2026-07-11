//! Deterministic interpretive insights (ADR-0194) — proven analogues of an
//! LLM-narrated codebase, computed from existing signals with zero inference.
//!
//! The flagship is [`surprising_connections`]: file pairs that change together
//! but have no direct import edge — computed today by no renderer, surfaced
//! here for the Overview. Correlational, so honestly labelled `~ estimated` at
//! the UI layer (the proof-tick contract, ADR-0193).

use crate::index::CodebaseIndex;
use serde::Serialize;

/// A co-change pair with no corresponding Import edge in the dependency graph.
///
/// `co_change_score` is the recency-decayed strength (`recency_weight`) carried
/// on the source [`crate::intelligence::co_change::CoChangeEdge`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SurprisingLink {
    pub a: String,
    pub b: String,
    pub co_change_score: f64,
}

/// Return every `index.co_changes` pair whose unordered `(file_a, file_b)` has
/// **no** Import edge in `index.graph` (in either direction), scored by the
/// edge's `recency_weight`, sorted by descending score then `(a, b)`.
pub fn surprising_connections(index: &CodebaseIndex) -> Vec<SurprisingLink> {
    let mut links: Vec<SurprisingLink> = index
        .co_changes
        .iter()
        .filter(|e| {
            let a_imports_b = index
                .graph
                .dependencies(&e.file_a)
                .is_some_and(|deps| deps.iter().any(|t| t.target == e.file_b));
            let b_imports_a = index
                .graph
                .dependencies(&e.file_b)
                .is_some_and(|deps| deps.iter().any(|t| t.target == e.file_a));
            !(a_imports_b || b_imports_a)
        })
        .map(|e| SurprisingLink {
            a: e.file_a.clone(),
            b: e.file_b.clone(),
            co_change_score: e.recency_weight,
        })
        .collect();
    links.sort_by(|l, r| {
        r.co_change_score
            .partial_cmp(&l.co_change_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| l.a.cmp(&r.a))
            .then_with(|| l.b.cmp(&r.b))
    });
    links
}

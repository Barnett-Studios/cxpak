//! Deterministic interpretive insights (ADR-0175) — proven analogues of an
//! LLM-narrated codebase, computed from existing signals with zero inference.
//!
//! The flagship is [`surprising_connections`]: file pairs that change together
//! but have no direct import edge — computed today by no renderer, surfaced
//! here for the Overview. Correlational, so honestly labelled `~ estimated` at
//! the UI layer (the proof-tick contract, ADR-0174).

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
    // RED baseline (node N1): the real filter is delegated to the local
    // cascade. Returns an empty set so the acceptance test fails on the
    // "unimported pair must surface" assertion until the body is filled.
    let _ = index;
    Vec::new()
}

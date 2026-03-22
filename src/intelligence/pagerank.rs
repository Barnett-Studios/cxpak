use crate::index::graph::DependencyGraph;
use std::collections::{HashMap, HashSet};

/// Compute PageRank scores for all files in the dependency graph.
///
/// Uses standard PageRank with dangling node redistribution.
/// Forward edges only: if A imports B, A transfers rank to B.
/// Scores normalized to 0.0–1.0 range (divided by max).
pub fn compute_pagerank(
    _graph: &DependencyGraph,
    _damping: f64,
    _max_iterations: usize,
) -> HashMap<String, f64> {
    HashMap::new() // Stub — implemented in Task 3
}

/// Build inverted index: symbol_name → set of files containing it.
/// Used for O(1) cross-reference lookups in symbol_importance().
pub fn build_symbol_cross_refs(
    _term_frequencies: &HashMap<String, HashMap<String, u32>>,
) -> HashMap<String, HashSet<String>> {
    HashMap::new() // Stub — implemented in Task 4
}

/// Compute importance score for a single symbol.
/// importance = file_pagerank * symbol_weight
/// where symbol_weight is 1.0 (public+referenced), 0.7 (public), or 0.3 (private).
pub fn symbol_importance(
    _symbol: &crate::parser::language::Symbol,
    _file_pagerank: f64,
    _cross_refs: &HashMap<String, HashSet<String>>,
    _file_path: &str,
) -> f64 {
    0.0 // Stub — implemented in Task 4
}

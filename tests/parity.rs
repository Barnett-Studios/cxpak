//! Parity tests: the incremental / warm-started / cached index paths must
//! produce structurally identical results to a full rebuild (within float
//! epsilon for PageRank). These are the definition of done for W1 — never
//! weaken an assertion to make one pass.

use cxpak::index::graph::{DependencyGraph, EdgeType};
use cxpak::intelligence::pagerank::{compute_pagerank, compute_pagerank_seeded};
use std::collections::HashMap;

/// Build a small dependency graph from `(from, to)` import edges.
fn graph_with_edges(edges: &[(&str, &str)]) -> DependencyGraph {
    let mut g = DependencyGraph::new();
    for (from, to) in edges {
        g.add_edge(from, to, EdgeType::Import);
    }
    g
}

fn top_k(m: &HashMap<String, f64>, k: usize) -> Vec<String> {
    let mut v: Vec<_> = m.iter().map(|(k, &s)| (k.clone(), s)).collect();
    // score desc, path asc as a deterministic tiebreak
    v.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    v.into_iter().take(k).map(|(k, _)| k).collect()
}

#[test]
fn warm_pagerank_matches_cold() {
    let g = graph_with_edges(&[("a", "b"), ("b", "c"), ("c", "a"), ("a", "c")]);
    let cold = compute_pagerank(&g, 0.85, 100);
    // Seeding the iteration from the converged cold result must reach the same
    // fixed point (PageRank's stationary distribution is unique for a graph).
    let warm = compute_pagerank_seeded(&g, 0.85, 100, &cold);
    for (k, &cv) in &cold {
        assert!(
            (warm[k] - cv).abs() <= 2e-6,
            "node {k}: warm {} cold {}",
            warm[k],
            cv
        );
    }
    // identical top-K ordering
    assert_eq!(top_k(&cold, 3), top_k(&warm, 3));
}

#[test]
fn empty_seed_equals_cold() {
    // An empty seed must reproduce exactly today's 1/N-initialised behaviour.
    let g = graph_with_edges(&[("a", "b"), ("b", "c"), ("c", "a")]);
    let cold = compute_pagerank(&g, 0.85, 100);
    let seeded_empty = compute_pagerank_seeded(&g, 0.85, 100, &HashMap::new());
    for (k, &cv) in &cold {
        assert!((seeded_empty[k] - cv).abs() <= 1e-12, "node {k} diverged");
    }
}

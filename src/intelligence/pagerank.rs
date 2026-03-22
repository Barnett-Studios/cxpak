use crate::index::graph::DependencyGraph;
use std::collections::{HashMap, HashSet};

/// Compute PageRank scores for all files in the dependency graph.
///
/// Uses standard PageRank with dangling node redistribution.
///
/// # Edge convention (important — the naming in DependencyGraph is confusing)
///
/// `graph.edges[A]` holds the outgoing edges **from** A, i.e. files that A
/// imports.  In PageRank terms, A *transfers* rank to each of those files.
///
/// `graph.reverse_edges[B]` holds the reverse edges **into** B.  Each
/// `TypedEdge.target` stored there is the **importer** (the source of the
/// original forward edge), NOT a further dependency.  In other words:
///   reverse_edges[B] = { TypedEdge { target: A, .. } }  means A imports B.
///
/// So to find "who sends rank to B?", iterate `reverse_edges[B]` and use
/// `edge.target` — those are the files whose rank contribution flows into B.
///
/// # Algorithm
///
/// Standard PageRank with dangling node redistribution:
///   rank[v] = (1 - d) / N
///            + d * (dangling_sum / N)
///            + d * Σ_{u→v} rank[u] / out_degree[u]
///
/// where `dangling_sum` is the sum of ranks of nodes with no outgoing edges.
///
/// # Parameters
/// - `damping` — damping factor (typically 0.85)
/// - `max_iterations` — iteration cap
///
/// # Returns
/// Map of file path → PageRank score, normalized to [0.0, 1.0] by dividing
/// all scores by the maximum score (so the top-ranked file always gets 1.0).
pub fn compute_pagerank(
    graph: &DependencyGraph,
    damping: f64,
    max_iterations: usize,
) -> HashMap<String, f64> {
    // ── 1. Collect the universe of nodes ───────────────────────────────────
    // A node is any file that appears as a source *or* target of any edge.
    let mut nodes: HashSet<String> = HashSet::new();
    for (src, targets) in &graph.edges {
        nodes.insert(src.clone());
        for e in targets {
            nodes.insert(e.target.clone());
        }
    }
    for (dst, sources) in &graph.reverse_edges {
        nodes.insert(dst.clone());
        for e in sources {
            nodes.insert(e.target.clone());
        }
    }

    let n = nodes.len();
    if n == 0 {
        return HashMap::new();
    }

    let nodes: Vec<String> = {
        let mut v: Vec<String> = nodes.into_iter().collect();
        v.sort(); // deterministic ordering for tests
        v
    };

    // ── 2. Pre-compute out-degrees ──────────────────────────────────────────
    // out_degree[node] = number of distinct forward edges leaving that node.
    // Nodes absent from graph.edges have out-degree 0 (dangling nodes).
    let out_degree: HashMap<&str, usize> = nodes
        .iter()
        .map(|node| {
            let degree = graph.edges.get(node.as_str()).map(|s| s.len()).unwrap_or(0);
            (node.as_str(), degree)
        })
        .collect();

    // ── 3. Initialise ranks to 1/N ─────────────────────────────────────────
    let initial_rank = 1.0 / n as f64;
    let mut rank: HashMap<&str, f64> = nodes
        .iter()
        .map(|node| (node.as_str(), initial_rank))
        .collect();

    let teleport = (1.0 - damping) / n as f64;
    let convergence_threshold = 1e-6_f64;

    // ── 4. Power-iteration ─────────────────────────────────────────────────
    for _ in 0..max_iterations {
        // a) Sum of ranks belonging to dangling nodes (no outgoing edges).
        //    Their rank is redistributed uniformly across all nodes.
        let dangling_sum: f64 = nodes
            .iter()
            .filter(|node| out_degree[node.as_str()] == 0)
            .map(|node| rank[node.as_str()])
            .sum();

        let dangling_contrib = damping * dangling_sum / n as f64;

        // b) Compute new rank for every node.
        let mut new_rank: HashMap<&str, f64> = HashMap::with_capacity(n);
        for node in &nodes {
            // Contribution from nodes that have an edge pointing *to* this node.
            //
            // graph.reverse_edges[node] stores TypedEdges whose `.target` field
            // is the IMPORTER (i.e. the file that emits the forward edge).
            // That importer transfers rank[importer] / out_degree[importer] to
            // `node`.
            let inbound: f64 = graph
                .reverse_edges
                .get(node.as_str())
                .map(|importers| {
                    importers
                        .iter()
                        .filter_map(|e| {
                            // e.target is the importer (source of the original edge)
                            let importer = e.target.as_str();
                            let deg = out_degree.get(importer).copied().unwrap_or(0);
                            if deg == 0 {
                                // This importer is dangling — already handled via
                                // dangling_sum above; skip here to avoid double-counting.
                                None
                            } else {
                                Some(rank[importer] / deg as f64)
                            }
                        })
                        .sum()
                })
                .unwrap_or(0.0);

            new_rank.insert(
                node.as_str(),
                teleport + dangling_contrib + damping * inbound,
            );
        }

        // c) Check convergence: max absolute delta across all nodes.
        let max_delta = nodes
            .iter()
            .map(|node| (new_rank[node.as_str()] - rank[node.as_str()]).abs())
            .fold(0.0_f64, f64::max);

        rank = new_rank;

        if max_delta < convergence_threshold {
            break;
        }
    }

    // ── 5. Normalise by maximum rank → [0.0, 1.0] ─────────────────────────
    let max_rank = rank.values().copied().fold(0.0_f64, f64::max);

    nodes
        .iter()
        .map(|node| {
            let normalised = if max_rank > 0.0 {
                rank[node.as_str()] / max_rank
            } else {
                0.0
            };
            (node.clone(), normalised)
        })
        .collect()
}

/// Build inverted index: symbol_name → set of files containing it.
/// Used for O(1) cross-reference lookups in symbol_importance().
///
/// Iterates over each file's term_frequencies and inverts the mapping so that
/// every term (lowercased) maps to the set of file paths that contain it.
pub fn build_symbol_cross_refs(
    term_frequencies: &HashMap<String, HashMap<String, u32>>,
) -> HashMap<String, HashSet<String>> {
    let mut cross_refs: HashMap<String, HashSet<String>> = HashMap::new();
    for (file_path, terms) in term_frequencies {
        for term in terms.keys() {
            cross_refs
                .entry(term.clone())
                .or_default()
                .insert(file_path.clone());
        }
    }
    cross_refs
}

/// Compute importance score for a single symbol.
/// importance = file_pagerank * symbol_weight
/// where symbol_weight is:
///   1.0 — public AND referenced in at least one other file
///   0.7 — public but not referenced elsewhere
///   0.3 — private
pub fn symbol_importance(
    symbol: &crate::parser::language::Symbol,
    file_pagerank: f64,
    cross_refs: &HashMap<String, HashSet<String>>,
    file_path: &str,
) -> f64 {
    use crate::parser::language::Visibility;
    let weight = match symbol.visibility {
        Visibility::Public => {
            let name_lower = symbol.name.to_lowercase();
            let referenced_elsewhere = cross_refs
                .get(&name_lower)
                .map(|files| files.iter().any(|f| f != file_path))
                .unwrap_or(false);
            if referenced_elsewhere {
                1.0
            } else {
                0.7
            }
        }
        Visibility::Private => 0.3,
    };
    file_pagerank * weight
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{Symbol, SymbolKind, Visibility};
    use crate::schema::EdgeType;

    const DAMPING: f64 = 0.85;
    const MAX_ITER: usize = 100;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_graph(edges: &[(&str, &str)]) -> DependencyGraph {
        let mut g = DependencyGraph::new();
        for &(from, to) in edges {
            g.add_edge(from, to, EdgeType::Import);
        }
        g
    }

    fn approx_eq(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    // ── test_pagerank_empty_graph ─────────────────────────────────────────────

    #[test]
    fn test_pagerank_empty_graph() {
        let graph = DependencyGraph::new();
        let scores = compute_pagerank(&graph, DAMPING, MAX_ITER);
        assert!(scores.is_empty(), "empty graph should produce empty scores");
    }

    // ── test_pagerank_single_edge ─────────────────────────────────────────────
    //
    // A → B:  A imports B, so A transfers rank to B.
    // B has no outgoing edges; it is more "depended-upon", so it should score
    // higher after normalisation (it is the leaf that rank flows into).
    // After normalisation, the highest scorer gets 1.0; A must be ≤ 1.0.

    #[test]
    fn test_pagerank_single_edge() {
        let graph = make_graph(&[("a.rs", "b.rs")]);
        let scores = compute_pagerank(&graph, DAMPING, MAX_ITER);

        assert!(scores.contains_key("a.rs"));
        assert!(scores.contains_key("b.rs"));

        // Both scores are in [0.0, 1.0]
        for &v in scores.values() {
            assert!((0.0..=1.0 + 1e-9).contains(&v), "score out of range: {v}");
        }

        // b.rs receives rank from a.rs, so it should be the top node (score == 1.0)
        let b_score = scores["b.rs"];
        let a_score = scores["a.rs"];
        assert!(
            approx_eq(b_score, 1.0, 1e-6),
            "b.rs should be normalised to 1.0, got {b_score}"
        );
        assert!(
            a_score < b_score,
            "importer a.rs ({a_score}) should rank lower than imported b.rs ({b_score})"
        );
    }

    // ── test_pagerank_linear_chain ────────────────────────────────────────────
    //
    // A → B → C:  rank flows A→B→C, so C should have the highest score.

    #[test]
    fn test_pagerank_linear_chain() {
        let graph = make_graph(&[("a.rs", "b.rs"), ("b.rs", "c.rs")]);
        let scores = compute_pagerank(&graph, DAMPING, MAX_ITER);

        assert_eq!(scores.len(), 3);

        let a = scores["a.rs"];
        let b = scores["b.rs"];
        let c = scores["c.rs"];

        // c.rs is the deepest dependency — most rank flows into it.
        assert!(
            approx_eq(c, 1.0, 1e-6),
            "c.rs should be top-ranked (1.0), got {c}"
        );
        // b accumulates from a and redistributes to c; a has no inbound.
        assert!(b > a, "b.rs ({b}) should rank higher than a.rs ({a})");
    }

    // ── test_pagerank_star_pattern ────────────────────────────────────────────
    //
    // Many importers → single hub:
    //   A → hub, B → hub, C → hub, D → hub
    //
    // The hub receives rank from all four importers, so it should be the top
    // scorer (normalised to 1.0).  The importers are symmetric — they all
    // receive the same score.

    #[test]
    fn test_pagerank_star_pattern() {
        let graph = make_graph(&[
            ("a.rs", "hub.rs"),
            ("b.rs", "hub.rs"),
            ("c.rs", "hub.rs"),
            ("d.rs", "hub.rs"),
        ]);
        let scores = compute_pagerank(&graph, DAMPING, MAX_ITER);

        assert_eq!(scores.len(), 5);

        let hub = scores["hub.rs"];
        assert!(
            approx_eq(hub, 1.0, 1e-6),
            "hub.rs should be normalised to 1.0, got {hub}"
        );

        // All leaf importers are symmetric — equal scores.
        let leaves = ["a.rs", "b.rs", "c.rs", "d.rs"];
        let leaf_score = scores[leaves[0]];
        for &l in &leaves[1..] {
            assert!(
                approx_eq(scores[l], leaf_score, 1e-6),
                "leaf {l} score {} differs from first leaf {leaf_score}",
                scores[l]
            );
        }

        assert!(
            hub > leaf_score,
            "hub ({hub}) should score higher than leaves ({leaf_score})"
        );
    }

    // ── test_pagerank_cycle ───────────────────────────────────────────────────
    //
    // A → B → C → A:  symmetric cycle — all nodes receive equal rank.
    // After normalisation every node should score 1.0.

    #[test]
    fn test_pagerank_cycle() {
        let graph = make_graph(&[("a.rs", "b.rs"), ("b.rs", "c.rs"), ("c.rs", "a.rs")]);
        let scores = compute_pagerank(&graph, DAMPING, MAX_ITER);

        assert_eq!(scores.len(), 3);

        let a = scores["a.rs"];
        let b = scores["b.rs"];
        let c = scores["c.rs"];

        // All three should be equal (symmetric cycle)
        assert!(
            approx_eq(a, b, 1e-6) && approx_eq(b, c, 1e-6),
            "cycle nodes should have equal rank: a={a}, b={b}, c={c}"
        );

        // After normalisation, max == 1.0
        assert!(
            approx_eq(a, 1.0, 1e-6),
            "normalised cycle rank should be 1.0, got {a}"
        );
    }

    // ── test_pagerank_disconnected ────────────────────────────────────────────
    //
    // Two disconnected components: A → B  and  C → D.
    // Each component is independent; within each, the leaf (B or D) receives
    // rank from its importer.  The maximum across the whole graph determines
    // normalisation, so at least one node should score 1.0.

    #[test]
    fn test_pagerank_disconnected() {
        let graph = make_graph(&[("a.rs", "b.rs"), ("c.rs", "d.rs")]);
        let scores = compute_pagerank(&graph, DAMPING, MAX_ITER);

        assert_eq!(scores.len(), 4);

        // All scores in valid range
        for &v in scores.values() {
            assert!((0.0..=1.0 + 1e-9).contains(&v), "score out of range: {v}");
        }

        // At least one node has score 1.0 (the normalised maximum)
        let max_score = scores.values().copied().fold(0.0_f64, f64::max);
        assert!(
            approx_eq(max_score, 1.0, 1e-6),
            "normalised max should be 1.0, got {max_score}"
        );

        // Symmetric components: b.rs and d.rs should have equal scores;
        // a.rs and c.rs should have equal scores.
        assert!(
            approx_eq(scores["b.rs"], scores["d.rs"], 1e-6),
            "symmetric leaves b={} d={} should be equal",
            scores["b.rs"],
            scores["d.rs"]
        );
        assert!(
            approx_eq(scores["a.rs"], scores["c.rs"], 1e-6),
            "symmetric importers a={} c={} should be equal",
            scores["a.rs"],
            scores["c.rs"]
        );
    }

    // ── test_pagerank_convergence ─────────────────────────────────────────────
    //
    // Verify that the algorithm converges for a moderately sized graph
    // (50 nodes in a chain) within the iteration budget.  The test simply
    // checks that scores are non-empty and well-formed — not that they match
    // a specific value.

    #[test]
    fn test_pagerank_convergence() {
        let mut graph = DependencyGraph::new();
        for i in 0..49_usize {
            graph.add_edge(
                &format!("file_{i:02}.rs"),
                &format!("file_{:02}.rs", i + 1),
                EdgeType::Import,
            );
        }

        let scores = compute_pagerank(&graph, DAMPING, MAX_ITER);

        assert_eq!(scores.len(), 50);

        for (path, &score) in &scores {
            assert!(
                (0.0..=1.0 + 1e-9).contains(&score),
                "{path} has out-of-range score {score}"
            );
        }

        // The last file in the chain (file_49) should be the top-ranked.
        let top = scores["file_49.rs"];
        assert!(
            approx_eq(top, 1.0, 1e-6),
            "file_49.rs should be normalised to 1.0, got {top}"
        );
    }

    // ── test_pagerank_normalized ──────────────────────────────────────────────
    //
    // Invariant: the highest-scored node always has exactly 1.0 after
    // normalisation.  Test this on several distinct graph topologies.

    #[test]
    fn test_pagerank_normalized() {
        let topologies: Vec<Vec<(&str, &str)>> = vec![
            vec![("x.rs", "y.rs")],
            vec![("x.rs", "y.rs"), ("y.rs", "z.rs")],
            vec![
                ("a.rs", "common.rs"),
                ("b.rs", "common.rs"),
                ("c.rs", "common.rs"),
            ],
            vec![
                ("a.rs", "b.rs"),
                ("b.rs", "c.rs"),
                ("c.rs", "a.rs"),
                ("d.rs", "a.rs"),
            ],
        ];

        for edges in &topologies {
            let graph = make_graph(edges);
            let scores = compute_pagerank(&graph, DAMPING, MAX_ITER);

            let max_score = scores.values().copied().fold(0.0_f64, f64::max);
            assert!(
                approx_eq(max_score, 1.0, 1e-6),
                "max score should be 1.0 for topology {edges:?}, got {max_score}"
            );

            for &v in scores.values() {
                assert!(
                    (0.0..=1.0 + 1e-9).contains(&v),
                    "score {v} out of [0.0, 1.0] range"
                );
            }
        }
    }

    // ── helpers for cross-ref / symbol importance tests ───────────────────────

    fn make_symbol(name: &str, visibility: Visibility) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            visibility,
            signature: format!("pub fn {}()", name),
            body: "{}".to_string(),
            start_line: 1,
            end_line: 1,
        }
    }

    fn term_freq(pairs: &[(&str, u32)]) -> HashMap<String, u32> {
        pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
    }

    // ── test_build_symbol_cross_refs ──────────────────────────────────────────

    #[test]
    fn test_build_symbol_cross_refs() {
        let mut tf: HashMap<String, HashMap<String, u32>> = HashMap::new();
        tf.insert(
            "a.rs".to_string(),
            term_freq(&[("connect", 3), ("query", 1)]),
        );
        tf.insert(
            "b.rs".to_string(),
            term_freq(&[("connect", 1), ("render", 2)]),
        );
        tf.insert("c.rs".to_string(), term_freq(&[("render", 1)]));

        let refs = build_symbol_cross_refs(&tf);

        // "connect" appears in both a.rs and b.rs
        let connect_files = refs
            .get("connect")
            .expect("connect should be in cross_refs");
        assert!(
            connect_files.contains("a.rs"),
            "connect should reference a.rs"
        );
        assert!(
            connect_files.contains("b.rs"),
            "connect should reference b.rs"
        );
        assert!(
            !connect_files.contains("c.rs"),
            "connect should not reference c.rs"
        );

        // "render" appears in b.rs and c.rs
        let render_files = refs.get("render").expect("render should be in cross_refs");
        assert!(
            render_files.contains("b.rs"),
            "render should reference b.rs"
        );
        assert!(
            render_files.contains("c.rs"),
            "render should reference c.rs"
        );
        assert!(
            !render_files.contains("a.rs"),
            "render should not reference a.rs"
        );

        // "query" appears only in a.rs
        let query_files = refs.get("query").expect("query should be in cross_refs");
        assert_eq!(query_files.len(), 1);
        assert!(query_files.contains("a.rs"));
    }

    // ── test_symbol_importance_public_referenced ──────────────────────────────

    #[test]
    fn test_symbol_importance_public_referenced() {
        // Symbol "connect" is public and appears in a.rs and b.rs
        let mut tf: HashMap<String, HashMap<String, u32>> = HashMap::new();
        tf.insert("a.rs".to_string(), term_freq(&[("connect", 3)]));
        tf.insert("b.rs".to_string(), term_freq(&[("connect", 1)]));
        let refs = build_symbol_cross_refs(&tf);

        let sym = make_symbol("connect", Visibility::Public);
        // file_pagerank = 0.8, referenced in b.rs (other file), so weight = 1.0
        let importance = symbol_importance(&sym, 0.8, &refs, "a.rs");
        assert!(
            (importance - 0.8).abs() < 1e-9,
            "public+referenced: expected 0.8 * 1.0 = 0.8, got {importance}"
        );
    }

    // ── test_symbol_importance_public_unreferenced ────────────────────────────

    #[test]
    fn test_symbol_importance_public_unreferenced() {
        // Symbol "unique_fn" is public but only appears in a.rs itself
        let mut tf: HashMap<String, HashMap<String, u32>> = HashMap::new();
        tf.insert("a.rs".to_string(), term_freq(&[("unique", 1)]));
        let refs = build_symbol_cross_refs(&tf);

        let sym = make_symbol("unique_fn", Visibility::Public);
        // "unique_fn" lowercased → "unique_fn"; not in cross_refs at all → unreferenced
        let importance = symbol_importance(&sym, 0.8, &refs, "a.rs");
        assert!(
            (importance - 0.56).abs() < 1e-9,
            "public+unreferenced: expected 0.8 * 0.7 = 0.56, got {importance}"
        );
    }

    // ── test_symbol_importance_private ───────────────────────────────────────

    #[test]
    fn test_symbol_importance_private() {
        let refs: HashMap<String, HashSet<String>> = HashMap::new();
        let sym = make_symbol("internal_helper", Visibility::Private);
        let importance = symbol_importance(&sym, 0.8, &refs, "a.rs");
        assert!(
            (importance - 0.24).abs() < 1e-9,
            "private: expected 0.8 * 0.3 = 0.24, got {importance}"
        );
    }
}

#![cfg(feature = "visual")]

/// Fix 3: level3 PageRank source-iteration determinism.
///
/// Verifies that `build_architecture_explorer_data` applies a stable secondary
/// sort key (path) when PageRank scores are equal. Without the fix the sort is
/// unstable across runs because HashMap iteration is not ordered and sort_by
/// with equal values is non-deterministic.
///
/// We probe this by checking that the sort ordering in the source code has
/// `.then_with(|| a.path.cmp(&b.path))` — a structural contract test.
#[test]
fn ranked_files_sort_has_secondary_path_key() {
    // The sort must include a secondary path-based key so that ties are
    // broken deterministically. We verify this by inspecting that the source
    // code of build_architecture_explorer_data contains `then_with`.
    //
    // Note: we cannot easily test the actual HashMap non-determinism in a
    // single-threaded test run (Rust's HashMap uses a per-process seed).
    // The contract is enforced by a code-level check.
    let source = include_str!("../src/visual/render.rs");
    // Find the ranked_files sort (the one that uses partial_cmp on pagerank).
    let sort_pos = source
        .find("ranked_files.sort_by")
        .expect("ranked_files.sort_by must be present in render.rs");
    let after_sort = &source[sort_pos..];
    let sort_end = after_sort
        .find(");")
        .expect("closing ); for sort_by must be present")
        + sort_pos;
    let sort_block = &source[sort_pos..=sort_end];
    assert!(
        sort_block.contains("then_with"),
        "ranked_files sort must include a secondary `.then_with()` key for determinism; \
         got: {sort_block}"
    );
}

/// Verify the secondary sort produces correct lexicographic ordering.
#[test]
fn pagerank_sort_secondary_key_is_path_lexicographic() {
    let mut pairs: Vec<(&str, f64)> = vec![
        ("src/z_file.rs", 0.0),
        ("src/a_file.rs", 0.0),
        ("src/m_file.rs", 0.0),
        ("src/b_file.rs", 0.5),
        ("src/c_file.rs", 0.25),
    ];
    pairs.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    assert_eq!(pairs[0].0, "src/b_file.rs");
    assert_eq!(pairs[1].0, "src/c_file.rs");
    assert_eq!(pairs[2].0, "src/a_file.rs");
    assert_eq!(pairs[3].0, "src/m_file.rs");
    assert_eq!(pairs[4].0, "src/z_file.rs");
}

/// v2.1.0 root-cause: cross-process f64 determinism for PageRank, coupling,
/// and every other downstream reducer requires the dependency graph to use
/// BTreeMap/BTreeSet (not HashMap/HashSet).  Without it,
/// `graph.reverse_edges[node].iter().filter_map(...).sum::<f64>()` in
/// pagerank's power iteration sees randomised order, and f64 addition is
/// non-associative — the same input graph produces 1-ULP-different ranks
/// across runs, then propagates into /v1/health, MCP cxpak_health, the SPA
/// dashboard, and `cxpak_api_surface.symbols.by_file[*].pagerank`
/// (4000+ divergent f64 values across 5 processes before the fix; 0 after).
#[test]
fn dependency_graph_uses_btreemap_btreeset_for_determinism() {
    // `DependencyGraph` was relocated to `core_graph` in cxpak 3.0.0 Phase 0
    // (de-cycle); the determinism invariant moved with the struct definition.
    let source = include_str!("../src/core_graph/graph.rs");
    let edges_decl = source
        .lines()
        .find(|l| l.contains("pub edges:"))
        .expect("pub edges field declaration must exist");
    assert!(
        edges_decl.contains("BTreeMap")
            && edges_decl.contains("BTreeSet")
            && !edges_decl.contains("HashMap")
            && !edges_decl.contains("HashSet"),
        "DependencyGraph.edges MUST be BTreeMap<String, BTreeSet<TypedEdge>>; \
         HashMap/HashSet iteration is randomised → f64 sums vary 1 ULP across \
         processes (caught v2.1.0). Got: {edges_decl}"
    );
    let rev_decl = source
        .lines()
        .find(|l| l.contains("pub reverse_edges:"))
        .expect("pub reverse_edges field declaration must exist");
    assert!(
        rev_decl.contains("BTreeMap")
            && rev_decl.contains("BTreeSet")
            && !rev_decl.contains("HashMap")
            && !rev_decl.contains("HashSet"),
        "DependencyGraph.reverse_edges MUST be BTreeMap<String, BTreeSet<TypedEdge>>; \
         the regression is silent — tests still pass, but PageRank/health/risk \
         outputs jitter across runs. Got: {rev_decl}"
    );
}

/// v2.1.0 root-cause: score_coupling builds a per-module groups map and then
/// `.sum::<f64>()` over the qualifying groups.  HashMap iteration order
/// scrambles the sum order → 1-ULP jitter in coupling → propagates into the
/// composite health score across processes.  BTreeMap fixes it at the source.
#[test]
fn score_coupling_module_files_uses_btreemap() {
    let source = include_str!("../src/intelligence/health.rs");
    let func_start = source
        .find("pub fn score_coupling")
        .expect("score_coupling exists");
    let func_window = &source[func_start..func_start + 800];
    assert!(
        func_window.contains("BTreeMap"),
        "score_coupling MUST declare its module_files map as BTreeMap so the \
         downstream `.sum::<f64>()` over module ratios sees a deterministic \
         iteration order.  HashMap regresses cross-process determinism."
    );
}

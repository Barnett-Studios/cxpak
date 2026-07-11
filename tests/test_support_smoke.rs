//! N0 acceptance: the shared `index_with()` builder yields a usable index —
//! populated co-change edges and a rankable risk set — that every downstream
//! RED test compiles against.

use cxpak::test_support::index_with;

#[test]
fn index_with_builds_graph_cochanges_and_rankable_risk() {
    let index = index_with()
        .file("A")
        .imports("B")
        .co_change("A", "B", 0.9)
        .n_risky_files(3)
        .with_cycle("X", "Y")
        .build();

    assert!(!index.co_changes.is_empty(), "co_changes populated");
    assert!(
        !cxpak::intelligence::risk::compute_risk_ranking(&index).is_empty(),
        "risk set is rankable"
    );
    // The Import edge A→B is present (the topology the builder claims to wire).
    assert!(
        index.graph.dependents("B").iter().any(|e| e.target == "A"),
        "A→B import edge present in graph"
    );
}

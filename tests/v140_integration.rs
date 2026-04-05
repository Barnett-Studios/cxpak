use cxpak::intelligence::co_change::CoChangeEdge;
use cxpak::intelligence::predict::{confidence_for_signals, predict, ImpactSignal};
use std::collections::HashMap;

#[test]
fn test_predict_empty_graph_returns_summary() {
    let graph = cxpak::index::graph::DependencyGraph::new();
    let pagerank = HashMap::new();
    let co_changes: Vec<CoChangeEdge> = vec![];
    let test_map = HashMap::new();

    let result = predict(
        &["src/lib.rs"],
        &graph,
        &pagerank,
        &co_changes,
        &test_map,
        3,
    );
    assert_eq!(result.changed_files, vec!["src/lib.rs"]);
    assert!(result.confidence_summary.contains("predicted"));
}

#[test]
fn test_security_surface_empty_index() {
    let index = cxpak::index::CodebaseIndex::empty();
    let surface = cxpak::intelligence::security::build_security_surface(
        &index,
        cxpak::intelligence::security::DEFAULT_AUTH_PATTERNS,
        None,
    );
    assert!(surface.unprotected_endpoints.is_empty());
    assert!(surface.secret_patterns.is_empty());
    assert!(surface.sql_injection_surface.is_empty());
}

#[test]
fn test_drift_on_tempdir_no_panic() {
    let dir = tempfile::TempDir::new().unwrap();
    let index = cxpak::index::CodebaseIndex::empty();
    let report = cxpak::intelligence::drift::build_drift_report(&index, dir.path(), false);
    assert!(report.baseline.is_none());
}

#[test]
fn test_drift_save_baseline_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let index = cxpak::index::CodebaseIndex::empty();
    // Save baseline
    let _ = cxpak::intelligence::drift::build_drift_report(&index, dir.path(), true);
    // Reload — baseline should exist
    let report = cxpak::intelligence::drift::build_drift_report(&index, dir.path(), false);
    assert!(report.baseline.is_some());
}

#[test]
fn test_confidence_for_all_signal_combos_in_range() {
    let combos: &[&[ImpactSignal]] = &[
        &[],
        &[ImpactSignal::Historical],
        &[ImpactSignal::Structural],
        &[ImpactSignal::CallBased],
        &[ImpactSignal::Structural, ImpactSignal::Historical],
        &[ImpactSignal::CallBased, ImpactSignal::Historical],
        &[ImpactSignal::Structural, ImpactSignal::CallBased],
        &[
            ImpactSignal::Structural,
            ImpactSignal::CallBased,
            ImpactSignal::Historical,
        ],
    ];
    for combo in combos {
        let conf = confidence_for_signals(combo);
        assert!(
            (0.0..=1.0).contains(&conf),
            "confidence for {:?} must be in [0,1], got {}",
            combo,
            conf
        );
    }
}

#[test]
fn test_predict_with_co_changes_produces_historical() {
    let graph = cxpak::index::graph::DependencyGraph::new();
    let pagerank = HashMap::new();
    let co_changes = vec![CoChangeEdge {
        file_a: "src/a.rs".to_string(),
        file_b: "src/b.rs".to_string(),
        count: 5,
        recency_weight: 0.8,
    }];
    let test_map = HashMap::new();

    let result = predict(&["src/a.rs"], &graph, &pagerank, &co_changes, &test_map, 3);
    assert!(
        !result.historical_impact.is_empty(),
        "co-changes should produce historical impact"
    );
    assert_eq!(result.historical_impact[0].path, "src/b.rs");
}

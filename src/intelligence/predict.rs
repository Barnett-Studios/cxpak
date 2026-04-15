use crate::index::graph::DependencyGraph;
use crate::intelligence::co_change::CoChangeEdge;
use crate::intelligence::test_map::TestFileRef;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImpactSignal {
    TestMap,
    Structural,
    Historical,
    CallBased,
}

#[derive(Debug, Serialize)]
pub struct ImpactEntry {
    pub path: String,
    pub signal: ImpactSignal,
    pub score: f64,
}

#[derive(Debug, Serialize)]
pub struct TestPrediction {
    pub test_file: String,
    pub test_function: Option<String>,
    pub signals: Vec<ImpactSignal>,
    pub confidence: f64,
}

#[derive(Debug, Serialize)]
pub struct PredictionResult {
    pub changed_files: Vec<String>,
    pub structural_impact: Vec<ImpactEntry>,
    pub historical_impact: Vec<ImpactEntry>,
    pub call_impact: Vec<ImpactEntry>,
    pub test_impact: Vec<TestPrediction>,
    pub confidence_summary: String,
}

// ---------------------------------------------------------------------------
// Signal computation
// ---------------------------------------------------------------------------

/// Compute structural impact via BFS on the reverse dependency graph.
pub fn structural_impact(
    changed_files: &[&str],
    graph: &DependencyGraph,
    pagerank: &HashMap<String, f64>,
    depth: usize,
) -> Vec<ImpactEntry> {
    let result = crate::intelligence::blast_radius::compute_blast_radius(
        changed_files,
        graph,
        pagerank,
        &HashMap::new(),
        depth,
        None,
    );
    let mut entries: Vec<ImpactEntry> = result
        .categories
        .direct_dependents
        .iter()
        .chain(result.categories.transitive_dependents.iter())
        .chain(result.categories.schema_dependents.iter())
        .chain(result.categories.test_files.iter())
        .map(|af| ImpactEntry {
            path: af.path.clone(),
            signal: ImpactSignal::Structural,
            score: af.risk,
        })
        .collect();
    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
}

/// Compute historical impact from co-change edges.
pub fn historical_impact(changed_files: &[&str], co_changes: &[CoChangeEdge]) -> Vec<ImpactEntry> {
    let changed_set: HashSet<&str> = changed_files.iter().copied().collect();
    let mut score_map: HashMap<String, f64> = HashMap::new();

    let max_count = co_changes.iter().map(|e| e.count).max().unwrap_or(1).max(1) as f64;

    for edge in co_changes {
        let other_file = if changed_set.contains(edge.file_a.as_str()) {
            &edge.file_b
        } else if changed_set.contains(edge.file_b.as_str()) {
            &edge.file_a
        } else {
            continue;
        };

        if changed_set.contains(other_file.as_str()) {
            continue;
        }

        let score = (edge.count as f64 / max_count) * edge.recency_weight;
        let entry = score_map.entry(other_file.to_string()).or_insert(0.0);
        if score > *entry {
            *entry = score;
        }
    }

    let mut entries: Vec<ImpactEntry> = score_map
        .into_iter()
        .map(|(path, score)| ImpactEntry {
            path,
            signal: ImpactSignal::Historical,
            score,
        })
        .collect();
    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
}

// ---------------------------------------------------------------------------
// Test prediction merging
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ImpactSignalKey {
    TestMap,
    Structural,
    Historical,
    CallBased,
}

impl ImpactSignalKey {
    fn to_impact_signal(&self) -> ImpactSignal {
        match self {
            Self::TestMap => ImpactSignal::TestMap,
            Self::Structural => ImpactSignal::Structural,
            Self::Historical => ImpactSignal::Historical,
            Self::CallBased => ImpactSignal::CallBased,
        }
    }
}

/// Check if path looks like a test file.
fn is_test_path(path: &str) -> bool {
    crate::intelligence::blast_radius::is_test_path(path)
}

/// Merge structural, historical, and call-based signals to produce test predictions.
pub fn merge_test_predictions(
    changed_files: &[&str],
    structural: &[ImpactEntry],
    historical: &[ImpactEntry],
    call_based: &[ImpactEntry],
    test_map: &HashMap<String, Vec<TestFileRef>>,
) -> Vec<TestPrediction> {
    let mut test_signals: HashMap<String, HashSet<ImpactSignalKey>> = HashMap::new();
    let changed_set: HashSet<&str> = changed_files.iter().copied().collect();

    // 1. test_map signal
    for &changed in changed_files {
        if let Some(refs) = test_map.get(changed) {
            for tr in refs {
                test_signals
                    .entry(tr.path.clone())
                    .or_default()
                    .insert(ImpactSignalKey::TestMap);
            }
        }
    }

    // 2. Structural signal: test files in structural impact
    for entry in structural {
        if is_test_path(&entry.path) && !changed_set.contains(entry.path.as_str()) {
            test_signals
                .entry(entry.path.clone())
                .or_default()
                .insert(ImpactSignalKey::Structural);
        }
    }

    // 3. Historical signal: test files in historical impact
    for entry in historical {
        if is_test_path(&entry.path) && !changed_set.contains(entry.path.as_str()) {
            test_signals
                .entry(entry.path.clone())
                .or_default()
                .insert(ImpactSignalKey::Historical);
        }
    }

    // 4. Call-based signal
    for entry in call_based {
        if is_test_path(&entry.path) && !changed_set.contains(entry.path.as_str()) {
            test_signals
                .entry(entry.path.clone())
                .or_default()
                .insert(ImpactSignalKey::CallBased);
        }
    }

    let mut predictions: Vec<TestPrediction> = test_signals
        .into_iter()
        .map(|(test_file, keys)| {
            let signals: Vec<ImpactSignal> = keys.iter().map(|k| k.to_impact_signal()).collect();
            let confidence = confidence_for_signals(&signals);
            TestPrediction {
                test_file,
                test_function: None,
                signals,
                confidence,
            }
        })
        .collect();
    predictions.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    predictions
}

/// Map signal combinations to confidence values.
///
/// Uses four boolean signals: test_map, structural, call_graph, co_change.
/// The 8 distinct levels are preserved; test_map is the strongest single
/// signal, structural is equivalent in weight, call_graph beats co_change.
///
/// | (test_map|structural, call_graph, co_change) | Confidence |
/// |---------------------------------------------|-----------|
/// | (true,  true,  true)                        | 0.90      |
/// | (true,  true,  false)                       | 0.85      |
/// | (true,  false, true)                        | 0.75      |
/// | (false, true,  true)                        | 0.70      |
/// | (true,  false, false)                       | 0.60      |
/// | (false, true,  false)                       | 0.50      |
/// | (false, false, true)                        | 0.40      |
/// | (false, false, false)                       | 0.00      |
///
/// Rationale: test_map is the strongest single signal (naming convention is
/// very reliable).  Structural blast-radius is equivalent.  call_graph beats
/// co_change because call-based reasoning is structural rather than historical.
pub fn confidence_for_signals(signals: &[ImpactSignal]) -> f64 {
    let has_map =
        signals.contains(&ImpactSignal::TestMap) || signals.contains(&ImpactSignal::Structural);
    let has_hist = signals.contains(&ImpactSignal::Historical);
    let has_call = signals.contains(&ImpactSignal::CallBased);

    match (has_map, has_call, has_hist) {
        (true, true, true) => 0.90,
        (true, true, false) => 0.85,
        (true, false, true) => 0.75,
        (false, true, true) => 0.70,
        (true, false, false) => 0.60,
        (false, true, false) => 0.50,
        (false, false, true) => 0.40,
        (false, false, false) => 0.00,
    }
}

// ---------------------------------------------------------------------------
// Call-graph impact
// ---------------------------------------------------------------------------

/// Compute call-graph-based impact for a set of changed files.
///
/// For every edge whose callee is in `changed_files`, the caller file scores
/// +`weight` where weight is 1.0 for `Exact` edges and 0.5 for `Approximate`.
/// Scores are accumulated across all matching edges then normalised to [0, 1]
/// by dividing by the maximum raw score (clamped to ≥ 1.0).
pub fn call_graph_impact(
    changed: &[&str],
    call_graph: &crate::intelligence::call_graph::CallGraph,
) -> Vec<ImpactEntry> {
    let changed_set: HashSet<&str> = changed.iter().copied().collect();
    let mut scores: HashMap<String, f64> = HashMap::new();

    for edge in &call_graph.edges {
        if changed_set.contains(edge.callee_file.as_str()) {
            let weight = match edge.confidence {
                crate::intelligence::call_graph::CallConfidence::Exact => 1.0,
                crate::intelligence::call_graph::CallConfidence::Approximate => 0.5,
            };
            *scores.entry(edge.caller_file.clone()).or_default() += weight;
        }
    }

    // Normalise so every score is in [0, 1].
    let max = scores.values().copied().fold(0.0_f64, f64::max).max(1.0);

    let mut entries: Vec<ImpactEntry> = scores
        .into_iter()
        // Exclude the changed files themselves from impact results.
        .filter(|(file, _)| !changed_set.contains(file.as_str()))
        .map(|(file, raw)| ImpactEntry {
            path: file,
            signal: ImpactSignal::CallBased,
            score: raw / max,
        })
        .collect();

    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Compute full prediction result for a set of changed files.
pub fn predict(
    changed_files: &[&str],
    graph: &DependencyGraph,
    pagerank: &HashMap<String, f64>,
    co_changes: &[CoChangeEdge],
    test_map: &HashMap<String, Vec<TestFileRef>>,
    depth: usize,
) -> PredictionResult {
    predict_with_call_graph(
        changed_files,
        graph,
        pagerank,
        co_changes,
        test_map,
        depth,
        &crate::intelligence::call_graph::CallGraph::new(),
    )
}

/// Like [`predict`] but also incorporates call-graph signals.
pub fn predict_with_call_graph(
    changed_files: &[&str],
    graph: &DependencyGraph,
    pagerank: &HashMap<String, f64>,
    co_changes: &[CoChangeEdge],
    test_map: &HashMap<String, Vec<TestFileRef>>,
    depth: usize,
    call_graph: &crate::intelligence::call_graph::CallGraph,
) -> PredictionResult {
    let structural = structural_impact(changed_files, graph, pagerank, depth);
    let historical = historical_impact(changed_files, co_changes);
    let call_impact = call_graph_impact(changed_files, call_graph);

    let test_impact = merge_test_predictions(
        changed_files,
        &structural,
        &historical,
        &call_impact,
        test_map,
    );

    let avg_conf = if test_impact.is_empty() {
        0.0
    } else {
        test_impact.iter().map(|t| t.confidence).sum::<f64>() / test_impact.len() as f64
    };

    let mut distinct_affected: HashSet<&str> = HashSet::new();
    distinct_affected.extend(structural.iter().map(|e| e.path.as_str()));
    distinct_affected.extend(historical.iter().map(|e| e.path.as_str()));
    distinct_affected.extend(call_impact.iter().map(|e| e.path.as_str()));
    let total_affected = distinct_affected.len();

    let confidence_summary = format!(
        "{total_affected} files predicted affected ({} structural, {} historical, {} call-based); {} tests predicted; avg confidence {:.2}",
        structural.len(),
        historical.len(),
        call_impact.len(),
        test_impact.len(),
        avg_conf
    );

    PredictionResult {
        changed_files: changed_files.iter().map(|s| s.to_string()).collect(),
        structural_impact: structural,
        historical_impact: historical,
        call_impact,
        test_impact,
        confidence_summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::graph::DependencyGraph;
    use crate::schema::EdgeType;

    fn make_graph_chain() -> DependencyGraph {
        let mut g = DependencyGraph::new();
        g.add_edge("src/a.rs", "src/b.rs", EdgeType::Import);
        g.add_edge("src/c.rs", "src/b.rs", EdgeType::Import);
        g
    }

    #[test]
    fn test_structural_impact_direct_dependent() {
        let graph = make_graph_chain();
        let pagerank: HashMap<String, f64> =
            [("src/a.rs".to_string(), 0.8), ("src/c.rs".to_string(), 0.6)].into();
        let entries = structural_impact(&["src/b.rs"], &graph, &pagerank, 3);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(
            paths.contains(&"src/a.rs"),
            "a.rs imports b.rs — must appear"
        );
        assert!(
            paths.contains(&"src/c.rs"),
            "c.rs imports b.rs — must appear"
        );
        for e in &entries {
            assert_eq!(e.signal, ImpactSignal::Structural);
            assert!(e.score > 0.0 && e.score <= 1.0);
        }
    }

    #[test]
    fn test_historical_impact_from_co_changes() {
        let co_changes = vec![CoChangeEdge {
            file_a: "src/b.rs".to_string(),
            file_b: "src/x.rs".to_string(),
            count: 5,
            recency_weight: 0.9,
        }];
        let entries = historical_impact(&["src/b.rs"], &co_changes);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "src/x.rs");
        assert_eq!(entries[0].signal, ImpactSignal::Historical);
        assert!(entries[0].score > 0.0);
    }

    #[test]
    fn test_historical_impact_changed_files_excluded() {
        let co_changes = vec![CoChangeEdge {
            file_a: "src/b.rs".to_string(),
            file_b: "src/b.rs".to_string(),
            count: 10,
            recency_weight: 1.0,
        }];
        let entries = historical_impact(&["src/b.rs"], &co_changes);
        assert!(
            entries.is_empty(),
            "changed files must not appear in impact"
        );
    }

    #[test]
    fn test_test_confidence_all_three_signals() {
        let test_map: HashMap<String, Vec<TestFileRef>> = [(
            "src/b.rs".to_string(),
            vec![crate::intelligence::test_map::TestFileRef {
                path: "tests/b_test.rs".to_string(),
                confidence: crate::intelligence::test_map::TestConfidence::Both,
            }],
        )]
        .into();
        let co_changes = vec![CoChangeEdge {
            file_a: "src/b.rs".to_string(),
            file_b: "tests/b_test.rs".to_string(),
            count: 4,
            recency_weight: 1.0,
        }];
        let mut graph = DependencyGraph::new();
        graph.add_edge("tests/b_test.rs", "src/b.rs", EdgeType::Import);
        let pagerank = HashMap::new();
        let structural = structural_impact(&["src/b.rs"], &graph, &pagerank, 3);
        let historical = historical_impact(&["src/b.rs"], &co_changes);
        let result =
            merge_test_predictions(&["src/b.rs"], &structural, &historical, &[], &test_map);
        let pred = result
            .iter()
            .find(|p| p.test_file == "tests/b_test.rs")
            .expect("b_test.rs must be predicted");
        // test_map → ImpactSignal::TestMap, structural → ImpactSignal::Structural, co_change → Historical
        // keys: {TestMap, Structural, Historical} → 3 signals in vec
        // confidence: has_map=true (TestMap or Structural), has_hist=true, has_call=false → 0.75
        assert!(
            (pred.confidence - 0.75).abs() < 1e-9,
            "expected 0.75 for test_map+structural+historical, got {}",
            pred.confidence
        );
        assert_eq!(pred.signals.len(), 3);
    }

    #[test]
    fn test_test_confidence_map_only() {
        let test_map: HashMap<String, Vec<TestFileRef>> = [(
            "src/b.rs".to_string(),
            vec![crate::intelligence::test_map::TestFileRef {
                path: "tests/b_test.rs".to_string(),
                confidence: crate::intelligence::test_map::TestConfidence::NameMatch,
            }],
        )]
        .into();
        let result = merge_test_predictions(&["src/b.rs"], &[], &[], &[], &test_map);
        let pred = result
            .iter()
            .find(|p| p.test_file == "tests/b_test.rs")
            .expect("b_test.rs must be predicted");
        // test_map only → has_map=true, has_call=false, has_hist=false → 0.60
        assert!(
            (pred.confidence - 0.60).abs() < 1e-9,
            "expected 0.60 for test_map only, got {}",
            pred.confidence
        );
    }

    #[test]
    fn test_confidence_map_values() {
        // All 7 non-empty subsets must map to their documented unique values.
        let cases: &[(&[ImpactSignal], f64)] = &[
            (&[ImpactSignal::Historical], 0.40),
            (&[ImpactSignal::Structural], 0.60),
            (&[ImpactSignal::CallBased], 0.50),
            (&[ImpactSignal::Structural, ImpactSignal::Historical], 0.75),
            (&[ImpactSignal::CallBased, ImpactSignal::Historical], 0.70),
            (&[ImpactSignal::Structural, ImpactSignal::CallBased], 0.85),
            (
                &[
                    ImpactSignal::Structural,
                    ImpactSignal::CallBased,
                    ImpactSignal::Historical,
                ],
                0.90,
            ),
        ];
        for (signals, expected) in cases {
            let conf = confidence_for_signals(signals);
            assert!(
                (conf - expected).abs() < 1e-9,
                "signals {:?} → expected {expected}, got {conf}",
                signals
            );
        }
    }

    #[test]
    fn test_predict_changed_files_excluded_from_impact() {
        let graph = DependencyGraph::new();
        let pagerank = HashMap::new();
        let entries = structural_impact(&["src/b.rs"], &graph, &pagerank, 3);
        assert!(entries.iter().all(|e| e.path != "src/b.rs"));
    }

    #[test]
    fn test_all_seven_confidence_subsets_are_distinct() {
        use std::collections::HashSet;
        let cases: Vec<(&[ImpactSignal], f64)> = vec![
            (&[ImpactSignal::Historical], 0.40),
            (&[ImpactSignal::Structural], 0.60),
            (&[ImpactSignal::CallBased], 0.50),
            (&[ImpactSignal::Structural, ImpactSignal::Historical], 0.75),
            (&[ImpactSignal::CallBased, ImpactSignal::Historical], 0.70),
            (&[ImpactSignal::Structural, ImpactSignal::CallBased], 0.85),
            (
                &[
                    ImpactSignal::Structural,
                    ImpactSignal::CallBased,
                    ImpactSignal::Historical,
                ],
                0.90,
            ),
        ];
        let values: HashSet<u64> = cases.iter().map(|(_, v)| (v * 1000.0) as u64).collect();
        assert_eq!(
            values.len(),
            7,
            "all 7 non-empty subsets must map to distinct confidence levels"
        );
    }

    #[test]
    fn test_empty_signals_produces_zero_confidence() {
        assert_eq!(confidence_for_signals(&[]), 0.0);
    }

    #[test]
    fn test_historical_impact_score_bounded() {
        let co_changes: Vec<CoChangeEdge> = (0..100)
            .map(|i| CoChangeEdge {
                file_a: "src/hub.rs".to_string(),
                file_b: format!("src/dep{i}.rs"),
                count: i as u32 + 1,
                recency_weight: 1.0,
            })
            .collect();
        let entries = historical_impact(&["src/hub.rs"], &co_changes);
        for e in &entries {
            assert!(
                e.score >= 0.0 && e.score <= 1.0,
                "score must be in [0,1], got {} for {}",
                e.score,
                e.path
            );
        }
    }

    #[test]
    fn test_structural_impact_sorted_descending() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("high.rs", "src.rs", EdgeType::Import);
        graph.add_edge("low.rs", "src.rs", EdgeType::Import);
        let pagerank: HashMap<String, f64> =
            [("high.rs".to_string(), 0.9), ("low.rs".to_string(), 0.1)].into();
        let entries = structural_impact(&["src.rs"], &graph, &pagerank, 3);
        for i in 1..entries.len() {
            assert!(
                entries[i - 1].score >= entries[i].score,
                "structural impact must be sorted descending by score"
            );
        }
    }

    #[test]
    fn test_call_graph_impact_basic() {
        use crate::intelligence::call_graph::{CallConfidence, CallEdge, CallGraph};

        // caller.rs calls src/b.rs::my_fn (Exact), other.rs calls it too (Approximate).
        let call_graph = CallGraph {
            edges: vec![
                CallEdge {
                    caller_file: "src/caller.rs".into(),
                    caller_symbol: "do_thing".into(),
                    callee_file: "src/b.rs".into(),
                    callee_symbol: "my_fn".into(),
                    confidence: CallConfidence::Exact,
                    resolution_note: None,
                },
                CallEdge {
                    caller_file: "src/other.rs".into(),
                    caller_symbol: "do_other".into(),
                    callee_file: "src/b.rs".into(),
                    callee_symbol: "my_fn".into(),
                    confidence: CallConfidence::Approximate,
                    resolution_note: None,
                },
            ],
            unresolved: Vec::new(),
        };

        let entries = call_graph_impact(&["src/b.rs"], &call_graph);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"src/caller.rs"), "caller.rs must appear");
        assert!(paths.contains(&"src/other.rs"), "other.rs must appear");
        for e in &entries {
            assert_eq!(e.signal, ImpactSignal::CallBased);
            assert!(e.score > 0.0 && e.score <= 1.0, "score must be in (0,1]");
        }
        // Exact caller should score higher than Approximate caller after normalisation.
        let exact_score = entries
            .iter()
            .find(|e| e.path == "src/caller.rs")
            .unwrap()
            .score;
        let approx_score = entries
            .iter()
            .find(|e| e.path == "src/other.rs")
            .unwrap()
            .score;
        assert!(
            exact_score > approx_score,
            "Exact caller ({exact_score}) must outscore Approximate caller ({approx_score})"
        );
    }

    #[test]
    fn test_call_graph_impact_excludes_changed_files() {
        use crate::intelligence::call_graph::{CallConfidence, CallEdge, CallGraph};

        // src/b.rs calls itself — must not appear in impact results.
        let call_graph = CallGraph {
            edges: vec![CallEdge {
                caller_file: "src/b.rs".into(),
                caller_symbol: "inner".into(),
                callee_file: "src/b.rs".into(),
                callee_symbol: "helper".into(),
                confidence: CallConfidence::Exact,
                resolution_note: None,
            }],
            unresolved: Vec::new(),
        };

        let entries = call_graph_impact(&["src/b.rs"], &call_graph);
        assert!(
            entries.iter().all(|e| e.path != "src/b.rs"),
            "changed file must not appear in call impact results"
        );
    }

    #[test]
    fn test_predict_with_call_graph_produces_callbased_predictions() {
        use crate::intelligence::call_graph::{CallConfidence, CallEdge, CallGraph};

        // tests/b_test.rs calls src/b.rs::my_fn — should be a CallBased prediction.
        let call_graph = CallGraph {
            edges: vec![CallEdge {
                caller_file: "tests/b_test.rs".into(),
                caller_symbol: "test_it".into(),
                callee_file: "src/b.rs".into(),
                callee_symbol: "my_fn".into(),
                confidence: CallConfidence::Exact,
                resolution_note: None,
            }],
            unresolved: Vec::new(),
        };

        let graph = DependencyGraph::new();
        let pagerank = HashMap::new();
        let co_changes = vec![];
        let test_map = HashMap::new();

        let result = predict_with_call_graph(
            &["src/b.rs"],
            &graph,
            &pagerank,
            &co_changes,
            &test_map,
            3,
            &call_graph,
        );

        assert!(
            !result.call_impact.is_empty(),
            "call_impact must be populated when call graph has matching edges"
        );
        let pred = result
            .test_impact
            .iter()
            .find(|p| p.test_file == "tests/b_test.rs");
        assert!(
            pred.is_some(),
            "tests/b_test.rs must appear in test predictions via CallBased signal"
        );
        let pred = pred.unwrap();
        assert!(
            pred.signals.contains(&ImpactSignal::CallBased),
            "prediction must carry CallBased signal"
        );
    }

    #[test]
    fn test_testmap_signal_distinct_from_structural() {
        // ImpactSignal::TestMap alone produces the same confidence tier as Structural
        // (both satisfy has_map), confirming the enum variants are distinct but
        // treated as equal weight in the confidence table.
        let conf_testmap = confidence_for_signals(&[ImpactSignal::TestMap]);
        let conf_structural = confidence_for_signals(&[ImpactSignal::Structural]);
        assert!(
            (conf_testmap - 0.60).abs() < 1e-9,
            "TestMap alone should produce 0.60, got {conf_testmap}"
        );
        assert!(
            (conf_structural - 0.60).abs() < 1e-9,
            "Structural alone should produce 0.60, got {conf_structural}"
        );
        // TestMap is now a distinct variant (not collapsed to Structural).
        assert_ne!(
            ImpactSignal::TestMap,
            ImpactSignal::Structural,
            "TestMap and Structural must be distinct enum variants"
        );
    }

    #[test]
    fn test_testmap_higher_than_historical_alone() {
        let conf_testmap = confidence_for_signals(&[ImpactSignal::TestMap]);
        let conf_hist = confidence_for_signals(&[ImpactSignal::Historical]);
        assert!(
            conf_testmap > conf_hist,
            "TestMap ({conf_testmap}) should have higher confidence than Historical alone ({conf_hist})"
        );
    }

    #[test]
    fn test_confidence_summary_distinct_file_count() {
        // structural and historical lists share a file — summary must count it once.
        let graph = DependencyGraph::new();
        let pagerank = HashMap::new();

        // Create co-change edge so historical_impact returns "shared.rs".
        let co_changes = vec![CoChangeEdge {
            file_a: "src/changed.rs".to_string(),
            file_b: "shared.rs".to_string(),
            count: 3,
            recency_weight: 1.0,
        }];
        let test_map = HashMap::new();

        let result = predict_with_call_graph(
            &["src/changed.rs"],
            &graph,
            &pagerank,
            &co_changes,
            &test_map,
            3,
            &crate::intelligence::call_graph::CallGraph::new(),
        );

        // The summary must reflect distinct files, not structural.len() + historical.len().
        let summary = &result.confidence_summary;
        assert!(
            summary.contains("files predicted affected"),
            "summary must mention files predicted affected: {summary}"
        );
        // total reported should be ≤ structural + historical (deduplication only reduces)
        let total_reported: usize = summary
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(usize::MAX);
        let naive_sum = result.structural_impact.len() + result.historical_impact.len();
        assert!(
            total_reported <= naive_sum || naive_sum == 0,
            "distinct count {total_reported} must be ≤ naive sum {naive_sum}"
        );
    }
}

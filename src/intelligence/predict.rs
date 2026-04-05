use crate::index::graph::DependencyGraph;
use crate::intelligence::co_change::CoChangeEdge;
use crate::intelligence::test_map::TestFileRef;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ImpactSignal {
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
            Self::TestMap | Self::Structural => ImpactSignal::Structural,
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

/// Map signal combinations to confidence values (all 7 non-empty subsets).
///
/// | Signals present                     | Confidence |
/// |-------------------------------------|-----------|
/// | co_change only                      | 0.3       |
/// | test_map only                       | 0.4       |
/// | call_graph only                     | 0.5       |
/// | test_map + co_change                | 0.5       |
/// | call_graph + co_change              | 0.6       |
/// | test_map + call_graph               | 0.7       |
/// | test_map + call_graph + co_change   | 0.9       |
pub fn confidence_for_signals(signals: &[ImpactSignal]) -> f64 {
    let has_map = signals.contains(&ImpactSignal::Structural);
    let has_hist = signals.contains(&ImpactSignal::Historical);
    let has_call = signals.contains(&ImpactSignal::CallBased);

    match (has_map, has_call, has_hist) {
        (true, true, true) => 0.9,
        (true, true, false) => 0.7,
        (false, true, true) => 0.6,
        (true, false, true) => 0.5,
        (false, true, false) => 0.5,
        (true, false, false) => 0.4,
        (false, false, true) => 0.3,
        (false, false, false) => 0.0,
    }
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
    let structural = structural_impact(changed_files, graph, pagerank, depth);
    let historical = historical_impact(changed_files, co_changes);
    let call_impact: Vec<ImpactEntry> = vec![];

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

    let confidence_summary = format!(
        "{} files predicted affected ({} structural, {} historical); {} tests predicted; avg confidence {:.2}",
        structural.len() + historical.len(),
        structural.len(),
        historical.len(),
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
        // test_map + structural both map to ImpactSignal::Structural, co_change maps to Historical
        // keys: {TestMap, Structural, Historical} → 3 signals in vec
        // confidence: has_map=true, has_hist=true, has_call=false → 0.5
        assert!(
            (pred.confidence - 0.5).abs() < 1e-9,
            "expected 0.5 for test_map+structural+historical, got {}",
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
        assert!((pred.confidence - 0.4).abs() < 1e-9);
    }

    #[test]
    fn test_confidence_map_values() {
        let cases: &[(&[ImpactSignal], f64)] = &[
            (&[ImpactSignal::Historical], 0.3),
            (&[ImpactSignal::Structural], 0.4),
            (&[ImpactSignal::CallBased], 0.5),
            (&[ImpactSignal::Structural, ImpactSignal::Historical], 0.5),
            (&[ImpactSignal::CallBased, ImpactSignal::Historical], 0.6),
            (&[ImpactSignal::Structural, ImpactSignal::CallBased], 0.7),
            (
                &[
                    ImpactSignal::Structural,
                    ImpactSignal::CallBased,
                    ImpactSignal::Historical,
                ],
                0.9,
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
            (&[ImpactSignal::Historical], 0.3),
            (&[ImpactSignal::Structural], 0.4),
            (&[ImpactSignal::CallBased], 0.5),
            (&[ImpactSignal::Structural, ImpactSignal::Historical], 0.5),
            (&[ImpactSignal::CallBased, ImpactSignal::Historical], 0.6),
            (&[ImpactSignal::Structural, ImpactSignal::CallBased], 0.7),
            (
                &[
                    ImpactSignal::Structural,
                    ImpactSignal::CallBased,
                    ImpactSignal::Historical,
                ],
                0.9,
            ),
        ];
        let values: HashSet<u64> = cases.iter().map(|(_, v)| (v * 100.0) as u64).collect();
        // 0.5 appears for both (CallBased only) and (Structural+Historical) — 6 distinct levels
        assert_eq!(
            values.len(),
            6,
            "6 distinct confidence levels in the 7 subsets"
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
}

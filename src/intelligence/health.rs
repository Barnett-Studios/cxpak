use crate::index::graph::DependencyGraph;
use crate::index::CodebaseIndex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct HealthScore {
    pub composite: f64,
    pub conventions: f64,
    pub test_coverage: f64,
    pub churn_stability: f64,
    pub coupling: f64,
    pub cycles: f64,
    pub dead_code: Option<f64>,
}

/// Compute the composite health score from the index.
///
/// When `dead_code` is None (v1.2.0), weights are renormalized over 5 dimensions.
pub fn compute_health(index: &CodebaseIndex) -> HealthScore {
    let conventions_score = score_conventions(index);
    let test_coverage_score = score_test_coverage(index);
    let churn_stability_score = score_churn_stability(index);
    let coupling_score = score_coupling(index, 2);
    let cycles_score = score_cycles(&index.graph);

    // dead_code is None until v1.3.0 populates it.
    let dead_code: Option<f64> = None;

    let composite = compute_composite(
        conventions_score,
        test_coverage_score,
        churn_stability_score,
        coupling_score,
        cycles_score,
        dead_code,
    );

    HealthScore {
        composite,
        conventions: conventions_score,
        test_coverage: test_coverage_score,
        churn_stability: churn_stability_score,
        coupling: coupling_score,
        cycles: cycles_score,
        dead_code,
    }
}

/// Conventions dimension: mean PatternStrength adherence across all detected patterns.
/// Convention = 10.0, Trend = 7.0, Mixed = 5.0. Empty profile = 10.0 (no violations detected).
fn score_conventions(index: &CodebaseIndex) -> f64 {
    use crate::conventions::PatternStrength;

    let p = &index.conventions;
    let mut scores: Vec<f64> = Vec::new();

    let mut push = |obs: &Option<crate::conventions::PatternObservation>| {
        if let Some(o) = obs {
            scores.push(match o.strength {
                PatternStrength::Convention => 10.0,
                PatternStrength::Trend => 7.0,
                PatternStrength::Mixed => 5.0,
            });
        }
    };

    push(&p.naming.function_style);
    push(&p.naming.type_style);
    push(&p.naming.constant_style);
    push(&p.imports.style);
    push(&p.errors.result_return);
    push(&p.visibility.public_ratio);
    // functions.avg_length is Option<f64>, not a PatternObservation — skip in this dimension

    if scores.is_empty() {
        return 10.0;
    }
    scores.iter().sum::<f64>() / scores.len() as f64
}

/// Test coverage dimension: ratio of source files with >= 1 mapped test file, scaled to [0, 10].
fn score_test_coverage(index: &CodebaseIndex) -> f64 {
    let source_files: Vec<&str> = index
        .files
        .iter()
        .filter(|f| {
            // Exclude test files themselves from the denominator
            let p = &f.relative_path;
            !p.contains("/tests/")
                && !p.contains("/test/")
                && !p.contains("/spec/")
                && !p.contains("_test.")
                && !p.contains(".test.")
                && !p.contains("_spec.")
                && !p.contains(".spec.")
        })
        .map(|f| f.relative_path.as_str())
        .collect();

    if source_files.is_empty() {
        return 10.0;
    }

    let covered = source_files
        .iter()
        .filter(|path| index.test_map.contains_key(*path as &str))
        .count();

    (covered as f64 / source_files.len() as f64) * 10.0
}

/// Churn stability: inverse of the ratio of "hot" files (>10 changes in 30d).
/// Score = 10.0 * (1.0 - hot_ratio). Empty churn = 10.0.
fn score_churn_stability(index: &CodebaseIndex) -> f64 {
    let churn = &index.conventions.git_health.churn_30d;
    if churn.is_empty() {
        return 10.0;
    }
    let total_files = index.total_files.max(1) as f64;
    let hot_files = churn.iter().filter(|e| e.modifications > 10).count() as f64;
    10.0 * (1.0 - (hot_files / total_files).min(1.0))
}

/// Coupling dimension: 1.0 - mean cross-module edge ratio across qualifying modules.
/// A module qualifies when it has >= 3 files. Returned score is on [0, 10].
/// When no modules qualify, returns 10.0.
/// When a qualifying module has 0 total edges, coupling = 0.0 (fully isolated → unhealthy signal).
pub fn score_coupling(index: &CodebaseIndex, module_depth: usize) -> f64 {
    // Group files into modules by taking the first `module_depth` path segments.
    let mut module_files: HashMap<String, Vec<&str>> = HashMap::new();
    for file in &index.files {
        let prefix = module_prefix(&file.relative_path, module_depth);
        module_files
            .entry(prefix)
            .or_default()
            .push(&file.relative_path);
    }

    let qualifying: Vec<(&String, &Vec<&str>)> = module_files
        .iter()
        .filter(|(_, files)| files.len() >= 3)
        .collect();

    if qualifying.is_empty() {
        return 10.0;
    }

    let module_set: HashSet<String> = qualifying
        .iter()
        .flat_map(|(_, files)| files.iter().map(|f| module_prefix(f, module_depth)))
        .collect();

    let mean_cross_ratio: f64 = qualifying
        .iter()
        .map(|(mod_name, files)| {
            let mut total_edges = 0usize;
            let mut cross_edges = 0usize;

            for &file in files.iter() {
                // Outgoing edges from this file
                if let Some(deps) = index.graph.edges.get(file) {
                    for edge in deps {
                        total_edges += 1;
                        let target_mod = module_prefix(&edge.target, module_depth);
                        if &target_mod != *mod_name {
                            cross_edges += 1;
                        }
                    }
                }
                // Incoming edges (reverse direction)
                if let Some(deps) = index.graph.reverse_edges.get(file) {
                    for edge in deps {
                        total_edges += 1;
                        let src_mod = module_prefix(&edge.target, module_depth);
                        if &src_mod != *mod_name {
                            cross_edges += 1;
                        }
                    }
                }
            }

            let _ = &module_set; // suppress unused warning

            if total_edges == 0 {
                0.0 // fully isolated: treat as 0.0 coupling ratio
            } else {
                cross_edges as f64 / total_edges as f64
            }
        })
        .sum::<f64>()
        / qualifying.len() as f64;

    (1.0 - mean_cross_ratio) * 10.0
}

/// Cycles dimension: 10.0 / (1.0 + scc_count), where scc_count is the number of
/// strongly connected components with size > 1. Logarithmic decay, not clamped.
pub fn score_cycles(graph: &DependencyGraph) -> f64 {
    let scc_count = count_nontrivial_sccs(graph);
    10.0 / (1.0 + scc_count as f64)
}

/// Tarjan's SCC algorithm. Returns the count of SCCs with >1 node (i.e., actual cycles).
pub fn count_nontrivial_sccs(graph: &DependencyGraph) -> usize {
    tarjan_sccs_count(graph)
}

/// Internal Tarjan SCC implementation using an explicit call stack to avoid clippy
/// "too many arguments" and deep recursion issues.
fn tarjan_sccs_count(graph: &DependencyGraph) -> usize {
    // Collect all nodes
    let nodes: Vec<String> = collect_nodes(graph);
    let n = nodes.len();
    if n == 0 {
        return 0;
    }

    let node_index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();

    // Build adjacency list
    let adj: Vec<Vec<usize>> = nodes
        .iter()
        .map(|node| {
            graph
                .edges
                .get(node.as_str())
                .map(|edges| {
                    edges
                        .iter()
                        .filter_map(|e| node_index.get(e.target.as_str()).copied())
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();

    let mut index_counter = 0usize;
    let mut scc_stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; n];
    let mut indices = vec![usize::MAX; n];
    let mut lowlinks = vec![0usize; n];
    let mut nontrivial_count = 0usize;

    // Explicit call stack: (node, adj_iter_pos, parent)
    let mut call_stack: Vec<(usize, usize)> = Vec::new();

    for start in 0..n {
        if indices[start] != usize::MAX {
            continue;
        }
        call_stack.push((start, 0));
        indices[start] = index_counter;
        lowlinks[start] = index_counter;
        index_counter += 1;
        scc_stack.push(start);
        on_stack[start] = true;

        while let Some((v, ref mut adj_pos)) = call_stack.last_mut() {
            let v = *v;
            let pos = *adj_pos;
            if pos < adj[v].len() {
                let w = adj[v][pos];
                *call_stack.last_mut().unwrap() = (v, pos + 1);
                if indices[w] == usize::MAX {
                    indices[w] = index_counter;
                    lowlinks[w] = index_counter;
                    index_counter += 1;
                    scc_stack.push(w);
                    on_stack[w] = true;
                    call_stack.push((w, 0));
                } else if on_stack[w] {
                    lowlinks[v] = lowlinks[v].min(indices[w]);
                }
            } else {
                call_stack.pop();
                if let Some(&(parent, _)) = call_stack.last() {
                    lowlinks[parent] = lowlinks[parent].min(lowlinks[v]);
                }
                if lowlinks[v] == indices[v] {
                    let mut scc_size = 0;
                    loop {
                        let w = scc_stack.pop().unwrap();
                        on_stack[w] = false;
                        scc_size += 1;
                        if w == v {
                            break;
                        }
                    }
                    if scc_size > 1 {
                        nontrivial_count += 1;
                    }
                }
            }
        }
    }

    nontrivial_count
}

fn collect_nodes(graph: &DependencyGraph) -> Vec<String> {
    let mut set = HashSet::new();
    for (k, edges) in &graph.edges {
        set.insert(k.clone());
        for e in edges {
            set.insert(e.target.clone());
        }
    }
    for (k, edges) in &graph.reverse_edges {
        set.insert(k.clone());
        for e in edges {
            set.insert(e.target.clone());
        }
    }
    let mut v: Vec<_> = set.into_iter().collect();
    v.sort(); // deterministic
    v
}

/// Composite with optional dead_code. When None, renormalize the 5 active weights to 1.0.
pub fn compute_composite(
    conventions: f64,
    test_coverage: f64,
    churn_stability: f64,
    coupling: f64,
    cycles: f64,
    dead_code: Option<f64>,
) -> f64 {
    match dead_code {
        Some(dc) => {
            // Full 6-dimension weights (sum = 1.0)
            0.20 * conventions
                + 0.20 * test_coverage
                + 0.15 * churn_stability
                + 0.20 * coupling
                + 0.15 * cycles
                + 0.10 * dc
        }
        None => {
            // 5-dimension weights renormalized: each / 0.90 (sum = 1.0)
            (0.20 / 0.90) * conventions
                + (0.20 / 0.90) * test_coverage
                + (0.15 / 0.90) * churn_stability
                + (0.20 / 0.90) * coupling
                + (0.15 / 0.90) * cycles
        }
    }
}

pub(crate) fn module_prefix(path: &str, depth: usize) -> String {
    // Split into at most (depth + 1) segments so we can take up to `depth` directory components.
    // The last segment is always the filename, so we never include it in the module prefix.
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() <= 1 {
        // No directory separator: the whole path is the module (top-level file)
        return path.to_string();
    }
    // Take up to `depth` segments from the directory portion (excluding filename)
    let dir_segments = &segments[..segments.len() - 1];
    let take = depth.min(dir_segments.len());
    dir_segments[..take].join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::graph::DependencyGraph;
    use crate::schema::EdgeType;

    #[test]
    fn test_compute_composite_without_dead_code_sums_to_10() {
        // All dimensions at 10.0, dead_code = None -> composite = 10.0
        let composite = compute_composite(10.0, 10.0, 10.0, 10.0, 10.0, None);
        assert!((composite - 10.0).abs() < 1e-6, "got {composite}");
    }

    #[test]
    fn test_compute_composite_with_dead_code_sums_to_10() {
        let composite = compute_composite(10.0, 10.0, 10.0, 10.0, 10.0, Some(10.0));
        assert!((composite - 10.0).abs() < 1e-6, "got {composite}");
    }

    #[test]
    fn test_compute_composite_weights_renormalized() {
        // With dead_code=None, renormalized weights must sum to 1.0.
        // Verify: (0.20+0.20+0.15+0.20+0.15)/0.90 = 0.90/0.90 = 1.0
        let w_sum: f64 = (0.20 + 0.20 + 0.15 + 0.20 + 0.15) / 0.90;
        assert!((w_sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_score_cycles_no_cycles() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        // Linear graph, no cycles -> 0 nontrivial SCCs -> 10.0 / 1.0 = 10.0
        let score = score_cycles(&graph);
        assert!((score - 10.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn test_score_cycles_one_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "a.rs", EdgeType::Import);
        // One nontrivial SCC -> 10.0 / 2.0 = 5.0
        let score = score_cycles(&graph);
        assert!((score - 5.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn test_score_cycles_two_independent_cycles() {
        let mut graph = DependencyGraph::new();
        // Cycle 1: a <-> b
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "a.rs", EdgeType::Import);
        // Cycle 2: c <-> d
        graph.add_edge("c.rs", "d.rs", EdgeType::Import);
        graph.add_edge("d.rs", "c.rs", EdgeType::Import);
        // Two nontrivial SCCs -> 10.0 / 3.0
        let score = score_cycles(&graph);
        assert!((score - 10.0 / 3.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn test_count_nontrivial_sccs_empty_graph() {
        let graph = DependencyGraph::new();
        assert_eq!(count_nontrivial_sccs(&graph), 0);
    }

    #[test]
    fn test_module_prefix_depth_2() {
        assert_eq!(module_prefix("src/api/handler.rs", 2), "src/api");
        assert_eq!(module_prefix("src/lib.rs", 2), "src");
        assert_eq!(module_prefix("main.rs", 2), "main.rs");
    }

    #[test]
    fn test_score_coupling_no_qualifying_modules() {
        // Fewer than 3 files per module -> score = 10.0
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn a() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "src/a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert!((score_coupling(&index, 2) - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_health_score_serialize() {
        let h = HealthScore {
            composite: 8.5,
            conventions: 9.0,
            test_coverage: 7.0,
            churn_stability: 8.0,
            coupling: 9.5,
            cycles: 10.0,
            dead_code: None,
        };
        let json = serde_json::to_string(&h).unwrap();
        assert!(json.contains("\"composite\":8.5"));
        assert!(json.contains("\"dead_code\":null"));
    }

    #[test]
    fn test_composite_all_combinations_within_range() {
        let values = [0.0_f64, 5.0, 10.0];
        for &c in &values {
            for &t in &values {
                for &ch in &values {
                    for &cp in &values {
                        for &cy in &values {
                            let comp = compute_composite(c, t, ch, cp, cy, None);
                            assert!(
                                (0.0..=10.0).contains(&comp),
                                "composite out of range [{c},{t},{ch},{cp},{cy}]: {comp}"
                            );
                            for &dc in &values {
                                let comp6 = compute_composite(c, t, ch, cp, cy, Some(dc));
                                assert!(
                                    (0.0..=10.0).contains(&comp6),
                                    "composite (with dead_code) out of range: {comp6}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_score_cycles_invariants() {
        let mut graph = DependencyGraph::new();
        let score_0 = score_cycles(&graph);
        assert!((score_0 - 10.0).abs() < 1e-6, "0 cycles must give 10.0");

        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "a.rs", EdgeType::Import);
        let score_1 = score_cycles(&graph);
        assert!(score_1 < score_0, "1 cycle must score lower than 0 cycles");
        assert!((score_1 - 5.0).abs() < 1e-6, "1 cycle -> 10/2 = 5.0");

        graph.add_edge("c.rs", "d.rs", EdgeType::Import);
        graph.add_edge("d.rs", "c.rs", EdgeType::Import);
        let score_2 = score_cycles(&graph);
        assert!(score_2 < score_1, "2 cycles must score lower than 1 cycle");
    }

    #[test]
    fn test_risk_score_multiplicative_floor() {
        let min = 0.01_f64 * 0.01 * 0.01;
        assert!(
            (min - 1e-6).abs() < 1e-15,
            "minimum risk floor must be 1e-6"
        );
        assert!((1.0_f64.max(0.01) * 1.0_f64.max(0.01) * 1.0_f64.max(0.01) - 1.0).abs() < 1e-9);
    }
}

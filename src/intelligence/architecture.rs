use crate::index::CodebaseIndex;
use crate::intelligence::health::module_prefix;
use crate::schema::EdgeType;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct ArchitectureMap {
    pub modules: Vec<ModuleInfo>,
    pub circular_deps: Vec<Vec<String>>,
}

/// A cross-module import that bypasses the module's public interface.
#[derive(Debug, Clone, Serialize)]
pub struct BoundaryViolation {
    pub source_file: String,
    pub target_file: String,
    pub target_module: String,
    pub edge_type: EdgeType,
}

/// Per-module quality metrics.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleInfo {
    // v1.2.0 fields
    pub prefix: String,
    pub file_count: usize,
    pub aggregate_pagerank: f64,
    pub coupling: f64,
    // v1.3.0 fields
    pub cohesion: f64,
    pub boundary_violations: Vec<BoundaryViolation>,
    pub god_files: Vec<String>,
}

/// Cohesion = ratio of actual intra-module edges to maximum possible.
/// Maximum possible for N files = N * (N-1) directed edges.
/// Returns 0.0 for single-file modules (undefined ratio).
pub fn compute_cohesion(intra_edges: usize, file_count: usize) -> f64 {
    if file_count <= 1 {
        return 0.0;
    }
    let max_possible = file_count * (file_count - 1);
    if max_possible == 0 {
        return 0.0;
    }
    (intra_edges as f64 / max_possible as f64).min(1.0)
}

/// Returns true when `target_path` is not a root file of `target_module`.
///
/// Root files are: mod.rs, lib.rs, index.ts, index.js, __init__.py.
/// A file in `src/db/internal/pool.rs` is not the root of `src/db` -> violation.
/// A file in `src/db/mod.rs` IS the root of `src/db` -> not a violation.
pub fn is_boundary_violation(target_path: &str, target_module: &str) -> bool {
    let root_files = ["mod.rs", "lib.rs", "index.ts", "index.js", "__init__.py"];
    let filename = target_path.rsplit('/').next().unwrap_or(target_path);
    // Not a violation if target is the barrel/root file of its module
    if root_files.contains(&filename) {
        let parent = target_path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        return parent != target_module;
    }
    // Violation if file is in a subdirectory of the target module
    let direct = format!("{}/", target_module);
    target_path
        .strip_prefix(&direct)
        .map(|rest| rest.contains('/'))
        .unwrap_or(true)
}

/// Files with inbound count > mean + 2sigma are god files.
pub fn detect_god_files<'a>(inbound_counts: &[(&'a str, usize)]) -> Vec<&'a str> {
    if inbound_counts.len() < 3 {
        return vec![];
    }
    let counts: Vec<f64> = inbound_counts.iter().map(|(_, c)| *c as f64).collect();
    let mean = counts.iter().sum::<f64>() / counts.len() as f64;
    let variance = counts.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / counts.len() as f64;
    let sigma = variance.sqrt();
    let threshold = mean + 2.0 * sigma;
    inbound_counts
        .iter()
        .filter(|(_, c)| *c as f64 > threshold)
        .map(|(path, _)| *path)
        .collect()
}

/// Build the architecture map for the index.
///
/// `module_depth` controls how many path segments form a module name (default 2).
pub fn build_architecture_map(index: &CodebaseIndex, module_depth: usize) -> ArchitectureMap {
    let mut module_files: HashMap<String, Vec<String>> = HashMap::new();
    for file in &index.files {
        let prefix = module_prefix(&file.relative_path, module_depth);
        module_files
            .entry(prefix)
            .or_default()
            .push(file.relative_path.clone());
    }

    // Pre-compute per-file inbound edge counts (for god file detection)
    let mut inbound_counts: HashMap<String, usize> = HashMap::new();
    for edges in index.graph.reverse_edges.values() {
        for edge in edges {
            *inbound_counts.entry(edge.target.clone()).or_default() += 1;
        }
    }
    // Also count call graph inbound edges
    for call_edge in &index.call_graph.edges {
        *inbound_counts
            .entry(call_edge.callee_file.clone())
            .or_default() += 1;
    }

    let modules: Vec<ModuleInfo> = {
        let mut mods: Vec<ModuleInfo> = module_files
            .iter()
            .map(|(prefix, files)| {
                let file_set: HashSet<&str> = files.iter().map(|f| f.as_str()).collect();
                let aggregate_pagerank: f64 = files
                    .iter()
                    .map(|f| index.pagerank.get(f.as_str()).copied().unwrap_or(0.0))
                    .sum();

                // Coupling: ratio of cross-module outgoing edges to total outgoing edges.
                //
                // Only outgoing edges (graph.edges) are iterated here.  Previously
                // the loop also iterated reverse_edges, which counted every edge
                // twice in `total_edges` while intra-module edges were only counted
                // once — inflating the denominator and making cohesion appear near
                // zero even for tightly coupled modules.
                let mut total_edges = 0usize;
                let mut cross_edges = 0usize;
                let mut intra_edges = 0usize;
                for file in files {
                    if let Some(deps) = index.graph.edges.get(file.as_str()) {
                        for edge in deps {
                            total_edges += 1;
                            let target_mod = module_prefix(&edge.target, module_depth);
                            if target_mod != *prefix {
                                cross_edges += 1;
                            } else {
                                intra_edges += 1;
                            }
                        }
                    }
                }
                let coupling = if total_edges == 0 {
                    0.0
                } else {
                    cross_edges as f64 / total_edges as f64
                };

                // Cohesion
                let cohesion = compute_cohesion(intra_edges, files.len());

                // Boundary violations: cross-module edges where target is not a root file
                let mut boundary_violations: Vec<BoundaryViolation> = Vec::new();
                for file in files {
                    if let Some(deps) = index.graph.edges.get(file.as_str()) {
                        for edge in deps {
                            let target_mod = module_prefix(&edge.target, module_depth);
                            if target_mod != *prefix
                                && !file_set.contains(edge.target.as_str())
                                && is_boundary_violation(&edge.target, &target_mod)
                            {
                                boundary_violations.push(BoundaryViolation {
                                    source_file: file.clone(),
                                    target_file: edge.target.clone(),
                                    target_module: target_mod,
                                    edge_type: edge.edge_type.clone(),
                                });
                            }
                        }
                    }
                }

                // God files: files in this module with inbound count > mean + 2sigma
                let module_inbound: Vec<(&str, usize)> = files
                    .iter()
                    .map(|f| {
                        (
                            f.as_str(),
                            inbound_counts.get(f.as_str()).copied().unwrap_or(0),
                        )
                    })
                    .collect();
                let god_files: Vec<String> = detect_god_files(&module_inbound)
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect();

                ModuleInfo {
                    prefix: prefix.clone(),
                    file_count: files.len(),
                    aggregate_pagerank,
                    coupling,
                    cohesion,
                    boundary_violations,
                    god_files,
                }
            })
            .collect();
        mods.sort_by(|a, b| {
            b.aggregate_pagerank
                .partial_cmp(&a.aggregate_pagerank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        mods
    };

    let circular_deps = find_circular_dep_groups(index);

    ArchitectureMap {
        modules,
        circular_deps,
    }
}

/// Returns ordered path lists for each non-trivial SCC using an iterative Tarjan implementation.
fn find_circular_dep_groups(index: &CodebaseIndex) -> Vec<Vec<String>> {
    let nodes: Vec<String> = {
        let mut set = HashSet::new();
        for (k, edges) in &index.graph.edges {
            set.insert(k.clone());
            for e in edges {
                set.insert(e.target.clone());
            }
        }
        let mut v: Vec<_> = set.into_iter().collect();
        v.sort();
        v
    };

    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }

    let node_index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();

    let adj: Vec<Vec<usize>> = nodes
        .iter()
        .map(|node| {
            index
                .graph
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
    let mut cycles: Vec<Vec<String>> = Vec::new();
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
                    let mut scc: Vec<String> = Vec::new();
                    loop {
                        let w = scc_stack.pop().unwrap();
                        on_stack[w] = false;
                        scc.push(nodes[w].clone());
                        if w == v {
                            break;
                        }
                    }
                    if scc.len() > 1 {
                        scc.sort();
                        cycles.push(scc);
                    }
                }
            }
        }
    }

    cycles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    #[test]
    fn test_build_architecture_map_empty_index() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let map = build_architecture_map(&index, 2);
        assert!(map.modules.is_empty());
        assert!(map.circular_deps.is_empty());
    }

    #[test]
    fn test_architecture_map_groups_by_prefix() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let make_file = |name: &str, content: &str| {
            let safe = name.replace('/', "_");
            let fp = dir.path().join(&safe);
            std::fs::write(&fp, content).unwrap();
            ScannedFile {
                relative_path: name.to_string(),
                absolute_path: fp,
                language: Some("rust".into()),
                size_bytes: content.len() as u64,
            }
        };
        let files = vec![
            make_file("src/api/handler.rs", "fn h() {}"),
            make_file("src/api/router.rs", "fn r() {}"),
            make_file("src/db/query.rs", "fn q() {}"),
        ];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let map = build_architecture_map(&index, 2);
        let prefixes: Vec<&str> = map.modules.iter().map(|m| m.prefix.as_str()).collect();
        assert!(prefixes.contains(&"src/api"), "src/api module expected");
        assert!(prefixes.contains(&"src/db"), "src/db module expected");
    }

    #[test]
    fn test_circular_deps_detected() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let make_file = |name: &str| {
            let safe = name.replace('/', "_");
            let fp = dir.path().join(&safe);
            std::fs::write(&fp, "fn f() {}").unwrap();
            ScannedFile {
                relative_path: name.to_string(),
                absolute_path: fp,
                language: Some("rust".into()),
                size_bytes: 9,
            }
        };
        let mut index = CodebaseIndex::build(
            vec![make_file("a.rs"), make_file("b.rs")],
            HashMap::new(),
            &counter,
        );
        index.graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        index.graph.add_edge("b.rs", "a.rs", EdgeType::Import);

        let map = build_architecture_map(&index, 2);
        assert_eq!(map.circular_deps.len(), 1, "one cycle expected");
        let cycle = &map.circular_deps[0];
        assert!(cycle.contains(&"a.rs".to_string()));
        assert!(cycle.contains(&"b.rs".to_string()));
    }

    #[test]
    fn test_architecture_map_serialize() {
        let map = ArchitectureMap {
            modules: vec![ModuleInfo {
                prefix: "src/api".into(),
                file_count: 3,
                aggregate_pagerank: 2.5,
                coupling: 0.4,
                cohesion: 0.5,
                boundary_violations: vec![],
                god_files: vec![],
            }],
            circular_deps: vec![vec!["a.rs".into(), "b.rs".into()]],
        };
        let json = serde_json::to_string(&map).unwrap();
        assert!(json.contains("\"prefix\":\"src/api\""));
        assert!(json.contains("\"circular_deps\""));
        assert!(json.contains("\"cohesion\""));
    }

    // --- Cohesion tests ---

    #[test]
    fn test_cohesion_fully_connected_module() {
        let cohesion = compute_cohesion(6, 3);
        assert!(
            (cohesion - 1.0).abs() < 1e-9,
            "expected 1.0, got {cohesion}"
        );
    }

    #[test]
    fn test_cohesion_isolated_module() {
        let cohesion = compute_cohesion(0, 3);
        assert!(
            (cohesion - 0.0).abs() < 1e-9,
            "expected 0.0, got {cohesion}"
        );
    }

    #[test]
    fn test_cohesion_single_file_module() {
        let cohesion = compute_cohesion(0, 1);
        assert!(
            (cohesion - 0.0).abs() < 1e-9,
            "single-file module cohesion = 0.0"
        );
    }

    // --- Boundary violation tests ---

    #[test]
    fn test_boundary_violation_detects_non_root_import() {
        assert!(is_boundary_violation("src/db/internal/pool.rs", "src/db"));
        assert!(!is_boundary_violation("src/db/mod.rs", "src/db"));
        assert!(!is_boundary_violation("src/db/lib.rs", "src/db"));
    }

    #[test]
    fn test_boundary_violation_index_ts_not_violation() {
        assert!(!is_boundary_violation("src/api/index.ts", "src/api"));
    }

    // --- God file tests ---

    #[test]
    fn test_god_file_detection_mean_plus_2sigma() {
        // mean = (1+2+2+100)/4 = 26.25
        // variance = ((1-26.25)^2 + (2-26.25)^2 + (2-26.25)^2 + (100-26.25)^2) / 4 = ~1507.19
        // sigma ~= 38.82, threshold ~= 103.9 ... still too high.
        // Use a tighter cluster with one clear outlier:
        // mean = (1+1+1+20)/4 = 5.75
        // variance = ((1-5.75)^2*3 + (20-5.75)^2) / 4 = (67.6875 + 203.0625) / 4 = 67.6875
        // sigma ~= 8.23, threshold ~= 22.2 => 20 < 22.2
        // Need even tighter. Use (1,1,1,1,1,50):
        // mean=9.17, var=338.8, sigma=18.4, threshold=45.97 => 50 > 45.97 works!
        let inbound_counts = vec![
            ("a.rs", 1usize),
            ("b.rs", 1),
            ("c.rs", 1),
            ("d.rs", 1),
            ("e.rs", 1),
            ("f.rs", 50),
        ];
        let god_files = detect_god_files(&inbound_counts);
        assert!(god_files.contains(&"f.rs"), "f.rs should be a god file");
        assert!(
            !god_files.contains(&"a.rs"),
            "a.rs should not be a god file"
        );
    }

    #[test]
    fn test_god_file_detection_requires_at_least_3_files() {
        let counts = vec![("a.rs", 100usize)];
        let gods = detect_god_files(&counts);
        assert!(gods.is_empty(), "single file should never be a god file");
    }

    // --- Coupling double-count regression (a7ef720) ---
    //
    // Bug: build_architecture_map() iterated BOTH graph.edges AND
    // graph.reverse_edges when computing total_edges, counting every edge
    // twice.  This inflated the denominator so cohesion appeared near zero
    // even for tightly-coupled modules.
    //
    // The test would FAIL against the pre-fix code:
    //  - With 2 intra-module edges and 0 cross-module edges the pre-fix code
    //    would set total_edges = 4 (edges counted via forward AND reverse),
    //    but intra_edges = 2.  cohesion = compute_cohesion(2, 2) = 1.0 because
    //    that helper was not the bug — the bug was the denominator passed into
    //    the coupling calculation.  However, with coupling = 0/4 = 0.0 and
    //    cohesion = 2/(2*(2-1)) = 1.0 for 2 intra-module files both sides were
    //    actually correct for the simple 2-file case.
    //
    //  The observable symptom is coupling reported as 0/total where total is
    //  double the actual edge count.  With 1 cross + 1 intra edge:
    //    pre-fix:  total = 4 (2 forward + 2 reverse), coupling = 1/4 = 0.25
    //    post-fix: total = 2 (forward only),           coupling = 1/2 = 0.50
    #[test]
    fn test_coupling_not_double_counted_via_reverse_edges() {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        // Build a small index: src/a/x.rs → src/a/y.rs (intra) and
        //                       src/a/x.rs → src/b/z.rs (cross).
        let make = |name: &str| {
            let fp = dir.path().join(name.replace('/', "_"));
            std::fs::write(&fp, "fn f() {}").unwrap();
            ScannedFile {
                relative_path: name.to_string(),
                absolute_path: fp,
                language: Some("rust".into()),
                size_bytes: 9,
            }
        };
        let mut index = CodebaseIndex::build(
            vec![make("src/a/x.rs"), make("src/a/y.rs"), make("src/b/z.rs")],
            HashMap::new(),
            &counter,
        );

        // One intra-module edge (a→a) and one cross-module edge (a→b).
        index
            .graph
            .add_edge("src/a/x.rs", "src/a/y.rs", EdgeType::Import);
        index
            .graph
            .add_edge("src/a/x.rs", "src/b/z.rs", EdgeType::Import);

        let map = build_architecture_map(&index, 2);
        let mod_a = map
            .modules
            .iter()
            .find(|m| m.prefix == "src/a")
            .expect("src/a module must exist");

        // Post-fix: 2 total outgoing edges, 1 cross → coupling = 0.5
        // Pre-fix: 4 total (each edge counted twice via reverse) → coupling = 0.25
        assert!(
            (mod_a.coupling - 0.5).abs() < 1e-9,
            "coupling must be 0.5 (1 cross / 2 total), got {}; \
             if 0.25 then reverse-edge double-count bug is back",
            mod_a.coupling
        );
    }

    // --- ModuleInfo v1.3.0 fields ---

    #[test]
    fn test_module_info_has_v130_fields() {
        let mi = ModuleInfo {
            prefix: "src/api".into(),
            file_count: 3,
            aggregate_pagerank: 0.8,
            coupling: 0.3,
            cohesion: 0.5,
            boundary_violations: vec![],
            god_files: vec![],
        };
        assert_eq!(mi.prefix, "src/api");
        assert!((mi.cohesion - 0.5).abs() < 1e-9);
    }
}

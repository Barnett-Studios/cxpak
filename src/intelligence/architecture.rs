use crate::index::CodebaseIndex;
use crate::intelligence::health::module_prefix;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct ArchitectureMap {
    pub modules: Vec<ModuleInfo>,
    pub circular_deps: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleInfo {
    pub prefix: String,
    pub file_count: usize,
    pub aggregate_pagerank: f64,
    pub coupling: f64,
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

    let modules: Vec<ModuleInfo> = {
        let mut mods: Vec<ModuleInfo> = module_files
            .iter()
            .map(|(prefix, files)| {
                let aggregate_pagerank: f64 = files
                    .iter()
                    .map(|f| index.pagerank.get(f.as_str()).copied().unwrap_or(0.0))
                    .sum();

                // Coupling: cross-module edge ratio (outgoing + incoming / total)
                let mut total_edges = 0usize;
                let mut cross_edges = 0usize;
                for file in files {
                    if let Some(deps) = index.graph.edges.get(file.as_str()) {
                        for edge in deps {
                            total_edges += 1;
                            let target_mod = module_prefix(&edge.target, module_depth);
                            if target_mod != *prefix {
                                cross_edges += 1;
                            }
                        }
                    }
                    if let Some(deps) = index.graph.reverse_edges.get(file.as_str()) {
                        for edge in deps {
                            total_edges += 1;
                            let src_mod = module_prefix(&edge.target, module_depth);
                            if src_mod != *prefix {
                                cross_edges += 1;
                            }
                        }
                    }
                }
                let coupling = if total_edges == 0 {
                    0.0
                } else {
                    cross_edges as f64 / total_edges as f64
                };

                ModuleInfo {
                    prefix: prefix.clone(),
                    file_count: files.len(),
                    aggregate_pagerank,
                    coupling,
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

    // Detect circular deps via Tarjan's SCC on the full dependency graph.
    // Each SCC with >1 node is a circular dependency group.
    let circular_deps = find_circular_dep_groups(index);

    ArchitectureMap {
        modules,
        circular_deps,
    }
}

/// Returns ordered path lists for each non-trivial SCC using an iterative Tarjan implementation.
fn find_circular_dep_groups(index: &CodebaseIndex) -> Vec<Vec<String>> {
    // Build node list (forward edges only for circular dep detection)
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

    // Build adjacency list
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

    // Explicit call stack to avoid clippy "too many arguments" from recursive fn
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
                        scc.sort(); // deterministic ordering
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
    use crate::schema::EdgeType;
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
        // Manually inject a cycle: a -> b -> a
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
            }],
            circular_deps: vec![vec!["a.rs".into(), "b.rs".into()]],
        };
        let json = serde_json::to_string(&map).unwrap();
        assert!(json.contains("\"prefix\":\"src/api\""));
        assert!(json.contains("\"circular_deps\""));
    }
}

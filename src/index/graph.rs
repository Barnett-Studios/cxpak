use super::CodebaseIndex;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<String, HashSet<String>>,
    pub reverse_edges: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_edge(&mut self, from: &str, to: &str) {
        self.edges
            .entry(from.to_string())
            .or_default()
            .insert(to.to_string());
        self.reverse_edges
            .entry(to.to_string())
            .or_default()
            .insert(from.to_string());
    }

    pub fn dependents(&self, path: &str) -> Vec<&str> {
        self.reverse_edges
            .get(path)
            .map(|set| set.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    pub fn dependencies(&self, path: &str) -> Option<&HashSet<String>> {
        self.edges.get(path)
    }

    /// Remove all outgoing edges from `source` and clean up corresponding reverse edges.
    ///
    /// Used during incremental re-indexing: call this before re-adding the new
    /// edges from a freshly parsed file.
    pub fn remove_edges_for(&mut self, source: &str) {
        if let Some(targets) = self.edges.remove(source) {
            for target in &targets {
                if let Some(rev) = self.reverse_edges.get_mut(target.as_str()) {
                    rev.remove(source);
                    if rev.is_empty() {
                        self.reverse_edges.remove(target.as_str());
                    }
                }
            }
        }
    }

    /// BFS from `start_files`, following edges in both directions.
    ///
    /// Returns the set of all reachable file paths, including the start files
    /// themselves.
    pub fn reachable_from(&self, start_files: &[&str]) -> HashSet<String> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        for &path in start_files {
            if visited.insert(path.to_string()) {
                queue.push_back(path.to_string());
            }
        }

        while let Some(current) = queue.pop_front() {
            // Follow outgoing edges (files that `current` imports)
            if let Some(deps) = self.edges.get(&current) {
                for dep in deps {
                    if visited.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }

            // Follow incoming edges (files that import `current`)
            if let Some(importers) = self.reverse_edges.get(&current) {
                for importer in importers {
                    if visited.insert(importer.clone()) {
                        queue.push_back(importer.clone());
                    }
                }
            }
        }

        visited
    }
}

/// Build a `DependencyGraph` from the index by resolving import source paths to
/// indexed file paths.  We do a best-effort match: convert the module path
/// (e.g. `crate::scanner`) to a file path (e.g. `src/scanner/mod.rs` or
/// `src/scanner.rs`) and look up whether such a file exists.
pub fn build_dependency_graph(index: &CodebaseIndex) -> DependencyGraph {
    let all_paths: HashSet<&str> = index
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();

    let mut graph = DependencyGraph::new();

    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };

        for import in &pr.imports {
            // Try to resolve the import source to an actual file in the index.
            // We convert path separators and try common file extensions.
            let candidate_base = import.source.replace("::", "/").replace('.', "/");
            let candidates = [
                format!("{candidate_base}.rs"),
                format!("{candidate_base}/mod.rs"),
                format!("src/{candidate_base}.rs"),
                format!("src/{candidate_base}/mod.rs"),
                format!("{candidate_base}.ts"),
                format!("{candidate_base}.js"),
                format!("{candidate_base}.py"),
                format!("{candidate_base}.go"),
                format!("{candidate_base}.java"),
            ];

            for candidate in &candidates {
                if all_paths.contains(candidate.as_str()) {
                    graph.add_edge(&file.relative_path, candidate);
                    break;
                }
            }
        }
    }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_graph() {
        let graph = DependencyGraph::new();
        assert!(graph.edges.is_empty());
        assert!(graph.dependents("any").is_empty());
        assert!(graph.dependencies("any").is_none());
    }

    #[test]
    fn test_add_edge() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        assert!(graph.edges.contains_key("a.rs"));
        assert!(graph.edges["a.rs"].contains("b.rs"));
    }

    #[test]
    fn test_dependents() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("c.rs", "b.rs");
        let deps = graph.dependents("b.rs");
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"a.rs"));
        assert!(deps.contains(&"c.rs"));
    }

    #[test]
    fn test_dependencies() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("a.rs", "c.rs");
        let deps = graph.dependencies("a.rs").unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.contains("b.rs"));
        assert!(deps.contains("c.rs"));
    }

    #[test]
    fn test_dependencies_none() {
        let graph = DependencyGraph::new();
        assert!(graph.dependencies("nonexistent").is_none());
    }

    #[test]
    fn test_reachable_from_single() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("b.rs", "c.rs");
        let reachable = graph.reachable_from(&["a.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
        assert!(reachable.contains("c.rs"));
    }

    #[test]
    fn test_reachable_from_reverse() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        let reachable = graph.reachable_from(&["b.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
    }

    #[test]
    fn test_reachable_from_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("b.rs", "c.rs");
        graph.add_edge("c.rs", "a.rs");
        let reachable = graph.reachable_from(&["a.rs"]);
        assert_eq!(reachable.len(), 3);
    }

    #[test]
    fn test_reachable_from_disconnected() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("c.rs", "d.rs");
        let reachable = graph.reachable_from(&["a.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
        assert!(!reachable.contains("c.rs"));
        assert!(!reachable.contains("d.rs"));
    }

    #[test]
    fn test_reachable_from_empty_start() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        let reachable = graph.reachable_from(&[]);
        assert!(reachable.is_empty());
    }

    #[test]
    fn test_duplicate_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("a.rs", "b.rs");
        assert_eq!(graph.edges["a.rs"].len(), 1);
    }

    #[test]
    fn test_reverse_edges_maintained() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("c.rs", "b.rs");
        graph.add_edge("a.rs", "d.rs");
        // reverse_edges should exist and be populated
        assert!(graph.reverse_edges.get("b.rs").unwrap().contains("a.rs"));
        assert!(graph.reverse_edges.get("b.rs").unwrap().contains("c.rs"));
        assert!(graph.reverse_edges.get("d.rs").unwrap().contains("a.rs"));
        assert_eq!(graph.reverse_edges.get("b.rs").unwrap().len(), 2);
    }

    #[test]
    fn test_remove_edges_for_file() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("a.rs", "c.rs");
        graph.add_edge("d.rs", "b.rs");

        graph.remove_edges_for("a.rs");

        // a.rs edges should be gone
        assert!(!graph.edges.contains_key("a.rs"));
        // b.rs should only have d.rs as dependent now
        let b_deps = graph.dependents("b.rs");
        assert_eq!(b_deps.len(), 1);
        assert!(b_deps.contains(&"d.rs"));
        // c.rs should have no dependents
        assert!(graph.dependents("c.rs").is_empty());
    }

    #[test]
    fn test_remove_edges_for_nonexistent() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.remove_edges_for("z.rs"); // no-op
        assert_eq!(graph.edges["a.rs"].len(), 1);
    }

    #[test]
    fn test_remove_and_readd_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("a.rs", "c.rs");

        // Simulate re-parse: remove old, add new
        graph.remove_edges_for("a.rs");
        graph.add_edge("a.rs", "d.rs");

        assert_eq!(graph.edges["a.rs"].len(), 1);
        assert!(graph.edges["a.rs"].contains("d.rs"));
        assert!(graph.dependents("b.rs").is_empty());
        assert!(graph.dependents("c.rs").is_empty());
        assert_eq!(graph.dependents("d.rs"), vec!["a.rs"]);
    }

    #[test]
    fn test_dependents_large_graph() {
        let mut graph = DependencyGraph::new();
        for i in 0..100 {
            graph.add_edge(&format!("file_{i}.rs"), "common.rs");
        }
        let deps = graph.dependents("common.rs");
        assert_eq!(deps.len(), 100);
    }
}

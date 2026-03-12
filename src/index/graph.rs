use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<String, HashSet<String>>,
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
    }

    pub fn dependents(&self, path: &str) -> Vec<&str> {
        self.edges
            .iter()
            .filter(|(_, deps)| deps.contains(path))
            .map(|(k, _)| k.as_str())
            .collect()
    }

    pub fn dependencies(&self, path: &str) -> Option<&HashSet<String>> {
        self.edges.get(path)
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
            for (importer, deps) in &self.edges {
                if deps.contains(&current) && visited.insert(importer.clone()) {
                    queue.push_back(importer.clone());
                }
            }
        }

        visited
    }
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
}

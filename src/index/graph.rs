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

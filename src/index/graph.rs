use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self { Self::default() }

    pub fn add_edge(&mut self, from: &str, to: &str) {
        self.edges.entry(from.to_string()).or_default().insert(to.to_string());
    }

    pub fn dependents(&self, path: &str) -> Vec<&str> {
        self.edges.iter()
            .filter(|(_, deps)| deps.contains(path))
            .map(|(k, _)| k.as_str())
            .collect()
    }

    pub fn dependencies(&self, path: &str) -> Option<&HashSet<String>> {
        self.edges.get(path)
    }
}

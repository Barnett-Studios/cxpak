use crate::conventions::PatternObservation;
use crate::index::CodebaseIndex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyConventions {
    pub strict_layers: Vec<DirectionPair>,
    pub circular_deps: Vec<String>,
    pub db_isolation: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectionPair {
    pub from: String,
    pub to: String,
    pub edge_count: usize,
    pub reverse_count: usize,
}

/// Extract dependency direction conventions from the dependency graph.
pub fn extract_deps(index: &CodebaseIndex) -> DependencyConventions {
    let mut dir_edges: HashMap<(String, String), usize> = HashMap::new();

    // Count edges between top-level directories
    for (from, tos) in &index.graph.edges {
        let from_dir = top_dir(from);
        for edge in tos {
            let to_dir = top_dir(&edge.target);
            if from_dir != to_dir && !from_dir.is_empty() && !to_dir.is_empty() {
                *dir_edges
                    .entry((from_dir.clone(), to_dir.clone()))
                    .or_insert(0) += 1;
            }
        }
    }

    // Find strict layers (edges in one direction only)
    let mut strict_layers = Vec::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

    for ((from, to), count) in &dir_edges {
        let pair = if from < to {
            (from.clone(), to.clone())
        } else {
            (to.clone(), from.clone())
        };
        if seen_pairs.contains(&pair) {
            continue;
        }
        seen_pairs.insert(pair);

        let reverse = dir_edges
            .get(&(to.clone(), from.clone()))
            .copied()
            .unwrap_or(0);
        if reverse == 0 && *count > 0 {
            strict_layers.push(DirectionPair {
                from: from.clone(),
                to: to.clone(),
                edge_count: *count,
                reverse_count: 0,
            });
        }
    }

    // Detect circular dependencies at directory level
    let circular_deps = detect_circular_deps(&dir_edges);

    DependencyConventions {
        strict_layers,
        circular_deps,
        db_isolation: None,
        additional: Vec::new(),
    }
}

fn top_dir(path: &str) -> String {
    // Extract first directory component: "src/api/handler.rs" → "src/api"
    // For convention purposes, use the first two path segments
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[0], parts[1])
    } else {
        parts[0].to_string()
    }
}

fn detect_circular_deps(dir_edges: &HashMap<(String, String), usize>) -> Vec<String> {
    let mut circulars = Vec::new();
    let mut checked: HashSet<(String, String)> = HashSet::new();

    for (from, to) in dir_edges.keys() {
        let pair = if from < to {
            (from.clone(), to.clone())
        } else {
            (to.clone(), from.clone())
        };
        if checked.contains(&pair) {
            continue;
        }
        checked.insert(pair);

        let forward = dir_edges
            .get(&(from.clone(), to.clone()))
            .copied()
            .unwrap_or(0);
        let reverse = dir_edges
            .get(&(to.clone(), from.clone()))
            .copied()
            .unwrap_or(0);

        if forward > 0 && reverse > 0 {
            circulars.push(format!("{from} ↔ {to}"));
        }
    }

    circulars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::graph::DependencyGraph;
    use crate::scanner::ScannedFile;
    use crate::schema::EdgeType;

    fn make_index_with_graph(edges: Vec<(&str, &str)>) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let mut all_files: HashSet<&str> = HashSet::new();
        for (from, to) in &edges {
            all_files.insert(from);
            all_files.insert(to);
        }

        let mut scanned = Vec::new();
        for path in &all_files {
            let fp = dir.path().join(path);
            if let Some(parent) = fp.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&fp, "x").unwrap();
            scanned.push(ScannedFile {
                relative_path: path.to_string(),
                absolute_path: fp,
                language: Some("rust".into()),
                size_bytes: 1,
            });
        }

        let mut index = CodebaseIndex::build(scanned, HashMap::new(), &counter);

        let mut graph = DependencyGraph::new();
        for (from, to) in &edges {
            graph.add_edge(from, to, EdgeType::Import);
        }
        index.graph = graph;
        index
    }

    #[test]
    fn test_strict_layering() {
        let index = make_index_with_graph(vec![
            ("src/api/handler.rs", "src/services/user.rs"),
            ("src/api/auth.rs", "src/services/auth.rs"),
        ]);

        let deps = extract_deps(&index);
        assert!(!deps.strict_layers.is_empty());
        assert!(deps.circular_deps.is_empty());
    }

    #[test]
    fn test_circular_deps_detected() {
        let index = make_index_with_graph(vec![
            ("src/api/handler.rs", "src/services/user.rs"),
            ("src/services/auth.rs", "src/api/common.rs"),
        ]);

        let deps = extract_deps(&index);
        assert!(!deps.circular_deps.is_empty());
    }

    #[test]
    fn test_no_deps() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let deps = extract_deps(&index);
        assert!(deps.strict_layers.is_empty());
        assert!(deps.circular_deps.is_empty());
    }
}

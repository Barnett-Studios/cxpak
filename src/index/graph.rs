use super::IndexedFile;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// Identifies a cross-language boundary type. Used as the payload of
/// [`EdgeType::CrossLanguage`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BridgeType {
    /// HTTP request from one service to another (fetch / axios / reqwest → route handler).
    HttpCall,
    /// FFI binding between languages (e.g. Rust extern "C" to a C function).
    FfiBinding,
    /// gRPC client call to a service defined in a `.proto` file.
    GrpcCall,
    /// GraphQL query/mutation against a typed schema.
    GraphqlCall,
    /// Two files that read/write the same database schema entity from different languages.
    SharedSchema,
    /// `subprocess.run` / `exec.Command` invocation of another binary tracked in the index.
    CommandExec,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeType {
    Import,
    ForeignKey,
    ViewReference,
    TriggerTarget,
    IndexTarget,
    FunctionReference,
    EmbeddedSql,
    OrmModel,
    MigrationSequence,
    /// Cross-language symbol resolution edge (v1.5.0).
    CrossLanguage(BridgeType),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypedEdge {
    pub target: String,
    pub edge_type: EdgeType,
}

#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<String, HashSet<TypedEdge>>,
    pub reverse_edges: HashMap<String, HashSet<TypedEdge>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_edge(&mut self, from: &str, to: &str, edge_type: EdgeType) {
        self.edges
            .entry(from.to_string())
            .or_default()
            .insert(TypedEdge {
                target: to.to_string(),
                edge_type: edge_type.clone(),
            });
        self.reverse_edges
            .entry(to.to_string())
            .or_default()
            .insert(TypedEdge {
                target: from.to_string(),
                edge_type,
            });
    }

    pub fn dependents(&self, path: &str) -> Vec<&TypedEdge> {
        self.reverse_edges
            .get(path)
            .map(|set| set.iter().collect())
            .unwrap_or_default()
    }

    pub fn dependencies(&self, path: &str) -> Option<&HashSet<TypedEdge>> {
        self.edges.get(path)
    }

    /// Remove all outgoing edges from `source` and clean up corresponding reverse edges.
    ///
    /// Used during incremental re-indexing: call this before re-adding the new
    /// edges from a freshly parsed file.
    pub fn remove_edges_for(&mut self, source: &str) {
        if let Some(targets) = self.edges.remove(source) {
            for edge in &targets {
                if let Some(rev) = self.reverse_edges.get_mut(edge.target.as_str()) {
                    rev.retain(|e| e.target != source);
                    if rev.is_empty() {
                        self.reverse_edges.remove(edge.target.as_str());
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
                for edge in deps {
                    if visited.insert(edge.target.clone()) {
                        queue.push_back(edge.target.clone());
                    }
                }
            }

            // Follow incoming edges (files that import `current`)
            if let Some(importers) = self.reverse_edges.get(&current) {
                for edge in importers {
                    if visited.insert(edge.target.clone()) {
                        queue.push_back(edge.target.clone());
                    }
                }
            }
        }

        visited
    }
}

/// Build a `DependencyGraph` from indexed files by resolving import source paths
/// to indexed file paths.  We do a best-effort match: convert the module path
/// (e.g. `crate::scanner`) to a file path (e.g. `src/scanner/mod.rs` or
/// `src/scanner.rs`) and look up whether such a file exists.
///
/// The optional `schema` parameter enables schema-aware edge injection via
/// `build_schema_edges`.  When present, FK, ORM, embedded-SQL,
/// migration-sequence, view-reference and function-reference edges are added
/// in addition to the import edges derived from parse results.
pub fn build_dependency_graph(
    files: &[IndexedFile],
    schema: Option<&crate::schema::SchemaIndex>,
) -> DependencyGraph {
    let all_paths: HashSet<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    let mut graph = DependencyGraph::new();

    for file in files {
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
                    graph.add_edge(&file.relative_path, candidate, EdgeType::Import);
                    break;
                }
            }
        }
    }

    // Inject schema-aware edges when a schema index is available.
    if let Some(schema_index) = schema {
        let schema_edges = crate::schema::link::build_schema_edges(files, schema_index);
        for (from, to, edge_type) in schema_edges {
            graph.add_edge(&from, &to, edge_type);
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
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        assert!(graph.edges.contains_key("a.rs"));
        assert!(graph.edges["a.rs"].iter().any(|e| e.target == "b.rs"));
    }

    #[test]
    fn test_dependents() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("c.rs", "b.rs", EdgeType::Import);
        let deps = graph.dependents("b.rs");
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|e| e.target == "a.rs"));
        assert!(deps.iter().any(|e| e.target == "c.rs"));
    }

    #[test]
    fn test_dependencies() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);
        let deps = graph.dependencies("a.rs").unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|e| e.target == "b.rs"));
        assert!(deps.iter().any(|e| e.target == "c.rs"));
    }

    #[test]
    fn test_dependencies_none() {
        let graph = DependencyGraph::new();
        assert!(graph.dependencies("nonexistent").is_none());
    }

    #[test]
    fn test_reachable_from_single() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&["a.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
        assert!(reachable.contains("c.rs"));
    }

    #[test]
    fn test_reachable_from_reverse() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&["b.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
    }

    #[test]
    fn test_reachable_from_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        graph.add_edge("c.rs", "a.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&["a.rs"]);
        assert_eq!(reachable.len(), 3);
    }

    #[test]
    fn test_reachable_from_disconnected() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("c.rs", "d.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&["a.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
        assert!(!reachable.contains("c.rs"));
        assert!(!reachable.contains("d.rs"));
    }

    #[test]
    fn test_reachable_from_empty_start() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&[]);
        assert!(reachable.is_empty());
    }

    #[test]
    fn test_duplicate_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        assert_eq!(graph.edges["a.rs"].len(), 1);
    }

    #[test]
    fn test_reverse_edges_maintained() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("c.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "d.rs", EdgeType::Import);
        // reverse_edges should exist and be populated
        assert!(graph
            .reverse_edges
            .get("b.rs")
            .unwrap()
            .iter()
            .any(|e| e.target == "a.rs"));
        assert!(graph
            .reverse_edges
            .get("b.rs")
            .unwrap()
            .iter()
            .any(|e| e.target == "c.rs"));
        assert!(graph
            .reverse_edges
            .get("d.rs")
            .unwrap()
            .iter()
            .any(|e| e.target == "a.rs"));
        assert_eq!(graph.reverse_edges.get("b.rs").unwrap().len(), 2);
    }

    #[test]
    fn test_remove_edges_for_file() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);
        graph.add_edge("d.rs", "b.rs", EdgeType::Import);

        graph.remove_edges_for("a.rs");

        // a.rs edges should be gone
        assert!(!graph.edges.contains_key("a.rs"));
        // b.rs should only have d.rs as dependent now
        let b_deps = graph.dependents("b.rs");
        assert_eq!(b_deps.len(), 1);
        assert!(b_deps.iter().any(|e| e.target == "d.rs"));
        // c.rs should have no dependents
        assert!(graph.dependents("c.rs").is_empty());
    }

    #[test]
    fn test_remove_edges_for_nonexistent() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.remove_edges_for("z.rs"); // no-op
        assert_eq!(graph.edges["a.rs"].len(), 1);
    }

    #[test]
    fn test_remove_and_readd_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);

        // Simulate re-parse: remove old, add new
        graph.remove_edges_for("a.rs");
        graph.add_edge("a.rs", "d.rs", EdgeType::Import);

        assert_eq!(graph.edges["a.rs"].len(), 1);
        assert!(graph.edges["a.rs"].iter().any(|e| e.target == "d.rs"));
        assert!(graph.dependents("b.rs").is_empty());
        assert!(graph.dependents("c.rs").is_empty());
        let deps = graph.dependents("d.rs");
        assert!(deps.iter().any(|e| e.target == "a.rs") && deps.len() == 1);
    }

    #[test]
    fn test_dependents_large_graph() {
        let mut graph = DependencyGraph::new();
        for i in 0..100 {
            graph.add_edge(&format!("file_{i}.rs"), "common.rs", EdgeType::Import);
        }
        let deps = graph.dependents("common.rs");
        assert_eq!(deps.len(), 100);
    }

    #[test]
    fn test_add_typed_edge() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("models/user.rs", "schema.sql", EdgeType::ForeignKey);
        let deps = graph.dependencies("models/user.rs").unwrap();
        assert!(deps
            .iter()
            .any(|e| e.target == "schema.sql" && e.edge_type == EdgeType::ForeignKey));
    }

    #[test]
    fn test_multiple_edge_types_same_target() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "b.rs", EdgeType::ForeignKey);
        // Two different TypedEdges (same target, different edge_type) → both stored
        assert_eq!(graph.edges["a.rs"].len(), 2);
    }

    #[test]
    fn test_dependents_returns_typed_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        let deps = graph.dependents("b.rs");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].target, "a.rs");
        assert_eq!(deps[0].edge_type, EdgeType::Import);
    }

    #[test]
    fn test_edge_type_local_to_module() {
        // EdgeType lives in src/index/graph.rs (this module) — not in crate::schema.
        // This test asserts the type is reachable via the local module path.
        let _import = EdgeType::Import;
        let _fk = EdgeType::ForeignKey;
    }

    #[test]
    fn test_cross_language_edge_hash() {
        let mut set = HashSet::new();
        set.insert(TypedEdge {
            target: "b.rs".into(),
            edge_type: EdgeType::CrossLanguage(BridgeType::HttpCall),
        });
        set.insert(TypedEdge {
            target: "b.rs".into(),
            edge_type: EdgeType::CrossLanguage(BridgeType::FfiBinding),
        });
        // Same target, different bridge types — both unique edges.
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_edge_type_cross_language_serialization() {
        let variants = [
            BridgeType::HttpCall,
            BridgeType::FfiBinding,
            BridgeType::GrpcCall,
            BridgeType::GraphqlCall,
            BridgeType::SharedSchema,
            BridgeType::CommandExec,
        ];
        for bt in variants {
            let edge = TypedEdge {
                target: "x.py".into(),
                edge_type: EdgeType::CrossLanguage(bt.clone()),
            };
            let json = serde_json::to_string(&edge).unwrap();
            let back: TypedEdge = serde_json::from_str(&json).unwrap();
            assert_eq!(back.edge_type, edge.edge_type);
        }
    }

    #[test]
    fn test_add_cross_language_edge() {
        let mut graph = DependencyGraph::new();
        graph.add_edge(
            "a.ts",
            "b.rs",
            EdgeType::CrossLanguage(BridgeType::FfiBinding),
        );
        let deps = graph.dependencies("a.ts").unwrap();
        assert!(deps.iter().any(|e| e.target == "b.rs"
            && e.edge_type == EdgeType::CrossLanguage(BridgeType::FfiBinding)));
    }
}

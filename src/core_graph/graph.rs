//! Dependency-graph primitives: the graph data model and its *pure* methods.
//!
//! These types are the shared foundation that `index`, `schema`,
//! `intelligence`, and `conventions` all depend on one-directionally (ADR-0007
//! module boundaries, cxpak 3.0.0 Phase 0). The graph *builder* functions
//! (`build_dependency_graph`, `resolve_*_import`, `edges_for_file`) stay in
//! `src/index/graph.rs` — they orchestrate over scanner/parser output and the
//! schema layer, so they belong above this boundary, not in it.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

// ---------------------------------------------------------------------------
// EdgeConfidence (Task 0.4 — ADR-0176 descriptive-honesty)
// ---------------------------------------------------------------------------

/// Whether an edge was structurally extracted from explicit source or
/// heuristically inferred from pattern matching.
///
/// "Every edge proven, never inferred" — and when it IS inferred, say so.
/// Added as the LAST field on [`TypedEdge`] so the derived `Ord` (which
/// compares `target` then `edge_type` first) remains stable: `BTreeSet`
/// iteration order and `edge_count` are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EdgeConfidence {
    /// Extracted from explicit source: `use` statements, FK/ORM/migration/view
    /// metadata — structurally unambiguous.
    Extracted,
    /// Inferred by heuristic pattern matching: embedded-SQL regex, cross-language
    /// bridge detection.  Correct most of the time but not provably exact.
    Inferred,
}

fn default_confidence_extracted() -> EdgeConfidence {
    EdgeConfidence::Extracted
}

/// Identifies a cross-language boundary type. Used as the payload of
/// [`EdgeType::CrossLanguage`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    /// Reference to a specific column node (cxpak 3.0.0 Task A2 — column-level
    /// lineage). Connects a source file (query / ORM model) to a `col:table.col`
    /// node, and that column node to its table-definition file. Confidence is
    /// per-edge: structurally [`Extracted`][EdgeConfidence::Extracted] for ORM
    /// fields and the column→table anchor, [`Inferred`][EdgeConfidence::Inferred]
    /// for heuristic embedded-SQL column refs and `SELECT *` fan-out — so the
    /// default below is the conservative `Inferred`, overridden per-edge via
    /// [`DependencyGraph::add_edge_with_confidence`] where the ref is structural.
    ColumnReference,
}

impl EdgeType {
    /// Canonical confidence for this edge type.  Single source of truth so
    /// every call site (including future ones) derives the correct value
    /// automatically.
    ///
    /// Inferred: `EmbeddedSql` (regex-based table-name matching) and
    ///           `CrossLanguage(_)` (heuristic bridge detection).
    /// Extracted: all other types (explicit `use` / FK / ORM / migration /
    ///            view / function metadata — structurally unambiguous).
    pub fn default_confidence(&self) -> EdgeConfidence {
        match self {
            // `ColumnReference` defaults to Inferred because the majority of
            // column edges originate from heuristic embedded-SQL parsing; the
            // structurally-extracted cases (ORM fields, the column→table anchor)
            // are stamped Extracted explicitly via `add_edge_with_confidence`.
            EdgeType::EmbeddedSql | EdgeType::CrossLanguage(_) | EdgeType::ColumnReference => {
                EdgeConfidence::Inferred
            }
            _ => EdgeConfidence::Extracted,
        }
    }

    /// Stable lowercase label used in human-readable edge renderings — the
    /// `(via: <label>)` markers in the overview/trace dependency sections, the
    /// auto_context dependency annotation, and the LSP inferred-edge
    /// diagnostic.  Single source of truth so every surface agrees on the
    /// spelling (previously inlined as four identical `match` arms that could
    /// drift independently).
    pub fn label(&self) -> String {
        match self {
            EdgeType::Import => "import".to_string(),
            EdgeType::ForeignKey => "foreign_key".to_string(),
            EdgeType::ViewReference => "view_reference".to_string(),
            EdgeType::TriggerTarget => "trigger_target".to_string(),
            EdgeType::IndexTarget => "index_target".to_string(),
            EdgeType::FunctionReference => "function_reference".to_string(),
            EdgeType::EmbeddedSql => "embedded_sql".to_string(),
            EdgeType::OrmModel => "orm_model".to_string(),
            EdgeType::MigrationSequence => "migration_sequence".to_string(),
            EdgeType::CrossLanguage(bt) => format!("cross_language:{bt:?}"),
            EdgeType::ColumnReference => "column_reference".to_string(),
        }
    }
}

impl EdgeConfidence {
    /// True when this edge was heuristically inferred rather than structurally
    /// extracted.  Drives the visible `inferred` tag in every edge rendering.
    pub fn is_inferred(&self) -> bool {
        matches!(self, EdgeConfidence::Inferred)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TypedEdge {
    pub target: String,
    pub edge_type: EdgeType,
    /// Whether this edge was structurally [`Extracted`][EdgeConfidence::Extracted]
    /// or heuristically [`Inferred`][EdgeConfidence::Inferred].
    ///
    /// Placed last so derived `Ord` keeps comparing `target`→`edge_type` first,
    /// leaving `BTreeSet<TypedEdge>` order and `edge_count` unchanged.
    ///
    /// Defaults to `Extracted` on deserialization so stale cache JSON without
    /// this field (written before CACHE_VERSION 5) is accepted without error.
    #[serde(default = "default_confidence_extracted")]
    pub confidence: EdgeConfidence,
}

/// Backed by `BTreeMap`/`BTreeSet` rather than the hash equivalents so iteration
/// order is fully deterministic.  PageRank, coupling, and other downstream
/// reducers do `iter().filter().map().sum::<f64>()` over these collections;
/// f64 addition is not associative, and HashMap/HashSet iteration order is
/// randomised by the std hasher, so the same input graph would produce
/// 1-ULP-different results across runs.  That divergence then propagated
/// into `/v1/health`, MCP `cxpak_health`, the SPA dashboard, and api_surface
/// pagerank fields — breaking the deterministic-tool contract for
/// cross-process reproducibility (caught during v2.1.0 manual QA).
///
/// BTreeMap insert is O(log n) vs HashMap's O(1) amortised, but n is bounded
/// by the file count (low thousands at most for the sizes cxpak indexes),
/// log₂(n) ≈ 11–13, and the graph is built once per index build.  No
/// measurable wall-clock impact.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub edges: BTreeMap<String, BTreeSet<TypedEdge>>,
    pub reverse_edges: BTreeMap<String, BTreeSet<TypedEdge>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total directed-edge count: sum of every adjacency-list length.
    ///
    /// Single source of truth so `commands::visual::make_metadata`,
    /// `tests/cross_channel_consistency.rs`, and any future renderer
    /// agree on the value.  Spec § Contract 8 requires both sides of
    /// the SPA-vs-MCP edge_count comparison to derive from the same
    /// helper; without this method they were inlined as identical
    /// lambdas in two files and could drift independently.
    pub fn edge_count(&self) -> usize {
        self.edges.values().map(|v| v.len()).sum()
    }

    pub fn add_edge(&mut self, from: &str, to: &str, edge_type: EdgeType) {
        let confidence = edge_type.default_confidence();
        self.add_edge_with_confidence(from, to, edge_type, confidence);
    }

    /// Add a typed edge with an explicit [`EdgeConfidence`], overriding the
    /// edge type's `default_confidence()`.
    ///
    /// Used by the column-lineage path (Task A2): a `ColumnReference` edge
    /// defaults to `Inferred`, but ORM-field and column→table-anchor edges are
    /// structurally proven and must be stamped `Extracted`. All other call
    /// sites go through [`add_edge`], which derives the default — so existing
    /// edge confidence values are unchanged.
    pub fn add_edge_with_confidence(
        &mut self,
        from: &str,
        to: &str,
        edge_type: EdgeType,
        confidence: EdgeConfidence,
    ) {
        self.edges
            .entry(from.to_string())
            .or_default()
            .insert(TypedEdge {
                target: to.to_string(),
                edge_type: edge_type.clone(),
                confidence,
            });
        self.reverse_edges
            .entry(to.to_string())
            .or_default()
            .insert(TypedEdge {
                target: from.to_string(),
                edge_type,
                confidence,
            });
    }

    pub fn dependents(&self, path: &str) -> Vec<&TypedEdge> {
        self.reverse_edges
            .get(path)
            .map(|set| set.iter().collect())
            .unwrap_or_default()
    }

    pub fn dependencies(&self, path: &str) -> Option<&BTreeSet<TypedEdge>> {
        self.edges.get(path)
    }

    /// True when `path` participates in at least one edge (as a source or a
    /// target). A file with no edges in either direction is not a node and
    /// contributes nothing to the graph. Used by the incremental delta to
    /// detect file-set changes (a changed path that is not yet a node is
    /// treated as an addition → conservative full rebuild).
    pub fn contains_node(&self, path: &str) -> bool {
        self.edges.contains_key(path) || self.reverse_edges.contains_key(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_type_equality() {
        assert_eq!(EdgeType::Import, EdgeType::Import);
        assert_ne!(EdgeType::Import, EdgeType::ForeignKey);
    }

    #[test]
    fn test_typed_edge_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TypedEdge {
            target: "a.rs".into(),
            edge_type: EdgeType::Import,
            confidence: EdgeConfidence::Extracted,
        });
        set.insert(TypedEdge {
            target: "a.rs".into(),
            edge_type: EdgeType::ForeignKey,
            confidence: EdgeConfidence::Extracted,
        });
        assert_eq!(
            set.len(),
            2,
            "same target, different types = different edges"
        );
    }

    #[test]
    fn test_edge_type_label_and_is_inferred() {
        assert_eq!(EdgeType::Import.label(), "import");
        assert_eq!(EdgeType::EmbeddedSql.label(), "embedded_sql");
        assert_eq!(EdgeType::ColumnReference.label(), "column_reference");
        assert_eq!(
            EdgeType::CrossLanguage(BridgeType::HttpCall).label(),
            "cross_language:HttpCall"
        );
        assert!(EdgeConfidence::Inferred.is_inferred());
        assert!(!EdgeConfidence::Extracted.is_inferred());
    }

    #[test]
    fn test_dependency_graph_add_and_query() {
        let mut g = DependencyGraph::new();
        g.add_edge("a.rs", "b.rs", EdgeType::Import);
        assert_eq!(g.edge_count(), 1);
        assert!(g.contains_node("a.rs"));
        assert!(g.contains_node("b.rs"));
        assert_eq!(g.dependents("b.rs").len(), 1);
        let reachable = g.reachable_from(&["a.rs"]);
        assert!(reachable.contains("b.rs"));
    }
}

//! Sugiyama layout engine for architecture visualizations.
//!
//! The layout module implements a multi-level graph layout algorithm (Sugiyama method)
//! for drawing dependency graphs, architecture diagrams, and data flow visualizations.
//!
//! Implementation includes:
//! - Layer assignment via topological sort (Task 3)
//! - Crossing minimization using heuristic ordering (Task 4)
//! - Coordinate assignment for node positioning (Task 4)
//! - Layout builders for module, file, and symbol graphs (Task 5)

use std::collections::HashMap;

/// A node in the layout graph — maps to a file, module, or symbol
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayoutNode {
    pub id: String,
    pub label: String,
    pub layer: usize,
    pub position: Point,
    pub width: f64,
    pub height: f64,
    pub node_type: NodeType,
    pub metadata: NodeMetadata,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum NodeType {
    Module,
    File,
    Symbol,
    /// Virtual node representing a condensed SCC
    Cluster {
        member_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NodeMetadata {
    pub pagerank: f64,
    pub risk_score: f64,
    pub token_count: usize,
    pub health_score: Option<f64>,
    pub is_god_file: bool,
    pub has_dead_code: bool,
    pub is_circular: bool,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayoutEdge {
    pub source: String,
    pub target: String,
    pub edge_type: EdgeVisualType,
    pub weight: f64,
    /// True when this edge participates in a cycle
    pub is_cycle: bool,
    /// Waypoints for edges that route through dummy nodes
    pub waypoints: Vec<Point>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum EdgeVisualType {
    Import,
    Call,
    Schema,
    CrossLanguage,
    CoChange,
    DataFlow,
}

/// Fully computed layout — positions ready for D3 rendering
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ComputedLayout {
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
    pub width: f64,
    pub height: f64,
    pub layers: Vec<Vec<String>>, // node ids per layer
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("graph is fully cyclic and cannot be layered")]
    #[allow(dead_code)]
    Cyclic,
    #[error("node not found: {0}")]
    NodeNotFound(String),
    #[error("empty graph")]
    Empty,
}

#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub layer_sep: f64,             // vertical gap between layers (default: 120.0)
    pub node_sep: f64,              // horizontal gap between nodes in same layer (default: 60.0)
    pub node_width: f64,            // default node width (default: 160.0)
    pub node_height: f64,           // default node height (default: 48.0)
    pub max_nodes_per_layer: usize, // default: 9 (7±2)
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            layer_sep: 120.0,
            node_sep: 60.0,
            node_width: 160.0,
            node_height: 48.0,
            max_nodes_per_layer: 9,
        }
    }
}

/// Assigns each node a layer index via longest-path layering.
/// Condenses SCCs to virtual nodes first so the input DAG is always acyclic.
/// Every node in an SCC gets the layer of its condensed component.
#[allow(dead_code)] // used by Task 4 compute_layout
pub(crate) fn layer_assign(
    node_ids: &[String],
    edges: &[(String, String)],
) -> Result<HashMap<String, usize>, LayoutError> {
    if node_ids.is_empty() {
        return Err(LayoutError::Empty);
    }

    // Step 1: Build petgraph DiGraph from node_ids and edges.
    // Map node id string → petgraph NodeIndex.
    let mut graph = petgraph::graph::DiGraph::<String, ()>::new();
    let mut node_index_map: HashMap<String, petgraph::graph::NodeIndex> = HashMap::new();

    for id in node_ids {
        let idx = graph.add_node(id.clone());
        node_index_map.insert(id.clone(), idx);
    }

    for (src, dst) in edges {
        let src_idx = node_index_map
            .get(src)
            .ok_or_else(|| LayoutError::NodeNotFound(src.clone()))?;
        let dst_idx = node_index_map
            .get(dst)
            .ok_or_else(|| LayoutError::NodeNotFound(dst.clone()))?;
        graph.add_edge(*src_idx, *dst_idx, ());
    }

    // Step 2: Compute SCCs via Kosaraju's algorithm.
    // kosaraju_scc returns SCCs in reverse topological order of the condensation DAG
    // (i.e., sinks first). Each SCC is a Vec of NodeIndex.
    let sccs = petgraph::algo::kosaraju_scc(&graph);

    // Step 3: Map each original node to its SCC index.
    // sccs[i] is the i-th SCC (in reverse topo order of condensation).
    let mut node_to_scc: HashMap<petgraph::graph::NodeIndex, usize> = HashMap::new();
    for (scc_idx, scc) in sccs.iter().enumerate() {
        for &node_idx in scc {
            node_to_scc.insert(node_idx, scc_idx);
        }
    }

    let n_sccs = sccs.len();

    // Step 4: Build the condensation DAG.
    // One node per SCC, an edge A→B if any original edge goes from a node in SCC A
    // to a node in SCC B (and A ≠ B).
    let mut condensation: petgraph::graph::DiGraph<usize, ()> = petgraph::graph::DiGraph::new();
    // Add one condensation node per SCC.
    let cond_nodes: Vec<petgraph::graph::NodeIndex> =
        (0..n_sccs).map(|i| condensation.add_node(i)).collect();

    // Use a set to avoid duplicate edges in the condensation.
    let mut cond_edges_seen: std::collections::HashSet<(usize, usize)> =
        std::collections::HashSet::new();

    for edge in graph.edge_indices() {
        let (src_idx, dst_idx) = graph.edge_endpoints(edge).unwrap();
        let src_scc = node_to_scc[&src_idx];
        let dst_scc = node_to_scc[&dst_idx];
        if src_scc != dst_scc && cond_edges_seen.insert((src_scc, dst_scc)) {
            condensation.add_edge(cond_nodes[src_scc], cond_nodes[dst_scc], ());
        }
    }

    // Step 5: Topological sort the condensation DAG.
    // kosaraju_scc already returned SCCs in reverse topo order of the condensation
    // (sinks-first). So sccs[0] is a sink SCC, sccs[n-1] is a source SCC.
    // For longest-path layering (sources at layer 0), we process sources first.
    // We use petgraph::algo::toposort on the condensation graph directly.
    let topo_order = petgraph::algo::toposort(&condensation, None)
        .expect("condensation is always a DAG — toposort cannot fail");

    // Step 6: Longest-path layering on the condensation DAG.
    // Layer of a source = 0. Layer of any node = max(layer of predecessors) + 1.
    let mut cond_layer: Vec<usize> = vec![0; n_sccs];

    for cond_node in &topo_order {
        let scc_idx = condensation[*cond_node];
        // Examine predecessors of this condensation node.
        let pred_layer = condensation
            .neighbors_directed(*cond_node, petgraph::Direction::Incoming)
            .map(|pred| cond_layer[condensation[pred]])
            .max()
            .map(|max_pred| max_pred + 1)
            .unwrap_or(0);
        cond_layer[scc_idx] = pred_layer;
    }

    // Step 7: Expand — every original node inherits the layer of its SCC.
    let mut result: HashMap<String, usize> = HashMap::new();
    for id in node_ids {
        let idx = node_index_map[id];
        let scc_idx = node_to_scc[&idx];
        result.insert(id.clone(), cond_layer[scc_idx]);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_assign_linear_chain() {
        // a -> b -> c => layers [0, 1, 2]
        let layers = layer_assign(
            &["a".into(), "b".into(), "c".into()],
            &[("a".into(), "b".into()), ("b".into(), "c".into())],
        )
        .unwrap();
        assert_eq!(layers["a"], 0);
        assert_eq!(layers["b"], 1);
        assert_eq!(layers["c"], 2);
    }

    #[test]
    fn test_layer_assign_diamond() {
        // a -> b, a -> c, b -> d, c -> d
        // d must be in layer >= 2
        let layers = layer_assign(
            &["a".into(), "b".into(), "c".into(), "d".into()],
            &[
                ("a".into(), "b".into()),
                ("a".into(), "c".into()),
                ("b".into(), "d".into()),
                ("c".into(), "d".into()),
            ],
        )
        .unwrap();
        assert!(layers["d"] >= 2);
        assert!(layers["b"] > layers["a"]);
        assert!(layers["c"] > layers["a"]);
    }

    #[test]
    fn test_layer_assign_handles_cycle_via_scc_condensation() {
        // a -> b -> a (cycle) — condensed to virtual node, does not error
        let result = layer_assign(
            &["a".into(), "b".into()],
            &[("a".into(), "b".into()), ("b".into(), "a".into())],
        );
        assert!(result.is_ok());
        let layers = result.unwrap();
        // both a and b should end up in the same layer (same SCC)
        assert_eq!(layers["a"], layers["b"]);
    }

    #[test]
    fn test_layer_assign_five_node_dag_no_forward_violations() {
        // 1 -> 2, 1 -> 3, 2 -> 4, 3 -> 4, 4 -> 5
        let nodes: Vec<String> = (1..=5).map(|i| i.to_string()).collect();
        let edges: Vec<(String, String)> = vec![
            ("1".into(), "2".into()),
            ("1".into(), "3".into()),
            ("2".into(), "4".into()),
            ("3".into(), "4".into()),
            ("4".into(), "5".into()),
        ];
        let layers = layer_assign(&nodes, &edges).unwrap();
        for (src, dst) in &edges {
            assert!(
                layers[src] < layers[dst],
                "edge {src}->{dst} violates layering: layer[{src}]={}, layer[{dst}]={}",
                layers[src],
                layers[dst]
            );
        }
    }

    #[test]
    fn test_layer_assign_empty_graph_errors() {
        let result = layer_assign(&[], &[]);
        assert!(matches!(result, Err(LayoutError::Empty)));
    }

    #[test]
    fn test_layer_assign_unknown_node_in_edge_errors() {
        let result = layer_assign(&["a".into()], &[("a".into(), "ghost".into())]);
        assert!(matches!(result, Err(LayoutError::NodeNotFound(_))));
    }
}

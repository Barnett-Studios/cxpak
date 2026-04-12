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

/// Inserts virtual (dummy) nodes on edges spanning multiple layers.
/// Returns augmented node list, augmented edge list, and dummy node ids.
pub(crate) fn insert_dummy_nodes(
    nodes: &[LayoutNode],
    edges: &[LayoutEdge],
    layers: &HashMap<String, usize>,
) -> (Vec<LayoutNode>, Vec<LayoutEdge>, Vec<String>) {
    let mut aug_nodes: Vec<LayoutNode> = nodes.to_vec();
    let mut aug_edges: Vec<LayoutEdge> = Vec::new();
    let mut dummy_ids: Vec<String> = Vec::new();

    for edge in edges {
        let src_layer = match layers.get(&edge.source) {
            Some(&l) => l,
            None => {
                aug_edges.push(edge.clone());
                continue;
            }
        };
        let dst_layer = match layers.get(&edge.target) {
            Some(&l) => l,
            None => {
                aug_edges.push(edge.clone());
                continue;
            }
        };

        let span = dst_layer.saturating_sub(src_layer);

        if span <= 1 {
            aug_edges.push(edge.clone());
            continue;
        }

        // Insert dummy nodes at each intermediate layer.
        let mut prev_id = edge.source.clone();
        for intermediate_layer in (src_layer + 1)..dst_layer {
            let dummy_id = format!(
                "__dummy_{}_{}_{}",
                edge.source, edge.target, intermediate_layer
            );
            let dummy_node = LayoutNode {
                id: dummy_id.clone(),
                label: String::new(),
                layer: intermediate_layer,
                position: Point::default(),
                width: 0.0,
                height: 0.0,
                node_type: NodeType::Symbol,
                metadata: NodeMetadata::default(),
            };
            aug_nodes.push(dummy_node);
            dummy_ids.push(dummy_id.clone());

            aug_edges.push(LayoutEdge {
                source: prev_id.clone(),
                target: dummy_id.clone(),
                edge_type: edge.edge_type.clone(),
                weight: edge.weight,
                is_cycle: edge.is_cycle,
                waypoints: vec![],
            });
            prev_id = dummy_id;
        }

        // Final segment: last dummy → target.
        aug_edges.push(LayoutEdge {
            source: prev_id,
            target: edge.target.clone(),
            edge_type: edge.edge_type.clone(),
            weight: edge.weight,
            is_cycle: edge.is_cycle,
            waypoints: vec![],
        });
    }

    (aug_nodes, aug_edges, dummy_ids)
}

/// One-sided barycenter crossing minimization.
/// Mutates layer ordering in place; 4 passes alternating top-down/bottom-up.
pub(crate) fn barycenter_sort(
    layer_order: &mut [Vec<String>],
    adjacency: &HashMap<String, Vec<String>>,
    reverse_adjacency: &HashMap<String, Vec<String>>,
) {
    let n_layers = layer_order.len();
    if n_layers < 2 {
        return;
    }

    for pass in 0..4 {
        let top_down = pass % 2 == 0;

        let indices: Vec<usize> = if top_down {
            (1..n_layers).collect()
        } else {
            (0..(n_layers - 1)).rev().collect()
        };

        for layer_idx in indices {
            // Build a position map for the fixed (adjacent) layer.
            let fixed_layer_idx = if top_down {
                layer_idx - 1
            } else {
                layer_idx + 1
            };
            let fixed_positions: HashMap<&str, f64> = layer_order[fixed_layer_idx]
                .iter()
                .enumerate()
                .map(|(pos, id)| (id.as_str(), pos as f64))
                .collect();

            // Compute barycenter for each node in the current layer.
            let current_positions: HashMap<&str, f64> = layer_order[layer_idx]
                .iter()
                .enumerate()
                .map(|(pos, id)| (id.as_str(), pos as f64))
                .collect();

            let neighbor_map = if top_down {
                reverse_adjacency
            } else {
                adjacency
            };

            let mut barycenters: Vec<(f64, String)> = layer_order[layer_idx]
                .iter()
                .map(|id| {
                    let neighbors = neighbor_map
                        .get(id.as_str())
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    let fixed_neighbor_positions: Vec<f64> = neighbors
                        .iter()
                        .filter_map(|nb| fixed_positions.get(nb.as_str()).copied())
                        .collect();
                    let barycenter = if fixed_neighbor_positions.is_empty() {
                        // No neighbors in the fixed layer — keep current position.
                        *current_positions.get(id.as_str()).unwrap_or(&0.0)
                    } else {
                        fixed_neighbor_positions.iter().sum::<f64>()
                            / fixed_neighbor_positions.len() as f64
                    };
                    (barycenter, id.clone())
                })
                .collect();

            barycenters.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            layer_order[layer_idx] = barycenters.into_iter().map(|(_, id)| id).collect();
        }
    }
}

/// Brandes-Kopf simplified coordinate assignment.
/// Returns x,y for each node id. y is determined by layer × layer_sep.
/// Layers are centered horizontally relative to each other; the global x minimum is
/// shifted to 0 so all positions are non-negative.
pub(crate) fn assign_coordinates(
    layer_order: &[Vec<String>],
    config: &LayoutConfig,
) -> HashMap<String, Point> {
    let mut coords: HashMap<String, Point> = HashMap::new();

    for (layer_idx, layer) in layer_order.iter().enumerate() {
        let y = layer_idx as f64 * config.layer_sep;
        let n = layer.len() as f64;
        // Total width of the layer: n nodes each of width node_width, separated by node_sep.
        let total_width = n * config.node_width + (n - 1.0).max(0.0) * config.node_sep;
        // Center horizontally: start at half the total width offset from 0.
        let x_start = -total_width / 2.0 + config.node_width / 2.0;

        for (pos, id) in layer.iter().enumerate() {
            let x = x_start + pos as f64 * (config.node_width + config.node_sep);
            coords.insert(id.clone(), Point { x, y });
        }
    }

    // Shift all x-coordinates so the global minimum is 0.
    if let Some(min_x) = coords.values().map(|p| p.x).reduce(f64::min) {
        if min_x < 0.0 {
            for point in coords.values_mut() {
                point.x -= min_x;
            }
        }
    }

    coords
}

/// Entry point — computes full Sugiyama layout.
pub fn compute_layout(
    nodes: Vec<LayoutNode>,
    edges: Vec<LayoutEdge>,
    config: &LayoutConfig,
) -> Result<ComputedLayout, LayoutError> {
    if nodes.is_empty() {
        return Err(LayoutError::Empty);
    }

    // 1. Collect node ids and edge pairs.
    let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let edge_pairs: Vec<(String, String)> = edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    // 2. Layer assignment.
    let layers = layer_assign(&node_ids, &edge_pairs)?;

    // 3. Insert dummy nodes for multi-layer spanning edges.
    let (aug_nodes, aug_edges, dummy_ids) = insert_dummy_nodes(&nodes, &edges, &layers);

    // Update the layers map to include dummy nodes (they carry their layer from insert_dummy_nodes).
    let mut all_layers = layers;
    for node in &aug_nodes {
        all_layers.entry(node.id.clone()).or_insert(node.layer);
    }

    // 4. Build layer_order: group node ids by layer.
    let max_layer = all_layers.values().copied().max().unwrap_or(0);
    let mut layer_order: Vec<Vec<String>> = vec![Vec::new(); max_layer + 1];
    for node in &aug_nodes {
        let l = all_layers[&node.id];
        layer_order[l].push(node.id.clone());
    }

    // 5. Build adjacency / reverse_adjacency from augmented edges.
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    let mut reverse_adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for edge in &aug_edges {
        adjacency
            .entry(edge.source.clone())
            .or_default()
            .push(edge.target.clone());
        reverse_adjacency
            .entry(edge.target.clone())
            .or_default()
            .push(edge.source.clone());
    }

    // 6. Crossing minimization.
    barycenter_sort(&mut layer_order, &adjacency, &reverse_adjacency);

    // 7. Coordinate assignment.
    let coords = assign_coordinates(&layer_order, config);

    // 8. Build the dummy id set for fast lookup.
    let dummy_set: std::collections::HashSet<&str> = dummy_ids.iter().map(|s| s.as_str()).collect();

    // 9. Collect dummy positions as waypoints on original edges, then build final edges.
    // Map each dummy node id → its position.
    let dummy_positions: HashMap<&str, Point> = dummy_ids
        .iter()
        .filter_map(|id| coords.get(id.as_str()).map(|&p| (id.as_str(), p)))
        .collect();

    // Reconstruct original edges with waypoints threaded through dummy chain.
    // For each original edge, find the dummy chain by traversing aug_edges.
    let mut final_edges: Vec<LayoutEdge> = Vec::new();
    for orig_edge in &edges {
        let src_layer = all_layers.get(&orig_edge.source).copied().unwrap_or(0);
        let dst_layer = all_layers.get(&orig_edge.target).copied().unwrap_or(0);
        let span = dst_layer.saturating_sub(src_layer);

        let mut waypoints: Vec<Point> = Vec::new();
        if span > 1 {
            for intermediate_layer in (src_layer + 1)..dst_layer {
                let dummy_id = format!(
                    "__dummy_{}_{}_{}",
                    orig_edge.source, orig_edge.target, intermediate_layer
                );
                if let Some(&pt) = dummy_positions.get(dummy_id.as_str()) {
                    waypoints.push(pt);
                }
            }
        }

        final_edges.push(LayoutEdge {
            source: orig_edge.source.clone(),
            target: orig_edge.target.clone(),
            edge_type: orig_edge.edge_type.clone(),
            weight: orig_edge.weight,
            is_cycle: orig_edge.is_cycle,
            waypoints,
        });
    }

    // 10. Build final node list — remove dummies, assign positions and layers.
    let mut final_nodes: Vec<LayoutNode> = aug_nodes
        .into_iter()
        .filter(|n| !dummy_set.contains(n.id.as_str()))
        .map(|mut n| {
            if let Some(&pt) = coords.get(&n.id) {
                n.position = pt;
            }
            if let Some(&l) = all_layers.get(&n.id) {
                n.layer = l;
            }
            n
        })
        .collect();

    // Also carry over any position/layer updates for nodes that were in the original list.
    // (aug_nodes started from nodes, so this is already handled above.)
    // Ensure ordering matches original node order for stability.
    let orig_order: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    final_nodes.sort_by_key(|n| orig_order.get(n.id.as_str()).copied().unwrap_or(usize::MAX));

    // 11. Compute overall width and height from node positions.
    let width = final_nodes
        .iter()
        .map(|n| n.position.x + n.width / 2.0)
        .fold(f64::NEG_INFINITY, f64::max)
        - final_nodes
            .iter()
            .map(|n| n.position.x - n.width / 2.0)
            .fold(f64::INFINITY, f64::min);

    let height = final_nodes
        .iter()
        .map(|n| n.position.y + n.height / 2.0)
        .fold(f64::NEG_INFINITY, f64::max)
        - final_nodes
            .iter()
            .map(|n| n.position.y - n.height / 2.0)
            .fold(f64::INFINITY, f64::min);

    let final_layer_order: Vec<Vec<String>> = layer_order
        .into_iter()
        .map(|layer| {
            layer
                .into_iter()
                .filter(|id| !dummy_set.contains(id.as_str()))
                .collect()
        })
        .collect();

    Ok(ComputedLayout {
        nodes: final_nodes,
        edges: final_edges,
        width: width.max(0.0),
        height: height.max(0.0),
        layers: final_layer_order,
    })
}

// ---------------------------------------------------------------------------
// Task 5: Layout builders
// ---------------------------------------------------------------------------

use crate::index::CodebaseIndex;

/// Derive a two-segment module prefix from a file path.
///
/// Delegates to `crate::intelligence::health::module_prefix` (depth 2).
fn file_module_prefix(path: &str) -> String {
    crate::intelligence::health::module_prefix(path, 2)
}

/// If a layer exceeds `max_per_layer`, group the tail nodes into a single
/// `Cluster` node and update `layer_order` accordingly.
///
/// The first `max_per_layer - 1` nodes in any over-full layer are kept as-is.
/// The remaining nodes are replaced by a single `Cluster` node whose
/// `member_ids` lists the displaced node ids.
///
/// `nodes` is consumed and returned with the cluster node appended (excess
/// nodes removed).
fn enforce_cognitive_limit(
    mut nodes: Vec<LayoutNode>,
    layer_order: &mut [Vec<String>],
    max_per_layer: usize,
) -> Vec<LayoutNode> {
    if max_per_layer == 0 {
        return nodes;
    }

    // Build a quick lookup: node id → index in `nodes`
    let mut id_to_idx: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();

    let mut ids_to_remove: Vec<String> = Vec::new();

    for layer in layer_order.iter_mut() {
        if layer.len() <= max_per_layer {
            continue;
        }

        // Keep the first (max_per_layer - 1) nodes; cluster the rest.
        let keep = max_per_layer - 1;
        let excess: Vec<String> = layer.drain(keep..).collect();

        // Build a cluster node from the first excess node's properties (position etc
        // will be overwritten by compute_layout anyway).
        let cluster_id = format!("__cluster_{}", layer.len());
        let cluster_node = LayoutNode {
            id: cluster_id.clone(),
            label: format!("{} more…", excess.len()),
            layer: 0, // will be set by compute_layout
            position: Point::default(),
            width: 160.0,
            height: 48.0,
            node_type: NodeType::Cluster {
                member_ids: excess.clone(),
            },
            metadata: NodeMetadata::default(),
        };

        // Mark excess nodes for removal.
        ids_to_remove.extend(excess);

        // Add cluster id to this layer.
        layer.push(cluster_id.clone());

        // Add cluster node.
        nodes.push(cluster_node);
        id_to_idx.insert(cluster_id, nodes.len() - 1);
    }

    // Remove excess nodes (keep cluster nodes that replaced them).
    let remove_set: std::collections::HashSet<&str> =
        ids_to_remove.iter().map(|s| s.as_str()).collect();
    nodes.retain(|n| !remove_set.contains(n.id.as_str()));

    nodes
}

/// Level 1 layout: one node per module, edges for cross-module imports.
///
/// Calls [`crate::intelligence::architecture::build_architecture_map`] with
/// `module_depth = 2` to derive the set of modules.
pub fn build_module_layout(
    index: &CodebaseIndex,
    config: &LayoutConfig,
) -> Result<ComputedLayout, LayoutError> {
    let arch = crate::intelligence::architecture::build_architecture_map(index, 2);

    if arch.modules.is_empty() {
        return Err(LayoutError::Empty);
    }

    // Collect which module prefixes participate in any circular dep.
    let circular_modules: std::collections::HashSet<String> = arch
        .circular_deps
        .iter()
        .flat_map(|cycle| cycle.iter().map(|path| file_module_prefix(path)))
        .collect();

    // Build one node per module.
    let nodes: Vec<LayoutNode> = arch
        .modules
        .iter()
        .map(|m| {
            let pr = m.aggregate_pagerank;
            let width = config.node_width * (1.0 + pr.min(1.0));
            LayoutNode {
                id: m.prefix.clone(),
                label: m.prefix.clone(),
                layer: 0,
                position: Point::default(),
                width,
                height: config.node_height,
                node_type: NodeType::Module,
                metadata: NodeMetadata {
                    pagerank: pr,
                    is_circular: circular_modules.contains(&m.prefix),
                    is_god_file: !m.god_files.is_empty(),
                    ..NodeMetadata::default()
                },
            }
        })
        .collect();

    // Build module id set for O(1) lookup.
    let module_ids: std::collections::HashSet<String> =
        nodes.iter().map(|n| n.id.clone()).collect();

    // Build cross-module import edges (deduplicated).
    let mut seen_edges: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut edges: Vec<LayoutEdge> = Vec::new();

    for (source_file, deps) in &index.graph.edges {
        let src_mod = file_module_prefix(source_file);
        if !module_ids.contains(&src_mod) {
            continue;
        }
        for dep in deps {
            let dst_mod = file_module_prefix(&dep.target);
            if dst_mod == src_mod || !module_ids.contains(&dst_mod) {
                continue;
            }
            let key = (src_mod.clone(), dst_mod.clone());
            if seen_edges.insert(key) {
                edges.push(LayoutEdge {
                    source: src_mod.clone(),
                    target: dst_mod.clone(),
                    edge_type: EdgeVisualType::Import,
                    weight: 1.0,
                    is_cycle: false,
                    waypoints: vec![],
                });
            }
        }
    }

    // Assign preliminary layer order for cognitive-limit enforcement.
    let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let edge_pairs: Vec<(String, String)> = edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    // Layer-assign to get initial layer grouping (ignore errors — fall back to one layer).
    let layer_map = layer_assign(&node_ids, &edge_pairs)
        .unwrap_or_else(|_| node_ids.iter().map(|id| (id.clone(), 0)).collect());

    let max_layer = layer_map.values().copied().max().unwrap_or(0);
    let mut layer_order: Vec<Vec<String>> = vec![Vec::new(); max_layer + 1];
    for id in &node_ids {
        let l = layer_map.get(id).copied().unwrap_or(0);
        layer_order[l].push(id.clone());
    }

    // Apply cognitive-limit clustering.
    let nodes = enforce_cognitive_limit(nodes, &mut layer_order, config.max_nodes_per_layer);

    compute_layout(nodes, edges, config)
}

/// Level 2 layout: files within a specific module prefix, edges for intra-module imports.
pub fn build_file_layout(
    index: &CodebaseIndex,
    module_prefix: &str,
    config: &LayoutConfig,
) -> Result<ComputedLayout, LayoutError> {
    // Filter files to this module.
    let module_files: Vec<&crate::index::IndexedFile> = index
        .files
        .iter()
        .filter(|f| file_module_prefix(&f.relative_path) == module_prefix)
        .collect();

    if module_files.is_empty() {
        return Err(LayoutError::Empty);
    }

    // Build risk lookup (path → risk_score).
    let risk_entries = crate::intelligence::risk::compute_risk_ranking(index);
    let risk_map: HashMap<&str, f64> = risk_entries
        .iter()
        .map(|r| (r.path.as_str(), r.risk_score))
        .collect();

    // Build god-file set from architecture map.
    let arch = crate::intelligence::architecture::build_architecture_map(index, 2);
    let god_file_set: std::collections::HashSet<&str> = arch
        .modules
        .iter()
        .filter(|m| m.prefix == module_prefix)
        .flat_map(|m| m.god_files.iter().map(|s| s.as_str()))
        .collect();

    // Build one node per file.
    let nodes: Vec<LayoutNode> = module_files
        .iter()
        .map(|f| {
            let pr = index
                .pagerank
                .get(f.relative_path.as_str())
                .copied()
                .unwrap_or(0.0);
            let risk = risk_map
                .get(f.relative_path.as_str())
                .copied()
                .unwrap_or(0.0);
            LayoutNode {
                id: f.relative_path.clone(),
                label: f.relative_path.clone(),
                layer: 0,
                position: Point::default(),
                width: config.node_width,
                height: config.node_height,
                node_type: NodeType::File,
                metadata: NodeMetadata {
                    pagerank: pr,
                    risk_score: risk,
                    token_count: f.token_count,
                    is_god_file: god_file_set.contains(f.relative_path.as_str()),
                    has_dead_code: false,
                    is_circular: false,
                    health_score: None,
                },
            }
        })
        .collect();

    // Build intra-module import edges.
    let file_ids: std::collections::HashSet<&str> = module_files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();

    let mut edges: Vec<LayoutEdge> = Vec::new();
    for f in &module_files {
        if let Some(deps) = index.graph.edges.get(f.relative_path.as_str()) {
            for dep in deps {
                if file_ids.contains(dep.target.as_str()) {
                    edges.push(LayoutEdge {
                        source: f.relative_path.clone(),
                        target: dep.target.clone(),
                        edge_type: EdgeVisualType::Import,
                        weight: 1.0,
                        is_cycle: false,
                        waypoints: vec![],
                    });
                }
            }
        }
    }

    // Preliminary layer order for cognitive-limit enforcement.
    let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let edge_pairs: Vec<(String, String)> = edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let layer_map = layer_assign(&node_ids, &edge_pairs)
        .unwrap_or_else(|_| node_ids.iter().map(|id| (id.clone(), 0)).collect());

    let max_layer = layer_map.values().copied().max().unwrap_or(0);
    let mut layer_order: Vec<Vec<String>> = vec![Vec::new(); max_layer + 1];
    for id in &node_ids {
        let l = layer_map.get(id).copied().unwrap_or(0);
        layer_order[l].push(id.clone());
    }

    let nodes = enforce_cognitive_limit(nodes, &mut layer_order, config.max_nodes_per_layer);

    compute_layout(nodes, edges, config)
}

/// Level 3 layout: symbols within a file, with call-graph edges.
///
/// Symbols are ordered by their appearance in the file (start_line).  When no
/// call-graph edges exist between the symbols, a simple linear chain is added
/// so the layout engine always produces a valid directed graph.
pub fn build_symbol_layout(
    index: &CodebaseIndex,
    file_path: &str,
    config: &LayoutConfig,
) -> Result<ComputedLayout, LayoutError> {
    let file = index
        .files
        .iter()
        .find(|f| f.relative_path == file_path)
        .ok_or(LayoutError::Empty)?;

    let parse_result = file.parse_result.as_ref().ok_or(LayoutError::Empty)?;

    if parse_result.symbols.is_empty() {
        return Err(LayoutError::Empty);
    }

    // Sort symbols by start_line for stable ordering.
    let mut symbols: Vec<&crate::parser::language::Symbol> = parse_result.symbols.iter().collect();
    symbols.sort_by_key(|s| s.start_line);

    let file_pr = index.pagerank.get(file_path).copied().unwrap_or(0.0);

    let nodes: Vec<LayoutNode> = symbols
        .iter()
        .map(|s| {
            let id = format!("{}::{}", file_path, s.name);
            LayoutNode {
                id: id.clone(),
                label: s.name.clone(),
                layer: 0,
                position: Point::default(),
                width: config.node_width,
                height: config.node_height,
                node_type: NodeType::Symbol,
                metadata: NodeMetadata {
                    pagerank: file_pr,
                    ..NodeMetadata::default()
                },
            }
        })
        .collect();

    // Build call-graph edges between symbols in this file.
    let symbol_id_map: HashMap<&str, String> = symbols
        .iter()
        .map(|s| (s.name.as_str(), format!("{}::{}", file_path, s.name)))
        .collect();

    let mut edges: Vec<LayoutEdge> = Vec::new();
    for call_edge in &index.call_graph.edges {
        if call_edge.caller_file != file_path || call_edge.callee_file != file_path {
            continue;
        }
        if let (Some(src_id), Some(dst_id)) = (
            symbol_id_map.get(call_edge.caller_symbol.as_str()),
            symbol_id_map.get(call_edge.callee_symbol.as_str()),
        ) {
            if src_id != dst_id {
                edges.push(LayoutEdge {
                    source: src_id.clone(),
                    target: dst_id.clone(),
                    edge_type: EdgeVisualType::Call,
                    weight: 1.0,
                    is_cycle: false,
                    waypoints: vec![],
                });
            }
        }
    }

    // If no call-graph edges found, create a linear chain by declaration order
    // so the layout engine always has a valid directed graph to work with.
    if edges.is_empty() && nodes.len() > 1 {
        for i in 0..(nodes.len() - 1) {
            edges.push(LayoutEdge {
                source: nodes[i].id.clone(),
                target: nodes[i + 1].id.clone(),
                edge_type: EdgeVisualType::Call,
                weight: 1.0,
                is_cycle: false,
                waypoints: vec![],
            });
        }
    }

    compute_layout(nodes, edges, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, layer: usize) -> LayoutNode {
        LayoutNode {
            id: id.to_string(),
            label: id.to_string(),
            layer,
            position: Point::default(),
            width: 160.0,
            height: 48.0,
            node_type: NodeType::Module,
            metadata: NodeMetadata::default(),
        }
    }

    fn make_edge(source: &str, target: &str) -> LayoutEdge {
        LayoutEdge {
            source: source.to_string(),
            target: target.to_string(),
            edge_type: EdgeVisualType::Import,
            weight: 1.0,
            is_cycle: false,
            waypoints: vec![],
        }
    }

    fn make_test_graph_5_nodes() -> (Vec<LayoutNode>, Vec<LayoutEdge>) {
        let nodes = vec![
            make_node("1", 0),
            make_node("2", 0),
            make_node("3", 0),
            make_node("4", 0),
            make_node("5", 0),
        ];
        let edges = vec![
            make_edge("1", "2"),
            make_edge("1", "3"),
            make_edge("2", "4"),
            make_edge("3", "4"),
            make_edge("4", "5"),
        ];
        (nodes, edges)
    }

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

    #[test]
    fn test_insert_dummy_nodes_creates_intermediates() {
        let nodes = vec![
            make_node("a", 0),
            make_node("b", 2), // layer 2, so edge a->b spans 2 layers
        ];
        let edges = vec![make_edge("a", "b")];
        let mut layers = HashMap::new();
        layers.insert("a".to_string(), 0);
        layers.insert("b".to_string(), 2);
        let (aug_nodes, aug_edges, dummy_ids) = insert_dummy_nodes(&nodes, &edges, &layers);
        assert_eq!(dummy_ids.len(), 1); // one dummy at layer 1
        assert_eq!(aug_nodes.len(), 3); // a, dummy, b
        assert_eq!(aug_edges.len(), 2); // a->dummy, dummy->b
    }

    #[test]
    fn test_assign_coordinates_no_horizontal_overlaps() {
        let layer_order = vec![
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            vec!["d".to_string(), "e".to_string()],
        ];
        let config = LayoutConfig::default();
        let coords = assign_coordinates(&layer_order, &config);
        let x_a = coords["a"].x;
        let x_b = coords["b"].x;
        let x_c = coords["c"].x;
        assert!(x_b > x_a + config.node_width);
        assert!(x_c > x_b + config.node_width);
    }

    #[test]
    fn test_compute_layout_produces_valid_positions() {
        let (nodes, edges) = make_test_graph_5_nodes();
        let layout = compute_layout(nodes, edges, &LayoutConfig::default()).unwrap();
        assert_eq!(layout.nodes.len(), 5);
        for node in &layout.nodes {
            assert!(node.position.x >= 0.0);
            assert!(node.position.y >= 0.0);
        }
        assert!(layout.width > 0.0);
        assert!(layout.height > 0.0);
    }

    #[test]
    fn test_barycenter_sort_preserves_optimal_order() {
        // Two parallel chains: a->c, b->d. Already optimal — should not swap.
        let mut layer_order = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["c".to_string(), "d".to_string()],
        ];
        let mut adj = HashMap::new();
        adj.insert("a".to_string(), vec!["c".to_string()]);
        adj.insert("b".to_string(), vec!["d".to_string()]);
        let mut rev_adj = HashMap::new();
        rev_adj.insert("c".to_string(), vec!["a".to_string()]);
        rev_adj.insert("d".to_string(), vec!["b".to_string()]);
        let original = layer_order.clone();
        barycenter_sort(&mut layer_order, &adj, &rev_adj);
        assert_eq!(layer_order, original);
    }

    // ---------------------------------------------------------------------------
    // Task 5 tests
    // ---------------------------------------------------------------------------

    /// Helper: build a minimal CodebaseIndex from (relative_path, content) pairs.
    fn make_minimal_index(paths: &[(&str, &str)]) -> crate::index::CodebaseIndex {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let files: Vec<ScannedFile> = paths
            .iter()
            .map(|(rel, content)| {
                let abs = dir.path().join(rel.replace('/', "_"));
                std::fs::write(&abs, content).unwrap();
                ScannedFile {
                    relative_path: rel.to_string(),
                    absolute_path: abs,
                    language: Some("rust".into()),
                    size_bytes: content.len() as u64,
                }
            })
            .collect();
        crate::index::CodebaseIndex::build(files, std::collections::HashMap::new(), &counter)
    }

    /// Helper: build a CodebaseIndex with symbols in one file.
    fn make_index_with_symbols(
        paths: &[(&str, &str)],
        file_with_symbols: &str,
        symbols: Vec<crate::parser::language::Symbol>,
    ) -> crate::index::CodebaseIndex {
        use crate::budget::counter::TokenCounter;
        use crate::parser::language::ParseResult;
        use crate::scanner::ScannedFile;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let mut files = Vec::new();
        let mut parse_results = std::collections::HashMap::new();
        let mut content_map = std::collections::HashMap::new();
        for (rel, content) in paths {
            let abs = dir.path().join(rel.replace('/', "_"));
            std::fs::write(&abs, content).unwrap();
            files.push(ScannedFile {
                relative_path: rel.to_string(),
                absolute_path: abs,
                language: Some("rust".into()),
                size_bytes: content.len() as u64,
            });
            content_map.insert(rel.to_string(), content.to_string());
        }
        parse_results.insert(
            file_with_symbols.to_string(),
            ParseResult {
                symbols,
                imports: vec![],
                exports: vec![],
            },
        );
        crate::index::CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    fn make_sym(name: &str, start_line: usize) -> crate::parser::language::Symbol {
        use crate::parser::language::{SymbolKind, Visibility};
        crate::parser::language::Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: format!("pub fn {}()", name),
            body: "{}".to_string(),
            start_line,
            end_line: start_line + 2,
        }
    }

    #[test]
    fn test_enforce_cognitive_limit_clusters_excess() {
        let nodes: Vec<LayoutNode> = (0..12).map(|i| make_node(&i.to_string(), 0)).collect();
        let mut layer_order = vec![(0..12).map(|i| i.to_string()).collect::<Vec<_>>()];
        let result = enforce_cognitive_limit(nodes, &mut layer_order, 9);
        // Layer should now have 9 entries: 8 original + 1 cluster
        assert_eq!(layer_order[0].len(), 9);
        // The last entry should be a cluster node
        let cluster_id = &layer_order[0][8];
        let cluster_node = result.iter().find(|n| n.id == *cluster_id).unwrap();
        assert!(
            matches!(cluster_node.node_type, NodeType::Cluster { .. }),
            "expected Cluster node, got {:?}",
            cluster_node.node_type
        );
        // Cluster should have 4 member ids (12 - 8 = 4)
        if let NodeType::Cluster { member_ids } = &cluster_node.node_type {
            assert_eq!(member_ids.len(), 4);
        }
        // Total nodes: 8 kept + 1 cluster (excess 4 removed)
        assert_eq!(result.len(), 9);
    }

    #[test]
    fn test_enforce_cognitive_limit_no_op_when_under_limit() {
        let nodes: Vec<LayoutNode> = (0..5).map(|i| make_node(&i.to_string(), 0)).collect();
        let mut layer_order = vec![(0..5).map(|i| i.to_string()).collect::<Vec<_>>()];
        let result = enforce_cognitive_limit(nodes, &mut layer_order, 9);
        assert_eq!(layer_order[0].len(), 5);
        assert_eq!(result.len(), 5);
        assert!(!result
            .iter()
            .any(|n| matches!(n.node_type, NodeType::Cluster { .. })));
    }

    #[test]
    fn test_build_module_layout_creates_nodes() {
        // Two files in different modules: src/a/mod.rs, src/b/mod.rs
        let index = make_minimal_index(&[
            ("src/a/mod.rs", "pub fn a() {}"),
            ("src/b/mod.rs", "pub fn b() {}"),
        ]);
        let config = LayoutConfig::default();
        let result = build_module_layout(&index, &config);
        assert!(
            result.is_ok(),
            "build_module_layout failed: {:?}",
            result.err()
        );
        let layout = result.unwrap();
        // Must have at least 2 nodes (one per module)
        assert!(
            layout.nodes.len() >= 2,
            "expected >= 2 nodes, got {}",
            layout.nodes.len()
        );
        // All nodes must have Module type
        for node in &layout.nodes {
            assert!(
                matches!(node.node_type, NodeType::Module | NodeType::Cluster { .. }),
                "unexpected node type: {:?}",
                node.node_type
            );
        }
        // Positions must be non-negative
        for node in &layout.nodes {
            assert!(node.position.x >= 0.0, "negative x for {}", node.id);
            assert!(node.position.y >= 0.0, "negative y for {}", node.id);
        }
    }

    #[test]
    fn test_build_file_layout_filters_by_module() {
        // Two files in src/a, one in src/b
        let index = make_minimal_index(&[
            ("src/a/one.rs", "fn one() {}"),
            ("src/a/two.rs", "fn two() {}"),
            ("src/b/other.rs", "fn other() {}"),
        ]);
        let config = LayoutConfig::default();
        let result = build_file_layout(&index, "src/a", &config);
        assert!(
            result.is_ok(),
            "build_file_layout failed: {:?}",
            result.err()
        );
        let layout = result.unwrap();
        // Only src/a files should appear
        for node in &layout.nodes {
            if matches!(node.node_type, NodeType::Cluster { .. }) {
                continue;
            }
            assert!(
                node.id.starts_with("src/a/"),
                "unexpected file in layout: {}",
                node.id
            );
        }
        // Must have exactly 2 file nodes (no clustering needed for 2 files)
        assert_eq!(
            layout.nodes.len(),
            2,
            "expected 2 nodes, got {}",
            layout.nodes.len()
        );
    }

    #[test]
    fn test_build_file_layout_empty_module_returns_error() {
        let index = make_minimal_index(&[("src/a/mod.rs", "fn a() {}")]);
        let config = LayoutConfig::default();
        let result = build_file_layout(&index, "src/nonexistent", &config);
        assert!(matches!(result, Err(LayoutError::Empty)));
    }

    #[test]
    fn test_build_symbol_layout_creates_symbol_nodes() {
        let symbols = vec![
            make_sym("alpha", 1),
            make_sym("beta", 10),
            make_sym("gamma", 20),
        ];
        let index = make_index_with_symbols(
            &[(
                "src/lib.rs",
                "pub fn alpha() {} pub fn beta() {} pub fn gamma() {}",
            )],
            "src/lib.rs",
            symbols,
        );
        let config = LayoutConfig::default();
        let result = build_symbol_layout(&index, "src/lib.rs", &config);
        assert!(
            result.is_ok(),
            "build_symbol_layout failed: {:?}",
            result.err()
        );
        let layout = result.unwrap();
        assert_eq!(
            layout.nodes.len(),
            3,
            "expected 3 symbol nodes, got {}",
            layout.nodes.len()
        );
        for node in &layout.nodes {
            assert!(
                matches!(node.node_type, NodeType::Symbol),
                "unexpected node type: {:?}",
                node.node_type
            );
            assert!(
                node.id.starts_with("src/lib.rs::"),
                "unexpected id: {}",
                node.id
            );
        }
    }

    #[test]
    fn test_build_symbol_layout_missing_file_returns_error() {
        let index = make_minimal_index(&[("src/lib.rs", "fn f() {}")]);
        let config = LayoutConfig::default();
        let result = build_symbol_layout(&index, "src/nonexistent.rs", &config);
        assert!(matches!(result, Err(LayoutError::Empty)));
    }

    #[test]
    fn test_build_symbol_layout_no_parse_result_returns_error() {
        // File exists in the index but has no parse_result (no symbols)
        let index = make_minimal_index(&[("src/lib.rs", "fn f() {}")]);
        let config = LayoutConfig::default();
        // The default build gives no parse results, so symbols will be empty
        let result = build_symbol_layout(&index, "src/lib.rs", &config);
        assert!(matches!(result, Err(LayoutError::Empty)));
    }
}

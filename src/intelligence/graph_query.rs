//! Deterministic graph-query core (cxpak 3.0.0 Task B1).
//!
//! Four primitives — [`node`], [`neighbors`], [`path`], [`subgraph`] — answer
//! structural questions over the typed [`DependencyGraph`]. This is the SINGLE
//! source of truth for graph-query: every surface (MCP `cxpak_graph`, LSP
//! `cxpak/graph`, CLI `graph`, HTTP `/v1/graph`) calls [`execute`] and reshapes
//! the result for transport — no surface re-derives (ADR-0153 single-source
//! invariant; the catalog's `graph` capability projects through
//! `capability::adapter`).
//!
//! # Determinism (hard contract, ADR-0176)
//!
//! Every output is byte-deterministic. The graph is backed by `BTreeMap` /
//! `BTreeSet` (see `core_graph::graph`), so neighbor iteration is already
//! sorted; on top of that:
//!
//! * [`neighbors`] sorts its combined result by `(node, direction, edge_type,
//!   confidence)` so the `both` direction is order-stable.
//! * [`subgraph`] returns nodes sorted and edges in `(from, to, edge_type)`
//!   order, induced over the included node set only.
//! * [`path`] resolves the **lexicographically-smallest shortest path** when
//!   several shortest paths exist (the explicit tiebreak): it computes each
//!   node's distance-to-target via a reverse BFS, then greedily walks from the
//!   source always choosing the smallest out-neighbour that still lies on a
//!   shortest path. Smallest-next-node at every step yields the lex-min node
//!   sequence; a diamond fixture proves the same canonical path every run.
//!
//! No `HashMap`/`HashSet` iteration order leaks into any output.
//!
//! Edge confidence is reported honestly: [`EdgeConfidence`] is rendered as
//! `extracted` / `inferred` (with an `inferred` bool) reusing
//! [`EdgeType::label`] and [`EdgeConfidence::is_inferred`] from Task 0.4.

use crate::core_graph::graph::{DependencyGraph, EdgeConfidence};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Lowercase, stable rendering of an [`EdgeConfidence`] for honest output.
fn confidence_label(c: EdgeConfidence) -> &'static str {
    match c {
        EdgeConfidence::Extracted => "extracted",
        EdgeConfidence::Inferred => "inferred",
    }
}

/// Direction selector for [`neighbors`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Outgoing edges — the dependencies of the node.
    Out,
    /// Incoming edges — the dependents of the node.
    In,
    /// Both directions, merged and re-sorted for determinism.
    Both,
}

impl Direction {
    /// Parse the `direction` parameter (`out` | `in` | `both`, case-insensitive).
    pub fn parse(s: &str) -> Option<Direction> {
        match s.to_ascii_lowercase().as_str() {
            "out" => Some(Direction::Out),
            "in" => Some(Direction::In),
            "both" => Some(Direction::Both),
            _ => None,
        }
    }

    /// Stable lowercase label echoed back in the result.
    pub fn label(self) -> &'static str {
        match self {
            Direction::Out => "out",
            Direction::In => "in",
            Direction::Both => "both",
        }
    }
}

/// Result of [`node`]: a node's existence and degree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: String,
    /// Whether the id participates in at least one edge in either direction.
    pub exists: bool,
    /// Number of outgoing edges (dependencies).
    pub out_degree: usize,
    /// Number of incoming edges (dependents).
    pub in_degree: usize,
}

/// Result of [`nodes`]: every node id known to the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeList {
    /// Every node id, sorted, each with its existence and degree.
    pub nodes: Vec<NodeInfo>,
}

/// One neighbour of a node, with the connecting edge described honestly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborEdge {
    /// The neighbouring node id.
    pub node: String,
    /// Edge type label (e.g. `import`, `foreign_key`).
    pub edge_type: String,
    /// `extracted` (structural) or `inferred` (heuristic).
    pub confidence: String,
    /// Whether the edge was heuristically inferred.
    pub inferred: bool,
    /// `out` if the edge points from the query node to this neighbour, `in` if
    /// it points from the neighbour to the query node.
    pub direction: String,
}

/// Result of [`neighbors`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighbors {
    pub id: String,
    pub direction: String,
    pub neighbors: Vec<NeighborEdge>,
}

/// One edge along a resolved path / in a subgraph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub edge_type: String,
    pub confidence: String,
    pub inferred: bool,
}

/// Result of [`path`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathResult {
    pub from: String,
    pub to: String,
    /// Whether a directed (out-edge) path exists.
    pub found: bool,
    /// The canonical shortest-path node sequence (inclusive of both endpoints),
    /// or empty when no path exists.
    pub nodes: Vec<String>,
    /// The edges traversed, in order.
    pub edges: Vec<GraphEdge>,
}

/// Result of [`subgraph`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subgraph {
    /// The real (edge-participating) seed node ids, sorted and de-duplicated.
    /// Unknown seeds are excluded here — see `unknown_seeds`.
    pub seeds: Vec<String>,
    pub depth: usize,
    /// All nodes within `depth` hops of any real seed (both directions), sorted.
    pub nodes: Vec<String>,
    /// The induced edges among those nodes, sorted by `(from, to, edge_type)`.
    pub edges: Vec<GraphEdge>,
    /// Seeds that are not edge-participating nodes (`graph.contains_node` is
    /// false), sorted. Never emitted as `nodes` — consistent with `node --id`
    /// reporting `exists:false` for the same id (ADR-0202).
    #[serde(default)]
    pub unknown_seeds: Vec<String>,
}

/// Error from [`execute`] when a request is malformed. Surfaces map these to
/// their own transport errors (HTTP 400, LSP `-32603`, MCP error text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphQueryError {
    /// A required parameter was absent or the wrong JSON type.
    MissingParam(String),
    /// A parameter was present but invalid (e.g. an unknown direction).
    InvalidParam(String),
    /// The `op` selector did not name a known primitive.
    UnknownOp(String),
}

impl fmt::Display for GraphQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphQueryError::MissingParam(p) => {
                write!(f, "missing or invalid required parameter: {p}")
            }
            GraphQueryError::InvalidParam(m) => write!(f, "invalid parameter: {m}"),
            GraphQueryError::UnknownOp(op) => write!(
                f,
                "unknown graph op `{op}`; expected one of nodes|node|neighbors|path|subgraph"
            ),
        }
    }
}

impl std::error::Error for GraphQueryError {}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

/// `node(id)` — existence and in/out degree of a node. Deterministic.
pub fn node(graph: &DependencyGraph, id: &str) -> NodeInfo {
    NodeInfo {
        id: id.to_string(),
        exists: graph.contains_node(id),
        out_degree: graph.dependencies(id).map(|s| s.len()).unwrap_or(0),
        in_degree: graph.dependents(id).len(),
    }
}

/// `nodes()` — every node id known to the graph (the union of `edges` and
/// `reverse_edges` keys), sorted with in/out degree. No prefix/focus filter
/// (ADR-0202): the bare enumerate op is all issue #20 asked for.
pub fn nodes(graph: &DependencyGraph) -> NodeList {
    let ids: BTreeSet<&String> = graph
        .edges
        .keys()
        .chain(graph.reverse_edges.keys())
        .collect();
    NodeList {
        nodes: ids.into_iter().map(|id| node(graph, id)).collect(),
    }
}

/// `neighbors(id, direction)` — direct neighbours with honest edge confidence,
/// returned in a fully deterministic order.
pub fn neighbors(graph: &DependencyGraph, id: &str, direction: Direction) -> Neighbors {
    let mut out = Vec::new();
    if matches!(direction, Direction::Out | Direction::Both) {
        if let Some(set) = graph.dependencies(id) {
            for e in set {
                out.push(NeighborEdge {
                    node: e.target.clone(),
                    edge_type: e.edge_type.label(),
                    confidence: confidence_label(e.confidence).to_string(),
                    inferred: e.confidence.is_inferred(),
                    direction: "out".to_string(),
                });
            }
        }
    }
    if matches!(direction, Direction::In | Direction::Both) {
        for e in graph.dependents(id) {
            out.push(NeighborEdge {
                node: e.target.clone(),
                edge_type: e.edge_type.label(),
                confidence: confidence_label(e.confidence).to_string(),
                inferred: e.confidence.is_inferred(),
                direction: "in".to_string(),
            });
        }
    }
    // Total order so the merged `both` result is byte-stable regardless of how
    // the two directions interleave.
    out.sort_by(|a, b| {
        a.node
            .cmp(&b.node)
            .then_with(|| a.direction.cmp(&b.direction))
            .then_with(|| a.edge_type.cmp(&b.edge_type))
            .then_with(|| a.confidence.cmp(&b.confidence))
    });
    Neighbors {
        id: id.to_string(),
        direction: direction.label().to_string(),
        neighbors: out,
    }
}

/// `path(from, to)` — the lexicographically-smallest shortest directed path
/// (following out-edges) from `from` to `to`, or `found = false`.
pub fn path(graph: &DependencyGraph, from: &str, to: &str) -> PathResult {
    // Trivial path: a node to itself, provided it is a real node.
    if from == to {
        let exists = graph.contains_node(from);
        return PathResult {
            from: from.to_string(),
            to: to.to_string(),
            found: exists,
            nodes: if exists {
                vec![from.to_string()]
            } else {
                vec![]
            },
            edges: vec![],
        };
    }

    // `dist[n]` = shortest number of out-edges from `n` to `to`. Computed by a
    // reverse BFS from `to` over incoming edges (deterministic: `dependents`
    // returns the reverse adjacency `BTreeSet` in sorted order).
    let mut dist: BTreeMap<String, usize> = BTreeMap::new();
    dist.insert(to.to_string(), 0);
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(to.to_string());
    while let Some(cur) = queue.pop_front() {
        let d = dist[&cur];
        for e in graph.dependents(&cur) {
            if !dist.contains_key(&e.target) {
                dist.insert(e.target.clone(), d + 1);
                queue.push_back(e.target.clone());
            }
        }
    }

    let Some(&from_dist) = dist.get(from) else {
        return PathResult {
            from: from.to_string(),
            to: to.to_string(),
            found: false,
            nodes: vec![],
            edges: vec![],
        };
    };

    // Greedy walk from the source: at each step take the smallest out-neighbour
    // (the graph's out-edge `BTreeSet` is sorted by target, then edge_type) that
    // still lies on a shortest path (`dist == remaining - 1`). Choosing the
    // smallest next node at every position yields the lexicographically-smallest
    // shortest path — the explicit, deterministic tiebreak.
    let mut nodes = vec![from.to_string()];
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut cur = from.to_string();
    let mut remaining = from_dist;
    while cur != to {
        let want = remaining - 1;
        let next = graph
            .dependencies(&cur)
            .and_then(|set| set.iter().find(|e| dist.get(&e.target) == Some(&want)))
            .expect("distance map guarantees a shortest-path successor exists");
        nodes.push(next.target.clone());
        edges.push(GraphEdge {
            from: cur.clone(),
            to: next.target.clone(),
            edge_type: next.edge_type.label(),
            confidence: confidence_label(next.confidence).to_string(),
            inferred: next.confidence.is_inferred(),
        });
        cur = next.target.clone();
        remaining = want;
    }

    PathResult {
        from: from.to_string(),
        to: to.to_string(),
        found: true,
        nodes,
        edges,
    }
}

/// `subgraph(seeds, depth)` — the induced subgraph of all nodes within `depth`
/// hops of any seed (both directions), with sorted nodes and induced edges.
pub fn subgraph(graph: &DependencyGraph, seeds: &[&str], depth: usize) -> Subgraph {
    // De-duplicated, sorted seeds → deterministic regardless of caller order.
    let sorted_seeds: BTreeSet<String> = seeds.iter().map(|s| s.to_string()).collect();

    // Partition into real (edge-participating) seeds, which drive the BFS,
    // and unknown seeds, which are reported honestly rather than echoed back
    // as a node — consistent with `node --id` (ADR-0202).
    let (real_seeds, unknown_seeds): (BTreeSet<String>, BTreeSet<String>) = sorted_seeds
        .into_iter()
        .partition(|s| graph.contains_node(s));

    // Multi-source bounded BFS, both directions, visiting each node once at its
    // minimum hop distance. `dist` keys form the included node set.
    let mut dist: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for s in &real_seeds {
        dist.insert(s.clone(), 0);
        queue.push_back(s.clone());
    }
    while let Some(cur) = queue.pop_front() {
        let d = dist[&cur];
        if d == depth {
            continue;
        }
        if let Some(set) = graph.dependencies(&cur) {
            for e in set {
                if !dist.contains_key(&e.target) {
                    dist.insert(e.target.clone(), d + 1);
                    queue.push_back(e.target.clone());
                }
            }
        }
        for e in graph.dependents(&cur) {
            if !dist.contains_key(&e.target) {
                dist.insert(e.target.clone(), d + 1);
                queue.push_back(e.target.clone());
            }
        }
    }

    let node_set: BTreeSet<String> = dist.keys().cloned().collect();
    let nodes: Vec<String> = node_set.iter().cloned().collect();

    // Induced edges: keep only out-edges whose both endpoints are in the set.
    // Iterating sorted nodes, each over its sorted out-edge `BTreeSet`, yields
    // edges already in `(from, to, edge_type)` order.
    let mut edges: Vec<GraphEdge> = Vec::new();
    for u in &nodes {
        if let Some(set) = graph.dependencies(u) {
            for e in set {
                if node_set.contains(&e.target) {
                    edges.push(GraphEdge {
                        from: u.clone(),
                        to: e.target.clone(),
                        edge_type: e.edge_type.label(),
                        confidence: confidence_label(e.confidence).to_string(),
                        inferred: e.confidence.is_inferred(),
                    });
                }
            }
        }
    }

    Subgraph {
        seeds: real_seeds.into_iter().collect(),
        depth,
        nodes,
        edges,
        unknown_seeds: unknown_seeds.into_iter().collect(),
    }
}

// ---------------------------------------------------------------------------
// Single dispatch entry point — every surface calls this.
// ---------------------------------------------------------------------------

/// Execute a graph-query `op` with JSON `params` against `graph`, returning the
/// deterministic JSON result. This is the one core all four surfaces invoke.
///
/// * `nodes` — params: none. Lists every node id (repo-relative file path)
///   known to the graph, sorted, each with in/out degree — the way to
///   discover valid ids for the other ops (ADR-0202).
/// * `node` — params: `{ "id": string }` (id: repo-relative file path,
///   enumerate with `nodes`)
/// * `neighbors` — params: `{ "id": string, "direction"?: "out"|"in"|"both" }`
/// * `path` — params: `{ "from": string, "to": string }`
/// * `subgraph` — params: `{ "seeds": [string], "depth"?: number }`; unknown
///   seeds are partitioned into `unknown_seeds`, never emitted as `nodes`
///   (ADR-0202)
pub fn execute(
    graph: &DependencyGraph,
    op: &str,
    params: &Value,
) -> Result<Value, GraphQueryError> {
    match op {
        "nodes" => Ok(to_json(&nodes(graph))),
        "node" => {
            let id = req_str(params, "id")?;
            Ok(to_json(&node(graph, id)))
        }
        "neighbors" => {
            let id = req_str(params, "id")?;
            let dir_str = params
                .get("direction")
                .and_then(|v| v.as_str())
                .unwrap_or("both");
            let dir = Direction::parse(dir_str).ok_or_else(|| {
                GraphQueryError::InvalidParam("direction must be out|in|both".to_string())
            })?;
            Ok(to_json(&neighbors(graph, id, dir)))
        }
        "path" => {
            let from = req_str(params, "from")?;
            let to = req_str(params, "to")?;
            Ok(to_json(&path(graph, from, to)))
        }
        "subgraph" => {
            let seeds_val = params
                .get("seeds")
                .and_then(|v| v.as_array())
                .ok_or_else(|| GraphQueryError::MissingParam("seeds".to_string()))?;
            let seeds: Vec<&str> = seeds_val.iter().filter_map(|v| v.as_str()).collect();
            if seeds.is_empty() {
                return Err(GraphQueryError::MissingParam("seeds".to_string()));
            }
            let depth = params
                .get("depth")
                .and_then(|v| v.as_u64())
                .map(|d| d as usize)
                .unwrap_or(1);
            Ok(to_json(&subgraph(graph, &seeds, depth)))
        }
        other => Err(GraphQueryError::UnknownOp(other.to_string())),
    }
}

/// Extract a required non-empty string parameter.
fn req_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, GraphQueryError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| GraphQueryError::MissingParam(key.to_string()))
}

/// Serialize a graph-query result struct to JSON. These structs only contain
/// strings, numbers, bools, and arrays thereof, so serialization is infallible.
fn to_json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("graph-query result structs always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_graph::graph::EdgeType;
    use serde_json::json;

    /// Linear graph: a -> b -> c (Import edges).
    fn linear() -> DependencyGraph {
        let mut g = DependencyGraph::new();
        g.add_edge("a", "b", EdgeType::Import);
        g.add_edge("b", "c", EdgeType::Import);
        g
    }

    /// Diamond: a -> b, a -> c, b -> d, c -> d. Two equal-length a..d paths.
    fn diamond() -> DependencyGraph {
        let mut g = DependencyGraph::new();
        g.add_edge("a", "b", EdgeType::Import);
        g.add_edge("a", "c", EdgeType::Import);
        g.add_edge("b", "d", EdgeType::Import);
        g.add_edge("c", "d", EdgeType::Import);
        g
    }

    #[test]
    fn node_existing_reports_degrees() {
        let g = linear();
        let n = node(&g, "b");
        assert!(n.exists);
        assert_eq!(n.out_degree, 1);
        assert_eq!(n.in_degree, 1);
        assert_eq!(n.id, "b");
    }

    #[test]
    fn node_missing_has_zero_degree() {
        let g = linear();
        let n = node(&g, "nope");
        assert!(!n.exists);
        assert_eq!(n.out_degree, 0);
        assert_eq!(n.in_degree, 0);
    }

    #[test]
    fn neighbors_out_sorted_with_confidence() {
        let mut g = DependencyGraph::new();
        g.add_edge("x", "z_import", EdgeType::Import);
        // EmbeddedSql defaults to Inferred — honest confidence tagging.
        g.add_edge("x", "a_sql", EdgeType::EmbeddedSql);
        let r = neighbors(&g, "x", Direction::Out);
        let ids: Vec<&str> = r.neighbors.iter().map(|n| n.node.as_str()).collect();
        assert_eq!(ids, vec!["a_sql", "z_import"], "sorted by node id");
        let sql = &r.neighbors[0];
        assert_eq!(sql.edge_type, "embedded_sql");
        assert_eq!(sql.confidence, "inferred");
        assert!(sql.inferred);
        assert_eq!(sql.direction, "out");
        let imp = &r.neighbors[1];
        assert_eq!(imp.confidence, "extracted");
        assert!(!imp.inferred);
    }

    #[test]
    fn neighbors_in_lists_dependents() {
        let g = linear();
        let r = neighbors(&g, "b", Direction::In);
        assert_eq!(r.neighbors.len(), 1);
        assert_eq!(r.neighbors[0].node, "a");
        assert_eq!(r.neighbors[0].direction, "in");
    }

    #[test]
    fn neighbors_both_merges_and_sorts() {
        let g = linear();
        let r = neighbors(&g, "b", Direction::Both);
        assert_eq!(r.direction, "both");
        // b depends on c (out) and is depended on by a (in).
        let pairs: Vec<(&str, &str)> = r
            .neighbors
            .iter()
            .map(|n| (n.node.as_str(), n.direction.as_str()))
            .collect();
        assert_eq!(pairs, vec![("a", "in"), ("c", "out")]);
    }

    #[test]
    fn path_found_linear() {
        let g = linear();
        let r = path(&g, "a", "c");
        assert!(r.found);
        assert_eq!(r.nodes, vec!["a", "b", "c"]);
        assert_eq!(r.edges.len(), 2);
        assert_eq!(r.edges[0].from, "a");
        assert_eq!(r.edges[0].to, "b");
        assert_eq!(r.edges[0].edge_type, "import");
    }

    #[test]
    fn path_none_when_unreachable() {
        let g = linear();
        // c has no out-edge to a.
        let r = path(&g, "c", "a");
        assert!(!r.found);
        assert!(r.nodes.is_empty());
        assert!(r.edges.is_empty());
    }

    #[test]
    fn path_from_equals_to() {
        let g = linear();
        let r = path(&g, "b", "b");
        assert!(r.found);
        assert_eq!(r.nodes, vec!["b"]);
        assert!(r.edges.is_empty());
    }

    #[test]
    fn path_diamond_tiebreak_is_canonical_lexmin() {
        let g = diamond();
        // Two shortest paths a-b-d and a-c-d; the canonical one is lex-min.
        let r = path(&g, "a", "d");
        assert!(r.found);
        assert_eq!(
            r.nodes,
            vec!["a", "b", "d"],
            "must pick lexicographically-smallest shortest path"
        );
        // Stable across repeated runs (no HashMap order leak).
        for _ in 0..100 {
            let again = path(&g, "a", "d");
            assert_eq!(again.nodes, r.nodes);
            assert_eq!(
                serde_json::to_string(&again).unwrap(),
                serde_json::to_string(&r).unwrap()
            );
        }
    }

    #[test]
    fn subgraph_depth_bound() {
        let g = linear(); // a -> b -> c
        let sg = subgraph(&g, &["a"], 1);
        // depth 1 from a reaches b (out) only.
        assert_eq!(sg.nodes, vec!["a", "b"]);
        assert_eq!(sg.depth, 1);
        assert_eq!(sg.seeds, vec!["a"]);
        // induced edge a->b only (b->c excluded: c not in set).
        assert_eq!(sg.edges.len(), 1);
        assert_eq!(sg.edges[0].from, "a");
        assert_eq!(sg.edges[0].to, "b");
    }

    #[test]
    fn subgraph_depth_two_includes_all_and_induced_edges() {
        let g = linear();
        let sg = subgraph(&g, &["a"], 2);
        assert_eq!(sg.nodes, vec!["a", "b", "c"]);
        assert_eq!(sg.edges.len(), 2);
    }

    #[test]
    fn subgraph_both_directions_from_middle() {
        let g = linear();
        let sg = subgraph(&g, &["b"], 1);
        // b reaches a (in) and c (out).
        assert_eq!(sg.nodes, vec!["a", "b", "c"]);
    }

    #[test]
    fn subgraph_seeds_sorted_and_deduped() {
        let g = diamond();
        let sg = subgraph(&g, &["c", "b", "b"], 0);
        assert_eq!(sg.seeds, vec!["b", "c"]);
        // depth 0 → only the seeds, no expansion.
        assert_eq!(sg.nodes, vec!["b", "c"]);
        assert!(sg.edges.is_empty(), "no edges among b,c directly");
    }

    #[test]
    fn nodes_enumerates_all_ids_sorted_with_degree() {
        let g = linear(); // a -> b -> c
        let list = nodes(&g);
        let ids: Vec<&str> = list.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["a", "b", "c"],
            "sorted, every edge-participating id"
        );
        for n in &list.nodes {
            assert!(n.exists, "every enumerated node exists by construction");
        }
        let a = list.nodes.iter().find(|n| n.id == "a").unwrap();
        assert_eq!(a.out_degree, 1);
        assert_eq!(a.in_degree, 0);
        let b = list.nodes.iter().find(|n| n.id == "b").unwrap();
        assert_eq!(b.out_degree, 1);
        assert_eq!(b.in_degree, 1);
        let c = list.nodes.iter().find(|n| n.id == "c").unwrap();
        assert_eq!(c.out_degree, 0);
        assert_eq!(c.in_degree, 1);
    }

    #[test]
    fn nodes_diamond_union_of_edges_and_reverse_edges() {
        let g = diamond(); // a->b, a->c, b->d, c->d
        let list = nodes(&g);
        let ids: Vec<&str> = list.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c", "d"]);
        let d = list.nodes.iter().find(|n| n.id == "d").unwrap();
        assert_eq!(d.out_degree, 0);
        assert_eq!(d.in_degree, 2);
    }

    #[test]
    fn nodes_empty_graph_is_empty() {
        let g = DependencyGraph::new();
        let list = nodes(&g);
        assert!(list.nodes.is_empty());
    }

    #[test]
    fn subgraph_partitions_unknown_seeds() {
        let g = linear(); // a -> b -> c
        let sg = subgraph(&g, &["a", "bogus"], 1);
        assert_eq!(sg.seeds, vec!["a"], "unknown seed excluded from seeds too");
        assert_eq!(sg.unknown_seeds, vec!["bogus"]);
        assert_eq!(
            sg.nodes,
            vec!["a", "b"],
            "bogus seed never emitted as a node"
        );
    }

    #[test]
    fn subgraph_all_unknown_empty_nodes() {
        let g = linear();
        let sg = subgraph(&g, &["totally", "bogus"], 1);
        assert_eq!(sg.seeds, Vec::<String>::new());
        assert_eq!(sg.unknown_seeds, vec!["bogus", "totally"], "sorted");
        assert!(sg.nodes.is_empty());
        assert!(sg.edges.is_empty());
    }

    #[test]
    fn subgraph_mixed_seeds() {
        let g = diamond(); // a->b, a->c, b->d, c->d
        let sg = subgraph(&g, &["a", "nope", "d"], 1);
        assert_eq!(sg.seeds, vec!["a", "d"]);
        assert_eq!(sg.unknown_seeds, vec!["nope"]);
        // depth 1 from a (out: b,c) and d (in: b,c) both real seeds.
        assert_eq!(sg.nodes, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn subgraph_real_but_edgeless_seed() {
        // A file with zero resolved edges is not a node (contains_node is
        // false), matching `node --id` exists:false for the same case
        // (ADR-0203 main.rs class) — it lands in unknown_seeds too.
        let g = linear();
        let sg = subgraph(&g, &["a", "src/main.rs"], 1);
        assert_eq!(sg.seeds, vec!["a"]);
        assert_eq!(sg.unknown_seeds, vec!["src/main.rs"]);
        assert_eq!(sg.nodes, vec!["a", "b"]);
    }

    #[test]
    fn execute_node_roundtrips() {
        let g = linear();
        let v = execute(&g, "node", &json!({"id": "b"})).unwrap();
        assert_eq!(v["exists"], json!(true));
        assert_eq!(v["out_degree"], json!(1));
    }

    #[test]
    fn execute_neighbors_default_direction_is_both() {
        let g = linear();
        let v = execute(&g, "neighbors", &json!({"id": "b"})).unwrap();
        assert_eq!(v["direction"], json!("both"));
    }

    #[test]
    fn execute_path_and_subgraph() {
        let g = diamond();
        let p = execute(&g, "path", &json!({"from": "a", "to": "d"})).unwrap();
        assert_eq!(p["nodes"], json!(["a", "b", "d"]));
        let s = execute(&g, "subgraph", &json!({"seeds": ["a"], "depth": 1})).unwrap();
        assert_eq!(s["depth"], json!(1));
    }

    #[test]
    fn execute_nodes_op_returns_sorted_list() {
        let g = linear();
        let v = execute(&g, "nodes", &json!({})).unwrap();
        let ids: Vec<&str> = v["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn execute_unknown_op_lists_nodes() {
        let g = linear();
        let err = execute(&g, "frobnicate", &json!({})).unwrap_err();
        assert!(
            err.to_string().contains("nodes"),
            "UnknownOp message must name `nodes` as a valid op: {err}"
        );
    }

    #[test]
    fn execute_missing_param_errors() {
        let g = linear();
        assert_eq!(
            execute(&g, "node", &json!({})),
            Err(GraphQueryError::MissingParam("id".into()))
        );
        assert_eq!(
            execute(&g, "path", &json!({"from": "a"})),
            Err(GraphQueryError::MissingParam("to".into()))
        );
        assert!(matches!(
            execute(&g, "subgraph", &json!({"depth": 1})),
            Err(GraphQueryError::MissingParam(_))
        ));
    }

    #[test]
    fn execute_invalid_direction_errors() {
        let g = linear();
        assert!(matches!(
            execute(
                &g,
                "neighbors",
                &json!({"id": "b", "direction": "sideways"})
            ),
            Err(GraphQueryError::InvalidParam(_))
        ));
    }

    #[test]
    fn execute_unknown_op_errors() {
        let g = linear();
        assert_eq!(
            execute(&g, "frobnicate", &json!({})),
            Err(GraphQueryError::UnknownOp("frobnicate".into()))
        );
    }

    #[test]
    fn outputs_are_byte_deterministic() {
        let g = diamond();
        let ops = [
            ("nodes", json!({})),
            ("node", json!({"id": "a"})),
            ("neighbors", json!({"id": "a", "direction": "both"})),
            ("path", json!({"from": "a", "to": "d"})),
            ("subgraph", json!({"seeds": ["a", "d"], "depth": 2})),
        ];
        for (op, params) in ops {
            let first = serde_json::to_string(&execute(&g, op, &params).unwrap()).unwrap();
            for _ in 0..50 {
                let again = serde_json::to_string(&execute(&g, op, &params).unwrap()).unwrap();
                assert_eq!(again, first, "op `{op}` must be byte-deterministic");
            }
        }
    }
}

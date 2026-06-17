---
id: '0047'
title: Reverse adjacency index for O(1) dependents lookup
status: ACCEPTED
date: 2026-03-12
triggered_by: v0.7.0 performance workstream addressing O(V*E) dependents() scan
loop: planning
---

# ADR-0047: Reverse adjacency index for O(1) dependents lookup

## Context

Released in v0.7.0. `DependencyGraph.dependents()` scanned every edge on each call, costing ~1M comparisons for a 500-file / 2000-edge repo during `rank_files()` and trace/diff walks. The fix needed O(1) reverse lookups without changing the public API.

## Options considered

- **Option A — reverse adjacency index (`reverse_edges` map):** Maintain a second map from each target to the set of files importing it, populated in `add_edge()`; `dependents()` becomes a single map lookup. Pros: O(1) dependents, identical public API so existing tests pass unchanged, also speeds `reachable_from()`'s incoming-edge traversal. Cons: extra memory; `add_edge` must keep both maps consistent. Someone could prefer it because it is API-compatible and reuses the existing hand-rolled adjacency representation.
- **Option B — keep the linear scan:** A reasonable alternative would have been to leave `dependents()` iterating all edges each call. Pros: zero code change, no extra memory. Cons: O(V*E) per call, the dominant cost in ranking. Someone could prefer it only to avoid touching the graph at all. Not formally evaluated.
- **Option C — switch to a petgraph `DiGraph`:** A reasonable alternative would have been to replace the hand-rolled adjacency maps with a graph library providing reverse edge iteration natively. Pros: reverse traversal built in, richer algorithms. Cons: larger refactor, new dependency on the core path. Someone could prefer it for the broader algorithm toolkit, but it was not pursued for this targeted fix.

## Decision

Add a `reverse_edges` map to `DependencyGraph`, maintained inside `add_edge()`, so `dependents()` is an O(1) lookup and `reachable_from()` uses `reverse_edges.get()` instead of scanning all edges. The existing public API stays identical. (The design proposed `HashMap<String, HashSet<String>>`; the shipped code uses `BTreeMap<String, BTreeSet<TypedEdge>>` for typed edges and deterministic ordering — the same decision, refined container types.)

## Consequences

### Positive
- `dependents()` drops from O(V*E) to O(1).
- `reachable_from()` incoming-edge traversal also accelerated.
- API-compatible — existing graph tests pass unchanged.

### Negative
- Extra memory proportional to edge count.

### Neutral
- Adds a parallel reverse map that must be kept consistent in `add_edge`.

## Revisit if
- The graph backing store is replaced (e.g. petgraph).
- Edge removal semantics change such that `reverse_edges` consistency is hard to maintain.

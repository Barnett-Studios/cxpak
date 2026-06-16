---
id: '0118'
title: Circular dependency detection via Tarjan's SCC, not full cycle enumeration
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.2.0 architecture map needs to report circular dependency groups
loop: planning
---

# ADR-0118: Circular dependency detection via Tarjan's SCC, not full cycle enumeration

## Context

Released in v1.2.0. The architecture map must report circular dependencies in the dependency graph. Full cycle enumeration is exponential in the worst case (the number of distinct cycles can grow exponentially with graph size). The reporting goal, however, is only to identify the groups of mutually-reachable nodes, not to list every concrete cycle — which makes a strongly-connected-components approach sufficient and far cheaper.

## Options considered

- **Option A — Tarjan's strongly-connected-components, forward edges only (chosen):** Run Tarjan's SCC (O(V+E)) over the dependency graph following only forward edges (not `reverse_edges`); each SCC with more than one node is a circular dependency group, reported as an ordered list of file paths. Pros: linear time, identifies cycle groups without exponential enumeration, deterministic. Cons: reports the SCC group, not each individual cycle within it.
- **Option B — full cycle enumeration:** A reasonable alternative would have been to enumerate every distinct cycle in the graph. Pros: lists each concrete cycle individually. Cons: can be exponential in the number of cycles, expensive for large graphs. Rejected — the cost is unjustified when SCC grouping answers the reporting need.

## Decision

Detect circular dependencies with Tarjan's SCC algorithm (O(V+E)) on the dependency graph, following only forward edges (not `reverse_edges`). Each strongly connected component with more than one node is reported as one circular dependency group — an ordered list of file paths. Full cycle enumeration is explicitly avoided because it can be exponential. Shipped as `find_circular_dep_groups()` in `src/intelligence/architecture.rs`, populating `ArchitectureMap.circular_deps`.

## Consequences

### Positive
- Linear-time cycle detection scales to large graphs.
- Deterministic SCC grouping.

### Negative
- Reports SCC groups rather than individual concrete cycles within a group.

### Neutral
- Coupling analysis elsewhere in the same module uses `reverse_edges`; cycle detection deliberately does not.

## Revisit if
- Users need individual cycles enumerated within an SCC.

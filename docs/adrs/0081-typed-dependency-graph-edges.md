---
id: '0081'
title: Replace string edge sets with TypedEdge carrying an EdgeType enum
status: ACCEPTED
date: 2026-03-21
triggered_by: v0.12.0 data layer awareness — need to distinguish import edges from schema relationships in one unified graph
loop: planning
---

# ADR-0081: Replace string edge sets with TypedEdge carrying an EdgeType enum

## Context

cxpak v0.12.0 introduces data-layer awareness. Before this release, the `DependencyGraph`
stored edges as `HashSet<String>` (bare target paths), so every relationship was an untyped
import. To represent foreign keys, embedded SQL, ORM mappings, and migration ordering
alongside imports inside a single graph, edges need a semantic tag.

The decision is a full migration of every consumer call site (~19 across 6 files) rather
than an incremental or parallel-graph approach. The graph lives in `src/index/graph.rs`.

## Options considered

- **Option A — `TypedEdge` with an `EdgeType` enum, full consumer migration:**
  store edges as a map from source to a set of `TypedEdge { target, edge_type }`, carry all
  edge types in one graph, and migrate every consumer to read `.target`. This yields a single
  unified graph in which every consumer benefits and semantic edges are available everywhere;
  the cost is touching all the call sites (~19 across 6 files) in one change. The design doc
  argues that doing the migration incrementally wastes time. Chosen.

- **Option B — incremental migration:** convert edge types call site by call site over
  multiple releases. Someone could prefer this for smaller, safer PRs. Rejected: the design
  doc explicitly notes that doing it incrementally wastes time.

- **Option C — separate parallel schema graph:** keep the import graph as strings and hold
  schema edges in a second structure. A reasonable alternative would have been to leave
  existing consumers untouched and traverse schema edges separately. Rejected: it forces two
  graphs to traverse, and graph algorithms (BFS, PageRank) could not see paths that cross
  between import and schema edges.

## Decision

Introduce `EdgeType` (at v0.12.0: `Import`, `ForeignKey`, `ViewReference`, `TriggerTarget`,
`IndexTarget`, `FunctionReference`, `EmbeddedSql`, `OrmModel`, `MigrationSequence`) and
`TypedEdge { target, edge_type }`. Rewrite the `DependencyGraph` edge storage from
`HashSet<String>` to a set of `TypedEdge`, add `edge_type` to `add_edge()`, and migrate all
consumers to extract `.target`. Existing import edges all receive `EdgeType::Import`, so
behaviour is unchanged for consumers that ignore types.

> As shipped, the storage type later moved from `HashSet<TypedEdge>` (the v0.12.0 design choice)
> to `BTreeSet<TypedEdge>` inside a `BTreeMap` in v2.1.0, for deterministic, cross-process
> iteration order.

## Consequences

### Positive
- A single unified graph carries semantic meaning on every edge.
- The same target reached by two edge types becomes two distinct edges (the hash/ordering
  includes `edge_type`).
- Downstream features (blast-radius edge weights, PageRank) can reason about the kind of
  relationship.

### Negative
- One large breaking change across `graph.rs`, `ranking.rs`, `seed.rs`, `trace.rs`,
  `diff.rs`, `serve.rs`, and `overview.rs`.
- The edge set no longer supports `.contains(&str)`, forcing
  `.iter().any(|e| e.target == ...)` in tests. This held for the original `HashSet<TypedEdge>`
  and still holds for the shipped `BTreeSet<TypedEdge>`.

### Neutral
- `dependents()` return type changed from `Vec<&str>` to `Vec<&TypedEdge>`.
- `EdgeType` was later consumed by v0.13.0 risk scoring with per-type weights.
- The 9-variant enum framing was for v0.12.0; the enum was later extended to a 10th variant,
  `CrossLanguage(BridgeType)`, in v1.5.0 — see the "Revisit if" trigger below, which was in
  fact exercised.

## Revisit if
- A new relationship kind appears that does not fit the enum's variants. (This was triggered:
  v1.5.0 added the `CrossLanguage(BridgeType)` variant.)
- Edge weights need to vary per-edge rather than per-type.

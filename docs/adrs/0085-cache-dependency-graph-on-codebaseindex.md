---
id: '0085'
title: Build and cache the DependencyGraph once on CodebaseIndex instead of on-demand per call site
status: ACCEPTED
date: 2026-03-22
triggered_by: PageRank and test mapping both need the full graph at build time, and five call sites were each rebuilding it
loop: planning
---

# ADR-0085: Build and cache the DependencyGraph once on CodebaseIndex instead of on-demand per call site

## Context

Before v0.13.0 the dependency graph was constructed on demand at each call site that needed it — `trace`, `diff`, `serve`, `overview`, and the relevance seed selector — so the same graph was built up to five times per invocation. v0.13.0 introduces build-time intelligence (PageRank file importance and source→test mapping) that both require the full graph during index construction, making redundant on-demand construction untenable.

The decision is to make the graph a build-time field on `CodebaseIndex` and have every caller reuse `index.graph`. This forces `build_dependency_graph` to take `&[IndexedFile]` plus `Option<&SchemaIndex>` rather than `&CodebaseIndex`, because the graph must be built during construction of the index itself and cannot reference an index that does not yet exist.

## Options considered

- **Option A — Cache graph on CodebaseIndex, build once:** Add `pub graph: DependencyGraph` (plus `pagerank` and `test_map`) to `CodebaseIndex`, built during `build()`/`build_with_content()` after the schema index. Change `build_dependency_graph` and `build_schema_edges` to accept `&[IndexedFile]` + `Option<&SchemaIndex>`. Pros: eliminates redundant construction, enables build-time PageRank and test mapping, all callers share one consistent graph. Cons: introduces a strict build-time ordering dependency, and the signature change ripples to `build_schema_edges` and integration-test call sites. This was the chosen option.
- **Option B — Keep on-demand construction:** Each caller continues to build its own graph as before. Pros: no struct change, no signature churn. Cons: redundant work on every call, and PageRank/test mapping cannot be precomputed or cached at build time. Someone could prefer this to avoid the ordering constraint and keep `CodebaseIndex` lean, but it cannot support the v0.13.0 intelligence features that need the graph during construction.

## Decision

Add `pub graph: DependencyGraph`, `pub pagerank`, and `pub test_map` to `CodebaseIndex`, built sequentially during `build()`/`build_with_content()` in the order: index → schema → graph → pagerank → test_map. Change `build_dependency_graph` and `build_schema_edges` to accept `&[IndexedFile]` + `Option<&SchemaIndex>` instead of `&CodebaseIndex`, and update `trace`, `diff`, `serve`, and `overview` to consume `index.graph`; the seed selector takes the prebuilt graph as a parameter. An explicit `rebuild_graph()` is provided for the case where the schema is mutated after construction (used by `serve`).

## Consequences

### Positive
- Redundant on-demand graph construction is eliminated; the graph is built exactly once.
- PageRank and test mapping are precomputed and cached on the index.
- All callers reuse one consistent graph instead of independently derived copies.

### Negative
- A strict build-time ordering is now required; later schema mutation must call `rebuild_graph()` explicitly.
- The signature change touches `build_schema_edges` and integration-test call sites.

### Neutral
- `seed.rs` retains a fallback graph build, but its `None` branch is documented as effectively dead since callers pass `index.graph`.

## Revisit if
- Graph construction cost begins to dominate index build time for very large repositories.
- Lazy graph construction becomes preferable for memory reasons.

---
id: '0054'
title: Defer dependency-graph caching to a later release
status: ACCEPTED
date: 2026-03-13
triggered_by: Design doc proposed a graph_cache.json keyed by a cache-mtime hash; implementation plan reassessed its value
loop: implementation
---

# ADR-0054: Defer dependency-graph caching to a later release

## Context
Decided during v0.7.0 implementation. The v0.7.0 design doc proposed persisting the dependency graph to `graph_cache.json` with a `cache_hash` derived from sorted path+mtime pairs, rebuilding only when the hash differs (Problem 3). The implementation plan reassessed this: graph construction is already linear in files × imports and cheap relative to parsing, and the design doc itself flagged it as the lowest-impact of the three perf changes. Confirmed in code: no `GraphCache`, `graph_cache`, or `cache_hash` symbols exist in `src/`, and no `graph_cache.json` artifact is produced. The two perf wins that were in scope did ship (reverse-edges index in src/index/graph.rs; `build_with_content` double-read fix in src/index/mod.rs; the `--since` feature).

## Options considered
- **Option A — ship graph cache in v0.7.0:** add a `GraphCache` struct with load/save and a `cache_hash` invalidation key, checked in trace/overview/diff before rebuilding (Problem 3 / Solution, design doc). Pros: skips the graph rebuild on unchanged repos. Cons: lowest-impact of the three perf changes; the graph build is already fast; adds cache-invalidation complexity and a second cache file. Rejected for v0.7.0.
- **Option B — defer to v0.7.1 / later (chosen):** do not implement graph caching now; revisit only if benchmarks show the graph build matters after the first two perf wins. This is the design doc's own recommendation. Pros: avoids low-value complexity; the first two changes are sufficient. Cons: the graph is still rebuilt from parse results on each cold invocation.

## Decision
Defer graph caching. The implementation plan explicitly drops Problem 3 from v0.7.0 scope: graph build is already fast, the reverse-index and double-read fixes are sufficient, and the design doc itself flagged it as the lowest-impact change to do only if benchmarks justify it.

## Consequences
### Positive
- Avoids adding a second cache file and its invalidation logic for marginal gain.
- Keeps v0.7.0 scope tight.

### Negative
- No persisted graph; large repos pay the graph-build cost on every cold run.

### Neutral
- The graph is still rebuilt from parse results on each cold invocation.

## Revisit if
- Benchmarks show graph construction is a measurable bottleneck.
- Cold-start latency on large repos becomes a complaint.

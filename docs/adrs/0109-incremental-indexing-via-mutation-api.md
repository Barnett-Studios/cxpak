---
id: '0109'
title: 'Incremental indexing: file-level mtime/size invalidation for parsing, full recompute for graph-derived scores'
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.2.0 incremental indexing to keep the index responsive without full re-parse
loop: planning
---

# ADR-0109: Incremental indexing: file-level mtime/size invalidation for parsing, full recompute for graph-derived scores

## Context

Introduced in v1.2.0. Re-indexing must avoid re-parsing unchanged files — tree-sitter parsing is the real bottleneck — while keeping graph-derived scores (PageRank, blast radius, coupling, co-changes, health) correct. The open question was whether to also incrementally update the graph algorithms as edges change, or to recompute them wholesale after the parse step.

## Options considered

- **Option A — Hybrid: file-level invalidation for parsing, full recompute for graph scores, via the existing mutation API:** Track mtime/size per file; re-parse only changed/new files through `upsert_file`, drop deleted ones through `remove_file`, call `rebuild_graph` once, then fully recompute PageRank/co-changes/health from the updated graph. Built on the existing mutation API rather than a new `build()` parameter. Pros: skips re-parsing unchanged files; graph algorithms run in milliseconds even for 10K files; no second-generation memory overlap; no stale-content risk; reuses existing API. Cons: requires adding mtime to `IndexedFile`; graph scores are recomputed in full even for a one-file change. Someone could prefer it for its correctness guarantees and minimal new surface area.

- **Option B — Fully incremental graph algorithms:** A reasonable alternative would have been to incrementally update PageRank, coupling, and blast radius as graph edges change. Pros: avoids recomputing graph scores on every re-index. Cons: incremental PageRank and coupling are complex and error-prone, with real risk of divergence from a from-scratch rebuild. Someone could prefer it if graph recompute ever became a bottleneck at very large scale.

## Decision

Use a hybrid incremental strategy: track mtime/size per file, re-parse only changed files through the existing mutation API (`upsert_file` / `remove_file`), call `rebuild_graph` once, then fully recompute graph-derived scores (PageRank, co-changes, health). Implemented on the existing mutation API rather than a new `build()` parameter, which eliminates second-generation memory overlap and stale-content risk. Requires adding mtime to `IndexedFile` (shipped as the `mtime_secs` field).

## Consequences

### Positive
- Skips re-parsing unchanged files — the real bottleneck.
- Graph recompute is cheap (milliseconds for 10K files).
- Reuses the existing mutation API; no stale content.

### Negative
- mtime must be stored per file on `IndexedFile` (as `mtime_secs`), with current mtime read from disk at rebuild time.
- Graph scores are recomputed in full even for a single-file change.

### Neutral
- The incremental path (`incremental_rebuild()`) sits alongside the full `build()` path; both share the same mutation primitives.

## Revisit if
- Graph recompute stops being cheap at very large scale.
- Incremental graph algorithms become worth their added complexity.

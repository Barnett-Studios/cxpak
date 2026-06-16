---
id: '0055'
title: Incremental in-memory index and graph updates (upsert/remove)
status: ACCEPTED
date: 2026-03-13
triggered_by: Daemon needs to keep the index hot across file changes without full rebuilds
loop: implementation
---

# ADR-0055: Incremental in-memory index and graph updates (upsert/remove)

## Context

As of v0.8.0, every cxpak invocation cold-started the full pipeline (scan, parse, index, build graph), taking 2-5 seconds for a 1000-file repo. Daemon mode (watch/serve) needs to turn that cost into a 10-50ms incremental update by mutating a single file's entry in place rather than rebuilding everything.

The blocker was that `CodebaseIndex::build()` was batch-only: it constructed the whole index from a complete file set with no path for updating one file. The dependency graph had no way to surgically remove a single source file's edges either, so a re-parse would have leaked stale edges.

## Options considered

- **Option A — In-place `upsert_file` / `remove_file` + `remove_edges_for`:** Add `CodebaseIndex::upsert_file()` and `remove_file()` that adjust `files`, totals, and `language_stats` incrementally (removing the old entry first, then inserting), plus `DependencyGraph::remove_edges_for()` that strips a file's outgoing edges and cleans the corresponding reverse edges before new edges are re-added. Pros: turns the 2-5s cold start into a 10-50ms incremental update, reuses `build_with_content` for the initial hot build, surgical graph edits. Cons: incremental stat bookkeeping (saturating subtraction) must stay correct, and there are more invariants to maintain than in a clean batch build.

- **Option B — Full rebuild on every change:** Re-run scan/parse/index/graph whenever any file changes. Pros: no incremental bookkeeping, always consistent by construction — someone could prefer this for the guaranteed correctness and simpler code. Cons: 2-5s per change is unacceptable for interactive/IDE use, which is the entire point of daemon mode.

## Decision

Add incremental mutation methods. `CodebaseIndex::upsert_file()` recomputes `total_tokens`, `total_bytes`, and per-language `language_stats` in place by removing the old entry first and then inserting the new one. `CodebaseIndex::remove_file()` adjusts the same stats using saturating subtraction to avoid underflow. `DependencyGraph::remove_edges_for()` removes a source file's outgoing edges and prunes the corresponding reverse edges before new edges are re-added on re-parse, keeping the forward and reverse maps consistent.

The daemon re-parses only the changed file and updates the in-memory index/graph entry through these methods. The batch `build()` path is retained alongside the incremental methods, and `build_with_content` is reused for the initial hot build.

## Consequences

### Positive
- Turns the 2-5s cold start into a 10-50ms incremental update.
- `remove_edges_for` keeps the forward and reverse edge maps consistent on re-parse.
- Provides the foundation for both `watch` and `serve` modes.

### Negative
- Incremental stat and edge bookkeeping is more error-prone than a clean rebuild; correctness now depends on maintaining invariants by hand.

### Neutral
- The batch `build()` path is retained alongside the incremental methods.
- Stats use saturating subtraction to avoid underflow.

## Revisit if
- Incremental stats drift from a full-rebuild baseline.
- Cross-file edge changes need recomputation beyond the single changed file (i.e. 1-hop re-parse proves insufficient).

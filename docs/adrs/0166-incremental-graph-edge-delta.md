---
id: '0166'
title: Incremental dependency-graph updates as an edge-delta extension, with the full rebuild as parity oracle
status: ACCEPTED
date: 2026-06-14
triggered_by: v2.3.0 W1 — incremental_rebuild re-parses only changed files but then rebuilds the whole graph
loop: planning
---

# ADR-0166: Incremental dependency-graph updates as an edge-delta extension

## Context

`CodebaseIndex::incremental_rebuild` (index/mod.rs:458) already re-parses only the files whose mtime/size changed, then calls `rebuild_graph()` (index/mod.rs:436), which rebuilds the **entire** dependency graph via `build_dependency_graph(&self.files, schema)`, rebuilds the call graph, and re-injects all cross-language edges (index/mod.rs:441-447). So graph reconstruction is O(repo) on every change, even a one-line edit — the dominant incremental cost alongside PageRank.

## Options considered

- **Option A — edge-delta extension, full path kept as oracle (chosen):** add `rebuild_graph_delta(changed, removed)` that (1) drops forward+reverse edges originating from `changed ∪ removed`, (2) recomputes outgoing edges for `changed` files only, (3) prunes inbound edges to `removed` files, (4) re-injects cross-language edges for `changed`. `incremental_rebuild` calls the delta path; the existing full `rebuild_graph` is retained as the cold-build path **and** as the oracle a parity property test checks against. Pros: O(changes); reuses the existing per-file edge extraction; the full path guarantees a ground truth. Cons: careful reverse-edge and cross-language handling; renames must be modeled as delete+add. Chosen because it bounds work to the change while keeping an exact reference.
- **Option B — always full rebuild (status quo):** Pros: simplest, exact, no drift risk. Cons: O(repo) per change; the very cost we are removing. Someone could prefer it to avoid all delta-correctness risk.
- **Option C — event-sourced graph with a persistent edge journal:** Pros: full history, replayable. Cons: large new subsystem, far beyond the need; violates extend-don't-add. Someone could prefer it for auditability we don't require.

## Decision

Option A, implemented with an explicit exactness boundary discovered during build. `rebuild_graph_delta(changed, removed)` takes the per-file delta path **only when the change cannot ripple into unchanged files' edges**; otherwise it falls back to the full `rebuild_graph` (still cheaper than the re-parse the caller already did). The boundary:

- **Pure content modifications, no data layer** → exact per-file delta: drop each changed file's *outgoing* edges (preserving the reverse edges *into* it, which belong to unchanged files), recompute its outgoing edges via the shared `edges_for_file` helper, and re-inject its cross-language edges.
- **Any structural change** (a removal, or a `changed` path not yet a graph node — i.e. an addition) → full rebuild. The subtle reason: adding/removing a path changes the file universe, so an *unchanged* file's import could newly resolve to, or stop resolving to, the added/removed path — a ripple a local per-file delta cannot observe.
- **Schema present** → full rebuild. FK / view / function / ORM / migration / embedded-SQL edges derive from the `SchemaIndex` and content scans, not a single file's imports.

`incremental_rebuild` calls the delta; the full `rebuild_graph` is the cold path and the parity oracle. "Done" means the delta equals the full rebuild on a fuzzed modify/remove sequence (proptest, 1000 cases) plus a schema-fallback case. The single source of truth keeping both paths equal by construction is `edges_for_file`, factored out of `build_dependency_graph`'s loop and used by both.

## Consequences

### Positive
- Graph updates scale with change size for the common live-edit case (content modifications), not repo size.
- The retained full path is both the fallback and the test oracle; the conservative fallback makes correctness the default — when in doubt, rebuild.

### Negative
- The delta only accelerates content modifications. Additions, removals, and any repo with a detected data layer take the full path. (Pure modifications dominate the `serve`/`watch`/LSP edit case, so the live-speed win still lands.)
- Reverse edges into a changed file must be preserved while edges *out of* it are recomputed — handled by removing only outgoing edges.

### Neutral
- `incremental_rebuild` still recomputes `call_graph` and `test_map` over the whole repo for consistency; only graph edges and PageRank are true deltas. Per-file deltas for those are a tracked follow-up (benches/BASELINES.md).
- Renames present as delete + add → full rebuild.

## Revisit if
- `call_graph` / `test_map` full recomputes become the incremental bottleneck (the benchmark shows they dominate per-edit cost) — give them per-file delta paths.
- The structural-change full-rebuild fallback proves too coarse under heavy file churn — model add/remove as a bounded re-resolution of just the importers of the affected paths instead of a full rebuild.
- The parity suite proves the delta cannot be kept equivalent at acceptable complexity — fall back to Option B for graph (keeping warm PageRank).

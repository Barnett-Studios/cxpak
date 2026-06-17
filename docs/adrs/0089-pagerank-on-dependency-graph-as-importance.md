---
id: '0089'
title: Use standard PageRank over the typed dependency graph for file importance, computed at build time
status: ACCEPTED
date: 2026-03-22
triggered_by: v0.13.0 intelligence — need a deterministic, zero-config measure of structural importance for ranking and degradation
loop: planning
---

# ADR-0089: Use standard PageRank over the typed dependency graph for file importance

## Context

v0.13.0 introduces graph-based intelligence and needs a deterministic, zero-config measure of structural importance to drive relevance ranking and degradation priority. The design adds file-level importance via standard PageRank (damping 0.85, max 100 iterations, 1e-6 convergence, forward edges, normalized 0–1) over the typed dependency graph, with all edge types contributing rank equally.

A hybrid file-level-PageRank + symbol-weight-heuristic approach is chosen because a true symbol-level call graph does not exist in cxpak, and building one would be a large undertaking.

## Options considered

- **Option A — File-level PageRank + symbol-weight heuristic:** Run PageRank on the file graph; derive symbol importance as `file_pagerank × symbol_weight` where the weight is 1.0 (public + referenced), 0.7 (public), or 0.3 (private). Pros: deterministic, zero config, reuses the existing typed graph, and avoids the massive undertaking of a symbol-level call graph. Cons: symbol importance is a heuristic, not derived from a real call graph. This was the chosen option.
- **Option B — Pure symbol-level PageRank:** Build a symbol call graph and rank symbols directly. Pros: more precise importance. Cons: the call graph does not exist, and building one is a major undertaking. This was the explicitly rejected alternative.
- **Option C — Simple in-degree / reference counting:** Rank files by number of importers. A reasonable alternative would have been to count importers directly, which is trivial to compute; it was not formally evaluated. It ignores the transitive importance that PageRank captures.

## Decision

Implement `compute_pagerank(graph, 0.85, 100)` with a 1e-6 convergence threshold, forward-edge traversal (rank flows from importer to imported), dangling-node redistribution, and 0–1 normalization. All edge types transfer rank equally. Cache the result on `CodebaseIndex.pagerank` at build time. Within-file symbol importance is `file_pagerank × symbol_weight` (1.0 / 0.7 / 0.3), using an inverted symbol-name→files cross-reference index built once from `term_frequencies`.

## Consequences

### Positive
- Deterministic, zero-config structural importance.
- Reuses the v0.12.0 typed graph; computed once and cached.
- O(1) cross-reference lookups via the inverted index.

### Negative
- Symbol importance is a heuristic multiplier, not a true call-graph rank.
- Treating all edge types equally may over- or under-weight schema edges.

### Neutral
- PageRank later feeds relevance signal #6 (weight 0.17) and degradation priority (×0.2 in the 0.6/0.2/0.2 formula).

## Revisit if
- A real symbol-level call graph becomes available.
- Equal edge weighting distorts importance for schema-heavy repos.

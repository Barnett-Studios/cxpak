---
id: '0015'
title: trace command walks the dependency/call graph with budget-controlled hop depth and ambiguity handling
status: ACCEPTED
date: 2026-03-05
triggered_by: Need a second mode that packs context around a specific symbol/error rather than the whole repo
loop: planning
---

# ADR-0015: trace command walks the dependency/call graph with budget-controlled hop depth and ambiguity handling

## Context

Beyond whole-repo overview, users want focused context around a specific function,
`file:line`, or error string. This decision was taken during initial design (v0.1.0). The
design defines a `trace` mode: resolve the target, walk the call graph outward, and let the
token budget control how many hops are included — target in full, direct dependencies as full
bodies, transitive dependencies as signatures-only. Ambiguous targets are handled via an
`--all` flag (split the budget across matches) or by defaulting to the first match with the
others listed on stderr.

## Options considered

- **Option A — budget-controlled graph walk with priority packing:** Resolve the target, BFS
  the call graph, and pack target full / direct deps full / transitive signatures, with depth
  governed by the available budget. Pros: focused, dependency-aware context where depth scales
  with available tokens. Cons: requires a real dependency/call graph and target-resolution
  heuristics. This was the chosen option and shipped.

- **Option B — flat grep + surrounding-lines window:** A reasonable alternative would have
  been to find the target text and include N lines of surrounding context. Pros: trivial to
  implement. Cons: no dependency awareness — it misses callers, callees, and type context that
  make focused context useful. Not formally evaluated.

## Decision

Implement `trace <target>` to resolve a function, `file:line`, or error string, walk the
dependency graph outward with hop depth governed by the token budget (target in full, direct
dependencies as full bodies, transitive dependencies as signatures-only), and handle ambiguity
via `--all` (budget split across matches) or default-to-first-match with the others printed on
stderr.

## Consequences

### Positive
- Delivers focused, dependency-aware context for a specific symbol or error.
- Depth automatically scales with the available token budget.

### Negative
- Originally deferred to v2 and stubbed in the implementation plan (`trace::run` exited with
  "not yet implemented"), so this design decision predated a working command.

### Neutral
- Shipped as a 1-hop default with full BFS via `--all`; non-import edges are rendered with
  `(via: edge_type)`; target resolution falls back from `find_symbol()` to content matches.

## Revisit if
- Graph-walk relevance proves worse than semantic/embedding-based retrieval for trace targets.

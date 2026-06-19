---
id: '0165'
title: Warm-started PageRank via an optional seed vector, converging to the same threshold
status: ACCEPTED
date: 2026-06-14
triggered_by: v2.3.0 W1 — incremental updates recompute PageRank from a uniform start every time
loop: planning
---

# ADR-0165: Warm-started PageRank via an optional seed vector

## Context

`intelligence::pagerank::compute_pagerank(graph, damping, max_iterations)` initializes every node's rank to `1/N` (pagerank.rs:81), runs power iteration up to `max_iterations`, and early-exits at a convergence threshold of `1e-6` (pagerank.rs:88). On every incremental update the watch/serve path recomputes PageRank from that uniform start with `max_iterations = 100`. After a small mutation the previous score vector is already an excellent approximation of the new fixed point, so starting from `1/N` wastes iterations.

## Options considered

- **Option A — optional seed vector, same convergence gate (chosen):** add `initial: Option<&HashMap<String,f64>>`. When `Some`, seed `rank` from the prior scores (carry forward existing nodes; `1/N` for new nodes) and iterate **until the same `1e-6` threshold**, not a fixed lower cap. Pros: provably the same fixed point (power iteration converges from any positive start), far fewer iterations after small changes, backward-compatible (`None` = today's behavior). Cons: must retain prior scores across updates. A reasonable default someone would pick for minimal change + provable equivalence.
- **Option B — incremental/local-push PageRank:** algorithms (e.g. Andersen local push) that update ranks only near changed nodes. Pros: sublinear in theory. Cons: approximate, complex, and a different algorithm to validate against the exact one. Someone could prefer it at extreme scale.
- **Option C — keep full recompute:** status quo. Pros: simplest, exact. Cons: O(iterations × edges) per change regardless of change size. Someone could prefer it for zero new code.

## Decision

Option A. Add the optional seed; warm runs iterate to the existing `1e-6` convergence threshold. The threshold — not the iteration count — is the correctness contract.

## Consequences

### Positive
- After small changes, convergence in a handful of iterations instead of up to 100, with the same result.
- Backward compatible; cold callers pass `None`.

### Negative
- The caller must retain the previous score vector (already available on the in-memory/persisted index).
- Two near-tied nodes (scores within the convergence threshold) could order differently between warm and cold; immaterial to ranking but means the parity test asserts per-node agreement within a small epsilon and identical *top-K* order, not bitwise-identical order of sub-threshold ties.

## Revisit if
- A mutation is large enough that the warm start no longer converges faster than cold (massive structural change) — detect and fall back to a cold recompute.
- Profiling shows PageRank is no longer the incremental bottleneck (then Option B's complexity is unjustified) or that it still is at extreme scale (then Option B becomes worth it).

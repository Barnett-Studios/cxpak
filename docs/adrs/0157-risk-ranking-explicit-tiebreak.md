---
id: '0157'
title: compute_risk_ranking gets an explicit path-ascending secondary sort key
status: ACCEPTED
date: 2026-04-17
triggered_by: Cross-channel risk consistency requires stable ordering for equal-score entries
loop: planning
---

# ADR-0157: compute_risk_ranking gets an explicit path-ascending secondary sort key

## Context
In v2.1.0, `compute_risk_ranking` (`src/intelligence/risk.rs`) sorted only by `risk_score` descending. Entries with equal scores were ordered by whatever input order happened to feed the sort. That emergent ordering is brittle to unrelated refactors and breaks the deterministic cross-channel comparison the SPA, `/v1`, and MCP surfaces all depend on: two entries with the same risk inputs could reorder when something elsewhere reshuffled the input sequence.

## Options considered
- **Option A — add `a.path.cmp(&b.path)` secondary key:** sort by `risk_score` descending, then by `path` ascending, resolving `partial_cmp` with `unwrap_or(Equal)`. Pros: deterministic, refactor-stable ordering; enables byte-stable cross-channel tests. Cons: none material; a tiny behavior change for equal-score display order. Someone could still prefer the status quo if they did not value cross-channel determinism.
- **Option B — leave ordering emergent from input order:** keep the single-key sort. Pros: no code change. Cons: brittle — ties reorder under unrelated refactors and break the determinism contract. The only reason to prefer this is to avoid touching the comparator at all.

## Decision
Update the comparator in `compute_risk_ranking` to break ties by `path` ascending:

```rust
b.risk_score
    .partial_cmp(&a.risk_score)
    .unwrap_or(std::cmp::Ordering::Equal)
    .then_with(|| a.path.cmp(&b.path))
```

A dedicated regression test (`risk_ranking_ties_break_by_path_ascending`) builds two files with identical risk inputs and asserts they produce identical `f64` score bits and then sort lexicographically by path. This also requires removing the `>= 0.05` dashboard filter so `top_risks` equals the first five of the unfiltered ranking; the separate `risk_score > 0.8` alerts filter is unchanged.

## Consequences
### Positive
- Deterministic risk ordering across the SPA, `/v1`, and MCP channels; resilient to unrelated refactors.
### Negative
- Equal-score risk display order may differ from the prior emergent order.
### Neutral
- Requires removing the `>= 0.05` dashboard filter so `top_risks` is the first five of the unfiltered ranking.

## Revisit if
- A different tertiary tie-break becomes necessary (e.g. equal paths require a further key).

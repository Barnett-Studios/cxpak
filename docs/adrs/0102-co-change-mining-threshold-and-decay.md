---
id: '0102'
title: Co-change edges mined from git, recency-decayed, piggybacking the git_health walk (>=3-commit threshold designed but unwired)
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.2.0 co-change analysis to power historical impact signals
loop: planning
---

# ADR-0102: Co-change edges mined from git, recency-decayed, piggybacking the git_health walk

## Context
v1.2.0 added co-change analysis: files that change together historically signal hidden
coupling, feeding the historical impact signal. A single co-occurrence is noise, and old
co-changes are weaker signals of current relevance, so the design doc specified "files
appearing in ≥3 commits together within 180 days. Threshold of ≥3 co-commits filters
noise (single co-occurrence is not a pattern)," with the data "computed during index
construction (piggybacking on the existing `git_health` git walk) and stored on
`CodebaseIndex` as `pub co_changes: Vec<CoChangeEdge>`."

This is a human decision because it sets noise-filtering and recency-decay constants and
chooses to reuse an existing git walk rather than add a pass — trade-offs the code cannot
infer. Shipped in `src/intelligence/co_change.rs` and `src/conventions/git_health.rs`,
stored on `src/index/mod.rs`.

Note on what shipped: the ≥3-commit threshold was designed and a threshold-enforcing
builder (`build_co_change_edges_with_dates`) was implemented and unit-tested, but it is
**not** wired into the production index pipeline. The pipeline calls `build_co_changes()`
(no threshold), so the stored `CodebaseIndex.co_changes` retains every pair co-appearing
at least once within the 180-day window. The recency decay and piggybacking did ship.

## Options considered
- **Option A — ≥3 co-commits in 180d, recency-weighted, computed during the git_health
  walk:** mine git log, keep file pairs co-occurring in ≥3 commits within 180 days;
  per-commit weight 1.0 for ≤30 days decaying to ~0.4 at 180 days; an edge's
  `recency_weight` is the weight of the most recent co-commit; stored on
  `CodebaseIndex.co_changes`. Pros: the threshold filters noise; decay favors current
  relevance; reuses the existing git_health walk so there is no extra git pass. Cons:
  fixed 3-commit / 180-day thresholds are not tunable; pairwise co-change is
  O(commit_size²) per commit. Someone could prefer this for noise-free, recency-aware
  edges at no extra git cost. (Chosen as designed — see note: the threshold did not
  ship as wired.)
- **Option B — any co-occurrence, unweighted:** record every file pair that ever changed
  together with a raw count. Pros: simplest. Cons: single co-occurrences flood the graph
  with noise; stale pairs are weighted equally with fresh ones. Someone could prefer it
  for simplicity. (The shipped storage behavior, lacking the wired threshold, sits
  between A and B: it keeps count≥1 edges but still applies the recency decay.)

## Decision
Mine co-change edges from git log over a 180-day window, piggybacking the existing
`git_health` git walk (no separate git pass), and store them on `CodebaseIndex` as
`pub co_changes: Vec<CoChangeEdge>`. Per-commit weight is 1.0 for `days_ago ≤ 30` and
`1.0 - 0.7 × (days_ago - 30) / 150` for `30 < days_ago ≤ 180`; commits older than 180
days are excluded. An edge's `recency_weight` is the weight of the most recent co-commit
(not the average). The ≥3-commit noise threshold was designed and a threshold-enforcing
builder exists and is unit-tested, but the production path (`git_health.rs` →
`build_co_changes` → `CodebaseIndex.co_changes` → `historical_impact`) applies **no**
count filter; the stored edges include count=1 and count=2 pairs.

## Consequences
### Positive
- Recency decay keeps the signal current.
- No additional git walk — reuses the git_health traversal.
- A ≥3-commit threshold builder exists for callers that want noise filtering.

### Negative
- The decay constants and the (unwired) threshold are hard-coded.
- Pairwise enumeration cost grows with large commits.
- The designed ≥3-commit noise threshold did not ship in the index pipeline, so stored
  edges include single- and double-occurrence pairs that the design intended to filter.

### Neutral
- The threshold-enforcing builder (`build_co_change_edges_with_dates`) is implemented and
  tested but unused by the index/auto_context/git_health pipeline.

## Revisit if
- The threshold or decay constants need to become configurable.
- Large commits make pairwise enumeration too expensive.
- The gap between the designed ≥3-commit threshold and the unwired shipped behavior
  causes noise problems that justify wiring the threshold builder into the pipeline.

---
id: '0042'
title: Graph-and-git-based file importance ranking for budget allocation
status: ACCEPTED
date: 2026-03-12
triggered_by: Budget allocation treated all files equally, wasting tokens on low-value helpers and truncating central orchestrators
loop: planning
---

# ADR-0042: Graph-and-git-based file importance ranking for budget allocation

## Context

Before v0.6.0, budget allocation treated all files equally — wasting tokens on low-value helpers while truncating central orchestrators. The v0.6.0 design (Workstream 1: Smart Context) introduces a new `src/index/ranking.rs` module that scores every file by combining signals already available in the `DependencyGraph` and git history into a single composite weight, then orders files so high-importance files get budget first. Confirmed shipped: `src/index/ranking.rs` exists with the documented weights.

## Options considered

- **Option A — Composite weighted score (`in_degree*0.4 + out_degree*0.1 + git_recency*0.3 + git_churn*0.2`):** A linear weighted combination of graph degree and git signals into one composite per file. Pro: reuses data already in the dependency graph and git context, needs no new budget algorithm (just weighted input ordering), and is no breaking change to output format. Con: hand-tuned weights without empirical validation, and `out_degree` contributes little at 0.1. Someone could prefer this for shipping a useful first iteration cheaply. (Considered and chosen.)
- **Option B — Keep equal budget per file:** The status quo — every file gets the same token allocation. Pro: simple, already implemented. Con: wastes tokens on utility helpers and truncates foundational files — the exact problem being solved. Someone could prefer it to avoid heuristic weighting. (Considered.)
- **Option C — PageRank over the dependency graph:** A reasonable alternative would have been iterative eigenvector centrality instead of raw in/out degree. Pro: captures transitive importance, not just direct dependents. Con: more compute and overkill for a first iteration. Someone could prefer it for accuracy; the intelligence pillar later did add PageRank.

## Decision

Add `src/index/ranking.rs` exposing `FileScore { path, in_degree, out_degree, git_recency, git_churn, composite }` and `rank_files()`. The composite weight is `in_degree*0.4 + out_degree*0.1 + git_recency*0.3 + git_churn*0.2`.

Ranking does not change the budget algorithm: `allocate()` still takes only a total budget. Instead, the index's file list is sorted by descending composite score before rendering, so high-importance files are budgeted and rendered first and survive truncation. The median / signatures-only / bottom-10% tiering described in the design doc was not implemented.

## Consequences

### Positive
- Important files (high in-degree, recently changed hotspots) get token budget before truncation.
- No breaking change — content is reordered/weighted, not restructured.
- Reuses existing `DependencyGraph` and `GitContext` data.

### Negative
- Weights are heuristic and unvalidated.
- `git_recency` was implemented as a binary always-1.0 noise signal in v0.6.0 (fixed in v0.6.1).

### Neutral
- The module was later extended with PageRank in the broader intelligence pillar (per CLAUDE.md).

## Revisit if
- Weights prove miscalibrated on real repos.
- A more sophisticated centrality measure is needed.

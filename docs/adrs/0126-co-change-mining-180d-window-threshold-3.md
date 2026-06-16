---
id: '0126'
title: 'Git co-change mining: 180-day window, >=3 co-commit threshold, linear recency decay to 0.3'
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.2.0 introducing co-change analysis as a historical-coupling signal stored on CodebaseIndex
loop: implementation
---

# ADR-0126: Git co-change mining: 180-day window, >=3 co-commit threshold, linear recency decay to 0.3

## Context

In v1.2.0, to capture files that change together, the team mined git history. They needed a window to bound the revwalk, a noise threshold so incidental pairs are excluded, and a recency weighting so old coupling counts less. The signal is stored on `CodebaseIndex` as a historical-coupling input.

## Options considered

- **Option A — 180-day window, threshold ≥3 co-commits, linear decay 1.0 (≤30d) to 0.3 (180d):** Pairs co-appearing in at least 3 commits within 180 days become edges; recency weight uses the most recent co-commit — flat 1.0 to 30 days, then linearly down to 0.3 at 180 days via `1.0 - 0.7*(days-30)/150`. Pros: bounds the git walk, filters incidental co-changes, recency weight rewards current coupling. Cons: all three constants (180, 3, decay slope) are heuristic. This is the chosen approach.
- **Option B — unbounded history with no threshold:** A reasonable alternative would have been to count every co-occurrence over the full repo history. Pros: maximum data. Cons: slow, noisy, and stale coupling dominates. Someone could prefer it for a young repo with little history.
- **Option C — average recency weight across all co-commits:** Weighting by mean age rather than most-recent was considered and rejected. Pros: smoother. Cons: the design spec explicitly chose the most-recent co-commit ("not the average, per the design spec"). Someone could prefer the average to avoid over-weighting a single recent edit.

## Decision

Mine co-change edges from the git revwalk over a 180-day window (`src/intelligence/co_change.rs`). A file pair becomes a `CoChangeEdge` when it co-occurs in at least 3 commits (threshold applied by the edge-builder wrappers; the git-health path emits all pairs and defers threshold filtering to the caller). `recency_weight` is derived from the most recent co-commit: 1.0 for ≤30 days, then linearly decaying to 0.3 at 180 days (`1.0 - 0.7*(days-30)/150`); commits older than 180 days are excluded. Co-change mining piggybacks on the existing git2 revwalk in `extract_git_health` (no second pass).

## Consequences

### Positive
- Bounded, deterministic git walk reusing the existing revwalk.
- The threshold of 3 removes incidental co-changes.
- Recency decay keeps the signal current.

### Negative
- Window / threshold / decay constants are unvalidated heuristics.
- Most-recent weighting can over-reward a single recent co-edit of an otherwise-stale pair.

### Neutral
- v1.4.0 re-implemented `co_change.rs` with the same 180d / threshold-3 / decay semantics but a configurable threshold and a date-aware `mine_co_changes_from_git`, populating `CodebaseIndex.co_changes` from `build_index` in `serve.rs`.

## Revisit if
- The 180d window or threshold-3 proves wrong for fast- or slow-moving repos.
- The recency decay slope needs tuning.

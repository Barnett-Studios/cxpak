---
id: '0098'
title: Git health uses two churn windows (30d and 180d) rather than one
status: ACCEPTED
date: 2026-03-27
triggered_by: Designing git_health churn metrics
loop: planning
---

# ADR-0098: Git health uses two churn windows (30d and 180d) rather than one

## Context
The v1.1.0 `git_health` profile needed a churn metric. A single churn count cannot
distinguish a file that is freshly hot from one that has been chronically churning
versus one that has stabilized versus one that is cold. The design doc resolved this
with two windows: "Two git windows (30d + 180d) ... Two dimensions reveal trend:
hot/stabilizing/chronic/cold."

This is a human decision because it trades a small amount of bookkeeping for a
qualitatively richer signal — the choice of how many windows and how to combine them
defines what the churn metric can express, and the code cannot infer that intent.
Shipped in `src/conventions/git_health.rs`.

## Options considered
- **Option A — two windows, 30-day and 180-day:** count modifications per file in both
  windows and compare them to derive a four-way churn trend (hot/stabilized/chronic/
  cold). Pros: two dimensions reveal trend; distinguishes newly-hot from chronically-
  churning from stabilized from cold. Cons: more per-file bookkeeping and counter
  logic. Someone could prefer this for the trend classification it enables. (Chosen.)
- **Option B — single churn window:** one window (e.g., 30 days) of modification
  counts. A reasonable alternative would have been the simplest metric. Pros: simpler.
  Cons: cannot express a trend — a hot file and a stabilizing file look identical.
  Someone could prefer it purely to avoid the extra bookkeeping.

## Decision
Track churn over two windows, 30-day and 180-day, and compute a churn trend
(hot/stabilized/chronic/cold) by comparing them. Implemented in
`src/conventions/git_health.rs`: a single git2 revwalk bounded at the 180-day cutoff
increments both the 30-day and 180-day counters inline, and `classify_trend()`
derives the four-way trend per file.

## Consequences
### Positive
- Trend classification distinguishes hot/stabilized/chronic/cold.
- Richer git-health signal for risk and DNA rendering.

### Negative
- More per-file counter bookkeeping and the trend-classification logic.

### Neutral
- Both counters are derived from one revwalk that breaks at the 180-day boundary, so
  the two windows do not cost two separate git passes.

## Revisit if
- A third window or configurable windows become necessary.
- The 30/180-day boundaries prove poorly calibrated for real repositories.

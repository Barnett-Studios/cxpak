---
id: '0125'
title: Change-impact prediction merging three signals into seven discrete confidence levels
status: ACCEPTED
date: 2026-04-01
triggered_by: "v1.4.0 'Prediction': predicting which tests/files a change affects before it is made"
loop: implementation
---

# ADR-0125: Change-impact prediction merging three signals into seven discrete confidence levels

## Context

In v1.4.0 ("Prediction"), given a set of changed files the tool predicts affected files and tests by combining structural (blast radius), historical (co-change), and call-based signals. The design needed a principled way to turn the presence/absence of each signal into a single confidence number.

## Options considered

- **Option A — discrete confidence lookup over the 7 non-empty signal subsets:** Map each combination of `{test_map/structural, co_change, call_graph}` to a fixed confidence. As shipped (`confidence_for_signals`, `src/intelligence/predict.rs`), the seven subsets map to seven distinct values: co_change-only 0.40, call_graph-only 0.50, test_map-only 0.60, call_graph+co_change 0.70, test_map+co_change 0.75, test_map+call_graph 0.85, all three 0.90. Pros: interpretable, deterministic, easy to reason about; encodes that call-graph and corroborating evidence raise confidence. Cons: confidence values are hand-assigned, not learned from observed outcomes. This is the chosen approach. (Note: the cited design doc documents a *proposed* table with different, partly-colliding values; the shipped table differs and has no shared value.)
- **Option B — weighted continuous score from the three signal magnitudes:** A reasonable alternative would have been `confidence = w1*structural + w2*historical + w3*call`. Pros: uses signal strength, not just presence. Cons: less interpretable; the team chose a presence-based discrete table. Someone could prefer it to exploit signal magnitude.

## Decision

Implement `predict()` (`src/intelligence/predict.rs`) combining structural impact (BFS on the reverse dependency graph via blast radius), historical impact (co-change edges scored by count/max × recency_weight), and call-based impact (`call_graph_impact`) into `TestPrediction` entries. The `has_map` flag is set when either the TestMap or the Structural signal is present. Confidence is a fixed lookup over the seven non-empty signal subsets, ranging 0.40 (co-change only) to 0.90 (all three), with call-graph-only at 0.50 and test_map-only at 0.60 — seven distinct values, no collisions. `PredictionResult` is returned by the `cxpak_predict` tool and populated on `AutoContextResult.predictions` when the task string names indexed file paths.

## Consequences

### Positive
- Interpretable confidence the LLM can reason about.
- Deterministic and unit-tested across all 7 subsets (`test_confidence_map_values`).
- Auto-populates predictions when a task mentions specific files.

### Negative
- Confidence values are heuristic, not calibrated against real change outcomes.

### Neutral
- The cited design doc documents a proposed confidence table that differs from the shipped values; the implementation (`predict.rs:255-264`) is authoritative. CLAUDE.md and `serve.rs` carry a stale "0.3–0.9" range string; the actual shipped range is 0.40–0.90.

## Revisit if
- Confidence values need calibration from observed test-failure data.
- Adding a fourth signal forces a larger lookup table.

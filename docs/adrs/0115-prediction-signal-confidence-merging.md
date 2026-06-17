---
id: '0115'
title: Change-impact test prediction merges three independent signals into seven confidence tiers
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.4.0 'Prediction' — predict which tests break from a change
loop: planning
---

# ADR-0115: Change-impact test prediction merges three independent signals into seven confidence tiers

## Context

Released in v1.4.0 ("Prediction"). The change-impact predictor must answer "which tests are likely to break given this change?" Three independent signals each nominate affected tests:

- **structural** — blast radius over the dependency graph
- **historical** — git co-change (files that changed together in the past)
- **call-based** — the call graph (tests that transitively call changed code)

A test flagged by more independent signals is more likely genuinely affected, so confidence should rise with corroboration. The output is consumed by an LLM, which benefits from a deterministic ranked list with explicit per-prediction provenance rather than an opaque blended score.

## Options considered

- **Option A — enumerate all non-empty signal subsets with fixed ascending confidences (chosen):** Assign a fixed confidence to each non-empty combination of signals, rising with corroboration. As designed and released in v1.4.0: co-change alone 0.3, test-map alone 0.4, call-graph alone 0.5, test-map+co-change 0.5, call+co-change 0.6, test-map+call 0.7, all three 0.9. Pros: deterministic, explainable provenance per prediction; corroboration raises confidence; produces a ranked list. Cons: the confidence values are hand-tuned constants, not learned from observed test failures.
- **Option B — single combined heuristic score:** A reasonable alternative would have been to blend the three signals into one weighted number. Pros: simpler to implement and tune. Cons: collapses per-signal provenance, so the LLM cannot reason about *why* a test was flagged. Rejected: the loss of provenance outweighs the simplicity.

## Decision

Merge the test-impact signals (test map, call graph, co-change) by enumerating non-empty subsets and assigning each a fixed confidence, so tests corroborated by multiple independent signals rank higher. The LLM receives a deterministic, provenance-tagged ranked list.

The confidence constants stated above are the **v1.4.0 as-released** values. They were subsequently rebalanced in the 2026-04-15 audit-wave-3 fix (commit 5f0acebd). The shipped table in `src/intelligence/predict.rs` (`fn confidence_for_signals`) is now keyed on three booleans `(has_map, has_call, has_hist)` — where `has_map` is `TestMap OR Structural` (test-map and structural blast-radius are equal-weight) — and yields: co-change alone 0.40, test-map/structural alone 0.60, call-graph alone 0.50, map+co-change 0.75, call+co-change 0.70, map+call 0.85, all three 0.90, no signal 0.00.

Note on the signal count: the shipped `ImpactSignal` enum has four variants (`TestMap`, `Structural`, `Historical`, `CallBased`). `TestMap` and `Structural` are collapsed into the single `has_map` dimension, so the confidence table is three-dimensional but driven by four signal sources.

## Consequences

### Positive
- Corroboration across independent signals raises confidence.
- Predictions are deterministic and carry per-prediction provenance the LLM can reason about.

### Negative
- The confidence constants are hand-tuned, not empirically calibrated. The v1.4.0-as-released values were already rebalanced once (audit-wave-3, 2026-04-15), confirming the calibration risk.

### Neutral
- The lattice is fixed by the current signal set; the merge of `TestMap` and `Structural` keeps the table three-dimensional even though four signal sources exist.

## Revisit if
- The hand-tuned confidence constants prove poorly calibrated against real test failures (this has already occurred once, in the 2026-04-15 rebalance).
- A new impact signal is added that does not collapse into an existing dimension, changing the subset lattice.

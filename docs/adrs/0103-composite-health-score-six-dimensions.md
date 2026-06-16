---
id: '0103'
title: Composite health score from six weighted dimensions with renormalization for null dead_code
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.2.0 'Codebase Health' — auto_context becomes a compound intelligence engine
loop: planning
---

# ADR-0103: Composite health score from six weighted dimensions with renormalization for null dead_code

## Context
The v1.2.0 roadmap ("Codebase Health") wants a single legible health number that combines the intelligence primitives cxpak already computes. The score is a weighted average over six dimensions: conventions, test coverage, churn stability, coupling, cycles, and dead code.

One dimension is not yet available. The `dead_code` dimension depends on the call graph, which does not ship until v1.3.0. In v1.2.0 it is `None`, so the composite has to behave correctly with only five live dimensions while keeping the slot for the sixth.

## Options considered
- **Option A — Six weighted dimensions, renormalize when dead_code is null:** weights are `conventions: 0.20, tests: 0.20, churn: 0.15, coupling: 0.20, cycles: 0.15, dead_code: 0.10`. In v1.2.0, with `dead_code = None`, the remaining five weights are renormalized to sum to 1.0 by dividing each by 0.90. Pros: ships a meaningful score before the call graph exists, and the new dimension adds real information when it arrives. Cons: composite scores are not directly comparable across v1.2.0 and v1.3.0 once `dead_code` activates. (Grounded — this is the shipped design.)
- **Option B — Defer the health score until all six dimensions exist:** wait for the v1.3.0 call graph before shipping any health score. A reasonable alternative would have been to ship nothing until the score is stable and comparable from day one. Someone could prefer this to avoid a cross-version comparability break. Rejected because it delays the headline v1.2.0 feature for a dimension worth only 0.10 of the weight. (Reconstructed — not formally evaluated in the source.)

## Decision
Compute the composite health score as a weighted average of six dimensions with weights `conventions 0.20, test_coverage 0.20, churn_stability 0.15, coupling 0.20, cycles 0.15, dead_code 0.10`.

- `cycles` uses `10.0 / (1.0 + scc_count)` — logarithmic in spirit, not clamped to a band.
- `coupling` only counts modules with `>= 3` files: it returns `10.0` if no module qualifies, and `0.0` for a qualifying module that is isolated (no edges).
- When `dead_code` is `None` (v1.2.0, before the call graph lands), the remaining five weights are renormalized to sum to 1.0 by dividing each by `0.90` (e.g. `(0.20 / 0.90) * conventions + ...`).

Document explicitly that scores produced under the five-dimension renormalization are not directly comparable to six-dimension scores produced once `dead_code` activates in v1.3.0.

## Consequences
### Positive
- A single legible health number ships in v1.2.0 instead of waiting on the call graph.
- Adding the `dead_code` dimension in v1.3.0 contributes genuinely new information rather than being a cosmetic addition.

### Negative
- Composite scores shift on upgrade to v1.3.0 and are not directly comparable across versions.
- The coupling-dimension module threshold (`>= 3` files) is a tuning parameter with no first-principles justification.

### Neutral
- The score is metadata over existing primitives; the renormalization branch and the six-dimension branch both live in `compute_composite()` in `src/intelligence/health.rs`, so the v1.2.0 path remains exercised even after v1.3.0 activates the sixth dimension.

## Revisit if
- The dimension weights need recalibration against real-world health outcomes.
- Cross-version comparability of the composite becomes a hard requirement.

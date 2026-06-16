---
id: '0094'
title: Tiered convention strength labels (Convention >=90%, Trend 70-89%, Mixed 50-69%, below 50% unreported)
status: ACCEPTED
date: 2026-03-27
triggered_by: Need to map observed pattern consistency to actionable verify severity
loop: planning
---

# ADR-0094: Tiered convention strength labels (Convention >=90%, Trend 70-89%, Mixed 50-69%, below 50% unreported)

## Context

Released in v1.1.0 (Repository DNA). Patterns are observed at varying consistency across a codebase. The LLM needs to know how strict a given rule is so it can weigh deviations appropriately. A flat "this is a convention" label loses the distinction between a 99% rule (a deviation is almost certainly a bug) and a 55% slight majority (a deviation is a coin flip). Strength must also map onto verify severity in a deterministic, legible way.

## Options considered

- **Option A — Three tiers with hard percentage cutoffs and a 50% floor (chosen):** Convention `>=90%` (verify severity high), Trend `70-89%` (medium), Mixed `50-69%` (low), and patterns below 50% consistency are not reported at all. Pros: maps cleanly onto verify severity; deterministic; gives the LLM rule-strictness context; suppresses sub-majority noise. Cons: hard cutoffs create cliff effects (89.9% vs 90.0%) and the 50% floor discards weak-but-real signals. Someone could prefer this for the clean severity mapping and determinism.
- **Option B — Continuous confidence score:** A reasonable alternative would have been to report the raw percentage as a 0–1 confidence with no discrete labels. Pros: no cliff effects at boundaries. Cons: harder to map to a discrete verify severity; less legible to the LLM than a named tier. Someone could prefer it to avoid boundary discontinuities.

## Decision

Use three discrete strength tiers:

- **Convention** — `>=90%` consistency, verify severity `high`.
- **Trend** — `70–89%`, severity `medium`.
- **Mixed** — `50–69%`, severity `low`.

Patterns below 50% consistency have no dominant pattern and are not reported. `PatternObservation::new()` encodes this: it returns `None` for `total == 0` or `percentage < 50`.

## Consequences

### Positive
- Strength maps 1:1 to verify severity.
- Sub-50% noise is filtered automatically.
- Legible, deterministic classification.

### Negative
- Cliff effects at the 70% and 90% boundaries (e.g., 89.9% vs 90.0% changes the label).

### Neutral
- The `PatternStrength` enum shipped with exactly these three variants (confirmed in `src/conventions/mod.rs`).

## Revisit if
- Cliff effects at tier boundaries cause confusing verify output.
- Sub-50% patterns turn out to carry useful signal worth reporting.

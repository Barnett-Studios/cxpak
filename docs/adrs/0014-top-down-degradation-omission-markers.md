---
id: '0014'
title: Top-down progressive degradation with explicit omission markers
status: ACCEPTED
date: 2026-03-05
triggered_by: When content exceeds the token budget, something must be cut — and the LLM must know what was cut and how to get it
loop: planning
---

# ADR-0014: Top-down progressive degradation with explicit omission markers

## Context

cxpak output frequently exceeds its token budget, so content must be cut. This decision was
taken during initial design (v0.1.0). The design specifies progressive degradation rather
than a hard cut: start with full detail (signatures + bodies), then strip to signatures-only,
then names-only, then omit entirely — prioritizing entry points first, then most-connected
modules, then leaf files. Wherever content is cut, a machine-readable HTML comment marker is
inserted telling the reader the omitted token count and the budget needed to include it.

## Options considered

- **Option A — progressive degradation + visible omission markers:** Strip detail in stages
  and leave a `<!-- ... omitted: ~Nk tokens. Use --tokens Xk+ to include -->` marker at each
  cut. Pros: graceful quality loss instead of a cliff edge, self-documenting truncation, and
  an actionable hint to the reader. Cons: the detail-level machinery and marker logic add
  complexity. This was the chosen option and shipped.

- **Option B — hard truncate at budget with no marker:** A reasonable alternative would have
  been to cut content silently once the budget is hit. Pros: trivial to implement. Cons: the
  consuming LLM cannot tell that content was dropped or how to recover it — defeating the
  point of a self-documenting context bundle. Not formally evaluated.

## Decision

Cut content via top-down progressive degradation (full bodies → signatures → names → omit),
prioritizing entry points, then most-connected modules, then leaf files. Insert an explicit
omission marker (`<!-- [section] omitted: ~Nk tokens. Use --tokens Xk+ to include -->`)
wherever content is cut.

## Consequences

### Positive
- Quality degrades gracefully instead of cliff-edging at the budget boundary.
- Markers make truncation transparent and actionable for the consuming LLM.

### Negative
- Markers consume a small slice of the budget themselves.

### Neutral
- Implemented as `omission_marker()` + `truncate_to_budget()`, originally reserving ~50
  tokens for the marker (later raised, as the in-code comment notes 50 was too small). Later
  generalized into the `context_quality` degradation module (`DetailLevel` Full → Trimmed →
  Documented → Signature → Stub).

## Revisit if
- The marker format needs to be machine-parseable by downstream tooling beyond serving as a
  hint to a human or LLM reader.

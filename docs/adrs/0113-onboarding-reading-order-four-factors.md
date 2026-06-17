---
id: '0113'
title: 'Onboarding map computes reading order from PageRank, topological dependency order, module grouping, and complexity progression'
status: ACCEPTED
date: 2026-03-31
triggered_by: v2.0.0 onboarding map — 'read these files in this order'
loop: planning
---

# ADR-0113: Onboarding map computes reading order from PageRank, topological dependency order, module grouping, and complexity progression

## Context

Introduced in v2.0.0 for the onboarding map ("read these files in this order"). A new-engineer reading order needs to balance importance (read the central files), prerequisite order (read dependencies before dependents), and locality (finish a module before moving on). A difficulty ramp (simpler files first within a module) was also a design goal, but proved incompatible with prerequisite order and was dropped in implementation.

## Options considered

- **Option A — Multi-factor ordering: PageRank, topological sort, module grouping, complexity progression:** Start with highest-PageRank files; order by dependency topological sort; group by module, completing one before the next; within a module, order simpler files first; output phases with rationale and estimated reading time. Pros: balances importance, prerequisites, locality, and (as designed) difficulty in one deterministic order; phased output with reading-time estimates is actionable. Cons: the factors can conflict (the most-important file may depend on less-important ones), requiring a fixed precedence; the within-module complexity sort discarded dependency order and was reverted, so the shipped ordering uses three factors, not four; reading-time estimate is heuristic (tokens/min). Someone could prefer the full four-factor design for the gentler difficulty ramp.

- **Option B — Pure PageRank ranking:** A reasonable alternative would have been to order files solely by importance score. Pros: simplest. Cons: ignores prerequisites (could surface dependents before their dependencies), module locality, and any difficulty ramp. Someone could prefer it for a one-line implementation.

## Decision

Compute onboarding reading order from these factors: (1) PageRank — start with the most important files; (2) dependency order — read dependencies before dependents via topological sort; (3) module grouping — complete one module before the next. A fourth factor, (4) complexity progression — simpler files first within each module — was designed but removed in implementation (commit 46ced99), because sorting a module's files by ascending token count discarded their dependency order; within a module, topological (dependency) order is preserved instead (see `test_group_into_phases_preserves_topo_order_within_module`). Token counts are retained only for the `estimated_tokens` display and the reading-time estimate, not for ordering.

Output an `OnboardingMap` of phases, each with name, module, rationale, and files (path, pagerank, symbols_to_focus_on, estimated_tokens), plus total files and estimated reading time. Canonical logic shipped in `src/intelligence/onboarding.rs` with a Kahn's-algorithm topological sort and 200 tokens/min reading-time estimation.

## Consequences

### Positive
- Reading order balances importance (PageRank), prerequisites (topological sort), and locality (module grouping).
- Phased, rationale-annotated output with reading-time estimates is directly actionable.

### Negative
- Conflicting factors require a fixed precedence ordering.
- The designed difficulty ramp (simpler files first) was dropped to preserve dependency order; reading order no longer reflects per-file complexity.
- Reading-time estimate is heuristic (tokens/min).

### Neutral
- Token/complexity counts still appear in the output (`estimated_tokens`, reading time) but no longer influence ordering.

## Revisit if
- The factor precedence produces orderings users find unintuitive.
- A within-module difficulty ramp can be reintroduced without breaking dependency order.
- Reading-time estimation needs calibration.

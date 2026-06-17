---
id: '0060'
title: Seed selection by score threshold with 1-hop dependency fan-out at a discount
status: ACCEPTED
date: 2026-03-17
triggered_by: Relevance ranking alone misses structurally-connected files that don't lexically match
loop: planning
---

# ADR-0060: Seed selection by score threshold with 1-hop dependency fan-out at a discount

## Context

As of v0.9.0, after scoring, the system must choose which files to surface. Top-scoring files may depend on relevant files that score low lexically. The design needed a way to pull in those structural neighbors without flooding the result set.

## Options considered

- **Option A — Threshold + 1-hop fan-out at 0.7x discount:** Files scoring above 0.3 become seeds; their 1-hop dependency-graph neighbors are added at 0.7x the seed's score; everything is sorted descending and truncated to a limit. Pros: captures structurally relevant neighbors, the discount keeps direct matches ranked above fan-out, and it is bounded by 1 hop. Cons: the threshold and discount are magic numbers, and a single hop may miss deeper relevant chains.

- **Option B — Pure top-N by score, no fan-out:** Return the N highest-scoring files only. A reasonable alternative would have been this for its simplicity and full determinism on score. Cons: misses dependencies that are relevant but lexically dissimilar. (Not formally evaluated; reconstructed here.)

- **Option C — Full transitive dependency closure:** Add all transitive neighbors of the seeds. A reasonable alternative would have been this for maximum structural coverage. Cons: explodes the result set and defeats token budgeting. (Not formally evaluated; reconstructed here.)

## Decision

Implement `select_seeds()`: filter scored files above `SEED_THRESHOLD = 0.3`, build the dependency graph, add each seed's 1-hop neighbors (both directions) at `FANOUT_DISCOUNT = 0.7x` the seed score, keep the highest score on multi-seed neighbors, sort descending, and truncate to the limit. Rationale: 0.3 is low enough to catch tangential files yet high enough to filter noise; 0.7x reflects that dependencies are relevant but less so than direct matches.

## Consequences

### Positive
- Surfaces structurally-connected files that lexical scoring misses.
- The discount preserves a sensible ranking between direct hits and fan-out hits.

### Negative
- The threshold was later changed in shipped code: `SEED_THRESHOLD` dropped from 0.3 to 0.10, indicating the original 0.3 over-filtered in practice (the weighted signal sum rarely exceeds 0.3 for natural-language queries with filler words). `FANOUT_DISCOUNT` stayed at 0.7.

### Neutral
- Bidirectional 1-hop traversal reuses the existing `DependencyGraph`.

## Revisit if
- The threshold continues to need tuning across repos.
- 1-hop proves too shallow for multi-layer architectures.

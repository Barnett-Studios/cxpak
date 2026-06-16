---
id: '0069'
title: 'Order degradation by composite score: 0.7 relevance + 0.3 concept priority'
status: ACCEPTED
date: 2026-03-19
triggered_by: Need a deterministic order in which symbols/files lose detail under budget pressure
loop: planning
---

# ADR-0069: Order degradation by composite score: 0.7 relevance + 0.3 concept priority

## Context

Introduced in the cxpak v0.11.0 context-quality design. Under token-budget pressure, the pipeline progressively reduces per-symbol/per-file detail. It needs a deterministic, explainable order in which content loses detail.

The chosen order is driven by a composite score combining relevance (weight 0.7) and a concept-taxonomy priority (weight 0.3). The concept priority is a 7-tier table mapping `SymbolKind` to a value (Function/Method 1.00 down to Imports 0.14). A file's concept priority is the maximum across its symbols. Lowest composite score degrades first.

## Options considered

- **Option A — `0.7 * relevance + 0.3 * concept_priority`, file priority = max symbol:** Relevance dominates the order; concept type breaks ties; a file's priority is its single highest-kind symbol. Pros: relevance leads but symbol kind disambiguates, so a "function file" survives longer than an "imports file". Cons: the weights and tier values are heuristic, not empirically tuned. (Grounded — this is the design as written and shipped.)

- **Option B — Relevance only:** Degrade purely by relevance score. A reasonable alternative would have been this for simplicity. Cons: ties between equally-relevant files are resolved arbitrarily, and it ignores that a function matters more than imports. (Reconstructed; not formally evaluated.)

- **Option C — File priority = average symbol priority:** Average rather than take the max across a file's symbols. A reasonable alternative would have been this to reflect whole-file composition. Cons: a single key function gets diluted by many imports, mis-ranking the file. (Reconstructed; not formally evaluated.)

## Decision

Use `degradation_priority = relevance_score * 0.7 + concept_priority * 0.3`, with a 7-tier `concept_priority` table keyed on `SymbolKind` and a file's priority taken as the maximum across its symbols. Lowest priority degrades first.

Confirmed shipped via `concept_priority()` and `file_concept_priority()` in `src/context_quality/degradation.rs`. Note: this 0.7/0.3 formula is the backwards-compatible path used when PageRank is unavailable. When PageRank is supplied, the primary path uses `0.6 * pagerank + 0.2 * concept_priority + 0.2 * file_role` (a later enhancement documented in `CLAUDE.md`); the 0.7/0.3 weights remain the shipped fallback.

## Consequences

### Positive
- Deterministic, explainable degradation order.
- Function-heavy files survive longer than config/import-heavy files.

### Negative
- The weights (0.7/0.3) and tier values are hand-picked, not data-derived.

### Neutral
- The max-symbol rule defines a file's character by its single most important symbol.

## Revisit if
- Empirical evaluation suggests different weights.
- A `SymbolKind` is mis-tiered for real workloads.

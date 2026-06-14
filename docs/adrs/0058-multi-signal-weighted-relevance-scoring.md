---
id: '0058'
title: Multi-signal weighted-sum relevance scoring with five deterministic signals
status: ACCEPTED
date: 2026-03-17
triggered_by: Need to rank files by relevance to a natural-language task with no embeddings dependency
loop: planning
---

# ADR-0058: Multi-signal weighted-sum relevance scoring with five deterministic signals

## Context

As of v0.9.0, files must be ranked against a natural-language task query. The design needed a scoring method that is deterministic, cheap, and dependency-free at first, while leaving room for semantic search later.

## Options considered

- **Option A — Weighted sum of five lexical/structural signals:** `SymbolMatch` (0.35), `PathSimilarity` (0.20), `TermFrequency` (0.20), `ImportProximity` (0.15), `RecencyBoost` (0.10), combined as a weighted sum normalized to 0.0-1.0. Pros: deterministic, no external deps, explainable per-signal breakdown, with symbol matching weighted highest as the strongest code signal. Cons: hand-tuned weights, lexical signals miss semantic relevance, and recency is neutral without git history.

- **Option B — Embedding-based semantic similarity:** Score via vector similarity against an embedding model. This was genuinely considered and explicitly deferred as a future swap-in behind the scorer trait. Pros: captures semantic relevance beyond lexical overlap. Cons: requires model download/inference, is non-deterministic, and is a heavier dependency — reasons to defer it past the first shippable version.

- **Option C — Single signal (e.g. BM25 term frequency only):** Rank purely on term-frequency relevance. A reasonable alternative would have been the simplest possible scorer. Cons: ignores structure (symbols, imports, path) and is weaker for code tasks. (Not formally evaluated; reconstructed here.)

## Decision

Adopt a `MultiSignalScorer` combining five weighted signals (`SymbolMatch` 0.35 strongest, `PathSimilarity` 0.20, `TermFrequency` 0.20, `ImportProximity` 0.15, `RecencyBoost` 0.10) as a weighted sum clamped to 0.0-1.0, exposing a per-signal `SignalResult` breakdown. Embedding search was explicitly deferred as a future swap-in behind the `RelevanceScorer` trait.

## Consequences

### Positive
- Deterministic, dependency-free scoring shippable in v0.9.0.
- Per-signal breakdown surfaced to the LLM for transparency.
- Symbol-match weighting matches code-task intuition.

### Negative
- `RecencyBoost` is dead weight (neutral 0.5) without git history.
- Later extended: shipped code ADDED `pagerank` and `embedding_similarity` as 6th and 7th signals (`RecencyBoost` retained but down-weighted to 0.05) and re-tuned weights — `without_embeddings`: sym=0.32, pagerank=0.17, embeddings=0.00; `with_embeddings`: sym=0.27, embeddings=0.15. Both presets sum to 1.0.

### Neutral
- Weights are hand-tuned, not learned.

## Revisit if
- Weights need per-project tuning or learning.
- The embedding signal makes the lexical signals redundant.

---
id: '0091'
title: Filter context noise in three layers: blocklist, Jaccard similarity dedup, and a 0.15 relevance floor
status: ACCEPTED
date: 2026-03-22
triggered_by: Including vendored/generated/duplicate/low-relevance files actively hurts LLM performance
loop: planning
---

# ADR-0091: Filter context noise in three layers: blocklist, Jaccard similarity dedup, and a 0.15 relevance floor

## Context

Released in v1.0.0. The `auto_context` pipeline packs candidate files into a token-budgeted briefing for an LLM. Not all candidates help: vendored and generated files, near-duplicate files, and files whose composite relevance is below the noise floor consume budget while degrading the signal-to-noise ratio the model sees. These three problems are distinct — a generated file is not necessarily a duplicate, a duplicate is not necessarily low-relevance — so a single filter cannot catch all of them.

The constraint is that filtering must be transparent: the LLM (or a downstream `pack_context` override) needs to know what was dropped and why, rather than silently shrinking the candidate set. A further subtlety is exact-match correctness for lock files — matching lock files by substring would wrongly exclude `deadlock.rs`, so lock filenames are matched by exact filename only.

## Options considered

- **Option A — Three-layer filter with transparent `filtered_out` (chosen):** A hardcoded blocklist (path patterns, exact lock filenames, generated-file markers in the first 5 lines), then Jaccard symbol-name similarity dedup at `>0.80` keeping the higher-PageRank file, then a composite relevance floor of `0.15`. Every excluded file is recorded in `filtered_out` with the layer that caught it. Pros: each layer targets a different noise class; the LLM can override via `pack_context`. Cons: the `0.80` and `0.15` thresholds are tuned constants, and the dedup pass is `O(N^2)` (mitigated by a small candidate set). Someone could prefer this for the layered defense and the transparency.
- **Option B — Single relevance-floor filter:** A reasonable alternative would have been to drop only files below a relevance threshold. Pros: simplest possible implementation. Cons: misses vendored/generated files and near-duplicates entirely, since those can score above the floor. Someone could prefer it for minimal code surface if duplicate/generated noise were rare in practice.
- **Option C — Silent exclusion:** Drop noise without reporting it. Pros: smaller response payload. Cons: no transparency for the LLM and no override path, so a wrongly-dropped file is unrecoverable. Someone could prefer it to keep responses compact, but the design explicitly rejected it in favor of `filtered_out` so the LLM can override with `pack_context`.

## Decision

Implement `filter_noise()` as three sequential layers:

1. `is_blocklisted()` over `NOISE_PATH_PATTERNS` plus exact lock filenames, combined with `has_generated_marker()` scanning the first 5 lines for generated-file markers. Lock files match by exact filename so `deadlock.rs` is not caught.
2. `jaccard_symbol_similarity()` dedup over candidate file pairs; when symbol-name overlap exceeds `0.80`, drop the lower-PageRank file.
3. A `DEFAULT_RELEVANCE_FLOOR` of `0.15`.

Every dropped file is recorded in `filtered_out` with a reason naming the layer that caught it.

## Consequences

### Positive
- Layered defense catches distinct noise classes (generated/vendored, near-duplicate, low-relevance) that no single filter would cover.
- `filtered_out` gives the LLM transparency and an override path via `pack_context`.
- Exact-filename lock matching avoids false positives such as `deadlock.rs`.

### Negative
- `0.80` and `0.15` are tuned magic constants without a closed-form derivation.
- The pairwise dedup pass is `O(N^2)`, mitigated only by the small candidate set.

### Neutral
- The `0.15` relevance floor is justified by ICSE 2026 research: below it, files are statistically more noise than signal.
- Jaccard similarity is computed on symbol-name sets only, not file content.

## Revisit if
- Thresholds over- or under-filter on real repositories (e.g., dropping relevant files or admitting obvious noise).
- A noise class is observed slipping through all three layers.

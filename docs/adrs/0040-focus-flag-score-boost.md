---
id: '0040'
title: --focus CLI flag with multiplicative score boost
status: ACCEPTED
date: 2026-03-12
triggered_by: Users need to bias context toward a specific area of the codebase without changing output structure
loop: planning
---

# ADR-0040: --focus CLI flag with multiplicative score boost

## Context

The v0.6.0 design (Workstream 1: Smart Context) adds a `--focus <path>` flag that works with `overview`, `trace`, and `diff`. It lets users bias context toward a specific area of the codebase without altering output structure, implemented as a multiplicative boost over the ranking composite scores. Confirmed shipped: `focus: Option<String>` is present across the CLI command variants in `src/cli/mod.rs`, and `apply_focus()` lives in `src/index/ranking.rs`.

## Options considered

- **Option A — Multiplicative boost (2x focus files, 1.5x direct dependencies):** `apply_focus()` multiplies composite scores for files under the focus path by 2x and their direct graph neighbors by 1.5x. Pro: composes cleanly with the existing ranking score, is one well-defined function, and leaves unrelated files unchanged. Con: the boost factors are arbitrary constants, and only direct (1-hop) dependencies are boosted. Someone could prefer this because it reuses the ranking score as the boost substrate with no new budget machinery. (Considered and chosen.)
- **Option B — Hard filter to focus subtree:** A reasonable alternative would have been to include only files under the focus path and exclude everything else. Pro: maximally concentrated context. Con: loses cross-cutting dependencies entirely — too aggressive for what is fundamentally an importance-ordering feature. Someone could prefer it when they truly want only the subtree.

## Decision

Add `--focus <path>` to `overview`, `trace`, and `diff`. `apply_focus()` multiplies the composite score by 2x for files under the focus path and 1.5x for their direct dependencies; unrelated files are unchanged. Without `--focus`, ranking still improves output silently.

## Consequences

### Positive
- Targeted context steering without restructuring output.
- Reuses the ranking `score_map` as the boost substrate.

### Negative
- v0.6.0 shipped the flag as a dead no-op (params `_`-prefixed) in `trace` and `diff` — only `overview` wired it; fixed in v0.6.1.

### Neutral
- The boost is purely an ordering signal, not a hard filter.

## Revisit if
- Multi-hop dependency boosting is needed.
- The boost constants prove ineffective.

---
id: '0046'
title: Parallelize file parsing with rayon
status: ACCEPTED
date: 2026-03-12
triggered_by: Parsing dominates pipeline time on large repos with a cold cache
loop: planning
---

# ADR-0046: Parallelize file parsing with rayon

## Context

Released in v0.6.0 (Workstream 2 — Speed). Parsing dominates pipeline time on large repos with a cold cache. The workstream adds rayon and converts the parse loop to `par_iter()`, after first establishing measurement via a `--timing` flag. Confirmed shipped: `rayon = "1"` in `Cargo.toml` line 34; `src/cache/parse.rs` uses `par_iter()` with per-thread parsers.

## Options considered

- **Option A — rayon `par_iter()` with per-thread `Parser` instances:** Each rayon worker creates its own tree-sitter `Parser` (parsers are not `Send`); cache lookups happen before parsing. Pros: ~2-4x speedup on large cold-cache repos, minimal code change (`iter`→`par_iter`), cached files skip work entirely. Cons: each thread must build its own parser; minimal benefit on small repos or a warm cache. Someone could prefer it because the per-thread parser pattern sidesteps the `Send` constraint with almost no structural change.
- **Option B — keep sequential parsing:** The status-quo single-threaded parse loop. Pros: no new dependency, deterministic ordering. Cons: leaves multi-core idle on the dominant pipeline stage. Someone could prefer it to avoid a dependency and keep parse order stable.
- **Option C — thread pool with a shared parser via mutex:** A reasonable alternative would have been a single `Parser` guarded by a lock across threads. Pros: fewer parser allocations. Cons: lock contention defeats the parallelism, and tree-sitter `Parser` is not thread-shareable anyway. Someone could naively prefer it to avoid per-thread allocation, but it does not actually parallelize. Not evaluated in the design.

## Decision

Add the rayon dependency and change the parse loop in `src/cache/parse.rs` to `par_iter()`. Each thread instantiates its own tree-sitter `Parser` (parsers are not `Send`); cache lookups precede parsing so cached files skip work. Gated by a measure-first step: add `--timing` first, then parallelize, then re-measure before optimizing other stages.

## Consequences

### Positive
- ~2-4x parse speedup on large repos with a cold cache.
- Measure-first sequencing avoids premature optimization elsewhere.

### Negative
- Per-thread parser allocation overhead.
- Negligible gain on small repos / warm cache.

### Neutral
- `TokenCounter` is read-only after init, so it is shared safely across threads.

## Revisit if
- Profiling shows another stage dominates after rayon lands.
- Parser allocation overhead becomes significant.

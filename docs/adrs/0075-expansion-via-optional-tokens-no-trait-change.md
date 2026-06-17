---
id: '0075'
title: Wire query expansion through optional expanded_tokens without changing the RelevanceScorer trait
status: ACCEPTED
date: 2026-03-20
triggered_by: Expansion must reach the scorer without breaking the existing RelevanceScorer interface or existing call sites
loop: implementation
---

# ADR-0075: Wire query expansion through optional expanded_tokens without changing the RelevanceScorer trait

## Context
v0.11.0 added query expansion (synonym/domain term sets) that must reach the
relevance scorer. To integrate it without churning the public interface,
`tokenize()` was made public and `term_frequency()`/`symbol_match()` gained a
trailing optional `expanded_tokens: Option<&HashSet<String>>` parameter (existing
callers pass `None` for unchanged behavior). `MultiSignalScorer` gained an
`expanded_tokens` field set via a `with_expansion()` builder.

Critically, the `RelevanceScorer` trait method signature was kept unchanged — the
scorer reads expansion from its own field internally, so `score_all()` picks it up
automatically. `context_for_task` builds the scorer with `expand_query(task,
index.domains)`.

## Options considered
- **Option A — optional param plus builder field, trait signature unchanged:** add
  `Option<&HashSet<String>>` to the signal functions and an `expanded_tokens` field
  set via `with_expansion()`; leave the trait method untouched. Pros: backward
  compatible; existing call sites pass `None`; no trait churn. Cons: carries
  expansion as implicit scorer state rather than an explicit argument. Someone could
  prefer it to avoid touching every trait implementor and call site. (Chosen.)
- **Option B — add `expanded_tokens` to the trait method signature:** thread
  expansion explicitly through `RelevanceScorer::score`. A reasonable alternative
  would have been to make the data flow explicit; the implementation plan only
  asserts the trait must *not* change, so this path was rejected implicitly rather
  than weighed in detail. Pros: explicit, no hidden state. Cons: breaks every
  implementor and call site of the trait. Someone could prefer it to avoid implicit
  scorer state.

## Decision
Make `tokenize()` public, add an optional `expanded_tokens` parameter to
`term_frequency()`/`symbol_match()`, and store expansion on `MultiSignalScorer` via
`with_expansion()` while keeping the `RelevanceScorer` trait signature unchanged.
`context_for_task` builds the scorer with `expand_query(task, index.domains)`.

## Consequences
### Positive
- No breaking change to the `RelevanceScorer` trait or existing callers.
- `score_all` automatically uses expansion when present.

### Negative
- Expansion becomes implicit scorer state rather than an explicit argument.

### Neutral
- Existing signal-function call sites pass `None` to preserve behavior.

## Revisit if
- Multiple scorers need different expansion behavior.
- Implicit scorer state causes confusion.

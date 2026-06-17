---
id: '0051'
title: Switch token counting to the o200k_base tokenizer
status: ACCEPTED
date: 2026-03-12
triggered_by: Budget counter used cl100k_base (GPT-3.5/4), producing systematically wrong token counts for Claude/GPT-4o targets
loop: planning
---

# ADR-0051: Switch token counting to the o200k_base tokenizer

## Context
Shipped in v0.6.1 as Bug 2 of the bugfix patch design. The budget counter (src/budget/counter.rs) used `cl100k_base`, the GPT-3.5/GPT-4 tokenizer, which the design doc notes produces "systematically wrong" token counts for the actual target models. Swapping to `o200k_base` aligns counts with the encoding the target models use. Confirmed shipped: counter.rs lines 1 and 16 both reference `o200k_base`, and the swap remains in the current code.

(Per the cited source doc, `o200k_base` is described as "the tokenizer used by Claude and GPT-4o." This record transcribes the source's framing; `o200k_base` is upstream the GPT-4o encoding.)

## Options considered
- **Option A — o200k_base (chosen):** the encoding the design doc targets for the actual models, already bundled in tiktoken-rs 0.6.0. Pros: accurate counts for the target models; no Cargo.toml change since tiktoken-rs already provides it. Cons: token counts shift, so budgets behave slightly differently than before (correctly — the old counts were wrong).
- **Option B — cl100k_base (status quo):** keep the GPT-3.5/GPT-4 tokenizer. Pros: no change. Cons: systematically wrong counts for the target models — the bug being fixed. Rejected because it is the defect.

## Decision
Change `src/budget/counter.rs` to import and initialize `o200k_base()` instead of `cl100k_base()`. tiktoken-rs 0.6.0 already provides it, so there is no dependency change. Existing tests use range assertions, so they keep passing.

## Consequences
### Positive
- Token counts now match the target tokenizer.
- Zero dependency churn (Cargo.toml unchanged: `tiktoken-rs = "0.6"`).

### Negative
- Budget behavior shifts vs v0.6.0 — correctly, since the old counts were wrong.

### Neutral
- Range-based tests are unaffected.

## Revisit if
- A new dominant target model adopts a different encoding.

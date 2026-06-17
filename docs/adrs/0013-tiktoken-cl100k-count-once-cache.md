---
id: '0013'
title: Token counting via tiktoken-rs cl100k_base, counted once during indexing and cached
status: ACCEPTED
date: 2026-03-05
triggered_by: Budget allocation requires a token count per file/section; needs a consistent tokenizer and must not be recomputed repeatedly
loop: planning
---

# ADR-0013: Token counting via tiktoken-rs cl100k_base, counted once during indexing and cached

## Context

cxpak budgets its output by tokens, so it needs a tokenizer. This decision was taken
during initial design (v0.1.0). The design picks `tiktoken-rs` with the cl100k_base
encoding as a "close enough across models" baseline, and mandates counting tokens once
during the index pass, then budgeting from the cached counts with no re-tokenization.

The two halves of this decision had different fates. The "count once / cache / no
re-tokenization" half shipped and remains in force. The specific cl100k_base encoding
choice did not survive: one week later (commit ce47712, 2026-03-12) the encoding was
switched to o200k_base — the Claude/GPT-4o tokenizer — which is what the shipped code
uses today (`src/budget/counter.rs`). This ADR records the planning-time decision as
made; the encoding change is captured below under Negative consequences and Revisit if.

## Options considered

- **Option A — tiktoken-rs cl100k_base, count once, cache:** Use OpenAI's cl100k BPE
  as a universal baseline and cache per-file counts on the index. Pros: widely used,
  fast, deterministic, single count pass. Cons: not exact for non-OpenAI models — one
  tokenizer approximates all targets. This was the chosen option at design time, and the
  caching strategy shipped; the encoding was later revised (see below).

- **Option B — per-model tokenizers selected at runtime:** A reasonable alternative would
  have been to load the target model's exact tokenizer per request. Pros: exact counts per
  model. Cons: many tokenizers to bundle, configuration burden, and it contradicts the
  "close enough" simplicity the design wanted. Not formally evaluated.

- **Option C — character/heuristic estimate:** A reasonable alternative would have been to
  approximate token counts from byte or character counts. Pros: no tokenizer dependency at
  all. Cons: inaccurate enough to undermine the budget guarantees the whole pipeline rests
  on. Not formally evaluated.

## Decision

Use `tiktoken-rs` as the token-counting dependency, count tokens once during the index pass,
and budget from the cached counts with no re-tokenization. At design time the encoding was
specified as cl100k_base as a universal "close enough across models" baseline. (The encoding
was subsequently changed to o200k_base in shipped code — see Negative consequences.)

`TokenCounter` wraps `CoreBPE` and exposes `count()` and `count_or_zero()`. Each file is
counted once during indexing; the per-file `token_count` and the aggregate `total_tokens`
are stored on the index, and budget allocation reads those cached numbers.

## Consequences

### Positive
- Single fast count pass during indexing; budgeting just reads cached numbers.
- Deterministic, model-agnostic baseline that avoids per-request tokenizer loading.

### Negative
- Counts are approximate for non-OpenAI models; budgets are guidance, not exact for every
  target model.
- The cl100k_base encoding specified here did not ship as-is. Commit ce47712 (2026-03-12,
  "fix: use o200k_base tokenizer (Claude/GPT-4o) instead of cl100k_base") switched the
  encoding to o200k_base, which is what `src/budget/counter.rs` uses today. This ADR's
  encoding choice is therefore superseded by that change.

### Neutral
- `TokenCounter` wraps `CoreBPE` with `count()` and `count_or_zero()`.

## Revisit if
- A target model's tokenization diverges enough from the chosen encoding that budget
  overshoot causes truncation in the consuming LLM. (This trigger effectively fired for
  cl100k_base and was resolved by switching to o200k_base, the Claude/GPT-4o tokenizer.)

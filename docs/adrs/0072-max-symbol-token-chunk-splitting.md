---
id: '0072'
title: Split symbols over 4000 tokens via AST-aware then blank-line then hard fallback
status: ACCEPTED
date: 2026-03-19
triggered_by: A single oversized symbol can blow a budget and lose coherence in an LLM
loop: planning
---

# ADR-0072: Split symbols over 4000 tokens via AST-aware then blank-line then hard fallback

## Context

Introduced in the cxpak v0.11.0 context-quality design. A single oversized symbol (a huge function, a generated module) can consume a budget on its own and loses coherence when fed whole to an LLM.

`MAX_SYMBOL_TOKENS = 4000` was chosen to match a typical LLM attention window for one unit. Symbols exceeding it are split with a 3-tier fallback: AST-aware split (re-parse with tree-sitter, split between top-level body children), then blank-line split, then a hard split at the 4000-token boundary. Each chunk becomes its own `Symbol` named `handler [1/3]` etc., carries the parent signature, has adjusted line numbers, and participates in degradation individually. AST re-parsing is on-demand (it avoids storing ASTs in the index; tree-sitter re-parse is ~1ms).

The shipped v0.11.0 implementation plan deferred true AST-aware boundaries to a TODO and shipped line-based splitting first.

## Options considered

- **Option A — 3-tier split (AST-aware → blank-line → hard), on-demand re-parse:** Re-parse source with tree-sitter only for oversized symbols, falling back progressively. Pros: clean semantic boundaries when possible, no ASTs stored in the index, ~1ms re-parse cost. Cons: AST re-parse requires `LanguageRegistry` access, and the AST tier was deferred initially. (Grounded — chosen; line-based splitting shipped, the AST tier was marked TODO.)

- **Option B — Always store ASTs in the index:** Keep parse trees so splitting needs no re-parse. Pros: no re-parse needed. Cons: large memory cost for what is a last-resort feature. (Grounded — discussed as the rejected counterpart to on-demand re-parse; "avoids storing ASTs in index" is the stated rationale.)

- **Option C — Pure hard split at the token boundary:** Always cut at 4000 tokens regardless of structure. A reasonable alternative would have been this for triviality. Cons: breaks mid-statement, poor coherence. (Reconstructed; only present as the last-resort tier, not a standalone considered option.)

## Decision

Set `MAX_SYMBOL_TOKENS = 4000` and split oversized symbols via AST-aware → blank-line → hard fallback, with on-demand re-parse rather than stored ASTs. Each chunk gets `[i/N]` naming, the duplicated parent signature, adjusted line numbers, and degrades independently.

Confirmed shipped: `MAX_SYMBOL_TOKENS` and `split_oversized_symbol()` in `src/context_quality/degradation.rs` produce `{name} [{i}/{N}]` chunks carrying the parent signature with adjusted line numbers, each an independently-degradable symbol. The AST-aware tier was deferred — the source-parameter is unused and there is no tree-sitter re-parse; the shipped splitter is generic token-greedy line accumulation with a single-line hard fallback.

## Consequences

### Positive
- Oversized functions stay usable as coherent chunks.
- The index avoids storing parse trees.
- Chunks degrade independently, so chunk 1 can stay Full while chunk 3 degrades.

### Negative
- The initial shipped version lacked true AST-aware boundaries (deferred to TODO); it splits on lines, not semantic structure.
- The hard-split fallback can still break mid-statement on minified code.

### Neutral
- The 4000-token limit is a heuristic tied to attention-window intuition.

## Revisit if
- The AST-aware split tier is fully implemented.
- A different per-symbol token ceiling proves better.
- Re-parse cost becomes significant on huge files.

---
id: '0071'
title: Prepend per-file [cxpak] annotations using language-correct comment syntax
status: ACCEPTED
date: 2026-03-19
triggered_by: The LLM needs to know each packed file's relevance, role, and detail level without breaking the packed context's syntax
loop: planning
---

# ADR-0071: Prepend per-file [cxpak] annotations using language-correct comment syntax

## Context

Introduced in the cxpak v0.11.0 context-quality design. When `pack_context` bundles files for an LLM, the model benefits from knowing each file's relevance, role, parent, and detail level — but that metadata must not corrupt the packed context's syntax, since the LLM may try to parse the code blocks.

The design prepends a 4-line `[cxpak]` annotation header to each file: path; score + role + parent; signal breakdown; detail level + tokens. `comment_syntax(language)` returns the correct comment delimiters per language family (`// `, `# `, `-- `, `<!-- -->`, `/* */`, `% `) so the header is valid syntax in that language. The signal line is omitted at detail Level 2+ to save tokens.

The design also intended the annotation's own token cost to be counted against the file's budget allocation (computed via `TokenCounter` and subtracted before rendering symbols). That budget-accounting half was specified but not implemented as shipped — see Decision and Consequences.

## Options considered

- **Option A — Language-aware comment-syntax annotations, counted in budget:** A 4-line header rendered in the file's native comment syntax; signal line dropped at Level 2+; annotation tokens charged to the file's budget. Pros: valid syntax avoids parse errors in packed context, and budget accounting is honest. Cons: a per-language comment mapping must be maintained. (Grounded — chosen; the comment-syntax half shipped, the budget-accounting half did not.)

- **Option B — Plain-text / markdown separator headers:** Use a uniform non-comment delimiter between files. A reasonable alternative would have been this for a single format across all languages. Cons: injects invalid syntax into code blocks the LLM may try to parse. (Reconstructed; not formally evaluated.)

- **Option C — Annotations free (not counted against budget):** Treat headers as overhead outside the token budget. Pros: simpler accounting. Cons: budget overrun; the design called for honest accounting. (Grounded as a discussed trade-off — and, as it turned out, the as-shipped behavior is effectively this: annotation tokens are not charged against the budget.)

## Decision

Render a 4-line `[cxpak]` annotation per file using `comment_syntax()` for the file's language, and omit the signal line at detail Level 2+.

The design also specified subtracting the annotation's `TokenCounter` cost from the budget before rendering symbols. This was design intent only and is NOT implemented in the shipped pack path: `allocate_with_degradation()` (`src/context_quality/degradation.rs`) budgets purely against symbols' rendered tokens, and `src/commands/serve.rs` generates the annotation after allocation and prepends it, accumulating only effective (symbol/content) tokens into `total_tokens`. Annotation header tokens are therefore not measured or charged.

Confirmed shipped: `src/context_quality/annotation.rs` (`comment_syntax()`, `annotate_file()`), wired into `pack_context` in `src/commands/serve.rs`. Unknown languages default to `// ` comments.

## Consequences

### Positive
- Packed context stays syntactically valid per language.
- The LLM is told each file's score, role, parent, and detail level.

### Negative
- A per-language comment-syntax table must be maintained alongside the language list.
- Annotation header tokens are excluded from budget accounting in the shipped pack path, so packed output can slightly exceed the requested budget by the per-file annotation overhead. (The design intended to charge these tokens; that was not implemented.)

### Neutral
- Unknown languages default to `// ` comments.

## Revisit if
- A newly supported language needs a comment syntax not in the table.
- Annotation overhead proves to consume too much budget (which would make implementing the deferred budget-accounting worthwhile).

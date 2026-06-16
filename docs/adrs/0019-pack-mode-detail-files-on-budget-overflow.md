---
id: '0019'
title: 'Pack mode: write full analysis to .cxpak/ detail files when a repo exceeds the token budget'
status: ACCEPTED
date: 2026-03-08
triggered_by: Large repos exceed the token budget, so the budgeted overview alone discards most of cxpak's computed analysis, leaving the LLM unable to recover detail.
loop: planning
---

# ADR-0019: Pack mode: write full analysis to .cxpak/ detail files when a repo exceeds the token budget

## Context

As of v0.2.0, the `overview` command produces a single token-budgeted document. For large repos most sections get truncated and the CPU-computed analysis (tree, modules, dependencies, signatures, key files, git) is thrown away. The design introduces a two-mode pipeline keyed off `index.total_tokens` vs `token_budget`: when the repo fits, behavior is unchanged; when it overflows, the full analysis is preserved on disk under a `.cxpak/` directory so the LLM can pull detail on demand instead of re-running cxpak.

## Options considered

- **Option A — Two-mode pipeline (single-file when it fits, pack mode when over budget):** When `index.total_tokens > token_budget`, render each section twice — budgeted (for the overview) and unbudgeted (for a detail file under `.cxpak/`). Pros: preserves all computed analysis on disk; overview stays within budget; the LLM can fetch a specific detail file on demand. Cons: writes files into the target repo, requiring stale-file cleanup and `.gitignore` handling. Chosen.
- **Option B — Always single-file, just truncate harder:** Keep the existing single-file behavior and accept lossy truncation for large repos. Pros: no filesystem side effects; simplest possible implementation. Cons: discards most of the analysis for large repos — the exact problem this design exists to solve. Someone could prefer it to avoid writing anything into the scanned repo.
- **Option C — Add a CLI flag to opt into detail files:** A reasonable alternative would have been to gate detail-file output behind an explicit flag rather than auto-detecting budget overflow. Pros: the user controls side effects explicitly. Cons: the design explicitly chose to add no new CLI args, preferring zero-config auto-detection; an opt-in flag means large repos silently lose detail unless the user knows to ask for it.

## Decision

Adopt a two-path render pipeline. If `index.total_tokens <= token_budget`, single-file mode is unchanged (byte-identical output). Otherwise pack mode renders each section twice — budgeted plus full — writes the full versions to standalone files under `.cxpak/` (tree, modules, dependencies, signatures, key-files, git), and rewrites the overview's omission markers into pointers to those files. No new CLI flags are added.

Shipped in `src/commands/overview.rs`: `struct SectionContent { budgeted, full, was_truncated }`, `let pack_mode = index.total_tokens > token_budget;`, and a `detail_sections` array writing the six sections under `.cxpak/`. Supporting code: `omission_pointer` / `truncate_to_budget_with_pointer` in `src/budget/degrader.rs`, `ensure_gitignore_entry` in `src/util.rs`, and `render_single_section` across the output renderers.

## Consequences

### Positive
- Overview stays within budget while no analysis is lost.
- The LLM can fetch a specific detail file instead of re-running cxpak.
- Single-file behavior is byte-identical when the repo fits.

### Negative
- Writes a `.cxpak/` directory into the target repo (needs `.gitignore` handling and stale-file cleanup).
- Each truncated section is rendered twice (extra CPU).

### Neutral
- Metadata gains "Token budget" and "Detail files" lines only in pack mode.

## Revisit if
- Detail files grow large enough that writing them dominates runtime.
- Users object to filesystem writes in the scanned repo.

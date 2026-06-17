---
id: '0027'
title: Diff command combines git2 diff with trace-style context using a diff-first budget strategy
status: ACCEPTED
date: 2026-03-10
triggered_by: There is no command that shows what changed in a repo plus the surrounding code needed to understand the change within a token budget.
loop: planning
---

# ADR-0027: Diff command combines git2 diff with trace-style context using a diff-first budget strategy

## Context

Before v0.4.0 there was no command that shows what changed in a repo together with the surrounding code needed to understand the change, packed within a token budget. `cxpak diff` fills that gap: it is `git diff` + `trace` combined — extract the git changes via git2, gather the dependency context (callers, types, imports) the way `trace` does, and pack both into a single budget.

The order of budget allocation determines what survives truncation, so it is the central design choice for the command.

## Options considered

- **Option A — Diff-first allocation: render all diff hunks, fill the remainder with BFS-ordered context:** Phase 1 renders the full diff (truncating the least-important hunks only if the diff alone overflows); phase 2 fills the remaining budget with context ordered by dependency distance. Pros: the change itself is never sacrificed for context, and context degrades gracefully by distance. Cons: a very large diff leaves no room for context. Someone could prefer this because the user asked about the change, so the change should always win the budget.
- **Option B — Context-first allocation:** A reasonable alternative would have been to reserve budget for context first, then fit the diff into what remains. Pros: guarantees some surrounding code is always present. Cons: could truncate the actual change the user asked about. Someone could prefer it when the surrounding code matters more than seeing every hunk.
- **Option C — Even split between diff and context:** A reasonable alternative would have been a fixed fraction of the budget to each side. Pros: predictable allocation. Cons: wastes budget when one side is small and can truncate the diff unnecessarily. Someone could prefer it for its simplicity and predictability.

## Decision

Implement `cxpak diff`: use git2 to diff a ref (default HEAD vs working tree) producing `FileChange` entries; scan and parse with the parse cache; build the `DependencyGraph` and gather context (1-hop default, full BFS with `--all`) excluding the changed files themselves; then allocate budget diff-first — render all diff hunks first, then fill the remaining budget with context ordered by BFS distance.

Confirmed shipped in `src/commands/diff.rs` (`extract_changes`, `FileChange`) and `src/index/graph.rs` (`reachable_from`).

## Consequences

### Positive
- The change is always shown in full when it fits; context is supplementary.
- Reuses Scanner/Parser/Index/DependencyGraph/Output and the parse cache.
- `--all` is opt-in for full transitive context.

### Negative
- Oversized diffs crowd out context entirely.
- Diff content reuses the `key_files`/`signatures` `OutputSections` slots rather than dedicated fields.

### Neutral
- Untracked and binary files are skipped; a no-change run prints "No changes detected." and exits 0.

## Revisit if
- Users frequently hit diffs large enough to starve context (consider a context floor or a cap on diff size).
- A dedicated diff `OutputSections` shape is warranted over reusing existing slots.

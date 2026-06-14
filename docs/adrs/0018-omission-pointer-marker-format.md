---
id: '0018'
title: Pointer-style omission markers reference .cxpak/ detail files instead of suggesting a higher token budget
status: ACCEPTED
date: 2026-03-08
triggered_by: Single-file omission markers tell the user to raise --tokens, which is meaningless in pack mode where the omitted content already lives in a detail file.
loop: implementation
---

# ADR-0018: Pointer-style omission markers reference .cxpak/ detail files

## Context

The single-file `omission_marker` emits `<!-- section omitted: ~Nk tokens. Use --tokens Mk+ to
include -->`. That hint is meaningless in pack mode (v0.2.0), where the omitted content already
lives in a `.cxpak/` detail file — telling the user to raise `--tokens` points them at nothing.
Pack mode needs a marker that points at the on-disk detail file instead.

## Options considered

- **Option A — add a new `omission_pointer` function and a `truncate_to_budget_with_pointer`
  wrapper:** Separate functions for pack-mode markers, leaving the existing
  `truncate_to_budget` untouched. Pros: single-file behavior is unchanged, and it avoids a
  boolean-flag-laden function. Cons: two near-duplicate truncation paths to maintain. This was
  the chosen option and shipped.

- **Option B — add a boolean flag to the existing `truncate_to_budget`:** Pass a `pack_mode`
  bool that switches the marker style inside one function. Pros: no duplicate function. Cons:
  the implementation plan explicitly rejected this as "ugly." This alternative was considered
  and rejected.

## Decision

Add a distinct `omission_pointer(section, filename, omitted_tokens)` producing
`<!-- {section} full content: .cxpak/{filename} ({tokens}) -->`, plus
`truncate_to_budget_with_pointer` wrapping the same truncation logic but emitting the pointer
marker. Do not modify the existing `omission_marker` / `truncate_to_budget`. Confirmed shipped
in `src/budget/degrader.rs`.

## Consequences

### Positive
- Pack-mode markers are actionable — they name the file to read.
- Single-file marker behavior is preserved verbatim.

### Negative
- Two parallel truncation functions exist in `degrader.rs`.

### Neutral
- Token-count formatting (`~N` vs `~N.Nk`) is shared between both markers.

## Revisit if
- A third marker variant is needed (which would argue for parameterizing the marker rather than
  adding another function).

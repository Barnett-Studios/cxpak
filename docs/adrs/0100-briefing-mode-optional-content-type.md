---
id: '0100'
title: Briefing mode shares full mode's structure with content as Option<String> (None in briefing)
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.2.0 adds a briefing mode to auto_context
loop: planning
---

# ADR-0100: Briefing mode shares full mode's structure with content as Option<String> (None in briefing)

## Context
v1.2.0 added a briefing mode to `auto_context`. Briefing gives the LLM the intelligence
layer (health, risks, architecture) and a scored file list without packing source
content, so the LLM can fetch only the files it needs via `cxpak_pack_context`. The
open question was how to represent "no content" without forcing the LLM to learn two
different result shapes. The design doc resolved it: briefing "sets `content: None` on
packed files (the `content` field on `PackedFile` changes from `String` to
`Option<String>` ... `Some(content)` in full mode, `None` in briefing mode) ... This is
a type-level distinction, not a convention based on empty strings."

This is a human decision because it accepts a breaking type change to `PackedFile` in
exchange for a single schema across both modes — a trade-off the code cannot make on
its own. Shipped in `src/auto_context/briefing.rs` and `src/auto_context/mod.rs`, with
the MCP wiring in `src/commands/serve.rs`.

## Options considered
- **Option A — same schema, `PackedFile.content` becomes `Option<String>`:** `mode:full`
  sets `content: Some(..)`, `mode:briefing` sets `content: None`; the intelligence
  fields are identical in both modes; the distinction is type-level, not an
  empty-string convention. Pros: one schema for the LLM to learn; type-safe distinction
  between modes; the LLM fetches needed files via `cxpak_pack_context`. Cons: changing
  `content` from `String` to `Option<String>` is a breaking type change. Someone could
  prefer this for the single learnable schema. (Chosen.)
- **Option B — separate briefing result type:** a distinct struct for briefing output.
  A reasonable alternative would have been to leave the full-mode type untouched. Pros:
  no change to `PackedFile`. Cons: the LLM must learn two schemas, and the
  intelligence-field plumbing is duplicated. Someone could prefer it to avoid the
  breaking type change.

## Decision
Add a `mode` parameter to `auto_context` (`full` default, `briefing`). Both modes return
identical structure; the only difference is `PackedFile.content` changes from `String`
to `Option<String>` — `Some(content)` in full mode, `None` in briefing. This is a
type-level distinction, not an empty-string convention. The intelligence layer is
identical in both modes. `cxpak_briefing` is an alias for `auto_context` with
`mode:briefing`, calling the same code path.

## Consequences
### Positive
- The LLM learns one schema for both modes.
- Type-safe distinction between full and briefing output.
- Briefing lets the LLM fetch only the files it needs via `cxpak_pack_context`.

### Negative
- The `PackedFile.content` type change from `String` to `Option<String>` is breaking.

### Neutral
- `cxpak_briefing` is a thin alias, not a separate implementation.

## Revisit if
- The shared-schema constraint forces awkward optionality elsewhere in the result type.

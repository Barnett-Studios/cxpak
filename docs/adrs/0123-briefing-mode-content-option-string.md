---
id: '0123'
title: 'Briefing mode: strip source by making PackedFile.content an Option<String> set to None'
status: ACCEPTED
date: 2026-04-01
triggered_by: 'v1.2.0 cxpak_briefing tool: deliver the intelligence layer for large codebases without spending token budget on source'
loop: implementation
---

# ADR-0123: Briefing mode: strip source by making PackedFile.content an Option<String> set to None

## Context

In v1.2.0, for large codebases an agent may want the intelligence layer — health, risks, architecture, co-changes — plus a ranked file list, but not the source bytes. The packer already counts tokens per file, so the design needed a way to omit content while preserving correct budget math.

## Options considered

- **Option A — `PackedFile.content` as `Option<String>`, `None` in briefing mode:** Token counting still runs (so budget allocation is unchanged); content is stripped to `None` just before return when `mode == "briefing"`. Pros: budget math stays correct, same code path as full mode, one boolean flag threads through `allocate_and_pack`. Cons: API consumers must handle `Option` content. This is the chosen approach.
- **Option B — separate briefing struct without a content field:** A reasonable alternative would have been to define a distinct type for briefing output. Pros: no `Option` for consumers to handle. Cons: duplicates the packing pipeline and the `AutoContextResult` shape. Someone could prefer it to keep the full-mode type free of an optional field.

## Decision

`PackedFile.content` is an `Option<String>` (`src/auto_context/briefing.rs`). `AutoContextOpts` gains a `mode` field (`"full"` default or `"briefing"`); `allocate_and_pack` gains a `briefing_mode: bool` derived from `opts.mode == "briefing"`. In briefing mode, token counting proceeds normally (so budget math is correct) but every `PackedFile.content` is set to `None` instead of `Some(...)`. `cxpak_briefing` is implemented as an alias to `auto_context` with `mode = "briefing"` (`src/commands/serve.rs`), reusing the pipeline rather than duplicating it.

## Consequences

### Positive
- Agents get full compound intelligence (health, risks, architecture, co-changes) plus a scored file list at zero source-token cost.
- Budget accounting remains identical to full mode (token counting runs before the content strip).
- `cxpak_briefing` reuses the `auto_context` pipeline rather than duplicating it.

### Negative
- All consumers of `PackedFile.content` must handle the `Option`.

### Neutral
- Implemented across `src/auto_context/briefing.rs` and `src/auto_context/mod.rs`.

## Revisit if
- A third packing mode is needed that requires more than a boolean toggle.

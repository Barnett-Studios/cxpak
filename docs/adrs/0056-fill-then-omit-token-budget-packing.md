---
id: '0056'
title: 'Token-budget packing: fill by relevance rank, omit overflow with reasons'
status: ACCEPTED
date: 2026-03-17
triggered_by: pack_context must respect a caller-supplied token budget
loop: implementation
superseded_by: 'v0.11.0 (commit fafa939, 2026-03-21) replaced greedy whole-file packing with progressive degradation via allocate_with_degradation(); the ''omitted''/''budget exceeded'' array was removed and replaced by per-file ''detail_level'' plus a ''not_found'' array.'
---

# ADR-0056: Token-budget packing: fill by relevance rank, omit overflow with reasons

## Context

In v0.9.0, `cxpak_pack_context` accepts a token budget (e.g. `'30k'`). When the selected files plus their dependencies exceed the budget, the tool must decide what to include and how to report what it dropped. This ADR documents the v0.9.0 `pack_context` as originally shipped; the current tool returns `detail_level` + `not_found` rather than an `omitted` array (see Superseded note below).

## Options considered

- **Option A — Greedy fill in rank order, omit overflow with explicit reasons:** Iterate the target files (selected first, then dependencies), add each whole file if it fits the remaining budget, otherwise record it in an `omitted` list with its token count and the reason `'budget exceeded'`. Pros: simple, deterministic, transparent about what was dropped and why, and prioritizes selected files over dependencies. Cons: all-or-nothing per file (no partial or degraded inclusion), so a large early file can block several smaller relevant ones.

- **Option B — Progressive degradation (trim/signature) to fit more files:** Reduce per-file detail (trim bodies, fall back to signatures) to squeeze more files into the budget. A reasonable alternative would have been to do this from the start, since it represents more files within the same budget. Cons: more complex, and it was not needed for the v0.9.0 pack tool. (This was not formally evaluated at the time, and is reconstructed here; it is in fact the approach that later superseded Option A.)

## Decision

`pack_context` parses the token budget (default 50k), assembles the selected files plus optional 1-hop dependencies, then greedily packs whole files in rank order while `total_tokens` stays within budget. Overflow files are recorded in an `omitted` array with per-file token counts and the reason `'budget exceeded'`. The tool returns `packed_files` count, `total_tokens`, `budget`, `files` (with content and an `included_as` tag distinguishing selected vs dependency), and `omitted`.

## Consequences

### Positive
- Deterministic, budget-respecting bundles.
- The caller sees exactly which files were dropped and why.
- `selected` vs `dependency` provenance is tagged on each file.

### Negative
- No partial-file inclusion; a single oversized file is either fully in or fully omitted. This all-or-nothing limitation materialized in practice and drove the v0.11.0 switch to progressive degradation (Option B) — the negative consequence is what motivated the supersession recorded in the frontmatter.

### Neutral
- Default budget is 50k tokens.

## Revisit if
- Whole-file packing wastes budget on large files.
- Progressive degradation is needed to represent more files. (This condition was met: v0.11.0 replaced the greedy logic with `allocate_with_degradation()`.)

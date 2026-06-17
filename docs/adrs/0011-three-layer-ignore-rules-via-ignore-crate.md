---
id: '0011'
title: Three-layer ignore model (.gitignore + built-in defaults + .cxpakignore) using the ripgrep ignore crate
status: ACCEPTED
date: 2026-03-05
triggered_by: Scanner must skip vendored/generated/binary files and honor existing repo conventions
loop: planning
---

# ADR-0011: Three-layer ignore model (.gitignore + built-in defaults + .cxpakignore) using the ripgrep ignore crate

## Context

File discovery in v0.1.0 needs to exclude noise — vendored, generated, and binary files — while honoring existing repo conventions. The design defines three ordered ignore layers: standard `.gitignore`, a built-in smart-defaults list (`node_modules`, `target`, lock files, binaries, etc.), and an optional project-specific `.cxpakignore`. All three are implemented on top of the ripgrep-ecosystem `ignore` crate's `WalkBuilder`.

## Options considered

- **Option A — ignore crate + 3 layered rule sources:** `WalkBuilder` honoring `.gitignore`/global/exclude, plus `BUILTIN_IGNORES` overrides and an optional `.cxpakignore`. Pros: reuses the battle-tested ripgrep walker, respects repo conventions, and offers a project override hook. Cons: the built-in defaults are a maintained hardcoded list. Someone could prefer this for correct gitignore semantics out of the box.
- **Option B — Hand-rolled directory walk + custom glob matching:** A reasonable alternative would have been implementing traversal and ignore matching from scratch. Pros: no external dependency. Cons: reinvents gitignore semantics, error-prone, and slower. Someone could prefer it to avoid pulling in the ripgrep dependency tree.

## Decision

Apply three ordered ignore layers — `.gitignore`, a built-in defaults list (`BUILTIN_IGNORES`: `node_modules`, `target`, `dist`, lock files, binary/media extensions, etc.), and an optional `.cxpakignore` — built on the ripgrep `ignore` crate's `WalkBuilder`.

## Consequences

### Positive
- Reuses proven gitignore semantics from the ripgrep ecosystem.
- Repos get sensible exclusions with zero config; projects can extend via `.cxpakignore`.

### Negative
- `BUILTIN_IGNORES` is a static list needing maintenance as ecosystems evolve.
- A `.gitignore`-respect regression was later found and fixed (the `WalkBuilder` was missing `git_ignore(true)`/`git_exclude(true)`; a regression test now guards it).

### Neutral
- The scanner requires a `.git` directory (`NotARepository` error) since it leans on git ignore semantics.

## Revisit if
- Users need to index non-git directories.
- Built-in defaults drift from real-world project layouts.

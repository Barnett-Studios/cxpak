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

## Amended (#39) — hidden entries are walked, and `BUILTIN_IGNORES` now carries three purposes

This ADR recorded the three *sources* of ignore rules and never said what happened to hidden
entries. The `WalkBuilder` carried `.hidden(true)` beside a comment reading "visit hidden files".
In the `ignore` crate `hidden(true)` **skips** hidden entries, so the code did the opposite of its
comment and every dotfile was silently absent from every bundle — `.github/workflows`,
`.editorconfig`, `.eslintrc`: the files that state how a project builds and what conventions it
holds. Nothing here or anywhere else recorded a decision to exclude them, so this is corrected to
`.hidden(false)` and the stated intent now holds.

The order the two halves of #39 landed in was not incidental. `.env` was excluded only *because it
was hidden*, so unhiding first would have exposed it on every repo in the window between the
changes. The credential denylist landed first (#67 and `3b8bbc52`), and only then this.

`BUILTIN_IGNORES` consequently now serves three distinct purposes, worth naming because they age
differently:

1. **Noise** — `node_modules`, `target`, lock files, binary and media extensions. The original list.
2. **Credential material** — exact filenames and key-material extensions. A *control*, not a
   convenience: before it, the only thing between a committed private key and a context bundle was
   whether the user's gitignore happened to cover it.
3. **Tool caches** — `.mypy_cache`, `.pytest_cache`, `.tox`, `.terraform` and kin. Reachable only
   since `hidden(false)`; while dotfiles were skipped wholesale these cost nothing.

`.cache` and `.yarn` are deliberately absent from (3): both are broad enough to hold real source
(Yarn PnP keeps `.yarn/patches` and `.yarn/releases`), and excluding real source silently is the
same defect as excluding dotfiles silently.

The "Revisit if" below gains one: **hidden-entry walking makes `BUILTIN_IGNORES` load-bearing for
directories it never had to cover.** A cache directory absent from the list is now indexed rather
than skipped, and the symptom is only a budget quietly spent on the wrong files.

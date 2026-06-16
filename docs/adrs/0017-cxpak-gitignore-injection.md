---
id: '0017'
title: Pack mode appends .cxpak/ to the target repo's .gitignore idempotently
status: ACCEPTED
date: 2026-03-08
triggered_by: Pack mode writes a .cxpak/ directory into the scanned repo, which would otherwise show up as untracked noise in git status.
loop: planning
---

# ADR-0017: Pack mode appends .cxpak/ to the target repo's .gitignore idempotently

## Context

Pack mode writes detail files into a `.cxpak/` directory at the scanned repo's root. This
decision was taken during the multi-file output design (v0.2.0). Without intervention,
`.cxpak/` would appear as untracked noise in the user's `git status`. To avoid polluting the
working tree, cxpak should ensure `.cxpak/` is git-ignored.

## Options considered

- **Option A — auto-append `.cxpak/` to `.gitignore` (create if absent), idempotent:** On pack
  mode, read `.gitignore`, and append the entry only if it is not already present, creating the
  file if it is missing. Pros: zero-config; keeps the user's `git status` clean. Cons: modifies
  a user-owned file as a side effect. This was the chosen option and shipped.

- **Option B — leave `.gitignore` alone; document that users should ignore `.cxpak/`:** A
  reasonable alternative would have been to never touch the user's files and rely on
  documentation. Pros: no surprising writes. Cons: untracked `.cxpak/` clutters `git status` by
  default. Not formally evaluated.

## Decision

On pack mode, read the repo's `.gitignore` and append `.cxpak/` if it is not already present,
creating the file if it is missing. The operation is idempotent — it matches a trimmed line and
skips the append if found. Implemented as `ensure_gitignore_entry` in `src/util.rs`.

## Consequences

### Positive
- Detail files never appear as untracked changes.
- Works whether or not a `.gitignore` already exists.

### Negative
- Silently edits a user-owned file.

### Neutral
- No-op if `.cxpak/` is already ignored.

## Revisit if
- Users report unwanted `.gitignore` edits.

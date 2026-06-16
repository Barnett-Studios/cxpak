---
id: '0025'
title: Stale .cxpak/ cleanup preserves the cache/ subdirectory; only output files are wiped
status: ACCEPTED
date: 2026-03-10
triggered_by: v0.3.0's cleanup did remove_dir_all('.cxpak'), which would now also delete the new parse cache on every run.
loop: planning
---

# ADR-0025: Stale .cxpak/ cleanup preserves the cache/ subdirectory; only output files are wiped

## Context

In pack mode, the `overview` command writes detail files (`tree.md`, `modules.md`, etc.) under `.cxpak/` and cleans the stale ones between runs. Through v0.3.0 this cleanup was a blanket `remove_dir_all(".cxpak")`.

v0.4.0 adds a per-file parse cache stored under `.cxpak/cache/`. The cache only delivers a speedup if it survives across runs. A blanket removal of `.cxpak/` would delete the cache on every invocation, defeating it entirely. The cleanup step therefore has to discriminate between output files (still stale, still removed) and the cache (must persist).

## Options considered

- **Option A — Selective cleanup: remove every `.cxpak/` child except `cache/`:** Iterate the `.cxpak/` directory entries and delete all of them except the cache directory. Pros: keeps the cache warm across runs while still clearing stale output files. Cons: the cleanup logic is no longer a single `remove_dir_all` call. Someone could prefer this because it keeps all cxpak state in one location and matches the design's explicit requirement to preserve only `cache/`.
- **Option B — Move the cache outside `.cxpak/`:** A reasonable alternative would have been to store the cache in a separate directory not subject to output cleanup, leaving the cleanup step as a simple `remove_dir_all(".cxpak")`. Pros: cleanup stays a one-liner. Cons: splits cxpak state across two filesystem locations, complicating both the `clean` command and any `.gitignore` guidance. Someone could prefer it purely for the simplicity of the cleanup code path.

## Decision

Change the `overview` stale-cleanup from `remove_dir_all(".cxpak")` to iterating the `.cxpak/` entries and removing everything except the `cache` directory. Parse cache survives between runs; stale output files are still cleared each run. The full-wipe path is reserved for the explicit `cxpak clean` command.

Shipped in `src/commands/overview.rs` (selective cleanup that preserves the `cache` directory); `src/commands/clean.rs` retains `remove_dir_all(".cxpak")` as the explicit full-wipe path.

## Consequences

### Positive
- Cache persists across runs, delivering the intended speedup.
- Stale output files are still removed each run.

### Negative
- Cleanup code is no longer a one-liner.

### Neutral
- `cxpak clean` remains the only path that deletes the cache.

## Revisit if
- Additional `.cxpak/` subdirectories are added that also need preserving (the exclusion list would need to grow, suggesting a more general rule).

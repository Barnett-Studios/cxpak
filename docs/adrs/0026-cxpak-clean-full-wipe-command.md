---
id: '0026'
title: cxpak clean subcommand fully removes the .cxpak/ directory (cache + outputs)
status: ACCEPTED
date: 2026-03-10
triggered_by: With caching and no time-based expiry, users need an explicit way to invalidate the cache and remove generated output.
loop: planning
---

# ADR-0026: cxpak clean subcommand fully removes the .cxpak/ directory (cache + outputs)

## Context

v0.4.0 introduces a per-file parse cache with no time-based expiry and no size limit, stored under `.cxpak/cache/`. Per-run cleanup (ADR-0025) deliberately preserves that cache, so it never invalidates on its own. Users therefore need an explicit way to invalidate the cache and remove generated output in one step. A separate, explicit command is the only full-wipe path.

## Options considered

- **Option A — Add a `clean` subcommand that does `rm -rf .cxpak/`:** A trivial command that deletes the entire `.cxpak/` directory and is a no-op success when nothing exists. Pros: explicit cache + output invalidation with a simple mental model. Cons: none significant. Someone could prefer this because it is discoverable, unambiguous, and complements the cache-preserving per-run cleanup.
- **Option B — Add a `--no-cache` flag to other commands:** A reasonable alternative would have been to bypass or clear the cache via a flag on existing commands rather than a dedicated subcommand. Pros: no new subcommand to document. Cons: a flag does not cleanly remove generated output files, and it is less discoverable than a named command. Someone could prefer it to avoid adding a subcommand to the CLI surface.

## Decision

Add a `cxpak clean [path]` subcommand that removes the entire `.cxpak/` directory (cache + outputs), succeeding as a no-op when nothing exists. This is the sole full-wipe path; per-run cleanup preserves the cache.

Confirmed shipped in `src/commands/clean.rs`: joins `.cxpak`, calls `remove_dir_all` when present, and prints a no-op message when absent.

## Consequences

### Positive
- A single explicit command resets all cxpak state.
- Complements the cache-preserving per-run cleanup (ADR-0025).

### Negative
- Deletes outputs and cache together with no granularity.

### Neutral
- Succeeds silently (no-op) when `.cxpak/` is absent.

## Revisit if
- Users want cache-only or output-only clearing, which would require finer-grained subcommands or flags.

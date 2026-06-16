---
id: '0030'
title: Extract a single parse_with_cache function shared by overview, trace, and diff
status: ACCEPTED
date: 2026-03-10
triggered_by: After wiring caching into overview and trace independently, the cache+parse loop was duplicated across commands and would be copied again into diff.
loop: implementation
---

# ADR-0030: Extract a single parse_with_cache function shared by overview, trace, and diff

## Context

While implementing v0.4.0, caching was wired into `overview` and `trace` independently (implementation-plan tasks 3 and 4). Each command inlined an identical cache-aware parse loop: load the cache, check mtime + size, parse on a miss, save, return the results. The forthcoming `diff` command needs the same logic, which would mean a third copy.

## Options considered

- **Option A — Extract `parse_with_cache(files, repo_root, counter, verbose)` into `src/cache/parse.rs`:** One function loads the cache, checks mtime + size, parses on a miss, saves, and returns the parse-results map. Pros: single source of truth — `overview`, `trace`, and `diff` all call it, and behavior stays identical. Cons: adds a module and couples commands to the cache module's fixed signature. Someone could prefer this to eliminate the duplication before it becomes a third copy.
- **Option B — Leave the loop inlined per command:** Accept three copies of the cache + parse loop. Pros: no new abstraction. Cons: triplicated logic, drift risk, and a DRY violation. Someone could prefer it to avoid adding a module, but the duplication was the exact problem this task set out to correct.

## Decision

Extract the cache-aware parse loop into `crate::cache::parse::parse_with_cache` and call it from `overview`, `trace`, and `diff`, removing the inlined loops and the now-unused `LanguageRegistry` imports.

Confirmed shipped: `src/cache/parse.rs` exists and is called from `overview.rs`, `trace.rs`, and `diff.rs`.

## Consequences

### Positive
- A single implementation of cache lookup + parse + save.
- `diff` reuses it for free.

### Negative
- Commands depend on the shared function's fixed signature (`repo_root`, `counter`, `verbose`).

### Neutral
- The cache directory is always `repo_root/.cxpak/cache`.

## Revisit if
- A command needs cache behavior that diverges from the shared signature (would require parameterizing or splitting the function).

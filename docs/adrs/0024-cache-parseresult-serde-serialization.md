---
id: '0024'
title: Make ParseResult and its components serde-serializable so parse results persist in the cache
status: ACCEPTED
date: 2026-03-10
triggered_by: Caching parse results requires serializing the full ParseResult (symbols, imports, exports), which were not previously serde-derivable.
loop: implementation
---

# ADR-0024: Make ParseResult and its components serde-serializable so parse results persist in the cache

## Context

v0.4.0 introduces a cache that stores an optional `ParseResult` per file. To round-trip it through `cache.json`, the parser data model — `Symbol`, `SymbolKind`, `Visibility`, `Import`, `Export`, and `ParseResult` — must derive `Serialize`/`Deserialize`. tree-sitter parsing is the expensive step the cache exists to skip, so the cache must carry the real parse tree, not just cheap scalars.

## Options considered

- **Option A — Add `Serialize`/`Deserialize` derives to the parser language types:** Add serde derives to `Symbol`, `SymbolKind`, `Visibility`, `Import`, `Export`, and `ParseResult` in `src/parser/language.rs`. Pros: lets the cache persist real parse results, and the serde `derive` feature is already a dependency. Cons: couples the parser data model to serde and to the cache format version. Chosen.
- **Option B — Re-parse on every run, cache only token counts:** A reasonable alternative would have been to cache only cheap scalars and never the parse tree. Pros: no serde coupling on the parser types. Cons: defeats the main goal — tree-sitter parsing is the expensive step being cached; someone could prefer it only to keep the parser model decoupled from the cache format.

## Decision

Add `#[derive(Serialize, Deserialize)]` to `Symbol`, `SymbolKind`, `Visibility`, `Import`, `Export`, and `ParseResult` in `src/parser/language.rs` so a `CacheEntry` can carry an `Option<ParseResult>`, persisted to `cache.json` under a versioned `FileCache`. The serde `derive` feature was already a dependency.

Confirmed shipped: all six types carry the derives in `src/parser/language.rs`; `src/cache/mod.rs` defines the versioned `FileCache` (with a `CACHE_VERSION` constant) and `CacheEntry { ..., parse_result: Option<ParseResult> }`.

## Consequences

### Positive
- Full parse results are cached and restored, skipping tree-sitter on cache hits.
- Reuses the existing serde dependency.

### Negative
- The parser data model is now coupled to the cache's serialized format and `CACHE_VERSION`.

### Neutral
- Format changes require bumping `CACHE_VERSION` to invalidate old caches.

## Revisit if
- The `ParseResult` shape changes (must bump `CACHE_VERSION`).
- Cache size from embedded parse results becomes a problem.

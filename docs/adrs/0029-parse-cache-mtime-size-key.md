---
id: '0029'
title: Parse cache keyed on file path + mtime + size, stored as JSON in .cxpak/cache/
status: ACCEPTED
date: 2026-03-10
triggered_by: Re-running cxpak re-parses every file with tree-sitter every time, which is wasteful on unchanged files.
loop: planning
---

# ADR-0029: Parse cache keyed on file path + mtime + size, stored as JSON in .cxpak/cache/

## Context

Every cxpak run re-parses every git-tracked file with tree-sitter and recomputes token counts, even when nothing has changed. This is wasteful on unchanged files.

v0.4.0 introduces a per-file cache of parse results and token counts to skip that recomputation. The cache lives alongside the `.cxpak/` output directory. The design must pick a cache key (how to detect a changed file), a storage format, and an invalidation policy.

## Options considered

- **Option A — Cache keyed on path + mtime + size, single `cache.json` (serde_json):** Each entry records mtime and size; a mismatch on either triggers a re-parse. One JSON file, easy to inspect and delete. Pros: simple, debuggable, human-inspectable; no hashing cost; no time-based expiry needed. Cons: mtime + size can theoretically miss a same-size, same-mtime edit, and JSON is larger than a binary format. Someone could prefer this because it avoids reading file contents on a cache hit and stays trivially inspectable.
- **Option B — Content-hash cache key:** A reasonable alternative would have been to key entries on a hash of file contents. Pros: detects any content change reliably. Cons: must read and hash every file on every run, defeating much of the speedup. Someone could prefer it for correctness guarantees over raw speed.
- **Option C — bincode binary cache format:** Serialize the cache with bincode instead of JSON. Pros: smaller and faster to (de)serialize. Cons: not human-inspectable; the design explicitly defers this until performance matters. This was considered and deferred, not rejected outright — it is the documented fallback if (de)serialization becomes a bottleneck.

## Decision

Cache tree-sitter parse results and token counts per file, keyed on relative path + mtime (Unix seconds) + size in bytes; a mismatch invalidates that entry and triggers a re-parse. Persist as a single `cache.json` under `.cxpak/cache/` with a `CACHE_VERSION` field; a version mismatch or corruption yields an empty cache. No time-based expiry and no size limits.

Confirmed shipped in `src/cache/mod.rs` (`CACHE_VERSION` later bumped to 2).

## Consequences

### Positive
- Unchanged files skip re-parsing on subsequent runs.
- The cache is trivially inspectable and removable.
- The version field allows safe format evolution.

### Negative
- mtime + size keying can miss pathological same-size/same-mtime edits.
- The JSON cache is larger than a binary format would be.

### Neutral
- Orphaned entries for deleted files are harmless and left unpruned.

## Revisit if
- Cache (de)serialization becomes a performance bottleneck (switch to bincode, Option C).
- mtime + size false-negatives cause stale parses in practice (move toward a content hash).

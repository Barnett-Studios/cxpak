---
id: '0044'
title: Pass file content from parser to indexer to eliminate double disk reads
status: ACCEPTED
date: 2026-03-12
triggered_by: v0.7.0 performance workstream addressing 1000 reads for a 500-file repo
loop: planning
---

# ADR-0044: Pass file content from parser to indexer to eliminate double disk reads

## Context

Released in v0.7.0. `parse.rs` read every file during parsing, and `index/mod.rs` read every file again during indexing, doubling disk reads — roughly 1000 reads for a 500-file repo. The content already read during parsing should be reused by the indexer instead of being re-read.

## Options considered

- **Option A — return a content map alongside parse results:** `parse_with_cache` returns `(parse_results, content_map)`; `CodebaseIndex` gains a `build_with_content()` that uses the map instead of re-reading; the original `build()` is kept for back-compat. Pros: smaller diff, same perf benefit, keeps `build()` for existing tests. Cons: two parallel build entry points; on a cache hit the content must still be read once. Someone could prefer it precisely because it is the minimal change that captures the expensive-path win.
- **Option B — intermediate `ParsedFile` struct:** Introduce `ParsedFile { relative_path, content, parse_result }` and have `build` take `Vec<ParsedFile>`. Pros: cleaner single data model folding scanned + parsed + content together. Cons: larger refactor of `build()` and all callers. Someone could prefer it as the tidier long-term model; the design flagged it as better suited for a later version if needed.
- **Option C — store content inside the parse cache:** Persist file content in `cache.json` so cache hits need no read at all. Pros: zero reads on a full cache hit. Cons: bloats the on-disk cache significantly. Someone could prefer it to fully eliminate reads, but the design rejected it on cache-size grounds.

## Decision

Adopt Option A: `parse_with_cache` returns a content map alongside parse results, and `CodebaseIndex::build_with_content()` consumes it instead of calling `read_to_string`. The original `build()` is retained for back-compat. Content is NOT stored in the cache (avoids bloat); on a cache miss the already-read content is reused, on a cache hit one read is still done.

## Consequences

### Positive
- Eliminates all cache-miss double reads (the expensive path).
- The same `CodebaseIndex` content-aware indexing path underpinned daemon in-memory indexing added in v0.8.0.

### Negative
- Callers (overview, trace, diff) must destructure and thread the content map.

### Neutral
- Two build entry points (`build`, `build_with_content`) coexist.
- A cache hit still incurs one read since content is not cached.

## Revisit if
- Cache-hit reads become a measured bottleneck (would force caching content despite bloat).
- `build()` and `build_with_content()` divergence causes maintenance drift.

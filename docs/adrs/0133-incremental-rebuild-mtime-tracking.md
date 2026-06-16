---
id: '0133'
title: Incremental index rebuild via mtime/size tracking on IndexedFile
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.2.0 adding incremental indexing so the daemon/watcher re-parses only changed files
loop: implementation
---

# ADR-0133: Incremental index rebuild via mtime/size tracking on IndexedFile

## Context

Shipped in v1.2.0. Full re-parsing on every change is wasteful for a long-running `serve`/daemon process. The index needed a cheap way to detect which files actually changed between scans so the watcher can refresh without re-parsing the whole tree.

## Options considered

- **Option A — mtime/size tracking on IndexedFile:** Store a Unix-epoch `mtime_secs` (and reuse `size_bytes`) on each `IndexedFile`, populated from `std::fs::metadata` during build. `incremental_rebuild` upserts files whose mtime increased or whose size differs, removes deleted ones, then rebuilds graph/pagerank/test_map. Pros: change detection is cheap (a single `metadata` call per file, no content read); falls back to always-reparse when mtime is unavailable. Cons: mtime granularity is one second and can be unreliable on some filesystems / for fast edits — mitigated by the size check. This is what shipped.
- **Option B — content hashing:** A reasonable alternative would have been to hash each file's bytes and compare against a stored hash. It would catch changes mtime misses (same-second edits, mtime-preserving writes), but it requires reading every file's full content on each scan, which defeats the cheapness goal that motivated incremental indexing. Reconstructed alternative; not formally evaluated in the plan.

## Decision

Add `mtime_secs: Option<u64>` to `IndexedFile`, populated from `std::fs::metadata` during `build`/`build_with_content`. Add `incremental_rebuild(current_files, parse_results, counter)` that removes files no longer present, upserts files whose mtime increased or whose `size_bytes` differs (`(Some(old), Some(new)) => new > old || file.size_bytes != existing.size_bytes`, with `_ => true` meaning "no mtime available: always re-parse"), then rebuilds the dependency graph, PageRank, and test_map. A regression test (`test_incremental_rebuild_removes_deleted_file`) asserts that `incremental_rebuild` drops files no longer present and leaves the correct file count.

## Consequences

### Positive
- Daemon/LSP can refresh the index cheaply on watcher events.
- Falls back safely to a full re-parse when mtime is missing.
- Regression-tested for correct deletion handling and file count.

### Negative
- mtime granularity (1s) can miss sub-second edits — mitigated by the size check.
- Derived scores (PageRank, test_map) are still fully recomputed after any change.

### Neutral
- The mtime/size incremental-update pattern is used by the LSP/serve file-watcher loop via `process_watcher_changes -> apply_incremental_update` (in `src/commands/serve.rs`); `CodebaseIndex::incremental_rebuild` itself is currently only exercised by its unit test.

## Revisit if
- mtime-based detection misses edits in practice (move to content hashing).
- Recomputing PageRank/test_map on every change becomes a bottleneck.

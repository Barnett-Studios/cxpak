---
id: '0204'
title: Watcher bounded memory — drop ignored paths, guard the clone, parse each file once
status: ACCEPTED
date: 2026-07-19
triggered_by: field report — `cxpak serve --mcp` grew to 6 GB RSS and pegged a CPU core for hours; killed manually
loop: implementation
---

# ADR-0204: Watcher bounded memory — drop ignored paths, guard the clone, parse each file once

## Context

A long-running `cxpak serve --mcp` process was observed growing to ~6 GB RSS with one
core pegged for ~8 hours before being killed. A CPU sample showed the freshness-watcher
thread spending ~100% of time in `intelligence::call_graph::build_call_graph`
(`ts_parser_parse` / `ts_tree_delete`), parsing **TypeScript** in a Rust-majority repo.

Three compounding defects in the watcher rebuild path (`spawn_mcp_watcher` →
`process_watcher_changes` → `rebuild_graph_delta` → `build_call_graph`) explain both the
runaway memory and the pegged CPU:

1. **The watcher ingested git-ignored files.** The initial index is built from
   git-tracked files only (`git_tracked_files`, `include_ignored(false)`), but
   `classify_changes` applied no ignore filter — every path under the watched root
   (`target/`, `.cxpak/cache/`, `.git/`, `node_modules/`, vendored/generated trees) was
   fed to `apply_incremental_update`, which `read_to_string` + `upsert_file`d it into the
   index. Any `cargo build`, git operation, or cache write therefore grew `index.files`
   without bound and re-triggered rebuilds indefinitely. The TypeScript in the sample was
   a large vendored/generated file under an ignored tree that should never have entered
   the index.

2. **`build_call_graph` re-parsed each file once per symbol.** For every symbol in a
   file it called `extract_call_sites_from_source(&file.content, …)`, which spun up a
   fresh tree-sitter parser and parsed the entire file from scratch — `Σ(symbols)` full
   parses and tree walks per file, quadratic on large files. Applied to a wrongly-ingested
   multi-thousand-symbol generated file, this is the CPU explosion the sample captured.

3. **The full-index deep clone ran before the no-op check.** `process_watcher_changes`
   deep-cloned the entire `CodebaseIndex` on *every* watcher wake, then discovered there
   was nothing to do (`update_count == 0`) and discarded it — churning hundreds of MB per
   spurious event even when every path was later filtered out.

These are engineering defects with an unambiguous correct fix; the ADR records the
chosen shape and the rejected alternatives, not a genuine either/or product decision.

## Options considered

- **Option A — filter at classification + guard the clone + parse once (chosen):** drop
  git-ignored paths in `classify_changes` (via `git2::is_path_ignored`, plus an explicit
  `.git/` guard), early-return in `process_watcher_changes` before the clone when no
  relevant path changed, and rebuild the call graph with one parse per file. Fixes the
  unbounded growth at its source (the index only ever holds tracked files), removes the
  needless clone, and collapses the parse count from `Σ(symbols)` to `files`.
- **Option B — cap the index size / evict:** bound `index.files` and evict least-recently-
  seen entries. Treats the symptom, not the cause: the index would still ingest junk, the
  call graph would still churn, and eviction would silently drop real files under load.
  Someone might prefer it as a generic safety net, but it hides the actual bug.
- **Option C — debounce harder / coalesce more aggressively:** widen the debounce window
  so build storms collapse into fewer rebuilds. Reduces frequency but not the per-rebuild
  cost or the unbounded ingestion; a slow steady drip of ignored writes still grows the
  index forever. Attractive because it is a one-line change, but it does not bound memory.

## Decision

Adopt Option A. `classify_changes` filters git-ignored paths (fail-open if the repo can't
be opened, preserving prior behaviour and existing tests); `process_watcher_changes`
returns before the deep clone when both change sets are empty; `build_call_graph` parses
each file exactly once via `extract_calls_by_symbol`, set-equivalent to the old per-symbol
path. The call-graph change is covered by `test_calls_by_symbol_matches_per_symbol_rust`;
the watcher changes by `test_classify_changes_skips_git_ignored` and
`test_process_watcher_changes_ignored_only_skips_rebuild`. Shipped in 3.1.2.

## Consequences

### Positive
- Watcher memory is bounded by the tracked-file set; `target/` / `.cxpak/` / `.git/` churn
  no longer grows the index or triggers rebuilds.
- Call-graph rebuild cost drops from `Σ(symbols)` parses to `files` parses — orders of
  magnitude fewer tree-sitter parses and far less allocation churn per rebuild.
- Spurious watcher wakes no longer deep-clone the full index.

### Negative
- `classify_changes` now opens the git repo per debounced batch and runs an ignore check
  per path — negligible next to the clone it prevents, but not free.
- A brand-new source file that is *untracked but not ignored* is still picked up (correct),
  so "ignored" — not "tracked" — is the boundary; a file force-added despite an ignore rule
  will not be watched until the ignore rule is removed.

### Neutral
- Call-graph output is unchanged (set-parity), so downstream ranking/prediction is
  unaffected.
- The `update_count == 0` post-apply guard is retained for the case where paths survive
  filtering but nothing actually changed on disk (mtime-identical writes).

## Revisit if

- A future mode needs to index untracked/ignored files deliberately (e.g. generated code
  the user wants in context) — the ignore filter would need an opt-out.
- The per-batch `git2::Repository::open` + ignore checks show up in profiles under
  extreme event rates — cache the repo handle / ignore matcher across the watcher's life.
- The call graph is made incrementally updatable per changed file (today `rebuild_graph_delta`
  still recomputes the whole call graph); that would supersede the "parse every file once
  per rebuild" cost recorded here with "parse only changed files".

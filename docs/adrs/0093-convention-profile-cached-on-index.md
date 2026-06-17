---
id: '0093'
title: Convention profile built at index time and cached on CodebaseIndex with incremental updates
status: ACCEPTED
date: 2026-03-27
triggered_by: v1.1.0 Repository DNA design — need evidence-based convention extraction available to auto_context
loop: planning
---

# ADR-0093: Convention profile built at index time and cached on CodebaseIndex with incremental updates

## Context

Released in v1.1.0 (Repository DNA). `auto_context` is the primary tool and must include a Repository DNA section on every call. Convention extraction reads symbols, the dependency graph, the `test_map`, and git history to quantify the codebase's actual patterns. The question was whether to compute this profile lazily on each tool call or to precompute and cache it on the index.

The hot path is `auto_context`, so per-call recomputation cost matters. Git health is special: it requires walking `git log`, which cannot be derived from the in-memory index alone.

## Options considered

- **Option A — Build at index time, cache on `CodebaseIndex`, incremental delta updates (chosen):** Populate `pub conventions: ConventionProfile` after `CodebaseIndex::build()`, and refresh incrementally from the file watcher by re-extracting only changed files (subtract the old file's contribution, add the new). Pros: available to the hot `auto_context` path at zero per-call cost; consistent with how schema and graph are handled; updates are `O(changed files)`. Cons: per-file contribution maps add memory (~100 bytes/file/category); `git_health` cannot be updated incrementally because it needs `git log`. Someone could prefer this for the zero-cost hot path and consistency with existing index construction.
- **Option B — Live re-scan on every tool call:** Recompute conventions fresh whenever `cxpak_verify` or `auto_context` runs. Pros: always fresh, no stale-cache risk, no per-file contribution bookkeeping. Cons: too expensive for the primary `auto_context` path; redundant work on unchanged files. Someone could prefer it to avoid the contribution-map memory and any staleness window.

## Decision

Build the convention profile after index construction via `build_convention_profile(&index, repo_path)`, store it on `CodebaseIndex` as `pub conventions: ConventionProfile`, and update it incrementally from the file watcher per-category (subtract old file contribution, add new). `git_health` is exempted from incremental update and instead refreshed on `verify`/`conventions` calls with a 60-second TTL cache. It is built after `build()` because `git_health` needs a `repo_path` that `build()` does not receive — the same pattern already used for schema and graph.

## Consequences

### Positive
- The DNA section costs nothing per `auto_context` call.
- Incremental updates are `O(changed files)`.
- Verify checks against the same cached profile the LLM was shown, creating a consistent contract.

### Negative
- Per-file contribution maps add ~100 bytes/file/category memory.
- `git_health` has a staleness window of up to 60 seconds.
- Three struct construction sites in `index/mod.rs` (`build`, `build_with_content`, `empty`) must each initialize the field.

### Neutral
- The profile is `#[serde(skip)]` on the `file_contributions` map, so it is not persisted.

## Revisit if
- Per-file contribution map memory becomes significant for very large repos.
- The `git_health` 60s TTL proves too stale or too expensive.

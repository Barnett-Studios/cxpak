---
id: '0145'
title: Time Machine snapshots from lightweight tree-walk deltas (no re-parse), sampled and cached
status: ACCEPTED
date: 2026-04-01
triggered_by: Time Machine view performance (Task 11)
loop: implementation
---

# ADR-0145: Time Machine snapshots from lightweight tree-walk deltas (no re-parse), sampled and cached

## Context

Shipped in v2.0.0. Visualizing architecture history (the Time Machine view) could require re-parsing the entire codebase at every historical commit, which is prohibitively expensive over long git histories. Task 11 needed a cheaper snapshot model that keeps the history view fast and cacheable while still surfacing meaningful architectural deltas across time.

## Options considered

- **Option A — Lightweight sampled snapshots, cached to `.cxpak/timeline/`:** Each `TimelineSnapshot` stores a file list, an edge count, module count, and health/cycle fields (~5KB, no full parse state). Sampling bounds the number of snapshots. Pros: fast, bounded snapshot size, avoids a full re-parse per commit, cached across runs. Cons: edges and per-file imports are heuristic rather than parser-exact; sampling loses per-commit granularity. This is the design the plan documented and the shipped option, though the shipped implementation diverged from the plan's prose on three specifics (see Decision). Someone could prefer it for any realistic repo where re-parsing every commit is intractable.
- **Option B — Full re-parse at each historical commit:** Check out and parse each sampled commit to recover the exact symbol/import graph. Pros: accurate graph at every snapshot. Cons: very slow, large memory and disk cost, impractical for long histories. A reasonable alternative would have been this when absolute graph fidelity per commit matters more than speed; it was not adopted because the cost is unbounded with history length.

## Decision

`compute_timeline_snapshots()` walks `git log`, samples up to `max_snapshots` commits via an even stride (`total.div_ceil(max_snapshots)`, then keeping commits where `i % stride == 0`), and for each sampled commit walks that commit's full git tree (`list_tree_files`) to build a lightweight ~5KB snapshot — file list, a directory-co-location heuristic `edge_count` (same-directory file pairs), `module_count`, and health/cycle fields — without re-parsing source. Snapshots are cached to `.cxpak/timeline/snapshots.json`.

Plan-vs-ship divergences (the plan's prose described intent that the shipped code did not implement exactly):

- The plan specified "sample every 5th commit or weekly, whichever yields fewer." The shipped code uses an even stride instead; there is no "every 5th" constant and no weekly/date-based sampling.
- The plan noted a default cap of 100. The shipped `max_snapshots` is a required parameter with no default wired in at the production caller (`commands/visual.rs` only loads cached snapshots).
- The plan described extracting "architecture diffs from git diff deltas." The shipped code does not use `git diff`: it walks each commit's full tree and leaves `SnapshotFile.imports` empty; `edge_count` is a directory-co-location heuristic, not a diff delta.

## Consequences

### Positive
- History view is cheap and cacheable; no per-commit re-parse.
- Snapshot size is bounded (~5KB each), so even longer histories stay small on disk.

### Negative
- Edges are a directory-co-location heuristic (same-dir file pairs), not parser-derived; per-file imports are not populated (`SnapshotFile.imports` is always empty).
- Sampling drops intermediate commits, so the timeline is not per-commit accurate.

### Neutral
- Key events (`CycleIntroduced`, `LargeChurn`, `HealthDropped`, and related variants in `render.rs`) are detected from snapshot deltas.

## Revisit if
- The directory-co-location edge heuristic proves too inaccurate for the timeline.
- Users need per-commit (unsampled) history.
- Anyone reconciling plan-vs-code wants the shipped sampling/edge heuristics brought back in line with the plan's "every 5th/weekly" sampling, default cap of 100, or true git-diff-derived edges.

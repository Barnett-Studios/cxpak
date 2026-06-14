---
id: '0120'
title: Architecture drift detection via persisted JSON snapshots and a stored baseline
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.4.0 architecture drift detection comparing the current architecture against past states
loop: implementation
---

# ADR-0120: Architecture drift detection via persisted JSON snapshots and a stored baseline

## Context

Implemented for v1.4.0. To detect architectural drift over time, the tool needs a record of past architecture metrics. cxpak is stateless per invocation, so historical comparison requires on-disk persistence. The design also needed timestamps for ordering snapshots, which introduces a date library.

## Options considered

- **Option A — persist timestamped JSON snapshots in `.cxpak/snapshots/` plus an explicit `.cxpak/baseline.json`, using chrono RFC3339 timestamps (chosen):** `snapshot_from_index` serializes module metrics; `build_drift_report` auto-saves a snapshot every call, optionally promotes it to a baseline, and compares current metrics against the baseline (deltas) and against historical snapshots (trend). Pros: enables drift over arbitrary time spans without a database; human-readable, diffable JSON; a baseline gives a deliberate reference point. Cons: adds the `chrono` dependency and writes to the working tree on every drift call.
- **Option B — derive history from git only:** A reasonable alternative would have been to reconstruct past architecture by checking out and re-indexing old commits. Pros: no new on-disk state. Cons: expensive (re-index per commit) and requires a git checkout side effect. Rejected — the cost and the checkout side effect are prohibitive for an on-demand command.

## Decision

Add `src/intelligence/drift.rs` with `ArchitectureSnapshot`/`ArchitectureMetrics` serialized to JSON. `build_drift_report` auto-saves a timestamped snapshot to `.cxpak/snapshots/` on every call, optionally promotes it to `.cxpak/baseline.json` (the `save_baseline` flag), and produces a `DriftReport` comparing current metrics against the baseline (deltas) and against historical snapshots (trend). Timestamps use `chrono::Utc::now().to_rfc3339()`, introducing `chrono` as a dependency. Snapshot filenames replace `:` with `-` for filesystem safety. Snapshot writes use an atomic tmp-file-plus-rename, and the snapshots directory is capped at 100 entries (oldest evicted).

## Consequences

### Positive
- Drift can be measured over any time span using accumulated snapshots.
- Human-readable, diffable JSON artifacts.
- An explicit baseline gives a deliberate comparison anchor.

### Negative
- Writes to `.cxpak/` on every drift call (working-tree side effect).
- Adds the `chrono` dependency.
- `cycle_count`/`boundary_violation_count` were placeholders (0) in the initial `snapshot_from_index`.

### Neutral
- The snapshots directory self-prunes at a fixed 100-entry cap.

## Revisit if
- The fixed 100-snapshot cap proves too small or too large, or a time-based retention policy is needed instead of the count-based one.
- Working-tree writes on every call become undesirable.

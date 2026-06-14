---
id: '0108'
title: Architecture drift uses stored snapshots plus a baseline, not git-diff reconstruction
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.4.0 architecture drift detection
loop: planning
---

# ADR-0108: Architecture drift uses stored snapshots plus a baseline, not git-diff reconstruction

## Context
v1.4.0 adds architecture drift detection, which requires comparing past and present architecture metrics. Reconstructing historical architecture by re-parsing every past commit is infeasible. The chosen approach stores cheap snapshots over time plus an explicit baseline, and diffs against those rather than rebuilding history from git.

## Options considered
- **Option A — Dual: stored `baseline.json` + auto-saved architecture snapshots:** save a baseline on the first `cxpak_drift` call or via `--save-baseline`, and auto-save lightweight architecture snapshots (module list + edge counts + metric values + timestamp, a few KB each) to `.cxpak/snapshots/`. A time-window trend then diffs stored snapshots against each other. Pros: no re-parsing of history; snapshots are cheap; supports both an explicit baseline comparison and a rolling trend. Cons: a trend needs at least two accumulated snapshots; requires snapshot storage and bootstrap handling. (Grounded — this is the shipped design.)
- **Option B — Reconstruct architecture from git diffs at each historical commit:** walk git history and rebuild the architecture metrics per commit. Pros: no need to pre-store snapshots. Cons: infeasible without re-parsing every commit — the design doc explicitly rejects this. Rejected on cost. (Grounded — explicitly rejected as infeasible in the source.)

## Decision
Compute drift two ways:

1. **Baseline** — a stored `.cxpak/baseline.json` saved on the first `cxpak_drift` call or via `cxpak drift --save-baseline`. Reset by removing `.cxpak/` (e.g. `cxpak clean`).
2. **Time-window trend** — lightweight architecture snapshots (module list + edge counts + metric values + timestamp) auto-saved to `.cxpak/snapshots/`. This is explicitly **NOT** git-diff reconstruction, which is infeasible without re-parsing.

Shipped behavior (differs from the design doc spec — see below):
- The trend diffs the **newest** stored snapshot against the **oldest** stored snapshot. The window labels `"30 days"` / `"30-180 days ago"` are emitted but the code does **not** filter snapshots by age.
- The trend returns null when fewer than 2 snapshots exist — a **count** threshold, not an age/day threshold. There is no "younger than 30 days" check.
- Snapshots are auto-saved on each `cxpak_drift` call (inside `build_drift_report`). `cxpak overview` / index build do **not** save snapshots in the shipped code.

The 30/180-day windowing and the ">30 days of history" bootstrap message described in the design doc are aspirational spec that did not ship as written; the day-window strings in the output are cosmetic labels.

## Consequences
### Positive
- No re-parsing of history — snapshots are cheap (a few KB each).
- Supports both an explicit baseline comparison and a rolling newest-vs-oldest trend.

### Negative
- The trend is null until at least two snapshots accumulate (one per `cxpak_drift` call).
- Requires snapshot storage and bootstrap edge-case handling.
- The 30/180-day window labels are cosmetic and do not reflect real age filtering, which can mislead a reader who trusts the labels.

### Neutral
- Baseline and snapshots both live under `.cxpak/` and share that directory's lifecycle; `cxpak clean` resets both by wiping the directory.
- `build_drift_report` is wired uniformly into the `cxpak_drift` MCP tool, the `/v1/drift` HTTP route, and the `cxpak/drift` LSP method.

## Revisit if
- Snapshot storage grows unwieldy.
- Real age-based windowing is needed (the current 30/180-day labels are not backed by age filtering).
- Users need finer-grained drift windows than the newest-vs-oldest diff.

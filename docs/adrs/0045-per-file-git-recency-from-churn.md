---
id: '0045'
title: Per-file git recency derived from churn rank instead of a binary commit check
status: ACCEPTED
date: 2026-03-12
triggered_by: git_recency in v0.6.0 was 1.0 for every file in any active repo — a 0.3-weight noise term
loop: planning
---

# ADR-0045: Per-file git recency derived from churn rank instead of a binary commit check

## Context

Released in v0.6.1 (Bug 3). The v0.6.0 `git_recency` signal used `commits.first().map(|_| 1.0)`, a global scalar that resolved to 1.0 for every file in any active repo — it contributed 0.3 weight to every file equally, pure noise. The fix computes a per-file `recency_map` from each file's position in the (commit-count-descending) `file_churn` list. Confirmed shipped: `ranking.rs` line 55 uses `recency_map.get(path...)`.

## Options considered

- **Option A — churn-rank proxy `recency = 1.0 - (index / len)`:** Files higher in the commit-count-sorted churn list get higher recency; files absent from the list get 0.0. Pros: reuses existing per-file churn data, cheap, differentiates files so the 0.3 weight becomes real signal. Cons: conflates frequency-of-change with recency and uses churn ordering as a recency proxy rather than actual last-modified dates. Someone could prefer it because the churn data is already collected, so it adds no git traversal.
- **Option B — per-file last-commit timestamps:** A reasonable alternative would have been to walk git history to record each file's actual last-modified date and decay by age. Pros: true recency rather than a churn proxy. Cons: requires additional git traversal and is more expensive. Someone could prefer it for correctness; the design instead chose the cheaper churn-list proxy and only gestured at dates as a future refinement.
- **Option C — keep the binary check (status quo):** 1.0 if any commit exists. Pros: trivial. Cons: pure noise, identical for every file. Retained only as the baseline being replaced.

## Decision

Replace the binary recency with a per-file `recency_map`: for the `file_churn` list (sorted by commit count descending), `recency = 1.0 - (i / len)`; files not in the churn list get 0.0. This reuses existing churn data as a recency proxy rather than walking history for per-file last-modified dates.

## Consequences

### Positive
- The 0.3-weight recency term now differentiates files instead of being constant.
- No extra git traversal — reuses `file_churn` already collected.

### Negative
- Recency is a churn-rank proxy, not true last-modified-date recency; the design acknowledges a more sophisticated version would track per-file dates.

### Neutral
- `test_rank_files_no_graph_no_git` still passes (all 0.0 with no git context).

## Revisit if
- Distinguishing recency from churn frequency matters.
- Per-file last-modified dates become available cheaply.

---
id: '0164'
title: Ship v2.3.0 as a single additive release (incremental index + cost reporting + review diff), no feature gates
status: ACCEPTED
date: 2026-06-14
triggered_by: v2.3.0 brainstorm — three workstreams emerged (scale, cost, review); question was how to package and de-risk them
loop: planning
---

# ADR-0164: Ship v2.3.0 as a single additive release, no feature gates

## Context

The v2.3.0 brainstorm produced three workstreams: W1 incremental indexing (warm PageRank + edge-delta graph + persistent derived cache), W2 cost/efficiency reporting, W3 review-aware `diff`. Every change is **additive** to the public surface — new fields on `AutoContextResult`, a new `--review` flag, an internal cache layer — so none breaks the v2.x stable MCP/HTTP/LSP contract; a minor bump is semver-correct for any grouping.

The real question was risk, not versioning. W2 and W3 are low-risk (reporting over existing data; composing already-tested functions). W1 is the high-risk part: incremental/derived state can silently diverge from a full rebuild, which is the one failure a *context* tool cannot tolerate. The options below trade release cohesion against isolating W1's correctness runway.

## Options considered

- **Option A — one 2.3.0, everything, no gates (chosen):** ship W1+W2+W3 together; W1 correctness guaranteed by parity tests (delta == full rebuild) rather than deferred behind a flag. Pros: cohesive "incremental, accountable, review-aware" release, one migration, one `CACHE_VERSION` bump. Cons: the cheap wins (W2/W3) wait on W1's runway, and a W1 regression raises the whole release's blast radius. A stakeholder could prefer this for a clean, single story.
- **Option B — phased minors (2.3 cost → 2.4 review → 2.5 scale):** isolate W1 as its own release. Pros: ship value fast, contain W1 risk. Cons: three releases, three migrations; the "incremental" headline slips. A stakeholder could prefer this to de-risk delivery.
- **Option C — one 2.3.0 with W1 behind an opt-in flag:** ship W2/W3 on, W1 off-by-default until proven. Pros: single release + isolated risk. Cons: a flag implies we don't trust our own parity tests; dead-code paths; promotion is a second decision. A stakeholder could prefer this for caution.

## Decision

Option A. Single `v2.3.0`, all three workstreams, no gates. Correctness for W1 is enforced by the parity test suite (see ADR-0166) as the definition of done — not deferred behind a flag.

## Consequences

### Positive
- One cohesive release and a single `CACHE_VERSION` migration.
- No flag-managed dead code; the incremental path is the path.

### Negative
- W2/W3 ship only when W1's parity suite is green; a W1 issue blocks the tag.
- Larger blast radius per release than a phased rollout.

### Neutral
- Implementation order is W2 → W3 → W1 (risk-ascending), all merged before the tag.

## Revisit if
- W1 parity proves materially harder than estimated and threatens the release window — fall back to Option B and split W1 into 2.4.0.
- A future workstream in the same release *would* break the public API — then it is a major bump, not a minor, and this packaging decision is reopened.

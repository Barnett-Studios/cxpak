---
id: '0178'
title: Hybrid interaction model: bounded embed + opt-in Live serve; add `/v1/diff`
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0178: Hybrid interaction model: bounded embed + opt-in Live serve; add `/v1/diff`

**Context.** The self-contained artifact can only answer questions the generator embedded — hence the dead Flow/Diff tabs. Users want arbitrary symbol/ref queries, but the "no network at runtime" invariant is sacred.

**Options considered.**
1. *Pure static.* Rejected — the dead-tab problem is intrinsic.
2. *Always requires `cxpak serve`.* Rejected — kills the self-contained deliverable.
3. *Hybrid: bounded deterministic answer-surface embedded by default + an opt-in "Live" toggle to a user-started server.*

**Decision.** Option 3. Base artifact embeds a bounded answer-surface (top-K symbols by PageRank for Flow, last-N snapshots for Timeline, one default Diff + last-~10 commits) — all budget-capped and deterministic. A **user-confirmed** "Live" toggle to a `cxpak serve` URL the user started unlocks arbitrary queries via `/v1/data_flow` and a **new `/v1/diff`** route (today only an unversioned `/diff` exists). Reading of the invariant: "no *external CDN*/network at runtime" = "no *unsolicited* network"; a user-initiated localhost call to a server the user launched is permitted. **The base file's bytes (and the golden fixture) are unaffected by whether Live is ever toggled.** Out-of-embedded-set picks with Live off show an actionable message, never a bare CLI-nag.

**Consequences.** Self-contained default preserved; power users get unbounded queries. `/v1/diff` inherits the existing bearer-gated, timing-safe, `route_layer(auth_layer)` pattern (ADR-0110/0146). New test obligation: no network call fires before the user toggles Live. **New persisted artifact:** wiring the Timeline embed requires actually writing the `.cxpak/timeline/` snapshot cache (`compute_timeline_snapshots` → `save_snapshots`, today caller-less) + backfilling `TimelineSnapshot.health_composite`/`circular_dep_count` — a new on-disk write path introduced here (see spec §14 MVP #1); it is git-ignorable cache, not a source artifact.

**Revisit if.** Browser CSP/security posture makes even user-initiated localhost calls untenable → fall back to a "re-run the CLI with these args" deep link (degrade to static, never silently fail).

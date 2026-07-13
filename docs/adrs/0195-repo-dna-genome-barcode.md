---
id: '0195'
title: Repo-DNA genome-barcode signature
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0195: Repo-DNA genome-barcode signature

**Context.** The Overview/History/Diff surfaces want an identity mark that is (a) deterministic (same repo → identical image), (b) discriminating (two repos look different), (c) animatable across commits (drift). Node-link graphs fail (a) and (b) at a glance.

**Options considered.**
1. *A node-link mini-map as the signature.* Rejected — hairball, non-discriminating, layout-sensitive.
2. *A containment/circle-packing overview.* Good for shape, weaker as a compact repeatable "barcode".
3. *A two-track genome barcode.*

**Decision.** Option 3. Track 1 (conventions genome): one band per `ConventionProfile` axis (naming/imports/errors/deps/testing/visibility/functions/git-health), height/opacity = `observation.percentage`, solidity = `PatternStrength` on its 3-level scale — Convention (≥90%, solid) → Trend (70–89%) → Mixed (50–69%, faint); an axis with no dominant observation renders as a **gap** (absence, not a strength level — there is no `None` variant, `conventions/mod.rs:19-23`). Track 2 (structure genome): PageRank-sorted file-importance spine sparkline, overlaid with cycle + dead-symbol ticks. All drivers deterministic. (Containment-as-overview from option 2 is kept as a *separate* Phase-3 experiment for the Explore default, adjudicated by the recall/usability lens — see spec §16.)

**Consequences.** A compact, deterministic, animatable fingerprint reusable in three surfaces. Requires the DNA render to obey the fixed-point coordinate rule (ADR-0196) so it stays byte-stable cross-platform.

**Revisit if.** User testing shows the barcode reads as decoration rather than signal → fall back to the containment overview as the primary identity mark.

---
id: '0180'
title: Broaden Visual round-trip conformance; no-dead-nav + provenance-completeness gates
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0180: Broaden Visual round-trip conformance; no-dead-nav + provenance-completeness gates

**Context.** ADR-0153 requires each surface's data to equal the core's. A real but narrow Visual round-trip exists — `tests/cross_channel_consistency.rs:423-449` asserts the SPA health composite bit-for-bit against `health_cached().composite` (it is *not* a stub). The redesign adds many new displayed numbers (risk percentiles, insights, DNA) and restructures nav.

**Options considered.**
1. *Trust the renderers; spot-check manually.* Rejected — the whole thesis is auditable determinism; the tests must embody it.
2. *Extend the existing conformance test to the new surfaces + add structural gates.*

**Decision.** Option 2. (a) Broaden the `cross_channel_consistency.rs` Visual arm from health-only to the Overview/Explore risk, pagerank, and insight payloads — every number shown equals its core output. (b) **No-dead-nav gate:** an automated check that Overview/Explore/History all render non-empty on the fixture repo (Flow-as-action, Timeline wired per spec §14 MVP #1, Diff default-embedded). (c) **Provenance-completeness gate:** every displayed score exposes a derivation (start with Overview risk + alerts, expand per phase — ADR-0174). (d) Palette-determinism assertion: no `--palette` → byte-identical to golden; `--palette X` differs only in the initial token block.

**Consequences.** Regressions in data fidelity, dead surfaces, or un-provable numbers fail CI. Test-authoring cost scales with each new displayed signal — acceptable and on-thesis. Determinism assertions stay macOS-gated per ADR-0177's platform note.

**Revisit if.** The conformance test becomes a maintenance drag disproportionate to the bugs it catches → generate the assertions from a single core→view field manifest rather than hand-writing each.

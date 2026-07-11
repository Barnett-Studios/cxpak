---
id: '0192'
title: Collapse 6 tabs into 3 modes; Flow becomes an inspector action
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0192: Collapse 6 tabs into 3 modes; Flow becomes an inspector action

**Context.** Today's SPA has 6 nav tabs. Three (Flow/Timeline/Diff) render CLI-nag empty states because the page can only show data the generating CLI embedded (`spa.rs:77-78` hardwire `flow_json`/`diff_json` to `null`; empty-state markup at `spa.rs:282-293`). The three "good" tabs overlap conceptually (Dashboard = readout, Architecture + Risk = the same spatial view with different color).

**Options considered.**
1. *Keep 6 tabs, just fill the dead ones.* Rejected — preserves conceptual redundancy (two spatial tabs) and the nav still advertises modes that are often empty.
2. *3 modes* — Overview (readout), Explore (one spatial canvas + lens toggle), History (one time-scrubber) — with Flow demoted to a contextual inspector action.

**Decision.** Option 2. Overview = Dashboard. Explore = Architecture + Risk unified under a `Dependencies·Risk·Coverage·Churn·Security` **lens toggle** (lens changes encoding only, never layout; default lens = the strongest existing view, the Risk treemap). History = Timeline + Diff under one scrubber. Flow stops being a nav item and becomes a "Trace data flow" action on a selected symbol — never a dead tab. Shared selection state (`CX.app.scope`) + a single URL-hash addressing scheme across all modes; a persistent breadcrumb strip (reuse `ArchitectureExplorerData.breadcrumbs`) and command palette (over the existing PageRank-ranked `SearchIndex`) in every mode.

**Consequences.** No nav item is ever a dead end (enforced by the no-dead-nav gate, ADR-0199). Fewer top-level surfaces to build well. Migration risk: existing deep-links / muscle memory to the 6 tabs break — acceptable for a major visual release. Lens infra is new but small (a color/size re-encode over a fixed layout).

**Revisit if.** A lens needs a *different layout* (not just re-encoding) — then it earns its own mode rather than a toggle.

## 3.1.0 implementation note (Blueprint redesign)

The three-mode nav shipped as **Overview / Explore / History**, but Flow and Diff
were **removed from the SPA outright** rather than demoted to a Flow-inspector
action / a Diff lens under History. Rationale: the SPA embeds only the data the
generating CLI serialised, and both `flow_json` and `diff_json` are hardwired to
`null` in every SPA render (they require `--symbol` / `--files` CLI params that the
`all` render does not supply). A "Trace data flow" inspector or an in-History Diff
built on absent data would have to **fabricate** a flow/diff — which violates the
release's hard integrity constraint (every rendered figure must trace to a real
computation). Flow and Diff remain fully available on the standalone
`cxpak visual --visual-type flow|diff` render path, which receives the params.
History therefore shows the Timeline scrubber only. No nav item is a dead end
(the removed tabs are simply gone, satisfying the no-dead-nav intent).

**Revisit if.** The SPA gains a way to carry real per-symbol flow data (or computes
a neighbourhood from the already-embedded dependency graph) — then Flow returns as
the "Trace data flow" inspector action this ADR originally specified, on real data.

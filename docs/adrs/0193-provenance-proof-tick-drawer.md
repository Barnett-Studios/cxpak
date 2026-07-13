---
id: '0193'
title: Provenance system: the proof-tick motif + the "prove-it" drawer
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0193: Provenance system: the proof-tick motif + the "prove-it" drawer

**Context.** cxpak's banner is "every edge proven, never inferred" (ADR-0097 descriptive-only). Competitors (graphify) narrate via LLM inference. The differentiator is to make cxpak's determinism *auditable in the UI itself* — the one thing an LLM-summarizing tool structurally cannot copy.

**Options considered.**
1. *A legend explaining proven-vs-inferred.* Weak — legends are ignored; it's a label, not a property of the mark.
2. *Encode provenance into the mark geometry + expose the literal derivation on demand.*

**Decision.** Option 2, two halves. (a) **Proof-tick visual atom:** a directly-computed relationship (import/FK/call/confirmed-AST-diff) = solid line + short perpendicular hash at midpoint + monospace datum tag; a derived/correlational one (co-change, predicted risk, heuristic rename) = dashed, no tick, `~`-prefixed. One motif reused verbatim across every view (graph edges, treemap borders, alert icons, diff gutters, timeline connectors, flow paths). (b) **"Prove-it" drawer:** every score/edge/cell exposes a *prove* affordance (select + `p`) opening an inspector Provenance tab that shows the *literal derivation with substituted numbers* — e.g. `risk 0.0402 = churn(0.51) × blast(0.079) × test_penalty(1.0)` + percentile; a schema edge → the literal FK/migration line (carried by `schema::link::build_schema_edges`, which today is discarded at graph-edge time — stop discarding it); a churn number → the contributing commit list; a dead-code flag → the exact call-graph absence.

**Consequences.** The banner becomes geometry + interaction, not marketing. Requires threading provenance metadata (formula terms, source spans, commit lists) from core through to the render payload — a real data-plumbing cost, phased (start with Overview's risk table + alerts, expand per lens). Every displayed score must be able to answer "prove it" (ADR-0199 completeness gate). No LLM anywhere (ADR-0097 upheld).

**Revisit if.** The provenance payload materially inflates the artifact past its size budget → lazy-load derivations via the Live channel (ADR-0197) for the long tail while keeping headline derivations inline.

---
id: '0194'
title: Deterministic interpretive layer (proven analogues of LLM narration)
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0194: Deterministic interpretive layer (proven analogues of LLM narration)

**Context.** graphify's perceived magic is narration: god-nodes, "surprising connections", community labels — mostly LLM-inferred. cxpak must match the *punch* while keeping every claim proven and descriptive-only (ADR-0097).

**Options considered.**
1. *Add an LLM narration path.* Rejected outright — violates the no-LLM invariant and the entire thesis.
2. *Compute proven analogues from existing deterministic signals.*

**Decision.** Option 2. Ship 10 insights, each from an existing signal (god-nodes ← `pagerank.rs::compute_pagerank` + `architecture.rs::detect_god_files`; **surprising connections ← `index.co_changes` set-minus Import edges** — computed today, surfaced by nothing, so ship first, labeled `~ estimated`; cross-layer ← schema↔code edges `schema/link.rs`; blast-radius narration ← `blast_radius.rs`; danger-zone ← high-blast×zero-tests `risk.rs`; proven cycles ← Tarjan `architecture.rs::circular_deps`; DNA ← `ConventionProfile`/`render_dna_section`; drift ← `drift.rs`; predict ← `predict.rs`; exposure ← `security.rs`). Overview shows the top 5 as always-on headlines. Every headline deep-links into Explore — none is a dead end.

**Consequences.** Narrative parity with an LLM tool, zero inference. Honest labeling: correlational insights (co-change) are marked `~`, keeping the proof-tick contract (ADR-0193) intact. There is **no `EdgeType::CoChange` graph arm** to reuse — co-change lives only on `index.co_changes: Vec<CoChangeEdge>`; the insight computes the set-difference fresh (small, deterministic).

**Revisit if.** A high-value insight genuinely requires a signal cxpak doesn't compute deterministically → add the *deterministic* computation, never an inference shortcut.

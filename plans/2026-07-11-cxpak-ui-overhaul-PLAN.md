---
title: cxpak UI Overhaul — Implementation Plan
status: DRAFT (pre-validation)
created: 2026-07-11
updated: 2026-07-11
spec: 2026-07-11-cxpak-ui-overhaul-SPEC.md
adrs: 2026-07-11-cxpak-ui-overhaul-ADRS.md (0172–0180)
complexity: High (multi-phase, spans Rust core + inlined-JS SPA)
risk: Medium (determinism boundary + a cross-platform float-geometry hazard)
---

# Plan: cxpak UI Overhaul

## Executive summary

Full-overhaul of the `visual/` surface delivered in three phases. **MVP is 6 slices** (spec §14) chosen for the biggest perception-shift per unit effort; each is small, deterministic, and independently shippable. Phases 2–3 are scoped at task level and **re-planned to node granularity when reached** (over-specifying deferred work is waste). Rust core + single-file inlined-JS SPA, no new dependencies, no LLM anywhere, byte-identical within a platform.

## 🚨 Critical implementation standards (non-negotiable gates)

| Gate | Rule |
|---|---|
| Determinism | Generated HTML byte-identical within a platform; golden fixture `tests/spa_determinism.rs` (macOS-gated) stays green. New Rust float geometry rounds to a fixed grid before serialization (ADR-0177). |
| No external origin | `!html.contains("cdn.jsdelivr.net")` (+ unpkg/cdnjs) must hold — asserted at `render.rs:3215`, `tests/spa_render.rs:93`, `tests/visual_cli.rs:405`. No webfont link. |
| No LLM | Every surfaced value is a proven computation (ADR-0097). No inference path. |
| No new deps | Reuse inlined D3 v7 (`assets/d3-bundle.min.js`), existing crates. |
| Coverage | ≥90% on new/changed code (tarpaulin CI gate). Tests authored with code. |
| Palette determinism | Palette is client-side runtime state; no `--palette` → byte-identical to golden; picker never changes emitted bytes. |
| No dead nav | Overview/Explore/History all render non-empty on the fixture repo. |

## Current-state analysis (verified against code)

| Area | State | Evidence |
|---|---|---|
| SPA data wiring | Dashboard/Risk/Architecture wired to real builders; Flow/Diff hardwired `null`; Timeline reads a cache nothing writes | `spa.rs:32,37,58` vs `spa.rs:68-78`, empty-state `spa.rs:282-293` |
| Timeline compute | `compute_timeline_snapshots` (`timeline.rs:38`) + `save_snapshots` have **zero non-test callers**; `health_composite`/`circular_dep_count` hardcoded `None`/`0` (`timeline.rs:128-129`) | — |
| Risk scale collapse | `risk = nc*nb*tc_term` (`risk.rs:105`), `norm_blast=dependents/total_files` (`:88`) → max ~0.04; ramp `domain([0,0.4,0.7,1.0])` collapses to band 1 (`render.rs:620`); opacity kludge (`render.rs:635`) | — |
| Risk ranking | `compute_risk_ranking` already deterministic (path tie-break `risk.rs:121-126`); `norm_churn` already a percentile (`:66-72`) | — |
| Co-change | `index.co_changes: Vec<CoChangeEdge>` (`index/mod.rs:34`) populated at `visual.rs:81`; surfaced by **no** renderer; **no `EdgeType::CoChange` arm** | — |
| Layout | Sugiyama engine present (`layout.rs`); D3 v7 inlined; `group_into_phases` (`onboarding.rs:176`), `collapse_passthrough_chains` (`render.rs:2157`) real | — |
| Conformance | Real health-only Visual round-trip (not a stub) `cross_channel_consistency.rs:423-449` | — |
| `/v1` | `/v1/data_flow` exists; `/v1/diff` does not (only unversioned `/diff`); bearer auth `route_layer(auth_layer)` `serve.rs:471` (ADR-0110/0146) | — |

## Target architecture

Defer to the validated **spec** (3-mode IA, proof-tick provenance, DNA barcode, Canvas-over-precomputed, hybrid Live, palette system) and **ADRs 0172–0180**. This plan only sequences and decomposes the work.

---

## Implementation phases

### Phase MVP — 6 slices (highest perception-shift per effort)

Order is dependency-sorted: **N0 (test scaffolding) lands first — it gates every local:true node**; then data/core nodes (N1, N2, N7) before the JS that consumes them (N5, N6, N8); N4 (timeline wire+backfill) and N9 (IA restructure) are the cross-cutting closers.

- **Slice 0 — Test scaffolding** (N0): the `index_with()` builder + fixture helpers. Prerequisite for the entire RED gate.
- **Slice A — Surprising-co-change insight** (N1 core → N-JS surface). Flagship; currently invisible.
- **Slice B — Risk scale-collapse fix** (N2 percentile field → N5 JS ramp/legend).
- **Slice C — Palette system + picker** (N6, Tokyo Night default). Determinism-neutral, high visible impact.
- **Slice D — Timeline wiring** (N4: wire+persist+per-snapshot backfill; former N3 merged in). Removes a dead tab.
- **Slice E — Provenance drawer** (N7 risk-term payload → N8 drawer UI) on Overview risk + alerts.
- **Slice F — Explore unification** (N9): one canvas, Dependencies+Risk lenses, module-level collapse.

### Phase 2 (re-plan to nodes when reached)
History scrubber (Timeline+Diff merge) + default embedded diff; Coverage/Churn/Security lenses; adjacency-matrix toggle; repo-DNA barcode; bounded Flow-as-inspector-action. Depends on: N4 (timeline wired), N9 (Explore canvas), N6 (palette).

### Phase 3 (re-plan to nodes when reached)
Live toggle + new `/v1/diff` (bearer-gated); full provenance coverage across every score; containment-as-overview; edge-bundling + semantic-zoom (Rust precompute, fixed-grid rounding per ADR-0177).

---

## Execution manifest (MVP)

Schema per `code-plan/references/manifest-template.md`. `local: true` = single-region, RED test authored & committed up front, implementation delegated to the local cascade. `local: false` = cross-cutting/risky, implemented by Opus.

```yaml
manifest:
  base_branch: 3.1-ui-overhaul   # new worktree off main (worktree-per-workstream); root stays on main
  nodes:

    - id: N0-test-support
      local: false   # PREREQUISITE — the shared test scaffolding every RED test compiles against; author FIRST
      region: src/test_support/mod.rs (new, `#[cfg(any(test, feature="test-support"))]`) + `pub mod test_support;` in src/lib.rs + tests/common/mod.rs
      change: >
        Author the shared test scaffolding the RED gate needs (none of this exists today —
        `tests/support/mod.rs` is a 16-byte `redact` helper, there is NO `test_support` module):
        (1) `index_with()` → a fluent `IndexBuilder` producing a real `CodebaseIndex` with
        `.file(name)`, `.imports(a,b)` (Import edge into `index.graph`), `.co_change(a,b,score)`
        (pushes a `CoChangeEdge` onto `index.co_changes`), `.n_risky_files(n)` (files with
        `git_health.churn_30d` + blast so `compute_risk_ranking` yields a spread of RiskEntry),
        `.with_cycle(a,b)` (mutual Import edges → an SCC), `.build()`.
        (2) `unordered_eq(link, x, y)` for SurprisingLink pair comparison.
        (3) `render_fixture_spa()` → thin wrapper over the existing `visual::spa::render_spa(&fixture_index(), &fixture_meta())`
        pattern, exposed once so N5/N6 tests share it instead of redefining ad-hoc fixtures.
        This is the single largest MVP prerequisite; it gates every local:true node — do it first, by Opus.
      accept: tests/test_support_smoke.rs (builder produces a non-empty index with graph edges + co_changes + a rankable risk set)
      deps: []

    - id: N1-surprising-cochange
      local: true
      region: src/intelligence/insights.rs (new) + one line `pub mod insights;` in src/intelligence/mod.rs
      change: >
        Add `pub struct SurprisingLink { pub a: String, pub b: String, pub co_change_score: f64 }`
        and `pub fn surprising_connections(index: &CodebaseIndex) -> Vec<SurprisingLink>`, and
        register the module (`pub mod insights;` in src/intelligence/mod.rs — N1 spans the new file
        + this one registration line). Return every `index.co_changes` (`CoChangeEdge`) whose
        (file_a, file_b) unordered pair has NO Import edge in `index.graph`; set
        `co_change_score = recency_weight` (the recency-decayed strength already on `CoChangeEdge`).
        Sort deterministically by (-co_change_score, a, b). Do NOT add an EdgeType variant — read
        `index.co_changes` directly (there is no `EdgeType::CoChange`).
      accept: tests/insights_surprising.rs (RED — authored below, committed first)
      deps: [N0-test-support]

    - id: N2-risk-percentile
      local: true
      region: src/intelligence/risk.rs  # extend `RiskEntry` (risk.rs:6) + compute_risk_ranking tail (after the sort at :121-126)
      change: >
        Add a `risk_percentile: f64` field to `RiskEntry` and populate it in `compute_risk_ranking`
        as `rank_index_ascending / (n - 1)` (top risk = 1.0, n==1 → 1.0), computed AFTER the
        existing deterministic sort so ties keep the path tie-break. Do not change `risk_score`.
      accept: tests/risk_percentile.rs (RED — authored below)
      deps: [N0-test-support]

    - id: N7-risk-provenance-terms
      local: true
      region: src/intelligence/risk.rs  # extend `RiskEntry` with the three factor terms (same struct + construction site as N2)
      change: >
        Add `churn_term: f64`, `blast_term: f64`, `test_penalty_term: f64` to `RiskEntry`, set to
        the exact `nc`, `nb`, `tc_term` computed at risk.rs:101-105 so that
        churn_term*blast_term*test_penalty_term == risk_score (within f64 epsilon). Feeds the
        provenance drawer's literal derivation. (deps N2: same struct + construction site → serialized.)
      accept: tests/risk_provenance_terms.rs (RED — authored below)
      deps: [N0-test-support, N2-risk-percentile]

    - id: N5-treemap-percentile-ramp
      local: true
      region: src/visual/render.rs  # the risk-heatmap template string around :620-635 (NOT the :536 ramp)
      change: >
        In the risk treemap (the :620 ramp, not the :536 one): color cells by `risk_percentile`
        (from N2) via a luminance-monotonic sequential ramp with a DATA-DRIVEN quantile legend
        (band thresholds from the percentile distribution, not the hardcoded domain); remove the
        `r < 0.1` opacity kludge at :635. Keep raw `risk_score` in the tooltip labeled "absolute".
        Emit `risk_percentile` into the treemap cell JSON.
      accept: tests/spa_risk_percentile_render.rs (RED — authored below)
      deps: [N0-test-support, N2-risk-percentile]

    - id: N6-palette-system
      local: true
      region: src/visual/render.rs  # one <script> palette-registry block + one <select> in header + :root token defaults
      change: >
        Add the palette system to the SPA template: a JS palette registry (btop-schema token sets
        bg/surface/ink/ink2/hair/accent/lo/mid/hi), an `applyPalette()` that sets CSS custom
        properties on :root, a header <select> picker, and Tokyo Night as the default (bg #1a1b26).
        Ship the ~19 palettes with the real hexes enumerated in SPEC §4 (canonical source — the
        session mockup is ephemeral). Palette is client-side only → no change to emitted bytes by default.
      accept: tests/spa_palette.rs (RED — authored below)
      deps: [N0-test-support]

    - id: N4-timeline-wire-persist-backfill
      local: false   # cross-cutting: new write path + call sites + the historical-snapshot backfill (absorbs former N3)
      region: src/visual/{spa.rs,timeline.rs} + src/commands/visual.rs + src/commands/serve.rs
      change: >
        Wire `compute_timeline_snapshots` into the render/serve path and persist via `save_snapshots`
        to `.cxpak/timeline/snapshots.json` (git-ignorable cache) so `load_cached_snapshots` has
        something to read; compute-on-miss fallback; bounded to last-N (budget-capped). Ungate the
        Timeline embed in spa.rs (stop passing bare `None`). BACKFILL (former N3, merged here because
        it needs a signature change this node owns): populate each `TimelineSnapshot.health_composite`
        and `circular_dep_count` — currently hardcoded `None`/`0` at timeline.rs:121-130 — from the
        snapshot's OWN per-commit state (`health_cached().composite` / `count_nontrivial_sccs`), NOT
        the current index's health (injecting current health into historical frames is a correctness bug).
      accept: extend tests/spa_determinism.rs + tests/timeline_wired.rs (non-empty timeline + per-snapshot health present on fixture)
      deps: [N0-test-support]

    - id: N8-provenance-drawer
      local: false   # cross-cutting inspector UI + Overview wiring, shared component
      region: src/visual/render.rs (inspector/drawer template + Overview risk table + alerts)
      change: >
        Add the prove-it drawer: a `p`-key/affordance on each Overview risk row + alert opens an
        inspector Provenance tab showing the literal derivation from N7's terms
        (`0.0402 = churn(0.51) × blast(0.079) × test_penalty(1.0)`) + percentile + absolute.
        Reuse across views later; MVP scope = Overview risk table + alerts only.
      accept: tests/spa_provenance.rs (drawer markup + derivation string present on fixture)
      deps: [N7-risk-provenance-terms]

    - id: N9-explore-unify
      local: false   # IA restructure: merge Architecture+Risk under a lens toggle, module collapse
      region: src/visual/{render.rs,spa.rs}  # nav + Explore mode shell + lens toggle
      change: >
        Collapse the Architecture and Risk tabs into one Explore mode with a Dependencies|Risk lens
        toggle (encoding-only switch over a fixed layout; default lens = Risk treemap). Default the
        module graph to module-level collapse via `collapse_passthrough_chains` +
        `group_into_phases` (7±2). Keep breadcrumbs + command palette in the mode.
      accept: tests/spa_explore_mode.rs (single Explore nav item; both lenses render; no Architecture/Risk tabs)
      deps: [N5-treemap-percentile-ramp]
```

### RED accept tests (authored by Opus, committed before delegating impl)

```rust
// tests/insights_surprising.rs  — N1
use cxpak::intelligence::insights::surprising_connections;
use cxpak::test_support::index_with; // existing test helper pattern

#[test]
fn surprising_connections_excludes_imported_pairs_and_keeps_unimported() {
    // A imports B AND they co-change → NOT surprising.
    // C and D co-change with NO import edge → surprising.
    let index = index_with()
        .file("A").imports("B")
        .co_change("A", "B", 0.9)
        .co_change("C", "D", 0.8)
        .build();
    let links = surprising_connections(&index);
    assert!(links.iter().all(|l| !unordered_eq(l, "A", "B")),
        "imported+co-changed pair must be filtered out");
    assert!(links.iter().any(|l| unordered_eq(l, "C", "D")),
        "co-changed-without-import pair must surface");
}

#[test]
fn surprising_connections_is_deterministic() {
    let index = index_with().co_change("C","D",0.8).co_change("E","F",0.8).build();
    assert_eq!(surprising_connections(&index), surprising_connections(&index));
}
```

```rust
// tests/risk_percentile.rs  — N2
use cxpak::intelligence::risk::compute_risk_ranking;
use cxpak::test_support::index_with;

#[test]
fn percentile_spreads_full_range_even_when_raw_scores_collapse() {
    // raw risk scores realistically live in ~[0, 0.04]; percentile must still span [0,1].
    let ranking = compute_risk_ranking(&index_with().n_risky_files(10).build());
    let ps: Vec<f64> = ranking.iter().map(|e| e.risk_percentile).collect();
    assert!((ps.iter().cloned().fold(f64::MIN, f64::max) - 1.0).abs() < 1e-9, "top == 1.0");
    assert!(ps.iter().cloned().fold(f64::MAX, f64::min) < 0.2, "bottom near 0");
    // monotonic with raw score
    for w in ranking.windows(2) { assert!(w[0].risk_score >= w[1].risk_score && w[0].risk_percentile >= w[1].risk_percentile); }
}
```

```rust
// tests/risk_provenance_terms.rs  — N7
#[test]
fn risk_terms_multiply_to_score() {
    for e in compute_risk_ranking(&index_with().n_risky_files(5).build()) {
        let recomposed = e.churn_term * e.blast_term * e.test_penalty_term;
        assert!((recomposed - e.risk_score).abs() < 1e-9, "terms must reproduce the score");
    }
}
```

```rust
// tests/spa_palette.rs  — N6  (string-presence: the repo's established SPA-assert pattern)
#[test]
fn spa_ships_palette_registry_tokyo_night_default_and_stays_deterministic() {
    let html = render_fixture_spa();
    assert!(html.contains("#1a1b26"), "Tokyo Night bg present as default");
    assert!(html.matches("applyPalette").count() >= 1);
    assert!(html.contains("catppuccin") && html.contains("gruvbox") && html.contains("everforest"),
        "popular palettes shipped");
    assert!(!html.contains("cdn.jsdelivr.net"), "no CDN");
    assert_eq!(render_fixture_spa(), html, "byte-identical across renders");
}
```

```rust
// tests/spa_risk_percentile_render.rs  — N5
#[test]
fn treemap_uses_percentile_not_broken_ramp() {
    let html = render_fixture_spa();
    assert!(html.contains("risk_percentile"), "percentile emitted into treemap cells");
    // NOTE: the real emitted string is spaced — `domain([0, 0.4, 0.7, 1.0])` (render.rs:620).
    // Assert on the spaces or the assertion is vacuously green.
    assert!(!html.contains("domain([0, 0.4, 0.7, 1.0])"), "hardcoded broken ramp removed");
}
```

```rust
// tests/test_support_smoke.rs  — N0 (RED for the prerequisite; proves the builder yields a usable index)
#[test]
fn index_with_builds_graph_cochanges_and_rankable_risk() {
    let index = index_with().file("A").imports("B").co_change("A","B",0.9)
        .n_risky_files(3).with_cycle("X","Y").build();
    assert!(!index.co_changes.is_empty(), "co_changes populated");
    assert!(!cxpak::intelligence::risk::compute_risk_ranking(&index).is_empty(), "risk set is rankable");
}
```

`local: false` nodes (N4, N8, N9) carry their acceptance tests as prose above and are implemented by Opus with full code at implementation time.

---

## Testing strategy

- **Unit (Rust):** the 6 RED tests above + per-node deterministic assertions; ≥90% on new code.
- **SPA render tests:** extend `tests/spa_render.rs` / `spa_determinism.rs` for palette-determinism, percentile emission, no-dead-nav, provenance markup.
- **Conformance:** broaden `cross_channel_consistency.rs` Visual arm from health-only to risk/pagerank/insight payloads (ADR-0180) — this is a Phase-2 test-debt item, tracked now.
- **A11y:** grayscale-survival of the risk ramp (new assertion alongside `spa_a11y.rs`); keyboard reach of prove/drawer/palette.
- **Runner:** all execution via the `test-runner` subagent (captures full output).

## Risk assessment

| Risk | Sev | Mitigation |
|---|---|---|
| New Rust float geometry breaks cross-platform determinism | Med | Fixed-grid coordinate rounding before serialization (ADR-0177); geometry is Phase 3 — MVP adds none. |
| Palette port bloats artifact / breaks golden | Low | Client-side only; determinism test in N6; ~19 token sets are small. |
| Timeline persistence introduces a stale-cache bug | Med | Compute-on-miss fallback; bounded last-N; cache is git-ignorable, never a source artifact (N4). |
| Provenance plumbing balloons the payload | Med | MVP scopes drawer to Overview risk+alerts only; long tail deferred to Live channel (ADR-0174/0178). |
| IA restructure breaks deep-links / muscle memory | Low | Acceptable for a major visual release; URL-hash addressing preserved. |

## Rollout

- New worktree `3.1-ui-overhaul` off `main` (root stays on `main`; never squash-merge). Each slice = its own commit(s), RED test committed before impl.
- Ship MVP as `3.1.0` (surface enhancement; `/v1/diff` + Live are Phase 3 → the breaking-change question defers with them).
- Version-sync the four files (Cargo.toml, plugin.json, marketplace.json, ensure-cxpak `REQUIRED_VERSION`), `cargo check` to regen `Cargo.lock`, commit before tag.
- Pre-tag HITL: `dotclaude detect-hitl --base origin/main --format json`; surface fired checkpoints.

## Success criteria

- [ ] All 6 MVP slices merged; each RED test now GREEN; ≥90% coverage on new code.
- [ ] Golden fixture byte-identical (macOS); no-CDN assertions hold; palette-determinism test green.
- [ ] Risk treemap visibly discriminates on a real repo (percentile), not uniform teal.
- [ ] Surprising-co-change insight visible on Overview.
- [ ] Timeline renders non-empty without CLI flags; no dead nav across Overview/Explore.
- [ ] Provenance drawer proves every Overview risk row in ≤2 clicks.
- [ ] Tokyo Night default; ≥15 palettes selectable; light+dark both correct.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.

## Local-execution split (report after run)

Planned: **5 `local: true`** (N1, N2, N5, N6, N7 — single-region, RED-gated) offloaded to the cascade; **4 `local: false`** (N0 test-support scaffolding [prerequisite], N4 timeline wire+backfill, N8 provenance drawer, N9 Explore IA — cross-cutting/foundational) implemented by Opus. N0 is authored FIRST (the whole RED gate compiles against it); former N3 (timeline backfill) was reclassified into N4 because it requires a signature change N4 owns and has a historical-vs-current-health correctness hazard. Actual split reported post-execution per the offload datapoint.

---
**Status**: Validated (fresh-subagent `code-validate-plan` — NEEDS-REVISION findings all applied: N0 prerequisite added, N3 merged into N4, N5 assertion fixed, N1 mod-registration + `index.graph` + `co_change_score` derivation corrected, N6 re-pointed to SPEC §4)
**Date**: 2026-07-11

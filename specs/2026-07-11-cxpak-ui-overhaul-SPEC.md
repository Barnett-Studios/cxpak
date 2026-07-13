---
title: cxpak UI Overhaul — "Determinism, Made Auditable"
status: DRAFT (pre-validation)
created: 2026-07-11
updated: 2026-07-11
target: cxpak 3.1.0 (visual surface) — MAJOR only if the /v1 + output contract breaks
supersedes-parts-of: ADR-0153 (SPA==intelligence round-trip), visual/* render approach
---

# cxpak UI Overhaul — Spec

## 1. Thesis (why this exists)

cxpak's 3.0.0 backend is strong and genuinely wired to the SPA (verified: real health, risk, pagerank, alerts render with 0 console errors). The **presentation layer** is the weak point. Today's SPA is a *static pre-rendered export*: of six nav tabs, two are good (Dashboard, Risk treemap), one is a sparse hairball (Architecture — and it's the flagship), and **three are dead** (Flow/Timeline/Diff show CLI-nag empty states because the page can only show data the generating CLI happened to embed).

The redesign has **one organizing idea**, arrived at independently by a research pass and a three-lens UX expert panel:

> **The UI's job is to make cxpak's determinism *auditable*.**

cxpak's banner — *"every edge proven, never inferred"* — becomes the organizing principle of the **interaction model** (click any number → its exact derivation), the **visual language** (solid+tick = proven, dashed = inferred), and the **content** (proven analogues of graphify's LLM-guessed insights). No LLM tool can copy this without abandoning inference. That is the moat.

## 2. Non-goals

- **No LLM / no inference anywhere.** Every surfaced value stays a proven computation (ADR-0097 descriptive-only extends to the UI).
- **Not a mandatory server app.** The self-contained single-file artifact remains the primary deliverable.
- **No new graph rendering library.** Layout stays deterministic in Rust; the browser renders precomputed coordinates.
- **Not a re-theming of the backend.** This is the `visual/` surface plus a small, additive HTTP/route contract; core intelligence is unchanged except the risk-scale fix (§12).

## 3. Hard constraints (invariants — a violation is a defect)

1. **Self-contained.** One HTML file, all CSS/JS inlined, **no external CDN/network at runtime** (existing `!html.contains("cdn.jsdelivr.net")` assertion generalizes to "no external origin").
2. **Byte-identical determinism** of the generated artifact **within a platform** (golden fixture `tests/snapshots/spa_golden.html`, ADR-0151; the fixture test is `#[cfg(target_os = "macos")]`-gated because f64 corner cases already drift cross-platform — the invariant is same-platform byte-identity, not universal). The **palette is client-side runtime state** — switching palettes does not change the emitted bytes, so the picker is determinism-neutral by construction. **New Rust-precomputed float geometry (§7 — edge-bundling control points, circle-packing/Brandes-Kopf coordinates) is exactly the class of computation that drives the existing cross-platform drift** → it must round coordinates to fixed decimals before serialization (integer/fixed-point grid) so the emitted geometry is platform-stable, and its determinism is asserted by the golden fixture like the rest.
3. **No LLM, ever.** Descriptive-only (ADR-0097).
4. **Light AND dark both first-class** — not one designed + one inverted.
5. **Rendering = Canvas 2D + already-inlined D3 v7** over Rust-precomputed coordinates. No new inlined graph lib (Canvas is the correct tier for 300–1000+ nodes; SVG lags at the top of range; WebGL/cosmos.gl is over-engineered and cosmos.gl is disqualified on GPU-float determinism; elkjs is EPL-2.0, fails the license bar).
6. **Accessibility:** risk ramp luminance-monotonic + colorblind-safe (Okabe-Ito / OKLCH), redundant icon+shape encoding for categorical flags, keyboard-operable, `prefers-reduced-motion` honored, a table/text fallback for every chart.
7. **The SPA is ONE of five projections** (MCP/LSP/CLI/HTTP/Visual) of a shared core; per-surface budgets and the ADR-0153 round-trip conformance (each surface's *data* equals the core) still hold.

## 4. Design system — one language, many palettes

**Decision (user-approved):** ship **one design language** (structure + typography + the proof-tick signature are constant) with a **palette picker**. This is the VS Code / Obsidian model — strong identity via structure, many color schemes — NOT three separate design languages (which would triple design debt and dilute the signature).

- **Design language = "Blueprint":** graph-paper grid as chrome only (never behind data), hairline 1px borders, **square corners, zero blur-radius shadows** (elevation via border-weight/fill-tint), corner registration marks, a title-block header. Typography: **system-sans body + monospace data** with `tabular-nums` — needs **no webfont** (honors no-CDN for free). Motion: mechanical/plotter (stroke-dashoffset draws, no bounce), `prefers-reduced-motion` respected.
- **Palette system:** btop-style token schema (mirrors the user's `mticky` `.theme` files: `bg / fg / hi_fg(accent) / inactive_fg(muted) / div_line(hair) / positive(good) / negative(bad)`), extended with `surface`, `ink2`, and a 3-stop risk ramp (lo/mid/hi). **~19 palettes** shipped:
  - **cxpak moods:** Cyanotype (light+dark), Phosphor (amber-on-void), Field-book (paper).
  - **popular schemes (real hexes):** **Tokyo Night (DEFAULT)**, Catppuccin (Macchiato + Latte), Everforest (dark+light), Gruvbox (dark+light), Nord, Dracula, Monokai, One Dark, Rosé Pine (Moon + Dawn), Solarized (dark+light).
  - Palette registry is data (a table of token sets); a `.cxpak/palettes/*.toml` drop-in path lets users add community palettes at ~zero core cost.
- **Default palette: Tokyo Night.** Light/dark toggle flips to a palette's paired variant where one exists.
- **Determinism note:** because palette is applied client-side (CSS custom properties), the emitted HTML bytes are identical regardless of default or selection; the golden fixture is unaffected. If a generation-time `--palette` flag is offered, it only changes which token block is the *initial* state (still deterministic per-input).

## 5. Information architecture — 6 tabs → 3 modes

Collapse the flat six-tab model (three of which are dead) into **three modes**, plus persistent global affordances.

| Mode | Replaces | What it is |
|---|---|---|
| **Overview** | Dashboard | Repo health readout (dial + dimension bars), the **repo-DNA barcode**, and **ranked narrated insights** (§8) — every row deep-links into Explore, none is a dead end. |
| **Explore** | Architecture + Risk | ONE spatial canvas with a **lens toggle** — `Dependencies · Risk · Coverage · Churn · Security`. A lens changes only *encoding* (color/size), never layout. Default lens = the (already-strongest) **Risk treemap**; the module graph is module-level + collapsible, with an **adjacency-matrix toggle** for dense hubs. |
| **History** | Timeline + Diff | ONE time-scrubber over Explore. Drag one thumb = timeline replay (recolor by snapshot); shift-click a second = diff (recolor by delta) + the **DNA-barcode diff**. |

- **Flow** stops being a tab — it becomes a **contextual inspector action** ("Trace data flow") available whenever a symbol node is selected. Never a dead nav item.
- **Persistent global affordances (mostly already built):** command palette (`Cmd/Ctrl+K` / `/`) over the PageRank-ranked `SearchIndex`; a **breadcrumb strip** (repo → module → file → symbol, reused from `ArchitectureExplorerData.breadcrumbs`) pinned under the header in *every* mode; the palette picker + light/dark; a `?` help overlay.
- **Single addressing scheme across modes:** URL hash carries scope everywhere (`#explore/module=src/intelligence`, `#history?scope=…&from=HEAD~5&to=HEAD`). Selection is shared state (`CX.app.scope`) — selecting a node in Explore sets the scope for History and the inspector's Flow action.
- **Inspector** is the single details surface for every mode: two tabs — **Details** and **Provenance** (§6).

## 6. The hero — provenance system ("show your work")

The differentiator, and the visual + interaction expression of the banner. Two halves that converged from two independent experts:

- **Visual atom — the proof-tick.** A directly-computed relationship (import / FK / call / confirmed AST diff) renders as a **solid line + a short perpendicular hash at its midpoint + a monospace datum tag** (`IMPORT`/`FK`/`CALL`/`CO-CHANGE:0.82`). Anything derived from correlation/prediction/heuristics (co-change, predicted risk, heuristic rename) renders **dashed, no tick, `~`-prefixed**. One motif, reused verbatim across all views (graph edges, treemap cell borders, alert icons, diff gutters, timeline connectors, flow paths). It is cxpak's fingerprint because it encodes the banner as geometry, not as a legend.
- **Interaction — "prove it."** Every score/edge/cell exposes a **"prove"** affordance (keyboard: select + `p`) opening the inspector's **Provenance** tab scoped to that exact value. It shows the *literal derivation with real substituted numbers*, not a restated label:
  - risk `0.0402 = churn(0.51) × blast(0.079) × test_penalty(1.0)`, plus the within-repo percentile and the absolute-vs-percentile explanation;
  - a schema edge → the literal FK constraint / migration line (`schema::link::build_schema_edges` carries this — stop discarding it at graph-edge time);
  - a churn number → the actual contributing commit list (`co_change.rs` 180-day walk);
  - a dead-code flag → the exact call-graph absence (`build_symbol_cross_refs`).

This makes "every edge proven, never inferred" auditable in ≤ 2 clicks — structurally impossible for an LLM-summarized tool.

## 7. Rendering approach (deterministic, no new lib)

- **Rust precomputes all coordinates and derived geometry**; the browser only renders. This removes the two hardest determinism risks (unseeded force layout, GPU float) and needs no layout engine shipped.
- **Canvas 2D** for the dense views (Explore graph, treemap) + **d3-quadtree** (already inlined) for O(log n) hover hit-testing + **d3-zoom** for pan/semantic-zoom. Keep SVG for small views (dial, dimension bars, small flow) where crisp text/a11y matter.
- **Anti-hairball techniques, all computed server-side in Rust (deterministic):** Hierarchical Edge Bundling (Holten — invented for software dependency graphs) control points; circle-packing / icicle containment coordinates for the module→file→symbol nesting; adjacency-matrix ordering (by SCC/community) for dense clusters; degree-of-interest scores for Flow focus+context; metanode collapse state for semantic zoom. Reuse `intelligence/onboarding.rs::group_into_phases` (7±2 clustering) and `render.rs::collapse_passthrough_chains` for module-level default collapse.
- **Overview-as-fingerprint:** replace node-link-as-overview with **containment-as-overview** (the repo-visualizer circle-packing / CodeCharta code-city precedent) so users grasp repo *shape* before touching the graph.

## 8. Deterministic interpretive layer (proven analogues of graphify)

graphify wins on narration (god-nodes, "surprising connections", community labels) — mostly via LLM inference. cxpak narrates with the same punch but **every claim is proven**. Ship these (each computable from existing signals; source in parens):

1. **God-nodes** → PageRank hubs + inbound fan-in (`pagerank.rs`, `architecture.rs::detect_god_files`).
2. **Surprising connections** → **co-change WITHOUT an import edge**: `index.co_changes: Vec<CoChangeEdge>` (`co_change.rs`, populated at `visual.rs:81` via the 180-day git walk) minus the `DependencyGraph` Import edges. **Flagship — computed today but never surfaced by any renderer** (there is NO `EdgeType::CoChange` graph arm; co-change lives only on `index.co_changes`, so the insight is `co_changes` set-minus Import edges, computed fresh). Ship first. Marked `~ estimated` (correlation, honestly labeled).
3. **Cross-layer coupling** → schema↔code edges (`EdgeType::{ForeignKey,OrmModel,EmbeddedSql}`, `schema/link.rs`). A category cxpak owns; graphify has no data layer.
4. **Blast-radius storytelling** → narrate `BlastRadiusCategories` four lanes as a sentence (`blast_radius.rs`).
5. **Danger-zone** → high blast × zero tests (`risk.rs`, empty `test_files`) — the single most actionable line.
6. **Proven cycles** → SCC circular deps + name the cheapest edge to cut (`architecture.rs::circular_deps`, Tarjan).
7. **Repo DNA** → quantified conventions fingerprint (`ConventionProfile`, `render_dna_section`) at `Convention|Trend` strength.
8. **Drift / decay** → baseline delta (`drift.rs::build_drift_report`).
9. **Change-impact prediction** → which tests to run (`predict.rs::PredictionResult`).
10. **Exposure** → security surface (`security.rs::build_security_surface`, real handler names).

Overview shows the top 5 as always-on headlines: #5, #2, #1, #6, #10.

## 9. Repo-DNA signature visualization

A **deterministic genome barcode** — cxpak's identity mark. Same repo → byte-identical image; two repos → instantly distinguishable; same repo across commits → an animatable drift (used in Overview hero, History frame, Diff headline).

- **Track 1 — conventions genome:** one band per `ConventionProfile` axis (naming, imports, errors, dependencies, testing, visibility, functions, git_health); band height/opacity = `observation.percentage`; band solidity = `PatternStrength` on its 3 levels — Convention (solid) → Trend → Mixed (faint); an axis with no dominant observation is a gap (no `None` variant exists — `conventions/mod.rs:19-23`).
- **Track 2 — structure genome:** PageRank-sorted spine sparkline of the file-importance distribution (top-heavy vs flat is visibly different), overlaid with cycle + dead-symbol ticks.
- Drivers: `ConventionProfile.*.{dominant,percentage,strength}`, PageRank distribution shape, coupling vector, cycle count, coverage ratio, language mix — all deterministic.

## 10. Interaction model — hybrid (user-approved)

Self-contained by default; a bounded, deterministic embedded **answer-surface** plus opt-in progressive enhancement.

- **Base artifact embeds a bounded, deterministic answer-surface:** top-K symbols by PageRank/API-surface for Flow (budget-capped like the 20k `SearchIndex` cap), last-N git snapshots for Timeline (`compute_timeline_snapshots` in `timeline.rs` **exists but has no non-test caller** — nothing ever writes the `.cxpak/timeline/` cache that every render path only *reads*; the work is to WIRE it into the render/serve path + persist, not merely remove a `None` fallback), and one documented default Diff (`working tree vs HEAD~1` or `vs origin/main`) + last-~10 commits as pre-diffable choices. (Note: `TimelineSnapshot.health_composite`/`circular_dep_count` are hardcoded `None`/`0` today — backfill them as part of the wiring or History narration reads empty.)
- **Progressive enhancement — a "Live" toggle:** a user-confirmed `cxpak serve` URL (never auto-probed/silent) unlocks *arbitrary* symbol/ref queries via `/v1/data_flow` and a **new `/v1/diff` route**. Reading of the constraint: "no external CDN/network at runtime" = "no *unsolicited* network"; a user-initiated localhost connection to a server the user started is allowed. **The base file's bytes and the golden fixture are unaffected by whether Live is ever toggled** — determinism of the artifact is preserved; only runtime capability differs.
- **Never silently degrade:** an out-of-embedded-set pick with Live off shows "not precomputed for this build — toggle Live or re-run `cxpak visual --symbol X`", never a bare CLI-nag.

## 11. Surface impact (the 5 projections)

- **Visual (primary):** the whole redesign.
- **HTTP `/v1`:** add `/v1/diff` (new — today only an unversioned `/diff` exists; bounded, bearer-gated per the existing pattern), keep `/v1/data_flow`; Live mode consumes them. Existing auth invariant (all data routes bearer-gated, timing-safe constant-time compare, `route_layer(auth_layer)` — ADR-0110/0146) extends to `/v1/diff`.
- **CLI:** `cxpak visual` gains `--palette <name>` (initial-state only), and the Flow/Timeline/Diff no-arg behaviors change from "nag" to "embed a bounded default".
- **MCP / LSP:** unchanged by this work (already ≤8 tools; the conventions token-budget fix shipped in 3.0.0).

## 12. Dataviz discipline + the risk-scale-collapse fix

- **Bug (correctly diagnosed):** the earlier "risk 6.88 out of range" was a misread of `6.886e-6` (in range). The **real** defect is **scale collapse**: `risk = norm_churn × norm_blast × tc_term` is a product of three sub-1 fractions (`norm_blast = dependents/total_files` → 60/761 = 0.079), so the observed max is ~0.04 and the treemap's `[0,0.4,0.7,1.0]` ramp maps *everything* to the first band (the uniform-teal treemap). **Fix:** (a) never emit a bare mantissa; (b) color the treemap/table by **within-repo percentile** (`risk_percentile = rank(risk)/N`), keep raw score in the tooltip labeled "absolute"; (c) fix the large-repo blast penalty (`dependents/total_files` shrinks with size → use percentile or `blast/max_blast`); (d) data-driven legend (quantile thresholds, not hardcoded 0.4/0.7). Sources: `risk.rs:105` (formula `nc * nb * tc_term`; `norm_blast = dependents/total_files` at `risk.rs:88`; `norm_churn` is *already* a percentile at `risk.rs:66-72`, and `compute_risk_ranking` already ranks deterministically with a path tie-break at `risk.rs:121-126` — so the percentile fix reuses an existing, determinism-safe pattern and applies it to the one non-percentile term, blast), `render.rs:620,634` (broken ramp + `color(d.data.risk_score)`; the opacity kludge at `render.rs:635` tacitly admits the collapsed range), `render.rs:1466` (`risk_severity`).
- **Encoding rules (one per channel per view):** SIZE = magnitude (tokens/blast/PageRank), never risk; POSITION = topology/hierarchy; COLOR = exactly one quantitative signal; SHAPE/BORDER/GLYPH = categorical flags (god-file, in-cycle, dead-code, unprotected) as redundant icons.
- **Chart-type matrix:** treemap (risk×blast), module node-link edge-bundled (topology N<~50), **adjacency matrix ordered by SCC/community (dense coupling)**, layered Sankey (single-symbol flow), timeline small-multiples (trends), bump/slope + DNA-barcode diff (before/after).
- **Ramp:** luminance-monotonic sequential (OKLCH / Okabe-Ito), colorblind-safe, works in light+dark; ship a **grayscale-survival assertion** as an a11y/determinism test alongside `spa_a11y.rs`.

## 13. Acceptance / testability

- **Determinism golden fixture** (ADR-0151) still byte-identical on the default path; the palette picker does not affect emitted bytes (add an assertion: two builds with different `--palette` init differ only in the initial token block, and no `--palette` → identical).
- **Round-trip conformance** (ADR-0153 generalized): every number shown in a view equals its core intelligence output. A real (health-only) Visual round-trip already exists — `tests/cross_channel_consistency.rs:423-449` asserts the SPA health composite bit-for-bit against `health_cached().composite`; the work is to *broaden* it to the Overview/Explore risk/pagerank/insight data, not to un-stub it (it is not a stub).
- **No dead nav:** an automated check that Overview/Explore/History all render non-empty on the fixture repo (Flow-as-action, Timeline ungated, Diff default-embedded).
- **A11y:** grayscale-survival of the risk ramp; keyboard reachability of prove/drawer/palette; reduced-motion.
- **Provenance completeness:** every displayed score exposes a derivation (start with Overview's risk table + alerts; expand per phase).
- **Live mode:** `/v1/diff` bounded + auth-gated; Live is user-confirmed, never auto-probed (test: no network call before toggle).

## 14. Phasing (full-overhaul scope, shipped in slices)

**MVP (biggest perception-shift per effort):**
1. Wire Timeline: call `compute_timeline_snapshots` in the render/serve path + persist the `.cxpak/timeline/` cache + backfill `health_composite`/`circular_dep_count` (NOT a one-line fallback removal — the compute fn currently has no non-test caller).
2. Fix the risk scale-collapse (percentile color + quantile legend).
3. Palette system + picker (Tokyo Night default) — determinism-neutral, high visible impact.
4. Explore = one canvas, `Dependencies + Risk` lenses; module-level collapse for the graph.
5. Provenance drawer on Overview's risk table + alerts.
6. Surprising-co-change insight (co-change − import) — the flagship, currently invisible.

**Phase 2:** History scrubber (Timeline+Diff merge) + default embedded diff; Coverage/Churn/Security lenses; adjacency-matrix toggle; repo-DNA barcode; bounded Flow-as-inspector-action.

**Phase 3:** Live toggle + `/v1/diff` (hybrid upgrade); full provenance coverage across every score in every lens; containment-as-overview fingerprint; edge-bundling + semantic-zoom polish.

## 15. ADRs to write (promote to docs/adrs/ when the UI worktree starts; highest existing is 0171, so the block starts at 0172)

- **0172** One design language + palette system (determinism-neutral client-side palettes; btop-schema; Tokyo Night default).
- **0173** 6-tab → 3-mode IA; Flow as inspector action.
- **0174** Provenance system (proof-tick geometry + prove-it drawer) as the UI expression of ADR-0097/banner.
- **0175** Deterministic interpretive layer (proven analogues; co-change−import flagship).
- **0176** Repo-DNA genome-barcode signature.
- **0177** Canvas-2D-over-Rust-precomputed-coordinates rendering (no new graph lib; reject cosmos.gl/elkjs with reasons; fixed-point coordinate rounding for cross-platform determinism).
- **0178** Hybrid interaction model + `/v1/diff` (bearer-gated per ADR-0110/0146); "no unsolicited network" reading.
- **0179** Risk normalization = within-repo percentile (fix scale-collapse); one-signal-per-channel encoding + grayscale-safe ramp.
- **0180** Generalize ADR-0153 conformance to broaden the real-core Visual round-trip (currently health-only); no-dead-nav gate.

## 16. Open questions

- Version: 3.1.0 (surface enhancement) unless `/v1/diff` + output changes are judged breaking → 4.0.0.
- Generation-time `--palette` default flag: ship in MVP or Phase 2?
- Containment-as-overview (circle-packing) vs keeping treemap as the Explore default — validate with the recall/usability lens before committing Phase 3.

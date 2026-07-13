---
id: '0196'
title: Canvas-2D over Rust-precomputed coordinates; no new graph library
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0196: Canvas-2D over Rust-precomputed coordinates; no new graph library

**Context.** Explore must render 300–1000+ nodes without a hairball, deterministically, in a self-contained file. A Sugiyama layout engine already exists in Rust (`src/visual/layout.rs`: SCC condensation, barycenter, Brandes-Kopf). D3 v7 is already inlined (`assets/d3-bundle.min.js`, 273 KB).

**Options considered.**
1. *SVG for everything.* Rejected — lags at the top of the node range; DOM weight.
2. *WebGL / cosmos.gl.* Rejected — over-engineered for the range, and cosmos.gl's GPU float math is nondeterministic (fails the golden fixture).
3. *Ship a JS layout engine (elkjs).* Rejected — EPL-2.0 license fails the bar, and it moves layout (hence determinism) into the browser.
4. *Rust precomputes all coordinates; browser renders them on Canvas 2D.*

**Decision.** Option 4. Rust computes every coordinate + derived geometry (Sugiyama positions, Hierarchical Edge Bundling control points, circle-packing/icicle containment, adjacency-matrix ordering, degree-of-interest, metanode collapse state — all deterministic, reusing `layout.rs`, `onboarding.rs::group_into_phases` for 7±2 clustering, `render.rs::collapse_passthrough_chains` for module collapse). Browser renders with Canvas 2D + the already-inlined d3-quadtree (hit-testing) + d3-zoom (pan/semantic-zoom). Keep SVG for small crisp views (dial, bars, small flow). **No new inlined graph library.**

**Consequences.** The two hardest determinism risks (unseeded force layout, GPU float) are removed by construction. Artifact size unchanged (no new lib). **Determinism caveat:** the golden fixture is `#[cfg(target_os="macos")]`-gated because f64 corners already drift cross-platform; the new precomputed float geometry is exactly that risk class → **coordinates are rounded to a fixed decimal / integer grid before serialization** to *reduce* cross-platform drift (it narrows the divergence but cannot guarantee byte-identity across a rounding-tie boundary). The macOS-gated golden fixture asserts same-platform byte-identity only; cross-platform stability is best-effort, consistent with the existing invariant (constraint #2). The fixed-point grid also keeps the emitted geometry compact.

**Revisit if.** Node counts routinely exceed Canvas 2D's comfortable ceiling (~a few thousand interactive) → reconsider a *deterministic* WebGL path (integer-quantized, CPU-fallback-verified), not cosmos.gl.

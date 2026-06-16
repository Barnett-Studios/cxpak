---
id: '0114'
title: 'Visualizations use Rust-precomputed Sugiyama layout shipped as self-contained HTML with inlined D3'
status: ACCEPTED
date: 2026-03-31
triggered_by: v2.0.0 'The Experience' — interactive intelligence dashboard
loop: planning
---

# ADR-0114: Visualizations use Rust-precomputed Sugiyama layout shipped as self-contained HTML with inlined D3

## Context

Introduced in v2.0.0 ("The Experience") for the interactive intelligence dashboard. Graph visualization layout can be computed in the browser (force-directed, 5-30s for large graphs, by which point users give up) or precomputed server-side. Distribution can rely on a CDN/npm/build step or ship as a single self-contained file.

## Options considered

- **Option A — Precompute layout in Rust (simplified Sugiyama), self-contained HTML with inlined custom D3 bundle:** ~500-800 lines of Rust — layer assignment via petgraph topological sort, barycenter crossing minimization, Brandes-Kopf coordinate assignment, grid/pack intra-module; ship positions as JSON inside a single HTML file with a ~100KB custom D3 subset inlined via `include_str!`; no force simulation in the browser, no CDN/npm/build; PNG via resvg behind the `visual` feature flag. Pros: HTML opens instantly (just renders precomputed positions); single shareable file with no network/build dependency; 7±2 grouping keeps SVG node count low so no WebGL is needed; resvg gives pure-Rust PNG. Cons: reimplementing Sugiyama in Rust (~500-800 LOC) since no Rust ELK crate exists; a static poster mode is needed for ungrouped 10K-file views. Someone could prefer it for instant load and single-file distribution.

- **Option B — Browser-side force-directed layout with CDN D3:** Ship a thin HTML that loads D3 from a CDN and runs the force simulation client-side. Pros: far less Rust code. Cons: 5-30s layout for large graphs (users give up); requires a network round-trip to the CDN; not a self-contained file. This was a genuine alternative the design weighed and rejected; someone could prefer it to avoid reimplementing a layout engine.

## Decision

Precompute visualization layout in Rust using a simplified Sugiyama algorithm (~500-800 lines: layer assignment via petgraph topological sort, barycenter crossing minimization, Brandes-Kopf coordinate assignment; grid/pack for intra-module) because no maintained Rust ELK crate exists. Ship each view as a single self-contained HTML file with a custom ~100KB D3 subset (d3-hierarchy/zoom/transition/scale/selection/shape/color/interpolate) inlined via `include_str!` — no CDN, npm, or build step. The 7±2 grouping constraint keeps visible nodes under ~50 so D3+SVG suffices (no WebGL); ungrouped large-repo views render as static SVG poster mode. PNG export via the resvg pure-Rust rasterizer behind the `visual` feature flag.

## Consequences

### Positive
- HTML opens instantly — the browser only renders precomputed positions.
- Single self-contained file, no network/build dependency.
- 7±2 grouping keeps SVG performant without WebGL.
- Pure-Rust PNG via resvg.

### Negative
- ~500-800 LOC Sugiyama reimplementation in Rust.
- Large ungrouped views fall back to a non-interactive static SVG poster mode.

### Neutral
- Visualization is gated behind the `visual` feature flag (`dep:resvg`, `dep:petgraph`, `dep:thiserror`).

## Revisit if
- A maintained Rust graph-layout crate (ELK equivalent) becomes available.
- Grouped views need to exceed the 7±2 / ~50-node SVG comfort zone.

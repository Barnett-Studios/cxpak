---
id: '0141'
title: Pre-compute graph layout in Rust (simplified Sugiyama), not client-side JS
status: ACCEPTED
date: 2026-04-01
triggered_by: v2.0.0 visual intelligence dashboard plan
loop: implementation
---

# ADR-0141: Pre-compute graph layout in Rust (simplified Sugiyama), not client-side JS

## Context

cxpak v2.0.0 ("The Experience") ships a visual intelligence dashboard that renders dependency and architecture graphs. The decision is whether layout — node positioning and crossing minimization — runs in the browser via a JS graph library at render time, or is computed ahead of time in Rust and shipped as fixed coordinates.

## Options considered

- **Option A — Pre-compute Sugiyama layout in Rust, ship coordinates (chosen):** Implement a simplified Sugiyama pipeline in Rust — layer assignment via longest-path/topological sort with SCC condensation, barycenter crossing minimization, Brandes-Kopf coordinate assignment — and let D3 only draw pre-positioned nodes. Pros: deterministic, byte-identical output (enables snapshot tests), works for static SVG/PNG export with no JS, smaller client bundle. Cons: graph-layout algorithms must be reimplemented in Rust, and there is no interactive re-layout in the browser. Preferred for the determinism and the no-JS export path.
- **Option B — Client-side layout via Sigma.js / force-directed D3 (rejected):** Ship raw graph data and let a JS library compute layout in the browser at render time. Pros: less Rust code, mature JS layout engines. Cons: non-deterministic output, requires JS to produce any image, larger/heavier client, no static SVG/PNG export path. Someone could prefer it for richer in-browser interactivity, but it was explicitly rejected ("No Sigma.js").

## Decision

Layout is pre-computed in Rust using a simplified Sugiyama pipeline (`src/visual/layout.rs`: Kosaraju SCC condensation, longest-path layering on the condensation DAG, one-sided barycenter crossing minimization, simplified Brandes-Kopf coordinate assignment). The browser receives fixed coordinates and D3 only renders them. Sigma.js is explicitly rejected.

## Consequences

### Positive
- Output is deterministic, enabling byte-for-byte snapshot/regression tests (Task 25 `visual_output_is_deterministic`).
- Static SVG/PNG/Mermaid export works without any JS execution.
- Self-contained HTML with a small inlined D3 bundle rather than a heavy graph library.

### Negative
- No browser-side dynamic re-layout; positions are fixed at generation time.
- Sugiyama edge cases (cycles) must be handled via SCC condensation in Rust.

### Neutral
- Requires Rust implementations of layer assignment, crossing minimization, and coordinate assignment in `src/visual/layout.rs`.

## Revisit if
- Graphs grow large enough that fixed server-side layout is too coarse for interactive exploration.
- A need arises for live re-layout or user-dragged nodes.

## Sources

- `2026-04-01-v200-implementation-plan-part1.md`: "Layout pre-computed in Rust (simplified Sugiyama). Self-contained HTML with custom D3 bundle (~100KB) inlined. PNG via resvg. No Sigma.js."
- `2026-04-01-v200-implementation-plan-part2.md`: "run the architecture HTML command twice on the same fixture repo; assert both outputs are byte-for-byte identical (layout is pre-computed in Rust, not randomised)."

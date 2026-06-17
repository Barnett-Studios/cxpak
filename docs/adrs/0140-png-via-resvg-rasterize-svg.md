---
id: '0140'
title: Render PNG by rasterizing self-generated SVG via resvg
status: ACCEPTED
date: 2026-04-01
triggered_by: Multi-format export requirements (Task 13)
loop: implementation
---

# ADR-0140: Render PNG by rasterizing self-generated SVG via resvg

## Context

cxpak v2.0.0 visual export must produce raster PNG output alongside HTML, SVG, Mermaid, C4, and JSON. The decision is which rasterization backend to use and whether to render PNG directly or by rasterizing an intermediate SVG.

## Options considered

- **Option A — `to_svg()` then resvg rasterization (chosen):** Generate pure SVG (rect/text/line) from pre-computed layout coordinates, then rasterize that SVG to PNG bytes with `resvg`, gated behind the `visual` feature. Pros: reuses the SVG renderer so geometry has a single source of truth, pure-Rust rasterizer, no headless browser. Cons: `resvg 0.44` adds ~2 MB to the binary, and the SVG exporter must cover every visual primitive needed for faithful raster output.
- **Option B — Headless browser screenshot of the HTML view:** A reasonable alternative would have been to drive a headless Chromium to screenshot the D3 HTML view. Pros: pixel-faithful to the interactive view. Cons: heavy external runtime dependency, slow, non-deterministic, and hard to test in CI. Someone could prefer it where exact visual parity with the interactive dashboard is the priority.

## Decision

`to_png()` is implemented under `#[cfg(feature = "visual")]` by calling `to_svg(layout, metadata)`, parsing the result with usvg, and rasterizing via `resvg::render` followed by `pixmap.encode_png()`. PNG dimensions default to 1920x1080 (dashboard) and 2560x1440 (architecture/risk), configurable. Tests assert on PNG magic bytes and minimum size.

## Consequences

### Positive
- Pure-Rust raster path with no browser dependency.
- Geometry is shared with the SVG exporter; output is testable via PNG magic bytes.

### Negative
- The SVG exporter must render every primitive needed for faithful PNG output.

### Neutral
- The PNG path is only available when the `visual` feature is enabled.

## Revisit if
- resvg cannot render an SVG feature the dashboards need.
- Output fidelity versus the interactive HTML diverges unacceptably.

## Sources

- `2026-04-01-v200-implementation-plan-part1.md`: "Implement `to_png()` under `#[cfg(feature = \"visual\")]` — calls `to_svg()` then rasterizes via `resvg`."
- `2026-04-01-v200-implementation-plan-part1.md`: "PNG rasterization via resvg" (a `///` doc-comment in the key-function-signatures block).

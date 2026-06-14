---
id: '0143'
title: Self-contained HTML output with an inlined custom D3 bundle (no CDN)
status: ACCEPTED
date: 2026-04-01
triggered_by: v2.0.0 visual dashboard HTML rendering
loop: implementation
---

# ADR-0143: Self-contained HTML output with an inlined custom D3 bundle (no CDN)

## Context

cxpak v2.0.0 dashboards must be openable as standalone files (`file://` context) without network access. The decision is how to deliver the D3 rendering library and the layout data: inline them into one HTML file, or reference D3 from a CDN.

## Options considered

- **Option A — Custom D3 bundle inlined via `include_str!` (chosen):** Build a trimmed D3 bundle (d3-hierarchy, d3-zoom, d3-transition, d3-scale, d3-selection, d3-shape, d3-color, d3-interpolate), commit it to `assets/`, and inline it plus the JSON layout into one HTML file via `include_str!`. The implementation plan set a minified target of ~100KB (part1 line 453); the committed `assets/d3-bundle.min.js` is ~273KB (279,706 bytes). Pros: single self-contained file, works offline / via `file://`, no CDN dependency or version drift, testable for absence of CDN URLs. Cons: ~270KB added to every HTML output; the D3 bundle is a committed asset to maintain.
- **Option B — Reference D3 from a CDN (jsdelivr/unpkg) (rejected):** Emit `<script src>` tags pointing at a CDN-hosted full D3. Pros: tiny HTML files, no committed asset. Cons: requires network access to render, breaks offline/air-gapped use, version/availability risk. Tests explicitly assert the absence of `cdn.jsdelivr.net` and `unpkg.com`, evidencing this as the rejected path. Someone could prefer it to keep output files tiny in a reliably-online environment.

## Decision

`render_html` inlines a pre-built ~270KB custom D3 bundle (`static D3_BUNDLE: &str = include_str!("../../assets/d3-bundle.min.js")`) and the JSON-serialized layout into a single self-contained HTML document. No CDN references are permitted; tests assert the absence of `cdn.jsdelivr.net` and `unpkg.com`.

## Consequences

### Positive
- Dashboards work offline and over `file://`.
- No runtime dependency on external CDNs or their versions.
- Tests can assert self-containment.

### Negative
- Every HTML output carries ~270KB of bundle.
- The committed `assets/d3-bundle.min.js` must be maintained out-of-band of the Rust build.

### Neutral
- Layout data is embedded in a `<script id="cxpak-data" type="application/json">` tag.
- `view_controller_js` is embedded as a Rust string literal per `VisualType`.

## Revisit if
- HTML size becomes a problem for large multi-view exports.
- The D3 bundle maintenance burden grows.

## Sources

- `2026-04-01-v200-implementation-plan-part1.md`: "Implement `render_html()` — inlines d3-bundle via `include_str!`, serializes layout as JSON, emits self-contained HTML."
- `2026-04-01-v200-implementation-plan-part1.md`: "assert!(!html.contains(\"cdn.jsdelivr.net\"));\n    assert!(!html.contains(\"unpkg.com\"));"

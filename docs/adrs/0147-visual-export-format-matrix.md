---
id: '0147'
title: Six-format visual export matrix (HTML, Mermaid, SVG, PNG, C4 DSL, JSON)
status: ACCEPTED
date: 2026-04-01
triggered_by: Multi-format export interface design (Task 13)
loop: implementation
---

# ADR-0147: Six-format visual export matrix (HTML, Mermaid, SVG, PNG, C4 DSL, JSON)

## Context

Shipped in v2.0.0. The visual output must serve distinct downstream consumers: interactive viewing, docs embedding, raster images, architecture-tool import, and programmatic re-use. The set of supported formats and their target tools is an interface decision made under Task 13.

## Options considered

- **Option A — Six explicit formats from one `ComputedLayout`:** `to_mermaid` (always `graph TD`; IDs sanitized — `/`, `.`, `-` → `_` — truncated to 32 chars), `to_svg` (pure rect/text/line, no JS), `to_png` (resvg), `to_c4` (Structurizr C4 Container DSL, module-level only), `to_json` (ComputedLayout passthrough), plus HTML. Pros: covers interactive, docs, raster, architecture-tool, and programmatic consumers from a single pre-computed layout. Cons: six renderers to maintain; format-specific quirks (Mermaid ID escaping, C4 module-only). This is the shipped design. (Note: the design doc proposed a file-level `graph LR` variant; the shipped code emits `graph TD` unconditionally.)
- **Option B — HTML only, leave conversion to the user:** Emit just the interactive HTML. Pros: one renderer. Cons: no docs-embeddable text, no raster, no architecture-tool interop, no machine-readable form. A reasonable alternative would have been this for a minimal first cut; it was rejected because it pushes every downstream format conversion onto the user.

## Decision

`export.rs` exposes six formats off a single `ComputedLayout`: `to_mermaid` (always `graph TD`, cycles styled red via `fill:#ff4444`, node IDs with `/`, `.`, `-` → `_` truncated to 32 chars), `to_svg` (pure SVG, no interactivity), `to_png` (resvg, behind the `visual` feature), `to_c4` (Structurizr C4 Container, module-level only), and `to_json`, in addition to `render_html`.

## Consequences

### Positive
- A single layout feeds all formats, giving consistent geometry across outputs.
- Serves docs (Mermaid/SVG), images (PNG), architecture tooling (C4), and programmatic use (JSON).

### Negative
- Six export paths to keep in sync as the layout model evolves.

### Neutral
- Mermaid IDs are sanitized and truncated to 32 chars; C4 is limited to the module (Container) level.

## Revisit if
- A consumer needs a format not in the matrix (e.g. GraphML, DOT).
- C4 needs deeper-than-Container detail.

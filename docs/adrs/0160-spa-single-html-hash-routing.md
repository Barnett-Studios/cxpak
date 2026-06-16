---
id: '0160'
title: Single self-contained HTML SPA with client-side hash routing for all six views
status: ACCEPTED
date: 2026-04-17
triggered_by: v2.0.0 shipped 6 standalone per-view HTML files; goal was to make them feel like one premium tool
loop: planning
---

# ADR-0160: Single self-contained HTML SPA with client-side hash routing for all six views

## Context
v2.0.0 emitted six separate self-contained HTML files, one per visual view. v2.1.0 needed to unify them into a single page the user can navigate without a reload, while preserving the no-build-step / no-server / opens-from-`file://` contract.

## Options considered
- **Option A — single HTML file + hash routing + embedded JSON data tags:** one top-level `render_spa()` produces one HTML file containing all six `<section id=view-X>` containers; a hash router toggles the `hidden` attribute; all view data is embedded as `<script type="application/json">` tags; D3 and the controller are inlined. Pros: opens instantly from `file://`, no build step / npm / server; the per-view renderers stay unchanged so they can still be embedded in reports. Cons: whole-repo JSON embedded inline can bloat the HTML on huge monorepos; CSP must permit `unsafe-inline`. This is the chosen option.
- **Option B — keep six separate HTML files (v2.0.0 status quo):** continue emitting one self-contained file per view. Pros: already works; smaller individual files; one view is embeddable in a report. Cons: no cross-view navigation; does not feel like a single tool. Someone could prefer it to avoid the inline-JSON bloat.
- **Option C — server-rendered multi-page app behind `cxpak serve`:** A reasonable alternative would have been to serve views as separate routes from the HTTP server, navigating via real page loads. Pros: no giant inline JSON blob; lazy data fetch. Cons: breaks the self-contained `file://` contract and requires a running server to view anything.

## Decision
Add a new top-level `render_spa()` (`src/visual/spa.rs`) that produces a single self-contained HTML file with all six views as `<section>` containers (five hidden), client-side hash routing, and all view data embedded as always-present `<script type="application/json">` tags (empty data represented uniformly as `null`). The existing six per-view renderers remain unchanged and continue to emit individual files; the SPA dispatches to them. The CLI default `--visual-type` flips from `dashboard` to `all`, which routes to `render_spa`.

## Consequences
### Positive
- One page, shared state, no reloads; preserves the `file://` self-contained contract.
- Per-view renderers reused unchanged; no refactor of the data-builder functions.
### Negative
- Default output filename changes from `cxpak-dashboard.html` to `cxpak-all.html` — a visible behavior change for scripts.
- Whole-repo data is inlined; large repos can produce multi-MiB HTML. (A 10 MiB CLI warning and a 20 MiB test-fail cap were specified in the v2.1.0 design doc but were NOT implemented in shipped code — no size check exists in `src/commands/visual.rs` and no test asserts the cap.)
### Neutral
- Flow/Diff views ship empty-state cards until `--symbol` / `--files` are provided.

## Revisit if
- Repos routinely produce very large SPA HTML (the design-doc 20 MiB hard cap was never implemented; revisit if size becomes a real problem).
- Users demand a real JS shared cross-view store (deferred to v2.2.0).

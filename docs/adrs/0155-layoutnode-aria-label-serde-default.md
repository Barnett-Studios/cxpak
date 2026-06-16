---
id: '0155'
title: aria_label field on LayoutNode built in Rust, with serde(default) for cache back-compat and a content allowlist
status: ACCEPTED
date: 2026-04-17
triggered_by: Screen-reader support for graph nodes; v2.0.0 cached layouts must still deserialize
loop: planning
---

# ADR-0155: aria_label field on LayoutNode built in Rust, with serde(default) for cache back-compat and a content allowlist

## Context

Designed for cxpak v2.1.0. Graph nodes in the visual dashboard need ARIA labels for screen-reader
accessibility. Computing them in the controller JS would scatter the logic into untested code and
risk innocently embedding source content (file contents, doc-comments) into ARIA strings, leaking it
into accessibility trees. Adding a non-default struct field would also break deserialization of
v2.0.0 cached layout JSON.

## Options considered

- **Option A — Rust-computed `aria_label` field with `#[serde(default)]` and a documented field
  allowlist:** `build_aria_label()` reads only an allowlist (`label`, `node_type` with
  `member_ids.len()` for clusters, `risk_score`, `token_count`, and the three flag bools); the field
  is `#[serde(default)]` so old caches deserialize as the empty string; applied to the DOM via
  `setAttribute` only. Pros: backward compatible, prevents future source-content leakage into the
  a11y tree, testable in Rust. Cons: the allowlist is described in a doc-comment but not mechanically
  enforced by a shipped grep test. Chosen.
- **Option B — Compute ARIA labels in the controller JS:** A reasonable alternative would have been
  to derive labels client-side from node metadata. Pros: no struct change. Cons: logic scattered into
  untested JS with a higher risk of embedding source content, and harder to constrain. Someone could
  prefer it to avoid touching the serialized struct, but it sacrifices testability and the leakage
  guard.

## Decision

Add `aria_label: String` to `LayoutNode` with `#[serde(default)]` (so v2.0.0 cached layouts
deserialize to the empty string), compute it in Rust via `build_aria_label()` wired into the three
layout builders (`build_module_layout`, `build_file_layout`, `build_symbol_layout`, plus cluster
nodes), restrict its reads to a documented allowlist (`label`; `node_type` exposing only
`member_ids.len()`; `risk_score`; `token_count`; the three flag bools `is_god_file`,
`has_dead_code`, `is_circular`), and apply it to the DOM exclusively via `setAttribute` (the D3
`.attr('aria-label', ...)` path). The allowlist is documented in the `build_aria_label` doc-comment;
the design prescribed a grep test enforcing it, but no such test was found in the shipped code —
the field reads are currently covered only by behavioral unit tests.

## Consequences

### Positive
- Accessible graph nodes with back-compatible deserialization of older caches.
- The allowlist prevents accidental leakage of file contents or doc-comments into screen-reader and
  a11y surfaces.

### Negative
- The allowlist is enforced only by a documenting doc-comment plus behavioral unit tests on label
  content; the design-prescribed grep test was not found in shipped code.

### Neutral
- Both implementation plans and the design doc specify `#[serde(default)]`; the original plan added
  it as an explicit step, while the v2 plan folds it directly into the `LayoutNode` struct
  definition.

## Revisit if
- A new node property genuinely needs to appear in the label (requires a spec amendment).
- The allowlist grep test is added or confirmed dropped (update the enforcement claim accordingly).

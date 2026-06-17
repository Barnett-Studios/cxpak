---
id: '0152'
title: Accept inline JS/CSS (unsafe-inline) as required; cxpak serve sets a restrictive CSP, file:// runs without CSP
status: ACCEPTED
date: 2026-04-17
triggered_by: Self-contained HTML requires inlined script and style
loop: planning
---

# ADR-0152: Accept inline JS/CSS (unsafe-inline) as required; cxpak serve sets a restrictive CSP, file:// runs without CSP

## Context

Designed for cxpak v2.1.0. The self-contained, `file://`-openable dashboard contract requires
inlined JS and CSS in a single HTML file — incompatible with any CSP that forbids `unsafe-inline`.
A Content-Security-Policy posture had to be chosen for both the `file://` context and any served
context.

The original design (`docs/plans/2026-04-17-v210-design.md`, section 1.11) proposed that
`cxpak serve` host the dashboard under a CSP that permitted `unsafe-inline`. During v2.1.0
final-validation hardening (2026-04-27, commit e1b4494) this posture was reversed: as shipped,
`cxpak serve` emits only JSON and never serves the dashboard HTML, so it sets a strictly
no-inline CSP. The dashboard HTML is produced by the `cxpak_visual` MCP tool (returned inline or
written to `.cxpak/visual/*.html`) and opened via `file://`.

## Options considered

- **Option A — Accept `unsafe-inline`; have the server host the dashboard under a locked-down
  CSP:** `file://` has no CSP (browser behavior); `cxpak serve` would set
  `default-src 'self'; script-src 'unsafe-inline' 'self'; style-src 'unsafe-inline' 'self';
  img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'`; the controller must not use
  `eval`/`new Function`. Pros: inlined dashboard works when served while blocking external loads
  and framing. Cons: permits `unsafe-inline`. This was the design-time posture (grounded), but was
  **superseded** before shipping — `cxpak serve` never serves HTML.
- **Option B — Strict CSP with hashed/external scripts:** A reasonable alternative would have been
  to move JS/CSS to external files or per-script hashes to avoid `unsafe-inline` entirely. Pros:
  strongest CSP. Cons: breaks the single self-contained `file://` file contract. Someone could
  prefer this for the stronger posture, but it is incompatible with the self-contained dashboard.

## Decision

The `file://` context runs the self-contained dashboard (inlined JS/CSS) with no CSP at all —
browsers ignore CSP delivered from filesystem URIs, so inline content runs unrestricted. As
shipped, `cxpak serve` does **not** permit `unsafe-inline`: it serves only JSON and never hosts the
dashboard, so it sets `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`
(`src/commands/serve.rs:309-311`), making any mistyped HTML response inert. The unsafe-inline serve
CSP from the design doc was a planning-time posture, reversed during final-validation hardening.

The controller must not use `eval()` or `new Function()`, enforced by a grep-style test
(`tests/controller_dom_safety.rs::no_eval_or_function_constructor`). That test does not check
string-argument `setTimeout`/`setInterval`.

## Consequences

### Positive
- The self-contained dashboard works everywhere via `file://`; the served context emits JSON only
  under a strict no-inline CSP and blocks framing.

### Negative
- The `file://` context has no CSP at all — inlined content runs unrestricted on the local
  filesystem.

### Neutral
- The no-`eval` constraint is enforced by a controller grep test, limited to `eval(`/`new Function(`
  (not string-argument timers).
- The design doc's serve CSP string describes a posture the shipped code diverges from at
  `serve.rs:309-311`.

## Revisit if
- A future build step allows hashed scripts without breaking the self-contained contract.
- `cxpak serve` ever starts hosting the dashboard HTML (it would then need an inline-permitting CSP).

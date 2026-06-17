---
id: '0154'
title: All user-derived strings reach the DOM only via textContent/setAttribute; all JSON tags pass through escape_script_tag
status: ACCEPTED
date: 2026-04-17
triggered_by: File paths, symbol names, ARIA labels, and router params are untrusted data embedded into HTML
loop: planning
---

# ADR-0154: All user-derived strings reach the DOM only via textContent/setAttribute; all JSON tags pass through escape_script_tag

## Context

Designed for cxpak v2.1.0. The SPA embeds repo-derived strings (file paths, symbol names, ARIA
labels) into inline JSON and renders them in the palette and inspector. A file path can legitimately
contain HTML metacharacters or `</script>`, and router params come from the URL. A consistent
injection-safety model was required so that untrusted data cannot become executable markup.

## Options considered

- **Option A — `textContent`/`setAttribute` only, plus `escape_script_tag` on every JSON tag and a
  router-param allowlist:** No `innerHTML`/`outerHTML`/`document.write`; every
  `<script type="application/json">` tag passes through `escape_script_tag()`; router params are
  validated against `^[A-Za-z0-9._/\-]{1,512}$`; NUL bytes rejected and bidi control chars flagged;
  no `eval`/`new Function`. Pros: defense-in-depth, mechanically enforced by grep tests on the
  controller plus a render-time escape test. Cons: more verbose DOM construction than `innerHTML`
  templating. Chosen.
- **Option B — `innerHTML` templating with manual escaping:** A reasonable alternative would have
  been to build markup strings and assign them via `innerHTML`, escaping interpolated values. Pros:
  less DOM-construction code. Cons: one missed escape is an XSS hole, and correctness is hard to
  enforce mechanically. Someone could prefer it for brevity, but it trades a structural guarantee for
  per-call vigilance.

## Decision

Treat all labels, paths, symbol names, and router params as untrusted. Insert them into the DOM only
via `textContent` or `setAttribute` (never `innerHTML`/`outerHTML`/`document.write`); route every
embedded JSON tag through `escape_script_tag()` (promoted to `pub(crate)` and used at every JSON
embed site); validate router params against `^[A-Za-z0-9._/\-]{1,512}$` (replacing failures with the
empty string); reject NUL bytes (`src/visual/search_index.rs`) and flag bidi control chars
(`sanitize_bidi` in `src/util.rs`); and forbid `eval`/`new Function`. Enforced by
`tests/controller_dom_safety.rs` (grep over the controller asset) and a render-time
all-tags-escaped test (`tests/spa_injection_safety.rs`).

The design specified highlighting matched palette characters via `createElement('mark')`. This was
not shipped — the palette renders result labels via plain `textContent` with no `<mark>` wrapping
(`createElement` is used only for `div`/`span`/`button`/`code`). The `textContent`-only invariant is
fully intact without it.

## Consequences

### Positive
- XSS via file paths, symbol names, or URL params is structurally prevented.
- Enforcement is mechanical (CI grep over `assets/cxpak-spa-controller.js` plus render tests) rather
  than reliant on reviewer vigilance.

### Negative
- DOM-building code in the controller is more verbose than `innerHTML` templating.

### Neutral
- `escape_script_tag` was promoted to `pub(crate)` and is confirmed used at every JSON embed site in
  shipped code.

## Revisit if
- A new data tag is added without routing through `escape_script_tag` (the all-tags test guards this).

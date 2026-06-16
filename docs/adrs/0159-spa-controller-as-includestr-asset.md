---
id: '0159'
title: SPA controller JS lives in an asset file inlined via include_str!, not a Rust string literal
status: ACCEPTED
date: 2026-04-17
triggered_by: Need ~300-800 lines of controller JS for routing/palette/inspector/theme/keyboard
loop: planning
---

# ADR-0159: SPA controller JS lives in an asset file inlined via include_str!, not a Rust string literal

## Context
The v2.1.0 single-page visual dashboard needs a substantial client-side controller (hash routing, command palette, inspector, theme toggle, keyboard handling — several hundred lines of JS). The team had to decide whether to embed that controller as a Rust string literal (the original implementation plan's implicit approach) or as a standalone asset file inlined into the HTML at render time, the same way the D3 bundle and CSS are handled.

## Options considered
- **Option A — standalone `assets/cxpak-spa-controller.js` inlined via `include_str!`:** the controller is a real `.js` file, edited with JS tooling, inlined into the HTML at render time like `d3-bundle.min.js` and `cxpak-visual.css`. Pros: lintable, readable in review, real sourcemaps in devtools, prettier/eslint usable; the self-contained-HTML contract is preserved via inlining. Cons: verified by grep assertions plus a golden fixture rather than unit tests, since there is no headless browser in the suite. This is the chosen option.
- **Option B — controller as a Rust string literal:** embed the ~500-line JS directly as a string in the render code. Pros: single source file; trivially compiled in. Cons: unlintable, unreadable in code review, terrible devtools sourcemaps, no JS editor tooling. Someone could prefer it to keep everything in one Rust module, but the four drawbacks outweigh that.

## Decision
Author the controller as `assets/cxpak-spa-controller.js` and inline it with `include_str!` at render time (`src/visual/spa.rs` declares `static SPA_CONTROLLER`, the same pattern as the D3 bundle and CSS). Verify it via Rust-side grep/regex assertions (`tests/controller_dom_safety.rs`, with a non-empty guard against vacuous passes), HTML-output integration tests, and a committed cross-process golden fixture (`tests/snapshots/spa_golden.html`, diffed by `tests/spa_determinism.rs`). This is explicitly NOT TDD: unit-testing the JS would require jsdom + node and violate the one-Rust-binary / no-npm principle.

## Consequences
### Positive
- Controller is reviewable and editable with standard JS tooling (prettier, eslint, sourcemaps).
- Self-contained HTML contract preserved — the controller is inlined at render time.
### Negative
- A logic bug invisible to grep and JSON-output assertions can survive CI, caught only at manual smoke test.

## Revisit if
- v2.2.0 adds headless-browser (Playwright) tests.
- The controller's logic surface grows beyond what grep plus output parsing can cover.

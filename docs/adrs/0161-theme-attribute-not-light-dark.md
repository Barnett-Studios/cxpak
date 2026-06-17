---
id: '0161'
title: Attribute-based theming (data-theme on <html>) instead of CSS light-dark()
status: ACCEPTED
date: 2026-04-17
triggered_by: Original implementation plan proposed CSS light-dark() / .light-mode; design spec reversed it
loop: planning
---

# ADR-0161: Attribute-based theming (data-theme on <html>) instead of CSS light-dark()

## Context
The v2.1.0 visual dashboard needs light/dark theme switching, which requires a CSS mechanism. The first implementation plan specified the native CSS `light-dark()` function with a `.light-mode` override. The converged design spec reversed that approach.

## Options considered
- **Option A — attribute-based `[data-theme="light"]` on `<html>`:** the theme is applied via a `data-theme` attribute, and CSS keys palette overrides off the attribute selector. Pros: deterministic control over every element regardless of OS preference; form controls and scrollbars never flip. Cons: slightly more CSS than a single function. This is the chosen option.
- **Option B — CSS `light-dark()` function:** use the native `light-dark()` with `color-scheme: light dark`. Pros: compact, native. Cons: `color-scheme: light dark` flips form controls and scrollbars to OS preference even when the theme is explicitly pinned. Someone could prefer it for the smaller stylesheet, but the OS-preference leakage is disqualifying.

## Decision
Use CSS attribute-based theming via `[data-theme="light"]` on `<html>`, not the CSS `light-dark()` function. `<html>` ships with `data-theme="dark"` and the controller toggles via `setAttribute('data-theme', ...)`. The light palette is a first-class design (accent hues shifted ~15% darker for contrast on off-white `#f8f9fc`, body text `#1a1a2e` rather than pure black), not a naive inversion of the dark palette. This supersedes the original implementation plan's `light-dark()` / `.light-mode` approach.

## Consequences
### Positive
- Deterministic control over every element; form controls and scrollbars never flip against the pinned theme.
- Light palette tuned for contrast, not a naive inversion.
### Negative
- Two parallel palettes to maintain in CSS.
### Neutral
- Shipped code confirms `data-theme="light"` selectors and zero `.light-mode` usage.

## Revisit if
- Browser support for `light-dark()` with finer `color-scheme` control improves.

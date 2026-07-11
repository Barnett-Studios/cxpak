---
id: '0172'
title: One design language + a client-side palette system
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0172: One design language + a client-side palette system

**Context.** The redesign needs a strong, ownable visual identity, but users demand their own color schemes (Tokyo Night, Catppuccin, Gruvbox, …). The SPA is a single self-contained HTML file with a byte-identical golden fixture (ADR-0151) and a no-external-origin invariant (`!html.contains("cdn.jsdelivr.net")`, asserted at `render.rs:3215`, `tests/spa_render.rs:93`, `tests/visual_cli.rs:405`). Webfonts are therefore off the table.

**Options considered.**
1. *One fixed theme.* Simplest; strongest identity; rejected — users have entrenched palette loyalties and competitors ship pickers.
2. *Three separate design languages* (Signal-Bench / Ledger / Blueprint) selectable at runtime. Rejected — 3× design + test surface, and it dilutes the signature that is supposed to be the moat.
3. *One design language ("Blueprint") + a palette picker.* The VS Code / Obsidian model: identity carried by structure/typography/the proof-tick motif (ADR-0174), color carried by a swappable token set.

**Decision.** Option 3. "Blueprint" = graph-paper grid as chrome only, hairline borders, square corners, zero-blur elevation, monospace data with `tabular-nums`, system-sans body (**no webfont → no-CDN satisfied for free**). Palettes are a data table of btop-schema token sets (`bg/surface/ink/ink2/hair/accent/lo/mid/hi`, mirroring the user's `mticky` `.theme` files), applied at runtime via CSS custom properties. Ship ~19 palettes; **Tokyo Night is the default**. A `.cxpak/palettes/*.toml` drop-in path admits community palettes at ~zero core cost.

**Consequences.** Palette switching is pure client-side state → **emitted bytes are identical regardless of selection** → the golden fixture is unaffected (ADR-0151 preserved; see ADR-0179's determinism note). Light/dark both first-class (each palette declares its pair). Cost: a ~19-entry token table to maintain + a validated ramp per palette (the dataviz grayscale-survival test in ADR-0179 gates this). An optional generation-time `--palette <name>` flag changes only the *initial* token block → still deterministic per input.

**Revisit if.** Palette count outgrows a hand-maintained table (→ move to a build-time generated registry), or a future requirement forces a webfont (→ inline as base64 data-URI, never a CDN link).

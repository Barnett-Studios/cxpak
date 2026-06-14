---
id: '0137'
title: Canonical onboarding logic in intelligence module; visual layer is render-only
status: ACCEPTED
date: 2026-04-01
triggered_by: Avoiding duplicate onboarding computation between visual and intelligence modules
loop: implementation
---

# ADR-0137: Canonical onboarding logic in intelligence module; visual layer is render-only

## Context

Shipped in v2.0.0. Both the visual module (`src/visual/onboard.rs`) and the intelligence module could host onboarding computation. Duplicated logic risks divergence between the CLI, MCP, and visual surfaces.

## Options considered

- **Option A — single source of truth in `src/intelligence/onboarding.rs`:** All shared computation (topo sort, phase grouping, reading time) and the shared types live in `src/intelligence/onboarding.rs`; `src/visual/onboard.rs` re-exports the types, provides the orchestrating entry point, and renders markdown/JSON. Pros: no duplicated logic, one place to change ordering rules, consistent across CLI/MCP/visual. Cons: a cross-module dependency from visual into intelligence. This is what shipped.
- **Option B — compute independently in the visual module:** A reasonable alternative would have been to implement onboarding logic where it is rendered, keeping the visual module self-contained. It avoids the cross-module dependency but duplicates logic and is divergence-prone. Reconstructed alternative; not formally evaluated.

## Decision

The canonical onboarding computation (`topological_sort_files`, `group_into_phases`, `format_reading_time`) and the shared types (`OnboardingMap`, `OnboardingPhase`, `OnboardingFile`) live in `src/intelligence/onboarding.rs`; `src/visual/onboard.rs` is a thin layer that re-exports those types, provides the orchestrating entry point `compute_onboarding_map` (which delegates to the intelligence functions), and provides `render_onboarding_markdown` / `render_onboarding_json`.

## Consequences

### Positive
- No duplicated onboarding logic; ordering rules change in one place.
- Consistent onboarding output across CLI, MCP, and visual surfaces.

### Negative
- The visual rendering layer depends on the intelligence module.

### Neutral
- The onboarding logic in intelligence is feature-gated `visual` in shipped code.

## Revisit if
- Onboarding needs visual-specific computation not shared with text surfaces.

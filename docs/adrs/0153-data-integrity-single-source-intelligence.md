---
id: '0153'
title: Every SPA number must equal the intelligence function output via JSON round-trip comparison
status: ACCEPTED
date: 2026-04-17
triggered_by: SPA, CLI, MCP, and LSP must not diverge in displayed numbers
loop: planning
---

# ADR-0153: Every SPA number must equal the intelligence function output via JSON round-trip comparison

## Context

Designed for cxpak v2.1.0. The same intelligence (health, risk, architecture, etc.) is surfaced
through four channels: the SPA dashboard, the `/v1` HTTP API, the MCP tools, and the LSP custom
methods. Without an enforced contract, each channel could recompute or round values independently,
producing inconsistent numbers (off-by-one counts, third-decimal rounding drift), undermining
trust in the dashboard.

## Options considered

- **Option A — Shared intelligence functions plus JSON round-trip / `to_bits()` integrity tests and
  a cross-channel matrix:** No duplicated computation; integrity tests extract the embedded JSON tag
  and compare numeric fields via a `serde_json` round-trip and `f64::to_bits()` for composite scores;
  a per-cell cross-channel test matrix enforces agreement across SPA/v1/MCP/LSP. Pros: catches
  third-decimal drift and HashMap-order nondeterminism; one source of truth. Cons: a large test
  matrix to maintain. Chosen.
- **Option B — Substring-match formatted values against raw HTML:** Assert the formatted score
  string appears somewhere in the rendered HTML. Pros: simple to write. Cons: misses precision drift
  (the formatted string can match while the underlying value differs) and is vulnerable to
  coincidental substring matches. Someone could prefer it for its low effort, but it does not detect
  the precision divergence this contract targets.

## Decision

Establish the hard invariant that every number displayed in the SPA comes from the same
intelligence function that powers the CLI and MCP tools — no duplicated computation. Enforce it
with integrity tests (`tests/cross_channel_consistency.rs`) that parse the specific embedded JSON
tag and compare numeric fields through a `serde_json` round-trip and `f64::to_bits()` for composite
scores (`compute_health.composite`, `compute_risk` `risk_score`, `build_architecture_map`), plus a
cross-channel consistency matrix (one test per cell) covering SPA/v1/MCP/LSP. Scores are formatted
by a single shared JS helper `CX.format.score` (`assets/cxpak-spa-controller.js`); every `toFixed`
must live inside it, enforced by `tests/controller_dom_safety.rs`.

## Consequences

### Positive
- No off-by-one counts and no rounding divergence across channels.
- Nondeterminism (e.g. HashMap order) is caught by the cross-process golden fixture
  (`tests/spa_determinism.rs`).

### Negative
- The cross-channel matrix and integrity tests add maintenance surface.

### Neutral
- Dashed matrix cells are intentionally not tested (the function is not surfaced on that channel).

## Revisit if
- A new channel or intelligence function is added (extend the matrix and integrity tests).

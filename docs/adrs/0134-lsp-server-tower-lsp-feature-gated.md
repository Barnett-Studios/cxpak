---
id: '0134'
title: LSP server over stdio via tower-lsp behind an lsp feature flag, with 4 standard + 14 custom methods
status: ACCEPTED
date: 2026-04-01
triggered_by: "v1.6.0 'The Platform': exposing cxpak intelligence to editors via an LSP server"
loop: implementation
---

# ADR-0134: LSP server over stdio via tower-lsp behind an lsp feature flag, with 4 standard + 14 custom methods

## Context

Shipped in v1.6.0 ("The Platform"). Editors speak LSP. cxpak wanted to expose its intelligence as both standard LSP features (code lens, hover, diagnostics, workspace symbols) and bespoke `cxpak/*` queries. `tower-lsp` 0.20 uses `tower` 0.4 internally while `axum` 0.8 uses `tower` 0.5, raising a dependency-coexistence risk that had to be validated before committing to the library.

## Options considered

- **Option A — tower-lsp 0.20 over stdio, feature-gated:** Add `src/lsp/` behind `lsp = ["dep:tower-lsp", "daemon"]` (included in `default`), running over stdio and reusing `build_index` plus the daemon `FileWatcher` for index freshness. Declare the 4 standard capabilities (codeLens, hover, diagnostic, workspace/symbol) and register 14 custom `cxpak/*` JSON-RPC methods via `LspService::build().custom_method(...)` rather than `execute_command`. Pros: standard editor integration; reuses the hot `CodebaseIndex` and watcher; the feature flag keeps the dependency optional. Cons: two `tower` versions coexist in the dependency graph; couples cxpak to tower-lsp's evolving API. This is what shipped.
- **Option B — hand-rolled JSON-RPC server:** A reasonable alternative would have been to implement the LSP protocol directly without tower-lsp, avoiding the tower version coexistence concern. It would reimplement the entire LSP framing and capability machinery from scratch, which is substantial work for no functional gain. Reconstructed alternative; the plan only discussed a fork/vendor fallback, not a hand-rolled server.

## Decision

Add an LSP server in `src/lsp/` behind an `lsp = ["dep:tower-lsp", "daemon"]` feature flag (added to `default`), built on `tower-lsp` 0.20 running over stdio and reusing `build_index` plus the daemon `FileWatcher` for index freshness. Implement the 4 standard methods (codeLens, hover, diagnostic, workspace/symbol) plus 14 custom `cxpak/*` JSON-RPC methods registered via `LspService::build().custom_method` (explicitly NOT `execute_command`). A documented hard gate runs `cargo check --all-features` first to confirm `tower-lsp` 0.20 (`tower` 0.4) and `axum` 0.8 (`tower` 0.5) coexist before any implementation.

## Consequences

### Positive
- cxpak intelligence is available natively in LSP editors.
- Reuses the shared hot index and watcher rather than a separate process.
- Optional via the feature flag; can be excluded from builds.

### Negative
- Carries two `tower` versions in the dependency graph.
- Coupled to tower-lsp's evolving API.

### Neutral
- Custom methods with required params return JSON-RPC `-32603 InternalError` on missing params, per the shipped `CLAUDE.md`.

## Revisit if
- `tower-lsp` and `axum` tower versions stop coexisting on the shared `http` crate.
- A newer LSP library supersedes tower-lsp.

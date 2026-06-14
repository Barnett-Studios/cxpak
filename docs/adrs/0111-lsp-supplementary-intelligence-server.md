---
id: '0111'
title: 'LSP server is supplementary intelligence over stdio (14 custom cxpak/* methods), gated behind lsp feature depending on daemon'
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.6.0 'The Platform' — give every IDE native access to cxpak intelligence
loop: planning
---

# ADR-0111: LSP server is supplementary intelligence over stdio (14 custom cxpak/* methods), gated behind lsp feature depending on daemon

## Context

Part of v1.6.0 ("The Platform"). To reach IDEs, cxpak can either build per-editor plugins or expose a Language Server Protocol server. The server should not duplicate the editor's own LSP (autocomplete, syntax highlighting) but add intelligence no other language server provides.

## Options considered

- **Option A — Supplementary LSP over stdio with standard + 14 custom methods, `lsp` feature gated on `daemon`:** Implement the standard LSP methods (codeLens, diagnostic, hover, workspace/symbol) repurposed for intelligence, plus 14 custom `cxpak/*` JSON-RPC methods; run as `cxpak lsp` over stdio reusing the watch index; feature flag `lsp = ["dep:tower-lsp", "daemon"]`, included in `default`. Pros: one server reaches every LSP-capable IDE; runs alongside the language's own LSP; reuses the watcher/incremental infra; intelligence-only scope avoids competing with native servers. Cons: `tower-lsp` must be compatible with axum 0.8 (shared tower/hyper/http versions); custom methods are non-standard, so clients need cxpak-specific glue. Someone could prefer it for single-binary distribution across all editors.

- **Option B — Per-editor native extensions:** A reasonable alternative would have been to build separate VS Code / JetBrains / etc. plugins. Pros: deeper, idiomatic editor integration. Cons: N plugins to maintain, no single distribution, far more work. Someone could prefer it where the deepest possible editor integration outweighs maintenance cost.

## Decision

Ship a supplementary LSP server (`cxpak lsp` over stdio) that provides intelligence rather than autocomplete: standard methods (`textDocument/codeLens`, diagnostic, hover, `workspace/symbol`) repurposed to surface health/risk/dead-code/convention data, plus 14 custom `cxpak/*` JSON-RPC methods: `health`, `conventions`, `blastRadius`, `overview`, `trace`, `diff`, `search`, `apiSurface`, `deadCode`, `callGraph`, `predict`, `drift`, `securitySurface`, `dataFlow`. (The original design-doc list — `risks`, `architecture`, `crossLang`, `briefing`, `coChanges` — was superseded; those five were not shipped as `cxpak/*` methods and were replaced by `overview`, `trace`, `diff`, `search`, `apiSurface`.) It maintains a hot index via the file watcher and runs alongside the language's own server. Feature flag `lsp = ["dep:tower-lsp", "daemon"]`, added to `default`; `tower-lsp`/axum 0.8 http-crate compatibility was verified before implementation.

## Consequences

### Positive
- Reaches every LSP-capable IDE from one binary.
- Reuses the watcher and incremental indexing.
- Coexists with the language's native LSP.

### Negative
- Custom `cxpak/*` methods require client-side glue.
- Dependency-version compatibility risk between `tower-lsp` and axum 0.8.

### Neutral
- `lsp` depends on the `daemon` feature, which already gates tokio/axum, so enabling LSP pulls in the daemon stack.

## Revisit if
- `tower-lsp` and axum 0.8 cannot share compatible http/tower versions.
- Standard LSP clients need the custom methods exposed differently.

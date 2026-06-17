---
id: '0038'
title: Gate daemon (notify/axum/tokio) behind a non-default feature flag
status: ACCEPTED
date: 2026-03-12
triggered_by: Daemon mode adds an async runtime and HTTP server not needed by the CLI
loop: planning
---

# ADR-0038: Gate daemon (notify/axum/tokio) behind a non-default feature flag

## Context

The v0.8.0 integrations work introduces a daemon mode that requires `notify` (file watcher), `axum` (HTTP server), and `tokio` (async runtime). These are heavy dependencies that the one-shot CLI does not need, and compiling them into every build would bloat the default binary with an async stack the common path never exercises.

## Options considered

- **Option A — `daemon` feature flag, not in default:** Add `notify` 7, `axum` 0.8, and `tokio` 1 (full) as optional dependencies exposed only under a `daemon` feature; keep `daemon` out of the default feature list so the plain CLI stays lean. CI release builds enable `--features daemon`. Pro: lean default CLI, async/HTTP deps compiled only when needed, both CLI and daemon build. Con: two build configurations to test (with and without `daemon`). Someone could prefer this because it keeps the default user's compile time and binary size low. (Considered and chosen.)
- **Option B — Always compile daemon deps:** A reasonable alternative would have been to make `notify`/`axum`/`tokio` mandatory dependencies. Pro: a single build matrix, simpler CI. Con: bloats every CLI build with an async runtime and HTTP stack. Someone could prefer it to avoid maintaining two build/test/clippy configurations.

## Decision

Put `notify`, `axum`, and `tokio` behind an optional `daemon` feature flag and deliberately exclude `daemon` from the default feature list so the plain CLI stays lean; release CI builds enable `--features daemon`. The `watch` and `serve` subcommands are `cfg`-gated on the `daemon` feature.

Code confirms the boundary was later loosened: in the shipped `Cargo.toml`, `daemon` now appears in the default feature list, and the `lsp` feature depends on `daemon` (`lsp = ["dep:tower-lsp", "daemon"]`).

## Consequences

### Positive
- The default CLI build avoids the async runtime and HTTP server.
- Both `cargo build` and `cargo build --features daemon` are supported configurations.

### Negative
- Two build/test/clippy configurations to maintain.

### Neutral
- The `watch` and `serve` subcommands are `cfg`-gated on the `daemon` feature.
- The boundary later evolved: `daemon` became part of the default feature set and a dependency of the `lsp` feature.

## Revisit if
- Daemon becomes core enough to always compile (it later did).
- Async deps need to be shared with another feature such as `lsp` (they later were).

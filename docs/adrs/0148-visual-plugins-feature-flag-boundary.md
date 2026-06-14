---
id: '0148'
title: Gate visual (resvg) and plugins (wasmtime) behind feature flags
status: ACCEPTED
date: 2026-04-01
triggered_by: Cargo.toml dependency footprint for v2.0.0
loop: implementation
---

# ADR-0148: Gate visual (resvg) and plugins (wasmtime) behind feature flags

## Context

Shipped in v2.0.0. `resvg` and `wasmtime` are heavy dependencies (resvg adds ~2MB to the binary). The plan had to decide whether these are always compiled or gated so users can build a leaner binary.

## Options considered

- **Option A — Optional deps behind feature flags:** `resvg` behind `visual`, `wasmtime` behind `plugins`. The plan proposed including both in the default feature set, removable via `--no-default-features`. Pros: a full-featured default build; users who don't need PNG/WASM can drop the weight. Cons: more feature-flag plumbing and cfg-gating across modules; a matrix of build configs to test. This option shipped, but with a deviation: `visual` is in the default set, while `plugins` was deliberately excluded from default (see Decision).
- **Option B — Always compile resvg and wasmtime:** Make both mandatory dependencies. Pros: simpler build, no cfg gates. Cons: forces binary bloat on every user, including those who never render PNG or run plugins. A reasonable alternative would have been this to avoid cfg complexity; it was rejected because it imposes the heavy deps on everyone.

## Decision

`resvg` is an optional dep behind feature `visual = ["dep:resvg"]` and `wasmtime` behind `plugins = ["dep:wasmtime"]`.

- `visual` IS in the default feature set and can be opted out with `--no-default-features`.
- `plugins` is NOT in the default set. The plan proposed including it, but the shipped code reversed that decision: the WASM loader is a known stub that errors with "guest function binding not yet implemented," and shipping it as a default feature would have made `cxpak plugin add` look functional while the loader was non-functional. It is opt-in via `--features plugins`.

The shipped code also expanded `visual` to additionally pull `petgraph` + `thiserror` and bumped to resvg 0.47 / wasmtime 43 (the plan referenced resvg 0.44 / wasmtime 28).

## Consequences

### Positive
- Users can build a lean binary with `--no-default-features --features daemon,embeddings`.
- PNG rasterization is cfg-gated so non-users pay no cost; the WASM plugin host is opt-in.

### Negative
- Adds cfg-gating throughout `src/visual` and `src/plugin`.
- Multiple build configurations must be checked in CI.
- The `plugins` feature was dropped from the default set after the plan because its loader is a non-functional stub.

### Neutral
- Plugin SDK types in `src/plugin/mod.rs` are gated behind `#[cfg(feature = "plugins")]` (NOT always compiled); the entire plugin module compiles out unless `--features plugins` is set. (The project `CLAUDE.md` still describes these types as "always-compiled," which is stale relative to the code.)

## Revisit if
- The resvg/wasmtime binary-size cost changes materially.
- A lighter PNG or WASM backend becomes available.
- The WASM loader's guest binding is implemented, at which point `plugins` could rejoin the default set.

---
id: '0149'
title: WASM as the plugin execution model, hosted via wasmtime
status: ACCEPTED
date: 2026-04-01
triggered_by: Plugin SDK design (Tasks 21-22)
loop: implementation
---

# ADR-0149: WASM as the plugin execution model, hosted via wasmtime

## Context

Shipped in v2.0.0. cxpak needs third-party extensibility (custom analyzers, detectors, output formats). The execution model must be sandboxable and language-agnostic, since plugins may come from untrusted sources.

## Options considered

- **Option A — WASM plugins loaded by wasmtime, host trait `CxpakPlugin`:** Plugins ship as `.wasm` binaries; the host wraps each in `Box<dyn CxpakPlugin>` via wasmtime; capabilities are `Analyzer`/`Detector`/`OutputFormat`; the WIT interface mirrors the trait. Pros: sandboxed, language-agnostic, capability-scoped, size-limitable. Cons: WASM toolchain complexity; v2.0.0 ships only a skeleton (guest binding not implemented). This is the shipped design.
- **Option B — Native dynamic-library (dlopen) plugins:** Load shared objects implementing the trait directly. Pros: full performance, simpler ABI to the host language. Cons: no sandbox, ABI fragility, platform-specific, unsafe to run untrusted code. A reasonable alternative would have been this for trusted first-party plugins where performance dominates; it was rejected because it cannot safely run untrusted third-party code.

## Decision

Plugins are WASM binaries hosted via wasmtime. The SDK in `src/plugin/mod.rs` defines `PluginCapability` (`Analyzer`/`Detector`/`OutputFormat`), snapshot types, and the `CxpakPlugin` trait. v2.0.0 ships `PluginLoader` as a skeleton that validates (10MB size cap, SHA256 checksum) and instantiates the module but returns a descriptive `Err` for the unimplemented guest binding (no `todo!`/`unimplemented!` per `CLAUDE.md`).

## Consequences

### Positive
- Sandboxed, language-agnostic extension model.
- The runtime is gated behind the `plugins` feature.

### Negative
- v2.0.0 cannot actually run plugin guest functions — `load()` returns a descriptive `Err` for the unfinished binding.

### Neutral
- wasmtime version pinned (plan referenced 28; shipped code uses 43).

## Revisit if
- The guest WIT binding is implemented in a later version.
- WASM overhead proves too high for plugin workloads.

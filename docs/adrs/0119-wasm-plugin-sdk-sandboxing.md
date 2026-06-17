---
id: '0119'
title: Plugin SDK uses WASM (wasmtime) with declared file-pattern scoping and 1MB return limits for sandboxing
status: ACCEPTED
date: 2026-03-31
triggered_by: v2.0.0 plugin/extension SDK for third-party analyzers
loop: planning
---

# ADR-0119: Plugin SDK uses WASM (wasmtime) with declared file-pattern scoping and 1MB return limits for sandboxing

## Context

Released in v2.0.0 (the plugin SDK capstone). Third-party plugins need to extend cxpak with custom analyzers, detectors, and output formats. That code is untrusted and must run safely and portably, without per-platform FFI ABI headaches and without remote-loading risk. The design therefore needs an execution model that sandboxes guest code, scopes its data access, and bounds what it can return.

## Options considered

- **Option A — WASM via wasmtime, manifest with checksum + file-pattern scoping, content-stripped by default, 1MB return cap (chosen):** The `CxpakPlugin` trait compiles to WASM and loads via the `wasmtime` crate; plugins register in `.cxpak/plugins.json` with name + path + SHA-256 checksum + declared file patterns; the host provides only matching files in `IndexSnapshot` and strips file contents unless the plugin declares `needs_content: true` (warned on first load); `Vec<Finding>` returns are capped at 1MB; local files only. Pros: sandboxed, cross-platform, no FFI; plugins in any WASM-targeting language; mature runtime; checksum + pattern scoping + content stripping + return cap limit data exfiltration. Cons: the WASM guest-binding bridge is non-trivial; the least-privilege model needs per-plugin manifest discipline.
- **Option B — native dynamic-library (FFI) plugins:** The design names FFI explicitly as the thing WASM avoids. A `.so`/`.dll` plugin loaded via FFI. Pros: no WASM runtime overhead. Cons: no sandboxing, platform-specific ABI headaches, unsafe execution of untrusted code. Rejected — running untrusted code with no sandbox is the core risk WASM removes.

## Decision

Implement the plugin SDK as WASM plugins loaded via the `wasmtime` crate, exposing a `CxpakPlugin` trait (`name`/`version`/`capabilities`/`analyze`/`detect`) with `PluginCapability` variants `Analyzer`/`Detector`/`OutputFormat`. Plugins register in `.cxpak/plugins.json` with name + path + SHA-256 checksum + declared file patterns.

Sandboxing: the host provides only files matching declared patterns in `IndexSnapshot`, strips file contents by default (plugins needing raw content must declare `needs_content: true`, warned on first load), caps `Vec<Finding>` returns at 1MB to prevent exfiltration, and loads local files only.

NOTE: the shipped code excludes `plugins` from default features because `PluginLoader::load()` is still a stub — it returns an error with the literal text "guest function binding not yet implemented" (`src/plugin/loader.rs`). All the surrounding machinery (trait, manifest, checksum verify, pattern scoping, content stripping, 1MB cap) is implemented; only the WASM guest-binding bridge is unfinished.

## Consequences

### Positive
- Sandboxed, cross-platform plugin execution with no FFI.
- Plugins authorable in any WASM-targeting language.
- Least-privilege via file-pattern scoping, content stripping, checksum verification, and the 1MB return cap.

### Negative
- The WASM guest-binding bridge is unfinished — the `plugins` feature is gated out of default and `PluginLoader::load()` errors as a stub.
- Per-plugin manifest discipline is required for the least-privilege model.

### Neutral
- The manifest enforces declared paths via `.cxpak/plugins.json`; there is no enforced `.cxpak/plugins/` directory location.

## Revisit if
- The WIT/guest-binding bridge is completed and plugins can move back into default features.
- The 1MB finding cap or the content-stripping default proves too restrictive.

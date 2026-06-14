---
id: '0139'
title: Plugin security model: checksum + pattern scoping + content opt-in + size limits
status: ACCEPTED
date: 2026-04-01
triggered_by: Plugin manifest and security design (Tasks 23-24)
loop: implementation
---

# ADR-0139: Plugin security model: checksum + pattern scoping + content opt-in + size limits

## Context

cxpak v2.0.0 introduced a WASM plugin SDK (`src/plugin/`). Untrusted WASM plugins must be constrained along four axes: which files they can see, whether they receive raw source content, how much data they can return to the host, and verification that the loaded binary has not been swapped. A plugin loaded from a configured path with full index access and unbounded return payloads is a tamper, data-exposure, and DoS hazard.

## Options considered

- **Option A — Multi-layer manifest-driven security (chosen):** The `.cxpak/plugins.json` manifest declares a SHA-256 checksum (verified on load), glob `file_patterns` scoping which files appear in the `IndexSnapshot` (empty = none visible, per the manifest doc-comment), a `needs_content` opt-in (default `false`, warns the user on first load), a 10 MB binary size cap, and a 1 MB serialized return-payload cap. Pros: defense in depth — tamper detection, least-privilege file access, explicit content consent, bounded resource exposure. Cons: larger manifest surface and checksum maintenance whenever a plugin binary updates. Preferred because each layer addresses a distinct, independent threat.
- **Option B — Trust-by-path with no manifest constraints:** A reasonable alternative would have been to load any `.wasm` on a configured path with full index access and unbounded returns. Pros: simplest possible configuration, no manifest to maintain. Cons: no tamper detection, full data exposure to plugin code, and DoS risk from arbitrarily large returns. Someone could prefer it for a fully trusted, single-author plugin set where the operational simplicity outweighs the absent guardrails.

## Decision

Plugins are governed by `.cxpak/plugins.json`. Each entry carries:

- A **SHA-256 checksum** verified on load (`manifest.rs` `verify_checksum`, timing-safe via `subtle::ConstantTimeEq`); `loader.rs::load()` verifies the checksum before wasmtime compilation.
- Glob **`file_patterns`** scoping which files populate the `IndexSnapshot`. The manifest doc-comment defines empty patterns as "no files visible"; non-empty patterns filter to the matched set.
- A **`needs_content`** flag (default `false` via `#[serde(default)]`) controlling whether `FileSnapshot.content` is populated; `warn_if_needs_content` surfaces a trust warning on first load.
- A **10 MB binary size cap** (`MAX_PLUGIN_BYTES`, enforced in `loader.rs`).
- A **1 MB serialized-JSON return-payload cap** (`MAX_RETURN_BYTES`) on findings and detections (`enforce_return_limit`, `enforce_detection_limit`).

A missing manifest is treated gracefully as zero plugins.

Note: the documented intent is that empty `file_patterns` means "no files visible," but the shipped `patterns_match` treats empty patterns as match-all. This is an implementation deviation from the documented least-privilege default and is worth reconciling.

## Consequences

### Positive
- Tamper detection via per-plugin checksum, verified before compilation.
- Least-privilege file visibility via glob patterns.
- Raw content access is explicit and user-warned, not implicit.
- Return-payload and binary-size limits bound resource and DoS exposure.

### Negative
- Checksums must be updated whenever a plugin binary changes.
- `file_patterns` and the various limits add manifest configuration burden.

### Neutral
- `sha2` dependency added behind the `plugins` feature for checksum verification.
- Missing manifest is handled gracefully as zero plugins rather than an error.

## Revisit if
- Plugins legitimately need to return payloads larger than 1 MB.
- A richer capability/permission model (beyond file-pattern scoping) is needed.
- The empty-`file_patterns` semantics need to be reconciled with the documented least-privilege default.

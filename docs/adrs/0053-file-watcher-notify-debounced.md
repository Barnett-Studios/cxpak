---
id: '0053'
title: File watcher via notify with channel-based debounced drain
status: ACCEPTED
date: 2026-03-13
triggered_by: Daemon mode needs cross-platform file-change detection feeding incremental updates
loop: implementation
---

# ADR-0053: File watcher via notify with channel-based debounced drain

## Context
Implemented in v0.8.0. Daemon mode must detect file creates/modifies/removes cross-platform and collapse rapid successive events so a burst of saves triggers one re-index, not many. Confirmed shipped in src/daemon/watcher.rs and consumed by both `watch` (src/commands/watch.rs) and `serve` (src/commands/serve.rs); `notify = { version = "7", optional = true }` in Cargo.toml.

## Options considered
- **Option A — notify RecommendedWatcher + mpsc channel + drain debounce (chosen):** wrap notify's `recommended_watcher`, map `EventKind::{Create,Modify,Remove}` to a `FileChange` enum (Created/Modified/Removed) over an `std::sync::mpsc` channel; the consumer blocks on `recv_timeout` for the first event, then sleeps ~50ms and `drain()`s all pending events, deduping modified/removed paths (case-insensitive HashSets in `classify_changes`). Pros: cross-platform via notify's recommended backend; simple debounce without a timer crate; clean enum surface. Cons: fixed ~50ms debounce window; manual dedup of paths in the consumer.
- **Option B — notify-debouncer-full crate:** A reasonable alternative would have been to use notify's dedicated debouncer crate for coalescing. Pros: battle-tested debounce semantics. Cons: extra dependency; more than the simple drain approach needs. Not discussed in the design docs; not pursued.
- **Option C — polling for mtime changes:** A reasonable alternative would have been to periodically stat files. Pros: no watcher backend quirks. Cons: latency bounded by poll interval; CPU cost on large trees. Not pursued.

## Decision
Implement `FileWatcher` around notify's `RecommendedWatcher`, sending a `FileChange` enum (Created/Modified/Removed) over an mpsc channel. The consumer blocks on `recv_timeout` for the first event, then sleeps ~50ms and `drain()`s all queued events, collapsing rapid successive events on a file into a single re-index. `watch` additionally caps the drain loop at `MAX_DEBOUNCE_ITERS = 40` (40 × 50ms = 2s). notify 7 is the cross-platform backend.

## Consequences
### Positive
- Cross-platform change detection without a custom backend.
- Rapid edits coalesce into one update.
- Simple enum/channel surface reused by both `watch` and `serve`.

### Negative
- The consumer must manually dedup modified vs removed paths.

### Neutral
- Debounce is a fixed ~50ms sleep-then-drain rather than an adaptive window.

## Revisit if
- The 50ms debounce proves too short/long under real editor save patterns.
- notify backend reliability issues surface on a target OS.

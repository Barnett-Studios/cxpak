---
id: '0132'
title: Relocate EdgeType/TypedEdge into index::graph and add CrossLanguage(BridgeType) variant
status: ACCEPTED
date: 2026-04-01
triggered_by: "v1.5.0 'Deep Flow': adding cross-language bridge edges to the dependency graph"
loop: implementation
---

# ADR-0132: Relocate EdgeType/TypedEdge into index::graph and add CrossLanguage(BridgeType) variant

## Context
Introduced in v1.5.0 ("Deep Flow"). Cross-language edges needed to be a variant of `EdgeType`, but `EdgeType` lived in `src/schema/mod.rs` while the new `BridgeType` and the consuming graph code lived in `src/index/graph.rs`. Adding `CrossLanguage(BridgeType)` in place would create a circular import between the two modules. The serialized cache format embeds `EdgeType`, so adding a variant invalidates old caches and must be guarded by a cache version bump.

## Options considered
- **Option A — Move `EdgeType`/`TypedEdge` to `index::graph`, re-export from schema, add `CrossLanguage(BridgeType)`, bump `CACHE_VERSION`:** One atomic move fixes all import paths via a `pub use` re-export; the new variant is added in `graph.rs`; the cache version bump invalidates stale serialized indices. Pros: breaks the circular dependency cleanly, the re-export preserves all existing callsites, and the cache bump prevents deserializing incompatible old indices. Cons: touches every `match` on `EdgeType` to add the new arm. Someone could prefer it because it resolves the dependency cycle in a single commit without breaking callers.
- **Option B — Keep `EdgeType` in schema and reference `BridgeType` across modules:** Leave the type where it is and import `BridgeType` into schema. Pros: no move. Cons: creates the circular import the plan explicitly set out to break. Someone could prefer it to minimize churn, if the cycle could somehow be tolerated.

## Decision
Move `EdgeType` and `TypedEdge` from `src/schema/mod.rs` into `src/index/graph.rs` (with a `pub use` re-export from schema to preserve all callsites), then add a `BridgeType` enum (`HttpCall`, `FfiBinding`, `GrpcCall`, `GraphqlCall`, `SharedSchema`, `CommandExec`) and a `CrossLanguage(BridgeType)` variant on `EdgeType`. Bump `CACHE_VERSION` in `src/cache/mod.rs` (the single authoritative location) so old serialized indices are invalidated. On version mismatch, `FileCache::load` (src/cache/mod.rs) discards the parsed cache and returns a fresh empty `FileCache` (via `Self::new()`), forcing a full re-parse; the stale file is then overwritten on the next `save()`. `load` returns `Self`, not `Option`, and never removes the file.

## Consequences
### Positive
- Breaks the circular import in one atomic, re-export-preserving commit.
- `CrossLanguage` edges live alongside the graph that consumes them.
- The cache version guard makes `load` fall back to a fresh cache on mismatch, preventing incompatible-deserialization crashes.
### Negative
- Every existing `match` on `EdgeType` had to gain a `CrossLanguage` arm.
- Bumping `CACHE_VERSION` forces a full re-index for all existing users.
### Neutral
- The six bridge types map directly to the six cross-language detectors in `cross_lang.rs`.

## Revisit if
- A seventh bridge type is needed (extend `BridgeType` + all matches).
- The schema/graph module split is reorganized again.

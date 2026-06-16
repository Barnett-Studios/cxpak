---
id: '0105'
title: Cross-language edges added as EdgeType::CrossLanguage(BridgeType); EdgeType+TypedEdge moved to index/graph.rs
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.5.0 cross-language symbol resolution adds new edge kinds to the dependency graph
loop: planning
---

# ADR-0105: Cross-language edges added as EdgeType::CrossLanguage(BridgeType); EdgeType+TypedEdge moved to index/graph.rs

## Context
The v1.5.0 cross-language work resolves symbols across language boundaries: HTTP calls, FFI bindings, gRPC, GraphQL, shared schema, and command execution. These bridges need to be first-class edges in the `DependencyGraph` so graph algorithms and the architecture map treat them uniformly.

The obstacle is module layout. `EdgeType` and `TypedEdge` currently live in `src/schema/mod.rs`, but `DependencyGraph` lives in `src/index/graph.rs`. Moving only `EdgeType` to sit next to the graph would create a circular import: `src/index/mod.rs` imports `crate::schema::SchemaIndex`, which references `TypedEdge`, which contains `EdgeType`.

## Options considered
- **Option A — Move both `EdgeType` and `TypedEdge` to `index/graph.rs`, re-export from schema, add `CrossLanguage(BridgeType)`, bump cache:** relocate both types to where `DependencyGraph` lives, re-export them from `schema/mod.rs` for backward compatibility, add the `CrossLanguage(BridgeType)` variant, make `BridgeType` derive the same traits `EdgeType` requires (`PartialEq`/`Eq`/`Hash`/`Serialize`/`Deserialize`), and bump the cache version to force a full re-index. Pros: cross-language edges become first-class graph edges; moving both types together avoids the circular import; the re-export preserves existing import paths. Cons: touches serialization/hash behavior and forces a full re-index on first v1.5.0 run. (Grounded — this is the shipped design.)
- **Option B — Move only `EdgeType`, leave `TypedEdge` in schema:** relocate just the enum. Pros: a smaller move. Cons: creates the circular import described above (`index/mod.rs` → `schema::SchemaIndex` → `TypedEdge` → `EdgeType`), which the design doc explicitly identifies. Rejected for exactly that reason. (Grounded — the circular-import failure mode is called out in the source.)
- **Option C — Keep cross-language edges in a separate structure outside `DependencyGraph`:** store bridges in a side list rather than as graph edges. A reasonable alternative would have been to avoid touching `EdgeType` and the cache entirely. Someone could prefer this to skip the migration and re-index. Rejected because side-list relationships are not traversable by the graph algorithms, and the architecture map could not flag `cross_language` uniformly with other edges. (Reconstructed — not formally evaluated in the source.)

## Decision
Add cross-language bridges to `DependencyGraph` as `EdgeType::CrossLanguage(BridgeType)`, where `BridgeType` has variants `HttpCall`, `FfiBinding`, `GrpcCall`, `GraphqlCall`, `SharedSchema`, `CommandExec`.

Migration: move **both** `EdgeType` and `TypedEdge` from `src/schema/mod.rs` to `src/index/graph.rs` (moving only `EdgeType` creates a circular import); re-export both from `schema/mod.rs` for backward compatibility; make `BridgeType` derive `PartialEq`/`Eq`/`Hash`/`Serialize`/`Deserialize` as `EdgeType` requires; and bump the cache version to force a full re-index on the first v1.5.0 run. Shipped: `EdgeType` and `EdgeType::CrossLanguage(BridgeType)` now live in `src/index/graph.rs`, with `pub use crate::index::graph::{EdgeType, TypedEdge};` in `src/schema/mod.rs`.

## Consequences
### Positive
- Cross-language relationships are first-class, graph-traversable edges, so existing algorithms and the architecture map handle them uniformly.
- Moving both types together avoids the circular-import trap.
- The backward-compatible re-export keeps existing call sites compiling unchanged.

### Negative
- Serialization and hash behavior change with the new variant.
- The cache-version bump forces a full re-index on the first v1.5.0 run.

### Neutral
- The `schema/mod.rs` re-export is a shim that can be removed once all call sites import directly from `index/graph.rs`.

## Revisit if
- All call sites migrate off the re-export shim (then the shim can be deleted).
- New bridge types are needed that require further `EdgeType` changes.

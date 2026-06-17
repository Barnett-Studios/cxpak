---
id: '0048'
title: cxpak serve HTTP API over a shared Arc<RwLock> hot index
status: ACCEPTED
date: 2026-03-12
triggered_by: IDE/MCP queries need sub-50ms responses, impossible with cold starts
loop: planning
---

# ADR-0048: cxpak serve HTTP API over a shared Arc<RwLock> hot index

## Context

Released in v0.8.0. Interactive clients (IDEs, MCP) cannot tolerate 2-5s cold starts per query. `cxpak serve` runs a long-running daemon that keeps the index in memory and answers queries over HTTP, with a background watcher keeping it fresh.

The v0.8.0 plan proposed two separate locks, `Arc<RwLock<CodebaseIndex>>` and `Arc<RwLock<DependencyGraph>>`. Implementation consolidated to a single shared, atomically-swappable handle: `pub type SharedIndex = Arc<RwLock<Arc<CodebaseIndex>>>` (`src/commands/serve.rs`), with `DependencyGraph` held as a field on `CodebaseIndex`. The serve module's own doc comment explains this snapshot pattern was adopted to stop long-running read handlers from starving the watcher writer.

## Options considered

- **Option A — axum HTTP server over a shared hot index, watcher thread:** Run an axum `Router` exposing GET `/health`, `/stats`, `/overview`, `/trace`, `/diff`; shared state behind a single `SharedIndex` (`Arc<RwLock<Arc<CodebaseIndex>>>`, atomic-Arc-swap snapshot); a background watcher loop applies incremental updates so reads see the hot index. (The plan proposed two separate `Arc<RwLock<_>>` locks and a tokio-spawned watcher; the shipped code uses one lock plus atomic Arc swap and a `std::thread::spawn` watcher.) Pros: <50ms in-memory queries, standard async HTTP via axum/tokio, concurrent reads via `RwLock`, same query code as the CLI. Cons: a write/swap during updates can briefly contend with readers; HTTP surface to secure. Someone could prefer it because it reuses the CLI query engine and keeps the index hot.
- **Option B — stateless re-index per request:** A reasonable alternative would have been to rebuild the index on each HTTP request. Pros: no shared-state concurrency concerns. Cons: 2-5s per request defeats the entire purpose. Someone could prefer it for simplicity in a non-interactive setting, but it fails the sub-50ms requirement. Not formally evaluated.

## Decision

Implement `cxpak serve` as an axum HTTP server holding a single shared, atomically-swappable index handle (`SharedIndex = Arc<RwLock<Arc<CodebaseIndex>>>`, with `DependencyGraph` as a field on `CodebaseIndex`), updated by a background watcher thread (`std::thread::spawn`), exposing GET routes `/health`, `/stats`, `/overview`, `/trace`, `/diff` that query the locked in-memory index for <50ms responses. The plan originally specified two separate locks and a tokio-spawned watcher; implementation simplified to the single-lock snapshot pattern to avoid long read handlers starving the watcher writer.

## Consequences

### Positive
- In-memory queries answer in <50ms.
- Reuses CLI query logic; concurrent reads via `RwLock`.
- Background watcher keeps state fresh.

### Negative
- The HTTP server is an exposure surface; bearer-token auth was added in later versions.

### Neutral
- GET query interface mirrors CLI flags as query params.
- Updates take a write lock briefly to swap in a new index snapshot.

## Revisit if
- `RwLock` contention under heavy concurrent load.
- Need for streaming or push (websocket) responses.

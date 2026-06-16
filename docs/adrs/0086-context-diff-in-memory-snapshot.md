---
id: '0086'
title: Implement cxpak_context_diff via an in-memory snapshot in a separate Arc<RwLock>, with git-ref fallback
status: ACCEPTED
date: 2026-03-22
triggered_by: Long MCP sessions need a cheap delta instead of re-reading full context after each auto_context
loop: planning
---

# ADR-0086: Implement cxpak_context_diff via an in-memory snapshot in a separate Arc<RwLock>

## Context

In a long MCP session, an agent calls `auto_context` repeatedly. Re-reading the full context after each call is wasteful when only a few files changed. v1.0.0 adds `cxpak_context_diff` (tool #11): `auto_context` stores a `ContextSnapshot` (file hashes, symbol sets, edge set) and `context_diff` compares the current index state against it to produce a cheap delta plus a recommendation.

The snapshot lives in a separate `Arc<RwLock<Option<ContextSnapshot>>>` alongside `SharedIndex` in the MCP server state — not inside `CodebaseIndex` or `SharedIndex` — so snapshot management does not require write-locking the main index. The snapshot is memory-only and is not persisted across restarts.

## Options considered

- **Option A — In-memory snapshot in a separate `Arc<RwLock>`, hash-based diff:** `auto_context` write-locks to store the snapshot; `context_diff` read-locks to compare file hashes, symbol sets, and edge sets. Pros: fast hash comparison, catches non-git/uncommitted changes, avoids write-locking the main index. Cons: lost on server restart; requires coordinating a separate lock through the call stack. This was the chosen option.
- **Option B — Snapshot inside `SharedIndex`/`CodebaseIndex`:** Store the snapshot on the main index structure. Pros: fewer moving parts, one lock. Cons: snapshot management would require write-locking the main index, contending with read traffic. Someone could prefer this for simplicity, but the write-lock contention was judged not worth it.
- **Option C — Git-based diff by default:** Always diff against a git ref. Pros: survives restarts since it reads committed state. Cons: slower, and does not capture uncommitted/non-git changes that an in-session agent cares about.
- **Option D — Persist snapshot to disk:** Write the snapshot to `.cxpak/` so it survives restarts. Pros: survives restart. Cons: a restart also rebuilds the index, so any prior session context is stale anyway; the ~1MB in-memory footprint is trivial, so disk persistence buys little.

## Decision

Add `cxpak_context_diff` (tool #11) backed by an in-memory `ContextSnapshot` (`file_hashes`, `symbol_set`, `edge_set`) stored in a separate `Arc<RwLock<Option<ContextSnapshot>>>` threaded through the MCP call stack (and as an `AppState` field for HTTP). `auto_context` writes the snapshot; `context_diff` reads it and computes modified/new/deleted files, symbol deltas, and graph-edge deltas. The snapshot is in-memory only with no persistence across restarts; when no snapshot exists, the tool returns a recommendation to call `auto_context` first.

The design intended a git-ref fallback (a `since` git ref such as `HEAD~1` routing through the existing `cxpak diff` infrastructure), but this did not ship. The MCP handler ignores `since` for git-ref purposes and always performs the in-memory snapshot diff; in the HTTP path, `since` is used solely as an ISO-8601 staleness threshold to reject a stale snapshot. Git-ref diffing remains available only via the separate `cxpak_diff` tool.

## Consequences

### Positive
- Fast hash-based delta that also catches non-git/uncommitted changes.
- The separate lock avoids write-locking the main index for snapshot management.

### Negative
- The snapshot is lost on server restart (acceptable, since the index rebuilds on restart anyway and prior context is already stale).
- Threads a new snapshot parameter through the full MCP stdio call stack.

### Neutral
- ~1MB memory for a 10k-file repo.
- No snapshot present → the tool recommends calling `auto_context` first.
- The MCP tool schema still advertises `since` as accepting a git ref, but the handler does not implement git-ref diffing — a schema/behavior mismatch versus the original design.

## Revisit if
- Sessions begin to span server restarts and the cost of losing context rises.
- The hash-based diff misses a class of change users care about.
- The unshipped git-ref fallback is needed and the schema/behavior mismatch must be resolved.

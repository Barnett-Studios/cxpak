---
id: '0185'
title: Non-blocking MCP startup (lazy index build off the handshake path)
status: ACCEPTED
date: 2026-07-03
triggered_by: cxpak 3.0.0 Phase R (Task R0) — dogfood fix for intermittent "MCP server not connected"
loop: implementation
---

# ADR-0185: Non-blocking MCP startup (lazy index build off the handshake path)

## Context

`cxpak serve --mcp` (`run_mcp`, `src/commands/serve.rs`) built the **full**
`CodebaseIndex` **synchronously** — `let index = build_index(path)?` — *before*
it answered the MCP `initialize` handshake. On a 427-file / 1.2M-token repo that
build is **13–34s**, which straddles Claude Code's ~30s MCP startup timeout: the
client gives up mid-build and reports **"MCP server not connected"**,
intermittently, depending on cold/warm cache and machine load.

The ADR-0167 persistent derived cache only speeds *warm* starts — it does not
move indexing off the handshake path, so a cold clone (or a cache miss) still
blocks the handshake for the full build time. The MCP transport is single
JSON-RPC-over-stdio; the handshake carries no index data (server capabilities
only), and post-C3 (ADR-0182) `tools/list` is a static catalog projection that
needs no index either. So nothing on the connect path *requires* a built index —
it was blocking purely as an artifact of build ordering.

This is a human decision because it trades a guarantee ("every tool call after
connect has a ready index") for availability ("the connection always succeeds,
but an early tool call may have to retry"), and it picks one of two legitimate
before-ready behaviors (graceful status vs. bounded block).

## Options considered

- **Option A — Synchronous build before handshake (status quo):** simplest;
  every tool call is guaranteed a ready index. But it structurally cannot meet a
  30s connect budget on a large cold repo — the very bug we are fixing. Rejected.
- **Option B — Background `std::thread` build + readiness-gated tool calls
  (chosen):** answer `initialize` immediately, build the index on a background
  thread, publish it into a shared readiness cell; tool calls before ready return
  a graceful status. No new deps (std threads only). The stdio loop stays
  synchronous and single-threaded, so framing/ordering is unchanged. Someone
  could dislike that an early tool call can transiently return "not ready", but
  a live connection that occasionally says "retry" is strictly better than an
  intermittently dead one.
- **Option C — Adopt `tokio` on the MCP stdio path for async build:** would make
  the build await-able, but pulls a runtime onto a path that is deliberately
  sync, is a far larger blast radius, and buys nothing over a single background
  thread for a one-shot build. Rejected (and the task forbids adding tokio here).
- **Before-ready sub-choice — graceful status vs. short bounded block:** a bounded
  block (sleep-poll up to N seconds inside the tool call) improves UX for warm
  builds but adds wall-clock coupling and a sleep loop to the single-threaded
  loop for no correctness gain. We chose the **immediate graceful status**: fully
  deterministic, no sleeps, and the client (or model) simply retries. Warm builds
  finish in well under the interval between `initialize` and the first real tool
  call anyway, so the retry window is small in practice.

## Decision

Implement **Option B** with an immediate graceful status. `run_mcp` publishes an
`Arc<RwLock<IndexReadiness>>` (`SharedReadiness`) initialized to `Building`,
prints a synchronous "accepting connections; indexing in background" line, and
spawns `spawn_mcp_index_build` (a `std::thread`) that runs `build_index`
(which loads the ADR-0167 derived cache on a fingerprint hit) and swaps the cell
to `Ready(Arc<CodebaseIndex>)` — or `Failed(String)` on error — under a **brief
write lock** (the long build happens in a thread-local; only the O(1) `Arc`
publish holds the lock). The stdio loop (`mcp_stdio_loop_readiness`) answers
`initialize`/`tools/list`/notifications instantly regardless of readiness; a
`tools/call` calls `snapshot_ready_index`, which clones the `Ready` `Arc` under a
brief read lock (dropped before the handler runs) or returns a graceful tool
`result` (`INDEXING_IN_PROGRESS_MESSAGE` while `Building`, a failure status while
`Failed`) — never a session-killing JSON-RPC `error`.

`IndexReadiness` is deliberately an **enum, not a bool**: **Phase R-E1** will
append a second background phase (embedding enrichment layered on top of the
ready base index) as an additional readiness state — e.g. a `ReadyEnriched`
variant — without reshaping the handshake path or the gating logic.

The pre-R0 `mcp_stdio_loop_with_io(index: &CodebaseIndex, …)` entry point is kept
as a thin compatibility wrapper that publishes the caller's already-built index
as `Ready` and delegates to `mcp_stdio_loop_readiness`, so its ~40 in-tree
callers and the two integration-test drivers are unchanged. The HTTP `serve`
path is untouched (it may block at startup; only MCP is timeout-sensitive).

## Consequences

### Positive
- The `initialize` handshake returns in milliseconds regardless of repo size —
  the "MCP server not connected" timeout is eliminated.
- No new dependencies (std `thread` only); the stdio loop stays synchronous and
  single-threaded, so framing/ordering and cross-channel parity are unchanged.
- Ranking/scoring/output are byte-identical once ready — this is a startup
  concurrency change only. `spa_output_matches_golden_fixture` is unaffected.
- Build failure is surfaced as a clear tool-call status instead of crashing or
  silently serving an empty index; poisoned-lock recovery keeps the server alive.
- A **panicking** `build_index` (e.g. deep unwrap, OOM, tree-sitter bug) is also
  caught via `std::panic::catch_unwind(AssertUnwindSafe(...))` inside the background
  thread and mapped to `Failed(String)` by `classify_build_outcome`, so the cell
  never stays `Building` forever in that case either.
- Clean extension seam for R-E1's background embedding phase.

### Negative
- A `tools/call` that arrives before the background build finishes returns a
  "retry" status rather than a result; the client/model must retry. The window is
  small (warm builds are fast) but non-zero on a cold large repo.
- On a hard `build_index` failure, tool calls now report a failure status rather
  than degrading to an empty-index best effort (a deliberate clarity trade-off).

### Neutral
- The compat wrapper clones the caller's index into an `Arc` (test/HTTP-adjacent
  paths only); the production `run_mcp` path never clones — it swaps the Arc the
  background thread built.
- Startup stderr wording changed ("accepting connections; indexing in
  background", then "MCP index ready" when the build completes); the
  `test_mcp_startup_message` subprocess test was updated to match.

## Revisit if

- Claude Code's MCP startup timeout is removed or made build-aware, making a
  synchronous build viable again (revert to Option A for simpler guarantees).
- Early-tool-call retries prove disruptive in practice — then switch the
  before-ready behavior to a short bounded block (the sub-choice above) without
  changing the readiness state machine.
- Long-lived MCP sessions become common and the "build once, never refresh"
  assumption starts serving stale context (add periodic/background rebuild on the
  same readiness cell).
- R-E1 or later phases need more than a linear Building→Ready→enriched
  progression (e.g. concurrent enrichment phases), warranting a richer state type.

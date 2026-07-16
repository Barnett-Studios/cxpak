---
id: '0200'
title: MCP index freshness — wire the file watcher into serve --mcp
status: ACCEPTED
date: 2026-07-16
triggered_by: issue #18 (serve --mcp serves a stale index after file edits)
loop: planning
---

# ADR-0200: MCP index freshness — wire the file watcher into `serve --mcp`

## Context

`cxpak serve --mcp` builds its index **once** at startup (`run_mcp` →
`spawn_mcp_index_build`, `src/commands/serve.rs:2238/2318`, ADR-0185) and never
refreshes it. Its own doc comment admits it (serve.rs:2448-2449: "a long-lived
session would keep the first ready index (no periodic rebuild)"). Reproduced
(REPRO.md): a warm MCP session returns `out_degree 0` for a symbol after an edit
that a fresh index shows as `out_degree 1` — silently, no staleness signal.

The HTTP path (`run`, serve.rs:1351) does **not** have this bug: it spawns a
`daemon::watcher::FileWatcher` and drives `process_watcher_changes`
(serve.rs:4231, takes `&SharedIndex`) — snapshot-then-swap, edge-delta graph
rebuild + warm PageRank (ADR-0165/0166), fresh derived caches — on every
debounced change batch, atomically swapping a `SharedIndex`. The delta==full
parity is already tested (serve.rs:6169). So the freshness *primitives* exist and
are proven; they are simply not wired into `run_mcp`.

The reuse is **not** verbatim, though: the HTTP watcher loop is an infinite
`loop {}` (serve.rs:1392-1399) that is detached and only dies on process exit.
That is fine for a server whose lifetime *is* the process, but the MCP stdio loop
returns on stdin EOF and is driven in-process by tests — a detached infinite
thread would leak into every test and cannot be joined on shutdown
(resource-cleanup). So this decision must also own the watcher's lifecycle.

There is also a **pre-existing second writer** of the readiness cell on the MCP
path: the opt-in embedding-enrichment phase (`enrich_ready_with_embeddings` →
`publish_ready_enriched`, serve.rs:2405-2417, ADR-0186) runs on the *same*
background build thread and, seconds after `Ready`, blind-swaps `ReadyEnriched`
built from the **startup** index. A watcher that republishes `Ready(edited)` in
that window would be silently clobbered by the late enrichment (stale, but
embedding-adorned) — reintroducing exactly the staleness #18 is about. So this
decision must *coordinate* the watcher with that existing writer, not merely add
one. (The default no-embeddings path returns early at serve.rs:2375 — the race is
config-gated and window-bounded, but a correctness defect nonetheless.)

Human decision: it re-opens the ADR-0185 trade-off (that ADR accepted "no
periodic rebuild" for a short-lived one-task-one-connection model) now that a
long-lived warm MCP session is a real usage pattern (a persistent commit-gate
reusing one session across edits), and it decides the thread-lifecycle and
embedding-staleness contracts.

## Options considered

- **Option A — Document "restart to refresh", change nothing:** honest, zero
  code. But it forfeits the warm-session value (cold-spawn-per-query pays the full
  10-14s index every call) and leaves MCP and HTTP with asymmetric freshness for
  the same command — a latent trap for every machine consumer.
- **Option B — Mirrored `SharedIndex` + republish into the readiness cell, with an
  owned, terminable watcher thread (chosen):** after the background build
  publishes `Ready`, a watcher thread seeds a `SharedIndex` from it, runs the
  existing `process_watcher_changes` on each debounced change batch, then
  republishes `Ready(new_index)` into the `SharedReadiness` cell.
  `mcp_stdio_loop` already snapshots readiness per `tools/call`, so it picks up
  the swap with no protocol change. The thread waits for `Ready` **or**
  `ReadyEnriched` (both mean ready-to-mirror; seed from whichever), and that
  pre-Ready wait itself checks the shutdown signal (`Arc<AtomicBool>`, polling
  with a sleep — not a busy-spin) so an EOF during the 13-34s initial build joins
  promptly; it aborts if the build resolves `Failed` (nothing to watch over).
  Once looping, it checks the same shutdown signal at each `recv_timeout(1s)` poll.
  The one-shot enrichment publish is made a **compare-and-swap** so it can never
  clobber a watcher republish (see Decision).
- **Option C — Extract a transport-agnostic `recompute_index(snapshot,changes)`
  core shared by HTTP and MCP:** cleaner single-source, but a larger refactor of a
  load-bearing, determinism-sensitive function for no behavioral gain over B;
  higher risk for a patch release. Deferred.
- **Explicit `refresh`/`reindex` op (DEFERRED):** a `tools/call` to force a
  synchronous rebuild deterministically (the issue's stated *nice-to-have*).
  Rejected for 3.1.1 because it introduces a **second writer** of the readiness
  cell racing the watcher (last-write-wins can republish an older index) and
  leaves the watcher's mirror stale (the next batch rebuilds from the stale mirror
  and clobbers the refresh). Making it coherent means routing refresh *through*
  the watcher (signal-to-drain-and-rebuild) — a real design, but scope beyond the
  core bug, which the debounced watcher already fixes. Deferred to a follow-up.

## Decision

Option B: reuse `FileWatcher` + `process_watcher_changes`, but own the watcher
thread's lifecycle (shutdown `AtomicBool` checked in both the pre-Ready wait and
the poll loop; wait predicate accepts `Ready | ReadyEnriched`; abort on a `Failed`
build) and republish base `Ready`. A watcher swap **actively clears**
`embedding_index` on the republished index (a delta-rebuilt clone would otherwise
carry the *stale* enrichment and score against an out-of-date 7th signal — see
Negative), dropping to the documented 6-signal fallback with a one-line log.

To coordinate with the pre-existing enrichment second writer (Context), make
`publish_ready_enriched` a **compare-and-swap**: it swaps in `ReadyEnriched` only
if the cell still holds the exact base `Arc` it enriched (`Arc::ptr_eq` on the
inner `Arc<CodebaseIndex>`); otherwise it skips and logs "index changed during
enrichment — serving fresh 6-signal". So a watcher republish is never overwritten
by a late enrichment (the enriched base and the current cell differ by identity).

The explicit `refresh` op is **deferred** — it would add a *third*, uncoordinated
writer. Watcher-start failure logs and leaves the one-shot index serving
(fail-open — never wedges the session).

## Consequences

### Positive
- MCP and HTTP have symmetric freshness; warm sessions are correct.
- Reuses proven, determinism-safe primitives; delta==full parity already tested.
- The watcher thread is owned and terminable — no leak into the stdio loop or
  in-process tests.

### Negative
- A watcher swap drops embedding enrichment (7→6 signal) for the rest of the
  session; the republished index must *actively clear* `embedding_index` (not just
  rely on the variant being discarded) so no *stale* embedding is ever served.
  Logged; a regression test asserts the 6-signal fallback and no stale embedding.
  Re-enrichment after a swap is a follow-up.
- A mirrored `SharedIndex` holds one extra `Arc` alongside the readiness cell
  (cheap; shared backing). The readiness cell has **two coordinated writers** — the
  watcher and the one-shot embedding enrichment — made coherent by the enrichment
  publish being a compare-and-swap (`Arc::ptr_eq` against the base it enriched), so
  a watcher republish is never overwritten by a late enrichment. `refresh` stays
  deferred because it would add a *third*, uncoordinated writer. (An earlier draft
  wrongly claimed the watcher was the sole writer — the enrichment phase, ADR-0186,
  is a pre-existing second writer; the CAS is what makes the pair safe.)

### Neutral
- No new dependency; `std::thread` + notify, same as HTTP serve. No golden-fixture
  impact (visual path untouched).

## Revisit if
- A transport-agnostic recompute core is extracted for other reasons — fold B into
  it (Option C) to drop the mirrored cell.
- A coherence-safe `refresh` design lands (routed through the watcher) — ship it
  then; the commit-gate use case still wants it.
- Embedding enrichment (ADR-0186) becomes common enough that dropping it on every
  edit is a measured regression — then re-enrich after each watcher swap.

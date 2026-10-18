---
title: A shared per-repo index server behind the stdio MCP shim
status: READY
created: 2026-10-18
updated: 2026-10-18
target: cxpak 3.6.0 (MINOR — no public API change; MCP wire contract unchanged)
adrs: 0211 (direction), 0214 (this spec's protocol/lifecycle decisions)
spec: specs/2026-10-15-warm-start-SPEC.md, specs/2026-10-15-warm-start-phase2-SPEC.md
---

# A shared per-repo index server — Spec

## 1. Problem (measured, ADR-0211)

Every `cxpak serve --mcp` process builds and holds its own `CodebaseIndex` and runs its own watcher
(ADR-0200). On 2026-10-15 a developer machine running the four factory roles plus ad-hoc sessions
had **7 concurrent `cxpak serve --mcp` processes on the same repos, ~2.4 GB combined `phys_footprint`**
(174–407 MB each). Warm start (ADR-0210) fixes *rebuild* cost on each new session; it does not fix
*duplication* across concurrently live sessions on the same repo.

## 2. Goal

N concurrent `cxpak serve --mcp <repo>` invocations on the same repo converge on **one** resident
index, with:
- Every client still speaking MCP over stdio — no harness-specific wiring (code-review-graph's
  editor-hook approach was rejected in ADR-0211 for exactly this reason).
- A client that cannot reach the server falls back to today's in-process index — the server is an
  optimization, never a hard dependency.
- `cxpak serve` (HTTP) can host the same shared index without a second implementation (ADR-0211
  "Neutral": HTTP already is a long-lived single-index server).

## 3. Grounding in the current code

- `src/commands/serve.rs:2658 run_mcp` spawns the stdio loop; `IndexReadiness`
  (`Building`/`Ready(Arc<CodebaseIndex>)`/`ReadyEnriched`/`Failed`, lines 61-83) is already the
  in-process publish point — the shim's job is to get a *remote* `Arc<CodebaseIndex>`-equivalent
  into the same seam instead of building one.
- `src/daemon/watcher.rs` (489 lines) is today's per-process file watcher, keeping the in-process
  index fresh (ADR-0200). A shared server needs exactly one of these per repo, not one per client.
- `serve` (HTTP) already runs a single long-lived index (ADR-0211 Neutral) — it is the closest
  existing analog to "the server," not a from-scratch design.

## 4. Decisions (resolved in ADR-0214 "Resolved design"; summarized here)

1. **Transport.** Unix domain socket on macOS/Linux (matches the existing non-Windows allocator
   split in `Cargo.toml`); a named pipe on Windows (roadmap platform per CLAUDE.md — the protocol
   must not assume Unix-only, even though Windows ships later).
2. **Spawn-on-demand + idle exit.** The first `serve --mcp` (or `serve` HTTP) for a repo that finds
   no live server at the expected socket path starts one as a DETACHED child of the same binary in
   a hidden server mode (it must outlive the spawning session) and then connects as a client; the server exits after an idle timeout with zero
   attached clients.
3. **Crash recovery.** A client that loses its connection (server crash, exit) falls back to its own
   in-process index immediately (never blocks on reconnection) and attempts one re-spawn; this must
   reuse the warm-start artifact store (ADR-0210) so the fallback build is the fast path, not the
   14 s cold path.
4. **Binary upgrade while clients are attached.** A client built from a different binary than the
   running server detects version skew at handshake (see ADR-0214) and falls back to in-process
   rather than trusting a version-mismatched server's index shape.
5. **Auth on a local socket.** Socket file permissions (0600, owner-only) are the baseline; no
   token is strictly needed for a same-user local socket, but the design must say why (or add one)
   given HTTP `serve` already requires bearer auth off loopback (ADR-0189) — a shared local socket
   must not become the unauthenticated side door that HTTP's bearer-auth hardening closed.
6. **Windows story.** Named pipes, not sockets; explicitly out of the initial build target (macOS
   today per CLAUDE.md) but the IPC abstraction must not be Unix-socket-typed at the API boundary.
7. **Fallback semantics.** Every behavior above must degrade to *today's* per-process in-process
   index on any failure — this spec adds no new required dependency for `serve --mcp` to keep
   working.

## 5. Acceptance (measured)

- Measurement protocol: N=4 concurrent `serve --mcp` sessions on a frozen clone
  (`/tmp/frz-srv/scriptorium`), total phys footprint (sum of `footprint -p`/peak footprint from
  `/usr/bin/time -l` per process, server included) vs 3.5.1 behavior via `CXPAK_INDEX_SERVER=off`.
  Target: total <= ~1 resident index + N thin clients.
- N (≥ 3) concurrent `cxpak serve --mcp` sessions on one repo hold **one** resident index: combined
  `phys_footprint` across N sessions ≤ 1 server's single-session footprint × 1.3 (vs today's
  measured N× — 7 servers ≈ 2.4 GB, i.e. roughly N× one server's 174–407 MB).
  A fallback-triggering failure (server killed mid-session) does not hang a client: time to
  fallback-ready ≤ the client's own warm-start `index_ready_s` (ADR-0210/0212 baselines).
- Byte-identical MCP tool responses whether a client is talking to the shared server or its own
  in-process index (determinism fixture, same as every prior ADR in this chain).
- No change to the MCP wire contract any client-side harness observes.

## 6. Non-goals

- Replacing the in-memory index with an on-disk queryable store (ADR-0211 Option C, rejected).
- A machine-wide (cross-repo) server registry — this spec is per-repo.
- Windows named-pipe implementation (tracked, not built, per CLAUDE.md's platform roadmap).

## 7. Risks

- Spawn-on-demand races: two clients starting within the same window both spawning. Closed by an
  flock'd lock file plus atomic `bind` (ADR-0214 item 3); proven by the N-process race test.
- Idle-exit default (600 s, `CXPAK_INDEX_SERVER_IDLE_SECS`) is reasoned, not yet measured; record
  the factory-workload observation in BASELINES.md and revisit.
- A hung (not crashed) server is harder to detect than a closed socket — the client needs a
  request-level timeout, not just a connection-level one, before falling back.

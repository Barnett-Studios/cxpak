---
title: Shared per-repo index server — implementation phasing
status: DRAFT
created: 2026-10-18
updated: 2026-10-18
spec: specs/2026-10-18-shared-index-server-SPEC.md
adrs: 0211, 0214
---

# Plan — shared index server

This triad is design-complete but implementation-deferred (ADR-0211: specified separately, after
warm start, which is still landing as 3.5.1). No code lands from this plan yet; it records the
phasing so the next planning pass can decompose into an execution manifest without re-deriving the
protocol decisions. Every phase below is cross-cutting (new IPC module, lifecycle state machine,
touches `serve.rs`'s existing readiness seam) — marked `local: false` in spirit; this plan does not
emit a `local: true` execution manifest because there is no RED-testable single-function node yet
until Phase 1's trait shape is chosen in code, not just in ADR-0214's prose.

## Phase 1 — IPC trait + Unix socket arm

- Define a `SharedIndexTransport` trait (connect, send request, recv response, close) in a new
  `src/daemon/ipc.rs`.
- Implement the Unix-domain-socket arm only (macOS/Linux — today's supported platform).
- Wire protocol: length-prefixed JSON frames carrying the same request/response shapes `run_mcp`'s
  stdio loop already uses internally, so the shim is a thin forward, not a re-serialization.
- Gate: a round-trip integration test, client process ↔ server process over the socket, byte-equal
  to the in-process tool-call result for the same query (determinism fixture reused).

## Phase 2 — spawn-on-demand + idle-exit lifecycle

- Exclusive-bind race protocol (ADR-0214 Option C): bind-or-connect at a deterministic per-repo
  socket path (derived from the canonicalized repo root, matching ADR-0209's absolute-root
  anchoring).
- Idle-exit timer, default TBD by measurement (candidate starting point: 10 minutes idle, no
  attached clients) — tune against a realistic multi-session factory workload before fixing it.
- Gate: a race test — N processes started concurrently against a fresh repo, exactly one becomes
  the server, the rest connect as clients; kill the server mid-session, confirm every attached
  client falls back to in-process without hanging.

## Phase 3 — version-skew handshake

- Connect-time exchange of cxpak version + wire-schema content hash; mismatch → client falls back,
  never attaches.
- Gate: a test harness running two different binaries (build-script-produced fixture binaries, or a
  stubbed version string) confirms fallback on mismatch and normal operation on match.

## Phase 4 — fold `run_mcp` onto the shim

- `run_mcp` tries the transport first (Phase 1-3); on any failure (no server, connect refused,
  handshake mismatch, request timeout) falls through to today's in-process `build_index` path
  unchanged. This is the change with the largest blast radius — every `serve --mcp` invocation goes
  through it — so it lands last, behind the fully-tested transport/lifecycle/handshake layers.
- Gate: the full existing `warm_start_tests.rs` suite plus the new transport/lifecycle/handshake
  tests, all green; the acceptance measurement from spec §5 (N concurrent sessions, one resident
  index, measured `phys_footprint`) run and recorded in `benches/BASELINES.md`.

## Phase 5 — `serve` (HTTP) convergence (optional, separate decision)

- Evaluate whether HTTP `serve` becomes a client of the same shared server or keeps its own
  long-lived index (ADR-0211 Neutral already notes it is the closest existing analog). Not blocking
  Phase 1-4; write as its own ADR if pursued.

## Open items for the next planning pass

- Fix the idle-exit default and the socket-path derivation exactly (this plan leaves both as
  measured-TBD, not guessed).
- Decide whether the lock/bind-race protocol needs a stale-socket cleanup path (a server that
  crashed without closing its socket file leaves a dead path other processes could wrongly treat as
  "someone else is the server").

---
id: '0214'
title: Shared index server — local-socket protocol, spawn-on-demand lifecycle, version-skew fallback
status: PROPOSED
date: 2026-10-18
triggered_by: specs/2026-10-18-shared-index-server-SPEC.md; ADR-0211 (direction already chosen)
loop: planning
---

# ADR-0214: Shared index server — local-socket protocol, spawn-on-demand lifecycle, version-skew fallback

## Context

ADR-0211 already chose "a shared per-repo index server" as the direction, specified separately,
after warm start (ADR-0210/0212, now shipped as 3.5.0/pending 3.5.1). This ADR makes the decisions
ADR-0211 deferred: the transport, who spawns the server and when it exits, what happens on a
server crash or a binary upgrade while clients are attached, and whether a local socket needs its
own auth given `serve` (HTTP) already requires bearer auth off loopback (ADR-0189). Every decision
must preserve ADR-0211's fallback promise: a client with no reachable server still works exactly as
today, unaided.

## Options considered

### Transport
- **Option A — OS-native local IPC** (Unix domain socket on macOS/Linux, named pipe on Windows,
  behind one trait). *Pros:* lowest latency, no network stack, file-permission-based access control
  for free on Unix. *Cons:* two implementations behind a trait; Windows ships later anyway
  (CLAUDE.md roadmap) so the trait's second arm is unbuilt until then — risk of an API shaped wrong
  for named pipes if designed Unix-first without the trait.
- **Option B — loopback TCP.** *Pros:* one implementation for every platform, reuses `serve`'s
  existing HTTP/axum stack almost as-is. *Cons:** a loopback port is visible to every process on the
  machine (not just the same user), reopening exactly the exposure ADR-0189 closed for HTTP `serve`
  by requiring bearer auth off loopback — a shared index server would need the same hardening for a
  weaker reason (no cross-host access needed at all).

### Spawn / lifecycle
- **Option C — spawn-on-demand with a lock-file-gated exclusive bind.** The first client to bind
  the socket path becomes the server; a client that gets a connection-refused/no-such-socket tries
  to become the server itself (bind succeeds → it is the server now; bind fails on
  AddrInUse → someone else won the race → connect as a client). Idle-exit timer resets on every
  request; zero clients for the timeout → exit, freeing the path for the next spawn. *Pros:* no
  separate supervisor process, no registry file to go stale. *Cons:* the bind-race window needs the
  OS's atomic bind-or-fail semantics to be trustworthy (it is, for Unix sockets — binding an
  existing path fails outright, no silent takeover).
- **Option D — a user-level launchd/systemd-managed daemon**, started explicitly (`cxpak daemon
  start`), not spawned implicitly. *Pros:* simpler state machine (always-expected-running, not
  spawn-on-demand). *Cons:* a manual step contradicts ADR-0211's "no editor hook, no daemon needed"
  framing for warm start — users would need to remember to start it per repo; platform-specific
  service managers (launchd vs systemd vs nothing on Windows) multiply the lifecycle code instead of
  the transport code.

### Version skew / binary upgrade
- **Option E — a version handshake on connect**, server and client exchange their binary's cxpak
  version + a content hash of the wire schema; any mismatch → client falls back to in-process,
  never trusts a differently-versioned server's index shape. The *old* server keeps running for
  *its* already-attached old clients; a newly-upgraded client simply never attaches to it. *Pros:*
  no shared-memory layout risk, no "drain and restart" choreography — mismatch is just a fallback,
  the same code path as "server unreachable." *Cons:* an upgraded machine runs the old server
  resident (wasting memory) until its idle-exit timer fires with zero attached old clients.
- **Option F — a supervisor that drains and restarts the server on upgrade.** *Pros:* no stale old
  server lingering. *Cons:* needs to track every attached client and coordinate a handoff
  mid-session — exactly the complexity ADR-0211 already flagged as this option's cost, now
  concentrated into one more subsystem.

### Local-socket auth
- **Option G — socket file permissions only (0600, owner-created dir).** *Pros:* matches Unix socket
  defaults, zero extra code, no token to manage or leak into logs. *Cons:* any process running as
  the same user can attach — acceptable because that process could already read the repo's files
  and `.cxpak/cache` directly; a local socket adds no privilege an attacker with that access lacked.
- **Option H — a token, like HTTP `serve`'s bearer auth** (ADR-0189). *Pros:* uniform story across
  both servers. *Cons:* HTTP's bearer auth defends a *network-reachable* (even if loopback) listener
  against *other users/hosts*; a same-user Unix socket is already scoped to the user by the
  filesystem, so a token adds a secret to manage for no additional boundary it is crossing.

## Decision

**A + C + E + G.** OS-native local IPC behind one trait (Windows arm implemented when Windows
support ships, not now); spawn-on-demand via exclusive bind with an idle-exit timer, no separate
daemon command; version-skew detected at a connect-time handshake, with mismatch always falling
back to in-process (never a drain/restart choreography); socket-file-permission auth, no token.
B, D, F, H are rejected for the reasons above — each either reopens a closed security boundary (B),
adds a manual step ADR-0211 explicitly avoided (D), adds supervisor complexity disproportionate to
the problem (F), or adds a secret with no boundary it defends (H).

## Consequences

### Positive
- Every decision degrades to today's working behavior on failure — the server is additive, never a
  new hard dependency, matching ADR-0211's framing.
- No new daemon-management UX; "it just works" the same way warm start did.
- Binary upgrades are safe by construction (old and new processes never share index state).

### Negative
- A stale old server can sit resident after an upgrade until its idle timer fires — a measurable,
  bounded cost (one process, one repo's index), not unbounded.
- The bind-race protocol needs careful testing (concurrent spawn attempts) — a real distributed-ish
  correctness surface in an otherwise single-machine feature.
- The IPC trait's Windows arm is unbuilt until Windows support ships — a design-now, build-later
  risk that the trait boundary turns out wrong for named pipes.

### Neutral
- `cxpak serve` (HTTP) can become a thin client of the same shared server internals later, or stay
  its own long-lived index — this ADR does not force HTTP `serve` to change.

## Revisit if

- Windows support ships and the IPC trait needs a shape change to fit named pipes cleanly.
- The idle-exit default (not fixed by this ADR — the plan measures it) thrashes or wastes memory in
  practice; needs a different default or an adaptive timeout.
- A harness gains a first-class shared-MCP-server mode (ADR-0211's own revisit condition), making
  this shim unnecessary.

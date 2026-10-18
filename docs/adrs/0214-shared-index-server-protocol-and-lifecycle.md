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

## Resolved design (implementation-ready)

Decisions that turn A+C+E+G into buildable code. They refine Option C: the pure "first client binds
and becomes the server" form is replaced by a detached server process, because a server hosted
inside the first MCP session dies with that session (stdio EOF) and every other attached client then
falls back at once. The server must outlive any one session.

1. **Process model.** A `serve --mcp` client that finds no live server spawns a DETACHED child of
   its own binary (`std::env::current_exe()`) running a hidden subcommand `cxpak index-server
   <repo>` (`#[command(hide = true)]`, not in `--help`). The child gets its own process group
   (`CommandExt::process_group(0)`, no `libc` needed), stdin `/dev/null`, stdout/stderr to
   `<repo>/.cxpak/index-server.log` (truncated per start; `/dev/null` if unwritable). It binds the
   socket FIRST, then builds/warm-starts the index on a background thread (ADR-0210/0212 path via
   `spawn_mcp_index_build`) and runs `spawn_mcp_watcher`, so a client attaches in milliseconds and a
   tool call during `Building` returns today's `Indexing` status. The spawning client then connects
   like any other. "First session hosts in-process" is rejected: no strong reason found, and it
   breaks the outlives-the-session requirement.
2. **Socket path.** Private run dir: macOS `std::env::temp_dir()` (per-user 0700 `/var/folders/.../T/`);
   Linux `$XDG_RUNTIME_DIR`, else `<user cache dir>/cxpak/run` created 0700. File name
   `cxpak-<h>.sock` with `h` = first 16 hex of sha256(canonical repo root `\0` cxpak version). The
   version in the hash gives an upgraded binary its own server, so an old server idles out beside it
   instead of blocking upgraded clients (bounded: one extra process per repo during an upgrade);
   the handshake hash stays as defense in depth. Total path must be <= 100 bytes (macOS `sun_path`
   is 104 incl. NUL); longer -> no server, in-process. Before bind or connect the run dir must be a
   real directory (not a symlink), owned by the effective uid, and neither group- nor world-writable;
   otherwise no server, in-process. Socket is chmod 0600 immediately after bind (dir is already 0700).
3. **Spawn race and stale socket.** `flock` (exclusive, `fs2`, already a dependency) on
   `cxpak-<h>.lock` in the same dir serializes the decision. Under the lock: connect-probe the
   socket. Success -> release lock, use it. `ECONNREFUSED`/`ENOENT` -> unlink any stale path, spawn
   the child, poll-connect (<= 5 s, 50 ms steps), release the lock. The child never takes the
   lock; its `bind` failing with `AddrInUse` (a stale path or a lost race) makes it exit 0 silently.
   Guarantee: exactly one server per (repo, version), because only a lock holder unlinks or spawns
   and `bind` is atomic. Lock contention waits <= 10 s, then falls back in-process.
4. **Idle exit.** Default 600 s with zero connected clients, timer reset on each disconnect;
   `CXPAK_INDEX_SERVER_IDLE_SECS` overrides (tests use 1-2 s). Justification: warm start makes a
   cold rebuild cheap, so a too-long timeout costs more (idle resident index, 174-407 MB) than a
   too-short one; 10 min covers the gap between factory role sessions and a coffee break, and a
   respawn after idle is the warm-start fast path, not the 14 s cold path. Revisit with data.
5. **Wire protocol.** Frames: `u32` big-endian length, then that many bytes of UTF-8 JSON; max frame
   64 MiB, larger or short read -> protocol error. First frame client->server:
   `{"version","wire_schema_hash","repo_root"}`; server replies `{"ok":true}` or
   `{"ok":false,"reason"}` and closes on mismatch. `wire_schema_hash` = sha256 of a const
   `WIRE_SCHEMA` revision string plus the serialized `mcp_tool_schemas()`, so any tool-surface
   change flips it. After the handshake each frame is a full JSON-RPC `tools/call` request object
   (the same `Value` the stdio loop parsed) and the reply frame is the full JSON-RPC response
   `Value` the stdio loop would have written; the client writes it to stdout unchanged, so output is
   byte-identical. `initialize`, `ping`, `tools/list`, notifications and parse errors stay in the
   client (instant, index-free). The server keeps one `SharedSnapshot` per connection (that state is
   per-session today) and runs the existing `snapshot_ready_index` + `handle_tool_call` against its
   one `SharedReadiness`.
6. **Freshness.** The server owns the single watcher; clients run none. Readiness semantics are the
   server's, identical to in-process: `Building` -> `Indexing` status, `Failed` -> terminal status.
7. **Fallback.** Connect timeout 2 s, handshake timeout 2 s, per-request read timeout 120 s
   (`CXPAK_INDEX_SERVER_REQ_TIMEOUT_SECS`), all bounded. Any connect/handshake/IO/timeout/protocol
   error -> the client starts today's `spawn_mcp_index_build` + `spawn_mcp_watcher` locally and
   permanently stays in-process for the rest of the session (no reconnect loop; one respawn attempt
   happens only at the next session start). The request that hit the error is answered with the
   standard retryable `Indexing` status while the local build runs, exactly today's `Building`
   behavior. `CXPAK_INDEX_SERVER=off` skips all of the above: code path identical to 3.5.1.
8. **Scope.** The transport trait `SharedIndexTransport` lives in `src/daemon/ipc.rs`; the Unix
   socket arm is `#[cfg(unix)]`; on non-Unix the connect function reports `Unavailable`, so Windows
   is always in-process. HTTP `serve` is unchanged (Phase 5, out of scope).
9. **Auth.** Option G stands, plus: dir owner/mode check (item 2) closes the swap-the-parent-symlink
   attack; no peer-credential check (same-uid processes already read the repo and `.cxpak/cache`).

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

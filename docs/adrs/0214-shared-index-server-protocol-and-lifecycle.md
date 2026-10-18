---
id: '0214'
title: Shared index server — local-socket protocol, spawn-on-demand lifecycle, version-skew fallback
status: ACCEPTED
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
falls back at once. The server must outlive any one session. A validation pass amended items 1-3, 6,
7 and 10-12 (log symlink, unlink race, build identity, setsid/reaping, env leakage, semver).

1. **Process model and no new CLI surface.** `pub enum Commands` is public and not
   `#[non_exhaustive]`, so a new variant or field would be a semver break. Server mode is therefore
   the SAME invocation, `cxpak serve --mcp <repo>`, re-exec'd by `std::env::current_exe()` with
   internal env `CXPAK_INDEX_SERVER_ROLE=server`, `CXPAK_INDEX_SERVER_SOCKET=<exact socket path>` and
   `CXPAK_INDEX_SERVER_BUILD_ID=<expected build id>`. `run_mcp` checks the role first. The child is
   detached with `setsid` (`pre_exec`, `SAFETY` comment: only async-signal-safe `setsid` runs between
   fork and exec), stdin `/dev/null`, stdout/stderr to the private log (item 3). The spawner keeps the
   `Child` handle on a reaper thread that `wait()`s, so the spawner never leaves a zombie while it
   lives; after the spawner exits the child is reparented to init. The server binds the socket FIRST,
   then builds/warm-starts the index on a background thread (`spawn_mcp_index_build`) and runs
   `spawn_mcp_watcher`, so a client attaches in milliseconds and a tool call during `Building`
   returns today's `Indexing` status. "First session hosts in-process" is rejected: it breaks the
   outlives-the-session requirement.
2. **Identity: root, version, build id.** `build id` = first 16 hex of sha256 of the running
   executable's CONTENT (streamed, cached per process); a rebuilt dev binary with an unchanged
   version string therefore never attaches to a stale server. Socket name `cxpak-<h>.sock`, `h` =
   first 16 hex of sha256(canonical repo root NUL version NUL build id). The same triple is in the
   handshake (defense in depth). The spawner passes the exact socket path and expected build id to
   the child; the child recomputes its own build id and EXITS on mismatch (binary replaced between
   the spawner's read and the exec), so a mid-session upgrade yields fallback, never a mislabelled
   server. Both sides canonicalize the repo root.
3. **Private run dir, log, pid file.** Base: `$XDG_RUNTIME_DIR` if set and absolute, else on macOS
   `std::env::temp_dir()` (per-user 0700 `/var/folders/.../T/`), else `<cache dir>/cxpak`. Under it
   a subdirectory `cxpak-<uid>` created 0700. Before bind or connect it must be a real directory
   (`symlink_metadata`, not a symlink), owned by the effective uid, and neither group- nor
   world-writable; otherwise no server, in-process. Files live ONLY there: `.sock` (chmod 0600
   after bind), `.lock`, `.pid` (written by the server after bind), `.log`. The log is NOT in the
   repo (a repo-planted `.cxpak/` symlink plus truncate-on-start could wipe a user file). It is
   opened `O_NOFOLLOW|O_APPEND`, 0600, never truncated; if it exceeds 256 KiB at spawn time it is
   rotated by `rename` to `.log.1` (replacing the previous one), inside the private dir. The total
   socket path must be <= 100 bytes (`sun_path` is 104 on macOS) else no server.
4. **Spawn race, stale socket, unlink race.** An exclusive `flock` (`fs2`, polled with
   `try_lock_exclusive`, <= 10 s, then fallback) on `cxpak-<h>.lock` serializes every decision to
   probe, spawn or unlink. Client under the lock: connect-probe; success -> use it;
   ECONNREFUSED/ENOENT -> unlink the stale path, spawn, poll-connect (<= 10 s, 50 ms steps; a cold debug binary on a loaded machine needs more than 5 s), release.
   The child never takes the lock; `bind` failing AddrInUse (lost race) exits 0 silently. Server
   exit: take the lock, re-check `clients == 0` (a client that connected after the idle decision
   aborts the exit), then unlink ONLY if `(dev, ino)` of the path still equals what the server
   recorded right after its own `bind`; otherwise leave it (a newer server owns the path). Because
   only a lock holder unlinks or spawns and `bind` is atomic, exactly one server per
   (repo, version, build id) exists.
5. **Idle exit.** Default 600 s with zero clients (timer restarts at server start and at every
   last-disconnect); `CXPAK_INDEX_SERVER_IDLE_SECS` overrides (tests: 1-2 s). Warm start makes a
   respawn cheap, so too long costs more (idle resident index, 174-407 MB) than too short; 10 min
   spans the gap between factory role sessions. Revisit with data.
6. **Wire protocol.** Frames: u32 BE length then UTF-8 JSON; max 64 MiB; larger/short read ->
   protocol error without allocating the claimed size. First client frame is
   `{"version","build_id","wire_schema_hash","repo_root","env_fingerprint"}`; the server replies
   `{"ok":true}` or `{"ok":false,"reason"}` and closes on mismatch or when it already serves 64
   clients. `wire_schema_hash` = sha256 of a const `WIRE_SCHEMA` revision plus serialized
   `mcp_tool_schemas()`. After the handshake each frame is a full JSON-RPC `tools/call` request and
   the reply is the full JSON-RPC response; the client writes it to stdout unchanged. Both the stdio
   loop and the server call ONE shared function, `dispatch_tools_call`, so output equality holds by
   construction. `initialize`, `ping`, `tools/list`, notifications and parse errors stay in the
   client. `SharedSnapshot` is audited: it is written only by the `context` op and read only by the
   context-diff op, i.e. pure per-session state, so the server holds one per connection. A client
   that falls back mid-session starts with an empty snapshot (context-diff then reports "no
   snapshot", the same text a fresh session gives).
7. **Environment and path leakage.** The server builds the index under the SPAWNER's environment
   and working directory, so output-affecting inputs are audited and pinned. Audit result (`grep`
   of `env::var`/`current_dir` in `src/`): `CXPAK_MAX_FILE_BYTES` (index build), `CXPAK_PG_DSN` and
   `CXPAK_MYSQL_DSN` (schema introspection); the embeddings remote `api_key_env` name is
   config-driven. The tool handlers read no cwd and no relative paths; the child is started with
   `current_dir` = canonical repo root and the root is always passed canonical. `env_fingerprint` =
   sha256 over the sorted `(name, value)` pairs of those variables that are set (hashed, so DSN
   secrets never cross the socket); a mismatch -> client falls back in-process. The embeddings
   `api_key_env` value is not fingerprinted (config-driven name; documented risk).
8. **Failed server.** The server exits (under the lock, with the dev/ino-guarded unlink) 2 s after
   its readiness becomes `Failed`. Attached clients see EOF on their next request, fall back, and
   their own local build reports the same terminal `Failed` text, so the "restart the MCP server"
   advice stays true; a new session spawns a fresh server (a retry of the build).
9. **Idle-shutdown race.** A handshake that ends in EOF/reset (server exited between probe and
   accept) triggers exactly one `ensure_server` retry before falling back. A version/build/env
   rejection never retries.
10. **Fallback.** Connect timeout 2 s (connect runs on a helper thread so a full backlog cannot
    block), handshake 2 s, per-request 120 s (`CXPAK_INDEX_SERVER_REQ_TIMEOUT_SECS`). Any
    connect/handshake/IO/timeout/protocol error -> the client starts today's
    `spawn_mcp_index_build` + `spawn_mcp_watcher` locally and stays in-process for the session; the
    failing request is answered with the retryable `Indexing` status while that build runs.
    `CXPAK_INDEX_SERVER=off|0|false` -> exactly the 3.5.1 path, no socket, lock or child.
11. **Scope.** `SharedIndexTransport` trait in `src/daemon/ipc.rs`, Unix arm `#[cfg(unix)]`;
    non-Unix reports `Unavailable` (always in-process). HTTP `serve` unchanged. `libc` becomes a
    plain `cfg(unix)` dependency (it was optional behind `client`).
12. **Auth.** Option G stands; the dir owner/mode check closes the parent-swap attack; no
    peer-credential check (same-uid processes already read the repo and `.cxpak/cache`).
13. **Test isolation.** Every spawning test sets `XDG_RUNTIME_DIR`, `TMPDIR` and `HOME` to a fresh
    short directory under `/tmp`, reads the server pid from the `.pid` file, and holds a drop
    guard that SIGKILLs every pid found, even on panic. The repo's `.cargo/config.toml` `[env]`
    sets `CXPAK_INDEX_SERVER=off` (overridable) so the pre-existing subprocess suites never spawn
    servers.

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

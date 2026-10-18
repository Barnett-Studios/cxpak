---
title: Shared per-repo index server — implementation phasing
status: READY
created: 2026-10-18
updated: 2026-10-18
spec: specs/2026-10-18-shared-index-server-SPEC.md
adrs: 0211, 0214
---

# Plan — shared index server

Implementation-ready. All protocol, lifecycle and path decisions are fixed in ADR-0214 "Resolved
design"; this plan only sequences them. Every phase is cross-cutting (new IPC module, lifecycle
state machine, edit to `serve.rs`'s readiness seam), so tasks are `local: false`. Each task writes
its RED test first, watches it fail for the right reason, then implements.

Module layout: `src/daemon/ipc.rs` (frames, handshake, `SharedIndexTransport` trait, Unix arm
`#[cfg(unix)]`), `src/daemon/index_server.rs` (server loop, idle exit, spawn/lock/path logic),
server role detected at the top of `run_mcp` from internal env vars (NO new `Commands` variant),
client fold in `run_mcp`; shared `dispatch_tools_call` extracted from the stdio loop. Tests: `tests/shared_index_server_tests.rs` (spawns the real binary via
`CARGO_BIN_EXE_cxpak`), unit tests beside each module. All env overrides read in one place
(`ServerConfig::from_env`) so tests inject values without mutating process env.

## Phase 1 — frames, handshake, transport trait, in-thread round trip

RED tests (unit, `ipc.rs`):
- frame encode/decode round trip; truncated length, truncated body, and a length > 64 MiB each
  return a typed error (no panic, no allocation of the claimed size).
- handshake accepts equal `{version, wire_schema_hash, repo_root}`, rejects each field
  mismatch with a reason; `wire_schema_hash` changes when `mcp_tool_schemas()` output changes.
- byte-equality: a `tools/call` served through `serve_connection` over a `UnixStream::pair()`
  equals `handle_tool_call` run in-process on the same index, byte for byte (determinism fixture).
Implement: `SharedIndexTransport` (connect w/ timeout, send frame, recv frame w/ timeout, close),
Unix arm, `serve_connection(readiness, snapshot_per_conn, stream)` reusing `snapshot_ready_index`
+ `handle_tool_call` + `mcp_tool_status`; non-Unix stub returns `Unavailable`.

## Phase 2 — socket path, dir validation, bind, server loop, idle exit

RED tests (`index_server.rs`):
- path derivation: stable for same (root, version), differs across root and across version;
  length <= 100 bytes else `PathTooLong`; macOS/Linux run-dir selection (Linux via injected
  `XDG_RUNTIME_DIR`).
- parent-dir rejection: not owned (skip-if-root), group-writable, world-writable, symlink,
  missing-and-uncreatable each yield `UnsafeRunDir`; a 0700 owned dir passes.
- bind: socket file mode is 0600; second bind on a live path returns `AddrInUse`.
- idle exit: server with idle 1 s and no clients exits and unlinks the socket; a connected client
  holds it alive past the timeout; exit only counts after the last disconnect.
- bounded connections: client N+1 over the cap is refused with a handshake `{"ok":false}`.
Implement: `ServerConfig`, `run_index_server(repo)` (bind first, then `spawn_mcp_index_build` +
`spawn_mcp_watcher`, accept loop with one thread per connection, atomic client count, shutdown
flag joins watcher/build/connection threads, unlink on every exit path).

## Phase 3 — client discovery, spawn lock, stale recovery, race

RED tests (`tests/shared_index_server_tests.rs`, real binary, temp repo, short idle):
- exactly-one: 8 concurrent `ensure_server` calls on a fresh repo -> exactly one server process
  (pid count via the log/`ps` on the socket's peer, or a pid file written by the server) and all 8
  connect.
- stale socket: leave a bound-then-abandoned socket file with no listener -> client unlinks under
  the lock, spawns, connects.
- lock contention timeout -> returns `Unavailable` (fallback), does not hang.
- server is detached: killing the spawning client does not kill the server.
- spawn failure (current_exe unusable via injected path) -> `Unavailable`.
Implement: flock'd lock, probe, unlink-if-stale, detached spawn (`setsid` via `pre_exec`, null stdin,
private-dir log, reaper thread), poll-connect <= 5 s.

Additional RED tests (validation findings): log symlink planted in the repo is never followed or
truncated; old server exit does not unlink a newer server's socket (dev/ino guard, run against two
overlapping servers); different build id -> different socket and handshake rejection; child refuses
when its build id differs from the expected one; server exits after `Failed`; handshake EOF during
idle shutdown retries once; env fingerprint mismatch -> fallback; no zombie after server idle exit
while the spawner lives; test isolation guard kills every pid.

## Phase 4 — fold `run_mcp` onto the client; fallback

RED tests (integration, real binary speaking stdio):
- `serve --mcp` with a live server answers a `tools/call` byte-identically to
  `CXPAK_INDEX_SERVER=off` on the same repo (after ready).
- tool call while the server is `Building` returns the same `Indexing` status object as in-process.
- kill -9 the server mid-session: next request returns the `Indexing` status within the connect/IO
  bound, then the local build completes and later calls match in-process output; the client never
  hangs (wall-clock asserted <= bound).
- server with mismatched version/wire hash (test hook env sets a fake hash in the server) ->
  client falls back, produces correct output, never attaches.
- `CXPAK_INDEX_SERVER=off`: no socket, no child process created, no lock file.
- hung server (accepts, never replies; `CXPAK_INDEX_SERVER_REQ_TIMEOUT_SECS=1`) -> fallback.
- existing `warm_start_tests.rs` and `serve.rs` MCP tests stay green untouched.
Implement: in `run_mcp`, `IndexServerMode::from_env`; if enabled and `ensure_server` yields a
connection, run a forwarding loop (a variant of `mcp_stdio_loop_readiness` that forwards
`tools/call` frames and answers the rest locally); on any transport error flip to the local path
by starting `spawn_mcp_index_build` + watcher then. Disabled/unavailable -> today's code unchanged.

## Phase 5 — acceptance measurement and docs

- Create the frozen clone `/tmp/frz-srv/scriptorium` (absent at plan time; clone then freeze HEAD
  and record the SHA), build release, run N=4 concurrent `serve --mcp` sessions each issuing one
  `cxpak_context` call and holding open; sample per-process `footprint`/`/usr/bin/time -l` peak
  footprint (server included); repeat with `CXPAK_INDEX_SERVER=off`. Target: total <= (one 3.5.1
  in-process footprint + 4 x thin-client footprint) x 1.2; report the numbers whatever they are. Record numbers and the idle-exit observation in `benches/BASELINES.md`.
- Docs in the same commits as code: `CXPAK_INDEX_SERVER`, `CXPAK_INDEX_SERVER_IDLE_SECS`,
  `CXPAK_INDEX_SERVER_REQ_TIMEOUT_SECS` documented in the env-var canonical doc; internal role env
  vars noted; CHANGELOG entry; ADR-0214 status ACCEPTED (done in phase 0).

## Out of scope

HTTP `serve` convergence (separate ADR if pursued), Windows named-pipe arm, machine-wide registry.

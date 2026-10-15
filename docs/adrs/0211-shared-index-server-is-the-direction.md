---
id: '0211'
title: A shared per-repo index server is the direction for cross-session memory — specified separately, after warm start
status: PROPOSED
date: 2026-10-15
triggered_by: specs/2026-10-15-warm-start-SPEC.md; comparison with code-review-graph's `crg-daemon`; measured MCP footprints
loop: planning
---

# ADR-0211: A shared per-repo index server is the direction for cross-session memory — specified separately, after warm start

## Context

Every `cxpak serve --mcp` process builds and holds its own index and runs its own watcher
(ADR-0200). That gives two separate problems.

**Freshness and persistence between sessions.** ADR-0210 solves these. Every consumer revalidates
per-file artifacts by content at start, so a stale or missing on-disk cache costs time, never
correctness. No editor hook, git hook or daemon is needed to keep it current. This matters for
two reasons:
- `cxpak hook install` correctly refuses a `core.hooksPath` outside the repo, which is common on
  machines with global hooks;
- editor hooks (code-review-graph's approach) are specific to each harness, which contradicts
  cxpak's harness-agnostic charter.

**Duplicated memory across concurrent sessions.** This one is unsolved, and measured. On 2026-10-15
a developer machine running the four factory roles plus ad-hoc sessions had **7 live
`cxpak serve --mcp` servers with physical footprints of 174–407 MB each, about 2.4 GB in total**
(`footprint -p`). Most of that is compressed or swapped out, so `ps` RSS shows only 142 MB. The
machine had 14 GB of swap in use.

code-review-graph avoids this by querying an on-disk SQLite graph rather than holding an in-memory
index per process.

## Options considered

- **Option A — a shared per-repo index server.** One long-lived process per repo (or per machine,
  with a registry) holds the index and the watcher. Each `serve --mcp` becomes a thin stdio shim
  that forwards to it over a local socket, falling back to an in-process index if the server is
  unavailable. *Pros:* one index per repo regardless of session count, and it stays
  harness-agnostic because every client still speaks MCP over stdio. *Cons:* a new lifecycle (spawn
  on demand, idle exit, crash recovery, binary upgrade while clients are attached), local-socket
  auth, version skew between shim and server, and Windows named pipes against Unix sockets.
- **Option B — editor-hook freshness** (Claude Code `PostToolUse`, Cursor and so on). It fixes
  neither memory nor anything the in-process watcher does not already see, and it is specific to
  each harness.
- **Option C — an on-disk queryable index** (SQLite or similar) instead of an in-memory one, as in
  code-review-graph. *Pros:* small resident memory per process. *Cons:* it rewrites every query path
  (graph, PageRank, call graph, search), adds a dependency, and lowers per-query latency for an
  index that already fits in memory.
- **Option D — do nothing:** accept per-session copies.

## Decision

**A is the direction**, delivered by its own spec, ADRs and plan **after** ADR-0210 ships. Warm start
comes first because the server's own start, restart and crash recovery depend on it, and it
benefits every consumer immediately. B is rejected. C is rejected for now: it costs more and carries
more risk than A for the same memory win. D is rejected on the measured footprint.

## Consequences

### Positive
- A clear, evidence-backed next step for the memory cost of parallel agent sessions.
- Freshness stays harness-agnostic and hook-free.

### Negative
- Until A ships, concurrent sessions on one repo each hold a full index (about 170–400 MB each).

### Neutral
- `cxpak serve` (HTTP) already runs as a long-lived single index server. Option A's server can
  reuse that surface.

## Revisit if

- A harness gains a first-class shared-MCP-server mode, making the shim unnecessary.
- Measured: the warm-start build of 3.5.0 itself cuts per-process steady-state footprint enough that
  the summed footprint across sessions falls below 1 GB on the same workload.

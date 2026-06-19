---
id: '0167'
title: Persist the derived index with a fail-closed, content-addressed fingerprint
status: ACCEPTED
date: 2026-06-16
triggered_by: v2.3.0 W1 — cold CLI invocations rebuild graph/pagerank/conventions/co-change from cached parses every time
loop: planning
---

# ADR-0167: Persist the derived index with fail-closed fingerprint invalidation

## Context

`cache::FileCache` (cache/mod.rs, `CACHE_VERSION = 2`) persists only `ParseResult`s, keyed on relative path + mtime + size and guarded by a compile-time grammar hash. The *derived* structures — dependency `graph`, `pagerank`, `conventions`, `co_changes` — are recomputed on every cold `build_index`, even when nothing changed since the last run. Co-change in particular is mined from git history via revwalk, so it depends on the current git HEAD, not only on file contents.

The danger of persisting derived state is staleness: serving a graph or PageRank that no longer matches the code is the one error a context tool must never make.

## Options considered

- **Option A — persist derived structures, content-addressed fail-closed fingerprint (chosen):** bump `CACHE_VERSION` to 3; write a `DerivedCache{grammar_hash, index_fingerprint, graph, call_graph, pagerank, conventions, co_changes}` alongside the parse cache. Valid iff `grammar_hash` matches **and** `index_fingerprint` matches, where the fingerprint hashes the sorted set of `(relative_path, sha256(file_bytes))` over all indexed files **plus the git HEAD oid** (co-change depends on history). Any mismatch, deserialize error, or truncation → discard and full rebuild. Same advisory-lock + atomic-write discipline as the parse cache. On a partial match (some files changed), run the ADR-0166 delta + ADR-0165 warm PageRank rather than a full recompute. `call_graph` is persisted alongside `graph` because `rebuild_graph` rebuilds both — caching only `graph` would serve an empty call graph on a hit. Pros: fast cold starts; staleness impossible by construction (fail closed); content-addressing makes the cache **portable across machines** (see Option D rationale). Cons: an invalidation surface to get right; per-file hashing cost; extra disk. Chosen because the conservative, content-based fingerprint makes correctness the default *and* unlocks portability.
- **Option B — no derived persistence (status quo):** Pros: zero staleness surface. Cons: every cold call pays full derived-recompute cost. Someone could prefer it for simplicity.
- **Option C — generation-counter or time-based invalidation:** Pros: cheap to check. Cons: fragile — a counter not bumped, or a clock skew, serves stale derived data. Rejected: trades the cardinal correctness property for convenience.
- **Option D — `(mtime, size)` fingerprint (the original draft, superseded):** hash `(relative_path, mtime, size)` like the existing parse cache. Pros: no hashing cost. Cons: (1) **not portable** — mtimes differ on every clone/CI runner, so a prebuilt cache never hits on another machine; (2) a same-size edit that preserves mtime evades detection (a real, if narrow, staleness hole). Rejected once it was clear `sha2` is **already a dependency** (`Cargo.toml`, conventions-export checksum), so content hashing costs zero new deps and removes both problems. The per-file hash cost is bounded by the bytes we already read to parse.

## Decision

Option A, fail-closed and **content-addressed**, with a **whole-repo (all-or-nothing) fingerprint** as built. The fingerprint is a single SHA-256 over every file's `(path, sha256(content))` plus the git HEAD oid; `DerivedCache::load` returns the cached graph / call_graph / PageRank / conventions / co-changes only on an exact match of version + grammar hash + fingerprint, else `None`. On a hit, `build_index` restores those derived fields — crucially skipping the expensive git-mined conventions/co-changes recompute. On any content (or HEAD) change the fingerprint misses and the index is rebuilt from scratch and re-saved. `DerivedCache` includes `call_graph` so a hit never serves an empty call graph.

A *partial* hit (restore, then delta-update only the changed files) is deliberately **not** implemented here — the all-or-nothing fingerprint is simpler and trivially fail-closed, and the in-process live-edit delta (ADR-0166 + warm PageRank) already handles incremental updates. Restore-then-delta is a tracked follow-up.

## Consequences

### Positive
- Cold `cxpak serve` startup restores the derived analysis on a fingerprint hit instead of re-mining git history (conventions + 180-day co-changes — the dominant cold cost), recomputing PageRank, and rebuilding the call graph.
- Staleness cannot be served: invalidation defaults to full rebuild.
- **Portable cache.** Because the fingerprint is content-based (not mtime-based), a derived cache built on one machine validates on any other with the same file contents. A team or CI job can publish a warm-index artifact that every clone/runner reuses — a concrete scale win, not just a local speedup.
- No same-size/same-mtime missed-edit hole: content hashing detects every byte change.

### Negative
- The fingerprint must cover every input to the derived structures (per-file content hash + git HEAD); missing an input is a staleness bug, so the fingerprint is deliberately broad (favoring unnecessary rebuilds over stale hits).
- Per-file SHA-256 adds CPU over an `(mtime,size)` stat. Bounded by the bytes already read for parsing, and `sha2` is already in the tree — but it is real cost on very large repos.
- Larger `.cxpak/cache/`; cleared by the existing `cxpak clean`.

### Neutral
- `CACHE_VERSION` bumps 2 → 3, auto-invalidating all prior caches on upgrade (one-time rebuild).
- The **parse cache** still keys on `(mtime, size)`; only the *derived* cache is content-addressed in this ADR. They are independent layers (separate files: `cache.json` vs `derived.json`); migrating the parse cache to content addressing is deferred (see Revisit-if).

## Revisit if
- Any staleness is ever observed (treat as P0) — broaden the fingerprint or narrow what is cached.
- The fingerprint causes excessive unnecessary rebuilds — narrow it carefully, with a parity test proving the narrower fingerprint still detects every staleness case.
- Co-change recomputation dominates even with caching — cache it separately keyed on HEAD oid alone.
- **(Deferred follow-up)** Migrate the *parse* cache off `(mtime, size)` to the same content hash, so the whole cache stack is portable and hole-free. Out of scope for W1; do it when the parse-cache mtime assumption causes a real missed-edit or blocks full-artifact portability.
- Per-file hashing measurably hurts cold-start on huge repos — consider hashing in parallel, or a cheaper content digest, with the parity tests still green.

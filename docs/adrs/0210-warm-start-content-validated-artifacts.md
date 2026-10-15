---
id: '0210'
title: Warm start — an internal content-validated artifact store, a fingerprint from fresh hashes, and a derived hit that skips the derived passes
status: PROPOSED
date: 2026-10-15
triggered_by: specs/2026-10-15-warm-start-SPEC.md (scriptorium MCP index-ready 14.9 s on an unchanged repo)
loop: planning
---

# ADR-0210: Warm start — an internal content-validated artifact store, a fingerprint from fresh hashes, and a derived hit that skips the derived passes

## Context

On an unchanged repo with the ADR-0167 derived cache hitting, `cxpak serve --mcp` still takes 14.9 s
to index a frozen scriptorium clone (4,426 files). Profiling shows about 11.7 s of that is
recomputation, all on one thread:

| Work | Time |
|---|---|
| tree-sitter parse | 3.4 s |
| tiktoken | 3.3 s |
| term frequencies | 1.2 s |
| call graph (re-parses every file) | 3.4 s |
| dependency graph | 0.4 s |

Each item is a pure function of either one file's bytes or of the whole content fingerprint.

Two latent correctness issues sit on the same path:
- The derived fingerprint reuses hashes keyed by (mtime, size) through the stat-index, so a
  same-size, same-mtime edit gives a stale derived hit. This is the cxpak#36 bug class, on the
  derived cache.
- Cross-language edge order is `HashMap`-dependent, and restored PageRank can be 1 ULP off.

Three constraints shape the choice:
- `cxpak::cache` and `cxpak::core_graph` are public, with all-public-field structs. No minor release
  since 3.0.0 has changed them, so this cannot break them under 3.5.0.
- Every consumer (MCP, HTTP, LSP, `watch`, and the CLI commands built on `build_index`) inherits the
  decision.
- Some harnesses kill an MCP server that is slow to start (ADR-0185).

## Options considered

- **Option A — extend `cache.json` (`CacheEntry` v7) and `DerivedCache`.** One store, with the
  derived cache carrying cross-language edges. *Pros:* fewest files, no new type. *Cons:* it changes
  public struct fields and `DerivedCache::new`'s arity, a semver break under a minor version. It
  also bumps `CACHE_VERSION`, which throws away every derived cache and the post-commit delta base.
  Someone could prefer it for a 4.0 release.
- **Option B — a new crate-private `ArtifactStore` (`.cxpak/cache/artifacts.json`) that becomes the
  only per-file store in use.** The CLI `parse_with_cache` moves to it with its signature unchanged.
  `FileCache` and `CacheEntry` are deprecated, not changed. Entries are validated by the SHA-256 of
  the settled bytes. *Pros:* no API break, one store in use, the derived cache and delta base
  survive. *Cons:* deprecated types linger until 4.0, and an old `cache.json` sits orphaned until
  `cxpak clean`.
- **Option C — trust (mtime, size) on the index path**, reusing the existing CLI cache. *Pros:* it
  does not re-hash unchanged files. *Cons:* the index has to hold every file's content anyway, so
  the read is never saved. It also keeps the cxpak#36 bug class alive by design.

For the derived passes and the fingerprint:

- **Option D — status quo:** compute the passes, then overwrite them on a hit, with the fingerprint
  coming from the stat-index.
- **Option E — compute the fingerprint first, from the hashes this build just took**, without
  touching the stat-index. On a hit, restore graph, PageRank and call graph without computing them.
  Re-detect cross-language edges, which is today's semantics, now sorted.

## Decision

**B + E.** The decision has four parts:
1. A crate-private, content-validated `ArtifactStore` in the flat cache dir, shared across workspace
   namespaces and with the CLI, holding the parse result, token count and term frequencies.
2. Parallel per-file work in scanner order.
3. A fingerprint computed from fresh hashes, so the derived cache is content-exact and mtime is
   never a validity key on the index path.
4. A derived hit that skips the passes it would overwrite.

Alongside these, `detect_cross_lang_edges` returns sorted output and `serde_json` gets
`float_roundtrip`. There is no public API change and no `CACHE_VERSION` bump.

## Consequences

### Positive
- A warm start costs reading and hashing each file, plus work proportional to what changed. The
  target is ≤ 40 % of the baseline on scriptorium (spec A1).
- The derived cache can no longer serve a stale hit after a same-size, same-mtime edit.
- Cross-language edges and PageRank are deterministic across processes.
- Harness-agnostic: every consumer of `build_index` benefits, with no hooks or daemon.

### Negative
- `artifacts.json` grows by the term-frequency maps (tens of MB on large repos). Its load time sits
  on the warm path and is measured in `BuildReport`.
- Deprecated public types and the unused stat-index code stay until 4.0.
- The first start after upgrade is cold for per-file work. The derived cache still hits.

### Neutral
- The watcher's in-session incremental path is unchanged. Edits made during a session reach the
  store at the next start, as one recompute per changed file.
- `hook.rs` still uses `content_fingerprint`, unchanged.

## Revisit if

- Store load exceeds 25 % of the warm index-ready time: use a binary encoding (`bincode` is already
  in the tree behind `embeddings`).
- A post-commit warm start (artifacts hit, derived miss) exceeds 6.0 s median on scriptorium: Phase
  2, a call graph from cached parse results plus conventions after `Ready` (spec §8).
- 4.0: delete `FileCache`, `CacheEntry`, `StatIndex` and `content_fingerprint_with_stat_index`.

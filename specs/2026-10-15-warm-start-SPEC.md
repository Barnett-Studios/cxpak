---
title: cxpak warm start — persistent per-file artifacts, exact fingerprint, no wasted derived work
status: DRAFT (revised after validation + 3 expert reviews)
created: 2026-10-15
updated: 2026-10-15
target: cxpak 3.5.0 (MINOR — no public API change; new internal store)
adrs: 0210 (warm start), 0211 (shared index server is the direction; separate spec)
---

# cxpak warm start — Spec

## 1. Problem (measured)

**Metric.** Wall time from spawning `cxpak serve --mcp <repo>` until the server prints
`cxpak: MCP index ready`, measured with `scripts/mcp-ready.py`. Measurements use **frozen clones**,
because a live repo's HEAD moves under it, and HEAD is part of the ADR-0167 fingerprint. The clones
are `/tmp/frz/{claude,scriptorium}`: `git clone` of `~/.claude` and of scriptorium. Taken on
cxpak 3.4.0, a MacBook M4 Pro (12 cores), load average 3.9–5.3, on 2026-10-15.

| Repo (frozen clone) | Files / tokens | Cold (no `.cxpak/`) | Warm (derived hit) |
|---|---|---|---|
| ai-sdlc (`claude`) | 967 / 1.53 M | 2.94 s | 2.00 s |
| scriptorium | 4426 | 21.31 s | 14.90 s (±0.05, n=3) |

**Profile.** `sample(1)` of the scriptorium warm start shows `build_index_with_workspace` taking
about 12.2 s of the 14.9 s. Of that:

- tree-sitter parse in the build loop: 3.4 s
- tiktoken token counting: 3.3 s
- term frequencies: 1.2 s
- `build_call_graph`: 3.4 s. It re-parses every file with tree-sitter, then its result is thrown
  away on a derived hit.
- `build_dependency_graph`: 0.4 s
- file reads: 0.4 s

Everything runs on one thread.

**Code facts** (`main` @ 2dcab31):

1. **The index path has no per-file cache.** `build_index_with_workspace`
   (`src/commands/serve.rs:191-340`) parses every file sequentially. Its callers are MCP, HTTP
   `serve`, LSP, `watch`, `conventions`, `onboard` and `visual`. `build_with_settled_content`
   (`src/index/mod.rs:505-569`, whose only caller is `serve.rs:248`) then tokenises every file and
   computes term frequencies. The CLI-only `parse_with_cache` (`src/cache/parse.rs:48`) keeps
   `.cxpak/cache/cache.json`, keyed by (mtime, size). Its `token_count` is an estimate that nothing
   reads (`overview.rs:198-215` and `trace.rs:74-92` only rewrite it).
2. **A derived-cache hit still pays for the derived passes.** `finish_build`
   (`src/index/mod.rs:323-387`) builds the graph, PageRank, call graph and cross-language edges
   *before* `build_index` consults the derived cache, which then overwrites graph, PageRank and call
   graph (`serve.rs:298-305`).
3. **The derived fingerprint trusts mtime.** `content_fingerprint_with_stat_index`
   (`cache/mod.rs:324-380`) reuses a stored hash whenever (mtime_ns, size) matches. A same-size,
   same-mtime edit therefore yields a stale fingerprint and a stale derived hit. The stat-index is
   saved on every build, even when nothing changed (`serve.rs:296`), and is never pruned (2,851
   entries for 976 files on `~/.claude`).
4. **`detect_cross_lang_edges` output order is nondeterministic.** `detect_shared_schema_bridges`
   (`intelligence/cross_lang.rs:598-646`) iterates a `HashMap`, and `dedup` (`:319`) keeps the first
   occurrence.
5. **PageRank does not round-trip exactly.** It is restored from JSON without `serde_json`'s
   `float_roundtrip` feature (`Cargo.toml:33`), so a restored value can differ from the computed one
   by 1 ULP.
6. **The public API is wide.** `pub mod cache` and `pub mod core_graph` (`lib.rs:5,15`) expose
   `FileCache`, `CacheEntry`, `DerivedCache` and `CrossLangEdge` with all-public fields. Changing any
   of them is a semver break, and there has been no such change in a minor release since 3.0.0.

*Aside (not cxpak's defect).* Live `~/.claude` indexes 2,683 files and 8.2 M tokens, against 967 and
1.5 M in a clean clone. Untracked, non-ignored files (`skills/synced/`, `plugins/*-cache*.json`)
account for the difference. That is a `.gitignore` gap in the dotclaude repo, tracked separately.

## 2. Goal

A cxpak process starting on a repo it has seen before pays for reading and hashing files, plus work
proportional to what changed. Freshness stays exact. This holds in every harness and for every
consumer, because the persistence lives in cxpak's own on-disk cache.

## 3. Non-goals

- A shared index server. ADR-0211 chooses it as the direction; it gets its own spec.
- Incremental recompute on a derived miss (call graph, conventions). This is Phase 2, gated in §8.
- Changes to the watcher's in-session path (ADR-0200/0204/0205), to `build_with_content` (the CLI
  `overview`/`trace` index builds), or to `hook.rs`.
- Any new crate dependency or public API change.

## 4. Hard constraints (a violation is a defect)

| ID | Constraint |
|---|---|
| C1 Parity | For identical repo content, a warm build equals a cold build on every one of: `files` (content, token_count, parse_result, mtime_ns), `term_frequencies`, `graph`, `pagerank` (exact bits), `call_graph`, `cross_lang_edges`, `test_map`, `schema`, `domains`, `conventions`, `co_changes`, `total_files`, `total_tokens`, `total_bytes`, `language_stats`. |
| C2 Content-validated | On the index path, both per-file artifact reuse and the derived fingerprint come from the SHA-256 of the bytes returned by this build's `SettledRead`. mtime is never used as a validity key. (`SettledRead` holds a `String` from `read_to_string`, so hashing `content.as_bytes()` hashes the raw bytes.) |
| C3 Fail-closed | A missing, truncated, corrupt, version- or grammar-mismatched store, or a failed save, gives a full recompute. It never surfaces as an indexing error. |
| C4 ADR-0206 / #40 | Unreadable, non-UTF-8 and over-cap files are absent from the index, the totals, the fingerprint **and the store**. |
| C5 Order | `index.files` keeps scanner order under parallel per-file work. |
| C6 No stale fields | Every `Some` field of a store entry was computed from the bytes its `content_sha256` names. A writer that did not compute a field stores `None`. |
| C7 API | No public item changes signature, fields or derives in a breaking way. `build_index`, `build_index_with_workspace`, `parse_with_cache`, `FileCache`, `CacheEntry`, `DerivedCache`, `StatIndex`, `content_fingerprint*` keep their shapes. Items that become unused are `#[deprecated]` (removed in 4.0). |
| C8 Coverage | ≥ 90 % on new and changed code (tarpaulin CI). |

## 5. Design

### 5.1 A new internal artifact store (replaces `cache.json` in use, not in API)

- **Location and type.** `pub(crate) struct ArtifactStore` lives at `.cxpak/cache/artifacts.json`.
  This is the **flat** cache dir, shared by every workspace namespace and by the CLI, because
  `relative_path` is always repo-relative (`scanner::scan_workspace` filters a full scan). It is
  guarded by the existing `.cxpak/cache/cache.lock`. The derived cache stays in the per-namespace
  dir (`cache_namespace`), so `build_index_with_workspace` uses two dirs: the flat one for artifacts
  and the namespaced one for derived.
- **Versioning.** `ARTIFACT_STORE_VERSION = 1` plus the compile-time grammar hash. A mismatch means
  an empty store (C3). `CACHE_VERSION` stays 6, so derived caches, the stat-index files and the
  post-commit delta base (ADR-0179) all survive the upgrade.
- **Entry.** `entries: BTreeMap<String, ArtifactEntry>`, giving deterministic bytes, where
  `ArtifactEntry` has:

  | Field | Notes |
  |---|---|
  | `content_sha256: String` | mandatory: every writer read the bytes |
  | `mtime_ns: u64`, `size_bytes: u64` | the CLI fast-path key only |
  | `language: Option<String>` | |
  | `parse_result: Option<ParseResult>` | |
  | `token_count: Option<usize>` | |
  | `term_frequencies: Option<BTreeMap<String, u32>>` | |

- **Index reader.**
  - *Hit:* the stored hash equals the sha of the settled bytes, the language matches, and
    `token_count` and `term_frequencies` are both `Some`.
  - *Partial hit:* the hash and language match. Reuse the parse result and compute the missing
    fields.
  - *Otherwise:* a miss.
- **CLI reader.** `parse_with_cache` keeps its signature and its (mtime_ns, size) hit test, now
  against the artifact store. On a miss it reads the file, hashes it, parses it, and writes
  `token_count: None` and `term_frequencies: None`.
- **Writers.** Each writer merges through
  `merge_store_entries(existing, fresh, workspace: Option<&str>)`:
  - a full scan keeps only the paths it saw;
  - a workspace scan replaces the entries under its prefix and keeps the rest.
  
  The index path saves only when there was a miss, a partial hit or a prune
  (`BuildReport.store_saved`), and saves **after** publishing `Ready`. The CLI saves as today.
- **Legacy.** `FileCache` and `CacheEntry` stay public but are no longer used, and are marked
  `#[deprecated]`. The token-count patch blocks in `overview.rs` and `trace.rs` are deleted. An old
  `cache.json` is left in place; `cxpak clean` removes it.
- **Memory.** Store entries are taken out by value (`HashMap::remove`) as each file is processed, so
  term-frequency maps are never held twice. The store is dropped before the derived load.

### 5.2 Parallel per-file work

`files.par_iter().map(..).collect::<Vec<_>>()`, which preserves scanner order (C5). For each file:
1. settled read (`None` means no trace, C4);
2. raw sha;
3. the stripped-body sha for the fingerprint (equal to the raw sha when `strip_md_frontmatter`
   returns the whole input);
4. store lookup (taking the entry out);
5. on a miss, `compute_file_artifacts`.

`TokenCounter` must be `Sync`; a compile-time assert checks this, with `map_init` per thread as the
fallback.

### 5.3 Exact fingerprint, computed first

The pairs-to-digest step of `content_fingerprint` and `content_fingerprint_with_stat_index` is
extracted into a `pub(crate) fn fingerprint_from_hashes`. Both keep their signatures and produce the
same value, which existing tests pin. `build_index` computes the fingerprint from step 3's fresh
hashes **before** constructing the index, and no longer reads or writes the stat-index (C2).
`content_fingerprint_with_stat_index` and `StatIndex` become `#[deprecated]` once they are unused.

### 5.4 A derived hit skips the derived passes

`finish_build` gains `precomputed: Option<Precomputed { graph, pagerank, call_graph }>`.

- **`Some`:** restore all three. Re-detect `cross_lang_edges` without adding them to the graph,
  because the restored graph already holds them. This is exactly today's hit semantics: a
  re-detection that runs `detect_shared_schema_bridges` against a graph that already holds
  `CrossLanguage` edges reads only `EmbeddedSql`, `OrmModel` and `ForeignKey` edges
  (`cross_lang.rs:609-612`), so it sees the same input. Always compute `schema`, `test_map` and
  `domains`; none of them reads graph, PageRank or call graph (`index/mod.rs:330-367`).
- **`None`:** today's `finish_build`, unchanged.

`build_with_settled_content` delegates with `None`. `DerivedCache` is unchanged, so there is no
format bump.

### 5.5 Determinism fixes

- `detect_cross_lang_edges` returns its edges sorted by (source_file, target_file, bridge_type,
  source_symbol, target_symbol, source_language, target_language). Order was arbitrary before.
- `serde_json` gains `features = ["float_roundtrip"]`, an existing dependency, so the restored
  PageRank is bit-exact.

### 5.6 Observability

The ready line becomes
`cxpak: MCP index ready (N files, T tokens; artifacts H hit/P partial/M miss, derived hit|miss, X.XXs)`.
A crate-internal `build_index_with_workspace_reported` returns a `BuildReport` with:
- `artifact_hits`, `artifact_partial`, `artifact_misses`;
- `derived_hit`;
- `store_saved`;
- per-phase `Duration`s: scan, per-file, fingerprint, derived load, assemble, store load.

Tests assert on the report. The public wrappers drop it (C7).

### 5.7 Benchmark method

`scripts/mcp-ready.py <cxpak> <repo> [timeout]` prints `index_ready_s`, `first_answer_s` and
`peak_rss_mb`, with `start` taken before `Popen`. The protocol is:
- frozen clones;
- **n ≥ 5 interleaved runs** of the old and new binaries;
- report the median and the min–max spread, and record `uptime` load;
- a hit or miss is confirmed from the ready line.

The baseline and the results go in `benches/BASELINES.md`.

## 6. Acceptance

| # | Check | How |
|---|---|---|
| A1 | Warm, unchanged: median `index_ready_s` ≤ 40 % of the baseline median on the scriptorium clone (14.9 → ≤ 6.0 s), and ≤ baseline on the claude clone | §5.7 |
| A2 | Cold: median ≤ baseline median × 1.10, with non-overlapping spreads reported honestly | §5.7 |
| A3 | Peak RSS (warm) ≤ baseline × 1.10 | §5.7. **Result:** not met on RSS (+15–45 %, noisy); peak `phys_footprint` −14 %, steady flat. See `benches/BASELINES.md`. |
| A4 | Parity, derived hit: report `derived_hit && hits == total_files`, and C1 fields equal | test |
| A5 | Parity, derived miss with artifacts hitting: report `!derived_hit && hits == total_files`, and C1 fields equal | test |
| A6 | Reuse is real: a sentinel `parse_result` planted under a matching hash is exposed | test |
| A7 | No stale reuse: same length, restored mtime, different bytes; the sentinel is not exposed and the new symbol is | test |
| A8 | Derived reuse is real: a sentinel call-graph edge in a matching-fingerprint `derived.json` is exposed; on a cold build it is absent | test |
| A9 | Exact fingerprint: a same-length edit with restored mtime gives a derived miss | test |
| A10 | C4: an unreadable file and an over-cap file each leave no entry in the store, the index or the totals | test |
| A11 | C3: a truncated store, a version mismatch and an unwritable cache dir each still build successfully, with all misses | test |
| A12 | Workspace scan keeps foreign entries; a later full scan prunes deleted paths | test |
| A13 | Concurrent builds (2 threads, same repo) leave a loadable store whose every entry's hash matches its file | test |
| A14 | Partial hit: a CLI-written entry (`term_frequencies: None`) reuses the parse and fills the rest | test |
| A15 | A warm unchanged build sets `store_saved == false` and writes no stat-index | test |
| A16 | Cross-language order is sorted | test |
| A17 | Gates: `cargo fmt --check`, `clippy --all-targets -D warnings`, `cargo test --all-features`, `scripts/feature-matrix.sh`, tarpaulin ≥ 90 % | CI |

## 7. Risks

| Risk | Mitigation |
|---|---|
| Store JSON load time grows | `BuildReport` times it. ADR-0210 "Revisit if" covers a binary encoding. |
| `TokenCounter` not `Sync` | Compile-time assert, with `map_init` as the fallback. |
| Peak RSS from parallel parsing | A3 ceiling; move-out of entries. |
| Concurrent writers | Lock plus atomic rename, last writer wins. Entries are content-validated, so a lost entry costs time, never correctness (A13). |
| A derived miss after every commit | Not addressed here. Phase 2 is gated on the measurement in §8. |

## 8. Phasing

- **Phase 1** (this spec, one PR, 3.5.0).
- **Phase 2** (gated). Measure a post-commit warm start on the scriptorium clone (an empty commit,
  then a start; artifacts hit, derived miss). If the median is > 6.0 s, write a spec for an
  incremental derived rebuild: a call graph from cached parse results instead of re-parsing, plus
  conventions published after `Ready`.
- **Phase 3.** The shared index server (ADR-0211), with its own triad.

---
title: cxpak warm start — Implementation Plan
status: DRAFT (revised after validation + 3 expert reviews)
created: 2026-10-15
updated: 2026-10-15
spec: specs/2026-10-15-warm-start-SPEC.md
adrs: 0210, 0211
complexity: Medium (one hot path, one new internal store, one constructor parameter)
risk: Medium (parity of derived structures; concurrency of the store)
---

# Plan: cxpak warm start

## Executive summary

Make a restart on an already-seen repo pay only for reading, hashing and what changed. Three changes
do this:
- a crate-private, content-validated `ArtifactStore`;
- parallel per-file work;
- a fingerprint computed from fresh hashes, after which a derived hit skips the passes it would
  overwrite.

There is no public API change and no `CACHE_VERSION` bump. One PR (`perf/warm-start`), released as
**3.5.0**. Phase 2 is gated by a measurement taken at the end.

## Gates

| Gate | Rule |
|---|---|
| TDD | All Phase 1 tests are committed red in one commit **before** any implementation commit. Each test is red for its stated reason. |
| Spec constraints | C1–C8 (spec §4). |
| No new deps | `serde_json` gains the `float_roundtrip` feature only. `rayon`, `sha2` and `git2` are already present. |
| Lint and tests | `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`; `cargo test --all-features`; `bash scripts/feature-matrix.sh` |
| Coverage | ≥ 90 % (tarpaulin CI). |
| Docs with code | ADR INDEX rows, a `benches/BASELINES.md` section, and a README note in the same PR. |

## Current state (verified on `main` @ 2dcab31)

| File | Today | Change |
|---|---|---|
| `src/commands/serve.rs:186-340` | sequential parse loop, then `build_with_settled_content`, then stat-index fingerprint, then derived overwrite | T8 |
| `src/index/mod.rs:323-387,505-569` | `finish_build`, `build_with_settled_content` (single caller) | T4, T7 |
| `src/cache/mod.rs:257-380` | `content_fingerprint`, `content_fingerprint_with_stat_index` (duplicated digest loop) | T3 |
| `src/cache/parse.rs:48-223` | CLI `parse_with_cache` on `FileCache` (mtime/size) | T5 |
| `src/commands/overview.rs:198-215`, `src/commands/trace.rs:74-92` | token-count patch of `cache.json` | T6 (delete) |
| `src/intelligence/cross_lang.rs:30-60,319` | unsorted output | T2 |
| `Cargo.toml:33` | `serde_json = "1"` | T2 |
| new `src/cache/artifacts.rs` | — | T3 |
| new `src/commands/warm_start_tests.rs` (`#[cfg(test)] #[path]` from serve.rs) | — | T1 |

## Phase 0 — measurement harness and baseline (done in the triad commit)

**T0.** `scripts/mcp-ready.py` (with `start` taken before `Popen`). The baseline numbers are in
spec §1. The final A1–A3 numbers follow spec §5.7: frozen clones `/tmp/frz/{claude,scriptorium}`,
n ≥ 5 interleaved runs of the old (3.4.0) and new binaries, median plus spread plus load average.

## Phase 1 — tests first (one commit, all red)

**Fixture `warm_repo()`** (in `warm_start_tests.rs`). A tempdir git repo:
- `src/a.rs` and `src/b.rs`, where b calls a;
- `frontend/api.ts` and `backend/users.py`, copied from `cross_lang.rs:793`, so that one `HttpCall`
  cross-language edge exists;
- `docs/note.md` with YAML frontmatter;
- committed with a **fixed** `git2::Signature::new("t","t@t",&Time::new(1_700_000_000,0))`, so
  conventions and co-changes are deterministic.

**Helpers.**
- `store_path(root) = root/.cxpak/cache/artifacts.json`;
- `derived_path(root) = root/.cxpak/cache/root/derived.json`;
- `snapshot(&CodebaseIndex) -> serde_json::Value` over every C1 field. `files` is mapped to
  (relative_path, content, token_count, parse_result, mtime_ns), and `pagerank` is compared through
  `f64::to_bits`.

| Test | File | Asserts | Red today because |
|---|---|---|---|
| T1.1 `artifact_store_roundtrip` / `artifact_store_rejects_version_and_grammar_mismatch` / `artifact_store_truncated_file_is_empty` | cache/artifacts.rs | save then load is equal; mismatch or truncation gives an empty store | the module does not exist (compile red) |
| T1.2 `merge_full_scan_prunes`, `merge_workspace_keeps_foreign`, `merge_fresh_replaces_stale`, `merge_output_sorted` | cache/artifacts.rs | §5.1 merge rules | does not exist |
| T1.3 `fingerprint_from_hashes_matches_content_fingerprint` | cache/mod.rs | for the fixture's files, the extracted function equals `content_fingerprint` | does not exist |
| T1.4 `compute_file_artifacts_matches_index_build` | index/mod.rs | for a Rust snippet, parse, token count and term frequencies equal what `build_with_settled_content` produces for the same file | does not exist |
| T1.5 `cross_lang_edges_are_sorted` | intelligence/cross_lang.rs | output == sorted output on a two-table SharedSchema plus HTTP fixture | unsorted `HashMap` order. Rerun the fixture with five shuffled insertion orders so the test is red deterministically, not by chance. |
| T1.6 `warm_equals_cold_on_derived_hit` | warm_start_tests.rs | report 2: `derived_hit && artifact_hits == total_files`; `snapshot(cold) == snapshot(warm)` | no `build_index_with_workspace_reported` / `BuildReport` (compile red) |
| T1.7 `warm_equals_cold_on_derived_miss` | warm_start_tests.rs | `derived.json` deleted between builds; report 2: `!derived_hit && artifact_hits == total_files`; snapshots equal | same |
| T1.8 `artifact_reused_by_content_hash` | warm_start_tests.rs | sentinel symbol planted in `src/a.rs`'s store entry (hash kept); the warm `parse_result` has the sentinel | same |
| T1.9 `stale_hash_not_reused` | warm_start_tests.rs | `src/a.rs` rewritten with same-length bytes and a different fn name; mtime restored via `File::set_modified`; no sentinel, the new fn name present, report `artifact_misses == 1` | same |
| T1.10 `derived_hit_does_not_rebuild_call_graph` | warm_start_tests.rs | a `#[cfg(test)]` thread-local counter in `build_call_graph` (`finish_build` runs on the caller's thread) reads 0 on the warm derived-hit build and ≥ 1 on the cold build; a sentinel call-graph edge planted in `derived.json` (fingerprint kept) is present in the warm `call_graph`; `cross_lang_edges` equals cold's | today `finish_build` always calls `build_call_graph`, so the counter is 1 |
| T1.11 `same_length_edit_with_restored_mtime_misses_derived` | warm_start_tests.rs | after the T1.9-style edit, report `!derived_hit` | stat-index trusts mtime, so there is a stale hit |
| T1.12 `unreadable_and_over_cap_leave_no_trace` | warm_start_tests.rs | a non-UTF-8 file and a (5 MiB + 1)-byte file are absent from the index, `total_files` and the store | no store |
| T1.13 `store_failure_is_not_an_indexing_error` | warm_start_tests.rs | truncated `artifacts.json`, version-mismatched store, and `.cxpak/cache` made read-only (`set_permissions` 0o555, unix-only `#[cfg(unix)]`): each build is `Ok`, with `artifact_misses == total_files` | no API |
| T1.14 `workspace_build_keeps_foreign_entries_then_full_build_prunes` | warm_start_tests.rs | `Some("src")` keeps the frontend/backend entries; after deleting `backend/users.py`, a full build prunes it | no API |
| T1.15 `concurrent_builds_leave_valid_store` | warm_start_tests.rs | two threads call `build_index` on the same repo after one edit; the store loads, and every entry's hash equals the sha of its file | no store |
| T1.16 `cli_entry_is_partial_hit` | warm_start_tests.rs | `parse_with_cache` first, then build: report `artifact_partial == parsed files`, and the parse result is reused (sentinel planted between steps) | no store |
| T1.17 `warm_unchanged_writes_nothing` | warm_start_tests.rs | report `store_saved == false`; `stat_index.json` is not created on a fresh repo's second build | stat-index is saved on every build |
| T1.18 `files_keep_scanner_order` | warm_start_tests.rs | the order of `index.files` paths equals `Scanner::scan_workspace` order (5 files) | no API |

The `BuildReport` fields used by the tests are `artifact_hits`, `artifact_partial`,
`artifact_misses`, `derived_hit`, `store_saved` and `timings`.

## Phase 2 — implementation (each task commits green for its tests)

- **T2. Determinism.**
  - `Cargo.toml`: `serde_json = { version = "1", features = ["float_roundtrip"] }`.
  - `cross_lang.rs`: `detect_cross_lang_edges` sorts its result by the spec §5.5 key, via a
    `sort_by` on a tuple of references; `CrossLangEdge` gets no new derive.
  
  Turns T1.5 green.
- **T3. Store and fingerprint.**
  - `src/cache/artifacts.rs` (`pub(crate)`):
    - `ArtifactEntry`;
    - `ArtifactStore { version, grammar_hash, entries: BTreeMap }`;
    - `load(cache_root)`, which takes the lock and is fail-closed;
    - `save(cache_root)`, which takes the lock and writes atomically;
    - `merge_store_entries`, sorted, using `scanner::path_starts_with_workspace`.
  - `cache/mod.rs`: extract `pub(crate) fn fingerprint_from_hashes(pairs: &mut Vec<(&str, String)>, head) -> String`.
    `content_fingerprint` and `content_fingerprint_with_stat_index` call it.
  
  Turns T1.1–T1.3 green.
- **T4. `compute_file_artifacts`** (index/mod.rs, `pub(crate)`): a tree-sitter parse via
  `LanguageRegistry` (the same calls as `serve.rs:229-238`), `count_or_zero` and
  `compute_term_frequencies`. Add `const _: fn() = || { fn s<T: Sync>() {} s::<TokenCounter>(); s::<LanguageRegistry>(); };`.
  `build_with_settled_content` is unchanged in this task. Turns T1.4 green.
- **T5. CLI on the store** (cache/parse.rs). `parse_with_cache`:
  - loads the `ArtifactStore`;
  - a hit is (mtime_ns, size) equal, returning the parse result;
  - a miss reads, hashes and parses, writing `token_count: None` and `term_frequencies: None`;
  - merges and saves.
  
  The signature and return values are unchanged, and the signature-estimate block is deleted.
  `FileCache` and `CacheEntry` get `#[deprecated(since = "3.5.0", note = "superseded by the internal artifact store (ADR-0210); removed in 4.0")]`,
  with `#[allow(deprecated)]` on their own tests. The existing `parse_with_cache` tests stay green.
- **T6. Delete the token patches** in `overview.rs` and `trace.rs`. Their tests stay green.
- **T7. `finish_build(precomputed: Option<Precomputed>)`** (index/mod.rs).
  `pub(crate) struct Precomputed { graph, pagerank, call_graph }`.
  - When `Some`, restore all three and set `cross_lang_edges = detect_cross_lang_edges(&index)`
    without injecting them.
  - When `None`, today's code.
  
  Add `pub(crate) fn from_artifacts(files: Vec<ArtifactFile>, precomputed: Option<Precomputed>) -> Self`,
  which assembles the totals, `language_stats` and `term_frequencies` in input order. Rewrite
  `build_with_settled_content` as "compute artifacts sequentially via T4, then
  `from_artifacts(.., None)`" (one assembly path). Existing index tests stay green.
- **T8. `build_index_with_workspace_reported`** (serve.rs). Steps:
  1. scan;
  2. `ArtifactStore::load(flat dir)` into a `HashMap`;
  3. `par_iter` over files producing `(ScannedFile, SettledRead parts, raw sha, body sha, entry taken by value)`.
     The store map is wrapped in a `Mutex` only for the take, or entries are pre-split by path into
     a `Vec<Option<entry>>` aligned with `files`, which is preferred because it has no lock;
  4. hit, partial or miss, filling through T4;
  5. `fingerprint_from_hashes`;
  6. `DerivedCache::load(ns dir, &fp)` mapped to `Precomputed`;
  7. `from_artifacts`;
  8. on a derived miss, run conventions as today and save derived with `base_commit` logic unchanged;
  9. build the `BuildReport`;
  10. return.
  
  The store save (merge, then save) happens in the caller after `Ready` is published.
  `spawn_mcp_index_build` saves after publishing; other callers save immediately. This needs a
  `PendingStore` value carried out of the reported fn. Remove the stat-index load and save from
  this path; `StatIndex` and `content_fingerprint_with_stat_index` are `#[deprecated]`.
  `build_index_with_workspace` calls the reported fn, saves pending, and drops the report.
  `classify_build_outcome` prints the report. Turns T1.6–T1.18 green.

## Phase 3 — docs, measurement, gate

- **T9.**
  - `docs/adrs/INDEX.md`: rows for 0210 and 0211.
  - README: a short "Persistence" note: `.cxpak/cache/artifacts.json` is content-validated and
    safe to delete.
  - `benches/BASELINES.md`: the "MCP index-ready on real repos" section with the baseline and after
    tables.
- **T10.** Measure A1–A3 (spec §5.7) and the Phase 2 gate (an empty commit in the scriptorium
  clone, then 5 runs). Record them in BASELINES and the PR body. If A1–A3 fail, say so; do not
  merge on a claim.

## Risks

See spec §7. Additional:
- **T8 step 3 memory.** Entries must be moved, not cloned (perf review 4).
- **T1.10 subtlety.** A sentinel in `derived.json` appears in today's code too. The discriminator is
  therefore the test-only thread-local call counter, not a field the implementation reports about
  itself.

## Rollout

- Merge (agent reviewer ≠ author, CI green).
- Run `chore: release cxpak 3.5.0` as a lockstep bump PR. The files involved are `Cargo.toml`,
  `Cargo.lock`, `plugin/.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, and
  `plugin/lib/ensure-cxpak` `REQUIRED_VERSION`, following #170.
- Tag `v3.5.0`, which triggers `release.yml`.
- Bump the downstream pins (ai-sdlc `verify-pins`).
- The first start after upgrade re-derives per-file artifacts once; the derived cache is kept.

## Execution manifest

The local fleet is down and would need about 10 GB of RAM on a machine with about 0.1 GB free and
14 GB of swap. Starting it would degrade the other running sessions, so `local: true` nodes fail open
to the author. Accept commands run with `set -o pipefail` and require at least one passing test.

```json
{
  "execution-manifest": [
    {"id": "determinism", "files": ["Cargo.toml", "src/intelligence/cross_lang.rs"], "change": "Enable serde_json float_roundtrip; sort detect_cross_lang_edges output by the spec 5.5 key.", "accept": "set -o pipefail; cargo test --lib cross_lang_edges_are_sorted 2>&1 | grep -E 'test result: ok\\. [1-9]'", "local": false},
    {"id": "merge-store-entries", "files": ["src/cache/artifacts.rs"], "change": "Implement merge_store_entries per its doc comment: full scan keeps only fresh; workspace scan replaces entries under the prefix and keeps the rest; fresh replaces stale for the same path; output ordered by path (BTreeMap).", "accept": "set -o pipefail; cargo test --lib cache::artifacts::tests::merge 2>&1 | grep -E 'test result: ok\\. [1-9]'", "forbid": ["new_deps"], "local": true},
    {"id": "artifact-store", "files": ["src/cache/artifacts.rs", "src/cache/mod.rs"], "change": "ArtifactStore load/save (lock, atomic rename, fail-closed) and fingerprint_from_hashes extraction.", "accept": "set -o pipefail; cargo test --lib cache:: 2>&1 | grep -E 'test result: ok\\. [1-9]'", "local": false},
    {"id": "compute-file-artifacts", "files": ["src/index/mod.rs"], "change": "Implement compute_file_artifacts: tree-sitter parse via the registry language, token count via count_or_zero, term frequencies via compute_term_frequencies; Sync assertion for TokenCounter and LanguageRegistry.", "accept": "set -o pipefail; cargo test --lib compute_file_artifacts_matches_index_build 2>&1 | grep -E 'test result: ok\\. [1-9]'", "forbid": ["new_deps"], "local": true},
    {"id": "cli-on-store", "files": ["src/cache/parse.rs", "src/cache/mod.rs"], "change": "parse_with_cache on ArtifactStore; deprecate FileCache/CacheEntry.", "accept": "set -o pipefail; cargo test --lib cache::parse 2>&1 | grep -E 'test result: ok\\. [1-9]'", "local": false},
    {"id": "drop-token-patches", "files": ["src/commands/overview.rs", "src/commands/trace.rs"], "change": "Delete the cache.json token-count patch blocks.", "accept": "set -o pipefail; cargo test --lib commands::overview 2>&1 | grep -E 'test result: ok\\. [1-9]'", "local": false},
    {"id": "finish-build-precomputed", "files": ["src/index/mod.rs"], "change": "Precomputed, from_artifacts, finish_build(Option<Precomputed>), build_with_settled_content delegates.", "accept": "set -o pipefail; cargo test --lib index:: 2>&1 | grep -E 'test result: ok\\. [1-9]'", "local": false},
    {"id": "build-index-reported", "files": ["src/commands/serve.rs", "src/commands/warm_start_tests.rs"], "change": "Parallel per-file artifacts with store reuse, fingerprint from fresh hashes, derived restore without recompute, deferred store save, BuildReport and ready line.", "accept": "set -o pipefail; cargo test --lib warm_start_tests 2>&1 | grep -E 'test result: ok\\. [1-9]'", "local": false}
  ]
}
```

## Success criteria

A1–A17 are met and recorded. The PR is reviewed by a non-author agent. CI is green. 3.5.0 is
released and the downstream pins are bumped. The Phase 2 gate number is recorded.

---
title: cxpak 3.5.1 — in-place republish (Arc::get_mut) + bounded-batch build_index, doc fix for ConventionsStatus
status: DRAFT
created: 2026-10-18
updated: 2026-10-18
spec: specs/2026-10-18-peak-rss-regression-SPEC.md
adrs: 0213
---

# Plan — cxpak 3.5.1

## Gates (every node, before commit)

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/feature-matrix.sh
cargo tarpaulin --all-features --out Stdout 2>&1 | tail -5   # >= 90%
```

No node in this plan changes any `CodebaseIndex` field type or name — ADR-0213's prior `Arc`-wrap
decision was rejected on review as a semver break (`core_graph` is `pub mod`). Every node below is
internal-logic-only.

## Phase 1 — pin current semantics (RED, author-committed)

Node `pin-republish-get-mut-or-clone` (local: false — author it, don't let a local model write its
own test). Add `src/commands/warm_start_tests.rs::republish_conventions_mutates_in_place_when_sole_owner`
asserting two cases against `republish_conventions_with`:
1. **Sole owner** — construct the `SharedReadiness` cell holding the only strong ref to its `Arc`
   (no extra clone held by the test), call republish; assert via `Arc::ptr_eq` that the cell's `Arc`
   *pointer* is unchanged afterward (mutated in place, not swapped) and that `conventions`/
   `co_changes` reflect the mined values.
2. **Shared** — hold an extra clone of the cell's `Arc` across the call (simulating
   `snapshot_ready_index` mid-request); assert the cell's `Arc` pointer *does* change afterward
   (fell back to clone-and-swap) and the held extra clone still observes the pre-republish content
   (today's existing guarantee — must not regress).
This test must FAIL against today's unconditional-clone `republish_conventions_with` (case 1's
ptr_eq assertion fails: today it always swaps). That failure is the RED signal.

Node `pin-watcher-does-not-pin-index` (local: false). Two tests in `warm_start_tests.rs`:
1. `watcher_seed_does_not_pin_the_published_index` — build to Ready via `spawn_mcp_index_build`, start
   `spawn_mcp_watcher`, poll (deadline 5 s) until the cell's `Arc::strong_count` is 1 with no tool
   call in flight; assert it. Fails today: the watcher's `shared` cell keeps a second strong ref
   indefinitely, so republish's `get_mut` would fail in practice right after Ready.
2. `watcher_edit_after_republish_still_lands` — after the above, write a new file into the repo and
   poll (deadline 20 s) for the cell to publish an index containing it (no lost update when the
   base is fetched from the cell at batch time).

## Phase 2 — the fix

### Execution manifest

```json
{
  "execution-manifest": [
    {
      "id": "republish-in-place-drop-precapture",
      "files": ["src/commands/serve.rs"],
      "change": "One node (the pre-capture removal and the get_mut path cannot land separately: the first alone has no test that can pass, the second alone is defeated by the self-inflicted strong ref). (a) In the background-build closure (serve.rs:2784-2834) delete the persistent `ready_base`; republish_conventions takes `base: Arc<CodebaseIndex>` BY VALUE (moved in, dropped right after mining, before any get_mut) and returns Option<Arc<..>> (None = nothing published: mining panic, cell not Ready/ReadyEnriched). The embeddings gate re-derives its base by cloning the cell's `Ready` Arc after the store save, immediately before enrich_ready_with_embeddings. (b) In republish_conventions_with's retry loop, under the write lock try Arc::get_mut on the cell's Arc; on Some apply_to in place, flip status to Fresh, return Arc::clone; on None fall back to today's clone-and-compare-and-swap unchanged. (c) Log the path taken (in-place vs clone) on the existing stderr line. No change to CodebaseIndex or any public signature.",
      "accept": "set -o pipefail; cargo test --lib warm_start_tests::republish 2>&1 | grep -E 'test result: ok\\. [0-9]+ passed; 0 failed'",
      "forbid": ["new_deps"],
      "local": false,
      "kind": "edit",
      "red_test": "pin-republish-get-mut-or-clone (Phase 1)"
    },
    {
      "id": "watcher-fetch-base-on-batch",
      "files": ["src/commands/serve.rs"],
      "change": "spawn_mcp_watcher must not retain a strong Arc of the published index between edit batches. classify_watcher_wait's WatcherWait::Seed(Arc::clone(idx)) is dropped as soon as the watcher sees a queryable cell (the wait only needs to know Ready happened), and the long-lived `shared` mirror is removed. When a batch arrives the watcher builds a batch-local SharedIndex from the readiness cell's current Ready/ReadyEnriched Arc (readiness stays the single source of truth; ADR-0200 semantics preserved), calls process_watcher_changes, publishes the returned Arc as Ready, and drops the batch-local cell. A cell that is Building/Failed at batch time skips the batch. The overflow path no longer needs the placeholder swap (the only strong ref is the readiness cell, already replaced by Building).",
      "accept": "set -o pipefail; cargo test --lib warm_start_tests::watcher 2>&1 | grep -E 'test result: ok\\. [0-9]+ passed; 0 failed'",
      "forbid": ["new_deps"],
      "local": false,
      "kind": "edit",
      "red_test": "watcher_seed_does_not_pin_the_published_index + watcher_edit_after_republish_still_lands (Phase 1)"
    },
    {
      "id": "pin-batch-size-seam",
      "status": "REJECTED (ADR-0213 outcome: no measured benefit, +11-16 % ready time)",
      "files": ["src/commands/serve.rs", "src/commands/warm_start_tests.rs"],
      "change": "Author-committed RED first (local: false). Add build_index_pending_batched(path, workspace, defer_conventions, batch: Option<usize>) as the single implementation behind build_index_pending_with (which passes None), and BuildReport.batches (pub(crate), Default 0). RED test build_index_batched_matches_unbatched builds the warm fixture cold and warm with batch None and Some(1)/Some(2), asserting snapshot() parity, equal hit/partial/miss counts, equal `changed` key sets, and report.batches == ceil(files / batch). Against the pre-fix body (batch ignored, batches == 1) the batches assertion fails: that is the RED signal. State chunks are contiguous in scanner order; `stored` and `had_entry` are consumed in the same chunk boundaries as `files`, so file order and merge_store_entries input are unchanged.",
      "accept": "set -o pipefail; cargo test --lib warm_start_tests::build_index_batched_matches_unbatched 2>&1 | grep -E 'test result: FAILED'",
      "forbid": ["new_deps"],
      "local": false,
      "kind": "test"
    },
    {
      "id": "bounded-batch-fold",
      "status": "REJECTED (ADR-0213 outcome: no measured benefit, +11-16 % ready time)",
      "files": ["src/commands/serve.rs"],
      "change": "Replace build_index_pending_with's single files.par_iter().zip(stored.into_par_iter()).map(..).collect() (lines ~471-554) with a loop over fixed-size chunks of (files, stored) pairs: run the existing per-file closure unchanged within each chunk via par_iter/collect, then fold that chunk's Vec<Option<Prepared>> into the running report/changed/scanned/content_shas/artifact_files/body_shas accumulators (the loop body at lines ~564-581) before processing the next chunk. Chunk size: files.len() / num_cpus().max(1), clamped to a sane floor/ceiling, overridable through the `batch` parameter of build_index_pending_batched (pin-batch-size-seam), not a cfg(test) branch. Per-file logic and merge_store_entries semantics must not change.",
      "accept": "set -o pipefail; cargo test --lib warm_start_tests::build_index_batched_matches_unbatched 2>&1 | grep -E 'test result: ok\\. 1'",
      "forbid": ["new_deps"],
      "local": true,
      "kind": "edit",
      "depends_on": ["pin-batch-size-seam"]
    },
    {
      "id": "conventions-status-doc-fix",
      "files": ["src/core_graph/conventions.rs"],
      "change": "Reword ConventionsStatus's doc comment (lines ~78-83): remove the claim that the stale window is visible externally. State instead that the status is tracked internally (serve.rs) for the background-refresh logic and is not currently exposed in any MCP/HTTP/LSP tool response.",
      "accept": "grep -q 'not currently exposed' src/core_graph/conventions.rs",
      "forbid": ["new_deps"],
      "local": true,
      "kind": "edit"
    }
  ]
}
```

## Phase 3 — measure

Not local-executable: run `scripts/mcp-ready.py --lifecycle 20` interleaved, 3.4.0 (binary) vs the
fix, n ≥ 5, on `/tmp/frz/scriptorium`, scenarios U and C, **each under two workloads**:
- **Idle** — no concurrent tool calls during the republish window (the `Arc::get_mut` best case).
- **Concurrent** — a second process polling a cheap tool call (e.g. `cxpak_context` stats) once per
  second throughout the server's lifecycle, simulating `snapshot_ready_index` contention (the
  clone-fallback case).

Confirm median `peak_mb` (full lifecycle) ≤ 3.4.0 × 1.10 on both scenarios **under both workloads**,
and median `index_ready_s` ≤ 3.5.0's measured figure × 1.10 per scenario. If the concurrent workload
shows the gate failing because `get_mut` rarely succeeds in practice, that is ADR-0213's flagged
"Revisit if" — do not ship on the idle-only number. Record final numbers in `benches/BASELINES.md`
(new dated section) and in this plan's changelog before merge.

## Phase 4 — release

Bump `Cargo.toml` to 3.5.1, update `CHANGELOG`/release notes per repo convention. No public-API or
source-compat note is needed (no field type changed). Confirm `docs-with-code` (no public-surface
doc needs updating beyond the `conventions.rs` comment already in the manifest).

## Outcome (2026-10-18)

`pin-batch-size-seam` and `bounded-batch-fold` were implemented, measured and reverted (ADR-0213
"Outcome"). Added instead: `DerivedCacheRef`, a borrowed serialization view, used by every derived
save path so no graph/call-graph/pagerank/conventions clone is made to write the cache.

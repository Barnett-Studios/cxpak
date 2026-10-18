---
title: cxpak 3.5.1 — bounded-batch build_index, doc fix for ConventionsStatus
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
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/feature-matrix.sh
cargo tarpaulin --all-features --out Stdout 2>&1 | tail -5   # >= 90%
```

## Phase 1 — pin current semantics (RED, author-committed)

Node `pin-merge-store-semantics` (local: false — this is the RED baseline the local nodes build
against; author it, don't let a local model write its own test). Add
`src/commands/warm_start_tests.rs::build_index_batched_matches_unbatched` asserting that
`build_index` run on a small fixture repo (reuse the existing warm-start fixture builder) produces
byte-identical `BuildReport` counts, identical `changed` key set, and identical `CodebaseIndex`
term-frequency/call-graph output regardless of an internal batch-size override (test two
sizes: 1 and `usize::MAX`, i.e. unbatched). This test must compile against today's `build_index`
signature (it will need a `#[cfg(test)]` batch-size hook added as a RED step too — see node
`batch-size-seam`) and must FAIL before `bounded-batch-fold` lands (today's code has no seam).

## Phase 2 — the fix

### Execution manifest

```json
{
  "execution-manifest": [
    {
      "id": "batch-size-seam",
      "files": ["src/commands/serve.rs"],
      "change": "Add a #[cfg(test)]-only way for warm_start_tests.rs to override build_index's internal batch size (a thread-local or a parameter threaded only under cfg(test)); production code path keeps today's single call site with a real default computed from num_cpus. Do not change build_index's public behavior or signature.",
      "accept": "cargo test --lib warm_start_tests::build_index_batched_matches_unbatched 2>&1 | grep -E 'test result: FAILED|error\\[' ",
      "forbid": ["new_deps"],
      "local": false,
      "kind": "edit"
    },
    {
      "id": "bounded-batch-fold",
      "files": ["src/commands/serve.rs"],
      "change": "Replace build_index's single files.par_iter().zip(stored.into_par_iter()).map(..).collect() (lines ~471-554) with a loop over fixed-size chunks of (files, stored) pairs: run the existing per-file closure unchanged within each chunk via par_iter/collect, then fold that chunk's Vec<Option<Prepared>> into the running report/changed/scanned/content_shas/artifact_files/body_shas accumulators (the loop body at lines ~564-581) before processing the next chunk. Chunk size comes from the seam added in batch-size-seam. Per-file logic and merge_store_entries semantics must not change.",
      "accept": "set -o pipefail; cargo test --lib warm_start_tests::build_index_batched_matches_unbatched 2>&1 | grep -E 'test result: ok\\. 1'",
      "forbid": ["new_deps"],
      "local": true,
      "kind": "edit"
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

## Phase 3 — measure and tune

Not local-executable: run `scripts/mcp-ready.py` interleaved, 3.4.0 (tag/binary) vs the fix, n ≥ 5,
on `/tmp/frz/scriptorium`, for warm start. Tune the batch-size default in `bounded-batch-fold` until
median peak RSS ≤ 3.4.0 × 1.10 and median `index_ready_s` ≤ 3.5.0's warm figure × 1.10. Record the
final numbers and chosen batch-size formula in `benches/BASELINES.md` (new dated section) and in
this plan's changelog before merge.

## Phase 4 — release

Bump `Cargo.toml` to 3.5.1, update `CHANGELOG`/release notes per repo convention, confirm
`docs-with-code` (no public-surface doc needs updating beyond the `conventions.rs` comment already
in the manifest).

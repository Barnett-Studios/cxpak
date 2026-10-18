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

## Phase 2 — the fix

### Execution manifest

```json
{
  "execution-manifest": [
    {
      "id": "drop-ready-base-precapture",
      "files": ["src/commands/serve.rs"],
      "change": "In the background-build closure (serve.rs:2784-2834), stop pre-cloning ready_base = Arc::clone(idx) at line 2785 as a persistent value held across the conventions-republish/store-save/embeddings sequence. Replace its two uses: (a) the fallback `base` argument to republish_conventions — have republish_conventions fetch its own fallback by cloning the cell's current Arc only inside the branches that need one (mining panic, cell not Ready/ReadyEnriched), never held outside those branches; (b) the embeddings-phase gate (`if let Some(base) = ready_base { enrich_ready_with_embeddings(...) }`) — re-derive this from a non-owning check (did `next` publish Ready/ReadyEnriched?) plus a fresh clone taken only at the point enrich_ready_with_embeddings is actually called, not held earlier. No change to CodebaseIndex or any public signature.",
      "accept": "cargo build --lib 2>&1 | tail -5; cargo test --lib warm_start_tests:: 2>&1 | grep -E 'test result: FAILED' ; test $? -ne 0",
      "forbid": ["new_deps"],
      "local": false,
      "kind": "edit"
    },
    {
      "id": "republish-get-mut-with-clone-fallback",
      "files": ["src/commands/serve.rs"],
      "change": "In republish_conventions_with's retry loop (serve.rs:2892-2927), before building `next = (**cur).clone()`, acquire the write lock once and attempt Arc::get_mut on the cell's Arc (requires drop-ready-base-precapture to have landed first, so no self-inflicted extra clone remains). On Some(index_mut): apply the mined conventions/co_changes to *index_mut in place (no clone, no new Arc), flip status to Fresh, return Arc::clone of the now-mutated cell Arc. On None (a concurrent snapshot or reader holds a clone): fall back to today's clone-and-compare-and-swap loop unchanged. Per-attempt behavior (ATTEMPTS=3, already-fresh short-circuit) stays as today for the fallback path.",
      "accept": "set -o pipefail; cargo test --lib warm_start_tests::republish_conventions_mutates_in_place_when_sole_owner 2>&1 | grep -E 'test result: ok\\. 1'",
      "forbid": ["new_deps"],
      "local": true,
      "kind": "edit"
    },
    {
      "id": "bounded-batch-fold",
      "files": ["src/commands/serve.rs"],
      "change": "Replace build_index's single files.par_iter().zip(stored.into_par_iter()).map(..).collect() (lines ~471-554) with a loop over fixed-size chunks of (files, stored) pairs: run the existing per-file closure unchanged within each chunk via par_iter/collect, then fold that chunk's Vec<Option<Prepared>> into the running report/changed/scanned/content_shas/artifact_files/body_shas accumulators (the loop body at lines ~564-581) before processing the next chunk. Chunk size: files.len() / num_cpus().max(1), clamped to a sane floor/ceiling, overridable only under #[cfg(test)] for pin-batch-size-seam. Per-file logic and merge_store_entries semantics must not change.",
      "accept": "set -o pipefail; cargo test --lib warm_start_tests::build_index_batched_matches_unbatched 2>&1 | grep -E 'test result: ok\\. 1'",
      "forbid": ["new_deps"],
      "local": true,
      "kind": "edit",
      "note": "needs its own RED test (pin-batch-size-seam, author it before this node, same pattern as pin-republish-get-mut-or-clone) asserting build_index's BuildReport/changed/CodebaseIndex output is byte-identical batched vs unbatched on the existing warm-start fixture — omitted here for brevity, required before this node is accepted."
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

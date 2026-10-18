---
title: cxpak 3.5.1 — Arc-share CodebaseIndex heavy fields, doc fix for ConventionsStatus
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

## Phase 1 — pin current semantics + audit (RED, author-committed)

Node `pin-no-deep-clone-on-republish` (local: false — this is the RED baseline the local nodes
build against; author it, don't let a local model write its own test). Add
`src/commands/warm_start_tests.rs::republish_conventions_shares_untouched_fields` that calls
`republish_conventions_with` on a fixture `CodebaseIndex` and asserts, via `Arc::ptr_eq`, that the
returned index's `files`, `graph`, `call_graph`, `pagerank`, and `term_frequencies` `Arc`s are
*identical pointers* to the input's (proving no deep clone happened for fields the mining pass does
not touch), while `conventions` and `co_changes` point at *different* `Arc`s (proving the mined
values were actually applied). This test must FAIL to compile against today's plain (non-`Arc`)
field types — that compile failure is the RED signal; it will only compile and pass once
`arc-wrap-codebase-index` and `republish-no-deep-clone` land.

Node `audit-external-field-access` (local: false — a judgment call, not a mechanical edit). Grep
this repo and, if reachable, `conductr-core`'s vendored/pinned `cxpak` usage for direct field access
or struct-literal construction of `CodebaseIndex` outside `src/core_graph/index.rs` and
`src/commands/serve.rs`. Read access is unaffected by `Arc`'s auto-deref; construction or
move-out sites need `Arc::new(..)`/cloning adjustment. Record findings as a short comment block atop
`arc-wrap-codebase-index`'s diff; if any out-of-crate construction site is found, stop and escalate
(ADR-0213's "Revisit if" — this is the risk that ADR flagged as unverified).

## Phase 2 — the fix

### Execution manifest

```json
{
  "execution-manifest": [
    {
      "id": "arc-wrap-codebase-index",
      "files": ["src/core_graph/index.rs"],
      "change": "Wrap CodebaseIndex's term_frequencies, graph, call_graph, pagerank, conventions, and co_changes fields in Arc, following the existing embedding_index: Option<Arc<EmbeddingIndex>> pattern (ADR-0205) at lines 48-55. Update the struct's constructor(s) and every in-crate construction/move-out site flagged by audit-external-field-access to Arc::new(..) the value. Do not change field names or add/remove fields.",
      "accept": "cargo build --lib 2>&1 | grep -E 'error\\[E0308\\]|error\\[E0277\\]' ; test $? -ne 0",
      "forbid": ["new_deps"],
      "local": false,
      "kind": "edit"
    },
    {
      "id": "republish-no-deep-clone",
      "files": ["src/commands/serve.rs"],
      "change": "Replace republish_conventions_with's `let mut next = (**cur).clone(); mined.clone().apply_to(&mut next);` (lines ~2909-2910) with: construct `next` by cloning cur's now-cheap Arc fields directly (files, graph, call_graph, pagerank, term_frequencies, plus the scalar fields total_files/total_bytes/total_tokens/language_stats/domains/schema/test_map/cross_lang_edges/embedding_index unchanged), then compute the mined conventions/co_changes as fresh values and assign them as new Arcs on next — ConventionProfile::apply_to (or its replacement) must build a fresh ConventionProfile and co_changes Vec rather than mutating through the Arc. No field of next may be a freshly deep-copied value except conventions and co_changes.",
      "accept": "set -o pipefail; cargo test --lib warm_start_tests::republish_conventions_shares_untouched_fields 2>&1 | grep -E 'test result: ok\\. 1'",
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

## Phase 3 — measure

Not local-executable: run `scripts/mcp-ready.py --lifecycle 20` interleaved, 3.4.0 (binary) vs the
fix, n ≥ 5, on `/tmp/frz/scriptorium`, scenarios U (warm unchanged) and C (warm after an empty
commit — exercises the `StalePending`/`republish_conventions` path this fix targets). Confirm median
`peak_mb` (full lifecycle, not just at Ready) ≤ 3.4.0 × 1.10 on both scenarios and median
`index_ready_s` ≤ 3.5.0's measured figure × 1.10 per scenario. If scenario U alone still fails the
RSS gate after this fix (ADR-0213 predicts Option B does not target the build-phase peak), pull in
bounded-batch-fold (ADR-0213 Option A) as a follow-up node, tuned and measured the same way, before
this release ships. Record final numbers in `benches/BASELINES.md` (new dated section) and in this
plan's changelog before merge.

## Phase 4 — release

Bump `Cargo.toml` to 3.5.1, update `CHANGELOG`/release notes per repo convention — note the
`CodebaseIndex` field-type change (same names, `Arc`-wrapped) as a source-compat note for any
out-of-crate consumer even though it is not a `cxpak::cache` public-surface or on-disk format
change. Confirm `docs-with-code` (no public-surface doc needs updating beyond the `conventions.rs`
comment already in the manifest and this release-notes line).

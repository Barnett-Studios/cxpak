---
id: '0215'
title: Attribute scenario C max RSS by phase before cutting; "drop content from Prepared" is a no-op because content moves into the index
status: ACCEPTED
date: 2026-10-18
triggered_by: ADR-0213 Outcome (scenario C max RSS 570 MB vs 504 MB limit) and its "Revisit if" naming Prepared.content
loop: planning
---

# ADR-0215: Attribute scenario C max RSS by phase before cutting

## Context

ADR-0213 shipped with scenario C max RSS at 569.5 MB against 504.2 MB (3.4.0 x 1.10); footprint and
scenario U meet the gate. Its fallback was to drop `content` from `Prepared` (`serve.rs:370`).
Tracing shows the file text is moved from the settled read (`serve.rs:483`) into `Prepared`
(`serve.rs:543`) and then into the public `IndexedFile.content` (`src/index/mod.rs:692`), where
tools, LSP, hooks and convention miners read it. It is never copied, so removing it from `Prepared`
frees nothing; removing it from the index is a public-API and behaviour change. Scenario C is a derived-cache
miss (HEAD is part of the fingerprint), so the 185 MB gap to U is build-time graph/PageRank/call-graph
analysis plus background mining and republish; the split is not yet attributed.

## Options considered

- **Option A — drop `content` from `Prepared` as ADR-0213 worded it.** No API risk, small diff.
  Rejected on evidence: ownership moves, so peak is unchanged. Someone could prefer it for being
  the documented next step; it would need a measurement showing otherwise.
- **Option B — drop or lazily load `IndexedFile.content`.** The only change that removes the text
  itself (~the whole repo size). Rejected for 3.5.x: `IndexedFile` is `pub`, every consumer in
  section 2 of the spec reads the field; this is a semver-minor/major design (separate ADR).
- **Option C — attribute first, then cut the transient duplicates.** Add phase-tagged RSS sampling
  (build, fold, derived load, mining, republish, store/derived save). Then target what the data
  names, within no public API change: mining allocations (`git_health` revwalk and path-list copies, the `co_changes` double hold), `finish_build` transient maps, release of build transients before the mining phase starts. Pros: evidence-led, API
  safe. Cons: may not reach 504 MB; macOS max RSS is a high-water mark that frees do not lower, so
  only avoided peaks count.
- **Option D — accept 570 MB.** Footprint (the gate metric the product cares about) passes; document
  that RSS gate is unmet. Legitimate if C fails to move; costs the stated 1.10x gate on RSS.

## Decision

Option C. Attribution first (RED test pins whichever duplicate the data names), Option A is not
implemented. If measurement shows no no-API lever reaches 504 MB median, ship the best measured
reduction (or none) with an honest unmet-gate note, and open a separate ADR for Option B.

## Consequences

### Positive
- No public API change; decision follows a measured attribution rather than a hypothesis.
### Negative
- The outcome is uncertain; gate may remain unmet.
- Phase sampling needs a small, non-default instrumentation hook or an external sampler.
### Neutral
- Content stays resident in the index; steady-state memory is unchanged.

## Revisit if

- Attribution shows the C-only peak is mining allocations inherent to `build_convention_profile`
  (not duplicates): then only Option B or a streaming miner moves it.
- A consumer audit shows `IndexedFile.content` can become lazy without breaking its readers.

## Outcome (2026-10-18)

Option C's attribution step ran; no cutting lever was implemented, and the RSS gate is neither met nor
shown to be missed on stable ground (Option D in effect). Measured on a fresh scriptorium clone
(`benches/BASELINES.md`, "max RSS re-measurement"), n=5 interleaved, load average 8-38:

- Peak memory footprint: 3.5.1 below 3.4.0 (U 376.7 vs 429.6; C 650.7 vs 689.0).
- Max RSS, 3.5.1: U 314.3, C 350.2; 3.4.0: U 252.9, C 306.8. C is under the fixed 504 MB limit and
  above 3.4.0 x 1.10 (337); U is above 3.4.0 x 1.10 (278). The 570 MB of ADR-0213 did not reproduce, and
  3.4.0's own RSS read 40-45 % lower than in that series, so the RSS metric moves with host memory
  pressure by more than the effect being chased.
- The maximum falls in the build phase (build peak 329-334 MB at 0.6-0.9 s, post-ready peak 254-339 MB),
  not in conventions mining or republish. This reverses the C-minus-U premise: the lever candidates in
  `git_health` (path-list copy, libgit2 cache) and `co_changes` sit in the non-binding phase and were
  not pursued; jemalloc decay tuning gave no reduction.
- `Prepared.content` removal remains a no-op (content is moved into the public `IndexedFile`).

Decision stands for any future attempt: attribute on `footprint` (stable) and a quiet host before
cutting. The remaining build-phase candidate, not tried, is the resident artifact store (57.6 MB JSON
loaded whole as `BTreeMap` entries at `serve.rs:~455`, ahead of the per-file stage); it needs streaming
parse and its own ADR.

## Option: consume the artifact store during the build (traced, not implemented)

Proposal: take the loaded store by value and move entries out as each file is prepared, so the store's
copy is freed before the index peaks. Trace of the current code (`src/commands/serve.rs`):

- `ArtifactStore::load` (`serve.rs:454`) streams the JSON (`src/cache/artifacts.rs:~58`), so no text copy
  sits next to the parsed form.
- Entries are already moved out, never cloned, into `stored: Vec<Option<ArtifactEntry>>`
  (`store.entries.remove`, `serve.rs:~457`) and the map is dropped (`drop(store)`, `serve.rs:~466`).
  `stored.into_par_iter()` then consumes each entry by value; on a hit its `parse_result` is moved and its
  `term_frequencies` rebuilt as the index-shaped `HashMap`, and the entry is freed at the end of that
  file's closure. `Prepared.fresh` and `changed` are empty on warm runs.
- The save path (`PendingStore::save`, `serve.rs:~324`) returns early when nothing is dirty, so scenarios
  U and C never reload the store. When dirty it reloads the store next to the live index and then
  serializes with `to_string` (`artifacts.rs:~76`, a second full-size text copy); that affects cold and
  changed builds only and was not measured.

So the proposed consumption is already how the build works; a structural RED test ("the store map no
longer holds moved-out entries after prepare") passes on today's code and cannot be written as a RED.
The store's entries are alive until their file is prepared, which is the bulk of the per-file stage, but
that is inherent: each entry's data turns into the index's data and the footprint is the sum. No code
was changed and nothing was measured for this option, because there is no change to measure.

## Gate decision

Peak memory footprint is the gate: it is stable run to run (3.5.1 U 376.5-377.4, C 645.6-654.8) and it
passes against 3.4.0 x 1.10 in both scenarios (U 376.7 vs 429.6; C 650.7 vs 689.0 measured the same day).
Max RSS is reported but not gated: it is not reproducible under macOS compressed-memory accounting (the
same 3.4.0 binary read 40-45 % lower than in the ADR-0213 series; identical runs differ by 100+ MB).
Any future build-phase attempt needs a new design (for example a streamed, incrementally-consumed store
format, which is an on-disk format change) and its own ADR.
